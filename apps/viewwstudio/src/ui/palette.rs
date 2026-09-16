//! The command palette: the whole command list, filtered by what you type.
//!
//! Drawn as an overlay layer rather than as a widget in the shell's column, so
//! it sits above the editor without the layout underneath it moving. It shows
//! nothing that is not in [`Command::ALL`] and runs everything through
//! [`Studio::run`], which is what stops it becoming a fourth place that knows
//! how to save a file.
//!
//! # The keystroke hint is computed, not written
//!
//! Each row's chord comes from [`Command::chord`] and is rendered for the host
//! platform at draw time. A palette with hand-written `⌘S` strings is a palette
//! that says `⌘S` on Linux and is wrong for everybody who reads it there.

use std::rc::Rc;

use vieww_foundation::{Alignment, Border, EdgeInsets, FontFamily, TextStyle};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, SizedBox};

use crate::command::Command;
use crate::state::{PaletteEntry, PaletteMode, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, gap, hairline, kbd, label, space};

/// How many rows fit before the list scrolls.
const VISIBLE: usize = 9;
const ROW: f32 = 30.0;
const WIDTH: f32 = 520.0;

#[derive(Debug)]
pub struct Palette {
    pub studio: Studio,
}

impl Widget for Palette {
    fn debug_name(&self) -> &'static str {
        "CommandPalette"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        if !self.studio.palette_open.get() {
            // Absent rather than transparent: an invisible overlay that is
            // still in the tree still takes the pointer events under it.
            return SizedBox::shrink().into();
        }

        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let colors = theme.colors;
        let query = self.studio.palette_query.get();
        let (mode, _) = PaletteMode::parse(&query);
        let results = self.studio.palette_results();
        let selected = self
            .studio
            .palette_index
            .get()
            .min(results.len().saturating_sub(1));

        // The window scrolls with the highlight rather than the highlight
        // being clamped to the window: arrowing past the bottom has to keep
        // going, and a list that stops at row nine is a list where half the
        // commands are unreachable from the keyboard.
        let first = selected.saturating_sub(VISIBLE - 1);
        let rows: Vec<WidgetNode> = results
            .iter()
            .cloned()
            .enumerate()
            .skip(first)
            .take(VISIBLE)
            .map(|(index, entry)| self.row(&entry, index == selected, &chrome, colors))
            .collect();

        let field = TextField::text(query.clone())
            .size(13.5)
            .show_cursor(self.studio.caret_visible(crate::caret::Active::Palette))
            .style(
                TextStyle::new(13.5)
                    .family(FontFamily::SansSerif)
                    .color(colors.on_surface),
            )
            .placeholder(mode.placeholder())
            .single_line()
            .selection_color(chrome.selection)
            .on_changed({
                let studio = self.studio.clone();
                Rc::new(move |value: vieww_foundation::TextEditingValue| {
                    studio.palette_query.set(value.text);
                    // Any change to the filter resets the highlight: leaving it
                    // on row four while the list shrinks to two rows is how a
                    // palette runs the wrong command.
                    studio.palette_index.set(0);
                })
            });

        let mut column: Vec<WidgetNode> = vec![
            Container::new()
                .padding(EdgeInsets::symmetric(14.0, 11.0))
                .child(field)
                .into(),
            hairline(chrome.line),
        ];

        if results.is_empty() {
            column.push(
                Container::new()
                    .height(ROW)
                    .padding(EdgeInsets::symmetric(14.0, 0.0))
                    .alignment(Alignment::CENTER_LEFT)
                    .child(label(
                        match mode {
                            // Each one names the way *out* of the mode it is
                            // in. An empty result is the moment a person is
                            // most likely to be in the wrong mode and least
                            // likely to know it — "no file matches" answers a
                            // question somebody typing `build` was not asking.
                            PaletteMode::Commands => {
                                "No matching command. Delete the > to search files instead."
                            }
                            PaletteMode::Files => {
                                "No file matches. Start with > for commands, @ for symbols."
                            }
                            PaletteMode::Symbols => {
                                "No definition in this file matches. Delete the @ to search files."
                            }
                            PaletteMode::Line => "Type a line number.",
                        },
                        12.5,
                        colors.on_surface_variant,
                    ))
                    .into(),
            );
        } else {
            column.extend(rows);
            if results.len() > VISIBLE {
                column.push(
                    Container::new()
                        .padding(EdgeInsets::symmetric(14.0, 6.0))
                        .child(label(
                            &format!("{} of {} shown", VISIBLE.min(results.len()), results.len()),
                            10.5,
                            colors.outline,
                        ))
                        .into(),
                );
            }
        }

        // The scrim is a real, tappable layer: clicking away from a palette has
        // to close it, and a decorative dark rectangle would leave the click
        // landing on the editor behind.
        let studio = self.studio.clone();
        let scrim = clickable(
            move || {
                Container::new()
                    .color(vieww_foundation::Color::rgba(0, 0, 0, 90))
                    .into()
            },
            move || studio.palette_open.set(false),
        );

        Stack::new()
            .alignment(Alignment::TOP_CENTER)
            .children(children![
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(scrim),
                Positioned::new().top(64.0).child(
                    Container::new()
                        .color(chrome.chrome_2)
                        .radius(10.0)
                        .width(WIDTH)
                        .border(Border {
                            color: chrome.line,
                            width: 1.0,
                        })
                        // Four points under the last row. Without it the row
                        // sat flush against the panel's rounded edge and the
                        // list read as cut off rather than finished.
                        .padding(EdgeInsets::only(0.0, 0.0, 0.0, 4.0))
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

widget_node_from!(Palette);

impl Palette {
    /// One row, whatever the palette is currently listing.
    ///
    /// The four kinds share a shape on purpose — a title, a dimmer detail, and
    /// a right-aligned hint — because the palette is one list that changes what
    /// it contains, not four lists that look different from each other.
    fn row(
        &self,
        entry: &PaletteEntry,
        selected: bool,
        chrome: &StudioTheme,
        colors: ColorScheme,
    ) -> WidgetNode {
        let studio = self.studio.clone();
        let chrome = *chrome;

        let (title, detail, hint, enabled) = match entry {
            PaletteEntry::Command(command) => (
                command.title().to_string(),
                command.menu().title().to_string(),
                studio
                    .chord_for(*command)
                    .map(|chord| chord.describe(studio.host))
                    .unwrap_or_default(),
                studio.can_run(*command),
            ),
            PaletteEntry::File { label, detail, .. } => {
                (label.clone(), detail.clone(), String::new(), true)
            }
            PaletteEntry::Symbol {
                label,
                detail,
                line,
            } => (label.clone(), detail.clone(), format!("{line}"), true),
            PaletteEntry::Line { line, detail } => {
                (format!("Line {line}"), detail.clone(), String::new(), true)
            }
        };

        let entry = entry.clone();
        crate::ui::chrome::sensed(
            move |sense| {
                let foreground = if !enabled {
                    colors.outline
                } else if selected {
                    colors.on_surface
                } else {
                    colors.on_surface_variant
                };

                let mut row: Vec<WidgetNode> = vec![label(&title, 12.5, foreground).into()];
                if !detail.is_empty() {
                    row.push(space(8.0));
                    row.push(label(&detail, 10.5, colors.outline).into());
                }
                row.push(gap());
                if !hint.is_empty() {
                    row.push(kbd(
                        &hint,
                        colors.on_surface_variant,
                        chrome.chrome_3,
                        chrome.line,
                    ));
                }

                Container::new()
                    // Hover on the row the pointer is over, selection on the
                    // row the keyboard is on. Both are "this is the one Enter
                    // acts on" from two different input devices, so hover is
                    // the lighter of the two rather than a colour of its own.
                    .color(if selected {
                        chrome.selection
                    } else {
                        crate::ui::chrome::hovered(chrome.chrome_2, &chrome, sense)
                    })
                    .height(ROW)
                    .padding(EdgeInsets::symmetric(14.0, 0.0))
                    .alignment(Alignment::CENTER_LEFT)
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(row),
                    )
                    .into()
            },
            move || studio.palette_activate(&entry),
        )
        .into()
    }
}

/// The label the omnibar shows when the palette is shut.
///
/// Lives here rather than in `title_bar.rs` so the two cannot describe the same
/// control differently.
#[must_use]
pub fn omnibar_hint(studio: &Studio) -> String {
    // `active_tab`, not `active`. This is called from the *title bar*, and
    // reading the buffer here subscribed the entire strip — wordmark, seven
    // menus, three icons — to the text of the file being typed into.
    studio
        .active_tab()
        .map_or_else(|| "vieww Studio".to_string(), |b| b.name)
}

/// What the omnibar's key chip says, for the host this is running on.
#[must_use]
pub fn omnibar_chord(studio: &Studio) -> String {
    studio
        .chord_for(Command::CommandPalette)
        .map(|chord| chord.describe(studio.host))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_element::Runtime;

    fn studio() -> (Runtime, Studio) {
        let runtime = Runtime::new();
        let studio = Studio::new(&runtime).with_host(vieww_foundation::TargetPlatform::Linux);
        (runtime, studio)
    }

    #[test]
    fn the_omnibar_chip_is_written_for_the_host() {
        let (_runtime, studio) = studio();
        assert_eq!(omnibar_chord(&studio), "Ctrl+P");

        let mac = studio.with_host(vieww_foundation::TargetPlatform::MacOS);
        assert_eq!(
            omnibar_chord(&mac),
            "⌘P",
            "a hard-coded ⌘P is wrong for everybody not on a Mac"
        );
    }

    #[test]
    fn a_shut_palette_is_absent_from_the_tree_rather_than_transparent() {
        let (_runtime, studio) = studio();
        let dump = vieww_widget::debug_tree(Palette {
            studio: studio.clone(),
        });
        assert!(
            !dump.contains("\"Command Palette\""),
            "an invisible overlay still swallows the clicks under it"
        );

        // **Through the command, not by flipping the signal.** The palette has
        // four modes now, and which one it opens in is the command's decision:
        // a test that sets `palette_open` by hand opens on the *default* mode,
        // which is files, and then asserts the command list is drawn. That is
        // the test asserting a path no user can take.
        studio.run(Command::CommandPalette);
        assert_eq!(
            studio.palette_query.get(),
            ">",
            "the Command Palette command opens the palette on commands"
        );
        let open = vieww_widget::debug_tree(Palette {
            studio: studio.clone(),
        });
        assert!(
            open.contains("\"Render\""),
            "the list is the command list, drawn"
        );
        // Open Folder joined the top of `Command::ALL` — a studio with no
        // folder open can do almost nothing else — which pushed Command
        // Palette itself past the nine visible rows. Any row's chord proves
        // the same property, and ⌘O is now one that is drawn.
        assert!(open.contains("Ctrl+O"), "with each row's chord beside it");
    }

    #[test]
    fn the_prefix_chooses_what_the_palette_is_filtering() {
        use crate::state::{PaletteEntry, PaletteMode};

        assert_eq!(PaletteMode::parse("save"), (PaletteMode::Files, "save"));
        assert_eq!(PaletteMode::parse(">save"), (PaletteMode::Commands, "save"));
        assert_eq!(
            PaletteMode::parse("@screen"),
            (PaletteMode::Symbols, "screen")
        );
        assert_eq!(PaletteMode::parse(":42"), (PaletteMode::Line, "42"));

        let (_runtime, studio) = studio();

        studio.palette_query.set(">render".to_string());
        assert!(
            matches!(
                studio.palette_results().first(),
                Some(PaletteEntry::Command(_))
            ),
            "a > query lists commands"
        );

        studio.palette_query.set(":9".to_string());
        let line = studio.palette_results();
        assert!(
            matches!(line.first(), Some(PaletteEntry::Line { line: 9, .. })),
            "a : query is one row, the line it names"
        );

        // A line past the end still shows, and says so. Listing nothing for a
        // number the user can see they typed reads as the palette being broken.
        studio.palette_query.set(":100000".to_string());
        let far = studio.palette_results();
        assert!(
            matches!(far.first(), Some(PaletteEntry::Line { .. })),
            "an out-of-range line is a row with an explanation, not an empty list"
        );
    }

    #[test]
    fn symbols_are_found_by_a_scan_and_the_scan_knows_what_it_misses() {
        let (_runtime, studio) = studio();
        studio.edit(vieww_foundation::TextEditingValue::new(concat!(
            "use vieww::prelude::*;\n",
            "\n",
            "pub fn screen() -> impl Widget {\n",
            "    let not_a_symbol = 1;\n",
            "}\n",
            "\n",
            "struct Tile;\n",
            "pub(crate) enum Mode { A }\n",
        )));

        let found = studio.symbols();
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["screen", "Tile", "Mode"],
            "pub, pub(crate) and bare declarations all count; a let binding does not"
        );
        assert_eq!(found[0].line, 3, "and each carries the line to jump to");
        assert_eq!(found[0].kind, "fn");
    }
}
