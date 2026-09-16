use vieww_foundation::{Constraints, Offset, Rect, Size};

use crate::sliver::{SliverConstraints, SliverGeometry};
use crate::{LayoutCtx, PaintCtx, RenderObject};

/// How a header behaves once it has been scrolled past.
///
/// # Closed, over a mechanism that is not
///
/// A third party cannot add a variant here — and does not need to. `RenderObject`
/// takes part in the sliver protocol through
/// [`layout_sliver`](RenderObject::layout_sliver), a defaulted trait method, and
/// [`FrameDriver::register`](crate::FrameDriver::register) puts the result in a
/// running application, so a header behaviour nobody here imagined is written as
/// **a sliver of its own** rather than as a case in this enum.
///
/// This used to point at `RenderFactory::register`, which is public and which an
/// application had no way to reach — the claim was true of the library and false
/// of anything built on it. Worth remembering when writing the next sentence of
/// this shape: "the mechanism is public" and "somebody can use it" are different
/// claims, and only the second one is worth making.
///
/// So this is a convenience over an open mechanism, which is the right way
/// round: the closed thing is the shortcut, and the general thing is the one
/// anybody can reach. Compare [`Command`](vieww_paint::Command), which is closed
/// with nothing underneath it — that one is a real ceiling and says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderBehaviour {
    /// Scrolls away with the content and does not come back until the content
    /// does.
    #[default]
    Scrolling,
    /// Collapses to [`min_extent`](RenderSliverAppBar::min_extent) and stays
    /// there.
    Pinned,
    /// Scrolls away entirely, and returns as soon as the user scrolls back —
    /// without waiting for the top of the content.
    Floating,
}

/// A header that collapses as the content scrolls under it.
///
/// # This is the object the sliver protocol was added for
///
/// Everything else in a scrolling screen can be faked. A list of rows can be a
/// column in a clipping viewport, more slowly. A header that is 200 pixels tall
/// at rest, shrinks to 56 as you scroll, and then **stays on screen while the
/// content keeps moving underneath it** cannot be, because it needs to report
/// two different numbers:
///
/// - it consumes `max_extent - min_extent` of *scroll* before it is collapsed;
/// - it occupies between `max_extent` and `min_extent` of the *screen*, and once
///   pinned it occupies `min_extent` for ever while consuming no further scroll.
///
/// A box layout has one number — a size — so it can express the first or the
/// second and never both. That is the entire argument, and it is why this file
/// is the test of whether the protocol was worth adding.
///
/// # Painting
///
/// The child is laid out at the *current* extent and stretched to it, so a title
/// that centres itself stays centred as the bar collapses. A child that wants to
/// fade or move as the bar shrinks reads [`collapsed_fraction`](Self::collapsed_fraction)
/// from the widget layer, which is a signal rather than a layout input — fading
/// must not relayout.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSliverAppBar {
    /// The height at rest, before any scrolling.
    pub max_extent: f32,
    /// The height it collapses to.
    pub min_extent: f32,
    pub behaviour: HeaderBehaviour,
    /// Filled in by layout: 0.0 fully expanded, 1.0 fully collapsed.
    collapsed: f32,
    /// How much of a [floating](HeaderBehaviour::Floating) bar is currently out.
    ///
    /// **State, and the only state any sliver here keeps.** A floating bar
    /// cannot be a pure function of the scroll position, because at any given
    /// offset it is either shown or hidden depending on which way the user
    /// arrived — that is the entire behaviour. So it integrates the scroll
    /// deltas instead, and this is the accumulator.
    ///
    /// Carried across rebuilds by [`adopt_reports`](RenderObject::adopt_reports),
    /// without which every frame of a drag would start it from scratch.
    revealed: f32,
    /// The scroll offset the previous layout ran at, for the delta above.
    last_scroll: Option<f32>,
}

impl RenderSliverAppBar {
    #[must_use]
    pub fn new(max_extent: f32, min_extent: f32) -> Self {
        Self {
            max_extent: max_extent.max(0.0),
            // A minimum above the maximum is a caller error that would make the
            // bar *grow* as it scrolled. Clamped rather than asserted: this is
            // reachable from an animated value passing through a bad frame.
            min_extent: min_extent.clamp(0.0, max_extent.max(0.0)),
            behaviour: HeaderBehaviour::Scrolling,
            collapsed: 0.0,
            revealed: max_extent.max(0.0),
            last_scroll: None,
        }
    }

    #[must_use]
    pub const fn behaviour(mut self, behaviour: HeaderBehaviour) -> Self {
        self.behaviour = behaviour;
        self
    }

    /// Stay on screen at [`min_extent`](Self::min_extent) once collapsed.
    #[must_use]
    pub const fn pinned(self) -> Self {
        self.behaviour(HeaderBehaviour::Pinned)
    }

    /// How far through its collapse the bar is: 0.0 expanded, 1.0 collapsed.
    ///
    /// What a title animates against. Written by layout and read by the widget
    /// layer, exactly as a viewport's scroll extents are.
    #[must_use]
    pub const fn collapsed_fraction(&self) -> f32 {
        self.collapsed
    }

    /// How much scroll this bar consumes before it is fully collapsed.
    #[must_use]
    pub fn collapse_range(&self) -> f32 {
        (self.max_extent - self.min_extent).max(0.0)
    }

    /// The extent the bar occupies at `scroll_offset`.
    fn extent_at(&self, scroll_offset: f32) -> f32 {
        (self.max_extent - scroll_offset.max(0.0)).clamp(self.min_extent, self.max_extent)
    }

    /// How much of a floating bar is showing, integrated from this frame's
    /// scroll delta.
    ///
    /// # Why this is integrated rather than computed
    ///
    /// Every other behaviour here is a pure function of `scroll_offset`. This
    /// one cannot be, and that is the definition of floating rather than a
    /// shortcoming: at 800 pixels down, a floating bar is **out** if you just
    /// scrolled up to get there and **gone** if you scrolled down. The same
    /// input, two answers, so the state is the difference between them.
    ///
    /// Three rules, and the first is the one that stops it drifting:
    ///
    /// - at the very top the bar is fully out, whatever the accumulator says,
    ///   so any rounding error accumulated over a long scroll is erased every
    ///   time the user reaches the top;
    /// - scrolling toward the end pushes it out of view, one pixel per pixel;
    /// - scrolling back pulls it in again, the same way.
    ///
    /// Moving with the finger rather than animating is deliberate. A bar that
    /// animates in on any upward scroll appears on a one-pixel twitch and then
    /// has to decide when to leave; one that tracks the gesture is reversible
    /// mid-drag, which is what makes it feel attached to the content.
    fn integrate_reveal(&mut self, constraints: &SliverConstraints, extent: f32) {
        if self.behaviour != HeaderBehaviour::Floating {
            // Kept pinned to the top of its range so that *switching* to
            // floating — an app that changes the behaviour on a setting — does
            // not start from a stale accumulator.
            self.revealed = extent;
            self.last_scroll = Some(constraints.scroll_offset);
            return;
        }

        let delta = match self.last_scroll {
            Some(last) => constraints.scroll_offset - last,
            // The first layout is not a scroll.
            None => 0.0,
        };
        self.last_scroll = Some(constraints.scroll_offset);

        if constraints.scroll_offset <= 0.0 {
            self.revealed = self.max_extent;
            return;
        }

        // Subtracting the delta is right in both directions: scrolling toward
        // the end has a positive delta and hides the bar, and scrolling back
        // has a negative one and reveals it.
        self.revealed = (self.revealed - delta).clamp(0.0, self.max_extent);
    }
}

impl RenderObject for RenderSliverAppBar {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Outside a viewport: the expanded bar, and nothing collapses.
        let size = Size::new(constraints.max_width, self.max_extent);
        if let Some(&child) = ctx.children().first() {
            ctx.layout_child(child, Constraints::tight(size));
            ctx.place_child(child, Offset::ZERO);
        }
        constraints.constrain(size)
    }

    fn layout_sliver(
        &mut self,
        ctx: &mut LayoutCtx<'_>,
        constraints: &SliverConstraints,
    ) -> Option<SliverGeometry> {
        let extent = self.extent_at(constraints.scroll_offset);

        self.collapsed = if self.collapse_range() > 0.0 {
            ((self.max_extent - extent) / self.collapse_range()).clamp(0.0, 1.0)
        } else {
            // A bar that does not collapse is either fully expanded or gone,
            // and reporting 0.0 keeps a title from jumping at the boundary.
            0.0
        };

        self.integrate_reveal(constraints, extent);

        // How far the bar has slid up out of the viewport, as distinct from how
        // far it has collapsed. **These are the two different ways a header can
        // get smaller**, and every behaviour below is a choice about which one
        // it uses:
        //
        // - collapsing shrinks the box and re-lays the child out into it, so a
        //   title re-centres as the bar closes;
        // - sliding keeps the box and moves it, so the top of the bar goes off
        //   the screen and the bottom is what is left.
        //
        // Conflating them is why a naive app bar looks like it is being
        // squashed rather than leaving.
        let collapsed_by = self.max_extent - extent;
        let slid = match self.behaviour {
            // Whatever is left of the scroll after the collapse absorbed its
            // share.
            HeaderBehaviour::Scrolling => (constraints.scroll_offset - collapsed_by).max(0.0),
            // Never slides. That is what pinned means.
            HeaderBehaviour::Pinned => 0.0,
            // Driven by the accumulator rather than by the scroll position,
            // because at any given offset a floating bar is shown or hidden
            // depending on which way the user arrived.
            HeaderBehaviour::Floating => (extent - self.revealed).max(0.0),
        };

        let child_size = constraints.size(extent, constraints.cross_axis_extent);
        if let Some(&child) = ctx.children().first() {
            ctx.layout_child(child, Constraints::tight(child_size));
            // Negative: the child is moved *up* out of the bar's own painted
            // band, and the bar's clip is what cuts it off. So the top of the
            // header leaves first, which is what "scrolls away" looks like.
            ctx.place_child(child, constraints.offset(-slid));
        }

        let painted = (extent - slid).clamp(0.0, constraints.remaining_paint_extent.max(0.0));

        // The bar always consumes its full extent of scroll, whatever it
        // currently looks like. That is what makes the content beneath it move
        // at the same rate as the bar shrinks rather than twice as fast.
        //
        // And what it *takes* from the content is only what is left of its own
        // uncollapsed self — reaching zero and staying there, which is how a
        // pinned bar keeps painting while the content keeps scrolling under it.
        // Those two numbers are the pair no box layout can report, and this is
        // the line that reports them.
        let taking = (self.max_extent - constraints.scroll_offset).clamp(0.0, painted);

        Some(SliverGeometry::new(self.max_extent, painted).taking(taking))
    }

    /// `max_extent` for [`Max`](crate::Extremum::Max), `min_extent` for
    /// [`Min`](crate::Extremum::Min), and `None` across.
    ///
    /// # The one object here where the extremum is not decoration
    ///
    /// Everything else in this crate that reports a fixed size reports the same
    /// number at both ends of the range, because nothing about it reflows. A
    /// collapsing header is the exception, and the two extremums are the two
    /// numbers in its name:
    ///
    /// - **`Max`** is `max_extent`, the height at rest, which is what the header
    ///   would like if nobody is scrolling. It is also exactly what the box
    ///   `layout` branch produces — `Size::new(constraints.max_width,
    ///   self.max_extent)` — so an `IntrinsicHeight` above an app bar reserves
    ///   the height the bar will actually take.
    /// - **`Min`** is `min_extent`, and this is a real answer rather than a
    ///   smaller guess: the definition of `Min` is the least extent at which the
    ///   object can be laid out without its content being clipped, and the whole
    ///   design of this header is that it is *intended* to work at
    ///   `min_extent` — that is where it collapses to and where a pinned bar
    ///   lives for the rest of the scroll. Repeating `max_extent` here would
    ///   claim the bar cannot survive being smaller, which is the opposite of
    ///   what it was built to do.
    ///
    /// [`new`](Self::new) clamps `min_extent` into `0..=max_extent`, so `Min` is
    /// never above `Max` even for a caller passing an animated value through a
    /// bad frame.
    ///
    /// # Read from the box branch, never from `layout_sliver`
    ///
    /// The current extent — the number this bar is actually painting at — is
    /// `extent_at(constraints.scroll_offset)`, and for a floating bar it also
    /// depends on `revealed`, an accumulator integrated across frames of a drag.
    /// Both are exactly what the purity contract forbids: an intrinsic derived
    /// from them would answer 200 at the top of a page and 56 halfway down, and
    /// a parent that had cached the first would be wrong for the rest of the
    /// scroll. `max_extent` and `min_extent` are configuration, so they are the
    /// two numbers this can honestly report.
    ///
    /// `behaviour` is deliberately not consulted either. It decides how the bar
    /// gets *out of the way*, which is a sliver-geometry question about paint and
    /// layout extents, and none of the three variants changes the range the
    /// header occupies.
    ///
    /// # Why the cross axis is `None`
    ///
    /// `layout` takes `constraints.max_width` and gives the child a tight size
    /// of it, so the header is as wide as it is told and the child has no vote.
    /// There is no content-derived width to report, and inventing one from the
    /// child would produce a number layout can never produce.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        match (query.axis, query.extremum) {
            (Axis::Vertical, crate::Extremum::Max) => Some(self.max_extent),
            (Axis::Vertical, crate::Extremum::Min) => Some(self.min_extent),
            (Axis::Horizontal, _) => None,
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        ctx.canvas().save();
        ctx.canvas().clip_rect(bounds);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The same clip, for a header child that owns a layer — see
    /// [`RenderObject::layer_clip`]. A collapsing bar is a shrinking window onto
    /// content that does not shrink with it, so the clip is what makes the
    /// collapse look like one.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let any: &dyn std::any::Any = new;
        match any.downcast_ref::<Self>() {
            Some(new) => {
                self.behaviour != new.behaviour
                    || (self.max_extent - new.max_extent).abs() > f32::EPSILON
                    || (self.min_extent - new.min_extent).abs() > f32::EPSILON
            }
            None => true,
        }
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        // `revealed` and `last_scroll` are not measurements — they are the
        // floating bar's memory, and a scroll rebuilds the tree on every frame
        // of the drag. Without this the accumulator resets sixty times a second
        // and a floating bar never moves at all.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.collapsed = old.collapsed;
            self.revealed = old.revealed;
            self.last_scroll = old.last_scroll;
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderSliverAppBar"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Axis;

    fn at(scroll: f32) -> SliverConstraints {
        SliverConstraints {
            scroll_offset: scroll,
            ..SliverConstraints::initial(Axis::Vertical, 600.0, 400.0)
        }
    }

    fn bar() -> RenderSliverAppBar {
        RenderSliverAppBar::new(200.0, 56.0)
    }

    #[test]
    fn an_expanded_bar_is_at_its_maximum() {
        assert_eq!(bar().extent_at(0.0), 200.0);
    }

    #[test]
    fn scrolling_collapses_it_towards_its_minimum() {
        assert_eq!(bar().extent_at(100.0), 100.0);
        assert_eq!(bar().extent_at(144.0), 56.0, "fully collapsed");
    }

    #[test]
    fn it_never_collapses_past_its_minimum_however_far_you_scroll() {
        assert_eq!(bar().extent_at(10_000.0), 56.0);
    }

    #[test]
    fn a_minimum_above_the_maximum_is_clamped_rather_than_growing_the_bar() {
        // Reachable from an animated value passing through a bad frame.
        let odd = RenderSliverAppBar::new(100.0, 500.0);
        assert_eq!(odd.min_extent, 100.0);
        assert_eq!(odd.extent_at(50.0), 100.0);
    }

    /// Lay `bar` out at a run of scroll offsets, returning the last geometry.
    ///
    /// A floating bar's state is the whole point, so a test that jumps straight
    /// to an offset is testing something else.
    fn scrolled_through(bar: RenderSliverAppBar, offsets: &[f32]) -> SliverGeometry {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(bar));
        tree.set_root(Some(id));

        let mut last = SliverGeometry::ZERO;
        for &offset in offsets {
            last = tree.layout_sliver(id, &at(offset));
        }
        last
    }

    #[test]
    fn a_pinned_bar_paints_after_it_has_stopped_taking_room() {
        // The two numbers a box layout cannot report separately, which is the
        // entire justification for the sliver protocol. If this test ever
        // collapses into a single assertion, the protocol was not needed.
        let geometry = scrolled_through(bar().pinned(), &[500.0]);

        assert_eq!(geometry.paint_extent, 56.0, "still on screen");
        assert_eq!(
            geometry.layout_extent, 0.0,
            "and taking nothing from the content, which keeps scrolling under it"
        );

        // And the same bar un-pinned is simply gone.
        assert_eq!(scrolled_through(bar(), &[500.0]).paint_extent, 0.0);
    }

    #[test]
    fn a_pinned_bar_needs_no_paint_origin_to_stay_at_the_top() {
        // It used to set one, and that was a misreading worth recording: the
        // viewport places each sliver at the running total of *layout* extents,
        // and a pinned bar stops contributing to that total — so it is already
        // at the viewport's leading edge without asking for anything.
        assert_eq!(scrolled_through(bar().pinned(), &[500.0]).paint_origin, 0.0);
    }

    // -------------------------------------------------------------- floating

    #[test]
    fn a_floating_bar_leaves_as_you_scroll_toward_the_end() {
        let gone = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 100.0, 300.0, 600.0],
        );
        assert_eq!(gone.paint_extent, 0.0);
    }

    #[test]
    fn a_floating_bar_comes_back_on_the_way_up_without_reaching_the_top() {
        // The behaviour, and the reason it needs state: 600 is 600 whichever
        // way you got there, and the bar is out at one and gone at the other.
        let away = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 300.0, 600.0],
        );
        let returning = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 300.0, 700.0, 600.0],
        );

        assert_eq!(away.paint_extent, 0.0, "arrived scrolling down: gone");
        assert!(
            returning.paint_extent > 0.0,
            "arrived scrolling back up: showing, at the same scroll offset"
        );
    }

    #[test]
    fn a_floating_bar_tracks_the_finger_rather_than_snapping_open() {
        // Scrolling back 40 pixels reveals 40 pixels of bar, so the gesture is
        // reversible mid-drag. A bar that animated open would be at its full
        // height here and would have to decide on its own when to leave.
        let partway = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 600.0, 560.0],
        );
        assert!(
            (partway.paint_extent - 40.0).abs() < 0.001,
            "expected 40 pixels revealed, got {}",
            partway.paint_extent
        );
    }

    #[test]
    fn a_floating_bar_takes_nothing_from_the_content_once_it_is_scrolled_past() {
        // It overlays rather than pushing, which is what distinguishes it from
        // the content simply having scrolled back.
        let returning = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 700.0, 600.0],
        );
        assert!(returning.paint_extent > 0.0);
        assert_eq!(returning.layout_extent, 0.0);
    }

    #[test]
    fn reaching_the_top_restores_a_floating_bar_in_full() {
        // The rule that keeps the accumulator from drifting over a long scroll.
        let home = scrolled_through(
            bar().behaviour(HeaderBehaviour::Floating),
            &[0.0, 5_000.0, 0.0],
        );
        assert_eq!(home.paint_extent, 200.0);
    }

    #[test]
    fn a_non_floating_bar_keeps_its_accumulator_pinned_to_its_extent() {
        // So that an app switching the behaviour at runtime does not start from
        // a stale one.
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(bar().pinned()));
        tree.set_root(Some(id));
        tree.layout_sliver(id, &at(1_000.0));

        let revealed = tree
            .object(id)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderSliverAppBar>()
            })
            .map(|bar| bar.revealed);
        assert_eq!(
            revealed,
            Some(56.0),
            "its collapsed extent, not a stale 200"
        );
    }

    #[test]
    fn the_collapsed_fraction_runs_from_zero_to_one() {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(bar()));
        tree.set_root(Some(id));

        tree.layout_sliver(id, &at(0.0));
        let expanded = tree
            .object(id)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderSliverAppBar>()
            })
            .map(RenderSliverAppBar::collapsed_fraction);
        assert_eq!(expanded, Some(0.0));

        tree.layout_sliver(id, &at(144.0));
        let collapsed = tree
            .object(id)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderSliverAppBar>()
            })
            .map(RenderSliverAppBar::collapsed_fraction);
        assert_eq!(collapsed, Some(1.0));
    }

    // ------------------------------------------------------------- intrinsics

    use crate::{IntrinsicQuery, RenderId, RenderTree};

    /// A bar in a tree, never laid out, optionally with a child.
    fn mounted(bar: RenderSliverAppBar, child: bool) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(bar));
        tree.set_root(Some(root));
        if child {
            tree.insert(Some(root), Box::new(Unmeasurable));
        }
        (tree, root)
    }

    /// A title that answers nothing, like any render object without an
    /// `intrinsic`. The bar's own answer must not depend on it.
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
    fn the_two_extremums_are_the_two_extents_in_the_bars_name() {
        // The one object in this pass where `Min` and `Max` genuinely differ. A
        // collapsing header is *designed* to work at 56, so reporting 200 for
        // the minimum would claim it cannot survive being smaller — the opposite
        // of what it is for.
        let (mut tree, root) = mounted(bar(), false);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(200.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(56.0)
        );
    }

    #[test]
    fn a_bar_that_cannot_collapse_reports_one_number_at_both_ends() {
        let (mut tree, root) = mounted(RenderSliverAppBar::new(56.0, 56.0), false);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(56.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(56.0)
        );

        // And a minimum above the maximum is clamped at construction, so the
        // minimum can never come back larger than the maximum here either.
        let (mut tree, root) = mounted(RenderSliverAppBar::new(100.0, 500.0), false);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(100.0)
        );
    }

    #[test]
    fn the_cross_axis_is_unknown_because_the_bar_is_told_its_width() {
        // `layout` takes `constraints.max_width` and hands the child a tight
        // size of it, so there is no content-derived width — and `Some(0.0)`
        // would collapse a header that plain layout would have shown full width.
        let (mut tree, root) = mounted(bar(), true);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), None);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), None);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width().across(200.0)),
            None
        );
    }

    #[test]
    fn a_known_width_does_not_change_either_extent_and_neither_does_a_child() {
        // The bar's height is its own configuration: nothing about it reflows,
        // and a title that cannot measure itself must not poison the answer,
        // because `layout` never asks the title anything either.
        let (mut tree, root) = mounted(bar(), true);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height().across(320.0)),
            Some(200.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height().across(320.0)),
            Some(56.0)
        );

        let (mut childless, root) = mounted(bar(), false);
        assert_eq!(
            childless.intrinsic(root, IntrinsicQuery::max_height()),
            Some(200.0),
            "a bar with no title is still a bar of the same height"
        );
    }

    #[test]
    fn the_answers_do_not_move_when_the_bar_is_scrolled_or_collapsed() {
        // The purity the trait asks for. `extent_at` and the floating
        // accumulator are functions of the scroll offset and of the frames a
        // drag passed through; an intrinsic reading either would answer 200 at
        // the top of the page and 56 halfway down, and a parent that cached the
        // first would be wrong for the rest of the scroll.
        for behaviour in [
            HeaderBehaviour::Scrolling,
            HeaderBehaviour::Pinned,
            HeaderBehaviour::Floating,
        ] {
            let (mut tree, root) = mounted(bar().behaviour(behaviour), false);
            for offset in [0.0, 144.0, 500.0, 400.0] {
                tree.layout_sliver(root, &at(offset));
            }
            assert_eq!(
                tree.intrinsic(root, IntrinsicQuery::max_height()),
                Some(200.0),
                "{behaviour:?} reported a different maximum after scrolling"
            );
            assert_eq!(
                tree.intrinsic(root, IntrinsicQuery::min_height()),
                Some(56.0),
                "{behaviour:?} reported a different minimum after scrolling"
            );
        }
    }

    #[test]
    fn the_maximum_is_the_height_the_box_layout_actually_produces() {
        // Where the agreement is meant to hold: outside a viewport nothing
        // collapses, so the box branch gives the expanded bar and `Max` is that
        // number. `Min` deliberately does not agree — nothing ever lays the bar
        // out at 56 outside a viewport — and that is what makes it the *other*
        // end of the range rather than a second guess at the same one.
        let (mut tree, root) = mounted(bar(), false);
        let wanted = tree
            .intrinsic(root, IntrinsicQuery::max_height())
            .expect("a bar always knows its own extents");

        let size = tree.layout_root(Constraints::new(0.0, 320.0, 0.0, f32::INFINITY));
        assert_eq!(size.height, wanted);
    }

    #[test]
    fn a_bar_that_cannot_collapse_reports_zero_rather_than_dividing_by_zero() {
        let fixed = RenderSliverAppBar::new(56.0, 56.0);
        assert_eq!(fixed.collapse_range(), 0.0);
        assert_eq!(fixed.extent_at(500.0), 56.0);
    }
}
