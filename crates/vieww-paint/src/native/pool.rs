//! Transient buffer reuse for `PushLayer`/`PopLayer` — the CPU-rasterizer
//! shape of "Renderer v2" pillar A's transient resource aliasing
//! (`docs/RENDERER-V2-NOTES.md`).
//!
//! `target.rs`'s own module doc has said since the vello migration that one
//! `Target` type serves "the root frame buffer and every offscreen layer
//! ... minus the pooling", tracked as follow-up. This is that follow-up:
//! every isolated-layer buffer used to come from a fresh `Target::new(w, h)`
//! on `PushLayer` and be dropped on `PopLayer`'s return trip; now
//! [`NativeRenderer`](super::NativeRenderer) keeps a small pool of
//! already-allocated buffers keyed by exact size and hands one back out
//! instead of allocating, whenever a layer of a size it has seen before
//! opens again.
//!
//! # Why exact-size keying, not a bucket/nearest-fit scheme
//!
//! A GPU transient-resource allocator buckets by size class because
//! resizing a GPU allocation mid-frame is expensive to avoid entirely. A
//! `Vec<Premul>` has no such asymmetry — reusing a too-big buffer still
//! means writing `width * height` new elements to clear it, the same cost
//! as a fresh `vec![TRANSPARENT; n]` allocation would pay to zero itself,
//! so nearest-fit buys nothing here that isn't already free. Exact-size
//! keying is what actually saves the allocation: same `PushLayer` bounds
//! next frame (the overwhelmingly common case — a widget's clip rectangle
//! does not usually change frame to frame) means the buffer clear is the
//! only work, no allocator call at all.
//!
//! # Why sibling reuse falls out for free
//!
//! Layers are pushed and popped as a strict stack
//! (`native/reference.rs`'s `stack: Vec<LayerFrame>`), so two sibling
//! `PushLayer` groups are never open at once — the first is always popped,
//! returning its buffer to the pool, before the second is pushed. A pool
//! keyed only by size, with no notion of "this frame" versus "last frame",
//! already reuses a same-size sibling's buffer immediately, and reuses a
//! *previous frame's* buffer just as well once the whole scene has been
//! walked once. Nothing about the pool needs to know which case it is in.
//!
//! # What this does not do
//!
//! Buffers are reused within one [`TargetPool`], but nothing forces a
//! single `NativeRenderer` — and therefore its pool — to be reused across
//! frames beyond what an application already chooses to do by keeping one
//! `NativeRenderer` alive. A cap on how many idle buffers a pool keeps
//! (`MAX_IDLE`) exists purely so a scene with one very large one-off layer
//! (e.g. a full-screen filter applied exactly once) does not pin that
//! buffer's memory forever; buffers past the cap are simply dropped rather
//! than kept.

use super::target::Target;

/// A same-size buffer is reused after this many idle buffers are already
/// held, past which the marginal buffer is more likely to be a one-off (a
/// screen resize, a filter applied to an unusually large region) than
/// something worth holding onto — dropped instead of pooled.
const MAX_IDLE: usize = 32;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// `acquire` calls a same-size idle buffer satisfied, no allocation.
    pub reused: usize,
    /// `acquire` calls that had to allocate a fresh buffer.
    pub allocated: usize,
    /// `release` calls whose buffer was kept for later reuse.
    pub kept: usize,
    /// `release` calls whose buffer was dropped because the pool was
    /// already at `MAX_IDLE`.
    pub dropped: usize,
}

/// A free list of already-allocated [`Target`] buffers, keyed by exact
/// `(width, height)`.
#[derive(Debug, Default)]
pub(crate) struct TargetPool {
    idle: Vec<Target>,
    stats: PoolStats,
}

impl TargetPool {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A `width x height` buffer, cleared to fully transparent — either an
    /// idle buffer of exactly that size, reused, or a fresh allocation.
    pub(crate) fn acquire(&mut self, width: u32, height: u32) -> Target {
        if let Some(index) = self
            .idle
            .iter()
            .position(|t| t.width == width && t.height == height)
        {
            let mut target = self.idle.swap_remove(index);
            target.clear();
            self.stats.reused += 1;
            target
        } else {
            self.stats.allocated += 1;
            Target::new(width, height)
        }
    }

    /// Return a buffer for possible reuse by a later [`acquire`](Self::acquire)
    /// of the same size.
    pub(crate) fn release(&mut self, target: Target) {
        if self.idle.len() >= MAX_IDLE {
            self.stats.dropped += 1;
            return;
        }
        self.stats.kept += 1;
        self.idle.push(target);
    }

    pub(crate) fn stats(&self) -> PoolStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_size_acquire_after_release_reuses_the_buffer() {
        let mut pool = TargetPool::new();
        let a = pool.acquire(10, 10);
        pool.release(a);
        let _b = pool.acquire(10, 10);
        assert_eq!(
            pool.stats(),
            PoolStats {
                reused: 1,
                allocated: 1,
                kept: 1,
                dropped: 0
            }
        );
    }

    #[test]
    fn different_size_acquire_does_not_reuse() {
        let mut pool = TargetPool::new();
        let a = pool.acquire(10, 10);
        pool.release(a);
        let _b = pool.acquire(20, 20);
        assert_eq!(
            pool.stats(),
            PoolStats {
                reused: 0,
                allocated: 2,
                kept: 1,
                dropped: 0
            }
        );
    }

    #[test]
    fn reused_buffer_comes_back_cleared() {
        use super::super::color::Premul;
        let mut pool = TargetPool::new();
        let mut a = pool.acquire(4, 4);
        a.set(
            1,
            1,
            Premul {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        );
        pool.release(a);
        let b = pool.acquire(4, 4);
        assert_eq!(b.get(1, 1), Premul::TRANSPARENT);
    }

    #[test]
    fn releases_past_the_cap_are_dropped_not_kept() {
        let mut pool = TargetPool::new();
        let mut buffers = Vec::new();
        for i in 0..(MAX_IDLE + 5) {
            // Distinct sizes so none of these releases satisfy each other's
            // acquire and this test is purely about the release-side cap.
            buffers.push(pool.acquire(1, i as u32 + 1));
        }
        for buffer in buffers {
            pool.release(buffer);
        }
        let stats = pool.stats();
        assert_eq!(stats.kept, MAX_IDLE);
        assert_eq!(stats.dropped, 5);
    }

    #[test]
    fn a_sibling_layer_reuses_a_popped_siblings_buffer() {
        // Mirrors the stack discipline `native/reference.rs` actually uses:
        // push, pop, push again — never two same-size buffers open at once.
        let mut pool = TargetPool::new();
        let first = pool.acquire(50, 50);
        pool.release(first);
        let second = pool.acquire(50, 50);
        pool.release(second);
        assert_eq!(
            pool.stats(),
            PoolStats {
                reused: 1,
                allocated: 1,
                kept: 2,
                dropped: 0
            }
        );
    }
}
