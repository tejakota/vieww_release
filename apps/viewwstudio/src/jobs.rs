//! §8.1's queue: every child process the studio has started, in one list.
//!
//! # Why a queue, when [`task`](crate::task) already runs a child
//!
//! `Task` runs *a* child. The studio runs several at once and always has: a
//! `cargo build`, a `cargo run` it started, a `rustfmt`, and — once N5 lands —
//! an `adb install` and a device scan. Before this module each of those was
//! held in whichever struct started it, which had three consequences worth
//! naming:
//!
//! 1. **Only the newest was visible.** The Output panel showed one stream, so
//!    a long build with a `rustfmt` on top of it looked like the build had
//!    stopped printing.
//! 2. **Only the newest could be cancelled.** There was no way to name the
//!    other one.
//! 3. **A finished task left nothing behind.** Its outcome went into whatever
//!    state machine owned it, and how long it took, and whether it succeeded,
//!    were not recoverable a second later.
//!
//! So: one queue, one cancel, one output — and a Tasks panel that is a view of
//! this and nothing else.
//!
//! # `Builds` is not rewritten in terms of this
//!
//! For the same reason `Job` is not rewritten in terms of `Task`, and the same
//! reason stated in that module: the build state machine parses cargo's JSON
//! into diagnostics, and routing the one pipeline that must never regress
//! through the newest code in the studio to save a hundred lines is a bad
//! trade. `Builds` registers a *record* here so the queue is complete, and
//! keeps its own task.
//!
//! # Nothing here touches the UI
//!
//! This module is `std` only — no vieww types, no signals — so it runs under
//! `ci/standalone.sh` with the rest of the pipeline logic. What a finished job
//! should *cause* is returned as a [`Follow`], for the caller to act on where
//! the application state lives.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::task::{Event, Outcome, Spec, Status, Stream, Task};

/// What a job is for, and what finishing it should cause.
///
/// The variant is chosen when the job is queued rather than inferred from its
/// program, because "what runs" and "what it means" are different questions:
/// two jobs can both be `rustfmt` and only one of them be a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Follow {
    /// Nothing. The output was the point.
    None,
    /// `rustfmt` rewrote `scratch` in place; read it back into the buffer at
    /// `buffer`, and delete it.
    ///
    /// The buffer's *index*, not its path: a format is applied to whatever the
    /// tab held when the job was queued, and a job that resolved a path again
    /// on completion would write a formatted file into a tab the user had
    /// since switched away from.
    Formatted { buffer: usize, scratch: PathBuf },
    /// The job produced an artefact at `artefact` — a bundle, an APK, an
    /// `.app`. Reported, not opened: what to do with it is the user's.
    Produced { artefact: PathBuf },
    /// One step of a running export finished; the studio advances the plan.
    ///
    /// The plan itself is not carried here. It lives in the studio, because a
    /// plan half-run is application state and this module is deliberately
    /// stateless about *why* anything runs.
    ExportStep,
    /// The job's stdout is a listing to be parsed rather than read — `adb
    /// devices`, and anything like it. The lines are on the job, which is what
    /// [`capture`](Queue::capture) is for.
    Listing,
    /// The job's stdout is `git status`, to be parsed into the source-control
    /// view. Captured, like [`Listing`](Self::Listing), and separate from it
    /// because the two are parsed by different modules.
    GitStatus,
    /// The job changed the repository — a stage, an unstage, a commit — so the
    /// status is now stale and has to be asked for again.
    GitChanged,
}

/// Where a job is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Running,
    /// Ended, with the verdict and how long it took.
    Ended(Outcome),
}

impl State {
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    /// The word the Tasks panel puts in the state cell.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Ended(outcome) => match outcome.status {
                Status::Succeeded => "done",
                Status::Failed { .. } => "failed",
                Status::Cancelled => "cancelled",
                Status::TimedOut => "timed out",
                Status::NotStarted(_) => "could not start",
                Status::Lost => "lost",
            },
        }
    }
}

/// One entry in the queue.
#[derive(Debug)]
pub struct Job {
    /// Stable for the life of the queue, so a row can be cancelled by name
    /// rather than by position — a list that reorders under a click is a list
    /// that cancels the wrong thing.
    pub id: u64,
    pub label: String,
    /// The command as a person would type it, for the row's detail line.
    pub command: String,
    pub state: State,
    pub started: Instant,
    /// How many lines it has printed. The text itself goes to the Output
    /// panel, which is the one place output belongs; this is so a row can say
    /// a silent job is silent.
    pub lines: usize,
    follow: Follow,
    /// Standard output, kept only for a job whose output is *data*.
    ///
    /// Off by default and deliberately: keeping every line of a ten-minute
    /// build in memory to answer a question nobody asked is how an editor
    /// comes to hold a gigabyte of log.
    captured: Vec<String>,
    task: Option<Task>,
}

impl Job {
    /// How long it has been running, or how long it ran.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        match &self.state {
            State::Running => self.started.elapsed(),
            State::Ended(outcome) => outcome.duration,
        }
    }

    /// Whether this job can still be cancelled.
    #[must_use]
    pub fn is_cancellable(&self) -> bool {
        self.state.is_running() && self.task.is_some()
    }

    /// The standard output this job was asked to keep, joined back into the
    /// text a parser expects.
    ///
    /// Empty unless the job was started with [`Queue::capture`].
    #[must_use]
    pub fn captured(&self) -> String {
        self.captured.join("\n")
    }
}

/// Something that happened while polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// A line to put in the Output panel, already prefixed with its job.
    Line(String),
    /// A job ended. The caller acts on the follow-up.
    Ended { id: u64, follow: Follow, ok: bool },
}

/// Every child process the studio has started, newest last.
#[derive(Debug, Default)]
pub struct Queue {
    jobs: Vec<Job>,
    next_id: u64,
    /// Ids of the jobs keeping their standard output. A short list — usually
    /// empty, at most one — so a linear search per line is cheaper than a
    /// flag read through a second indirection.
    capturing: Vec<u64>,
}

/// How many finished jobs are kept.
///
/// A studio left open all day starts hundreds. Keeping every one turns the
/// Tasks panel into a log nobody reads and holds a `Task` per entry; keeping
/// none loses the answer to "did that build actually succeed?" the instant it
/// is asked. Twenty is a session's worth of recent history.
pub const HISTORY: usize = 20;

impl Queue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `spec`, and remember it.
    ///
    /// Returns the job's id so a caller that wants to cancel its own job later
    /// — a re-format while the last one is still going — can name it.
    pub fn start(&mut self, spec: Spec, follow: Follow, wake: impl Fn() + Send + 'static) -> u64 {
        self.start_inner(spec, follow, false, wake)
    }

    /// The same, keeping standard output on the job so a caller can parse it.
    ///
    /// For a job whose output is *data* — `adb devices`, a version probe. Not
    /// for a build: see [`Job::captured`].
    pub fn capture(&mut self, spec: Spec, follow: Follow, wake: impl Fn() + Send + 'static) -> u64 {
        self.start_inner(spec, follow, true, wake)
    }

    fn start_inner(
        &mut self,
        spec: Spec,
        follow: Follow,
        capture: bool,
        wake: impl Fn() + Send + 'static,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let job = Job {
            id,
            label: spec.label.clone(),
            command: spec.command_line(),
            state: State::Running,
            started: Instant::now(),
            lines: 0,
            follow,
            captured: Vec::new(),
            task: Some(Task::spawn(spec, wake)),
        };
        let capturing = capture;
        self.jobs.push(job);
        if capturing {
            self.capturing.push(id);
        }
        self.trim();
        id
    }

    /// Record a job this queue did not start.
    ///
    /// For [`Builds`](crate::builds), which keeps its own task for the reason
    /// the module docs give. Without this the Tasks panel would be missing the
    /// longest-running thing in the studio, which is the one a person most
    /// wants to see.
    pub fn adopt(&mut self, label: impl Into<String>, command: impl Into<String>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job {
            id,
            label: label.into(),
            command: command.into(),
            state: State::Running,
            started: Instant::now(),
            lines: 0,
            follow: Follow::None,
            captured: Vec::new(),
            task: None,
        });
        self.trim();
        id
    }

    /// Finish an adopted job. A no-op for one this queue owns, which ends
    /// through its own task.
    pub fn resolve(&mut self, id: u64, outcome: Outcome) {
        if let Some(job) = self
            .jobs
            .iter_mut()
            .find(|job| job.id == id && job.task.is_none())
        {
            job.state = State::Ended(outcome);
        }
    }

    /// Drain every running job. One `try_recv` per job when nothing has
    /// happened, which is the common frame.
    pub fn poll(&mut self) -> Vec<Report> {
        let mut reports = Vec::new();
        let capturing = std::mem::take(&mut self.capturing);
        for job in &mut self.jobs {
            let Some(task) = &job.task else {
                continue;
            };
            for event in task.poll() {
                match event {
                    Event::Line(stream, line) => {
                        job.lines += 1;
                        if stream == Stream::Out && capturing.contains(&job.id) {
                            job.captured.push(line.clone());
                        }
                        // Tagged with the job, because the Output panel is now
                        // shared. Untagged interleaved output from two child
                        // processes is worse than either alone.
                        let mark = match stream {
                            Stream::Out => "",
                            Stream::Err => "! ",
                        };
                        reports.push(Report::Line(format!("[{}] {mark}{line}", job.label)));
                    }
                    Event::Finished(outcome) => {
                        let ok = outcome.succeeded();
                        reports.push(Report::Line(format!(
                            "[{}] {} in {:.2}s",
                            job.label,
                            outcome.status.describe(),
                            outcome.duration.as_secs_f32()
                        )));
                        job.state = State::Ended(outcome);
                        reports.push(Report::Ended {
                            id: job.id,
                            follow: job.follow.clone(),
                            ok,
                        });
                        // Dropped so the channel and the worker go with it. The
                        // row stays; it is the *process* that is over.
                        job.task = None;
                    }
                }
            }
        }
        self.capturing = capturing;
        reports
    }

    /// Ask a job to stop. Cancelling a finished one is a no-op rather than an
    /// error: the click and the last poll can land in either order.
    pub fn cancel(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter().find(|job| job.id == id) {
            if let Some(task) = &job.task {
                task.cancel();
            }
        }
    }

    /// Ask every running job to stop.
    pub fn cancel_all(&mut self) {
        for job in &self.jobs {
            if let Some(task) = &job.task {
                task.cancel();
            }
        }
    }

    /// Forget every job that has ended. The panel's clear button.
    pub fn clear_finished(&mut self) {
        self.jobs.retain(|job| job.state.is_running());
    }

    #[must_use]
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// How many are still going. The status bar's cell.
    #[must_use]
    pub fn running(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.state.is_running())
            .count()
    }

    #[must_use]
    pub fn job(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    /// Drop the oldest finished jobs past [`HISTORY`].
    ///
    /// Running jobs are never dropped, however old — a queue that forgot a
    /// twenty-minute release build because twenty short ones happened during
    /// it would lose the only handle on the one that mattered.
    fn trim(&mut self) {
        let finished = self
            .jobs
            .iter()
            .filter(|job| !job.state.is_running())
            .count();
        if finished <= HISTORY {
            return;
        }
        let mut to_drop = finished - HISTORY;
        self.jobs.retain(|job| {
            if to_drop > 0 && !job.state.is_running() {
                to_drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(label: &str, script: &str) -> Spec {
        Spec::new(label, "sh", std::env::temp_dir())
            .arg("-c")
            .arg(script)
            .timeout(Duration::from_secs(20))
    }

    /// Poll until every job has ended, or give up. The queue is asynchronous
    /// by construction, so every test here is a loop.
    fn drain(queue: &mut Queue) -> Vec<Report> {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut all = Vec::new();
        while Instant::now() < deadline {
            all.extend(queue.poll());
            if queue.running() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        all
    }

    #[test]
    fn two_jobs_run_at_once_and_both_are_listed() {
        // The whole reason this module exists. Before it, starting the second
        // of these lost the first.
        let mut queue = Queue::new();
        queue.start(sh("one", "echo first"), Follow::None, || {});
        queue.start(sh("two", "echo second"), Follow::None, || {});
        assert_eq!(queue.running(), 2);

        let reports = drain(&mut queue);
        let lines: Vec<&str> = reports
            .iter()
            .filter_map(|r| match r {
                Report::Line(line) => Some(line.as_str()),
                Report::Ended { .. } => None,
            })
            .collect();
        assert!(lines.iter().any(|line| line.contains("[one] first")));
        assert!(lines.iter().any(|line| line.contains("[two] second")));
        assert_eq!(queue.jobs().len(), 2, "both are still listed after ending");
        assert_eq!(queue.running(), 0);
    }

    #[test]
    fn every_line_says_which_job_it_came_from() {
        // One Output panel and two children is unreadable without this.
        let mut queue = Queue::new();
        queue.start(sh("fmt", "echo out; echo bad 1>&2"), Follow::None, || {});
        let reports = drain(&mut queue);
        let lines: Vec<&String> = reports
            .iter()
            .filter_map(|r| match r {
                Report::Line(line) => Some(line),
                Report::Ended { .. } => None,
            })
            .collect();
        assert!(lines.iter().all(|line| line.starts_with("[fmt]")));
        assert!(
            lines.iter().any(|line| line.contains("! bad")),
            "and which pipe: {lines:?}"
        );
    }

    #[test]
    fn a_follow_up_is_reported_once_with_the_verdict() {
        let mut queue = Queue::new();
        let scratch = std::env::temp_dir().join("viewwstudio-jobs-test.rs");
        let follow = Follow::Formatted {
            buffer: 3,
            scratch: scratch.clone(),
        };
        queue.start(sh("rustfmt", "exit 0"), follow.clone(), || {});
        let reports = drain(&mut queue);
        let ended: Vec<&Report> = reports
            .iter()
            .filter(|r| matches!(r, Report::Ended { .. }))
            .collect();
        assert_eq!(ended.len(), 1, "exactly one end, whatever happened");
        assert_eq!(
            ended[0],
            &Report::Ended {
                id: 0,
                follow,
                ok: true
            }
        );
    }

    #[test]
    fn a_failed_job_reports_its_follow_up_with_ok_false() {
        // The follow-up still arrives — the caller has a scratch file to
        // delete either way — but it is told not to apply the result.
        let mut queue = Queue::new();
        queue.start(
            sh("rustfmt", "echo 'error: expected `;`' 1>&2; exit 1"),
            Follow::Formatted {
                buffer: 0,
                scratch: PathBuf::from("/tmp/nothing.rs"),
            },
            || {},
        );
        let reports = drain(&mut queue);
        let ok = reports.iter().find_map(|r| match r {
            Report::Ended { ok, .. } => Some(*ok),
            Report::Line(_) => None,
        });
        assert_eq!(ok, Some(false));
        assert_eq!(queue.jobs()[0].state.label(), "failed");
    }

    #[test]
    fn cancelling_names_one_job_and_leaves_the_others() {
        let mut queue = Queue::new();
        let slow = queue.start(sh("slow", "sleep 30"), Follow::None, || {});
        let other = queue.start(sh("other", "sleep 30"), Follow::None, || {});
        queue.cancel(slow);

        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            queue.poll();
            if !queue.job(slow).expect("listed").state.is_running() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(queue.job(slow).expect("listed").state.label(), "cancelled");
        assert!(
            queue.job(other).expect("listed").state.is_running(),
            "the other job is untouched"
        );
        queue.cancel_all();
    }

    #[test]
    fn cancelling_a_finished_job_is_not_an_error() {
        // The click and the poll that ends the job can land in either order,
        // and one of those orders must not be a panic.
        let mut queue = Queue::new();
        let id = queue.start(sh("quick", "exit 0"), Follow::None, || {});
        drain(&mut queue);
        queue.cancel(id);
        queue.cancel(9999);
    }

    #[test]
    fn an_adopted_job_is_listed_and_resolved_by_its_owner() {
        // `Builds` keeps its own task; the queue is still complete.
        let mut queue = Queue::new();
        let id = queue.adopt("Build (debug)", "cargo build");
        assert_eq!(queue.running(), 1);
        assert!(
            !queue.job(id).expect("listed").is_cancellable(),
            "an adopted job has no task here to cancel"
        );
        assert!(queue.poll().is_empty(), "polling one is a no-op");
        queue.resolve(
            id,
            Outcome {
                status: Status::Succeeded,
                duration: Duration::from_secs(3),
            },
        );
        assert_eq!(queue.job(id).expect("listed").state.label(), "done");
        assert_eq!(queue.job(id).expect("listed").elapsed().as_secs(), 3);
    }

    #[test]
    fn history_is_bounded_but_never_drops_a_running_job() {
        let mut queue = Queue::new();
        let long = queue.adopt("release", "cargo build --release");
        for _ in 0..HISTORY + 5 {
            let id = queue.adopt("short", "true");
            queue.resolve(
                id,
                Outcome {
                    status: Status::Succeeded,
                    duration: Duration::ZERO,
                },
            );
            // `trim` runs on the next start, which is what the loop is for.
            queue.adopt("probe", "true");
            let probe = queue.jobs().last().expect("just added").id;
            queue.resolve(
                probe,
                Outcome {
                    status: Status::Succeeded,
                    duration: Duration::ZERO,
                },
            );
        }
        queue.adopt("one more", "true");
        assert!(
            queue.job(long).is_some(),
            "the long-running job survived every trim"
        );
        let finished = queue
            .jobs()
            .iter()
            .filter(|job| !job.state.is_running())
            .count();
        assert!(finished <= HISTORY, "{finished} finished jobs kept");
        queue.cancel_all();
    }

    #[test]
    fn clearing_keeps_what_is_still_running() {
        let mut queue = Queue::new();
        let running = queue.adopt("build", "cargo build");
        let done = queue.adopt("fmt", "rustfmt");
        queue.resolve(
            done,
            Outcome {
                status: Status::Succeeded,
                duration: Duration::ZERO,
            },
        );
        queue.clear_finished();
        assert_eq!(queue.jobs().len(), 1);
        assert_eq!(queue.jobs()[0].id, running);
    }
}
