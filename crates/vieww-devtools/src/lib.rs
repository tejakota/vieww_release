//! Developer tools: the inspector, snapshot testing, and a set of focused
//! inspectors over the numbers a performance or accessibility investigation
//! actually needs.
//!
//! # The inspector
//!
//! [`Inspector`] walks the element tree and reports what it finds: the
//! widget tree with types, build counts, signal subscriptions, and layout
//! bounds. It is read-only — it observes and reports, never mutates —
//! because a devtools that can change the tree is a devtools that can
//! corrupt the tree.
//!
//! # Snapshot testing
//!
//! Render a widget to an image, compare against a stored golden. The
//! comparison is pixel-exact by default and perceptual with a threshold,
//! because anti-aliasing differences between backends make exact matching
//! flaky on anything but the same machine.
//!
//! Both are feature-gated (`snapshots`) because the inspector's tree walk
//! is not free and the snapshot machinery needs vieww's own rasterizer.
//!
//! # The rest of the inspectors
//!
//! Each of the modules below reports on one real data source that already
//! exists elsewhere in the workspace — none of them compute anything a
//! second time, and each module's own docs say exactly which existing type
//! it reads and why. What is genuinely real, and what (honestly) is not:
//!
//! - [`frame_timeline`] — a bounded history of [`vieww_paint::FrameStats`],
//!   the per-phase timings `vieww-paint`'s own [`vieww_paint::FrameScheduler`]
//!   already measures against a real clock. Fully real: this only adds
//!   average/p95/max/histogram queries over a caller-fed window, never a
//!   second stopwatch.
//! - [`render_graph_inspector`] — a human-readable pass/resource/dependency
//!   report over a real, already-compiled `vieww-render-graph`
//!   [`vieww_render_graph::Graph`]/[`vieww_render_graph::ExecutionPlan`]
//!   pair. Fully real: culling, ordering, and aliasing are exactly what
//!   that crate's own `compile` produced.
//! - [`damage_inspector`] — region count, area, and coverage percentage
//!   over a real [`vieww_paint::Damage`] value. Fully real: every number is
//!   `Damage`'s own arithmetic, repackaged.
//! - `memory_inspector` (feature `snapshots`) — combines the target-buffer pool's reuse counts,
//!   the glyph-outline cache's hit/miss/eviction counts, and the image
//!   residency cache's real byte count into one report. Real, but honestly
//!   partial: the pool and glyph counters were never designed to report
//!   byte sizes, so `MemoryReport::total_estimated_bytes`
//!   is exactly the image cache's bytes and nothing invented for the other
//!   two — see that module's own docs. Requires the `snapshots` feature,
//!   since two of its three inputs only exist behind `vieww-paint/native`.
//! - [`semantics_inspector`] — node/role/focusable/live-region counts and
//!   an unlabeled-interactive-node count over a real
//!   [`vieww_render::SemanticsTree`]. Fully real: every count is a filter
//!   or a tally over that tree's own nodes.
//! - [`gpu_inspector`] — real reporting over `vieww-gpu`'s backend-agnostic
//!   [`vieww_gpu::resources::ResourceHeap`]/queues, plus
//!   [`gpu_inspector::DriverStats`]: an honestly-empty shape (every field
//!   `None`, always, in this build) for the numbers only a real GPU driver
//!   on real hardware can produce — VRAM as the driver itself reports it,
//!   GPU-side timestamp-query timing, a real pipeline cache's hit rate.
//!   Nothing here is a guess dressed as a measurement.
//! - [`json_export`] — a small hand-rolled JSON writer (this workspace has
//!   no `serde` dependency anywhere, and does not gain one here) providing
//!   [`json_export::ToJson`] for [`inspector::InspectorNode`] and every report type
//!   above, so a browser-based devtools UI has something structured to
//!   actually read.

pub mod damage_inspector;
pub mod frame_timeline;
pub mod gpu_inspector;
pub mod inspector;
pub mod json_export;
pub mod png_compare;
pub mod render_graph_inspector;
pub mod semantics_inspector;

// Snapshot testing rasterises, and the rasteriser is `vieww-paint`'s
// `native` feature. The module is gated on the `snapshots` feature — which
// turns that one on — rather than compiled always: without the gate,
// `vieww_paint::native` is simply absent and the crate fails to build in
// its own default configuration.
#[cfg(feature = "snapshots")]
pub mod snapshot;

// `memory_inspector` reads `vieww_paint::native::{PoolStats, ResidencyStats}`
// for two of its three inputs, so it is gated the same way and for the same
// reason as `snapshot` above.
#[cfg(feature = "snapshots")]
pub mod memory_inspector;

pub use damage_inspector::DamageReport;
pub use frame_timeline::FrameTimeline;
pub use gpu_inspector::{DriverStats, GpuReport};
pub use inspector::Inspector;
pub use json_export::ToJson;
#[cfg(feature = "snapshots")]
pub use memory_inspector::MemoryReport;
pub use render_graph_inspector::{PassReport, RenderGraphReport};
pub use semantics_inspector::{is_interactive, SemanticsReport};
#[cfg(feature = "snapshots")]
pub use snapshot::{SnapshotResult, SnapshotTester};
