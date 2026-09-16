//! Running things: commands, the preview, and the palette.
//!
//! Everything the user starts and then watches.
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
    // ----- Running a command --------------------------------------------

    /// How many commands the Run box remembers.
    pub const RUN_HISTORY: usize = 12;

    /// Start a command in the workspace and stream its output to the panel.
    ///
    /// # This is not a terminal, and saying so is the point
    ///
    /// The prototype gave the bottom panel a Terminal tab. A terminal is a PTY:
    /// a pseudo-terminal pair, a line discipline, escape-sequence parsing, a
    /// grid model, and a keyboard path that can send a `SIGINT`. None of that
    /// is in this repository and none of it is a weekend.
    ///
    /// What *is* in this repository is a task queue that starts child
    /// processes, streams their output line by line, and cancels them —
    /// `crate::task` and `crate::jobs`, already carrying every `cargo` and
    /// `rustfmt` the studio runs. This is that, with the command coming from
    /// the user instead of from a button.
    ///
    /// So: `cargo test`, `git log --oneline`, `ls`, a script — all fine, and
    /// all visible in the Output panel with a cancel on them in Tasks.
    /// Anything interactive — a REPL, `vim`, a password prompt, anything that
    /// wants a `SIGINT` — is not, and the refusal below says which is which
    /// rather than hanging on a process waiting for input that cannot arrive.
    pub fn run_command(&self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let Some(dir) = self.root.peek().clone() else {
            self.notify("Running a command needs a folder open");
            return;
        };

        // Split on whitespace, honouring double quotes so a path with a space
        // in it is one argument. Not a shell: there is no globbing, no pipes,
        // no redirection and no variable expansion, because implementing a
        // quarter of a shell is how a studio ends up with a command line that
        // works until it does not.
        let Some(parts) = split_command(line) else {
            self.notify("Unbalanced quotes in that command");
            return;
        };
        let Some((program, args)) = parts.split_first() else {
            return;
        };
        if INTERACTIVE.contains(&program.as_str()) {
            self.notify(&format!(
                "{program} wants a terminal, and this is not one \u{2014} run it outside the studio"
            ));
            return;
        }

        let mut spec = crate::task::Spec::new(format!("Run: {line}"), program.clone(), dir);
        spec.args = args.to_vec();
        // Colour codes would arrive as escape sequences and be drawn literally
        // — the same reason the build sets it.
        spec.env
            .push(("CARGO_TERM_COLOR".to_owned(), "never".to_owned()));

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::None, wake);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));

        // Remembered before the panel opens, so the history is right even if
        // the command fails immediately.
        let mut history = (*self.run_history.get()).clone();
        history.retain(|existing| existing != line);
        history.insert(0, line.to_owned());
        history.truncate(Self::RUN_HISTORY);
        self.run_history.set(Rc::new(history));

        self.panel_open.set(true);
        self.panel_tab.set(PanelTab::Output);
    }

    /// Run whatever the Run box holds.
    pub fn run_typed_command(&self) {
        let line = self.run_input.get();
        self.run_command(&line);
    }

    /// Restart the language server.
    ///
    /// # Why this is not just "toggle twice"
    ///
    /// The client could start a server and kill one, and had no way to bring a
    /// dead one back. `rust-analyzer` is OOM-killed on large projects and
    /// crashes on some macro expansions, and when it went the studio said
    /// nothing: diagnostics simply stopped updating, and the last set stayed on
    /// screen looking current. Stale diagnostics presented as live are worse
    /// than none — the user trusts a red underline that is describing code they
    /// have already fixed.
    ///
    /// Stopping first and starting unconditionally is what makes this usable as
    /// the answer to "it has gone quiet", which is the only symptom the user
    /// has to go on.
    pub fn restart_analyzer(&self) {
        let was_running = self.analyzer.borrow().is_some();
        if was_running {
            *self.analyzer.borrow_mut() = None;
            self.analyzer_state.set("off");
        }
        // `toggle_analyzer` now sees a stopped client and starts one.
        self.toggle_analyzer();
        if was_running {
            self.notify("rust-analyzer restarted.");
        }
    }

    /// Open `root` as the workspace: from the picker, a drop, or the welcome
    /// view's recent list.
    ///
    /// The one front door, so that "opened a folder" means the same three
    /// things everywhere — the tree is scanned, the buffers are replaced, and
    /// the workspace goes to the top of the recent list.
    pub fn open_workspace(&self, root: &std::path::Path) {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let workspace = Workspace::open(&root);
        if workspace.root.is_none() {
            self.notify(&format!("{} could not be opened", root.display()));
            return;
        }
        self.adopt_workspace(workspace);
        self.remember_workspace(&root);
    }

    /// The size the preview stage frames, honouring the Devices tab.
    ///
    /// Falls back to the platform's own default, which is what the preview did
    /// before the tab existed — so a studio nobody has visited the tab in
    /// behaves exactly as it always has.
    #[must_use]
    pub fn preview_screen_size(&self) -> (f32, f32) {
        let device = self.preview_device();
        (device.width, device.height)
    }

    /// The device the preview is framing, whether or not one was chosen.
    ///
    /// # Why the platform default is a `Device` too
    ///
    /// Three places resolved "chosen device, else platform default" with their
    /// own `match`, and each applied rotation **inside the `Some` arm only**.
    /// So on a studio nobody had opened the Devices tab in — which is every
    /// studio on its first run — the frame could not be rotated at all, and
    /// `toggle_landscape` covered for it by refusing with "Choose a device
    /// first": a dead end reached by pressing Rotate on a preview that was
    /// already the right size to rotate.
    ///
    /// Making the fallback a `Device` removes the special case rather than
    /// duplicating the rotation into it. Every caller now has one, and
    /// `Device::rotated` is applied once, here.
    #[must_use]
    pub fn preview_device(&self) -> Device {
        let device = self.device.get().unwrap_or_else(|| {
            let platform = self.platform.get();
            let (width, height) = platform.screen();
            Device {
                name: platform.label(),
                width,
                height,
                platform,
                // The platform's own screen is its portrait one; `rotated`
                // below is the only thing that turns it.
                landscape: false,
            }
        });
        if self.landscape.get() {
            device.rotated()
        } else {
            device
        }
    }

    /// The metrics the previewed screen is *told*, honouring the Devices tab.
    ///
    /// # Why this is not just `platform.view_metrics()`
    ///
    /// `preview_screen_size` already returns the chosen device's width and
    /// height, but the same code that drew at the chosen size used to publish
    /// `platform.view_metrics()` — the *platform default's* size, DPR and
    /// insets. Pick "Tablet" and the guest laid out at 834×1194 while being
    /// told it was on a 393×852 phone with iPhone insets and DPR 3.0. Rotation
    /// was invisible to `ViewMetrics` for the same reason: the rotated device
    /// and the platform default disagreed.
    ///
    /// This returns metrics whose `size` matches [`preview_screen_size`], whose
    /// DPR follows the chosen device's platform, and whose insets are the
    /// platform's — there is no per-device inset table, and a 360-wide phone
    /// has the same notch as a 430-wide one.
    ///
    /// [`preview_screen_size`]: Self::preview_screen_size
    #[must_use]
    pub fn preview_view_metrics(&self) -> vieww_foundation::ViewMetrics {
        self.preview_device().view_metrics()
    }

    /// Choose a device, and follow it to its platform.
    ///
    /// Choosing "Android tablet" and leaving the previewed tree being told it
    /// is on iOS would be a frame that is the right shape and the wrong
    /// everything-else — `ThemeData::adaptive` branches on the platform, and so
    /// does anything in the screen that asks.
    pub fn choose_device(&self, device: Device) {
        self.device.set(Some(device));
        self.set_platform(device.platform);
        self.notify(&format!("Preview framed at {}", device.size_label()));
    }

    /// Back to the platform's own default size.
    pub fn clear_device(&self) {
        self.device.set(None);
        self.landscape.set(false);
    }

    /// Turn the chosen device on its side.
    /// Turn the previewed device on its side.
    ///
    /// **No longer refuses without a chosen device.** It used to answer
    /// "Choose a device first", because the two functions that resolve the
    /// frame only rotated inside their `Some(device)` arm — so on a studio
    /// whose Devices tab had never been opened, rotating genuinely did nothing
    /// and the message was the honest report of a gap. `preview_device` closes
    /// the gap: the platform's own default is a `Device` like any other, and
    /// rotates like one.
    pub fn toggle_landscape(&self) {
        self.landscape.set(!self.landscape.get());
        let device = self.preview_device();
        self.notify(&format!(
            "{} · {}",
            if self.landscape.peek() {
                "Landscape"
            } else {
                "Portrait"
            },
            device.size_label()
        ));
    }

    /// The multiplier every chrome text size is drawn at.
    ///
    /// One for the ordinary case, so the arithmetic is a no-op on the frames
    /// nobody has asked for larger text.
    #[must_use]
    pub fn ui_scale(&self) -> f32 {
        if self.large_ui.get() {
            LARGE_UI_SCALE
        } else {
            1.0
        }
    }

    /// Say something in the status bar.
    pub fn notify(&self, message: &str) {
        self.notice.set(Some(message.to_owned()));
    }

    /// Put the sample screen back in the active buffer.
    ///
    /// The one buffer in the application guaranteed to compile and render, and
    /// until this existed, emptying it was irreversible past the end of the
    /// undo stack — which is exactly what happens in the screencast, at which
    /// point nothing the user can do gets a render back.
    ///
    /// Goes through [`Studio::edit`], so it is one undo step like any other
    /// rather than a state change that steps around the history.
    pub fn restore_sample(&self) {
        let Some(buffer) = self.active() else { return };
        let mut value = buffer.value.clone();
        value.text = crate::buffer::SCRATCH.to_owned();
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        value.secondary.clear();
        value.composing = None;
        self.edit(value);
        self.notify("Sample buffer restored. Press Render.");
    }

    /// Put a lesson's source in the active buffer, ready to Render.
    ///
    /// Goes through [`Studio::edit`], like [`Self::restore_sample`], so loading
    /// a lesson is one undo step rather than a state change that steps around
    /// the history. The lesson replaces whatever is in the buffer — same
    /// contract as Restore Sample, because a lesson is a starting point
    /// rather than an insertion.
    pub fn load_lesson(&self, lesson: &crate::lessons::Lesson) {
        // **Say first, Rust a click away.** A lesson with a Say body loads
        // the Say one: the language a newcomer should meet is the lesson's
        // teaching surface, and the Rust it becomes stays under the
        // Generated Rust tab. The Rust body stays one row behind it.
        match lesson.say_body {
            Some(say_body) => {
                self.load_lesson_body(lesson, say_body, crate::language::Language::Say, "say");
            }
            None => self.load_lesson_rust(lesson),
        }
    }

    /// Force the lesson's Rust body, for the link under a Say lesson.
    pub fn load_lesson_rust(&self, lesson: &crate::lessons::Lesson) {
        self.load_lesson_body(lesson, lesson.body, crate::language::Language::Rust, "rs");
    }

    fn load_lesson_body(
        &self,
        lesson: &crate::lessons::Lesson,
        text: &str,
        language: crate::language::Language,
        extension: &str,
    ) {
        let Some(buffer) = self.active() else { return };
        // The buffer's language follows the body, and the tab is renamed to
        // match — a Say lesson in a buffer named `scratch.rs` would run
        // rustc against English. The name is the lesson slug, so every
        // lesson lands under a tab that says what it is.
        let slug = lesson
            .name
            .to_ascii_lowercase()
            .replace([' ', ':', ','], "_");
        let index = self.active_buffer.peek();
        let mut buffers = (*self.buffers.peek()).clone();
        if let Some(slot) = buffers.get_mut(index) {
            slot.name = format!("{slug}.{extension}");
            slot.language = language;
        }
        self.put_buffers(buffers);
        let mut value = buffer.value.clone();
        value.text = text.to_owned();
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        value.secondary.clear();
        value.composing = None;
        self.edit(value);
        let which = if language == crate::language::Language::Say {
            " in Say — the Rust it compiles to is under Generated Rust"
        } else {
            " in Rust"
        };
        self.notify(&format!("Loaded “{}”{which}. Press Render.", lesson.name));
    }

    // ----- live preview ---------------------------------------------------

    /// Accept the live-preview caution and mount the demo.
    pub fn accept_live_preview(&self) {
        self.live_caution.set(false);
        self.live_preview.set(true);
        self.notify("Live preview: drawing live.rs. Render compiles a real screen.");
    }

    /// What the Live Preview should draw right now.
    ///
    /// # Read on every frame, on purpose
    ///
    /// The buffer comes first and the file second. A `live.rs` that is *open in
    /// the editor* is previewed from the text in the editor, so the frame
    /// changes as it is typed and there is nothing to save and no build to wait
    /// for — which is the whole difference between this and `Render`. A
    /// `live.rs` that is not open is read from disk, so the preview still works
    /// on a project whose file nobody has opened.
    ///
    /// Parsing is a scan over a few dozen short lines; doing it per frame costs
    /// less than deciding whether to.
    #[must_use]
    pub fn live_source(&self) -> crate::live::LiveSource {
        use crate::live::LiveSource;

        let Some(root) = self.root.peek().clone() else {
            return LiveSource::Missing;
        };
        let path = crate::livedoc::path_in(&root);

        let open = (*self.buffers.get())
            .iter()
            .find(|buffer| buffer.path.as_deref() == Some(path.as_path()))
            .map(|buffer| buffer.value.text.clone());

        let source = match open {
            Some(text) => text,
            None => match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(_) => return LiveSource::Missing,
            },
        };

        match crate::livedoc::parse(&source) {
            Ok(doc) => LiveSource::Doc(doc),
            Err(error) => LiveSource::Broken(error),
        }
    }

    /// The screen a `mount "…"` names, if it has been rendered this session.
    ///
    /// Resolved against the workspace root, so `live.rs` names files the way the
    /// Explorer shows them.
    #[must_use]
    pub fn live_mount(&self, relative: &str) -> Option<crate::loaded::Preview> {
        let root = self.root.peek().clone()?;
        let path = root.join(relative);
        let mounts = self.live_mounts.borrow();
        mounts
            .get(&path)
            .or_else(|| {
                // Also by file name, so `mount "card_grid.rs"` finds
                // `screens/card_grid.rs` — the flow file should not have to
                // repeat the folder layout to be useful.
                mounts.iter().find_map(|(rendered, preview)| {
                    (rendered.file_name() == std::path::Path::new(relative).file_name())
                        .then_some(preview)
                })
            })
            .cloned()
    }

    /// Every `mount` in `source`, resolved to a rendered screen where there is
    /// one. Keyed by the string the file wrote, so the frame can name the ones
    /// it could not resolve.
    #[must_use]
    pub fn live_mounts_for(
        &self,
        source: &crate::live::LiveSource,
    ) -> std::collections::HashMap<String, crate::loaded::Preview> {
        let mut resolved = std::collections::HashMap::new();
        if let crate::live::LiveSource::Doc(doc) = source {
            for screen in &doc.screens {
                for item in &screen.items {
                    if let crate::livedoc::LiveItem::Mount(file) = item {
                        if let Some(preview) = self.live_mount(file) {
                            resolved.insert(file.clone(), preview);
                        }
                    }
                }
            }
        }
        resolved
    }

    /// Whether this workspace has a `live.rs` at all.
    #[must_use]
    pub fn has_live_file(&self) -> bool {
        self.root
            .peek()
            .as_ref()
            .is_some_and(|root| crate::livedoc::path_in(root).exists())
    }

    /// Write the starter `live.rs` and open it.
    ///
    /// Never overwrites: a file that is already there is opened instead, which
    /// is what somebody who pressed this twice meant both times.
    pub fn create_live_file(&self) {
        let Some(root) = self.root.peek().clone() else {
            self.notify("Open a folder first — live.rs goes at the workspace root.");
            return;
        };
        let path = crate::livedoc::path_in(&root);
        if !path.exists() {
            if let Err(error) = std::fs::write(&path, crate::livedoc::TEMPLATE) {
                self.notify(&format!("Could not write live.rs: {error}"));
                return;
            }
            self.rescan_tree();
        }
        self.open_path(path);
        self.notify("live.rs is open. The Live Preview draws it as you type.");
    }

    /// Cancel the live-preview caution dialog without mounting.
    pub fn cancel_live_preview(&self) {
        self.live_caution.set(false);
    }

    /// Exit the live preview and return to the compiled screen.
    pub fn exit_live_preview(&self) {
        self.live_preview.set(false);
    }

    /// Put the sample `flow.rs` content in the active buffer, ready to click
    /// Render and trigger the live preview.
    pub fn restore_flow(&self) {
        let Some(buffer) = self.active() else { return };
        let mut value = buffer.value.clone();
        value.text = crate::buffer::FLOW_SCRATCH.to_owned();
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        value.secondary.clear();
        value.composing = None;
        self.edit(value);
        self.notify("Flow file loaded. Click Render to start the Live Preview.");
    }
}
