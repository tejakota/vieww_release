# ViewW

A Rust UI framework built as a stack of independently testable layers —
widgets describe intent, elements hold identity across rebuilds, a scene
graph and render graph describe what to draw, and CPU and GPU backends
actually draw it. Cross-platform by design: winit for desktop windowing, a
capability layer for camera/location/biometrics/sensors/notifications, and a
stable C-ABI plugin system for loading third-party `cdylib`s independent of
which `rustc` built them.

**New here?** [`docs/guide/`](./docs/guide/README.md) is the documentation for
people *building an application* with vieww — getting started, widgets,
state, layout, theming, testing and accessibility. Everything else linked
below is written for people working *on* vieww.

Three files answer "what is actually true about this repository", and they are
deliberately different questions:

- [`TRACKER.md`](./TRACKER.md) — what has been **done**, and how it was
  verified. A crate-by-crate status matrix separating what is real and tested
  on this machine, what is real but pending a GPU-backed machine or a
  platform-specific toolchain, and what is still scaffolding.
- [`PENDING.md`](./PENDING.md) — what has **not** been done. Every known gap,
  what it blocks, why it is not closed, and the command that would start it.
  Nothing is on both lists.
- [`docs/architecture/`](./docs/architecture/README.md) — the *why* behind the
  layering below, one file per group of crates.

## Layering, and the rule that keeps it a layering

Each stage below depends only on the ones before it — a widget never reaches
past its own element into the render tree, and nothing above `vieww-scene`
knows whether a given frame will be drawn by the CPU rasterizer or a GPU
backend:

```
widget (vieww-widget)
  -> element (vieww-element)
    -> render (vieww-render)
      -> scene (vieww-scene)
        -> render graph (vieww-render-graph)
          -> render planner (vieww-render-planner)
            -> CPU (vieww-paint) | GPU (vieww-gpu -> vieww-hal) | Hybrid
```

The GPU branch is vieww's own, all the way down. `vieww-gpu` is the
architecture — scene planning, tessellation, batching, resource
virtualisation — and knows nothing about any graphics API; `vieww-hal` is the
backends under it, Vulkan first, with Metal and D3D12 behind the same traits.
There is no `wgpu` in this workspace and that is a deliberate choice rather
than an omission: the render graph, the damage tracking, the CPU/GPU
placement decision and the frame scheduling are the things vieww exists to
own, and designing them around another abstraction's model is the outcome
that choice avoids.

**What the GPU path can draw today**: fills, strokes, transforms, per-shape
colour, alpha blending, rectangular clips — and **text** — batched into one
vertex buffer and as few draw calls as the scissor changes allow, verified
against the CPU rasterizer pixel for pixel (`cargo test -p vieww-hal --features
vulkan -- --ignored`). **What it cannot**: images, shadows, gradients, shaped
clips and layers. `vieww_gpu::ScenePlan::is_complete` is the gate that says
which, and a frame it refuses belongs on the CPU rasterizer — which is what
every screenshot in this repository was drawn with.

Text was the entry on that second list that mattered, because every real
application screen has a label on it: a renderer that cannot draw text cannot
draw a single real frame, whatever else it supports. `vieww_gpu::Planner` holds
a glyph atlas and turns a glyph run into one textured quad per glyph, in the
same batch as the shapes around it — a panel and the label on it are one
`draw_indexed`, because solid geometry samples a reserved full-coverage texel
in the same atlas rather than needing a second pipeline.

The coverage in that atlas is **the CPU rasterizer's own**, not a second
implementation of it. That is what lets `tests/vulkan_text.rs` assert something
much stronger than the shape suite can: every pixel of every glyph, within
1/255, rim included — a bound that a mirrored frame, a glyph placed one pixel
off, or a colour applied before coverage instead of after cannot fit inside.
See `vieww_paint::native::glyph_coverage`'s module doc for why a second glyph
rasterizer would have made that suite unable to catch anything at all.

Cutting across that stack: `vieww-foundation` (geometry, color, input,
accessibility preferences, capability types — no dependency on anything
above it), `vieww-gestures`, `vieww-animation`, `vieww-text`, `vieww-asset`,
`vieww-image`, and the platform bridge (`vieww-platform`,
`vieww-platform-winit`, `vieww-hardware`).

## Quick start

```sh
# Check everything compiles.
cargo check --workspace

# Run one crate's tests.
cargo test -p vieww-widget

# The renderer's own tests need its feature — `vieww-paint`'s rasterizer is
# behind `native` so that the layout and paint *model* stay dependency-free.
# `ci/check/checks.sh` turns it on for the whole workspace run.
cargo test -p vieww-paint --features native

# Run a feature example (headless — no window, no GPU).
cargo run -p feature-accessibility-audit

# Every rendering feature as a picture and a number, plus GIFs of the
# animated ones. This is where a rendering change is reviewed.
cargo run --release -p fixtures

# Scaffold a new vieww application.
cargo run -p vieww-cli -- new my-app

# Every check, certification and release script: `ci/vieww` lists them
# (see ci/README.md). Everything CI runs, in order:
ci/vieww checks

# Releasing: run the gate on each OS, then follow
# docs/release/BETA-RELEASE-CHECKLIST.md.
ci/vieww gate --mode gpu

# Does this compile for a given platform on this machine? One command each,
# and each either passes, fails at the compiler, or names the missing SDK,
# target or toolchain rather than failing forty seconds in.
./ci/check/platform-check.sh                 # every one this machine can attempt
./ci/check/platform-check.sh macos           # or one of: linux macos windows ios android web
```

**Debug builds are usable, and that is not free.** `[profile.dev]` in
`Cargo.toml` optimises the pixel-facing crates (`vieww-paint`, `vieww-text`,
`vieww-foundation`, `vieww-effects`) while leaving the widget, element and
application layers unoptimised and debuggable. Without those overrides an
unoptimised `vieww-paint` makes a frame take seconds rather than milliseconds,
which presents as a window that appears to hang rather than as a slow one — see
that file's comment, and `TRACKER.md` for the measurement.

Every crate under `crates/` is independently testable; `examples/features/*`
is a growing library of small, focused, headless demonstrations of one
feature each (see that directory for the full numbered list).

## Building a plugin

`vieww-plugin` and `vieww-plugin-macros` let a third-party `cdylib` — built
against a different `rustc` release than the host, at a different time —
load into a running `vieww` application. Implement `vieww_plugin::Plugin`,
annotate the type with `#[vieww_plugin::vieww_plugin]`, and build it as a
`cdylib`; a host loads it with `vieww_plugin::PluginRegistry::load`. See
`crates/vieww-plugin/src/abi.rs`'s module doc for why the boundary is a
`#[repr(C)]` vtable of function pointers rather than a Rust trait object, and
`crates/vieww-plugin/tests/example_plugin.rs` for a complete, real,
end-to-end example (built and `dlopen`ed for real, not simulated).

## Contributing conventions

- No `serde` anywhere in this workspace, by design — see
  `crates/vieww-devtools/src/json_export.rs`'s module doc for the reasoning
  and the hand-rolled alternative. New code should follow the same
  minimal-dependency ethos: reach for a real, justified dependency only when
  hand-rolling the actual surface needed would be a worse trade, and say so
  in a doc comment when you do.
- Every doc comment on a non-trivial public item should say *why*, not just
  *what* — see almost any file in `crates/vieww-foundation` for the house
  style.
- An honest stub beats code that looks like it works and doesn't. If a real
  API doesn't exist yet on some platform (see `vieww-interaction`'s
  `NullDragStarter` for an example), say so in the doc rather than faking a
  success path.
