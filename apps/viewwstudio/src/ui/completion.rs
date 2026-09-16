//! The completion list.
//!
//! # Why this was the cheapest large win available
//!
//! The LSP client speaks `initialize`, `didOpen`, `didChange` and reads
//! `publishDiagnostics` back. Its own module docs named what was left out:
//! *"Hover, go-to-definition and completion are the same transport and are not
//! written."* Same transport, same connection, already open and already
//! carrying diagnostics — and for a **Rust** editor specifically, completion is
//! not a nicety. The language is not usable at speed without it.
//!
//! # Positioned at the caret, in the editor's own coordinates
//!
//! The list is mounted **inside the code pane's stack**, beside the inline
//! diagnostics, rather than as a window-level overlay. That is the difference
//! between needing the field's global offset — which nothing in the studio
//! tracks, and which the sidebar's width and two scroll offsets all move — and
//! needing the caret rect, which the layout probe already publishes.
//!
//! The anchor is the caret's rect from that probe: the only thing that knows
//! where a character is after shaping. `place` drops the list below the caret
//! and lifts it above when there is no room, never *over* it — covering the
//! caret hides the thing being completed.

use vieww_foundation::{Alignment, Border, Color, EdgeInsets, Offset};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, SizedBox};

use crate::state::{Completions, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, label, mono, space};

const WIDTH: f32 = 340.0;
const ROW: f32 = 24.0;
/// Padding above and below the rows.
const CHROME: f32 = 10.0;

#[derive(Debug)]
pub struct CompletionList {
    pub studio: Studio,
    /// The caret's rect, in the code pane's coordinates, from the layout probe.
    pub caret: vieww_foundation::Rect,
    /// The pane's own size, so the list can be flipped rather than clipped.
    pub bounds: vieww_foundation::Size,
}

impl Widget for CompletionList {
    fn debug_name(&self) -> &'static str {
        "CompletionList"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(open) = self.studio.completion.get() else {
            // Absent rather than transparent: an invisible overlay still eats
            // the pointer events under it, which here would be the editor.
            return SizedBox::shrink().into();
        };
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        let shown = open.items.len().min(Completions::VISIBLE);
        let mut rows: Vec<WidgetNode> = Vec::new();
        // Scrolled to keep the highlight visible, by starting the window at
        // whichever row keeps it in range — a list whose selection has moved
        // below the fold is a list the arrow keys appear to have stopped
        // moving.
        let first = open.index.saturating_sub(Completions::VISIBLE - 1);
        for (offset, item) in open.items.iter().skip(first).take(shown).enumerate() {
            let index = first + offset;
            let selected = index == open.index;
            let studio = self.studio.clone();
            let label_text = item.label.clone();
            let detail = item.detail.clone();
            rows.push(
                clickable(
                    move || {
                        Container::new()
                            .height(ROW)
                            .color(if selected {
                                chrome.selection
                            } else {
                                Color::TRANSPARENT
                            })
                            .padding(EdgeInsets::symmetric(10.0, 0.0))
                            .alignment(Alignment::CENTER_LEFT)
                            .child(
                                Flex::row()
                                    .cross_axis_alignment(CrossAxisAlignment::Center)
                                    .children(children![
                                        Flexible::expanded(1).child(mono(
                                            &label_text,
                                            11.5,
                                            colors.on_surface
                                        )),
                                        space(8.0),
                                        label(&detail, 10.0, colors.outline),
                                    ]),
                            )
                            .into()
                    },
                    move || {
                        studio.move_completion_to(index);
                        studio.accept_completion();
                    },
                )
                .into(),
            );
        }

        if open.items.len() > shown {
            // Counted, not silently truncated — the same rule workspace search
            // and the quit dialog follow.
            rows.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(10.0, 3.0))
                    .child(label(
                        &format!("{} more \u{2014} keep typing", open.items.len() - shown),
                        10.0,
                        colors.outline,
                    ))
                    .into(),
            );
        }

        #[expect(
            clippy::cast_precision_loss,
            reason = "the list is capped at nine visible rows"
        )]
        let height = ROW * shown as f32 + CHROME;
        let at = place(
            Offset::new(self.caret.left, self.caret.bottom),
            self.bounds,
            height,
        );

        Stack::new()
            .alignment(Alignment::TOP_LEFT)
            .children(children![Positioned::new().left(at.dx).top(at.dy).child(
                Container::new()
                    .color(chrome.chrome_3)
                    .radius(6.0)
                    .width(WIDTH)
                    .border(Border {
                        color: chrome.line,
                        width: 1.0,
                    })
                    .padding(EdgeInsets::symmetric(0.0, 5.0))
                    .child(
                        Flex::column()
                            .main_axis_size(MainAxisSize::Min)
                            .cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .children(rows)
                    )
            )])
            .into()
    }
}

widget_node_from!(CompletionList);

/// Where to draw a list of `height` for a caret at `at`, inside `size`.
///
/// Below the caret when there is room, above it when there is not — never
/// *over* it, which is the one placement that hides the thing being completed.
#[must_use]
pub fn place(at: Offset, size: vieww_foundation::Size, height: f32) -> Offset {
    /// How far below the caret the list sits, so it clears the line.
    const DROP: f32 = 18.0;

    let dx = if at.dx + WIDTH > size.width {
        (at.dx - WIDTH).max(0.0)
    } else {
        at.dx
    };
    let below = at.dy + DROP;
    let dy = if below + height > size.height {
        (at.dy - height - 4.0).max(0.0)
    } else {
        below
    };
    Offset::new(dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Size;

    const WINDOW: Size = Size {
        width: 1440.0,
        height: 900.0,
    };

    #[test]
    fn a_list_with_room_opens_below_the_caret() {
        let placed = place(Offset::new(400.0, 300.0), WINDOW, 200.0);
        assert!(placed.dy > 300.0, "below: {placed:?}");
        assert_eq!(placed.dx, 400.0);
    }

    /// Above, not over: covering the caret hides the thing being completed.
    #[test]
    fn a_list_near_the_bottom_opens_above_the_caret() {
        let at = Offset::new(400.0, 880.0);
        let placed = place(at, WINDOW, 200.0);
        assert!(placed.dy + 200.0 <= at.dy, "clear of the caret: {placed:?}");
    }

    #[test]
    fn a_list_near_the_right_edge_flips_left() {
        let placed = place(Offset::new(1430.0, 200.0), WINDOW, 200.0);
        assert!(
            placed.dx + WIDTH <= WINDOW.width + f32::EPSILON,
            "{placed:?}"
        );
    }

    #[test]
    fn a_window_smaller_than_the_list_keeps_it_on_screen() {
        let tiny = Size {
            width: 120.0,
            height: 100.0,
        };
        let placed = place(Offset::new(110.0, 90.0), tiny, 400.0);
        assert!(placed.dx >= 0.0 && placed.dy >= 0.0, "{placed:?}");
    }
}
