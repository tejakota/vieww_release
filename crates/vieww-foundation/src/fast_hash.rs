//! A hasher for keys the framework already knows are small integers.
//!
//! # Why not the standard one
//!
//! `std`'s default is SipHash-1-3, chosen so a `HashMap` exposed to untrusted
//! input cannot be turned into a linked list by an attacker feeding it colliding
//! keys. That is the right default for a map holding request headers. It is the
//! wrong one for the maps this framework runs a frame through, which are keyed
//! by [`ElementId`](../vieww_element/struct.ElementId.html) and
//! [`RenderId`](../vieww_render/struct.RenderId.html) — a pair of `u32`s the
//! framework hands out itself, from a counter, that no input of any kind can
//! influence.
//!
//! The cost was measured before this module existed. A profile of one keystroke
//! in the studio — `apps/viewwstudio/examples/bench --only=typing` under
//! callgrind — put **12% of the whole frame** in `sip::Hasher::write` and
//! `BuildHasher::hash_one`, spread across the element tree's stateful set, the
//! render tree's pending-paint and boundary maps, and the layer bookkeeping.
//! Twelve per cent of every frame, to defend eight bytes of counter against an
//! adversary who would have to already be inside the process.
//!
//! # What this is
//!
//! FxHash — the multiply-and-rotate hash `rustc` uses on its own internal maps,
//! for the same reason. One multiply and one rotate per 8 bytes, no state
//! beyond a `u64`, and the compiler inlines all of it away for a key this size.
//!
//! # When *not* to use it
//!
//! Anything a user, a file, or a network can put keys into. `FastMap` has no
//! collision resistance whatsoever: an attacker who picks the keys can pick
//! collisions. Settings read from disk, LSP responses, anything parsed — those
//! keep the standard hasher, and the type alias is deliberately not a
//! drop-in-everywhere replacement so that choice stays visible at the
//! declaration.

use std::hash::{BuildHasherDefault, Hasher};

/// A `HashMap` keyed by something the framework generated itself.
pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// A `HashSet` of something the framework generated itself.
pub type FastSet<K> = std::collections::HashSet<K, BuildHasherDefault<FastHasher>>;

/// The multiplier. `rustc`'s, which is the fractional part of the golden ratio
/// scaled to 64 bits — chosen so that consecutive keys, which is exactly what a
/// counter produces, land far apart in the table.
const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// FxHash: one multiply and one rotate per word.
#[derive(Debug, Default, Clone, Copy)]
pub struct FastHasher {
    state: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        // Rotate before mixing so the high bits, which the multiply spreads
        // best, are the ones the table's mask actually looks at.
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.state
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        // The general case, for a key that is not one of the integer types
        // below — a tuple containing a `&str`, say. Eight bytes at a time, then
        // whatever is left, so a short tail is one more `add` rather than a
        // loop over single bytes.
        let mut rest = bytes;
        while rest.len() >= 8 {
            let (word, tail) = rest.split_at(8);
            self.add(u64::from_ne_bytes(word.try_into().unwrap_or([0; 8])));
            rest = tail;
        }
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_ne_bytes(last));
        }
    }

    // The integer paths, which are the ones that matter. `#[derive(Hash)]` on a
    // struct of two `u32`s calls `write_u32` twice and never reaches `write` at
    // all, so without these the specialised hasher would be doing the slow thing
    // on precisely the keys it exists for.
    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u16(&mut self, value: u16) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }
    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }
    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hash;

    fn hash_of(value: impl Hash) -> u64 {
        let mut hasher = FastHasher::default();
        value.hash(&mut hasher);
        hasher.finish()
    }

    /// The property a hash table needs and the only one this promises.
    #[test]
    fn equal_keys_hash_equal() {
        assert_eq!(hash_of((1u32, 2u32)), hash_of((1u32, 2u32)));
        assert_eq!(hash_of("element"), hash_of("element"));
    }

    /// The property it is *for*: consecutive ids, which is what a counter
    /// hands out, must not land in neighbouring buckets.
    #[test]
    fn consecutive_ids_are_spread_apart() {
        let ids: Vec<u64> = (0u32..64).map(|index| hash_of((0u32, index))).collect();
        // Low seven bits — a 128-bucket table's index. A hash that left the
        // counter's low bits alone would produce 64 distinct values here and
        // still pile every one of them into a run of consecutive buckets, which
        // is the failure this multiply exists to prevent.
        let mut buckets: Vec<u64> = ids.iter().map(|hash| hash & 0x7F).collect();
        buckets.sort_unstable();
        buckets.dedup();
        assert!(
            buckets.len() > 48,
            "64 consecutive ids collapsed into {} buckets of 128",
            buckets.len()
        );
    }

    /// Order matters: `(1, 2)` and `(2, 1)` are different keys.
    #[test]
    fn the_fields_are_not_commutative() {
        assert_ne!(hash_of((1u32, 2u32)), hash_of((2u32, 1u32)));
    }

    /// A tail shorter than a word still contributes.
    #[test]
    fn a_short_tail_changes_the_hash() {
        assert_ne!(hash_of("abcdefghi"), hash_of("abcdefghj"));
        assert_ne!(hash_of("abc"), hash_of("abd"));
    }

    #[test]
    fn the_map_alias_works_as_a_map() {
        let mut map: FastMap<(u32, u32), &str> = FastMap::default();
        for index in 0..1000u32 {
            map.insert((0, index), "x");
        }
        assert_eq!(map.len(), 1000);
        assert_eq!(map.get(&(0, 999)), Some(&"x"));
        assert_eq!(map.get(&(1, 999)), None);
    }
}
