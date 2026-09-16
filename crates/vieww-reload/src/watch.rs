//! Noticing that the guest library has been rebuilt.
//!
//! # Why polling, and why a copy
//!
//! Polling a modification time rather than an inotify/kqueue crate: this runs
//! once every few hundred milliseconds in a development loop, the dependency
//! would be new where `libloading` was already present, and a filesystem watcher
//! is a surprising amount of platform code to carry for a `stat`.
//!
//! The subtler half is that **a rebuilt library must be copied before it is
//! opened**. `cargo` writes the `.so` in place, so `dlopen` on the path cargo is
//! still linking gets a half-written file; and once opened, the file is mapped,
//! which on some platforms stops the next build from replacing it. Loading a
//! numbered copy sidesteps both, and the copies are the leak that
//! [`crate::Guest`] documents rather than a second one.

use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What a poll of the guest's path found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Nothing has moved since the last look.
    Unchanged,
    /// The library was rebuilt and is ready to load.
    Rebuilt,
    /// The path does not exist, or cannot be read.
    ///
    /// Ordinary rather than an error: it is what a `cargo build` looks like
    /// halfway through, and the next poll is expected to find the file back.
    Missing,
}

/// Watches one library path for rebuilds.
///
/// Holds only the last modification time seen, so it is cheap to poll and has
/// no thread of its own — the caller decides how often to ask, which lets the
/// event loop own the cadence rather than a background thread racing it.
#[derive(Debug)]
pub struct Watch {
    path: PathBuf,
    /// Where the copy goes, when it may not go beside the original.
    ///
    /// `None` means "beside it", which is right wherever one directory is both
    /// writable and executable — a desktop `target/` — and is the only case
    /// this crate had until Android.
    staging_dir: Option<PathBuf>,
    /// `None` until the first successful look, so the first poll after start-up
    /// is `Unchanged` rather than a spurious `Rebuilt`.
    seen: Option<SystemTime>,
    /// Bumped per accepted rebuild, and the suffix of the copy that gets
    /// loaded. Also the reload counter a host can show.
    generation: u32,
}

impl Watch {
    /// Watch `path`, treating whatever is there now as the starting point.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let seen = modified(&path);
        Self {
            path,
            staging_dir: None,
            seen,
            generation: 0,
        }
    }

    /// Copy into `dir` rather than beside the watched library.
    ///
    /// # Why this exists, which is a fact about Android rather than a preference
    ///
    /// On a desktop the library is watched in `target/`, which is writable and
    /// executable, so the copy goes beside it and nothing has to be said. **On a
    /// phone those are two different directories and neither can do the other's
    /// job.** The rebuilt library arrives over `adb`, which can only write
    /// somewhere world-reachable — the app's external directory or
    /// `/data/local/tmp` — and `dlopen` has to happen from the app's private
    /// directory, which `adb` cannot write and which is the only place the app
    /// may both write and load. External storage is a FUSE mount and generally
    /// `noexec` besides.
    ///
    /// So on Android this is not optional: watch where the push lands, stage
    /// into `internal_data_path()`. Measured working on a Redmi Note 7 Pro at
    /// `target_sdk_version = 34`, in a non-debuggable release build — see
    /// `dlopen_a_real_guest_library` in the platform crate's `examples/`.
    #[must_use]
    pub fn stage_in(mut self, dir: impl Into<PathBuf>) -> Self {
        self.staging_dir = Some(dir.into());
        self
    }

    /// The library being watched.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many rebuilds have been accepted.
    #[must_use]
    pub const fn generation(&self) -> u32 {
        self.generation
    }

    /// Look once.
    ///
    /// A change is reported only when the timestamp **differs** from the last
    /// one seen, rather than when it is newer: a rebuild that restores an older
    /// artefact — `git stash`, a branch switch — is still a rebuild, and
    /// comparing for "newer" would ignore it and leave the developer editing a
    /// file that no longer runs.
    pub fn poll(&mut self) -> Change {
        let Some(now) = modified(&self.path) else {
            return Change::Missing;
        };
        match self.seen {
            Some(seen) if seen == now => Change::Unchanged,
            _ => {
                self.seen = Some(now);
                self.generation += 1;
                Change::Rebuilt
            }
        }
    }

    /// Where to copy the current library before opening it.
    ///
    /// Suffixed with the generation, so the copies are obvious in a target
    /// directory and trivially removable. A leaked library per reload is
    /// expected — see [`crate::Guest`].
    ///
    /// Beside the original unless [`stage_in`](Self::stage_in) said otherwise,
    /// which on Android it must.
    #[must_use]
    pub fn staged_path(&self) -> PathBuf {
        let extension = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("so");
        let stem = self
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("guest");
        let name = format!("{stem}-reload-{}.{extension}", self.generation);
        match &self.staging_dir {
            Some(dir) => dir.join(name),
            None => self.path.with_file_name(name),
        }
    }

    /// Copy the library to [`staged_path`](Self::staged_path) and hand back
    /// where it went.
    ///
    /// Creates the staging directory when one was named: on Android
    /// `internal_data_path()` exists but a subdirectory of it may not, and a
    /// reload failing on a missing directory would be read as the load being
    /// refused.
    pub fn stage(&self) -> io::Result<PathBuf> {
        let staged = self.staged_path();
        if let Some(dir) = &self.staging_dir {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::copy(&self.path, &staged)?;
        Ok(staged)
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A scratch file that cleans up after itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("vieww-reload-{name}.so"));
            let _ = std::fs::remove_file(&path);
            Self(path)
        }

        fn write(&self, contents: &str) {
            std::fs::write(&self.0, contents).expect("scratch is writable");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn a_path_that_is_not_there_is_missing_rather_than_an_error() {
        // What a `cargo build` looks like halfway through. Reporting it as a
        // failure would put an error in the log on every single rebuild.
        let mut watch = Watch::new(std::env::temp_dir().join("vieww-reload-absent.so"));
        assert_eq!(watch.poll(), Change::Missing);
    }

    #[test]
    fn the_first_poll_after_start_up_is_not_a_rebuild() {
        // The library that is already there is the one already running. A
        // `Rebuilt` here would reload the application the moment it started.
        let file = Scratch::new("first-poll");
        file.write("v1");
        let mut watch = Watch::new(&file.0);
        assert_eq!(watch.poll(), Change::Unchanged);
        assert_eq!(watch.generation(), 0);
    }

    #[test]
    fn a_rewrite_is_seen_once_and_then_settles() {
        let file = Scratch::new("rewrite");
        file.write("v1");
        let mut watch = Watch::new(&file.0);

        // Filesystem timestamps are coarse on some platforms, so the write has
        // to land in a different tick to be a fair test of the comparison
        // rather than of the clock.
        std::thread::sleep(Duration::from_millis(20));
        file.write("v2");

        assert_eq!(watch.poll(), Change::Rebuilt);
        assert_eq!(watch.generation(), 1);
        assert_eq!(
            watch.poll(),
            Change::Unchanged,
            "a rebuild is reported once, not on every poll after it"
        );
    }

    #[test]
    fn the_staged_copy_is_named_per_generation() {
        // Each reload opens its own copy, because the original is what cargo is
        // about to overwrite and what the process may still have mapped.
        let file = Scratch::new("staging");
        file.write("v1");
        let watch = Watch::new(&file.0);
        let staged = watch.staged_path();
        assert!(
            staged.to_string_lossy().contains("-reload-0."),
            "{staged:?}"
        );
        assert_ne!(staged, watch.path());
        assert_eq!(
            staged.extension(),
            watch.path().extension(),
            "a loader still has to recognise it as a library"
        );
    }

    #[test]
    fn staging_produces_a_file_that_is_a_copy_rather_than_a_link() {
        let file = Scratch::new("copy");
        file.write("contents");
        let watch = Watch::new(&file.0);
        let staged = watch.stage().expect("temp dir is writable");
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "contents",
            "the copy has the bytes the loader will open"
        );
        let _ = std::fs::remove_file(staged);
    }

    #[test]
    fn a_named_directory_takes_the_copy_instead_of_the_original_s_neighbour() {
        // The Android case: the watched path is where `adb` can write, and the
        // copy has to land where the app may load. On a desktop these are one
        // directory and the distinction is invisible, which is why it went
        // unnoticed until a phone needed it.
        let file = Scratch::new("stage-in");
        file.write("contents");
        let elsewhere = std::env::temp_dir().join("vieww-reload-stage-in-dir");
        let _ = std::fs::remove_dir_all(&elsewhere);

        let watch = Watch::new(&file.0).stage_in(&elsewhere);
        let staged = watch.stage().expect("the directory is created for us");

        assert_eq!(
            staged.parent(),
            Some(elsewhere.as_path()),
            "the copy belongs in the directory that was named, not beside the source"
        );
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            "contents",
            "and it is still a real copy of the library"
        );
        assert!(
            !file
                .0
                .with_file_name("vieww-reload-stage-in-reload-0.so")
                .exists(),
            "nothing should have been written beside the original"
        );

        let _ = std::fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn the_staging_directory_is_created_rather_than_assumed() {
        // `internal_data_path()` exists on Android but a subdirectory of it need
        // not, and a reload that failed on a missing directory would be read as
        // the *load* being refused — the one diagnosis this crate most needs to
        // keep separate from the others.
        let file = Scratch::new("stage-mkdir");
        file.write("contents");
        let nested = std::env::temp_dir().join("vieww-reload-mkdir/one/two");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("vieww-reload-mkdir"));
        assert!(
            !nested.exists(),
            "the test is pointless if it is already there"
        );

        let watch = Watch::new(&file.0).stage_in(&nested);
        watch.stage().expect("the directory is created on the way");

        assert!(nested.is_dir());
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("vieww-reload-mkdir"));
    }
}
