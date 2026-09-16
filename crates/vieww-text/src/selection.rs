//! Text selection: ranges, anchors, and the semantics of shift-click.
//!
//! # Why selection is its own module
//!
//! Selecting text looks simple and is not. The state is a pair of offsets
//! (anchor and focus), but the *semantics* — what a drag does, what
//! shift-click does, what double-click does — form a small state machine
//! that every text surface needs and that would otherwise be duplicated
//! with bugs in each copy.

/// A selection between two character offsets.
///
/// The `anchor` is where the selection started; the `focus` is where it
/// ended. They may be in either order — a selection made right-to-left has
/// `anchor > focus` — and code that assumes otherwise breaks exactly the
/// drag-backwards case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextSelection {
    pub anchor: usize,
    pub focus: usize,
}

impl TextSelection {
    /// An empty (collapsed) selection at `position` — a cursor.
    #[must_use]
    pub const fn cursor(position: usize) -> Self {
        Self {
            anchor: position,
            focus: position,
        }
    }

    /// `true` if anchor == focus (a cursor, not a range).
    #[must_use]
    pub const fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selection as an ordered range, regardless of direction.
    #[must_use]
    pub const fn range(&self) -> std::ops::Range<usize> {
        let (start, end) = self.ordered();
        start..end
    }

    /// The (start, end) pair in ascending order.
    #[must_use]
    pub const fn ordered(&self) -> (usize, usize) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    /// The earlier of anchor and focus — where a backwards selection began.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.ordered().0
    }

    /// The later of anchor and focus.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.ordered().1
    }

    /// Extend the selection to `position`, keeping the anchor.
    ///
    /// This is the drag behaviour: the anchor stays where the mouse went
    /// down, the focus follows the mouse.
    #[must_use]
    pub const fn extend_to(&self, position: usize) -> Self {
        Self {
            anchor: self.anchor,
            focus: position,
        }
    }

    /// Move both anchor and focus — a new selection from scratch.
    #[must_use]
    pub const fn collapse_to(&self, position: usize) -> Self {
        Self::cursor(position)
    }
}

/// What a click should select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Position the cursor.
    Position,
    /// Select the word at the position.
    Word,
    /// Select the paragraph (line) at the position.
    Paragraph,
    /// Select everything.
    Document,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_is_collapsed() {
        let sel = TextSelection::cursor(5);
        assert!(sel.is_collapsed());
        assert_eq!(sel.range(), 5..5);
    }

    #[test]
    fn a_backwards_selection_is_ordered_correctly() {
        // Dragged from 10 back to 3.
        let sel = TextSelection {
            anchor: 10,
            focus: 3,
        };
        assert_eq!(sel.start(), 3);
        assert_eq!(sel.end(), 10);
        assert_eq!(sel.range(), 3..10);
        assert!(!sel.is_collapsed());
    }

    #[test]
    fn extend_keeps_the_anchor() {
        let sel = TextSelection::cursor(5);
        let extended = sel.extend_to(10);
        assert_eq!(extended.anchor, 5);
        assert_eq!(extended.focus, 10);
    }

    #[test]
    fn extend_backward_past_the_anchor() {
        let sel = TextSelection::cursor(5);
        let extended = sel.extend_to(2);
        // anchor stays, focus went before it.
        assert_eq!(extended.anchor, 5);
        assert_eq!(extended.focus, 2);
        assert_eq!(extended.range(), 2..5);
    }
}
