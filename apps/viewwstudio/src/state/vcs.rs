//! Source control.
//!
//! The git seam.
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
    // ----- Source control ----------------------------------------------------

    /// Whether the open folder is a git repository.
    #[must_use]
    pub fn is_repository(&self) -> bool {
        self.root
            .get()
            .as_ref()
            .is_some_and(|root| crate::git::is_repository(root))
    }

    /// Ask git what changed.
    pub fn refresh_git(&self) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        if !crate::git::is_repository(&root) {
            return;
        }
        let wake = self.frame_waker();
        self.jobs.borrow_mut().capture(
            crate::git::status_spec(&root),
            crate::jobs::Follow::GitStatus,
            wake,
        );
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Turn a finished `git status` into the source-control view's state.
    pub(super) fn read_git_status(&self, id: u64) {
        let output = self
            .jobs
            .borrow()
            .job(id)
            .map(crate::jobs::Job::captured)
            .unwrap_or_default();
        self.git.set(Rc::new(crate::git::parse_status(&output)));
        self.git_scanned.set(true);
    }

    /// Stage or unstage one path.
    pub fn stage(&self, path: &Path, staged: bool) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        let spec = if staged {
            crate::git::unstage_spec(&root, path)
        } else {
            crate::git::stage_spec(&root, path)
        };
        let wake = self.frame_waker();
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::GitChanged, wake);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Commit what is in the index, with the message in the box.
    ///
    /// Refuses an empty message and a commit with nothing staged, and says
    /// which — those are the two things a commit button does wrong, and git's
    /// own message for the second is four lines of advice about `git add`.
    pub fn commit(&self) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        let message = self.commit_message.peek();
        if self.git.peek().staged() == 0 {
            self.append_output(vec![
                "nothing is staged — a commit would be empty".to_string()
            ]);
            self.show_output();
            return;
        }
        let Some(spec) = crate::git::commit_spec(&root, &message) else {
            self.append_output(vec!["a commit needs a message".to_string()]);
            self.show_output();
            return;
        };
        let wake = self.frame_waker();
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::GitChanged, wake);
        // Cleared now rather than on success: the message is already in the
        // command, and a box that empties only when git agrees leaves people
        // pressing commit twice.
        self.commit_message.set(String::new());
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
    }

    /// Send this branch to `origin`.
    ///
    /// The first push of a new branch sets its upstream, because git's refusal
    /// for a branch with none is advice ending in a command — and running the
    /// command the tool just told you to run is the studio's job.
    pub fn push(&self) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        let status = self.git.peek();
        let Some(branch) = status.branch.clone() else {
            self.append_output(vec![
                "this repository has no commits yet, so there is nothing to push".to_owned(),
            ]);
            self.show_output();
            return;
        };
        let upstream = crate::git::has_upstream(&root, &branch);
        self.start_git(crate::git::push_spec(&root, &branch, upstream));
    }

    /// Fast-forward this branch from `origin`.
    pub fn pull(&self) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        self.start_git(crate::git::pull_spec(&root));
    }

    /// Refresh the remote-tracking refs the ahead/behind counts are measured
    /// against.
    pub fn fetch(&self) {
        let Some(root) = self.root.get().as_ref().cloned() else {
            return;
        };
        self.start_git(crate::git::fetch_spec(&root));
    }

    /// Run a git command through the job queue, and re-read the status after.
    ///
    /// Through the queue rather than inline for the reason everything else is:
    /// a push over a slow link is minutes, and minutes on the UI thread is a
    /// frozen window. `Follow::GitChanged` is what makes the panel and the
    /// status bar catch up when it finishes.
    pub(super) fn start_git(&self, spec: crate::task::Spec) {
        let wake = self.frame_waker();
        self.jobs
            .borrow_mut()
            .start(spec, crate::jobs::Follow::GitChanged, wake);
        self.jobs_generation.update(|n| *n = n.wrapping_add(1));
        self.show_output();
    }

    /// The change git reports for `path`, which the file tree draws a letter
    /// for. `path` is absolute; git's are relative to the repository.
    #[must_use]
    pub fn git_change(&self, path: &Path) -> Option<crate::git::Change> {
        let root = self.root.get();
        let root = root.as_ref()?;
        let relative = path.strip_prefix(root).ok()?;
        self.git.get().change_for(relative)
    }
}
