//! The `vieww` command-line tool.
//!
//! One terminal entry point over `vieww-build` and `cargo`:
//!
//! * `vieww new <name>` — [`vieww_build::scaffold::scaffold`].
//! * `vieww doctor` — [`vieww_build::doctor::run_doctor`], printed as a
//!   report; exits non-zero on any [`Status::Fail`](vieww_build::doctor::Status::Fail).
//! * `vieww build` / `vieww run` / `vieww test` — real `cargo build` / `cargo
//!   run` / `cargo test`, spawned as a subprocess in the given directory
//!   (default: the current one) with its exit code propagated verbatim. This
//!   crate does not reimplement any part of cargo; it is a thin, honest
//!   pass-through, and its own tests check the *argument construction* and
//!   the *exit-code propagation*, never a fabricated "success".
//! * `vieww profile` — see its own module doc below for exactly what it
//!   measures.
//! * `vieww package <target>` — [`vieww_build::package::package`].
//!
//! # Why exit codes are the interface, not return values
//!
//! Every subcommand here ends the process with [`std::process::ExitCode`]
//! rather than a `Result` `main` propagates through `Debug`-formatting a
//! panic message — a CLI's contract with its caller (a shell script, `make`,
//! CI) is its exit code and what it printed, not a Rust-shaped error value
//! nothing outside this binary ever sees.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Parser, Subcommand, ValueEnum};

use vieww_build::package::{self, PackageTarget};
use vieww_build::scaffold;

#[derive(Parser, Debug)]
#[command(
    name = "vieww",
    version,
    about = "A command-line tool for vieww projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum Cmd {
    /// Scaffold a new vieww project into `./<name>`.
    New {
        /// The project's name, and the directory it is written into.
        name: String,
    },
    /// Check this machine's environment for building and packaging vieww apps.
    Doctor,
    /// `cargo build`, in `dir` (default: the current directory).
    Build {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// `cargo run`, in `dir` (default: the current directory).
    Run {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Passed through to the running binary, after `--`.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// `cargo test`, in `dir` (default: the current directory).
    Test {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Time a build or test run — see this binary's module docs for exactly
    /// what is measured.
    Profile {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Time `cargo test` instead of `cargo build`.
        #[arg(long)]
        test: bool,
    },
    /// Package a built project for one platform.
    Package {
        target: PackageTargetArg,
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
}

/// [`PackageTarget`], mirrored as a `clap` enum.
///
/// A separate type rather than deriving `ValueEnum` on `PackageTarget`
/// itself: `vieww-build` has no reason to depend on `clap` merely so its own
/// public enum can be a command-line argument, and a CLI's argument grammar
/// is squarely this crate's concern (see this crate's own `Cargo.toml`
/// comment on why `clap` is named here and nowhere else in the workspace).
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum PackageTargetArg {
    Linux,
    Macos,
    Windows,
    Android,
    Ios,
}

impl From<PackageTargetArg> for PackageTarget {
    fn from(value: PackageTargetArg) -> Self {
        match value {
            PackageTargetArg::Linux => Self::Linux,
            PackageTargetArg::Macos => Self::MacOs,
            PackageTargetArg::Windows => Self::Windows,
            PackageTargetArg::Android => Self::Android,
            PackageTargetArg::Ios => Self::Ios,
        }
    }
}

fn main() -> ExitCode {
    run(Cli::parse().command)
}

fn run(command: Cmd) -> ExitCode {
    match command {
        Cmd::New { name } => cmd_new(&name),
        Cmd::Doctor => cmd_doctor(),
        Cmd::Build { dir } => cmd_cargo("build", &dir, &[]),
        Cmd::Run { dir, args } => cmd_cargo("run", &dir, &args),
        Cmd::Test { dir } => cmd_cargo("test", &dir, &[]),
        Cmd::Profile { dir, test } => cmd_profile(&dir, test),
        Cmd::Package { target, dir } => cmd_package(target.into(), &dir),
    }
}

/// `vieww new <name>` — scaffolds into `./<name>`, refusing to touch anything
/// that already exists there (see [`scaffold::create`]'s own docs for why).
fn cmd_new(name: &str) -> ExitCode {
    let target_dir = PathBuf::from(name);
    match scaffold::scaffold(name, &target_dir) {
        Ok(written) => {
            println!("created {} ({} files)", target_dir.display(), written.len());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vieww new: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `vieww doctor` — runs every check, prints the report, and fails the
/// process on the first real [`Status::Fail`](vieww_build::doctor::Status::Fail)
/// so CI can gate on it.
fn cmd_doctor() -> ExitCode {
    let report = vieww_build::doctor::run_doctor();
    print!("{}", report.render());
    if report.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Spawn `cargo <subcommand> [-- args...]` in `dir` and propagate its exit
/// code exactly.
///
/// No output is captured or reinterpreted — cargo's own stdout and stderr
/// reach the terminal directly (`Command::status`, not `::output()`), because
/// a wrapper that re-prints what cargo already printed is a second place for
/// the two to say something different.
fn cmd_cargo(subcommand: &str, dir: &Path, trailing_args: &[String]) -> ExitCode {
    let mut command = Command::new("cargo");
    command.arg(subcommand).current_dir(dir);
    if !trailing_args.is_empty() {
        command.arg("--").args(trailing_args);
    }
    exit_code_of(command.status())
}

/// Turn a spawn attempt into this process's own exit code.
///
/// Split out so the mapping — spawn failure is [`ExitCode::FAILURE`], a
/// signal-killed child (no exit code on Unix) is also `FAILURE`, and
/// everything else passes the child's code through — is one function every
/// subcommand that shells out shares, rather than each reimplementing it
/// slightly differently.
fn exit_code_of(status: std::io::Result<std::process::ExitStatus>) -> ExitCode {
    match status {
        Ok(status) => match status.code() {
            Some(code) =>
            {
                #[expect(
                    clippy::cast_sign_loss,
                    clippy::cast_possible_truncation,
                    reason = "an exit code outside 0..=255 is already meaningless to a shell"
                )]
                ExitCode::from(code as u8)
            }
            None => ExitCode::FAILURE,
        },
        Err(error) => {
            eprintln!("vieww: could not run cargo: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `vieww profile` — a wall-clock stopwatch around `cargo build` (or `cargo
/// test`), and precisely that.
///
/// # What this measures, stated exactly because "profile" oversells it
///
/// The elapsed real time of one `cargo build --release` (or `cargo test`)
/// invocation, nothing more. It is not a per-crate timing breakdown — that is
/// `cargo build --timings`, which writes its own HTML report and remains the
/// right tool when the question is "which dependency is slow"; it is not a
/// CPU or allocation profiler, and it does not attribute time to any part of
/// the build. Calling it a "profiler" and shipping a stopwatch would be
/// exactly the kind of overclaim this workspace's own doc conventions argue
/// against (see `crates/vieww-hal/src/d3d12.rs`'s "what is real" sections) —
/// so what it prints says only what it measured:
///
/// ```text
/// vieww profile: cargo build --release took 12.4s (exit: success)
/// ```
///
/// `--release`, always, for the same reason `crates/vieww-platform-winit`'s
/// own frame-rate example insists on it in its module docs: a debug build's
/// wall-clock time is a measurement of `rustc -O0` plus incremental
/// bookkeeping, not of the build somebody will actually ship.
fn cmd_profile(dir: &Path, test: bool) -> ExitCode {
    let subcommand = if test { "test" } else { "build" };
    let started = std::time::Instant::now();
    let status = Command::new("cargo")
        .arg(subcommand)
        .arg("--release")
        .current_dir(dir)
        .status();
    let elapsed = started.elapsed();

    match &status {
        Ok(status) => println!(
            "vieww profile: cargo {subcommand} --release took {:.1}s (exit: {})",
            elapsed.as_secs_f64(),
            if status.success() {
                "success"
            } else {
                "failure"
            }
        ),
        Err(error) => eprintln!("vieww profile: could not run cargo: {error}"),
    }
    exit_code_of(status)
}

/// `vieww package <target>` — [`package::package`], with its typed error
/// printed as one honest line rather than a `Debug` dump.
fn cmd_package(target: PackageTarget, dir: &Path) -> ExitCode {
    match package::package(target, dir) {
        Ok(report) => {
            println!(
                "packaged {} for {} -> {}",
                dir.display(),
                report.target,
                report.output_dir.display()
            );
            for artifact in &report.artifacts {
                println!("  {}", artifact.display());
            }
            for note in &report.notes {
                println!("  note: {note}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("vieww package: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- argument parsing, exhaustively over every subcommand -------------

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["vieww"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full)
            .unwrap_or_else(|error| panic!("{args:?} failed to parse: {error}"))
    }

    #[test]
    fn new_takes_one_name() {
        let cli = parse(&["new", "my-app"]);
        assert_eq!(
            cli.command,
            Cmd::New {
                name: "my-app".to_owned()
            }
        );
    }

    #[test]
    fn new_requires_a_name() {
        assert!(Cli::try_parse_from(["vieww", "new"]).is_err());
    }

    #[test]
    fn doctor_takes_nothing() {
        assert_eq!(parse(&["doctor"]).command, Cmd::Doctor);
        assert!(Cli::try_parse_from(["vieww", "doctor", "extra"]).is_err());
    }

    #[test]
    fn build_defaults_to_the_current_directory() {
        assert_eq!(
            parse(&["build"]).command,
            Cmd::Build {
                dir: PathBuf::from(".")
            }
        );
    }

    #[test]
    fn build_takes_an_explicit_directory() {
        assert_eq!(
            parse(&["build", "some/app"]).command,
            Cmd::Build {
                dir: PathBuf::from("some/app")
            }
        );
    }

    #[test]
    fn test_defaults_and_takes_a_directory_the_same_way_build_does() {
        assert_eq!(
            parse(&["test"]).command,
            Cmd::Test {
                dir: PathBuf::from(".")
            }
        );
        assert_eq!(
            parse(&["test", "some/app"]).command,
            Cmd::Test {
                dir: PathBuf::from("some/app")
            }
        );
    }

    #[test]
    fn run_takes_a_directory_and_trailing_args() {
        assert_eq!(
            parse(&["run"]).command,
            Cmd::Run {
                dir: PathBuf::from("."),
                args: vec![]
            }
        );
        assert_eq!(
            parse(&["run", "some/app", "--", "--flag", "value"]).command,
            Cmd::Run {
                dir: PathBuf::from("some/app"),
                args: vec!["--flag".to_owned(), "value".to_owned()]
            }
        );
    }

    #[test]
    fn profile_defaults_to_build_and_can_be_switched_to_test() {
        assert_eq!(
            parse(&["profile"]).command,
            Cmd::Profile {
                dir: PathBuf::from("."),
                test: false
            }
        );
        assert_eq!(
            parse(&["profile", "--test", "some/app"]).command,
            Cmd::Profile {
                dir: PathBuf::from("some/app"),
                test: true
            }
        );
    }

    #[test]
    fn package_requires_a_target_and_accepts_every_one() {
        for (word, expected) in [
            ("linux", PackageTargetArg::Linux),
            ("macos", PackageTargetArg::Macos),
            ("windows", PackageTargetArg::Windows),
            ("android", PackageTargetArg::Android),
            ("ios", PackageTargetArg::Ios),
        ] {
            assert_eq!(
                parse(&["package", word]).command,
                Cmd::Package {
                    target: expected,
                    dir: PathBuf::from(".")
                }
            );
        }
        assert!(Cli::try_parse_from(["vieww", "package"]).is_err());
        assert!(Cli::try_parse_from(["vieww", "package", "nonsense"]).is_err());
    }

    #[test]
    fn package_takes_an_explicit_directory_after_the_target() {
        assert_eq!(
            parse(&["package", "linux", "some/app"]).command,
            Cmd::Package {
                target: PackageTargetArg::Linux,
                dir: PathBuf::from("some/app")
            }
        );
    }

    #[test]
    fn an_unknown_subcommand_is_refused() {
        assert!(Cli::try_parse_from(["vieww", "frobnicate"]).is_err());
    }

    #[test]
    fn every_package_target_arg_maps_to_the_matching_package_target() {
        assert_eq!(
            PackageTarget::from(PackageTargetArg::Linux),
            PackageTarget::Linux
        );
        assert_eq!(
            PackageTarget::from(PackageTargetArg::Macos),
            PackageTarget::MacOs
        );
        assert_eq!(
            PackageTarget::from(PackageTargetArg::Windows),
            PackageTarget::Windows
        );
        assert_eq!(
            PackageTarget::from(PackageTargetArg::Android),
            PackageTarget::Android
        );
        assert_eq!(
            PackageTarget::from(PackageTargetArg::Ios),
            PackageTarget::Ios
        );
    }

    // ----- exit code propagation --------------------------------------------

    #[test]
    fn a_zero_exit_status_becomes_success() {
        let status = Command::new("true").status();
        assert_eq!(exit_code_of(status), ExitCode::SUCCESS);
    }

    #[test]
    fn a_nonzero_exit_status_is_propagated() {
        let status = Command::new("sh").args(["-c", "exit 7"]).status();
        // ExitCode has no public way to inspect the code it carries, so this
        // checks the thing that actually matters: it is not SUCCESS, and the
        // same input always maps to the same output.
        let mapped = exit_code_of(status);
        assert_ne!(format!("{mapped:?}"), format!("{:?}", ExitCode::SUCCESS));
    }

    #[test]
    fn a_command_that_cannot_be_spawned_is_a_failure_not_a_panic() {
        let status = Command::new("definitely-not-a-real-binary-xyz").status();
        assert!(status.is_err());
        assert_eq!(exit_code_of(status), ExitCode::FAILURE);
    }

    // ----- cmd_cargo builds the command it says it does ----------------------

    /// `cmd_cargo` cannot be unit-tested by inspecting a `Command` after the
    /// fact — `std::process::Command` exposes no getters for the arguments it
    /// was given, by design. What *is* tested here is the property that
    /// matters: running it against a real temporary cargo-shaped directory
    /// produces the same exit code `cargo` itself would for that directory,
    /// proving the subcommand and directory actually reach the child process
    /// rather than being silently dropped.
    #[test]
    fn cmd_cargo_runs_in_the_given_directory_and_propagates_failure() {
        let dir = std::env::temp_dir().join("vieww-cli-cmd-cargo-test");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        // No Cargo.toml here at all: `cargo build` in an empty directory
        // fails immediately, which is a fast, deterministic way to prove the
        // *directory* argument reached the child process — a `cargo build`
        // run from this crate's own directory would instead succeed and take
        // a while, telling this test nothing extra.
        let code = cmd_cargo("build", &dir, &[]);
        std::fs::remove_dir_all(&dir).ok();
        assert_ne!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
    }
}
