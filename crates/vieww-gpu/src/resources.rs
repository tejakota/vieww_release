//! The resource virtualization layer.
//!
//! `docs/RENDERER-V2-NOTES.md`: "resource virtualization layer: buffers,
//! textures, pipelines, samplers, atlases, external textures, residency —
//! with deferred destruction, fence-based reclamation, lifetime tracking,
//! residency priorities, compaction, aliasing, budget enforcement, upload
//! scheduling," building on the identity/residency split that same
//! document calls for (`GeometryCache → GpuResidencyCache →
//! GpuAllocation`). Everything in this module is backend-agnostic
//! bookkeeping — real device memory only exists inside a concrete
//! [`crate::device::Device`] implementation, which is exactly why this can
//! be exhaustively unit-tested without one.

use std::collections::HashMap;

use vieww_foundation::Size;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BufferUsage {
    Vertex,
    Index,
    Uniform,
    /// Instance data — the buffer a batched draw's per-instance transforms,
    /// colors, and UVs live in. See [`crate::batch`].
    Instance,
    Storage,
    /// Written by the CPU, read by the GPU — the destination side of
    /// [`UploadQueue`].
    Staging,
    /// Written by the GPU, read by the CPU — the destination side of
    /// [`ReadbackQueue`] (a screenshot, a color-picker probe).
    Readback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8UnormSrgb,
    /// scRGB-shaped: linear, extended-range, negative values meaningful —
    /// the format a wide-gamut/HDR pipeline actually composites in. See
    /// `vieww-foundation::color`'s `ColorSpace` for the color-managed value
    /// type this format stores, and this crate's own `hdr` module for the
    /// tone-mapping step down to a display's real format.
    Rgba16Float,
    Depth32Float,
}

impl TextureFormat {
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb => 4,
            Self::Rgba16Float => 8,
            Self::Depth32Float => 4,
        }
    }

    #[must_use]
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::Rgba16Float)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureUsage {
    ColorTarget,
    DepthTarget,
    Sampled,
    Presentable,
}

/// Identifies one resource allocation tracked by a [`ResourceHeap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AllocationId(u64);

/// Whether an allocation should be reclaimed the moment nothing references
/// it (transient — a frame's intermediate layer target) or kept around
/// across frames until explicitly released or evicted under budget
/// pressure (persistent — a decoded image, a glyph atlas page).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lifetime {
    Transient,
    Persistent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Allocation {
    bytes: u64,
    lifetime: Lifetime,
    /// Set once a fence confirms the GPU is done with this allocation —
    /// only then is it actually safe to reuse or free the underlying
    /// memory. Modeling this explicitly (rather than freeing the moment a
    /// caller drops its handle) is the "deferred destruction, fence-based
    /// reclamation" the module doc names — a CPU `Vec`'s destructor can run
    /// synchronously; a GPU allocation's cannot, because the GPU may still
    /// be reading it.
    gpu_retired: bool,
    priority: ResidencyPriority,
}

/// How eagerly an allocation is evicted under memory pressure, when it is
/// eligible for eviction at all ([`Lifetime::Persistent`] only —
/// [`Lifetime::Transient`] allocations are reclaimed on retirement, not by
/// priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResidencyPriority {
    /// Evict first (a prefetched image nothing on screen references yet).
    Low,
    Normal,
    /// Evict last (a glyph atlas backing visible text).
    High,
    /// Never evict automatically — only an explicit release frees it (the
    /// presentable surface's own backing store).
    Critical,
}

/// Tracks every live allocation's size, lifetime, and GPU-retirement state,
/// enforcing a total memory budget by evicting the lowest-priority
/// [`Lifetime::Persistent`] allocations first — the policy
/// `docs/RENDERER-V2-NOTES.md` asks for, expressed independently of any
/// real device memory (a concrete `Device` maps an `AllocationId` to its
/// own real buffer/texture and actually frees it when this heap says to).
#[derive(Debug)]
pub struct ResourceHeap {
    budget_bytes: u64,
    used_bytes: u64,
    next_id: u64,
    allocations: HashMap<AllocationId, Allocation>,
}

impl ResourceHeap {
    #[must_use]
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            used_bytes: 0,
            next_id: 0,
            allocations: HashMap::new(),
        }
    }

    /// Register a new allocation of `bytes`, evicting lower-priority
    /// persistent allocations first if needed to stay within budget.
    ///
    /// Returns the allocations that were evicted to make room, so a caller
    /// (a real `Device`) knows which underlying GPU objects it must
    /// actually free. Returns `Err(bytes)` — the size that would not fit —
    /// if even evicting everything evictable cannot make room (e.g. a
    /// single allocation larger than the whole budget, or every existing
    /// allocation is [`ResidencyPriority::Critical`]).
    pub fn allocate(
        &mut self,
        bytes: u64,
        lifetime: Lifetime,
        priority: ResidencyPriority,
    ) -> Result<(AllocationId, Vec<AllocationId>), u64> {
        let mut evicted = Vec::new();
        while self.used_bytes + bytes > self.budget_bytes {
            let victim = self
                .allocations
                .iter()
                .filter(|(_, a)| {
                    a.lifetime == Lifetime::Persistent && a.priority != ResidencyPriority::Critical
                })
                .min_by_key(|(_, a)| (a.priority, a.bytes))
                .map(|(id, _)| *id);

            match victim {
                Some(id) => {
                    let a = self.allocations.remove(&id).expect("just found");
                    self.used_bytes -= a.bytes;
                    evicted.push(id);
                }
                None => return Err(bytes),
            }
        }

        let id = AllocationId(self.next_id);
        self.next_id += 1;
        self.used_bytes += bytes;
        self.allocations.insert(
            id,
            Allocation {
                bytes,
                lifetime,
                gpu_retired: false,
                priority,
            },
        );
        Ok((id, evicted))
    }

    /// Mark an allocation as confirmed-done-with-by-the-GPU (a fence this
    /// allocation's last use was submitted under has signaled). A
    /// transient allocation becomes eligible for reuse only after this.
    pub fn retire(&mut self, id: AllocationId) {
        if let Some(a) = self.allocations.get_mut(&id) {
            a.gpu_retired = true;
        }
    }

    #[must_use]
    pub fn is_retired(&self, id: AllocationId) -> bool {
        self.allocations.get(&id).is_some_and(|a| a.gpu_retired)
    }

    /// Explicitly free an allocation (the caller no longer needs it,
    /// independent of eviction pressure).
    pub fn free(&mut self, id: AllocationId) {
        if let Some(a) = self.allocations.remove(&id) {
            self.used_bytes -= a.bytes;
        }
    }

    #[must_use]
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    #[must_use]
    pub fn budget_bytes(&self) -> u64 {
        self.budget_bytes
    }
}

/// Schedules host-to-device transfers. Backend-agnostic in the same sense
/// as [`ResourceHeap`]: this decides *what order* pending uploads should
/// happen in (largest-first, so a big background image doesn't starve a
/// dozen small icon uploads queued behind it — smallest-first drains more
/// requests within a frame's bandwidth budget instead) and reports how much
/// bandwidth budget is left; the actual `memcpy`-to-staging-buffer and
/// `vkCmdCopyBuffer`-equivalent happen inside a real `Device`.
#[derive(Debug, Default)]
pub struct UploadQueue {
    pending: Vec<(u64, u64)>, // (id, bytes), reusing AllocationId's inner value informally
}

impl UploadQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enqueue(&mut self, id: u64, bytes: u64) {
        self.pending.push((id, bytes));
    }

    /// Drain uploads that fit within `budget_bytes` this frame,
    /// smallest-first, leaving the rest queued for the next frame.
    pub fn drain_within_budget(&mut self, budget_bytes: u64) -> Vec<u64> {
        self.pending.sort_by_key(|(_, bytes)| *bytes);
        let mut used = 0u64;
        let mut drained = Vec::new();
        let mut remaining = Vec::new();
        for (id, bytes) in self.pending.drain(..) {
            if used + bytes <= budget_bytes {
                used += bytes;
                drained.push(id);
            } else {
                remaining.push((id, bytes));
            }
        }
        self.pending = remaining;
        drained
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

/// The GPU-to-host counterpart of [`UploadQueue`] — a screenshot request, a
/// color-picker probe, or a devtools "inspect this texture" pull.
#[derive(Debug, Default)]
pub struct ReadbackQueue {
    pending: Vec<u64>,
}

impl ReadbackQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&mut self, id: u64) {
        if !self.pending.contains(&id) {
            self.pending.push(id);
        }
    }

    pub fn take_pending(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.pending)
    }
}

/// A cache key for a compiled pipeline — reflection-derived, not a raw
/// source hash, so two shaders that differ only in whitespace or comments
/// still share one compiled pipeline. `vieww-shaders` produces the
/// `naga`-validated module this key is derived from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PipelineCacheKey {
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub color_format: TextureFormat,
    pub blend_enabled: bool,
}

/// Caches compiled pipelines by [`PipelineCacheKey`] so a repeated draw of
/// the same shader/format combination never recompiles or re-links a
/// pipeline state object — the "pipeline cache" `docs/RENDERER-V2-NOTES.md`
/// and the target architecture both name. Generic over the backend's own
/// pipeline handle type.
#[derive(Debug, Default)]
pub struct PipelineCache<P> {
    entries: HashMap<PipelineCacheKey, P>,
}

impl<P> PipelineCache<P> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Look up `key`, compiling with `compile` on a miss.
    pub fn get_or_compile(&mut self, key: PipelineCacheKey, compile: impl FnOnce() -> P) -> &P {
        self.entries.entry(key).or_insert_with(compile)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A rough byte estimate for a texture of `size` in `format` — shared by
/// every caller that needs to know before actually allocating one (budget
/// checks, transient sizing).
#[must_use]
pub fn texture_bytes(size: Size, format: TextureFormat) -> u64 {
    (size.width.max(0.0) as u64)
        * (size.height.max(0.0) as u64)
        * u64::from(format.bytes_per_pixel())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_within_budget_needs_no_eviction() {
        let mut heap = ResourceHeap::new(1000);
        let (_, evicted) = heap
            .allocate(100, Lifetime::Persistent, ResidencyPriority::Normal)
            .unwrap();
        assert!(evicted.is_empty());
        assert_eq!(heap.used_bytes(), 100);
    }

    #[test]
    fn low_priority_allocations_are_evicted_before_high_priority_ones() {
        let mut heap = ResourceHeap::new(150);
        let (low, _) = heap
            .allocate(100, Lifetime::Persistent, ResidencyPriority::Low)
            .unwrap();
        let (_high, evicted) = heap
            .allocate(100, Lifetime::Persistent, ResidencyPriority::High)
            .unwrap();
        assert_eq!(evicted, vec![low]);
    }

    #[test]
    fn a_critical_allocation_is_never_evicted() {
        let mut heap = ResourceHeap::new(100);
        heap.allocate(100, Lifetime::Persistent, ResidencyPriority::Critical)
            .unwrap();
        let result = heap.allocate(1, Lifetime::Persistent, ResidencyPriority::Normal);
        assert_eq!(result, Err(1));
    }

    #[test]
    fn upload_queue_drains_smallest_first_within_budget() {
        let mut q = UploadQueue::new();
        q.enqueue(1, 500);
        q.enqueue(2, 100);
        q.enqueue(3, 300);
        let drained = q.drain_within_budget(450);
        assert_eq!(drained, vec![2, 3]);
        assert_eq!(q.pending_count(), 1);
    }

    #[test]
    fn pipeline_cache_compiles_once_per_key() {
        let mut cache: PipelineCache<u32> = PipelineCache::new();
        let key = PipelineCacheKey {
            vertex_entry: "vs_main",
            fragment_entry: "fs_main",
            color_format: TextureFormat::Rgba8Unorm,
            blend_enabled: true,
        };
        let mut compiles = 0;
        cache.get_or_compile(key.clone(), || {
            compiles += 1;
            1
        });
        cache.get_or_compile(key, || {
            compiles += 1;
            2
        });
        assert_eq!(compiles, 1);
        assert_eq!(cache.len(), 1);
    }
}
