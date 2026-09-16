//! The modal the studio did not have.
//!
//! # Why a shared one rather than three bespoke ones
//!
//! Three of the findings in the readiness audit are the same missing thing
//! wearing different hats:
//!
//! * closing the window on a modified buffer lost it, with no dialog;
//! * an external change to a *dirty* buffer was dropped in silence, because
//!   there was nowhere to ask "keep yours or take theirs?";
//! * deleting a file had no confirmation, which is most of why
//!   `file_tree::delete_to_trash` was written and never called.
//!
//! Each one is a card with a sentence, a list, and two or three buttons. Built
//! three times they would drift — different padding, different button order,
//! one of them missing the scrim — and the third would be the one somebody
//! wrote at the end of a session. Built once, the *shape* of an irreversible
//! question is decided in one place.
//!
//! # The rules this encodes
//!
//! **The scrim does not dismiss.** The folder picker's does, and that is right
//! for a picker: clicking away from "which folder?" means "none, thanks". It is
//! wrong for "you have unsaved work" — a stray click outside the card must not
//! be the answer to a question about losing data. Cancel is a button here, and
//! only a button.
//!
//! **The safe answer is on the right and drawn as the default.** The dangerous
//! one is on the left, in the error colour, and never styled as the primary
//! action. A user who dismisses this dialog by muscle memory should end up
//! having kept their work.
//!
//! **The list is bounded and says so.** Twelve unsaved files is a list; ninety
//! is a wall the buttons fall off the bottom of.

use vieww_foundation::{Alignment, Border, Color, EdgeInsets};
use vieww_widget::prelude::*;
use vieww_widget::{Container, Flexible, SizedBox};

use crate::theme::StudioTheme;
use crate::ui::chrome::{button, clickable, hairline, label, label_bold, mono, space};

const WIDTH: f32 = 460.0;

/// How many list rows are drawn before the rest are summarised.
pub const VISIBLE: usize = 8;

/// How a dialog button is drawn, which is a statement about what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The safe answer. Filled, and the one the eye lands on.
    Primary,
    /// Neither safe nor dangerous — "cancel", usually.
    Neutral,
    /// Irreversible. Drawn in the error colour and never filled, because a
    /// filled red button beside a filled blue one is two primaries.
    Danger,
}

/// One button in the row along the bottom.
///
/// `Rc` rather than `Box` for the handler, and that is not an implementation
/// detail: `Widget::build` takes `&self`, while the closure a `Pressable` wants
/// must be `'static`. An `Rc` can be cloned out of a borrow; a `Box` cannot.
/// The actions are rebuilt on every frame, so nothing here outlives its tree.
pub struct Action {
    pub label: String,
    pub tone: Tone,
    pub on_tap: std::rc::Rc<dyn Fn()>,
}

impl Action {
    pub fn new(label: impl Into<String>, tone: Tone, on_tap: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            tone,
            on_tap: std::rc::Rc::new(on_tap),
        }
    }
}

impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Action")
            .field("label", &self.label)
            .field("tone", &self.tone)
            .finish_non_exhaustive()
    }
}

/// A modal question: a title, a sentence, an optional list, and answers.
///
/// Actions are given **safe-first** and drawn right-to-left, so the caller
/// writes them in the order they think about them and the layout puts the safe
/// one under the cursor's resting place.
#[derive(Debug)]
pub struct Dialog {
    pub title: String,
    pub message: String,
    /// Monospaced, because every use of it so far is a list of file names.
    pub items: Vec<String>,
    /// Something to put between the message and the buttons — a text field,
    /// for the dialogs that ask rather than only tell.
    pub body: Option<WidgetNode>,
    /// A line under the body, in the error colour. For validation that has to
    /// be visible without moving the buttons.
    pub footnote: Option<String>,
    pub actions: Vec<Action>,
}

impl Dialog {
    #[must_use]
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            items: Vec::new(),
            body: None,
            footnote: None,
            actions: Vec::new(),
        }
    }

    #[must_use]
    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    #[must_use]
    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }
}

impl Widget for Dialog {
    fn debug_name(&self) -> &'static str {
        "Dialog"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        let mut column: Vec<WidgetNode> = vec![
            Container::new()
                .padding(EdgeInsets::only(18.0, 16.0, 18.0, 8.0))
                .child(label_bold(&self.title, 14.0, colors.on_surface))
                .into(),
            Container::new()
                .padding(EdgeInsets::only(18.0, 0.0, 18.0, 14.0))
                .child(label(&self.message, 12.0, colors.on_surface_variant))
                .into(),
        ];

        if !self.items.is_empty() {
            let mut rows: Vec<WidgetNode> = Vec::new();
            for item in self.items.iter().take(VISIBLE) {
                rows.push(
                    Container::new()
                        .padding(EdgeInsets::symmetric(0.0, 2.0))
                        .child(mono(item, 11.5, colors.on_surface))
                        .into(),
                );
            }
            if self.items.len() > VISIBLE {
                // Counted, not truncated in silence. "and 4 more" is the
                // difference between a list and a claim that this is all of
                // them — the same rule workspace search follows.
                rows.push(
                    Container::new()
                        .padding(EdgeInsets::symmetric(0.0, 2.0))
                        .child(label(
                            &format!("and {} more", self.items.len() - VISIBLE),
                            11.0,
                            colors.outline,
                        ))
                        .into(),
                );
            }
            column.push(
                Container::new()
                    .padding(EdgeInsets::only(18.0, 0.0, 18.0, 14.0))
                    .child(
                        Container::new()
                            .color(chrome.chrome_1)
                            .radius(6.0)
                            .padding(EdgeInsets::symmetric(12.0, 10.0))
                            .child(
                                Flex::column()
                                    .main_axis_size(MainAxisSize::Min)
                                    .cross_axis_alignment(CrossAxisAlignment::Start)
                                    .children(rows),
                            ),
                    )
                    .into(),
            );
        }

        if let Some(body) = &self.body {
            column.push(
                Container::new()
                    .padding(EdgeInsets::only(18.0, 0.0, 18.0, 10.0))
                    .child(body.clone())
                    .into(),
            );
        }
        if let Some(footnote) = &self.footnote {
            column.push(
                Container::new()
                    .padding(EdgeInsets::only(18.0, 0.0, 18.0, 12.0))
                    .child(label(footnote, 11.5, colors.error))
                    .into(),
            );
        }

        column.push(hairline(chrome.line));
        column.push(self.buttons(colors, chrome));

        Stack::new()
            .alignment(Alignment::TOP_CENTER)
            .children(children![
                // A scrim that swallows clicks and answers nothing. See the
                // module docs: dismissing a data-loss question by clicking
                // beside it is not an answer anybody meant to give.
                Positioned::new()
                    .left(0.0)
                    .right(0.0)
                    .top(0.0)
                    .bottom(0.0)
                    .child(swallow(Color::rgba(0, 0, 0, 120))),
                Positioned::new().top(96.0).child(
                    Container::new()
                        .color(chrome.chrome_2)
                        .radius(10.0)
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

vieww_widget::widget_node_from!(Dialog);

impl Dialog {
    fn buttons(&self, colors: ColorScheme, chrome: StudioTheme) -> WidgetNode {
        let mut row: Vec<WidgetNode> = vec![Flexible::expanded(1).child(SizedBox::shrink()).into()];
        // Reversed: the caller writes safe-first and the safe answer belongs on
        // the right, where a dialog's default has been for forty years.
        for action in self.actions.iter().rev() {
            row.push(space(8.0));
            row.push(pill(action, colors, chrome));
        }
        Container::new()
            .padding(EdgeInsets::symmetric(14.0, 12.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(row),
            )
            .into()
    }
}

fn pill(action: &Action, colors: ColorScheme, chrome: StudioTheme) -> WidgetNode {
    let (fill, text) = match action.tone {
        Tone::Primary => (Some(colors.primary), colors.on_primary),
        Tone::Neutral => (None, colors.on_surface),
        Tone::Danger => (None, colors.error),
    };
    let label_text = action.label.clone();
    let handler = std::rc::Rc::clone(&action.on_tap);
    button(
        action.label.clone(),
        move || {
            let mut container = Container::new()
                .radius(6.0)
                .padding(EdgeInsets::symmetric(14.0, 8.0))
                .alignment(Alignment::CENTER)
                .child(label_bold(&label_text, 12.0, text));
            container = match fill {
                Some(color) => container.color(color),
                None => container.border(Border {
                    color: chrome.line,
                    width: 1.0,
                }),
            };
            container.into()
        },
        move || (handler)(),
    )
}

/// A block that eats every pointer event and does nothing with them.
///
/// `Pressable` with an empty handler rather than a plain `Container`: a
/// container is not a hit-test target, so clicks would fall through the scrim
/// to the editor behind it and a modal that can be typed around is not modal.
fn swallow(color: Color) -> WidgetNode {
    Pressable::new(move |_press| Container::new().color(color).into())
        .on_tap(|| {})
        .into()
}

/// The unsaved-work question, raised by a vetoed close.
///
/// Absent from the tree unless [`Studio::quit_prompt`] holds something, which
/// is the same rule the palette and the picker follow: an overlay that is
/// present-but-transparent still eats the pointer events under it.
///
/// [`Studio::quit_prompt`]: crate::Studio::quit_prompt
#[derive(Debug)]
pub struct QuitPrompt {
    pub studio: crate::state::Studio,
}

impl Widget for QuitPrompt {
    fn debug_name(&self) -> &'static str {
        "QuitPrompt"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let Some(unsaved) = self.studio.quit_prompt.get() else {
            return SizedBox::shrink().into();
        };
        let count = unsaved.len();
        let message = if count == 1 {
            "One file has changes that have not been written to disk.".to_owned()
        } else {
            format!("{count} files have changes that have not been written to disk.")
        };

        let saving = self.studio.clone();
        let discarding = self.studio.clone();
        let cancelling = self.studio.clone();

        Dialog::new("Close vieww Studio?", message)
            .items((*unsaved).clone())
            // Safe first — `Dialog` draws them right to left, so this ends up
            // as [Close without saving] [Cancel] [Save all and close].
            .action(Action::new(
                "Save all and close",
                Tone::Primary,
                move || {
                    saving.confirm_quit_saving();
                    saving.request_close();
                },
            ))
            .action(Action::new("Cancel", Tone::Neutral, move || {
                cancelling.cancel_quit();
            }))
            .action(Action::new(
                "Close without saving",
                Tone::Danger,
                move || {
                    discarding.confirm_quit_discarding();
                    discarding.request_close();
                },
            ))
            .into()
    }
}

vieww_widget::widget_node_from!(QuitPrompt);

/// The file-changed-on-disk question.
///
/// Raised by [`Studio::drain_watcher`] when a buffer with unsaved edits has
/// its file changed underneath it. Before this existed that branch was a
/// silent no-op — see the comment on it, which is the shortest description of
/// the bug this dialog closes.
///
/// [`Studio::drain_watcher`]: crate::Studio::drain_watcher
#[derive(Debug)]
pub struct ConflictPrompt {
    pub studio: crate::state::Studio,
}

impl Widget for ConflictPrompt {
    fn debug_name(&self) -> &'static str {
        "ConflictPrompt"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let names = self.studio.conflict_names();
        if names.is_empty() {
            return SizedBox::shrink().into();
        }
        let message = if names.len() == 1 {
            "This file changed on disk while you had unsaved edits to it. \
             Only one of the two versions can survive."
                .to_owned()
        } else {
            format!(
                "{} files changed on disk while you had unsaved edits to them. \
                 Only one of the two versions can survive.",
                names.len()
            )
        };

        let mine = self.studio.clone();
        let theirs = self.studio.clone();

        Dialog::new("Changed on disk", message)
            .items(names)
            // "Keep mine" is the safe answer here — it is the one that does not
            // discard anything the user typed — so it is the primary, and
            // reloading is the destructive one.
            .action(Action::new("Keep my version", Tone::Primary, move || {
                mine.keep_mine();
            }))
            .action(Action::new(
                "Discard mine and reload",
                Tone::Danger,
                move || theirs.take_theirs(),
            ))
            .into()
    }
}

vieww_widget::widget_node_from!(ConflictPrompt);

/// "What should it be called?" — for New File, New Folder and Rename.
///
/// A dialog rather than an inline editable row in the tree. Inline is nicer and
/// is a much larger change to the Explorer; this one exists because the
/// operations behind it had no caller at all, and a dialog that works beats an
/// inline editor that is not written.
#[derive(Debug)]
pub struct NamePromptDialog {
    pub studio: crate::state::Studio,
}

impl Widget for NamePromptDialog {
    fn debug_name(&self) -> &'static str {
        "NamePromptDialog"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(prompt) = self.studio.name_prompt.get() else {
            return SizedBox::shrink().into();
        };
        let chrome = *StudioTheme::of(ctx);
        let colors = ThemeData::of(ctx).colors;

        let studio = self.studio.clone();
        let field: WidgetNode = Container::new()
            .color(chrome.chrome_1)
            .radius(6.0)
            .padding(EdgeInsets::symmetric(10.0, 8.0))
            .border(Border {
                color: if prompt.error.is_some() {
                    colors.error
                } else {
                    chrome.line
                },
                width: 1.0,
            })
            .child(
                TextField::text(prompt.text.clone())
                    .size(12.5)
                    .style(
                        TextStyle::new(12.5)
                            .family(vieww_foundation::FontFamily::Monospace)
                            .color(colors.on_surface),
                    )
                    .placeholder(match prompt.kind {
                        // Different hints for different rules: a crate name is
                        // lowercased by cargo but may contain `-`, while a file
                        // name may be anything that is not a separator. The
                        // placeholder is what somebody types over, so the rule
                        // it implies is the one the user carries away.
                        crate::state::NameKind::NewProject => "my-app",
                        _ => "name",
                    })
                    .selection_color(chrome.selection)
                    .cursor(colors.primary, 2.0)
                    .on_changed(std::rc::Rc::new(
                        move |value: vieww_foundation::TextEditingValue| {
                            studio.set_prompt_name(value.text);
                        },
                    )),
            )
            .into();

        // **The kind is a preference-shaped choice on the card where it
        // belongs** — not a startup question. Two chips under the name field;
        // either project accepts both languages afterwards, and the card says
        // so in one line so nobody feels they are signing something.
        let kind_chips: WidgetNode = if prompt.kind == crate::state::NameKind::NewProject {
            let choosing_say = self.studio.clone();
            let choosing_rust = self.studio.clone();
            let say_chip: WidgetNode = {
                let selected = prompt.project_kind == crate::scaffold::ProjectKind::Say;
                clickable(
                    move || {
                        Container::new()
                            .padding(EdgeInsets::symmetric(14.0, 6.0))
                            .radius(6.0)
                            .color(if selected {
                                chrome.selection
                            } else {
                                chrome.chrome_1
                            })
                            .border(Border {
                                color: if selected {
                                    colors.primary
                                } else {
                                    chrome.line
                                },
                                width: 1.0,
                            })
                            .child(label_bold("Say", 12.0, colors.on_surface))
                            .into()
                    },
                    move || choosing_say.set_prompt_kind(crate::scaffold::ProjectKind::Say),
                )
                .into()
            };
            let rust_chip: WidgetNode = {
                let selected = prompt.project_kind == crate::scaffold::ProjectKind::Rust;
                clickable(
                    move || {
                        Container::new()
                            .padding(EdgeInsets::symmetric(14.0, 6.0))
                            .radius(6.0)
                            .color(if selected {
                                chrome.selection
                            } else {
                                chrome.chrome_1
                            })
                            .border(Border {
                                color: if selected {
                                    colors.primary
                                } else {
                                    chrome.line
                                },
                                width: 1.0,
                            })
                            .child(label_bold("Rust", 12.0, colors.on_surface))
                            .into()
                    },
                    move || choosing_rust.set_prompt_kind(crate::scaffold::ProjectKind::Rust),
                )
                .into()
            };
            Flex::row()
                .spacing(8.0)
                .children(children![say_chip, rust_chip])
                .into()
        } else {
            SizedBox::shrink().into()
        };

        let confirming = self.studio.clone();
        let cancelling = self.studio.clone();

        let mut dialog = Dialog::new(
            prompt.kind.title(),
            // Says *where*, because "New File" with no location is a question
            // the user cannot answer confidently from a folder tree that has
            // twenty directories in it. New Project says it too, because
            // "New Project" without a parent is the same shape of question
            // — except the parent here is a directory the studio chose rather
            // than one the user clicked, so naming it is what tells the user
            // where on disk the project will land.
            format!("In {}", prompt.target.display()),
        );
        let kind_blurb: &str = match prompt.kind {
            crate::state::NameKind::NewProject => prompt.project_kind.blurb(),
            _ => "",
        };
        let blurb_line: WidgetNode = label(kind_blurb, 11.0, colors.outline).into();
        let body_column = Flex::column()
            .spacing(10.0)
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(children![kind_chips, field, blurb_line]);
        dialog.body = Some(body_column.into());
        if let Some(error) = &prompt.error {
            dialog.footnote = Some(error.clone());
        }
        dialog
            .action(Action::new(
                // The button says what will happen. "Create" is right for the
                // three that make something new and wrong for the two that do
                // not: a Save As whose button says Create reads as a second
                // file appearing beside the one you were editing.
                match prompt.kind {
                    crate::state::NameKind::Rename => "Rename",
                    crate::state::NameKind::SaveAs => "Save",
                    crate::state::NameKind::NewFile
                    | crate::state::NameKind::NewFolder
                    | crate::state::NameKind::NewProject => "Create",
                },
                Tone::Primary,
                move || confirming.confirm_name_prompt(),
            ))
            .action(Action::new("Cancel", Tone::Neutral, move || {
                cancelling.close_name_prompt();
            }))
            .into()
    }
}

vieww_widget::widget_node_from!(NamePromptDialog);

/// "Move this to the trash?"
///
/// The confirmation that made `delete_to_trash` reachable. Worth being exact
/// about what it promises: this is a *move*, into `.viewwstudio-trash` at the
/// workspace root, which the tree scan skips. Nothing is unlinked, which is
/// what makes saying yes survivable.
#[derive(Debug)]
pub struct DeletePrompt {
    pub studio: crate::state::Studio,
}

impl Widget for DeletePrompt {
    fn debug_name(&self) -> &'static str {
        "DeletePrompt"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let Some(name) = self.studio.pending_delete_name() else {
            return SizedBox::shrink().into();
        };
        let confirming = self.studio.clone();
        let cancelling = self.studio.clone();

        // **The workspace root gets an explanation, not an offer.** The trash is
        // a folder inside the workspace, so the root cannot be moved into it —
        // the kernel's answer is `EINVAL`, which reached the status bar as
        // "Invalid argument" and read, correctly, as the studio ignoring the
        // click. The Explorer no longer offers the entry; this covers every
        // other way the request can arrive.
        if self
            .studio
            .pending_delete
            .get()
            .is_some_and(|path| self.studio.is_workspace_root(&path))
        {
            let dismiss = self.studio.clone();
            return Dialog::new(
                "That folder is the workspace",
                "The trash is a folder inside it, so the workspace cannot be \
                 moved into its own trash. Close the workspace to stop working \
                 on it, or delete the folder from outside the studio."
                    .to_owned(),
            )
            .items(vec![name])
            .action(Action::new("OK", Tone::Neutral, move || {
                dismiss.cancel_delete();
            }))
            .into();
        }

        Dialog::new(
            "Move to trash?",
            "It goes to a .viewwstudio-trash folder at the workspace root, \
             not to the wastebasket and not to nothing. You can put it back by hand."
                .to_owned(),
        )
        .items(vec![name])
        .action(Action::new("Cancel", Tone::Neutral, move || {
            cancelling.cancel_delete();
        }))
        .action(Action::new("Move to trash", Tone::Danger, move || {
            confirming.confirm_delete();
        }))
        .into()
    }
}

vieww_widget::widget_node_from!(DeletePrompt);

/// "This will change 30 files." — the only unreviewable action in the studio,
/// made reviewable.
///
/// There is no undo across files: the history is per buffer, by design. So a
/// replace-all with no preview is a change to thirty files that cannot be
/// taken back from inside the editor. This is what the user says yes to.
#[derive(Debug)]
pub struct ReplacePlanDialog {
    pub studio: crate::state::Studio,
}

impl Widget for ReplacePlanDialog {
    fn debug_name(&self) -> &'static str {
        "ReplacePlanDialog"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let Some(plan) = self.studio.replace_plan.get() else {
            return SizedBox::shrink().into();
        };
        let root = self.studio.root.get();
        let total = plan.total();
        let files = plan.files.len();
        let needle = self.studio.find_query.get();
        let replacement = self.studio.find_replacement.get();

        let message = if total == 0 {
            "Nothing would be changed — every file that matched is being skipped.".to_owned()
        } else {
            format!(
                "Replace {total} match{} across {files} file{}. \
                 This cannot be undone: the editor's history is per file, and \
                 these files are written directly.",
                if total == 1 { "" } else { "es" },
                if files == 1 { "" } else { "s" }
            )
        };

        let confirming = self.studio.clone();
        let cancelling = self.studio.clone();

        let mut dialog = Dialog::new(
            format!(
                "Replace \u{201c}{}\u{201d} with \u{201c}{}\u{201d}?",
                needle, replacement
            ),
            message,
        )
        .items(plan.lines(root.as_deref()));

        if total > 0 {
            dialog = dialog.action(Action::new("Cancel", Tone::Neutral, move || {
                cancelling.cancel_workspace_replace();
            }));
            dialog = dialog.action(Action::new(
                format!(
                    "Replace in {files} file{}",
                    if files == 1 { "" } else { "s" }
                ),
                Tone::Danger,
                move || confirming.confirm_workspace_replace(),
            ));
        } else {
            dialog = dialog.action(Action::new("Close", Tone::Primary, move || {
                cancelling.cancel_workspace_replace();
            }));
        }
        dialog.into()
    }
}

vieww_widget::widget_node_from!(ReplacePlanDialog);

/// The live-preview caution: explains that the live preview is a visual
/// representation, not the user's code, and that Build and Run is needed for
/// the real app.
///
/// "Continue" mounts the demo; "Cancel" closes the dialog without mounting.
#[derive(Debug)]
pub struct LiveCaution {
    pub studio: crate::state::Studio,
}

impl Widget for LiveCaution {
    fn debug_name(&self) -> &'static str {
        "LiveCaution"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        if !self.studio.live_caution.get() {
            return SizedBox::shrink().into();
        }

        let accepting = self.studio.clone();
        let cancelling = self.studio.clone();

        Dialog::new(
            "Live Preview",
            "This draws live.rs from this workspace without compiling it, so it \
             follows the file as you type. It is a sketch of the flow — screens, \
             rows and the buttons between them — and its switches and counters \
             are the preview's, not your app's.\n\nA `mount` line places one of \
             your own screens in the flow, as it last rendered. Render compiles \
             a screen for real; Build and Run gives you the app.",
        )
        .action(Action::new("Cancel", Tone::Neutral, move || {
            cancelling.cancel_live_preview();
        }))
        .action(Action::new("Continue", Tone::Primary, move || {
            accepting.accept_live_preview();
        }))
        .into()
    }
}

vieww_widget::widget_node_from!(LiveCaution);

/// "There is no live.rs here."
///
/// The Live Preview used to answer a workspace with no flow file by showing a
/// demo compiled into the studio — three screens of somebody else's app, in
/// every project, identical. This is the honest answer, and it carries the one
/// action that fixes it.
#[derive(Debug)]
pub struct LiveMissing {
    pub studio: crate::state::Studio,
}

impl Widget for LiveMissing {
    fn debug_name(&self) -> &'static str {
        "LiveMissing"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        if !self.studio.live_missing.get() {
            return SizedBox::shrink().into();
        }
        let creating = self.studio.clone();
        let cancelling = self.studio.clone();

        Dialog::new(
            "This workspace has no live.rs",
            "The Live Preview draws live.rs at the workspace root — screens, \
             rows, and the buttons between them — without compiling it. There \
             is no such file here, so there is nothing live to show.\n\nWriting \
             one gives you a flow to edit; deleting it later turns the Live \
             Preview off again.",
        )
        .items(vec![crate::livedoc::FILE.to_owned()])
        .action(Action::new("Not now", Tone::Neutral, move || {
            cancelling.live_missing.set(false);
        }))
        .action(Action::new("Create live.rs", Tone::Primary, move || {
            creating.live_missing.set(false);
            creating.create_live_file();
        }))
        .into()
    }
}

vieww_widget::widget_node_from!(LiveMissing);
