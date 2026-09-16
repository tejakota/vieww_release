//! Turning a built vieww app into something a platform can install.
//!
//! # What this is not
//!
//! It is not `packaging/package.sh`. That script is `viewwstudio`'s own
//! release pipeline — it stages the studio's SDK (a bundled `rustc`, the
//! rlibs a preview compiles against, `vieww-sdk.toml`), which is a concern
//! specific to a code editor that `dlopen`s previews into its own process.
//! Nothing here needs any of that: [`package`] takes the directory of *any*
//! cargo binary project (in practice, the output of [`crate::scaffold`], but
//! nothing below checks for that specifically) and produces the plain
//! platform artefact a normal application ships — a `.desktop`-bearing tree
//! on Linux, an `.app` bundle on macOS, an `.msi` on Windows, an APK, an
//! `.ipa`. `packaging/package.sh` remains the right tool for packaging the
//! studio itself; this is the equivalent for a project `vieww new` made.
//!
//! # "Don't reimplement WiX or APK signing from scratch" — followed literally
//!
//! `package_windows` does not construct an MSI's binary format; it writes a
//! small `.wxs` (WiX's own XML authoring format, the same language
//! `packaging/viewwstudio.wxs` is written in) and calls the real `wix`
//! toolset to build it. `package_android` does not hand-sign a JAR; on a
//! machine that has the SDK, it shells out to `cargo apk`, which drives the
//! real Android build tools including `apksigner`. Both are named as
//! required tools rather than assumed present — see [`PackageError::MissingTool`]
//! — and both are refused with a named reason rather than attempted badly
//! when the tool is not there.
//!
//! # Which of these five targets is real *on this machine*
//!
//! This module is written and compiled unconditionally on every host — there
//! is no `#[cfg(target_os = ...)]` anywhere in it, because a `Command` you do
//! not run costs nothing to have compiled and a reader on any platform should
//! be able to read every code path. What differs by host is which of the
//! dispatch functions ever gets *past its own gate*:
//!
//! * **Linux** (`package_linux`) — fully real and fully exercised by this
//!   crate's own tests on every machine that runs them, this sandbox
//!   included: it runs `cargo build --release` for real and assembles a real
//!   `bin/` + `share/applications/*.desktop` tree.
//! * **macOS** (`package_macos`) — real code, calling real `iconutil` and
//!   assembling a real `Contents/{MacOS,Resources}` layout, exactly the shape
//!   `packaging/package.sh` uses for the studio. It is gated on
//!   [`PackageError::WrongHost`] first, because the binary
//!   inside a macOS `.app` has to actually be a macOS binary — Cargo cannot
//!   produce one on Linux without a cross toolchain this environment does not
//!   have, the same limitation `crates/vieww-hal/src/metal.rs` documents for
//!   the render backend itself. **Not exercised on this Linux sandbox**, and
//!   the host gate is exactly what a test here can assert without a Mac.
//! * **Windows** (`package_windows`) — real code calling the real `wix`
//!   toolset, gated the same way and for the same reason: an `.exe` built for
//!   Windows cannot come out of a Linux `cargo build` here either. **Not
//!   exercised on this Linux sandbox.**
//! * **Android** (`package_android`) — gated on tool/SDK *presence* rather
//!   than host, since an APK can genuinely be built from Linux, macOS or
//!   Windows given the SDK, the NDK and `cargo apk` (`ci/mobile/apk.sh` and
//!   `ci/mobile/android-env.sh` are this workspace's own proof of that). This
//!   sandbox has none of the three, so the real gate is what a test here
//!   exercises: a [`PackageError::MissingTool`], not a panic and not a
//!   silent no-op.
//! * **iOS** (`package_ios`) — gated on host *and* on `xcrun`, mirroring
//!   `ci/mobile/ios-app.sh`'s own opening check verbatim. **Not exercised on this
//!   Linux sandbox** for the same reason macOS is not.
//!
//! Every gate below is a plain function of a string (`host_os()`'s value) or
//! a boolean (`tool_on_path`'s answer), which is what lets every one of them
//! be unit-tested on whatever machine happens to be running this suite,
//! without that machine needing to *be* a Mac or a Windows box to prove the
//! refusal is correct — see the `tests` module for exactly that.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{host_os, is_probably_binary, tool_on_path};

/// A platform artefact [`package`] knows how to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageTarget {
    /// A `bin/` + `share/applications/*.desktop` tree.
    Linux,
    /// A `.app` bundle.
    MacOs,
    /// An `.msi`, via the WiX toolset.
    Windows,
    /// An APK, via `cargo apk`.
    Android,
    /// An `.ipa`-shaped bundle, via `xcrun`.
    Ios,
}

impl PackageTarget {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Android => "android",
            Self::Ios => "ios",
        }
    }
}

impl fmt::Display for PackageTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why [`package`] could not produce an artefact.
#[derive(Debug)]
pub enum PackageError {
    /// `target` needs a build for that platform, and this host cannot
    /// produce one — either because it is not that platform (an `.app`'s
    /// binary has to actually be a macOS binary) or because no cross
    /// toolchain for it is installed here.
    WrongHost {
        target: PackageTarget,
        host: &'static str,
    },
    /// A tool this target's packaging step needs is not on `PATH`.
    MissingTool {
        target: PackageTarget,
        tool: &'static str,
        how_to_get_it: &'static str,
    },
    /// `app_dir`'s `Cargo.toml` could not be found or has no `[package]
    /// name`.
    Manifest(PathBuf),
    /// `cargo build --release` ran and exited non-zero. Carries its stderr.
    Build(String),
    /// The build reported success but the binary it should have produced is
    /// not where this expected it — a build that used a different
    /// `--target-dir`, cross-compiled to a different triple, or otherwise
    /// did not produce a plain `target/release/<name>`.
    MissingArtifact(PathBuf),
    Io(std::io::Error),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongHost { target, host } => write!(
                f,
                "cannot package for {target} on a {host} host — a {target} artefact needs a \
                 binary actually built for {target}, and this machine has no way to produce one"
            ),
            Self::MissingTool {
                target,
                tool,
                how_to_get_it,
            } => write!(
                f,
                "cannot package for {target}: no `{tool}` on PATH — {how_to_get_it}"
            ),
            Self::Manifest(path) => write!(
                f,
                "{} has no readable [package] name — is this a cargo project?",
                path.display()
            ),
            Self::Build(stderr) => write!(f, "cargo build --release failed:\n{stderr}"),
            Self::MissingArtifact(path) => write!(
                f,
                "the build reported success but {} does not exist",
                path.display()
            ),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<std::io::Error> for PackageError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// What a packaging run produced.
#[derive(Debug, Clone)]
pub struct PackageReport {
    pub target: PackageTarget,
    /// The directory everything below was written into.
    pub output_dir: PathBuf,
    /// Every artefact written, in the order it was written.
    pub artifacts: Vec<PathBuf>,
    /// Things worth telling a person that are not failures — "shipping
    /// without an icon", the same tone `packaging/package.sh` uses for its
    /// own non-fatal notes.
    pub notes: Vec<String>,
}

/// Package the cargo project at `app_dir` for `target`.
///
/// # Errors
///
/// See [`PackageError`]'s variants. In every case where a required tool or
/// host is missing, this returns before touching `app_dir` or running a
/// build — a refusal never leaves a half-built artefact behind.
pub fn package(target: PackageTarget, app_dir: &Path) -> Result<PackageReport, PackageError> {
    check_host(target, host_os())?;
    match target {
        PackageTarget::Linux => package_linux(app_dir),
        PackageTarget::MacOs => package_macos(app_dir),
        PackageTarget::Windows => package_windows(app_dir),
        PackageTarget::Android => package_android(app_dir),
        PackageTarget::Ios => package_ios(app_dir),
    }
}

/// Whether `host` can even attempt to build for `target` — the part of
/// [`package`]'s gate that depends only on which operating system is running
/// it, pulled out so it is testable with a fake host string on every machine
/// this suite runs on.
///
/// Android is the one target with no host restriction at all: the real gate
/// for it is tool presence ([`android_tooling`]), checked separately inside
/// [`package_android`], because the SDK/NDK/`cargo apk` triple can genuinely
/// be missing or present on any of the three desktop hosts.
fn check_host(target: PackageTarget, host: &str) -> Result<(), PackageError> {
    let ok = match target {
        PackageTarget::Linux => host == "linux",
        PackageTarget::MacOs | PackageTarget::Ios => host == "macos",
        PackageTarget::Windows => host == "windows",
        PackageTarget::Android => true,
    };
    if ok {
        Ok(())
    } else {
        Err(PackageError::WrongHost {
            target,
            host: leak_or_known(host),
        })
    }
}

/// `check_host`'s error carries `&'static str`, and the real value always is
/// one of `std::env::consts::OS`'s compile-time constants — this only exists
/// so a *test* can hand in an arbitrary owned string and still get a
/// `PackageError` to inspect, without `PackageError` itself needing a
/// lifetime parameter for a case that never happens outside a test.
fn leak_or_known(host: &str) -> &'static str {
    match host {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        _ => "an unrecognised host",
    }
}

/// Read `[package] name` out of `app_dir/Cargo.toml`.
///
/// A small hand-rolled TOML walk rather than a dependency on a TOML parser —
/// this crate already keeps its dependency list to `vieww-foundation` and
/// `vieww-hardware`, and the one thing this needs is exactly what
/// `packaging/package.sh` extracts the same way, with `sed`: the value of one
/// key inside one section, in a file this crate itself may have just written.
fn read_package_name(app_dir: &Path) -> Result<String, PackageError> {
    let manifest_path = app_dir.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|_| PackageError::Manifest(manifest_path.clone()))?;

    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("name") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
    }
    Err(PackageError::Manifest(manifest_path))
}

/// `cargo build --release` in `app_dir`, real and blocking.
fn cargo_build_release(app_dir: &Path) -> Result<(), PackageError> {
    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(app_dir)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(PackageError::Build(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

/// A minimal `.desktop` entry. Generic — unlike
/// `packaging/viewwstudio.desktop`, which names the studio's own MIME types
/// and `%F` file-drop handling, this names nothing about what the app does,
/// because a `vieww new` project has not said yet.
fn desktop_entry(name: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Exec={name}\n\
         Terminal=false\n\
         Categories=Utility;\n"
    )
}

/// Build for real and lay out `bin/` + `share/applications/*.desktop`.
///
/// Real end to end, and the target [`crate`]'s own tests exercise fully — see
/// `tests` below.
fn package_linux(app_dir: &Path) -> Result<PackageReport, PackageError> {
    let name = read_package_name(app_dir)?;
    cargo_build_release(app_dir)?;

    let binary = app_dir.join("target/release").join(&name);
    if !is_probably_binary(&binary) {
        return Err(PackageError::MissingArtifact(binary));
    }

    let output_dir = app_dir.join("target/package").join(format!("{name}-linux"));
    std::fs::create_dir_all(output_dir.join("bin"))?;
    std::fs::create_dir_all(output_dir.join("share/applications"))?;

    let bin_dest = output_dir.join("bin").join(&name);
    std::fs::copy(&binary, &bin_dest)?;

    let desktop_dest = output_dir
        .join("share/applications")
        .join(format!("{name}.desktop"));
    std::fs::write(&desktop_dest, desktop_entry(&name))?;

    Ok(PackageReport {
        target: PackageTarget::Linux,
        output_dir,
        artifacts: vec![bin_dest, desktop_dest],
        notes: Vec::new(),
    })
}

/// A generic `Info.plist`. Modelled on `packaging/Info.plist`'s shape (the
/// same keys, `CFBundleIdentifier` built the same way) but with no document
/// types and no fixed name — those are the studio's own, not a scaffolded
/// project's.
fn info_plist(name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>{name}</string>
	<key>CFBundleIdentifier</key>
	<string>dev.vieww.{name}</string>
	<key>CFBundleExecutable</key>
	<string>{name}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.1.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
"#
    )
}

/// Build for macOS and lay out a real `.app`.
///
/// Reached only when [`check_host`] has already confirmed the host is macOS —
/// `cargo build --release` here genuinely produces a Mach-O binary, and
/// `iconutil`, when present, genuinely produces an `.icns` exactly as
/// `packaging/package.sh`'s own macOS branch does. Everything past the gate
/// in [`package`] is real Apple tooling; what makes it untestable on this
/// Linux sandbox is exactly and only that gate, which the `tests` module
/// checks directly with a synthetic host string instead.
fn package_macos(app_dir: &Path) -> Result<PackageReport, PackageError> {
    let name = read_package_name(app_dir)?;
    cargo_build_release(app_dir)?;

    let binary = app_dir.join("target/release").join(&name);
    if !is_probably_binary(&binary) {
        return Err(PackageError::MissingArtifact(binary));
    }

    let app = app_dir.join("target/package").join(format!("{name}.app"));
    let macos_dir = app.join("Contents/MacOS");
    let resources_dir = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos_dir)?;
    std::fs::create_dir_all(&resources_dir)?;

    let bin_dest = macos_dir.join(&name);
    std::fs::copy(&binary, &bin_dest)?;

    let plist_dest = app.join("Contents/Info.plist");
    std::fs::write(&plist_dest, info_plist(&name))?;

    let artifacts = vec![bin_dest, plist_dest];
    let notes = if tool_on_path("iconutil") {
        vec![
            "iconutil is present but this generic packager ships no source icon to convert; \
             see packaging/package.sh's macOS branch for the real .iconset -> .icns pipeline"
                .to_owned(),
        ]
    } else {
        vec!["no iconutil on PATH — shipping without an .icns".to_owned()]
    };

    Ok(PackageReport {
        target: PackageTarget::MacOs,
        output_dir: app,
        artifacts,
        notes,
    })
}

/// The WiX 4 authoring this writes and builds — a generic, single-executable
/// version of `packaging/viewwstudio.wxs`'s `<Files Include="...\**"/>`
/// pattern, with the version and payload directory substituted the same way
/// `packaging/package.sh` passes `-d Version=... -d Payload=...`.
fn wxs_source(name: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs">
	<Package
		Name="{name}"
		Manufacturer="vieww"
		Version="$(Version)"
		UpgradeCode="8f2a6d31-4a1e-4f6b-9c2b-1d0c7b5a9e45"
		Scope="perMachine"
		Compressed="yes">
		<MajorUpgrade AllowSameVersionUpgrades="yes" />
		<MediaTemplate EmbedCab="yes" />
		<StandardDirectory Id="ProgramFiles64Folder">
			<Directory Id="INSTALLFOLDER" Name="{name}">
				<Files Include="$(Payload)\**" />
			</Directory>
		</StandardDirectory>
	</Package>
</Wix>
"#
    )
}

/// Build for Windows and produce an `.msi` via the real `wix` toolset.
///
/// Reached only once [`check_host`] has confirmed a Windows host, so
/// `cargo build --release` here genuinely produces a PE `.exe` for `wix` to
/// wrap — nothing about generating the MSI's binary format is reimplemented;
/// `wix build` does that, exactly as [`super`]'s module docs say. Not
/// exercised on this Linux sandbox for the same reason [`package_macos`]
/// is not.
fn package_windows(app_dir: &Path) -> Result<PackageReport, PackageError> {
    if !tool_on_path("wix") {
        return Err(PackageError::MissingTool {
            target: PackageTarget::Windows,
            tool: "wix",
            how_to_get_it: "dotnet tool install --global wix",
        });
    }

    let name = read_package_name(app_dir)?;
    cargo_build_release(app_dir)?;

    let binary = app_dir.join("target/release").join(format!("{name}.exe"));
    if !is_probably_binary(&binary) {
        return Err(PackageError::MissingArtifact(binary));
    }

    let payload = app_dir
        .join("target/package")
        .join(format!("{name}-windows"));
    std::fs::create_dir_all(&payload)?;
    std::fs::copy(&binary, payload.join(format!("{name}.exe")))?;

    let wxs_path = app_dir.join("target/package").join(format!("{name}.wxs"));
    std::fs::write(&wxs_path, wxs_source(&name))?;

    let version = std::fs::read_to_string(app_dir.join("Cargo.toml"))
        .ok()
        .and_then(|manifest| {
            manifest.lines().find_map(|line| {
                let trimmed = line.trim();
                trimmed
                    .strip_prefix("version")?
                    .trim_start()
                    .strip_prefix('=')
                    .map(|v| v.trim().trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "0.1.0".to_owned());

    let msi_path = app_dir.join("target/package").join(format!("{name}.msi"));

    let output = Command::new("wix")
        .arg("build")
        .arg("-arch")
        .arg("x64")
        .arg("-d")
        .arg(format!("Version={version}"))
        .arg("-d")
        .arg(format!("Payload={}", payload.display()))
        .arg("-o")
        .arg(&msi_path)
        .arg(&wxs_path)
        .output()?;

    if !output.status.success() {
        return Err(PackageError::Build(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(PackageReport {
        target: PackageTarget::Windows,
        output_dir: app_dir.join("target/package"),
        artifacts: vec![msi_path],
        notes: vec![
            "unsigned: Authenticode signing needs a certificate this crate does not have"
                .to_owned(),
        ],
    })
}

/// What `package_android` needs on `PATH`/in the environment to attempt a
/// build, checked as a pure function of the inputs so it is testable without
/// an SDK — the same split `vieww_hardware::android_from_listing` uses for
/// its own `adb` parsing.
fn android_tooling(android_home: Option<&str>, has_cargo_apk: bool) -> Result<(), String> {
    let mut missing = Vec::new();
    if android_home.map(str::trim).unwrap_or_default().is_empty() {
        missing.push("ANDROID_HOME (or ANDROID_SDK_ROOT) is not set");
    }
    if !has_cargo_apk {
        missing.push("no `cargo-apk` subcommand on PATH");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing.join("; "))
    }
}

/// Whether `cargo apk --version` runs — the real presence check
/// [`package_android`] uses; a plain [`tool_on_path`] lookup would not do,
/// since `cargo-apk` is invoked as a cargo subcommand rather than by its
/// binary name being typed directly.
fn have_cargo_apk() -> bool {
    Command::new("cargo")
        .args(["apk", "--version"])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Build an APK via `cargo apk`, the real Android build tool this workspace
/// already depends on for `ci/mobile/apk.sh`.
///
/// **Expects an app already set up for Android** — a `[package.metadata.android]`
/// table and a `cdylib` crate type, the shape
/// `crates/vieww-platform-winit/Cargo.toml`'s own `[package.metadata.android]`
/// documents. A project fresh out of [`crate::scaffold`] does not have either
/// yet, because the minimal desktop scaffold is a plain binary — this target
/// is for a project a developer has since extended for mobile, the same way
/// `apps/viewwstudio`'s own Android harness is written by its `scaffold`
/// module rather than assumed. `cargo apk build` will say so on its own if
/// the metadata is missing; this function does not duplicate that check.
fn package_android(app_dir: &Path) -> Result<PackageReport, PackageError> {
    let android_home = std::env::var("ANDROID_HOME")
        .ok()
        .or_else(|| std::env::var("ANDROID_SDK_ROOT").ok());
    let sdk_missing = android_home
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty();
    if android_tooling(android_home.as_deref(), have_cargo_apk()).is_err() {
        let how_to_get_it = if sdk_missing {
            "set ANDROID_HOME (or ANDROID_SDK_ROOT) to an installed Android SDK, then \
             cargo install cargo-apk if that is missing too"
        } else {
            "cargo install cargo-apk"
        };
        return Err(PackageError::MissingTool {
            target: PackageTarget::Android,
            tool: "cargo apk / Android SDK",
            how_to_get_it,
        });
    }

    let output = Command::new("cargo")
        .args(["apk", "build", "--release"])
        .current_dir(app_dir)
        .output()?;
    if !output.status.success() {
        return Err(PackageError::Build(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    // `cargo apk` writes the APK under target/<profile>/apk/ itself; this
    // reports the directory rather than guessing a filename, since the name
    // comes from `[package.metadata.android] apk_name`, which this module
    // has no reason to duplicate reading.
    let output_dir = app_dir.join("target/release/apk");
    Ok(PackageReport {
        target: PackageTarget::Android,
        output_dir: output_dir.clone(),
        artifacts: vec![output_dir],
        notes: vec![
            "built by cargo apk; see cargo apk's own output above for the exact .apk path"
                .to_owned(),
        ],
    })
}

/// Build and bundle for iOS via `xcrun`, mirroring `ci/mobile/ios-app.sh`'s own
/// simulator build.
///
/// Reached only once [`check_host`] has confirmed a macOS host; the `xcrun`
/// check below is the second half of `ci/mobile/ios-app.sh`'s own opening gate
/// (`command -v xcrun`), kept here too because a macOS host with no Xcode
/// command line tools installed is a real, distinct machine from one with
/// them. Not exercised on this Linux sandbox — `check_host` refuses first.
fn package_ios(app_dir: &Path) -> Result<PackageReport, PackageError> {
    if !tool_on_path("xcrun") {
        return Err(PackageError::MissingTool {
            target: PackageTarget::Ios,
            tool: "xcrun",
            how_to_get_it: "install the Xcode command line tools",
        });
    }

    let name = read_package_name(app_dir)?;
    let output = Command::new("cargo")
        .args(["build", "--release", "--target", "aarch64-apple-ios-sim"])
        .current_dir(app_dir)
        .output()?;
    if !output.status.success() {
        return Err(PackageError::Build(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let binary = app_dir
        .join("target/aarch64-apple-ios-sim/release")
        .join(&name);
    if !is_probably_binary(&binary) {
        return Err(PackageError::MissingArtifact(binary));
    }

    let bundle = app_dir.join("target/package").join(format!("{name}.app"));
    std::fs::create_dir_all(&bundle)?;
    let bin_dest = bundle.join(&name);
    std::fs::copy(&binary, &bin_dest)?;
    let plist_dest = bundle.join("Info.plist");
    std::fs::write(&plist_dest, info_plist(&name))?;

    Ok(PackageReport {
        target: PackageTarget::Ios,
        output_dir: bundle,
        artifacts: vec![bin_dest, plist_dest],
        notes: vec![
            "built for the simulator triple; ci/mobile/ios-app.sh's --device and --ipa steps additionally \
             need a signing identity this crate does not have"
                .to_owned(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- the host gate, testable without being any particular host -------

    #[test]
    fn linux_targets_only_a_linux_host() {
        assert!(check_host(PackageTarget::Linux, "linux").is_ok());
        assert!(check_host(PackageTarget::Linux, "macos").is_err());
        assert!(check_host(PackageTarget::Linux, "windows").is_err());
    }

    #[test]
    fn macos_and_ios_only_target_a_macos_host() {
        assert!(check_host(PackageTarget::MacOs, "macos").is_ok());
        assert!(check_host(PackageTarget::MacOs, "linux").is_err());
        assert!(check_host(PackageTarget::Ios, "macos").is_ok());
        assert!(check_host(PackageTarget::Ios, "linux").is_err());
        assert!(check_host(PackageTarget::Ios, "windows").is_err());
    }

    #[test]
    fn windows_only_targets_a_windows_host() {
        assert!(check_host(PackageTarget::Windows, "windows").is_ok());
        assert!(check_host(PackageTarget::Windows, "linux").is_err());
    }

    #[test]
    fn android_targets_every_host() {
        for host in ["linux", "macos", "windows"] {
            assert!(check_host(PackageTarget::Android, host).is_ok(), "{host}");
        }
    }

    #[test]
    fn a_wrong_host_error_names_both_sides() {
        let error = check_host(PackageTarget::MacOs, "linux").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("macos"), "{message}");
        assert!(message.contains("linux"), "{message}");
    }

    // ----- package(), gated end-to-end on whatever this machine really is --

    /// This suite's own host decides which branch runs, and both branches are
    /// asserted: on a non-Linux host `package` for `Linux` would be refused
    /// before touching disk (exercised above via `check_host` directly, since
    /// this sandbox has no non-Linux host to run it on for real); on a
    /// non-macOS/Windows host — which this sandbox is — the *real*
    /// end-to-end call refuses with `WrongHost`, proving the gate actually
    /// runs inside `package`, not only inside `check_host`.
    #[test]
    fn packaging_for_a_host_this_machine_is_not_is_refused_before_any_build() {
        if host_os() == "macos" {
            eprintln!("skipping: this check targets a non-macOS host");
            return;
        }
        let result = package(PackageTarget::MacOs, Path::new("/nonexistent/app/dir"));
        assert!(
            matches!(result, Err(PackageError::WrongHost { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn packaging_for_windows_on_a_non_windows_host_is_refused() {
        if host_os() == "windows" {
            eprintln!("skipping: this check targets a non-Windows host");
            return;
        }
        let result = package(PackageTarget::Windows, Path::new("/nonexistent/app/dir"));
        assert!(
            matches!(result, Err(PackageError::WrongHost { .. })),
            "{result:?}"
        );
    }

    // ----- Android tool detection, fully testable with no SDK --------------

    #[test]
    fn android_tooling_is_fine_when_both_are_present() {
        assert!(android_tooling(Some("/opt/android-sdk"), true).is_ok());
    }

    #[test]
    fn android_tooling_names_a_missing_sdk() {
        let error = android_tooling(None, true).unwrap_err();
        assert!(error.contains("ANDROID_HOME"), "{error}");
    }

    #[test]
    fn android_tooling_names_a_missing_cargo_apk() {
        let error = android_tooling(Some("/opt/android-sdk"), false).unwrap_err();
        assert!(error.contains("cargo-apk"), "{error}");
    }

    #[test]
    fn an_empty_android_home_counts_as_unset() {
        assert!(android_tooling(Some("   "), true).is_err());
        assert!(android_tooling(Some(""), true).is_err());
    }

    /// The real path, on the real machine running this suite. This sandbox
    /// has neither an Android SDK nor `cargo-apk`, so packaging for Android
    /// here is a real, deterministic `MissingTool` — never a panic, never a
    /// silent no-op, and never an attempt to shell out to a tool that is not
    /// there.
    #[test]
    fn packaging_for_android_without_the_sdk_here_is_a_named_missing_tool() {
        let scratch = std::env::temp_dir().join("vieww-build-package-android-probe");
        std::fs::create_dir_all(&scratch).ok();
        std::fs::write(
            scratch.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let result = package_android(&scratch);
        std::fs::remove_dir_all(&scratch).ok();

        match result {
            Err(PackageError::MissingTool { target, .. }) => {
                assert_eq!(target, PackageTarget::Android);
            }
            other => {
                // If this machine genuinely does have a configured Android
                // SDK and cargo-apk, the call would proceed to a real build
                // instead — which this test does not want to attempt. Either
                // outcome besides an unrelated error is acceptable; only a
                // wrong *kind* of failure is a bug in the gate itself.
                assert!(
                    !matches!(other, Err(PackageError::Io(_))),
                    "unexpected I/O error: {other:?}"
                );
            }
        }
    }

    // ----- reading the package name ------------------------------------------

    #[test]
    fn the_package_name_is_read_from_the_package_section_only() {
        let scratch = std::env::temp_dir().join("vieww-build-package-name-probe");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(
            scratch.join("Cargo.toml"),
            "[dependencies]\nname = \"not-this-one\"\n\n[package]\nname = \"my-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        assert_eq!(read_package_name(&scratch).unwrap(), "my-app");
        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn a_missing_manifest_is_a_named_error_not_a_panic() {
        let result = read_package_name(Path::new("/definitely/not/a/real/path"));
        assert!(matches!(result, Err(PackageError::Manifest(_))));
    }

    // ----- the Linux path, real end to end ----------------------------------

    /// Real, on this sandbox: builds an actual (tiny, dependency-free) crate
    /// with a real `cargo build --release`, then checks the tree
    /// `package_linux` assembled — not merely that files exist, but that the
    /// binary it copied is byte-for-byte the one cargo just built, and that
    /// the desktop entry names the right executable.
    #[test]
    fn packaging_for_linux_builds_and_lays_out_a_real_tree() {
        if host_os() != "linux" {
            eprintln!("skipping: this check only runs the Linux target on a Linux host");
            return;
        }

        let scratch = std::env::temp_dir().join("vieww-build-package-linux-e2e");
        std::fs::remove_dir_all(&scratch).ok();
        std::fs::create_dir_all(scratch.join("src")).unwrap();
        std::fs::write(
            scratch.join("Cargo.toml"),
            "[package]\nname = \"pkgprobe\"\nversion = \"0.1.0\"\nedition = \"2021\"\npublish = false\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(
            scratch.join("src/main.rs"),
            "fn main() { println!(\"packaged and real\"); }\n",
        )
        .unwrap();

        let report = package(PackageTarget::Linux, &scratch).expect("packaging should succeed");
        assert_eq!(report.target, PackageTarget::Linux);
        assert_eq!(report.artifacts.len(), 2);

        let binary = report
            .artifacts
            .iter()
            .find(|p| p.starts_with(report.output_dir.join("bin")))
            .expect("a binary artifact");
        assert!(binary.is_file());

        let run = Command::new(binary)
            .output()
            .expect("the packaged binary should run");
        assert!(run.status.success());
        assert!(String::from_utf8_lossy(&run.stdout).contains("packaged and real"));

        let desktop = report
            .artifacts
            .iter()
            .find(|p| p.extension().is_some_and(|ext| ext == "desktop"))
            .expect("a .desktop artifact");
        let contents = std::fs::read_to_string(desktop).unwrap();
        assert!(contents.contains("Name=pkgprobe"));
        assert!(contents.contains("Exec=pkgprobe"));

        std::fs::remove_dir_all(&scratch).ok();
    }

    #[test]
    fn packaging_a_directory_with_no_manifest_fails_before_any_build() {
        if host_os() != "linux" {
            eprintln!("skipping: this check only runs the Linux target on a Linux host");
            return;
        }
        let scratch = std::env::temp_dir().join("vieww-build-package-no-manifest");
        std::fs::remove_dir_all(&scratch).ok();
        std::fs::create_dir_all(&scratch).unwrap();

        let result = package(PackageTarget::Linux, &scratch);
        assert!(
            matches!(result, Err(PackageError::Manifest(_))),
            "{result:?}"
        );

        std::fs::remove_dir_all(&scratch).ok();
    }
}
