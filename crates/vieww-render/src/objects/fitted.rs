use std::cell::Cell;

use vieww_foundation::{Constraints, Offset, Size, Transform};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Shrinks its child until it fits, rather than letting it run off the edge.
///
/// The child is measured **unconstrained** — its natural size — and then painted
/// through a uniform scale small enough to bring it inside the room available.
///
/// # It only ever shrinks
///
/// A child that already fits is left alone at 1.0. Growing a small child to fill
/// its parent is a different operation with a different name, and doing it here
/// would blow a short label up to fill a screen the moment somebody reached for
/// "make this fit". This is `BoxFit::scaleDown` rather than
/// `BoxFit::contain`.
///
/// # The scale is uniform
///
/// Both axes take the smaller factor, so nothing is distorted. Text squeezed
/// horizontally is worse than text that is merely small.
///
/// # What this costs, and when not to reach for it
///
/// **Scaling shrinks the text with everything else**, so a screen that does not
/// fit is usually better fixed by making it scrollable or by letting a child
/// flex. Reach for this when the layout is genuinely fixed and must be seen
/// whole — a diagram, a card designed at one size, a seating plan.
///
/// The child is measured with **unbounded** constraints, so a `Text` inside will
/// not wrap: it reports one long line and is then scaled down. That is the
/// honest consequence of asking for a natural size, and it matches the classics.
#[derive(Debug)]
pub struct RenderFitted {
    /// Written by layout, read by paint and by hit testing.
    ///
    /// A `Cell` because [`RenderObject::transform`] takes `&self` and has to
    /// answer with exactly the matrix `paint` applied — if the two ever
    /// disagree, what is drawn and what can be touched come apart.
    scale: Cell<f32>,
}

impl RenderFitted {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            scale: Cell::new(1.0),
        }
    }

    /// The factor the child is currently drawn at. 1.0 means it fitted.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale.get()
    }

    /// The largest uniform factor that brings `natural` inside `room`, capped at
    /// 1.0.
    ///
    /// Split out and pure because it is the whole behaviour, and because the
    /// degenerate cases — a zero-sized child, an unbounded axis — are easier to
    /// pin here than through a layout.
    fn factor(natural: Size, room: Size) -> f32 {
        // A zero extent would divide by zero, and an unbounded one constrains
        // nothing. Either way that axis has no opinion, so it votes 1.0 and lets
        // the other decide.
        let axis = |natural: f32, room: f32| {
            if natural <= 0.0 || !room.is_finite() {
                1.0
            } else {
                room / natural
            }
        };
        axis(natural.width, room.width)
            .min(axis(natural.height, room.height))
            .min(1.0)
    }
}

impl Default for RenderFitted {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderObject for RenderFitted {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            self.scale.set(1.0);
            return constraints.smallest();
        };

        // Unbounded, so the child reports what it would like to be rather than
        // what it has been squeezed into. Measuring it against the real
        // constraints would hand back an already-clamped size and there would be
        // nothing left to scale.
        let natural = ctx.layout_child(child, Constraints::UNBOUNDED);
        ctx.place_child(child, Offset::ZERO);

        let scale = Self::factor(natural, constraints.biggest());
        self.scale.set(scale);

        // The scale is about this object's own origin and the child sits at that
        // origin, so the drawn result occupies exactly `natural * scale` — which
        // is what this reports, and why no pivot is needed here even though a
        // centred scale would have needed one.
        constraints.constrain(Size::new(natural.width * scale, natural.height * scale))
    }

    fn transform(&self) -> Option<Transform> {
        Some(Transform::scale(self.scale.get(), self.scale.get()))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // The same sandwich `RenderTransform` uses, and for the same reason: the
        // canvas transform is global, so the origin is moved to zero, the scale
        // applied, and the origin put back.
        let origin = ctx.origin();
        let scale = self.scale.get();
        let canvas = ctx.canvas();
        canvas.save();
        canvas.translate(origin);
        canvas.transform(Transform::scale(scale, scale));
        canvas.translate(Offset::new(-origin.dx, -origin.dy));
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The child's **natural** extent, shrunk by the same factor `layout` would
    /// pick for the room the query describes.
    ///
    /// # Why the child is asked at its maximum on both axes
    ///
    /// `layout` measures the child at [`Constraints::UNBOUNDED`], and the
    /// intrinsic spelling of "as much room as you like" is
    /// [`Extremum::Max`](crate::Extremum) with no cross extent. That is what is
    /// asked here, for *both* extrema of the incoming query, because the size
    /// this object reports is a function of the child's natural size and nothing
    /// else. Passing `query.extremum` through would answer a minimum query with
    /// the child's narrowest-without-clipping width — a number `layout` never
    /// looks at — and the two would disagree for every wrapping child.
    ///
    /// That also means this object has no slack of its own: it does not reflow,
    /// it scales, so its minimum and its maximum are one number. Answering a
    /// smaller minimum would be telling a parent "squeeze me for free", when the
    /// price is shrinking the content further — the opposite of what a minimum
    /// promises.
    ///
    /// # Why `cross` may be folded into the scale, and why it does not recurse
    ///
    /// A scale needs *both* axes of the natural size — the factor is the smaller
    /// of the two ratios — so answering a query with `cross` set means asking the
    /// child about the axis that was not asked about. That is allowed
    /// ([`IntrinsicCtx::child_intrinsic`](crate::IntrinsicCtx::child_intrinsic)
    /// is there for it) and here it is also *sound*: the query goes to the
    /// child, never back to this object, so the only way it returns is through
    /// the subtree below, and each node's cache is keyed by axis, so a subtree
    /// asked on both axes is walked twice rather than exponentially.
    ///
    /// The arithmetic mirrors `layout` exactly for the constraints a query with
    /// `cross` describes — the cross axis pinned to `extent`, the queried axis
    /// free. `Self::factor` gives an unbounded axis no vote, so the scale is
    /// `min(1, extent / natural_cross)`, and `constrain` leaves the free axis
    /// alone, so the size is `natural_along * scale`. It is also a fixed point:
    /// feeding the reported extent back as the room on that axis reproduces the
    /// same scale, so a parent that tightens this box to the answer it was given
    /// gets the layout it was promised rather than a second, smaller round of
    /// shrinking.
    ///
    /// With no `cross`, there is no room to fit *to* on either axis, the scale is
    /// 1.0, and the answer is simply the child's natural extent — which is what
    /// `layout` returns under unbounded constraints.
    ///
    /// A childless fitted box is `Some(0.0)` rather than `None`: `layout`'s
    /// childless branch returns `constraints.smallest()`, which is zero where an
    /// intrinsic is asking, so zero is genuinely the answer rather than a stand-in
    /// for one. An *unmeasurable* child is a different thing and stays `None`.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        /// The child's unbounded extent on `axis`, the way `layout` measures it.
        fn natural(ctx: &mut crate::IntrinsicCtx<'_>, axis: Axis) -> Option<f32> {
            ctx.only_child_intrinsic(crate::IntrinsicQuery {
                axis,
                extremum: crate::Extremum::Max,
                cross: None,
            })
        }

        if ctx.child_count() == 0 {
            return Some(0.0);
        }

        let along = natural(ctx, query.axis)?;

        // A non-finite cross extent is an axis with no bound on it, which is the
        // same thing `None` says and the same thing `Self::factor` does with it:
        // no vote, scale 1.0.
        let Some(room) = query.cross.filter(|extent| extent.is_finite()) else {
            return Some(along);
        };

        let across = natural(ctx, query.axis.cross())?;

        // The zero case is `Self::factor`'s, restated: a child with no extent on
        // an axis cannot be divided into, and it is already inside any room at
        // all. The clamp on `room` keeps a caller's negative extent from
        // inverting the answer instead of collapsing it.
        let scale = if across <= 0.0 {
            1.0
        } else {
            (room.max(0.0) / across).min(1.0)
        };

        Some(along * scale)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // Never, on its own account: this object carries no configuration, so a
        // replacement describes exactly the same thing. A child that changed
        // size is pending in its own right and drags the scale along with it.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>().is_none()
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        // Carry the scale across a rebuild, or the frame between the rebuild and
        // the next layout paints the child at 1.0 — a flash of the unscaled,
        // overflowing version, on every rebuild.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.scale.set(old.scale.get());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderFitted"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntrinsicQuery, LayoutCtx, RenderConstrainedBox, RenderTree};

    const ROOM: Size = Size::new(100.0, 100.0);

    #[test]
    fn a_child_that_already_fits_is_left_alone() {
        // Never enlarging is the whole difference from `contain`, and the case
        // somebody will otherwise "fix" into a surprise.
        assert_eq!(RenderFitted::factor(Size::new(50.0, 50.0), ROOM), 1.0);
        assert_eq!(RenderFitted::factor(ROOM, ROOM), 1.0);
    }

    #[test]
    fn a_child_too_big_is_brought_inside() {
        assert_eq!(RenderFitted::factor(Size::new(200.0, 100.0), ROOM), 0.5);
        assert_eq!(RenderFitted::factor(Size::new(100.0, 400.0), ROOM), 0.25);
    }

    #[test]
    fn the_tighter_axis_decides_and_nothing_is_distorted() {
        // One factor for both axes. Squeezing a shape to fit is worse than
        // making it smaller, and the shape is usually the information.
        assert_eq!(RenderFitted::factor(Size::new(400.0, 200.0), ROOM), 0.25);
    }

    #[test]
    fn a_zero_sized_child_does_not_divide_by_zero() {
        assert_eq!(RenderFitted::factor(Size::new(0.0, 0.0), ROOM), 1.0);
        assert_eq!(RenderFitted::factor(Size::new(0.0, 400.0), ROOM), 0.25);
    }

    #[test]
    fn an_unbounded_axis_constrains_nothing() {
        // Inside a scrollable the main axis is infinite, so there is no room to
        // fit *to* on that axis and the other one decides alone. Scaling to
        // infinity would be the alternative, and it is not a size.
        let room = Size::new(100.0, f32::INFINITY);
        assert_eq!(RenderFitted::factor(Size::new(400.0, 9000.0), room), 0.25);
        assert_eq!(RenderFitted::factor(Size::new(50.0, 9000.0), room), 1.0);
    }

    #[test]
    fn the_matrix_paint_uses_is_the_matrix_hit_testing_inverts() {
        // These are two methods reading one field on purpose. If they ever come
        // apart, a control is drawn in one place and touched in another — the
        // hardest class of bug to see and the easiest to introduce here.
        let fitted = RenderFitted::new();
        fitted.scale.set(0.25);
        assert_eq!(
            fitted.transform(),
            Some(Transform::scale(0.25, 0.25)),
            "transform() must report exactly what paint applies"
        );
    }

    // -------------------------------------------------------------- intrinsics

    /// The natural size of the child every intrinsic test below uses.
    ///
    /// A tight `SizedBox` because it is the one child that answers an intrinsic
    /// without being laid out first, on both axes and at both extrema — which is
    /// exactly the property these tests need to isolate this object's own
    /// arithmetic from anybody else's.
    const NATURAL: Size = Size::new(200.0, 100.0);

    /// A fitted box over a `natural`-sized child, and the tree holding it.
    fn tree_with_child(natural: Size) -> (RenderTree, crate::RenderId) {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderFitted::new()));
        tree.insert(
            Some(id),
            Box::new(RenderConstrainedBox::new(Constraints::tight(natural))),
        );
        (tree, id)
    }

    /// Ask a fitted box a question, without ever laying it out.
    fn intrinsic(query: IntrinsicQuery) -> Option<f32> {
        let (mut tree, id) = tree_with_child(NATURAL);
        tree.intrinsic(id, query)
    }

    /// The same, with no child at all.
    fn intrinsic_childless(query: IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderFitted::new()));
        tree.intrinsic(id, query)
    }

    /// The same, with a child that answers nothing.
    fn intrinsic_unmeasurable(query: IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderFitted::new()));
        tree.insert(Some(id), Box::new(Unmeasurable));
        tree.intrinsic(id, query)
    }

    /// A child that answers nothing, like any render object that has not
    /// implemented `intrinsic`.
    #[derive(Debug)]
    struct Unmeasurable;

    impl RenderObject for Unmeasurable {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            constraints.smallest()
        }

        fn debug_name(&self) -> &'static str {
            "Unmeasurable"
        }
    }

    #[test]
    fn with_no_room_named_the_answer_is_the_childs_natural_size() {
        // Nothing to fit to means nothing to shrink by — `layout` under
        // unbounded constraints returns the natural size untouched, and this has
        // to say the same.
        assert_eq!(intrinsic(IntrinsicQuery::max_width()), Some(200.0));
        assert_eq!(intrinsic(IntrinsicQuery::max_height()), Some(100.0));
    }

    #[test]
    fn a_cross_extent_shrinks_the_answer_by_the_factor_layout_would_use() {
        // 200x100 in a 50-tall band scales by a half, so it reports 100 wide.
        // Answering the unscaled 200 would have a parent reserve room for a
        // drawing twice the size of the one that appears.
        assert_eq!(
            intrinsic(IntrinsicQuery::max_width().across(50.0)),
            Some(100.0)
        );
        // And the other way: 100 wide over a 200-wide child is also a half.
        assert_eq!(
            intrinsic(IntrinsicQuery::max_height().across(100.0)),
            Some(50.0)
        );
    }

    #[test]
    fn a_cross_extent_larger_than_the_child_never_enlarges_it() {
        // The shrink-only rule, restated where it is easiest to lose: the
        // factor is capped at 1.0, so a roomy parent does not blow a small
        // diagram up to fill it.
        assert_eq!(
            intrinsic(IntrinsicQuery::max_width().across(9000.0)),
            Some(200.0)
        );
    }

    #[test]
    fn an_infinite_cross_extent_is_the_same_statement_as_none() {
        // Inside a scrollable the cross axis can arrive as infinity, and it
        // constrains nothing — the same vote `factor` gives it.
        assert_eq!(
            intrinsic(IntrinsicQuery::max_height().across(f32::INFINITY)),
            intrinsic(IntrinsicQuery::max_height())
        );
    }

    #[test]
    fn scaling_has_no_slack_so_the_minimum_equals_the_maximum() {
        // This object does not reflow, it scales, so both ends of its range are
        // one number. A smaller minimum would invite a parent to squeeze it, and
        // the price of that squeeze is the content getting smaller.
        assert_eq!(
            intrinsic(IntrinsicQuery::min_width()),
            intrinsic(IntrinsicQuery::max_width())
        );
        assert_eq!(
            intrinsic(IntrinsicQuery::min_height().across(100.0)),
            intrinsic(IntrinsicQuery::max_height().across(100.0))
        );
    }

    #[test]
    fn the_intrinsic_answer_is_the_size_layout_actually_produces() {
        // The property that makes the arithmetic above worth having rather than
        // merely plausible. The constraints are the ones a query describes — the
        // cross axis pinned, the queried axis free — so `layout` is being asked
        // the same question in its own language.
        for room in [25.0_f32, 50.0, 100.0, 400.0] {
            let (mut tree, id) = tree_with_child(NATURAL);
            let laid_out = tree.layout(id, Constraints::new(0.0, f32::INFINITY, room, room));
            assert_eq!(
                tree.intrinsic(id, IntrinsicQuery::max_width().across(room)),
                Some(laid_out.width),
                "width in a {room}-tall band: intrinsic and layout disagree"
            );

            let (mut tree, id) = tree_with_child(NATURAL);
            let laid_out = tree.layout(id, Constraints::new(room, room, 0.0, f32::INFINITY));
            assert_eq!(
                tree.intrinsic(id, IntrinsicQuery::max_height().across(room)),
                Some(laid_out.height),
                "height in a {room}-wide column: intrinsic and layout disagree"
            );
        }
    }

    #[test]
    fn a_childless_fitted_box_reports_zero_rather_than_unknown() {
        // `layout`'s childless branch returns `smallest()`, which is zero where
        // an intrinsic is asking. Zero is the answer here, not a stand-in for
        // one — and saying `None` would make an empty placeholder collapse the
        // measurement of everything around it.
        for query in [
            IntrinsicQuery::max_width(),
            IntrinsicQuery::min_height().across(40.0),
        ] {
            assert_eq!(intrinsic_childless(query), Some(0.0), "{query:?}");
        }
    }

    #[test]
    fn a_child_that_cannot_be_measured_stays_unknown() {
        // The other half of the pair above, and the module's rule: this object's
        // size *is* the child's, so a child that does not know cannot be guessed
        // at. `None` makes the asking widget transparent; a zero would collapse
        // the diagram it was measuring.
        assert_eq!(intrinsic_unmeasurable(IntrinsicQuery::max_width()), None);
        assert_eq!(
            intrinsic_unmeasurable(IntrinsicQuery::max_height().across(50.0)),
            None
        );
    }

    #[test]
    fn a_child_with_no_extent_on_the_cross_axis_does_not_divide_by_zero() {
        // `factor`'s zero case, reached through the intrinsic path instead: an
        // empty child is already inside any room at all, so the scale is 1.0
        // rather than an infinity the tree would rewrite to zero.
        let (mut tree, id) = tree_with_child(Size::new(200.0, 0.0));
        assert_eq!(
            tree.intrinsic(id, IntrinsicQuery::max_width().across(50.0)),
            Some(200.0)
        );
    }
}
