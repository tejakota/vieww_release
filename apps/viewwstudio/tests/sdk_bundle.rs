//! An installed studio finding, checking and using the SDK it ships with.
//!
//! # What is faked, and what is not
//!
//! Only the compiler. Copying a 600 MB sysroot into a temporary directory to
//! assert which path `discover` chose would make this test unrunnable on the
//! machines it most needs to run on, so `bin/rustc` here is a two-line script
//! that prints a version string. Everything else — the manifest, the layout,
//! the rlib naming, the search order — is the real thing, and the assertions
//! are about *which file the studio decided to run* rather than about what
//! came out of it.
//!
//! The version the fake prints is read from the real `rustc` at test time,
//! because `Toolchain::discover` compares it against the compiler that built
//! this test binary. A hard-coded string would make this file expire.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use viewwstudio::compile::{Toolchain, ToolchainError};
use viewwstudio::sdk::{self, Manifest};

/// The real compiler's version — the one this test binary was built by.
fn host_rustc_version() -> String {
    let out = Command::new("rustc")
        .arg("--version")
        .output()
        .expect("rustc on PATH");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A temporary directory that cleans itself up.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("vieww-sdk-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("mkdir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write a `bin/rustc` that reports `version`.
fn fake_toolchain(dir: &Path, version: &str) {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join(sdk::TOOLCHAIN_DIR).join("bin");
    std::fs::create_dir_all(&bin).expect("mkdir");
    let rustc = bin.join("rustc");
    std::fs::write(&rustc, format!("#!/bin/sh\necho '{version}'\n")).expect("write");
    std::fs::set_permissions(&rustc, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

/// A bundle laid out the way `package.sh` lays one out.
///
/// `rlib_names` are created in order, oldest first, and the manifest names
/// **the last argument** — deliberately not the newest, so a test can tell the
/// manifest apart from the mtime ordering it replaces.
fn bundle(dir: &Path, rustc_version: &str, rlib_names: &[&str], named: &str) {
    std::fs::create_dir_all(dir.join("deps")).expect("mkdir");
    for name in rlib_names {
        // **Both places, because `package.sh` puts them in both.** The copy at
        // the root is what `install::is_usable` looks for; the copies under
        // `deps/` are what `vieww_candidates` enumerates and what a compile
        // links transitively. A helper that wrote only one of them would be
        // testing a layout the packaging script never produces.
        std::fs::write(dir.join(name), b"not really an rlib").expect("write");
        std::fs::write(dir.join("deps").join(name), b"not really an rlib").expect("write");
        // Distinct mtimes, so "newest" is well defined rather than a tie.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let manifest = Manifest {
        format: sdk::FORMAT,
        vieww: "0.0.1".into(),
        rustc: rustc_version.to_owned(),
        host: sdk::host_triple().to_owned(),
        rlib: named.to_owned(),
    };
    std::fs::write(dir.join(sdk::MANIFEST), manifest.to_text()).expect("write");
}

#[test]
fn a_packaged_studio_compiles_with_the_compiler_it_ships() {
    // The whole point of option 2. Not "a rustc that matches" — *this* rustc,
    // the one in the bundle, so host and guest are one compilation by
    // construction rather than by the user having installed the right thing.
    let version = host_rustc_version();
    let dir = Dir::new("bundled");
    bundle(
        dir.path(),
        &version,
        &["libvieww-aaaa.rlib"],
        "libvieww-aaaa.rlib",
    );
    fake_toolchain(dir.path(), &version);

    let toolchain = Toolchain::discover(dir.path()).expect("a complete bundle");
    assert_eq!(
        toolchain.rustc,
        dir.path()
            .join(sdk::TOOLCHAIN_DIR)
            .join("bin")
            .join("rustc"),
        "the bundled compiler, not the one on PATH"
    );
    // And it says so, because "guaranteed" and "checked and it happened to be
    // right" are different states and only one of them survives the user
    // changing their `PATH`.
    assert!(toolchain.is_bundled());
    assert!(
        toolchain.stamp().contains("bundled"),
        "the status bar should be able to tell them apart: {}",
        toolchain.stamp()
    );
}

#[test]
fn the_manifest_decides_which_rlib_rather_than_the_modification_time() {
    // `vieww_candidates` orders by mtime because, scavenging a directory it
    // does not own, that is the best signal available. A bundle *knows*, and
    // this is the assertion that knowing wins — the newest rlib here is
    // deliberately not the one the manifest names.
    let version = host_rustc_version();
    let dir = Dir::new("named");
    bundle(
        dir.path(),
        &version,
        &["libvieww-old.rlib", "libvieww-new.rlib"],
        "libvieww-old.rlib",
    );
    fake_toolchain(dir.path(), &version);

    let toolchain = Toolchain::discover(dir.path()).expect("a complete bundle");
    assert_eq!(
        toolchain.vieww_rlib,
        dir.path().join("libvieww-old.rlib"),
        "the manifest names it, so the guessing is over"
    );
    // And the newest is still *reachable*, second, through `with_candidate` —
    // the manifest reorders the list rather than shortening it, so a bundle
    // whose manifest is somehow wrong degrades to the old behaviour instead of
    // to no behaviour.
    assert!(
        toolchain
            .candidates
            .contains(&dir.path().join("deps").join("libvieww-new.rlib")),
        "the other one stays reachable as a fallback: {:?}",
        toolchain.candidates
    );
}

#[test]
fn a_partially_updated_bundle_is_refused_with_the_useful_sentence() {
    // Every file present, every presence check passing, and the compiler is
    // not the one that produced the rlibs beside it. This is what copying a
    // new studio over an old SDK looks like from the inside.
    let dir = Dir::new("stale");
    bundle(
        dir.path(),
        "rustc 1.93.0 (stale 2025-01-01)",
        &["libvieww-aaaa.rlib"],
        "libvieww-aaaa.rlib",
    );
    fake_toolchain(dir.path(), &host_rustc_version());

    let error = Toolchain::discover(dir.path()).expect_err("a damaged bundle");
    match error {
        ToolchainError::Sdk(inner) => {
            let text = inner.to_string();
            assert!(text.contains("1.93.0"), "names what it expected: {text}");
            assert!(
                text.contains("partially updated"),
                "and what to make of it: {text}"
            );
        }
        // Specifically *not* `Mismatch`: that one tells the user to fix their
        // PATH, which is neither the problem nor the fix for a bundle.
        other => panic!("the SDK check has to come first: {other}"),
    }
}

#[test]
fn a_bundle_built_for_another_machine_is_refused_before_the_first_render() {
    let version = host_rustc_version();
    let dir = Dir::new("foreign");
    bundle(
        dir.path(),
        &version,
        &["libvieww-aaaa.rlib"],
        "libvieww-aaaa.rlib",
    );
    fake_toolchain(dir.path(), &version);
    // Rewrite the host to something this machine is not.
    let text = std::fs::read_to_string(dir.path().join(sdk::MANIFEST)).expect("read");
    let foreign = text.replace(sdk::host_triple(), "sparc64-unknown-nothing");
    std::fs::write(dir.path().join(sdk::MANIFEST), foreign).expect("write");

    let error = Toolchain::discover(dir.path()).expect_err("a bundle for another target");
    assert!(
        matches!(error, ToolchainError::Sdk(sdk::SdkError::Host { .. })),
        "an rlib is not portable between targets: {error}"
    );
}

#[test]
fn a_development_checkout_has_no_manifest_and_is_not_punished_for_it() {
    // The configuration every contributor is in. No manifest, no bundled
    // compiler, and `HOST_RUSTC` is what checks that PATH holds the right one
    // — so discovery has to succeed here on its own terms rather than
    // demanding a bundle that a checkout has no reason to contain.
    let dir = Dir::new("checkout");
    std::fs::create_dir_all(dir.path().join("deps")).expect("mkdir");
    std::fs::write(dir.path().join("deps/libvieww-aaaa.rlib"), b"x").expect("write");

    let toolchain = Toolchain::discover(dir.path()).expect("a plain target directory");
    assert_eq!(
        toolchain.rustc,
        PathBuf::from("rustc"),
        "nothing bundled, so PATH it is"
    );
    assert!(!toolchain.is_bundled());
    assert!(
        !toolchain.stamp().contains("bundled"),
        "and it must not claim a guarantee it does not have: {}",
        toolchain.stamp()
    );
}
