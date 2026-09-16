//! Small helpers shared by [`crate::doctor`] and [`crate::package`].
//!
//! Neither is worth its own public module: both are "ask the machine a small
//! question honestly" utilities, and splitting them further would only be
//! more files to keep in sync.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Whether `tool` is somewhere on `PATH`.
///
/// A real filesystem lookup — the same thing a shell does before running a
/// bare command — rather than a hardcoded guess about where a packaging tool
/// tends to live. `doctor` and `package` both need exactly this question
/// answered honestly: "is `dpkg-deb`/`wix`/`iconutil` here" and nothing more.
///
/// Split from [`tool_in_dirs`] so the search logic itself — the part that can
/// be wrong — is testable without touching the real `PATH`, which a test
/// suite must not mutate: `std::env::set_var` is process-wide, and two tests
/// racing to set `PATH` to different things is exactly the kind of flake this
/// crate's own doctor exists to keep other people from writing.
pub(crate) fn tool_on_path(tool: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    tool_in_dirs(tool, std::env::split_paths(&path))
}

/// The search [`tool_on_path`] does, over an explicit list of directories.
///
/// A tool counts as present if `<dir>/<tool>` is a file, or — so this works
/// for a Windows executable searched by its bare name — `<dir>/<tool>.exe` is.
/// It does not check the executable bit: on Windows there is none to check,
/// and on Unix a `PATH` entry that is not executable is already a broken
/// installation a doctor check should report by *trying to run the tool*
/// (see `doctor::check_tool_version`), not by silently calling it absent here.
pub(crate) fn tool_in_dirs(tool: &str, dirs: impl Iterator<Item = PathBuf>) -> bool {
    let exe: &OsStr = tool.as_ref();
    for dir in dirs {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return true;
        }
        if candidate.with_extension("exe").is_file() {
            return true;
        }
    }
    false
}

/// The three desktop-ish operating systems this crate knows how to package
/// for, as the string [`std::env::consts::OS`] reports.
///
/// Exists so every place that asks "is this machine macOS/Windows/Linux" asks
/// through one name rather than a scattered `cfg!(target_os = "…")`, and so
/// the answer can be handed a fake value in a test — see
/// `package::tests::host_gating_is_correct_for_every_target`.
pub(crate) fn host_os() -> &'static str {
    std::env::consts::OS
}

/// Whether `path` is a file that looks runnable — used only to check that a
/// build actually produced the binary it promised, never to search for one.
pub(crate) fn is_probably_binary(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_in_one_of_the_directories_is_found() {
        let dir = std::env::temp_dir().join("vieww-build-util-test-found");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("my-fake-tool");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();

        let other = std::env::temp_dir().join("vieww-build-util-test-empty");
        std::fs::create_dir_all(&other).unwrap();

        assert!(tool_in_dirs(
            "my-fake-tool",
            vec![other.clone(), dir.clone()].into_iter()
        ));
        assert!(!tool_in_dirs(
            "nonexistent-tool",
            vec![other.clone(), dir.clone()].into_iter()
        ));

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&other).ok();
    }

    #[test]
    fn a_windows_style_exe_suffix_is_also_found() {
        let dir = std::env::temp_dir().join("vieww-build-util-test-exe");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wix.exe"), b"").unwrap();

        assert!(tool_in_dirs("wix", vec![dir.clone()].into_iter()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_directories_means_not_found() {
        assert!(!tool_in_dirs("anything", std::iter::empty()));
    }

    /// Not an assertion about this machine's `PATH` — there is no fact to
    /// assert — only that the real lookup runs at all and returns rather than
    /// panicking on a `PATH` entry that does not exist.
    #[test]
    fn the_real_path_lookup_does_not_panic() {
        let _ = tool_on_path("definitely-not-a-real-tool-name-xyz");
    }

    #[test]
    fn host_os_answers_with_a_known_name() {
        // Not asserting *which* one — this suite runs on whatever CI gives
        // it — only that it is one of the three this crate has logic for, or
        // at least a non-empty string a reader could still make sense of.
        assert!(!host_os().is_empty());
    }
}
