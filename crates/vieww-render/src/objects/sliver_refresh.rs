use vieww_foundation::{Constraints, Offset, Rect, Size};

use crate::sliver::{SliverConstraints, SliverGeometry};
use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Occupies the gap opened by dragging the content past its start.
///
/// # Why this could not exist before overscroll did
///
/// A pull-to-refresh control lives in a place that does not exist during
/// ordinary scrolling: above the first item, in a gap that only appears while
/// the user is holding the content below where it can go. A viewport that
/// clamps its scroll offset at zero has nowhere to put it — not "no widget for
/// it", *nowhere*.
///
/// So this is the one control whose absence was a statement about the layout
/// engine rather than about the widget library, and it is here to prove the
/// engine changed.
///
/// # The two states, and why their geometry is opposite
///
/// **Being pulled.** The gap already exists — the viewport made it — so this
/// paints *into* it and takes nothing from the content. `layout_extent` is zero
/// and [`paint_origin`](SliverGeometry::paint_origin) is negative, which lifts
/// it off the content's leading edge and into the gap above.
///
/// **Refreshing.** The finger is gone and the gap has sprung shut, so there is
/// no longer anywhere to be. Now it *makes* room: `layout_extent` equals its
/// extent, the content is pushed down, and it sits in the space it created.
///
/// Two states, opposite geometry, one object. Reporting the same numbers for
/// both is the bug where the spinner either disappears the moment you let go or
/// leaves a permanent gap at the top of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSliverRefresh {
    /// How far the content must be pulled before a refresh is offered.
    pub trigger_extent: f32,
    /// `true` while the application is actually refreshing.
    ///
    /// Set by the caller, not by this object. Deciding that a gesture ended past
    /// the trigger is a *gesture* question, and this object never sees a
    /// pointer — the same division that keeps the scroll offset in a controller.
    pub refreshing: bool,
    /// Filled in by layout: how far the content has been pulled.
    pulled: f32,
}

impl RenderSliverRefresh {
    #[must_use]
    pub const fn new(trigger_extent: f32) -> Self {
        Self {
            trigger_extent,
            refreshing: false,
            pulled: 0.0,
        }
    }

    #[must_use]
    pub const fn refreshing(mut self, refreshing: bool) -> Self {
        self.refreshing = refreshing;
        self
    }

    /// How far the content has been pulled past its start, in logical pixels.
    #[must_use]
    pub const fn pulled(&self) -> f32 {
        self.pulled
    }

    /// How far through the pull the user is: 0.0 untouched, 1.0 at the trigger.
    ///
    /// What a spinner rotates by, so that the indicator tracks the finger rather
    /// than appearing at a threshold.
    #[must_use]
    pub fn pulled_fraction(&self) -> f32 {
        if self.trigger_extent <= 0.0 {
            return 0.0;
        }
        (self.pulled / self.trigger_extent).clamp(0.0, 1.0)
    }

    /// `true` when letting go now should start a refresh.
    #[must_use]
    pub fn armed(&self) -> bool {
        self.trigger_extent > 0.0 && self.pulled >= self.trigger_extent
    }
}

/// What the control was given and what it reported back, on
/// `VIEWW_TRACE_FRAMES`, and only when it changes.
///
/// # Why a render object carries an instrument
///
/// Because this one's whole contract is a pair of numbers that are *opposite* in
/// its two states, and every way of checking them from outside is blind. The
/// unit tests below call `layout_sliver` directly with constraints written by
/// hand, so they prove the arithmetic and say nothing about what a real viewport
/// passes down. The suite is green while the control is visibly wrong on screen.
///
/// The three questions this answers in one run, in the order they narrow:
///
/// 1. **Does `overscroll` arrive at all, and with what sign?** Zero at rest and
///    positive while pulled is correct. Negative means `overscroll.min(room)` is
///    negative and every extent below it is nonsense.
/// 2. **Is `painted` zero at rest?** It is what the node's size becomes, and the
///    size is what `paint` clips to — so a non-zero value at rest is a control
///    that cannot be hidden by its own clip.
/// 3. **Do the two states report opposite geometry?** `taking=0, shift=-n` while
///    pulled and `taking=n, shift=0` while refreshing. The same numbers for both
///    is the documented bug at the top of this file.
///
/// Change-only and behind an environment variable, for the reason
/// `FrameScheduler::trace` is: `stderr` is unbuffered and a syscall per layout
/// would itself change what is being measured.
fn trace_layout(overscroll: f32, room: f32, painted: f32, taking: f32, shift: f32) {
    use std::cell::Cell;
    use std::sync::OnceLock;

    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("VIEWW_TRACE_FRAMES").is_some()) {
        return;
    }

    thread_local! {
        static LAST: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
    }

    LAST.with(|last| {
        let moved = match last.get() {
            Some((was_overscroll, was_painted)) => {
                (overscroll - was_overscroll).abs() > 0.1 || (painted - was_painted).abs() > 0.1
            }
            None => true,
        };
        if moved {
            last.set(Some((overscroll, painted)));
            eprintln!(
                "refresh overscroll={overscroll:8.2} room={room:7.1} \
                 painted={painted:6.2} taking={taking:6.2} shift={shift:7.2}"
            );
        }
    });
}

impl RenderObject for RenderSliverRefresh {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Outside a viewport there is no overscroll to occupy, so it is as tall
        // as it is while refreshing and empty otherwise.
        let height = if self.refreshing {
            self.trigger_extent
        } else {
            0.0
        };
        let size = Size::new(constraints.max_width, height);
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
        self.pulled = constraints.overscroll;
        let room = constraints.remaining_paint_extent.max(0.0);

        let (painted, taking, shift) = if self.refreshing {
            // The gap has sprung shut, so make one.
            let extent = self.trigger_extent.min(room);
            (extent, extent, 0.0)
        } else {
            // The gap exists. Paint into it, and lift out of the content's way
            // — the viewport placed this sliver at the content's leading edge,
            // which is *below* the gap.
            let extent = constraints.overscroll.min(room);
            (extent, 0.0, -extent)
        };

        let child_size = constraints.size(painted, constraints.cross_axis_extent);
        if let Some(&child) = ctx.children().first() {
            ctx.layout_child(child, Constraints::tight(child_size));
            ctx.place_child(child, Offset::ZERO);
        }

        trace_layout(constraints.overscroll, room, painted, taking, shift);

        // Zero scroll extent: this is not part of the content, and counting it
        // would make the scrollbar report a page longer than the list and leave
        // a dead band at the top of every drag.
        Some(
            SliverGeometry::new(0.0, painted)
                .taking(taking)
                .shifted(shift),
        )
    }

    /// Zero along the main axis — a real zero — and `None` across it.
    ///
    /// # Zero is a fact here, not an unknown wearing a number
    ///
    /// The module documentation is emphatic that `Some(0.0)` must never stand in
    /// for "I do not know", so this needs a reason rather than an assertion, and
    /// it has one: **this control is not part of the content.** `layout_sliver`
    /// reports `SliverGeometry::new(0.0, painted)` — a scroll extent of exactly
    /// zero, deliberately, because counting it would make a scrollbar report a
    /// page longer than the list and leave a dead band at the top of every drag.
    /// The room the control occupies is borrowed: from the overscroll gap the
    /// viewport opened while the finger is down, and from the viewport itself
    /// while refreshing. It never adds to how long the page is.
    ///
    /// So a parent asking "how much room does your content need" is asking a
    /// question this object has a confident answer to, and the answer is none.
    /// An `IntrinsicHeight` over a list with a refresh control at the top should
    /// report the list, and this contributes nothing to it.
    ///
    /// # The two answers this refuses, and why
    ///
    /// **`trigger_extent` unconditionally** would be the box branch's height in
    /// its refreshing state, promoted to always. It is precisely the bug this
    /// file's own documentation names at the top: a permanent gap at the top of
    /// the list, 80 points of nothing reserved for a spinner that is not there,
    /// on every screen that measures its content.
    ///
    /// **`self.refreshing.then(...)`**, the exact mirror of the box branch,
    /// would agree with `layout` in both states and is still wrong. `refreshing`
    /// is runtime state — the application flips it when a request starts and
    /// again when it finishes — so the answer would change under a caller who
    /// asked twice, and an `Accordion` that measured mid-refresh would animate to
    /// a height that included a spinner about to vanish. The trait's contract is
    /// that an answer is a pure function of the subtree's *configuration*; a flag
    /// that tracks a network request in flight is not that. The same goes doubly
    /// for `layout_sliver`, which derives its extent from
    /// [`SliverConstraints::overscroll`] — how far the finger has dragged, this
    /// frame.
    ///
    /// Reporting zero in both states means the intrinsic and the box branch
    /// disagree while refreshing, and that disagreement is deliberate and
    /// harmless: it costs a transient spinner its reserved room in a measuring
    /// parent, which is the *smaller* of the two errors and the one that heals
    /// itself when the refresh ends.
    ///
    /// # Why the cross axis is `None`
    ///
    /// The box branch takes `constraints.max_width` and gives the child a tight
    /// size of it, so the control is as wide as it is told. There is no
    /// content-derived width, and no `Some(0.0)` here either — a zero across
    /// would be the unknown-as-zero mistake, since a spinner laid out at zero
    /// width is clipped away rather than absent by design.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        match query.axis {
            Axis::Vertical => Some(0.0),
            Axis::Horizontal => None,
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

    /// The same clip, for a spinner that owns a layer — see
    /// [`RenderObject::layer_clip`].
    ///
    /// Load-bearing here rather than defensive: the control is zero-height until
    /// it is pulled, so *the clip is the only thing hiding it at rest*. An
    /// indeterminate `CircularProgress` wraps itself in a `RepaintBoundary`, so
    /// without this the spinner is visible before any drag and never retracts.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let any: &dyn std::any::Any = new;
        match any.downcast_ref::<Self>() {
            Some(new) => {
                self.refreshing != new.refreshing
                    || (self.trigger_extent - new.trigger_extent).abs() > f32::EPSILON
            }
            None => true,
        }
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.pulled = old.pulled;
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderSliverRefresh"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderTree;
    use vieww_foundation::Axis;

    fn pulled_by(overscroll: f32) -> SliverConstraints {
        SliverConstraints {
            overscroll,
            ..SliverConstraints::initial(Axis::Vertical, 600.0, 400.0)
        }
    }

    fn laid_out(object: RenderSliverRefresh, constraints: &SliverConstraints) -> SliverGeometry {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.set_root(Some(id));
        tree.layout_sliver(id, constraints)
    }

    #[test]
    fn an_unpulled_control_is_not_there_at_all() {
        let geometry = laid_out(RenderSliverRefresh::new(80.0), &pulled_by(0.0));
        assert_eq!(geometry.paint_extent, 0.0);
        assert_eq!(geometry.layout_extent, 0.0);
    }

    #[test]
    fn being_pulled_paints_into_the_gap_and_takes_nothing() {
        // The viewport already made the room; taking more would push the
        // content down twice as fast as the finger moved.
        let geometry = laid_out(RenderSliverRefresh::new(80.0), &pulled_by(50.0));
        assert_eq!(geometry.paint_extent, 50.0);
        assert_eq!(geometry.layout_extent, 0.0);
        assert_eq!(
            geometry.paint_origin, -50.0,
            "lifted off the content's leading edge and into the gap above it"
        );
    }

    #[test]
    fn refreshing_makes_its_own_room_because_the_gap_has_closed() {
        // The opposite geometry, and the reason both states are one object: the
        // finger is gone and the overscroll has sprung back to zero.
        let geometry = laid_out(
            RenderSliverRefresh::new(80.0).refreshing(true),
            &pulled_by(0.0),
        );
        assert_eq!(geometry.paint_extent, 80.0);
        assert_eq!(
            geometry.layout_extent, 80.0,
            "or the spinner vanishes the instant you let go"
        );
        assert_eq!(geometry.paint_origin, 0.0);
    }

    #[test]
    fn it_never_consumes_any_scroll() {
        // Counting it would make the scrollbar report a page longer than the
        // list and leave a dead band at the top of every drag.
        for geometry in [
            laid_out(RenderSliverRefresh::new(80.0), &pulled_by(50.0)),
            laid_out(
                RenderSliverRefresh::new(80.0).refreshing(true),
                &pulled_by(0.0),
            ),
        ] {
            assert_eq!(geometry.scroll_extent, 0.0);
        }
    }

    #[test]
    fn the_pull_fraction_tracks_the_finger_rather_than_a_threshold() {
        let mut object = RenderSliverRefresh::new(80.0);
        object.pulled = 40.0;
        assert_eq!(object.pulled_fraction(), 0.5);
        assert!(!object.armed());

        object.pulled = 90.0;
        assert_eq!(object.pulled_fraction(), 1.0, "clamped, not unbounded");
        assert!(object.armed());
    }

    // ------------------------------------------------------------- intrinsics

    use crate::{IntrinsicQuery, RenderId};
    use vieww_foundation::{Constraints, Size};

    /// A control in a tree, never laid out, optionally with a spinner in it.
    fn mounted(object: RenderSliverRefresh, spinner: bool) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(object));
        tree.set_root(Some(root));
        if spinner {
            tree.insert(Some(root), Box::new(Unmeasurable));
        }
        (tree, root)
    }

    /// A spinner that answers nothing. The control's own answer must not depend
    /// on it — `layout` hands it a tight size and never asks.
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
    fn the_main_axis_is_zero_because_the_control_is_never_part_of_the_content() {
        // The zero that `layout_sliver` already reports as this object's scroll
        // extent, said again to the measuring pass: the room it uses is borrowed
        // from the overscroll gap or from the viewport, and it never lengthens
        // the page. A confident fact, not an unknown dressed as one.
        let (mut tree, root) = mounted(RenderSliverRefresh::new(80.0), true);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(0.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(0.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height().across(320.0)),
            Some(0.0),
            "a known width tells a fixed-trigger control nothing new"
        );

        let (mut childless, root) = mounted(RenderSliverRefresh::new(80.0), false);
        assert_eq!(
            childless.intrinsic(root, IntrinsicQuery::max_height()),
            Some(0.0)
        );
    }

    #[test]
    fn a_refreshing_control_still_reports_zero_because_refreshing_is_runtime_state() {
        // The deliberate disagreement with the box branch, pinned. Mirroring
        // `refreshing` here would make the answer change under a caller who
        // asked twice — the flag is flipped when a network request starts and
        // again when it ends — and the contract is that an answer is a pure
        // function of the subtree's configuration. An `Accordion` that measured
        // mid-refresh would otherwise animate to a height that included a
        // spinner about to disappear.
        let (mut tree, root) = mounted(RenderSliverRefresh::new(80.0).refreshing(true), true);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(0.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_height()),
            Some(0.0)
        );
    }

    #[test]
    fn the_answer_does_not_move_when_the_content_is_pulled() {
        // `layout_sliver` reads `constraints.overscroll` — how far the finger
        // has dragged, this frame — so an intrinsic derived from it would give
        // a different number sixty times a second during a pull.
        let (mut resting, root) = mounted(RenderSliverRefresh::new(80.0), false);
        let at_rest = resting.intrinsic(root, IntrinsicQuery::max_height());

        let (mut pulled, root) = mounted(RenderSliverRefresh::new(80.0), false);
        pulled.layout_sliver(root, &pulled_by(50.0));
        assert_eq!(
            pulled.intrinsic(root, IntrinsicQuery::max_height()),
            at_rest
        );
    }

    #[test]
    fn the_cross_axis_is_unknown_rather_than_zero() {
        // Zero across would be the unknown-as-zero mistake: the control is as
        // wide as it is told, and a spinner laid out at zero width is clipped
        // away rather than absent by design.
        let (mut tree, root) = mounted(RenderSliverRefresh::new(80.0), true);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), None);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), None);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width().across(80.0)),
            None
        );
    }

    #[test]
    fn the_intrinsic_is_the_height_the_box_layout_produces_at_rest() {
        // Where the agreement is meant to hold — and the state the control
        // spends essentially all of its life in. Outside a viewport an unpulled,
        // un-refreshing control is zero tall, and that is exactly what it
        // reports.
        let (mut tree, root) = mounted(RenderSliverRefresh::new(80.0), false);
        let wanted = tree
            .intrinsic(root, IntrinsicQuery::max_height())
            .expect("a refresh control knows it takes no room");

        let size = tree.layout_root(Constraints::new(0.0, 320.0, 0.0, f32::INFINITY));
        assert_eq!(size.height, wanted);
    }

    #[test]
    fn a_zero_trigger_never_arms_and_never_divides_by_zero() {
        let mut object = RenderSliverRefresh::new(0.0);
        object.pulled = 40.0;
        assert_eq!(object.pulled_fraction(), 0.0);
        assert!(!object.armed());
    }
}
