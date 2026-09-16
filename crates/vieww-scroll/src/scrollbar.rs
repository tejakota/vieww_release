//! A draggable, proportionally-sized scrollbar.
//!
//! # A controlled widget, like `Scrollable` itself
//!
//! `vieww-widget::Scrollable` holds no state of its own: it is *told* an
//! offset and reports the drags it receives, because owning a live position
//! needs a signal and signals live above the widget layer
//! (`docs/DESIGN.md` §7 — see that type's own doc for the full argument).
//! [`Scrollbar`] makes the identical choice for the identical reason: it
//! takes the current `offset` and [`ScrollExtents`] as plain values, reports
//! a drag on the thumb through `on_drag`, and holds nothing that
//! would need to survive a rebuild. Whatever owns the real
//! [`ScrollPosition`](vieww_gestures::ScrollPosition) — a
//! `vieww_element::ScrollController`, or this crate's own future
//! integration of one — re-renders both the scrollable content and this
//! bar from the same offset, which is what keeps a scrollbar's thumb from
//! ever drifting out of sync with what it represents.
//!
//! # Placement
//!
//! A scrollbar overlays its track, so it is placed in a `Stack` alongside
//! the scrollable content it represents, not wrapped around it — `Scrollbar`
//! itself expands to fill whatever `Positioned` gives it along the cross
//! axis and only occupies `thickness` along the scroll axis.

use vieww_foundation::{Axis, Color, DragDetails, Key};

use vieww_widget::{
    widget_node_from, BuildContext, Container, GestureDetector, Handler, Positioned, ScrollExtents,
    Stack, Widget, WidgetKind, WidgetNode,
};

/// Where a proportional thumb sits and how long it is, in the same units as
/// `track_length`.
///
/// Pure geometry — no widget, no gesture, no drag — kept separate from
/// [`Scrollbar`] itself so every edge case (content shorter than the
/// viewport, a zero-length track, an offset past the end) is a table of
/// numbers to check rather than something read off a rendered pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThumbGeometry {
    /// Distance from the start of the track to the start of the thumb.
    pub start: f32,
    /// The thumb's own length.
    pub length: f32,
}

impl ThumbGeometry {
    /// Compute a thumb for content of `extents.content` shown through
    /// `extents.viewport`, currently scrolled to `offset`, drawn on a track
    /// `track_length` long, with the thumb never drawn shorter than
    /// `min_length` — a thumb representing a tiny fraction of a huge list
    /// still has to be long enough to see and to grab.
    ///
    /// # The three cases
    ///
    /// - **Content fits already** (`content <= viewport`, including
    ///   `viewport <= 0` or `content <= 0`): the thumb fills the whole
    ///   track. Nothing to scroll to means nothing for a thumb to indicate
    ///   a *position* within, and filling the track is the same convention
    ///   every platform scrollbar uses for "nothing to hide".
    /// - **Ordinary case**: thumb length is the track scaled by
    ///   `viewport / content` (how much of the content is visible), and its
    ///   start is the track scaled by how far into the *scrollable range*
    ///   `offset` is (`offset / max_offset`, not `offset / content` — the
    ///   thumb's own length already accounts for the viewport, so its
    ///   *travel* is the track minus its own length, not the whole track).
    /// - **`offset` outside `0..=max_offset`** (an overscroll bounce): the
    ///   fraction is clamped to `0.0..=1.0` rather than let the thumb run
    ///   off either end of the track — a bounced scrollbar thumb that
    ///   detached from its track would look like a rendering bug, not
    ///   physics.
    #[must_use]
    pub fn compute(
        extents: ScrollExtents,
        offset: f32,
        track_length: f32,
        min_length: f32,
    ) -> Self {
        if track_length <= 0.0 {
            return Self {
                start: 0.0,
                length: 0.0,
            };
        }
        let max_offset = extents.max_offset();
        if extents.content <= extents.viewport || extents.content <= 0.0 {
            return Self {
                start: 0.0,
                length: track_length,
            };
        }

        let visible_fraction = (extents.viewport / extents.content).clamp(0.0, 1.0);
        let length =
            (track_length * visible_fraction).clamp(min_length.min(track_length), track_length);

        let travel = track_length - length;
        let scrolled_fraction = if max_offset > 0.0 {
            (offset / max_offset).clamp(0.0, 1.0)
        } else {
            0.0
        };

        Self {
            start: travel * scrolled_fraction,
            length,
        }
    }
}

/// A draggable, proportionally-sized scrollbar thumb over a track.
///
/// See this module's own doc for why it is a controlled widget with no
/// state of its own.
///
/// ```
/// use vieww_scroll::Scrollbar;
/// use vieww_widget::ScrollExtents;
/// use vieww_foundation::Axis;
///
/// let bar = Scrollbar::new(Axis::Vertical, 40.0, ScrollExtents::new(200.0, 800.0));
/// ```
#[derive(Clone)]
pub struct Scrollbar {
    axis: Axis,
    offset: f32,
    extents: ScrollExtents,
    thickness: f32,
    min_thumb_length: f32,
    track_color: Color,
    thumb_color: Color,
    on_drag: Option<Handler<DragDetails>>,
    key: Option<Key>,
}

impl std::fmt::Debug for Scrollbar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scrollbar")
            .field("axis", &self.axis)
            .field("offset", &self.offset)
            .field("extents", &self.extents)
            .field("thickness", &self.thickness)
            .finish_non_exhaustive()
    }
}

/// The default track thickness — comfortably above the smallest platform
/// guidelines call out for a draggable target's *short* axis (a scrollbar
/// thumb's long axis is the one that needs to be grabbable; its thickness
/// is conventionally much smaller purely for visual weight, so this errs
/// toward "visible and grabbable" over "matches a native OS scrollbar
/// exactly", which varies by platform and is not this widget's job to
/// mimic).
const DEFAULT_THICKNESS: f32 = 10.0;
/// Below this, a thumb reads as a sliver rather than a handle.
const DEFAULT_MIN_THUMB_LENGTH: f32 = 24.0;

impl Scrollbar {
    /// A scrollbar for `axis`, currently scrolled to `offset` within
    /// `extents`.
    #[must_use]
    pub fn new(axis: Axis, offset: f32, extents: ScrollExtents) -> Self {
        Self {
            axis,
            offset,
            extents,
            thickness: DEFAULT_THICKNESS,
            min_thumb_length: DEFAULT_MIN_THUMB_LENGTH,
            track_color: Color::rgba(0, 0, 0, 20),
            thumb_color: Color::rgba(0, 0, 0, 110),
            on_drag: None,
            key: None,
        }
    }

    /// Track thickness (the scrollbar's own width if vertical, height if
    /// horizontal). Default: `DEFAULT_THICKNESS`.
    #[must_use]
    pub const fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// The shortest the thumb may ever be drawn. Default:
    /// `DEFAULT_MIN_THUMB_LENGTH`.
    #[must_use]
    pub const fn min_thumb_length(mut self, min_thumb_length: f32) -> Self {
        self.min_thumb_length = min_thumb_length;
        self
    }

    /// Colours for the track and the thumb. Defaults are a translucent
    /// black suited to a light background; a dark-themed caller supplies
    /// its own.
    #[must_use]
    pub const fn colors(mut self, track: Color, thumb: Color) -> Self {
        self.track_color = track;
        self.thumb_color = thumb;
        self
    }

    /// Called as the thumb is dragged, carrying the drag in the same
    /// [`DragDetails`] shape `Scrollable::on_drag` reports — a caller wires
    /// both to the same handler and the two stay in lock-step by
    /// construction rather than by convention.
    #[must_use]
    pub fn on_drag(mut self, handler: Handler<DragDetails>) -> Self {
        self.on_drag = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Scrollbar {
    fn debug_name(&self) -> &'static str {
        "Scrollbar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    /// Lays the thumb out with [`Positioned`] inside whatever `Stack` this
    /// scrollbar was placed in — see this module's own "Placement" doc.
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // The track length is not known until layout, and this widget has
        // no layout pass of its own to read it back from — the same
        // constraint `Scrollable` documents for why it does not own a
        // position. `extents.viewport` is the caller's own measurement of
        // exactly that length along the scroll axis (it is what the
        // scrollable content is laid out against), so it doubles as the
        // track length here rather than this widget needing a second,
        // redundant measurement pass.
        let track_length = self.extents.viewport;
        let thumb = ThumbGeometry::compute(
            self.extents,
            self.offset,
            track_length,
            self.min_thumb_length,
        );

        let thumb_box = Container::new()
            .color(self.thumb_color)
            .radius(self.thickness / 2.0);
        let draggable_thumb = match &self.on_drag {
            Some(handler) => {
                let handler = handler.clone();
                GestureDetector::new()
                    .on_drag_update(move |details| handler(details))
                    .child(thumb_box)
                    .into()
            }
            None => WidgetNode::from(thumb_box),
        };

        let mut thumb_position = Positioned::new();
        thumb_position = match self.axis {
            Axis::Vertical => thumb_position
                .top(thumb.start)
                .height(thumb.length)
                .right(0.0)
                .width(self.thickness),
            Axis::Horizontal => thumb_position
                .left(thumb.start)
                .width(thumb.length)
                .bottom(0.0)
                .height(self.thickness),
        };

        let track_position = match self.axis {
            Axis::Vertical => Positioned::new()
                .top(0.0)
                .bottom(0.0)
                .right(0.0)
                .width(self.thickness),
            Axis::Horizontal => Positioned::new()
                .left(0.0)
                .right(0.0)
                .bottom(0.0)
                .height(self.thickness),
        };

        let _ = ctx;
        Stack::new()
            .push(track_position.child(Container::new().color(self.track_color)))
            .push(thumb_position.child(draggable_thumb))
            .into()
    }
}

widget_node_from!(Scrollbar);

#[cfg(test)]
mod tests {
    use vieww_widget::debug_tree;

    use super::*;

    fn extents(viewport: f32, content: f32) -> ScrollExtents {
        ScrollExtents::new(viewport, content)
    }

    #[test]
    fn content_that_already_fits_gives_a_full_length_thumb() {
        let thumb = ThumbGeometry::compute(extents(200.0, 150.0), 0.0, 200.0, 24.0);
        assert_eq!(
            thumb,
            ThumbGeometry {
                start: 0.0,
                length: 200.0
            }
        );
    }

    #[test]
    fn a_zero_length_track_produces_no_thumb_rather_than_dividing_by_zero() {
        let thumb = ThumbGeometry::compute(extents(200.0, 800.0), 0.0, 0.0, 24.0);
        assert_eq!(
            thumb,
            ThumbGeometry {
                start: 0.0,
                length: 0.0
            }
        );
    }

    #[test]
    fn the_thumb_length_is_the_track_scaled_by_visible_fraction() {
        // A quarter of the content is visible, so the thumb is a quarter of
        // the track (well above the minimum, so the clamp does not engage).
        let thumb = ThumbGeometry::compute(extents(200.0, 800.0), 0.0, 400.0, 24.0);
        assert_eq!(thumb.length, 100.0);
    }

    #[test]
    fn the_thumb_start_tracks_the_scrolled_fraction_of_the_scrollable_range() {
        // viewport=200, content=800 -> max_offset=600, visible_fraction=0.25.
        // Track 400: thumb length = 100, travel = 300.
        // At the exact midpoint of the scrollable range (offset=300 of 600):
        let thumb = ThumbGeometry::compute(extents(200.0, 800.0), 300.0, 400.0, 24.0);
        assert_eq!(thumb.length, 100.0);
        assert_eq!(thumb.start, 150.0, "half of the 300px travel");
    }

    #[test]
    fn the_thumb_reaches_exactly_the_far_end_at_max_offset() {
        let extents = extents(200.0, 800.0); // max_offset = 600
        let thumb = ThumbGeometry::compute(extents, extents.max_offset(), 400.0, 24.0);
        assert_eq!(
            thumb.start + thumb.length,
            400.0,
            "flush with the end of the track"
        );
    }

    #[test]
    fn an_overscrolled_offset_clamps_rather_than_running_off_the_track() {
        let extents = extents(200.0, 800.0);
        let past_the_end = ThumbGeometry::compute(extents, 10_000.0, 400.0, 24.0);
        assert_eq!(past_the_end.start + past_the_end.length, 400.0);

        let before_the_start = ThumbGeometry::compute(extents, -500.0, 400.0, 24.0);
        assert_eq!(before_the_start.start, 0.0);
    }

    #[test]
    fn a_tiny_fraction_of_a_huge_list_still_gets_a_grabbable_minimum_thumb() {
        // viewport=10, content=100_000 -> visible_fraction is effectively 0.
        let thumb = ThumbGeometry::compute(extents(10.0, 100_000.0), 0.0, 500.0, 24.0);
        assert_eq!(
            thumb.length, 24.0,
            "never smaller than the minimum, however small the content fraction"
        );
    }

    #[test]
    fn the_minimum_is_capped_by_the_track_itself() {
        // A minimum longer than the track cannot be honoured literally —
        // the thumb is clamped to the track's own length instead of
        // overflowing it.
        let thumb = ThumbGeometry::compute(extents(10.0, 100_000.0), 0.0, 20.0, 24.0);
        assert_eq!(thumb.length, 20.0);
    }

    #[test]
    fn zero_content_is_treated_the_same_as_content_that_already_fits() {
        let thumb = ThumbGeometry::compute(extents(200.0, 0.0), 0.0, 200.0, 24.0);
        assert_eq!(
            thumb,
            ThumbGeometry {
                start: 0.0,
                length: 200.0
            }
        );
    }

    #[test]
    fn builds_into_a_stack_with_a_track_and_a_thumb() {
        let bar = Scrollbar::new(Axis::Vertical, 0.0, ScrollExtents::new(200.0, 800.0));
        let dump = debug_tree(bar);
        assert!(dump.contains("Stack"), "{dump}");
        assert!(dump.contains("Positioned"), "{dump}");
        assert!(dump.contains("Container"), "{dump}");
    }

    #[test]
    fn a_drag_handler_wraps_the_thumb_in_a_gesture_detector() {
        let bar = Scrollbar::new(Axis::Vertical, 0.0, ScrollExtents::new(200.0, 800.0))
            .on_drag(std::rc::Rc::new(|_| {}));
        let dump = debug_tree(bar);
        assert!(dump.contains("GestureDetector"), "{dump}");
    }
}
