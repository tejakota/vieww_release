# GPU renderer status

What the GPU path (`vieww-gpu` planner + `vieww-hal` Vulkan executor) does, how
it is verified, and what is still missing — kept separate from
`VIEWW-PHASE-STATUS.md` because the CPU renderer and the GPU renderer are at
different stages and one "Phase 1 complete" line cannot describe both.

## Coverage: every `Command` plans, and the gaps are resource limits

`vieww_gpu::Planner::plan` turns a `vieww_paint::Scene` into a `ScenePlan`:
one batched vertex buffer plus an ordered **step program** a backend executes.

| command / feature | GPU implementation | definition taken from |
|---|---|---|
| fills, strokes, transforms | CPU tessellation (lyon), batched, scissored runs | — |
| rectangular clips | dynamic scissor per run | — |
| **shaped clips** (rounded, path, nested) | R8 mask atlas, `textureLoad` at the fragment's pixel; on layers, a mask multiply at pop | `GpuSeam::clip_mask` = the CPU clip rasterizer's coverage |
| text (monochrome) | R8 glyph atlas, texel-exact quads | `GlyphCoverage` = the CPU glyph rasterizer |
| **colour glyphs** (COLRv0, CBDT/sbix) | layers as coverage glyphs in palette colours; bitmaps through the image path | `GpuSeam::color_glyph` |
| gradients (linear, radial, sweep, dither) | **per-fragment** geometry + RGBA32F ramp atlas + Bayer dither in the shader | `gradient_ramp`, `BAYER4` |
| images | RGBA8 atlas, shader-side bilinear on premultiplied texels clamped to the patch; **rotated quads**; **mip blending** past 2× minification | `GpuSeam::image_mips`, `minification_ratio` |
| **layers / group opacity** | offscreen RGBA16F target per nesting level; opaque `Normal` unfiltered layers pass through (exact: source-over is associative) | `NativeRenderer`'s `PushLayer`/`PopLayer` |
| **28 blend modes** | destination snapshot + `post.wgsl` BLEND, a line-for-line port of `native/color.rs` | `native/color.rs` |
| **layer filters** (blur, colour matrix) | three horizontal+vertical box passes; matrix pass | `box_radius_for_sigma`, `effects::color_matrix` |
| **backdrop filters** | parent copy at push, filtered before the layer's own content | `ImageFilter::backdrop` semantics |
| **shadows** (outer, spread, inset, rotated, scaled, clipped) | caster mask → GPU three-box blur → inset complement → tint → composite | `GpuSeam::shadow_masks` = `render_shadow`'s silhouettes |

`ScenePlan::unsupported` is now only reached when a resource limit is hit: a
glyph/mask/ramp/image atlas that cannot grow past its maximum, a layer stack
deeper than `MAX_TARGET_DEPTH` (32 targets), or an image larger than the image
atlas. `SceneRenderer::render` still refuses an incomplete plan.

### Bugs found and fixed on the way

- **The shipped `scene.wgsl` did not parse** (`let x = if …` is not WGSL). Every
  Vulkan scene test and `vieww-shaders`' own "every built-in shader validates"
  test failed on the uploaded tree; the analytic outer-shadow material the
  planner emitted had never executed.
- **Group opacity was folded into child alpha**, which is wrong wherever
  children overlap (two opaque squares at 50% group opacity are 50%, not 75%,
  where they cross). Now an offscreen composite.
- **Radial and sweep gradients were interpolated per vertex** — four vertices
  on a rectangle — and **rotated images were drawn as their bounding box**.
  Both silently wrong; both now per-fragment.
- **Glyph/image atlas growth moved existing entries** while vertices earlier in
  the same frame still addressed their old positions. Growth now keeps every
  entry at its texel.
- **Atlas versions were per-instance counters**, so a renderer shared between
  two `Planner`s skipped uploads when the counts coincided (found by the
  fixture census: gradients in the previous screen's colours, clips cut by the
  previous screen's mask). Versions are now process-unique generations.
- **Fills used lyon's default even-odd rule**; the display list is non-zero.
  Icons built from overlapping subpaths had holes (census: `10-icon-grid`).
- **Strokes were tessellated in device space**, so their width ignored the
  transform's scale (the CPU strokes in local space and transforms the
  outline), and **dash patterns were ignored** entirely. Both now follow the
  CPU stroker; dashes are split by its own `dashed_segments`.
- **lyon's miter limit is twice as permissive** as the CPU stroker's (SVG's)
  definition; sharp joins mitred on the GPU where the CPU bevels (census:
  `21-dashboard`'s sparkline peak, 5 px taller). The limit is now converted.
- **Image identity was an address that could be reused.** The image atlas and
  the mip cache (both the GPU's and the **CPU renderer's own** `MipCache`)
  keyed images by `Arc` address without keeping the allocation alive, so a new
  image allocated where a dropped one had been was served the dead image's
  texels or pyramid. Intermittent by nature: `test-gpu-work` failed parity on
  about one run in three with a minified photo drawn from a stale mip level.
  Both caches now hold a `Weak` (which pins the address and detects death), and
  the image atlas evicts between frames.
- **16-bit tessellation indices** turned a path past 65,536 vertices into an
  empty mesh — a silently missing shape. Indices are 32-bit.
- The Vulkan target used straight-alpha readback of a premultiplied buffer;
  targets are premultiplied RGBA16F and readback un-premultiplies with the CPU
  renderer's rounding.

## Verification

| check | what it proves | result (lavapipe, this run) |
|---|---|---|
| `cargo test -p vieww-gpu` | planning, step structure, atlases, ink regions | pass |
| `cargo test -p vieww-shaders` | `scene.wgsl` and `post.wgsl` validate and cross-compile to SPIR-V, MSL, HLSL | pass |
| `vieww-hal` `vulkan_compositor` (ignored suite, 17 tests) | **every pixel** against `NativeRenderer` for group opacity, all 28 blend modes, blur, colour matrix, backdrop blur, shaped clips, 7 shadow variants, gradients ± dither, 4 image cases, nested mixed layers, transparent clear, non-zero winding, dashed strokes under scale, miter-limit joins, shared renderer across planners | max channel difference **1** in every non-rotated case; rotated geometry differs only on its non-antialiased edge |
| mutation checks | the suite detects a one-off blur kernel and a wrong Hue formula (6 tests fail), even-odd winding (1,024 px), and an unconverted miter limit (9 px) | each mutation fails |
| repeat runs | `test-gpu-work` ten times in a row | identical `parity_worst_fraction` every run (it varied, and failed ~1 in 3, before the identity fix) |
| `test-gpu-work` | 24-frame animated scene using every feature; per-frame parity vs CPU; capability and resource-limit probes | `unsupported_gpu_commands` **measured** 0; worst frame 0.37% of pixels over tolerance 8 (edges) |
| `fixtures --census` | the whole gallery through GPU and CPU | **23 of 23 plan complete**; 0 flat-region mismatches beyond the geometry-edge band |

## What is still missing

1. **Geometry edge antialiasing.** Tessellated fills are drawn without edge AA
   (no MSAA, no analytic coverage), so curved and rotated *geometry* edges are
   jagged next to the CPU renderer's analytic coverage. Masks (clips, shadows)
   and glyphs are exact. This is the single visible quality gap and the reason
   the census's strict rule still lists edge pixels. Options: 4× MSAA with
   resolve per target, or analytic SDF coverage for rects/rounded rects (the
   overwhelming majority of UI geometry) plus MSAA for general paths.
2. **The live window does not use it.** `SceneRenderer` renders headlessly with
   readback; `vieww-platform-winit` still presents CPU-rasterized pixels
   through `vulkan::swapchain`. Presenting the RGBA16F frame target to the
   swapchain (a format-converting blit or final pass) is the integration step.
3. **Cost shape, unmeasured on real hardware.** Offscreen targets are
   frame-sized per nesting level; every shadow is a 7-pass sequence; clip and
   shadow masks are rasterized on the CPU (cached across frames by content).
   lavapipe timings are not a GPU performance claim. Needed next: timings on
   real hardware, then shadow-result caching and bounds-sized targets if they
   matter.
4. **The glyph atlas never evicts** (`PENDING.md` 2.7). Mask, ramp and image
   atlases compact between frames.
5. **Metal and D3D12 do not execute a `ScenePlan`.** Both shaders cross-compile
   (HLSL needs the push-constant register this change configures), but neither
   backend has an executor. The Windows certification reports D3D12 as
   `BUILD-ONLY`, never as a rendering pass.
6. **The linear-light colour pipeline** (`NativeRenderer::with_color_pipeline`)
   has no GPU equivalent; the planner has no way to be told which pipeline the
   frame wants, so GPU parity is defined against the default gamma-space
   pipeline.
