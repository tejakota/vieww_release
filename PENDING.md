# ViewW pending work

Everything known to be missing, in one place.

## How this file relates to `TRACKER.md`

They answer opposite questions and nothing belongs on both.

- [`TRACKER.md`](./TRACKER.md) is what has been **done**, and how it was
  verified. It is a record, written after the fact, and its value is that its
  claims are checkable.
- This file is what has **not**. Its value is different: it is the thing that
  stops a gap from being rediscovered, re-scoped and re-argued by the next
  person, and it is the answer to "what would I work on".

When an item here is finished it moves to `TRACKER.md` with the evidence
attached, and its entry here is deleted rather than ticked. A file of
struck-through lines is an archive; this is meant to be a worklist.

Every item says four things: **what** it is, **why** it is not done, **what it
blocks**, and **where to start** — a real command or a real file, not a
gesture. An item that cannot say the fourth is not understood well enough to be
written down yet, and there are two of those, marked as such.

## Read this before believing anything is easy here

Two facts sit behind most of the list and are not repeated in every entry:

1. **No code in this workspace has ever run on real graphics hardware.** The
   GPU path is verified against `lavapipe`, Mesa's *software* Vulkan
   implementation. Every GPU statement anywhere in this repository is a
   correctness claim and none of them is a performance claim.
2. **Only Linux has ever been built.** macOS, Windows, iOS, Android and the web
   are real, reviewed, unit-tested code that no compiler for that target has
   ever seen. `ci/check/platform-check.sh` is one command each that says so
   precisely, and is where every one of those entries starts.

---

# 1. Blocked on hardware or a toolchain

Not ordinary work. Each of these needs a machine this one is not, and none of
them can be closed by reading the code more carefully.

## 1.1 A GPU frame on real graphics hardware

**What.** Run `cargo test -p vieww-hal --features vulkan -- --ignored` on a
machine with a discrete or integrated GPU, and measure `examples/fixtures` with
`Placement::Gpu` where `ScenePlan::is_complete` allows it.

**Why not.** This machine has no GPU. `lavapipe` stands in, and it is a
software rasterizer wearing a Vulkan interface — it can prove that the shader
maths, the vertex layout, the atlas coordinates and the blend state are right,
and it can prove nothing whatever about speed.

**Blocks.** Every performance claim about the GPU path. Also the only honest
answer to "is the GPU actually faster than your CPU rasterizer here", which is
currently unknown in both directions: the CPU path is unusually good, and the
GPU path is one draw call for a whole frame, and nobody has measured them
against each other on hardware.

**Start.** The tests already exist and already run; a GPU-backed machine needs
to do nothing but run them, then run the gallery. Record the numbers in
`TRACKER.md` beside the CPU ones, and delete the "no number was measured on
real hardware" caveat from this file, `TRACKER.md`, `README.md`,
`docs/guide/README.md` and `crates/vieww-hal/src/lib.rs` — it is deliberately
stated in all five.

## 1.2 macOS — the Metal backend

**What.** `crates/vieww-hal/src/metal.rs` brings a device and a command queue
up, and stops there. `render_clear_to_pixels` and `render_mesh_to_pixels` are
not ported, and neither is `SceneRenderer`.

**Why not.** No macOS Rust target and no Xcode here, so not one line of it can
be type-checked, let alone run. The module's own doc explains at length why an
unverified 150-line pipeline port was not written blind, and that reasoning
still holds.

**Blocks.** macOS and iOS entirely — they share this backend.

**Start.** `ci/check/platform-check.sh macos` on a Mac. Expect the compile to have
things to say; that is the point. Then port
`vulkan::VulkanDevice::render_clear_to_pixels` onto the calls
`src/metal.rs`'s doc lists (they were checked against the real `metal` 0.31
source, so the port is translation rather than discovery), translating WGSL with
`naga::back::msl::Writer` instead of the SPIR-V writer.

## 1.3 Windows — the D3D12 backend

**What.** `crates/vieww-hal/src/d3d12.rs` does adapter and device bring-up and
one command queue. No render pipeline.

**Why not.** As above: no Windows target, no MSVC.

**Blocks.** Windows GPU rendering. Note that Windows also has a *separate*
unsolved problem — see 5.3.

**Start.** `ci/check/platform-check.sh windows` on a Windows machine with the MSVC
toolchain.

## 1.4 iOS

**What.** `crates/vieww-platform-winit/src/ios.rs` is 140 lines of real code
that has never been compiled. No frame has ever reached an iPhone, simulated or
real.

**Why not.** The iOS SDK exists only on macOS.

**Blocks.** The mobile half of "cross-platform".

**Start.** `ci/check/platform-check.sh ios` for the compile, then
`ci/mobile/ios-app.sh --sim` — which already exists and already knows how to build a
`.app`, boot a simulator and install onto it.

## 1.5 Android

**What.** Real code, an NDK-aware build (`ci/mobile/apk.sh`), a device suite
(`ci/mobile/device-suite.sh`), an accessibility suite (`ci/mobile/a11y-android.sh`) and a
hot-reload path — none of it exercised here.

**Why not.** `rustup target add aarch64-linux-android` needs
`static.rust-lang.org`, which this machine's network policy does not reach; and
there is no NDK and no device.

**Blocks.** The other mobile half.

**Start.** `ci/check/platform-check.sh android` for the compile. `ci/mobile/android-env.sh`
already resolves the SDK and NDK and already knows the traps (a version
directory without `source.properties` is a half-finished download); ask it
rather than guessing.

## 1.6 The web backend on other browsers

**What.** The byte-for-byte canvas parity (`examples/test-web`,
`verify_web.py`) is a Chromium-on-Linux result. `ci/check/wasm-check.sh` passed on
2026-09-14; see `TRACKER.md`. (This entry used to say the crate had never been
compiled, which stopped being true then.)

**Why not.** No Firefox/WebKit run has been scripted.

**Start.** `verify_web.py` drives Playwright; `p.firefox` and `p.webkit` are the
same API.

## 1.7 A full-suite run on the pinned toolchain

**What.** `cargo test --workspace --no-fail-fast` passed in one pass on
2026-09-15 (4,225 tests, 0 failures) — on rustc **1.95.0**, because the
machine could not download the pinned 1.98.1. Lints differ between the two
(1.95's clippy flags pre-existing `collapsible_match`/`ptr_arg` sites in
`test-text-fidelity` and `viewwstudio`'s examples under a workspace-wide
`-D warnings`; the certification's core crates are clean).

**Start.** `ci/certify/certify.sh` on a machine that can install 1.98.1.

---

# 2. GPU renderer coverage

`vieww_gpu::ScenePlan::is_complete` is the gate. As of 2026-09-15 every
`Command` plans on the GPU — images, shadows (outer/inset/rotated), gradients
(per fragment), shaped clips, layers with group opacity, all 28 blend modes,
layer and backdrop filters — and all 23 gallery fixtures plan complete. The old
items 2.1–2.5 moved to `TRACKER.md` with their evidence; the full picture is
[`docs/GPU-RENDERER-STATUS.md`](./docs/GPU-RENDERER-STATUS.md). What remains:

## 2.0 Geometry edges are not antialiased on the GPU

**What.** Tessellated fills and strokes are rasterized without edge coverage:
no MSAA and no analytic AA. Masks (clips, shadows) and glyphs are exact, so
the difference is confined to geometry edges — but on a rounded card that is
not clipped by a mask it is visible, and it is why `fixtures --census`'s strict
interior rule still counts edge pixels.

**Why not.** Not attempted in the compositor work; it is its own design
decision (4× MSAA with a resolve per offscreen target vs. analytic SDF coverage
for rects/rounded rects plus MSAA for general paths).

**Blocks.** A claim of visual parity with the CPU renderer for arbitrary
geometry, and the premium look of the GPU path.

**Start.** `crates/vieww-gpu/src/scene.rs`'s `emit_painted` (where geometry
becomes vertices) and `crates/vieww-shaders/shaders/scene.wgsl`'s SOLID
material. Measure with `fixtures --census`: the strict counts should fall to
zero.

## 2.6 A GPU frame has never reached a window

**What.** `SceneRenderer` renders headlessly and reads the pixels back.
`vulkan::swapchain` presents pixels, but the pixels it presents are the CPU
rasterizer's. Nothing connects the two.

**Why not.** Until `is_complete` could say yes to a real screen, a GPU present
path would have had nothing to present. That is now untrue — every fixture
plans complete and executes headlessly — so this is the next integration step,
after 2.0: a frame target presented as-is on screen would show the jagged
geometry edges 2.0 describes.

**Blocks.** Any end-to-end GPU claim at all. The correctness suite reads back
into a buffer; a window is a different code path with its own failure modes
(resize, vsync, swapchain recreation, presentation mode).

**Start.** `crates/vieww-hal/src/vulkan/swapchain.rs`, and
`vieww-platform-winit`'s `NativeRenderer::for_window_with`, which already takes
the rasterizer as a parameter — the seam for choosing a GPU path is already
there.

## 2.7 The atlas cannot evict

**What.** `Atlas::insert` returns `None` when the texture is full at 4096², and
the planner reports the glyph as a gap so the whole frame goes to the CPU. It
never evicts.

**Why not.** Deliberate, and documented in `crates/vieww-gpu/src/atlas.rs`:
evicting mid-frame would invalidate UVs already written into this frame's
vertex buffer, and a frame that drew nine of a word's ten glyphs is worse than
a frame drawn on the CPU.

**Blocks.** Nothing yet. 4096² of *distinct* glyph coverage is a very large
working set, and no measurement has ever hit it.

**Start.** Only when a real application hits it. The fix is generational —
evict between frames, never within one — and it should be written with a test
that fills the atlas deliberately rather than in response to a guess.

---

# 3. Text and typography

The shaping stack is `cosmic-text`/`harfrust` and is genuinely good: real
shaping, bidi, font fallback, grapheme-correct editing, IME preedit. What is
missing is in the *rasterizer*, and it is visible.

## 3.1 Colour fonts — emoji do not render

**What.** `CBDT` (bitmap) and `COLR` (layered vector) glyphs are out of scope
in `crates/vieww-paint/src/native/glyph.rs`. A glyph with no monochrome outline
comes back as `GlyphAlpha::NoOutline` and draws nothing.

**Why not.** The rasterizer is a coverage-mask rasterizer end to end: one alpha
value per pixel, multiplied by one colour. A colour glyph is not that shape,
and neither is the glyph atlas that was just built on the same assumption.

**Blocks.** Emoji anywhere — in a label, in a text field, in a chat. For a
framework whose pitch is polish, this is the most visible single gap in it: a
user typing 🎉 sees nothing at all.

**Start.** `skrifa` is already a dependency (via `cosmic-text`) and reads both
tables. The design decision to make first, and to write down, is what the
rasterizer's glyph result becomes when it is no longer "an alpha bitmap" —
because `GlyphAlpha`, `InkedGlyph`, the raster cache, the compositor's
`composite_coverage` path and `vieww_gpu::Atlas` all assume single-channel
coverage today. Doing this properly means an `RGBA` glyph variant threaded
through all five, not a special case bolted to one.

## 3.2 Variable fonts

**What.** No `fvar` axis support. A variable font renders at its default
instance only.

**Why not.** Same module, same scope note.

**Blocks.** Anything shipping a modern variable typeface and expecting to pick
a weight from it. Less visible than 3.1 because the fallback is a real,
correct-looking glyph.

**Start.** `skrifa` handles instancing; the axis coordinates need to reach
`GlyphCache::outline`, and — this is the part to be careful about — into
`GlyphKey`, or two instances of one glyph will share a cache entry and an atlas
patch. That is a silently-wrong-pixels bug, not a missing-feature bug.

## 3.3 No subpixel antialiasing, no hinting

**What.** Greyscale AA only, unhinted.

**Why not.** Both are deliberate-by-omission rather than decided. Subpixel AA
is also a genuine trade — it is LCD-geometry-dependent, it breaks under
rotation and translucency, and Apple removed it.

**Blocks.** Nothing functional. It is a sharpness difference on low-DPI
displays, and it is where "premium" is judged by people comparing side by side.

**Start.** Write down the decision before writing code — a note in
`native/glyph.rs` saying "greyscale only, and here is why" would close this as
a *question* even if it never closes as a feature.

---

# 4. CPU rasterizer performance

## 4.1 Two fixtures are over a 60 Hz full-repaint budget

**What.** `00-layers-nested` (~25 ms) and `23-editor-glass` (~22 ms) against a
16.7 ms budget, at 1366×679.

**Why not closed.** It is measured, understood and bounded rather than
neglected: `TRACKER.md`'s rendering-performance section documents a 1.28×
whole-gallery improvement, the four optimisations that were tried and
*removed* for making things worse, and the fact that a **retained** repaint of
the same screen is 9.9 ms and byte-identical — which is what a live window
actually does. The full-repaint number is the right alarm for a first frame, a
resize and a theme change.

**Blocks.** Resize smoothness on the heaviest screens.

**Start.** Explicit SIMD in the compositing loops is the one remaining large
lever; the frame is roughly 200 M instructions for ~4 M blended pixels and the
cost is spread across compositing (22%), output conversion (16%), layer resolve
(8%) and blur (7.5%) rather than concentrated. Read the "four optimisations
that were tried, measured, and removed" section first — the machine's
run-to-run spread is wider than most of these effects, and `callgrind`
instruction counts are the only stable signal.

---

# 5. Platform and product gaps

## 5.1 Outgoing drag-and-drop is a stub

**What.** `vieww-interaction`'s `NullDragStarter`. Dragging a file *out* of a
vieww window to another application does nothing.

**Why not.** No portable API in `winit` 0.30 — tracked upstream as winit issue
#1550. The stub is honest and documented rather than faking a success path.

**Blocks.** File-manager-shaped applications.

**Start.** Per-platform, below winit: `NSDraggingSession` on macOS,
`DoDragDrop` on Windows, XDND / the `wlr-data-control` protocol on Linux.

## 5.2 The display refresh rate is not read from the platform

**What.** `winit`'s `refresh_rate_millihertz` returns `None` unconditionally on
this backend — the framework carries a `FIXME` quoting that. `App::refresh_rate`
lets an application state it instead.

**Why not.** Upstream.

**Blocks.** Correct frame budgeting on a 120 Hz display without the application
saying so. Mis-states the jank count and nothing else.

**Start.** `crates/vieww-platform-winit/src/app.rs:1099` and `insets.rs:296`.

## 5.3 viewwstudio's panic boundary cannot work on Windows

**What.** The studio `dlopen`s a preview `cdylib` and catches panics from it.
That requires host and guest to share one `libstd`, which `.cargo/config.toml`
arranges with `-C prefer-dynamic` — **and the MSVC toolchain ships no dynamic
std**, so the flag is deliberately not set there.

**Why not.** Not fixable with a flag. `.cargo/config.toml` says so, at length,
and names the real answer: an out-of-process preview.

**Blocks.** A Windows studio. A guest panic there takes the window and the
unsaved buffer with it.

**Start.** Design the out-of-process preview. This is the largest single item
in this file that is *not* GPU work, and it is worth doing for every platform,
not only Windows — an in-process guest is a hazard everywhere and merely a
survivable one on Linux and macOS.

## 5.4 `ListView` cannot be scrolled to an index programmatically

**What.** Noted in `crates/vieww-widget/src/controls/list_view.rs:225`: the
framework has no way to answer a scroll-to-index request back through a signal.

**Why not.** Needs a controller shape the widget layer does not have yet.

**Blocks.** "Jump to result", "restore scroll position", and anything
keyboard-driven over a long list.

**Start.** The mechanism probably wants to look like the existing
`Inherited<T>` publish-and-read machinery rather than a new one.

---

# 6. Process and infrastructure

## 6.1 The coverage gate has never produced a number

**What.** `ci/check/coverage.sh` exists, enforces a floor with
`--fail-under-lines`, and is **not** called from `ci/check/checks.sh`.

**Why not.** It needs `cargo-llvm-cov`, which is a crates.io binary and not a
rustup component, so nothing in `rust-toolchain.toml` can install it. The
script's own preflight explains that its absence used to present as "your tests
regressed", because `cargo llvm-cov` exits non-zero for both.

**Blocks.** Any statement about how much of this workspace is actually covered.
Given how much of it is platform code that cannot run here, the number is
probably interesting.

**Start.** `cargo install cargo-llvm-cov && ./ci/check/coverage.sh`. Then decide
whether it joins `ci/check/checks.sh` — and if it does, it needs the same
"missing tool is not a failure" shape the web-sys audit stage got.

## 6.2 There is no CI workflow, and there never was one here

**What.** No `.github/workflows`, or any other CI definition.

**Why not.** `TRACKER.md` records that the release archive this workspace came
from contained **no dotfiles at all** — `.cargo/config.toml` was reconstructed
because its absence had a diagnosable symptom, and anything else that lived in a
dotfile is simply gone with no record of what it contained.

**Blocks.** Everything in `ci/` running automatically. Every gate in this
repository is currently a gate somebody has to remember to run.

**Start.** `ci/check/checks.sh` is already "everything CI runs, in order" and is
designed to be the single entry point. A workflow that runs it on Linux, plus
`ci/check/platform-check.sh` on macOS and Windows runners, would close most of §1
without anyone owning the hardware.

## 6.3 `vieww-platform-web` is `publish = false`

**What.** Deliberate, and correct: publishing code no compiler has seen, under
a version number that looks like every other crate's, would let an application
depend on it on false pretences.

**Blocks.** Nothing. Listed so nobody "fixes" it before 1.6.

**Start.** When `ci/check/wasm-check.sh` passes: remove the line, delete the banner
in that crate's `src/lib.rs`, and move its row in `TRACKER.md`.

---

# 7. Known-unknowns

Two things are worth working on and are not yet understood well enough to have
a "start here". They are listed so they are not mistaken for oversights.

## 7.1 Is the GPU path actually worth it on this workload?

Nobody knows. The CPU rasterizer is unusually good — a full studio frame in
about 49 ms of wall clock after a 1.73× improvement, and a *retained* repaint
of the same screen in under 10 ms — and the GPU path is one draw call for a
whole frame but has never run on hardware. It is entirely possible that for
dense, static, text-heavy interfaces the honest answer is "the CPU path is the
product and the GPU path is for animation-heavy screens". §1.1 is what would
tell us, and until it happens this repository should be careful not to imply an
answer it does not have.

## 7.2 What is the story for embedded native views?

Video, a map, a web view, a camera preview — anything the OS draws that vieww
must position and clip but not rasterize. `vieww_foundation::capability` has
`platform_view` types, and no application has ever used one. Whether the
existing shape is the right one is unknown, and it is the kind of thing whose
design is decided by the second consumer rather than the first.

---

# Deliberate non-goals

Listed so they are not repeatedly re-raised as gaps. Each has its reasoning
written where the decision lives.

- **`serde`.** Not used anywhere in this workspace, by choice — see
  `crates/vieww-devtools/src/json_export.rs`'s module doc.
- **`wgpu`.** Removed with vello and not coming back. The render graph, the
  damage tracking, the CPU/GPU placement decision and the frame scheduling are
  what vieww exists to own; designing them around another abstraction's model
  is the outcome that choice avoids. See `README.md`.
- **Cross-backend parity as a suite.** There is one renderer compiled two ways,
  not two renderers. Where two genuinely independent implementations *do* meet
  — tessellated geometry versus analytic scanline coverage — `vulkan_scene.rs`
  compares them. Where they do not, `vulkan_text.rs` demands exactness instead.
  See `TRACKER.md`.
- **A separate container-query widget.** `LayoutBuilder` already is one.
- **Renaming crates to match an earlier architecture sketch**
  (`vieww-window`, `vieww-native`). The functionality exists under other names;
  a risky rename to match a document is not a fix.
