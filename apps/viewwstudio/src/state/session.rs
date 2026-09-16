//! What the studio remembers between launches.
//!
//! Persistence, the assets read off disk, and the crash recovery that reads a
//! session back after a hard exit.
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
    // ----- What the studio remembers between launches -------------------
    //
    // The store is the framework's `Storage` service, which `main` installs
    // and which writes a file under `vieww_platform_winit::storage::data_dir`.
    // Every method here is a no-op when there is no store, which is the case
    // in every test that builds a bare `Studio` — persistence is not something
    // a unit test should have to opt out of, and a test that wrote to the
    // developer's real settings file would be a test that changed their editor.

    /// The key-value store, if one was provided.
    #[must_use]
    pub fn storage(&self) -> Option<Rc<dyn vieww_foundation::Storage>> {
        self.services.get::<dyn vieww_foundation::Storage>()
    }

    /// Every persisted preference, read out of the live signals.
    #[must_use]
    pub fn settings(&self) -> crate::settings::Settings {
        crate::settings::Settings {
            dark: self.dark.get(),
            show_insets: self.show_insets.get(),
            comforts: self.comforts.get(),
            indent_guides: self.indent_guides.get(),
            format_on_save: self.format_on_save.get(),
            inline_diagnostics: self.inline_diagnostics.get(),
            release_profile: self.profile.get() == Profile::Release,
            auto_render: self.auto_render.get(),
            minimap: self.minimap.get(),
            blink: self.blink_enabled.get(),
            highlight: self.highlight_enabled.get(),
            reduce_motion: self.preview_reduce_motion.get(),
            preview_dark: self.preview_dark.get(),
            word_wrap: self.word_wrap.get(),
            large_ui: self.large_ui.get(),
            high_contrast: self.high_contrast.get(),
            accent: self.accent.get(),
            lint: self.lint_enabled.get(),
            theme: self.theme_name.get(),
            font_size: self.font_size.get(),
            tab_width: self.tab_width.get(),
            text_scale: self.preview_text_scale.get(),
            sidebar_width: self.sidebar_width.get(),
            preview_width: self.preview_width.get(),
            panel_open: self.panel_open.get(),
            right_open: self.right_open.get(),
            activity_bar_open: self.activity_bar_open.get(),
            android_home: self.android_home.get(),
            android_ndk: self.android_ndk.get(),
            java_home: self.java_home.get(),
            keystore: self.keystore.get(),
            keystore_alias: self.keystore_alias.get(),
            apple_team: self.apple_team.get(),
        }
    }

    /// Put a settings record into the live signals.
    ///
    /// Every `set` here is a signal write and therefore a rebuild, so this runs
    /// once at launch rather than per frame. Values are written even when they
    /// equal what is already there: `Signal::set` on an unchanged value is
    /// cheap, and the alternative is twenty-one comparisons that have to be
    /// kept in step with the twenty-one assignments.
    pub fn apply_settings(&self, settings: &crate::settings::Settings) {
        self.dark.set(settings.dark);
        self.android_home.set(settings.android_home.clone());
        self.android_ndk.set(settings.android_ndk.clone());
        self.java_home.set(settings.java_home.clone());
        self.keystore.set(settings.keystore.clone());
        self.keystore_alias.set(settings.keystore_alias.clone());
        self.apple_team.set(settings.apple_team.clone());
        self.show_insets.set(settings.show_insets);
        self.comforts.set(settings.comforts);
        self.indent_guides.set(settings.indent_guides);
        self.format_on_save.set(settings.format_on_save);
        self.inline_diagnostics.set(settings.inline_diagnostics);
        self.profile.set(if settings.release_profile {
            Profile::Release
        } else {
            Profile::Debug
        });
        self.auto_render.set(settings.auto_render);
        self.minimap.set(settings.minimap);
        self.blink_enabled.set(settings.blink);
        self.highlight_enabled.set(settings.highlight);
        self.preview_reduce_motion.set(settings.reduce_motion);
        self.preview_dark.set(settings.preview_dark);
        self.word_wrap.set(settings.word_wrap);
        self.large_ui.set(settings.large_ui);
        self.high_contrast.set(settings.high_contrast);
        self.accent.set(settings.accent.clone());
        self.lint_enabled.set(settings.lint);
        self.theme_name.set(settings.theme.clone());
        self.font_size.set(settings.font_size);
        self.tab_width.set(settings.tab_width);
        self.preview_text_scale.set(settings.text_scale);
        self.sidebar_width.set(settings.sidebar_width);
        self.preview_width.set(settings.preview_width);
        self.panel_open.set(settings.panel_open);
        self.right_open.set(settings.right_open);
        self.activity_bar_open.set(settings.activity_bar_open);
    }

    /// Where the user is now, for the session file.
    ///
    /// Untitled buffers are not written — there is nothing to read them back
    /// from — so `active` is recomputed against the list that *is* written
    /// rather than copied from `active_buffer`. Getting that wrong restores
    /// the wrong tab whenever an untitled buffer sits to the left of the
    /// active one, which is the common case after `New File`.
    #[must_use]
    pub fn session(&self) -> crate::settings::Session {
        let buffers = self.buffers.get();
        let active_index = self.active_buffer.get();
        let mut open = Vec::new();
        let mut active = 0;
        for (index, buffer) in buffers.iter().enumerate() {
            if let Some(path) = &buffer.path {
                if index <= active_index {
                    active = open.len();
                }
                open.push(path.clone());
            }
        }
        crate::settings::Session {
            root: self.root.get(),
            open,
            active,
            view: self.view.get().title().to_owned(),
            panel_tab: self.panel_tab.get().label().to_owned(),
            recent: (*self.recent.get()).clone(),
            // Filled in by `main`, which is the only place that can ask the
            // window how big it is.
            window: None,
        }
    }

    /// Reopen what the session file names.
    ///
    /// **A file that has gone is skipped, not reported.** Between two launches
    /// a branch gets switched and half the tabs name files that no longer
    /// exist; a studio that opens with six error notices about that is a
    /// studio nobody wants to launch. What *is* worth saying is when the root
    /// itself has gone, because then nothing else in the session makes sense —
    /// and that is the caller's call, not this one's.
    pub fn apply_session(&self, session: &crate::settings::Session) {
        self.recent.set(Rc::new(session.recent.clone()));
        if let Some(view) = View::from_title(&session.view) {
            self.view.set(view);
        }
        if let Some(tab) = PanelTab::from_label(&session.panel_tab) {
            self.panel_tab.set(tab);
        }
        if session.open.is_empty() {
            return;
        }
        let mut buffers: Vec<crate::buffer::Buffer> = Vec::new();
        let mut active = 0;
        for path in &session.open {
            if !path.is_file() {
                continue;
            }
            if let Ok(buffer) = crate::buffer::Buffer::open(path) {
                if session.open.iter().position(|p| p == path) <= Some(session.active) {
                    active = buffers.len();
                }
                buffers.push(buffer);
            }
        }
        if buffers.is_empty() {
            return;
        }
        self.focus_buffer(active.min(buffers.len() - 1));
        self.put_buffers(buffers);
        self.sync_caret();
    }

    /// Write the preferences out. Silent on failure, by design.
    ///
    /// A store that cannot be written is a read-only home directory or a full
    /// disk, and there is nothing the user can do about either from inside the
    /// editor. Reporting it on every toggle would put a red message under a
    /// checkbox sixty times a session. The place it *is* worth saying is on
    /// the way out, which is what [`Studio::save_all_state`] does.
    pub fn save_settings(&self) {
        if let Some(store) = self.storage() {
            let _ = store.set(crate::settings::SETTINGS_KEY, &self.settings().render());
        }
    }

    /// Write the session out, optionally stamping the window size.
    pub fn save_session(&self, window: Option<(f32, f32)>) {
        if let Some(store) = self.storage() {
            let mut session = self.session();
            session.window = window;
            let _ = store.set(crate::settings::SESSION_KEY, &session.render());
        }
    }

    /// Both records, on the way out.
    pub fn save_all_state(&self, window: Option<(f32, f32)>) {
        self.save_settings();
        self.save_session(window);
    }

    /// Whether this launch should open on the welcome overlay.
    ///
    /// The rule is "there is nothing here to work on": no folder, and nothing
    /// but the scratch buffer. A studio that restored a session or was pointed
    /// at a project opens on the work, because putting a welcome screen in
    /// front of someone who already knows what they are doing is the fastest
    /// way to make them stop reading welcome screens.
    #[must_use]
    pub fn should_welcome(&self) -> bool {
        self.root.peek().is_none()
            && self
                .buffers
                .peek()
                .iter()
                .all(|buffer| buffer.path.is_none())
    }

    // ----- Themes, snippets and keybindings from disk --------------------

    /// The directory the customisation files live in.
    #[must_use]
    pub fn customise_dir() -> std::path::PathBuf {
        crate::about::data_dir()
    }

    /// The chord for `command`, honouring the user's keymap.
    ///
    /// Everything that draws or matches a shortcut goes through here rather
    /// than through `Command::chord`, so an override reaches the menus, the
    /// palette, the welcome screen's hints **and** the key handler at once.
    /// Before the keymap existed, `Command::chord` was the only answer and it
    /// was a compiled-in `match`.
    #[must_use]
    pub fn chord_for(&self, command: Command) -> Option<crate::command::Chord> {
        match self.keymap.get(&command) {
            Some(override_) => override_.clone(),
            None => command.chord(),
        }
    }

    /// The command a key event runs, honouring the keymap.
    #[must_use]
    pub fn command_for_key(&self, event: &vieww_foundation::KeyEvent) -> Option<Command> {
        let host = self.host;
        Command::ALL.into_iter().find(|command| {
            self.chord_for(*command)
                .is_some_and(|chord| chord.matches(event, host))
        })
    }

    /// Read `theme.txt`, `snippets.txt` and `keymap.txt`, if they are there.
    ///
    /// Every one is optional and every one fails softly: a studio whose theme
    /// file has a typo in it opens with the built-in theme and a note, not with
    /// an error dialog and no window.
    pub fn load_customisations(&mut self) {
        let dir = Self::customise_dir();
        let mut problems: Vec<String> = Vec::new();

        // --- theme ---
        if let Ok(text) = std::fs::read_to_string(dir.join(crate::customise::THEME_FILE)) {
            let theme = crate::customise::Theme::parse(&text);
            self.dark.set(theme.dark);
            if !theme.is_empty() {
                // Fed through the same `token_edits` map the Tokens view
                // writes, so there is one path from "a colour was overridden"
                // to "the window is that colour" — see `Studio::themed`.
                let mut edits = (*self.token_edits.get()).clone();
                for token in crate::customise::TOKENS {
                    if let Some([r, g, b, _]) = theme.get(token) {
                        let rgb = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
                        edits.insert((*token).to_owned(), rgb);
                    }
                }
                self.token_edits.set(Rc::new(edits));
            }
        }

        // --- snippets ---
        if let Ok(text) = std::fs::read_to_string(dir.join(crate::customise::SNIPPETS_FILE)) {
            let snippets = crate::customise::parse_snippets(&text);
            self.user_snippets.set(Rc::new(snippets));
        }

        // --- keymap ---
        if let Ok(text) = std::fs::read_to_string(dir.join(crate::customise::KEYMAP_FILE)) {
            let keymap = crate::customise::parse_keymap(&text);
            problems.extend(keymap.problems.iter().cloned());
            let mut resolved = std::collections::HashMap::new();
            for (name, spec) in &keymap.bindings {
                let Some(command) = Command::ALL
                    .into_iter()
                    .find(|command| command.title().eq_ignore_ascii_case(name))
                else {
                    problems.push(format!("no command called {name:?}"));
                    continue;
                };
                resolved.insert(command, chord_from_spec(spec));
            }
            // **A collision is refused, not resolved.** Two commands on one
            // chord means one of them silently stops working, and the one that
            // stops is whichever `Command::ALL` reaches second — an ordering
            // the user cannot see. The studio already has a test asserting its
            // own chords are unique; a keymap file has to meet the same bar.
            let mut taken: Vec<(Command, crate::command::Chord)> = Vec::new();
            for command in Command::ALL {
                let chord = match resolved.get(&command) {
                    Some(over) => over.clone(),
                    None => command.chord(),
                };
                let Some(chord) = chord else { continue };
                if let Some((other, _)) = taken.iter().find(|(_, existing)| *existing == chord) {
                    problems.push(format!(
                        "{} wants {}, which {} already has",
                        command.title(),
                        chord.describe(self.host),
                        other.title()
                    ));
                    resolved.remove(&command);
                    continue;
                }
                taken.push((command, chord));
            }
            self.keymap = Rc::new(resolved);
        }

        if !problems.is_empty() {
            // Said once, on the way in, with the count — a keymap file where
            // one line silently did nothing is a file people keep editing
            // without understanding why.
            self.notify(&format!(
                "{} problem{} in your customisation files: {}",
                problems.len(),
                if problems.len() == 1 { "" } else { "s" },
                problems.join("; ")
            ));
        }
    }

    /// Write the three starting-point files, and say where they are.
    ///
    /// The difference between "themes are supported" and "themes are supported
    /// and here is how". Existing files are never overwritten.
    pub fn write_customisation_templates(&self) {
        let dir = Self::customise_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            self.notify("Could not create the settings directory");
            return;
        }
        let current: Vec<(String, String)> = Command::ALL
            .into_iter()
            .filter_map(|command| {
                self.chord_for(command)
                    .map(|chord| (command.title().to_owned(), chord.describe(self.host)))
            })
            .collect();

        let files: [(&str, String); 3] = [
            (
                crate::customise::THEME_FILE,
                crate::customise::Theme::template(),
            ),
            (
                crate::customise::SNIPPETS_FILE,
                crate::customise::snippets_template(),
            ),
            (
                crate::customise::KEYMAP_FILE,
                crate::customise::keymap_template(&current),
            ),
        ];

        let mut written = 0;
        for (name, contents) in files {
            let path = dir.join(name);
            // Never overwritten: these are files people put work into.
            if path.exists() {
                continue;
            }
            if std::fs::write(&path, contents).is_ok() {
                written += 1;
            }
        }
        self.notify(&format!(
            "{written} template{} written to {}",
            if written == 1 { "" } else { "s" },
            dir.display()
        ));
    }

    /// Read both records back. Called once, by `main`, after services are in.
    pub fn load_persisted(&self) {
        let Some(store) = self.storage() else { return };
        if let Ok(Some(text)) = store.get(crate::settings::SETTINGS_KEY) {
            self.apply_settings(&crate::settings::Settings::parse(&text));
        }
        if let Ok(Some(text)) = store.get(crate::settings::SESSION_KEY) {
            self.apply_session(&crate::settings::Session::parse(&text));
        }
    }

    /// The stored session, or an empty one. For `main`, which needs the window
    /// size and the last root *before* there is a `Studio` to put them in.
    #[must_use]
    pub fn stored_session(
        store: Option<&Rc<dyn vieww_foundation::Storage>>,
    ) -> crate::settings::Session {
        store
            .and_then(|store| store.get(crate::settings::SESSION_KEY).ok().flatten())
            .map(|text| crate::settings::Session::parse(&text))
            .unwrap_or_default()
    }

    /// Note that `root` was opened, so the welcome view can offer it again.
    pub fn remember_workspace(&self, root: &std::path::Path) {
        // Canonicalised so `~/app` and `~/app/` are one entry rather than two.
        // A path that cannot be canonicalised — a directory removed between
        // the click and here — is stored as given rather than dropped.
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let mut recent = (*self.recent.get()).clone();
        recent.retain(|existing| existing != &root);
        recent.insert(0, root);
        recent.truncate(crate::settings::RECENT_LIMIT);
        self.recent.set(Rc::new(recent));
        self.save_session(None);
    }

    /// Every unsaved buffer, named the way the quit dialog lists them.
    #[must_use]
    pub fn unsaved_names(&self) -> Vec<String> {
        self.buffers
            .get()
            .iter()
            .filter(|buffer| buffer.dirty)
            .map(|buffer| buffer.name.clone())
            .collect()
    }

    /// Answer a close request. `true` means "let the window go".
    ///
    /// Called from `main`'s close hook, which is the only place that can veto
    /// one. Three outcomes, and the order matters: an already-confirmed quit
    /// goes straight through (otherwise the dialog's own "Close anyway" would
    /// raise the dialog again), a clean studio goes through after writing its
    /// state, and a dirty one is stopped with the list of what would be lost.
    pub fn may_close(&self, window: Option<(f32, f32)>) -> bool {
        if self.quit_confirmed.get() {
            self.save_all_state(window);
            // A shutdown the user decided on is not a crash. Leaving the
            // recovery files behind would offer to restore this session's work
            // at the start of the next one, every time.
            crate::recovery::clear();
            return true;
        }
        let unsaved = self.unsaved_names();
        if unsaved.is_empty() {
            self.save_all_state(window);
            crate::recovery::clear();
            return true;
        }
        self.quit_prompt.set(Some(Rc::new(unsaved)));
        false
    }

    /// The quit dialog's "Save all and close".
    pub fn confirm_quit_saving(&self) {
        self.save_all();
        self.quit_confirmed.set(true);
        self.quit_prompt.set(None);
    }

    /// The quit dialog's "Close without saving".
    pub fn confirm_quit_discarding(&self) {
        self.quit_confirmed.set(true);
        self.quit_prompt.set(None);
    }

    /// The quit dialog's "Cancel".
    pub fn cancel_quit(&self) {
        self.quit_prompt.set(None);
    }

    /// Ask for the window to close. Drained by `main`'s frame hook.
    pub fn request_close(&self) {
        self.close_requested.set(true);
        vieww_foundation::task::FrameWaker::wake(&self.waker);
    }

    /// Take the pending close request, if there is one.
    pub fn take_close_request(&self) -> bool {
        self.close_requested.replace(false)
    }

    /// Write the settings out if they have changed since the last check.
    ///
    /// Called once a frame. The comparison is a `PartialEq` on a twenty-one
    /// field record of `Copy` scalars and one short `String`, which is cheaper
    /// than the signal reads that built it — and both together are far cheaper
    /// than the alternative, which is every one of twenty-one controls
    /// remembering to save. A toggle that saves is a toggle nobody has to wire.
    pub fn poll_settings(&self, last: &RefCell<Option<crate::settings::Settings>>) {
        let now = self.settings();
        let mut last = last.borrow_mut();
        if last.as_ref() == Some(&now) {
            return;
        }
        let first = last.is_none();
        *last = Some(now);
        // The first call is the baseline, not a change: writing here would
        // stamp the file on every launch even when nothing was touched.
        if !first {
            self.save_settings();
        }
    }

    // ----- Crash recovery ------------------------------------------------

    /// Write the unsaved buffers out, if enough has changed to be worth it.
    ///
    /// Called from the same frame hook as [`poll_settings`](Self::poll_settings)
    /// and shaped the same way: a cheap comparison on the frame where nothing
    /// moved, a write on the one where something did.
    ///
    /// # The two guards, and why both
    ///
    /// **Time**, so a fast typist does not turn every keystroke into a file
    /// write — [`RECOVERY_INTERVAL`] is the longest amount of work a crash can
    /// now cost. **Content**, so a studio left open overnight with a dirty
    /// buffer nobody is touching does not rewrite the same bytes every few
    /// seconds for eight hours.
    ///
    /// A buffer that becomes clean has its recovery file removed rather than
    /// left behind: the file on disk is the text now, and a stale recovery file
    /// is an offer to restore something the user already has.
    pub fn poll_recovery(&self, last: &RefCell<Option<std::time::Instant>>) {
        let now = std::time::Instant::now();
        {
            let mut last = last.borrow_mut();
            match *last {
                Some(at) if now.duration_since(at) < RECOVERY_INTERVAL => return,
                _ => *last = Some(now),
            }
        }

        let mut written = (*self.recovered_state.borrow()).clone();
        let mut problems: Vec<String> = Vec::new();
        let mut live: Vec<Option<std::path::PathBuf>> = Vec::new();
        for buffer in self.buffers.peek().iter() {
            if !buffer.dirty {
                continue;
            }
            live.push(buffer.path.clone());
            if written.get(&buffer.path) == Some(&buffer.value.text) {
                continue;
            }
            match crate::recovery::write(buffer.path.as_deref(), &buffer.value.text) {
                Ok(()) => {
                    written.insert(buffer.path.clone(), buffer.value.text.clone());
                }
                Err(error) => problems.push(format!("recovery file for {}: {error}", buffer.name)),
            }
        }
        // Anything written earlier that is no longer dirty — saved, reverted or
        // closed — stops being offered.
        written.retain(|origin, _| {
            if live.contains(origin) {
                return true;
            }
            crate::recovery::forget(origin.as_deref());
            false
        });
        *self.recovered_state.borrow_mut() = written;
        if !problems.is_empty() {
            self.append_output(problems);
        }
    }

    /// Put back what a previous run did not get to save.
    ///
    /// Called once, at startup. Restores each recovered text into the buffer it
    /// belongs to — opening the file first if the session did not — and leaves
    /// it **dirty**, which is the honest state: this text is not what is on
    /// disk, and the user decides whether it should be.
    ///
    /// Nothing is restored silently. The Output panel gets a line per file and
    /// the status area a summary, because a buffer that quietly differs from
    /// the file it names is the same defect the conflict dialog exists to
    /// prevent.
    pub fn restore_recovered(&self) {
        let pending: Vec<crate::recovery::Recovered> = crate::recovery::pending()
            .into_iter()
            .filter(crate::recovery::differs_from_disk)
            .collect();
        if pending.is_empty() {
            // Nothing to restore, but there may be files describing work that
            // has since been saved. They have done their job.
            crate::recovery::clear();
            return;
        }

        let mut lines = vec![format!(
            "Recovered {} unsaved file{} from a previous session:",
            pending.len(),
            if pending.len() == 1 { "" } else { "s" }
        )];
        let mut names: Vec<String> = Vec::new();
        for item in pending {
            if let Some(origin) = item.origin.clone() {
                self.open_path(origin.clone());
            }
            let mut buffers = (*self.buffers.get()).clone();
            let index = buffers
                .iter()
                .position(|buffer| buffer.path == item.origin)
                .or_else(|| item.origin.is_none().then_some(0));
            let Some(index) = index else { continue };
            let Some(buffer) = buffers.get_mut(index) else {
                continue;
            };
            let mut value = buffer.value.clone();
            value.text.clone_from(&item.text);
            value.selection = vieww_foundation::TextSelection::collapsed(0);
            value.secondary.clear();
            buffer.value = value;
            buffer.dirty = true;
            buffer.history.reset(buffer.value.clone());
            self.put_buffers(buffers);
            lines.push(format!("  {} — unsaved, review before saving", item.name));
            names.push(item.name);
        }
        crate::recovery::clear();
        self.append_output(lines);
        self.notify(&format!("Recovered unsaved work: {}", names.join(", ")));
    }
}
