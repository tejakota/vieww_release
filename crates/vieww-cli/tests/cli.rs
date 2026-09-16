//! True end-to-end tests: the compiled `vieww` binary, run as a real
//! subprocess via `env!("CARGO_BIN_EXE_vieww")` — the standard pattern for
//! testing a binary crate from outside its own process, and the only way to
//! prove `main`'s wiring (argument parsing through to exit code) is correct
//! rather than merely the functions `src/main.rs`'s own unit tests call
//! directly.

use std::path::PathBuf;
use std::process::Command;

fn vieww() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vieww"))
}

/// `vieww doctor` on the real machine running this suite. Not asserting a
/// clean bill of health — a CI box may genuinely have no display — only that
/// the binary runs to completion, prints something recognisable, and exits
/// the way its own report says it should: zero unless a real `Fail` is in
/// the report cargo test itself would have already hit (no `rustc`/`cargo`),
/// which cannot be true of the machine currently running `cargo test`.
#[test]
fn doctor_runs_for_real_and_reports_a_clean_toolchain() {
    let output = vieww().arg("doctor").output().expect("vieww should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("rustc"), "{stdout}");
    assert!(stdout.contains("cargo"), "{stdout}");
    assert!(
        output.status.success(),
        "doctor should exit zero on the machine that just built and is running it:\n{stdout}"
    );
}

/// `vieww new` on a real temporary directory, exercising the full path from
/// argument parsing through `vieww_build::scaffold` to files actually landing
/// on disk — the same round trip a person typing `vieww new my-app` gets.
#[test]
fn new_scaffolds_a_real_project_on_disk() {
    let scratch = std::env::temp_dir().join("vieww-cli-e2e-new");
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();

    let project = scratch.join("greeter");
    let output = vieww()
        .arg("new")
        .arg("greeter")
        .current_dir(&scratch)
        .output()
        .expect("vieww should run");

    assert!(
        output.status.success(),
        "vieww new failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project.join("Cargo.toml").is_file());
    assert!(project.join("src/main.rs").is_file());

    let manifest = std::fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(manifest.contains(r#"name = "greeter""#), "{manifest}");

    std::fs::remove_dir_all(&scratch).ok();
}

/// `vieww new` into a directory that already has something in it is refused,
/// with a non-zero exit and nothing overwritten — the CLI-level version of
/// `scaffold::create`'s own `NotEmpty` guarantee.
#[test]
fn new_refuses_a_directory_that_already_has_something_in_it() {
    let scratch = std::env::temp_dir().join("vieww-cli-e2e-new-not-empty");
    std::fs::remove_dir_all(&scratch).ok();
    let project = scratch.join("taken");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("keep-me.txt"), b"do not lose this").unwrap();

    let output = vieww()
        .arg("new")
        .arg("taken")
        .current_dir(&scratch)
        .output()
        .expect("vieww should run");

    assert!(!output.status.success());
    assert!(
        project.join("keep-me.txt").is_file(),
        "existing file must survive"
    );
    assert!(
        !project.join("Cargo.toml").exists(),
        "nothing scaffolded on refusal"
    );

    std::fs::remove_dir_all(&scratch).ok();
}

/// An unrecognised subcommand is `clap`'s own usage error, not a panic —
/// this pins the exit code shape a shell script would actually see.
#[test]
fn an_unknown_subcommand_exits_nonzero_with_a_usage_message() {
    let output = vieww().arg("not-a-real-subcommand").output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty());
}

/// `vieww package` on a target this sandbox cannot build for exits non-zero
/// and names the reason on stderr — never a panic, and never a silent
/// success for a bundle that was not actually produced.
#[test]
fn package_for_an_unreachable_target_fails_with_a_named_reason() {
    let scratch = std::env::temp_dir().join("vieww-cli-e2e-package");
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();

    // Whichever desktop target this host is not, it cannot package for.
    let unreachable = if cfg!(target_os = "macos") {
        "windows"
    } else {
        "macos"
    };

    let output = vieww()
        .arg("package")
        .arg(unreachable)
        .arg(&scratch)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(unreachable), "{stderr}");

    std::fs::remove_dir_all(&scratch).ok();
}

/// Not run by default, and deliberately so: this is the one test in the
/// suite that pays for a real `cargo build` of a full vieww app (the
/// scaffolded project depends on `vieww` and `vieww-platform-winit`, which
/// this crate's own `tests/scaffold_build.rs` sibling in `vieww-build`
/// already exercises). Kept here, `#[ignore]`d, as the complete round trip
/// through the actual `vieww` binary — `new` then `build` — for a person to
/// run by hand with `cargo test -p vieww-cli -- --ignored` when touching
/// either subcommand's wiring.
#[test]
#[ignore = "compiles a full scaffolded vieww app; run explicitly with --ignored"]
fn new_then_build_produces_a_working_binary() {
    let scratch = std::env::temp_dir().join("vieww-cli-e2e-new-then-build");
    std::fs::remove_dir_all(&scratch).ok();
    std::fs::create_dir_all(&scratch).unwrap();

    let new_output = vieww()
        .arg("new")
        .arg("roundtrip")
        .current_dir(&scratch)
        .output()
        .unwrap();
    assert!(new_output.status.success());

    let project: PathBuf = scratch.join("roundtrip");
    let build_output = vieww().arg("build").arg(&project).output().unwrap();
    assert!(
        build_output.status.success(),
        "{}",
        String::from_utf8_lossy(&build_output.stderr)
    );

    std::fs::remove_dir_all(&scratch).ok();
}
