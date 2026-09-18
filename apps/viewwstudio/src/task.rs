//! N3: one long child process, streamed, cancellable, polled from a frame hook.
//!
//! `compile::Job` proved the shape — a worker thread, a cancel flag, an `mpsc`
//! back to the UI thread, and a waker so a finished job causes a frame on an
//! idle window. Plan 2 §8.1 asks for that shape generalised, because `cargo
//! build`, `cargo run`, `adb install`, `xcodebuild` and a workspace-wide search
//! are all the same problem. This is that generalisation.
//!
//! `Job` is deliberately **not** rewritten in terms of this. It has one thing
//! this does not: it produces a `Compiled` — a typed value with a library path
//! — where a `Task` produces output and an exit status. Forcing the preview
//! pipeline through a stringly-typed channel to save a hundred lines would make
//! the one path in the studio that must never regress depend on the newest code
//! in it.
//!
//! # What this adds over `Job`
//!
//! | | `Job` | `Task` |
//! |---|---|---|
//! | result | one value at the end | **a stream of lines while it runs** |
//! | streams | `rustc`'s stderr, read at exit | stdout and stderr, tagged, live |
//! | failure to start | an `io::Error` at the call site | a [`Status::NotStarted`] event |
//!
//! The streaming is the point. A `cargo build` takes minutes and prints as it
//! goes; collecting that and showing it at the end is indistinguishable from a
//! hang.
//!
//! # Two ordering guarantees
//!
//! Both are asserted by tests, because both are the kind of thing that works on
//! a fast machine and fails on a loaded one:
//!
//! 1. **Every [`Event::Line`] arrives before [`Event::Finished`].** The
//!    supervisor joins both reader threads before sending the outcome. Without
//!    that join the panel prints "Build failed" and then, a frame later, the
//!    error that explains why — which reads as two unrelated events.
//! 2. **[`Event::Finished`] is sent exactly once**, whatever happened. Cancel,
//!    timeout, a program that does not exist, a panicking reader: each ends in
//!    one outcome, so a caller can drive a state machine off it.
//!
//! # What is *not* guaranteed
//!
//! **The interleaving of stdout and stderr.** They are two pipes drained by two
//! threads and the OS does not order them relative to each other. This is why
//! [`Event::Line`] carries its [`Stream`]: `cargo --message-format=json` puts
//! machine-readable records on stdout and human progress on stderr, and a
//! consumer that parsed both would try to read `Compiling app v0.1.0` as JSON.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Which pipe a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
}

/// Something a running task did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// One line, with its trailing newline removed.
    Line(Stream, String),
    /// The task ended. Always last, and always exactly one of these.
    Finished(Outcome),
}

/// How a task ended, and how long it took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub status: Status,
    pub duration: Duration,
}

impl Outcome {
    #[must_use]
    pub const fn succeeded(&self) -> bool {
        matches!(self.status, Status::Succeeded)
    }
}

/// The verdict on a finished task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Succeeded,
    /// A non-zero exit. `code` is `None` when a signal ended it — which on Unix
    /// is what a segfaulting build script looks like, and is not the same as
    /// "exited with an error", so it is not flattened into one.
    Failed {
        code: Option<i32>,
    },
    /// [`Task::cancel`] was called and the child was killed.
    ///
    /// Distinct from `Failed` on purpose: a killed process exits non-zero, and
    /// reporting the user's own Cancel click as a build failure is a lie the
    /// Output panel would keep on screen.
    Cancelled,
    /// The task outlived its [`Spec::timeout`] and was killed.
    TimedOut,
    /// The program could not be started at all — almost always "not found",
    /// which is the single most likely failure of `cargo-ndk` or `xcodebuild`
    /// on a machine that has never had them.
    NotStarted(String),
    /// The supervisor thread went away without a verdict. Only reachable if it
    /// panicked, which is a bug here — reported rather than swallowed, so it
    /// cannot look like a build that never ends.
    Lost,
}

impl Status {
    /// A sentence for the Output panel.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Succeeded => "finished".into(),
            Self::Failed { code: Some(code) } => format!("failed with exit code {code}"),
            Self::Failed { code: None } => "killed by a signal".into(),
            Self::Cancelled => "cancelled".into(),
            Self::TimedOut => "timed out".into(),
            Self::NotStarted(why) => format!("could not start: {why}"),
            Self::Lost => "lost: the supervisor thread went away".into(),
        }
    }
}

/// What to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// What the Output panel calls this — "Build (debug)", "Run", "adb install".
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// The directory to run in. For cargo this decides which project is built,
    /// so it is required rather than defaulted to the studio's own cwd — which
    /// is wherever the user happened to launch it from.
    pub dir: PathBuf,
    /// Extra environment. Applied on top of the studio's own.
    pub env: Vec<(String, String)>,
    /// A backstop, not a responsiveness guard — the task is off the UI thread
    /// and cancellable, exactly as `compile::TIMEOUT` explains. `None` for the
    /// honest case: a release build of a large project has no sane upper bound.
    pub timeout: Option<Duration>,
}

impl Spec {
    /// A task with no extra environment and no timeout.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        program: impl Into<String>,
        dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            args: Vec::new(),
            dir: dir.into(),
            env: Vec::new(),
            timeout: None,
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Run somewhere other than where [`new`](Self::new) was pointed.
    #[must_use]
    pub fn dir_of(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = dir.into();
        self
    }

    /// The command as a person would type it, for the first line of the Output
    /// panel. Not shell-quoted: it is for reading, not for re-running.
    #[must_use]
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How often the supervisor checks on the child.
///
/// Fast enough that Cancel feels immediate, slow enough that a ten-minute build
/// costs a few thousand cheap `try_wait` calls rather than a busy core.
const TICK: Duration = Duration::from_millis(10);

/// A running child process.
#[derive(Debug)]
pub struct Task {
    events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    /// What the Output panel calls this.
    pub label: String,
    pub started: Instant,
    /// Whether an [`Event::Finished`] has already been handed out, so a poll
    /// after the end cannot invent a second outcome from a closed channel.
    finished: std::cell::Cell<bool>,
}

impl Task {
    /// Start `spec` on a worker thread.
    ///
    /// `wake` is called when there is something new to show and once at the
    /// end. It is what makes output appear on an idle window; the studio hands
    /// it the platform waker.
    ///
    /// Never fails. A program that cannot be started arrives as
    /// [`Status::NotStarted`] through the same channel as everything else,
    /// which means the UI has **one** path from "started a task" to "the task
    /// ended" instead of two.
    #[must_use]
    pub fn spawn(spec: Spec, wake: impl Fn() + Send + 'static) -> Self {
        let (sender, events) = std::sync::mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let label = spec.label.clone();
        let flag = Arc::clone(&cancelled);

        let thread = std::thread::Builder::new()
            .name(format!("viewwstudio-task-{label}"))
            .spawn(move || supervise(spec, &sender, &flag, wake));

        let started = Instant::now();

        if let Err(error) = thread {
            // A thread that cannot be spawned is a machine out of resources.
            // The channel above is already dropped, so answer through a fresh
            // one rather than leaving a Task that never finishes.
            let (sender, events) = std::sync::mpsc::channel();
            sender
                .send(Event::Finished(Outcome {
                    status: Status::NotStarted(error.to_string()),
                    duration: Duration::ZERO,
                }))
                .ok();
            return Self {
                events,
                cancelled,
                label,
                started,
                finished: std::cell::Cell::new(false),
            };
        }

        Self {
            events,
            cancelled,
            label,
            started,
            finished: std::cell::Cell::new(false),
        }
    }

    /// Ask the task to stop.
    ///
    /// Advisory, like [`compile::Job::cancel`](crate::compile::Job::cancel):
    /// the child is killed at the next `TICK`, so the UI treats the task as
    /// going away now rather than waiting for a confirmation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Whether an outcome has already been reported.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.finished.get()
    }

    /// Everything the task has produced since the last call. Non-blocking.
    ///
    /// Called once a frame from the studio's frame hook, on the UI thread,
    /// which is where writing signals is allowed.
    pub fn poll(&self) -> Vec<Event> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => {
                    if matches!(event, Event::Finished(_)) {
                        self.finished.set(true);
                    }
                    out.push(event);
                }
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => {
                    // The supervisor is gone. If it never said how it ended,
                    // say so here — a task with no outcome is a spinner that
                    // never stops.
                    if !self.finished.get() {
                        self.finished.set(true);
                        out.push(Event::Finished(Outcome {
                            status: Status::Lost,
                            duration: self.started.elapsed(),
                        }));
                    }
                    return out;
                }
            }
        }
    }

    /// Anything that arrives *after* the outcome, which must be nothing.
    ///
    /// **Tests only.** Waits a beat rather than polling once, because the
    /// failure being looked for is a reader thread that is still running — and
    /// a single non-blocking poll would miss it by exactly the margin that
    /// makes the bug intermittent.
    #[cfg(test)]
    fn stragglers(&self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.recv_timeout(Duration::from_millis(100)) {
            out.push(event);
        }
        out
    }

    /// Block until the task ends, collecting everything. **Tests only** — this
    /// is the UI thread in the studio, and blocking it is the freeze the worker
    /// thread exists to prevent.
    #[cfg(test)]
    fn wait(&self) -> (Vec<Event>, Outcome) {
        let mut events = Vec::new();
        loop {
            match self.events.recv() {
                Ok(Event::Finished(outcome)) => {
                    self.finished.set(true);
                    return (events, outcome);
                }
                Ok(event) => events.push(event),
                Err(_) => {
                    return (
                        events,
                        Outcome {
                            status: Status::Lost,
                            duration: self.started.elapsed(),
                        },
                    )
                }
            }
        }
    }
}

/// The supervisor thread: start the child, drain both pipes, decide the verdict.
fn supervise(
    spec: Spec,
    sender: &Sender<Event>,
    cancelled: &Arc<AtomicBool>,
    wake: impl Fn() + Send + 'static,
) {
    let started = Instant::now();
    let finish = |status: Status| {
        sender
            .send(Event::Finished(Outcome {
                status,
                duration: started.elapsed(),
            }))
            .ok();
        wake();
    };

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    // Its own process group, so cancelling can take the whole build down rather
    // than just the process at the top of it. See `kill_tree`.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            // The message the user needs is "which program", and `io::Error`
            // does not include it: "No such file or directory (os error 2)"
            // about an unnamed thing is a bug report nobody can act on.
            finish(Status::NotStarted(format!("{}: {error}", spec.program)));
            return;
        }
    };

    // Something was printed since the last wake. Set by the readers, cleared by
    // the supervisor, so a build that prints ten thousand lines asks for frames
    // at the tick rate rather than ten thousand times.
    let pending = Arc::new(AtomicBool::new(false));

    let out = child
        .stdout
        .take()
        .map(|pipe| drain(pipe, Stream::Out, sender.clone(), Arc::clone(&pending)));
    let err = child
        .stderr
        .take()
        .map(|pipe| drain(pipe, Stream::Err, sender.clone(), Arc::clone(&pending)));

    let status = watch(&mut child, &spec, cancelled, started, &pending, &wake);

    // **Before the outcome, not after.** Joining the readers is what makes
    // "every line arrives before Finished" true; without it the last few lines
    // of a failed build land after the failure they explain.
    for reader in [out, err].into_iter().flatten() {
        reader.join().ok();
    }
    if pending.swap(false, Ordering::Relaxed) {
        wake();
    }

    finish(status);
}

/// Wait for the child, killing it if asked or if it runs too long.
fn watch(
    child: &mut Child,
    spec: &Spec,
    cancelled: &Arc<AtomicBool>,
    started: Instant,
    pending: &Arc<AtomicBool>,
    wake: &(impl Fn() + Send + 'static),
) -> Status {
    loop {
        match child.try_wait() {
            Ok(Some(exit)) => {
                return if exit.success() {
                    Status::Succeeded
                } else {
                    Status::Failed { code: exit.code() }
                }
            }
            Ok(None) => {}
            Err(error) => return Status::NotStarted(error.to_string()),
        }

        if cancelled.load(Ordering::Relaxed) {
            kill_tree(child);
            return Status::Cancelled;
        }

        if spec.timeout.is_some_and(|limit| started.elapsed() > limit) {
            kill_tree(child);
            return Status::TimedOut;
        }

        // One frame's worth of output, coalesced into one wake.
        if pending.swap(false, Ordering::Relaxed) {
            wake();
        }

        std::thread::sleep(TICK);
    }
}

/// End the child **and everything it started**.
///
/// # Why `Child::kill` is not enough, found by a test
///
/// `Child::kill` sends `SIGKILL` to one process id. The processes that matter
/// here all spawn others: `cargo` runs `rustc` once per crate, `cargo-ndk` runs
/// a linker, a shell script runs whatever it likes. Killing only the parent
/// leaves the grandchildren running — and, because they inherited the write
/// ends of the pipes, **the pipes never close**, so the reader threads block
/// for as long as the orphans live.
///
/// The first version of this file did exactly that. Its cancel test asked
/// `/bin/sh -c 'sleep 30'` to stop, and the outcome arrived thirty seconds
/// later: `sh` died instantly and `sleep` held the pipe. In the studio that is
/// a Cancel button that appears to do nothing while a full `cargo build` keeps
/// a core busy in the background.
///
/// So the child is made a **process group leader** at spawn time, and the whole
/// group is signalled here. `kill(2)` with a negative pid is the call that does
/// it; reaching it through `/bin/kill` rather than `libc` keeps the studio's
/// dependency list where it is, and this is a once-per-cancel cost on a path
/// that is already killing processes.
///
/// # Windows gets the same guarantee, through the same kind of seam
///
/// This used to fall back to `Child::kill` off Unix, which is the exact defect
/// the paragraph above describes: `cargo` died, every `rustc` it had started
/// kept the pipe open, the `Finished` event never arrived, and the studio's job
/// queue stayed stuck on a build the user had already cancelled — with the
/// orphans still burning cores until they were killed by hand in Task Manager.
///
/// `taskkill /PID <pid> /T /F` is the Windows answer, and it is reached the
/// same way the Unix one is: through a program that ships with the operating
/// system rather than through an FFI dependency. `/T` is the tree — taskkill
/// walks the process snapshot's parent links and terminates every descendant —
/// and `/F` makes it a terminate rather than a request, which matters because a
/// `rustc` has no message loop to receive the polite version.
///
/// A Job Object created at spawn time with `KILL_ON_JOB_CLOSE` is the stronger
/// guarantee, because it also catches a grandchild whose parent has already
/// exited and been re-parented. It needs `windows-sys`, `unsafe`, and a Windows
/// machine to verify on. `taskkill` covers the case this studio actually has —
/// `cargo` is alive right up until it is killed, so its children are still
/// reachable from it — and needs neither.
fn kill_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // The group id equals the leader's pid, which is the child's, because
        // of `process_group(0)` at spawn.
        let group = child.id().to_string();
        let killed = Command::new("/bin/kill")
            .args(["-s", "KILL", "--", &format!("-{group}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !killed {
            // No `/bin/kill`, or the group was already gone. Either way the
            // direct kill is still worth trying and cannot make things worse.
            child.kill().ok();
        }
    }
    #[cfg(not(unix))]
    {
        let pid = child.id().to_string();
        let killed = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !killed {
            // Already gone, or a Windows so stripped it has no `taskkill`.
            // The direct kill cannot make either case worse.
            child.kill().ok();
        }
    }
    child.wait().ok();
}

/// Read one pipe line by line, forwarding each into the channel.
///
/// `read_until` and `from_utf8_lossy` rather than `BufRead::lines`, which
/// yields an `Err` for a line that is not UTF-8 and stops. A build script that
/// prints a byte in someone's local encoding would end the stream there, and
/// the Output panel would show a build that went quiet rather than one that
/// printed something odd.
fn drain(
    pipe: impl Read + Send + 'static,
    stream: Stream,
    sender: Sender<Event>,
    pending: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                        buffer.pop();
                    }
                    let line = String::from_utf8_lossy(&buffer).into_owned();
                    if sender.send(Event::Line(stream, line)).is_err() {
                        // The Task was dropped — the user closed the window, or
                        // cancelled. Nothing to report to.
                        return;
                    }
                    pending.store(true, Ordering::Relaxed);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Every test runs a real child process. A mocked one would test the mock:
    /// the properties here — pipe draining, kill, exit codes, ordering — are
    /// properties of the operating system's process handling, not of this file.
    ///
    /// **`sh` by name on Windows, `/bin/sh` by path elsewhere.** There is no
    /// `/bin/sh` on Windows, and hard-coding it made all twenty of these tests
    /// fail on the runner with `NotStarted("/bin/sh: The system cannot find
    /// the path specified")` — which says nothing about pipes, kills or exit
    /// codes, the things they exist to check. Git for Windows puts a POSIX
    /// `sh` on `PATH` (the studio's own `setup::shell` is a different
    /// question: that one runs a *user's* command line, so it uses `cmd`).
    fn sh(script: &str) -> Spec {
        let program = if cfg!(windows) { "sh" } else { "/bin/sh" };
        Spec::new("test", program, std::env::temp_dir())
            .arg("-c")
            .arg(script)
    }

    fn lines(events: &[Event], stream: Stream) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Line(s, line) if *s == stream => Some(line.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn stdout_and_stderr_arrive_tagged() {
        let task = Task::spawn(sh("echo out; echo err 1>&2; echo out2"), || {});
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        assert!(outcome.succeeded());
        assert_eq!(lines(&events, Stream::Out), ["out", "out2"]);
        assert_eq!(lines(&events, Stream::Err), ["err"]);
    }

    #[test]
    fn a_non_zero_exit_carries_its_code() {
        let task = Task::spawn(sh("exit 3"), || {});
        assert_eq!(task.wait().1.status, Status::Failed { code: Some(3) });
    }

    /// The property the Output panel depends on, tested where it can actually
    /// break.
    ///
    /// # Why the child outlives itself here
    ///
    /// The obvious version — print a lot, exit, check the order — does not test
    /// this. The writer cannot exit until the reader has taken almost
    /// everything, so by the time the supervisor notices the exit there is
    /// nothing left in flight and the outcome is last whether or not the
    /// readers were joined. Removing the join left that version green while
    /// failing *other* tests intermittently, under parallel load only.
    ///
    /// This shape makes it deterministic: the shell exits at once and leaves a
    /// grandchild holding the pipe, which prints 300ms later. Joining the
    /// readers is then the only thing that can keep `late` ahead of the
    /// outcome — and it is the honest behaviour too, because that line is
    /// output of the task and the task is not over until its output is.
    #[test]
    fn the_outcome_is_last_even_when_output_outlives_the_child() {
        let task = Task::spawn(sh("( sleep 0.3; echo late ) & exit 0"), || {});
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        assert_eq!(
            lines(&events, Stream::Out),
            ["late"],
            "the line printed after the child exited must arrive before the outcome"
        );
        assert_eq!(task.stragglers(), Vec::new());
    }

    /// The other half: under a flood, no line may be dropped.
    #[test]
    fn every_line_arrives_before_the_outcome() {
        // Enough lines to overflow any pipe buffer, so the reader is genuinely
        // still working when the child exits. A handful of lines fits in the
        // kernel buffer and passes whether or not the join is there.
        let task = Task::spawn(
            sh("i=0; while [ $i -lt 4000 ]; do echo line-$i; i=$((i+1)); done"),
            || {},
        );
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        let out = lines(&events, Stream::Out);
        assert_eq!(out.len(), 4000, "no line may be dropped");
        assert_eq!(out[0], "line-0");
        assert_eq!(out[3999], "line-3999");
        // `wait` stops at the outcome, so the count above only proves the lines
        // that arrived *first*. This is the other half, and the half that
        // catches a supervisor which does not join its readers: nothing at all
        // may arrive after the outcome.
        assert_eq!(
            task.stragglers(),
            Vec::new(),
            "a line arrived after the task said it had finished"
        );
        assert!(!events.iter().any(|e| matches!(e, Event::Finished(_))));
    }

    #[test]
    fn a_cancelled_task_is_cancelled_not_failed() {
        let task = Task::spawn(sh("sleep 30"), || {});
        // Let it actually start, so this tests killing a running child rather
        // than a race with spawn.
        std::thread::sleep(Duration::from_millis(80));
        task.cancel();
        assert!(task.is_cancelled());
        let outcome = task.wait().1;
        assert_eq!(outcome.status, Status::Cancelled);
        assert!(
            outcome.duration < Duration::from_secs(5),
            "cancel must not wait for the child's own end: {:?}",
            outcome.duration
        );
    }

    /// The finding that produced `kill_tree`, kept as a test.
    ///
    /// `cargo` is a process that spawns `rustc`. Killing only the top process
    /// leaves the real work running **and** holds the pipes open, so the
    /// outcome does not arrive until the orphan finishes on its own. The first
    /// version of this file took thirty seconds to report a cancelled `sleep
    /// 30`; with the group kill it takes milliseconds, and the grandchild is
    /// gone rather than orphaned.
    #[cfg(unix)]
    #[test]
    fn cancelling_kills_what_the_child_started_too() {
        // Print the grandchild's pid, then hold the shell open so the pipe
        // stays owned by something the direct kill would not reach.
        let task = Task::spawn(sh("sleep 30 & echo $!; wait"), || {});
        std::thread::sleep(Duration::from_millis(150));
        task.cancel();

        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Cancelled);
        assert!(
            outcome.duration < Duration::from_secs(5),
            "an orphan holding the pipe delayed the outcome by {:?}",
            outcome.duration
        );

        let pid = lines(&events, Stream::Out)
            .first()
            .expect("the shell printed the grandchild's pid")
            .clone();

        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !still_running(&pid),
            "the grandchild (pid {pid}) outlived the cancel"
        );
    }

    /// Whether a pid names a process that is still *running*.
    ///
    /// # Why this is not `kill -0`
    ///
    /// `kill -0` asks whether a pid exists, and a **zombie** exists: a killed
    /// process stays in the table until its parent reaps it, and this one's
    /// parent was killed alongside it, so it is reparented to pid 1 — which, in
    /// a container, is frequently not an init that reaps anything.
    ///
    /// The first version of the test above used `kill -0` and failed against a
    /// working group kill, reporting a corpse as a survivor. The state field of
    /// `/proc/<pid>/stat` tells them apart: `Z` is dead-but-unreaped.
    ///
    /// The state is the **third** field, and the second is the executable name
    /// in parentheses — which may itself contain spaces and parentheses, so it
    /// is skipped by finding the *last* `)` rather than by splitting.
    #[cfg(unix)]
    fn still_running(pid: &str) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            // No proc entry, or not Linux: gone, or unanswerable. Either way
            // this test is not the place to guess.
            return false;
        };
        let Some(after_name) = stat.rfind(')').map(|at| &stat[at + 1..]) else {
            return false;
        };
        !matches!(after_name.split_whitespace().next(), Some("Z") | None)
    }

    #[test]
    fn a_task_that_runs_too_long_times_out() {
        let task = Task::spawn(sh("sleep 30").timeout(Duration::from_millis(120)), || {});
        let outcome = task.wait().1;
        assert_eq!(outcome.status, Status::TimedOut);
        assert!(outcome.duration < Duration::from_secs(5));
    }

    #[test]
    fn no_timeout_means_no_timeout() {
        let task = Task::spawn(sh("sleep 0.3; echo done"), || {});
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        assert_eq!(lines(&events, Stream::Out), ["done"]);
    }

    /// The failure a machine without `cargo-ndk` or `xcodebuild` gets, and the
    /// reason `spawn` does not return a `Result`: it comes back through the
    /// same channel as a normal end, so the caller's state machine has one exit.
    #[test]
    fn a_program_that_does_not_exist_finishes_rather_than_failing_to_start() {
        let task = Task::spawn(
            Spec::new(
                "build",
                "definitely-not-a-real-program",
                std::env::temp_dir(),
            ),
            || {},
        );
        let outcome = task.wait().1;
        match outcome.status {
            Status::NotStarted(ref why) => assert!(
                why.contains("definitely-not-a-real-program"),
                "the message must name the program: {why}"
            ),
            other => panic!("expected NotStarted, got {other:?}"),
        }
    }

    #[test]
    fn the_working_directory_is_the_one_asked_for() {
        let dir = std::env::temp_dir().join("vieww-task-cwd-test");
        std::fs::create_dir_all(&dir).unwrap();
        let task = Task::spawn(sh("pwd").dir_of(&dir), || {});
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        let printed = lines(&events, Stream::Out).remove(0);
        // macOS reports /private/var for /var, so compare what the shell itself
        // resolves rather than the path as written.
        assert!(
            printed.ends_with("vieww-task-cwd-test"),
            "ran in {printed}, expected {}",
            dir.display()
        );
    }

    #[test]
    fn the_environment_reaches_the_child() {
        let task = Task::spawn(
            sh("echo $VIEWW_TASK_TEST").env("VIEWW_TASK_TEST", "hello"),
            || {},
        );
        let (events, _) = task.wait();
        assert_eq!(lines(&events, Stream::Out), ["hello"]);
    }

    /// Output that is not UTF-8 must not truncate the stream. `BufRead::lines`
    /// returns an `Err` and stops; this keeps going with a replacement char.
    #[test]
    fn invalid_utf8_does_not_end_the_stream() {
        let task = Task::spawn(sh("printf 'a\\xffb\\n'; echo after"), || {});
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        let out = lines(&events, Stream::Out);
        assert_eq!(
            out.len(),
            2,
            "the line after the bad byte must survive: {out:?}"
        );
        assert_eq!(out[1], "after");
    }

    #[test]
    fn a_line_with_no_trailing_newline_still_arrives() {
        let task = Task::spawn(sh("printf 'no-newline'"), || {});
        let (events, _) = task.wait();
        assert_eq!(lines(&events, Stream::Out), ["no-newline"]);
    }

    #[test]
    fn carriage_returns_are_stripped_with_the_newline() {
        let task = Task::spawn(sh("printf 'windows\\r\\n'"), || {});
        let (events, _) = task.wait();
        assert_eq!(lines(&events, Stream::Out), ["windows"]);
    }

    /// The waker is what makes output appear on an idle window. It must fire —
    /// and it must not fire once per line, which is what the coalescing tick is
    /// for.
    #[test]
    fn the_waker_fires_but_not_once_per_line() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let task = Task::spawn(
            sh("i=0; while [ $i -lt 4000 ]; do echo line-$i; i=$((i+1)); done"),
            move || {
                counter.fetch_add(1, Ordering::Relaxed);
            },
        );
        let (events, outcome) = task.wait();
        assert_eq!(outcome.status, Status::Succeeded);
        assert_eq!(lines(&events, Stream::Out).len(), 4000);

        let count = wakes.load(Ordering::Relaxed);
        assert!(count >= 1, "a finished task must cause a frame");
        assert!(
            count < 200,
            "4000 lines asked for {count} frames; the tick coalescing is not working"
        );
    }

    #[test]
    fn poll_is_non_blocking_and_reports_the_end_once() {
        let task = Task::spawn(sh("echo hi"), || {});
        let mut all = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !task.is_finished() && Instant::now() < deadline {
            all.extend(task.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(task.is_finished(), "the task never reported an end");
        assert_eq!(lines(&all, Stream::Out), ["hi"]);
        let finishes = all
            .iter()
            .filter(|e| matches!(e, Event::Finished(_)))
            .count();
        assert_eq!(finishes, 1, "exactly one outcome");
        // Polling past the end invents nothing.
        assert_eq!(task.poll(), Vec::new());
        assert_eq!(task.poll(), Vec::new());
    }

    #[test]
    fn the_command_line_reads_as_typed() {
        let spec = Spec::new("Build", "cargo", "/w").args(["build", "--release"]);
        assert_eq!(spec.command_line(), "cargo build --release");
    }
}
