use std::fmt;

/// A handle to an element in an [`ElementTree`](crate::ElementTree).
///
/// Elements live in an arena rather than owning each other, because a tree
/// where children know their parent is a cycle, and a cycle of `Rc` leaks. The
/// arena also gives O(1) access from a signal subscription straight to the
/// element it should pending, without walking down from the root.
///
/// The generation is what makes the handle safe to hold across frames: slots
/// are reused after an unmount, so a stale id would otherwise silently address
/// whatever element landed in that slot next. Comparing generations turns that
/// into a clean "this element is gone".
/// # Why the tree is part of the identity
///
/// Because two trees can share one [`Runtime`](crate::Runtime), and the runtime
/// keys its pending set by this type. `index` is an **arena slot within one
/// tree**, so without a tree tag the first element of one tree and the first
/// element of another are the same id: drawing one window would discard the
/// other's pending marks, and `is_alive` would answer true for an element in a
/// different tree entirely.
///
/// That was a real defect, found the first time anything shared a runtime —
/// which was the first two-window test, long after `ElementTree::with_runtime`
/// went public.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId {
    pub(crate) tree: u32,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl ElementId {
    pub(crate) const fn new(tree: u32, index: u32, generation: u32) -> Self {
        Self {
            tree,
            index,
            generation,
        }
    }

    /// This id as an opaque handle, for a layer that must not name
    /// [`ElementId`].
    ///
    /// `vieww-widget` sits *below* this crate and records which elements read
    /// an inherited value; it cannot hold an `ElementId`, so it holds one of
    /// these. The tree is left out on purpose — a reader and the value it reads
    /// are always in the same tree, and [`from_handle`](Self::from_handle) puts
    /// it back.
    #[must_use]
    pub const fn to_handle(self) -> u64 {
        ((self.index as u64) << 32) | (self.generation as u64)
    }

    /// Rebuild an id from [`to_handle`](Self::to_handle), in `tree`.
    ///
    /// A handle for an element that has since been unmounted decodes to an id
    /// whose generation no longer matches its slot, which every accessor
    /// already treats as dead. That is the whole safety argument: a stale
    /// handle cannot reach a live element.
    #[must_use]
    pub const fn from_handle(tree: u32, handle: u64) -> Self {
        Self::new(tree, (handle >> 32) as u32, handle as u32)
    }

    /// Which tree this element lives in.
    ///
    /// Trees are numbered from a counter, so ids from different trees never
    /// collide even though their arena slots do.
    #[must_use]
    pub const fn tree(self) -> u32 {
        self.tree
    }

    /// The arena slot this id addresses.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// How many times that slot had been reused when this id was issued.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Tree 0 prints as it always did, so a single-tree dump — which is
        // every dump before multi-window — reads unchanged. A second tree
        // announces itself rather than being silently ambiguous.
        if self.tree == 0 {
            write!(f, "#{}v{}", self.index, self.generation)
        } else {
            write!(f, "t{}#{}v{}", self.tree, self.index, self.generation)
        }
    }
}
