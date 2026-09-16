//! The sidebar column: a header, collapsible sections, and rows.
//!
//! **All ten views are real.** This paragraph used to say that M0 drew the
//! Explorer and left "the other six as a stated placeholder", which was true
//! once and had not been true for a long time — a reader assessing the studio
//! from its source would have concluded most of the sidebar was scaffolding.
//!
//! The ten: Explorer, Search, Snippets, Problems, Inspector, Toolchain,
//! Source, Export, Tokens and Settings. The header's two icon buttons run
//! `NewFile` and `CollapseFolders`; they were inert until the commands behind
//! them were wired, which is its own note at the call site.

use std::rc::Rc;
use vieww_foundation::{Alignment, Color, EdgeInsets, FontFamily, Offset, TextStyle};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Container, Flexible, SizedBox};

use crate::state::{Studio, View};
use crate::theme::StudioTheme;
use crate::ui::chrome::{clickable, gap, hairline, icon_button, label, label_bold, mono, space};
use crate::ui::icons;

/// Header height, matching the tab strip's so the two line up across the split.
const HEADER: f32 = 34.0;
const ROW: f32 = 22.0;
/// The most search hits drawn at once. Past this the list is longer than
/// anyone reads and the caption says how many were left out.
const MAX_HITS: usize = 200;

#[derive(Debug)]
pub struct Sidebar {
    pub studio: Studio,
}

impl Widget for Sidebar {
    fn debug_name(&self) -> &'static str {
        "Sidebar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = StudioTheme::of(ctx);
        let theme = ThemeData::of(ctx);
        let view = self.studio.view.get();
        let width = self.studio.sidebar_width.get();

        // **Collapsed means not built.**
        //
        // `ToggleSidebar` and Zen mode collapse this pane to a width of zero
        // and it went on building, laying out and reporting on every row it
        // held: twenty-odd `RenderRow overflowed by 128px — 128 of children
        // into 0 of space` warnings per frame, and the layout work behind each
        // of them, for a pane nobody can see. The width is the state (see
        // `Command::ToggleSidebar` for why it is a width rather than a bool),
        // so this is where it has to be read.
        if width <= 0.0 {
            return SizedBox::shrink().into();
        }

        let content = match view {
            View::Explorer => self.explorer(&chrome, theme.colors),
            View::Search => self.search(&chrome, theme.colors),
            View::Snippets => self.snippets(&chrome, theme.colors),
            View::Problems => self.problems(&chrome, theme.colors),
            View::Inspector => self.inspector(&chrome, theme.colors),
            View::Learn => self.learn(&chrome, theme.colors),
            View::Docs => self.docs(&chrome, theme.colors),
            View::Toolchain => self.toolchain(&chrome, theme.colors),
            View::Source => self.source(&chrome, theme.colors),
            View::Export => self.export(&chrome, theme.colors),
            View::Tokens => self.tokens(&chrome, theme.colors),
            View::Settings => self.settings(&chrome, theme.colors),
        };

        // Every view used to clip rather than scroll: nothing owned an offset
        // to give `Scrollable`, so a list past the sidebar's height was simply
        // cut off with no way to reach the rest of it. `Studio::sidebar_scroll`
        // is one controller per view (`ui/editor.rs`'s `editor_scroll` is the
        // same pattern, one level up) — keyed on `view` so switching tabs does
        // not carry one view's scroll position into another's.
        //
        // Keyed by [`vieww_widget::Key`] on the view too, not only by the
        // controller's offset changing: `Scrollable` is `WidgetKind::Composed`
        // and reconciles by position among `content`'s siblings otherwise, so
        // without a key a rebuild after switching views would try to diff the
        // old view's subtree against the new one's instead of remounting.
        let scroll = self.studio.sidebar_scroll(view).clone();
        let body = Scrollable::vertical(scroll.offset())
            .key(view.title())
            .on_drag(scroll.on_drag())
            .on_drag_end(scroll.on_drag_end())
            .on_extents(scroll.on_extents())
            .child(content);

        let column = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![
                Container::new()
                    .height(HEADER)
                    .padding(EdgeInsets::only(12.0, 0.0, 8.0, 0.0))
                    .alignment(Alignment::CENTER_LEFT)
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                label_bold(
                                    &view.title().to_uppercase(),
                                    10.5,
                                    theme.colors.on_surface_variant
                                ),
                                gap(),
                                // **Two buttons that used to do nothing.**
                                // Drawn, hoverable, clickable and inert — the
                                // same shape `panel.rs` names as a bug where it
                                // found it on the Output panel's trash icon.
                                // Both now run the command a person pressing
                                // them is asking for, so the icon, the menu and
                                // the palette are one thing.
                                //
                                // **And they belong to the Explorer.** New File
                                // and Collapse Folders are operations on a file
                                // tree, and this header is shared by all twelve
                                // views, so they sat above Snippets, Learn,
                                // Theme Tokens, Source Control and the rest —
                                // where Collapse Folders has nothing to
                                // collapse and does nothing visible when
                                // pressed. That is the inert-button defect this
                                // header's own comment was written about,
                                // reintroduced by putting working buttons in a
                                // place where their subject is absent: a
                                // control that responds to a click by doing
                                // nothing teaches people to stop trusting the
                                // toolbar it is in.
                                //
                                // A view that has no header actions gets none.
                                if view == View::Explorer {
                                    Flex::row()
                                        .main_axis_size(MainAxisSize::Min)
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .children(children![
                                            icon_button(
                                                icons::plus(),
                                                13.0,
                                                theme.colors.on_surface_variant,
                                                {
                                                    let studio = self.studio.clone();
                                                    move || {
                                                        studio.run(crate::command::Command::NewFile)
                                                    }
                                                }
                                            ),
                                            icon_button(
                                                icons::collapse_all(),
                                                13.0,
                                                theme.colors.on_surface_variant,
                                                {
                                                    let studio = self.studio.clone();
                                                    move || {
                                                        studio.run(
                                                        crate::command::Command::CollapseFolders
                                                    )
                                                    }
                                                }
                                            ),
                                        ])
                                        .into()
                                } else {
                                    let empty: WidgetNode = SizedBox::shrink().into();
                                    empty
                                },
                            ])
                    ),
                Flexible::expanded(1).child(body),
            ]);

        Container::new()
            .color(chrome.chrome_1)
            .width(width)
            .child(column)
            .into()
    }
}

widget_node_from!(Sidebar);

/// The three counts under "This file" in the Explorer.
///
/// # Why three labels are a widget
///
/// They are the only thing in the whole Explorer that reads the buffer's
/// **text**, and as three lines inside `Sidebar::build` that subscription
/// belonged to the sidebar — so every character typed rebuilt the project
/// tree, the open-buffer list and the session scratch list to recount the
/// characters in a file none of them shows.
///
/// Same fix as `editor::Breadcrumbs`: an element of its own, so the signal
/// invalidates what reads it.
#[derive(Debug)]
pub struct FileStats {
    pub studio: Studio,
}

widget_node_from!(FileStats);

impl Widget for FileStats {
    fn debug_name(&self) -> &'static str {
        "FileStats"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;
        // What is actually knowable about the open buffer is its shape, so that
        // is what it says. These were three literals once.
        let Some(buffer) = self.studio.active() else {
            return SizedBox::shrink().into();
        };
        let text = &buffer.value.text;
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(
                [
                    ("Lines", buffer.line_count().to_string()),
                    ("Characters", text.chars().count().to_string()),
                    ("Widgets named", text.matches("::new()").count().to_string()),
                ]
                .into_iter()
                .map(|(name, count)| {
                    row(
                        RowSpec {
                            text: name.to_string(),
                            trailing: Some(Trailing::Text(count)),
                            selected: false,
                            indent: 26.0,
                            leading: Leading::None,
                        },
                        &chrome,
                        colors,
                        || {},
                    )
                })
                .collect::<Vec<_>>(),
            )
            .into()
    }
}

impl Sidebar {
    /// Find across the **workspace**, not the active buffer.
    ///
    /// This view used to search `self.studio.active()` — one file, the one
    /// already on screen, which is what the find bar is for. Plan 2 §5.5 asks
    /// for find across the workspace, and `file_tree::FileTree::find` has done
    /// it since N1 with nothing calling it: the search existed, was tested, and
    /// was unreachable from the window. This is the wire.
    ///
    /// Results are grouped by file, because a flat list of ninety hits across
    /// eleven files is a list you read by scanning for the paths anyway.
    fn search(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let query = self.studio.search_query.get();
        let root = self.studio.root.get();

        let matches = if query.trim().is_empty() {
            Vec::new()
        } else {
            // Capped, and the cap is *reported* — a silent truncation reads as
            // "that is all of them", which is the one thing a search must never
            // imply. `find` walks every file in the tree on the UI thread; at
            // project size that is milliseconds, and when it stops being so it
            // becomes a `Task` like every other long operation (§8.1).
            self.studio.tree.get().find(query.trim())
        };
        let total = matches.len();
        let shown: Vec<_> = matches.into_iter().take(MAX_HITS).collect();

        // Group in file order, preserving the order `find` produced.
        let mut groups: Vec<(std::path::PathBuf, Vec<crate::file_tree::Match>)> = Vec::new();
        for hit in shown {
            match groups.last_mut() {
                Some((path, list)) if *path == hit.path => list.push(hit),
                _ => groups.push((hit.path.clone(), vec![hit])),
            }
        }

        let mut rows: Vec<WidgetNode> = Vec::new();
        for (path, hits) in groups {
            // Relative to the workspace root when there is one, so the list
            // reads `src/screens/home.rs` rather than a home directory's worth
            // of prefix repeated on every group header.
            let name = root
                .as_deref()
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            rows.push(row(
                RowSpec {
                    text: name,
                    trailing: Some(Trailing::Text(hits.len().to_string())),
                    selected: false,
                    indent: 8.0,
                    leading: Leading::File,
                },
                chrome,
                colors,
                || {},
            ));
            for hit in hits {
                let studio = self.studio.clone();
                let target = hit.path.clone();
                let (line, column) = (hit.line as u32, hit.column as u32);
                let text = hit.text.trim_start().to_string();
                let number = format!("{line:>4}");
                rows.push(
                    clickable(
                        move || {
                            Container::new()
                                .height(ROW)
                                .padding(EdgeInsets::only(20.0, 0.0, 8.0, 0.0))
                                .alignment(Alignment::CENTER_LEFT)
                                .child(
                                    Flex::row()
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .children(children![
                                            mono(&number, 10.5, colors.outline),
                                            space(8.0),
                                            Flexible::expanded(1).child(mono(
                                                &text,
                                                11.0,
                                                colors.on_surface
                                            )),
                                        ]),
                                )
                                .into()
                        },
                        move || {
                            // Open first, *then* jump: the caret has nowhere to
                            // land until the file it names is a buffer. Same
                            // order the Problems panel's jump uses, and for the
                            // same reason.
                            studio.open_path(target.clone());
                            studio.jump_to(line, column);
                        },
                    )
                    .into(),
                );
            }
        }

        let caption = if query.trim().is_empty() {
            "Type to search every file in the workspace.".to_string()
        } else if total == 0 {
            format!("No matches for \u{201c}{}\u{201d}.", query.trim())
        } else if total > MAX_HITS {
            format!("{MAX_HITS} of {total} results \u{2014} narrow the query to see the rest")
        } else {
            format!("{total} result{}", if total == 1 { "" } else { "s" })
        };

        let search = TextField::text(query)
            .size(12.0)
            .style(
                TextStyle::new(12.0)
                    .family(FontFamily::SansSerif)
                    .color(colors.on_surface),
            )
            .placeholder("Search the workspace")
            .selection_color(chrome.selection)
            .on_changed({
                let signal = self.studio.search_query.clone();
                Rc::new(move |value| signal.set(value.text))
            });

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![
                Container::new().padding(EdgeInsets::all(8.0)).child(search),
                Container::new()
                    .padding(EdgeInsets::only(10.0, 0.0, 10.0, 6.0))
                    .child(label(&caption, 11.0, colors.on_surface_variant)),
                Flexible::expanded(1).child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .children(rows)
                ),
            ])
            .into()
    }

    /// The widget library.
    ///
    /// Eleven entries out of [`crate::edit_ops::SNIPPETS`] rather than three
    /// written inline here, and every row carries the file under `crates/` its
    /// entry came from — which is the prototype's own rule for this view, and
    /// the difference between a library and a list of three strings.
    fn snippets(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let mut rows: Vec<WidgetNode> = vec![note(
            colors,
            "Inserted as whole lines, indented to the caret.",
        )];

        // **Two groups, because they go in two different places.** The list was
        // one run of eleven, ten of which were expressions to drop inside a
        // `build` — and the reader's first question is not "which widget" but
        // "what does a file look like". Items first, and said so.
        let snippet_row =
            |snippet: &'static crate::edit_ops::Snippet, studio: Studio| -> WidgetNode {
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        row(
                            RowSpec {
                                text: snippet.name.to_owned(),
                                trailing: None,
                                selected: false,
                                indent: 12.0,
                                leading: Leading::None,
                            },
                            chrome,
                            colors,
                            move || studio.insert_snippet(snippet)
                        ),
                        Container::new()
                            .padding(EdgeInsets::only(12.0, 0.0, 12.0, 6.0))
                            .child(label(snippet.source, 10.5, colors.outline)),
                    ])
                    .into()
            };

        rows.push(section_title(
            "Whole items — a file is made of these",
            colors,
        ));
        rows.extend(
            crate::edit_ops::SNIPPETS
                .iter()
                .filter(|snippet| snippet.kind == crate::edit_ops::SnippetKind::Item)
                .map(|snippet| snippet_row(snippet, self.studio.clone())),
        );

        rows.push(section_title("Inside a build — expressions", colors));
        rows.extend(
            crate::edit_ops::SNIPPETS
                .iter()
                .filter(|snippet| snippet.kind == crate::edit_ops::SnippetKind::Expression)
                .map(|snippet| snippet_row(snippet, self.studio.clone())),
        );

        // Whatever the user put in their own snippets file, after the built-in
        // eleven. Before this, `SNIPPETS` was a `const` array and the library
        // of widget starting points — in a studio whose purpose is authoring
        // widgets — could not be added to.
        let user = self.studio.user_snippets.get();
        rows.push(section_title("Yours", colors));
        if user.is_empty() {
            rows.push(note(
                colors,
                "None yet. Write Customisation Templates from the palette to \
                 get a snippets file to edit.",
            ));
        } else {
            rows.extend(user.iter().map(|snippet| {
                let studio = self.studio.clone();
                let body = snippet.body.clone();
                row(
                    RowSpec {
                        text: snippet.name.clone(),
                        trailing: None,
                        selected: false,
                        indent: 12.0,
                        leading: Leading::None,
                    },
                    chrome,
                    colors,
                    move || studio.insert_text_snippet(&body),
                )
            }));
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    fn problems(&self, _chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let diagnostics = self.studio.active_diagnostics();
        if diagnostics.is_empty() {
            return note(colors, "No problems in the active buffer.");
        }
        let rows = diagnostics
            .into_iter()
            .map(|d| {
                let studio = self.studio.clone();
                let location = d.location();
                row(
                    RowSpec {
                        // Truncated to what the column can hold. The full text is one
                        // click away in the Problems panel, and a message that runs off
                        // the edge of a 248-point sidebar — as `rustc was still running
                        // after 10s and w` did — is worse than one that admits it was
                        // cut.
                        text: elide(&d.message, 34),
                        trailing: Some(Trailing::Text(location)),
                        selected: false,
                        indent: 12.0,
                        leading: Leading::None,
                    },
                    _chrome,
                    colors,
                    move || {
                        studio.open_named(&d.file);
                        studio.jump_to(d.line, d.column);
                        studio.panel_open.set(true);
                    },
                )
            })
            .collect::<Vec<_>>();
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// The activity-bar entry for the inspector, which now lives in the right
    /// pane beside the thing it describes.
    ///
    /// This view used to *be* the inspector: five strings about the buffer,
    /// under a heading that said Inspector, because nothing in the framework
    /// would answer a question about the render tree. It does now
    /// (`RenderObject::describe`, `RenderTree::describe_subtree`,
    /// `FrameDriver::renders`), and a tree read in a 248-point sidebar while
    /// the screen it describes is on the other side of the window is the
    /// arrangement that made the stub easy to live with.
    fn inspector(&self, _chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let studio = self.studio.clone();
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![
                note(
                    colors,
                    "The widget inspector reads the render tree of the previewed screen, and \
                     shows it beside the screen itself.",
                ),
                Container::new()
                    .padding(EdgeInsets::only(12.0, 0.0, 12.0, 8.0))
                    .child(clickable(
                        move || {
                            Container::new()
                                .height(30.0)
                                .radius(6.0)
                                .color(colors.primary)
                                .alignment(Alignment::CENTER)
                                .child(label("Open the Inspector", 12.0, colors.on_primary))
                                .into()
                        },
                        move || studio.run(crate::command::Command::ShowInspector),
                    )),
            ])
            .into()
    }

    /// The reference: the list of pages, and the one that is open under it.
    ///
    /// # Why the page is rendered here and not opened as a tab
    ///
    /// A doc opened as a buffer is a file you can edit, save and break, and one
    /// more tab between you and your own code. This is a glance: the list stays
    /// visible, the page scrolls with the rest of the view, and the editor is
    /// untouched. Drag the divider if a page wants more width — which is the
    /// same answer the Explorer gives to a long file name.
    fn docs(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let open = self.studio.docs_page.get();
        let mut rows: Vec<WidgetNode> = vec![note(
            colors,
            "Five pages, for the questions of the first hour.",
        )];

        for (index, page) in crate::docs::PAGES.iter().enumerate() {
            let signal = self.studio.docs_page.clone();
            let selected = index == open;
            rows.push(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        row(
                            RowSpec {
                                text: page.title.to_owned(),
                                trailing: None,
                                selected,
                                indent: 12.0,
                                leading: Leading::None,
                            },
                            chrome,
                            colors,
                            move || signal.set(index)
                        ),
                        // The summary under the title, so the list can be
                        // scanned without opening five pages to find one.
                        Container::new()
                            .padding(EdgeInsets::only(12.0, 0.0, 12.0, 6.0))
                            .child(label(page.summary, 10.5, colors.outline)),
                    ])
                    .into(),
            );
        }

        rows.push(hairline(chrome.line));
        rows.push(
            Container::new()
                .padding(EdgeInsets::symmetric(12.0, 10.0))
                // `Markdown` is a widget in the framework this studio is built
                // from. Rendering the docs with it means the docs view costs a
                // dozen lines *and* that the widget gets used by its own tool,
                // which is the cheapest way to find out it is wrong.
                .child(vieww_widget::Inherited::new(
                    // A shade smaller than the studio's own text. `Markdown`
                    // sets its headings from the theme's typography, which is
                    // sized for a document rather than for a 260-point column —
                    // and `Accessibility::text_scale` is the one lever that
                    // shrinks a whole subtree proportionally, headings and code
                    // blocks together, rather than restyling six elements.
                    // It multiplies whatever the user has already chosen, so a
                    // studio set to large text still gets large docs.
                    vieww_foundation::Accessibility {
                        text_scale: self.studio.ui_scale() * 0.82,
                        high_contrast: self.studio.high_contrast.get(),
                        ..vieww_foundation::Accessibility::default()
                    },
                    vieww_widget::Markdown::new(crate::docs::page(open).body),
                ))
                .into(),
        );

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// The Learn view: editable lessons, one per feature.
    ///
    /// # What this is, and what it is not
    ///
    /// The repo ships ~50 `examples/features/*` programs, each a complete
    /// `fn main()`. They prove the framework does what it claims; they do not,
    /// on their own, teach somebody how to write a screen, because what the
    /// editor needs is `pub fn screen() -> impl Widget` — a different shape.
    /// This view is the bridge: each lesson is a complete `pub fn screen()`
    /// source that compiles and renders in the studio's preview. A click puts
    /// its source in the active buffer; Render shows what the example
    /// demonstrated, in the device frame the studio already has.
    ///
    /// See [`crate::lessons`] for why this is a curated subset rather than a
    /// literal port of all fifty.
    fn learn(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let mut rows: Vec<WidgetNode> = vec![note(
            colors,
            "Click a lesson to put its source in the buffer. Render shows it.",
        )];

        // One row per lesson, mirroring the Docs view's shape: a title with
        // the summary under it, so the list can be scanned without opening
        // twelve files to find one.
        //
        // `.iter()` rather than `for lesson in LESSONS` because the latter
        // (edition 2021) consumes the array by value, and we want a borrow —
        // `&Lesson` is `Copy` and `lesson.body`/`lesson.name` are `&'static str`
        // either way, but the borrow makes the intent clearer: this view is
        // reading the lessons, not moving them.
        for lesson in crate::lessons::LESSONS.iter() {
            let studio = self.studio.clone();
            // `*lesson` because the row's closure needs to own a value, and
            // `&'static Lesson` cannot be moved into an `Fn` — `*lesson` is a
            // copy of the struct, which is `Copy`, and the closure captures it
            // by value. The studio reads `lesson.body` and `lesson.name` out
            // of it.
            let lesson = *lesson;
            rows.push(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        row(
                            RowSpec {
                                text: lesson.name.to_owned(),
                                trailing: None,
                                selected: false,
                                indent: 12.0,
                                leading: Leading::None,
                            },
                            chrome,
                            colors,
                            move || studio.load_lesson(&lesson),
                        ),
                        Container::new()
                            .padding(EdgeInsets::only(12.0, 0.0, 12.0, 6.0))
                            .child(label(lesson.summary, 10.5, colors.outline)),
                    ])
                    .into(),
            );
            // A lesson with a Say body loaded Say; the Rust body stays one
            // row behind it, indented under the summary where it reads as a
            // footnote rather than a second lesson.
            if lesson.say_body.is_some() {
                let studio = self.studio.clone();
                let lesson_name = lesson.name.to_owned();
                rows.push(
                    Container::new()
                        .padding(EdgeInsets::only(24.0, 0.0, 12.0, 6.0))
                        .child(clickable(
                            move || {
                                label("the Rust version of this lesson", 10.5, colors.primary)
                                    .into()
                            },
                            move || {
                                studio.load_lesson_rust(&lesson);
                                let _ = &lesson_name;
                            },
                        ))
                        .into(),
                );
            }
        }

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// N4: what is installed, what is missing, and what installs it.
    ///
    /// Two toolchains, not one, and the distinction is the whole view:
    ///
    /// * The **preview** toolchain is `rustc` plus this workspace's own rlibs.
    ///   It is what Render uses, it is already discovered at startup, and its
    ///   failure mode is "Render is disabled".
    /// * The **build** toolchains are per target — cargo for desktop, an SDK,
    ///   an NDK, a JDK and `cargo-ndk` for Android, Xcode for iOS. Their
    ///   failure mode is "Build is disabled", and plan 2 §6.2 asks that each
    ///   missing piece name the command that installs it rather than leaving
    ///   the user to search.
    fn toolchain(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let preview = match self.studio.toolchain.as_ref() {
            // `rustc` resolves to a bare command name when it came off `PATH`,
            // so this line read "rustc rustc". What is worth showing is the
            // stamp, which is the compiler *and* the vieww build the preview is
            // pinned to — the thing that decides whether a dylib will load.
            Ok(toolchain) => toolchain.stamp(),
            // The wording is pinned by `tests/shell.rs`, and deliberately: the
            // promise is that a studio with no compiler *says so* rather than
            // showing an empty column, and a promise about what a person reads
            // is only kept if a test reads it too.
            Err(error) => format!("Toolchain unavailable — {error}"),
        };

        let mut rows: Vec<WidgetNode> = vec![
            section_title("Preview", colors),
            Container::new()
                .padding(EdgeInsets::symmetric(12.0, 4.0))
                .child(
                    Text::new(preview)
                        .style(TextStyle::new(11.0).family(FontFamily::Monospace))
                        .color(colors.on_surface_variant),
                )
                .into(),
        ];

        for report in self.studio.toolchains.get().iter() {
            let ready = report.ready();
            rows.push(section_title(report.target.title(), colors));

            // The reason a target is impossible goes above its requirements,
            // because no amount of installing will change it — see plan 2's
            // open question 3, answered in `toolchains::Report::impossible`.
            if let Some(reason) = report.impossible {
                rows.push(
                    Container::new()
                        .padding(EdgeInsets::symmetric(12.0, 4.0))
                        .child(
                            Text::new(reason)
                                .style(TextStyle::new(11.0))
                                .color(colors.on_surface_variant),
                        )
                        .into(),
                );
            } else {
                rows.push(
                    Container::new()
                        .padding(EdgeInsets::symmetric(12.0, 2.0))
                        .child(label(
                            if ready { "ready" } else { "not ready" },
                            11.0,
                            if ready {
                                colors.primary
                            } else {
                                colors.outline
                            },
                        ))
                        .into(),
                );
            }

            for requirement in &report.requirements {
                rows.push(requirement_row(requirement, colors));
                // **The command, as a button rather than as a sentence.** The
                // checklist has always shown what to run; what it did not do
                // was run it. Only for what is missing: a row that is already
                // satisfied does not need an Install button, and one drawn
                // anyway is an invitation to reinstall something that works.
                if !requirement.satisfied() {
                    // Only where there is a command. Two of the requirements are
                    // instructions rather than commands — "Xcode > Settings >
                    // Accounts > your Apple ID > Manage Certificates > +" is a
                    // path through a menu, and a button offering to *run* it
                    // would be a button that cannot do what it says.
                    if let Some(command) = crate::state::runnable(requirement.install).as_deref() {
                        rows.push(install_row(
                            &self.studio,
                            requirement,
                            command,
                            chrome,
                            colors,
                        ));
                    }
                }
            }
        }

        // **Where things are, when the environment cannot say.**
        //
        // A studio launched from the Dock, a `.desktop` entry or Explorer never
        // saw the shell that exported `ANDROID_HOME`, so a perfectly installed
        // SDK read as missing and there was nothing to be done about it from
        // inside the window. Each of these is laid over the real environment by
        // `Studio::configured_env`, and blank means "ask the environment" — so
        // a studio started from a terminal behaves exactly as it did.
        rows.push(section_title("Paths", colors));
        rows.push(path_row(
            "Android SDK",
            "ANDROID_HOME",
            "The folder holding platform-tools.",
            &self.studio.android_home,
            &self.studio,
            chrome,
            colors,
        ));
        rows.push(path_row(
            "Android NDK",
            "ANDROID_NDK_HOME",
            "A versioned folder inside the SDK.",
            &self.studio.android_ndk,
            &self.studio,
            chrome,
            colors,
        ));
        rows.push(path_row(
            "JDK",
            "JAVA_HOME",
            "17 or newer, for Gradle.",
            &self.studio.java_home,
            &self.studio,
            chrome,
            colors,
        ));

        rows.push(section_title("Signing", colors));
        rows.push(path_row(
            "Android keystore",
            "path to .jks or .keystore",
            "The keystore a release is signed with.",
            &self.studio.keystore,
            &self.studio,
            chrome,
            colors,
        ));
        rows.push(path_row(
            "Key alias",
            "alias",
            "The name of the key inside that keystore.",
            &self.studio.keystore_alias,
            &self.studio,
            chrome,
            colors,
        ));
        rows.push(path_row(
            "Apple team ID",
            "ABCDE12345",
            "Ten characters, from developer.apple.com.",
            &self.studio.apple_team,
            &self.studio,
            chrome,
            colors,
        ));
        rows.push(
            Container::new()
                .padding(EdgeInsets::only(12.0, 2.0, 12.0, 8.0))
                .child(label(
                    "The keystore password is never written here. Set \
                     VIEWW_KEYSTORE_PASSWORD in the environment the studio is \
                     started from, and a release build reads it from there.",
                    10.5,
                    colors.outline,
                ))
                .into(),
        );

        // A button, not `action_row` — that draws an ON/OFF state, and this has
        // none. It read "Re-scan  OFF", which invites the reader to wonder what
        // turning re-scanning on would do.
        rows.push(button_row("Re-scan", colors, {
            let studio = self.studio.clone();
            move || studio.rescan_toolchains()
        }));

        // `Sidebar::build` now wraps every view's content in one `Scrollable`
        // keyed to that view's own `ScrollController` — see its comment. A
        // second one here would nest scrollables on the same axis, which is
        // two competing owners of one drag.
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// Source control: the branch, what changed, and a box to commit with.
    ///
    /// # Staged and unstaged are two lists, not one with a checkbox
    ///
    /// The distinction decides what a commit will contain, which is the one
    /// question this view exists to answer. A single list with a mark beside
    /// some rows makes that answer something you count rather than something
    /// you read.
    fn source(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let root = self.studio.root.get();
        let Some(_root) = root.as_ref() else {
            return note(colors, "No folder is open.");
        };
        if !self.studio.is_repository() {
            // Not an error and not a button that does nothing: it is a folder
            // that is not a repository, which is a normal thing to have open.
            return note(
                colors,
                "This folder is not a git repository. `git init` in it and re-scan.",
            );
        }

        let status = self.studio.git.get();
        let mut rows: Vec<WidgetNode> = Vec::new();

        rows.push(section_title(&status.describe(), colors));

        // The commit box. A real `TextField`, so it has a caret and a
        // pasteboard like every other field in the window.
        let message = self.studio.commit_message.get();
        let signal = self.studio.commit_message.clone();
        rows.push(
            Container::new()
                .padding(EdgeInsets::symmetric(10.0, 4.0))
                .child(
                    Container::new()
                        .color(chrome.chrome_1)
                        .radius(6.0)
                        .padding(EdgeInsets::symmetric(8.0, 7.0))
                        .child(
                            TextField::text(message.as_str())
                                .style(TextStyle::new(12.0).color(colors.on_surface))
                                .placeholder("Message")
                                .cursor(colors.primary, 2.0)
                                .show_cursor(false)
                                .on_changed(std::rc::Rc::new(
                                    move |value: vieww_foundation::TextEditingValue| {
                                        signal.set(value.text);
                                    },
                                )),
                        ),
                )
                .into(),
        );
        rows.push(button_row(
            &format!("Commit {} staged", status.staged()),
            colors,
            {
                let studio = self.studio.clone();
                move || studio.commit()
            },
        ));

        // **The other half of source control.** Commit was the end of the road
        // here: a developer could see, stage and commit, and then had to leave
        // for a terminal to make any of it visible to a colleague. These three
        // are the remote.
        //
        // Each is greyed by its own command's `enabled` rule rather than always
        // drawn live, so the row says what the repository is — "nothing to
        // push" is information, and a button that runs git to print
        // "Everything up-to-date" is not.
        rows.push(
            Container::new()
                .padding(EdgeInsets::symmetric(4.0, 0.0))
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(
                            [
                                (
                                    crate::command::Command::Push,
                                    format!("Push {}", status.ahead),
                                ),
                                (
                                    crate::command::Command::Pull,
                                    format!("Pull {}", status.behind),
                                ),
                                (crate::command::Command::Fetch, "Fetch".to_owned()),
                            ]
                            .into_iter()
                            .map(|(command, title)| {
                                let enabled = self.studio.can_run(command);
                                let studio = self.studio.clone();
                                let tint = if enabled {
                                    colors.primary
                                } else {
                                    colors.outline
                                };
                                let title = title.clone();
                                clickable(
                                    move || {
                                        Container::new()
                                            .height(30.0)
                                            .padding(EdgeInsets::symmetric(8.0, 0.0))
                                            .alignment(Alignment::CENTER_LEFT)
                                            .child(label(&title, 12.0, tint))
                                            .into()
                                    },
                                    move || {
                                        if enabled {
                                            studio.run(command);
                                        }
                                    },
                                )
                                .into()
                            })
                            .collect::<Vec<WidgetNode>>(),
                        ),
                )
                .into(),
        );

        let (staged, unstaged): (Vec<_>, Vec<_>) =
            status.entries.iter().partition(|entry| entry.staged);

        for (title, entries, is_staged) in [
            ("Staged changes", &staged, true),
            ("Changes", &unstaged, false),
        ] {
            if entries.is_empty() {
                continue;
            }
            rows.push(section_title(title, colors));
            for entry in entries {
                let path = entry.path.clone();
                let studio = self.studio.clone();
                let name = path.to_string_lossy().to_string();
                let change = entry.change;
                let tint = match change {
                    crate::git::Change::Conflicted => colors.error,
                    crate::git::Change::Untracked | crate::git::Change::Added => colors.success,
                    _ => colors.on_surface_variant,
                };
                rows.push(
                    clickable(
                        move || {
                            Container::new()
                                .padding(EdgeInsets::symmetric(12.0, 5.0))
                                .child(
                                    Flex::row()
                                        .cross_axis_alignment(CrossAxisAlignment::Center)
                                        .children(children![
                                            label_bold(change.letter(), 10.5, tint),
                                            space(8.0),
                                            Flexible::expanded(1).child(label(
                                                &name,
                                                12.0,
                                                colors.on_surface
                                            )),
                                            // The verb, not an icon: "stage"
                                            // and "unstage" are the two things
                                            // a click here can mean and there
                                            // is no glyph people read as
                                            // either.
                                            label(
                                                if is_staged { "unstage" } else { "stage" },
                                                10.5,
                                                colors.primary
                                            ),
                                        ]),
                                )
                                .into()
                        },
                        move || studio.stage(&path, is_staged),
                    )
                    .into(),
                );
            }
        }

        if status.entries.is_empty() {
            rows.push(note(
                colors,
                if self.studio.git_scanned.get() {
                    "Nothing has changed since the last commit."
                } else {
                    "Not scanned yet."
                },
            ));
        }

        rows.push(button_row("Refresh", colors, {
            let studio = self.studio.clone();
            move || studio.refresh_git()
        }));

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// N5: what this project can be packaged as, and what is plugged in.
    ///
    /// # Every format is listed, including the ones that cannot run
    ///
    /// Plan 2's open question 3, answered the same way `toolchains` answers
    /// it: a missing row is a bug report. An iOS `.ipa` on a Linux machine
    /// stays on screen, greyed, with Apple's rule as its reason — because the
    /// question a person actually has is "can I ship this from here", and a
    /// list that silently omits the answer does not answer it.
    fn export(&self, _chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        use crate::export::{self, Format};

        let mut rows: Vec<WidgetNode> = Vec::new();
        let root = self.studio.root.get();
        let Some(root) = root.as_ref() else {
            return note(
                colors,
                "No folder is open. Export packages a project on disk, so there is nothing to \
                 package yet.",
            );
        };
        let name = self.studio.package_name();
        let running = self.studio.exporting();

        rows.push(section_title("Package", colors));
        rows.push(
            Container::new()
                .padding(EdgeInsets::symmetric(12.0, 2.0))
                .child(label(
                    &name.clone().map_or_else(
                        || "no [package] in Cargo.toml".to_string(),
                        |name| format!("{name} — into target/export"),
                    ),
                    11.0,
                    colors.on_surface_variant,
                ))
                .into(),
        );

        for format in Format::ALL {
            let refusal = export::plan(
                &self.studio.build_env,
                root,
                name.as_deref().unwrap_or_default(),
                format,
            )
            .err();
            let available = refusal.is_none() && !running;

            rows.push(
                clickable(
                    {
                        let title = format.title().to_string();
                        // The refusal *is* the detail line when there is one:
                        // a row that says "Android APK" and nothing else,
                        // which does nothing when clicked, teaches people the
                        // studio is broken.
                        let detail = refusal.as_ref().map_or_else(
                            || format.detail().to_string(),
                            std::string::ToString::to_string,
                        );
                        let tint = if available {
                            colors.on_surface
                        } else {
                            colors.outline
                        };
                        move || {
                            Container::new()
                                .padding(EdgeInsets::symmetric(12.0, 7.0))
                                .child(
                                    Flex::column()
                                        .cross_axis_alignment(CrossAxisAlignment::Start)
                                        .children(children![
                                            label(&title, 12.0, tint),
                                            space(2.0),
                                            label(&detail, 10.5, colors.on_surface_variant),
                                        ]),
                                )
                                .into()
                        }
                    },
                    {
                        let studio = self.studio.clone();
                        move || {
                            if available {
                                studio.export_format.set(format);
                                studio.start_export(format);
                            }
                        }
                    },
                )
                .into(),
            );
        }

        if running {
            rows.push(button_row("Cancel export", colors, {
                let studio = self.studio.clone();
                move || studio.cancel_export()
            }));
        }

        // ---- devices ------------------------------------------------------
        rows.push(section_title("Devices", colors));
        let devices = self.studio.devices.get();
        if devices.is_empty() {
            rows.push(note(
                colors,
                if self.studio.devices_scanned.get() {
                    "Nothing connected. `adb` reported no devices."
                } else {
                    // Not scanned automatically on every open of this view: it
                    // is a process launch, and a sidebar that starts one every
                    // time it is looked at is a sidebar that costs something
                    // to look at.
                    "Not scanned yet. Android devices are listed by `adb`."
                },
            ));
        }
        for device in devices.iter() {
            let ready = device.state.is_ready();
            rows.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(12.0, 6.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .children(children![
                                label(
                                    &device.name,
                                    12.0,
                                    if ready {
                                        colors.on_surface
                                    } else {
                                        colors.outline
                                    }
                                ),
                                space(2.0),
                                label(device.state.label(), 10.5, colors.on_surface_variant),
                            ]),
                    )
                    .into(),
            );
            // **The button §6 said was missing.** `Studio::install_on` has
            // existed since N5 with nothing calling it: the wire was there and
            // the artefact was printed to the Output panel and dropped, so
            // there was nothing to install. `Studio::artefact` keeps it, and
            // this is the row that uses it.
            //
            // Only on a device that is ready, and only when something has been
            // built — a button that is always there and usually refuses is a
            // button people stop reading.
            if ready {
                if let Some(artefact) = self.studio.artefact.get() {
                    let studio = self.studio.clone();
                    let device = device.clone();
                    let name = artefact
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    rows.push(
                        Container::new()
                            .padding(EdgeInsets::only(12.0, 0.0, 12.0, 6.0))
                            .child(clickable(
                                move || {
                                    Container::new()
                                        .height(26.0)
                                        .radius(5.0)
                                        .color(colors.primary)
                                        .alignment(Alignment::CENTER)
                                        .child(label(
                                            &format!("Install {name}"),
                                            11.0,
                                            colors.on_primary,
                                        ))
                                        .into()
                                },
                                move || studio.install_on(&device, &artefact),
                            ))
                            .into(),
                    );
                }
            }
        }
        rows.push(button_row("Scan for devices", colors, {
            let studio = self.studio.clone();
            move || studio.scan_devices()
        }));

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    /// N8: the theme's tokens.
    ///
    /// Every value with a swatch, its hex, and the contrast it makes against
    /// the surface it will be drawn on — which is the question this view exists
    /// to answer. Two nudges per row, because *lighter* and *darker* is the
    /// edit somebody makes here and a colour picker is four controls the studio
    /// does not have. See `crate::tokens`.
    fn tokens(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        use crate::tokens::{contrast, contrast_verdict, hex, Group};

        let group = self.studio.token_group.get();
        let all = self.studio.tokens(*chrome);
        let edited = self.studio.token_edits.get();

        let mut rows: Vec<WidgetNode> = vec![note(
            colors,
            "Read from the live theme. Edits apply to the studio immediately and              can be exported as a theme.rs.",
        )];

        // The group picker.
        let mut picker: Vec<WidgetNode> = Vec::new();
        for candidate in Group::ALL {
            let studio = self.studio.clone();
            let selected = candidate == group;
            picker.push(
                clickable(
                    move || {
                        Container::new()
                            .radius(5.0)
                            .color(if selected {
                                colors.primary
                            } else {
                                colors.surface_variant
                            })
                            .padding(EdgeInsets::symmetric(9.0, 4.0))
                            .child(label(
                                candidate.title(),
                                11.0,
                                if selected {
                                    colors.on_primary
                                } else {
                                    colors.on_surface_variant
                                },
                            ))
                            .into()
                    },
                    move || studio.token_group.set(candidate),
                )
                .into(),
            );
            picker.push(space(5.0));
        }
        rows.push(
            Container::new()
                .padding(EdgeInsets::only(12.0, 0.0, 12.0, 8.0))
                .child(Flex::row().children(picker))
                .into(),
        );

        // **What each row's contrast is measured against.**
        //
        // Not one ground for the whole group. `ColorScheme` is built in pairs —
        // `on_primary` is the colour drawn *on `primary`*, and measuring it
        // against the surface answers a question nobody asked and reports
        // "fails" for a pair that is fine. So an `on_x` is measured against
        // `x`, and everything else against the surface it lands on.
        //
        // A token has no contrast with itself, so `surface` and `chrome_2` get
        // no readout at all rather than a meaningless 1.0:1.
        let rgb_of = |name: &str| all.iter().find(|t| t.name == name).map(|t| t.rgb);
        let default_ground = match group {
            Group::Colors => "surface",
            Group::Chrome | Group::Syntax => "chrome_2",
        };

        for token in all.iter().filter(|token| token.group == group) {
            let ground_name = token.name.strip_prefix("on_").unwrap_or(default_ground);
            let ratio = (ground_name != token.name)
                .then(|| rgb_of(ground_name).map(|ground| contrast(token.rgb, ground)))
                .flatten();
            let changed = edited.contains_key(token.name);
            let name = token.name;
            let rgb = token.rgb;

            let nudge = |factor: f32, glyph: &'static str| -> WidgetNode {
                let studio = self.studio.clone();
                clickable(
                    move || {
                        Container::new()
                            .size(20.0, 20.0)
                            .radius(4.0)
                            .color(colors.surface_variant)
                            .alignment(Alignment::CENTER)
                            .child(label(glyph, 11.0, colors.on_surface_variant))
                            .into()
                    },
                    move || studio.nudge_token(name, rgb, factor),
                )
                .into()
            };

            rows.push(
                Container::new()
                    .padding(EdgeInsets::symmetric(12.0, 4.0))
                    .child(
                        Flex::row()
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                Container::new()
                                    .size(18.0, 18.0)
                                    .radius(4.0)
                                    .color(vieww_foundation::Color::hex(token.rgb))
                                    .border(vieww_foundation::Border {
                                        color: colors.outline,
                                        width: 1.0,
                                    }),
                                space(8.0),
                                Flexible::expanded(1).child(
                                    Flex::column()
                                        .cross_axis_alignment(CrossAxisAlignment::Start)
                                        .children(children![
                                            label(
                                                token.name,
                                                11.5,
                                                if changed {
                                                    colors.primary
                                                } else {
                                                    colors.on_surface
                                                }
                                            ),
                                            // Hex and contrast on one line: the
                                            // value and what it is worth.
                                            mono(
                                                &match ratio {
                                                    Some(ratio) => format!(
                                                        "{}  ·  {ratio:.1}:1 on {ground_name} {}",
                                                        hex(token.rgb),
                                                        contrast_verdict(ratio, token.role)
                                                    )
                                                    .trim_end()
                                                    .to_owned(),
                                                    None => hex(token.rgb),
                                                },
                                                9.5,
                                                // Red only where the token
                                                // *has* a bar and misses it.
                                                // A background reported at
                                                // 1.2:1 is a depth step, not a
                                                // defect, and colouring it as
                                                // one is how a checker gets
                                                // scrolled past.
                                                if ratio
                                                    .zip(token.role.bar())
                                                    .is_some_and(|(ratio, bar)| ratio < bar)
                                                {
                                                    colors.error
                                                } else {
                                                    colors.outline
                                                }
                                            ),
                                        ])
                                ),
                                nudge(0.85, "\u{2212}"),
                                space(4.0),
                                nudge(1.18, "+"),
                            ]),
                    )
                    .into(),
            );
        }

        if !edited.is_empty() {
            rows.push(button_row("Reset to the theme", colors, {
                let studio = self.studio.clone();
                move || studio.reset_tokens()
            }));
        }
        rows.push(button_row("Export as theme.rs", colors, {
            let studio = self.studio.clone();
            let chrome = *chrome;
            move || studio.export_tokens(chrome)
        }));

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    fn settings(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        let dark = self.studio.dark.get();
        let insets = self.studio.show_insets.get();
        let comforts = self.studio.comforts.get();
        let release = self.studio.profile.get() == crate::builds::Profile::Release;
        let dark_signal = self.studio.dark.clone();
        let inset_signal = self.studio.show_insets.clone();
        let comfort_signal = self.studio.comforts.clone();
        let profile_signal = self.studio.profile.clone();
        let guides = self.studio.indent_guides.get();
        let guide_signal = self.studio.indent_guides.clone();
        let inline = self.studio.inline_diagnostics.get();
        let inline_signal = self.studio.inline_diagnostics.clone();
        let fmt = self.studio.format_on_save.get();
        let fmt_signal = self.studio.format_on_save.clone();
        let auto_render = self.studio.auto_render.get();
        let auto_signal = self.studio.auto_render.clone();
        let minimap = self.studio.minimap.get();
        let minimap_signal = self.studio.minimap.clone();
        let blink = self.studio.blink_enabled.get();
        let highlight = self.studio.highlight_enabled.get();
        let highlight_signal = self.studio.highlight_enabled.clone();
        let reduce_motion = self.studio.preview_reduce_motion.get();
        let reduce_signal = self.studio.preview_reduce_motion.clone();
        let large_ui = self.studio.large_ui.get();
        let high_contrast = self.studio.high_contrast.get();
        let lint = self.studio.lint_enabled.get();
        let font_size = self.studio.font_size.get();
        let tab_width = self.studio.tab_width.get();
        let text_scale = self.studio.preview_text_scale.get();
        let sizes = [11.0_f32, 12.0, 13.0, 14.0, 16.0];
        let tabs = [2.0_f32, 4.0, 8.0];
        let scales = [1.0_f32, 1.3, 1.6, 2.0];
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children![
                action_row("Dark theme".to_string(), dark, colors, move || dark_signal
                    .set(!dark)),
                action_row(
                    "Show preview safe area".to_string(),
                    insets,
                    colors,
                    move || inset_signal.set(!insets)
                ),
                // N6. Off is a real choice, not a debug switch: auto-indent and
                // bracket pairing are the features people most reliably disagree
                // about, and an editor that rewrites what you typed and cannot be
                // told to stop is worse than one that never did.
                action_row(
                    "Auto-indent and bracket pairing".to_string(),
                    comforts,
                    colors,
                    move || comfort_signal.set(!comforts)
                ),
                action_row("Indent guides".to_string(), guides, colors, move || {
                    guide_signal.set(!guides)
                }),
                // `rustfmt` is a child process, so the file is written first and
                // the formatted text arrives a moment later — see `Studio::run`'s
                // Save arm. Off for anyone who would rather that not happen.
                action_row(
                    "Format on save (rustfmt)".to_string(),
                    fmt,
                    colors,
                    move || fmt_signal.set(!fmt)
                ),
                // Off is a real choice here too: a hint after every flagged line is
                // the fastest way to see what is wrong and the fastest way to lose
                // the right-hand third of a narrow pane.
                action_row(
                    "Inline diagnostic messages".to_string(),
                    inline,
                    colors,
                    move || inline_signal.set(!inline)
                ),
                action_row("Build in release".to_string(), release, colors, move || {
                    profile_signal.set(if release {
                        crate::builds::Profile::Debug
                    } else {
                        crate::builds::Profile::Release
                    });
                }),
                action_row(
                    "Auto render on change".to_string(),
                    auto_render,
                    colors,
                    move || {
                        auto_signal.set(!auto_render);
                    }
                ),
                action_row("Minimap".to_string(), minimap, colors, move || {
                    minimap_signal.set(!minimap)
                }),
                // **Caret blink is not listed here.** It used to be, *and* under
                // Accessibility below — two rows with the same name and the same
                // state, one scroll apart, in a pane that is one column. Toggling
                // either changed both, which reads as a bug rather than as a
                // deliberate cross-reference: there is nothing on either row to say
                // they are one control.
                //
                // Kept under Accessibility rather than here, because that is where
                // the reason it can be turned off lives, and a person looking for
                // it passes that heading either way.
                action_row(
                    "Syntax highlighting".to_string(),
                    highlight,
                    colors,
                    move || highlight_signal.set(!highlight)
                ),
                section_title("Editor", colors),
                // Two numbers rather than two toggles, because neither has a
                // sensible on/off. The steps are the values people actually use;
                // a free-entry field for a font size is a field somebody types 400
                // into once.
                choice_row("Font size", &sizes, font_size, colors, {
                    let signal = self.studio.font_size.clone();
                    move |value| signal.set(value)
                }),
                choice_row("Tab width", &tabs, tab_width as f32, colors, {
                    let signal = self.studio.tab_width.clone();
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the choices are 2, 4 and 8"
                    )]
                    move |value| signal.set(value as usize)
                }),
                section_title("Previewed screen", colors),
                // **The accessibility settings apply to the preview, not to the
                // studio.** `text_scale` is a property of the device the previewed
                // app is pretending to run on, and `vieww-foundation`'s own
                // accessibility note says the design has to survive 2× — *"or the
                // design breaks at 2× and nobody notices until it ships"*. Until
                // now nothing in the studio could ask for 2×, so nobody could
                // notice anything.
                choice_row("Text scale", &scales, text_scale, colors, {
                    let signal = self.studio.preview_text_scale.clone();
                    move |value| signal.set(value)
                }),
                action_row(
                    "Reduce motion".to_string(),
                    reduce_motion,
                    colors,
                    move || reduce_signal.set(!reduce_motion)
                ),
                // **The studio's own accessibility, which had no group at all.**
                // The prototype listed one; the studio had genuine `Semantics`
                // plumbing — button, region and named-icon-button helpers across
                // the whole chrome, and an overlay for inspecting it — and nothing
                // a user could set. Everything under "Previewed screen" above is
                // about the *simulated device*; these three are about the window
                // the user is actually looking at.
                section_title("Accent", colors),
                WidgetNode::from(
                    Container::new()
                        .padding(EdgeInsets::only(12.0, 0.0, 12.0, 2.0))
                        .child(label(
                            "The studio's own colour. Applies to the Render button, \
                         the tab and activity indicators, selection and focus.",
                            11.0,
                            colors.on_surface_variant,
                        ))
                ),
                accent_swatches(&self.studio, chrome, colors),
                section_title("Accessibility", colors),
                // The caret blink is the one thing in this window that asks for a
                // frame twice a second forever, and a blinking caret is a real
                // problem for some people. It is repeated here rather than moved,
                // because somebody looking for it will look under this heading.
                action_row("Caret blink".to_string(), blink, colors, {
                    let signal = self.studio.blink_enabled.clone();
                    move || signal.set(!blink)
                }),
                action_row("Larger interface text".to_string(), large_ui, colors, {
                    let signal = self.studio.large_ui.clone();
                    move || signal.set(!large_ui)
                }),
                action_row("High contrast chrome".to_string(), high_contrast, colors, {
                    let signal = self.studio.high_contrast.clone();
                    move || signal.set(!high_contrast)
                }),
                section_title("Diagnostics", colors),
                // The third producer, and off is a real choice: a lint layer
                // nobody trusts is worse than none. See `crate::lint`.
                action_row(
                    "vieww-lint findings in Problems".to_string(),
                    lint,
                    colors,
                    {
                        let signal = self.studio.lint_enabled.clone();
                        move || signal.set(!lint)
                    }
                ),
            ])
            .into()
    }

    /// Open buffers, the session's scratch renders, and the widget catalogue.
    fn explorer(&self, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
        // `tabs`, not `buffers`. The open-buffer list is names and dots; the
        // *statistics* below it are the only thing in this view that needs the
        // text, and they read it from their own element — see `FileStats`.
        let buffers = self.studio.tabs.get();
        let active = self.studio.active_buffer.get();

        // N1: the recursive directory tree. A scratch workspace has none, and
        // the section says so rather than vanishing — an explorer with no
        // "Project files" heading is one the user cannot tell is loading from
        // one that is empty. A recognised vieww project gets a small badge.
        let mut tree_rows = Vec::new();
        let is_vieww = self.studio.is_vieww.get();
        if self.studio.tree.get().root().is_some() {
            if is_vieww {
                tree_rows.push(note(colors, "vieww project — Build is available."));
            }
            // The watcher has run since N1 and has never been visible. That is
            // worth fixing for one reason: it is the thing standing between a
            // `git checkout` and a stale buffer silently overwriting the
            // branch someone just switched to, and a guarantee nobody knows
            // they have is a guarantee they work around.
            tree_rows.push(note(
                colors,
                if self.studio.watcher.is_some() {
                    "Watching for changes on disk — polling mtimes."
                } else {
                    "Not watching this folder. A change made outside the window will not be noticed."
                },
            ));
            let tree = self.studio.tree.get();
            if let Some(root) = tree.root() {
                self.tree_node(root, 12.0, chrome, colors, &mut tree_rows);
            }
        } else {
            // The empty state used to be this sentence and nothing else — a
            // true statement with no way to act on it, in the one view whose
            // whole job is a folder. Everything that needs a root (the tree,
            // workspace search, git, cargo, build, export) is unreachable from
            // here until somebody opens one, so the button belongs here more
            // than anywhere else in the window.
            tree_rows.push(note(
                colors,
                "No folder open. The editor holds a scratch buffer, which renders but cannot be built.",
            ));
            let studio = self.studio.clone();
            tree_rows.push(
                Container::new()
                    .padding(EdgeInsets::only(12.0, 0.0, 12.0, 8.0))
                    .child(clickable(
                        move || {
                            Container::new()
                                .height(30.0)
                                .radius(6.0)
                                .color(colors.primary)
                                .alignment(Alignment::CENTER)
                                .child(label("Open Folder…", 12.0, colors.on_primary))
                                .into()
                        },
                        move || studio.open_picker(),
                    ))
                    .into(),
            );
        }

        let buffer_rows = buffers
            .iter()
            .enumerate()
            .map(|(index, buffer)| {
                let signal = self.studio.active_buffer.clone();
                row(
                    RowSpec {
                        text: buffer.name.clone(),
                        trailing: buffer.dirty.then_some(Trailing::Dot(colors.primary)),
                        selected: index == active,
                        indent: 26.0,
                        leading: Leading::File,
                    },
                    chrome,
                    colors,
                    move || signal.set(index),
                )
            })
            .collect::<Vec<_>>();

        // **The session's real renders**, read from the temp directory the
        // compile pipeline writes into. This list used to be three invented
        // filenames and three invented clock times, hard-coded — a studio that
        // had never compiled anything still showed "preview-0007.rs · 14:22".
        // Fiction that renders identically to fact is the worst kind of
        // placeholder, because nothing about it looks unfinished.
        let artefacts = self
            .studio
            .session
            .as_ref()
            .as_ref()
            .map(crate::compile::Session::artefacts)
            .unwrap_or_default();
        let scratch_rows: Vec<WidgetNode> = if artefacts.is_empty() {
            vec![note(colors, "Nothing rendered yet this session.")]
        } else {
            artefacts
                .iter()
                .take(8)
                .map(|path| {
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into());
                    let size = std::fs::metadata(path)
                        .map_or_else(|_| String::new(), |meta| format!("{} B", meta.len()));
                    row(
                        RowSpec {
                            text: name,
                            trailing: Some(Trailing::Text(size)),
                            selected: false,
                            indent: 26.0,
                            leading: Leading::File,
                        },
                        chrome,
                        colors,
                        || {},
                    )
                })
                .collect()
        };

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(
                [section("Project files", colors)]
                    .into_iter()
                    .chain(tree_rows)
                    .chain([hairline(chrome.line), section("Open buffers", colors)])
                    .chain(buffer_rows)
                    .chain([hairline(chrome.line), section("Session scratch", colors)])
                    .chain(scratch_rows)
                    .chain([
                        hairline(chrome.line),
                        section("This file", colors),
                        // Its own element, so the three counts below can read
                        // the buffer's text without dragging the file tree, the
                        // open-buffer list and the scratch list along with them.
                        WidgetNode::from(FileStats {
                            studio: self.studio.clone(),
                        }),
                    ])
                    .collect::<Vec<_>>(),
            )
            .into()
    }

    /// Render one tree node and, for an expanded directory, its children.
    ///
    /// Directories get a `▾`/`▸` chevron prefix and toggle on tap; files get the
    /// file icon and open on tap. Indentation grows with depth so the structure
    /// reads at a glance even before a folder is expanded.
    fn tree_node(
        &self,
        node: &crate::file_tree::TreeNode,
        indent: f32,
        chrome: &StudioTheme,
        colors: ColorScheme,
        out: &mut Vec<WidgetNode>,
    ) {
        use crate::file_tree::NodeKind;
        let studio = self.studio.clone();
        let path = node.path.clone();
        match &node.kind {
            NodeKind::Dir { children } => {
                let studio_for_click = studio.clone();
                let path_for_click = path.clone();
                out.push(row(
                    RowSpec {
                        // The chevron used to be pasted onto the front of this
                        // string as `▾ ` or `▸ `, and the bundled font has
                        // neither character. See `Leading`.
                        text: node.name.clone(),
                        trailing: None,
                        selected: false,
                        indent,
                        leading: Leading::Directory {
                            expanded: node.expanded,
                        },
                    },
                    chrome,
                    colors,
                    move || studio_for_click.toggle_expand(path_for_click.clone()),
                ));
                // Right-click on a folder: the same menu a file gets, with
                // "New File" and "New Folder" meaning *inside* this one. See
                // `ui::context_menu` for why this was the highest-value
                // missing wire in the studio.
                let last = out.pop().expect("just pushed");
                out.push(with_context_menu(last, &studio, &path));
                if node.expanded {
                    for child in children {
                        self.tree_node(child, indent + 12.0, chrome, colors, out);
                    }
                }
            }
            NodeKind::File => {
                let studio_for_click = studio.clone();
                let path_for_click = path.clone();
                out.push(row(
                    RowSpec {
                        text: node.name.clone(),
                        // The git letter, where there is one. In the trailing
                        // slot rather than beside the name so a column of them
                        // lines up and can be read down.
                        trailing: self
                            .studio
                            .git_change(&path)
                            .map(|change| Trailing::Text(change.letter().to_string())),
                        selected: false,
                        indent: indent + 14.0,
                        leading: Leading::File,
                    },
                    chrome,
                    colors,
                    move || studio_for_click.open_path(path_for_click.clone()),
                ));
                let last = out.pop().expect("just pushed");
                out.push(with_context_menu(last, &studio, &path));
            }
        }
    }
}

/// Wrap a tree row so a secondary click opens the context menu on it.
///
/// The framework has had `on_secondary_tap` since before the studio existed,
/// and the studio had not one reference to it. This is the whole of the wire.
fn with_context_menu(
    child: WidgetNode,
    studio: &crate::state::Studio,
    path: &std::path::Path,
) -> WidgetNode {
    let studio = studio.clone();
    let path = path.to_path_buf();
    vieww_widget::GestureDetector::new()
        .on_secondary_tap(move |details| {
            // `position`, not `local`: the menu is positioned against the
            // window, because a menu placed in the row's own coordinates would
            // appear at the top-left of the sidebar however far down the tree
            // the click was.
            studio.open_context_menu(details.position, path.clone());
        })
        .child(child)
        .into()
}

/// A collapsible section header. M0 draws it; M1 makes the chevron work.
fn section(title: &str, colors: ColorScheme) -> WidgetNode {
    Container::new()
        .height(24.0)
        .padding(EdgeInsets::only(8.0, 0.0, 8.0, 0.0))
        .alignment(Alignment::CENTER_LEFT)
        .child(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .children(children![
                    crate::ui::chrome::glyph(
                        icons::chevron_down(),
                        11.0,
                        colors.on_surface_variant,
                    ),
                    space(4.0),
                    label_bold(title, 11.0, colors.on_surface),
                ]),
        )
        .into()
}

/// What sits at the end of a row: an unsaved dot, or a small right-aligned tag.
enum Trailing {
    Dot(Color),
    Text(String),
}

/// What sits in front of a row's name.
///
/// # Why a directory's chevron is an icon and not a character
///
/// It was `▾` and `▸` — U+25BE and U+25B8 — pasted into the row's text. The
/// studio ships its own text stack and its own font, and that font does not
/// carry the geometric-shapes block, so every expanded folder in the Explorer
/// drew a **tofu box** where its chevron should be. The first panel of the
/// first screen of the studio, with a missing glyph in it.
///
/// A `.notdef` is the one rendering failure that cannot be argued with, and it
/// is also the one that survives review: it is small, it is grey, and at a
/// glance it reads as *some* icon. It was found by looking at a rasterised
/// screenshot at 3× rather than by looking at the studio.
///
/// The icon set is vector data compiled into the binary, so it cannot go
/// missing for want of a glyph, and the section headers a few rows above were
/// already drawing their chevrons from it — the tree was the odd one out.
enum Leading {
    /// Nothing; the name starts at the indent.
    None,
    /// A file.
    File,
    /// A directory, drawn open or closed.
    Directory { expanded: bool },
}

struct RowSpec {
    text: String,
    trailing: Option<Trailing>,
    selected: bool,
    indent: f32,
    /// What is drawn in front of the name.
    leading: Leading,
}

fn row(
    spec: RowSpec,
    chrome: &StudioTheme,
    colors: ColorScheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    let RowSpec {
        text,
        trailing,
        selected,
        indent,
        leading,
    } = spec;
    let background = if selected {
        chrome.selection
    } else {
        chrome.chrome_1
    };
    let outline = colors.outline;
    let chrome = *chrome;

    crate::ui::chrome::sensed(
        move |sense| {
            let tail: WidgetNode = match &trailing {
                Some(Trailing::Dot(color)) => Container::new()
                    .color(*color)
                    .radius(3.0)
                    .size(6.0, 6.0)
                    .into(),
                Some(Trailing::Text(value)) => label(value, 10.0, outline).into(),
                None => SizedBox::shrink().into(),
            };

            // Same argument as the editor's tab strip: the fill has almost no
            // room between `chrome_1` and `chrome.selection`, and the text has
            // all of it.
            let foreground = if selected || sense.emphasis() > 0.0 {
                colors.on_surface
            } else {
                colors.on_surface_variant
            };

            let mut row: Vec<WidgetNode> = Vec::new();
            match leading {
                Leading::None => {}
                Leading::File => {
                    row.push(crate::ui::chrome::glyph(icons::file(), 12.0, outline).into());
                    row.push(space(6.0));
                }
                Leading::Directory { expanded } => {
                    row.push(
                        crate::ui::chrome::glyph(
                            if expanded {
                                icons::chevron_down()
                            } else {
                                icons::chevron_right()
                            },
                            14.0,
                            outline,
                        )
                        .into(),
                    );
                    row.push(space(4.0));
                }
            }
            // **The name gives way, and is cut where it runs out of room.**
            //
            // It was a fixed-size label beside a `gap()`, so a row whose name
            // and trailing value together exceeded the sidebar simply
            // overflowed — `preview-0002.rs` and its byte count at the sidebar's
            // 180-point minimum, which is the width somebody dragging the
            // divider in reaches immediately. `Flexible` lets the name shrink
            // and `EllipsisMiddle` cuts it *at the pixel*, keeping the head and
            // the extension, which for a file name is the readable half.
            row.push(
                Flexible::expanded(1)
                    .child(
                        label(&text, 12.5, foreground)
                            .max_lines(1)
                            .overflow(vieww_text::TextOverflow::EllipsisMiddle),
                    )
                    .into(),
            );
            row.push(space(8.0));
            row.push(tail);

            // Selection keeps its wash; everything else answers the pointer.
            // A file tree is the one list in the shell where rows are a pixel
            // apart and every one of them is a target, so "which row am I
            // about to open" is the question it most needs to answer before
            // the click rather than after.
            let mut container = Container::new()
                .color(if selected {
                    background
                } else {
                    crate::ui::chrome::hovered(background, &chrome, sense)
                })
                .height(ROW)
                .padding(EdgeInsets::only(indent, 0.0, 8.0, 0.0));
            // The selected row is a raised surface too, for the reason the
            // selected tab is: `chrome.selection` against `chrome_1` is a
            // small step, and in a list of twenty rows a small step is what
            // makes people re-read the list to find where they are.
            if selected {
                container = container.radius(4.0).shadow(vieww_foundation::Shadow::new(
                    Color::rgba(0, 0, 0, 70),
                    vieww_foundation::Offset::new(0.0, 1.0),
                    4.0,
                ));
            }
            container
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(row),
                )
                .into()
        },
        on_tap,
    )
    .into()
}

/// A row that does something when tapped, with no state to show.
fn button_row(title: &str, colors: ColorScheme, on_tap: impl Fn() + 'static) -> WidgetNode {
    let title = title.to_string();
    crate::ui::chrome::sensed(
        move |sense| {
            Container::new()
                .color(crate::ui::chrome::hover_overlay(colors, sense))
                .height(34.0)
                .padding(EdgeInsets::symmetric(12.0, 0.0))
                .alignment(Alignment::CENTER_LEFT)
                .child(label(&title, 12.0, colors.primary))
                .into()
        },
        on_tap,
    )
    .into()
}

/// A small heading inside a sidebar view.
/// The accent swatches: one row of every colour the studio ships.
///
/// # Why swatches and not a dropdown of names
///
/// The thing being chosen *is* a colour. A list reading "Purple, Blue, Teal"
/// asks the reader to imagine eight interfaces; eight filled circles show them.
/// Each swatch is painted in the ramp it selects, in the theme currently in use,
/// so what is on the button is what the studio will look like — including the
/// fact that the light theme's accents are deeper than the dark theme's.
///
/// The current one is ringed rather than ticked. A tick has to be drawn in a
/// colour, and every colour it could be drawn in is illegible on at least one
/// of these eight.
fn accent_swatches(studio: &Studio, chrome: &StudioTheme, colors: ColorScheme) -> WidgetNode {
    let dark = chrome.dark;
    let current = crate::theme::accent_named(&studio.accent.get());
    let swatches = crate::theme::ACCENTS
        .iter()
        .map(|(name, brand)| {
            let chosen = *brand == current;
            let brand = *brand;
            let signal = studio.accent.clone();
            let name = *name;
            Semantics::new()
                .label(format!(
                    "{name} accent{}",
                    if chosen { ", selected" } else { "" }
                ))
                .child(clickable(
                    move || {
                        Container::new()
                            .size(26.0, 26.0)
                            .radius(13.0)
                            .alignment(Alignment::CENTER)
                            // The ring is a border on an outer disc rather than
                            // a second widget: one box, and the swatch keeps
                            // its size whether it is selected or not, so the
                            // row does not shuffle when the choice changes.
                            .border(vieww_foundation::Border {
                                color: if chosen {
                                    colors.on_surface
                                } else {
                                    vieww_foundation::Color::TRANSPARENT
                                },
                                width: 2.0,
                            })
                            .child(
                                Container::new()
                                    .size(
                                        if chosen { 16.0 } else { 20.0 },
                                        if chosen { 16.0 } else { 20.0 },
                                    )
                                    .radius(10.0)
                                    .gradient(
                                        vieww_foundation::Gradient::linear(
                                            Offset::new(0.0, 0.0),
                                            Offset::new(1.0, 1.0),
                                        )
                                        .between(brand.far(dark), brand.near(dark)),
                                    ),
                            )
                            .into()
                    },
                    move || signal.set(name.to_owned()),
                ))
                .into()
        })
        .collect::<Vec<WidgetNode>>();

    // **Two rows of four, each cell flexible — not one row of eight.**
    //
    // One `Flex::row` of fixed-size swatches needed 232 points (eight 26-point
    // discs, 24 of padding) and asked the pane for all of it. The sidebar's
    // floor is 180, so below 232 the row ran past its own pane and the last
    // swatches ended up under the divider and behind the editor card — a
    // picker that eats two of its eight choices exactly when the user has made
    // the sidebar narrower, which is the one time they were short on space.
    //
    // Splitting 4 + 4 needs 128 points at the floor and stretches with the
    // pane: each cell takes a quarter of the row, the discs stay 26 points
    // and centred, and `SpaceBetween` within a fixed set was doing nothing a
    // flexible cell does not do more honestly. The cool-to-warm order the
    // doc comment on `ACCENTS` asks for reads the same way — across, then
    // down — and two rows of four is how a palette has been printed since
    // palettes were printed.
    let rows = swatches
        .chunks(4)
        .map(|chunk| {
            Flex::row()
                .children(
                    chunk
                        .iter()
                        .map(|swatch| Flexible::expanded(1).child(swatch.clone()).into())
                        .collect::<Vec<WidgetNode>>(),
                )
                .into()
        })
        .collect::<Vec<WidgetNode>>();

    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 4.0))
        .child(Flex::column().children(rows))
        .into()
}

fn section_title(title: &str, colors: ColorScheme) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::only(12.0, 12.0, 12.0, 4.0))
        .child(label_bold(title, 10.5, colors.on_surface_variant))
        .into()
}

/// One line of the toolchain checklist.
///
/// A found requirement shows where it was found; a missing one shows **the
/// command that installs it**. A checklist that says "missing" and stops is a
/// research task handed to the user.
fn requirement_row(
    requirement: &crate::toolchains::Requirement,
    colors: ColorScheme,
) -> WidgetNode {
    let found = requirement.satisfied();
    let detail = requirement.found.as_ref().map_or_else(
        || requirement.install.to_string(),
        |path| path.display().to_string(),
    );

    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 5.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            // A mark rather than colour alone: "installed" and
                            // "missing" have to be distinguishable without
                            // seeing the difference between two greys.
                            //
                            // A vector icon rather than `\u{2713}`, which is
                            // what this was and which **rendered as nothing** —
                            // the bundled font has no glyph for it, so every
                            // satisfied requirement showed a blank where its
                            // tick should be. Found by looking at the
                            // screenshot; no assertion about a tree would have
                            // caught it, because the text was there.
                            if found {
                                WidgetNode::from(crate::ui::chrome::glyph(
                                    icons::check(),
                                    12.0,
                                    colors.primary,
                                ))
                            } else {
                                WidgetNode::from(crate::ui::chrome::glyph(
                                    icons::error(),
                                    11.0,
                                    colors.outline,
                                ))
                            },
                            space(6.0),
                            label(requirement.name, 11.5, colors.on_surface),
                        ]),
                    space(2.0),
                    Text::new(elide(&detail, 46))
                        .style(TextStyle::new(10.5).family(FontFamily::Monospace))
                        .color(if found {
                            colors.outline
                        } else {
                            colors.on_surface_variant
                        }),
                ]),
        )
        .into()
}

/// The buttons under a missing requirement: run it, or open a terminal for it.
///
/// # Why two buttons and not one
///
/// Because the two commands are not the same kind of thing. `cargo install
/// cargo-ndk` runs unattended and belongs in the Output panel with every other
/// build. `sdkmanager --install "ndk;…"` stops and asks you to accept three
/// licence agreements — run in a panel that has no keyboard it does not fail,
/// it hangs, which costs the user the ten minutes it takes to work out why.
///
/// So the studio says which one it is. The recommended action is first and
/// tinted; the other is available anyway, because a machine whose terminal the
/// studio guessed wrong about still has a person sitting at it.
fn install_row(
    studio: &Studio,
    requirement: &crate::toolchains::Requirement,
    command: &str,
    chrome: &StudioTheme,
    colors: ColorScheme,
) -> WidgetNode {
    use crate::setup::Runs;

    let name = requirement.name.to_owned();
    let command = command.to_owned();
    let wants = crate::setup::how_it_runs(&command);

    let small = |title: &'static str, tinted: bool, on_tap: Box<dyn Fn()>| -> WidgetNode {
        let chrome = *chrome;
        crate::ui::chrome::sensed(
            move |sense| {
                Container::new()
                    .color(if tinted {
                        chrome.chrome_3
                    } else {
                        crate::ui::chrome::hovered(chrome.chrome_1, &chrome, sense)
                    })
                    .radius(5.0)
                    .height(24.0)
                    .padding(EdgeInsets::symmetric(8.0, 0.0))
                    .alignment(Alignment::CENTER)
                    .child(label(
                        title,
                        11.0,
                        if tinted {
                            colors.primary
                        } else {
                            colors.on_surface_variant
                        },
                    ))
                    .into()
            },
            on_tap,
        )
        .into()
    };

    let terminal_first = wants == Runs::Interactively;

    let in_terminal = {
        let studio = studio.clone();
        let name = name.clone();
        let command = command.clone();
        // "Terminal", not "Open a terminal": the row lives in a sidebar whose
        // minimum width is 218 points, and three buttons with generous labels
        // overflowed it by four — which the framework reported and which nobody
        // would have seen in a wide window.
        small(
            "Terminal",
            terminal_first,
            Box::new(move || studio.install_in_terminal(&name, &command)),
        )
    };
    let in_panel = {
        let studio = studio.clone();
        let name = name.clone();
        let command = command.clone();
        small(
            "Install",
            !terminal_first,
            Box::new(move || studio.install_in_panel(&name, &command)),
        )
    };
    let copy = {
        let studio = studio.clone();
        let command = command.clone();
        small(
            "Copy",
            false,
            Box::new(move || studio.copy_install_command(&command)),
        )
    };

    let (first, second) = if terminal_first {
        (in_terminal, in_panel)
    } else {
        (in_panel, in_terminal)
    };

    Container::new()
        .padding(EdgeInsets::only(30.0, 0.0, 12.0, 8.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    // **Two rows, not three buttons across.** The sidebar's
                    // minimum is 180 points and this row sits 30 in from the
                    // left, which leaves 138 — the three side by side
                    // overflowed it by nine, on the one width somebody dragging
                    // the divider narrow is guaranteed to hit. There is no
                    // wrapping flex in the library, so the wrap is written
                    // here, where the grouping also means something: the two
                    // ways to run it, then the way to take it elsewhere.
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![first, space(6.0), second]),
                    space(4.0),
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![copy]),
                    space(4.0),
                    // What is about to run, in full. A button that hides its
                    // command is a button people are right not to press.
                    Text::new(command.clone())
                        .style(TextStyle::new(10.0).family(FontFamily::Monospace))
                        .color(colors.outline),
                ]),
        )
        .into()
}

/// A labelled text field for a path or an identifier.
///
/// Writes straight into the signal, and re-scans on every keystroke — which is
/// cheap (`toolchains::detect` is a `PATH` walk and a handful of `is_dir`
/// calls) and is what makes the tick beside "Android SDK" turn green as the
/// last character of a correct path is typed. A field that needed a separate
/// Apply is a field people fill in wrong and never find out.
/// One setting: a title, the sentence that explains it, and the field.
///
/// # Why the sentence is not the placeholder
///
/// Reported, with a screenshot of the Toolchain view in a default-width
/// sidebar: boxes of wrapped prose stacked down the column, "clumsy". The
/// explanation used to *be* the placeholder — "ANDROID_HOME — the folder
/// holding platform-tools" inside a 26-point single-line box — so a sentence
/// that needs three lines was drawn inside a control one line tall, in a
/// sidebar 218 points wide. Every one of the six settings did it, and the
/// section read as six paragraphs in six boxes.
///
/// A placeholder answers "what goes in here" in the space of the thing typed;
/// anything longer is documentation and belongs beside the field, not inside
/// it. So the variable name stays in the box, where it is also what the user
/// is about to paste a value next to, and the sentence moves above it as a
/// caption that is allowed to wrap because it is prose.
fn path_row(
    title: &str,
    placeholder: &str,
    hint: &str,
    signal: &vieww_element::Signal<String>,
    studio: &Studio,
    chrome: &StudioTheme,
    colors: ColorScheme,
) -> WidgetNode {
    let value = signal.get();
    let field = TextField::text(value)
        .size(11.5)
        .style(
            TextStyle::new(11.5)
                .family(FontFamily::Monospace)
                .color(colors.on_surface),
        )
        .placeholder(placeholder)
        .single_line()
        .selection_color(chrome.selection)
        .on_changed({
            let signal = signal.clone();
            let studio = studio.clone();
            Rc::new(move |value: vieww_foundation::TextEditingValue| {
                signal.set(value.text);
                studio.rescan_toolchains();
            })
        });

    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 4.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .children(children![
                    label(title, 11.0, colors.on_surface_variant),
                    space(2.0),
                    Text::new(hint.to_owned())
                        .style(TextStyle::new(10.0).line_height(1.35))
                        .color(colors.outline),
                    space(4.0),
                    Container::new()
                        .color(chrome.chrome_0)
                        .radius(5.0)
                        .height(26.0)
                        .padding(EdgeInsets::symmetric(8.0, 0.0))
                        .alignment(Alignment::CENTER_LEFT)
                        .border(vieww_foundation::Border {
                            color: chrome.line,
                            width: 1.0,
                        })
                        .child(field),
                ]),
        )
        .into()
}

/// `text`, cut to `characters` and given an ellipsis if it had to be cut.
///
/// Counted in `char`s rather than bytes, so a multi-byte identifier is not
/// sliced through the middle of a character — which panics rather than
/// truncating.
fn elide(text: &str, characters: usize) -> String {
    let mut out: String = text.chars().take(characters).collect();
    if text.chars().count() > characters {
        out.push('…');
    }
    out
}

fn note(colors: ColorScheme, text: &str) -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::all(12.0))
        .child(label(text, 12.0, colors.on_surface_variant))
        .into()
}

/// A setting with a handful of values rather than two.
///
/// Every choice is a button and the current one is filled. A stepper would need
/// a bound, a wrap rule and two more hit targets to say the same thing, and
/// with three to five options there is nothing a stepper buys.
fn choice_row(
    title: &str,
    choices: &[f32],
    current: f32,
    colors: ColorScheme,
    on_pick: impl Fn(f32) + Clone + 'static,
) -> WidgetNode {
    let mut chips: Vec<WidgetNode> = Vec::new();
    for &choice in choices {
        let picked = (choice - current).abs() < 0.001;
        let on_pick = on_pick.clone();
        chips.push(
            clickable(
                move || {
                    Container::new()
                        .radius(4.0)
                        .color(if picked {
                            colors.primary
                        } else {
                            colors.surface_variant
                        })
                        .padding(EdgeInsets::symmetric(7.0, 3.0))
                        .child(label(
                            // Trimmed rather than `{:.1}`: "13" and "1.3" are
                            // both what somebody wrote, and "13.0" is neither.
                            &format!("{choice}"),
                            10.5,
                            if picked {
                                colors.on_primary
                            } else {
                                colors.on_surface_variant
                            },
                        ))
                        .into()
                },
                move || on_pick(choice),
            )
            .into(),
        );
        chips.push(space(4.0));
    }
    // **Title and choices on two lines, not one row.**
    //
    // One row of "Font size  11 12 13 14 16" needed about 205 points and the
    // sidebar's floor leaves 154 of content, so at a narrow sidebar the last
    // chips were cut by the card's clip — the same class as the accent
    // swatches overflowing their pane, one pane over. Title above, choices
    // below: the chips need about 146 at the worst setting and fit at the
    // floor with room, and a settings pane that groups its label above the
    // values it describes is the shape every settings panel people arrive
    // from already uses.
    Container::new()
        .padding(EdgeInsets::symmetric(12.0, 0.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    Container::new()
                        .padding(EdgeInsets::only(0.0, 0.0, 0.0, 4.0))
                        .child(label(title, 12.0, colors.on_surface)),
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(chips),
                ]),
        )
        .into()
}

fn action_row(
    title: String,
    active: bool,
    colors: ColorScheme,
    on_tap: impl Fn() + 'static,
) -> WidgetNode {
    // **A settings row had no way of looking clickable.** It is a line of text
    // and the word ON — the same shape as the read-only stat rows a few
    // hundred pixels above it in the same pane — so the only way to find out
    // whether it was a control was to click it. Hover is what separates the two.
    //
    // The `ON`/`OFF` brightens with it: at rest it is `outline`, which is the
    // right weight for a value nobody is looking at and too faint to read as
    // the thing about to change.
    crate::ui::chrome::sensed(
        move |sense| {
            let lit = sense.emphasis() > 0.0;
            Container::new()
                .color(crate::ui::chrome::hover_overlay(colors, sense))
                .height(34.0)
                .padding(EdgeInsets::symmetric(12.0, 0.0))
                .child(
                    Flex::row()
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            // **The title is the row's one flexible child, and
                            // this is the Diagnostics fix.** It used to be a
                            // bare label with a `gap()` after it, which a flex
                            // lays out at its *natural* width — and the longest
                            // title in the pane ("vieww-lint findings in
                            // Problems") is wider than the sidebar's floor, so
                            // it painted past the card's clip and came back cut
                            // mid-glyph, "…findings in Proble".
                            //
                            // Making the title the expanded child and dropping
                            // the spacer keeps the original geometry in every
                            // case that already worked — the ON/OFF is still
                            // laid out first at its natural size and still ends
                            // at the right edge, and a short title still starts
                            // at the left — while giving the label a bounded
                            // width to elide against, which is the one thing a
                            // bare label in a flex never has. `Text`'s own docs
                            // say it: without a bounded width "nothing is ever
                            // too wide and nothing is ever cut".
                            Flexible::expanded(1).child(
                                label(&title, 12.0, colors.on_surface)
                                    .overflow(vieww_text::TextOverflow::Ellipsis)
                                    .max_lines(1),
                            ),
                            label(
                                if active { "ON" } else { "OFF" },
                                10.0,
                                if lit {
                                    colors.on_surface_variant
                                } else {
                                    colors.outline
                                },
                            )
                        ]),
                )
                .into()
        },
        on_tap,
    )
    .into()
}
