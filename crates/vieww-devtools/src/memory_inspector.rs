//! A single rollup over the memory-shaped counters that already exist
//! scattered across the paint and image layers.
//!
//! Nothing here counts bytes itself. Three real sources already exist:
//!
//! - [`vieww_paint::native::NativeRenderer::pool_stats`] — how often the
//!   `PushLayer`/`PopLayer` offscreen-buffer pool reused an existing buffer
//!   versus allocating a fresh one (see `native/pool.rs`'s module docs).
//! - [`vieww_paint::native::NativeRenderer::glyph_outline_cache_stats`] —
//!   hit/miss/eviction counts for the per-font glyph-outline residency
//!   cache (`native/glyph.rs`, `native/residency.rs`).
//! - [`vieww_image::residency::ImageResidencyCache`] — a byte-budgeted LRU
//!   for decoded images, which (unlike the two above) does report an exact
//!   current byte count.
//!
//! This module does not own a live [`NativeRenderer`](vieww_paint::native::NativeRenderer)
//! or [`ImageResidencyCache`](vieww_image::residency::ImageResidencyCache) —
//! `vieww-devtools` inspects other layers, it does not host them — so
//! [`MemoryReport::build`] takes each source's already-read statistics as
//! plain values. A caller that owns the renderer and the image cache calls
//! their `*_stats()`/`current_bytes()` methods once a frame and hands the
//! results here.
//!
//! # Why `total_estimated_bytes` is not "the app's memory usage"
//!
//! [`PoolStats`](vieww_paint::native::PoolStats) and the paint-side
//! [`ResidencyStats`](vieww_paint::native::ResidencyStats) are exactly what
//! their own modules chose to expose: reuse/hit/miss/eviction *counts*,
//! with no byte size attached to any of them (`native/pool.rs`'s
//! `TargetPool` and `native/residency.rs`'s `ResidencyCache` both track
//! cost internally but keep it private — nothing outside those modules can
//! ask "how many bytes is this cache holding right now"). Only the image
//! residency cache reports an exact byte count
//! ([`ImageResidencyCache::current_bytes`](vieww_image::residency::ImageResidencyCache::current_bytes)).
//!
//! So [`MemoryReport::total_estimated_bytes`] is honestly just the image
//! cache's bytes — a real number, not a guess — plus zero for the other two
//! caches, which is not the same claim as "the other two caches use no
//! memory". Reporting an invented byte-per-entry multiplier for them would
//! be exactly the "wrong-looking pseudocode" this codebase's own principle
//! warns against: a plausible-looking total that is quietly missing two of
//! its three terms is worse than a total that is visibly partial, alongside
//! the hit/miss counters that *are* real and cost-comparable on their own
//! terms.
//!
//! # Why this is behind the `snapshots` feature
//!
//! [`PoolStats`] and the paint-side [`ResidencyStats`] only exist when
//! `vieww-paint`'s `native` feature is enabled — `vieww-devtools`'s
//! `snapshots` feature is what turns that on (see `lib.rs`'s module docs).
//! Without it there is no [`NativeRenderer`] and therefore nothing to read
//! these two counters from at all.

use vieww_image::residency::ResidencyStats as ImageResidencyStats;
use vieww_paint::native::{PoolStats, ResidencyStats as GlyphResidencyStats};

/// One rollup of memory-shaped counters from across the paint and image
/// layers. See the module docs for what each field actually means and why
/// [`total_estimated_bytes`](Self::total_estimated_bytes) is partial by
/// honest necessity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemoryReport {
    /// `PushLayer`/`PopLayer` offscreen-buffer reuse. No byte size — see
    /// the module docs.
    pub target_pool: PoolStats,
    /// Glyph-outline cache hit/miss/eviction counts, summed across every
    /// font. No byte size — see the module docs.
    pub glyph_outline_cache: GlyphResidencyStats,
    /// Decoded-image residency cache hit/miss/eviction counts.
    pub image_cache: ImageResidencyStats,
    /// The decoded-image residency cache's exact current byte count
    /// ([`ImageResidencyCache::current_bytes`](vieww_image::residency::ImageResidencyCache::current_bytes)).
    pub image_cache_bytes: u64,
}

impl MemoryReport {
    /// Combine already-read statistics from each source into one report.
    ///
    /// Takes plain values rather than the live caches themselves — see the
    /// module docs for why.
    #[must_use]
    pub const fn build(
        target_pool: PoolStats,
        glyph_outline_cache: GlyphResidencyStats,
        image_cache: ImageResidencyStats,
        image_cache_bytes: u64,
    ) -> Self {
        Self {
            target_pool,
            glyph_outline_cache,
            image_cache,
            image_cache_bytes,
        }
    }

    /// The only bytes this report actually knows about — see the module
    /// docs on why the other two caches do not contribute a number here.
    #[must_use]
    pub const fn total_estimated_bytes(&self) -> u64 {
        self.image_cache_bytes
    }

    /// The target pool's reuse rate, in `0.0..=1.0` — `reused / (reused +
    /// allocated)`. `None` if `acquire` was never called, which is not the
    /// same as a 0% reuse rate.
    #[must_use]
    pub fn target_pool_reuse_rate(&self) -> Option<f64> {
        let total = self.target_pool.reused + self.target_pool.allocated;
        (total > 0).then(|| self.target_pool.reused as f64 / total as f64)
    }

    /// The glyph-outline cache's hit rate, in `0.0..=1.0`. `None` if it was
    /// never queried.
    #[must_use]
    pub fn glyph_outline_hit_rate(&self) -> Option<f64> {
        let total = self.glyph_outline_cache.hits + self.glyph_outline_cache.misses;
        (total > 0).then(|| self.glyph_outline_cache.hits as f64 / total as f64)
    }

    /// The image cache's hit rate, in `0.0..=1.0`. `None` if it was never
    /// queried.
    #[must_use]
    pub fn image_cache_hit_rate(&self) -> Option<f64> {
        let total = self.image_cache.hits + self.image_cache.misses;
        (total > 0).then(|| self.image_cache.hits as f64 / total as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_estimated_bytes_is_exactly_the_image_caches_bytes() {
        let report = MemoryReport::build(
            PoolStats {
                reused: 10,
                allocated: 2,
                kept: 5,
                dropped: 1,
            },
            GlyphResidencyStats {
                hits: 100,
                misses: 20,
                evictions: 3,
            },
            ImageResidencyStats {
                hits: 4,
                misses: 1,
                evictions: 0,
            },
            65_536,
        );
        assert_eq!(
            report.total_estimated_bytes(),
            65_536,
            "the pool and glyph cache carry no byte size to add"
        );
    }

    #[test]
    fn target_pool_reuse_rate_is_the_hand_computed_fraction() {
        let report = MemoryReport::build(
            PoolStats {
                reused: 30,
                allocated: 10,
                kept: 0,
                dropped: 0,
            },
            GlyphResidencyStats::default(),
            ImageResidencyStats::default(),
            0,
        );
        // 30 / (30 + 10) = 0.75
        assert_eq!(report.target_pool_reuse_rate(), Some(0.75));
    }

    #[test]
    fn a_pool_never_used_reports_no_reuse_rate_rather_than_zero() {
        let report = MemoryReport::build(
            PoolStats::default(),
            GlyphResidencyStats::default(),
            ImageResidencyStats::default(),
            0,
        );
        assert_eq!(
            report.target_pool_reuse_rate(),
            None,
            "never-used is a different fact than 0% reuse"
        );
    }

    #[test]
    fn glyph_outline_hit_rate_is_the_hand_computed_fraction() {
        let report = MemoryReport::build(
            PoolStats::default(),
            GlyphResidencyStats {
                hits: 90,
                misses: 10,
                evictions: 0,
            },
            ImageResidencyStats::default(),
            0,
        );
        assert_eq!(report.glyph_outline_hit_rate(), Some(0.9));
    }

    #[test]
    fn image_cache_hit_rate_is_the_hand_computed_fraction() {
        let report = MemoryReport::build(
            PoolStats::default(),
            GlyphResidencyStats::default(),
            ImageResidencyStats {
                hits: 3,
                misses: 1,
                evictions: 0,
            },
            0,
        );
        assert_eq!(report.image_cache_hit_rate(), Some(0.75));
    }

    #[test]
    fn a_default_report_is_all_zero_and_every_rate_is_none() {
        let report = MemoryReport::default();
        assert_eq!(report.total_estimated_bytes(), 0);
        assert_eq!(report.target_pool_reuse_rate(), None);
        assert_eq!(report.glyph_outline_hit_rate(), None);
        assert_eq!(report.image_cache_hit_rate(), None);
    }
}
