//! The property `vieww_build::scaffold`'s module docs promise: `vieww new`'s
//! output is not merely rendered as strings that look like Rust, it compiles.
//!
//! `scaffold::files` is the single definition of what `vieww new` writes, so
//! this scaffolds a real temporary directory with the real function and hands
//! it to a real `cargo build` — the same command `vieww build` itself would
//! run on the result. Mirrors `apps/viewwstudio/tests/scaffold.rs` in
//! structure and in its skip discipline: loud, on stderr, and only when this
//! machine genuinely cannot run the check (no checkout to depend on, no
//! `cargo` on `PATH`) rather than silently reporting success for nothing.

use std::path::PathBuf;
use std::process::Command;

use vieww_build::scaffold::{self, Dependency};

/// A scratch directory this test owns, removed on drop so a failed run does
/// not leave a stale scaffold behind for the next one to trip over.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vieww-build-scaffold-build-{name}"));
        std::fs::remove_dir_all(&dir).ok();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn have_cargo() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The template, scaffolded and built with a real `cargo build`.
///
/// This is the whole reason `scaffold` is a separate function from the
/// pure `files` — a fixture that only renders to strings could describe a
/// project that reads fine and does not compile, and that is precisely the
/// class of defect this test exists to catch before a real `vieww new` user
/// does.
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
    scaffold::create(&scratch.0, "vieww_build_scaffold_check", &dependency)
        .expect("scaffold::create should write every file it lists");

    // A dedicated `--target-dir`, out of tree, so this never shares — and
    // never corrupts — this workspace's own `target/`.
    let target_dir = std::env::temp_dir().join("vieww-build-scaffold-build-check-target");

    let output = Command::new("cargo")
        .arg("build")
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&scratch.0)
        .output()
        .expect("cargo should run");

    let ok = output.status.success();
    std::fs::remove_dir_all(&target_dir).ok();

    assert!(
        ok,
        "the scaffolded project failed to build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
