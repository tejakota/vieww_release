use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Explicit identity for a widget across rebuilds.
///
/// Reconciliation (Phase 2) reuses an element for a new widget at the same tree
/// position iff the widget's `TypeId` **and** its key both match. Without a key
/// that comparison is `None == None`, which is why position alone decides
/// identity for unkeyed widgets — and why reordering a list of unkeyed children
/// silently reassigns their state.
///
/// See `docs/DESIGN.md` §2.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Key {
    /// Identity from a string, e.g. a record's slug.
    Str(String),
    /// Identity from an integer, e.g. a database id.
    Int(i64),
    /// Identity from a process-unique counter — never equal to any other key,
    /// including a clone of itself made before this one.
    ///
    /// Use it to *force* a remount: a widget carrying a fresh `Unique` key can
    /// never match the previous frame's element, so state is discarded.
    Unique(u64),
}

impl Key {
    /// A string key.
    #[must_use]
    pub fn str(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    /// An integer key.
    #[must_use]
    pub const fn int(value: i64) -> Self {
        Self::Int(value)
    }

    /// A key that matches nothing but a clone of itself.
    #[must_use]
    pub fn unique() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self::Unique(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<i64> for Key {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Str(s) => write!(f, "<{s}>"),
            Self::Int(i) => write!(f, "<#{i}>"),
            Self::Unique(u) => write!(f, "<unique:{u}>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_valued_keys_of_the_same_flavour_match() {
        assert_eq!(Key::str("row"), Key::str("row"));
        assert_eq!(Key::int(7), Key::int(7));
    }

    #[test]
    fn keys_of_different_flavours_never_match() {
        assert_ne!(Key::str("7"), Key::int(7));
    }

    #[test]
    fn unique_keys_never_collide_but_clones_do_match() {
        let key = Key::unique();
        assert_ne!(key, Key::unique());
        assert_eq!(key, key.clone());
    }
}
