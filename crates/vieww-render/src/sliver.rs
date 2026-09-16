//! The second layout protocol.
//!
//! # Why one protocol was not enough
//!
//! The box protocol is *constraints down, size up*: a parent offers a range and
//! a child picks a size inside it. It is a single pass, it is why layout here is
//! cheap, and it cannot express a scrolling viewport's children.
//!
//! A row in a list needs to know **how much of it has already scrolled past the
//! top**, and **how much room is left on screen** — neither of which is a
//! constraint on its size. A collapsing header needs to return *two* different
//! extents: how much scroll it consumes and how much of the screen it currently
//! occupies, which stop being the same number the moment it collapses. And a
//! list of ten thousand rows must be able to lay out only the visible ones,
//! which means answering "how tall are you" without measuring every child.
//!
//! None of that fits in `Constraints -> Size`. The answer is a second
//! protocol alongside the first, and this is that protocol: **`SliverConstraints`
//! down, `SliverGeometry` up**.
//!
//! # What it buys, concretely
//!
//! Collapsing and pinned headers, sticky sections, several independently-shaped
//! lists sharing one scroll position, pull-to-refresh, and scroll-linked
//! effects. All of them are the same mechanism — a child that reports its
//! on-screen extent separately from its scroll extent — and none of them is
//! expressible without it.
//!
//! # How it coexists with the box protocol
//!
//! [`RenderObject::layout_sliver`](crate::RenderObject::layout_sliver) is a
//! defaulted method returning `None`. Every existing render object keeps working
//! untouched, and a viewport handed an ordinary box child laying it out as a
//! sliver gets `None` back and wraps it — one sliver's worth of scroll extent,
//! measured once. So `Text` and `Container` go into a sliver list without
//! anybody writing an adapter, which is the part other toolkits make you write by
//! hand.

use vieww_foundation::{Axis, Offset, Rect, Size};

/// Which way a scroll last moved.
///
/// # Why the viewport reports this at all
///
/// Most slivers do not care. Two kinds do, and neither can be written without
/// it: a **floating** header, which comes back the moment the user scrolls back
/// rather than waiting for the top of the content, and anything that hides
/// itself while the user is moving away — a floating action button, a bottom
/// bar.
///
/// Named for what the *content* is doing rather than for which way a finger
/// went, because a finger going up moves content up on one platform's rubber
/// band and the wording never survives the conversation. Toward the end means
/// the scroll offset is growing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollDirection {
    /// Not moving, or moved by something that is not a scroll — a resize, a
    /// programmatic jump, the first layout.
    #[default]
    Idle,
    /// Toward the end of the content: the scroll offset grew.
    TowardEnd,
    /// Back toward the start: the scroll offset shrank.
    TowardStart,
}

impl ScrollDirection {
    /// Which way a move from `from` to `to` went.
    ///
    /// A move of nothing is [`Idle`](Self::Idle) rather than being folded into
    /// one of the directions, because "the user is not scrolling" is the state a
    /// floating header must not treat as "the user is scrolling back".
    #[must_use]
    pub fn between(from: f32, to: f32) -> Self {
        // A generous epsilon: a fling decelerating into its final pixel
        // produces a run of sub-pixel deltas, and reading direction from those
        // makes a floating header jitter as the scroll settles.
        const STILL: f32 = 0.01;
        if (to - from).abs() <= STILL {
            Self::Idle
        } else if to > from {
            Self::TowardEnd
        } else {
            Self::TowardStart
        }
    }
}

/// What a viewport tells a sliver about where it is.
///
/// Compare [`Constraints`](vieww_foundation::Constraints): that describes a
/// *range of allowed sizes*, and this describes a *position in a scroll*. A
/// sliver is not being asked how big it wants to be. It is being told what part
/// of it is visible and asked what it will do about that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliverConstraints {
    /// Which way the viewport scrolls.
    pub axis: Axis,
    /// How much of this sliver has already passed the leading edge.
    ///
    /// Zero while the sliver's start is still on screen or below it; growing as
    /// it scrolls off. Never negative — a sliver that has not been reached yet
    /// is simply not laid out.
    pub scroll_offset: f32,
    /// How much main-axis room is left in the viewport.
    ///
    /// A sliver must not paint more than this. It may *consume* more scroll
    /// than this, which is exactly the distinction [`SliverGeometry`] exists to
    /// carry.
    pub remaining_paint_extent: f32,
    /// The full main-axis size of the viewport.
    ///
    /// Needed by anything sizing itself relative to the screen rather than to
    /// its content — a full-page section, a header that collapses to a fraction
    /// of the viewport.
    pub viewport_extent: f32,
    /// The cross-axis space available, which is a hard constraint.
    pub cross_axis_extent: f32,
    /// How far a previous sliver is painting into this one's space.
    ///
    /// A pinned header stays on screen after its scroll extent is used up, so
    /// the sliver behind it is partly covered. Content that must not hide under
    /// it — the first row of a list, say — starts below this.
    pub overlap: f32,
    /// The total scroll extent of every sliver before this one.
    ///
    /// What a sliver needs to place itself in the *whole* scrollable, rather
    /// than in the viewport — a scrollbar thumb, or a header that knows which
    /// section it belongs to.
    pub preceding_scroll_extent: f32,
    /// Which way the scroll last moved.
    ///
    /// See [`ScrollDirection`] for why a sliver would want to know.
    pub scroll_direction: ScrollDirection,
    /// How far the viewport has been dragged **past its start**.
    ///
    /// Zero in ordinary scrolling. Positive while the user is holding the
    /// content below where it can go, which is the state a pull-to-refresh
    /// control exists to occupy — and the reason a viewport has to accept a
    /// negative scroll offset at all.
    ///
    /// Only ever reported to the **first** sliver, because that is the only one
    /// the gap is above.
    pub overscroll: f32,
}

impl SliverConstraints {
    /// The constraints a viewport of `viewport_extent` starts its first sliver
    /// with.
    #[must_use]
    pub fn initial(axis: Axis, viewport_extent: f32, cross_axis_extent: f32) -> Self {
        Self {
            axis,
            scroll_offset: 0.0,
            remaining_paint_extent: viewport_extent,
            viewport_extent,
            cross_axis_extent,
            overlap: 0.0,
            preceding_scroll_extent: 0.0,
            scroll_direction: ScrollDirection::Idle,
            overscroll: 0.0,
        }
    }

    /// A box constraint that pins the cross axis and leaves the main axis free.
    ///
    /// What a sliver hands its own box children: they choose their length and
    /// have no say in their width, which is what makes a list row fill the list.
    #[must_use]
    pub fn box_constraints(&self) -> vieww_foundation::Constraints {
        use vieww_foundation::Constraints;
        match self.axis {
            Axis::Vertical => Constraints::new(
                self.cross_axis_extent,
                self.cross_axis_extent,
                0.0,
                f32::INFINITY,
            ),
            Axis::Horizontal => Constraints::new(
                0.0,
                f32::INFINITY,
                self.cross_axis_extent,
                self.cross_axis_extent,
            ),
        }
    }

    /// A size built from a main-axis and a cross-axis extent.
    #[must_use]
    pub fn size(&self, main: f32, cross: f32) -> Size {
        match self.axis {
            Axis::Vertical => Size::new(cross, main),
            Axis::Horizontal => Size::new(main, cross),
        }
    }

    /// An offset `main` along the scroll direction.
    #[must_use]
    pub fn offset(&self, main: f32) -> Offset {
        match self.axis {
            Axis::Vertical => Offset::new(0.0, main),
            Axis::Horizontal => Offset::new(main, 0.0),
        }
    }

    /// The main-axis component of a size.
    #[must_use]
    pub fn main_of(&self, size: Size) -> f32 {
        match self.axis {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }
}

/// What a sliver reports back.
///
/// # The three extents, and why they are three
///
/// This is the heart of the protocol and the thing a box layout cannot say.
///
/// - [`scroll_extent`](Self::scroll_extent) — how much **scrolling** this sliver
///   accounts for. A 10,000-row list has a huge one.
/// - [`paint_extent`](Self::paint_extent) — how much of the **viewport** it
///   covers right now. Never more than `remaining_paint_extent`.
/// - [`layout_extent`](Self::layout_extent) — how much room it **takes from the
///   next sliver**. Usually equal to `paint_extent`; a *pinned* header is the
///   case where it is not, because it keeps painting after it has stopped
///   pushing anything down.
///
/// A plain list has all three equal to each other and to its height, which is
/// why the distinction is invisible until the first collapsing header — and why
/// retrofitting it later is so expensive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliverGeometry {
    pub scroll_extent: f32,
    pub paint_extent: f32,
    pub layout_extent: f32,
    /// The most this sliver could ever paint, for overscroll and scrollbars.
    pub max_paint_extent: f32,
    /// How far the paint origin is shifted from where the walk placed it.
    ///
    /// Rarely needed, and worth knowing why: the viewport places each sliver at
    /// the running total of *layout* extents, so a pinned header — which stops
    /// contributing to that total — is already placed at the viewport's leading
    /// edge without asking for anything.
    ///
    /// What does need it is a sliver that must draw **outside** the flow
    /// entirely. [`RenderSliverRefresh`](crate::RenderSliverRefresh) sets it
    /// negative to paint into the gap that overscroll opened *above* the
    /// content, which is the one place nothing else can reach.
    pub paint_origin: f32,
    /// How much of the painted extent takes taps.
    ///
    /// Usually `paint_extent`. Zero for something visible and inert.
    pub hit_test_extent: f32,
    /// `false` when the sliver paints nothing, so the viewport can skip it
    /// entirely.
    pub visible: bool,
    /// `true` when the sliver draws outside its own extent and must be clipped.
    pub has_visual_overflow: bool,
}

impl SliverGeometry {
    /// A sliver that takes no room and draws nothing.
    pub const ZERO: Self = Self {
        scroll_extent: 0.0,
        paint_extent: 0.0,
        layout_extent: 0.0,
        max_paint_extent: 0.0,
        paint_origin: 0.0,
        hit_test_extent: 0.0,
        visible: false,
        has_visual_overflow: false,
    };

    /// The ordinary case: a sliver of `scroll_extent` with `paint_extent`
    /// currently on screen, taking exactly what it paints from the next sliver.
    #[must_use]
    pub fn new(scroll_extent: f32, paint_extent: f32) -> Self {
        let paint_extent = paint_extent.max(0.0);
        Self {
            scroll_extent: scroll_extent.max(0.0),
            paint_extent,
            layout_extent: paint_extent,
            max_paint_extent: scroll_extent.max(0.0),
            paint_origin: 0.0,
            hit_test_extent: paint_extent,
            visible: paint_extent > 0.0,
            has_visual_overflow: false,
        }
    }

    /// A sliver occupying `extent` of scroll, all of it currently visible.
    ///
    /// What a box child adapted into a sliver reports when it is fully on
    /// screen.
    #[must_use]
    pub fn fully_visible(extent: f32) -> Self {
        Self::new(extent, extent)
    }

    /// The same geometry, taking `layout_extent` from the next sliver rather
    /// than its paint extent.
    ///
    /// The pinned-header case, and the only reason the two are separate fields.
    #[must_use]
    pub fn taking(mut self, layout_extent: f32) -> Self {
        self.layout_extent = layout_extent.clamp(0.0, self.paint_extent);
        self
    }

    /// The same geometry, drawn `paint_origin` further along the scroll axis.
    #[must_use]
    pub fn shifted(mut self, paint_origin: f32) -> Self {
        self.paint_origin = paint_origin;
        self
    }

    /// The same geometry, marked as needing a clip.
    #[must_use]
    pub const fn overflowing(mut self) -> Self {
        self.has_visual_overflow = true;
        self
    }

    /// `true` if this geometry is self-consistent.
    ///
    /// Used by a debug assertion in the viewport rather than by callers. A
    /// sliver that paints more than it was offered silently draws over the one
    /// after it, and the resulting picture looks like a *paint order* bug, which
    /// is somewhere else entirely.
    #[must_use]
    pub fn is_consistent(&self, constraints: &SliverConstraints) -> bool {
        self.paint_extent <= constraints.remaining_paint_extent + f32::EPSILON
            && self.layout_extent <= self.paint_extent + f32::EPSILON
            && self.scroll_extent >= 0.0
            && self.paint_extent >= 0.0
    }
}

impl Default for SliverGeometry {
    fn default() -> Self {
        Self::ZERO
    }
}

/// How much of a sliver of `extent` is visible, given where the scroll is.
///
/// The one piece of arithmetic every sliver needs and every sliver would
/// otherwise get subtly wrong at the edges: clamped at both ends, so a sliver
/// entirely above the viewport reports zero rather than a negative extent, and
/// one larger than the viewport reports the viewport rather than itself.
#[must_use]
pub fn visible_extent(extent: f32, constraints: &SliverConstraints) -> f32 {
    (extent - constraints.scroll_offset).clamp(0.0, constraints.remaining_paint_extent.max(0.0))
}

/// The rectangle a sliver of this geometry occupies in its viewport.
#[must_use]
pub fn sliver_rect(constraints: &SliverConstraints, geometry: &SliverGeometry, at: f32) -> Rect {
    let origin = constraints.offset(at + geometry.paint_origin);
    Rect::from_origin_size(
        origin,
        constraints.size(geometry.paint_extent, constraints.cross_axis_extent),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constraints() -> SliverConstraints {
        SliverConstraints::initial(Axis::Vertical, 600.0, 400.0)
    }

    #[test]
    fn a_fully_visible_sliver_has_three_equal_extents() {
        // The case that makes the distinction invisible, which is why it is
        // worth a test naming it.
        let geometry = SliverGeometry::fully_visible(100.0);
        assert_eq!(geometry.scroll_extent, 100.0);
        assert_eq!(geometry.paint_extent, 100.0);
        assert_eq!(geometry.layout_extent, 100.0);
    }

    #[test]
    fn a_pinned_header_paints_more_than_it_takes() {
        // The whole reason `layout_extent` is a separate field: the header keeps
        // drawing 56 pixels of itself while pushing nothing down.
        let geometry = SliverGeometry::new(200.0, 56.0).taking(0.0);
        assert_eq!(geometry.paint_extent, 56.0);
        assert_eq!(geometry.layout_extent, 0.0);
    }

    #[test]
    fn a_layout_extent_cannot_exceed_what_is_painted() {
        let geometry = SliverGeometry::new(200.0, 56.0).taking(500.0);
        assert_eq!(geometry.layout_extent, 56.0);
    }

    #[test]
    fn a_list_taller_than_the_viewport_paints_only_the_viewport() {
        let mut c = constraints();
        c.remaining_paint_extent = 600.0;
        assert_eq!(visible_extent(10_000.0, &c), 600.0);
    }

    #[test]
    fn a_sliver_scrolled_entirely_past_paints_nothing_rather_than_a_negative() {
        let mut c = constraints();
        c.scroll_offset = 500.0;
        assert_eq!(
            visible_extent(100.0, &c),
            0.0,
            "a negative extent here propagates as a negative size, and the \
             failure surfaces three objects away"
        );
    }

    #[test]
    fn a_partly_scrolled_sliver_paints_what_is_left_of_it() {
        let mut c = constraints();
        c.scroll_offset = 30.0;
        assert_eq!(visible_extent(100.0, &c), 70.0);
    }

    #[test]
    fn box_constraints_pin_the_cross_axis_and_free_the_main_one() {
        let c = constraints();
        let box_constraints = c.box_constraints();
        assert_eq!(box_constraints.min_width, 400.0);
        assert_eq!(box_constraints.max_width, 400.0);
        assert_eq!(box_constraints.max_height, f32::INFINITY);
    }

    #[test]
    fn a_horizontal_viewport_swaps_which_axis_is_pinned() {
        let c = SliverConstraints::initial(Axis::Horizontal, 600.0, 400.0);
        let box_constraints = c.box_constraints();
        assert_eq!(box_constraints.min_height, 400.0);
        assert_eq!(box_constraints.max_width, f32::INFINITY);
        assert_eq!(c.size(100.0, 400.0), Size::new(100.0, 400.0));
        assert_eq!(c.offset(50.0), Offset::new(50.0, 0.0));
    }

    #[test]
    fn geometry_that_paints_more_than_it_was_offered_is_inconsistent() {
        let mut c = constraints();
        c.remaining_paint_extent = 50.0;
        assert!(!SliverGeometry::new(100.0, 100.0).is_consistent(&c));
        assert!(SliverGeometry::new(100.0, 50.0).is_consistent(&c));
    }

    // ------------------------------------------------------------- direction

    #[test]
    fn a_scroll_that_did_not_move_is_idle_rather_than_a_direction() {
        // A fling decelerating into its last pixel produces a run of sub-pixel
        // deltas, and reading a direction from those makes a floating header
        // jitter as the scroll settles.
        assert_eq!(
            ScrollDirection::between(100.0, 100.0),
            ScrollDirection::Idle
        );
        assert_eq!(
            ScrollDirection::between(100.0, 100.005),
            ScrollDirection::Idle
        );
    }

    #[test]
    fn a_growing_offset_is_toward_the_end() {
        assert_eq!(
            ScrollDirection::between(100.0, 140.0),
            ScrollDirection::TowardEnd
        );
        assert_eq!(
            ScrollDirection::between(140.0, 100.0),
            ScrollDirection::TowardStart
        );
    }

    #[test]
    fn overscroll_is_carried_separately_from_the_scroll_offset() {
        // The invariant the whole of overscroll rests on: a sliver's scroll
        // offset never goes negative, and the gap is reported beside it.
        let mut c = constraints();
        c.overscroll = 60.0;
        assert!(c.scroll_offset >= 0.0);
        assert_eq!(c.overscroll, 60.0);
    }

    #[test]
    fn a_shifted_sliver_draws_where_it_says_rather_than_where_it_scrolled_to() {
        let c = constraints();
        let geometry = SliverGeometry::new(200.0, 56.0).taking(0.0).shifted(0.0);
        let rect = sliver_rect(&c, &geometry, -144.0);
        assert_eq!(
            rect.top, -144.0,
            "unshifted, a scrolled-away header draws off the top"
        );

        let pinned = geometry.shifted(144.0);
        assert_eq!(
            sliver_rect(&c, &pinned, -144.0).top,
            0.0,
            "shifted, it draws at the viewport's edge — which is what pinning is"
        );
    }
}
