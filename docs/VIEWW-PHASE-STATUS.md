# Vieww Phases 1–4 Implementation Status

This file tracks the evaluation's four phases against what this checkout actually contains: the renderer completeness gaps (CPU and GPU, reported separately), the typography gaps, the eight canonical certification suites, and the contracts layer that makes "superior" measurable.

**At a glance (2026-09-15):**

```text
CPU/native renderer, Phase 1   complete
GPU planner + Vulkan executor  every Command plans and executes headlessly; parity-tested per pixel
                               remaining: geometry edge AA, live-window integration, real-HW timing,
                               Metal/D3D12 executors  (docs/GPU-RENDERER-STATUS.md)
Typography, Phase 2            substantially complete (Linux/headless; other platforms unexercised)
Certification, Phase 3         one command runs every suite; skips are explicit, never passes
Vieww standard, Phase 4        all twelve clauses measured (allocations by a counting allocator,
                               GPU completeness by the real planner)
Release hygiene                patch leftovers and stale evidence removed; ci/check/release-clean-check.sh gates it
```

## Phase 1 — renderer completeness

The CPU/native rasterizer (`vieww-paint::native::NativeRenderer`) — the shipped renderer — now implements the full Phase-1 list:

- layers/offscreen compositing — real isolated buffers per `PushLayer`, pooled and reused; **a layer's own clip is honored** (rect half narrows the buffer at push, shape half multiplies coverage at pop — previously the clip was discarded outright)
- gradients — linear, radial and sweep, geometry folded per shape, with optional **ordered 4×4 Bayer dithering** (`Gradient::with_dither()`), off by default so existing goldens stay byte-valid
- shadows — outer, inset, rotated; cached patches; pixel-tested
- images — bilinear sampling, inverse-transform hoisting, and **mipmapping for minification** (`MipCache`: a premultiplied-safe box-filter pyramid per image, trilinear sampling at ≥2× minification, byte-identical below it), plus edge-clamped bilinear interiors
- shaped clips — rounded-rect, path, nested; **nested shaped clips intersect** (coverage product) where they previously unioned and leaked
- backdrop/filter chains — real destination sampling at push, blur + colour-matrix chains, order-preserving
- all 28 blend modes — handled natively, layer-scoped, zero `unsupported_blends`; the full matrix is rendered by `test-native-surface`
- real window presentation — CPU rasterize → `vieww-hal` Vulkan `VK_KHR_swapchain` (unchanged, in `vieww-platform-winit`)

`SceneReport` now counts shaped clips on **every** command type (previously glyph runs only), colour glyphs, and the renderer exposes `decoded_color_glyphs()` / `generated_mip_chains()` for the certification suites.

### The GPU path, separately

`vieww-gpu`'s `Planner` + `vieww-hal`'s Vulkan `SceneRenderer` now cover the same list: offscreen layers and group opacity, all 28 blend modes (a line-for-line shader port of `native/color.rs`), layer and backdrop filters (the same three-box blur and colour matrix), shaped clips (the CPU clip rasterizer's coverage as a GPU mask), outer/inset/rotated/scaled shadows (CPU silhouettes, GPU blur), per-fragment linear/radial/sweep gradients with dithering, rotated and mip-blended images, and colour glyphs. `ScenePlan::unsupported` is reached only by resource limits. Every feature is compared with `NativeRenderer` over every pixel in `vieww-hal`'s `vulkan_compositor` suite (max channel difference 1 on lavapipe), and all 23 gallery fixtures plan complete in `fixtures --census`.

It is **not** yet the renderer a window uses, its geometry edges are not antialiased, it has not been timed on real GPU hardware, and Metal/D3D12 do not execute plans. Details, the bugs this work found in the previous GPU path (including a scene shader that did not parse), and the remaining list: [`GPU-RENDERER-STATUS.md`](./GPU-RENDERER-STATUS.md).

## Phase 2 — premium typography

- font fallback — deterministic embedded-first chain, now **including an embedded CJK face** (`NotoSansCJK-subset.ttf`, 26 KB): mixed Latin/CJK shapes through two real faces headlessly; uncovered scripts still report `.notdef` + tofu boxes through `missing_characters`
- **variable fonts** — `FontData` carries `FontVariation`s; the store attaches the shaper's `wght` location to variable faces so the rasterizer draws the interpolated instance (cosmic-text 0.19 already shaped at that location; only the outline extraction was default-instance before). Any weight in the axis range works, verified visually at 300/400/500/700 on a 200–900 axis
- **colour fonts** — COLRv0 layered glyphs (palette-resolved, painted through the ordinary coverage path per layer) and CBDT/sbix bitmap strikes (PNG decode, cached per glyph, composited through the image sampler) — Noto Color Emoji renders in colour, headlessly
- glyph atlas lifecycle — outline LRU per font, raster LRU with exact-phase keys, GPU atlas with versioned repack (GPU-side eviction still deliberately absent, documented)
- subpixel positioning — exact f32 phase (unchanged; `test-text-fidelity/06-subpixel` renders four phases of one glyph as its visual proof)
- AA modes — two, selectable per renderer: the shared 4×-supersample scanline AA (shapes and glyphs, one policy — the default, byte-identical to every existing golden) and **RGB LCD subpixel AA** (`NativeRenderer::with_aa_mode(AaMode::Lcd)`: glyph coverage rasterised at 3× horizontal resolution, one alpha per RGB sub-column, per-channel blending on the opaque root). LCD falls back to grayscale — as a separate cache entry, not a conversion — inside layers, over translucent backgrounds, under the linear-light pipeline, and for colour glyphs; the three channels of an LCD rasterisation average to exactly the gray coverage, so the two modes cannot disagree about what a pixel holds. Verified headlessly in `test-text-fidelity/08`: 14,963 channel-split edge pixels between the two modes, systematic red-right / blue-left fringe polarity, and zero re-rasterisation on re-render
- selection/caret — geometry from the shaping pass, visually verified in `test-text-fidelity/05-selection-caret`
- large text stress — `test-text-fidelity/07-large-text`: 2 139 glyphs in one frame, second pass all cache hits

## Phase 3 — the canonical certification suites

All eight now exist as runnable examples, each writing evidence plus a machine-readable result and exiting non-zero on failure (this line said "seven" while the table listed eight):

| suite | what it certifies |
|---|---|
| `examples/test-premium-ui` | one composed premium screen, 56-frame deterministic GIF (fixed this session to pass on this checkout — its cards were authored against another tree) |
| `examples/test-layout-stress` | 5 000-sibling breadth, 24-level depth, intrinsic sizing; one changed cell rebuilds 1 of 5 004 commands |
| `examples/test-animation-stress` | 100 simultaneous animated properties, p95 frame time inside the 16.6 ms budget, every frame moving |
| `examples/test-text-fidelity` | variable weights, COLRv0 layers, CBDT emoji, CJK fallback, selection/caret, subpixel phases, large-text stress, LCD subpixel AA (gray-vs-LCD byte comparison + 8× zoom pair) |
| `examples/test-image-effects` | mip-minification (averaged, not moiré), zoom-out across levels, rotation, filters, image-over-image blends |
| `examples/test-scroll-stress` | 5 000-row virtualised feed; 24–27 runs in the tree wherever the window lands; hard jumps rebuild one window |
| `examples/test-native-surface` | paint-level completeness: all 28 blends isolated, all gradient geometries ± dither, outer/inset/rotated shadows, nested shaped clips, layer-under-clip, backdrop blur, resize path, pool reuse |
| `examples/test-web` | **the web backend, end to end**: the same deterministic scene (gradients, shadow, blurred circle-clipped layer, rotated group, Latin+CJK text, live counter) rasterised natively and mounted on a canvas through `vieww-platform-web` in headless Chromium — **canvas readback equal to the native buffer, byte for byte, at `devicePixelRatio` 1, in two pointer states**; taps re-render exactly the native "tapped" state; wheel events don't kill the loop; zero console errors |

`examples/vieww-standard` (Phase 4's runner, below) completes the picture.

### Orchestration: one command runs all of it

`ci/certify/certify.sh` previously ran fmt/check/clippy, the workspace tests, the Vulkan suites, `test-gpu-work`, `test-premium-ui` and the fixtures — and **not** the layout, animation, text, image, scroll or native-surface suites, `vieww-standard`, or the web backend. It now runs every one of them, plus the GPU fixture census, a release-cleanliness gate, and the web build + browser verification (`examples/test-web/verify_web.py`, which previously lived outside the repository). A stage the machine cannot run is `SKIPPED(reason)`; `COMPLETE=` in the summary says whether anything was skipped, and `VIEWW_CERT_STRICT=1` fails on any skip. Evidence is written under `target/cert-linux`, the directory is emptied first, and `meta/source.txt` carries a digest of the source tree it certified. `ci/certify/certify-windows.ps1` follows the same rules (and no longer names a parameter `$Args`, PowerShell's automatic variable); it has not been executed in this session — there was no Windows machine.

### The web target

`ci/check/wasm-check.sh` — compile + `clippy -D warnings` for `wasm32-unknown-unknown` — **passes** (first time: 2026-09-14; its first honest run found seven target-dependent lints in `vieww-paint`, all fixed). Beyond compiling, the backend has been *run*: `examples/test-web/build-web.sh` builds the scene as wasm, and the certification runner serves it, drives headless Chromium, reads the canvas back, and byte-compares it with the native baseline written by `cargo run -p test-web --example baseline`. `apps/viewwsite` — the DOM-backend product page with a live canvas demo island inside it — loads, scrolls, reveals and taps in the same session, with zero page errors. Still open, honestly: other browsers/OSes (the byte-parity claim is Chromium-on-Linux), and the GPU `ScenePlan` gates below.

## Phase 4 — the Vieww standard

`vieww-render-planner::QualityContract` now carries **twelve clauses**, each with a measured counterpart in `QualityObservation` and a named `QualityViolation`: frame budget, **frame pacing (p95)**, **startup**, steady-state allocations, clean-scene rebuilds, GPU completeness, pixel parity, input latency, **animation latency**, **memory-trim pixel stability**, **accessibility**, **typography determinism**.

`examples/vieww-standard` measures all of them from a real workload — cold first frame, animated loop, a signal flip, a synthesised pointer press through the real dispatch, a `Critical` trim, two shaping passes, the semantics audit — and prints the verdict.

**Two clauses used to be literals.** The runner constructed `steady_allocations: 0` and `unsupported_gpu_commands: 0`, so the contract enforced an assumption. Now:

- `steady_allocations` is counted by a `#[global_allocator]` over 60 steady frames through the retained present path, and the runner refuses to report if the allocator is not installed. Its first honest reading was **7 allocations per steady frame** (420/60). Fixed at the source: scratch buffers in `ElementTree::tick_states`/`poll_states`, the boundary list and owner map in `RenderTree::paint_layers_with_ratio`, `LayerTree::end_frame`'s id list — and one that was a real performance bug: `RenderOwner::sync` re-asserted the same root every frame and `RenderTree::set_root` cleared the incremental boundary cache on every call, so that cache never reused a subtree in a real frame loop. `NativeRenderer::render_in_place` / `render_retained_in_place` / `last_frame` also let the winit presenter stop copying the whole frame into a fresh buffer per present.
- `unsupported_gpu_commands` is summed from `vieww_gpu::Planner` plans of every animated frame and the settled frame.

On the 2026-09-15 certification run (2-core container, lavapipe, rustc 1.95.0): startup 27.5 ms, worst frame 13.1 ms, p95 11.4 ms, animation and input latency 1 interval, pixel drift 0, trim changes 0, typography drift 0, a11y findings 0, clean-scene rebuilds 0, **steady allocations 0 over 60 frames (counted)**, **unsupported GPU commands 0 over 91 frames (planned)** — PASS.

## Certification result (2026-09-15, Linux)

`FAILED=1`, `COMPLETE=false`. Passed: release-clean, fmt, check, core clippy, workspace tests (4,225/0), GPU planner, shaders, quality, paint-native, Vulkan (37/0), GPU workload, census (23/23 complete, 0 beyond band), premium UI, fixtures, layout/text/image/scroll/native-surface suites, `vieww-standard`, web baseline, and every measured gate. Failed: `test-animation-stress` (p95 24.3 ms against 16.6 ms; the pristine source measures the same on this machine). Skipped: wasm-check and web build (target not installable here), web verify (ran separately against the archive's prebuilt wasm: byte-equal in both states), desktop suite (no display).
