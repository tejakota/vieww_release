//! A devtools-facing view of `vieww-gpu`'s backend-agnostic bookkeeping,
//! plus an honestly-empty shape for the numbers only a real GPU driver can
//! produce.
//!
//! # What is real
//!
//! Everything `vieww-gpu` itself computes without a GPU: the
//! [`vieww_gpu::resources::ResourceHeap`]'s tracked byte usage and budget,
//! and pending-transfer counts on its
//! [`vieww_gpu::resources::UploadQueue`] and
//! [`vieww_gpu::resources::ReadbackQueue`]. This crate does
//! not maintain a second copy of any of that bookkeeping — [`GpuReport`]
//! reads it straight off the real types.
//!
//! # What is not
//!
//! `ResourceHeap` tracks what *this process* believes it has allocated
//! against its own declared budget — it has no way to ask an actual GPU
//! driver how much VRAM is really free, how long a draw call actually took
//! on the device, or whether a pipeline cache hit on real hardware.
//! [`DriverStats`] is the honestly-empty shape those numbers would fill:
//! every field is `None` in this CPU-only workspace, and stays `None` until
//! a real backend (`vieww-gpu-vulkan`, `vieww-gpu-metal`, `vieww-gpu-d3d12`,
//! all built on `vieww-hal`) runs on hardware capable of answering the
//! query. Per this crate's own principle, a plausible-looking made-up
//! number here would be worse than an honest absence — a devtools panel
//! reading a fabricated "42% VRAM used" on a machine with no GPU backend at
//! all would be reporting a fact nobody measured.
//!
//! # Why `vieww_gpu::batch` has no counterpart here
//!
//! [`vieww_gpu::batch`]'s `group_by_mesh`/`worth_batching` are pure
//! functions over a caller-supplied draw list, not something that owns
//! state across frames — there is no batcher instance to ask "how are you
//! doing", only a call site's draw list and the batches it produced this
//! one time. A report over that would just be re-describing the caller's
//! own input back to them. If a real backend later keeps a running
//! batching-efficiency counter (batches produced versus draws submitted,
//! say), that counter would belong here; today there is not one to report.

use vieww_gpu::resources::{ResourceHeap, UploadQueue};

/// What `vieww-gpu`'s own backend-agnostic bookkeeping already knows,
/// without touching a GPU.
///
/// Not `Eq`: [`budget_used_percent`](Self::budget_used_percent) is a
/// measurement, and a measurement is the one kind of field for which exact
/// equality is not a question anybody should be asking (the same reasoning
/// `vieww_paint::FrameStats` gives for the same choice).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuReport {
    /// Bytes the [`ResourceHeap`] believes are currently allocated.
    pub used_bytes: u64,
    /// The heap's configured budget.
    pub budget_bytes: u64,
    /// `used_bytes / budget_bytes` as a percentage in `0.0..=100.0`. `0.0`
    /// for a zero budget rather than a division's undefined result.
    pub budget_used_percent: f64,
    /// Uploads queued but not yet drained this frame
    /// ([`UploadQueue::pending_count`]).
    pub pending_uploads: usize,
    /// Readbacks requested but not yet taken
    /// (`ReadbackQueue::take_pending` not yet called).
    pub pending_readbacks: usize,
    /// Real-GPU-only measurements. Always all-`None` here — see
    /// [`DriverStats`]'s own docs.
    pub driver: DriverStats,
}

impl GpuReport {
    /// Build a report from the real heap and queues a caller already owns.
    ///
    /// `pending_readbacks` is taken as a plain count rather than by draining
    /// the queue, because draining is a state-changing operation
    /// (`ReadbackQueue::take_pending` empties it) and a devtools read
    /// must not have a side effect on the very system it is inspecting —
    /// the same reason [`crate::inspector::Inspector`] is read-only. A
    /// caller passes `readback_queue`'s pending count from its own
    /// bookkeeping (or a `peek`-shaped accessor, if one is ever added to
    /// `vieww-gpu`) rather than this function consuming the queue to find
    /// out.
    #[must_use]
    pub fn build(
        heap: &ResourceHeap,
        upload_queue: &UploadQueue,
        pending_readbacks: usize,
    ) -> Self {
        let used_bytes = heap.used_bytes();
        let budget_bytes = heap.budget_bytes();
        let budget_used_percent = if budget_bytes > 0 {
            100.0 * used_bytes as f64 / budget_bytes as f64
        } else {
            0.0
        };
        Self {
            used_bytes,
            budget_bytes,
            budget_used_percent,
            pending_uploads: upload_queue.pending_count(),
            pending_readbacks,
            driver: DriverStats::default(),
        }
    }
}

/// Measurements only a real GPU driver on real hardware can produce.
///
/// **Every field is `None` in this CPU-sandboxed build, always.** There is
/// no fallback estimate and no placeholder number — a backend that actually
/// runs on a GPU (`vieww-gpu-vulkan`, `vieww-gpu-metal`, `vieww-gpu-d3d12`,
/// through `vieww-hal`) is the only thing that can fill these in, by
/// querying the driver's own memory-usage extension, a timestamp query
/// pool, and the pipeline cache's own hit counters respectively. Until one
/// of those exists and runs on hardware, this struct's shape documents what
/// a real backend *would* report, without pretending any of it was
/// measured.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DriverStats {
    /// Bytes of VRAM the driver reports as actually resident, as opposed to
    /// [`GpuReport::used_bytes`]'s own-process bookkeeping. Populated by a
    /// GPU backend via a vendor memory-budget extension
    /// (`VK_EXT_memory_budget` and equivalents); always `None` here.
    pub driver_reported_vram_bytes: Option<u64>,
    /// Total device memory the driver reports as available, for computing a
    /// real "percent of the whole card" figure rather than only "percent of
    /// this process's self-imposed budget". Always `None` here.
    pub driver_reported_vram_budget_bytes: Option<u64>,
    /// GPU-side time the most recently submitted frame took, from a real
    /// timestamp query pair around the frame's command buffer. Distinct
    /// from any CPU-side timing `vieww-paint`'s `FrameStats` already
    /// reports, which measures how long this process spent *recording*
    /// work, not how long the device spent *executing* it. Always `None`
    /// here.
    pub gpu_frame_time_micros: Option<u64>,
    /// Pipeline cache hits versus total lookups, on the real driver's own
    /// on-disk/on-device pipeline cache — a different thing from
    /// [`vieww_gpu::resources::PipelineCache`]'s in-process compile cache
    /// (that one needs no GPU to report on: it is just a `HashMap`, and its
    /// `len()` is a real, always-available number). Always `None` here.
    pub driver_pipeline_cache_hit_rate: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_gpu::resources::{Lifetime, ResidencyPriority};

    #[test]
    fn a_fresh_heap_reports_zero_usage_and_no_driver_stats() {
        let heap = ResourceHeap::new(1_000_000);
        let uploads = UploadQueue::new();
        let report = GpuReport::build(&heap, &uploads, 0);

        assert_eq!(report.used_bytes, 0);
        assert_eq!(report.budget_bytes, 1_000_000);
        assert_eq!(report.budget_used_percent, 0.0);
        assert_eq!(report.pending_uploads, 0);
        assert_eq!(report.pending_readbacks, 0);
        assert_eq!(report.driver, DriverStats::default());
    }

    #[test]
    fn every_driver_stat_field_is_none_by_construction() {
        let stats = DriverStats::default();
        assert_eq!(stats.driver_reported_vram_bytes, None);
        assert_eq!(stats.driver_reported_vram_budget_bytes, None);
        assert_eq!(stats.gpu_frame_time_micros, None);
        assert_eq!(stats.driver_pipeline_cache_hit_rate, None);
    }

    #[test]
    fn budget_used_percent_matches_the_hand_computed_fraction() {
        let mut heap = ResourceHeap::new(1000);
        heap.allocate(250, Lifetime::Persistent, ResidencyPriority::Normal)
            .unwrap();
        let uploads = UploadQueue::new();
        let report = GpuReport::build(&heap, &uploads, 0);

        assert_eq!(report.used_bytes, 250);
        assert_eq!(report.budget_used_percent, 25.0);
    }

    #[test]
    fn a_zero_budget_reports_zero_percent_rather_than_dividing_by_zero() {
        let heap = ResourceHeap::new(0);
        let uploads = UploadQueue::new();
        let report = GpuReport::build(&heap, &uploads, 0);
        assert_eq!(report.budget_used_percent, 0.0);
    }

    #[test]
    fn pending_upload_and_readback_counts_are_reported_exactly() {
        let heap = ResourceHeap::new(1000);
        let mut uploads = UploadQueue::new();
        uploads.enqueue(1, 10);
        uploads.enqueue(2, 20);

        let report = GpuReport::build(&heap, &uploads, 3);
        assert_eq!(report.pending_uploads, 2);
        assert_eq!(report.pending_readbacks, 3);
    }
}
