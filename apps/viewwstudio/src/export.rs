//! N5: turning a project into something you can hand to somebody.
//!
//! # What this module is, and what it deliberately is not
//!
//! It is a **planner**. Given the environment ([`toolchains::Env`]), a target
//! and a chosen [`Format`], it returns either a [`Plan`] — an ordered list of
//! child processes with an artefact expected at the end — or a [`Refusal`]
//! saying why there is nothing worth starting.
//!
//! It is not a runner. Running is [`jobs::Queue`](crate::jobs)'s job, which
//! already streams, cancels and reports. Keeping the plan separate from the
//! run is what makes every case here reachable from a test on a machine with
//! no Android SDK, no Xcode, and no phone plugged in — which is this container,
//! and every CI runner.
//!
//! # Refusal before start is the whole design
//!
//! The alternative — start `cargo ndk`, let it fail, print the error — costs a
//! process launch and produces a message about a linker when the real answer
//! is "you have not installed the NDK". [`toolchains`] already knows what is
//! missing and what command installs it. This asks it *first*, and the
//! refusal names the requirement and its fix.
//!
//! The one refusal that is not about a missing tool is iOS on a machine that
//! is not a Mac. That is a rule of Apple's, not a gap to work around, and it
//! is reported as such rather than as a missing dependency somebody might go
//! looking for.
//!
//! # Nothing here is biased toward a platform
//!
//! Every target is described the same way — a list of steps and an artefact —
//! and the host's own platform appears only where it genuinely decides
//! something: the executable's file name, and Apple's macOS rule. A target
//! this studio does not yet know how to package says so; it does not fall back
//! to the desktop's answer.

use std::path::{Path, PathBuf};

use crate::task::Spec;
use crate::toolchains::{self, Env, Host, Target};

/// What a target can be asked to produce.
///
/// Separate from [`Target`] because one target has more than one answer: an
/// iOS build is a simulator `.app` or a signed `.ipa`, and those differ in
/// what they need, not only in what they emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// An optimised binary for the machine the studio is running on, copied
    /// out of `target/release` into an export directory beside it.
    DesktopBinary,
    /// A debug-signed `.apk`.
    ///
    /// Debug-signed, and named so in the UI: a release-signed APK needs a
    /// keystore, a key alias and two passwords, none of which the studio has
    /// anywhere to put and none of which should be typed into a text field
    /// that logs to an Output panel. A debug-signed APK installs on any phone
    /// with developer mode on, which is what "give this to a colleague"
    /// means.
    AndroidApk,
    /// A Windows `.exe`, cross-compiled when the host is not Windows.
    ///
    /// The one export that produces something for a machine the user is very
    /// likely *not* sitting at, which is exactly why it is worth having: a
    /// person developing on Linux can hand a colleague a Windows build without
    /// owning a Windows machine to make it on.
    WindowsExe,
    /// An unsigned `.app` for the iOS simulator.
    IosSimulatorApp,
    /// A signed `.ipa` for a device.
    IosIpa,
}

impl Format {
    /// Every format, in the order the export sheet lists them.
    pub const ALL: [Self; 5] = [
        Self::DesktopBinary,
        Self::WindowsExe,
        Self::AndroidApk,
        Self::IosSimulatorApp,
        Self::IosIpa,
    ];

    #[must_use]
    pub const fn target(self) -> Target {
        match self {
            Self::DesktopBinary => Target::Desktop,
            Self::WindowsExe => Target::Windows,
            Self::AndroidApk => Target::Android,
            Self::IosSimulatorApp | Self::IosIpa => Target::Ios,
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::DesktopBinary => "Desktop binary",
            Self::WindowsExe => "Windows .exe",
            Self::AndroidApk => "Android APK (debug-signed)",
            Self::IosSimulatorApp => "iOS simulator app",
            Self::IosIpa => "iOS .ipa (signed)",
        }
    }

    /// The one-line explanation under the row.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::DesktopBinary => "an optimised build for this machine, copied into export/",
            Self::WindowsExe => "a 64-bit Windows build; cross-compiled when this is not Windows",
            Self::AndroidApk => "installs on any phone with developer mode on",
            Self::IosSimulatorApp => "runs in the simulator; not installable on a device",
            Self::IosIpa => "needs a signing identity, and macOS",
        }
    }
}

/// Why an export was not started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The target cannot be built on this host whatever is installed.
    Impossible(&'static str),
    /// Something is missing. Named, with the command that installs it.
    Missing {
        requirement: &'static str,
        install: &'static str,
    },
    /// There is no project open, or it is not a cargo project.
    NoProject,
    /// The studio does not know how to package this yet, and says so rather
    /// than producing something that is not what was asked for.
    NotImplemented(&'static str),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Impossible(why) => write!(f, "{why}"),
            Self::Missing {
                requirement,
                install,
            } => write!(f, "{requirement} is missing — install it with: {install}"),
            Self::NoProject => write!(f, "no cargo project is open"),
            Self::NotImplemented(what) => write!(f, "{what}"),
        }
    }
}

/// One child process in a plan, with what it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What the Tasks panel calls it — "Build (release)", "Package APK".
    pub spec: Spec,
    /// One line for the export sheet's preview, so a person can see what is
    /// about to run before it runs.
    pub why: &'static str,
}

/// Everything an export will do, and what it will leave behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub format: Format,
    /// Run in order. A step that fails stops the plan — a package built from
    /// a failed compile is worse than no package.
    pub steps: Vec<Step>,
    /// Where the result is expected. Reported at the end; the studio does not
    /// open it, because what to do with a build is the user's business.
    pub artefact: PathBuf,
}

/// The directory exports are written to.
///
/// Inside `target/`, not beside the project: it is generated output, it is
/// already in every `.gitignore` that exists, and putting an APK next to
/// `src/` is how a repository acquires a fifty-megabyte binary nobody meant to
/// commit.
#[must_use]
pub fn export_dir(root: &Path) -> PathBuf {
    root.join("target").join("export")
}

/// Plan an export, or say why there is none.
///
/// `name` is the package's name, which decides the artefact's file name. The
/// caller reads it from `Cargo.toml`; it is a parameter rather than something
/// this module parses, so every case below is reachable from a test.
pub fn plan(env: &Env, root: &Path, name: &str, format: Format) -> Result<Plan, Refusal> {
    if name.is_empty() {
        return Err(Refusal::NoProject);
    }

    // The checklist first, always. See the module docs: a refusal that names
    // the missing tool and its install command is worth more than the error
    // the tool would print about itself.
    let report = toolchains::report(env, format.target());
    if let Some(why) = report.impossible {
        return Err(Refusal::Impossible(why));
    }
    // A simulator build is signed ad hoc by Xcode itself ("Sign to Run
    // Locally"); only a device `.ipa` needs a real identity. Requiring one
    // here refused the simulator export to everybody without an Apple
    // developer account, which is the format that exists for exactly them.
    let missing = report
        .missing()
        .into_iter()
        .find(|r| !(format == Format::IosSimulatorApp && r.name == "Signing identity"));
    if let Some(missing) = missing {
        return Err(Refusal::Missing {
            requirement: missing.name,
            install: missing.install,
        });
    }

    let out = export_dir(root);
    match format {
        Format::DesktopBinary => Ok(desktop(env, root, name, &out)),
        Format::WindowsExe => Ok(windows(env, root, name, &out)),
        Format::AndroidApk => android(env.host, root, name, &out),
        Format::IosSimulatorApp => ios_app(env, root, name, &out),
        Format::IosIpa => ios_ipa(env, root, name, &out),
    }
}

fn desktop(env: &Env, root: &Path, name: &str, out: &Path) -> Plan {
    // The host's own naming, which is the one place a desktop export is
    // allowed to care what it is running on.
    let file = match env.host {
        Host::Windows => format!("{name}.exe"),
        _ => name.to_string(),
    };
    let built = root.join("target").join("release").join(&file);
    let artefact = out.join(&file);

    let mut steps = vec![Step {
        spec: Spec::new("Build (release)", "cargo", root).args(["build", "--release"]),
        why: "an optimised build, which is what ships",
    }];
    // Copied through the queue rather than with `std::fs::copy` on the UI
    // thread: it is one more step in the same queue, cancellable and logged
    // with everything else, and a copy of a hundred-megabyte binary is not
    // free.
    steps.extend(
        copy_steps(env.host, root, &built, &artefact)
            .into_iter()
            .map(|spec| Step {
                spec,
                why: "copied out of target/release, where the next build overwrites it",
            }),
    );

    Plan {
        format: Format::DesktopBinary,
        steps,
        artefact,
    }
}

/// A Windows `.exe`, from whichever host is asking.
///
/// On Windows this is [`desktop`] with a different name on the row — the host
/// toolchain already produces a `.exe`, and passing `--target` there would ask
/// for a cross-compile of a machine to itself, which needs a target that is not
/// installed by default and buys nothing.
///
/// Everywhere else it is an explicit `--target x86_64-pc-windows-gnu`, which
/// also moves where the binary lands: `target/<triple>/release/` rather than
/// `target/release/`. Getting that path wrong is the classic cross-compile bug
/// — the build succeeds, the copy silently takes the *host* binary sitting in
/// `target/release` from an earlier run, and the user is handed a Linux ELF
/// named `.exe`.
fn windows(env: &Env, root: &Path, name: &str, out: &Path) -> Plan {
    const TRIPLE: &str = "x86_64-pc-windows-gnu";
    let file = format!("{name}.exe");
    let cross = env.host != Host::Windows;

    let built = if cross {
        root.join("target").join(TRIPLE).join("release").join(&file)
    } else {
        root.join("target").join("release").join(&file)
    };
    let artefact = out.join(&file);

    let spec = if cross {
        Spec::new("Build (release, Windows)", "cargo", root).args([
            "build",
            "--release",
            "--target",
            TRIPLE,
        ])
    } else {
        Spec::new("Build (release, Windows)", "cargo", root).args(["build", "--release"])
    };

    let mut steps = vec![Step {
        spec,
        why: if cross {
            "cross-compiled for Windows; the MinGW linker produces the .exe"
        } else {
            "an optimised build, which is what ships"
        },
    }];
    steps.extend(
        copy_steps(env.host, root, &built, &artefact)
            .into_iter()
            .map(|spec| Step {
                spec,
                why: "copied out of the target directory, where the next build overwrites it",
            }),
    );

    Plan {
        format: Format::WindowsExe,
        steps,
        artefact,
    }
}

/// `./gradlew` when the project has a wrapper, and `gradle` when it does not.
///
/// # Why this is a choice rather than a constant
///
/// The wrapper is the right thing to run — it pins the Gradle version, so a
/// build does not change under you when Homebrew updates. But a wrapper is a
/// shell script *and a binary JAR*, and [`scaffold`](crate::scaffold) writes
/// source files; emitting a JAR from a scaffolder is not something to start
/// doing for one build step.
///
/// So a scaffolded project ships `gradle-wrapper.properties` and no JAR, this
/// runs the system `gradle` until somebody runs `gradle wrapper` once, and from
/// then on it runs the wrapper — including for every project that arrived with
/// one already, which is most projects that are not new.
///
/// Checked on disk rather than assumed, because the answer changes during the
/// life of a project and an export that hard-coded either one would be wrong
/// for half of it.
fn gradle_command(root: &Path) -> &'static str {
    let wrapper = if cfg!(windows) {
        root.join("android").join("gradlew.bat")
    } else {
        root.join("android").join("gradlew")
    };
    if wrapper.is_file() {
        if cfg!(windows) {
            "gradlew.bat"
        } else {
            "./gradlew"
        }
    } else {
        "gradle"
    }
}

fn android(host: Host, root: &Path, name: &str, out: &Path) -> Result<Plan, Refusal> {
    let artefact = out.join(format!("{name}.apk"));
    let gradle_output = root
        .join("android")
        .join("app")
        .join("build")
        .join("outputs")
        .join("apk")
        .join("debug")
        .join("app-debug.apk");
    let mut steps = vec![
        Step {
            // `cargo ndk` rather than `cargo build --target`: the NDK's
            // linker has to be on the path with the right sysroot for the
            // API level, and `cargo-ndk` exists to do exactly that. Doing
            // it by hand means reproducing its environment setup here and
            // getting it wrong on the next NDK release.
            spec: Spec::new("Build (Android)", "cargo", root).args([
                "ndk",
                "-t",
                "arm64-v8a",
                "-o",
                "target/android/jniLibs",
                "build",
                "--release",
            ]),
            why: "cross-compiles the Rust for arm64 with the NDK's linker",
        },
        Step {
            spec: Spec::new("Package APK", gradle_command(root), root.join("android"))
                .args(["assembleDebug"]),
            why: "packages and debug-signs the apk",
        },
    ];
    steps.extend(
        copy_steps(host, root, &gradle_output, &artefact)
            .into_iter()
            .map(|spec| Step {
                spec,
                why: "copied out of the gradle output tree under its own name",
            }),
    );
    Ok(Plan {
        format: Format::AndroidApk,
        steps,
        artefact,
    })
}

fn ios_app(env: &Env, root: &Path, name: &str, out: &Path) -> Result<Plan, Refusal> {
    if env.host != Host::MacOs {
        return Err(Refusal::Impossible(
            "Apple's toolchain runs only on macOS, so no iOS build is possible on this machine",
        ));
    }
    let artefact = out.join(format!("{name}.app"));
    Ok(Plan {
        format: Format::IosSimulatorApp,
        steps: vec![
            Step {
                spec: Spec::new("Build (iOS simulator)", "cargo", root).args([
                    "build",
                    "--release",
                    "--target",
                    "aarch64-apple-ios-sim",
                ]),
                why: "the simulator runs arm64 on an Apple-silicon Mac",
            },
            Step {
                // `CONFIGURATION_BUILD_DIR`: without it the `.app` lands in
                // Xcode's DerivedData, nothing copied it out, and the plan's
                // artefact path never existed — an export that "succeeded"
                // and produced nothing.
                spec: Spec::new("Package .app", "xcodebuild", root.join("ios"))
                    .args([
                        "-scheme",
                        name,
                        "-sdk",
                        "iphonesimulator",
                        "-configuration",
                        "Release",
                        "build",
                    ])
                    .arg(format!("CONFIGURATION_BUILD_DIR={}", out.to_string_lossy())),
                why: "assembles the bundle Xcode's project describes",
            },
        ],
        artefact,
    })
}

fn ios_ipa(env: &Env, root: &Path, name: &str, out: &Path) -> Result<Plan, Refusal> {
    if env.host != Host::MacOs {
        return Err(Refusal::Impossible(
            "a signed .ipa can only be produced on macOS — an Apple rule, not a limitation to \
             work around",
        ));
    }
    let artefact = out.join(format!("{name}.ipa"));
    Ok(Plan {
        format: Format::IosIpa,
        steps: vec![
            Step {
                spec: Spec::new("Build (iOS device)", "cargo", root).args([
                    "build",
                    "--release",
                    "--target",
                    "aarch64-apple-ios",
                ]),
                why: "every device since the iPhone 5s is arm64",
            },
            Step {
                spec: Spec::new("Archive", "xcodebuild", root.join("ios")).args([
                    "-scheme",
                    name,
                    "-sdk",
                    "iphoneos",
                    "-configuration",
                    "Release",
                    "archive",
                    "-archivePath",
                    "build/app.xcarchive",
                ]),
                why: "signs with whichever identity the project is configured for",
            },
            Step {
                spec: Spec::new("Export .ipa", "xcodebuild", root.join("ios"))
                    .args(["-exportArchive", "-archivePath", "build/app.xcarchive"])
                    .arg("-exportPath")
                    .arg(out.to_string_lossy().to_string())
                    .args(["-exportOptionsPlist", "ExportOptions.plist"]),
                why: "turns the signed archive into an installable .ipa",
            },
        ],
        artefact,
    })
}

/// A copy step, expressed as a command so it joins the same queue.
///
/// # Why this branches on the host
///
/// It used to be one `sh -c "mkdir -p … && cp -R …"` for every host, on the
/// reasoning that `cp` is what "the two shells this runs under both have".
/// **There is no `sh` on a stock Windows machine** — no `cp` and no `mkdir -p`
/// either — so a Windows desktop export failed before it copied anything, with
/// an error naming a shell the user had never heard of and had no reason to
/// install. A developer who installed the studio and nothing else hit it on
/// their first export.
///
/// So the shell is gone on both sides rather than swapped for a different one:
///
/// * **Unix** runs `cp -R` directly. No `sh`, so no quoting question at all —
///   the arguments are passed as arguments, which is stronger than any shell
///   quoting could be.
/// * **Windows** runs `cmd /C` with `xcopy`. `xcopy` is chosen over `copy`
///   because it creates the destination directory itself (answering `/I`) and
///   handles a directory source, which is what the `-R` on the Unix side is
///   for. The `mkdir` step disappears with it.
///
/// The directory is still created explicitly on Unix, as a separate step, so
/// that a failure to create it is reported as its own line in the Output panel
/// rather than as a confusing "cp: no such file or directory".
fn copy_steps(host: Host, root: &Path, from: &Path, to: &Path) -> Vec<Spec> {
    let dest_dir = to
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    if host == Host::Windows {
        // `xcopy src dest* /Y /I /E /Q`. The trailing `*` on a file
        // destination is how `xcopy` is told the target is a file and not a
        // directory it should prompt about — the prompt is what would hang a
        // headless queue forever.
        let destination = format!("{}*", to.to_string_lossy());
        vec![Spec::new("Copy artefact", "cmd", root)
            .arg("/C")
            .arg("xcopy")
            .arg(from.to_string_lossy().to_string())
            .arg(destination)
            .args(["/Y", "/I", "/E", "/Q"])]
    } else {
        vec![
            Spec::new("Create export directory", "mkdir", root)
                .arg("-p")
                .arg(dest_dir.to_string_lossy().to_string()),
            Spec::new("Copy artefact", "cp", root)
                .arg("-R")
                .arg(from.to_string_lossy().to_string())
                .arg(to.to_string_lossy().to_string()),
        ]
    }
}

/// The `name` under `[package]` in a `Cargo.toml`.
///
/// # Why this is a scan and not a TOML parse
///
/// The studio has no TOML dependency and the question is small: the first
/// `name = "..."` inside the `[package]` table. Sections are tracked so a
/// `name` under `[[bin]]` or `[dependencies.serde]` is not mistaken for the
/// package's — which is the whole reason a plain search for `name =` is wrong.
///
/// A manifest this cannot read returns `None`, and the caller refuses with
/// [`Refusal::NoProject`] rather than guessing from the directory's name: a
/// folder and its package differ often enough that guessing is a coin flip,
/// and the artefact's name is what somebody double-clicks.
#[must_use]
pub fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(value) = line.strip_prefix("name") else {
            continue;
        };
        let value = value.trim_start();
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        // Comments and quotes, in that order: `name = "app" # the binary`.
        let value = value.split('#').next().unwrap_or_default().trim();
        let name = value.trim_matches(|c| c == '"' || c == '\'');
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Connected devices
// ---------------------------------------------------------------------------

/// A device an export could be installed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// What `adb -s` takes, or what `simctl` calls a UDID.
    pub id: String,
    /// A name a person recognises. Falls back to the id.
    pub name: String,
    pub kind: Target,
    pub state: DeviceState,
}

/// Whether a device can be installed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    Ready,
    /// Plugged in, but the phone has not accepted this computer's key. The
    /// commonest state for a phone somebody has just connected, and the one
    /// worth naming: "offline" would send them looking at the cable.
    Unauthorised,
    /// Listed but not usable — booting, offline, or in recovery.
    Unavailable(String),
}

impl DeviceState {
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            Self::Ready => "ready",
            Self::Unauthorised => "unauthorised — accept the prompt on the device",
            Self::Unavailable(why) => why,
        }
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// The command that lists Android devices.
#[must_use]
pub fn adb_devices(root: &Path) -> Spec {
    Spec::new("Scan devices", "adb", root)
        .args(["devices", "-l"])
        .timeout(std::time::Duration::from_secs(15))
}

/// Parse `adb devices -l`.
///
/// # The format, and why it is parsed rather than asked for as JSON
///
/// `adb` has no machine-readable mode. The output is a header line, then one
/// line per device: an id, a state, and space-separated `key:value` pairs of
/// which `model:` and `device:` are the ones worth showing. Anything that does
/// not match that shape is skipped rather than guessed at — a warning adb
/// prints on startup ("daemon not running") would otherwise become a device
/// named `*`.
#[must_use]
pub fn parse_adb_devices(output: &str) -> Vec<Device> {
    let mut devices = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("List of devices")
            || line.starts_with('*')
            || line.starts_with("adb server")
            || line.starts_with("daemon")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(id), Some(state)) = (parts.next(), parts.next()) else {
            continue;
        };
        let mut model = None;
        for pair in parts {
            if let Some(value) = pair.strip_prefix("model:") {
                model = Some(value.replace('_', " "));
            }
        }
        devices.push(Device {
            name: model.unwrap_or_else(|| id.to_string()),
            id: id.to_string(),
            kind: Target::Android,
            state: match state {
                "device" => DeviceState::Ready,
                "unauthorized" | "unauthorised" => DeviceState::Unauthorised,
                other => DeviceState::Unavailable(other.to_string()),
            },
        });
    }
    devices
}

/// Install an artefact on a device.
///
/// Only Android today, and the refusal for anything else says so rather than
/// running something that would half-work: installing to an iOS device is
/// `xcrun devicectl`, which needs a signed `.ipa` and a paired device, and
/// pretending otherwise would fail three steps later with an Xcode error.
pub fn install(root: &Path, device: &Device, artefact: &Path) -> Result<Spec, Refusal> {
    match device.kind {
        Target::Android => Ok(Spec::new("Install", "adb", root)
            .args(["-s", &device.id, "install", "-r"])
            .arg(artefact.to_string_lossy().to_string())
            .timeout(std::time::Duration::from_secs(300))),
        Target::Ios => Err(Refusal::NotImplemented(
            "installing to an iOS device needs `xcrun devicectl` and a paired device; the \
             simulator is reachable with `xcrun simctl install booted`",
        )),
        Target::Desktop | Target::Windows => Err(Refusal::NotImplemented(
            "a desktop binary is run, not installed — it is in target/export",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A fake environment with real files behind it.
    ///
    /// `toolchains` deliberately looks at the filesystem — a tool is a file on
    /// `PATH` with the executable bit, an SDK is a directory that exists — so
    /// a test that wants "everything installed" has to make it so rather than
    /// assert it. Everything lands under one temp directory that is removed
    /// with the test.
    fn complete(host: Host, tools: &[&str]) -> (Env, tempdir::Dir) {
        let dir = tempdir::Dir::new("viewwstudio-export");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("create bin");
        for tool in tools {
            let names: Vec<String> = if host == Host::Windows {
                vec![format!("{tool}.exe")]
            } else {
                vec![(*tool).to_string()]
            };
            for name in names {
                let path = bin.join(name);
                std::fs::write(&path, "").expect("write stub");
                executable(&path);
            }
        }

        let mut vars = BTreeMap::new();
        for (key, folder) in [
            ("ANDROID_HOME", "sdk"),
            ("ANDROID_NDK_HOME", "ndk"),
            ("JAVA_HOME", "jdk"),
        ] {
            let path = dir.path().join(folder);
            std::fs::create_dir_all(&path).expect("create sdk dir");
            vars.insert(key.to_string(), path.to_string_lossy().into_owned());
        }

        let env = Env {
            host,
            vars,
            path: vec![bin],
            rust_targets: toolchains::Probed::Given(vec![
                "aarch64-linux-android".into(),
                "aarch64-apple-ios".into(),
                "aarch64-apple-ios-sim".into(),
            ]),
            signing_identity: toolchains::Probed::Given(Some(
                "Apple Development: Test (TEAMID)".into(),
            )),
        };
        (env, dir)
    }

    /// Give `path` the executable bit, where the platform has one.
    fn executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
    }

    /// A minimal scoped temp directory, so these tests do not need a
    /// dependency — the crate is `std`-only on purpose.
    mod tempdir {
        use std::path::{Path, PathBuf};

        #[derive(Debug)]
        pub(super) struct Dir(PathBuf);

        impl Dir {
            pub(super) fn new(prefix: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "{prefix}-{}-{:?}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("after the epoch")
                        .as_nanos()
                ));
                std::fs::create_dir_all(&path).expect("create temp dir");
                Self(path)
            }

            pub(super) fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn a_missing_tool_is_refused_before_a_process_is_started() {
        // The whole design. An empty PATH means no cargo, and the refusal
        // names it and how to get it rather than letting the shell say
        // "command not found".
        let (mut env, _dir) = complete(Host::Linux, &[]);
        // An empty PATH: nothing is installed.
        env.path.clear();
        let refusal = plan(&env, Path::new("/project"), "app", Format::DesktopBinary)
            .expect_err("no cargo on an empty PATH");
        assert_eq!(
            refusal,
            Refusal::Missing {
                requirement: "cargo",
                install: "https://rustup.rs",
            }
        );
        assert!(refusal.to_string().contains("rustup.rs"));
    }

    #[test]
    fn ios_on_a_machine_that_is_not_a_mac_is_impossible_rather_than_missing() {
        // Not a dependency somebody could go and install, so it must not read
        // like one.
        let (env, _dir) = complete(Host::Linux, &["cargo"]);
        for format in [Format::IosSimulatorApp, Format::IosIpa] {
            let refusal = plan(&env, Path::new("/project"), "app", format).expect_err("not a Mac");
            assert!(
                matches!(refusal, Refusal::Impossible(_)),
                "{format:?} gave {refusal:?}"
            );
            assert!(refusal.to_string().contains("macOS"));
        }
    }

    #[test]
    fn a_desktop_export_builds_release_and_copies_the_binary_out() {
        let (env, _dir) = complete(Host::Linux, &["cargo"]);
        let plan = plan(&env, Path::new("/project"), "app", Format::DesktopBinary)
            .expect("a complete desktop toolchain");
        // Build, create the directory, copy. The directory is its own step so
        // a failure to create it reads as itself rather than as a confusing
        // "cp: no such file or directory".
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].spec.args, vec!["build", "--release"]);
        assert_eq!(plan.artefact, PathBuf::from("/project/target/export/app"));
        // Copied out, because the next release build overwrites the original.
        assert!(plan.steps[2].spec.command_line().contains("cp -R"));
    }

    #[test]
    fn a_windows_desktop_export_names_the_executable_the_way_windows_does() {
        // The one place a desktop export is allowed to care about the host.
        let (env, _dir) = complete(Host::Windows, &["cargo"]);
        let plan = plan(&env, Path::new("/project"), "app", Format::DesktopBinary)
            .expect("a complete desktop toolchain");
        assert_eq!(
            plan.artefact,
            PathBuf::from("/project/target/export/app.exe")
        );
    }

    #[test]
    fn an_android_export_cross_compiles_then_packages_then_copies() {
        let (env, _dir) = complete(Host::Linux, &["cargo", "cargo-ndk", "gradle", "javac"]);
        let plan = plan(&env, Path::new("/project"), "app", Format::AndroidApk)
            .expect("a complete Android toolchain");
        let labels: Vec<&str> = plan.steps.iter().map(|s| s.spec.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Build (Android)",
                "Package APK",
                "Create export directory",
                "Copy artefact"
            ]
        );
        assert!(plan.steps[0].spec.args.contains(&"ndk".to_string()));
        assert_eq!(
            plan.artefact,
            PathBuf::from("/project/target/export/app.apk")
        );
    }

    #[test]
    fn an_export_lands_in_target_rather_than_beside_the_source() {
        // A fifty-megabyte APK next to `src/` is how a repository acquires a
        // binary nobody meant to commit.
        assert_eq!(
            export_dir(Path::new("/a/b")),
            PathBuf::from("/a/b/target/export")
        );
    }

    #[test]
    fn a_project_with_no_name_is_refused_as_no_project() {
        let (env, _dir) = complete(Host::Linux, &["cargo"]);
        assert_eq!(
            plan(&env, Path::new("/project"), "", Format::DesktopBinary),
            Err(Refusal::NoProject)
        );
    }

    #[test]
    fn a_path_with_a_space_or_a_quote_in_it_is_one_argument() {
        // The normal case on two of the three desktops, and the one that used
        // to turn a copy step into two broken arguments. There is no shell to
        // quote for any more: the path is passed as an argument, which is why
        // it survives untouched.
        let awkward = Path::new("/Users/a b/it's/app");
        let steps = copy_steps(Host::MacOs, Path::new("/p"), awkward, Path::new("/out/app"));
        let copy = steps.last().expect("a copy step");
        assert_eq!(copy.program, "cp");
        assert!(
            copy.args.contains(&awkward.to_string_lossy().to_string()),
            "the path is one argument, verbatim: {:?}",
            copy.args
        );
    }

    #[test]
    fn a_windows_export_does_not_reach_for_a_unix_shell() {
        // The blocker this replaced: `sh` is not on a stock Windows machine,
        // and neither are `cp` and `mkdir`. An export that needs them fails
        // before it copies anything, naming a shell the user never installed.
        let (env, _dir) = complete(Host::Windows, &["cargo"]);
        let plan = plan(&env, Path::new("/project"), "app", Format::DesktopBinary)
            .expect("a complete desktop toolchain");
        for step in &plan.steps {
            assert!(
                !matches!(step.spec.program.as_str(), "sh" | "cp" | "mkdir"),
                "a Windows export must not run {}: {:?}",
                step.spec.program,
                step.spec
            );
        }
        let copy = plan
            .steps
            .iter()
            .find(|s| s.spec.label == "Copy artefact")
            .expect("a copy step");
        assert_eq!(copy.spec.program, "cmd");
        assert!(
            copy.spec.args.iter().any(|a| a == "xcopy"),
            "{:?}",
            copy.spec.args
        );
    }

    #[test]
    fn a_unix_export_copies_without_a_shell_wrapper() {
        let (env, _dir) = complete(Host::Linux, &["cargo"]);
        let plan = plan(&env, Path::new("/project"), "app", Format::DesktopBinary)
            .expect("a complete desktop toolchain");
        let copy = plan
            .steps
            .iter()
            .find(|s| s.spec.label == "Copy artefact")
            .expect("a copy step");
        assert_eq!(copy.spec.program, "cp");
        assert!(
            plan.steps.iter().any(|s| s.spec.program == "mkdir"),
            "the export directory is created as its own step"
        );
    }

    #[test]
    fn adb_output_becomes_devices_and_its_noise_does_not() {
        let output = "\
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554          device product:sdk_gphone64 model:Pixel_8 device:emu64a
FA7B2X01               unauthorized usb:1-3
R58M12ABCDE            offline

";
        let devices = parse_adb_devices(output);
        assert_eq!(devices.len(), 3, "the two daemon lines are not devices");

        assert_eq!(devices[0].id, "emulator-5554");
        assert_eq!(
            devices[0].name, "Pixel 8",
            "underscores are not in the name"
        );
        assert_eq!(devices[0].state, DeviceState::Ready);

        assert_eq!(devices[1].state, DeviceState::Unauthorised);
        assert!(
            devices[1].state.label().contains("accept the prompt"),
            "the commonest state says what to do about it"
        );
        // No `model:` on that line, so the id is the name rather than a blank.
        assert_eq!(devices[1].name, "FA7B2X01");

        assert_eq!(
            devices[2].state,
            DeviceState::Unavailable("offline".to_string())
        );
        assert!(!devices[2].state.is_ready());
    }

    #[test]
    fn empty_adb_output_is_no_devices_rather_than_an_error() {
        assert!(parse_adb_devices("List of devices attached\n\n").is_empty());
        assert!(parse_adb_devices("").is_empty());
    }

    #[test]
    fn installing_names_the_device_it_was_asked_about() {
        let device = Device {
            id: "emulator-5554".into(),
            name: "Pixel 8".into(),
            kind: Target::Android,
            state: DeviceState::Ready,
        };
        let spec = install(
            Path::new("/project"),
            &device,
            Path::new("/project/target/export/app.apk"),
        )
        .expect("android installs");
        // `-s <id>`, because a second phone plugged in mid-export would
        // otherwise make `adb install` ambiguous and it would refuse.
        assert!(spec.command_line().contains("-s emulator-5554"));
        assert!(spec.command_line().ends_with("app.apk"));
    }

    #[test]
    fn installing_to_a_target_with_no_path_says_so_rather_than_half_working() {
        for kind in [Target::Ios, Target::Desktop] {
            let device = Device {
                id: "x".into(),
                name: "x".into(),
                kind,
                state: DeviceState::Ready,
            };
            let refusal = install(Path::new("/p"), &device, Path::new("/p/app"))
                .expect_err("not implemented");
            assert!(matches!(refusal, Refusal::NotImplemented(_)));
        }
    }

    #[test]
    fn every_format_names_the_target_it_belongs_to() {
        // The sheet is built from `Format::ALL`, so a format with the wrong
        // target would run the wrong checklist and refuse for the wrong
        // reason.
        assert_eq!(Format::DesktopBinary.target(), Target::Desktop);
        assert_eq!(Format::AndroidApk.target(), Target::Android);
        assert_eq!(Format::IosSimulatorApp.target(), Target::Ios);
        assert_eq!(Format::IosIpa.target(), Target::Ios);
        for format in Format::ALL {
            assert!(!format.title().is_empty());
            assert!(!format.detail().is_empty());
        }
    }
    #[test]
    fn the_package_name_comes_from_the_package_table_and_nowhere_else() {
        let manifest = "\
[package]
name = \"photo-studio\"
version = \"0.1.0\"

[[bin]]
name = \"something-else\"

[dependencies.serde]
name = \"not-this-either\"
";
        assert_eq!(
            package_name(manifest),
            Some("photo-studio".to_string()),
            "a `name` under [[bin]] is not the package's"
        );
    }

    #[test]
    fn a_name_with_a_trailing_comment_is_still_the_name() {
        assert_eq!(
            package_name("[package]\nname = \"app\" # the binary\n"),
            Some("app".to_string())
        );
    }

    #[test]
    fn a_manifest_with_no_package_table_has_no_name() {
        // A workspace root, which is a real thing to have open and not a
        // thing to export.
        assert_eq!(package_name("[workspace]\nmembers = [\"a\"]\n"), None);
        assert_eq!(package_name(""), None);
    }
}
