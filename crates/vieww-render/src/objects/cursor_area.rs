use vieww_foundation::{Constraints, Cursor, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Declares the pointer shape for everything inside it.
///
/// Layout, paint, semantics and input are its child's, unchanged. The only
/// thing it adds is an answer to [`RenderObject::cursor`], which
/// [`FrameDriver::cursor_at`](crate::FrameDriver::cursor_at) asks of whatever is
/// under the pointer.
///
/// # Why this did not exist
///
/// [`Cursor`] has had eleven shapes and a platform mapping since the pointer
/// module was written, and `cursor_at` has been wired to the window since the
/// winit backend was. But the only object in the workspace that ever *answered*
/// was `RenderEditableText`, with an I-beam. Every other shape in the enum —
/// [`ResizeColumn`](Cursor::ResizeColumn) above all — described something no
/// widget could ask for, so a draggable split had the same arrow over it as the
/// wall beside it.
///
/// The studio's pane divider is the case that produced this: a six-point gutter
/// that resizes two panes, paints nothing at rest, and gave the pointer no hint
/// that it was there at all. See `apps/viewwstudio/src/ui/divider.rs`.
///
/// # It is offered where the child is drawn, and no wider
///
/// A `hit_bounds` expansion the way `RenderGestureDetector` reaches its touch
/// target would do nothing here: `RenderTree::hit_test` runs the expanded pass
/// only as a *fallback* for a point that hit nothing, precisely so an invisible
/// widened target cannot claim a neighbour's pixels. So the shape covers the
/// child's own box.
///
/// That is not a shortfall, it is the honest answer. Measured on the studio's
/// divider, a press and drag resizes across exactly the six points of drawn
/// gutter — its widened grab is a fallback both neighbouring cards refuse to
/// yield — so a cursor offered any wider would promise a drag that does not
/// start. **The shape and the gesture cover the same pixels**, which is the
/// property worth having; a knob for offering it wider was written, measured to
/// do nothing, and removed rather than left as a dial that lies.
///
/// # Innermost wins
///
/// `cursor_at` walks the hit path from the target outwards and stops at the
/// first object with an opinion, so a text field inside a `CursorArea` still
/// gets its I-beam. Wrapping a wide region and letting the controls inside it
/// override is the intended way to use this.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderCursorArea {
    /// The shape the pointer takes over this subtree.
    pub cursor: Cursor,
}

impl RenderCursorArea {
    #[must_use]
    pub const fn new(cursor: Cursor) -> Self {
        Self { cursor }
    }
}

impl RenderObject for RenderCursorArea {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    /// Hittable, unlike a plain layout box.
    ///
    /// The pointer shape is read off the hit path, so an area that declines to
    /// be hit is an area whose cursor is never asked for. This is the one place
    /// where being a target and drawing nothing is correct: it takes no
    /// gestures, so nothing is stolen from a control behind it — but a
    /// `CursorArea` over empty space still has to be able to answer.
    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        true
    }

    fn cursor(&self, _local: Offset) -> Option<Cursor> {
        Some(self.cursor)
    }

    crate::baseline::pass_through_baseline!();

    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        ctx.only_child_intrinsic(query)
    }

    fn debug_name(&self) -> &'static str {
        "RenderCursorArea"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_answers_with_the_shape_it_was_given() {
        assert_eq!(
            RenderCursorArea::new(Cursor::ResizeColumn).cursor(Offset::ZERO),
            Some(Cursor::ResizeColumn)
        );
    }
}
