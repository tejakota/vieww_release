use vieww_foundation::{Constraints, Offset, Rect, Size};

use crate::sliver::{visible_extent, SliverConstraints, SliverGeometry};
use crate::{LayoutCtx, PaintCtx, RenderObject};

/// A run of equal-height children inside a scrolling viewport.
///
/// # What makes this different from `RenderFlex` inside a viewport
///
/// A column of ten thousand rows in a box viewport lays out ten thousand rows,
/// every frame, to discover that twenty are visible. This lays out the twenty.
///
/// It can, because the sliver protocol tells it `scroll_offset` and
/// `remaining_paint_extent` — so the arithmetic `first = scroll_offset / extent`
/// answers "which row is at the top" without measuring anything. That is the
/// whole argument for a second layout protocol, in one line of division.
///
/// # Why the item extent is fixed
///
/// Because it is what makes the division above legal. Variable-height rows need
/// either a measured prefix sum or an estimate that is corrected as the user
/// scrolls, and both are real designs with real costs —
/// [`ListView::variable`](vieww_widget) already carries the measured one for the
/// box path. This is the case that is exactly right and exactly cheap, and it is
/// the case almost every list actually is.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSliverFixedList {
    /// How many children exist, whether or not they are in the tree.
    ///
    /// The count is *declared* rather than counted from the children, because
    /// the point is that the children which are off screen are not there. A
    /// list of ten thousand rows holds twenty render objects and a `count` of
    /// ten thousand.
    pub count: usize,
    /// The main-axis size of every child.
    pub item_extent: f32,
    /// Which child the first mounted one corresponds to.
    ///
    /// Set by the widget layer, which owns building them. Layout reports which
    /// window it *wants* through [`visible_range`](Self::visible_range); the
    /// element layer supplies it on the next build, exactly as
    /// `ListView` already does for the box path.
    pub first_index: usize,
    /// Filled in by layout: the window this sliver would like mounted.
    wanted: Option<(usize, usize)>,
}

impl RenderSliverFixedList {
    #[must_use]
    pub const fn new(count: usize, item_extent: f32) -> Self {
        Self {
            count,
            item_extent,
            first_index: 0,
            wanted: None,
        }
    }

    #[must_use]
    pub const fn starting_at(mut self, first_index: usize) -> Self {
        self.first_index = first_index;
        self
    }

    /// The half-open range of children the last layout wanted mounted.
    #[must_use]
    pub const fn visible_range(&self) -> Option<(usize, usize)> {
        self.wanted
    }

    /// The total main-axis extent of every child, mounted or not.
    #[must_use]
    pub fn total_extent(&self) -> f32 {
        self.item_extent.max(0.0) * self.count as f32
    }

    /// Which children are on screen at `constraints`.
    ///
    /// Half-open, clamped to the count, and with one row of slack at each end so
    /// a scroll does not expose an unbuilt row for the frame it takes to notice.
    fn window(&self, constraints: &SliverConstraints) -> (usize, usize) {
        if self.item_extent <= 0.0 || self.count == 0 {
            return (0, 0);
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "both are clamped non-negative and then against `count`"
        )]
        let first = (constraints.scroll_offset / self.item_extent)
            .floor()
            .max(0.0) as usize;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "as above"
        )]
        let visible = (constraints.remaining_paint_extent / self.item_extent)
            .ceil()
            .max(0.0) as usize;

        // One row of slack each way. Without it, the row entering the viewport
        // is unbuilt for exactly the frame in which it becomes visible — which
        // reads as a flicker at the edge of every fast scroll.
        let first = first.saturating_sub(1);
        let last = first
            .saturating_add(visible)
            .saturating_add(2)
            .min(self.count);
        (first, last)
    }
}

impl RenderObject for RenderSliverFixedList {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Outside a viewport this is an ordinary column of whatever is mounted.
        // A sliver has to work here too — it may be put somewhere unscrollable,
        // and failing at that would make the widget non-composable.
        let mut used = 0.0;
        for child in ctx.children_owned() {
            let size = ctx.layout_child(
                child,
                Constraints::new(
                    constraints.max_width,
                    constraints.max_width,
                    self.item_extent,
                    self.item_extent,
                ),
            );
            ctx.place_child(child, Offset::new(0.0, used));
            used += size.height;
        }
        constraints.constrain(Size::new(constraints.max_width, used))
    }

    fn layout_sliver(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        constraints: &SliverConstraints,
    ) -> Option<SliverGeometry> {
        let (first, last) = self.window(constraints);
        self.wanted = Some((first, last));

        let total = self.total_extent();
        let child_constraints = match constraints.axis {
            vieww_foundation::Axis::Vertical => Constraints::new(
                constraints.cross_axis_extent,
                constraints.cross_axis_extent,
                self.item_extent,
                self.item_extent,
            ),
            vieww_foundation::Axis::Horizontal => Constraints::new(
                self.item_extent,
                self.item_extent,
                constraints.cross_axis_extent,
                constraints.cross_axis_extent,
            ),
        };

        // A row's place in the *list* is `index * item_extent`; its place in
        // this sliver is that minus however much of the list has scrolled past.
        //
        // # The subtraction is the whole coordinate system, so it is worth being
        // explicit about which one this is
        //
        // Two conventions were available. Either a sliver's origin sits where
        // its *content* starts — above the viewport once scrolled — and rows are
        // placed at their raw list offsets; or it sits at the top of its own
        // **visible band** and rows are shifted by the scroll. This is the
        // second, and it has to be, because a sliver clips to `ctx.bounds()` and
        // those bounds are its paint extent. Under the first convention the clip
        // would sit as far above the viewport as the list had scrolled, and the
        // rows would be cut off by a rectangle nowhere near them.
        //
        // Getting this wrong is invisible at scroll offset zero, which is where
        // every first look happens.
        for (slot, child) in ctx.children_owned().into_iter().enumerate() {
            ctx.layout_child(child, child_constraints);
            let index = self.first_index.saturating_add(slot);
            let along = index as f32 * self.item_extent - constraints.scroll_offset;
            ctx.place_child(child, constraints.offset(along));
        }

        Some(SliverGeometry::new(
            total,
            visible_extent(total, constraints),
        ))
    }

    /// `count * item_extent` along the main axis, and `None` across it.
    ///
    /// # The one intrinsic in the crate that measures nothing
    ///
    /// Every row is `item_extent` tall by construction, and there are `count` of
    /// them, so the total is a multiplication — no child is asked, and none has
    /// to be. That is the same argument the file makes for the division in
    /// `window`, read in the other direction, and it is what makes this answer
    /// possible at all: a list of ten thousand rows holds twenty render objects,
    /// so an intrinsic that summed its children would report the height of the
    /// twenty that happen to be mounted and change every time the user scrolled.
    ///
    /// **This mirrors the box `layout` branch, not `layout_sliver`.**
    /// `layout_sliver` is a function of `scroll_offset` — which rows are on
    /// screen, how much of the list is above the fold — and reading any of that
    /// from here would produce an answer that goes stale the instant the user
    /// drags. `total_extent` is the number `layout_sliver` reports as its
    /// *scroll* extent for exactly this reason: it is the one extent that does
    /// not depend on where the scroll is.
    ///
    /// The box branch agrees whenever the whole list is mounted, which is the
    /// case in which the box branch is used at all — put somewhere unscrollable,
    /// there is nothing to virtualise and every row is there. Mounted or not,
    /// the answer is the same, because the declared `count` is the truth about
    /// how many rows exist.
    ///
    /// # Why `Min` and `Max` are one number
    ///
    /// Nothing here reflows. A fixed-extent list cannot be made shorter by
    /// giving it more room across, so "the least it can be without clipping" and
    /// "the most it would benefit from" are the same multiplication, and `cross`
    /// changes nothing either.
    ///
    /// # Why the cross axis is `None`
    ///
    /// Because the box branch takes `constraints.max_width` and hands each child
    /// a **tight** width of that. The rows have no vote in how wide this is —
    /// they are told — so there is no content-derived cross extent to report,
    /// and deriving one from the children would produce a number `layout` can
    /// never produce. `None` makes an `IntrinsicWidth` above a list transparent
    /// and the list keeps filling the width it is offered, which is what it did
    /// before this method existed and what it should keep doing.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        match query.axis {
            // `total_extent` already floors a negative item extent at zero, and
            // an empty list is genuinely zero tall rather than unmeasurable.
            vieww_foundation::Axis::Vertical => Some(self.total_extent()),
            vieww_foundation::Axis::Horizontal => None,
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Rows outside the sliver's own extent must not draw over the sliver
        // after it — the slack rows above are exactly that case.
        let bounds = ctx.bounds();
        ctx.canvas().save();
        ctx.canvas().clip_rect(bounds);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The same clip, for a row that owns a layer — see
    /// [`RenderObject::layer_clip`]. A row wrapped in a `RepaintBoundary` for
    /// its repaint cost must still not draw over the sliver after this one.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let any: &dyn std::any::Any = new;
        match any.downcast_ref::<Self>() {
            Some(new) => {
                self.count != new.count
                    || self.first_index != new.first_index
                    || (self.item_extent - new.item_extent).abs() > f32::EPSILON
            }
            // `wanted` is written *by* layout, so comparing it would make every
            // layout pending the object it just produced.
            None => true,
        }
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.wanted = old.wanted;
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderSliverFixedList"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Axis;

    fn constraints(scroll: f32, remaining: f32) -> SliverConstraints {
        SliverConstraints {
            scroll_offset: scroll,
            remaining_paint_extent: remaining,
            ..SliverConstraints::initial(Axis::Vertical, 600.0, 400.0)
        }
    }

    #[test]
    fn a_list_of_ten_thousand_rows_wants_only_what_is_on_screen() {
        // The claim the whole protocol exists to make good on.
        let list = RenderSliverFixedList::new(10_000, 50.0);
        let (first, last) = list.window(&constraints(0.0, 600.0));
        assert_eq!(first, 0);
        assert!(
            last <= 16,
            "a 600px viewport of 50px rows wanted {last} rows mounted"
        );
    }

    #[test]
    fn scrolling_moves_the_window_by_division_rather_than_by_measuring() {
        let list = RenderSliverFixedList::new(10_000, 50.0);
        let (first, _) = list.window(&constraints(5_000.0, 600.0));
        assert_eq!(first, 99, "row 100, minus one row of slack");
    }

    #[test]
    fn the_window_has_slack_so_a_fast_scroll_shows_no_gap() {
        // Without the slack the row entering the viewport is unbuilt for the
        // frame in which it becomes visible, which reads as a flicker.
        let list = RenderSliverFixedList::new(10_000, 50.0);
        let (first, last) = list.window(&constraints(500.0, 600.0));
        assert!(first < 10, "a row above the fold is kept: {first}");
        assert!(last > 22, "and one below it: {last}");
    }

    #[test]
    fn the_window_never_runs_past_the_end_of_the_list() {
        let list = RenderSliverFixedList::new(5, 50.0);
        let (_, last) = list.window(&constraints(0.0, 600.0));
        assert_eq!(last, 5);
    }

    #[test]
    fn an_empty_list_wants_nothing_rather_than_dividing_by_zero() {
        assert_eq!(
            RenderSliverFixedList::new(0, 50.0).window(&constraints(0.0, 600.0)),
            (0, 0)
        );
        assert_eq!(
            RenderSliverFixedList::new(10, 0.0).window(&constraints(0.0, 600.0)),
            (0, 0)
        );
    }

    #[test]
    fn the_scroll_extent_is_the_whole_list_and_the_paint_extent_is_the_screen() {
        // The two numbers a box layout cannot report separately, which is why
        // a scrollbar over a virtualised list is impossible without this.
        let list = RenderSliverFixedList::new(10_000, 50.0);
        assert_eq!(list.total_extent(), 500_000.0);
        assert_eq!(
            visible_extent(list.total_extent(), &constraints(0.0, 600.0)),
            600.0
        );
    }

    // ------------------------------------------------------------- intrinsics

    use crate::{IntrinsicQuery, RenderConstrainedBox, RenderId, RenderTree};

    /// A list with `mounted` rows actually in the tree, never laid out.
    ///
    /// The two numbers are deliberately allowed to differ: a list of ten
    /// thousand rows holds twenty render objects, and an intrinsic that counted
    /// the children rather than reading `count` would report the twenty.
    fn list_of(count: usize, item_extent: f32, mounted: usize) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(
            None,
            Box::new(RenderSliverFixedList::new(count, item_extent)),
        );
        tree.set_root(Some(root));
        for _ in 0..mounted {
            tree.insert(
                Some(root),
                Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
                    300.0,
                    item_extent,
                )))),
            );
        }
        (tree, root)
    }

    #[test]
    fn the_main_axis_is_the_whole_list_and_no_row_is_asked_anything() {
        // Ten thousand rows, twenty of them mounted, and the answer is about
        // all ten thousand — which is only possible because the extent is fixed.
        let (mut tree, root) = list_of(10_000, 50.0, 20);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(500_000.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(500_000.0),
            "nothing here reflows, so the two ends of the range coincide"
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height().across(300.0)),
            Some(500_000.0),
            "and a known width changes nothing either"
        );
    }

    #[test]
    fn the_cross_axis_is_unknown_because_rows_are_told_their_width_rather_than_asked() {
        // `layout` hands every row a tight width of `constraints.max_width`, so
        // the rows have no vote in how wide the list is and there is no
        // content-derived width to report. Zero would collapse a list that plain
        // layout would have shown at whatever width it was offered.
        let (mut tree, root) = list_of(10_000, 50.0, 20);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), None);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), None);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width().across(400.0)),
            None
        );
    }

    #[test]
    fn an_empty_list_is_genuinely_zero_tall_rather_than_unmeasurable() {
        // The one place `Some(0.0)` is right: no rows is no height, and saying
        // so lets an `IntrinsicHeight` above an empty list close to nothing
        // instead of falling back.
        let (mut tree, root) = list_of(0, 50.0, 0);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(0.0)
        );

        // The same for rows with no extent, which `window` already refuses to
        // divide by.
        let (mut tree, root) = list_of(10, 0.0, 0);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(0.0)
        );
    }

    #[test]
    fn the_answer_does_not_move_when_the_list_is_scrolled() {
        // The purity the trait asks for, and the reason this mirrors the box
        // branch rather than `layout_sliver`: the sliver path is a function of
        // `scroll_offset`, and an answer derived from it would differ between
        // two callers asking on either side of a drag.
        let (mut resting, root) = list_of(10_000, 50.0, 20);
        let at_rest = resting.intrinsic(root, IntrinsicQuery::max_height());

        let (mut scrolled, root) = list_of(10_000, 50.0, 20);
        scrolled.layout_sliver(root, &constraints(250_000.0, 600.0));
        assert_eq!(
            scrolled.intrinsic(root, IntrinsicQuery::max_height()),
            at_rest,
            "halfway down a list is still the same list"
        );
    }

    #[test]
    fn the_intrinsic_is_the_height_the_box_layout_actually_produces() {
        // Where the agreement is meant to hold: outside a viewport nothing is
        // virtualised, every row is mounted, and the column of them is exactly
        // as tall as the multiplication says.
        let (mut tree, root) = list_of(3, 50.0, 3);
        let wanted = tree
            .intrinsic(root, IntrinsicQuery::max_height())
            .expect("a fixed list always knows its own length");

        let size = tree.layout_root(Constraints::new(0.0, 300.0, 0.0, f32::INFINITY));
        assert_eq!(size.height, wanted, "150 of rows, measured and laid out");
    }

    #[test]
    fn layout_writing_its_own_window_does_not_pending_the_object() {
        let mut measured = RenderSliverFixedList::new(100, 50.0);
        measured.wanted = Some((0, 20));
        assert!(!RenderSliverFixedList::new(100, 50.0).layout_differs(&measured));
    }
}
