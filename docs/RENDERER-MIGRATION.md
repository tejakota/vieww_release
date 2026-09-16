# Renderer migration: vello, `vello_cpu`, `vello_hybrid` → `vieww_paint::native`

This document replaces an earlier version of itself. That version described a
side-by-side experiment: a new `vieww-renderer` crate built alongside vello,
which stayed the default. That phase is over. **Vello, `vello_cpu` and
`vello_hybrid` are gone from this workspace — not deprioritized, not kept as
a fallback, removed** — and `vieww_paint::native::NativeRenderer` is the only
renderer anywhere in the tree, headless and windowed alike. This document is
the honest accounting of that completed migration: what was deleted, what
replaced it, the real architectural differences the replacement forced, and
what still doesn't compile-check-and-test itself into "done" without being
said plainly.

## 1. What is gone

Every line that named `vello`, `vello_cpu`, `vello_hybrid`, or the types that
wrapped them:

- `vieww_paint`'s `gpu`, `hybrid`, and `cpu` modules — and the `GpuRenderer`,
  `HybridRenderer`, `CpuRenderer`, `GpuContext`, `GpuSurface`, and `GpuError`
  types they exposed — deleted outright, not banner-marked "legacy."
- The `gpu`/`hybrid`/`cpu` Cargo features on `vieww` and `vieww-paint`,
  collapsed to one feature: `native`.
- Every downstream crate, example, app, and test file that imported any of
  the above — roughly 90 files across `crates/`, `examples/`, and
  `apps/viewwstudio/` — rewritten to call `vieww_paint::native::NativeRenderer`
  directly, or deleted where what they tested no longer has a referent (see
  §3).
- `vieww-hardware`'s old `Capability::CustomRenderer`/`custom-renderer`
  probe pairing, superseded by `Capability::Gpu` probing
  `vieww_hal::vulkan::VulkanDevice::new` directly.

`grep -rln "vello_cpu\|vello_hybrid\|GpuRenderer\|HybridRenderer\|CpuRenderer\|vieww_paint::(cpu|gpu|hybrid)\|GpuContext\|GpuSurface\|GpuError"` across `crates`,
`examples`, and `apps` returns nothing but historical prose — doc comments
that say, in effect, "this used to depend on vello and no longer does,"
written at the point a future reader would otherwise wonder. `Cargo.lock`
carries no package named `vello*` at all.

## 2. What replaced it

**`vieww_paint::native::NativeRenderer`** — a from-scratch CPU rasterizer,
now the *only* rasterizer, used identically whether a scene is headed to a
PNG on disk or a live window:

- Scanline fill with nonzero winding, adaptive cubic Bézier flattening,
  stroke-to-fill (miter/round/bevel joins, butt/round/square caps, dashing).
- Linear and radial gradients, rounded-rect and path clips, drop shadows,
  bilinear image sampling straight from `vieww_foundation::Image` — no
  intermediate cache — and glyph outline extraction/rasterization.
- Isolated-layer compositing for `PushLayer`/`PopLayer`, including the
  Porter-Duff and non-separable blend modes that need a real backdrop to
  mask against (`SrcIn` etc.) — see `tests/native_parity.rs`'s
  `isolated_blend_mode_masks_against_its_own_backdrop`.
- `render_to_pixels`, `render_to_png`, and `render_damaged`, returning
  straight-alpha, tightly-packed `Pixels` and a `SceneReport` of counted
  work (shapes, images, glyph runs, glyphs, clips, shadows, layers).
- A `Trim` implementation that clears the one cache this renderer actually
  keeps — extracted glyph outlines — at `MemoryPressure::Critical` and
  `Backgrounded`. There is no image cache to trim, because there is no image
  cache at all (§4).

**`vieww-hal`'s raw Vulkan swapchain**, driven by `vieww-platform-winit`'s
`native.rs`, is the windowed path: every window opens its own
`vieww_hal::vulkan::VulkanDevice`, builds a swapchain, and presents
`NativeRenderer`-rasterized frames onto it directly — no `wgpu`, no compute
or graphics shader pipeline of its own, because there is nothing left to
shade; the CPU rasterizer produces final pixels, and Vulkan's only job is
getting them on screen.

## 3. Real architectural differences this migration surfaced

Two backends built years apart do not have the same shape everywhere, and
three genuine gaps had to be resolved with real replacement mechanisms
rather than papered over:

- **No image cache.** The old vello-backed `CpuRenderer` held a converted
  copy of every image it drew. `NativeRenderer` samples
  `vieww_foundation::Image` data directly, so there is nothing large and
  idle to release under memory pressure — `crates/vieww/tests/memory_pressure.rs`
  was rewritten around this real difference (see its own module doc) rather
  than kept passing against a fabricated cache.
- **No persistent, damage-composited render target.** The old
  `GpuRenderer::render_damaged(scene, surface, damage, base)` mutated a
  retained surface in place; `NativeRenderer::render_damaged` renders into a
  fresh target every call. The actual replacement is
  `vieww_test_harness::Persistent`, which composites each frame's damaged
  regions onto a retained `Frame` on the caller's side — `overlay_to_pixels.rs`
  and `damage_to_pixels.rs` now drive it directly, and
  `damage_to_pixels.rs` computes its own pixel-cost figure from
  `Damage::repaint_regions()` since `SceneReport` doesn't carry one.
- **No cross-window device sharing.** The old `vieww_paint::gpu::GpuContext`
  let two windows share one GPU device; the new Vulkan backend gives every
  window its own `VulkanDevice` with no shared-instance equivalent yet.
  `vieww-platform-winit/tests/wait_loop.rs`'s `two_windows_share_one_gpu_device`
  scenario tested a mechanism that no longer exists and was removed, with a
  comment pointing at this gap for whoever adds device sharing later — it is
  a real, open gap, not a silently dropped test.

## 4. Test results

Every suite in the workspace passes, run against the actual compiler with no
skips other than the ones that genuinely need a display this sandbox
provides through Xvfb (and were run that way — see §5):

| Suite | Result |
|---|---|
| `vieww-foundation`, `vieww-hal`, `vieww-hardware`, `vieww-paint` (unit + doc) | ok |
| `vieww-paint/tests/native_parity.rs` (7) | ok |
| `vieww-test-harness`, `vieww-devtools --features snapshots` | ok |
| `vieww` (lib + all 39 integration test files, `--features native`) | ok |
| `vieww-render` | ok |
| `vieww-platform-winit` (116 unit tests + `window_to_gesture.rs`) | ok |
| `vieww-platform-winit/tests/wait_loop.rs` (4 scenarios, real display via Xvfb) | ok |
| every `examples/*` crate, `--all-targets` | ok (`cargo check`; `editor`'s own 4 tests: ok) |
| `apps/viewwstudio` (lib: 604 tests; 25 integration test files, ~450 tests) | ok |
| `cargo check --workspace --all-targets` | ok, zero errors, zero vello references left |

Two test files needed real fixes, not renames, once run against a compiler
rather than reasoned about:

- `crates/vieww-paint/tests/native_parity.rs`'s pixel-sample coordinates
  were picked assuming vello's inclusive rect edges; `Rect` here is
  half-open (`contains` excludes `right`/`bottom`), so two samples landed
  just outside the shape they meant to test. Moved to points unambiguously
  inside the relevant region.
- `crates/vieww/tests/text_to_pixels.rs` asserted `SceneReport::glyphs`
  equals a paragraph's total shaped glyph count. A space shapes to a real,
  positioned glyph with no outline, and `NativeRenderer` correctly skips
  rasterizing it rather than drawing an empty mask — so the assertion now
  compares against the count of non-whitespace characters, which is what
  the renderer actually inks.

## 5. Visual verification

Pixel tests prove correctness; a picture proves it to a person. Three were
produced and are attached to this migration's own delivery, not just
described here:

1. `examples/screenshot`'s headless coverage sheet — gradient fill, clip-
   rounded avatars, a drop shadow, group opacity, a stroked outline, and
   shaped text, rendered with `NativeRenderer` and no display at all — plus
   its frame-2 diff (one row recolored, and only that row's worth of
   commands rebuilt).
2. `apps/viewwstudio`'s own `screenshot` example — the full studio shell:
   syntax-highlighted editor with a live minimap, file explorer, and a
   rendered iOS device preview — proving the widget-heavy application, not
   just a synthetic test scene, draws correctly end to end.
3. A real windowed run: `examples/features/00-rectangle` launched under
   Xvfb with Mesa's lavapipe as a software Vulkan 1.2 ICD, screenshotted
   with `import` while the window was live, and killed. The captured PNG
   shows the filled rectangle exactly where the widget tree places it — the
   live `vieww-hal` Vulkan swapchain path, not the headless one.

## 6. Two unrelated, pre-existing gaps closed along the way

Neither is a rendering concern; both were blocking "the whole workspace
compiles and every test passes" and were fixed under the same standing
instruction to fold in cheap, bounded fixes as they were found:

- **`apps/viewwstudio/docs/*.md` did not exist.** `src/docs.rs`
  `include_str!`s five reference pages that were simply missing from this
  checkout, failing the build before any renderer code ran. Written from
  scratch against `docs.rs`'s own tests (each page ≥200 bytes, starts with
  `# `, no fenced code line past 46 columns, and the Rust page names both
  "normal Rust" and "Build and Run").
- **`.cargo/config.toml` did not exist.** `apps/viewwstudio`'s top-level
  `Cargo.toml` documents at length that the studio and the `cdylib` preview
  it `dlopen`s must both build `-C prefer-dynamic` so they share one
  `libstd` — otherwise a panic inside a loaded preview is a *foreign*
  exception to the host's unwinder, and the process aborts with "Rust
  cannot catch foreign exceptions" instead of the panic boundary catching
  it. The preview side already set the flag in `compile.rs`; the host side
  was supposed to get it from `.cargo/config.toml`, which was absent. Two
  `apps/viewwstudio/tests/pipeline.rs` tests
  (`a_panicking_build_is_caught_too_rather_than_aborting_the_process` and
  `a_panicking_build_on_a_later_rebuild_is_caught_by_the_host_not_aborted`)
  reproduced the abort until the file was restored, scoped to desktop
  triples only (never Android/iOS, where a dynamic `libstd` is a link error
  against a file the NDK does not ship).

## 7. What is not done

- **Cross-window Vulkan device sharing** (§3's third gap) has no
  replacement yet. Every window opens its own device; nothing shares one.
- **The six "big pillars" an external architecture review raised against
  `docs/RENDERER-SPEC.pdf`** — a first-class render graph, a dedicated
  color-management pipeline, input-to-photon latency instrumentation,
  GPU-driven execution for repeated UI elements, resource virtualization,
  and an independent blend-correctness oracle — were explicitly out of
  scope for this migration, as agreed going in, and nothing here built an
  unverified sketch of any of them just to look further along than it was.
  They are now being implemented one at a time, each validated before the
  next, as a separate, later piece of work — see
  `docs/RENDERER-V2-NOTES.md`, which records per-pillar status as each
  lands.
