//! N3: Build, Build-and-Run, and the state a person watches while it happens.
//!
//! [`task::Task`](crate::task) runs a child process and [`cargo`]
//! reads what it says. This is the piece between them and the window: one
//! in-flight build, the output it has produced, the diagnostics parsed out of
//! it, and what to do when it succeeds.
//!
//! # The rule from plan 2 §4.3, enforced here
//!
//! > **Preview renders a widget. Build produces an application.**
//!
//! They stay different mechanisms with different costs. This module never
//! touches [`compile::Job`](crate::compile) and never escalates a preview into
//! a build; [`Kind`] names which one a person asked for so the UI can say so
//! rather than blur them.
//!
//! # Why the state is here and not in signals
//!
//! Everything below is plain data with no `Signal`, no `Rc` and no widget. The
//! studio holds one of these and mirrors it into signals in its frame hook, the
//! same way it already polls `compile::Job`. That is what lets the whole
//! build-and-run state machine — including a failing build, a cancelled one,
//! and one whose binary then crashes — be tested against a **fake cargo**: a
//! shell script that prints real cargo JSON and exits with a chosen code. The
//! tests at the bottom do exactly that, so the parsing, the ordering, the
//! refusal rules and the run-after-build handoff are all checked without a
//! project, a network, or a twenty-second wait.

use std::path::PathBuf;
use std::time::Duration;

use crate::cargo::{self, Message};
use crate::state::Diagnostic;
use crate::task::{Event, Outcome, Spec, Status, Stream, Task};
use crate::toolchains::{self, Env, Report, Target};

/// Which of the two long operations is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `cargo build`.
    Build,
    /// `cargo build`, and then run what it produced.
    BuildAndRun,
    /// The binary a successful `BuildAndRun` produced, now running.
    Run,
}

impl Kind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Build => "Build",
            Self::BuildAndRun => "Build and Run",
            Self::Run => "Run",
        }
    }
}

/// What the toolbar and status bar show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum State {
    /// Nothing has been built this session.
    #[default]
    Idle,
    Running(Kind),
    /// The last build finished. `warnings` is a count because a build with
    /// warnings is a success that still has something to say.
    Succeeded {
        kind: Kind,
        warnings: usize,
    },
    /// It failed, or was cancelled, or could not start. The reason is the
    /// task's own, so "cancelled" never reads as "failed".
    Failed {
        kind: Kind,
        status: Status,
    },
}

impl State {
    #[must_use]
    pub const fn busy(&self) -> bool {
        matches!(self, Self::Running(_))
    }

    /// Whether this state is worth putting in the status light at all.
    ///
    /// `Idle` is not: it means no build has run this session, and a status
    /// light announcing that would be reporting an absence over the preview's
    /// actual news.
    #[must_use]
    pub const fn reportable(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// The status-light phrase, naming *which* operation it is about.
    ///
    /// "Build failed" and "Run failed" rather than a bare "Failed": this cell
    /// is shared with the preview pipeline, and the whole reason it now
    /// arbitrates between the two is that an unqualified verdict left the user
    /// to guess which of them it belonged to.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Idle => "No build yet".to_string(),
            Self::Running(Kind::Run) => "Running…".to_string(),
            Self::Running(_) => "Building…".to_string(),
            Self::Succeeded { kind, warnings: 0 } => format!("{} succeeded", kind.label()),
            Self::Succeeded { kind, warnings } => format!(
                "{} succeeded · {warnings} warning{}",
                kind.label(),
                if *warnings == 1 { "" } else { "s" }
            ),
            // A cancelled build is not a failed one, and the task's own status
            // is what knows the difference. Reporting "Build failed" for a
            // build the user stopped on purpose is the same class of lie this
            // cell was fixed to stop telling.
            Self::Failed {
                kind,
                status: Status::Cancelled,
            } => format!("{} cancelled", kind.label()),
            Self::Failed { kind, .. } => format!("{} failed", kind.label()),
        }
    }

    /// Whether this state should paint the light red.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// A build profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    #[must_use]
    pub const fn flag(self) -> Option<&'static str> {
        match self {
            Self::Debug => None,
            Self::Release => Some("--release"),
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// Why a build was refused before it started.
///
/// Refusing is a feature, not an error path: plan 2 §6.2 asks that a build
/// which cannot succeed never start, because the alternative is failing twenty
/// minutes in on a missing NDK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Something is already running. One at a time, like the preview.
    Busy(Kind),
    /// The open directory is not a Cargo project that depends on vieww.
    NotAProject,
    /// The target's toolchain is incomplete or impossible here. Carries the
    /// sentence [`Report::refusal`] wrote, which names everything missing.
    Toolchain(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy(kind) => write!(f, "{} is already running", kind.label()),
            Self::NotAProject => write!(
                f,
                "this folder has no Cargo.toml that depends on vieww, so there is nothing to build"
            ),
            Self::Toolchain(why) => write!(f, "{why}"),
        }
    }
}

/// One in-flight build and everything it has said.
#[derive(Debug, Default)]
pub struct Builds {
    task: Option<Task>,
    kind: Option<Kind>,
    /// Set when a `BuildAndRun` is in its build half, so the run can start the
    /// moment the build succeeds without the UI having to notice and re-ask.
    run_after_build: bool,
    /// The project directory the current operation belongs to.
    root: Option<PathBuf>,
    /// Every line, both streams, in arrival order — the Output panel's content.
    pub output: Vec<String>,
    /// Diagnostics parsed from this build. Replaced wholesale at the *start* of
    /// a build, never appended across builds: a Problems panel showing errors
    /// from two builds ago is worse than an empty one.
    pub diagnostics: Vec<Diagnostic>,
    pub state: State,
    /// The binary the last successful build produced, if it made one.
    pub binary: Option<PathBuf>,
    /// How long the last finished operation took.
    pub last_duration: Option<Duration>,
}

impl Builds {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            ..Default::default()
        }
    }

    /// Whether something is running right now.
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.task.is_some()
    }

    /// Check everything that can be checked before spending a build on it.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming what stopped it, which is what the UI shows next to
    /// the disabled button.
    pub fn check(&self, root: &std::path::Path, target: Target, env: &Env) -> Result<(), Refusal> {
        if let Some(kind) = self.kind.filter(|_| self.busy()) {
            return Err(Refusal::Busy(kind));
        }
        if !crate::scaffold::is_vieww_project(root) {
            return Err(Refusal::NotAProject);
        }
        let report: Report = toolchains::report(env, target);
        report
            .refusal()
            .map_or(Ok(()), |why| Err(Refusal::Toolchain(why)))
    }

    /// The command a build runs.
    ///
    /// Separate from [`start`](Self::start) so a test — and the Output panel's
    /// first line — can see exactly what is about to run without running it.
    #[must_use]
    pub fn spec(root: &std::path::Path, target: Target, profile: Profile, kind: Kind) -> Spec {
        let mut spec = Spec::new(kind.label(), "cargo", root);

        // **`--bin` on the desktop targets, and it is worth 1.3 GB.**
        //
        // A scaffolded project's `[lib]` declares
        // `crate-type = ["rlib", "cdylib", "staticlib"]` — the rlib for the
        // desktop binary, the cdylib because an Android activity loads a shared
        // library, the staticlib because `UIApplicationMain` links against one.
        // All three are needed, and none of them is needed *here*.
        //
        // A plain `cargo build` builds every declared crate type, so pressing
        // Build for the desktop produced the Android and iOS libraries too.
        // Measured on a freshly scaffolded project, debug:
        //
        //     libsaydemo.a     813.5 MB   (iOS)
        //     libsaydemo.so    228.4 MB   (Android)
        //     libsaydemo.rlib   41.7 MB
        //     saydemo          260.5 MB
        //
        // — a gigabyte of artefacts for two platforms the person is not
        // building for, on every build, before anything is even wrong. Naming
        // the binary is the whole fix: cargo then builds the rlib it links
        // against and stops.
        //
        // The mobile targets keep the plain `build`, because there the cdylib
        // and the staticlib are exactly what is wanted.
        let binary = crate::scaffold::binary_name(root)
            .unwrap_or_else(|| crate::harness::lib_name("app").replace('_', "-"));

        spec = match target {
            // `cargo-ndk` is a cargo subcommand, so the program stays `cargo`
            // and the target becomes part of the argument list.
            Target::Android => spec.args(["ndk", "-t", "arm64-v8a", "build"]),
            // **iOS used to fall into the arm below**, which ran a plain
            // `cargo build` and produced a binary for the machine the studio
            // was running on — a desktop build wearing an iOS label, which is
            // worse than a refusal because it looks like it worked. The target
            // triple is what makes it an iOS build.
            //
            // `aarch64-apple-ios` is the device triple; every iPhone since the
            // 5s is arm64, so there is no second architecture to fatten into.
            // The simulator's `aarch64-apple-ios-sim` is a separate format in
            // `export.rs` rather than a branch here, because the two differ in
            // what is done with the artefact and not only in how it is built.
            Target::Ios => spec.args(["build", "--target", "aarch64-apple-ios"]),
            // A cross-compile from anywhere that is not Windows, and a plain
            // build when it is. Same reasoning as `export::windows`, and the
            // same triple, so `Build` and `Export ▸ Windows .exe` cannot
            // disagree about what a Windows build is.
            Target::Windows if !cfg!(windows) => spec.args([
                "build",
                "--bin",
                &binary,
                "--target",
                "x86_64-pc-windows-gnu",
            ]),
            Target::Desktop | Target::Windows => spec.args(["build", "--bin", &binary]),
        };

        if let Some(flag) = profile.flag() {
            spec = spec.arg(flag);
        }

        // The whole reason the Problems panel can be filled while the build is
        // still running.
        spec.arg("--message-format=json")
            // Colour codes in a panel that does not interpret them are noise,
            // and cargo turns them on whenever it thinks a terminal is watching.
            .env("CARGO_TERM_COLOR", "never")
    }

    /// Start a build. Call [`check`](Self::check) first — this trusts it.
    ///
    /// `wake` is the platform waker, as for [`Task::spawn`].
    pub fn start(
        &mut self,
        root: &std::path::Path,
        target: Target,
        profile: Profile,
        kind: Kind,
        wake: impl Fn() + Send + 'static,
    ) {
        self.output.clear();
        self.diagnostics.clear();
        self.binary = None;
        self.root = Some(root.to_path_buf());
        self.run_after_build = kind == Kind::BuildAndRun;

        let spec = Self::spec(root, target, profile, kind);
        self.output.push(format!("$ {}", spec.command_line()));
        self.state = State::Running(kind);
        self.kind = Some(kind);
        self.task = Some(Task::spawn(spec, wake));
    }

    /// Start a task directly. The seam the tests use to put a **fake cargo** in
    /// place of the real one, and the seam a future `adb install` or
    /// `xcodebuild` step reuses without this module growing a branch per tool.
    pub fn start_spec(&mut self, spec: Spec, kind: Kind, wake: impl Fn() + Send + 'static) {
        self.output.clear();
        self.diagnostics.clear();
        self.binary = None;
        self.run_after_build = kind == Kind::BuildAndRun;
        self.output.push(format!("$ {}", spec.command_line()));
        self.root = Some(spec.dir.clone());
        self.state = State::Running(kind);
        self.kind = Some(kind);
        self.task = Some(Task::spawn(spec, wake));
    }

    /// The command line of the task that last started, for the Tasks panel.
    ///
    /// Read back out of the output, whose first line `start` writes as
    /// `$ <command>` — rather than kept a second time in a field that could
    /// come to disagree with what was actually run.
    #[must_use]
    pub fn last_command(&self) -> Option<String> {
        self.output
            .iter()
            .rev()
            .find_map(|line| line.strip_prefix("$ ").map(str::to_owned))
    }

    /// Ask the running operation to stop.
    pub fn cancel(&self) {
        if let Some(task) = &self.task {
            task.cancel();
        }
    }

    /// Take everything the task has produced. Call once a frame.
    ///
    /// Returns whether anything changed, so the frame hook can skip writing
    /// signals — and therefore skip a rebuild — on the overwhelming majority of
    /// frames during a long build, where nothing has arrived.
    pub fn poll(&mut self, wake: impl Fn() + Send + 'static) -> bool {
        let Some(task) = &self.task else {
            return false;
        };

        let events = task.poll();
        if events.is_empty() {
            return false;
        }

        let mut finished = None;
        for event in events {
            match event {
                Event::Line(stream, line) => {
                    // Only stdout can carry cargo's JSON, and **only some of it
                    // does**. Cargo puts human progress on stderr, and a
                    // running application puts whatever it likes on both.
                    //
                    // So the rule is: a line that reads as a cargo record is
                    // handled as one; **everything else goes to the panel**.
                    // The first version dropped every unparsed stdout line,
                    // which silently swallowed the entire output of the binary
                    // Build-and-Run had just started — the one thing the person
                    // pressing that button is waiting to see.
                    let record = match stream {
                        Stream::Out => cargo::parse_line(&line),
                        Stream::Err => None,
                    };
                    match record {
                        Some(Message::Diagnostic(d)) => {
                            self.output.push(format!(
                                "{}: {} [{}]",
                                d.location(),
                                d.message,
                                d.code
                            ));
                            self.diagnostics.push(d);
                        }
                        Some(Message::Artifact {
                            executable: Some(path),
                        }) => self.binary = Some(path),
                        // `build-finished` is cargo's verdict, and the task's
                        // exit code says the same thing; the task's is the one
                        // used, because it is also there when cargo dies
                        // without printing it. An artefact with no executable
                        // is a library, and there is nothing to say about it.
                        Some(_) => {}
                        None => self.output.push(line),
                    }
                }
                Event::Finished(outcome) => finished = Some(outcome),
            }
        }

        if let Some(outcome) = finished {
            self.finish(outcome, wake);
        }
        true
    }

    fn finish(&mut self, outcome: Outcome, wake: impl Fn() + Send + 'static) {
        let kind = self.kind.unwrap_or(Kind::Build);
        self.task = None;
        self.last_duration = Some(outcome.duration);

        let warnings = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == crate::state::Severity::Warning)
            .count();

        self.output.push(format!(
            "{} {} in {:.1}s",
            kind.label(),
            outcome.status.describe(),
            outcome.duration.as_secs_f32()
        ));

        if !outcome.succeeded() {
            self.run_after_build = false;
            self.state = State::Failed {
                kind,
                status: outcome.status,
            };
            return;
        }

        // The build half of a Build-and-Run succeeded: start the binary.
        //
        // Only when there *is* one. A library project builds successfully and
        // produces nothing to run, and inventing a failure for that would be
        // wrong — the build did succeed.
        if self.run_after_build && kind == Kind::BuildAndRun {
            self.run_after_build = false;
            if let Some(binary) = self.binary.clone() {
                let dir = self.root.clone().unwrap_or_else(|| PathBuf::from("."));
                let spec = Spec::new(Kind::Run.label(), binary.to_string_lossy(), dir);
                self.output.push(format!("$ {}", spec.command_line()));
                self.state = State::Running(Kind::Run);
                self.kind = Some(Kind::Run);
                self.task = Some(Task::spawn(spec, wake));
                return;
            }
            self.output
                .push("nothing to run: the build produced no binary".into());
        }

        self.state = State::Succeeded { kind, warnings };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Severity;

    /// A **fake cargo**: a shell script that prints exactly what cargo prints,
    /// then exits with a chosen code.
    ///
    /// The alternative — running the real cargo — would make every test below
    /// need a project, a network and twenty seconds, and would test cargo
    /// rather than this file. What is under test here is the handling of
    /// cargo's *output*, and that output is a known format, so a script that
    /// emits it is the honest double. The format itself is pinned separately by
    /// `cargo.rs`'s tests against records copied from a real run.
    fn fake_cargo(script: &str) -> Spec {
        // `sh` by name on Windows: see `task.rs`'s `sh` for why.
        let program = if cfg!(windows) { "sh" } else { "/bin/sh" };
        Spec::new("Build", program, std::env::temp_dir())
            .arg("-c")
            .arg(script)
    }

    /// Shell for `printf '%s\n' '<line>'` — **not** `echo`.
    ///
    /// `echo` is where a Windows path went to die. A cargo artefact line
    /// carries `"executable":"C:\\Users\\..."`, JSON-escaped as cargo
    /// writes it, and Git for Windows' `sh` runs `echo` in the XSI sense: it
    /// expands `\\` back to a single `\`, so what reached the parser was
    /// `C:\Users\...`, which is not valid JSON. The line was dropped, no
    /// artefact was seen, and `builds.binary` stayed `None` — twice, because
    /// escaping the path correctly does not help when the shell then unescapes
    /// it. `printf` does not touch its `%s` arguments.
    fn emit(line: &str) -> String {
        format!("printf '%s\\n' '{line}'")
    }

    const ERROR_JSON: &str = r#"{"reason":"compiler-message","message":{"level":"error","message":"cannot find value `x` in this scope","code":{"code":"E0425"},"children":[],"spans":[{"file_name":"src/main.rs","line_start":7,"column_start":13,"is_primary":true}]}}"#;
    const WARN_JSON: &str = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `y`","code":{"code":"unused_variables"},"children":[],"spans":[{"file_name":"src/main.rs","line_start":3,"column_start":9,"is_primary":true}]}}"#;

    ///
    /// **The path is escaped, because a Windows one is full of `\`.** JSON
    /// reads `\U` in `C:\Users\...` as an escape, so the line cargo would
    /// have written came out malformed, the artifact was never parsed, and
    /// `builds.binary` stayed `None` while the test said only that it
    /// expected `Some(app.bat)`. Real cargo escapes them; so does this.
    fn artifact_json(path: &str) -> String {
        let path = path.replace('\\', "\\\\");
        format!(
            r#"{{"reason":"compiler-artifact","target":{{"kind":["bin"],"name":"app"}},"executable":"{path}","fresh":false}}"#
        )
    }

    /// Drive a `Builds` to completion the way the frame hook does.
    fn drive(builds: &mut Builds) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while builds.busy() && std::time::Instant::now() < deadline {
            builds.poll(|| {});
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!builds.busy(), "the build never finished");
    }

    #[test]
    fn a_failing_build_fills_the_problems_panel_and_stays_failed() {
        let mut builds = Builds::new();
        builds.start_spec(
            fake_cargo(&format!(
                "echo '   Compiling app v0.1.0' 1>&2; {}; {}; exit 101",
                emit(ERROR_JSON),
                emit(WARN_JSON)
            )),
            Kind::Build,
            || {},
        );
        drive(&mut builds);

        assert_eq!(builds.diagnostics.len(), 2);
        assert_eq!(builds.diagnostics[0].severity, Severity::Error);
        assert_eq!(builds.diagnostics[0].file, "src/main.rs");
        assert_eq!(builds.diagnostics[0].line, 7);

        match &builds.state {
            State::Failed { kind, status } => {
                assert_eq!(*kind, Kind::Build);
                assert_eq!(*status, Status::Failed { code: Some(101) });
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        // Human progress from stderr reaches the panel; the JSON does not.
        assert!(builds.output.iter().any(|l| l.contains("Compiling app")));
        assert!(
            !builds.output.iter().any(|l| l.starts_with('{')),
            "raw JSON leaked into the Output panel: {:?}",
            builds.output
        );
        assert!(builds.output.iter().any(|l| l.contains("E0425")));
    }

    #[test]
    fn a_successful_build_counts_its_warnings() {
        let mut builds = Builds::new();
        builds.start_spec(
            fake_cargo(&format!("{}; exit 0", emit(WARN_JSON))),
            Kind::Build,
            || {},
        );
        drive(&mut builds);
        assert_eq!(
            builds.state,
            State::Succeeded {
                kind: Kind::Build,
                warnings: 1
            }
        );
        assert!(builds.last_duration.is_some());
    }

    /// The one that has to be right, because getting it wrong means the button
    /// says "Build failed" when the user pressed Cancel.
    #[test]
    fn a_cancelled_build_is_not_a_failed_build() {
        let mut builds = Builds::new();
        builds.start_spec(fake_cargo("sleep 30"), Kind::Build, || {});
        std::thread::sleep(Duration::from_millis(100));
        builds.cancel();
        drive(&mut builds);

        match &builds.state {
            State::Failed { status, .. } => assert_eq!(*status, Status::Cancelled),
            other => panic!("expected a cancelled outcome, got {other:?}"),
        }
        assert!(builds.output.iter().any(|l| l.contains("cancelled")));
    }

    /// Build-and-Run is two processes with one button. The handoff is the part
    /// that can silently not happen.
    #[test]
    fn build_and_run_starts_the_binary_the_build_produced() {
        let s = std::env::temp_dir().join("vieww-builds-run");
        std::fs::create_dir_all(&s).unwrap();

        // **What stands in for "the binary the build produced".**
        //
        // On Unix, a shell script with the executable bit — what it has always
        // been. On Windows neither half of that works: `CreateProcess` cannot
        // run a `.bat` at all (`std::process::Command` does not shell out for
        // one), and there is no executable bit to set. So the stand-in there
        // is a real program that every Windows has, prints something and exits
        // 0. The claim under test is the *handoff* — that the path cargo
        // reported is the one that gets run — and that is the same claim
        // whichever program sits at the end of it.
        let binary = if cfg!(windows) {
            PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()))
                .join("System32")
                .join("whoami.exe")
        } else {
            let path = s.join("app");
            std::fs::write(&path, "#!/bin/sh\necho the app is running\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        };

        // **The artefact line goes through a file, not through the shell.**
        // It carries a Windows path — `"executable":"C:\\Users\\..."` — and
        // every attempt to get those backslashes through `sh -c` intact failed
        // in a different way: `echo` expanded them (XSI), and escaping them
        // first only moved the problem. Cargo writes this line to a pipe, not
        // to a shell, so the fake one reads it from a file and the quoting
        // question disappears.
        let line = s.join("artifact.json");
        std::fs::write(&line, artifact_json(&binary.to_string_lossy())).unwrap();

        let mut builds = Builds::new();
        builds.start_spec(
            // Forward slashes: `sh` accepts them on Windows, and they need no
            // escaping anywhere.
            fake_cargo(&format!(
                "cat '{}'; exit 0",
                line.to_string_lossy().replace('\\', "/")
            )),
            Kind::BuildAndRun,
            || {},
        );

        // The build finishes and the run starts inside the same poll, so
        // `drive` must not stop at the first `!busy` — it does not, because the
        // run is already in place by the time `poll` returns.
        drive(&mut builds);

        assert_eq!(builds.binary.as_deref(), Some(binary.as_path()));
        assert_eq!(
            builds.state,
            State::Succeeded {
                kind: Kind::Run,
                warnings: 0
            },
            "the reported state is the run's, not the build's"
        );
        // The stand-in program prints its own thing on each platform — the
        // script's line on Unix, the user's name on Windows — so what is
        // asserted is that *something* of the run's reached the panel.
        let ran = if cfg!(windows) {
            builds.output.iter().any(|l| !l.trim().is_empty())
        } else {
            builds.output.iter().any(|l| l == "the app is running")
        };
        assert!(
            ran,
            "the binary's own output belongs in the panel: {:?}",
            builds.output
        );

        std::fs::remove_dir_all(&s).ok();
    }

    #[test]
    fn a_failed_build_never_runs_anything() {
        let mut builds = Builds::new();
        builds.start_spec(
            fake_cargo(&format!("{}; exit 101", emit(&artifact_json("/bin/echo")))),
            Kind::BuildAndRun,
            || {},
        );
        drive(&mut builds);
        assert!(matches!(builds.state, State::Failed { .. }));
        assert!(
            !builds.output.iter().any(|l| l.contains("$ /bin/echo")),
            "a failed build must not start the binary"
        );
    }

    /// A library project builds successfully and has nothing to run. Inventing
    /// a failure for that would be wrong; saying nothing would look like the
    /// button did nothing.
    #[test]
    fn build_and_run_with_no_binary_succeeds_and_says_why() {
        let mut builds = Builds::new();
        builds.start_spec(fake_cargo("exit 0"), Kind::BuildAndRun, || {});
        drive(&mut builds);
        assert_eq!(
            builds.state,
            State::Succeeded {
                kind: Kind::BuildAndRun,
                warnings: 0
            }
        );
        assert!(builds.output.iter().any(|l| l.contains("nothing to run")));
    }

    #[test]
    fn a_missing_cargo_is_reported_rather_than_hanging() {
        let mut builds = Builds::new();
        builds.start_spec(
            Spec::new("Build", "definitely-not-cargo", std::env::temp_dir()),
            Kind::Build,
            || {},
        );
        drive(&mut builds);
        match &builds.state {
            State::Failed { status, .. } => {
                assert!(matches!(status, Status::NotStarted(_)), "{status:?}");
            }
            other => panic!("expected NotStarted, got {other:?}"),
        }
    }

    /// Diagnostics from the previous build must not survive into this one.
    #[test]
    fn a_new_build_clears_the_last_ones_problems() {
        let mut builds = Builds::new();
        builds.start_spec(
            fake_cargo(&format!("{}; exit 101", emit(ERROR_JSON))),
            Kind::Build,
            || {},
        );
        drive(&mut builds);
        assert_eq!(builds.diagnostics.len(), 1);

        builds.start_spec(fake_cargo("exit 0"), Kind::Build, || {});
        assert!(
            builds.diagnostics.is_empty(),
            "cleared at the start, not at the end — the panel must not show \
             last build's errors while this one runs"
        );
        drive(&mut builds);
        assert!(builds.diagnostics.is_empty());
    }

    #[test]
    fn poll_says_nothing_changed_when_nothing_changed() {
        let mut builds = Builds::new();
        assert!(!builds.poll(|| {}), "an idle Builds has no news");

        builds.start_spec(fake_cargo("sleep 0.4; echo done"), Kind::Build, || {});
        // Immediately after starting, the child has not printed anything.
        assert!(!builds.poll(|| {}), "nothing has arrived yet");
        drive(&mut builds);
        assert!(!builds.poll(|| {}), "and nothing arrives after the end");
    }

    // ---- the commands, and what stops them --------------------------------

    fn project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vieww-builds-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        crate::scaffold::create(
            &dir,
            "app",
            &crate::scaffold::Dependency::Path(PathBuf::from("/w/vieww")),
        )
        .unwrap();
        dir
    }

    /// A desktop build names the binary, so cargo builds the rlib it links
    /// against and stops.
    ///
    /// Without `--bin`, cargo builds every crate type the `[lib]` declares —
    /// and a scaffolded project declares `cdylib` for Android and `staticlib`
    /// for iOS as well as the `rlib`. Measured on a freshly scaffolded project,
    /// debug: an 813 MB `.a` and a 228 MB `.so`, built by a desktop build, for
    /// two platforms nobody asked for.
    #[test]
    fn a_desktop_build_names_the_binary_from_the_manifest() {
        let dir = project("bin-name");
        let spec = Builds::spec(&dir, Target::Desktop, Profile::Debug, Kind::Build);
        assert_eq!(
            spec.command_line(),
            "cargo build --bin app --message-format=json"
        );
    }

    /// The mobile targets keep the plain `build`: there the cdylib and the
    /// staticlib are the artefact.
    #[test]
    fn the_mobile_targets_still_build_every_crate_type() {
        let dir = project("mobile-types");
        for target in [Target::Android, Target::Ios] {
            let line = Builds::spec(&dir, target, Profile::Release, Kind::Build).command_line();
            assert!(
                !line.contains("--bin"),
                "{target:?} must not narrow to the binary: {line}"
            );
        }
    }

    #[test]
    fn the_command_is_cargo_build_with_json() {
        // `/w/app` has no manifest to read a name from, so `binary_name`
        // answers `None` and the fallback stands. A real project is covered by
        // `a_desktop_build_names_the_binary_from_the_manifest` below.
        let spec = Builds::spec(
            std::path::Path::new("/w/app"),
            Target::Desktop,
            Profile::Debug,
            Kind::Build,
        );
        assert_eq!(
            spec.command_line(),
            "cargo build --bin app --message-format=json"
        );

        let release = Builds::spec(
            std::path::Path::new("/w/app"),
            Target::Desktop,
            Profile::Release,
            Kind::Build,
        );
        assert_eq!(
            release.command_line(),
            "cargo build --bin app --release --message-format=json"
        );

        let android = Builds::spec(
            std::path::Path::new("/w/app"),
            Target::Android,
            Profile::Release,
            Kind::Build,
        );
        assert_eq!(
            android.command_line(),
            "cargo ndk -t arm64-v8a build --release --message-format=json"
        );
    }

    #[test]
    fn a_folder_that_is_not_a_vieww_project_is_refused() {
        let dir = std::env::temp_dir().join("vieww-builds-bare");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::remove_file(dir.join("Cargo.toml")).ok();

        let builds = Builds::new();
        let env = Env::bare(toolchains::Host::Linux);
        assert_eq!(
            builds.check(&dir, Target::Desktop, &env),
            Err(Refusal::NotAProject)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_toolchain_refuses_before_the_build_starts() {
        let dir = project("toolchain");
        let builds = Builds::new();

        // Nothing installed: even Desktop has no cargo.
        let bare = Env::bare(toolchains::Host::Linux);
        match builds.check(&dir, Target::Desktop, &bare) {
            Err(Refusal::Toolchain(why)) => assert!(why.contains("cargo"), "{why}"),
            other => panic!("expected a toolchain refusal, got {other:?}"),
        }

        // And the impossible one stays impossible.
        match builds.check(&dir, Target::Ios, &bare) {
            Err(Refusal::Toolchain(why)) => assert!(why.contains("macOS"), "{why}"),
            other => panic!("expected a toolchain refusal, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_second_build_is_refused_while_the_first_runs() {
        let dir = project("busy");
        let mut builds = Builds::new();
        builds.start_spec(fake_cargo("sleep 5"), Kind::Build, || {});

        let env = Env::bare(toolchains::Host::Linux);
        assert_eq!(
            builds.check(&dir, Target::Desktop, &env),
            Err(Refusal::Busy(Kind::Build))
        );

        builds.cancel();
        drive(&mut builds);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_ios_build_targets_ios_rather_than_this_machine() {
        // It used to fall into the desktop arm and produce a host binary under
        // an iOS label — which looks like success and is not.
        let spec = Builds::spec(
            std::path::Path::new("/p"),
            Target::Ios,
            Profile::Release,
            Kind::Build,
        );
        assert!(
            spec.args
                .windows(2)
                .any(|w| w == ["--target", "aarch64-apple-ios"]),
            "{:?}",
            spec.args
        );
    }

    #[test]
    fn a_desktop_build_still_names_no_target() {
        let spec = Builds::spec(
            std::path::Path::new("/p"),
            Target::Desktop,
            Profile::Debug,
            Kind::Build,
        );
        assert!(
            !spec.args.iter().any(|a| a == "--target"),
            "{:?}",
            spec.args
        );
    }
}
