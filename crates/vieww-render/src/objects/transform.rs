use vieww_foundation::{Constraints, Offset, Size, Transform};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Draws its child somewhere other than where it was laid out.
///
/// Layout is untouched: the child is measured and placed exactly as it would be
/// without this, and this object reports the child's size. Only the *drawing*
/// moves. That is the whole point — a transform is a paint-time property, so
/// changing it must not relayout anything, which is what makes it the right tool
/// for animating position.
///
/// # Why this is not padding
///
/// Phase 7's drag-to-dismiss moves a card by changing a `Padding`, because until
/// now there was nothing else. That works and is measurably the wrong shape: a
/// changed inset is a *layout* change, so every frame of the animation re-lays
/// out the row the card sits in and everything below it. A transform changes one
/// matrix and repaints; wrapped in a
/// [`RepaintBoundary`](crate::RenderRepaintBoundary) it does not even repaint,
/// because a layer carries its own transform and moving one is a composite.
///
/// # The origin is this object's, not the window's
///
/// A rotation turns about the object's top-left, and a scale grows from it. That
/// is what makes a transform composable — a widget can be moved by an ancestor
/// without its own transform meaning something different.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderTransform {
    pub transform: Transform,
}

impl RenderTransform {
    #[must_use]
    pub const fn new(transform: Transform) -> Self {
        Self { transform }
    }

    /// Shifted by `offset`, without laying anything out again.
    #[must_use]
    pub const fn translate(offset: Offset) -> Self {
        Self::new(Transform::translate(offset))
    }
}

impl RenderObject for RenderTransform {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        match ctx.children().first().copied() {
            Some(child) => {
                let size = ctx.layout_child(child, constraints);
                ctx.place_child(child, Offset::ZERO);
                size
            }
            None => constraints.constrain(Size::ZERO),
        }
    }

    fn transform(&self) -> Option<Transform> {
        Some(self.transform)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // About this object's own origin. The canvas transform is in global
        // coordinates, so the origin is moved to zero, the transform applied,
        // and the origin put back — the standard sandwich, and the reason a
        // rotation here turns about the widget rather than the window.
        let origin = ctx.origin();
        let canvas = ctx.canvas();
        canvas.save();
        canvas.translate(origin);
        canvas.transform(self.transform);
        canvas.translate(Offset::new(-origin.dx, -origin.dy));
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    // The child's answer, **untransformed** — deliberately not the bounds the
    // matrix would map that answer onto.
    //
    // Reporting the transformed bounds is the tempting version and it is wrong,
    // because an intrinsic has to agree with the size this object actually takes.
    // `layout` above hands the incoming constraints to the child unchanged,
    // places it at the origin and returns *the child's* size; the matrix is never
    // consulted. So a `RenderTransform` scaled by 2.0 that answered "twice the
    // child" would have an `IntrinsicWidth` above it reserve 200 points for
    // something that then lays out to 100 — a column sized against a number no
    // pass in the framework will ever produce, with the surplus showing up as
    // unexplained slack next to a widget drawn the size it always was.
    //
    // This is the same fact `layout_differs` below is stating, from the other
    // side: a transform is paint-only, so changing it must not relayout anything,
    // and an intrinsic that varied with it would mean the answer *did* depend on
    // a property layout ignores. The two must move together — if this object ever
    // grows a mode where the transform does affect layout (a `Transform` that
    // sizes to its transformed bounds is a plausible future widget), both this
    // and `layout_differs` have to change in the same commit, or a rotation will
    // silently keep a stale size.
    //
    // Painting outside the reported box is *expected* here and is what
    // `RenderClip` is for; it is not something an intrinsic can or should
    // describe.
    crate::intrinsics::pass_through_intrinsic!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Never. A transform cannot move a glyph relative to its neighbours or
        // change any size, so relaying out for one would defeat the relayout
        // boundary above it on every frame of an animation — which is precisely
        // the cost this object exists to avoid.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderTransform"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntrinsicQuery, RenderConstrainedBox, RenderTree};

    /// The natural size of the child every test below wraps.
    const CHILD: Size = Size::new(200.0, 100.0);

    /// Ask a transformed child a question, without ever laying it out.
    ///
    /// A tight `SizedBox` for the child because it answers on both axes and at
    /// both extrema without a layout, so anything that moves here moved because
    /// of the transform rather than because of the child.
    fn intrinsic(transform: Transform, query: IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderTransform::new(transform)));
        tree.insert(
            Some(id),
            Box::new(RenderConstrainedBox::new(Constraints::tight(CHILD))),
        );
        tree.intrinsic(id, query)
    }

    /// Every question this object can be asked, at one cross extent and none.
    fn all_queries() -> [IntrinsicQuery; 8] {
        [
            IntrinsicQuery::min_width(),
            IntrinsicQuery::max_width(),
            IntrinsicQuery::min_height(),
            IntrinsicQuery::max_height(),
            IntrinsicQuery::min_width().across(40.0),
            IntrinsicQuery::max_width().across(40.0),
            IntrinsicQuery::min_height().across(40.0),
            IntrinsicQuery::max_height().across(40.0),
        ]
    }

    #[test]
    fn the_answer_is_the_childs_own_on_both_axes_and_at_both_ends() {
        for query in all_queries() {
            let expected = match query.axis {
                vieww_foundation::Axis::Horizontal => CHILD.width,
                vieww_foundation::Axis::Vertical => CHILD.height,
            };
            assert_eq!(
                intrinsic(Transform::IDENTITY, query),
                Some(expected),
                "{query:?}"
            );
        }
    }

    #[test]
    fn no_transform_changes_the_answer_because_none_of_them_changes_the_size() {
        // **The whole point.** `layout` hands the child the incoming constraints
        // and reports the child's size without ever reading the matrix, so an
        // intrinsic that reported transformed bounds would promise a size no
        // layout pass can produce — a scale of 2.0 would have an `IntrinsicWidth`
        // above reserve 400 points for a box that lays out to 200.
        for transform in [
            Transform::scale(2.0, 3.0),
            Transform::scale(0.25, 0.25),
            Transform::translate(Offset::new(50.0, -20.0)),
            Transform::rotate(std::f32::consts::FRAC_PI_4),
        ] {
            for query in all_queries() {
                assert_eq!(
                    intrinsic(transform, query),
                    intrinsic(Transform::IDENTITY, query),
                    "{transform:?} moved an intrinsic it cannot move a layout by: {query:?}"
                );
            }
        }
    }

    #[test]
    fn the_intrinsic_and_layout_agree_on_the_size_a_transform_takes() {
        // Stated against `layout` rather than against a constant, because the
        // agreement is the property and the constant is only today's spelling of
        // it.
        let mut tree = RenderTree::new();
        let id = tree.insert(
            None,
            Box::new(RenderTransform::new(Transform::scale(3.0, 3.0))),
        );
        tree.insert(
            Some(id),
            Box::new(RenderConstrainedBox::new(Constraints::tight(CHILD))),
        );
        let laid_out = tree.layout(id, Constraints::UNBOUNDED);

        assert_eq!(
            tree.intrinsic(id, IntrinsicQuery::max_width()),
            Some(laid_out.width)
        );
        assert_eq!(
            tree.intrinsic(id, IntrinsicQuery::max_height()),
            Some(laid_out.height)
        );
    }

    #[test]
    fn a_childless_transform_answers_nothing() {
        // There is no content to measure, and the pass-through macro says so
        // rather than inventing a zero — the same answer every other paint-only
        // wrapper gives, so a caller sees one consistent rule instead of a
        // per-object guess.
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderTransform::translate(Offset::ZERO)));
        for query in all_queries() {
            assert_eq!(tree.intrinsic(id, query), None, "{query:?}");
        }
    }
}
