//! Buffers, the watcher, undo and find.
//!
//! The buffer list, and the three things that operate on the open one.
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
    // ----- buffers -------------------------------------------------------

    /// Move `step` tabs along, wrapping.
    pub(super) fn step_tab(&self, step: i32) {
        let count = self.buffers.get().len();
        if count == 0 {
            return;
        }
        let at = self.active_buffer.peek();
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "a tab count that overflows i32 is not a tab strip"
        )]
        let next = (at as i32 + step).rem_euclid(count as i32) as usize;
        self.focus_buffer(next);
    }

    /// Open a new scratch buffer and switch to it.
    ///
    /// Named `untitled-N.rs` with N the first number not already taken, rather
    /// than always `untitled.rs`: two of those in the tab strip are two tabs
    /// with the same label and no way to tell them apart.
    pub fn new_file(&self) {
        let mut buffers = (*self.buffers.get()).clone();
        let mut number = 1;
        while buffers
            .iter()
            .any(|b| b.name == format!("untitled-{number}.rs"))
        {
            number += 1;
        }
        buffers.push(Buffer::scratch(
            format!("untitled-{number}.rs"),
            crate::buffer::SCRATCH,
        ));
        self.focus_buffer(buffers.len() - 1);
        self.put_buffers(buffers);
        self.mark_dirty();
    }

    /// The welcome card's first row: an untitled Say counter that renders
    /// straight away.
    ///
    /// No folder, no name, no Cargo.toml, no toolchain conversation —
    /// `render` reads the active buffer alone, and the bundled toolchain
    /// compiles what codegen produces. The one click a stranger needs to see
    /// what this studio is. Named like `new_file` names its buffers, so two
    /// visits do not fight over a tab label.
    pub fn try_say_screen(&self) {
        let mut buffers = (*self.buffers.get()).clone();
        let mut number = 1;
        while buffers
            .iter()
            .any(|b| b.name == format!("counter-{number}.say"))
        {
            number += 1;
        }
        let name = format!("counter-{number}.say");
        let mut buffer = Buffer::scratch(name, crate::scaffold::SAY_COUNTER.to_string());
        buffer.language = crate::language::Language::Say;
        buffers.push(buffer);
        self.focus_buffer(buffers.len() - 1);
        self.put_buffers(buffers);
        self.mark_dirty();
        // Render immediately: the screen appearing in the device frame is
        // the whole pitch, and asking for a second click would dilute it.
        self.render();
    }

    /// Close the active buffer.
    ///
    /// Refuses to close the last one: a studio with no buffer is a studio with
    /// an empty editor pane and no way to get one back, and "New File" being
    /// one keystroke away does not make that a good state to be able to reach.
    pub fn close_active(&self) {
        let mut buffers = (*self.buffers.get()).clone();
        // **The last tab closes too.** It used to refuse below two, so the
        // editor could never be empty: closing everything left one file open
        // that nothing would take off the screen, and "close this" was a
        // command that silently did nothing on the one tab somebody was most
        // likely to be pointing at. An empty editor is a real state — the pane
        // says so and offers the two ways out — and it is what a person
        // clearing their desk is asking for.
        if buffers.is_empty() {
            return;
        }
        let index = self.active_buffer.peek().min(buffers.len() - 1);
        let closed = buffers.remove(index);
        // Diagnostics for a file that is no longer open point at nothing.
        self.diagnostics.update(|diagnostics| {
            let next: Vec<Diagnostic> = diagnostics
                .iter()
                .filter(|d| d.file != closed.name)
                .cloned()
                .collect();
            *diagnostics = Rc::new(next);
        });
        self.active_buffer
            .set(index.min(buffers.len().saturating_sub(1)));
        self.put_buffers(buffers);
        self.sync_caret();
    }

    /// Write every dirty buffer that has somewhere to go.
    pub fn save_all(&self) {
        let mut buffers = (*self.buffers.get()).clone();
        let mut written = 0usize;
        let mut failures: Vec<String> = Vec::new();
        for buffer in &mut buffers {
            if !buffer.dirty || buffer.path.is_none() {
                continue;
            }
            match buffer.save() {
                Ok(()) => written += 1,
                Err(error) => failures.push(format!("{}: {error}", buffer.name)),
            }
        }
        self.put_buffers(buffers);

        let mut log = vec![format!("saved {written} file(s)")];
        log.extend(failures);
        self.append_output(log);
    }

    // ----- N1: the tree, the watcher, on-demand opening ------------------

    /// Drain the external-change watcher and rescan the tree if anything moved.
    /// Called every frame from the platform hook; a no-op on the common frame
    /// where nothing changed.
    ///
    /// An external change to an *open, clean* buffer is the dangerous case: the
    /// in-memory text would overwrite the file on the next save. Such a buffer
    /// is reloaded from disk. A buffer with unsaved edits is left alone — the
    /// user's work wins over the disk's.
    pub fn drain_watcher(&self) {
        let Some(watcher) = &self.watcher else {
            return;
        };
        let events = watcher.drain();
        if events.is_empty() {
            return;
        }
        let mut reloaded = false;
        let mut conflicts: Vec<std::path::PathBuf> = Vec::new();
        let mut buffers = (*self.buffers.get()).clone();
        for event in &events {
            if let crate::file_tree::WatchEvent::Changed(path) = event {
                if let Some(index) = buffers.iter().position(|b| b.path.as_deref() == Some(path)) {
                    if buffers[index].dirty {
                        // **This branch used to do nothing at all.** A clean
                        // buffer reloaded; a dirty one took the `else` and the
                        // event was dropped in silence. The user carried on
                        // typing into a buffer that no longer matched disk and
                        // found out at save time, by overwriting whatever the
                        // other change was — a `git checkout`, a `rustfmt`, a
                        // colleague's edit on a shared drive.
                        //
                        // Recorded rather than resolved here: the answer is a
                        // question for the user, and `drain_watcher` runs
                        // inside the frame hook where there is nothing to ask.
                        conflicts.push(path.clone());
                    } else if buffers[index].reload().is_ok() {
                        reloaded = true;
                    }
                }
            }
        }
        if reloaded {
            self.put_buffers(buffers);
            self.sync_caret();
        }
        if !conflicts.is_empty() {
            self.raise_conflicts(conflicts);
        }
        self.rescan_tree();
    }

    /// Add `paths` to the set of buffers that disagree with disk.
    ///
    /// Additive, and de-duplicated: a file saved three times by a formatter
    /// running outside the window is one conflict, not three, and a second file
    /// changing while the dialog is up must not replace the first.
    pub fn raise_conflicts(&self, paths: Vec<std::path::PathBuf>) {
        let mut pending = (*self.conflicts.get()).clone();
        for path in paths {
            if !pending.contains(&path) {
                pending.push(path);
            }
        }
        self.conflicts.set(Rc::new(pending));
    }

    /// Take the user's edits and forget the change on disk.
    ///
    /// The buffer stays dirty, which is the point: the next save writes over
    /// the other change, and that is now a decision the user made rather than
    /// something that happened to them.
    pub fn keep_mine(&self) {
        self.conflicts.set(Rc::new(Vec::new()));
        self.notify("Keeping your version — saving will overwrite the change on disk");
    }

    /// Reload every conflicted buffer from disk, discarding the user's edits.
    pub fn take_theirs(&self) {
        let paths = self.conflicts.get();
        let mut buffers = (*self.buffers.get()).clone();
        let mut reloaded = 0;
        for path in paths.iter() {
            if let Some(index) = buffers
                .iter()
                .position(|b| b.path.as_deref() == Some(path.as_path()))
            {
                if buffers[index].reload().is_ok() {
                    reloaded += 1;
                }
            }
        }
        self.put_buffers(buffers);
        self.conflicts.set(Rc::new(Vec::new()));
        self.sync_caret();
        self.notify(&format!(
            "Reloaded {reloaded} file{} from disk",
            if reloaded == 1 { "" } else { "s" }
        ));
    }

    /// The names the conflict dialog lists.
    #[must_use]
    pub fn conflict_names(&self) -> Vec<String> {
        self.conflicts
            .get()
            .iter()
            .map(|path| {
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
            })
            .collect()
    }

    /// Re-scan the directory tree from disk, preserving expansion state.
    pub fn rescan_tree(&self) {
        let mut tree = self.tree.get();
        tree.rescan();
        self.tree.set(tree);
    }

    /// Flip the expansion of the directory at `path` in the Explorer tree.
    pub fn toggle_expand(&self, path: std::path::PathBuf) {
        let mut tree = self.tree.get();
        tree.toggle_dir(&path);
        self.tree.set(tree);
    }

    /// Close every folder in the Explorer tree.
    ///
    /// Behind the sidebar header's collapse-all icon, which drew and hovered
    /// and did nothing until now.
    pub fn collapse_folders(&self) {
        let mut tree = self.tree.get();
        if tree.collapse_all() {
            self.tree.set(tree);
        } else {
            self.append_output(vec!["Every folder is already closed.".to_owned()]);
        }
    }

    /// Open `path` from the tree as a buffer and make it active. On-demand
    /// loading: a project's tree may list hundreds of files, but only the ones
    /// the user clicks are loaded into memory.
    pub fn open_path(&self, path: std::path::PathBuf) {
        // **Canonicalised, so one file is one buffer.** Without this a
        // symlinked file reached by two paths — `src/shared.rs` and the
        // `../common/shared.rs` it points at — opened as two independent
        // buffers over the same bytes, and saving one silently discarded the
        // edits made in the other. `canonicalize` also resolves `..` and `.`,
        // which is how the same file arrived twice far more often than
        // symlinks did. A path that cannot be canonicalised (it was deleted
        // between the click and here) is used as given, so the open fails with
        // a real error rather than being quietly rewritten.
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        let buffers = (*self.buffers.get()).clone();
        if let Some(index) = buffers
            .iter()
            .position(|b| b.path.as_deref() == Some(&path))
        {
            self.focus_buffer(index);
            self.sync_caret();
            return;
        }
        // **The refusal is now said out loud.** This was `if let Ok(..)`, and
        // an open that failed did nothing at all — the user clicked a file in
        // the Explorer and the editor did not change, which reads as a broken
        // click rather than as "that is a 400 MB log" or "that file is
        // binary". `Buffer::open`'s errors are written to be shown, so they
        // are shown.
        match Buffer::open(&path) {
            Ok(buffer) => {
                let read_only = buffer.read_only;
                let lossy = buffer.encoding == crate::buffer::Encoding::Unknown;
                let mut buffers = buffers;
                buffers.push(buffer);
                let index = buffers.len() - 1;
                self.put_buffers(buffers);
                self.focus_buffer(index);
                self.sync_caret();
                // Said on open rather than on save, which is the whole point of
                // detecting either of them early.
                if lossy {
                    self.notify("Not valid UTF-8 — shown as text, and saving is refused");
                } else if read_only {
                    self.notify("Read-only — this file cannot be saved from here");
                }
            }
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Create a new file under the workspace root and open it. A scratch
    /// workspace (no root) falls back to `new_file`'s untitled buffer, because a
    /// file created in `.` is not one the studio can keep track of.
    pub fn new_file_at(&self, name: String) {
        let Some(root) = self.root.peek() else {
            self.new_file();
            return;
        };
        let path = root.join(&name);
        if crate::file_tree::create_file(&path).is_ok() {
            self.open_path(path);
            self.rescan_tree();
        }
    }

    /// Throw away unsaved edits and re-read the file from disk.
    pub fn revert_active(&self) {
        let index = self.active_buffer.peek();
        let mut buffers = (*self.buffers.get()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return;
        };
        let name = buffer.name.clone();
        match buffer.reload() {
            Ok(()) => {
                self.put_buffers(buffers);
                self.sync_caret();
                self.append_output(vec![format!("reverted {name}")]);
                self.mark_dirty();
            }
            Err(error) => self.append_output(vec![format!("could not revert {name}: {error}")]),
        }
    }

    // ----- undo and redo -------------------------------------------------

    /// Step the active buffer back one edit.
    pub fn undo(&self) {
        self.step_history(true);
    }

    /// Step it forward again.
    pub fn redo(&self) {
        self.step_history(false);
    }

    pub(super) fn step_history(&self, backwards: bool) {
        let index = self.active_buffer.peek();
        let mut buffers = (*self.buffers.get()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return;
        };
        let stepped = if backwards {
            buffer.history.undo()
        } else {
            buffer.history.redo()
        };
        let Some(value) = stepped else {
            return;
        };
        buffer.value = value;
        buffer.dirty = true;
        self.put_buffers(buffers);
        self.sync_caret();
        self.mark_dirty();
    }

    /// Re-read the caret from the active buffer.
    ///
    /// Called by every path that replaces a buffer's value without going
    /// through [`edit`](Self::edit) — undo, revert, closing a tab. The status
    /// bar reads one signal, and this is what keeps it from being a stale copy
    /// of a buffer that is no longer showing.
    pub(super) fn sync_caret(&self) {
        if let Some(buffer) = self.active() {
            self.caret.set(buffer.caret());
        }
    }

    // ----- find and replace ----------------------------------------------

    /// What the find bar is currently looking for.
    #[must_use]
    pub fn query(&self) -> Query {
        Query {
            needle: self.find_query.get(),
            case_sensitive: self.find_case_sensitive.get(),
            whole_word: self.find_whole_word.get(),
        }
    }

    /// Every match of the current query in the active buffer.
    #[must_use]
    pub fn find_matches(&self) -> Vec<(usize, usize)> {
        self.active()
            .map(|buffer| self.query().matches(&buffer.value.text))
            .unwrap_or_default()
    }

    /// Show the find bar, optionally with the replace row.
    ///
    /// Seeds the query from the selection when there is one, which is what
    /// every editor does and what makes "select a word, press ⌘F" work.
    pub fn open_find(&self, replacing: bool) {
        if let Some(buffer) = self.active() {
            let selected = buffer.value.selected_text();
            if !selected.is_empty() && !selected.contains('\n') {
                self.find_query.set(selected.to_string());
            }
        }
        self.find_open.set(true);
        self.find_replacing.set(replacing);
        self.find_index.set(0);
        self.select_match();
    }

    /// Move `step` matches along and select the result.
    pub(super) fn find_step(&self, step: i32) {
        let matches = self.find_matches();
        if matches.is_empty() {
            return;
        }
        let at = self.find_index.peek();
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "a match count that overflows i32 is not a file anybody edits"
        )]
        let next = (at as i32 + step).rem_euclid(matches.len() as i32) as usize;
        self.find_index.set(next);
        self.select_match();
    }

    /// Put the selection on the current match, so it is visible in the editor.
    pub(super) fn select_match(&self) {
        let matches = self.find_matches();
        let Some(buffer) = self.active() else {
            return;
        };
        let index = self.find_index.peek().min(matches.len().saturating_sub(1));
        if matches.is_empty() {
            return;
        }
        let next = find::select(&buffer.value, &matches, index);
        // Through `edit` so the caret readout and the tab dot stay in step —
        // and it records nothing in the history, because selecting is not an
        // edit.
        self.edit(next);
    }

    /// Replace the match the find bar is on.
    pub fn replace_current(&self) {
        let matches = self.find_matches();
        let Some(buffer) = self.active() else {
            return;
        };
        let index = self.find_index.peek();
        let Some((next, _)) =
            find::replace_one(&buffer.value, &matches, index, &self.find_replacement.get())
        else {
            return;
        };
        self.edit(next);
        // The list shifted under us; clamp rather than step, so the bar lands
        // on the match that is now at this index rather than skipping one.
        let remaining = self.find_matches().len();
        self.find_index.set(index.min(remaining.saturating_sub(1)));
        self.select_match();
    }

    /// Replace every match at once.
    pub fn replace_all(&self) {
        let matches = self.find_matches();
        if matches.is_empty() {
            return;
        }
        let Some(buffer) = self.active() else {
            return;
        };
        let (next, count) =
            find::replace_all(&buffer.value, &matches, &self.find_replacement.get());
        self.edit(next);
        self.find_index.set(0);
        self.append_output(vec![format!("replaced {count} occurrence(s)")]);
    }

    // ----- the palette ---------------------------------------------------

    /// The commands the palette is showing.
    #[must_use]
    pub fn palette_results(&self) -> Vec<PaletteEntry> {
        let query = self.palette_query.get();
        let (mode, term) = PaletteMode::parse(&query);
        let term = term.trim();

        match mode {
            PaletteMode::Commands => Command::matching(term)
                .into_iter()
                .map(PaletteEntry::Command)
                .collect(),

            PaletteMode::Files => {
                let root = self.root.get();
                let lower = term.to_lowercase();
                self.tree
                    .get()
                    .files()
                    .into_iter()
                    .filter(|path| {
                        term.is_empty() || path.to_string_lossy().to_lowercase().contains(&lower)
                    })
                    .take(PALETTE_MAX)
                    .map(|path| {
                        let relative = root
                            .as_deref()
                            .and_then(|root| path.strip_prefix(root).ok())
                            .unwrap_or(&path)
                            .to_path_buf();
                        PaletteEntry::File {
                            label: relative
                                .file_name()
                                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                            detail: relative
                                .parent()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_default(),
                            path,
                        }
                    })
                    .collect()
            }

            PaletteMode::Symbols => {
                let lower = term.to_lowercase();
                self.symbols()
                    .into_iter()
                    .filter(|symbol| term.is_empty() || symbol.name.to_lowercase().contains(&lower))
                    .map(|symbol| PaletteEntry::Symbol {
                        // Indented by nesting, so a method reads as belonging
                        // to its `impl` rather than as a second top-level item
                        // that happens to share a name with one.
                        label: format!("{}{}", "  ".repeat(symbol.depth), symbol.name),
                        detail: symbol.kind.to_owned(),
                        line: symbol.line,
                    })
                    .collect()
            }

            PaletteMode::Line => {
                let total = self
                    .active()
                    .as_ref()
                    .map_or(0, crate::buffer::Buffer::line_count);
                let Ok(line) = term.parse::<u32>() else {
                    return vec![PaletteEntry::Line {
                        line: 1,
                        detail: format!("{total} lines in this file"),
                    }];
                };
                vec![PaletteEntry::Line {
                    line: line.max(1),
                    // `u32` to `usize` is a widening cast on every target the
                    // studio builds for, so there is nothing to allow here.
                    detail: if line as usize > total {
                        format!("past the end \u{2014} this file has {total} lines")
                    } else {
                        format!("of {total}")
                    },
                }]
            }
        }
    }

    /// Definitions in the active buffer, from the highlighter's own parse.
    ///
    /// This doc used to say "by a scan rather than a parse", deferring the real
    /// thing to a language server. The scan is gone — `highlight::symbols`
    /// walks the tree-sitter tree — and the deferral with it, so what is left
    /// to say is where the answer comes from.
    #[must_use]
    pub fn symbols(&self) -> Vec<crate::highlight::Symbol> {
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        // The same parser that highlights every keystroke. Sharing it is not an
        // optimisation so much as the point: two answers to "what is in this
        // file" is two things to keep in step, and the line scan this replaces
        // disagreed with the highlighter about doc comments, about `impl`
        // blocks and about anything not at the start of its line.
        self.highlighter
            .borrow_mut()
            .symbols(&buffer.value.text, buffer.language)
    }

    /// The definitions containing the caret, outermost first.
    ///
    /// `[impl CardGrid, fn build]` for a caret inside that method — the trail a
    /// breadcrumb bar shows and this studio's did not: it stopped at the file
    /// name, with a comment deferring the rest to "M6's tree-sitter". M6
    /// landed; [`Studio::symbols`] has parsed the buffer ever since, and
    /// nothing was reading it here.
    ///
    /// # Why nesting is inferred from `depth` rather than from a range
    ///
    /// A [`Symbol`](crate::highlight::Symbol) records the line it starts on and
    /// how deeply it is nested, not where it ends. That is enough: symbols come
    /// back in source order, so the innermost definition at each depth that
    /// starts at or above the caret is the one the caret is inside, and a
    /// symbol at a shallower depth closes everything deeper than it. What this
    /// cannot see is a caret in the gap *between* two items at the same depth —
    /// it reads as still inside the first — which shows a trail one item stale
    /// on a blank line and is not worth a second parse to fix.
    #[must_use]
    pub fn symbol_trail(&self) -> Vec<crate::highlight::Symbol> {
        let (line, _) = self.caret.get();
        let mut trail: Vec<crate::highlight::Symbol> = Vec::new();
        for symbol in self.symbols() {
            if symbol.line > line {
                break;
            }
            trail.truncate(symbol.depth);
            if trail.len() == symbol.depth {
                trail.push(symbol);
            }
        }
        trail
    }

    /// Move the palette's highlight, wrapping.
    pub fn palette_step(&self, step: i32) {
        let count = self.palette_results().len();
        if count == 0 {
            return;
        }
        let at = self.palette_index.peek();
        #[expect(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "the command list is thirty long"
        )]
        let next = (at as i32 + step).rem_euclid(count as i32) as usize;
        self.palette_index.set(next);
    }

    /// Run whatever the palette's highlight is on.
    pub fn palette_accept(&self) {
        let results = self.palette_results();
        let Some(entry) = results.get(self.palette_index.peek()).cloned() else {
            return;
        };
        self.palette_activate(&entry);
    }

    /// Do whatever one palette row means, and close the palette after.
    ///
    /// A command goes through [`run`](Self::run) rather than being dispatched
    /// here, which is the property that keeps the palette from becoming a
    /// second place that knows how to save a file. The other three kinds have
    /// no command to route through — there is no `Command::OpenFile(path)`,
    /// and adding one per file is not what a registry is for.
    pub fn palette_activate(&self, entry: &PaletteEntry) {
        match entry {
            PaletteEntry::Command(command) => {
                if self.can_run(*command) {
                    self.run(*command);
                }
                return;
            }
            PaletteEntry::File { path, .. } => self.open_path(path.clone()),
            PaletteEntry::Symbol { line, .. } | PaletteEntry::Line { line, .. } => {
                self.jump_to(*line, 1);
            }
        }
        self.palette_open.set(false);
    }
}
