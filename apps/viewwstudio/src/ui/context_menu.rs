//! The Explorer's right-click menu.
//!
//! # Why this is the highest-value thing in its batch
//!
//! `file_tree::rename` and `delete_to_trash` were written, documented and
//! tested, and had **zero callers**. `create_file` had one. The studio had no
//! context menu of any kind and no secondary-click handling anywhere, so a
//! user could not rename or delete a file from inside the editor — they left
//! for a file manager and came back. The operations were a menu away from
//! shipping and read, in the source, as dead code.
//!
//! The framework was never the obstacle: `GestureDetector::on_secondary_tap`
//! and `TapRecognizer::for_button` have been there the whole time. Nothing in
//! the studio used them.
//!
//! # Positioned at the pointer, and flipped when it would not fit
//!
//! A menu that appears somewhere other than where the click was is a menu the
//! user has to go and find. A menu that appears at the pointer and runs off the
//! bottom of the window is worse. So: at the pointer, and flipped up or left
//! when the window says there is no room — which is what every desktop menu
//! does and what makes the last row reachable when you right-click near an edge.

use vieww_foundation::{Alignment, Border, Color, EdgeInsets, Offset};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, SizedBox};

use crate::state::{NameKind, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, hairline, label};

const WIDTH: f32 = 196.0;
const ROW: f32 = 28.0;
/// Padding above and below the rows, plus the separator.
const CHROME: f32 = 13.0;

/// One entry, and what it does.
struct Item {
    label: &'static str,
    /// Drawn in the error colour, and separated from what is above it.
    destructive: bool,
    run: Box<dyn Fn()>,
}

#[derive(Debug)]
pub struct ContextMenuLayer {
    pub studio: Studio,
}

impl Widget for ContextMenuLayer {
    fn debug_name(&self) -> &'static str {
        "ContextMenuLayer"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(menu) = self.studio.context_menu.get() else {
            return SizedBox::shrink().into();
        };
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        let items = self.items(&menu.path, menu.is_dir);
        let height = ROW * items.len() as f32 + CHROME;

        let mut column: Vec<WidgetNode> = Vec::new();
        for item in items {
            if item.destructive {
                column.push(hairline(chrome.line));
            }
            let color = if item.destructive {
                colors.error
            } else {
                colors.on_surface
            };
            let text = item.label;
            let studio = self.studio.clone();
            let run = item.run;
            column.push(
                crate::ui::chrome::sensed(
                    move |sense| {
                        Container::new()
                            // A context menu is opened *at* the pointer, so the
                            // row under it is whichever one the finger stopped
                            // on. Without a hover state the only way to know
                            // which was about to fire was to fire it — and one
                            // of these rows moves a file to the trash.
                            .color(crate::ui::chrome::hover_overlay(colors, sense))
                            .height(ROW)
                            .padding(EdgeInsets::symmetric(12.0, 0.0))
                            .alignment(Alignment::CENTER_LEFT)
                            .child(label(text, 12.0, color))
                            .into()
                    },
                    move || {
                        // Closed first: every one of these opens something
                        // else, and a menu still on screen behind a dialog is
                        // a menu that takes the next click.
                        studio.close_context_menu();
                        run();
                    },
                )
                .into(),
            );
        }

        let size = self.studio.window_size.get();
        let at = place(menu.at, size, height);

        let studio = self.studio.clone();
        Stack::new()
            .alignment(Alignment::TOP_LEFT)
            .children(children![
                // Clicking anywhere else dismisses, which is what a context
                // menu is expected to do and the opposite of the rule
                // `ui::dialog` follows — nothing is lost by dismissing this.
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(clickable(
                        move || Container::new().color(Color::TRANSPARENT).into(),
                        move || studio.close_context_menu(),
                    )),
                Positioned::new().left(at.dx).top(at.dy).child(
                    Container::new()
                        .color(chrome.chrome_2)
                        .radius(8.0)
                        .width(WIDTH)
                        .border(Border {
                            color: chrome.line,
                            width: 1.0,
                        })
                        .padding(EdgeInsets::symmetric(0.0, 6.0))
                        .child(
                            Flex::column()
                                .main_axis_size(MainAxisSize::Min)
                                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .children(column)
                        )
                ),
            ])
            .into()
    }
}

widget_node_from!(ContextMenuLayer);

impl ContextMenuLayer {
    fn items(&self, path: &std::path::Path, is_dir: bool) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();

        if !is_dir {
            let studio = self.studio.clone();
            let target = path.to_path_buf();
            items.push(Item {
                label: "Open",
                destructive: false,
                run: Box::new(move || studio.open_path(target.clone())),
            });
        }

        for (label, kind) in [
            ("New File\u{2026}", NameKind::NewFile),
            ("New Folder\u{2026}", NameKind::NewFolder),
        ] {
            let studio = self.studio.clone();
            let target = path.to_path_buf();
            items.push(Item {
                label,
                destructive: false,
                run: Box::new(move || studio.ask_for_name(kind, &target)),
            });
        }

        {
            let studio = self.studio.clone();
            let target = path.to_path_buf();
            items.push(Item {
                label: "Rename\u{2026}",
                destructive: false,
                run: Box::new(move || studio.ask_for_name(NameKind::Rename, &target)),
            });
        }

        {
            let studio = self.studio.clone();
            let target = path.to_path_buf();
            items.push(Item {
                label: "Copy Path",
                destructive: false,
                run: Box::new(move || studio.copy_path(&target)),
            });
        }

        // **Not offered on the workspace root.** The trash is a folder *inside*
        // the workspace, so moving the root into it is a rename of a directory
        // into itself — `EINVAL`, and reported from use as a confirmation that
        // said yes and did nothing. A menu entry that cannot work is worse than
        // a missing one: it teaches that the studio ignores clicks.
        if !self.studio.is_workspace_root(path) {
            let studio = self.studio.clone();
            let target = path.to_path_buf();
            items.push(Item {
                // Named for what it does. "Delete" would be a lie about a
                // move to the workspace trash, and "Move to Trash" is what
                // makes the confirmation an easy yes.
                label: "Move to Trash\u{2026}",
                destructive: true,
                run: Box::new(move || studio.ask_to_delete(target.clone())),
            });
        }

        items
    }
}

/// Where to draw a menu of `height` opened at `at`, inside a window of `size`.
///
/// Flipped rather than clamped: a menu clamped to the bottom edge sits *under*
/// the pointer and the first row is the one the release lands on, which is how
/// a right-click near the bottom of a list deletes something. Flipping puts the
/// menu above the pointer instead, where nothing is under the finger.
#[must_use]
pub fn place(at: Offset, size: vieww_foundation::Size, height: f32) -> Offset {
    let dx = if at.dx + WIDTH > size.width {
        (at.dx - WIDTH).max(0.0)
    } else {
        at.dx
    };
    let dy = if at.dy + height > size.height {
        (at.dy - height).max(0.0)
    } else {
        at.dy
    };
    Offset::new(dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Size;

    const WINDOW: Size = Size {
        width: 1000.0,
        height: 700.0,
    };

    #[test]
    fn a_menu_with_room_opens_where_the_pointer_is() {
        assert_eq!(
            place(Offset::new(120.0, 200.0), WINDOW, 160.0),
            Offset::new(120.0, 200.0)
        );
    }

    /// The important one: clamping would put a row under the pointer, and the
    /// release would land on it.
    #[test]
    fn a_menu_near_the_bottom_flips_above_the_pointer() {
        let at = Offset::new(120.0, 690.0);
        let placed = place(at, WINDOW, 160.0);
        assert!(placed.dy < at.dy, "it opens upward: {placed:?}");
        assert!(placed.dy + 160.0 <= WINDOW.height + f32::EPSILON);
    }

    #[test]
    fn a_menu_near_the_right_edge_flips_left() {
        let placed = place(Offset::new(990.0, 100.0), WINDOW, 160.0);
        assert!(
            placed.dx + WIDTH <= WINDOW.width + f32::EPSILON,
            "{placed:?}"
        );
    }

    /// A window too small to hold the menu at all must still put it on screen
    /// rather than at a negative offset, where it cannot be clicked.
    #[test]
    fn a_window_smaller_than_the_menu_does_not_push_it_off_the_top() {
        let tiny = Size {
            width: 100.0,
            height: 80.0,
        };
        let placed = place(Offset::new(90.0, 70.0), tiny, 300.0);
        assert!(placed.dx >= 0.0 && placed.dy >= 0.0, "{placed:?}");
    }
}
