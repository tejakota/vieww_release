//! Plan 2 §5.2's first property: every template file [`scaffold::files`] emits
//! is compiled by a test, not merely rendered as strings that happen to look
//! like Rust.
//!
//! `scaffold::files` is the single definition of what New Project writes, so
//! this is not a second copy of the template to keep in sync — it scaffolds a
//! real temp directory with the real function and runs the real `cargo build`
//! against it, the same command the studio would eventually orchestrate.
//!
//! Skips — loudly, on stderr — when there is no [`Toolchain`]-shaped checkout
//! to depend on or no `cargo` on `PATH`, for the same reason `pipeline.rs`
//! skips: a suite that goes green on a machine that cannot run the pipeline at
//! all is worse than one that says so.

use std::path::PathBuf;
use std::process::Command;

use viewwstudio::scaffold::{self, Dependency};

/// A scratch directory this test owns, removed on drop so a failed run does
/// not leave a stale scaffold for the next one to trip over.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vieww-scaffold-build-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Whether `cargo` runs at all — separate from [`scaffold::checkout_root`],
/// which only checks the two crates a template names are on disk.
fn have_cargo() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The template, scaffolded and built with a real `cargo build`.
///
/// This is the property the plan promises and the one thing about N2 that was
/// claimed and never checked: a fresh project, exactly as New Project would
/// write it, compiles on the first try.
#[test]
fn a_scaffolded_project_builds() {
    let Some(root) = scaffold::checkout_root() else {
        eprintln!("skipping: not running from within the vieww checkout");
        return;
    };
    if !have_cargo() {
        eprintln!("skipping: no cargo on PATH");
        return;
    }

    let scratch = Scratch::new("builds");
    let dependency = Dependency::Path(root);
    scaffold::create(&scratch.0, "scaffold_build_check", &dependency)
        .expect("scaffold::create should write every file it lists");

    // A dedicated `--target-dir`, out of tree, so this never shares — and
    // never corrupts — the workspace's own `target/`.
    let target_dir = std::env::temp_dir().join("vieww-scaffold-build-check-target");

    let output = Command::new("cargo")
        .arg("build")
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&scratch.0)
        .output()
        .expect("cargo should run");

    assert!(
        output.status.success(),
        "the scaffolded project failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::fs::remove_dir_all(&target_dir).ok();
}

/// A **Say** project builds, and the screen in the built binary is the screen
/// in the `.say` file.
///
/// # The property this pins
///
/// `vieww-say-codegen` used to be called from exactly one place — vieww
/// Studio's preview — so a `.say` screen was something you could look at and
/// not something you could ship. `cargo build` produced a real application that
/// did not contain the screen: the scaffolded library mounted a placeholder
/// reading "Open src/screens/home.say and press Render", and that is what the
/// finished, built, runnable application said when you ran it.
///
/// So this compiles the Say template for real and then greps the binary for a
/// string that exists **only** in `home.say` — "Tapped" — and asserts the
/// placeholder's text is *not* there. Building is not enough on its own: the
/// old scaffold built perfectly and shipped the wrong screen.
#[test]
fn a_scaffolded_say_project_builds_and_contains_its_screen() {
    let Some(root) = scaffold::checkout_root() else {
        eprintln!("skipping: not running from within the vieww checkout");
        return;
    };
    if !have_cargo() {
        eprintln!("skipping: no cargo on PATH");
        return;
    }

    let scratch = Scratch::new("say-builds");
    let dependency = Dependency::Path(root);
    scaffold::create_for(
        &scratch.0,
        "say_build_check",
        &dependency,
        scaffold::ProjectKind::Say,
    )
    .expect("scaffold::create_for should write every file it lists");

    let target_dir = std::env::temp_dir().join("vieww-scaffold-say-build-target");

    // `--bin`, the same narrowing the studio's own Build uses: the cdylib and
    // the staticlib are an Android and an iOS artefact and this test wants
    // neither, nor the gigabyte they weigh.
    let output = Command::new("cargo")
        .args(["build", "--bin", "say_build_check", "--target-dir"])
        .arg(&target_dir)
        .current_dir(&scratch.0)
        .output()
        .expect("cargo should run");

    assert!(
        output.status.success(),
        "the scaffolded Say project failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // `.exe` on Windows: cargo names the binary after the host, and reading
    // the extensionless path there failed with "The system cannot find the
    // file specified" against a build that had just succeeded.
    let binary = target_dir.join("debug").join(if cfg!(windows) {
        "say_build_check.exe"
    } else {
        "say_build_check"
    });
    let bytes = std::fs::read(&binary).expect("the built binary should exist");

    assert!(
        contains(&bytes, b"Tapped"),
        "the built binary does not contain the screen's own text — \
         `home.say` was not compiled into it"
    );
    assert!(
        !contains(&bytes, b"press Render"),
        "the built binary still carries the placeholder that told the user to \
         go and press Render in the studio"
    );

    std::fs::remove_dir_all(&target_dir).ok();
}

/// `haystack.contains(needle)`, for bytes.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// [`scaffold::is_vieww_project`] on the directory [`scaffold::create`] just
/// wrote — the studio's own recognition rule applied to its own output, so a
/// template that stopped matching it would light up no build controls at all.
#[test]
fn a_scaffolded_project_is_recognised_as_one() {
    let Some(root) = scaffold::checkout_root() else {
        eprintln!("skipping: not running from within the vieww checkout");
        return;
    };

    let scratch = Scratch::new("recognised");
    let dependency = Dependency::Path(root);
    scaffold::create(&scratch.0, "scaffold_recognise_check", &dependency)
        .expect("scaffold::create should write every file it lists");

    assert!(scaffold::is_vieww_project(&scratch.0));
}
