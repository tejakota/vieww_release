//! The bottom strip: render state, device, toolchain stamp, caret.

use vieww_foundation::{Alignment, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container};

use crate::state::{Activity, PreviewState, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{gap, label, space};
use crate::ui::icons;

pub const HEIGHT: f32 = 24.0;

#[derive(Debug)]
pub struct StatusBar {
    pub studio: Studio,
}

impl Widget for StatusBar {
    fn debug_name(&self) -> &'static str {
        "StatusBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let colors = theme.colors;
        let muted = colors.on_surface_variant;

        let state = self.studio.preview.get();
        // Read so the cell rebuilds when the queue changes; the queue itself
        // is not a signal. See `Studio::jobs_generation`.
        let _jobs_generation = self.studio.jobs_generation.get();
        let running = self.studio.jobs.borrow().running();
        let analyzer = self.studio.analyzer_state.get();
        let platform = self.studio.platform.get();
        let (toolchain, abi) = match self.studio.toolchain.as_ref() {
            Ok(found) => (found.stamp(), "ABI ok"),
            Err(_) => ("no rustc".to_string(), "ABI unchecked"),
        };

        // **Two pipelines, one light.** `preview` tracks the studio's own
        // `rustc`-`dlopen`-mount render; `build_state` tracks `cargo
        // build`/`run` against the open project. Neither is a stage of the
        // other, and this cell reports both.
        //
        // It used to report the preview whenever no build was *in flight*,
        // which produced the defect this arbitration exists for: press Render
        // on a file with no `screen()` (fails), then press Build (succeeds
        // five minutes later), and the Output panel says `Build finished in
        // 336.8s` while the light two inches below it says a red **Failed**.
        // Both cells are correct about their own pipeline and the pair is
        // unreadable — everyone who saw it concluded the build had failed,
        // because the build is what they had just sat through.
        //
        // So the rule is the one every status bar uses: **the most recent
        // thing wins**, and `Studio::activity` is what records which that was.
        // A running build still takes precedence outright, because an
        // operation in flight is more interesting than any finished one.
        let build = self.studio.build_state.get();
        let building = build.busy();
        let show_build =
            building || (self.studio.activity.get() == Activity::Build && build.reportable());

        // The state light is a colour *and* a word. Colour alone fails the one
        // reader the success role's own docs call out.
        let light = if show_build {
            if building {
                chrome.warning
            } else if build.is_failure() {
                colors.error
            } else {
                colors.success
            }
        } else {
            match state {
                PreviewState::Rendered => colors.success,
                PreviewState::Failed | PreviewState::AbiRefused => colors.error,
                PreviewState::Compiling => chrome.warning,
                PreviewState::Empty => colors.outline,
            }
        };

        // The word matches the light, and **names its pipeline**: "Build
        // succeeded", "Render failed". A bare "Failed" beside a finished build
        // is precisely the ambiguity above, and a word that says which of the
        // two it belongs to cannot reproduce it even if the arbitration above
        // is ever wrong.
        let state_label = if show_build {
            build.describe()
        } else {
            state.status_label().to_string()
        };

        let timing: WidgetNode = match self.studio.last_render.get() {
            Some(duration) => {
                let mut row: Vec<WidgetNode> = vec![
                    crate::ui::chrome::glyph(icons::clock(), 12.0, muted).into(),
                    space(5.0),
                    label(&format!("{:.2}s", duration.as_secs_f32()), 11.0, muted).into(),
                ];
                if self.studio.is_compiling() {
                    row.push(space(5.0));
                    row.push(label("· compiling", 11.0, chrome.warning).into());
                }
                Container::new()
                    .padding(EdgeInsets::symmetric(8.0, 0.0))
                    .alignment(Alignment::CENTER)
                    .child(
                        Flex::row()
                            .main_axis_size(MainAxisSize::Min)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(row),
                    )
                    .into()
            }
            None => vieww_widget::SizedBox::shrink().into(),
        };

        // Every cell is the same shape; the ones that carry an icon pass one.
        let cell = move |text: String, icon: Option<vieww_foundation::IconData>| -> WidgetNode {
            status_cell(&text, icon, muted)
        };

        // Absent when nothing is running, which is the honest state and also
        // what keeps a bar people read at a glance free of a permanent
        // "0 tasks". Clickable, because the only reason to read it is to go
        // and look at what those tasks are.
        let tasks_cell: WidgetNode = if running == 0 {
            vieww_widget::SizedBox::shrink().into()
        } else {
            let text = format!("{running} task{}", if running == 1 { "" } else { "s" });
            let studio = self.studio.clone();
            crate::ui::chrome::clickable(
                move || cell(text.clone(), Some(icons::clock())),
                move || studio.run(crate::command::Command::ShowTasks),
            )
            .into()
        };

        // Absent when nothing is folded, for the same reason the task cell is.
        let hidden = {
            let folds = self.studio.folds.get();
            if folds.is_empty() {
                0
            } else {
                let source = self
                    .studio
                    .active()
                    .map(|b| b.value.text)
                    .unwrap_or_default();
                source.split('\n').count() - self.studio.folded_view().lines.len()
            }
        };
        let folded_cell: WidgetNode = if hidden == 0 {
            vieww_widget::SizedBox::shrink().into()
        } else {
            let text = format!("{hidden} line{} folded", if hidden == 1 { "" } else { "s" });
            let studio = self.studio.clone();
            crate::ui::chrome::clickable(
                move || cell(text.clone(), None),
                move || studio.run(crate::command::Command::UnfoldAll),
            )
            .into()
        };

        // The branch, where there is a repository. Clicking it opens the
        // Source Control view, which is the only reason to look at it.
        let branch_cell: WidgetNode = if self.studio.is_repository() {
            let status = self.studio.git.get();
            let text = status.describe();
            let studio = self.studio.clone();
            crate::ui::chrome::clickable(
                move || cell(text.clone(), Some(icons::branch())),
                move || studio.run(crate::command::Command::ShowSource),
            )
            .into()
        } else {
            vieww_widget::SizedBox::shrink().into()
        };

        // The three cells that used to be two string literals, plus a
        // read-only marker. All four read the active buffer, so a `Cargo.toml`
        // says TOML and a file with no write permission says so before the
        // user has typed into it rather than when the save fails.
        // Metadata only — the four cells below are an encoding, a line
        // ending, a language and a permission, and none of them is in the text.
        let file = self.studio.active_tab();
        let encoding_cell: WidgetNode = file.as_ref().map_or_else(
            || vieww_widget::SizedBox::shrink().into(),
            |buffer| cell(buffer.encoding.name().to_string(), None),
        );
        let endings_cell: WidgetNode = file.as_ref().map_or_else(
            || vieww_widget::SizedBox::shrink().into(),
            |buffer| cell(buffer.endings.name().to_string(), None),
        );
        let language_cell: WidgetNode = file.as_ref().map_or_else(
            || vieww_widget::SizedBox::shrink().into(),
            |buffer| cell(buffer.language.name().to_string(), None),
        );
        let read_only_cell: WidgetNode = match file.as_ref() {
            Some(buffer) if buffer.read_only => cell("Read-only".to_string(), None),
            _ => vieww_widget::SizedBox::shrink().into(),
        };

        // What just happened, or why it did not.
        //
        // Between the state light and the toolchain, because it is about the
        // thing the user did most recently and that is where the eye already
        // is. Absent when there is nothing to say — a permanently occupied
        // slot showing the last thing from ten minutes ago is a status bar
        // people stop reading.
        //
        // This is where a refused snippet lands. The Output panel is where a
        // *build* talks; a refusal is about the click that just happened, and
        // routing it there would mean opening a panel to find out why a
        // sidebar row did nothing.
        let notice_cell: WidgetNode = match self.studio.notice.get() {
            Some(message) => {
                let studio = self.studio.clone();
                crate::ui::chrome::clickable(
                    move || {
                        Container::new()
                            .padding(EdgeInsets::symmetric(8.0, 0.0))
                            .alignment(Alignment::CENTER)
                            .child(label(&message, 11.0, colors.primary))
                            .into()
                    },
                    // Dismissable, because it is cleared by the next command
                    // and somebody who has read it should not have to run one.
                    move || studio.notice.set(None),
                )
                .into()
            }
            None => vieww_widget::SizedBox::shrink().into(),
        };

        let row = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(children![
                Container::new()
                    .padding(EdgeInsets::symmetric(8.0, 0.0))
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                // A disc with a halo of its own colour, so
                                // the one always-visible indicator of whether
                                // the studio has a working screen reads as a
                                // *light* rather than as a full stop.
                                crate::ui::chrome::status_dot(
                                    light,
                                    matches!(
                                        state,
                                        PreviewState::Rendered | PreviewState::Compiling
                                    ),
                                ),
                                space(6.0),
                                label(&state_label, 11.0, muted),
                            ])
                    ),
                notice_cell,
                cell(platform.describe(), Some(icons::phone())),
                // The real toolchain, or the reason there is none. A studio
                // that cannot compile should say so in the one place that is
                // always visible, not the first time somebody clicks Render.
                cell(toolchain, None),
                // The *studio's* version. It read `vieww {version}` before,
                // which is this crate's `CARGO_PKG_VERSION` wearing the
                // framework's name — two things that are free to diverge the
                // moment either is released on its own.
                cell(format!("studio {}", env!("CARGO_PKG_VERSION")), None),
                cell(abi.to_string(), Some(icons::shield())),
                // Absent when it is off, which is the default and the usual
                // state — a permanent "rust-analyzer: off" is a cell people
                // stop reading, and the whole point of this one is that
                // "starting" turns into "ready" while you watch.
                if analyzer == "off" {
                    vieww_widget::SizedBox::shrink().into()
                } else {
                    cell(format!("rust-analyzer {analyzer}"), None)
                },
                gap(),
                // Its own element: this is the one cell that changes on every
                // keystroke and every arrow key, and reading the caret here put
                // the whole bar — toolchain, git, language, encoding, timing —
                // on that signal. See `CaretCells`.
                WidgetNode::from(CaretCells {
                    studio: self.studio.clone(),
                }),
                cell(format!("Spaces: {}", self.studio.tab_width.get()), None),
                tasks_cell,
                // Lines are hidden, and the pane does not otherwise say so.
                // A file that is missing half its content with no explanation
                // is the one failure mode folding has.
                folded_cell,
                branch_cell,
                // **These two used to be string literals.** `cell("UTF-8")`
                // and `cell("Rust")`, side by side, reporting the same two
                // words over a Latin-1 file and over a `Cargo.toml`. A status
                // bar that reports a constant is worse than one that reports
                // nothing, because the user believes it. All three now come
                // off the buffer — and the line-ending cell is new, because
                // "which endings does this file use" is the question a
                // Windows-authored file makes people ask.
                read_only_cell,
                encoding_cell,
                endings_cell,
                language_cell,
                // What the last render actually cost. This cell used to read
                // `1.92s` from a string literal — on a studio that had never
                // compiled anything, and unchanged after one that took four
                // seconds. Absent until there is a number to show, because an
                // absent cell is honest and a made-up one is not.
                timing,
            ]);

        Container::new()
            .color(chrome.chrome_0)
            .height(HEIGHT)
            .padding(EdgeInsets::symmetric(4.0, 0.0))
            .child(row)
            .into()
    }
}

widget_node_from!(StatusBar);

/// One cell of the status bar: a label, optionally behind an icon.
///
/// A free function rather than a closure inside `StatusBar::build`, so
/// [`CaretCells`] draws cells that are the same shape by construction rather
/// than by two copies of the same six lines.
fn status_cell(
    text: &str,
    icon: Option<vieww_foundation::IconData>,
    muted: vieww_foundation::Color,
) -> WidgetNode {
    let mut row: Vec<WidgetNode> = Vec::new();
    if let Some(icon) = icon {
        row.push(crate::ui::chrome::glyph(icon, 12.0, muted).into());
        row.push(space(5.0));
    }
    row.push(label(text, 11.0, muted).into());

    Container::new()
        .padding(EdgeInsets::symmetric(8.0, 0.0))
        .alignment(Alignment::CENTER)
        .child(
            Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(row),
        )
        .into()
}

/// The caret readout, and the multiple-cursor count beside it.
///
/// # Why the two cells are their own element
///
/// They are the only part of the status bar that a keystroke changes, and they
/// were read at the top of `StatusBar::build` — so `Ln 1, Col 2` becoming
/// `Ln 1, Col 3` rebuilt the render light, the platform picker, the toolchain
/// stamp, the ABI mark, the git branch, the language, the encoding, the line
/// endings and the timing: **24 elements** to redraw one number, on every
/// character typed and every arrow key.
///
/// Split out, the bar itself now reads nothing a keystroke writes, and this
/// element is six.
#[derive(Debug)]
pub struct CaretCells {
    pub studio: Studio,
}

widget_node_from!(CaretCells);

impl Widget for CaretCells {
    fn debug_name(&self) -> &'static str {
        "CaretCells"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let muted = ThemeData::of(ctx).colors.on_surface_variant;
        let cell = move |text: String, icon: Option<vieww_foundation::IconData>| -> WidgetNode {
            status_cell(&text, icon, muted)
        };
        let (line, column) = self.studio.caret.get();
        // §7 of the handoff, honoured: if this is ever deferred again, say so
        // in the UI. It is not deferred — so the cell appears when there is
        // more than one caret and says how many, which is the difference
        // between a feature and a shortcut that silently does one thing.
        let carets = self.studio.caret_count();
        let carets_cell: WidgetNode = if carets <= 1 {
            vieww_widget::SizedBox::shrink().into()
        } else {
            let text = format!("{carets} cursors");
            let studio = self.studio.clone();
            crate::ui::chrome::clickable(
                move || cell(text.clone(), None),
                move || studio.run(crate::command::Command::ClearCursors),
            )
            .into()
        };

        Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(children![
                cell(format!("Ln {line}, Col {column}"), None),
                carets_cell,
            ])
            .into()
    }
}
