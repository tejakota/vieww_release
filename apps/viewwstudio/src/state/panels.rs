//! The Output panel, building and the job queue.
//!
//! What fills the bottom panel, and the queue that feeds it.
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
    // ----- the Output panel ----------------------------------------------

    /// Put `lines` at the bottom of the Output panel.
    ///
    /// Everything that used to go to `eprintln!` comes here instead. A save
    /// that failed into a terminal nobody is looking at is a save that failed
    /// silently, which for the one operation whose whole job is not losing work
    /// is the worst place to be quiet.
    pub fn append_output(&self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        self.output.update(|output| {
            let mut next = (**output).clone();
            next.extend(lines);
            // Bounded: a long session of failed saves must not grow without
            // limit behind a panel nobody has open.
            let overflow = next.len().saturating_sub(500);
            next.drain(..overflow);
            *output = Rc::new(next);
        });
    }

    // ----- N3: building ---------------------------------------------------

    /// Why a build cannot start, or `None` if it can.
    ///
    /// One function, read by the Build button's enabled state, by the menu, and
    /// by the sentence shown next to a disabled control. The alternative —
    /// a `bool` for the button and a message computed somewhere else — is how a
    /// control ends up greyed out with no explanation, or explained and still
    /// clickable.
    #[must_use]
    pub fn build_refusal(&self) -> Option<String> {
        let root = self.root.get();
        let Some(root) = root.as_ref() else {
            return Some(
                "no folder is open — the preview works on a single file, but a build needs a \
                 Cargo project"
                    .into(),
            );
        };
        self.builds
            .borrow()
            .check(root, self.target.get(), &self.build_env)
            .err()
            .map(|refusal| refusal.to_string())
    }

    /// Start a build, or say why not.
    pub fn start_build(&self, kind: Kind) {
        if let Some(why) = self.build_refusal() {
            self.append_output(vec![format!("{}: {why}", kind.label())]);
            self.panel_tab.set(PanelTab::Output);
            self.panel_open.set(true);
            return;
        }
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };

        // The panel opens on Output, not Problems: a build that has just
        // started has no problems yet, and an empty Problems list is the least
        // informative thing the window could show at that moment.
        self.panel_tab.set(PanelTab::Output);
        self.panel_open.set(true);

        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        self.builds
            .borrow_mut()
            .start(&root, self.target.get(), self.profile.get(), kind, wake);
        // Registered in the queue, not run by it: `Builds` keeps its own task
        // for the reason `jobs`'s module docs give. Without this the Tasks
        // panel would be missing the longest-running thing in the studio.
        let command = self
            .builds
            .borrow()
            .last_command()
            .unwrap_or_else(|| "cargo".to_string());
        let id = self.jobs.borrow_mut().adopt(kind.label(), command);
        self.build_job.set(Some(id));
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        self.build_state.set(builds::State::Running(kind));
        // The build is now the most recent thing that happened, so it is what
        // the status light reports until the preview says otherwise.
        self.activity.set(Activity::Build);
    }

    /// Ask the running build to stop.
    pub fn cancel_build(&self) {
        self.builds.borrow().cancel();
    }

    /// Take everything the build has produced and publish it into signals.
    ///
    /// Called once a frame from the studio's `before_frame` hook, beside
    /// [`poll_compile`](Self::poll_compile). Returns whether anything changed,
    /// so a frame during a long build costs one `try_recv` and no signal writes.
    ///
    /// # Why the diagnostics are replaced rather than merged
    ///
    /// A build's diagnostics are about the whole project; the preview's are
    /// about one buffer. Merging them would put an error from a file nobody has
    /// open beside one in the buffer on screen with nothing to tell them apart,
    /// and the next preview compile would clear half the list. Whichever ran
    /// last owns the panel, and the Output panel above it says which that was.
    pub fn poll_builds(&self) -> bool {
        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        let (changed, lines, diagnostics, state) = {
            let mut builds = self.builds.borrow_mut();
            let changed = builds.poll(wake);
            if !changed {
                return false;
            }
            (
                changed,
                std::mem::take(&mut builds.output),
                builds.diagnostics.clone(),
                builds.state.clone(),
            )
        };

        self.append_output(lines);
        self.diagnostics.set(Rc::new(diagnostics));
        // The queue's copy of the build follows the state machine's verdict
        // rather than the task's, because the state machine is what knows a
        // Build-and-Run has started a *second* child and is not over yet.
        if !state.busy() {
            if let Some(id) = self.build_job.take() {
                let elapsed = self
                    .jobs
                    .borrow()
                    .job(id)
                    .map_or(std::time::Duration::ZERO, |job| job.started.elapsed());
                let status = match &state {
                    builds::State::Failed { status, .. } => status.clone(),
                    _ => crate::task::Status::Succeeded,
                };
                self.jobs.borrow_mut().resolve(
                    id,
                    crate::task::Outcome {
                        status,
                        duration: elapsed,
                    },
                );
                self.jobs_generation.update(|n| *n = n.wrapping_add(1));
            }
        }
        self.build_state.set(state);
        // Reached only when `poll` reported a change, so this is a real
        // transition of the build machine — started, finished, or moved from
        // building to running — and therefore the newest thing to report.
        self.activity.set(Activity::Build);
        changed
    }

    // ----- §8.1: the job queue --------------------------------------------

    /// Drain every running job. Called once a frame, beside `poll_builds`.
    ///
    /// Returns whether anything changed, so the frame hook can stay quiet on
    /// the overwhelmingly common frame where nothing did.
    pub fn poll_jobs(&self) -> bool {
        let reports = self.jobs.borrow_mut().poll();
        if reports.is_empty() {
            return false;
        }

        let mut lines = Vec::new();
        let mut follows = Vec::new();
        for report in reports {
            match report {
                crate::jobs::Report::Line(line) => lines.push(line),
                crate::jobs::Report::Ended { id, follow, ok } => follows.push((id, follow, ok)),
            }
        }
        self.append_output(lines);
        for (id, follow, ok) in follows {
            self.finish_job(id, &follow, ok);
        }
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        true
    }

    /// Ask a job to stop, by the id its row carries.
    pub fn cancel_job(&self, id: u64) {
        self.jobs.borrow_mut().cancel(id);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Forget every job that has ended.
    pub fn clear_finished_jobs(&self) {
        self.jobs.borrow_mut().clear_finished();
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// What a finished job caused.
    pub(super) fn finish_job(&self, id: u64, follow: &crate::jobs::Follow, ok: bool) {
        match follow {
            crate::jobs::Follow::None => {}
            crate::jobs::Follow::Produced { artefact } => {
                self.append_output(vec![format!("produced {}", artefact.display())]);
                // Kept, not just printed. A path in a scrolling log is a path
                // nobody can act on; `install_on` needs exactly this.
                self.artefact.set(Some(artefact.clone()));
                if self.devices_scanned.peek() {
                    self.notify("Built. The Export view can install it on a connected device.");
                }
            }
            crate::jobs::Follow::ExportStep => self.advance_export(ok),
            crate::jobs::Follow::Listing => self.read_device_listing(id),
            crate::jobs::Follow::GitStatus => self.read_git_status(id),
            // Whatever it did, the status is now stale. Asking again is one
            // cheap process and is the only way the view can be trusted —
            // predicting what a commit did to the index is how a source
            // control panel comes to disagree with the repository.
            crate::jobs::Follow::GitChanged => self.refresh_git(),
            crate::jobs::Follow::Formatted { buffer, scratch } => {
                if ok {
                    match std::fs::read_to_string(scratch) {
                        Ok(formatted) => self.apply_formatting(*buffer, &formatted),
                        Err(error) => self.append_output(vec![format!(
                            "rustfmt succeeded but its output could not be read: {error}"
                        )]),
                    }
                }
                // Deleted whether it worked or not. A scratch file left behind
                // on every failed format is a temp directory that grows for as
                // long as the studio is open.
                let _ = std::fs::remove_file(scratch);
            }
        }
    }
}
