use vieww_foundation::{Axis, Constraints, Offset, Rect, Size};

use crate::sliver::{ScrollDirection, SliverConstraints};
use crate::{LayoutCtx, PaintCtx, RenderObject};

/// A viewport whose children are laid out under the sliver protocol.
///
/// The counterpart to [`RenderViewport`](crate::RenderViewport), which scrolls
/// **one** box child. This scrolls a *sequence* of slivers, and that is the
/// whole difference — but it is the difference between a scrolling list and a
/// screen with a collapsing header above three sections and a footer, sharing
/// one scroll position.
///
/// # The layout walk
///
/// Each sliver in turn is told how much of it has already scrolled past and how
/// much room is left, and answers with how much scroll it consumes and how much
/// of the viewport it covers. Those two are different numbers, and everything
/// interesting lives in the gap between them — see
/// [`sliver`](crate::sliver).
///
/// The walk stops as soon as nothing is left to paint. That is what makes a list
/// of ten thousand rows cost the twenty on screen: the slivers after the fold
/// are never asked, and the one that is asked is told exactly which window of
/// itself to build.
///
/// # Overscroll
///
/// A **negative** offset is legal and means the content has been dragged below
/// where it can go — the state an iOS rubber band is in, and the one a
/// pull-to-refresh control lives in. The content is shifted down by that much
/// and the gap opens above it; no sliver has to understand a negative scroll
/// position, because the first one is told about the gap through
/// [`SliverConstraints::overscroll`] and the rest never see it.
///
/// Past the **end** needs nothing special: the slivers run out, the tail scrolls
/// up, and the gap opens below on its own. The asymmetry is real — one end needs
/// a shift and the other falls out — and it is why only the leading case appears
/// in the code.
///
/// Where the offset *comes from* is still not this object's business:
/// [`ScrollPhysics`](vieww_gestures) decides how far past the end a drag may go
/// and how it springs back, and hands the result down. This one lays out
/// whatever number it is given.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderScrollView {
    pub axis: Axis,
    /// How far down the content the viewport has been scrolled.
    ///
    /// Negative means overscrolled past the start. See the type's own docs.
    pub offset: f32,
    /// Whether the slivers run from the far edge back toward the origin.
    ///
    /// A horizontal scroll view in a right-to-left interface. Like
    /// [`RenderViewport`](crate::RenderViewport), only the *placement* moves —
    /// every sliver is laid out against identical constraints and the scroll
    /// offset stays a distance into the content, so the sliver protocol needs
    /// no notion of direction and neither do the physics.
    pub reverse: bool,
    /// Filled in by layout: the total scroll extent of every sliver.
    ///
    /// What a scrollbar and the physics need, and the reason this is a field
    /// rather than a return value — layout returns a `Size`, and the content
    /// extent is not it.
    content_extent: f32,
    /// Filled in by layout: how much of the viewport the content falls short
    /// of, if it does.
    viewport_extent: f32,
    /// The offset the previous layout ran at, for deriving a direction.
    ///
    /// `None` before the first layout, which is what makes the first frame
    /// [`Idle`](crate::sliver::ScrollDirection::Idle) rather than a spurious
    /// scroll toward the end from zero.
    last_offset: Option<f32>,
    /// Filled in by layout, and carried across rebuilds.
    direction: ScrollDirection,
}

impl RenderScrollView {
    #[must_use]
    pub const fn new(axis: Axis) -> Self {
        Self {
            axis,
            offset: 0.0,
            reverse: false,
            content_extent: 0.0,
            viewport_extent: 0.0,
            last_offset: None,
            direction: ScrollDirection::Idle,
        }
    }

    #[must_use]
    pub const fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// Run the slivers from the far edge back toward the origin.
    ///
    /// See [`reverse`](Self::reverse). Set by the factory for a horizontal
    /// scroll view under a right-to-left `Directionality`.
    #[must_use]
    pub const fn reversed(mut self, reverse: bool) -> Self {
        self.reverse = reverse;
        self
    }

    /// The total scrollable extent measured by the last layout.
    #[must_use]
    pub const fn content_extent(&self) -> f32 {
        self.content_extent
    }

    /// Which way the scroll last moved.
    #[must_use]
    pub const fn direction(&self) -> ScrollDirection {
        self.direction
    }

    /// How far past the start the content has been dragged.
    #[must_use]
    pub fn overscroll(&self) -> f32 {
        (-self.offset).max(0.0)
    }

    /// The furthest this viewport can be scrolled.
    ///
    /// Zero when the content fits, which is what stops a short page scrolling
    /// at all.
    #[must_use]
    pub fn max_scroll_extent(&self) -> f32 {
        (self.content_extent - self.viewport_extent).max(0.0)
    }
}

impl RenderObject for RenderScrollView {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let size = constraints.biggest();
        let (main, cross) = match self.axis {
            Axis::Vertical => (size.height, size.width),
            Axis::Horizontal => (size.width, size.height),
        };

        self.viewport_extent = main;

        // Where the walk currently is in *scroll* space, and where the next
        // sliver's paint lands in *viewport* space. They diverge the moment a
        // sliver's layout extent differs from its paint extent, which is what a
        // pinned header does — and keeping them as two variables is the whole
        // trick.
        let mut consumed_scroll = 0.0_f32;
        let mut overlap = 0.0_f32;

        self.direction = match self.last_offset {
            Some(last) => ScrollDirection::between(last, self.offset),
            // The first layout is not a scroll. Reporting one would make a
            // floating header slide in on the frame the screen appears.
            None => ScrollDirection::Idle,
        };
        self.last_offset = Some(self.offset);

        // Dragged past the start: the content sits this far down and a gap
        // opens above it. The whole of overscroll is these two lines — every
        // sliver still sees a non-negative scroll offset, and only the first is
        // told the gap exists.
        let overscroll = (-self.offset).max(0.0);
        let mut placed_at = overscroll;

        let offset = self.offset.max(0.0);

        for (index, child) in ctx.children_owned().into_iter().enumerate() {
            let remaining = (main - placed_at).max(0.0);

            let constraints = SliverConstraints {
                axis: self.axis,
                // How much of *this* sliver is already above the fold: the
                // scroll position minus everything before it, floored at zero
                // for a sliver the scroll has not reached.
                scroll_offset: (offset - consumed_scroll).max(0.0),
                remaining_paint_extent: remaining,
                viewport_extent: main,
                cross_axis_extent: cross,
                overlap,
                preceding_scroll_extent: consumed_scroll,
                scroll_direction: self.direction,
                // Only the first sliver: the gap is above the content, so it is
                // above exactly one of them, and telling the rest would have a
                // refresh control halfway down the page think it was being
                // pulled.
                overscroll: if index == 0 { overscroll } else { 0.0 },
            };

            let geometry = ctx.layout_sliver_child(child, &constraints);

            // Placed at the top of its own visible band, then shifted by
            // whatever the sliver asked for. A pinned header needs no shift —
            // it stops contributing to `placed_at`, so it is already at the
            // leading edge. A refresh control needs a negative one, to reach the
            // overscroll gap that opened *above* the content.
            // `at` is already in viewport space — distance from the leading
            // edge — so reversing is a mirror *within the viewport* and needs
            // no total content extent. That is why the sliver protocol is
            // untouched: a sliver is laid out identically either way and never
            // learns which end of the screen it landed on.
            let at = placed_at + geometry.paint_origin;
            let at = if self.reverse {
                main - at - geometry.paint_extent
            } else {
                at
            };
            ctx.place_child(child, offset_along(self.axis, at));

            consumed_scroll += geometry.scroll_extent;
            placed_at += geometry.layout_extent;
            // A sliver painting more than it took is covering what comes next.
            overlap = (geometry.paint_extent - geometry.layout_extent).max(0.0);

            // Nothing left to paint into, and every remaining sliver would be
            // told so. This is where a list of ten thousand rows stops costing
            // ten thousand rows.
            if placed_at >= main {
                break;
            }
        }

        self.content_extent = consumed_scroll;
        size
    }

    /// The largest sliver across the scroll axis, and `None` along it.
    ///
    /// # The main axis, and the failure it refuses
    ///
    /// `layout` returns [`Constraints::biggest`] unconditionally. Not "the
    /// content, clamped" and not "the content if it fits" — the biggest thing
    /// allowed, every time. So this object has no content-derived main extent to
    /// report, and the number that *looks* like one is a trap.
    ///
    /// That number would be `content_extent`: the sum of every sliver's scroll
    /// extent, which is sitting right there in a field. Reporting it would put
    /// an `IntrinsicHeight` around a scroll view into the position of reserving
    /// **the entire scrollable length** — five hundred thousand points for a
    /// list of ten thousand rows — and then tightening the view to it. Every row
    /// would be on screen at once, the walk in `layout` would stop breaking
    /// early because nothing would ever fill up, and the virtualisation this
    /// whole file exists to provide would be gone. A scroll view measured that
    /// way does not scroll: it is just a very long column, laid out in full,
    /// every frame.
    ///
    /// It is worse than merely useless, too, because the field is written *by*
    /// layout. Reading it from an intrinsic would answer with whatever the last
    /// layout happened to see — a different number before the first frame, after
    /// a scroll that revealed a new sliver, or after a rebuild — and the
    /// contract on [`RenderObject::intrinsic`] is that an answer is a pure
    /// function of the subtree's configuration. Two callers asking the same
    /// question at different moments would get different answers.
    ///
    /// `None` says the honest thing: this axis is not content-sized, so the
    /// asking widget should be transparent and the view should take the main
    /// extent it is offered. `Some(0.0)` would instead claim it wants no room at
    /// all and collapse the whole screen — the module's cardinal error.
    ///
    /// # The cross axis
    ///
    /// Genuinely meaningful, and the mirror of
    /// [`RenderViewport`](crate::RenderViewport)'s: every sliver is laid out
    /// with `cross_axis_extent` equal to this view's own cross extent, so a
    /// parent that tightens this object to the widest sliver hands each sliver
    /// exactly that. The largest of the children, with `largest` poisoning to
    /// `None` if any one of them cannot answer — an unmeasurable sliver could be
    /// the widest, and skipping it reports a view narrower than its own contents.
    ///
    /// **Every child is asked, not just the ones on screen.** `layout` stops
    /// walking once the viewport is full, and that early exit is a function of
    /// the scroll offset, which an intrinsic must not read. Asking all of them
    /// costs more and is the only answer that does not change as the user
    /// scrolls.
    ///
    /// `cross` is dropped on the way down for the reason it is in a box
    /// viewport: the caller's value is an extent along the *scroll* axis, and
    /// the content is free to be longer than it.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        if query.axis == self.axis {
            return None;
        }

        let children = ctx.children_owned();
        if children.is_empty() {
            // No slivers is no content, and zero across is what that is — the
            // same answer an empty `RenderFlex` gives, and a fact rather than a
            // shrug.
            return Some(0.0);
        }

        let mut inner = query;
        inner.cross = None;
        let answers: Vec<Option<f32>> = children
            .into_iter()
            .map(|child| ctx.child_intrinsic(child, inner))
            .collect();
        crate::largest(answers)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // A viewport shows a window onto something larger by definition, so the
        // clip is not an optimisation.
        let bounds = ctx.bounds();
        ctx.canvas().save();
        ctx.canvas().clip_rect(bounds);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The same clip, for the boundaries below that `paint` cannot reach — see
    /// [`RenderObject::layer_clip`]. A sliver that owns a layer is exactly as
    /// far outside the window as one that does not.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    fn handle_scroll(&self, _event: &vieww_foundation::ScrollEvent) -> bool {
        // The offset is owned by a controller in the widget layer, exactly as
        // `RenderViewport`'s is — this object is told where it is, and does not
        // decide. Returning false lets the event reach whatever does.
        false
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // A changed scroll offset relays the children out, because which of
        // them are visible is a function of it. A changed *measurement* — the
        // two fields layout itself writes — must not, or every layout would
        // pending the object that just produced it.
        //
        // `reverse` belongs with the axis rather than with the measurements: it
        // decides where every sliver is placed, so a subtree that gains a
        // `Directionality` has to be laid out again even though its axis and
        // offset are unchanged.
        let any: &dyn std::any::Any = new;
        match any.downcast_ref::<Self>() {
            Some(new) => {
                self.axis != new.axis
                    || self.reverse != new.reverse
                    || (self.offset - new.offset).abs() > f32::EPSILON
            }
            None => true,
        }
    }

    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        // The measurements survive a rebuild, so a scrollbar built from
        // `content_extent` does not flicker to zero on the frame the tree is
        // replaced and before layout has run again.
        //
        // `last_offset` is here for a sharper reason: a scroll rebuilds the
        // tree, so without this every frame of a drag would arrive with no
        // previous offset and report `Idle` — and a floating header would never
        // move, on the exact gesture it exists to answer.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.content_extent = old.content_extent;
            self.viewport_extent = old.viewport_extent;
            self.last_offset = old.last_offset;
            self.direction = old.direction;
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderScrollView"
    }
}

fn offset_along(axis: Axis, main: f32) -> Offset {
    match axis {
        Axis::Vertical => Offset::new(0.0, main),
        Axis::Horizontal => Offset::new(main, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::RenderSliverFixedList;
    use crate::RenderTree;

    /// A tree with a scroll view over `slivers`, laid out at `size`.
    fn scrolled(
        view: RenderScrollView,
        slivers: Vec<Box<dyn RenderObject>>,
        size: Size,
    ) -> (RenderTree, crate::RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(view));
        tree.set_root(Some(root));
        for sliver in slivers {
            tree.insert(Some(root), sliver);
        }
        tree.layout_root(Constraints::tight(size));
        (tree, root)
    }

    fn list(rows: usize, height: f32) -> Box<dyn RenderObject> {
        Box::new(RenderSliverFixedList::new(rows, height))
    }

    #[test]
    fn several_slivers_share_one_scroll_position() {
        // The thing a box viewport cannot do at all: two independently-shaped
        // sections scrolling as one.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical),
            vec![list(3, 50.0), list(4, 20.0)],
            Size::new(300.0, 400.0),
        );

        let view = tree
            .object(root)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderScrollView>()
            })
            .expect("the view");
        assert_eq!(
            view.content_extent(),
            3.0 * 50.0 + 4.0 * 20.0,
            "the content is the sum of the slivers, not the tallest of them"
        );
    }

    #[test]
    fn a_short_page_cannot_be_scrolled() {
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical),
            vec![list(2, 50.0)],
            Size::new(300.0, 400.0),
        );
        let view = tree
            .object(root)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderScrollView>()
            })
            .expect("the view");
        assert_eq!(view.max_scroll_extent(), 0.0);
    }

    #[test]
    fn the_walk_stops_once_the_viewport_is_full() {
        // Ten slivers, a viewport that fits two. The rest must never be laid
        // out — this is the property that makes a long page cheap, and it is
        // invisible in a screenshot, so it needs a test.
        let slivers: Vec<Box<dyn RenderObject>> = (0..10).map(|_| list(1, 200.0)).collect();
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical),
            slivers,
            Size::new(300.0, 400.0),
        );

        let laid_out = tree
            .children(root)
            .iter()
            .filter(|&&child| tree.layout_count(child) > 0)
            .count();
        assert!(
            laid_out <= 3,
            "{laid_out} of 10 slivers were laid out for a viewport that fits 2"
        );
    }

    /// The window a mounted sliver list last asked for.
    fn window_of(tree: &RenderTree, id: crate::RenderId) -> Option<(usize, usize)> {
        tree.object(id)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderSliverFixedList>()
            })
            .and_then(RenderSliverFixedList::visible_range)
    }

    #[test]
    fn scrolling_past_one_sliver_carries_into_the_next() {
        // The first list is 100 tall and entirely above the fold, so 220 of
        // scroll leaves 120 for the second — three rows of 50, less a row of
        // slack. Asserting on the *window* rather than on a placement is what
        // makes this a test of the walk rather than of a coordinate convention.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(220.0),
            vec![list(2, 50.0), list(10, 50.0)],
            Size::new(300.0, 400.0),
        );

        assert_eq!(
            window_of(&tree, tree.children(root)[1]).map(|window| window.0),
            Some(1),
            "row 2 is at the top, and one row of slack is kept above it"
        );
    }

    #[test]
    fn a_sliver_sits_at_the_top_of_its_visible_band_rather_than_where_its_content_starts() {
        // The coordinate convention, pinned. A scrolled sliver is placed at the
        // top of what it paints, and its *rows* carry the scroll — because it
        // clips to its own paint extent, and a clip placed where the content
        // starts would sit above the viewport and cut off the very rows it was
        // meant to show.
        //
        // Invisible at offset zero, which is where every first look happens.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(220.0),
            vec![list(2, 50.0), list(10, 50.0)],
            Size::new(300.0, 400.0),
        );

        assert_eq!(
            tree.offset(tree.children(root)[1]).dy,
            0.0,
            "the second sliver paints from the top of the viewport"
        );
    }

    #[test]
    fn overscrolling_past_the_start_pushes_the_content_down() {
        // A negative offset is the state an iOS rubber band is in, and the one
        // a pull-to-refresh control lives in. The content moves down and a gap
        // opens above it.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(-50.0),
            vec![list(4, 50.0)],
            Size::new(300.0, 400.0),
        );
        assert_eq!(
            tree.offset(tree.children(root)[0]).dy,
            50.0,
            "the first sliver starts below the top, and the gap is the pull"
        );
    }

    #[test]
    fn no_sliver_ever_sees_a_negative_scroll_offset() {
        // The invariant that keeps overscroll from leaking into every sliver:
        // the viewport absorbs it as a placement, and the protocol stays
        // non-negative. `RenderSliverFixedList::window` divides by the item
        // extent, and a negative numerator there is a panic in release.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(-120.0),
            vec![list(10, 50.0)],
            Size::new(300.0, 400.0),
        );
        let first = tree.children(root)[0];
        let sliver = tree
            .object(first)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderSliverFixedList>()
            })
            .expect("the list");
        assert_eq!(sliver.visible_range().map(|range| range.0), Some(0));
    }

    #[test]
    fn overscrolling_past_the_end_needs_no_shift_at_all() {
        // The asymmetry worth a test: the leading end needs the viewport to
        // move everything down, and the trailing end needs nothing — the
        // slivers simply run out of visible extent and the gap opens below on
        // its own. That is why only the leading case appears in the code.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(10_000.0),
            vec![list(4, 50.0)],
            Size::new(300.0, 400.0),
        );
        assert_eq!(
            tree.size(tree.children(root)[0]).height,
            0.0,
            "scrolled entirely past, so it paints nothing and needs no shifting"
        );
    }

    #[test]
    fn the_first_layout_is_not_a_scroll() {
        // Reporting one would slide a floating header in on the frame the
        // screen appears.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical).offset(100.0),
            vec![list(20, 50.0)],
            Size::new(300.0, 400.0),
        );
        let view = tree
            .object(root)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderScrollView>()
            })
            .expect("the view");
        assert_eq!(view.direction(), ScrollDirection::Idle);
    }

    #[test]
    fn a_second_layout_at_a_larger_offset_reports_scrolling_toward_the_end() {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderScrollView::new(Axis::Vertical)));
        tree.set_root(Some(root));
        tree.insert(Some(root), list(20, 50.0));

        tree.layout_root(Constraints::tight(Size::new(300.0, 400.0)));
        tree.replace_object(
            root,
            Box::new(RenderScrollView::new(Axis::Vertical).offset(80.0)),
        );
        tree.layout_root(Constraints::tight(Size::new(300.0, 400.0)));

        let view = tree
            .object(root)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderScrollView>()
            })
            .expect("the view");
        assert_eq!(
            view.direction(),
            ScrollDirection::TowardEnd,
            "a scroll rebuilds the tree, so the previous offset has to survive \
             the replacement or every frame of a drag reports Idle"
        );
    }

    #[test]
    fn a_box_child_needs_no_adapter() {
        // The claim in `RenderObject::layout_sliver`'s docs, tested: an object
        // that has never heard of slivers goes into a sliver viewport.
        let (tree, root) = scrolled(
            RenderScrollView::new(Axis::Vertical),
            vec![Box::new(crate::RenderConstrainedBox::new(
                Constraints::tight(Size::new(300.0, 150.0)),
            ))],
            Size::new(300.0, 400.0),
        );

        let view = tree
            .object(root)
            .and_then(|object| {
                let any: &dyn std::any::Any = object;
                any.downcast_ref::<RenderScrollView>()
            })
            .expect("the view");
        assert_eq!(
            view.content_extent(),
            150.0,
            "a box child is adapted to a sliver of exactly its own size"
        );
    }

    #[test]
    fn a_changed_offset_relayouts_and_a_changed_measurement_does_not() {
        let mut measured = RenderScrollView::new(Axis::Vertical);
        measured.content_extent = 5_000.0;

        assert!(
            !RenderScrollView::new(Axis::Vertical).layout_differs(&measured),
            "layout writing its own measurement must not pending the object that \
             just produced it"
        );
        assert!(
            RenderScrollView::new(Axis::Vertical)
                .layout_differs(&RenderScrollView::new(Axis::Vertical).offset(10.0)),
            "but which children are visible is a function of the offset"
        );
    }

    #[test]
    fn a_reversed_scroll_view_tiles_its_slivers_from_the_far_edge() {
        // 400 of window and two lists of 150 each. Forward they tile rightward
        // from zero; reversed they tile leftward from 400 — and the sliver that
        // ends up flush right is the **first** one, which is where reading
        // begins in Arabic.
        let (forward, root) = scrolled(
            RenderScrollView::new(Axis::Horizontal),
            vec![list(3, 50.0), list(3, 50.0)],
            Size::new(400.0, 300.0),
        );
        let kids = forward.children(root).to_vec();
        assert_eq!(forward.offset(kids[0]).dx, 0.0);
        assert_eq!(forward.offset(kids[1]).dx, 150.0);

        let (reversed, root) = scrolled(
            RenderScrollView::new(Axis::Horizontal).reversed(true),
            vec![list(3, 50.0), list(3, 50.0)],
            Size::new(400.0, 300.0),
        );
        let kids = reversed.children(root).to_vec();
        assert_eq!(
            reversed.offset(kids[0]).dx,
            250.0,
            "the first sliver is flush against the right edge: 400 - 0 - 150"
        );
        assert_eq!(
            reversed.offset(kids[1]).dx,
            100.0,
            "and the second tiles to its left rather than past the edge"
        );
    }

    #[test]
    fn reversing_is_a_mirror_and_changes_no_sliver_geometry() {
        // The claim that keeps the sliver protocol out of this: every sliver is
        // laid out against identical constraints either way and reports the
        // same geometry. Only `place_child` sees a different number, so a
        // sliver never learns which end of the screen it landed on.
        let extents = |reverse: bool| {
            let (tree, root) = scrolled(
                RenderScrollView::new(Axis::Horizontal).reversed(reverse),
                vec![list(3, 50.0), list(10, 50.0)],
                Size::new(400.0, 300.0),
            );
            let kids = tree.children(root).to_vec();
            kids.iter().map(|&id| tree.size(id)).collect::<Vec<_>>()
        };
        assert_eq!(extents(false), extents(true));
    }

    // ------------------------------------------------------------- intrinsics

    use crate::{IntrinsicQuery, RenderConstrainedBox, RenderId};

    /// A scroll view over `slivers`, **not** laid out.
    ///
    /// Never laid out on purpose. Every other helper in this file lays out
    /// first, and an intrinsic that only worked afterwards would be no use to
    /// the callers that need one — and would quietly be reading the fields
    /// layout writes.
    fn unlaid(
        view: RenderScrollView,
        slivers: Vec<Box<dyn RenderObject>>,
    ) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(view));
        tree.set_root(Some(root));
        for sliver in slivers {
            tree.insert(Some(root), sliver);
        }
        (tree, root)
    }

    /// A box child of a fixed size, which every axis can measure.
    fn box_child(size: Size) -> Box<dyn RenderObject> {
        Box::new(RenderConstrainedBox::new(Constraints::tight(size)))
    }

    #[test]
    fn the_cross_axis_is_the_widest_sliver() {
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![
                box_child(Size::new(120.0, 400.0)),
                box_child(Size::new(200.0, 400.0)),
            ],
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width()),
            Some(200.0)
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::min_width()),
            Some(200.0)
        );

        // Horizontal scroll: the meaningful axis is the height instead.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Horizontal),
            vec![
                box_child(Size::new(400.0, 120.0)),
                box_child(Size::new(400.0, 200.0)),
            ],
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            Some(200.0)
        );
    }

    #[test]
    fn the_main_axis_is_unknown_because_reporting_the_content_would_defeat_scrolling() {
        // The most valuable `None` in the crate. Ten thousand rows of 50 is
        // 500 000 points of content; an `IntrinsicHeight` that read it would
        // tighten this view to 500 000, every row would be on screen, the walk
        // in `layout` would never break early, and the virtualisation would be
        // gone. `layout` returns `constraints.biggest()` and never the content,
        // so there is nothing here to report.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![list(10_000, 50.0)],
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height()),
            None,
            "and not Some(0.0), which would collapse the screen instead"
        );
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_height()), None);
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_height().across(300.0)),
            None,
            "knowing the width does not make the main axis content-sized"
        );
    }

    #[test]
    fn a_known_extent_along_the_scroll_axis_does_not_narrow_the_cross_answer() {
        // The content may be longer than the window — that is the point — so a
        // sliver is not asked how wide it would be if squeezed into 300 of
        // scroll, because it never is.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![box_child(Size::new(200.0, 4_000.0))],
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width().across(300.0)),
            Some(200.0)
        );
    }

    #[test]
    fn an_empty_scroll_view_has_no_cross_extent_and_still_no_main_one() {
        let (mut tree, root) = unlaid(RenderScrollView::new(Axis::Vertical), vec![]);
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_width()), Some(0.0));
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::min_width()), Some(0.0));
        assert_eq!(tree.intrinsic(root, IntrinsicQuery::max_height()), None);
    }

    #[test]
    fn an_unmeasurable_sliver_poisons_the_cross_axis_rather_than_being_skipped() {
        // It could be the widest one, and skipping it reports a view narrower
        // than its own contents.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![box_child(Size::new(120.0, 400.0)), Box::new(Unmeasurable)],
        );
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
    fn the_slivers_below_the_fold_are_measured_even_though_layout_never_reaches_them() {
        // `layout` stops walking once the viewport is full, and that early exit
        // is a function of the scroll offset — which an intrinsic must not read,
        // or the reported width would change as the user scrolled. The widest
        // sliver here is the last one, far past the fold.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![
                box_child(Size::new(120.0, 4_000.0)),
                box_child(Size::new(360.0, 400.0)),
            ],
        );
        assert_eq!(
            tree.intrinsic(root, IntrinsicQuery::max_width()),
            Some(360.0)
        );
    }

    #[test]
    fn tightening_to_the_cross_intrinsic_produces_exactly_that_width_in_layout() {
        // The agreement that justifies forwarding the cross axis, and the
        // disagreement the main axis would have: laid out at the reported
        // width the view *is* that wide, and it is still only as tall as the
        // window rather than as tall as its content.
        let (mut tree, root) = unlaid(
            RenderScrollView::new(Axis::Vertical),
            vec![box_child(Size::new(200.0, 4_000.0))],
        );
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
            tree.size(root).height,
            300.0,
            "the window, not the 4000 of content — which is why the main axis \
             reports nothing"
        );
    }

    #[test]
    fn gaining_a_reading_direction_relayouts_the_slivers() {
        // Same axis, same offset, every sliver somewhere else. Without this the
        // placement would survive a `Directionality` appearing above it.
        let ltr = RenderScrollView::new(Axis::Horizontal);
        let rtl = RenderScrollView::new(Axis::Horizontal).reversed(true);
        assert!(ltr.layout_differs(&rtl));
        assert!(rtl.layout_differs(&ltr));
    }
}
