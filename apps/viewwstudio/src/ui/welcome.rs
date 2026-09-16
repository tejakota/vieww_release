//! The first screen, and the one that says what this is.
//!
//! # The finding
//!
//! First launch gave a scratch buffer. There was no welcome view, no "open a
//! folder / try a sample / read the docs" triage, no keyboard cheat sheet, and
//! no discoverable hint that a command palette existed at all. The palette is
//! the studio's best feature and nothing pointed at it.
//!
//! Worse, with nothing persisted there was also no list of what you were
//! working on yesterday — so opening a project was a fresh navigation through
//! the folder picker every single morning. The recent list is the other half of
//! this file, and it only became possible once the session file existed.
//!
//! # Why it is an overlay and not a sidebar view
//!
//! A view would put "how do I start?" behind the activity bar, which is the
//! part of the window a new user has not learned yet. This is raised in front
//! of everything on a launch with no workspace and nothing restored, and it is
//! dismissible — a welcome that cannot be got rid of is worse than none.

use vieww_foundation::{Alignment, Border, Color, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, SizedBox};

use crate::command::Command;
use crate::state::Studio;
use crate::theme::StudioTheme;
use crate::ui::chrome::{button, clickable, hairline, label, label_bold, mono, space};

const WIDTH: f32 = 560.0;
const ROW: f32 = 30.0;

/// How many recent workspaces the panel offers.
///
/// Five, not the ten the session file keeps: the list is a shortcut, and a
/// shortcut with ten entries is a directory listing with extra steps.
const RECENT_SHOWN: usize = 5;

#[derive(Debug)]
pub struct Welcome {
    pub studio: Studio,
}

impl Widget for Welcome {
    fn debug_name(&self) -> &'static str {
        "Welcome"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        if !self.studio.welcome.get() {
            return SizedBox::shrink().into();
        }
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        let mut column: Vec<WidgetNode> = vec![
            Container::new()
                .padding(EdgeInsets::only(22.0, 22.0, 22.0, 4.0))
                .child(label_bold("vieww Studio", 20.0, colors.on_surface))
                .into(),
            Container::new()
                .padding(EdgeInsets::only(22.0, 0.0, 22.0, 18.0))
                .child(label(
                    "Edit a vieww screen on the left, compile it, and watch it \
                     mount on the right.",
                    12.5,
                    colors.on_surface_variant,
                ))
                .into(),
            heading("Start", colors),
        ];

        // **First, because it is why most people opened this — and it is the
        // one row that ends in a rendered screen rather than a dialog.** A
        // stranger's first question is not "Rust or not Rust", it is "what
        // does this do?", and this answers it in one click: an untitled Say
        // counter appears, compiles, and mounts. No folder, no name, no
        // Cargo.toml, no toolchain conversation. Say is the default, not a
        // question — every other row is still here underneath it.
        column.push(self.start_row(
            "Try a screen right now",
            "An editable Say counter that renders straight away. No project, no setup.",
            colors,
            Command::TrySayScreen,
        ));
        column.push(self.start_row(
            "New project\u{2026}",
            "A folder, a Cargo.toml and a screen that already renders.",
            colors,
            Command::NewProject,
        ));
        column.push(self.start_row(
            "Open a folder\u{2026}",
            "A Rust project. Its Cargo.toml decides whether Render works.",
            colors,
            Command::OpenFolder,
        ));
        column.push(self.start_row(
            "New screen from a template",
            "A file that compiles and renders, to start from.",
            colors,
            Command::NewScreen,
        ));

        let recent = self.studio.recent.get();
        // Filtered at display time, not when the list is written: a project on
        // an unmounted drive is not gone, it is not here *now*, and forgetting
        // it because a volume was unplugged is the wrong answer.
        let openable: Vec<_> = recent
            .iter()
            .filter(|path| path.is_dir())
            .take(RECENT_SHOWN)
            .cloned()
            .collect();

        column.push(heading("Recent", colors));
        if openable.is_empty() {
            column.push(
                Container::new()
                    .padding(EdgeInsets::only(22.0, 2.0, 22.0, 12.0))
                    .child(label(
                        "Nothing yet. Folders you open show up here.",
                        11.5,
                        colors.outline,
                    ))
                    .into(),
            );
        } else {
            for path in openable {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |n| n.to_string_lossy().into(),
                );
                let full = path.display().to_string();
                let studio = self.studio.clone();
                let target = path.clone();
                column.push(
                    clickable(
                        move || {
                            Container::new()
                                .height(ROW)
                                .padding(EdgeInsets::symmetric(22.0, 0.0))
                                .alignment(Alignment::CENTER_LEFT)
                                .child(
                                    Flex::row()
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .children(children![
                                            label_bold(&name, 12.0, colors.on_surface),
                                            space(10.0),
                                            Flexible::expanded(1).child(mono(
                                                &full,
                                                10.5,
                                                colors.outline
                                            )),
                                        ]),
                                )
                                .into()
                        },
                        move || {
                            studio.open_workspace(&target);
                            studio.welcome.set(false);
                        },
                    )
                    .into(),
                );
            }
        }

        column.push(heading("Worth knowing", colors));
        // The palette, first and by name. It is the studio's best feature and
        // nothing pointed at it.
        for (chord, what) in self.hints() {
            column.push(
                Container::new()
                    .padding(EdgeInsets::only(22.0, 3.0, 22.0, 3.0))
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                Container::new().width(96.0).child(mono(
                                    &chord,
                                    11.0,
                                    colors.primary
                                )),
                                Flexible::expanded(1).child(label(
                                    &what,
                                    11.5,
                                    colors.on_surface_variant
                                )),
                            ]),
                    )
                    .into(),
            );
        }

        column.push(space(12.0));
        column.push(hairline(chrome.line));
        column.push(self.footer(colors, chrome));

        let studio = self.studio.clone();
        Stack::new()
            .alignment(Alignment::TOP_CENTER)
            .children(children![
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(clickable(
                        move || Container::new().color(Color::rgba(0, 0, 0, 110)).into(),
                        move || studio.welcome.set(false),
                    )),
                Positioned::new().top(56.0).child(
                    Container::new()
                        .color(chrome.chrome_2)
                        .radius(12.0)
                        .width(WIDTH)
                        .border(Border {
                            color: chrome.line,
                            width: 1.0,
                        })
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

widget_node_from!(Welcome);

impl Welcome {
    /// The chords worth learning on day one, rendered for *this* platform.
    ///
    /// Read from `Command::chord` rather than typed here, so a shortcut that
    /// moves does not leave a welcome screen teaching the old one.
    fn hints(&self) -> Vec<(String, String)> {
        let host = self.studio.host;
        [
            // **What it actually opens on.** This line used to promise "every
            // command, searchable" — and the palette opens in *files* mode, so
            // the first thing a new user did with the first shortcut they were
            // taught was type a command name and be told "No file in this
            // workspace matches". The prefixes are the feature; the welcome is
            // the one place they can be taught before they are needed.
            (
                Command::CommandPalette,
                "Files by name — then > for commands, @ for symbols, : for a line.",
            ),
            (
                Command::Render,
                "Compile the buffer and mount it in the preview.",
            ),
            (Command::Find, "Find in this file."),
            (Command::ShowSearch, "Search every file in the workspace."),
            (Command::ShowShortcuts, "The rest of the shortcuts."),
        ]
        .into_iter()
        .filter_map(|(command, what)| {
            // Through the studio, so a rebound chord is the one the welcome
            // screen teaches. A cheat sheet showing the default a user has
            // already changed is worse than no cheat sheet.
            self.studio
                .chord_for(command)
                .map(|chord| (chord.describe(host), what.to_owned()))
        })
        .collect()
    }

    fn start_row(
        &self,
        title: &str,
        detail: &str,
        colors: ColorScheme,
        command: Command,
    ) -> WidgetNode {
        let studio = self.studio.clone();
        let title = title.to_owned();
        let detail = detail.to_owned();
        clickable(
            move || {
                Container::new()
                    .padding(EdgeInsets::symmetric(22.0, 8.0))
                    .child(
                        Flex::column()
                            .main_axis_size(MainAxisSize::Min)
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .children(children![
                                label_bold(&title, 12.5, colors.primary),
                                label(&detail, 11.0, colors.outline),
                            ]),
                    )
                    .into()
            },
            move || {
                studio.welcome.set(false);
                studio.run(command);
            },
        )
        .into()
    }

    fn footer(&self, colors: ColorScheme, chrome: StudioTheme) -> WidgetNode {
        let dismissing = self.studio.clone();
        let about = self.studio.clone();
        Container::new()
            .padding(EdgeInsets::symmetric(16.0, 12.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        button(
                            "About vieww Studio",
                            move || label("About", 11.5, colors.on_surface_variant).into(),
                            move || about.about.set(true)
                        ),
                        Flexible::expanded(1).child(SizedBox::shrink()),
                        button(
                            "Close",
                            move || {
                                Container::new()
                                    .color(colors.primary)
                                    .radius(6.0)
                                    .padding(EdgeInsets::symmetric(16.0, 8.0))
                                    .alignment(Alignment::CENTER)
                                    .child(label_bold("Get started", 12.0, colors.on_primary))
                                    .into()
                            },
                            move || dismissing.welcome.set(false)
                        ),
                    ]),
            )
            .border(Border {
                color: Color::TRANSPARENT,
                width: 0.0,
            })
            .color(chrome.chrome_1)
            .into()
    }
}

/// A small section heading inside the card.
fn heading(text: &str, colors: ColorScheme) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::only(22.0, 12.0, 22.0, 4.0))
        .child(label_bold(text, 10.5, colors.outline))
        .into()
}

/// Version, build and licence.
///
/// The studio's version existed in exactly one place — a sixty-pixel status-bar
/// cell — and there was no About box, no licence screen and no build
/// identifier. Support for a shipped desktop application begins with "what
/// version are you on?", and the answer has to be somewhere a person can find
/// and copy.
#[derive(Debug)]
pub struct About {
    pub studio: Studio,
}

impl Widget for About {
    fn debug_name(&self) -> &'static str {
        "About"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        if !self.studio.about.get() {
            return SizedBox::shrink().into();
        }
        let closing = self.studio.clone();
        let copying = self.studio.clone();
        let report = crate::about::report();

        let mut dialog = crate::ui::dialog::Dialog::new(
            "vieww Studio",
            "A code editor and device-framed preview for vieww screens, built with vieww.",
        )
        .items(crate::about::lines());
        dialog.footnote = None;
        dialog
            .action(crate::ui::dialog::Action::new(
                "Close",
                crate::ui::dialog::Tone::Primary,
                move || closing.about.set(false),
            ))
            // The reason an About box exists at all: someone has to paste this
            // into a bug report.
            .action(crate::ui::dialog::Action::new(
                "Copy details",
                crate::ui::dialog::Tone::Neutral,
                move || copying.copy_text(&report),
            ))
            .into()
    }
}

widget_node_from!(About);
