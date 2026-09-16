//! Where an installed studio finds the things it needs to compile a preview.
//!
//! # The finding this closes
//!
//! The preview compiles the user's screen against `libvieww.rlib`, and the
//! only place the studio looked for it was **three directories above
//! `CARGO_MANIFEST_DIR`** — the `target/` of the framework checkout it was
//! built in. Copy the binary anywhere else and `Toolchain::discover` failed;
//! the studio opened with the Render button explaining itself, which is
//! graceful but still means the central feature is off.
//!
//! So every user needed a Rust toolchain, a clone of the vieww repository, a
//! completed build of it, and the right environment variable. That is a
//! contributor's setup, not a user's, and it is why the readiness audit put
//! "there is no way to give this to anyone" above everything else.
//!
//! # The chain, and why it is in this order
//!
//! 1. **`VIEWWSTUDIO_TARGET_DIR`.** An explicit answer always wins. This is
//!    what a contributor working on two checkouts at once sets, and what the
//!    test harness sets.
//! 2. **Beside the executable.** The layouts a *packaged* studio has: a
//!    `lib/vieww` sibling on Linux and Windows, and `../Resources/vieww` inside
//!    a macOS `.app`. This is the case that did not exist before.
//! 3. **The development checkout.** `target/debug` or `target/release` three
//!    levels up. Last, not first — a packaged studio must not pick up a stale
//!    build tree that happens to be on the machine.
//!
//! Each candidate is checked for `libvieww.rlib` before being accepted, so a
//! directory that exists but holds nothing useful does not shadow the one that
//! does. The first that actually contains the rlib wins, and if none does the
//! last candidate is returned anyway — so the error the user sees names the
//! place they would most likely put it rather than an empty `Option`.
//!
//! # This module answers "where", not "is it any good"
//!
//! [`is_usable`] asks whether a directory holds an rlib and a `deps/`, which
//! was the only question available when the studio was scavenging a target
//! directory it did not own. A packaged bundle can answer a much better one,
//! and [`crate::sdk::Manifest`] is where that happens: which `vieww`, built by
//! which `rustc`, for which target, and which of the several rlibs present is
//! the one the studio binary actually linked.
//!
//! The split is deliberate. Finding a directory has to keep working for a
//! development checkout, which has no manifest and never will; verifying one
//! only makes sense where there is a claim to check. So a missing manifest is
//! not a failure here, and a *wrong* one is a failure in `compile::Toolchain`.

use std::path::{Path, PathBuf};

/// The environment variable that overrides everything.
pub const TARGET_DIR_ENV: &str = "VIEWWSTUDIO_TARGET_DIR";

/// The directory name a packaged studio keeps its rlibs in.
pub const BUNDLED_DIR: &str = "vieww";

/// Whether `dir` holds a `vieww` rlib and the `deps` a compile needs.
///
/// Both, not either: a `deps` with no `libvieww.rlib` is a target directory for
/// some other crate, and a `libvieww.rlib` with no `deps` cannot be linked
/// against. Accepting either one would mean the studio reports "found a
/// toolchain" and then fails at the first Render.
#[must_use]
pub fn is_usable(dir: &Path) -> bool {
    if !dir.join("deps").is_dir() {
        return false;
    }
    // The rlib is named with a metadata hash in a real target directory, so
    // this looks for the prefix rather than an exact name — the same shape
    // `compile::vieww_candidates` searches for.
    if dir.join("libvieww.rlib").is_file() {
        return true;
    }
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("libvieww") && name.ends_with(".rlib"))
        })
    })
}

/// Every place worth looking, best first.
///
/// Pure apart from reading the environment: `exe` and `manifest` are passed in
/// so the ordering can be tested without a packaged build to test it against.
#[must_use]
pub fn candidates(exe: Option<&Path>, manifest: &Path, release: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(explicit) = std::env::var_os(TARGET_DIR_ENV) {
        out.push(PathBuf::from(explicit));
    }

    if let Some(dir) = exe.and_then(Path::parent) {
        // Linux and Windows: `bin/viewwstudio` with `lib/vieww` beside it, and
        // the flat layout where everything sits in one directory.
        out.push(dir.join(BUNDLED_DIR));
        if let Some(prefix) = dir.parent() {
            out.push(prefix.join("lib").join(BUNDLED_DIR));
            // macOS: `Foo.app/Contents/MacOS/viewwstudio` with the rlibs in
            // `Foo.app/Contents/Resources/vieww`.
            out.push(prefix.join("Resources").join(BUNDLED_DIR));
        }
    }

    // The development checkout, last.
    let profile = if release { "release" } else { "debug" };
    out.push(manifest.join("../../target").join(profile));

    out
}

/// The first candidate that actually holds a `vieww` rlib, or the last one.
///
/// Returning the last rather than `None` is deliberate: the caller turns this
/// into an error message, and "no rlibs in `/usr/lib/vieww`" is a sentence
/// somebody can act on, while "no toolchain" is not.
#[must_use]
pub fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chain = candidates(exe.as_deref(), &manifest, !cfg!(debug_assertions));
    chain
        .iter()
        .find(|dir| is_usable(dir))
        .cloned()
        .or_else(|| chain.last().cloned())
        .unwrap_or_else(|| manifest.join("../../target/debug"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(exe: &str) -> Vec<PathBuf> {
        candidates(
            Some(Path::new(exe)),
            Path::new("/src/apps/viewwstudio"),
            false,
        )
    }

    /// The layout that did not exist before: a studio installed under a prefix.
    #[test]
    fn a_unix_install_looks_beside_and_above_the_binary() {
        let chain = chain("/usr/local/bin/viewwstudio");
        assert!(chain.contains(&PathBuf::from("/usr/local/bin/vieww")));
        assert!(chain.contains(&PathBuf::from("/usr/local/lib/vieww")));
    }

    #[test]
    fn a_mac_app_bundle_looks_in_its_resources() {
        let chain = chain("/Applications/vieww Studio.app/Contents/MacOS/viewwstudio");
        assert!(chain.contains(&PathBuf::from(
            "/Applications/vieww Studio.app/Contents/Resources/vieww"
        )));
    }

    /// A packaged studio must not pick up whatever build tree happens to be on
    /// the machine, so the checkout is looked at last.
    #[test]
    fn the_development_checkout_is_the_last_resort() {
        let chain = chain("/usr/local/bin/viewwstudio");
        let last = chain.last().expect("never empty");
        assert!(
            last.to_string_lossy().contains("target"),
            "the checkout is last: {chain:?}"
        );
    }

    #[test]
    fn there_is_always_at_least_one_candidate_even_with_no_executable() {
        let chain = candidates(None, Path::new("/src/apps/viewwstudio"), false);
        assert!(!chain.is_empty());
    }

    #[test]
    fn release_and_debug_checkouts_are_different_directories() {
        let debug = candidates(None, Path::new("/s"), false);
        let release = candidates(None, Path::new("/s"), true);
        assert_ne!(debug.last(), release.last());
    }

    /// Both halves, not either: a `deps` with no rlib is some other crate's
    /// target directory, and accepting it means "found a toolchain" followed by
    /// a failure at the first Render.
    #[test]
    fn a_directory_is_only_usable_with_both_the_rlib_and_its_deps() {
        let base = std::env::temp_dir().join(format!("viewwstudio-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);

        let no_deps = base.join("no-deps");
        std::fs::create_dir_all(&no_deps).expect("mkdir");
        std::fs::write(no_deps.join("libvieww.rlib"), b"x").expect("write");
        assert!(
            !is_usable(&no_deps),
            "an rlib with no deps cannot be linked"
        );

        let no_rlib = base.join("no-rlib");
        std::fs::create_dir_all(no_rlib.join("deps")).expect("mkdir");
        assert!(
            !is_usable(&no_rlib),
            "deps alone is somebody else's target dir"
        );

        let good = base.join("good");
        std::fs::create_dir_all(good.join("deps")).expect("mkdir");
        std::fs::write(good.join("libvieww.rlib"), b"x").expect("write");
        assert!(is_usable(&good));

        // And the hashed name a real build produces.
        let hashed = base.join("hashed");
        std::fs::create_dir_all(hashed.join("deps")).expect("mkdir");
        std::fs::write(hashed.join("libvieww-3f2a91cc.rlib"), b"x").expect("write");
        assert!(is_usable(&hashed));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_missing_directory_is_not_usable_and_does_not_panic() {
        assert!(!is_usable(Path::new("/definitely/not/here/at/all")));
    }
}
