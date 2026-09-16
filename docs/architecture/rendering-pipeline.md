# The rendering pipeline: scene → render graph → render planner → CPU/GPU/Hybrid

This is the half of the workspace built specifically in response to the
external architectural assessment's central point: treat the render graph "as
a first-class component, not an implementation detail," and reframe the
target question from "how do I execute these commands" to "what is the
cheapest possible execution plan for this frame." Four crates answer that,
each producing input for the next and executing nothing itself until the
final stage:

```
vieww_render::Scene (vieww-paint)
        │  vieww_scene::build (renderer-independent scene IR)
        ▼
   SceneGraph                (a real tree, not a flat command list; per-node cost)
        │  scene_bridge::build_graph
        ▼
      Graph                  (passes + resources, as declared)
        │  Graph::compile
        ▼
  ExecutionPlan               (ordered, culled, aliased, batched, barriered)
        │  planner::plan_frame  (+ DeviceProfile + FrameBudget)
        ▼
    FramePlan                (a Placement — Cpu / Gpu / Hybrid(split) — per pass)
        │
        ▼
  vieww-paint::native (CPU)  |  vieww-gpu + vieww-hal + vieww-shaders (GPU)  |  Hybrid
```

`vieww-runtime` sits beside this whole pipeline rather than inside it — see
the last section below.

## `vieww-scene`: renderer-independent scene IR

Before this crate existed, the only representation of "what to draw" was
`vieww_paint::Scene`'s flat command list with matching push/pop markers for
layers — workable for a single CPU rasterizer, but not something a render
graph, a CPU/GPU/hybrid planner, or a real GPU backend could all reason about
on equal footing without each one reaching back into that list on its own
terms. `vieww-scene`'s `SceneGraph` is a tree over the *same* drawing
primitives `vieww_paint::Command` already defines — rects, paths, strokes,
shadows, glyph runs, images, layers — deliberately not a new set of
primitives, since duplicating those would itself be the isolated-feature
problem the workspace's own docs warn against elsewhere. What's actually new
is structure (a real tree instead of a flat list, in `node`) and cost (`cost`
— a heuristic classification of how dynamic, cacheable, and GPU-suited each
node is, which a flat command list has nowhere to record).

## `vieww-render-graph`: the graph itself

Built directly on `SceneGraph` rather than on `vieww_paint::Scene`'s command
list, this crate turns a scene into a `Graph` of passes and resources
(`scene_bridge::build_graph`), then compiles that into an `ExecutionPlan` —
ordered, with dead passes culled, resources aliased where their lifetimes
don't overlap, work batched, and barriers placed. Nothing in this crate
executes anything; it produces the plan and stops. Its own tests cover the
graph-building rules directly: a pass with no declared writer for a resource
is rejected, multiple writers to the same resource is rejected, independent
passes land in the same concurrency batch, and a dead (unread) pass is culled
from the compiled plan rather than silently run.

## `vieww-render-planner`: CPU/GPU/hybrid as policy, not as three backends

This crate is the direct implementation of the assessment's other specific
point: "CPU/GPU/hybrid should be a policy, not just three backends." It takes
an `ExecutionPlan` plus a `DeviceProfile` (what this device's CPU and GPU
actually cost, per pixel, learned from an exponential moving average over
measured history where available and a heuristic otherwise — see `cost`) and
a `FrameBudget` (how much time is left before a missed deadline, at whatever
refresh rate the display is running), and produces a `FramePlan`: one
`Placement` — `Cpu`, `Gpu`, or `Hybrid` with a split fraction — per pass.
`policy` layers real affinity rules on top of the raw cost comparison (a pass
pinned `CpuAffinity` never moves to the GPU even if the numbers say it's
cheaper there; a tiny pass is never split even when its CPU and GPU cost
estimates are close, because the split's own overhead would dominate); and
`power` adds an energy dimension distinct from raw speed (a plugged-in device
never trades speed for power, a battery-powered one sometimes will, and a
tight frame budget never trades speed for power regardless of power state).
Nothing here executes a pass either — a `FramePlan` is handed to whichever
backend actually carries it out.

## The three executors

- **CPU — `vieww-paint`.** The paint layer, and specifically its `native`
  module, is where every `Placement::Cpu` pass actually runs today. Its own
  doc describes the pieces: `Canvas` (the drawing interface render objects
  see, deliberately Skia-shaped because that model maps onto every backend
  worth having), `Scene` (a `Canvas` that records instead of drawing,
  resolving transforms and clips into absolute coordinates as each command
  arrives), `LayerTree` (repaint boundaries — subtrees that keep their own
  recording so a repaint stops at a boundary instead of running to the root),
  `Damage` (what changed, so a frame can redraw a corner of the screen rather
  than all of it), and `FrameScheduler` (when a frame runs and in what
  order). Together these are what make a repaint cost something proportional
  to the change rather than to the screen size.
- **GPU — `vieww-gpu` + `vieww-hal` + `vieww-shaders`.** `vieww-hal` is the
  hardware abstraction layer itself: a closed, sealed trait family
  (`Device`) implemented once per backend, Vulkan first. It depends on
  `vieww-foundation` alone — not on `vieww-paint`'s `Scene`/`Canvas`/`Damage`
  machinery — specifically so anything that only needs to ask "is there a
  working GPU on this machine" (`vieww-hardware`'s `Capability::Gpu` probe)
  can do so without pulling in the paint layer. `vieww-gpu` builds the
  backend-agnostic execution pipeline on top: a resource virtualization layer
  (`resources`), the CPU-side geometry prep every backend needs
  (`tessellate`, `batch`), the scene planner that turns a `vieww_paint::Scene`
  into one batched vertex buffer (`scene`), and the glyph atlas that lets that
  buffer carry text as well as shapes (`atlas`). `vieww-shaders` is the one WGSL shader library
  every backend compiles through — cross-compiled via `naga` to
  SPIR-V/MSL/HLSL, with reflection, pipeline cache keys, and hot-reload
  hooks — so shader source lives in exactly one place instead of being
  copy-pasted per backend the way `vieww-hal::vulkan`'s early
  `CLEAR_SHADER_WGSL`/`MESH_SHADER_WGSL` constants were before this crate
  existed. **What's real without a GPU, and what needs one:** every trait in
  `vieww-hal`/`vieww-gpu` is unit-tested against `null::NullDevice`, a
  complete software implementation, so construction, validation, and
  CPU-side bookkeeping are exercised anywhere.

  The Vulkan backend goes further than that now, and this paragraph used to
  say otherwise: with `mesa-vulkan-drivers` installed, `lavapipe` provides an
  ICD and the four `#[ignore]`d suites — `vulkan_smoke`, `vulkan_mesh_smoke`,
  `vulkan_scene` and `vulkan_text` — **run**, 20 tests, comparing whole frames
  against `vieww_paint`'s CPU rasterizer. `cargo test -p vieww-hal --features
  vulkan -- --ignored` is the command.

  Two limits on what that proves, both of which matter: `lavapipe` is a
  *software* Vulkan implementation, so this is evidence of correctness and
  none at all of performance; and `ScenePlan` still cannot express images,
  shadows, gradients, shaped clips or layers, so most real screens are still
  refused by `is_complete` and drawn on the CPU. See
  [`TRACKER.md`](../../TRACKER.md) for what has been verified and
  [`PENDING.md`](../../PENDING.md) for what has not.
- **Hybrid.** A `Placement::Hybrid(split)` pass runs part of its work on each
  executor according to the planner's chosen split fraction; the mechanics
  of *how* a single pass divides are the executor's own concern, not the
  planner's — the planner only ever hands down a fraction and a deadline.

## `vieww-runtime`: the frame around all of it

`vieww-runtime` doesn't sit in this pipeline — it's what runs it, once per
frame, in order: input → scheduler → state update → tree update → layout →
render-plan → GPU/CPU scheduling. Its `scheduler::Scheduler` is a
priority-ordered thread pool (input work always drains ahead of everything
else; background work always drains last), and its
`frame::FrameOrchestrator` is that named pipeline, tracking which stage a
frame is in and which one is the slowest so a dropped frame can be attributed
rather than just observed.
