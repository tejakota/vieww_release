use vieww_foundation::{Constraints, Offset, Size};
use vieww_widget::StackPosition;

use crate::{LayoutCtx, RenderObject};

/// Carries a stack position for its parent to read.
///
/// Layout-transparent, exactly as [`RenderFlexible`](crate::RenderFlexible) is:
/// constraints pass straight through and the size comes straight back, so
/// wrapping a child in one cannot move a pixel by itself. The *parent* changes
/// behaviour — [`RenderStack`](crate::RenderStack) asks each child for
/// [`RenderObject::stack_position`], leaves the ones that answer out of its own
/// sizing, and places them against its edges afterwards.
///
/// Outside a stack it does nothing, which is the right answer: a position
/// nobody is placing against is not an error, it is unused.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPositioned {
    pub position: StackPosition,
}

impl RenderPositioned {
    #[must_use]
    pub const fn new(position: StackPosition) -> Self {
        Self { position }
    }
}

impl RenderObject for RenderPositioned {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Whatever arrives is handed on unchanged. Under a stack these are
        // already the constraints it derived from this object's own edges, so
        // the child is sized by the position without this object having to know
        // what the position was.
        match ctx.children().first().copied() {
            Some(child) => {
                let size = ctx.layout_child(child, constraints);
                ctx.place_child(child, Offset::ZERO);
                size
            }
            // A position with nothing under it still has to satisfy its
            // constraints, and under a tight one that is not zero.
            None => constraints.constrain(Size::ZERO),
        }
    }

    fn stack_position(&self) -> Option<StackPosition> {
        // `None` when nothing is pinned, so "is this positioned?" is answered in
        // one place and the stack only has to check the `Option`.
        self.position.is_positioned().then_some(self.position)
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    /// The position is the parent's to use, not this object's.
    ///
    /// This object lays out transparently — it cannot move anything — so
    /// marking only itself when the position changes dirties nothing that
    /// matters. See [`RenderObject::parent_reads_configuration`].
    fn parent_reads_configuration(&self) -> bool {
        true
    }

    fn debug_name(&self) -> &'static str {
        "RenderPositioned"
    }
}
