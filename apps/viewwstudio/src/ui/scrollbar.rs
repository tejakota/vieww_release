//! A scrollbar you can grab, for anything driven by a [`ScrollController`].
//!
//! # The gap this closes
//!
//! Every scrollable surface in the studio — the Output panel, the code pane, the
//! Explorer, the Devices tab — could be *scrolled* and none of them could be
//! *steered*. There was no thumb, so there was nothing on screen saying how much
//! of a build log you were looking at or where in it you were, and the only ways
//! to move were a wheel and a drag. On a two-thousand-line `cargo build` that is
//! a lot of wheel, and it is why "scroll up to see what happened" was a chore
//! rather than a glance.
//!
//! # What it is, and what it is not
//!
//! One widget, laid out by [`LayoutBuilder`] so the track is however long the
//! space it was given is, drawing a thumb sized by the fraction of the content
//! that fits. It reads its geometry from the controller — the same one the
//! `Scrollable` beside it reads — so the two cannot disagree about where the
//! content is.
//!
//! It is **not** a viewport and it does not wrap the content. It is a sibling,
//! drawn over the scrollable in a `Stack`, which is what lets it overlay the
//! last few points of a code pane without taking a column away from the text.
//!
//! # Dragging
//!
//! A drag of `d` points along the track moves the content by `d × content/track`
//! — not by `d`. That ratio is the whole of a scrollbar: the thumb travels the
//! length of the track exactly as the content travels its whole scrollable
//! extent, so the gesture is "point at the part of the log you want" rather than
//! "nudge it".
//!
//! A press on the track *outside* the thumb pages towards the press, which is
//! the behaviour of every scrollbar on every platform and costs one comparison.

use vieww_element::ScrollController;
use vieww_foundation::{Alignment, Axis, Color, Offset};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, GestureDetector, LayoutBuilder, SizedBox};

use crate::theme::StudioTheme;

/// How wide the bar is, and how much of that the thumb fills.
///
/// Ten points is the width of a scrollbar people already have muscle memory
/// for; the thumb is inset so the track reads as a groove rather than as a
/// second column of chrome.
pub const WIDTH: f32 = 10.0;
const THUMB: f32 = 6.0;

/// The shortest a thumb is allowed to get.
///
/// Proportional sizing alone gives a one-pixel thumb for a ten-thousand-line
/// log, which is a scrollbar that reports a fact nobody can grab. Below this the
/// thumb stops shrinking and only its *position* keeps meaning something —
/// which is the trade every scrollbar makes.
const MIN_THUMB: f32 = 28.0;

#[derive(Debug)]
pub struct Scrollbar {
    pub scroll: ScrollController,
    pub axis: Axis,
    /// Drawn only while there is something to scroll. A track on a pane whose
    /// content fits is chrome that says "there is more" when there is not.
    pub chrome: StudioTheme,
    pub color: Color,
}

impl Widget for Scrollbar {
    fn debug_name(&self) -> &'static str {
        "Scrollbar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let scroll = self.scroll.clone();
        let axis = self.axis;
        let color = self.color;
        let chrome = self.chrome;

        // Read through `offset()` rather than `peek()`: this is the
        // subscription that rebuilds the thumb as the content moves. Without it
        // the bar is drawn once and then lies for the rest of the session.
        let offset = scroll.offset();
        let viewport = scroll.viewport();
        let max = scroll.max_offset();

        // Nothing to steer. Not a zero-size box but an empty one, so the layout
        // around it does not jump when a log grows past a screenful.
        if max <= 0.5 || viewport <= 0.0 {
            return SizedBox::from_size(match axis {
                Axis::Vertical => vieww_foundation::Size::new(WIDTH, 0.0),
                Axis::Horizontal => vieww_foundation::Size::new(0.0, WIDTH),
            })
            .into();
        }

        let content = viewport + max;

        LayoutBuilder::new(move |constraints: Constraints| {
            let track = match axis {
                Axis::Vertical => constraints.max_height,
                Axis::Horizontal => constraints.max_width,
            };
            if !track.is_finite() || track <= 0.0 {
                return SizedBox::shrink().into();
            }

            let thumb = ((viewport / content) * track).clamp(MIN_THUMB.min(track), track);
            // Where the thumb sits, as a fraction of how far it can travel. The
            // divisor is the *scrollable* extent, not the content: at
            // `max_offset` the thumb's far edge is the track's far edge.
            let travel = (track - thumb).max(0.0);
            let at = if max > 0.0 {
                (offset / max).clamp(0.0, 1.0) * travel
            } else {
                0.0
            };

            // A drag of the thumb is a scroll of `content/track`, which is the
            // same thing said as `max/travel`: crossing the whole track scrolls
            // the whole extent.
            let scale = if travel > 0.0 { max / travel } else { 0.0 };

            let dragging = scroll.clone();
            let paging = scroll.clone();

            let bar: WidgetNode = match axis {
                Axis::Vertical => Container::new()
                    .padding(vieww_foundation::EdgeInsets::only(
                        (WIDTH - THUMB) / 2.0,
                        at,
                        (WIDTH - THUMB) / 2.0,
                        0.0,
                    ))
                    .child(
                        Container::new()
                            .color(color)
                            .radius(THUMB / 2.0)
                            .size(THUMB, thumb),
                    )
                    .into(),
                Axis::Horizontal => Container::new()
                    .padding(vieww_foundation::EdgeInsets::only(
                        at,
                        (WIDTH - THUMB) / 2.0,
                        0.0,
                        (WIDTH - THUMB) / 2.0,
                    ))
                    .child(
                        Container::new()
                            .color(color)
                            .radius(THUMB / 2.0)
                            .size(thumb, THUMB),
                    )
                    .into(),
            };

            Container::new()
                .color(chrome.chrome_0.with_alpha(0x00))
                .alignment(Alignment::TOP_LEFT)
                .child(
                    GestureDetector::new()
                        // Zero, because the bar is ten points wide and a touch
                        // target grown around it would swallow presses meant for
                        // the text beside it. The same rule `ui::divider::GRAB`
                        // and the minimap follow.
                        .touch_target(0.0)
                        .drag_axis(axis)
                        .on_drag_update(move |details: vieww_foundation::DragDetails| {
                            let along = match axis {
                                Axis::Vertical => details.delta.dy,
                                Axis::Horizontal => details.delta.dx,
                            };
                            dragging.jump_to(dragging.peek() + along * scale);
                        })
                        .on_tap_down(move |details| {
                            // A press on the track pages towards it. The thumb's
                            // own presses land here too, and pressing the thumb
                            // must not move anything — hence the comparison
                            // against where it is.
                            let point = match axis {
                                Axis::Vertical => details.local.dy,
                                Axis::Horizontal => details.local.dx,
                            };
                            if point < at {
                                paging.jump_to(paging.peek() - viewport);
                            } else if point > at + thumb {
                                paging.jump_to(paging.peek() + viewport);
                            }
                        })
                        .child(
                            // The whole track is the target, so a press anywhere
                            // on it is answered. A transparent fill still hit
                            // tests, which is what makes the groove clickable
                            // without drawing one.
                            Container::new().color(Color::rgba(0, 0, 0, 1)).child(bar),
                        ),
                )
                .into()
        })
        .into()
    }
}

widget_node_from!(Scrollbar);

/// Where a scroll offset sits relative to the end, for "am I at the bottom?".
///
/// Used by the panel's stick-to-bottom: a log that is scrolled to within a line
/// of its end is a log the reader is following, and one scrolled further back is
/// a log they are reading.
#[must_use]
pub fn at_end(scroll: &ScrollController, slack: f32) -> bool {
    scroll.peek() >= scroll.max_offset() - slack
}

/// Offset a scrollbar overlay by, so it does not sit on the pane's own edge.
pub const INSET: Offset = Offset { dx: 2.0, dy: 2.0 };
