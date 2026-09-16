use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Keeps its child mounted and does nothing else with it.
///
/// The child is not laid out, not painted, not hit tested and not announced to
/// a screen reader — but it is still in the element tree, so its state, its
/// scroll position and its animations survive. Coming back onstage is a rebuild,
/// not a rebirth.
///
/// # Why mounted rather than removed
///
/// Removing the subtree is what a naive navigator does, and it is why so many
/// applications forget where a list was scrolled to when you go back. The whole
/// point of keeping every route on the stack is that going back returns you to
/// the screen you left; painting the ones nobody can see is the cost that buys,
/// and this is the object that stops paying it.
///
/// # Why the tree honours it rather than this object
///
/// Layout is this object's own business — it simply declines to lay the child
/// out. Painting, hit testing and semantics are not: the tree walks children
/// itself, for repaint boundaries and paint order and reading order, so it is
/// the tree that has to be told to stop. See [`RenderObject::skips_children`].
///
/// # Size
///
/// Whatever its constraints force, and [`Size::ZERO`] when they allow it. It
/// cannot always be zero — under a `StackFit::Expand` it is handed a tight
/// full-surface box — but it draws nothing at any size, so the difference is
/// only ever a layout one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOffstage {
    /// `true` while the child is hidden. Not a separate object, so that going
    /// on and off stage is a property change rather than a reparent — a
    /// reparent would unmount the very state this exists to preserve.
    pub offstage: bool,
}

impl RenderOffstage {
    #[must_use]
    pub const fn new(offstage: bool) -> Self {
        Self { offstage }
    }
}

impl RenderObject for RenderOffstage {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        if self.offstage {
            // Not laid out at all, which is the saving. The child keeps the
            // geometry it had; nothing reads it while it is hidden, and coming
            // back onstage lays it out against whatever the constraints are then.
            return constraints.smallest();
        }
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn skips_children(&self) -> bool {
        self.offstage
    }

    /// Zero while offstage, because that is the size it takes.
    ///
    /// Not the child's answer: an offstage subtree is not laid out at all, and
    /// reporting what it *would* want would make an `IntrinsicHeight` above it
    /// reserve room for something that is not there.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        if self.offstage {
            return Some(0.0);
        }
        ctx.only_child_intrinsic(query)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // Going on or off stage changes everything about the geometry beneath,
        // so this is the one property here that must relayout.
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderOffstage"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;

    use super::*;
    use crate::{RenderColoredBox, RenderTree};

    /// Tight constraints, deliberately: under loose ones a childless box sizes
    /// to zero and the onstage case would look exactly like the offstage one.
    fn tree_with(offstage: bool) -> (RenderTree, crate::RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderOffstage::new(offstage)));
        let child = tree.insert(Some(root), Box::new(RenderColoredBox::new(Color::RED)));
        let _ = tree.layout_root(Constraints::tight(Size::new(100.0, 100.0)));
        (tree, child)
    }

    #[test]
    fn an_offstage_child_is_not_laid_out() {
        let (tree, child) = tree_with(true);
        assert_eq!(
            tree.size(child),
            Size::ZERO,
            "a hidden child costs no layout"
        );
    }

    #[test]
    fn an_onstage_child_is_laid_out_normally() {
        let (tree, child) = tree_with(false);
        assert_eq!(tree.size(child), Size::new(100.0, 100.0));
    }

    #[test]
    fn a_hidden_subtree_takes_no_taps() {
        let (tree, _) = tree_with(true);
        assert!(
            tree.hit_test(Offset::new(10.0, 10.0)).target().is_none(),
            "a route nobody can see must not be pressable"
        );
    }
}
