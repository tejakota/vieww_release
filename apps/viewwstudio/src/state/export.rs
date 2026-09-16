//! Export, formatting and scaffolding a new project.
//!
//! The three things that produce something outside the workspace.
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
    // ----- N5: export --------------------------------------------------------

    /// Start exporting in `format`, or say why not.
    ///
    /// # Why the refusal comes first
    ///
    /// [`export::plan`](crate::export::plan) asks the toolchain checklist
    /// before it builds a plan, so a missing NDK is reported as a missing NDK
    /// with the command that installs it — rather than as whatever `cargo
    /// ndk` says about a linker three minutes into a build that was never
    /// going to work.
    pub fn start_export(&self, format: crate::export::Format) {
        if self.export_run.borrow().is_some() {
            self.append_output(vec![
                "an export is already running — cancel it first".to_string()
            ]);
            return;
        }
        let Some(root) = self.root.get().as_ref().cloned() else {
            self.append_output(vec![crate::export::Refusal::NoProject.to_string()]);
            self.show_output();
            return;
        };
        let name = self.package_name().unwrap_or_default();

        match crate::export::plan(&self.build_env, &root, &name, format) {
            Err(refusal) => {
                self.append_output(vec![format!("{}: {refusal}", format.title())]);
                self.show_output();
            }
            Ok(plan) => {
                self.append_output(vec![format!(
                    "{} — {} step(s)",
                    format.title(),
                    plan.steps.len()
                )]);
                // The whole plan, before it starts. An export is minutes of
                // child processes and being able to read what it is about to
                // do is the difference between watching it and waiting on it.
                for (index, step) in plan.steps.iter().enumerate() {
                    self.append_output(vec![format!(
                        "  {}. {} — {}",
                        index + 1,
                        step.spec.command_line(),
                        step.why
                    )]);
                }
                *self.export_run.borrow_mut() = Some(ExportRun { plan, next: 0 });
                self.show_output();
                self.advance_export(true);
            }
        }
    }

    /// Run the next step of the export, or finish it.
    ///
    /// `ok` is the previous step's verdict. A failed step ends the plan: a
    /// package assembled from a compile that failed is worse than no package,
    /// and worse still because it looks like one that worked.
    pub(super) fn advance_export(&self, ok: bool) {
        let next = {
            let mut slot = self.export_run.borrow_mut();
            let Some(run) = slot.as_mut() else {
                return;
            };
            if !ok {
                let step = run.next;
                let plan = run.plan.clone();
                *slot = None;
                drop(slot);
                self.append_output(vec![format!(
                    "{} stopped at step {step} — nothing was produced",
                    plan.format.title()
                )]);
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
                return;
            }
            let index = run.next;
            run.next += 1;
            run.plan.steps.get(index).map(|step| step.spec.clone())
        };

        match next {
            Some(spec) => {
                let wake = self.frame_waker();
                self.jobs
                    .borrow_mut()
                    .start(spec, crate::jobs::Follow::ExportStep, wake);
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
            }
            None => {
                let plan = self.export_run.borrow_mut().take().map(|run| run.plan);
                if let Some(plan) = plan {
                    self.append_output(vec![format!(
                        "{} finished — {}",
                        plan.format.title(),
                        plan.artefact.display()
                    )]);
                }
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
            }
        }
    }

    /// Stop an export between steps, and cancel the one that is running.
    pub fn cancel_export(&self) {
        if self.export_run.borrow_mut().take().is_some() {
            self.jobs.borrow_mut().cancel_all();
            self.append_output(vec!["export cancelled".to_string()]);
            self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        }
    }

    /// Whether an export is part-way through.
    #[must_use]
    pub fn exporting(&self) -> bool {
        self.export_run.borrow().is_some()
    }

    /// Ask `adb` what is plugged in.
    ///
    /// Android only, and it says so: `adb` is the one device list this studio
    /// can read without a platform-specific tool it may not have. An iOS
    /// device list is `xcrun devicectl`, which exists only on macOS — so a
    /// scan on Linux that pretended to cover both would report "no devices"
    /// for a reason that had nothing to do with what was connected.
    pub fn scan_devices(&self) {
        let root = self
            .root
            .get()
            .as_ref()
            .cloned()
            .unwrap_or_else(std::env::temp_dir);
        let wake = self.frame_waker();
        self.jobs.borrow_mut().capture(
            crate::export::adb_devices(&root),
            crate::jobs::Follow::Listing,
            wake,
        );
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Turn a finished `adb devices` into the device list.
    pub(super) fn read_device_listing(&self, id: u64) {
        let output = self
            .jobs
            .borrow()
            .job(id)
            .map(crate::jobs::Job::captured)
            .unwrap_or_default();
        let devices = crate::export::parse_adb_devices(&output);
        self.append_output(vec![format!("{} device(s) connected", devices.len())]);
        self.devices.set(Rc::new(devices));
        self.devices_scanned.set(true);
    }

    /// Install the last export's artefact on `device`.
    pub fn install_on(&self, device: &crate::export::Device, artefact: &Path) {
        match crate::export::install(
            self.root
                .get()
                .as_ref()
                .map_or(Path::new("."), |root| root.as_path()),
            device,
            artefact,
        ) {
            Ok(spec) => {
                let wake = self.frame_waker();
                self.jobs
                    .borrow_mut()
                    .start(spec, crate::jobs::Follow::None, wake);
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
            }
            Err(refusal) => {
                self.append_output(vec![refusal.to_string()]);
                self.show_output();
            }
        }
    }

    /// The open project's compiled library, if this buffer refers to it.
    ///
    /// `None` when the buffer does not use `crate::` or `super::` at all, which
    /// is the bundled-screens case and every standalone snippet.
    #[must_use]
    pub(super) fn project_lib(&self, source: &str) -> Option<crate::compile::ProjectLib> {
        if !Self::needs_project(source) {
            return None;
        }
        let root = self.root.peek().clone()?;
        let name = self.package_name()?;
        let studio_deps = self
            .toolchain
            .as_ref()
            .as_ref()
            .ok()
            .map(|toolchain| toolchain.deps.clone())?;
        crate::compile::ProjectLib::discover(&root, &name, &studio_deps)
    }

    /// Whether this buffer reaches into the crate around it.
    ///
    /// # A heuristic, and one with a known false positive
    ///
    /// It reads hand-written Rust, where `use super::` means "this file reaches
    /// into its parent module in the user's own crate". Say's code generator
    /// emits `use super::SayState;` as well, and *that* `super` is the
    /// generated file's own root module — nothing to do with the project. So
    /// this answers `true` for every Say screen ever written, and every caller
    /// on the Say path has to say so; see `Studio::render_preview`, which does,
    /// and which for one release did it for the lookup and not for the refusal.
    pub(super) fn needs_project(source: &str) -> bool {
        source.contains("crate::") || source.contains("use super::")
    }

    /// The diagnostic for "this file needs the project and the project has not
    /// been built".
    ///
    /// # Why this is a refusal rather than an implicit build
    ///
    /// Rendering is the fast half of this studio — half a second, on a
    /// keystroke. `cargo build` on a project the size of the one this was
    /// written against is twelve minutes the first time. Starting one because
    /// somebody pressed Render would turn the responsive action into the slow
    /// one with no warning, so the studio says what is missing and leaves the
    /// decision where it belongs.
    #[must_use]
    pub(super) fn project_lib_refusal(
        &self,
        source: &str,
        project: Option<&crate::compile::ProjectLib>,
    ) -> Option<Diagnostic> {
        if !Self::needs_project(source) {
            return None;
        }
        let name = self.package_name();
        let buffer = self.active()?;

        // Two different failures, and telling them apart is the whole value of
        // asking here rather than letting rustc try and report the wrong thing.
        let (message, help) = match project {
            None => (
                match name.as_ref() {
                    Some(name) => format!(
                        "this file uses `crate::`, so the preview needs {name}'s own library built"
                    ),
                    None => "this file uses `crate::`, but no cargo project is open".to_owned(),
                },
                "Render compiles this one file and links it against the project around it. Run Build once, or `cargo build` in the project, so there is a library to link — and Render will work from then on without rebuilding it."
                    .to_owned(),
            ),
            // Built, and built against a different vieww. Linking it fails as
            // `can't find crate for <project>`, which names the wrong crate and
            // explains nothing — see `ProjectLib::vieww_rlib`.
            Some(project) if !self.shares_vieww(project) => {
                let name = name.unwrap_or_else(|| "this project".to_owned());
                let target = self
                    .toolchain
                    .as_ref()
                    .as_ref()
                    .ok()
                    .and_then(|toolchain| toolchain.deps.parent())
                    .map_or_else(
                        || "<the studio's target/debug>".to_owned(),
                        |dir| dir.display().to_string(),
                    );
                (
                    format!(
                        "{name} was built against a different build of vieww than the preview uses"
                    ),
                    format!(
                        "The preview loads the compiled screen into this window, so the screen, the project and the studio all have to be one build of vieww rather than three copies of the same source. {name} has its own target directory, so its vieww is a separate compilation.\n\nThe way that holds: make {name} a member of the same cargo workspace as the studio, so cargo compiles vieww once and every crate in the graph shares it.\n\nSetting CARGO_TARGET_DIR={target} also unifies them, and leaves the project's own vieww builds in that directory afterwards, where they compete with the studio's — so prefer the workspace.\n\nBuild and Run are unaffected; only the live preview has this relationship with the studio."
                    ),
                )
            }
            Some(_) => return None,
        };

        Some(Diagnostic {
            file: buffer.name,
            severity: Severity::Error,
            code: "project-lib".to_owned(),
            message,
            help: Some(help),
            // The first line, not the whole file: a range spanning the buffer
            // draws a squiggle under every line of it, which says "everything
            // here is wrong" about a file whose only problem is that something
            // outside it has not been built.
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        })
    }

    /// Whether the project's library and the preview would load the same
    /// compilation of vieww.
    ///
    /// Compared by *path*, which is the only comparison available before
    /// compiling and the right one in practice: cargo writes one rlib per
    /// compilation, so the same file is the same build and two files are two
    /// builds.
    pub(super) fn shares_vieww(&self, project: &crate::compile::ProjectLib) -> bool {
        let Ok(toolchain) = self.toolchain.as_ref() else {
            return false;
        };
        // A project with no vieww in its deps is not reaching for one; leave it
        // to rustc, which will say something sensible about whatever it is.
        project.vieww_rlib.as_ref().is_none_or(|theirs| {
            std::fs::canonicalize(theirs).ok() == std::fs::canonicalize(&toolchain.vieww_rlib).ok()
        })
    }

    /// The package's name, from the open workspace's `Cargo.toml`.
    ///
    /// Read from the file rather than from the directory's name: those differ
    /// often enough that naming an artefact after the folder is a coin flip,
    /// and the artefact's name is what somebody double-clicks.
    #[must_use]
    pub fn package_name(&self) -> Option<String> {
        let root = self.root.get().as_ref().cloned()?;
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
        crate::export::package_name(&manifest)
    }

    /// Open the Output panel. What every refusal here does, because a refusal
    /// nobody sees is a button that did nothing.
    pub(super) fn show_output(&self) {
        self.panel_tab.set(PanelTab::Output);
        self.panel_open.set(true);
    }

    /// A closure that asks the platform for a frame from a worker thread.
    pub(super) fn frame_waker(&self) -> impl Fn() + Send + 'static {
        let waker = self.waker.clone();
        move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        }
    }

    // ----- Formatting -------------------------------------------------------

    /// Run `rustfmt` over the active buffer.
    ///
    /// # Why a scratch file rather than a pipe
    ///
    /// `rustfmt` reads a file and writes it back; formatting through stdin
    /// needs `--emit stdout`, and [`Task`](crate::task) reports *lines* rather
    /// than bytes, so reassembling a file from it would guess at the trailing
    /// newline — the one byte a formatter is most opinionated about. Writing
    /// the buffer to a scratch file, formatting it in place and reading it
    /// back has no such guess in it.
    ///
    /// The scratch file, rather than the real one, so that formatting a buffer
    /// never touches the file on disk. Format is an *edit*: it goes through
    /// the undo history like any other, and a file only changes when it is
    /// saved.
    ///
    /// # Nothing is assumed about the toolchain
    ///
    /// `rustfmt` may not be installed. That arrives as an ordinary
    /// [`Status::NotStarted`](crate::task::Status::NotStarted) in the Output
    /// panel rather than as a check here, which is the same path every other
    /// missing tool takes — and the Toolchains view is where a checklist
    /// belongs.
    pub fn format_active(&self) {
        let index = self.active_buffer.get();
        let Some(buffer) = self.active() else {
            return;
        };
        // A file rustfmt would refuse is a file not worth spawning a process
        // for, and the refusal it prints is about a parse error rather than
        // about the extension, which reads as a bug in the buffer.
        if !buffer.name.ends_with(".rs") {
            self.append_output(vec![format!(
                "rustfmt formats Rust; {} is not a .rs file",
                buffer.name
            )]);
            return;
        }

        let scratch =
            std::env::temp_dir().join(format!("viewwstudio-fmt-{}-{index}.rs", std::process::id()));
        if let Err(error) = std::fs::write(&scratch, &buffer.value.text) {
            self.append_output(vec![format!("could not write a scratch file: {error}")]);
            return;
        }

        let spec = crate::task::Spec::new(
            "rustfmt",
            "rustfmt",
            scratch
                .parent()
                .map_or_else(std::env::temp_dir, Path::to_path_buf),
        )
        // Named explicitly: rustfmt defaults to the 2015 edition when it is
        // not run through cargo, and 2015 cannot parse `async` or `dyn` as
        // keywords — so an unedition'd format of a modern file fails with a
        // syntax error in code that compiles.
        .args(["--edition", "2021"])
        .arg(scratch.to_string_lossy().to_string())
        .timeout(std::time::Duration::from_secs(30));

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        self.jobs.borrow_mut().start(
            spec,
            crate::jobs::Follow::Formatted {
                buffer: index,
                scratch,
            },
            wake,
        );
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Replace `buffer`'s text with `formatted`, as one undoable edit.
    ///
    /// # The caret
    ///
    /// A formatter moves every byte after the first change, so a caret kept at
    /// its old offset lands somewhere arbitrary. What is kept instead is the
    /// **line**: reformatting rarely moves a statement to a different line,
    /// and a caret that stays on the code it was on is what a person means by
    /// "where I was". The column is clamped to the reformatted line's length.
    pub(super) fn apply_formatting(&self, buffer: usize, formatted: &str) {
        let buffers = self.buffers.get();
        let Some(current) = buffers.get(buffer) else {
            return;
        };
        if current.value.text == formatted {
            // Said out loud. A format that silently did nothing is
            // indistinguishable from a format that silently failed.
            self.append_output(vec!["rustfmt: already formatted".to_string()]);
            return;
        }
        // Only the buffer the job was queued against. Switching tabs during a
        // format must not write the result into whatever is open now.
        if buffer != self.active_buffer.get() {
            let mut all = (*buffers).clone();
            if let Some(target) = all.get_mut(buffer) {
                let line = line_of(&target.value.text, target.value.selection.cursor().offset);
                target.value = reformatted_value(formatted, line);
                target.dirty = true;
            }
            self.put_buffers(all);
            return;
        }

        let line = line_of(&current.value.text, current.value.selection.cursor().offset);
        // Through `edit`, so it lands in the undo history: ⌘Z after a format
        // is the escape hatch for a formatter that did something unwanted, and
        // an edit that bypassed the history would have none.
        self.edit(reformatted_value(formatted, line));
        self.append_output(vec!["rustfmt: 1 file reformatted".to_string()]);
    }

    /// Empty the Output panel — the trash button, which used to do nothing.
    pub fn clear_output(&self) {
        self.output.set(Rc::new(Vec::new()));
    }

    /// Re-read what is installed. The Toolchains view's refresh.
    /// Run one requirement's install command, in the Output panel.
    ///
    /// The command is the `install` string the checklist already showed, run
    /// through a shell and streamed into the panel like every build — so a
    /// failed install is read where every other failure is read, and a slow one
    /// can be cancelled from Tasks.
    pub fn install_in_panel(&self, name: &str, command: &str) {
        let command = strip_backticks(command);

        // **A command that needs root does not go in the panel.** Reported from
        // use: Install beside Gradle streamed apt's "Unable to acquire the dpkg
        // frontend lock, are you root?" into the Output panel and failed with
        // exit code 100. The panel has no terminal and no way to answer a
        // password prompt, so the job could only ever end that way.
        //
        // Handed to the terminal instead of refused, because the user asked for
        // the install and the terminal is where it can actually happen — and
        // said out loud, so the window that opens is expected rather than a
        // surprise. `install_in_terminal` already handles the machine with no
        // terminal application by naming the command to run by hand.
        if crate::setup::needs_root(&command) {
            self.notify(&format!(
                "{command} needs root, which the Output panel cannot give it \u{2014} \
                 opening a terminal instead."
            ));
            self.install_in_terminal(name, &command);
            return;
        }
        let dir = self.root.peek().clone().unwrap_or_else(std::env::temp_dir);
        let spec = crate::setup::quiet_spec(format!("Install {name}"), &command, dir);

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::None, wake);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        self.panel_open.set(true);
        self.panel_tab.set(PanelTab::Output);
        self.notify(&format!("Installing {name}. Watch the Output panel."));
    }

    /// Open the platform's terminal with one requirement's command in it.
    ///
    /// For the commands that ask something — an Android licence, a password, a
    /// system dialog. See [`crate::setup`] for why that is a different thing
    /// from running it in the panel rather than a nicer version of it.
    pub fn install_in_terminal(&self, name: &str, command: &str) {
        let command = strip_backticks(command);
        let dir = self.root.peek().clone().unwrap_or_else(std::env::temp_dir);

        let Some(spec) = crate::setup::terminal_spec(&command, dir) else {
            // Said plainly, with the command, so the fallback is a copy and a
            // paste rather than a dead end.
            self.notify(&format!(
                "No terminal application found. Run this yourself: {command}"
            ));
            return;
        };

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::None, wake);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        self.notify(&format!("Opened a terminal to install {name}."));
    }

    /// Put a requirement's command on the pasteboard.
    ///
    /// The third way, and the one that always works: somebody with a terminal
    /// already open, or a machine whose package manager the studio has no
    /// business guessing at.
    pub fn copy_install_command(&self, command: &str) {
        let command = strip_backticks(command);
        self.copy_text(&command);
        self.notify("Command copied.");
    }

    pub fn rescan_toolchains(&self) {
        self.toolchains
            .set(Rc::new(toolchains::detect(&self.configured_env())));
    }

    /// The machine's environment, with whatever the user typed in Toolchain
    /// laid over it.
    ///
    /// # Why an overlay rather than an edit
    ///
    /// `build_env` is read at launch from the real process — `PATH`, the
    /// variables, the installed Rust targets, the keychain — and it is what a
    /// studio started from a terminal should keep using. The settings are the
    /// answer for the case it cannot cover, which is every launch that did not
    /// come from a shell. Laying one over the other means a typed path wins
    /// where it is given and changes nothing where it is not, and that a blank
    /// field is not the same as a wrong one.
    ///
    /// Everything that asks the machine a question goes through here — the
    /// checklist, the export plan, the build check — so what the checklist
    /// reports and what a build actually uses cannot disagree.
    #[must_use]
    pub fn configured_env(&self) -> toolchains::Env {
        let mut env = (*self.build_env).clone();
        let mut set = |key: &str, value: String| {
            let value = value.trim().to_owned();
            if !value.is_empty() {
                env.vars.insert(key.to_owned(), value);
            }
        };
        set("ANDROID_HOME", self.android_home.peek());
        // `sdkmanager` writes the SDK root under both names and different tools
        // read different ones; the studio's own checklist reads the first, and
        // Gradle reads the second.
        set("ANDROID_SDK_ROOT", self.android_home.peek());
        set("ANDROID_NDK_HOME", self.android_ndk.peek());
        set("ANDROID_NDK_ROOT", self.android_ndk.peek());
        set("JAVA_HOME", self.java_home.peek());
        env
    }

    /// The signing variables a release build needs, ready to hand to a task.
    ///
    /// The password is read from the environment the studio itself was given —
    /// never from the settings file, which is plain text in a config directory.
    /// A studio launched without it produces a build that is unsigned and says
    /// so, which is a better failure than one that silently signs with a
    /// password saved on disk.
    #[must_use]
    pub fn signing_env(&self) -> Vec<(String, String)> {
        let mut env = Vec::new();
        let keystore = self.keystore.peek();
        if !keystore.trim().is_empty() {
            env.push(("VIEWW_KEYSTORE".to_owned(), keystore.trim().to_owned()));
        }
        let alias = self.keystore_alias.peek();
        if !alias.trim().is_empty() {
            env.push(("VIEWW_KEYSTORE_ALIAS".to_owned(), alias.trim().to_owned()));
        }
        let team = self.apple_team.peek();
        if !team.trim().is_empty() {
            env.push(("VIEWW_APPLE_TEAM".to_owned(), team.trim().to_owned()));
        }
        env
    }

    // ----- N2: a new project ----------------------------------------------

    /// Scaffold a new vieww project and open it.
    ///
    /// # The missing directory picker, stated
    ///
    /// Plan 2 §5.2 asks for "a directory picker and an application name". There
    /// is no file-dialog service in vieww — not in `vieww-foundation`'s service
    /// list and not in `vieww-platform-winit` — so the studio's in-app picker
    /// is what stands in for the directory half, and the name prompt is what
    /// stands in for the name half.
    ///
    /// The flow is two steps, both modal, in this order:
    ///
    /// 1. **Folder picker** in [`PickerMode::NewProject`] — pick the *parent*
    ///    the new project will live inside. The picker reuses the same UI the
    ///    File → Open Folder… command uses, with a different title and confirm
    ///    button so the user knows what confirming will do.
    /// 2. **Name prompt** with [`NameKind::NewProject`] — type the project's
    ///    crate name, validated against Cargo's rules as it is typed. Confirming
    ///    scaffolds the project inside the folder step 1 picked.
    ///
    /// **The order matters.** Asking for the name first would land the project
    /// in a directory the user did not get to choose; asking for the folder
    /// first lets the user see where they are navigating to and pick the
    /// parent deliberately. The folder picker is also a more natural first
    /// step on every desktop — the OS file dialog asks "where?" before
    /// "what?".
    ///
    /// Adding a `FileDialog` service to the platform crate is the real fix for
    /// the directory half and is framework work, not studio work; until then,
    /// the in-app picker is what makes the command shippable.
    pub fn new_project(&self) {
        self.picker_mode.set(PickerMode::NewProject);
        let at = self.root.peek().clone().unwrap_or_else(|| {
            // Fall back to the current directory rather than the open folder's
            // parent: a studio with nothing open is the most common case for
            // New Project, and `cwd` is where the user launched it from.
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"))
        });
        let picker = crate::picker::Picker::at(at);
        self.picker.set(Some(Rc::new(picker)));
    }

    /// The half of New Project that runs after the user has typed a name and
    /// pressed Create. Lifted out of [`new_project`](Self::new_project)
    /// because the name prompt is the only sensible way to ask, and asking
    /// splits the work in two: the dialog owns the name, this owns the disk.
    pub(super) fn scaffold_named_project(
        &self,
        parent: &std::path::Path,
        name: &str,
        kind: crate::scaffold::ProjectKind,
    ) -> Result<String, String> {
        use crate::scaffold::{self, Dependency};

        let target = parent.join(name);
        if target.exists() {
            return Err(format!("{name} already exists in {}", parent.display()));
        }

        // A path dependency on this checkout, because that is the only kind the
        // preview can load — see `scaffold::Dependency`.
        let dependency = scaffold::checkout_root()
            .map_or_else(|| Dependency::Version("0.0.1".into()), Dependency::Path);

        let written =
            scaffold::create_for(&target, name, &dependency, kind).map_err(|e| e.to_string())?;
        let previewable = dependency.previewable();

        // Open it as a workspace before reporting, so the file tree fills
        // in the same instant the message lands in the Output panel.
        self.open_workspace(&target);

        Ok(if previewable {
            format!(
                "created {name} ({} files) — vieww is a path dependency, so Render works",
                written.len()
            )
        } else {
            format!(
                "created {name} ({} files) — vieww is a published version, so Build works and Render will refuse",
                written.len()
            )
        })
    }

    /// Close what is open and open `root` instead.
    pub fn adopt_workspace(&self, workspace: Workspace) {
        // **Focus a file Render can actually render.** Not buffer zero, which
        // in every scaffolded project is `src/main.rs` — four lines with no
        // `screen()` in them, and so the one file in the project the preview
        // refuses. See `Workspace::preferred_buffer` for the first-run
        // sequence that made this worth changing.
        //
        // Read before the struct is taken apart below, because it borrows the
        // whole workspace.
        let active = workspace.preferred_buffer();
        self.root.set(workspace.root.clone());
        self.tree.set(workspace.tree);
        self.is_vieww.set(workspace.is_vieww);
        self.put_buffers(workspace.buffers);
        self.active_buffer.set(active);
        self.sync_caret();
        // **Point the previewed screen's `AssetBundle` at the new workspace.**
        // A guest that asks for `images/avatar.png` resolves against the
        // project's `assets/` first; before this it got the studio's own
        // bundle, which is exactly backwards. The wrapper is held on
        // `workspace_bundle` so changing workspaces is one `set_project` call
        // rather than a service re-registration.
        if let Some(bundle) = self.workspace_bundle.as_ref() {
            bundle.set_project(workspace.root.as_deref());
        }
        // A Rust project just opened, so the thing that answers questions about
        // Rust should be starting. See [`Self::start_analyzer`] for why the
        // studio does this rather than waiting to be asked.
        self.start_analyzer(false);
    }

    // ----- N6: the edits an editor makes for you --------------------------

    /// The value a change becomes once the editor comforts have had a look.
    ///
    /// Unchanged when nothing applies, which is the overwhelming majority of
    /// keystrokes.
    pub(super) fn with_comforts(
        &self,
        value: vieww_foundation::TextEditingValue,
    ) -> vieww_foundation::TextEditingValue {
        if !self.comforts.get() {
            return value;
        }
        // **A change that did not change the text is not a keystroke.**
        //
        // The comforts read one typed character; a report that carries the same
        // string the buffer already holds carries no typed character at all.
        // Without this guard `after_typing` can still fire on one: it
        // reconstructs "the selection, replaced by one character" and compares,
        // and that comparison succeeds *trivially* whenever the previous
        // selection was exactly one character long — because replacing a
        // one-character selection with its own character is the identity. A
        // pointer drag passes through a one-character selection on its way, so
        // a plain selection gesture could reach `typed_bracket`, take its
        // surround-a-selection branch, and commit a rewritten buffer.
        //
        // Cheap, too: this is the case on every caret move and every click, and
        // it now costs one length comparison instead of a scan of the buffer.
        if let Some(before) = self.active() {
            if before.value.text == value.text {
                return value;
            }
        }
        // The comforts read *one* keystroke against *one* caret and rewrite
        // the whole text around it — auto-indent looks at the line the caret
        // is on, and auto-close wraps the selection. With several carets there
        // is no single line and no single selection, so applying them would
        // indent one caret's line and leave the rest, which is worse than not
        // applying them at all. Said out loud rather than left to be found:
        // typing with several carets is deliberately plain.
        if value.is_multi_caret() {
            return value;
        }
        // An in-flight IME composition is the platform's text, not the user's
        // final text, and rewriting it under the composing range would corrupt
        // what the input method thinks it owns.
        if value.composing.is_some() {
            return value;
        }
        let Some(before) = self.active() else {
            return value;
        };
        if before.value.composing.is_some() {
            return value;
        }

        let selection = (before.value.selection.base, before.value.selection.extent);
        let Some(edit) = crate::edit_ops::after_typing(&before.value.text, selection, &value.text)
        else {
            return value;
        };

        vieww_foundation::TextEditingValue {
            text: edit.text,
            selection: vieww_foundation::TextSelection {
                base: edit.selection.0,
                extent: edit.selection.1,
                affinity: value.selection.affinity,
            },
            // The comforts run over the *primary* caret's keystroke and
            // rewrite the whole text around it, so they cannot be applied
            // per-caret — see `with_comforts`'s own note. Multi-caret typing
            // therefore goes through the model's own path with the comforts
            // off, which `with_comforts` refuses for.
            secondary: value.secondary.clone(),
            composing: None,
        }
    }

    /// Apply an [`edit_ops`](crate::edit_ops) result to the active buffer.
    ///
    /// One place, so every comfort goes through the same undo history and the
    /// same dirty flag as typing does.
    pub(super) fn apply_edit(&self, edit: crate::edit_ops::Edit) {
        let Some(mut value) = self.active().map(|b| b.value) else {
            return;
        };
        value.text = edit.text;
        value.selection = vieww_foundation::TextSelection {
            base: edit.selection.0,
            extent: edit.selection.1,
            affinity: value.selection.affinity,
        };
        value.composing = None;
        // Straight to `edit`: `after_typing` already declines anything this
        // produces, because a comfort's output is never "the selection replaced
        // by one character". Going through it anyway would be one wasted
        // comparison per command and one more place for the two to disagree.
        self.edit(value);
    }

    /// Comment or uncomment the lines the selection touches.
    pub fn toggle_comment(&self) {
        let Some(buffer) = self.active() else { return };
        let value = &buffer.value;
        let selection = (value.selection.base, value.selection.extent);
        // The buffer's language, not Rust's `//`: see `edit_ops::toggle_comment`.
        self.apply_edit(crate::edit_ops::toggle_comment(
            &value.text,
            buffer.language,
            selection,
        ));
    }

    /// Indent or outdent the lines the selection touches.
    pub fn shift_indent(&self, deeper: bool) {
        let Some(buffer) = self.active() else { return };
        let value = &buffer.value;
        let selection = (value.selection.base, value.selection.extent);
        self.apply_edit(crate::edit_ops::shift_indent(
            &value.text,
            selection,
            deeper,
        ));
    }

    /// Report the outcome of an operation into the Output panel.
    pub(super) fn report(&self, result: std::io::Result<()>, what: &str) {
        let name = self.active().map_or_else(String::new, |b| b.name);
        match result {
            Ok(()) => self.append_output(vec![format!("{what} {name}")]),
            Err(error) => self.append_output(vec![format!("could not save {name}: {error}")]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason `render_preview` has to special-case Say, pinned as a fact
    /// about the generated code rather than left as a claim in a comment.
    ///
    /// If Say's code generator ever stops emitting `use super::`, this test
    /// fails and the special case can go — which is the outcome worth being
    /// told about, since the special case only exists to work around this.
    #[test]
    fn generated_say_trips_the_needs_project_heuristic() {
        let generated = vieww_say_codegen::compile("home.say", crate::scaffold::HOME_SAY)
            .expect("the scaffold's own screen must compile");
        assert!(
            Studio::needs_project(&generated.rust),
            "generated Say no longer reaches for `super` — the Say special case \
             in `render_preview` can be removed"
        );
    }

    /// And the file a new project actually starts with contains neither of the
    /// things the heuristic looks for, which is why matching on the *generated*
    /// code is a false positive rather than a true one.
    #[test]
    fn the_scaffolded_screen_itself_reaches_for_nothing() {
        let source = crate::scaffold::HOME_SAY;
        assert!(!source.contains("crate::"), "{source}");
        assert!(!source.contains("use super::"), "{source}");
    }
}
