# Architecture

This directory is the per-layer companion to the root [`README.md`](../../README.md)
and [`TRACKER.md`](../../TRACKER.md). The README gives the one-paragraph shape of
the workspace and how to run it; the tracker gives the honest, crate-by-crate
test status. This directory exists for the question those two don't answer:
*why is the workspace shaped this way, and what does each layer refuse to know
about the others?*

It was written after the fact, from the crates as they actually exist and the
module docs already committed alongside them — not as a design document that
predates the code. Where a crate's own `lib.rs` doc already explains something
well, these files point at it rather than restating it; where the *interesting*
part is how two or three crates fit together, that's what's written out here.

## The rule

One dependency direction, no exceptions:

```
widget (vieww-widget)
  -> element (vieww-element)
    -> render (vieww-render)
      -> scene (vieww-scene)
        -> render graph (vieww-render-graph)
          -> render planner (vieww-render-planner)
            -> CPU (vieww-paint) | GPU (vieww-gpu + vieww-hal + vieww-shaders) | Hybrid
```

A crate on the left never depends on, imports from, or reaches into a crate on
the right. `vieww-widget` cannot see a `RenderId`; `vieww-render` cannot see a
`Placement::Gpu`. Each arrow is a real `Cargo.toml` dependency edge, and each
absence of an arrow (`vieww-widget` has no path to `vieww-scene`, transitively
or otherwise) is enforced by there being no such dependency to add, not by
convention alone.

Two families of crates sit *beside* that spine rather than on it, and both are
covered in their own file here:

- **Cross-cutting crates** — `vieww-foundation`, `vieww-text`, `vieww-asset`,
  `vieww-image`, `vieww-gestures`, `vieww-animation`, `vieww-accessibility`,
  `vieww-interaction`, `vieww-scroll`, `vieww-hardware`, `vieww-platform` /
  `vieww-platform-winit`, `vieww-effects`. Each depends on `vieww-foundation`
  and little else, and is usable without pulling in the render stack at all.
  See [`cross-cutting.md`](./cross-cutting.md).
- **Tooling and distribution crates** — `vieww-devtools`, `vieww-test-harness`,
  `vieww-plugin` / `vieww-plugin-macros`, `vieww-build` / `vieww-cli`,
  `vieww-reload`. These sit *around* the framework rather than in its runtime
  path: an application never links against `vieww-cli`, and `vieww-devtools`'s
  inspector is read-only by construction. See
  [`tooling-and-distribution.md`](./tooling-and-distribution.md).

## The files in this directory

| File | Covers |
|---|---|
| [`ui-tree.md`](./ui-tree.md) | The three trees: `vieww-widget` → `vieww-element` → `vieww-render`, and why there are three instead of one. |
| [`rendering-pipeline.md`](./rendering-pipeline.md) | `vieww-scene` → `vieww-render-graph` → `vieww-render-planner` → CPU (`vieww-paint`) / GPU (`vieww-gpu`, `vieww-hal`, `vieww-shaders`) / Hybrid, and `vieww-runtime`'s frame orchestration around all of it. |
| [`cross-cutting.md`](./cross-cutting.md) | The crates every layer can reach: foundation, text, assets/images, gestures/animation, accessibility, interaction/scroll, hardware/platform capability probing. |
| [`tooling-and-distribution.md`](./tooling-and-distribution.md) | Devtools, the test harness, the plugin ABI, packaging/CLI, and hot reload — everything that ships around an application rather than inside one. |

## What this directory is not

It is not a substitute for reading a crate's own `lib.rs` doc — every crate
listed above already carries one, written in more depth than a cross-crate
summary can afford, and these files cite them rather than duplicate them. It
is also not a design proposal: everything described here is built, compiled,
and — except for the GPU-execution and non-Linux-toolchain paths called out
explicitly in [`TRACKER.md`](../../TRACKER.md) — tested on this machine.
