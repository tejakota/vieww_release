//! The bottom panel: Problems, Output, Tasks, rustc, Timings.
//!
//! Collapsible to its tab strip rather than closable, so the panel's own height
//! is never a thing the user has to restore from a menu.

use vieww_foundation::{Alignment, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Animated, Container, Flexible, Positioned, Semantics, Stack};

use vieww_element::ScrollController;
use vieww_widget::ScrollExtents;

use crate::command::Command;
use crate::state::{Diagnostic, PanelTab, Severity, Studio};
use crate::theme::StudioTheme;
use crate::ui::chrome::{
    clickable, gap, hairline, icon_button, label, label_bold, mono, segmented, space,
    SegmentedColors,
};
use crate::ui::icons;

const TABS: f32 = 32.0;

#[derive(Debug)]
pub struct Panel {
    pub studio: Studio,
}

impl Widget for Panel {
    fn debug_name(&self) -> &'static str {
        "Panel"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let open = self.studio.panel_open.get();
        let tab = self.studio.panel_tab.get();
        let diagnostics = self.studio.diagnostics.get();
        let output = self.studio.output.get();
        let timings = self.studio.timings.get();

        let mut column = vec![self.tabs(ctx, &chrome, theme.colors, tab, diagnostics.len())];

        if open {
            column.push(hairline(chrome.line));
            let scroll = self.studio.panel_scroll(tab).clone();
            column.push(
                Container::new()
                    .color(chrome.chrome_1)
                    // Dragged through the seam above; see `ui::divider`. It was
                    // a constant until the seam existed, and the constant is
                    // still the height the studio opens at.
                    .height(self.studio.panel_height.get())
                    .child(
                        Stack::new()
                            .fit(vieww_widget::StackFit::Expand)
                            .alignment(Alignment::TOP_LEFT)
                            .children(children![
                                vieww_widget::Clip::rect().child(
                                    Scrollable::vertical(scroll.offset())
                                        .key(tab.label())
                                        .on_drag(scroll.on_drag())
                                        .on_drag_end(scroll.on_drag_end())
                                        .on_extents(follow_the_end(&scroll))
                                        .child(body(
                                            ctx,
                                            &Contents {
                                                studio: &self.studio,
                                                chrome: &chrome,
                                                colors: theme.colors,
                                                tab,
                                                diagnostics: &diagnostics,
                                                output: &output,
                                                timings: &timings,
                                            },
                                        )),
                                ),
                                // Over the log rather than beside it: the panel
                                // is 176 points tall and a column of chrome
                                // taken out of it is a line of the build you
                                // cannot read.
                                Positioned::new().right(2.0).top(2.0).bottom(2.0).child(
                                    crate::ui::scrollbar::Scrollbar {
                                        scroll: scroll.clone(),
                                        axis: vieww_foundation::Axis::Vertical,
                                        chrome: *chrome,
                                        color: chrome.chrome_3,
                                    }
                                ),
                            ]),
                    )
                    .into(),
            );
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(column)
            .into()
    }
}

widget_node_from!(Panel);

/// The extents handler that keeps a running build at the newest line.
///
/// # The behaviour this replaces
///
/// The panel appended and stayed where it was. A `cargo build` writes its output
/// over a minute or two, and every line of it landed below the fold: the way to
/// watch a build was to scroll down, wait, scroll down again — and the line that
/// mattered, the first error, went past unseen while the reader was still
/// looking at the top of the file list.
///
/// # Following, and stopping following
///
/// The rule is the one every terminal uses, and it needs no flag to remember:
/// **if you were at the end before this line arrived, you are at the end after
/// it.** `was_at_end` is measured against the *old* extent — before `resize`
/// moves the goalposts — so scrolling up genuinely detaches, and scrolling back
/// down re-attaches without a control to toggle.
///
/// `SLACK` is about a line: a reader who is one pixel off the bottom is
/// following the log, not studying it.
fn follow_the_end(scroll: &ScrollController) -> vieww_widget::Handler<ScrollExtents> {
    const SLACK: f32 = 24.0;
    let scroll = scroll.clone();
    std::rc::Rc::new(move |extents: ScrollExtents| {
        let was_at_end = scroll.peek() >= scroll.max_offset() - SLACK;
        scroll.resize(extents);
        if was_at_end {
            scroll.jump_to(scroll.max_offset());
        }
    })
}

impl Panel {
    fn tabs(
        &self,
        ctx: &BuildContext,
        chrome: &StudioTheme,
        colors: ColorScheme,
        current: PanelTab,
        problems: usize,
    ) -> WidgetNode {
        // Read out where a `BuildContext` still exists: the builders below
        // outlive this build. See `chrome::quick`.
        let (duration, curve) = crate::ui::chrome::quick(ctx);
        let tabs = PanelTab::ALL
            .iter()
            .copied()
            .map(|tab| {
                let selected = tab == current;
                let signal = self.studio.panel_tab.clone();
                let open = self.studio.panel_open.clone();
                let count = matches!(tab, PanelTab::Problems).then_some(problems);
                let chrome = *chrome;

                // `Tab` rather than `Button`: a screen reader says "tab, 2 of
                // 5, selected" for one and "button" for the other, and the
                // panel is a set of pages.
                Semantics::new()
                    .role(vieww_widget::SemanticRole::Tab)
                    .label(tab.label())
                    .child(crate::ui::chrome::sensed(
                        move |sense| {
                            let foreground = if selected {
                                colors.on_surface
                            } else {
                                colors.on_surface_variant
                            };

                            // The selected tab lifts off the strip. The strip is
                            // `chrome.chrome_1` and the unselected tab is
                            // transparent — so without a fill on the selected
                            // one, both render as the same colour and the only
                            // differentiator is the marker below. `chrome.chrome_2`
                            // is the editor surface, lighter than the strip in both
                            // themes, and is the same colour the editor tab strip
                            // uses for its selected tab — so the two strips read
                            // the same way and a user who has learned one does not
                            // have to relearn the other.
                            let background = if selected {
                                chrome.chrome_2
                            } else {
                                crate::ui::chrome::hovered(chrome.chrome_1, &chrome, sense)
                            };

                            let mut row: Vec<WidgetNode> =
                                vec![label_bold(&tab.label().to_uppercase(), 11.0, foreground)
                                    .into()];

                            if let Some(count) = count {
                                row.push(space(6.0));
                                row.push(badge(count, colors, chrome));
                            }

                            // The marker grows out of the strip rather than
                            // switching on, and it is pinned over the tab
                            // rather than stacked under it — see
                            // `editor.rs`'s tab strip for both arguments, and
                            // `chrome::behind` for why a painting has to be
                            // positioned to be sized.
                            let marker = Animated::new(if selected { 1.0 } else { 0.0 })
                                .duration(duration)
                                .curve(curve)
                                .key(format!("panel-marker-{}", tab.label()))
                                .build(move |t| {
                                    crate::ui::chrome::TabIndicator {
                                        accent: chrome.accent,
                                        accent_soft: chrome.accent_soft,
                                        t,
                                    }
                                    .widget()
                                });

                            Stack::new()
                                .children(children![
                                    Container::new()
                                        .color(background)
                                        .padding(EdgeInsets::symmetric(9.0, 0.0))
                                        .alignment(Alignment::CENTER)
                                        .child(
                                            Flex::row()
                                                .main_axis_size(MainAxisSize::Min)
                                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                                .children(row)
                                        ),
                                    Positioned::new()
                                        .left(0.0)
                                        .right(0.0)
                                        .bottom(0.0)
                                        .height(3.0)
                                        .child(marker),
                                ])
                                .into()
                        },
                        move || {
                            // Clicking the tab you are on collapses the panel: the
                            // strip stays, so the panel is one click from coming
                            // back and never disappears entirely.
                            if selected {
                                open.set(!open.peek());
                            } else {
                                signal.set(tab);
                                open.set(true);
                            }
                        },
                    ))
                    .into()
            })
            .collect::<Vec<WidgetNode>>();

        Container::new()
            .color(chrome.chrome_1)
            .height(TABS)
            .padding(EdgeInsets::symmetric(8.0, 0.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(
                        tabs.into_iter()
                            .chain([
                                gap(),
                                // Was `|| {}` — drawn, hovering, and inert.
                                icon_button(icons::trash(), 13.0, colors.on_surface_variant, {
                                    let studio = self.studio.clone();
                                    move || studio.run(Command::ClearOutput)
                                }),
                                icon_button(icons::close(), 13.0, colors.on_surface_variant, {
                                    let open = self.studio.panel_open.clone();
                                    move || open.set(false)
                                }),
                            ])
                            .collect::<Vec<_>>(),
                    ),
            )
            .into()
    }
}

fn badge(count: usize, colors: ColorScheme, chrome: StudioTheme) -> WidgetNode {
    let bad = count > 0;
    crate::ui::chrome::pill(
        &count.to_string(),
        if bad {
            colors.on_error
        } else {
            colors.on_surface_variant
        },
        if bad { colors.error } else { chrome.chrome_4 },
    )
}

/// Which tab's contents to draw, and everything they are drawn from.
///
/// A struct because the argument list had reached eight — `ctx` was the one
/// that tipped it — and eight positional arguments of which four are slices is
/// a call site nobody can read and a swap nobody would notice.
struct Contents<'a> {
    studio: &'a Studio,
    chrome: &'a StudioTheme,
    colors: ColorScheme,
    tab: PanelTab,
    diagnostics: &'a [Diagnostic],
    output: &'a [String],
    timings: &'a [(String, String)],
}

fn body(ctx: &BuildContext, contents: &Contents<'_>) -> WidgetNode {
    let Contents {
        studio,
        chrome,
        colors,
        tab,
        diagnostics,
        output,
        timings,
    } = *contents;
    match tab {
        PanelTab::Problems => problems(ctx, studio, chrome, colors, diagnostics),
        PanelTab::Output => {
            if output.is_empty() {
                return note(colors, "Nothing has been compiled yet.");
            }
            log(
                colors,
                &output.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
        PanelTab::Run => run_box(studio, chrome, colors),
        PanelTab::Tasks => tasks(studio, chrome, colors),
        PanelTab::Rustc => {
            // The raw JSON is what the Problems list was parsed from, kept so a
            // diagnostic the parser dropped is still reachable. Until a compile
            // has run there is nothing to show, and saying so beats an empty
            // pane that looks broken.
            let raw = studio.rustc_json.get();
            if raw.is_empty() {
                return note(colors, "rustc has not been run yet.");
            }
            log(colors, &raw.iter().map(String::as_str).collect::<Vec<_>>())
        }
        PanelTab::Timings => {
            if timings.is_empty() {
                return note(colors, "No render has been timed yet.");
            }
            let lines: Vec<String> = timings
                .iter()
                .map(|(what, value)| format!("{what:<16}{value}"))
                .collect();
            log(
                colors,
                &lines.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        }
    }
}

/// §8.1's queue: every child process the studio has started.
///
/// # Why finished jobs stay
///
/// The question a person asks this panel is almost never "what is running" —
/// the status bar answers that. It is "did that finish, and how long did it
/// take", which is a question about something that is already over. A list
/// that emptied itself on completion could not answer it.
fn tasks(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    // Read so the panel rebuilds when the queue changes. The queue itself is
    // not a signal — it holds `Task`s, which are not `Clone` — so this is the
    // subscription.
    let _generation = studio.jobs_generation.get();

    let queue = studio.jobs.borrow();
    if queue.jobs().is_empty() {
        return note(
            colors,
            "Nothing has been run yet. Builds, formats and exports all appear here.",
        );
    }

    let rows = queue
        .jobs()
        .iter()
        .rev()
        .map(|job| {
            let running = job.state.is_running();
            let tint = match &job.state {
                crate::jobs::State::Running => colors.primary,
                crate::jobs::State::Ended(outcome) if outcome.succeeded() => {
                    colors.on_surface_variant
                }
                crate::jobs::State::Ended(_) => colors.error,
            };
            let elapsed = format!("{:.1}s", job.elapsed().as_secs_f32());

            let mut right: Vec<WidgetNode> = vec![
                mono(&elapsed, 11.0, colors.on_surface_variant).into(),
                space(10.0),
                label(job.state.label(), 11.0, tint).into(),
            ];
            // A cancel only where there is something to cancel. A stop button
            // on a finished row is a button that teaches people it does
            // nothing.
            if job.is_cancellable() {
                right.push(space(6.0));
                right.push(icon_button(
                    icons::close(),
                    12.0,
                    colors.on_surface_variant,
                    {
                        let studio = studio.clone();
                        let id = job.id;
                        move || studio.cancel_job(id)
                    },
                ));
            }

            Container::new()
                .padding(EdgeInsets::symmetric(12.0, 7.0))
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            // A dot rather than a spinner: the panel is a list
                            // and a list of spinners is a list nobody can read.
                            Container::new()
                                .color(if running {
                                    colors.primary
                                } else {
                                    chrome.chrome_4
                                })
                                .radius(3.0)
                                .size(6.0, 6.0),
                            space(9.0),
                            Flexible::expanded(1).child(
                                Flex::column()
                                    .cross_axis_alignment(CrossAxisAlignment::Start)
                                    .children(children![
                                        label(&job.label, 12.0, colors.on_surface),
                                        space(2.0),
                                        // The command line, so a failure can be
                                        // reproduced in a terminal without
                                        // guessing what the studio ran.
                                        mono(&job.command, 10.5, colors.on_surface_variant),
                                    ])
                            ),
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .children(right),
                        ]),
                )
                .into()
        })
        .collect::<Vec<WidgetNode>>();

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(rows)
        .into()
}

/// What a panel says when it has nothing to say.
fn note(colors: ColorScheme, text: &str) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 10.0))
        .child(label(text, 12.0, colors.on_surface_variant))
        .into()
}

fn problems(
    ctx: &BuildContext,
    studio: &Studio,
    chrome: &StudioTheme,
    colors: ColorScheme,
    diagnostics: &[Diagnostic],
) -> WidgetNode {
    if diagnostics.is_empty() {
        return note(colors, "No problems. The buffer compiled clean.");
    }

    let filter = studio.problem_filter.get();
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = diagnostics.len() - errors;

    // The filter strip. Counted labels rather than bare words, because "3" and
    // "0" is the answer to the question the strip is there to ask and reading
    // it should not require also clicking it.
    let strip = Container::new()
        .padding(EdgeInsets::symmetric(10.0, 6.0))
        .alignment(Alignment::CENTER_LEFT)
        .child(segmented(
            ctx,
            &[
                &format!("All {}", diagnostics.len()),
                &format!("Errors {errors}"),
                &format!("Warnings {warnings}"),
            ],
            match filter {
                None => 0,
                Some(Severity::Error) => 1,
                Some(Severity::Warning) => 2,
            },
            SegmentedColors {
                track: chrome.chrome_1,
                border: chrome.line,
                accent: chrome.accent,
                accent_far: chrome.accent_soft,
                on_accent: Color::WHITE,
                muted: colors.on_surface_variant,
            },
            {
                let signal = studio.problem_filter.clone();
                move |index| {
                    signal.set(match index {
                        1 => Some(Severity::Error),
                        2 => Some(Severity::Warning),
                        _ => None,
                    });
                }
            },
        ));

    let visible: Vec<&Diagnostic> = diagnostics
        .iter()
        .filter(|d| filter.is_none_or(|wanted| d.severity == wanted))
        .collect();

    if visible.is_empty() {
        return Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![
                strip,
                note(
                    colors,
                    match filter {
                        Some(Severity::Error) => "No errors. There are warnings.",
                        Some(Severity::Warning) => "No warnings. There are errors.",
                        None => "No problems.",
                    }
                ),
            ])
            .into();
    }

    let rows = visible
        .iter()
        .copied()
        .map(|diagnostic| {
            let severity_color = match diagnostic.severity {
                Severity::Error => colors.error,
                Severity::Warning => chrome.warning,
            };
            // The two icons differ in *shape* as well as colour — a filled
            // circle against a triangle — because the severity has to survive
            // being read by someone who cannot tell the error red from the
            // warning amber.
            let glyph = match diagnostic.severity {
                Severity::Error => icons::error(),
                Severity::Warning => icons::warning(),
            };

            let diagnostic = diagnostic.clone();
            let jump = {
                let studio = studio.clone();
                let diagnostic = diagnostic.clone();
                move || {
                    // Open the file the diagnostic is about *first*: jumping to
                    // line 24 of whatever happens to be showing is the bug this
                    // panel used to have.
                    studio.open_named(&diagnostic.file);
                    studio.jump_to(diagnostic.line, diagnostic.column);
                }
            };

            clickable(
                move || {
                    let mut lines: Vec<WidgetNode> = vec![Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            label(&diagnostic.message, 12.5, colors.on_surface),
                            space(8.0),
                            mono(&diagnostic.code, 11.0, colors.outline),
                        ])
                        .into()];

                    if let Some(help) = &diagnostic.help {
                        lines.push(
                            label(&format!("help: {help}"), 11.5, colors.on_surface_variant).into(),
                        );
                    }

                    Container::new()
                        .padding(EdgeInsets::symmetric(12.0, 5.0))
                        .child(
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(children![
                                    crate::ui::chrome::glyph(glyph.clone(), 13.0, severity_color),
                                    space(8.0),
                                    Flexible::expanded(1).child(
                                        Flex::column()
                                            .cross_axis_alignment(CrossAxisAlignment::Start)
                                            .spacing(2.0)
                                            .children(lines)
                                    ),
                                    space(8.0),
                                    // The file, not just the line: the panel
                                    // lists every buffer's problems, and
                                    // "17:31" alone is a location in whichever
                                    // file the reader happens to be looking at.
                                    mono(
                                        &format!("{} {}", diagnostic.file, diagnostic.location()),
                                        11.0,
                                        colors.outline,
                                    ),
                                ]),
                        )
                        .into()
                },
                jump,
            )
            .into()
        })
        .collect::<Vec<WidgetNode>>();

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(
            std::iter::once(strip.into())
                .chain(rows)
                .collect::<Vec<_>>(),
        )
        .into()
}

/// A fixed-pitch log. Output, rustc's JSON and the timings are all this shape.
fn log(colors: ColorScheme, lines: &[&str]) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 8.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(3.0)
                .children(
                    lines
                        .iter()
                        .map(|line| mono(line, 11.5, colors.on_surface_variant).into())
                        .collect::<Vec<WidgetNode>>(),
                ),
        )
        .into()
}

/// The Run box: a command line that is not a shell and not a terminal.
///
/// What it is and — more to the point — what it is not is on
/// [`Studio::run_command`]. The caption below says the same thing to the user
/// in one sentence, because a box that looks like a terminal and is not one is
/// worse than no box at all.
fn run_box(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    use vieww_foundation::{FontFamily, TextStyle};
    use vieww_widget::TextField;

    let text = studio.run_input.get();
    let history = studio.run_history.get();
    let has_root = studio.root.get().is_some();

    let field = {
        let studio = studio.clone();
        TextField::text(text.clone())
            .size(12.0)
            .style(
                TextStyle::new(12.0)
                    .family(FontFamily::Monospace)
                    .color(colors.on_surface),
            )
            .placeholder(if has_root {
                "cargo test"
            } else {
                "open a folder first"
            })
            .selection_color(chrome.selection)
            .cursor(colors.primary, 2.0)
            .on_changed({
                let signal = studio.run_input.clone();
                std::rc::Rc::new(move |value: vieww_foundation::TextEditingValue| {
                    signal.set(value.text);
                })
            })
            // Enter runs it, which is the only thing anybody will try.
            .on_submit(std::rc::Rc::new(move |_| studio.run_typed_command()))
    };

    let mut rows: Vec<WidgetNode> = vec![
        Container::new()
            .padding(EdgeInsets::symmetric(12.0, 10.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        Flexible::expanded(1).child(
                            Container::new()
                                .color(chrome.chrome_2)
                                .radius(6.0)
                                .padding(EdgeInsets::symmetric(10.0, 7.0))
                                .child(field)
                        ),
                        crate::ui::chrome::space(8.0),
                        crate::ui::chrome::button(
                            "Run",
                            {
                                move || {
                                    Container::new()
                                        .color(colors.primary)
                                        .radius(6.0)
                                        .padding(EdgeInsets::symmetric(14.0, 8.0))
                                        .alignment(vieww_foundation::Alignment::CENTER)
                                        .child(label("Run", 12.0, colors.on_primary))
                                        .into()
                                }
                            },
                            {
                                let studio = studio.clone();
                                move || studio.run_typed_command()
                            }
                        ),
                    ]),
            )
            .into(),
        Container::new()
            .padding(EdgeInsets::only(12.0, 0.0, 12.0, 8.0))
            .child(label(
                "Runs one program in the workspace and streams its output to the \
                 Output tab. Not a shell — no pipes, no globs, no redirection — and \
                 not a terminal: anything that wants one is refused rather than left \
                 hanging. Cancel from the Tasks tab.",
                10.5,
                colors.outline,
            ))
            .into(),
    ];

    if history.is_empty() {
        rows.push(note(colors, "Nothing run yet this session."));
    } else {
        rows.push(
            Container::new()
                .padding(EdgeInsets::only(12.0, 4.0, 12.0, 4.0))
                .child(label("Recent", 10.5, colors.outline))
                .into(),
        );
        for entry in history.iter() {
            let studio = studio.clone();
            let line = entry.clone();
            let shown = entry.clone();
            rows.push(
                crate::ui::chrome::clickable(
                    move || {
                        Container::new()
                            .height(24.0)
                            .padding(EdgeInsets::symmetric(12.0, 0.0))
                            .alignment(vieww_foundation::Alignment::CENTER_LEFT)
                            .child(mono(&shown, 11.0, colors.on_surface_variant))
                            .into()
                    },
                    move || {
                        // Put it in the box as well as running it, so a command
                        // that needs one word changed is one edit away.
                        studio.run_input.set(line.clone());
                        studio.run_command(&line);
                    },
                )
                .into(),
            );
        }
    }

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .children(rows)
        .into()
}
