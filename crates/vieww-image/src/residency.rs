//! A byte-budgeted, LRU-evictable cache for decoded images — the piece
//! `vieww-asset`'s own `ImageCache` (see its `cache` module) deliberately
//! does not provide.
//!
//! # Why this exists next to `vieww-asset::ImageCache`
//!
//! `vieww-asset`'s cache module documents its own design choice plainly: no
//! eviction, ever, short of an explicit `clear()`. Its reasoning is that an
//! LRU needs a budget, a budget needs a number, and the right number depends
//! on the device and on what the application shows — so a wrong default
//! there would be a memory leak on one phone and a thrashing cache on
//! another, and an application that actually wants bounded residency should
//! ask for it explicitly rather than get it as a hidden behaviour of the
//! decode-on-demand cache every image load already goes through.
//!
//! This module is that explicit ask: an application that *does* know its
//! budget — "keep at most 64MB of decoded thumbnails resident" — reaches for
//! [`ResidencyCache`] (or the [`ImageResidencyCache`] specialisation) on
//! purpose, instead of `vieww-asset`'s cache growing eviction logic that
//! would be wrong for every caller that does not want it.
//!
//! # Why this duplicates the design in `vieww-paint::native::residency`
//! rather than depending on it
//!
//! `vieww-paint`'s `native` module has the identical shape of cache
//! (`ResidencyCache<K, V>`: a `HashMap` plus a `Vec<K>` recency order,
//! evicted down to a byte budget, `hits`/`misses`/`evictions` stats) for its
//! `GlyphCache`. Two real obstacles rule out reusing it directly rather than
//! it just being convenient not to:
//!
//! 1. It is `pub(crate)` inside `vieww-paint` — not part of that crate's
//!    public API for anything outside it to name, image caching included.
//! 2. Even if it were exported, pulling in `vieww-paint` as a dependency
//!    just to borrow one internal generic cache type would run the wrong
//!    way for what this crate is: `vieww-image` is decode-adjacent
//!    tooling (mipmaps, atlases, sequencing, profiles, residency) that
//!    other crates — including, potentially, a future `vieww-paint`
//!    texture-residency path — might reasonably want to depend on, not a
//!    consumer of the paint layer's own rendering machinery.
//!
//! So the design is copied — same `HashMap` + `Vec<K>` linear-scan
//! recency order, same "budget is a target, not a hard cap" softness for a
//! single oversized entry, same stats shape — because it is already the
//! right, proven answer to "small-cache LRU under a byte budget" for this
//! codebase, and re-deriving a different one here would be a second design
//! to keep in sync with the first for no benefit. The *tests* are not
//! copied; they are written fresh against this module's own
//! [`ImageResidencyCache`] specialisation so the byte-accounting here is
//! independently checked against real [`Image`] sizes rather than trusted
//! by analogy.
//!
//! # Why linear-scan recency rather than an intrusive list
//!
//! Same trade `vieww-paint`'s version documents: the cache sizes this exists
//! for (a screen's worth of thumbnails or atlas source images, not an
//! unbounded content feed) make an O(n) scan-and-move over a few dozen or
//! few hundred entries cheaper in practice than the pointer-chasing and
//! unsafe code an intrusive doubly-linked list needs, and correct-by-
//! construction is worth more than a marginal win on a cache whose entire
//! job is being an optimisation nobody can see if it is subtly wrong.

use std::collections::HashMap;
use std::hash::Hash;

use vieww_foundation::Image;

/// Hit/miss/eviction counters. Exact and monotonic — nothing here is
/// sampled or approximated, so a caller can assert on it directly.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResidencyStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

/// A generic byte(-or-whatever-cost)-budgeted LRU cache.
///
/// See the module docs for why this exists as a small standalone type here
/// rather than reusing `vieww-paint`'s cache of the same shape.
#[derive(Debug)]
pub struct ResidencyCache<K, V> {
    entries: HashMap<K, V>,
    cost: HashMap<K, usize>,
    /// Least-recently-used first, most-recently-used last.
    order: Vec<K>,
    total_cost: usize,
    budget: usize,
    stats: ResidencyStats,
}

impl<K: Eq + Hash + Clone, V> ResidencyCache<K, V> {
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            cost: HashMap::new(),
            order: Vec::new(),
            total_cost: 0,
            budget,
            stats: ResidencyStats::default(),
        }
    }

    /// The value for `key` — resident already (a hit; `derive` and
    /// `cost_of` go unused) or derived and inserted now (a miss, possibly
    /// evicting other entries to stay under budget).
    pub fn get_or_insert_with(
        &mut self,
        key: K,
        derive: impl FnOnce() -> V,
        cost_of: impl FnOnce(&V) -> usize,
    ) -> &V {
        if self.entries.contains_key(&key) {
            self.touch(&key);
            self.stats.hits += 1;
        } else {
            let value = derive();
            let cost = cost_of(&value);
            self.entries.insert(key.clone(), value);
            self.cost.insert(key.clone(), cost);
            self.total_cost += cost;
            self.order.push(key.clone());
            self.stats.misses += 1;
            self.evict_to_budget();
        }
        self.entries.get(&key).expect("just touched or inserted")
    }

    /// The value for `key` if already resident, without deriving it and
    /// without disturbing recency order — for a caller that wants to peek
    /// without counting as a use.
    #[must_use]
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    /// Move `key` to the most-recently-used end of the eviction order.
    fn touch(&mut self, key: &K) {
        if let Some(index) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(index);
            self.order.push(key);
        }
    }

    /// Evict least-recently-used entries while over budget, always leaving
    /// at least one entry resident — the budget is a target, not a hard
    /// cap, so a single request larger than the whole budget is still kept
    /// rather than making every other access pay to evict everything for
    /// something that will not fit anyway.
    fn evict_to_budget(&mut self) {
        while self.total_cost > self.budget && self.order.len() > 1 {
            let victim = self.order.remove(0);
            let cost = self.cost.remove(&victim).unwrap_or(0);
            self.entries.remove(&victim);
            self.total_cost -= cost;
            self.stats.evictions += 1;
        }
    }

    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn current_cost(&self) -> usize {
        self.total_cost
    }

    #[must_use]
    pub fn stats(&self) -> ResidencyStats {
        self.stats
    }

    /// Forget everything and reset cost accounting. Stats are left alone —
    /// they describe this cache's history, which a `clear` does not erase.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cost.clear();
        self.order.clear();
        self.total_cost = 0;
    }
}

/// [`ResidencyCache`] specialised to decoded [`Image`]s, costed by their
/// exact pixel-buffer byte size (`width * height * 4` — [`Image`] is always
/// tightly-packed RGBA8, so this is not an estimate).
#[derive(Debug)]
pub struct ImageResidencyCache<K> {
    inner: ResidencyCache<K, Image>,
}

impl<K: Eq + Hash + Clone> ImageResidencyCache<K> {
    #[must_use]
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            inner: ResidencyCache::new(budget_bytes),
        }
    }

    /// The image for `key`, decoding (or otherwise producing) it via
    /// `decode` on first ask.
    pub fn get_or_decode_with(&mut self, key: K, decode: impl FnOnce() -> Image) -> &Image {
        self.inner
            .get_or_insert_with(key, decode, |image| image.pixels().len())
    }

    #[must_use]
    pub fn peek(&self, key: &K) -> Option<&Image> {
        self.inner.peek(key)
    }

    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        self.inner.contains(key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    #[must_use]
    pub fn current_bytes(&self) -> usize {
        self.inner.current_cost()
    }

    #[must_use]
    pub fn stats(&self) -> ResidencyStats {
        self.inner.stats()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_hit_does_not_call_the_deriver_again_and_stats_are_exact() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(1000);
        let calls = Cell::new(0);
        cache.get_or_insert_with(
            1,
            || {
                calls.set(calls.get() + 1);
                100
            },
            |_| 1,
        );
        cache.get_or_insert_with(
            1,
            || {
                calls.set(calls.get() + 1);
                100
            },
            |_| 1,
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(
            cache.stats(),
            ResidencyStats {
                hits: 1,
                misses: 1,
                evictions: 0
            }
        );
    }

    #[test]
    fn eviction_removes_the_least_recently_used_entry_first() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(20);
        cache.get_or_insert_with(1, || 1, |_| 10);
        cache.get_or_insert_with(2, || 2, |_| 10);
        // Touch 1 so 2 becomes the least-recently-used of the two.
        cache.get_or_insert_with(1, || panic!("must be a hit"), |_| 10);
        cache.get_or_insert_with(3, || 3, |_| 10); // Pushes total to 30 > 20: evicts the LRU.

        assert!(cache.contains(&1), "recently touched, must survive");
        assert!(!cache.contains(&2), "least recently used, must be evicted");
        assert!(cache.contains(&3), "just inserted, must survive");
        assert_eq!(cache.stats().evictions, 1);
    }

    #[test]
    fn a_single_entry_larger_than_the_budget_is_kept_anyway() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(5);
        cache.get_or_insert_with(1, || 1, |_| 1000);
        assert!(cache.contains(&1), "budget is a target, not a hard cap");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.stats().evictions, 0);
    }

    /// The `ImageResidencyCache` specialisation, with byte costs computed
    /// exactly by hand (width * height * 4, straight RGBA8):
    ///
    /// - "a": 4x4  -> 64 bytes
    /// - "b": 4x4  -> 64 bytes
    /// - "c": 2x2  -> 16 bytes
    ///
    /// Budget 100 bytes: "a" then "b" totals 128 > 100, evicting "a"
    /// (least-recently-used) down to 64. Then "c" totals 80 <= 100, no
    /// eviction needed.
    #[test]
    fn image_cache_evicts_by_exact_byte_cost() {
        let mut cache: ImageResidencyCache<&'static str> = ImageResidencyCache::new(100);
        let image_4x4 = || Image::from_rgba8(vec![0u8; 4 * 4 * 4], 4, 4);
        let image_2x2 = || Image::from_rgba8(vec![0u8; 2 * 2 * 4], 2, 2);

        cache.get_or_decode_with("a", image_4x4);
        assert_eq!(cache.current_bytes(), 64);

        cache.get_or_decode_with("b", image_4x4);
        assert_eq!(
            cache.current_bytes(),
            64,
            "adding b (64) would total 128 > 100, so a (LRU) is evicted, leaving just b's 64"
        );
        assert!(!cache.contains(&"a"));
        assert!(cache.contains(&"b"));
        assert_eq!(
            cache.stats(),
            ResidencyStats {
                hits: 0,
                misses: 2,
                evictions: 1
            }
        );

        cache.get_or_decode_with("c", image_2x2);
        assert_eq!(
            cache.current_bytes(),
            80,
            "64 (b) + 16 (c) = 80 <= 100, nothing evicted"
        );
        assert!(cache.contains(&"b"));
        assert!(cache.contains(&"c"));
        assert_eq!(
            cache.stats(),
            ResidencyStats {
                hits: 0,
                misses: 3,
                evictions: 1
            }
        );
    }

    #[test]
    fn eviction_then_reaccess_rederives_the_same_logical_image() {
        let mut cache: ImageResidencyCache<u32> = ImageResidencyCache::new(20);
        let calls = Cell::new(0);
        let make = |value: u8, calls: &Cell<u32>| {
            calls.set(calls.get() + 1);
            Image::from_rgba8(vec![value; 4], 1, 1) // 4 bytes each
        };

        let first = cache.get_or_decode_with(1, || make(7, &calls)).clone();
        assert_eq!(first.pixels(), &[7, 7, 7, 7]);

        // Fill past the budget with unrelated entries to force eviction of key 1.
        for key in 100..110u32 {
            cache.get_or_decode_with(key, || make(0, &calls));
        }
        assert!(
            !cache.contains(&1),
            "budget of 20 bytes / 4 bytes each must have evicted key 1 by now"
        );

        let recovered = cache.get_or_decode_with(1, || make(7, &calls));
        assert_eq!(
            recovered.pixels(),
            &[7, 7, 7, 7],
            "re-derivation must produce the same logical image"
        );
    }

    #[test]
    fn clearing_frees_everything_but_keeps_stats() {
        let mut cache: ImageResidencyCache<&'static str> = ImageResidencyCache::new(1000);
        cache.get_or_decode_with("a", || Image::from_rgba8(vec![0; 16], 2, 2));
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.current_bytes(), 0);
        assert_eq!(
            cache.stats().misses,
            1,
            "stats describe history, clear does not erase it"
        );
    }

    #[test]
    fn peek_does_not_derive_or_change_stats() {
        let cache: ImageResidencyCache<&'static str> = ImageResidencyCache::new(1000);
        assert!(cache.peek(&"missing").is_none());
        assert_eq!(cache.stats(), ResidencyStats::default());
    }
}
