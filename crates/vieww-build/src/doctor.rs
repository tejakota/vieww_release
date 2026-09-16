//! "Can a build here even be attempted" — asked of the machine, not assumed.
//!
//! # Why this exists
//!
//! `vieww new my-app && cd my-app && vieww build` is the whole promise of the
//! `vieww` CLI, and every step of it can fail for a reason that has nothing to
//! do with the project: no `rustc`, a toolchain that is not what
//! `rust-toolchain.toml` pins, no display to preview against, no packaging
//! tool for the platform someone is about to ask for. Cargo reports most of
//! these eventually, several minutes into a build, in an error about a crate
//! nobody wrote. `vieww doctor` asks the same questions up front, in one pass,
//! and says which of them is the reason before a build gets the chance to fail
//! confusingly instead.
//!
//! # Every check is a real probe
//!
//! [`run_doctor`] shells out to the real `rustc`/`cargo` on `PATH`
//! ([`std::process::Command`], exactly as [`vieww_hardware`]'s own probes do
//! for `adb` and `xcrun`), reads the real `RUSTUP_TOOLCHAIN` environment
//! variable, and reuses [`vieww_hardware::Capability::Display`] — the same
//! probe the rest of the workspace gates its window-opening tests on — rather
//! than inventing a second, drifting opinion about whether this machine has a
//! screen. A [`DoctorCheck`] is never fabricated: every one of them is the
//! direct result of running something and looking at what came back.
//!
//! # What is deliberately not checked here: the GPU
//!
//! `vieww_hardware::Capability::Gpu` exists and would be the honest way to
//! probe for a Vulkan-capable adapter, and it is not used. That capability
//! only exists behind `vieww-hardware`'s `gpu` feature, which pulls in
//! `vieww-hal` and, through it, `ash` and `naga` — a real Vulkan instance and
//! device get created just to answer the doctor's question. A build-health
//! check that a user runs before every `cargo build` has no business dragging
//! a graphics stack into its own compile graph to answer "is there a display
//! adapter", so this crate depends on `vieww-hardware` with no features at
//! all (see its `Cargo.toml`) and the doctor simply does not have a GPU line.
//! `vieww run` will still fail loudly on a machine with no adapter — it is
//! `vieww-platform-winit`'s own `VulkanDevice::new` that discovers that, at
//! the point it actually matters.
//!
//! # Pass, Warn, Fail — and why there are three
//!
//! A boolean would collapse "this will not compile" and "this will compile
//! but not preview" into one signal, which is exactly the distinction a
//! doctor exists to draw. [`Status::Fail`] is reserved for what stops `cargo
//! build` itself — no `rustc`, no `cargo`. Everything that only narrows what
//! is *possible* — no display, a packaging tool not on `PATH` — is
//! [`Status::Warn`]: still worth printing, never worth an exit code, because a
//! headless CI box that only ever runs `cargo test` is a legitimate machine to
//! run this doctor on.

use std::process::Command;

use crate::util::{host_os, tool_on_path};

/// The environment variable this workspace's own tooling pins its compiler
/// with. Not required — `rustup`'s own toolchain resolution is a fine
/// default — but its presence or absence is worth a line, because a build run
/// under a different `RUSTUP_TOOLCHAIN` than the one `rust-toolchain.toml`
/// pins is a real, previously-seen source of "works on my machine".
const RUSTUP_TOOLCHAIN_VAR: &str = "RUSTUP_TOOLCHAIN";

/// How serious a [`DoctorCheck`] is.
///
/// Ordered so a report can be sorted worst-first if a caller wants that, and
/// `Ord`/`PartialOrd` are derived in declaration order — `Fail` is not first
/// by coincidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Everything this check looked at was fine.
    Pass,
    /// Not fatal, but worth a person's attention — a missing display, a
    /// packaging tool that is not on `PATH`.
    Warn,
    /// Whatever this checks is required for a build to even start.
    Fail,
}

impl Status {
    /// A short, fixed-width mark for printing a report as a table.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

/// One question asked of the machine, and what it answered.
#[derive(Debug, Clone)]
pub struct DoctorCheck {
    /// What this check is about, e.g. `"rustc"` or `"packaging: dpkg-deb"`.
    pub name: String,
    pub status: Status,
    /// What was actually seen — a version line, an error message, a reason a
    /// capability was absent. Never empty: a status with no detail is a
    /// status nobody can act on.
    pub detail: String,
}

impl DoctorCheck {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Pass,
            detail: detail.into(),
        }
    }

    fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            detail: detail.into(),
        }
    }

    fn with_status(name: impl Into<String>, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

/// Every check `vieww doctor` ran, in the order it ran them.
#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Whether anything here should stop a CI job.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|check| check.status == Status::Fail)
    }

    /// One line per check, aligned, for a terminal.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for check in &self.checks {
            out.push_str(&format!(
                "[{}] {:<28} {}\n",
                check.status.glyph(),
                check.name,
                check.detail
            ));
        }
        out
    }
}

/// Run every check this crate knows and collect the results.
///
/// Real, not simulated: this runs `rustc --version`, `cargo --version`, reads
/// `RUSTUP_TOOLCHAIN`, probes for a display through `vieww-hardware`, and
/// looks for the packaging tools relevant to the platform actually running it.
/// It cannot fail — a probe that could not run is itself a [`Status::Fail`]
/// check with the reason in its detail, never a `Result::Err`, because a
/// doctor that panics or bails out on its first missing tool has told the
/// caller nothing about the other nine.
#[must_use]
pub fn run_doctor() -> DoctorReport {
    let mut checks = vec![
        check_tool_version("rustc", "rustc", &["--version"], Status::Fail),
        check_tool_version("cargo", "cargo", &["--version"], Status::Fail),
        check_rustup_toolchain(std::env::var(RUSTUP_TOOLCHAIN_VAR).ok()),
        check_display(),
    ];
    checks.extend(check_packaging_tools(host_os()));

    DoctorReport { checks }
}

/// Run `binary args...` and turn its first line of output into a check.
///
/// Shared by the `rustc` and `cargo` checks, which differ only in what they
/// run — everything about *judging* the result (did it run, did it exit
/// zero, did the first line look like the tool that was asked for) is one
/// function so the two cannot silently start disagreeing about what "found"
/// means.
fn check_tool_version(
    name: &str,
    binary: &str,
    args: &[&str],
    absent_status: Status,
) -> DoctorCheck {
    match Command::new(binary).args(args).output() {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            match first_line_starting_with(&stdout, binary) {
                Some(line) => DoctorCheck::pass(name, line),
                None => DoctorCheck::warn(
                    name,
                    format!(
                        "`{binary} {}` exited 0 but did not say `{binary} ...`: {:?}",
                        args.join(" "),
                        stdout.trim()
                    ),
                ),
            }
        }
        Ok(output) => DoctorCheck::with_status(
            name,
            absent_status,
            format!(
                "`{binary} {}` exited {}: {}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ),
        Err(error) => DoctorCheck::with_status(
            name,
            absent_status,
            format!("`{binary}` could not be run: {error}"),
        ),
    }
}

/// The parsing half of [`check_tool_version`], pulled out so it can be tested
/// against fixed strings instead of whatever `rustc`/`cargo` happen to print
/// on the machine running the suite.
///
/// Returns the first line, trimmed, if it starts with `tool` — which is the
/// one thing this crate actually relies on (`rustc 1.85.0 (...)`, `cargo
/// 1.85.0 (...)`); anything else is treated as output this crate does not
/// understand rather than parsed further, because a version doctor that
/// mis-parses a compiler's own banner and reports a wrong version is worse
/// than one that admits it did not recognise the format.
fn first_line_starting_with<'a>(output: &'a str, tool: &str) -> Option<&'a str> {
    let line = output.lines().next()?.trim();
    line.starts_with(tool).then_some(line)
}

/// Whether `RUSTUP_TOOLCHAIN` is set, and to what.
///
/// A pure function of the value the real check reads, for the same reason
/// [`first_line_starting_with`] is split out: the decision is one `match`, and
/// it should be testable without touching this process's actual environment.
fn check_rustup_toolchain(value: Option<String>) -> DoctorCheck {
    match value {
        Some(toolchain) if !toolchain.trim().is_empty() => DoctorCheck::pass(
            "toolchain pin",
            format!("{RUSTUP_TOOLCHAIN_VAR}={toolchain}"),
        ),
        _ => DoctorCheck::warn(
            "toolchain pin",
            format!(
                "{RUSTUP_TOOLCHAIN_VAR} is not set — cargo will use rustup's own default \
                 toolchain resolution, which may not be the one this workspace's \
                 rust-toolchain.toml pins"
            ),
        ),
    }
}

/// Whether there is somewhere to put a window, via the same probe the rest of
/// the workspace already gates its window tests on.
///
/// [`Status::Warn`], never [`Status::Fail`]: `vieww build` and `vieww test`
/// need no display at all, and a doctor that failed a headless CI box for not
/// having a screen would be wrong about what a screen is for.
fn check_display() -> DoctorCheck {
    let availability = vieww_hardware::Capability::Display.probe();
    if availability.is_present() {
        DoctorCheck::pass("display", availability.reason().to_owned())
    } else {
        DoctorCheck::warn("display", availability.reason().to_owned())
    }
}

/// The packaging tools relevant to `host` — never all of them, because
/// reporting "no `wix`" on a Linux box tells a Linux user nothing they can or
/// need to act on, and would bury the one packaging warning they can.
///
/// Split from [`run_doctor`] and parametrised on the host string so the
/// per-platform *selection* is testable on every machine this suite runs on,
/// not only on the three it happens to have.
fn check_packaging_tools(host: &str) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    match host {
        "linux" => checks.push(packaging_tool_check(
            "packaging: dpkg-deb",
            "dpkg-deb",
            "a .deb bundle (apt install dpkg-dev)",
        )),
        "macos" => {
            checks.push(packaging_tool_check(
                "packaging: iconutil",
                "iconutil",
                "an .icns app icon (ships with Xcode's command line tools)",
            ));
            checks.push(packaging_tool_check(
                "packaging: hdiutil",
                "hdiutil",
                "a .dmg (present on every macOS install)",
            ));
        }
        "windows" => checks.push(packaging_tool_check(
            "packaging: wix",
            "wix",
            "an .msi (dotnet tool install --global wix)",
        )),
        other => checks.push(DoctorCheck::warn(
            "packaging",
            format!("no packaging tool checks are defined for host `{other}`"),
        )),
    }
    // Cross-platform regardless of host: an Android package needs `adb` and a
    // macOS host is the only one that can produce an iOS one at all, but
    // *checking* for either tool costs nothing on any host, and a Linux
    // machine that already has `adb` for device testing (`ci/mobile/adb-runner.sh`)
    // gets to see that here too.
    checks.push(packaging_tool_check(
        "packaging: adb (Android)",
        "adb",
        "installing an APK (part of the Android SDK platform-tools)",
    ));
    checks.push(packaging_tool_check(
        "packaging: xcrun (iOS, macOS only)",
        "xcrun",
        "building or signing an iOS app (ships with Xcode)",
    ));
    checks
}

fn packaging_tool_check(name: &str, binary: &str, needed_for: &str) -> DoctorCheck {
    if tool_on_path(binary) {
        DoctorCheck::pass(name, format!("`{binary}` is on PATH"))
    } else {
        DoctorCheck::warn(
            name,
            format!("no `{binary}` on PATH — needed for {needed_for}"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- the parsing this crate actually relies on -----------------------

    #[test]
    fn a_real_looking_rustc_banner_is_recognised() {
        let banner = "rustc 1.85.0 (4d91de4e4 2025-02-17)\n";
        assert_eq!(
            first_line_starting_with(banner, "rustc"),
            Some("rustc 1.85.0 (4d91de4e4 2025-02-17)")
        );
    }

    #[test]
    fn a_real_looking_cargo_banner_is_recognised() {
        let banner = "cargo 1.85.0 (d73d2caf9 2025-02-24)\n";
        assert_eq!(
            first_line_starting_with(banner, "cargo"),
            Some("cargo 1.85.0 (d73d2caf9 2025-02-24)")
        );
    }

    #[test]
    fn output_that_does_not_start_with_the_tool_name_is_rejected() {
        // This is what a shim, an alias gone wrong, or a completely different
        // program on PATH under the same name would print, and a doctor that
        // reported a version out of it would be reporting a lie.
        assert_eq!(
            first_line_starting_with("command not found\n", "rustc"),
            None
        );
        assert_eq!(first_line_starting_with("", "rustc"), None);
    }

    #[test]
    fn check_tool_version_is_a_pass_only_when_the_command_actually_ran() {
        let check = check_tool_version("rustc", "rustc", &["--version"], Status::Fail);
        // Not asserting the version — only that *this machine's* real rustc,
        // which every other test in this workspace also depends on, is
        // recognised by the same parsing the doctor ships.
        assert_eq!(check.status, Status::Pass, "{check:?}");
        assert!(check.detail.starts_with("rustc"), "{check:?}");
    }

    #[test]
    fn a_binary_that_does_not_exist_is_the_configured_absent_status() {
        let check = check_tool_version(
            "definitely-not-a-real-binary",
            "definitely-not-a-real-binary-xyz",
            &["--version"],
            Status::Fail,
        );
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("could not be run"), "{check:?}");
    }

    // ----- the toolchain pin -------------------------------------------------

    #[test]
    fn a_set_toolchain_is_a_pass_naming_it() {
        let check = check_rustup_toolchain(Some("stable-x86_64-unknown-linux-gnu".to_owned()));
        assert_eq!(check.status, Status::Pass);
        assert!(check.detail.contains("stable-x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn an_unset_toolchain_is_a_warning_not_a_failure() {
        assert_eq!(check_rustup_toolchain(None).status, Status::Warn);
    }

    #[test]
    fn an_empty_toolchain_variable_is_treated_as_unset() {
        // `RUSTUP_TOOLCHAIN=` is what a shell leaves behind when something
        // unset it badly — the same edge case `vieww-hardware`'s own
        // `DISPLAY=` check exists for, and for the same reason: an empty
        // string is not a toolchain name.
        assert_eq!(
            check_rustup_toolchain(Some(String::new())).status,
            Status::Warn
        );
        assert_eq!(
            check_rustup_toolchain(Some("   ".to_owned())).status,
            Status::Warn
        );
    }

    // ----- the display check reuses vieww-hardware, and only reuses it -----

    #[test]
    fn the_display_check_never_fails_a_build() {
        // Whatever this machine's display situation actually is, the doctor
        // must never treat its absence as fatal — see the module docs.
        assert_ne!(check_display().status, Status::Fail);
    }

    // ----- which packaging tools get checked, per host ----------------------

    #[test]
    fn linux_only_checks_the_linux_tool_plus_the_cross_platform_ones() {
        let names: Vec<String> = check_packaging_tools("linux")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.iter().any(|n| n == "packaging: dpkg-deb"));
        assert!(!names.iter().any(|n| n.contains("wix")));
        assert!(!names.iter().any(|n| n.contains("iconutil")));
        assert!(names.iter().any(|n| n.contains("adb")));
        assert!(names.iter().any(|n| n.contains("xcrun")));
    }

    #[test]
    fn macos_checks_iconutil_and_hdiutil_and_not_wix() {
        let names: Vec<String> = check_packaging_tools("macos")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.iter().any(|n| n.contains("iconutil")));
        assert!(names.iter().any(|n| n.contains("hdiutil")));
        assert!(!names.iter().any(|n| n.contains("dpkg-deb")));
        assert!(!names.iter().any(|n| n.contains("wix")));
    }

    #[test]
    fn windows_checks_wix_and_not_the_unix_tools() {
        let names: Vec<String> = check_packaging_tools("windows")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert!(names.iter().any(|n| n.contains("wix")));
        assert!(!names.iter().any(|n| n.contains("dpkg-deb")));
        assert!(!names.iter().any(|n| n.contains("iconutil")));
    }

    #[test]
    fn an_unknown_host_still_gets_the_cross_platform_checks() {
        let checks = check_packaging_tools("plan9");
        assert!(checks.iter().any(|c| c.name == "packaging"));
        assert!(checks.iter().any(|c| c.name.contains("adb")));
    }

    #[test]
    fn a_missing_packaging_tool_is_a_warning_never_a_failure() {
        let check = packaging_tool_check(
            "packaging: definitely-not-installed",
            "definitely-not-a-real-packaging-tool-xyz",
            "nothing real",
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check
            .detail
            .contains("no `definitely-not-a-real-packaging-tool-xyz`"));
    }

    // ----- the whole report, run for real ------------------------------------

    /// Not an assertion about what this particular machine has — that is the
    /// whole point of a doctor, and pinning it here would make the suite fail
    /// on the next machine that has a different answer. What is pinned: the
    /// report is never empty, it never panics getting there, and a report
    /// with no `Fail` in it says so.
    #[test]
    fn run_doctor_returns_a_real_non_empty_report_and_does_not_panic() {
        let report = run_doctor();
        assert!(!report.checks.is_empty());
        for check in &report.checks {
            assert!(!check.detail.is_empty(), "{check:?} has no detail");
        }
        // This machine — the one this suite is running on — has a real cargo
        // and a real rustc, because `cargo test` itself needed both to get
        // this far. If either came back Fail, the doctor's own probe would be
        // disagreeing with the fact of its own existence.
        let rustc = report.checks.iter().find(|c| c.name == "rustc").unwrap();
        let cargo = report.checks.iter().find(|c| c.name == "cargo").unwrap();
        assert_eq!(rustc.status, Status::Pass, "{rustc:?}");
        assert_eq!(cargo.status, Status::Pass, "{cargo:?}");

        let rendered = report.render();
        assert!(rendered.contains("rustc"));
        assert!(!report.has_failures(), "{rendered}");
    }
}
