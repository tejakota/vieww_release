//! The folder picker.
//!
//! A modal file browser, with its own navigation, filtering and history.
//!
//! # Why this is a separate file and not a separate type
//!
//! Because it is the same object. `Studio` holds the application's signals in
//! one place deliberately — that is what lets any widget read any of them
//! without a chain of props threaded through the tree, and it is why no widget
//! in the studio owns state of its own. Splitting the *type* would mean
//! inventing ownership boundaries the interface does not have.
//!
//! What was worth splitting is the **code**. An inherent `impl` may be written
//! in as many blocks as it has subjects; `state.rs` had already marked those
//! subjects with comment rules, and this file is one of those rules made
//! structural. Nothing moved between types and no signature changed.
//!
//! # `pub(super)` on the helpers
//!
//! The private helpers below were private *to a file* when there was one file,
//! and several are called from what are now sibling modules. `pub(super)` is
//! that same reachability written down: visible throughout `state` and nowhere
//! else. Nothing here became public, and the crate's outside surface is byte for
//! byte what it was.

use super::*;

impl Studio {
    // ----- the folder picker ---------------------------------------------

    /// Open the picker in [`PickerMode::Open`], at the workspace root if there
    /// is one. The File → Open Folder… command.
    /// Drop a menu, or shut the one that is open.
    ///
    /// Through here rather than by setting `menu_open` directly, so a menu
    /// always opens scrolled to its first item — see [`Self::menu_scroll`].
    pub fn open_menu(&self, menu: Option<crate::command::Menu>) {
        if menu.is_some() {
            self.menu_scroll.jump_to(0.0);
        }
        self.menu_open.set(menu);
    }

    pub fn open_picker(&self) {
        self.picker_mode.set(PickerMode::Open);
        let at = self.root.peek().clone();
        let picker = at.map_or_else(crate::picker::Picker::start, crate::picker::Picker::at);
        self.picker.set(Some(Rc::new(picker)));
    }

    /// Shut it without choosing anything.
    pub fn close_picker(&self) {
        self.picker.set(None);
    }

    /// Move the picker to `path`.
    pub fn picker_to(&self, path: &std::path::Path) {
        self.picker
            .set(Some(Rc::new(crate::picker::Picker::at(path))));
    }

    /// Move the picker to the directory above, if there is one.
    pub fn picker_up(&self) {
        let Some(parent) = self.picker.peek().as_ref().and_then(|p| p.parent()) else {
            return;
        };
        self.picker_to(&parent);
    }

    /// Open whatever the picker is showing, and shut it.
    ///
    /// What "open" means depends on [`picker_mode`](Studio::picker_mode):
    ///
    /// - [`PickerMode::Open`] opens the picked folder as the workspace. The
    ///   historical behaviour, and what File → Open Folder… wants.
    /// - [`PickerMode::NewProject`] does **not** open anything — it closes
    ///   the picker and opens the name prompt with the picked folder as the
    ///   parent, so the second step (typing the project's name) is what the
    ///   user actually chose to do.
    ///
    /// The branch is here rather than in the picker's UI because the picker
    /// does not know what mode it is in — it draws a card and a confirm
    /// button, and the studio is what those confirmations are addressed to.
    /// Keeping the mode here is also what makes the picker reusable: any
    /// future caller can set the mode and the picker does not change.
    pub fn picker_choose(&self) {
        let Some(at) = self.picker.peek().as_ref().map(|p| p.cwd.clone()) else {
            return;
        };
        let mode = self.picker_mode.get();
        self.close_picker();

        match mode {
            PickerMode::Open => {
                self.open_workspace(&at);
                let name = at
                    .file_name()
                    .map_or_else(|| at.display().to_string(), |n| n.to_string_lossy().into());
                if self.is_vieww.peek() {
                    self.notify(&format!("Opened {name}."));
                } else {
                    // Stated rather than left to be discovered when Build greys itself
                    // out: a folder with no vieww in its Cargo.toml is a folder the
                    // studio can edit and cannot preview.
                    self.notify(&format!(
                        "Opened {name}. Its Cargo.toml does not mention vieww, so Render will refuse."
                    ));
                }
            }
            PickerMode::NewProject => {
                // Don't open anything. The picked folder is the *parent* the
                // new project will live inside, and the name is the half the
                // user actually cares about — so hand off to the name prompt,
                // which `scaffold_named_project` then writes to disk.
                self.ask_for_name(NameKind::NewProject, &at);
            }
        }
    }

    /// The first clause of `why`, as a sentence.
    ///
    /// Refusals elsewhere in the studio are written lower-case and unpunctuated
    /// because they are appended to a line of output. A menu row is neither, so
    /// they are normalised here rather than rewritten at every call site — and
    /// cut at the first em dash, because the part before it is the reason and
    /// the part after it is the explanation.
    #[must_use]
    pub(super) fn sentence_of(why: &str) -> String {
        let head = why.split(" \u{2014} ").next().unwrap_or(why).trim();
        let mut characters = head.chars();
        let mut out = match characters.next() {
            Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
            None => return String::new(),
        };
        if !out.ends_with(['.', '!', '?']) {
            out.push('.');
        }
        out
    }

    /// Why `command` is greyed out, in one sentence, or `None` when it is not.
    ///
    /// # Why a disabled item has to explain itself
    ///
    /// In the screencast, seven of the nine Build menu items are grey and the
    /// menu says nothing about any of them. The disabling is *correct* — there
    /// is no project open — but a menu that greys seven rows and explains none
    /// reads as a broken application rather than an unconfigured one. The
    /// Toolchain view already names the command that installs each missing
    /// requirement; this is the same courtesy for everything else.
    ///
    /// Kept beside [`Studio::can_run`] on purpose: the two answer the same
    /// question and must not be able to disagree about it.
    #[must_use]
    pub fn why_disabled(&self, command: Command) -> Option<String> {
        use Command as C;
        if self.can_run(command) {
            return None;
        }
        Some(match command {
            C::Render => {
                if self.is_compiling() {
                    "A render is already running.".to_owned()
                } else {
                    "No usable rustc — see the Toolchain view.".to_owned()
                }
            }
            C::CancelRender => "Nothing is rendering.".to_owned(),
            C::Undo => "Nothing to undo.".to_owned(),
            C::Redo => "Nothing to redo.".to_owned(),
            C::Save => "No unsaved changes.".to_owned(),
            C::SaveAll => "No unsaved changes.".to_owned(),
            C::RevertFile => "This buffer has no file on disk to revert to.".to_owned(),
            C::CloseTab => "Nothing is open.".to_owned(),
            C::NextTab | C::PreviousTab => "Only one buffer is open.".to_owned(),
            C::Format => "Only Rust files can be formatted.".to_owned(),
            C::ShowSource | C::RefreshGit | C::Fetch => {
                "The open folder is not a git repository.".to_owned()
            }
            C::Push if !self.is_repository() => {
                "The open folder is not a git repository.".to_owned()
            }
            C::Pull if !self.is_repository() => {
                "The open folder is not a git repository.".to_owned()
            }
            C::Push => "Nothing to push — this branch matches its remote.".to_owned(),
            C::Pull => "Nothing to pull — this branch is up to date.".to_owned(),
            C::Commit => "Nothing is staged.".to_owned(),
            C::ClearCursors => "There is only one caret.".to_owned(),
            C::ToggleFold | C::FoldAll => "Nothing in this file folds.".to_owned(),
            C::UnfoldAll => "Nothing is folded.".to_owned(),
            C::Export | C::ExportSelected => {
                if self.root.peek().is_none() {
                    "Export packages a project — open a folder first.".to_owned()
                } else {
                    "An export is already running.".to_owned()
                }
            }
            C::CancelExport => "Nothing is exporting.".to_owned(),
            C::ClearFinishedTasks | C::CancelAllTasks => "Nothing has run yet.".to_owned(),
            C::FindNext | C::FindPrevious => "Type something to find first.".to_owned(),
            // `build_refusal` writes for the Output panel, where there is room
            // for a clause explaining the trade-off. A menu row is 380 points
            // wide and `Text` has no truncation (`HTMLPROTOTYPEMAPPING.md` §7),
            // so what goes here is the first clause, made into a sentence. The
            // full text still reaches the panel when somebody actually builds.
            C::Build | C::BuildRelease | C::BuildAndRun => self.build_refusal().map_or_else(
                || "Build is unavailable.".to_owned(),
                |why| Self::sentence_of(&why),
            ),
            C::CancelBuild => "No build is running.".to_owned(),
            C::ToggleComment | C::Indent | C::Outdent | C::RestoreSample => {
                "There is no buffer open.".to_owned()
            }
            C::ClearOutput => "The Output panel is already empty.".to_owned(),
            C::NewScreen | C::NewWidget => {
                "A template goes into a project — open a folder first.".to_owned()
            }
            C::ToggleAnalyzer => "rust-analyzer needs a project — open a folder first.".to_owned(),
            C::Cut | C::Copy | C::Paste => "There is no buffer open.".to_owned(),
            C::ZoomFit => "The preview already fits the pane.".to_owned(),
            // `can_run` said no and nothing above claims this command, which
            // means the two lists have drifted. Say so rather than showing an
            // empty tooltip that reads as a rendering bug.
            other => format!("{} is unavailable.", other.title()),
        })
    }

    /// Every range-keyed mark the editor should draw over the active buffer.
    ///
    /// Three kinds, in paint order — later ones draw over earlier ones:
    ///
    /// 1. **A squiggle under each diagnostic's span.** Under the text that is
    ///    wrong, not under the line it is on; the line-wide wash stays, because
    ///    the two answer different questions ("where is the problem" against
    ///    "which lines have problems", the second being what you scan for).
    /// 2. **A box around the bracket at the caret and its partner.**
    ///    `edit_ops::match_bracket` has done the matching since N6, correctly
    ///    and with tests, and nothing has ever called it.
    /// 3. **An underline under every other occurrence of the word at the
    ///    caret.** Not the one under the caret: marking what you are already
    ///    looking at tells you nothing, and it is the *others* you want to
    ///    find.
    ///
    /// All of it is derived fresh on every rebuild, which is affordable
    /// precisely because decorations are paint-only — see `TextDecoration`.
    /// Recomputing this cannot re-shape the paragraph.
    #[must_use]
    pub fn editor_decorations(&self, chrome: crate::theme::StudioTheme) -> Vec<TextDecoration> {
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        let text = &buffer.value.text;
        let colors = crate::theme::colors(self.dark.get());
        let mut marks = Vec::new();

        for diagnostic in self.active_diagnostics() {
            if let Some(range) = diagnostic.span_in(text) {
                marks.push(TextDecoration::squiggle(
                    range,
                    match diagnostic.severity {
                        Severity::Error => colors.error,
                        Severity::Warning => chrome.warning,
                    },
                ));
            }
        }

        let caret = buffer.value.selection.cursor().offset;
        if let Some((one, other)) = crate::edit_ops::match_bracket(text, caret) {
            let width = |at: usize| text[at..].chars().next().map_or(1, char::len_utf8);
            marks.push(TextDecoration::boxed(
                TextRange::new(one, one + width(one)),
                colors.outline,
            ));
            marks.push(TextDecoration::boxed(
                TextRange::new(other, other + width(other)),
                colors.outline,
            ));
        }

        for range in occurrences_of_word_at(text, caret) {
            marks.push(
                TextDecoration::squiggle(range, chrome.gutter)
                    .shape(vieww_foundation::TextDecorationShape::Underline)
                    .thickness(1.0),
            );
        }

        if self.indent_guides.get() {
            marks.extend(indent_guides(text, self.tab_width.get(), chrome.line));
        }

        marks
    }

    /// The diagnostics that belong to the buffer currently open.
    #[must_use]
    pub fn active_diagnostics(&self) -> Vec<Diagnostic> {
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        self.active_diagnostics_for(&buffer)
    }

    /// The same, for a buffer already in hand.
    ///
    /// Split out for `gutter_rows_now`, which runs on the *write* side where
    /// `Studio::active` would both re-read the buffer it was just handed and
    /// track a subscription outside any build.
    #[must_use]
    pub fn active_diagnostics_for(&self, buffer: &crate::buffer::Buffer) -> Vec<Diagnostic> {
        let mut out: Vec<Diagnostic> = self
            .diagnostics
            .peek()
            .iter()
            .filter(|d| d.file == buffer.name)
            .cloned()
            .collect();
        // The third producer. `rustc` and `rust-analyzer` are general Rust
        // tools; neither knows what a `Signal` is or why a panic inside a
        // `build` is different from a panic anywhere else. See `crate::lint`.
        out.extend(self.lint_findings(buffer).iter().cloned());
        out.sort_by_key(|d| (d.line, d.column));
        out
    }

    /// The vieww-specific findings for `buffer`, as diagnostics.
    ///
    /// Off by default and behind [`Studio::lint_enabled`], because a lint layer
    /// nobody trusts is worse than none — see the module docs. Rust files only:
    /// running a Rust parser over a `Cargo.toml` to lint it would be the same
    /// mistake the highlighter used to make.
    pub(super) fn lint_findings(&self, buffer: &crate::buffer::Buffer) -> Rc<Vec<Diagnostic>> {
        if !self.lint_enabled.get() || !buffer.language.highlighted() {
            return Rc::new(Vec::new());
        }
        // **Memoised, because this is called several times per frame.**
        //
        // `active_diagnostics` is asked for by the gutter (which lines are
        // flagged), the inline messages, the squiggle decorations and the
        // Problems list — four callers, one build. Each one used to re-walk the
        // whole syntax tree and allocate a fresh `Vec<Finding>`: a megabyte of
        // allocation per keystroke in a profile of the studio, for four
        // identical answers.
        //
        // The key is the same one the highlighter uses, and for the same
        // reason: length first so the common "nothing changed, but the file is
        // large" case is an integer compare, then the full text, because two
        // different files of equal length are not the same file.
        {
            let cached = self.lint_cache.borrow();
            if let Some(pass) = cached.as_ref() {
                if pass.name == buffer.name
                    && pass.text.len() == buffer.value.text.len()
                    && pass.text == buffer.value.text
                {
                    return Rc::clone(&pass.findings);
                }
            }
        }
        // Through the highlighter's own parse rather than a second one of our
        // own: on a large file the lint pass used to double the per-frame
        // parsing cost for a tree that was already sitting in memory.
        let findings = {
            let mut highlighter = self.highlighter.borrow_mut();
            highlighter
                .tree_for(&buffer.value.text, buffer.language)
                .map_or_else(Vec::new, |tree| {
                    crate::lint::check_tree(tree, &buffer.value.text)
                })
        };
        let out: Vec<Diagnostic> = findings
            .into_iter()
            .map(|finding| Diagnostic {
                file: buffer.name.clone(),
                severity: finding.severity,
                // The rule's own name as the code, exactly as `rustc` puts a
                // lint name there — so a finding can be recognised, looked up,
                // and argued with rather than only read.
                code: finding.rule.to_owned(),
                message: format!("vieww-lint: {}", finding.message),
                help: None,
                line: finding.line,
                column: finding.column,
                // A syntax-level rule knows where a node starts and has no
                // useful claim about where the *problem* ends, so the span is
                // the one character the caret would land on. Better than a
                // fabricated range under the wrong tokens.
                end_line: finding.line,
                end_column: finding.column + 1,
            })
            .collect();
        let out = Rc::new(out);
        *self.lint_cache.borrow_mut() = Some(LintPass {
            name: buffer.name.clone(),
            text: buffer.value.text.clone(),
            findings: Rc::clone(&out),
        });
        out
    }

    /// Write the active buffer back to the file it came from.
    ///
    /// Returns what happened, for the caller to put somewhere a person can see
    /// it — a save that failed silently is the worst outcome available here.
    /// Ask where a never-saved buffer should go, then write it there.
    ///
    /// # Why the workspace root and not a file dialog
    ///
    /// There is no platform file dialog in vieww — `crate::picker` is the
    /// studio's own, and it chooses *directories*. So the question is split the
    /// way the rest of the studio splits it: the folder is the one that is
    /// open, and the prompt asks for the name. Somebody who wants it somewhere
    /// else opens that folder.
    ///
    /// With no folder open there is nowhere honest to put it, and the message
    /// says which of the two things to do rather than reporting a failure.
    pub fn save_active_as(&self) {
        if self.active().is_none() {
            self.notify("There is no buffer to save.");
            return;
        }
        let Some(root) = self.root.peek().clone() else {
            self.notify(
                "This buffer has never been saved. Open a folder — File > Open Folder — \
                 and Save will ask what to call it.",
            );
            return;
        };
        self.ask_for_name(NameKind::SaveAs, &root);
    }

    pub fn save_active(&self) -> std::io::Result<()> {
        let index = self.active_buffer.get();
        let mut buffers = (*self.buffers.get()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return Ok(());
        };
        buffer.save()?;
        self.put_buffers(buffers);
        Ok(())
    }

    /// Which text field the keyboard is going to.
    #[must_use]
    pub fn active_field(&self) -> Active {
        Active::current(
            self.palette_open.get(),
            self.find_open.get(),
            self.find_replacing.get(),
        )
    }

    /// Whether `field` should paint a caret this frame.
    ///
    /// Both halves of `show_cursor` in one answer: the field has to be the one
    /// taking the keyboard, **and** the blink has to be in its on phase. Four
    /// fields each painting a motionless caret is what this replaced.
    #[must_use]
    pub fn caret_visible(&self, field: Active) -> bool {
        if self.active_field() != field {
            return false;
        }
        !self.blink_enabled.get() || self.blink.borrow().is_on()
    }

    /// Register the caret's blink with the frame scheduler.
    pub fn attach_blink(&self, tickers: &mut vieww_animation::Tickers) {
        tickers.add(&self.blink);
    }

    /// Publish a new buffer list, and the metadata derived from it.
    ///
    /// **The one door.** [`buffers`](Self::buffers) and [`tabs`](Self::tabs)
    /// describe the same files, and two signals written at twenty-one separate
    /// call sites are two signals that disagree exactly once — on whichever
    /// site somebody adds next and forgets. So nothing writes `buffers`
    /// directly; everything comes here, and the derivation happens in one
    /// place where it can be read.
    ///
    /// `tabs` goes out with
    /// [`set_if_changed`](vieww_element::Signal::set_if_changed), which is the
    /// whole point: typing changes text and nothing else, so the comparison
    /// fails to find a difference and the tab strip, the explorer's open-buffer
    /// list, the Save button and the status bar are never told. Opening,
    /// closing, renaming or saving a file *does* change it, and those are rare.
    ///
    /// The comparison walks a handful of short names and scalars — the open
    /// files, not their contents — so it costs far less than the one rebuild it
    /// avoids, let alone the hundreds.
    /// Publish a new buffer list.
    ///
    /// Everything derived from it — [`tabs`](Self::tabs) and
    /// [`gutter`](Self::gutter) — is a [`Memo`] and re-derives itself, so this
    /// is a plain write with nothing to remember.
    ///
    /// **It used to be a door, and four more beside it.** Two derived signals
    /// had to be recomputed at every one of twenty-one buffer writes, plus
    /// three further doors for the caret, the folds and the diagnostics, and a
    /// test whose only job was to catch the site somebody forgot. That is
    /// exactly the pattern `Runtime::memo` exists to delete, and
    /// `docs/PERFORMANCE.md` named it as a framework-level fix waiting to
    /// happen. It happened; this is what is left of it.
    ///
    /// Kept as a named function rather than inlined because "publish the
    /// buffers" is worth one place to put a breakpoint on.
    pub(super) fn put_buffers(&self, buffers: Vec<Buffer>) {
        self.buffers.set(Rc::new(buffers));
    }

    /// The inputs [`gutter`](Self::gutter) derives from, as handles.
    pub(super) fn gutter_source(&self) -> GutterSource {
        GutterSource {
            buffers: self.buffers.clone(),
            active_buffer: self.active_buffer.clone(),
            folds: self.folds.clone(),
            diagnostics: self.diagnostics.clone(),
            lint_enabled: self.lint_enabled.clone(),
            lint_cache: Rc::clone(&self.lint_cache),
            fold_cache: Rc::clone(&self.fold_cache),
        }
    }

    /// The gutter's rows as they are right now, derived from scratch and
    /// **untracked**.
    ///
    /// The same computation [`gutter`](Self::gutter) memoises.
    /// `the_gutter_matches_a_fresh_derivation` asserts the two agree — a memo
    /// is only as correct as its definition, and this is the oracle for that
    /// definition.
    #[must_use]
    pub fn gutter_rows_now(&self) -> Vec<GutterRow> {
        self.gutter_source().rows(Tracking::Off)
    }

    /// Mark the buffer edited. Every edit, and every platform switch, goes
    /// through here — the plan treats them identically (§4.6).
    pub fn mark_dirty(&self) {
        // `set_if_changed`, not `set`. After the first keystroke of a session
        // this flag is already `true`, and a plain `set` still notified every
        // subscriber — which is the title bar, the status bar, the preview pane
        // and the Render button. Four regions rebuilt per character typed, to
        // produce exactly the tree they already had.
        self.dirty.set_if_changed(true);
    }

    // ================= commands =========================================

    /// Perform `command`.
    ///
    /// **The one door.** A menu item, a palette row, a keystroke and a toolbar
    /// button all arrive here, so there is exactly one description of what
    /// "Save" means and no way for the four of them to drift. See
    /// [`crate::command`].
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per command is the point: \
        a dispatcher that groups arms to be shorter is a dispatcher where a \
        missing command is invisible"
    )]
    pub fn run(&self, command: Command) {
        use Command as C;

        // Any command dismisses an open menu. Without this the strip stays
        // pulled down over the thing the command just changed.
        if self.menu_open.peek().is_some() {
            self.menu_open.set(None);
        }
        // And clears the last notice, which was about the *previous* thing the
        // user did. Leaving it up next to the result of this command is how a
        // status bar starts lying.
        if self.notice.peek().is_some() {
            self.notice.set(None);
        }

        match command {
            C::Render => {
                self.palette_open.set(false);
                self.render();
            }
            C::CancelRender => self.cancel_render(),
            C::PlatformIos => self.set_platform(Platform::Ios),
            C::PlatformAndroid => self.set_platform(Platform::Android),
            C::PlatformDesktop => self.set_platform(Platform::Desktop),
            C::ToggleSafeArea => self.show_insets.set(!self.show_insets.peek()),
            C::TogglePreviewDark => {
                let next = !self.preview_dark.peek();
                self.preview_dark.set(next);
                self.notify(if next {
                    "Preview: dark"
                } else {
                    "Preview: light"
                });
            }
            C::LivePreview => {
                // **No file, no preview.** The Live Preview draws `live.rs` at
                // the workspace root; without one there is nothing live about
                // it, and the studio used to answer this by mounting a built-in
                // demo of somebody else's app. Now it says which file is
                // missing and offers to write one — see `LiveMissing`.
                if self.has_live_file() {
                    // The caution first: a preview that draws a sketch has to
                    // say it is a sketch before it is mistaken for the build.
                    self.live_caution.set(true);
                } else {
                    self.live_missing.set(true);
                }
            }
            C::CreateLiveFile => self.create_live_file(),
            C::RestoreFlow => self.restore_flow(),

            C::TrySayScreen => self.try_say_screen(),
            C::NewProject => self.new_project(),
            C::OpenFolder => self.open_picker(),
            C::RestoreSample => self.restore_sample(),
            C::NewFile => self.new_file(),
            C::CollapseFolders => self.collapse_folders(),
            C::NewScreen => self.new_from_template(crate::scaffold::Template::Screen),
            C::NewWidget => self.new_from_template(crate::scaffold::Template::Widget),
            // Format first, then save. `format_active` is asynchronous —
            // `rustfmt` is a child process — so this cannot be "format, wait,
            // save" without blocking the frame. What it is instead is stated
            // plainly: the file is written now, and the formatter's result
            // lands in the buffer a moment later, leaving it dirty again.
            // Saving twice is the honest shape of an out-of-process formatter
            // and is what the status line says happened.
            C::Save => {
                // **A buffer with no path is asked where to go, not refused.**
                // This is the whole of the bug: Save was gated on the buffer
                // already having a file, so a new file could be typed into,
                // filled from a snippet, and never written — the command that
                // exists to not lose work was the one command it could not do.
                if self.active().is_some_and(|b| b.path.is_none()) {
                    self.save_active_as();
                } else {
                    if self.format_on_save.peek()
                        && self.active().is_some_and(|b| b.name.ends_with(".rs"))
                    {
                        self.format_active();
                    }
                    self.report(self.save_active(), "saved");
                }
            }
            C::SaveAll => self.save_all(),
            C::RevertFile => self.revert_active(),
            C::CloseTab => self.close_active(),

            C::Undo => self.undo(),
            C::Redo => self.redo(),
            C::ToggleComment => self.toggle_comment(),
            C::Indent => self.shift_indent(true),
            C::Outdent => self.shift_indent(false),
            C::Find => self.open_find(false),
            C::Replace => self.open_find(true),
            C::FindNext => self.find_step(1),
            C::FindPrevious => self.find_step(-1),
            C::CloseFind => self.escape(),

            C::CommandPalette => {
                let open = !self.palette_open.peek();
                self.palette_open.set(open);
                if open {
                    // Opened with the command prefix already in it. The palette
                    // has four modes now (`PaletteMode`), and this command's
                    // name says which one it means — a ⌘K that opened on files
                    // would be a command palette that does not list commands.
                    self.palette_query.set(">".to_string());
                    self.palette_index.set(0);
                }
            }
            C::GoToFile => {
                self.palette_open.set(true);
                self.palette_query.set(String::new());
                self.palette_index.set(0);
            }
            C::GoToSymbol => {
                self.palette_open.set(true);
                self.palette_query.set("@".to_string());
                self.palette_index.set(0);
            }
            C::GoToLine => {
                self.palette_open.set(true);
                self.palette_query.set(":".to_string());
                self.palette_index.set(0);
            }
            C::TogglePanel => self.panel_open.set(!self.panel_open.peek()),
            C::ToggleSidebar => {
                // Collapsed to zero rather than removed from the tree, so the
                // divider keeps the pane's place and one press brings it back
                // at the open width.
                let width = self.sidebar_width.peek();
                self.sidebar_width
                    .set(if width > 0.0 { 0.0 } else { OPEN_SIDEBAR });
            }
            C::ToggleTheme => self.dark.set(!self.dark.peek()),
            // Handed straight to the focused object rather than carried out
            // here. `RenderEditableText` implements all three against the
            // `Clipboard` service and is the only thing that knows what is
            // selected; a second implementation in the studio would be a second
            // answer to "what does ⌘C copy".
            C::Cut | C::Copy | C::Paste => self.clipboard_command(command),
            C::ToggleRightPane => self.right_open.set(!self.right_open.peek()),
            C::ToggleActivityBar => {
                self.activity_bar_open.set(!self.activity_bar_open.peek());
                self.notify(if self.activity_bar_open.peek() {
                    "Activity bar shown."
                } else {
                    "Activity bar hidden — Ctrl+Alt+U brings it back."
                });
            }
            C::ShowInspector => {
                self.right_tab.set(RightTab::Inspector);
                self.right_open.set(true);
            }
            C::ZenMode => {
                // A toggle, not a one-way door. Zen that cannot be left is a
                // window with no sidebar and no way to ask for one.
                //
                // The sidebar collapses to a width rather than leaving the
                // tree, for the reason `ToggleSidebar` gives, so "is it shut"
                // is a width question there and a bool everywhere else.
                let showing = self.sidebar_width.peek() > 0.0
                    || self.panel_open.peek()
                    || self.right_open.peek();
                self.sidebar_width
                    .set(if showing { 0.0 } else { OPEN_SIDEBAR });
                self.panel_open.set(!showing);
                self.right_open.set(!showing);
                self.notify(if showing {
                    "Zen mode \u{2014} run it again to bring the panes back."
                } else {
                    "Panes restored."
                });
            }
            C::ZoomIn => self.zoom_by(1.25),
            C::ZoomOut => self.zoom_by(0.8),
            C::ToggleDamageOverlay => {
                let on = !self.show_damage.peek();
                self.show_damage.set(on);
                self.notify(if on {
                    "Repainted regions shown. Every box is a rectangle the compositor was asked for."
                } else {
                    "Repainted regions hidden."
                });
            }
            C::ToggleSemanticsOverlay => {
                let on = !self.show_semantics.peek();
                self.show_semantics.set(on);
                self.notify(if on {
                    "Semantics tree shown \u{2014} role and label, as a screen reader gets them."
                } else {
                    "Semantics tree hidden."
                });
            }
            C::PickWidget => {
                // The Inspector has to be capturing for a pick to land on a
                // row, so turning it on is part of turning pick on.
                self.right_tab.set(RightTab::Inspector);
                self.right_open.set(true);
                self.pick_mode.set(true);
                self.notify("Click anything in the window to select it in the Inspector.");
            }
            C::ZoomFit => {
                self.preview_zoom.set(None);
                self.notify("Preview fitted to the pane.");
            }
            C::AddCursorBelow => self.add_caret_line(true),
            C::AddCursorAbove => self.add_caret_line(false),
            C::AddCursorAtNextOccurrence => self.add_caret_at_next_occurrence(),
            C::ClearCursors => {
                self.clear_extra_carets();
            }
            C::ToggleFold => self.toggle_fold(),
            C::FoldAll => self.fold_all(true),
            C::UnfoldAll => self.fold_all(false),
            C::Format => self.format_active(),
            C::Export => self.view.set(View::Export),
            C::ExportSelected => self.start_export(self.export_format.peek()),
            C::CancelExport => self.cancel_export(),
            C::ScanDevices => self.scan_devices(),
            C::ToggleFormatOnSave => self.format_on_save.set(!self.format_on_save.peek()),
            C::ShowTasks => {
                self.panel_tab.set(PanelTab::Tasks);
                self.panel_open.set(true);
            }
            C::ClearFinishedTasks => self.clear_finished_jobs(),
            C::CancelAllTasks => {
                self.jobs.borrow_mut().cancel_all();
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
            }
            C::ToggleIndentGuides => self.indent_guides.set(!self.indent_guides.peek()),
            C::ToggleInlineDiagnostics => {
                self.inline_diagnostics.set(!self.inline_diagnostics.peek());
            }
            C::NextTab => self.step_tab(1),
            C::PreviousTab => self.step_tab(-1),
            C::ShowProblems => {
                self.view.set(View::Problems);
                self.panel_tab.set(PanelTab::Problems);
                self.panel_open.set(true);
            }
            C::ShowExplorer => self.view.set(View::Explorer),
            C::ShowSearch => self.view.set(View::Search),
            C::ShowToolchain => self.view.set(View::Toolchain),
            C::ShowTokens => self.view.set(View::Tokens),
            C::ShowRun => {
                self.panel_open.set(true);
                self.panel_tab.set(PanelTab::Run);
            }
            C::Complete => self.request_completion(),
            C::Hover => self.request_hover(),
            C::GotoDefinition => self.goto_definition(),
            C::FindReferences => self.find_references(),
            C::ReplaceInFiles => self.plan_workspace_replace(),
            C::WriteTemplates => self.write_customisation_templates(),
            C::CloseOtherTabs => self.close_tabs(TabScope::Others),
            C::CloseTabsToTheRight => self.close_tabs(TabScope::ToTheRight),
            C::CloseSavedTabs => self.close_tabs(TabScope::Saved),
            C::ToggleAnalyzer => self.toggle_analyzer(),
            C::RestartAnalyzer => self.restart_analyzer(),
            C::ToggleWordWrap => {
                let wrapping = !self.word_wrap.get();
                self.word_wrap.set(wrapping);
                self.notify(if wrapping {
                    "Word wrap on"
                } else {
                    "Word wrap off"
                });
            }
            C::About => self.about.set(true),
            C::ShowWelcome => self.welcome.set(true),
            C::TogglePreviewNote => {
                let on = !self.preview_note.peek();
                self.preview_note.set(on);
                self.notify(if on {
                    "The preview note is back under the device frame."
                } else {
                    "Preview note hidden. Bring it back from this palette."
                });
            }
            C::ToggleAutoRender => {
                let on = !self.auto_render.peek();
                self.auto_render.set(on);
                if !on {
                    *self.render_due.borrow_mut() = None;
                }
                self.notify(if on {
                    "Auto Render on \u{2014} the buffer renders 0.6s after you stop typing."
                } else {
                    "Auto Render off."
                });
            }
            C::ShowSource => {
                self.view.set(View::Source);
                // Scanned on the way in rather than on a timer: it is a
                // process launch, and the moment somebody opens this view is
                // exactly when the answer needs to be fresh.
                self.refresh_git();
            }
            C::RefreshGit => self.refresh_git(),
            C::Push => self.push(),
            C::Pull => self.pull(),
            C::Fetch => self.fetch(),
            C::Commit => self.commit(),
            C::ShowSettings => self.view.set(View::Settings),
            C::Build => self.start_build(Kind::Build),
            C::BuildRelease => {
                self.profile.set(Profile::Release);
                self.start_build(Kind::Build);
            }
            C::BuildAndRun => self.start_build(Kind::BuildAndRun),
            C::CancelBuild => self.cancel_build(),
            C::ClearOutput => self.clear_output(),
            // **Help → Keyboard Shortcuts, and it has to show shortcuts.**
            //
            // It opened the palette with an *empty* query, and an empty query
            // is `PaletteMode::Files` — so the menu item called "Keyboard
            // Shortcuts" produced a list of `AndroidManifest.xml`,
            // `build.gradle`, `gradle.properties`. Not a shortcut anywhere on
            // screen. The only menu item in the studio whose result has nothing
            // to do with its name.
            //
            // `>` is the command mode, and the command rows already carry every
            // chord and the menu it lives in — which is a keyboard reference,
            // filterable, and one that cannot drift from the bindings because
            // it *is* the bindings. So this is one character, not a new sheet.
            C::ShowShortcuts => {
                self.palette_open.set(true);
                self.palette_query.set(">".to_owned());
                self.palette_index.set(0);
            }
        }

        // The palette closes behind whatever was chosen from it, except for
        // the command that *is* the palette.
        if !matches!(command, C::CommandPalette | C::ShowShortcuts) {
            self.palette_open.set(false);
        }
    }

    /// Whether `command` would do anything right now.
    ///
    /// What greys a menu item out. Answered from the same state the command
    /// itself reads, so a disabled item and a no-op cannot disagree.
    #[must_use]
    pub fn can_run(&self, command: Command) -> bool {
        use Command as C;
        match command {
            C::Render => !self.is_compiling() && self.toolchain.is_ok(),
            C::CancelRender => self.is_compiling(),
            C::Undo => self.active().is_some_and(|b| b.history.can_undo()),
            C::Redo => self.active().is_some_and(|b| b.history.can_redo()),
            // Dirty is the whole condition. Whether it has a path decides
            // *how* it is saved — straight to disk, or through the name prompt
            // — and not whether it can be.
            C::Save => self.active().is_some_and(|b| b.dirty),
            C::SaveAll => self
                .buffers
                .get()
                .iter()
                .any(|b| b.dirty && b.path.is_some()),
            C::RevertFile => self.active().is_some_and(|b| b.path.is_some()),
            C::CloseTab => !self.buffers.get().is_empty(),
            C::Format => self.active().is_some_and(|b| b.name.ends_with(".rs")),
            C::ShowSource | C::RefreshGit | C::Fetch => self.is_repository(),
            // A push with nothing ahead is a no-op that prints "Everything
            // up-to-date"; a pull with nothing behind is the same. Both are
            // greyed rather than run, so the button says what the repository
            // is rather than only what git can be asked.
            C::Push => self.is_repository() && self.git.get().ahead > 0,
            C::Pull => self.is_repository() && self.git.get().behind > 0,
            C::Commit => self.is_repository() && self.git.get().staged() > 0,
            C::ClearCursors => self.caret_count() > 1,
            C::ToggleFold | C::FoldAll => !self.foldable().is_empty(),
            C::UnfoldAll => !self.folds.get().is_empty(),
            C::Export | C::ExportSelected => self.root.get().is_some() && !self.exporting(),
            C::CancelExport => self.exporting(),
            C::ClearFinishedTasks | C::CancelAllTasks => !self.jobs.borrow().jobs().is_empty(),
            C::FindNext | C::FindPrevious => !self.find_query.get().is_empty(),
            C::CloseFind => self.find_open.get() || self.palette_open.get(),
            C::NextTab | C::PreviousTab => self.buffers.get().len() > 1,
            // Answered from the same check the command itself runs, so a greyed
            // item and a refusal can never disagree — and so hovering the item
            // is how you find out *why*, through `build_refusal`.
            C::Build | C::BuildRelease | C::BuildAndRun => self.build_refusal().is_none(),
            C::CancelBuild => self.builds.borrow().busy(),
            C::ToggleComment | C::Indent | C::Outdent | C::RestoreSample => self.active().is_some(),
            C::NewScreen | C::NewWidget => self.root.peek().is_some(),
            // Stoppable whenever it is running; startable only with a project.
            C::ToggleAnalyzer => self.analyzer.borrow().is_some() || self.root.peek().is_some(),
            C::Cut | C::Copy | C::Paste => self.active().is_some(),
            C::ZoomFit => self.preview_zoom.peek().is_some(),
            C::ClearOutput => !self.output.get().is_empty(),
            _ => true,
        }
    }

    /// Escape: shut whatever is open, innermost first.
    ///
    /// Ordered rather than closing everything at once, because a palette
    /// opened over a find bar should leave the find bar behind — one press,
    /// one thing dismissed, which is the only behaviour that is predictable
    /// when several are open.
    pub fn escape(&self) {
        // Extra carets first, and before the find bar: with several carets
        // placed, Escape is overwhelmingly "stop doing that", and having to
        // press it twice because a find bar was also open is the kind of
        // ordering people notice once and remember badly.
        //
        // The dialogs added since go **above** everything, in the order a
        // person would expect to dismiss them: the thing most recently put in
        // front of you is the thing Escape takes away. The quit prompt is
        // deliberately *not* here — it is a question about losing work, and
        // dismissing it with a keystroke is exactly the accident it exists to
        // prevent. Same reasoning as the scrim in `ui::dialog`.
        // Above everything: a completion list is the most recently opened
        // thing on screen whenever it is open, and Escape means "not that".
        if self.completion.peek().is_some() {
            self.close_completion();
        } else if self.about.peek() {
            self.about.set(false);
        } else if self.welcome.peek() {
            self.welcome.set(false);
        } else if self.name_prompt.peek().is_some() {
            self.close_name_prompt();
        } else if self.pending_delete.peek().is_some() {
            self.cancel_delete();
        } else if self.context_menu.peek().is_some() {
            self.close_context_menu();
        } else if self.caret_count() > 1 {
            self.clear_extra_carets();
        } else if self.picker.peek().is_some() {
            // Above the menu strip: the picker is a modal over everything, and
            // Escape with one open means "not that folder" every time.
            self.close_picker();
        } else if self.menu_open.peek().is_some() {
            self.menu_open.set(None);
        } else if self.palette_open.peek() {
            self.palette_open.set(false);
        } else if self.find_open.peek() {
            self.find_open.set(false);
            self.find_replacing.set(false);
        }
    }

    /// Switch the simulated platform. An edit, per the plan's §4.7.
    pub fn set_platform(&self, platform: Platform) {
        if self.platform.peek() != platform {
            self.platform.set(platform);
            self.mark_dirty();
        }
    }
}
