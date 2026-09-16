//! Deciding whether a new library's state can be adopted or has to be dropped.
//!
//! # The hazard this exists for
//!
//! `docs/HOT-RELOAD.md` §3 names it as one of three costs, and it is the only
//! one that is *unsound* rather than merely wasteful:
//!
//! > If the application edits the fields of something held in a `Signal` or an
//! > `ElementState`, the old allocation has the old layout and the new code
//! > reads it with the new one. That is undefined behaviour, not a glitch.
//!
//! An element that survives a reload keeps its `Box<dyn ElementState>` — the
//! allocation the *old* library made. The new library's code then reads it. If
//! the struct changed shape in between, every field access is at the wrong
//! offset.
//!
//! # What is detected, and what is not
//!
//! A fingerprint of each state type's **name, size and alignment**. Adding,
//! removing or retyping a field almost always moves one of the three.
//!
//! **A same-size reorder does not.** Swapping two `u32` fields keeps the name,
//! the size and the alignment, and this will happily adopt the old allocation
//! into code that reads the fields the other way round. That is a real hole and
//! it is stated rather than papered over — the mitigation is that
//! [`Verdict::Restart`] is cheap and always available, so an application that
//! sees nonsense after a reload should restart before debugging it.
//!
//! The other direction has the same limit: editing a state class's
//! fields forces a hot *restart* there, full stop. This is more permissive and
//! correspondingly sharper.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// A summary of every state type a guest library owns.
///
/// Compared across two builds to decide whether element state can be carried
/// over. Built by the `guest!` macro, which is what makes it hard to forget a
/// type — it is written once next to the root function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint(u64);

impl Fingerprint {
    /// A fingerprint of nothing, for a guest that holds no state at all.
    ///
    /// Distinct from any real one, so "declared no state" and "declared one
    /// type that happens to hash to zero" cannot be confused.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Build one from `(name, size, align)` triples, in declaration order.
    ///
    /// **Order matters and that is deliberate.** Reordering the list is a
    /// change to what the guest declared, and treating it as equivalent would
    /// mean a fingerprint that ignores the one thing it is cheap to notice.
    #[must_use]
    pub fn of(types: &[(&str, usize, usize)]) -> Self {
        if types.is_empty() {
            return Self::empty();
        }
        let mut hasher = DefaultHasher::new();
        for (name, size, align) in types {
            name.hash(&mut hasher);
            size.hash(&mut hasher);
            align.hash(&mut hasher);
        }
        // Zero is reserved for `empty`, so a real fingerprint that hashes to it
        // is nudged rather than reported as "no state".
        Self(hasher.finish() | 1)
    }

    /// The raw value, for crossing the library boundary as a plain `u64`.
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0
    }

    /// Rebuild one from the other side of that boundary.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }
}

/// What the host should do with a newly built library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Swap the root and keep every element's state.
    Reload,
    /// Tear the tree down and mount the new root fresh.
    ///
    /// Not a failure — the application is still running and the developer still
    /// did not restart the process. It is the honest answer when the shape of
    /// the state changed, and it is what stands between a reload and undefined
    /// behaviour.
    Restart,
}

impl Verdict {
    /// Compare what the running guest declared against what the new one does.
    #[must_use]
    pub fn between(running: Fingerprint, incoming: Fingerprint) -> Self {
        if running == incoming {
            Self::Reload
        } else {
            Self::Restart
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER: (&str, usize, usize) = ("app::Counter", 8, 4);

    #[test]
    fn an_unchanged_guest_keeps_its_state() {
        let before = Fingerprint::of(&[COUNTER]);
        let after = Fingerprint::of(&[COUNTER]);
        assert_eq!(Verdict::between(before, after), Verdict::Reload);
    }

    #[test]
    fn a_state_type_that_grew_forces_a_restart() {
        // The case this exists for: the old allocation is 8 bytes and the new
        // code reads 12. Adopting it is undefined behaviour rather than a
        // glitch, so the tree goes instead.
        let before = Fingerprint::of(&[COUNTER]);
        let after = Fingerprint::of(&[("app::Counter", 12, 4)]);
        assert_eq!(Verdict::between(before, after), Verdict::Restart);
    }

    #[test]
    fn a_realignment_forces_a_restart_even_at_the_same_size() {
        let before = Fingerprint::of(&[COUNTER]);
        let after = Fingerprint::of(&[("app::Counter", 8, 8)]);
        assert_eq!(Verdict::between(before, after), Verdict::Restart);
    }

    #[test]
    fn adding_a_state_type_forces_a_restart() {
        let before = Fingerprint::of(&[COUNTER]);
        let after = Fingerprint::of(&[COUNTER, ("app::Scroll", 4, 4)]);
        assert_eq!(Verdict::between(before, after), Verdict::Restart);
    }

    #[test]
    fn reordering_the_declaration_is_a_change() {
        // Cheap to notice, so noticed. A fingerprint that ignored order would be
        // silently weaker for no saving.
        let scroll = ("app::Scroll", 4, 4);
        assert_ne!(
            Fingerprint::of(&[COUNTER, scroll]),
            Fingerprint::of(&[scroll, COUNTER])
        );
    }

    #[test]
    fn no_state_at_all_is_its_own_answer() {
        assert_eq!(Fingerprint::of(&[]), Fingerprint::empty());
        assert_eq!(
            Verdict::between(Fingerprint::empty(), Fingerprint::empty()),
            Verdict::Reload,
            "a guest with no state can always be reloaded"
        );
        assert_ne!(
            Fingerprint::of(&[COUNTER]),
            Fingerprint::empty(),
            "declaring a type must never collide with declaring none"
        );
    }

    #[test]
    fn a_fingerprint_survives_the_trip_across_the_library_boundary() {
        // It crosses as a plain `u64` because that is the only thing about this
        // ABI that is guaranteed to mean the same on both sides.
        let original = Fingerprint::of(&[COUNTER]);
        assert_eq!(Fingerprint::from_bits(original.to_bits()), original);
    }

    #[test]
    fn a_same_size_reorder_is_the_documented_hole() {
        // Not a bug in this type — a limit of what size and alignment can see.
        // Asserted so that nobody later reads the docs, disbelieves them, and
        // has to rediscover it on a corrupted tree.
        let before = Fingerprint::of(&[("app::Counter", 8, 4)]);
        let after = Fingerprint::of(&[("app::Counter", 8, 4)]);
        assert_eq!(
            Verdict::between(before, after),
            Verdict::Reload,
            "two different field orders at one size are indistinguishable here"
        );
    }
}
