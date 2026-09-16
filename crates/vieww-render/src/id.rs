use std::fmt;

/// A handle to a render object in a [`RenderTree`](crate::RenderTree).
///
/// Generational for the same reason [`ElementId`](vieww_element::ElementId) is:
/// slots are reused when a subtree is torn down, so a stale handle must compare
/// unequal rather than silently address whatever landed there next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RenderId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl RenderId {
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
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

impl fmt::Display for RenderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}v{}", self.index, self.generation)
    }
}
