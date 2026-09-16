//! The draggable split between two panes.
//!
//! **The divider is the gutter.** In the card shell every region is a rounded
//! surface with [`CARD_GAP`](crate::ui::chrome::CARD_GAP) of window ground
//! showing around it, and this occupies exactly that gap rather than adding a
//! strip of its own — otherwise a resizable split is visibly wider than a
//! fixed one and the shell's rhythm breaks at every second seam.
//!
//! So it paints nothing at rest. The ground behind it is the separator.
//!
//! # It used to paint nothing at *any* time until it was already moving
//!
//! Reported: "there is a minimizer functionality between panes, but the
//! minimizer icon/pointer is not showing up — the user will get confused."
//!
//! Both halves of that were true, and they are two different mechanisms:
//!
//! 1. **No pointer.** [`Cursor::ResizeColumn`] has existed since the pointer
//!    module was written and the winit backend has passed the shape to the
//!    window all along, but nothing in the workspace could *ask* for one:
//!    `RenderEditableText`'s I-beam was the only cursor any object returned.
//!    So the mouse showed a plain arrow over a seam that resizes two panes, and
//!    the one thing every desktop user recognises a split by was missing.
//!    [`CursorArea`] exists now and this is its first
//!    caller.
//!
//! 2. **No mark.** The gutter is the separator at rest, which is right, and it
//!    was *also* the separator while the pointer was on it, which is not: the
//!    seam gave no sign it could be grabbed until a drag was already underway,
//!    at which point the person has learned it by accident. A grip now fades in
//!    under the pointer — the same thing every editor with a sash does, and the
//!    reason none of them need to explain it.
//!
//! The grip is drawn only while the pointer is over the seam or a drag is in
//! flight, so a still screenshot of the studio is unchanged and the shell's
//! rhythm at rest is exactly what it was.

use vieww_foundation::{Axis, Cursor};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, CursorArea};

/// The hit width, which is the gutter width — see the module note.
pub const WIDTH: f32 = crate::ui::chrome::CARD_GAP;

/// How wide the divider is to grab, and why it is not the theme's touch target.
///
/// # The defect this fixes
///
/// `GestureDetector` expands every control to `Metrics::touch_target` — 48
/// points — centred on its own box, so that a small control is still reachable
/// with a finger. Applied to a **six-point line in a six-point gutter** that is
/// a hit area reaching twenty-one points into the card on each side, sitting on
/// top of whatever those cards have at their edges. What they have at their
/// edges is controls: the tab strip's New File button ends flush against the
/// editor card's right edge, and the preview pane's tabs start flush against
/// its left one.
///
/// The symptom, reported from use: pressing the drawn "+" did nothing, and
/// pressing a few millimetres to its left worked. Measured, the "+" is painted
/// across x 948.9–957.1 and was reachable only across 929–944 — everything from
/// 945 rightwards went to the divider, which is invisible.
///
/// `GestureDetector::touch_target`'s own documentation names this exact case as
/// its one considered exception: `Chip`, "because chips come in rows where an
/// expanded target would overlap its neighbours on both sides". A divider is
/// that with the neighbours pressed right up against it.
///
/// Twelve rather than six, so that a divider is still grabbable where a card
/// does not want the point, and three points of reach on each side stays inside
/// the cards' rounded corners rather than over their contents.
///
/// # What those extra points are actually worth, measured
///
/// **In this shell: nothing, and that is correct.** `RenderTree::hit_test` runs
/// the widened pass only as a *fallback* for a point that hit nothing — the very
/// rule that fixed the "+" above — and both of this seam's neighbours are cards
/// that take the tight pass across their whole width. Driven headlessly, a press
/// and drag resizes the sidebar across exactly x 312–317 on a 1440-point window
/// and nowhere else: the six points of drawn gutter, and not one point more.
///
/// So this number is a *ceiling on the damage* rather than a promise of reach.
/// It is why the seam no longer eats its neighbours' controls; it is not why the
/// seam can be grabbed. Anything that wants a genuinely wider grab has to take
/// the space in layout, which would make the gutter wider than
/// [`CARD_GAP`](crate::ui::chrome::CARD_GAP) and break the shell's rhythm — the
/// thing the module note opens by refusing.
///
/// `apps/viewwstudio/tests/seam.rs` pins the consequence that matters: the band
/// where a drag starts and the band where the pointer promises one are the same
/// band. A cursor that changed over a card's edge would be a promise the click
/// there does not keep.
pub const GRAB: f32 = 12.0;

/// How long the grip is, down the middle of the seam.
///
/// A short bar rather than the full height: a line running the whole way is a
/// border, and the shell already decided its seams are ground rather than
/// borders. A bar centred in the gutter reads as a handle, which is what it is.
const GRIP: f32 = 28.0;

/// Which way the pane grows when the divider moves right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grows {
    /// The pane on the left of the divider — dragging right makes it wider.
    Leading,
    /// The pane on the right — dragging right makes it narrower.
    Trailing,
}

/// A vertical divider that writes a width, clamped, on every drag update.
#[derive(Debug)]
pub struct VerticalDivider {
    pub width: vieww_element::Signal<f32>,
    pub grows: Grows,
    pub min: f32,
    pub max: f32,
    /// True while this seam is under the pointer's drag.
    ///
    /// A signal rather than a `Pressable` press amount because a drag is not a
    /// press: the pointer leaves the divider's own bounds within a few pixels
    /// of starting, and a widget that dimmed only while the finger was still
    /// on it would go dark for the whole of every real resize.
    pub active: vieww_element::Signal<bool>,
    /// True while the pointer is resting on this seam, so the grip can appear
    /// before anything is dragged. Separate from `active` for the reason above:
    /// during a drag the pointer is usually somewhere else entirely, and the
    /// grip has to stay lit.
    pub hovered: vieww_element::Signal<bool>,
}

impl Widget for VerticalDivider {
    fn debug_name(&self) -> &'static str {
        "VerticalDivider"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let width = self.width.clone();
        let active = self.active.clone();
        let (grows, min, max) = (self.grows, self.min, self.max);

        let (start, end) = (active.clone(), active.clone());
        let (enter, leave) = (self.hovered.clone(), self.hovered.clone());

        // Lit while the pointer is on the seam *or* a drag is in flight. The
        // drag half matters because the pointer is off the seam for all but the
        // first few pixels of a resize.
        let dragging = self.active.get();
        let lit = dragging || self.hovered.get();

        let seam = GestureDetector::new()
            // See `GRAB`: the theme's 48-point minimum would put this
            // invisible strip on top of the controls at both cards' edges.
            .touch_target(GRAB)
            .drag_axis(Axis::Horizontal)
            .on_hover(move |inside| {
                if inside {
                    enter.set(true);
                } else {
                    leave.set(false);
                }
            })
            .on_drag_start(move |_| start.set(true))
            .on_drag_end(move |_| end.set(false))
            .on_drag_update(move |details| {
                let delta = match grows {
                    Grows::Leading => details.delta.dx,
                    Grows::Trailing => -details.delta.dx,
                };
                // Clamped here rather than in the layout: a pane that stops at
                // its minimum should stop *tracking the finger* there, or the
                // finger and the edge separate and never meet again.
                let next = (width.peek() + delta).clamp(min, max);
                if (next - width.peek()).abs() > f32::EPSILON {
                    width.set(next);
                }
            })
            .child(
                Container::new()
                    .width(WIDTH)
                    .alignment(vieww_foundation::Alignment::CENTER)
                    .child(
                        Container::new()
                            .color(if dragging {
                                // The seam being dragged is the one thing on
                                // screen that is moving; it says so in the
                                // accent colour.
                                theme.colors.primary
                            } else if lit {
                                // Under the pointer but not yet grabbed: a
                                // grip, not an announcement. `outline` is the
                                // theme's quietest line, so this reads as part
                                // of the chrome rather than as a control that
                                // has appeared out of nowhere.
                                theme.colors.outline
                            } else {
                                // Transparent, not `chrome.line`: at rest the
                                // window ground behind the gutter is already
                                // the separator, and a line drawn on top of it
                                // is a second one.
                                vieww_foundation::Color::TRANSPARENT
                            })
                            .radius(WIDTH / 2.0)
                            .width(WIDTH - 2.0)
                            // Full height while dragging — the moving seam is
                            // worth following with the eye down the whole
                            // window — and a short grip while merely hovered.
                            .height(if dragging { f32::INFINITY } else { GRIP }),
                    ),
            );

        // The pointer shape, which is what tells somebody the seam is draggable
        // *before* they try it. See the module note: nothing in the workspace
        // could ask for a cursor until `CursorArea`.
        // The shape covers the drawn gutter, which — measured — is exactly
        // where a drag starts. See `GRAB`: the extra reach is a fallback that
        // this seam's neighbours never yield, so promising a resize any wider
        // would be promising one that does not happen.
        CursorArea::new(Cursor::ResizeColumn).child(seam).into()
    }
}

widget_node_from!(VerticalDivider);

/// The same seam, lying down: the split between the panes and the bottom panel.
///
/// # Why this did not exist
///
/// Reported: "no pane dragger/hider for the bottom terminal/output window pane,
/// similar to what we have for editor and renderer." Correct — the panel was a
/// fixed 176 points. The tab strip collapses it to nothing and brings it back at
/// the same height, which is a *hider* and was the whole story; a build whose
/// output is forty lines long and a `cargo test` run that prints two hundred got
/// the same seven visible rows.
///
/// Everything below is [`VerticalDivider`] with the axis turned: the gutter is
/// the same [`CARD_GAP`](crate::ui::chrome::CARD_GAP), the grip is the same bar
/// rotated, and the cursor is [`Cursor::ResizeRow`] — which, like its column
/// twin, had never been asked for by anything in the workspace.
///
/// Dragging **up** makes the panel taller, which is why `on_drag_update` negates
/// `dy`: the panel is the pane *below* the seam, and a seam that moved the wrong
/// way would be worse than none.
#[derive(Debug)]
pub struct HorizontalDivider {
    pub height: vieww_element::Signal<f32>,
    pub min: f32,
    pub max: f32,
    pub active: vieww_element::Signal<bool>,
    pub hovered: vieww_element::Signal<bool>,
}

impl Widget for HorizontalDivider {
    fn debug_name(&self) -> &'static str {
        "HorizontalDivider"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let height = self.height.clone();
        let (min, max) = (self.min, self.max);

        let (start, end) = (self.active.clone(), self.active.clone());
        let (enter, leave) = (self.hovered.clone(), self.hovered.clone());

        let dragging = self.active.get();
        let lit = dragging || self.hovered.get();

        let seam = GestureDetector::new()
            .touch_target(GRAB)
            .drag_axis(Axis::Vertical)
            .on_hover(move |inside| {
                if inside {
                    enter.set(true);
                } else {
                    leave.set(false);
                }
            })
            .on_drag_start(move |_| start.set(true))
            .on_drag_end(move |_| end.set(false))
            .on_drag_update(move |details| {
                // Up is negative, and up makes the panel taller.
                let next = (height.peek() - details.delta.dy).clamp(min, max);
                if (next - height.peek()).abs() > f32::EPSILON {
                    height.set(next);
                }
            })
            .child(
                Container::new()
                    .height(WIDTH)
                    .alignment(vieww_foundation::Alignment::CENTER)
                    .child(
                        Container::new()
                            .color(if dragging {
                                theme.colors.primary
                            } else if lit {
                                theme.colors.outline
                            } else {
                                vieww_foundation::Color::TRANSPARENT
                            })
                            .radius(WIDTH / 2.0)
                            .height(WIDTH - 2.0)
                            .width(if dragging { f32::INFINITY } else { GRIP }),
                    ),
            );

        CursorArea::new(Cursor::ResizeRow).child(seam).into()
    }
}

widget_node_from!(HorizontalDivider);
