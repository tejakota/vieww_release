# Renderer v2 notes — backlog against a future spec revision

Captured 2026-08-31, from an external architecture review of
`docs/RENDERER-SPEC.pdf`. Not implemented. This is a list of what a v2
version of the spec should address before any of it is built — the review's
own conclusion (point 30) is that the plan justifies replacing vello but is
not yet a plan for "beyond everything," and that the gap is a handful of
missing pillars, not missing polish.

**One item from this review was checked against the shipped code and fixed
immediately** (not listed below, since it's done): the non-separable blend
modes (Hue/Saturation/Color/Luminosity) were flagged as possibly using
"LCH-based formulas" per the spec PDF's wording, instead of the W3C
Compositing and Blending Level 1 §3.7 normative Lum/Sat/SetLum/SetSat
equations. `crate::native::color`'s actual implementation already used the
correct W3C formulas; only its doc comment echoed the spec's wrong label.
The comment is fixed in `crates/vieww-paint/src/native/color.rs`.

Everything else below is unstarted design work, roughly in the order the
review raised it, except the first "Architecture" bullet, whose status is
recorded inline now that part of it has shipped.

## Architecture

- **Render graph as a first-class component**, not an implementation
  detail: dependency graph, pass fusion, transient resource aliasing,
  lifetime analysis, barrier generation, load/store optimization, damage-
  aware pass pruning, snapshot reuse, filter-chain fusion. The review's
  target framing: "what is the cheapest possible execution plan for this
  frame," not "how do I execute these commands."

  **Status: partially shipped**, scoped to what this CPU-scanline renderer
  can actually use — see `crates/vieww-paint/src/graph.rs`'s own module
  docs for the full account, and its `tests` submodule for what validates
  it. In one sentence per item:
  - *Dependency graph* — shipped. `PassGraph` is a real, explicit graph of
    `PushLayer`/`PopLayer` passes with parent/child edges, built from any
    `Scene` in one walk.
  - *Damage-aware pass pruning* — shipped, and strictly more precise than
    what already existed: `PassGraph::prune` tests each damage region
    individually (`Damage::intersects`), where `Scene::damage_cull`'s
    existing pruning tests only the bounding box of all regions combined.
    Validated two ways: an exact-agreement differential test against
    `damage_cull` for single-region damage (where the two approaches are
    mathematically identical), and a test demonstrating the precision gain
    for multi-region damage (a pass `damage_cull` conservatively keeps that
    `PassGraph::prune` correctly drops).
  - *Transient resource aliasing* — shipped, as its one real CPU analogue:
    `native/pool.rs`'s `TargetPool` reuses `PushLayer` offscreen buffers by
    exact size across sibling layers and across frames, instead of
    allocating fresh on every `PushLayer` and dropping on every `PopLayer`
    (this closes a gap `target.rs`'s own module doc had flagged since the
    vello migration: "one type serves ... every offscreen layer ... minus
    the pooling"). Wired directly into `NativeRenderer`; `native_parity.rs`
    still passes byte-for-byte with it in place, which is what "reuse a
    buffer" has to mean — same pixels, fewer allocations.
  - *Pass fusion, barrier generation, load/store optimization* — **not
    done, and not fakeable here honestly**: all three describe GPU
    render-pass costs (attachment binds, memory barriers, tile load/store
    traffic) that a CPU scanline rasterizer, running passes strictly one at
    a time on one thread into a plain `Vec`, never pays. `PassGraph`
    exposes `co_schedulable_siblings` — which sibling passes have no data
    dependency and *could* share a GPU pass or run concurrently — as real
    groundwork for a future GPU backend, deliberately wired to no
    scheduling mechanism today, since none would be real.
  - *Lifetime analysis, snapshot reuse, filter-chain fusion* — not started.
    Genuine further work, not reinterpreted or scoped down; left for a
    future pass at this pillar.
- **Split geometry identity from GPU residency.** Today the plan conflates
  "retained mesh, keyed by content hash" with "the GPU allocation backing
  it." A v2 cache hierarchy: `GeometryCache` → `GpuResidencyCache` →
  `GpuAllocation`, so a mesh can stay logically retained while its physical
  allocation is evicted, compacted, or moved.
- **Resource virtualization layer**: buffers, textures, pipelines,
  samplers, atlases, external textures, residency — with deferred
  destruction, fence-based reclamation, lifetime tracking, residency
  priorities, compaction, aliasing, budget enforcement, upload scheduling.

  **Status: partially shipped**, scoped to what a CPU rasterizer with no
  GPU allocations to virtualize can actually use — see
  `crates/vieww-paint/src/native/residency.rs`'s own module docs.
  - *Identity split from residency, budget enforcement, LRU eviction* —
    shipped as `ResidencyCache<K, V>`: a generic, budget-bound, LRU-evicting
    backing store, `get_or_insert_with(key, derive, cost_of)` its one entry
    point — a cache hit never runs `derive`, a miss (first touch **or**
    after eviction) does, so a caller cannot tell the two apart except by
    speed. Applied concretely in `native/glyph.rs`: each font's per-glyph
    outline cache was an unbounded `HashMap` before this, growing for the
    life of the `NativeRenderer` no matter how many distinct glyphs a
    long-lived process ever touched; it is now one `ResidencyCache` per
    font, budgeted (`GLYPH_OUTLINE_BUDGET_BYTES`, overridable per-renderer
    via `NativeRenderer::with_glyph_outline_budget_bytes`), costed by each
    outline's real `Vec<PathVerb>` size. Validated at two levels: the cache
    alone (`native/residency.rs`'s own unit tests — budget enforcement, LRU
    order, a single oversized entry kept anyway) and end to end
    (`tests/glyph_residency.rs` — a real `NativeRenderer` drawing real
    shaped text under a tiny budget, checking eviction actually happens and
    that a glyph redrawn after its outline was evicted-and-re-derived inks
    the exact same pixels as its first draw).
  - *A new path/coverage-mask cache* — **not done**. Caching a rasterized
    `CoverageMask` (rather than the outline that produces it) needs a key
    that includes the transform a shape was drawn under, and this
    renderer's transforms are continuous — real UI content rarely repeats
    one bit-exact `f32` transform twice, so a naive exact-key cache would
    almost never hit, and a quantized/snapped key would change *which
    pixels a shape lands on*, which is a real rendering-correctness
    decision (how much sub-pixel positioning error a UI accepts for a
    repeated-shape speedup) that this delivery is not the place to make
    silently. Left open rather than shipped with a quietly chosen
    tolerance; the batching case this would mostly serve — many instances
    of *literally the same* geometry — is pillar D's job instead, and
    pillar D (below) took the safer, already-exact-key route: reusing
    rasterized coverage only when the geometry is content-identical, never
    approximated.
  - *Deferred destruction, fence-based reclamation, compaction, upload
    scheduling* — **not applicable, not faked**: all four describe GPU
    allocation lifecycle (a fence signaling when the GPU has actually
    finished with a buffer, compacting fragmented device memory, scheduling
    a host-to-device upload) that has no referent on a CPU rasterizer with
    no GPU allocations in this renderer's own pipeline. `native/pool.rs`'s
    `TargetPool` (pillar A) is this renderer's one real "resource
    lifecycle" mechanism, and it needs none of these: a CPU buffer's
    "destruction" is a synchronous `Vec` drop, so there is nothing to defer
    or fence.
- **Capability-driven rendering**, richer than a single "GPU has feature X"
  flag: max texture size, MSAA levels, framebuffer fetch, storage buffers,
  subgroup ops, timeline sync, external memory, HDR formats, sampler
  limits, storage image support — so the renderer can ask "what's the
  cheapest implementation of X on this device," not just "can I do X."
- Keep the HAL brutally small (`Device`, `Queue`, `CommandEncoder`,
  `Buffer`, `Texture`, `Sampler`, `Pipeline`, `Fence`, `Surface` — not much
  past that) — the risk flagged is accidentally rebuilding half of `wgpu`.

## Color and text

- **A dedicated color pipeline chapter**, not folded into "sRGB vs.
  non-sRGB surfaces": linear light → working color space → premultiplied
  compositing → effect processing → display transform → surface encoding,
  with a path to Display P3, wide-gamut textures, 10-bit surfaces, HDR,
  scRGB-style extended range, image color profiles, and color-correct
  screenshots/exports.

  **Status: partially shipped** — see `crates/vieww-paint/src/native/linear.rs`'s
  own module docs for the full account.
  - *Linear light → premultiplied compositing → display transform* —
    shipped, as an opt-in `ColorPipeline` on `NativeRenderer`
    (`ColorPipeline::GammaSpace`, the default and this renderer's original,
    unchanged behavior; `ColorPipeline::LinearLight`, selected via
    `NativeRenderer::with_color_pipeline`). `LinearLight` sRGB-decodes both
    blend operands (the real IEC 61966-2-1 piecewise transfer function, not
    a `x^2.2` approximation — the toe segment near black is where a
    shadow's or a translucent overlay's detail actually lives), runs the
    exact same `color::blend` this renderer already had, and sRGB-encodes
    the result back — the blend-mode math itself is not duplicated, only
    the color space it runs in changes. Validated at both levels the task
    asked for: `native/linear.rs`'s own unit tests (`GammaSpace` is a
    byte-identical passthrough — every existing pixel test keeps passing
    unchanged because of this; the transfer function round-trips; a known
    closed-form case — 50%-alpha white over black — comes out at the
    textbook encoded-0.5 gray in gamma space and a measurably lighter,
    colorimetrically correct linear-computed gray, matching the closed-form
    answer to within one 8-bit step) and `tests/linear_light_pipeline.rs`
    (the same comparison end to end through a real `Scene` and
    `NativeRenderer::render_to_pixels`, not just the one blend function).
  - *Working color space, effect processing* — partially shipped as a
    consequence of where the pipeline was wired in: compositing
    (`PushLayer`/`PopLayer`, coverage fills, image sampling) all go through
    `LinearLight` when selected, but `native/effects.rs`'s blur and color
    matrix (applied to a layer's buffer *between* `PushLayer` and the
    composite that would decode it) still operate on whatever space the
    buffer is already in without their own explicit space handling — not
    wrong for `GammaSpace` (nothing changed there) but not yet audited for
    `LinearLight`.
  - *Display P3, wide-gamut textures, 10-bit surfaces, HDR, scRGB extended
    range, image color profiles, color-correct screenshots/exports* — **not
    done, and not fakeable here**: this renderer's pixel format is 8-bit
    sRGB in and out end to end (`Color`, `Pixels`), there is no wider-gamut
    color type anywhere in `vieww-foundation` to carry such a value, and
    this build's own test environment (headless PNGs, a software Vulkan
    swapchain under Xvfb) has no display that could show a wide-gamut
    result even if one were computed. Left open rather than built as an
    unverified sketch nothing here could check.
- Redefine the text goal as "best text under every rendering condition"
  rather than "best desktop LCD text" — LCD subpixel AA is
  orientation/compositor-dependent and the spec already disables it in
  several cases; fractional scaling, variable fonts, optical sizing,
  complex scripts and HDR/wide-gamut text correctness are the broader bar.

## Performance and latency

- **Input-to-photon latency as a first-class metric** (pointer-down → first
  visible pixel, pointer-move → visible response, keyboard → glyph
  appearance, scroll → visual movement, animation trigger → first frame),
  measured at median/p95/p99 — not just frames per second. The two-to-
  three-frame-flight model trades this away without measuring it.

  **Status: partially shipped.** The measurement mechanism is real, tested,
  and independent of any specific input source: `crates/vieww-platform-winit/
  src/latency.rs`'s `InputLatencyLog` records an `Instant` per input arrival,
  and on `frame_presented(presented_at)` measures every still-pending input
  against that one presentation time (correctly handling several inputs
  coalesced into a single frame, and correctly leaving an input unmeasured
  until the frame that actually shows its effect presents). It keeps a
  rolling window of the most recent 240 samples and reports p50/p95/p99 and
  worst-case via `stats.rs`'s existing `percentile()` helper (widened from
  private to `pub(crate)` so both modules share one implementation rather
  than two copies that could disagree). Like `stats.rs`'s own `FrameLog`, it
  never calls `Instant::now()` itself — every timestamp is caller-supplied —
  which is what makes it possible to test with a deterministic mock clock
  instead of real wall-clock time; `latency.rs`'s own `#[cfg(test)]` module
  has 7 such tests, covering an empty log, a single input closed by the next
  frame, an input still pending before its frame presents, several inputs
  coalesced into one frame, percentile ordering across a spread of
  latencies, and the rolling window dropping its oldest samples.
  - *Wired into `app.rs`'s winit event loop* — **not done**: `app.rs` is
    2794 lines with many plausible input-dispatch sites and one existing
    `self.log.record(stats, elapsed, pixels, Instant::now())` call that
    would be the natural `frame_presented` call site; picking the exact
    input-dispatch points to call `input_arrived` from, across every
    winit event this crate handles, is real surgery across that whole file
    and was judged too large a blast radius to make safely alongside five
    other pillars in one pass. `InputLatencyLog` and `InputLatencyReport`
    are `pub`, re-exported from the crate root, and ready for an `App` to
    hold one and call into it from both ends — that wiring is honestly left
    as the next step, not faked as done.
  - *Reported anywhere a person can see it* (an overlay, a log line, a
    debug HUD) — **not done**, and blocked on the wiring above: there is
    nothing yet calling `InputLatencyLog::report()` in a running
    application.
- **A per-frame work budget**, not just "zero allocations": bytes
  allocated/copied, commands traversed, draw records touched, cache
  misses, tessellation/atlas work, pass count, barrier count, state and
  pipeline changes, descriptor updates, GPU bandwidth/occupancy. Target:
  minimum work for identical pixels, not merely zero allocations.
- **Damage as a cost model, not a fixed policy**: choose dynamically among
  full-frame / region-render / tile-render / reuse-existing / skip, based
  on damage area, region count/overlap, pass count, tile locality and
  target GPU — the current fixed 16-region limit is a starting point, not
  the end state. Partial rendering is not automatically a win on
  tile-based GPUs (extra load/store traffic, synchronization).
- Add **power-per-frame** to whatever benchmark suite eventually exists —
  a renderer 5% faster but 20% more power-hungry is not a win on a phone.

## GPU-driven execution

- Selectively move large, repeated UI workloads (icon grids, virtualized
  lists, tables, charts, scrolling surfaces) onto a GPU-driven path:
  retained draw records → GPU-visible instance buffers → GPU culling →
  indirect draw generation → batched execution. Not everywhere — the
  review is explicit this should stay selective, not a wholesale rewrite
  of the execution model.

  **Status: partially shipped**, as the one piece of this that has a real
  CPU-rasterizer analogue — see `crates/vieww-paint/src/native/instancing.rs`'s
  own module docs for the full reasoning.
  - *GPU-visible instance buffers, GPU culling, indirect draw generation* —
    **not applicable, not faked**: all three name specific GPU mechanisms
    (a buffer the GPU reads instance data from directly, a compute pass
    that culls off-screen instances before they reach a draw call, a draw
    call whose parameters are themselves read from GPU memory rather than
    supplied by the CPU) that have no meaning without a GPU execution
    pipeline, which this renderer does not have (`vieww-hal`'s Vulkan
    backend only presents pixels the CPU already rasterized — see
    `native/mod.rs`'s own docs).
  - *"Do the expensive geometric work once for identical shapes, not once
    per instance"* — the one part of "batched execution" with a direct,
    exact CPU translation — **shipped**: `InstancedMask::rasterize_once`
    runs the scanline rasterizer exactly once for a repeated shape;
    `InstancedMask::translated` produces every further instance by copying
    the resulting coverage to a new whole-pixel device origin, never
    re-walking an edge. Exact, not approximate, by construction — see that
    module's own "Why this is exact" section — and only for whole-pixel
    translations, the one case where reuse is mathematically lossless.
    Validated exactly as this task asked: a test renders five identically-
    shaped, differently-positioned squares through today's ordinary
    `Scene`/`NativeRenderer` path and through this module directly, checks
    the pixels come out byte-identical, and — via a `#[cfg(test)]`-only
    call counter added to `geometry::fill::rasterize` itself — measures
    that the instanced path rasterizes once where the ordinary path
    rasterizes once per shape (5 calls vs. 1), a real, counted reduction
    rather than one asserted only by the shape of the code.
  - *Retained draw records, detecting which draws are actually repeats* —
    **not done**: nothing upstream of this module (in `Scene`,
    `LayerTree`, or `vieww-widget`) currently identifies which commands
    share a shape, so `InstancedMask` is proven correct and ready but has
    no caller wiring it into `NativeRenderer::apply`'s per-command loop
    yet — see the module's own "Integration status" for exactly what that
    would take.

## Dynamic properties (animation)

- Formalize a **Dynamic Property System**: any value that can
  mathematically stay GPU-resident should (transform, opacity, color,
  corner radius, border width, shadow/blur radius, gradient params, clip
  params, morph progress, image crop, UV transform, suitable stroke
  widths, shader params) — generalizing the plan's existing
  transform/opacity/color uniform-update idea to the rest of the property
  surface.

## Correctness and testing

- Split blend-mode/compositing correctness into an explicit
  `BlendSemantics` layer — reference implementation, GPU implementation,
  then backend-specific optimization — each tested against the authoritative
  equations and known vectors, not "looks close to the CPU reference."
- **Reduce common-mode risk in the parity oracle.** The GPU and CPU paths
  sharing outline code means a shared bug reads as agreement. Add
  independent mathematical tests and an external known-good image corpus
  alongside the cross-implementation diff, so the CPU reference isn't the
  only thing certifying itself.

  **Status: shipped**, scoped to the one renderer this build actually has —
  there is no separate GPU blend implementation yet to compare against (a
  gap `docs/RENDERER-MIGRATION.md` already records), so "reference vs. GPU
  vs. backend-optimized" collapses to "reference vs. an independent
  check on the reference," which is exactly what got built:
  `crates/vieww-paint/src/native/blend_oracle.rs`, a `#[cfg(test)]`-only
  module (never compiled into a shipped build) implementing every Porter-Duff
  operator and all sixteen blend functions directly from the W3C
  Compositing and Blending Level 1 spec's equations — its own module docs
  spell out exactly what it deliberately does *not* share with
  `native/color.rs` (a different colour type, `f64` instead of `f32`, a
  literal `Fa`/`Fb` coefficient table instead of `color.rs`'s per-mode
  hand-simplified algebra, independently transcribed `Lum`/`ClipColor`/
  `Sat`/`SetLum`/`SetSat`). Three tests: `known_vectors` checks the oracle
  itself against numbers worked out a *third* way, by a Python script
  external to both Rust implementations; `known_identities` checks
  representation-independent invariants (multiply-by-white is identity,
  screen-with-black is identity, difference-with-self is black); and
  `agrees_with_the_shipped_renderer` is the actual deliverable — every one
  of the 28 modes, diffed against the oracle across 64 representative
  premultiplied colour pairs (opaque, translucent, near-transparent, and
  the zero-alpha edges each mode's own guards exist for), 1,792
  comparisons total, all passing within the tolerance below.
- **Split the global "2% drift" tolerance by feature category** (solid
  fills near-exact, rect edges extremely tight, text perceptual + stem
  metrics, gradients low numerical error, blends exact-vs-equations, blur/
  backdrop/shadows perceptual+structural, images controlled sampling
  tolerance) — a single global image-level metric can hide a locally
  catastrophic error.

  **Status: partially shipped**, scoped to blends specifically (the one
  category with an independent oracle to check against — the others in
  this bullet's list are about image-level perceptual comparison, which
  needs the "external known-good image corpus" the previous bullet also
  flags as not yet built). `agrees_with_the_shipped_renderer` splits its
  tolerance in two, not one global epsilon: `5e-5` for the twelve pure
  Porter-Duff operators (linear algebra on premultiplied channels, no
  division, so an f32-vs-f64 difference should not exceed a couple of ULPs
  at this magnitude) and `3e-3` for the sixteen blend-function modes
  (separable and non-separable alike go through an unpremultiply, a
  per-channel nonlinear function, and a re-premultiply — three more
  f32-rounding opportunities than a pure coverage operator). The full
  category list this bullet asks for (text, gradients, blur/shadow,
  images) is **not done** — each needs its own oracle or reference corpus
  built the same deliberate way blends just got, not a borrowed tolerance
  number with no independent check behind it.

## Benchmarking

- Expand the benchmark matrix along three axes: **workload** (static UI,
  dynamic UI, scrolling, text editing, large lists, charts, images, SVG,
  blur, backdrop blur, shadows, nested clipping, blend modes, hero
  transitions, large animations), **device class** (high-end desktop,
  integrated desktop GPU, mid laptop, low-end laptop, modern iPhone/iPad,
  mid/low-end Android), and **metric** (frame time p50/p95/p99/p99.9,
  input latency, CPU/GPU time and overlap, power, memory, bandwidth,
  startup shader cost, first-frame/warm-cache/cold-cache time, jank/frame
  drop count) — always same pixels, same workload, same device, against
  named baselines (Skia, native platform rendering, browser
  compositor, native toolkit renderers where relevant).
- Correct the layer framing: general-purpose UI toolkits are UI programming
  models, not renderers; application frameworks sit a layer up; Skia/browser
  compositors are the actual renderer-layer competitors. Keep those layers
  separate when making a comparative claim.

## Suggested milestone reordering

Original: M0 reference → M1 Vulkan → M2 geometry → M3 text → M4 compositor
→ M5 effects → M6 Metal → M7 integration → M8 Android/D3D12/perf.

Proposed: M0 reference + Frame IR + benchmark harness → M1 Vulkan + render
graph + resource system → M2 geometry + retained cache → M3 compositor +
damage + blend correctness → M4 text + color management → M5 effects +
backdrop + shader system → M6 dynamic properties/animation + GPU-driven
paths → M7 Metal + mobile tuning → M8 D3D12 + final optimization.
Rationale: render graph and the resource system need to land early, or
every later system gets built around execution assumptions that then have
to be redesigned.

## The reframed definition of "superior"

Instead of "ViewW must beat competitor X on feature Y," the review proposes:
for the same pixels, same workload, same device, same refresh target,
minimize total system cost across quality, latency, CPU, GPU, memory,
power, startup, jank, portability and capability — a renderer that "wins"
FPS while losing battery, latency, memory, startup, text fidelity, HDR or
stability has not actually won.
