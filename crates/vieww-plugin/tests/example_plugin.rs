//! End-to-end: builds a real plugin `cdylib` fixture with a real `cargo
//! build` subprocess, `dlopen`s it for real through [`PluginRegistry`], and
//! calls into it exactly as an embedding host would — the one thing
//! `abi.rs`'s own in-process unit tests structurally cannot cover, since
//! they call the trampolines directly rather than crossing an actual
//! compiled-library boundary. Mirrors `vieww-build`'s own
//! `tests/scaffold_build.rs` in structure and skip discipline: loud, on
//! stderr, and only when this machine genuinely cannot run the check.

use std::path::{Path, PathBuf};
use std::process::Command;

use vieww_plugin::{stderr_host_vtable, PluginError, PluginRegistry};

fn fixture_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/example-plugin/Cargo.toml")
}

fn have_cargo() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// A scratch `--target-dir` this test owns, removed on drop — a dedicated
/// directory out of this workspace's own `target/`, exactly for the reason
/// `vieww-build`'s own scaffold test uses one: this must never share, and
/// never corrupt, the real workspace build.
struct ScratchTargetDir(PathBuf);

impl ScratchTargetDir {
    /// `label` distinguishes concurrently-running tests — the default test
    /// harness runs tests in parallel threads, and two tests racing to
    /// `remove_dir_all` and rebuild the *same* directory is a spurious
    /// failure this fixture, not the code under test, would be causing.
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("vieww-plugin-example-fixture-target-{label}"));
        std::fs::remove_dir_all(&dir).ok();
        Self(dir)
    }
}

impl Drop for ScratchTargetDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Builds the fixture plugin with a real `cargo build` and returns the path
/// to the shared library it produced — whichever extension this platform
/// uses, since the fixture's own manifest does not hardcode one.
fn build_fixture(target_dir: &Path) -> Option<PathBuf> {
    let output = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(fixture_manifest())
        .arg("--target-dir")
        .arg(target_dir)
        .output()
        .expect("cargo should run at all");

    if !output.status.success() {
        eprintln!(
            "fixture plugin build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    let dir = target_dir.join("debug");
    [
        "libvieww_example_plugin.so",
        "libvieww_example_plugin.dylib",
        "vieww_example_plugin.dll",
    ]
    .into_iter()
    .map(|name| dir.join(name))
    .find(|path| path.exists())
}

#[test]
fn a_real_compiled_plugin_loads_and_runs_across_the_actual_cdylib_boundary() {
    if !have_cargo() {
        eprintln!("skipping: no cargo on PATH");
        return;
    }

    let scratch = ScratchTargetDir::new("lifecycle");
    let Some(library_path) = build_fixture(&scratch.0) else {
        panic!("fixture plugin did not produce a shared library at any expected path");
    };

    let mut registry = PluginRegistry::new();
    let host = stderr_host_vtable();

    // SAFETY: `library_path` was just built from `tests/fixtures/example-plugin`,
    // a real `vieww` plugin this same test controls the source of.
    let loaded = unsafe { registry.load(&library_path, &host) }
        .expect("a compatible, well-formed plugin should load");

    assert_eq!(loaded.name(), "vieww-example-plugin");
    assert_eq!(loaded.version(), "0.1.0");
    assert_eq!(loaded.commands().len(), 2);
    assert!(loaded
        .commands()
        .iter()
        .any(|c| c.id == "greet" && c.title == "Say hello"));
    assert!(loaded.commands().iter().any(|c| c.id == "fail"));

    let plugin = &mut registry.plugins_mut()[0];

    assert!(plugin.invoke("greet").is_ok());
    assert!(
        plugin.invoke("greet").is_ok(),
        "state must survive across repeated calls through the real ABI"
    );

    match plugin.invoke("does-not-exist") {
        Err(PluginError::UnknownCommand(id)) => assert_eq!(id, "does-not-exist"),
        other => panic!("expected UnknownCommand, got {other:?}"),
    }

    match plugin.invoke("fail") {
        Err(PluginError::InvokeFailed(id)) => assert_eq!(id, "fail"),
        other => panic!("expected InvokeFailed, got {other:?}"),
    }

    // Exercises the real shutdown/drop sequence through the loaded library —
    // see `LoadedPlugin`'s own `Drop` impl doc for exactly what this runs.
    drop(registry);
}

#[test]
fn loading_the_same_plugin_twice_produces_two_independent_instances() {
    if !have_cargo() {
        eprintln!("skipping: no cargo on PATH");
        return;
    }

    let scratch = ScratchTargetDir::new("twice");
    let Some(library_path) = build_fixture(&scratch.0) else {
        panic!("fixture plugin did not produce a shared library at any expected path");
    };

    let mut registry = PluginRegistry::new();
    let host = stderr_host_vtable();

    unsafe {
        registry
            .load(&library_path, &host)
            .expect("first load should succeed");
        registry
            .load(&library_path, &host)
            .expect("a second, independent load should also succeed");
    }
    assert_eq!(registry.len(), 2);

    // Advance only the first instance's counter; the second must be
    // unaffected, since each `load` produced its own boxed instance.
    registry.plugins_mut()[0].invoke("greet").unwrap();

    assert!(registry.unload(0).is_some());
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.plugins()[0].name(), "vieww-example-plugin");
}
