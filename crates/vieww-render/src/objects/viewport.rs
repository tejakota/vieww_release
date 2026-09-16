use std::cell::Cell;
use std::fmt;

use vieww_foundation::{Axis, Constraints, Offset, Rect, Size};
use vieww_widget::{Handler, ScrollExtents};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Shows part of a longer child, clipped to its own bounds.
///
/// # Unbounded on the main axis
///
/// The child is laid out with **infinite** space along the scroll axis and the
/// viewport's own extent across it. That is what makes a list able to be longer
/// than the screen — and it is also why a child that tries to fill its main axis
/// is a bug rather than a stretch: infinity is not a size, and a `SizedBox`
/// asked to fill an unbounded axis has nothing to fill.
///
/// # It is a repaint boundary
///
/// Scrolling changes only where the child is drawn, and a viewport's whole
/// purpose is that this happens often. Without a boundary every scrolled pixel
/// would re-record the layer containing the entire page.
pub struct RenderViewport {
    pub axis: Axis,
    pub offset: f32,
    /// Whether the content is anchored at the far edge instead of the origin.
    ///
    /// What a horizontal viewport does in a right-to-left interface: the first
    /// row sits against the **right** edge and scrolling walks leftward. The
    /// scroll offset itself is unaffected — it stays a distance *into the
    /// content*, from zero to the same maximum — so nothing downstream of here
    /// has to learn about reading direction. Only the anchor moves.
    pub reverse: bool,
    on_extents: Option<Handler<ScrollExtents>>,
    /// The last extents handed upward, so the same numbers are never reported
    /// twice.
    reported: Cell<Option<ScrollExtents>>,
}

impl RenderViewport {
    #[must_use]
    pub fn new(axis: Axis, offset: f32) -> Self {
        Self {
            axis,
            offset,
            reverse: false,
            on_extents: None,
            reported: Cell::new(None),
        }
    }

    /// Anchor the content at the far edge rather than the origin.
    ///
    /// See [`reverse`](Self::reverse). Set by the factory for a horizontal
    /// viewport under a right-to-left [`Directionality`](vieww_widget::Directionality).
    #[must_use]
    pub const fn reversed(mut self, reverse: bool) -> Self {
        self.reverse = reverse;
        self
    }

    /// Report the viewport's and the content's length whenever they change.
    #[must_use]
    pub fn on_extents(mut self, handler: Handler<ScrollExtents>) -> Self {
        self.on_extents = Some(handler);
        self
    }

    /// The child's displacement for the current offset.
    ///
    /// Reversed, the content's trailing edge starts flush with the viewport's
    /// and the child walks *forward* as the offset grows — the mirror image of
    /// the forward case, which starts the leading edges flush and walks back.
    /// At the maximum offset the two agree: `viewport - content + max` is zero,
    /// which is where the forward case begins.
    fn shift(&self, viewport: f32, content: f32) -> Offset {
        let main = if self.reverse {
            (viewport - content) + self.offset
        } else {
            -self.offset
        };
        match self.axis {
            Axis::Vertical => Offset::new(0.0, main),
            Axis::Horizontal => Offset::new(main, 0.0),
        }
    }

    const fn along(&self, size: Size) -> f32 {
        match self.axis {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }

    /// The extent at right angles to the scroll axis.
    const fn across(&self, size: Size) -> f32 {
        match self.axis {
            Axis::Vertical => size.width,
            Axis::Horizontal => size.height,
        }
    }

    /// A size from its main- and cross-axis extents.
    const fn sized(&self, main: f32, cross: f32) -> Size {
        match self.axis {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }

    /// Hand the extents upward, but only when they are news.
    ///
    /// A handler here writes a signal, and a signal written every layout would
    /// mark its readers pending every frame — a rebuild loop that never settles and
    /// looks exactly like a runaway animation. Reporting only changes is what
    /// makes calling out of layout safe at all.
    ///
    /// **This guard now holds the frame loop up as well as the rebuild loop.**
    /// `FrameDriver::needs_frame` asks whether any element is pending, because a
    /// signal written from *layout* is marked after the build phase and nothing
    /// else would ask for the frame that shows it. So an unconditional report
    /// here no longer costs a wasted rebuild — it pins the display at sixty
    /// frames a second for the life of the application. This is the only
    /// handler in the framework called from layout; anything that joins it
    /// inherits the same obligation.
    fn report_extents(&self, extents: ScrollExtents) {
        if self.reported.get() == Some(extents) {
            return;
        }
        self.reported.set(Some(extents));
        if let Some(handler) = &self.on_extents {
            handler(extents);
        }
    }
}

impl fmt::Debug for RenderViewport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderViewport")
            .field("axis", &self.axis)
            .field("offset", &self.offset)
            .field("extents", &self.reported.get())
            .finish_non_exhaustive()
    }
}

impl RenderObject for RenderViewport {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // **Along the axis**, the viewport is as big as it is allowed to be: a
        // window onto the content, not a thing sized by it.
        let (main_min, main_max) = match self.axis {
            Axis::Vertical => (constraints.min_height, constraints.max_height),
            Axis::Horizontal => (constraints.min_width, constraints.max_width),
        };
        let (cross_min, cross_max) = match self.axis {
            Axis::Vertical => (constraints.min_width, constraints.max_width),
            Axis::Horizontal => (constraints.min_height, constraints.max_height),
        };
        let main = if main_max.is_finite() {
            main_max
        } else {
            main_min
        };

        // **Across it, the viewport is as big as it is allowed to be — unless
        // it is not allowed anything, in which case it is as big as its child.**
        //
        // The second half is what a horizontal viewport inside a vertical one
        // needs, which is every code editor ever written: the outer one hands
        // its child an unbounded main axis, so the inner one's *cross* axis
        // arrives unbounded. Falling back to `min` there — which is zero in
        // that composition — made the inner viewport zero-tall, and a
        // zero-tall window is one that draws nothing and hit-tests nothing.
        // The failure is silent and reads as "the pane went blank", which is
        // some distance from "the cross-axis constraint was infinite".
        //
        // Sizing to the child instead is the same rule `RenderText` and every
        // other content-sized object follows when it is given no bound, and it
        // cannot change any bounded case: the branch is not taken there.
        let cross_bounded = cross_max.is_finite();
        let child_cross = if cross_bounded {
            (cross_max, cross_max)
        } else {
            (cross_min, f32::INFINITY)
        };

        let Some(&child) = ctx.children().first() else {
            let size = constraints.constrain(self.sized(main, child_cross.0));
            self.report_extents(ScrollExtents::new(self.along(size), 0.0));
            return size;
        };

        let child_constraints = match self.axis {
            Axis::Vertical => Constraints {
                min_width: child_cross.0,
                max_width: child_cross.1,
                min_height: 0.0,
                max_height: f32::INFINITY,
            },
            Axis::Horizontal => Constraints {
                min_width: 0.0,
                max_width: f32::INFINITY,
                min_height: child_cross.0,
                max_height: child_cross.1,
            },
        };
        let content = ctx.layout_child(child, child_constraints);
        let cross = if cross_bounded {
            cross_max
        } else {
            self.across(content)
        };
        let size = constraints.constrain(self.sized(main, cross));

        ctx.place_child(child, self.shift(self.along(size), self.along(content)));
        self.report_extents(ScrollExtents::new(self.along(size), self.along(content)));
        size
    }

    /// Across the scroll axis, the child's answer. Along it, `None`.
    ///
    /// # Why the main axis is not the child's answer
    ///
    /// `layout` above says it out loud: *"a window onto the content, not a thing
    /// sized by it"*. The main extent comes entirely from the constraints —
    /// `max` where it is finite and `min` where it is not — and the child is then
    /// laid out against `0..INFINITY` along that axis precisely so it may be
    /// longer than the window. There is no configuration in which the child's
    /// main extent becomes this object's main extent, so forwarding it would
    /// report a number `layout` can never produce.
    ///
    /// It would also be actively harmful rather than merely wrong. An
    /// `IntrinsicHeight` above a vertical viewport would read "9000" off a long
    /// page and tighten the viewport to it, so the viewport would be exactly as
    /// tall as its content, nothing would ever be off screen, and the scroll
    /// would have nothing left to do. The widget whose whole purpose is to show
    /// less than it holds would have been talked out of it by the measuring pass.
    ///
    /// `None` rather than `Some(0.0)`, for the reason the module documentation
    /// gives: zero is a confident claim that this wants no room, and a caller
    /// acting on it collapses the viewport to nothing. `None` makes the asking
    /// widget transparent, layout runs exactly as it did before the intrinsic
    /// pass existed, and the viewport takes the main extent it is offered — which
    /// is the correct behaviour and the one it already had.
    ///
    /// # Why the cross axis *is*
    ///
    /// Because on that axis the child is laid out **tight** to this object's own
    /// cross extent, so the two are the same number by construction. A parent
    /// that tightens this viewport to the reported cross extent hands the child
    /// exactly that, and the child gets what it asked for: the intrinsic and
    /// `layout` agree, which is the only test that matters for whether an
    /// intrinsic should be forwarded at all.
    ///
    /// `cross` is dropped on the way down on purpose. The caller's cross value
    /// for a cross-axis query is an extent along the *scroll* axis, and the
    /// child is laid out with infinity there — telling it that it has 300 points
    /// of height when scrolling exists to give it as many as it likes would get
    /// the wrapped-to-the-window answer instead of the natural one.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        if query.axis == self.axis {
            return None;
        }

        // An empty window has no content to be as wide as, and zero is the
        // honest answer rather than an unknown dressed as one — there is
        // nothing in there to measure.
        if ctx.child_count() == 0 {
            return Some(0.0);
        }

        let mut inner = query;
        inner.cross = None;
        ctx.only_child_intrinsic(inner)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Clip before the children draw. The tree paints them straight after
        // this returns, and `Scene` resolves the clip into every command
        // recorded while it is in force — so the restore has to happen after
        // them, which is what `paint_clip_children` is for.
        let bounds = ctx.bounds();
        ctx.canvas().save();
        ctx.canvas().clip_rect(bounds);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The same clip, for the children `paint` cannot reach.
    ///
    /// **This is the object that made the hook necessary.** A viewport clips
    /// *and* is a repaint boundary, so a `RepaintBoundary` anywhere inside a
    /// scrollable records into a scene the `clip_rect` above never touches and
    /// drew outside the scroll view — including the one `CircularProgress`
    /// inserts by itself for an indeterminate spinner.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    fn is_repaint_boundary(&self) -> bool {
        true
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // The axis and the offset decide where the child is placed; the handler
        // is a closure handed down fresh on every rebuild and cannot be compared
        // — comparing it would mean a relayout per frame.
        //
        // `reverse` is in here because it decides the anchor: a subtree that
        // gains a `Directionality` keeps its axis and its offset and still has
        // to place the child somewhere else entirely.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>().is_none_or(|new| {
            self.axis != new.axis
                || self.reverse != new.reverse
                || (self.offset - new.offset).abs() > f32::EPSILON
        })
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        // Carry what has already been reported across the rebuild, or the first
        // layout after every scroll re-reports the same extents — and each report
        // marks the reader pending, which schedules the rebuild that does it again.
        //
        // **`adopt_reports`, not `adopt_layout_cache`.** This used to be the
        // latter, which the tree calls only when `layout_differs` says no — and
        // a scroll changes the offset, so it always says yes. The one hook that
        // could have saved this was the one never called on a scrolling
        // viewport, which is every viewport that matters.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.reported.set(old.reported.get());
        }
    }

    fn semantics(&self) -> Option<crate::Semantics> {
        // Announced as a region so a screen reader knows its contents can move,
        // which is what makes "scroll down" an offer rather than a surprise.
        Some(crate::Semantics::new(crate::Role::ScrollView))
    }

    fn debug_name(&self) -> &'static str {
        "RenderViewport"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn the_same_extents_are_never_reported_twice() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let viewport = RenderViewport::new(Axis::Vertical, 0.0)
            .on_extents(Rc::new(move |extents| sink.borrow_mut().push(extents)));

        viewport.report_extents(ScrollExtents::new(300.0, 900.0));
        viewport.report_extents(ScrollExtents::new(300.0, 900.0));
        viewport.report_extents(ScrollExtents::new(300.0, 1200.0));

        assert_eq!(
            *seen.borrow(),
            vec![
                ScrollExtents::new(300.0, 900.0),
                ScrollExtents::new(300.0, 1200.0)
            ],
            "reporting an unchanged extent every layout is a rebuild loop"
        );
    }

    #[test]
    fn a_rebuild_carries_what_was_already_reported() {
        let old = RenderViewport::new(Axis::Vertical, 0.0);
        old.report_extents(ScrollExtents::new(300.0, 900.0));

        let mut fresh = RenderViewport::new(Axis::Vertical, 40.0);
        fresh.adopt_reports(&old);

        assert_eq!(fresh.reported.get(), Some(ScrollExtents::new(300.0, 900.0)));
    }

    #[test]
    fn a_scrolled_viewport_does_not_re_report_the_extents_it_already_sent() {
        // The regression this exists for, and the reason the test above was not
        // enough: it adopts by hand, and the *tree* used to adopt only when
        // `layout_differs` was false. A scroll changes the offset, so it is
        // always true — so on every scrolled frame the ledger was dropped, the
        // identical extents went out again, and the handler's `Signal::set`
        // marked its reader pending whether or not anything had changed. The tree
        // then never settled: `pending=1` for the life of the window, measured on
        // a desktop.
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let at_rest = RenderViewport::new(Axis::Vertical, 0.0)
            .on_extents(Rc::new(move |extents| sink.borrow_mut().push(extents)));
        at_rest.report_extents(ScrollExtents::new(300.0, 900.0));

        // The rebuild a scroll causes: a new object at a new offset, which
        // `layout_differs` correctly calls different.
        let mut scrolled = RenderViewport::new(Axis::Vertical, 40.0);
        assert!(
            at_rest.layout_differs(&scrolled),
            "a scroll must still relayout — the fix is not to stop noticing"
        );
        // `adopt_reports` is what the tree calls unconditionally. Calling
        // `adopt_layout_cache` here instead would pass while the real path
        // failed, which is exactly how this went unnoticed.
        scrolled.adopt_reports(&at_rest);
        scrolled.report_extents(ScrollExtents::new(300.0, 900.0));

        assert_eq!(
            seen.borrow().len(),
            1,
            "the same extents reported twice is a rebuild loop that never settles"
        );
    }

    #[test]
    fn scrolling_relays_out_but_a_new_handler_alone_does_not() {
        let still = RenderViewport::new(Axis::Vertical, 40.0);
        let scrolled = RenderViewport::new(Axis::Vertical, 41.0);
        let same_but_reporting =
            RenderViewport::new(Axis::Vertical, 40.0).on_extents(Rc::new(|_| {}));

        assert!(still.layout_differs(&scrolled));
        assert!(!still.layout_differs(&same_but_reporting));
    }

    /// 300 of window onto 900 of content, so the furthest it can scroll is 600.
    const WINDOW: f32 = 300.0;
    const CONTENT: f32 = 900.0;
    const MAX: f32 = CONTENT - WINDOW;

    #[test]
    fn a_forward_viewport_starts_flush_with_the_origin() {
        let at_rest = RenderViewport::new(Axis::Horizontal, 0.0);
        assert_eq!(at_rest.shift(WINDOW, CONTENT), Offset::new(0.0, 0.0));

        let scrolled = RenderViewport::new(Axis::Horizontal, 100.0);
        assert_eq!(scrolled.shift(WINDOW, CONTENT), Offset::new(-100.0, 0.0));
    }

    #[test]
    fn a_reversed_viewport_starts_flush_with_the_far_edge() {
        // The content's trailing edge meets the window's: 300 - 900 = -600 puts
        // the content's right edge at 300, which is the window's right edge.
        // This is the whole feature — an Arabic list opens on its first row.
        let at_rest = RenderViewport::new(Axis::Horizontal, 0.0).reversed(true);
        assert_eq!(at_rest.shift(WINDOW, CONTENT), Offset::new(-MAX, 0.0));

        // And it walks *forward* as the offset grows, where the other walks back.
        let scrolled = RenderViewport::new(Axis::Horizontal, 100.0).reversed(true);
        assert_eq!(scrolled.shift(WINDOW, CONTENT), Offset::new(-500.0, 0.0));
    }

    #[test]
    fn the_two_anchors_are_each_others_start_and_finish() {
        // The property that says the pair is a mirror rather than two unrelated
        // formulas: each one ends exactly where the other begins. If this holds
        // at both extremes, the scroll offset still means the same thing in
        // both — a distance into the content, from zero to the same maximum.
        let forward_end = RenderViewport::new(Axis::Horizontal, MAX).shift(WINDOW, CONTENT);
        let reversed_start = RenderViewport::new(Axis::Horizontal, 0.0)
            .reversed(true)
            .shift(WINDOW, CONTENT);
        assert_eq!(forward_end, reversed_start);

        let forward_start = RenderViewport::new(Axis::Horizontal, 0.0).shift(WINDOW, CONTENT);
        let reversed_end = RenderViewport::new(Axis::Horizontal, MAX)
            .reversed(true)
            .shift(WINDOW, CONTENT);
        assert_eq!(forward_start, reversed_end);
    }

    #[test]
    fn content_shorter_than_the_window_sits_against_the_far_edge_when_reversed() {
        // No scrolling is possible either way, but the resting place differs —
        // and getting this wrong is invisible in every test that uses content
        // longer than its window, which is all the others here.
        let forward = RenderViewport::new(Axis::Horizontal, 0.0);
        assert_eq!(forward.shift(WINDOW, 100.0), Offset::new(0.0, 0.0));

        let reversed = RenderViewport::new(Axis::Horizontal, 0.0).reversed(true);
        assert_eq!(
            reversed.shift(WINDOW, 100.0),
            Offset::new(200.0, 0.0),
            "a short row still begins at the right edge in Arabic"
        );
    }

    // ------------------------------------------------------------- intrinsics

    use crate::{IntrinsicQuery, RenderConstrainedBox, RenderId, RenderTree};

    /// A viewport over a `content`-sized child, never laid out.
    ///
    /// Not laid out on purpose: the callers that need an intrinsic — an
    /// `Accordion` animating open, an `IntrinsicWidth` deciding what to hand
    /// down — ask before any of this is on screen.
    fn measurable(axis: Axis, content: Size) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderViewport::new(axis, 0.0)));
        tree.set_root(Some(root));
        tree.insert(
            Some(root),
            Box::new(RenderConstrainedBox::new(Constraints::tight(content))),
        );
        (tree, root)
    }

    /// A viewport with nothing in it.
    fn empty(axis: Axis) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderViewport::new(axis, 0.0)));
        tree.set_root(Some(root));
        (tree, root)
    }

    #[test]
    fn the_cross_axis_forwards_the_child_because_the_child_is_laid_out_tight_to_it() {
        // 120 wide and far too long to fit, which is the shape a viewport is
        // for. The width is still an honest question with an honest answer.
        let (mut tree, root) = measurable(Axis::Vertical, Size::new(120.0, 9_000.0));
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width()),
            Some(120.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_width()),
            Some(120.0)
        );

        // And the same the other way round: across a horizontal scroll the
        // meaningful axis is the height.
        let (mut tree, root) = measurable(Axis::Horizontal, Size::new(9_000.0, 120.0));
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(120.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(120.0)
        );
    }

    #[test]
    fn the_main_axis_is_unknown_rather_than_the_length_of_the_content() {
        // The claim `layout` makes — a window onto the content, not a thing
        // sized by it — carried into the measuring pass. Forwarding 9000 here
        // would let an `IntrinsicHeight` tighten the viewport to its own
        // content, and a viewport exactly as tall as what it holds has nothing
        // left to scroll.
        let (mut tree, root) = measurable(Axis::Vertical, Size::new(120.0, 9_000.0));
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            None,
            "and emphatically not Some(0.0), which would collapse the window"
        );
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_height()), None);

        let (mut tree, root) = measurable(Axis::Horizontal, Size::new(9_000.0, 120.0));
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), None);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), None);
    }

    #[test]
    fn a_known_extent_along_the_scroll_axis_does_not_narrow_the_cross_answer() {
        // The child is laid out against infinity on the scroll axis, so telling
        // it that only 300 points of height are available would get the answer
        // for a child that had been squeezed — a squeeze that never happens.
        let (mut tree, root) = measurable(Axis::Vertical, Size::new(120.0, 9_000.0));
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width().across(300.0)),
            Some(120.0)
        );
    }

    #[test]
    fn an_empty_viewport_is_no_wider_than_its_absent_content() {
        // Zero because there is genuinely nothing in there, which is not the
        // same statement as "I cannot tell".
        let (mut tree, root) = empty(Axis::Vertical);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), Some(0.0));
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), Some(0.0));
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            None,
            "the main axis is still whatever it is given, child or no child"
        );
    }

    #[test]
    fn an_unmeasurable_child_makes_the_cross_axis_unknown_rather_than_zero() {
        // The propagation rule: a viewport over something that cannot answer
        // cannot answer either, and reporting zero would collapse a subtree that
        // plain layout would have shown at whatever width it was offered.
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderViewport::new(Axis::Vertical, 0.0)));
        tree.set_root(Some(root));
        tree.insert(Some(root), Box::new(Unmeasurable));
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), None);
    }

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
    fn tightening_to_the_cross_intrinsic_gives_the_child_exactly_what_it_asked_for() {
        // The agreement that justifies forwarding the cross axis at all: an
        // `IntrinsicWidth` reads the number, hands it down tight, and layout
        // produces that same number rather than something else.
        let (mut tree, root) = measurable(Axis::Vertical, Size::new(120.0, 9_000.0));
        let wanted = tree
            .intrinsic(root, IntrinsicQuery::max_width())
            .expect("the cross axis is measurable");

        tree.layout_root(Constraints {
            min_width: wanted,
            max_width: wanted,
            min_height: 0.0,
            max_height: 300.0,
        });

        assert_eq!(tree.size(root).width, wanted);
        assert_eq!(
            tree.size(tree.children(root)[0]).width,
            wanted,
            "the child is laid out tight to the cross extent, which is why the \
             intrinsic could be forwarded"
        );
        assert_eq!(
            tree.size(root).height,
            300.0,
            "and the main axis is still the window it was given, not the 9000 \
             of content inside it"
        );
    }

    /// **A horizontal viewport inside a vertical one is not zero-tall.**
    ///
    /// The outer viewport hands its child an unbounded main axis, so the inner
    /// one's *cross* axis arrives unbounded — and the cross extent used to fall
    /// back to `min`, which is zero in exactly that composition. A zero-tall
    /// window draws nothing and hit-tests nothing, so a code pane that gained
    /// horizontal scrolling went blank and stopped taking clicks. This is the
    /// composition every code editor is, and nothing tested it.
    #[test]
    fn a_viewport_with_no_cross_axis_bound_takes_its_size_from_its_child() {
        let mut tree = RenderTree::new();
        let outer = tree.insert(None, Box::new(RenderViewport::new(Axis::Vertical, 0.0)));
        tree.set_root(Some(outer));
        let inner = tree.insert(
            Some(outer),
            Box::new(RenderViewport::new(Axis::Horizontal, 0.0)),
        );
        tree.insert(
            Some(inner),
            Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
                2_000.0, 480.0,
            )))),
        );

        tree.layout(
            outer,
            Constraints {
                min_width: 0.0,
                max_width: 300.0,
                min_height: 0.0,
                max_height: 200.0,
            },
        );

        assert_eq!(
            tree.size(inner).height,
            480.0,
            "the inner viewport collapsed to nothing instead of taking its              child's height"
        );
        assert_eq!(
            tree.size(inner).width,
            300.0,
            "and its own axis is still the window it was given, not the 2000              of content inside it"
        );
    }

    /// The bounded case, unchanged — the branch above must not touch it.
    #[test]
    fn a_bounded_cross_axis_still_fills_what_it_was_given() {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderViewport::new(Axis::Horizontal, 0.0)));
        tree.set_root(Some(root));
        tree.insert(
            Some(root),
            Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
                2_000.0, 40.0,
            )))),
        );

        let size = tree.layout(
            root,
            Constraints {
                min_width: 0.0,
                max_width: 300.0,
                min_height: 0.0,
                max_height: 200.0,
            },
        );

        assert_eq!(size.height, 200.0, "not the child's 40");
        assert_eq!(size.width, 300.0);
    }

    #[test]
    fn gaining_a_reading_direction_relayouts() {
        // Same axis, same offset, different anchor. Without `reverse` in
        // `layout_differs` a list that gained a `Directionality` would keep the
        // placement it had, and nothing would ever put it right.
        let ltr = RenderViewport::new(Axis::Horizontal, 40.0);
        let rtl = RenderViewport::new(Axis::Horizontal, 40.0).reversed(true);
        assert!(ltr.layout_differs(&rtl));
        assert!(rtl.layout_differs(&ltr));
    }
}
