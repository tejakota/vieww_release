//! N4: what is installed, what is missing, and what installs it.
//!
//! Plan 2 §6.2 states the honest framing this module is built on: **the studio
//! orchestrates toolchains it does not own.** Desktop is nearly free. Android
//! needs an SDK, an NDK, a JDK and four Rust targets. iOS needs macOS, Xcode
//! and a signing identity, and a release IPA cannot be produced on Linux at
//! all — not a limitation to work around, a rule Apple enforces.
//!
//! So the order is: **detect, refuse clearly, then orchestrate.** Never start a
//! build that will fail twenty minutes in on a missing NDK.
//!
//! # Why this is a pure function of an [`Env`]
//!
//! Detection reads environment variables, walks `PATH`, and looks for files.
//! Written against the real process environment it would be testable only on a
//! machine that has an NDK — which is to say, not in CI and not on the machine
//! of whoever is reading a failure. [`detect`] therefore takes an [`Env`] value:
//! the studio passes [`Env::host`], and a test passes one pointing at a
//! directory it just made. The logic under test is the same logic either way.
//!
//! # What "found" means, and what it deliberately does not
//!
//! Found means *present at a plausible path*. It does **not** mean "works", and
//! this module never runs what it finds to check — probing by execution costs a
//! process launch per requirement on every open of the view, and a `cargo-ndk`
//! that is present but broken fails at build time with its own message, which
//! is better than this module's guess at one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Which operating system the studio is running on.
///
/// Its own enum rather than `cfg!(target_os)` at each site, because the whole
/// point is that a test can ask what a Linux host would report while running on
/// any host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Host {
    /// What this build is running on.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Other
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Other => "this platform",
        }
    }

    /// The file name an executable takes here.
    fn executable(self, stem: &str) -> Vec<String> {
        match self {
            Self::Windows => vec![
                format!("{stem}.exe"),
                format!("{stem}.bat"),
                format!("{stem}.cmd"),
            ],
            _ => vec![stem.to_string()],
        }
    }
}

/// The pieces of the outside world detection is allowed to look at.
///
/// Everything is data. There is no call into the real environment below
/// [`Env::host`], which is what makes every case in this file reachable from a
/// test.
#[derive(Debug, Clone)]
pub struct Env {
    pub host: Host,
    pub vars: BTreeMap<String, String>,
    /// The directories `PATH` names, already split.
    pub path: Vec<PathBuf>,
    /// Rust target triples that are installed — `rustup target list --installed`.
    /// Read through [`Env::rust_targets`], never directly.
    pub rust_targets: Probed<Vec<String>>,
    /// The name of a code-signing identity in the login keychain, if there is
    /// one — `security find-identity -v -p codesigning`. Read through
    /// [`Env::signing_identity`], never directly.
    ///
    /// macOS only, and `None` everywhere else. Data here for the same reason
    /// `rust_targets` is: the checklist has to be renderable from a test on a
    /// machine with no keychain at all.
    pub signing_identity: Probed<Option<String>>,
}

/// A fact about the machine that is either supplied or asked for on first read.
///
/// # Why the two subprocess answers are not resolved eagerly
///
/// Both of the questions this wraps are answered by **spawning a process**, and
/// `Env::host` is on the studio's startup path — it is called from
/// `Studio::new`, before the first frame. So every launch paid for a `rustup`
/// spawn and (on macOS) a keychain query before anything appeared on screen,
/// for two facts nothing needs until somebody opens the Toolchain view or
/// starts an export.
///
/// The cost is not a constant, either, and the worst case is the common one: a
/// studio opened on a freshly cloned project whose `rust-toolchain.toml` pins a
/// toolchain the user has not installed makes `rustup target list --installed`
/// try to *fetch* that toolchain. Measured in this workspace, which pins a
/// toolchain that is not present: **624–741 ms**, against **25–27 ms** for the
/// same command where the pin resolves. On a slow network there is no upper
/// bound at all, and all of it landed before the window had drawn.
///
/// `Given` is what tests and the builder methods produce, so every branch of
/// the checklist stays reachable without a `rustup` on the machine running the
/// test — which is the property the module docs are about. `Host` is what
/// [`Env::host`] produces, and it resolves through the `OnceLock`s below, so
/// the first reader pays and every later one does not.
///
/// **Known remaining sharp edge.** The first read still blocks whichever frame
/// asks. That is now a frame where the user has opened the Toolchain view and
/// is looking at a list that is visibly populating, rather than a frame before
/// the application exists — a large improvement, but not a substitute for
/// running these off the UI thread, which is the proper fix and wants a
/// threading story this module does not currently have.
#[derive(Debug, Clone)]
pub enum Probed<T> {
    /// Supplied by the caller — tests, and the builder methods.
    Given(T),
    /// Ask the real machine, the first time somebody reads it.
    Host,
}

/// The triples `rustup target list --installed` names, cached for the process.
///
/// # Why this shells out when the rest of the module refuses to
///
/// Everything below [`Env::host`] is data so that every branch is reachable
/// from a test. `Env::host` is the one seam that is allowed to touch the real
/// machine, and this is the only question about the machine that no environment
/// variable and no `PATH` walk can answer: a target is a directory inside the
/// active toolchain, and the name of the active toolchain is `rustup`'s to
/// decide. Reading the filesystem directly would mean re-implementing toolchain
/// resolution — overrides, `rust-toolchain.toml`, the default — and getting it
/// subtly wrong on the machines that matter most.
///
/// It fails to `Vec::new()`, which is the honest answer on a machine with no
/// `rustup`: the studio reports the cross-compilation targets as missing rather
/// than claiming a target it cannot prove is there.
///
/// Cached in a `OnceLock` because `Env::host` is called on startup **and** on
/// every "Refresh toolchain", and a process spawn per refresh is a visible
/// stall for an answer that cannot change without a `rustup target add` — which
/// is itself a restart-the-check action.
/// How long the `rustup` probe is allowed to take before it is abandoned.
///
/// # Why this has a deadline at all
///
/// `rustup target list --installed` is not a local question when the studio is
/// opened on a project. It resolves the project's toolchain first, and if
/// `rust-toolchain.toml` pins one the user has not installed, **rustup tries to
/// download it** — which is the common case for a freshly cloned repository,
/// exactly the moment somebody opens it in an editor for the first time.
///
/// Measured in this workspace, which pins a toolchain that is not present:
/// 624 ms and 741 ms. The same command where the pin resolves: 25 ms and
/// 27 ms. On a slow or captive network there is no upper bound at all, and
/// before this the whole of it landed inside `Studio::new`, before the first
/// frame — the studio appeared to hang on launch for a reason nothing on screen
/// could explain.
///
/// # Why 300 and not 150
///
/// 150 ms was set from a container where a healthy `rustup target list
/// --installed` took 25 ms. On a real development machine — a 2017 laptop with
/// the pinned toolchain actually installed — the same command takes **40–67 ms
/// warm, and over 150 ms on a cold page cache**: the first run after boot hit
/// the deadline exactly and reported no targets at all. Six times the healthy
/// case was the right multiple against the wrong healthy case.
///
/// A spurious timeout is not a disaster — nothing is cached, so the next ask
/// or a Re-scan click gets the real answer — but a checklist that says "no
/// cross-compilation targets" on a machine that has them is a support
/// conversation, and it fires on exactly the run where a first impression is
/// formed.
///
/// 300 ms still refuses the case this exists for, which is not a slow machine
/// but a *network*: rustup fetching an uninstalled pinned toolchain measured
/// 624–741 ms here and is unbounded on a bad connection.
///
/// **This constant is a stopgap and should stop existing.** It is only load
/// bearing because `detect()` runs inside `Studio::new`, on the first frame,
/// for a sidebar view that is not on screen at launch. Move that off the
/// first frame and nothing waits on this at all — the studio reaches its
/// first frame in about a millisecond with the probe out of the way.
const RUSTUP_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

fn rustup_installed_targets() -> Vec<String> {
    /// The answer, once the probe has actually produced one. A timeout is
    /// deliberately **not** stored here: a studio launched while rustup was
    /// busy fetching a toolchain would otherwise report "no targets installed"
    /// for the rest of the session, with Re-scan unable to fix it — trading a
    /// slow start for a permanently wrong checklist.
    static TARGETS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    /// The probe that is already running, if one is. One probe per process,
    /// not one per question.
    static IN_FLIGHT: std::sync::Mutex<Option<std::sync::mpsc::Receiver<Vec<String>>>> =
        std::sync::Mutex::new(None);

    if let Some(cached) = TARGETS.get() {
        return cached.clone();
    }

    // A poisoned lock here means a previous caller panicked mid-probe. The
    // value behind it is a channel receiver, which cannot be left inconsistent
    // by a panic, so recovering is correct and refusing to would turn a
    // detection question into a crash.
    let mut in_flight = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());

    // `detect` asks about several triples in one pass, and before this was
    // single-flight each of them started its own probe and waited its own
    // deadline — three targets turned a 150 ms bound into 450 ms, which is
    // what the measurement showed. The first ask starts the probe and is the
    // only one allowed to wait; every later ask takes the answer if it has
    // landed and otherwise reports not-yet-known immediately.
    let first_ask = in_flight.is_none();
    if first_ask {
        let (tx, rx) = std::sync::mpsc::channel();
        // Detached on purpose. If the probe outruns its deadline the thread is
        // left to finish and drop its result into a channel that is still
        // held here, so a later ask picks it up; it ends when rustup does.
        // Killing the child instead would mean holding the handle across the
        // timeout, and a half-killed rustup is worse than one that finishes
        // talking to itself.
        std::thread::spawn(move || {
            let targets = match std::process::Command::new("rustup")
                .args(["target", "list", "--installed"])
                .output()
            {
                Ok(output) if output.status.success() => {
                    parse_installed_targets(&String::from_utf8_lossy(&output.stdout))
                }
                // No rustup, or it refused: the honest answer is "nothing
                // proven installed", and that one *is* worth caching.
                _ => Vec::new(),
            };
            let _ = tx.send(targets);
        });
        *in_flight = Some(rx);
    }

    let rx = in_flight.as_ref().expect("just set above");
    let landed = if first_ask {
        rx.recv_timeout(RUSTUP_PROBE_TIMEOUT).ok()
    } else {
        rx.try_recv().ok()
    };

    match landed {
        Some(targets) => TARGETS.get_or_init(|| targets).clone(),
        None => Vec::new(),
    }
}

/// Split `rustup target list --installed` output into triples.
///
/// Separate from the spawn so the parse is testable without a `rustup` on the
/// machine running the test. `rustup` prints one triple per line and nothing
/// else, but it prints an explanatory line to stdout when no toolchain is
/// active, so a line with a space in it is not a triple and is dropped.
fn parse_installed_targets(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.contains(' '))
        .map(ToOwned::to_owned)
        .collect()
}

/// The first code-signing identity `security` reports, if any.
///
/// # Why iOS needs this and the checklist did not ask
///
/// The module's own first paragraph says iOS needs "Xcode and a signing
/// identity". The checklist asked for `xcodebuild`, `xcrun` and the Rust
/// target, and never asked for the identity — so a Mac with Xcode installed
/// and no Apple ID added to it reported iOS as **ready**, started a build, and
/// failed inside `xcodebuild` with a signing error that names a certificate,
/// not an action. That is the exact failure the refusal-before-start design
/// exists to prevent.
///
/// `security` is macOS-only and this returns `None` on every other host without
/// spawning anything, so a Linux or Windows studio pays nothing for it.
///
/// Cached, like the target list, because "Refresh toolchain" should not spawn a
/// keychain query per click.
fn signing_identity() -> Option<String> {
    if Host::current() != Host::MacOs {
        return None;
    }
    static IDENTITY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let output = std::process::Command::new("security")
                .args(["find-identity", "-v", "-p", "codesigning"])
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| parse_signing_identity(&String::from_utf8_lossy(&output.stdout)))?
        })
        .clone()
}

/// Pull the quoted identity name out of `security find-identity` output.
///
/// The output looks like:
///
/// ```text
///   1) 0123ABCD… "Apple Development: Someone (TEAMID)"
///      1 valid identities found
/// ```
///
/// Split out so the parse is testable without a keychain. Only Development and
/// Distribution certificates count: a self-signed certificate in the keychain
/// is not something an iOS build can use, and reporting it as an identity puts
/// the failure back inside `xcodebuild`.
fn parse_signing_identity(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .filter(|line| line.contains("Development") || line.contains("Distribution"))
        .find_map(|line| {
            let start = line.find('"')?;
            let end = line.rfind('"')?;
            (end > start).then(|| line[start + 1..end].to_owned())
        })
}

impl Env {
    /// The real environment this studio is running in.
    ///
    /// Cheap, and deliberately so: this is called from `Studio::new`, before
    /// the first frame. It reads the process's own environment and nothing
    /// else. The two answers that cost a subprocess are left as
    /// [`Probed::Host`] and resolved by [`Env::rust_targets`] and
    /// [`Env::signing_identity`] when something actually asks — see `Probed`.
    #[must_use]
    pub fn host() -> Self {
        Self {
            host: Host::current(),
            vars: std::env::vars().collect(),
            path: std::env::var_os("PATH")
                .map(|value| std::env::split_paths(&value).collect())
                .unwrap_or_default(),
            rust_targets: Probed::Host,
            signing_identity: Probed::Host,
        }
    }

    /// The installed Rust target triples.
    ///
    /// An empty list means "not installed", which is what the checklist
    /// reports as missing — the honest answer on a machine with no `rustup`.
    #[must_use]
    pub fn rust_targets(&self) -> Vec<String> {
        match &self.rust_targets {
            Probed::Given(targets) => targets.clone(),
            Probed::Host => rustup_installed_targets(),
        }
    }

    /// The code-signing identity an iOS build would use, if there is one.
    #[must_use]
    pub fn signing_identity(&self) -> Option<String> {
        match &self.signing_identity {
            Probed::Given(identity) => identity.clone(),
            Probed::Host => signing_identity(),
        }
    }

    /// An empty environment on `host`, for tests and for the "nothing is
    /// installed" case the UI has to render well.
    #[must_use]
    pub fn bare(host: Host) -> Self {
        Self {
            host,
            vars: BTreeMap::new(),
            path: Vec::new(),
            rust_targets: Probed::Given(Vec::new()),
            signing_identity: Probed::Given(None),
        }
    }

    #[must_use]
    pub fn var(mut self, key: &str, value: impl AsRef<Path>) -> Self {
        self.vars.insert(
            key.to_string(),
            value.as_ref().to_string_lossy().into_owned(),
        );
        self
    }

    #[must_use]
    pub fn path_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.path.push(dir.into());
        self
    }

    /// Add an installed target, pinning the list to what the caller supplies.
    #[must_use]
    pub fn rust_target(mut self, triple: impl Into<String>) -> Self {
        let mut targets = match self.rust_targets {
            Probed::Given(targets) => targets,
            // Naming a target makes this env's list the caller's, rather than
            // the caller's one entry silently appended to the real machine's.
            Probed::Host => Vec::new(),
        };
        targets.push(triple.into());
        self.rust_targets = Probed::Given(targets);
        self
    }

    /// Pin the code-signing identity, rather than asking the keychain.
    #[must_use]
    pub fn with_signing_identity(mut self, name: impl Into<String>) -> Self {
        self.signing_identity = Probed::Given(Some(name.into()));
        self
    }

    /// The first directory on `PATH` holding an executable called `stem`.
    ///
    /// Not `which`: a subprocess per requirement, and one that does not exist
    /// on Windows. The executable bit is checked on Unix, because a *directory*
    /// called `cargo` on `PATH` would otherwise read as the tool being present.
    #[must_use]
    pub fn find_on_path(&self, stem: &str) -> Option<PathBuf> {
        for dir in &self.path {
            for name in self.host.executable(stem) {
                let candidate = dir.join(&name);
                if is_executable_file(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    fn var_path(&self, key: &str) -> Option<PathBuf> {
        let value = self.vars.get(key)?;
        if value.is_empty() {
            return None;
        }
        let path = PathBuf::from(value);
        path.is_dir().then_some(path)
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// What can be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Desktop,
    /// A Windows `.exe`, built from any host.
    ///
    /// Separate from [`Desktop`](Self::Desktop), which builds *for the machine
    /// it is running on*: from Linux or macOS this is a cross-compile with its
    /// own Rust target and its own linker, and neither is present by default.
    /// Folding it into `Desktop` would mean one row in the export sheet whose
    /// requirements changed depending on which host was reading it.
    Windows,
    Android,
    Ios,
}

impl Target {
    pub const ALL: [Self; 4] = [Self::Desktop, Self::Windows, Self::Android, Self::Ios];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Desktop => "Desktop",
            Self::Windows => "Windows",
            Self::Android => "Android",
            Self::Ios => "iOS",
        }
    }

    #[must_use]
    pub const fn produces(self) -> &'static str {
        match self {
            Self::Desktop => "a native binary for this machine",
            Self::Windows => "a Windows .exe",
            Self::Android => "a .apk",
            Self::Ios => "a .app for the simulator, or a signed .ipa",
        }
    }
}

/// One thing a target needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// What to call it in the checklist — "Android NDK", "cargo-ndk".
    pub name: &'static str,
    /// Where it was found, if it was.
    pub found: Option<PathBuf>,
    /// What to run to get it. Shown next to the missing item, because a
    /// checklist that says "missing" without saying "and here is the command"
    /// is a bug report the user has to research.
    pub install: &'static str,
    /// Why the target needs it, for the row's second line.
    pub why: &'static str,
}

impl Requirement {
    #[must_use]
    pub const fn satisfied(&self) -> bool {
        self.found.is_some()
    }
}

/// Everything one target needs, and whether it has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub target: Target,
    /// Set when the target is impossible on this host **whatever is installed**
    /// — which today means exactly one thing: Apple does not permit building
    /// for iOS anywhere but macOS.
    ///
    /// Plan 2's open question 3 asks whether to hide the target or show it
    /// disabled with the reason. This is the answer: a missing button is a bug
    /// report, so the row stays and says why.
    pub impossible: Option<&'static str>,
    pub requirements: Vec<Requirement>,
}

impl Report {
    /// Whether a build for this target can be started at all.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.impossible.is_none() && self.requirements.iter().all(Requirement::satisfied)
    }

    /// What is still needed, in checklist order.
    #[must_use]
    pub fn missing(&self) -> Vec<&Requirement> {
        self.requirements
            .iter()
            .filter(|r| !r.satisfied())
            .collect()
    }

    /// The sentence shown when a build is refused, naming **everything**
    /// missing rather than the first thing.
    ///
    /// One at a time is the shape that makes a user install four things across
    /// four failed attempts; the whole list is one trip.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        if let Some(reason) = self.impossible {
            return Some(format!(
                "{} cannot be built here: {reason}",
                self.target.title()
            ));
        }
        let missing = self.missing();
        if missing.is_empty() {
            return None;
        }
        let names: Vec<&str> = missing.iter().map(|r| r.name).collect();
        Some(format!(
            "{} needs {} — install {} first",
            self.target.title(),
            names.join(", "),
            if names.len() == 1 { "it" } else { "them" }
        ))
    }
}

/// Look at `env` and report on every target.
#[must_use]
pub fn detect(env: &Env) -> Vec<Report> {
    Target::ALL
        .iter()
        .map(|&target| report(env, target))
        .collect()
}

/// Look at `env` and report on one target.
#[must_use]
pub fn report(env: &Env, target: Target) -> Report {
    match target {
        Target::Desktop => Report {
            target,
            impossible: None,
            requirements: vec![cargo(env)],
        },
        Target::Windows => Report {
            target,
            impossible: None,
            requirements: windows(env),
        },
        Target::Android => Report {
            target,
            impossible: None,
            requirements: android(env),
        },
        Target::Ios => Report {
            target,
            impossible: (env.host != Host::MacOs).then_some(
                "Apple's toolchain runs only on macOS, so no iOS build is possible on this machine",
            ),
            requirements: ios(env),
        },
    }
}

fn cargo(env: &Env) -> Requirement {
    Requirement {
        name: "cargo",
        found: env.find_on_path("cargo"),
        install: "https://rustup.rs",
        why: "builds the project",
    }
}

/// The Android chain: SDK, NDK, a JDK, `cargo-ndk`, and the Rust targets.
///
/// The **NDK** is looked for in three places because three are in use:
/// `ANDROID_NDK_HOME` (what `cargo-ndk` reads), `ANDROID_NDK_ROOT` (the name
/// Google's own tooling moved to), and `<SDK>/ndk/<version>` (where Android
/// Studio actually puts it, and where a user who installed through the IDE has
/// one without either variable set). Checking only the variables reports a
/// missing NDK to the majority of people who have one.
fn android(env: &Env) -> Vec<Requirement> {
    let sdk = env
        .var_path("ANDROID_HOME")
        .or_else(|| env.var_path("ANDROID_SDK_ROOT"));

    let ndk = env
        .var_path("ANDROID_NDK_HOME")
        .or_else(|| env.var_path("ANDROID_NDK_ROOT"))
        .or_else(|| sdk.as_deref().and_then(newest_ndk));

    let jdk = env
        .var_path("JAVA_HOME")
        .or_else(|| env.find_on_path("javac"))
        .or_else(|| env.find_on_path("java"));

    vec![
        cargo(env),
        Requirement {
            name: "Android SDK",
            found: sdk,
            install: "Android Studio, or `sdkmanager --install \"platform-tools\"`; then set ANDROID_HOME",
            why: "provides adb, and the platform the apk is built against",
        },
        Requirement {
            name: "Android NDK",
            found: ndk,
            install: "`sdkmanager --install \"ndk;27.0.12077973\"`; then set ANDROID_NDK_HOME",
            why: "cross-compiles Rust for the phone's architecture",
        },
        Requirement {
            name: "JDK",
            found: jdk,
            install: "`apt install default-jdk`, `brew install openjdk`, \
                      `winget install Microsoft.OpenJDK.17`, or Android Studio's bundled JDK",
            why: "packages and signs the apk",
        },
        Requirement {
            name: "cargo-ndk",
            found: env.find_on_path("cargo-ndk"),
            install: "`cargo install cargo-ndk`",
            why: "points the NDK's linker at the Rust build",
        },
        // **A scaffolded project has no `gradlew`.** The wrapper needs a binary
        // JAR, which `scaffold` does not write, so the second half of an
        // Android export runs the system `gradle` until somebody runs
        // `gradle wrapper` once. Without this row the checklist reported
        // "ready" and the build then failed inside a step whose error is a
        // shell's "command not found".
        Requirement {
            name: "Gradle",
            found: env.find_on_path("gradle"),
            install: "`brew install gradle`, `apt install gradle`, \
                      `winget install Gradle.Gradle`, or Android Studio's bundled copy \
                      — or run `gradle wrapper` once in the project's `android/` directory",
            why: "assembles the apk from the compiled Rust",
        },
        rust_target(env, "aarch64-linux-android", "every phone shipped since 2017"),
    ]
}

/// The iOS chain. Only reached in full on macOS; on any other host the report
/// is already `impossible` and these rows are the *reason it would still fail*,
/// which is worth showing rather than hiding.
fn ios(env: &Env) -> Vec<Requirement> {
    vec![
        cargo(env),
        Requirement {
            name: "Xcode",
            found: env.find_on_path("xcodebuild"),
            install: "the Mac App Store, then `xcode-select --install`",
            why: "builds, signs and packages the app",
        },
        Requirement {
            name: "simctl",
            found: env.find_on_path("xcrun"),
            install: "included with Xcode",
            why: "installs and launches on the simulator",
        },
        rust_target(env, "aarch64-apple-ios", "every device since the iPhone 5s"),
        Requirement {
            name: "Signing identity",
            found: env.signing_identity().map(PathBuf::from),
            // `\u{25b8}` — a small right-pointing triangle — is not in the
            // bundled font, so this menu path rendered as a row of empty boxes
            // in the one place a user reads it. The same tofu the toolchain
            // list's tick had, found the same way: by looking at the picture.
            install: "Xcode > Settings > Accounts > your Apple ID > Manage Certificates > +",
            why: "every iOS binary must be signed, including one for the simulator",
        },
    ]
}

/// The Windows chain: the GNU target, and a linker that can produce PE.
///
/// # Why `-gnu` and not `-msvc`
///
/// `x86_64-pc-windows-msvc` links with Microsoft's `link.exe`, which is not
/// redistributable and does not exist on Linux — so choosing it would make the
/// row permanently impossible on the machine most likely to want a cross-build.
/// `x86_64-pc-windows-gnu` links with MinGW's `ld`, which `apt`, `dnf` and
/// `brew` all package, and produces an ordinary `.exe` that runs on stock
/// Windows with no runtime to install.
///
/// On Windows itself neither is needed: the host toolchain already links PE, so
/// this asks for `cargo` and nothing else and the plan skips `--target`
/// entirely. That is the point of checking the host here rather than in the
/// plan — a checklist that demanded MinGW on Windows would be asking somebody
/// to install a cross-compiler to build for the machine they are sitting at.
fn windows(env: &Env) -> Vec<Requirement> {
    if env.host == Host::Windows {
        return vec![cargo(env)];
    }
    vec![
        cargo(env),
        rust_target(
            env,
            "x86_64-pc-windows-gnu",
            "the 64-bit Windows target every desktop since Windows 7 runs",
        ),
        Requirement {
            name: "MinGW-w64 linker",
            found: env
                .find_on_path("x86_64-w64-mingw32-gcc")
                .or_else(|| env.find_on_path("x86_64-w64-mingw32-ld")),
            install: "`apt install mingw-w64`, `dnf install mingw64-gcc`, \
                      `brew install mingw-w64`, or `choco install mingw`",
            why: "links the Windows executable; rustc cannot produce a PE binary without it",
        },
    ]
}

fn rust_target(env: &Env, triple: &'static str, why: &'static str) -> Requirement {
    Requirement {
        name: triple,
        // A target is not a file on `PATH`; the caller supplies the installed
        // list. An empty list means "not asked yet", which is reported as
        // missing — the honest answer, and one `rustup target add` fixes it
        // whether or not it was already there.
        found: env
            .rust_targets()
            .iter()
            .any(|t| t == triple)
            .then(|| PathBuf::from(triple)),
        install: match triple {
            "aarch64-linux-android" => "`rustup target add aarch64-linux-android`",
            "x86_64-pc-windows-gnu" => "`rustup target add x86_64-pc-windows-gnu`",
            _ => "`rustup target add aarch64-apple-ios`",
        },
        why,
    }
}

/// The highest-numbered directory under `<sdk>/ndk`.
///
/// Android Studio installs side-by-side NDK versions and leaves them all in
/// place. Sorted by the numeric parts of the name rather than as strings,
/// because `27.0.12077973` sorts *before* `9.0.0` alphabetically and picking
/// the oldest NDK is a compile failure the user cannot explain.
fn newest_ndk(sdk: &Path) -> Option<PathBuf> {
    let mut versions: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(sdk.join("ndk"))
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let parts = name
                .split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect();
            (parts, entry.path())
        })
        .collect();

    versions.sort();
    versions.pop().map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that cleans up after itself, so the tests below can
    /// look at real files — which is the point, since `is_executable_file` and
    /// `newest_ndk` are about the filesystem.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("vieww-toolchains-{name}"));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        /// A file with the executable bit set, like a real tool on `PATH`.
        fn exe(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"#!/bin/sh\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            path
        }

        /// A file **without** it.
        fn plain(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"not a program").unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn nothing_installed_is_reported_as_nothing_installed() {
        let reports = detect(&Env::bare(Host::Linux));
        // One per `Target::ALL` — Desktop, Windows, Android, iOS. Asserted
        // against the enum rather than a literal, so adding a target updates
        // this instead of breaking it.
        assert_eq!(reports.len(), Target::ALL.len());
        for report in &reports {
            assert!(!report.ready(), "{:?} claimed to be ready", report.target);
            assert!(report.refusal().is_some());
        }
    }

    #[test]
    // Windows has no executable bit — a file is a program there because of its
    // extension — so the middle assertion below is a claim about Unix, and the
    // test is one too rather than being weakened for both.
    #[cfg_attr(not(unix), ignore = "the executable bit is a Unix idea")]
    fn cargo_is_found_on_path_and_only_if_it_is_executable() {
        let s = Scratch::new("cargo");
        let bin = s.dir("bin");

        let env = Env::bare(Host::Linux).path_dir(&bin);
        assert_eq!(env.find_on_path("cargo"), None);

        // A *directory* named cargo is not cargo.
        s.dir("bin/cargo");
        assert_eq!(
            env.find_on_path("cargo"),
            None,
            "a directory is not a program"
        );

        std::fs::remove_dir(bin.join("cargo")).unwrap();
        s.plain("bin/cargo");
        assert_eq!(
            env.find_on_path("cargo"),
            None,
            "a file with no executable bit is not a program"
        );

        std::fs::remove_file(bin.join("cargo")).unwrap();
        let real = s.exe("bin/cargo");
        assert_eq!(env.find_on_path("cargo"), Some(real));
    }

    #[test]
    fn desktop_needs_only_cargo() {
        let s = Scratch::new("desktop");
        s.exe("bin/cargo");
        let env = Env::bare(Host::Linux).path_dir(s.dir("bin"));
        let report = report(&env, Target::Desktop);
        assert!(report.ready());
        assert_eq!(report.refusal(), None);
    }

    #[test]
    fn ios_is_impossible_off_a_mac_and_says_so() {
        let report = report(&Env::bare(Host::Linux), Target::Ios);
        assert!(!report.ready());
        let refusal = report.refusal().unwrap();
        assert!(refusal.contains("macOS"), "{refusal}");

        // And it stays impossible even with every tool somehow present, which
        // is the case a "just check the requirements" implementation gets wrong.
        let s = Scratch::new("ios-liar");
        for tool in ["cargo", "xcodebuild", "xcrun"] {
            s.exe(&format!("bin/{tool}"));
        }
        let env = Env::bare(Host::Linux)
            .path_dir(s.dir("bin"))
            .rust_target("aarch64-apple-ios")
            .with_signing_identity("Apple Development: Someone (TEAM1234)");
        let with_tools = super::report(&env, Target::Ios);
        assert!(
            with_tools.missing().is_empty(),
            "every requirement is present"
        );
        assert!(!with_tools.ready(), "and it is still impossible");
    }

    #[test]
    fn ios_on_a_mac_is_merely_a_matter_of_installing_things() {
        let s = Scratch::new("ios-mac");
        for tool in ["cargo", "xcodebuild", "xcrun"] {
            s.exe(&format!("bin/{tool}"));
        }
        let env = Env::bare(Host::MacOs)
            .path_dir(s.dir("bin"))
            .rust_target("aarch64-apple-ios")
            .with_signing_identity("Apple Development: Someone (TEAM1234)");
        let report = report(&env, Target::Ios);
        assert_eq!(report.impossible, None);
        assert!(report.ready(), "missing: {:?}", report.missing());
    }

    #[test]
    fn a_complete_android_setup_is_ready() {
        let s = Scratch::new("android-ok");
        // `gradle` as well: a scaffolded project has no `gradlew`, so the
        // second half of an Android export runs the system Gradle.
        for tool in ["cargo", "cargo-ndk", "gradle", "javac"] {
            s.exe(&format!("bin/{tool}"));
        }
        let sdk = s.dir("sdk");
        let ndk = s.dir("sdk/ndk/27.0.12077973");
        let env = Env::bare(Host::Linux)
            .path_dir(s.dir("bin"))
            .var("ANDROID_HOME", &sdk)
            .rust_target("aarch64-linux-android");

        let report = report(&env, Target::Android);
        assert!(report.ready(), "missing: {:?}", report.missing());
        let found_ndk = report
            .requirements
            .iter()
            .find(|r| r.name == "Android NDK")
            .unwrap();
        assert_eq!(found_ndk.found.as_deref(), Some(ndk.as_path()));
    }

    /// The case that makes the three-place NDK search worth having: installed
    /// through Android Studio, so it is on disk under the SDK and neither
    /// environment variable is set.
    #[test]
    fn the_ndk_is_found_under_the_sdk_with_no_variable_set() {
        let s = Scratch::new("android-ide");
        let sdk = s.dir("sdk");
        s.dir("sdk/ndk/26.1.10909125");
        let newest = s.dir("sdk/ndk/27.0.12077973");
        s.dir("sdk/ndk/9.0.0");

        let env = Env::bare(Host::Linux).var("ANDROID_HOME", &sdk);
        let report = report(&env, Target::Android);
        let ndk = report
            .requirements
            .iter()
            .find(|r| r.name == "Android NDK")
            .unwrap();

        // Sorted numerically: `9.0.0` is the *string*-largest and the
        // version-smallest, and picking it is a compile failure nobody can read.
        assert_eq!(ndk.found.as_deref(), Some(newest.as_path()));
    }

    #[test]
    fn an_explicit_ndk_variable_wins_over_the_sdk_copy() {
        let s = Scratch::new("android-explicit");
        let sdk = s.dir("sdk");
        s.dir("sdk/ndk/27.0.12077973");
        let chosen = s.dir("elsewhere/android-ndk-r26");

        let env = Env::bare(Host::Linux)
            .var("ANDROID_HOME", &sdk)
            .var("ANDROID_NDK_HOME", &chosen);
        let report = report(&env, Target::Android);
        let ndk = report
            .requirements
            .iter()
            .find(|r| r.name == "Android NDK")
            .unwrap();
        assert_eq!(ndk.found.as_deref(), Some(chosen.as_path()));
    }

    /// A variable pointing at nothing is not an installation. Trusting the
    /// string is how a build starts and dies on a missing linker.
    #[test]
    fn a_variable_pointing_at_a_missing_directory_is_not_found() {
        let env = Env::bare(Host::Linux)
            .var("ANDROID_HOME", "/definitely/not/here")
            .var("ANDROID_NDK_HOME", "");
        let report = report(&env, Target::Android);
        for name in ["Android SDK", "Android NDK"] {
            let requirement = report.requirements.iter().find(|r| r.name == name).unwrap();
            assert!(!requirement.satisfied(), "{name} was reported as found");
        }
    }

    /// The refusal names everything missing at once. One at a time is four
    /// failed builds.
    #[test]
    fn the_refusal_lists_everything_missing_not_the_first_thing() {
        let s = Scratch::new("android-partial");
        s.exe("bin/cargo");
        let env = Env::bare(Host::Linux).path_dir(s.dir("bin"));
        let report = report(&env, Target::Android);

        let refusal = report.refusal().unwrap();
        for expected in [
            "Android SDK",
            "Android NDK",
            "JDK",
            "cargo-ndk",
            "Gradle",
            "aarch64-linux-android",
        ] {
            assert!(
                refusal.contains(expected),
                "{expected} missing from: {refusal}"
            );
        }
    }

    #[test]
    fn every_missing_requirement_says_how_to_get_it() {
        for report in detect(&Env::bare(Host::Linux)) {
            for requirement in report.missing() {
                assert!(
                    !requirement.install.is_empty() && !requirement.why.is_empty(),
                    "{} has nothing to tell the user",
                    requirement.name
                );
            }
        }
    }

    #[test]
    fn a_rust_target_is_found_only_when_it_is_installed() {
        let env = Env::bare(Host::Linux).rust_target("x86_64-unknown-linux-gnu");
        let report = report(&env, Target::Android);
        let target = report
            .requirements
            .iter()
            .find(|r| r.name == "aarch64-linux-android")
            .unwrap();
        assert!(!target.satisfied());

        let env = env.rust_target("aarch64-linux-android");
        let installed = super::report(&env, Target::Android);
        let target = installed
            .requirements
            .iter()
            .find(|r| r.name == "aarch64-linux-android")
            .unwrap();
        assert!(target.satisfied());
    }

    #[test]
    fn windows_looks_for_the_extensions_windows_uses() {
        let s = Scratch::new("windows");
        s.exe("bin/cargo-ndk.bat");
        let env = Env::bare(Host::Windows).path_dir(s.dir("bin"));
        assert!(env.find_on_path("cargo-ndk").is_some());
        // And a Linux host does not accept the same file.
        let env = Env::bare(Host::Linux).path_dir(s.dir("bin"));
        assert_eq!(env.find_on_path("cargo-ndk"), None);
    }

    #[test]
    fn installed_targets_are_parsed_out_of_rustups_listing() {
        // The blocker this closes: `Env::host` used to leave `rust_targets`
        // empty, so Android and iOS were reported as "target not installed" on
        // a machine that had them — `rustup target add` said "already
        // installed" and the studio still refused.
        let listing = "aarch64-linux-android\nx86_64-unknown-linux-gnu\naarch64-apple-ios\n\n";
        let targets = parse_installed_targets(listing);
        assert_eq!(
            targets,
            vec![
                "aarch64-linux-android".to_string(),
                "x86_64-unknown-linux-gnu".to_string(),
                "aarch64-apple-ios".to_string(),
            ]
        );
    }

    #[test]
    fn rustups_explanatory_lines_are_not_mistaken_for_triples() {
        let listing = "error: no override and no default toolchain set\naarch64-apple-ios\n";
        assert_eq!(
            parse_installed_targets(listing),
            vec!["aarch64-apple-ios".to_string()]
        );
    }

    #[test]
    fn the_host_environment_finds_the_targets_this_machine_has() {
        // # Why the guard runs the real command rather than `rustup --version`
        //
        // It used to check that `rustup --version` succeeded, on the reasoning
        // that a distro-packaged Rust has no `rustup` and an empty target list
        // is correct there. That is the right idea guarding the wrong thing:
        // it establishes that the *binary exists*, while the assertion below
        // depends on `rustup target list --installed` actually **answering**.
        //
        // Those come apart, and not rarely. `rustup` re-syncs its channel
        // metadata before answering a query, so on a machine whose network
        // cannot reach `static.rust-lang.org` — an offline build, a CI runner
        // behind a strict egress policy, the sandbox this was found in —
        // `rustup --version` prints a version and exits 0 while `target list`
        // fails to download and prints nothing. `rustup_installed_targets`
        // then correctly returns an empty list, and this test failed pointing
        // at the studio, which had done nothing wrong.
        //
        // So the guard now runs the command whose output is being asserted on.
        // If it cannot answer, there is nothing here to test.
        let Ok(output) = std::process::Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        if parse_installed_targets(&String::from_utf8_lossy(&output.stdout)).is_empty() {
            return;
        }

        let env = Env::host();
        assert!(
            !env.rust_targets().is_empty(),
            "rustup answered with targets, so `Env::host` must have found them too"
        );
    }

    #[test]
    fn a_signing_identity_is_read_out_of_securitys_listing() {
        let listing = "  1) ABCDEF0123 \"Apple Development: Someone (TEAM1234)\"\n     1 valid identities found\n";
        assert_eq!(
            parse_signing_identity(listing).as_deref(),
            Some("Apple Development: Someone (TEAM1234)")
        );
    }

    #[test]
    fn a_keychain_with_no_development_certificate_has_no_identity() {
        assert_eq!(
            parse_signing_identity("     0 valid identities found\n"),
            None
        );
    }

    #[test]
    fn ios_without_a_signing_identity_is_not_ready_and_says_which_piece() {
        // Xcode installed, Apple ID never added: the case that used to report
        // "ready" and then fail inside `xcodebuild` with a certificate error.
        let s = Scratch::new("ios-unsigned");
        for tool in ["cargo", "xcodebuild", "xcrun"] {
            s.exe(&format!("bin/{tool}"));
        }
        let unsigned = Env::bare(Host::MacOs)
            .path_dir(s.dir("bin"))
            .rust_target("aarch64-apple-ios");
        let unsigned_report = report(&unsigned, Target::Ios);
        assert!(!unsigned_report.ready());
        assert!(
            unsigned_report
                .missing()
                .iter()
                .any(|r| r.name == "Signing identity"),
            "{:?}",
            unsigned_report
                .missing()
                .iter()
                .map(|r| r.name)
                .collect::<Vec<_>>()
        );

        // And the same machine with an identity is ready.
        let signed = unsigned.with_signing_identity("Apple Development: Someone (TEAM1234)");
        assert!(report(&signed, Target::Ios).ready());
    }
}
