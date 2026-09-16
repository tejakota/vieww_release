//! Files, the tree and workspace-wide replace.
//!
//! The Explorer's operations, and the two things that walk the whole workspace
//! rather than one buffer.
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
    // ----- The Explorer's context menu, and the operations behind it -----
    //
    // `file_tree::rename` and `delete_to_trash` were written, tested, and had
    // **zero callers**. A user could not rename or delete a file from inside
    // the editor at all; they had to leave for a file manager. Everything in
    // this section is the wire that was missing.

    /// Open the context menu at `at`, about `path`.
    pub fn open_context_menu(&self, at: vieww_foundation::Offset, path: std::path::PathBuf) {
        let is_dir = path.is_dir();
        self.context_menu
            .set(Some(ContextMenu { at, path, is_dir }));
    }

    pub fn close_context_menu(&self) {
        self.context_menu.set(None);
    }

    /// Put `text` on the pasteboard.
    pub fn copy_text(&self, text: &str) {
        use vieww_foundation::Clipboard;
        let Some(clipboard) = self.services.get::<dyn Clipboard>() else {
            self.notify("This window has no pasteboard.");
            return;
        };
        match clipboard.write_text(text) {
            Ok(()) => self.notify("Copied"),
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Put a path on the pasteboard.
    pub fn copy_path(&self, path: &std::path::Path) {
        use vieww_foundation::Clipboard;
        let Some(clipboard) = self.services.get::<dyn Clipboard>() else {
            self.notify("This window has no pasteboard.");
            return;
        };
        let text = path.display().to_string();
        match clipboard.write_text(&text) {
            Ok(()) => self.notify("Path copied"),
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Record the window size, from the frame hook. A no-op on the frames
    /// where it has not changed, which is nearly all of them.
    ///
    /// **Panes are clamped to the window they are in.** The dividers clamp
    /// their own drags, but a window shrinking under a pane that is already
    /// wider than the space left for it used to leave the layout to divide a
    /// negative number: the editor collapsed to nothing while the preview
    /// kept a width the window could no longer afford, and no amount of
    /// dragging the seam put it right, because the seam's own clamp only runs
    /// while the pointer is moving it. The same rule the divider enforces is
    /// applied here, so a shrunken window and a dragged seam cannot disagree.
    pub fn note_window_size(&self, size: vieww_foundation::Size) {
        if self.window_size.peek() != size {
            self.window_size.set(size);

            // The preview's cap, exactly as `MainColumn` passes it to the
            // divider: 70% of the window, and never below a bound the
            // `MIN_PANE` floor could not clear.
            let max_preview = (size.width * 0.7).max(MIN_PANE * 2.0);
            let preview = self.preview_width.peek().clamp(MIN_PANE, max_preview);
            if (preview - self.preview_width.peek()).abs() > f32::EPSILON {
                self.preview_width.set(preview);
            }

            // The panel's cap, from the same place `MainColumn` gets it. A
            // panel taller than two thirds of a window that shrank holds the
            // editor to a sliver no drag can recover, for the same reason as
            // above.
            let max_panel_h = max_panel(size.height);
            let panel = self.panel_height.peek().clamp(MIN_PANEL, max_panel_h);
            if (panel - self.panel_height.peek()).abs() > f32::EPSILON {
                self.panel_height.set(panel);
            }
        }
    }

    /// The directory a new item created from `path`'s menu belongs in.
    ///
    /// Inside a folder, and *beside* a file. Getting this backwards is the
    /// single most annoying thing a file tree can do.
    #[must_use]
    pub(super) fn container_of(path: &std::path::Path) -> std::path::PathBuf {
        if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent()
                .map_or_else(|| path.to_path_buf(), std::path::Path::to_path_buf)
        }
    }

    /// Ask for a name. Closes the context menu, because the prompt replaces it.
    pub fn ask_for_name(&self, kind: NameKind, path: &std::path::Path) {
        self.close_context_menu();
        let (target, text) = match kind {
            NameKind::Rename => (
                path.to_path_buf(),
                path.file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            ),
            // Pre-filled with nothing rather than with "untitled.rs": a
            // pre-filled name is one people accept without reading, and a
            // project full of `untitled.rs` is what that produces.
            //
            // For `NewProject`, `path` is the *parent directory* the studio
            // chose — so `target` is `path` itself, not its container, and
            // the new project lands directly inside it. The default name is
            // empty for the same reason as `NewFile`: a pre-filled
            // "untitled-app" is a name the user accepts without reading.
            NameKind::NewFile | NameKind::NewFolder => (Self::container_of(path), String::new()),
            NameKind::NewProject => (path.to_path_buf(), String::new()),
            // Pre-filled, unlike the two above, and for the opposite reason:
            // this buffer already has a name — `untitled-2.rs`, or whatever a
            // template gave it — and the user is confirming or editing it
            // rather than inventing one. An empty box here would make them
            // retype something they can already see in the tab.
            NameKind::SaveAs => (
                path.to_path_buf(),
                self.active()
                    .map_or_else(String::new, |buffer| buffer.name.clone()),
            ),
        };
        self.name_prompt.set(Some(NamePrompt {
            kind,
            target,
            text,
            error: None,
            project_kind: crate::scaffold::ProjectKind::default(),
        }));
    }

    /// Choose what a new project's screens are written in. Only the New
    /// Project prompt reads it; the chips in the dialog are the UI.
    pub fn set_prompt_kind(&self, kind: crate::scaffold::ProjectKind) {
        let Some(prompt) = self.name_prompt.get() else {
            return;
        };
        self.name_prompt.set(Some(NamePrompt {
            project_kind: kind,
            ..prompt
        }));
    }

    /// Type into the name prompt. Validates as it goes, so the button can be
    /// refused with a reason rather than failing after the click.
    pub fn set_prompt_name(&self, text: String) {
        let Some(prompt) = self.name_prompt.get() else {
            return;
        };
        // The validation rules differ by kind: a file name may not contain a
        // path separator, a crate name may not be a Rust keyword or start
        // with a digit. Asking the right one here is what keeps the button
        // greyed for a real reason rather than for a rule that does not apply.
        let error = match prompt.kind {
            NameKind::NewProject => match crate::scaffold::check_name(text.trim()) {
                Ok(()) => None,
                Err(error) => Some(error.to_string()),
            },
            NameKind::NewFile | NameKind::NewFolder | NameKind::Rename | NameKind::SaveAs => {
                Self::name_error(&text)
            }
        };
        self.name_prompt.set(Some(NamePrompt {
            text,
            error,
            ..prompt
        }));
    }

    pub fn close_name_prompt(&self) {
        self.name_prompt.set(None);
    }

    /// Why `name` is not usable as a file name, if it is not.
    ///
    /// The separator check is the important one: a "name" containing a `/` is
    /// a path, and letting one through turns "rename this file" into "move it
    /// somewhere the user did not look at", including out of the workspace.
    #[must_use]
    pub fn name_error(name: &str) -> Option<String> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Some("A name is needed.".to_owned());
        }
        if trimmed == "." || trimmed == ".." {
            return Some("That is a directory, not a name.".to_owned());
        }
        if trimmed.contains('/') || trimmed.contains('\\') {
            return Some("A name cannot contain a path separator.".to_owned());
        }
        if trimmed.contains('\0') {
            return Some("A name cannot contain a null byte.".to_owned());
        }
        None
    }

    /// Carry out whatever the name prompt was for.
    pub fn confirm_name_prompt(&self) {
        let Some(prompt) = self.name_prompt.get() else {
            return;
        };
        let name = prompt.text.trim().to_owned();
        if let Some(error) = Self::name_error(&name) {
            self.name_prompt.set(Some(NamePrompt {
                error: Some(error),
                ..prompt
            }));
            return;
        }
        // `NewProject` validates against Cargo's crate-name rules rather
        // than the file-name rules above, so re-check here. `set_prompt_name`
        // already does this on every keystroke, but a name pasted in via the
        // platform's clipboard (which bypasses the field's `on_changed`) would
        // otherwise reach `scaffold_named_project` unchecked. The scaffold
        // itself calls `check_name` too, so this is the second of three lines
        // of defence — same rule, asked three times, because a project that
        // fails to scaffold after the dialog has already said "looks good"
        // is the worst of the three outcomes.
        if matches!(prompt.kind, NameKind::NewProject) {
            if let Err(error) = crate::scaffold::check_name(&name) {
                self.name_prompt.set(Some(NamePrompt {
                    error: Some(error.to_string()),
                    ..prompt
                }));
                return;
            }
        }

        let outcome = match prompt.kind {
            NameKind::NewFile => {
                let path = prompt.target.join(&name);
                if path.exists() {
                    Err(format!("{name} is already there"))
                } else {
                    crate::file_tree::create_file(&path)
                        .map(|()| {
                            self.open_path(path);
                            format!("Created {name}")
                        })
                        .map_err(|error| error.to_string())
                }
            }
            NameKind::NewFolder => {
                let path = prompt.target.join(&name);
                if path.exists() {
                    Err(format!("{name} is already there"))
                } else {
                    crate::file_tree::create_dir(&path)
                        .map(|()| format!("Created {name}/"))
                        .map_err(|error| error.to_string())
                }
            }
            NameKind::Rename => self.rename_path(&prompt.target, &name),
            // The crate-name rules were already enforced at type-time in
            // `set_prompt_name`, so the only remaining failures are about the
            // disk: the directory already exists, or it could not be written.
            // `scaffold_named_project` reports either as a sentence.
            NameKind::NewProject => {
                self.scaffold_named_project(&prompt.target, &name, prompt.project_kind)
            }
            NameKind::SaveAs => self.write_active_as(&prompt.target.join(&name)),
        };

        match outcome {
            Ok(message) => {
                self.name_prompt.set(None);
                self.rescan_tree();
                self.notify(&message);
            }
            Err(error) => self.name_prompt.set(Some(NamePrompt {
                error: Some(error),
                ..prompt
            })),
        }
    }

    /// Give the active buffer a place on disk and write it there.
    ///
    /// The second half of Save for a buffer that has never had a path. The
    /// buffer keeps its history, its language and its caret — it is the same
    /// buffer, now with somewhere to live — so this deliberately does not close
    /// and reopen the file, which would lose the undo stack the user has been
    /// building since they typed the first character.
    pub(super) fn write_active_as(&self, path: &std::path::Path) -> Result<String, String> {
        if path.exists() {
            // Refused rather than confirmed-over. A studio that overwrites on a
            // name collision is one that eats a file the user forgot was there,
            // and the prompt is still open behind this message, so the cost of
            // refusing is one keystroke.
            return Err(format!(
                "{} is already there — pick another name",
                path.file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
            ));
        }

        let index = self.active_buffer.peek();
        let mut buffers = (*self.buffers.peek()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return Err("There is no buffer to save.".to_owned());
        };

        buffer.path = Some(path.to_path_buf());
        buffer.name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        // The language follows the extension: a scratch buffer saved as `.rs`
        // is a Rust file from that moment, and the highlighter, the formatter
        // and Render all read this rather than the name.
        buffer.language = crate::language::Language::of_path(path);

        match buffer.save() {
            Ok(()) => {
                let name = buffer.name.clone();
                self.put_buffers(buffers);
                Ok(format!("Saved {name}"))
            }
            Err(error) => {
                // The path is *not* kept on a failed write: a buffer that
                // remembers a location it could not be written to would report
                // itself as a file on disk and fail again silently on the next
                // ⌘S.
                Err(error.to_string())
            }
        }
    }

    /// Rename `from` to `name` in the same directory, following any open
    /// buffer to the new path.
    ///
    /// **Following the buffer is the whole point.** A rename that leaves the
    /// open tab pointing at a path that no longer exists is a rename that
    /// turns the next save into a file the user did not ask for, under the old
    /// name, beside the new one.
    pub(super) fn rename_path(&self, from: &std::path::Path, name: &str) -> Result<String, String> {
        let Some(parent) = from.parent() else {
            return Err("That has nowhere to be renamed within.".to_owned());
        };
        let to = parent.join(name);
        if to == from {
            return Ok("Unchanged".to_owned());
        }
        if to.exists() {
            return Err(format!("{name} is already there"));
        }
        crate::file_tree::rename(from, &to).map_err(|error| error.to_string())?;

        let mut buffers = (*self.buffers.get()).clone();
        let mut moved = false;
        for buffer in &mut buffers {
            if buffer.path.as_deref() == Some(from) {
                buffer.path = Some(to.clone());
                buffer.name = name.to_owned();
                buffer.language = crate::language::Language::of_path(&to);
                moved = true;
            }
        }
        if moved {
            self.put_buffers(buffers);
        }
        Ok(format!("Renamed to {name}"))
    }

    /// Ask whether to delete `path`.
    pub fn ask_to_delete(&self, path: std::path::PathBuf) {
        self.close_context_menu();
        self.pending_delete.set(Some(path));
    }

    pub fn cancel_delete(&self) {
        self.pending_delete.set(None);
    }

    /// Move the pending path to the workspace trash.
    ///
    /// To trash, not `unlink` — §5.4's rule, and the reason a confirmation is
    /// survivable rather than final. The trash directory is skipped by the tree
    /// scan, so a deleted file leaves the Explorer and stays on disk.
    pub fn confirm_delete(&self) {
        let Some(path) = self.pending_delete.get() else {
            return;
        };
        self.pending_delete.set(None);
        let Some(root) = self.root.peek() else {
            self.notify("A scratch workspace has no trash to move it to");
            return;
        };
        match crate::file_tree::delete_to_trash(&root, &path) {
            Ok(_) => {
                // Any buffer over the deleted file is closed: leaving it open
                // means the next save recreates the file the user just deleted.
                let buffers: Vec<crate::buffer::Buffer> = (*self.buffers.get())
                    .iter()
                    .filter(|buffer| buffer.path.as_deref() != Some(path.as_path()))
                    .cloned()
                    .collect();
                let buffers = if buffers.is_empty() {
                    vec![crate::buffer::Buffer::scratch(
                        "scratch.rs",
                        crate::buffer::SCRATCH,
                    )]
                } else {
                    buffers
                };
                let active = self.active_buffer.get().min(buffers.len() - 1);
                self.put_buffers(buffers);
                self.focus_buffer(active);
                self.sync_caret();
                self.rescan_tree();
                self.notify(&format!(
                    "Moved {} to the workspace trash",
                    path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned()
                    )
                ));
            }
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Whether `path` is the workspace root itself.
    ///
    /// The trash lives inside the workspace, so the root cannot go into it —
    /// see `file_tree::delete_to_trash`. The Explorer uses this to leave "Move
    /// to Trash" off the root's menu, and the confirmation uses it to explain
    /// rather than to offer an action that ends in `EINVAL`.
    #[must_use]
    pub fn is_workspace_root(&self, path: &std::path::Path) -> bool {
        self.root.peek().as_deref() == Some(path)
    }

    /// The name the delete confirmation is about.
    #[must_use]
    pub fn pending_delete_name(&self) -> Option<String> {
        self.pending_delete.get().map(|path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
    }

    /// Close a group of tabs at once.
    ///
    /// # The rule that makes this safe
    ///
    /// **A tab with unsaved edits is never closed by a bulk close.** Not by
    /// "close others", not by "close to the right". A bulk action is one click
    /// away from an accident, and the accident it would otherwise cause is
    /// losing work that was never named in a dialog. Anything kept back is
    /// counted and said, so "Closed 6 tabs, kept 2 with unsaved changes" is the
    /// worst outcome rather than a silent loss.
    ///
    /// `CloseSavedTabs` is the same rule stated as a feature: it is the one a
    /// user reaches for *because* it leaves the dirty ones alone.
    pub fn close_tabs(&self, scope: TabScope) {
        let buffers = self.buffers.get();
        let active = self.active_buffer.get();
        let mut kept: Vec<crate::buffer::Buffer> = Vec::new();
        let mut new_active = 0;
        let mut closed = 0;
        let mut spared = 0;

        for (index, buffer) in buffers.iter().enumerate() {
            let in_scope = match scope {
                TabScope::Others => index != active,
                TabScope::ToTheRight => index > active,
                TabScope::Saved => true,
            };
            if in_scope && buffer.dirty {
                spared += 1;
            }
            if in_scope && !buffer.dirty {
                closed += 1;
                continue;
            }
            if index == active {
                new_active = kept.len();
            }
            kept.push(buffer.clone());
        }

        if closed == 0 {
            self.notify(if spared == 0 {
                "Nothing to close"
            } else {
                "Every one of those has unsaved changes"
            });
            return;
        }

        // The editor is never empty — the same promise `Workspace::close_buffer`
        // makes, and for the same reason.
        if kept.is_empty() {
            kept.push(crate::buffer::Buffer::scratch(
                "scratch.rs",
                crate::buffer::SCRATCH,
            ));
            new_active = 0;
        }
        let last = kept.len() - 1;
        self.put_buffers(kept);
        self.focus_buffer(new_active.min(last));
        self.sync_caret();

        self.notify(&match spared {
            0 => format!("Closed {closed} tab{}", if closed == 1 { "" } else { "s" }),
            n => format!("Closed {closed}, kept {n} with unsaved changes",),
        });
    }

    // ----- Replace across the workspace ---------------------------------

    /// The query the find bar currently describes.
    #[must_use]
    pub(super) fn workspace_query(&self) -> Option<crate::find::Query> {
        let needle = self.find_query.get();
        if needle.trim().is_empty() {
            return None;
        }
        Some(crate::find::Query {
            needle,
            case_sensitive: self.find_case_sensitive.get(),
            whole_word: self.find_whole_word.get(),
        })
    }

    /// Work out what a workspace-wide replace would do, and offer it.
    ///
    /// Nothing is written here. The plan counts matches per file and records
    /// the files it would refuse — read-only, or open with unsaved edits —
    /// because the answer to "why did three of my thirty files not change" has
    /// to arrive *with* the offer rather than after it.
    pub fn plan_workspace_replace(&self) {
        let Some(query) = self.workspace_query() else {
            self.notify("Type something to find first");
            return;
        };
        if self.root.peek().is_none() {
            self.notify("Replace across files needs a folder open");
            return;
        }

        let dirty: Vec<std::path::PathBuf> = self
            .buffers
            .get()
            .iter()
            .filter(|buffer| buffer.dirty)
            .filter_map(|buffer| buffer.path.clone())
            .collect();

        let mut plan = crate::find::Plan::default();
        for path in self.tree.get().files() {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let count = query.matches(&text).len();
            if count == 0 {
                continue;
            }
            if dirty.contains(&path) {
                plan.skipped.push((path, "it has unsaved changes"));
            } else if std::fs::metadata(&path).is_ok_and(|meta| meta.permissions().readonly()) {
                plan.skipped.push((path, "it is read-only"));
            } else {
                plan.files.push((path, count));
            }
        }

        if plan.is_empty() && plan.skipped.is_empty() {
            self.notify("No matches in this workspace");
            return;
        }
        self.replace_plan.set(Some(Rc::new(plan)));
    }

    pub fn cancel_workspace_replace(&self) {
        self.replace_plan.set(None);
    }

    /// Carry out the planned replace.
    ///
    /// Re-reads and re-matches each file rather than trusting the counts in the
    /// plan: between planning and confirming, a build can have run and a
    /// formatter can have rewritten half of them. The plan decides *which*
    /// files are touched; the file on disk decides what is in them.
    pub fn confirm_workspace_replace(&self) {
        let Some(plan) = self.replace_plan.get() else {
            return;
        };
        self.replace_plan.set(None);
        let Some(query) = self.workspace_query() else {
            return;
        };
        let replacement = self.find_replacement.get();

        let mut changed = 0usize;
        let mut replaced = 0usize;
        let mut failed = 0usize;
        for (path, _) in &plan.files {
            let Ok(text) = std::fs::read_to_string(path) else {
                failed += 1;
                continue;
            };
            let matches = query.matches(&text);
            if matches.is_empty() {
                continue;
            }
            // Back to front, for the reason `find::replace_all` gives: forwards
            // shifts every range after the first by the difference in length,
            // which is the classic way a replace-all quietly corrupts a file.
            let mut edited = text;
            for &(start, end) in matches.iter().rev() {
                edited.replace_range(start..end, &replacement);
            }
            if std::fs::write(path, &edited).is_err() {
                failed += 1;
                continue;
            }
            changed += 1;
            replaced += matches.len();
        }

        // Any open, clean buffer over a file that changed is reloaded, so the
        // editor is not showing the pre-replace text of a file it just wrote.
        let mut buffers = (*self.buffers.get()).clone();
        for buffer in &mut buffers {
            if let Some(path) = buffer.path.clone() {
                if !buffer.dirty && plan.files.iter().any(|(changed, _)| *changed == path) {
                    let _ = buffer.reload();
                }
            }
        }
        self.put_buffers(buffers);
        self.sync_caret();
        self.rescan_tree();

        let mut message = format!(
            "Replaced {replaced} match{} in {changed} file{}",
            if replaced == 1 { "" } else { "es" },
            if changed == 1 { "" } else { "s" }
        );
        if !plan.skipped.is_empty() {
            message.push_str(&format!(", skipped {}", plan.skipped.len()));
        }
        if failed > 0 {
            message.push_str(&format!(", {failed} could not be written"));
        }
        self.notify(&message);
    }

    /// Take files dropped on the window and do the obvious thing with them.
    ///
    /// **The framework has delivered these the whole time.** `HoveredFile`,
    /// `DroppedFile` and `HoveredFileCancelled` are handled in
    /// `vieww-platform-winit`, gathered into a `DroppedFiles` on the driver,
    /// and `vieww_foundation::file_drop` documents the design. The studio read
    /// none of it — so dragging a file onto the window, which is how most
    /// people open a file in a desktop editor for the very first time, did
    /// nothing at all.
    ///
    /// A directory becomes the workspace; files become tabs. A drop of both is
    /// resolved as "the folder, then the files", which is the order that leaves
    /// the files open at the end.
    pub fn accept_dropped(&self, paths: &[std::path::PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let (dirs, files): (Vec<_>, Vec<_>) = paths.iter().cloned().partition(|path| path.is_dir());

        // One folder is a workspace. Several is ambiguous, and picking the
        // first would be a guess about which project the user meant — so it is
        // refused with a sentence rather than resolved silently.
        match dirs.len() {
            0 => {}
            1 => self.open_workspace(&dirs[0]),
            n => self.notify(&format!(
                "Dropped {n} folders — drop one at a time to choose a workspace"
            )),
        }

        let count = files.len();
        for path in files {
            self.open_path(path);
        }
        if count > 1 {
            self.notify(&format!("Opened {count} files"));
        }
    }
}
