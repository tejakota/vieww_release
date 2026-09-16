//! A budget-enforced, LRU-evictable backing store — "Renderer v2" pillar E
//! (`docs/RENDERER-V2-NOTES.md`): split a cache's *logical identity* (a
//! content key that always means the same thing) from its *physical
//! residency* (whether the derived value is actually sitting in memory right
//! now).
//!
//! [`ResidencyCache::get_or_insert_with`] is the one entry point: given a
//! key and a way to derive the value if it isn't resident, it returns the
//! value either way — a caller cannot tell "was already there" from "just
//! got evicted and was re-derived" except by speed, which is exactly the
//! property this pillar asks for ("a mesh can stay logically retained while
//! its physical allocation is evicted"). `native/glyph.rs`'s `GlyphCache` is
//! the concrete application: a glyph's outline is cheap, deterministic,
//! read-only data re-extracted from the font's own bytes, so evicting one
//! under memory pressure changes nothing about correctness — only how long
//! the next draw of that glyph takes.
//!
//! # Why a `Vec`-ordered LRU rather than an intrusive linked list
//!
//! The textbook O(1) LRU is a hash map plus an intrusive doubly-linked list.
//! This is a `HashMap` plus a `Vec<K>` recency ordering, touched with an
//! O(n) linear scan-and-move on every hit. That trade is deliberate: the
//! cache sizes this crate actually has (the distinct glyphs on one page of
//! text, not a browser's worth of DOM nodes) make O(n) over a few hundred
//! entries cheaper in practice than the pointer-chasing and unsafe code an
//! intrusive list needs, and correct-by-construction beats
//! correct-if-the-unsafe-invariants-hold for a cache whose entire job is
//! being an optimization no one can see if it's wrong.
//!
//! # Why the budget is a target, not a hard cap
//!
//! A single entry larger than the whole budget is still kept — eviction
//! never empties the cache down to nothing to make room for the thing that
//! was just asked for, matching `native/pool.rs`'s `TargetPool` accepting
//! the same softness for the same reason: a pathological one-off should not
//! make every *other* access pay for an allocator dance that only benefits
//! it once.

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResidencyStats {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

#[derive(Debug)]
pub(crate) struct ResidencyCache<K, V> {
    entries: HashMap<K, V>,
    cost: HashMap<K, usize>,
    /// Least-recently-used first, most-recently-used last.
    order: Vec<K>,
    total_cost: usize,
    budget: usize,
    stats: ResidencyStats,
}

impl<K: Eq + Hash + Clone, V> ResidencyCache<K, V> {
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            cost: HashMap::new(),
            order: Vec::new(),
            total_cost: 0,
            budget,
            stats: ResidencyStats::default(),
        }
    }

    /// The value for `key` — resident already (a hit, `derive` and `cost_of`
    /// unused) or derived and inserted now (a miss, possibly evicting other
    /// entries to stay under budget).
    pub(crate) fn get_or_insert_with(
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

    /// Move `key` to the most-recently-used end of the eviction order.
    fn touch(&mut self, key: &K) {
        if let Some(index) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(index);
            self.order.push(key);
        }
    }

    /// Evict least-recently-used entries while over budget, always leaving
    /// at least one entry — see the module docs' "target, not a hard cap".
    fn evict_to_budget(&mut self) {
        while self.total_cost > self.budget && self.order.len() > 1 {
            let victim = self.order.remove(0);
            let cost = self.cost.remove(&victim).unwrap_or(0);
            self.entries.remove(&victim);
            self.total_cost -= cost;
            self.stats.evictions += 1;
        }
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn current_cost(&self) -> usize {
        self.total_cost
    }

    pub(crate) fn stats(&self) -> ResidencyStats {
        self.stats
    }

    /// Not called by `native/glyph.rs` today — dropping a font's whole
    /// entry already drops its `ResidencyCache` with it — but a real part
    /// of this type's own contract (and exercised directly in this file's
    /// own tests), kept rather than trimmed to only what one caller needs.
    #[allow(dead_code)]
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.cost.clear();
        self.order.clear();
        self.total_cost = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_hit_does_not_call_the_deriver_again() {
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
    fn eviction_stays_under_budget() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(30);
        for key in 0..10u32 {
            cache.get_or_insert_with(key, || key, |_| 10);
        }
        assert!(
            cache.current_cost() <= 30,
            "budget of 30 with cost-10 entries must keep at most 3 resident"
        );
        assert_eq!(cache.len(), 3);
        assert!(cache.stats().evictions >= 7);
    }

    #[test]
    fn eviction_removes_the_least_recently_used_entry_first() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(20);
        cache.get_or_insert_with(1, || 1, |_| 10);
        cache.get_or_insert_with(2, || 2, |_| 10);
        // Touch 1 so 2 becomes the least-recently-used of the two.
        cache.get_or_insert_with(1, || panic!("must be a hit"), |_| 10);
        cache.get_or_insert_with(3, || 3, |_| 10); // Pushes total to 30 > 20: evicts the LRU entry.

        assert!(cache.contains(&1), "recently touched, must survive");
        assert!(!cache.contains(&2), "least recently used, must be evicted");
        assert!(cache.contains(&3), "just inserted, must survive");
    }

    #[test]
    fn a_single_entry_larger_than_the_budget_is_kept_anyway() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(5);
        cache.get_or_insert_with(1, || 1, |_| 1000);
        assert!(
            cache.contains(&1),
            "budget is a target, not a hard cap — see the module docs"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn eviction_then_reaccess_rederives_and_returns_correctly() {
        // The property that makes eviction safe at all: a miss after
        // eviction re-derives the exact same logical value, indistinguishable
        // from having stayed resident except for the deriver call count.
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(10);
        let calls = Cell::new(0);
        let derive_for = |key: u32| {
            let calls = &calls;
            move || {
                calls.set(calls.get() + 1);
                key * 100
            }
        };

        let a = *cache.get_or_insert_with(1, derive_for(1), |_| 10);
        assert_eq!(a, 100);
        cache.get_or_insert_with(2, derive_for(2), |_| 10); // Evicts key 1 (budget 10, cost 10 each).
        assert!(!cache.contains(&1));

        let a_again = *cache.get_or_insert_with(1, derive_for(1), |_| 10);
        assert_eq!(
            a_again, 100,
            "re-derivation after eviction must produce the same logical value"
        );
        assert_eq!(
            calls.get(),
            3,
            "key 1 was derived twice (once, evicted, then again) and key 2 once"
        );
    }

    #[test]
    fn clear_drops_everything_and_resets_cost() {
        let mut cache: ResidencyCache<u32, u32> = ResidencyCache::new(1000);
        cache.get_or_insert_with(1, || 1, |_| 50);
        cache.get_or_insert_with(2, || 2, |_| 50);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.current_cost(), 0);
        assert!(!cache.contains(&1));
    }
}
