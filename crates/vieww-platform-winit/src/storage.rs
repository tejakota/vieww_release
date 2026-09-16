//! [`Storage`] backed by a file in the application's own directory.
//!
//! # Why a file rather than each platform's settings store
//!
//! Android has `SharedPreferences` and iOS has `NSUserDefaults`, and the obvious
//! implementation is one JNI bridge plus one `objc2` bridge. This is a file on
//! all three targets instead, and the reasoning is worth stating because the
//! obvious version is not wrong, just worse here:
//!
//! - **It is one implementation, not three.** The JNI and `objc2` versions would
//!   be the first platform code in this repository with no way to test it
//!   except on a device — `insets.rs` and `scale.rs` at least produce numbers a
//!   device suite can assert. A settings store that silently loses writes on one
//!   platform is exactly the bug that survives to production.
//! - **The semantics are ours.** `SharedPreferences` is typed, `NSUserDefaults`
//!   is plist-typed, and [`Storage`] is strings. Bridging either means choosing
//!   what happens to a key some other part of the app wrote as an integer.
//! - **Nothing is given up.** The platform stores buy interoperability with
//!   native code and backup integration, and an application that needs either
//!   can register its own [`Storage`] — which is the entire point of the
//!   [`service`](vieww_foundation::service) seam.
//!
//! The file is written atomically, because a mobile process is killed without
//! warning and a half-written settings file is worse than a lost write.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use vieww_foundation::{ServiceError, Storage};

/// The file inside the storage directory.
const FILE: &str = "storage.kv";

/// Where a half-written file lives until it is complete.
const TEMPORARY: &str = "storage.kv.tmp";

/// A writable directory for this application's own data.
///
/// - **Linux**: `$XDG_DATA_HOME/vieww` or `$HOME/.local/share/vieww`
/// - **macOS**: `$HOME/Library/Application Support/vieww`
/// - **Windows**: `%APPDATA%\vieww`
/// - **iOS**: `$HOME/Documents`, which inside an app sandbox is the app's own
///   documents directory — `HOME` is the sandbox root, which is why this needs
///   no `objc2` call.
///
/// **Android is not here**, for the reason it has no [`crash_dir`](crate::crash)
/// either: the path lives on the activity and cannot be derived without one.
/// Use `FileStorage::for_android`.
///
/// Falls back to the temporary directory when no home can be found, which keeps
/// an application running in a stripped environment rather than failing at
/// startup — with the documented consequence that settings may not survive.
#[must_use]
pub fn data_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else if cfg!(target_os = "ios") {
        // Inside the sandbox `HOME` is the app container, and `Documents` is
        // the directory that is backed up and that the app owns.
        return std::env::var_os("HOME").map_or_else(
            || std::env::temp_dir().join("vieww"),
            |home| PathBuf::from(home).join("Documents"),
        );
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    };

    base.unwrap_or_else(std::env::temp_dir).join("vieww")
}

/// Key-value storage in a file.
///
/// Reads are served from memory — the file is loaded once, at construction — so
/// [`Storage::get`] costs a hash lookup and the synchronous API in the trait is
/// honest. Writes go through to disk immediately, because the alternative is
/// deciding when to flush and being wrong about it when the process is killed.
#[derive(Debug)]
pub struct FileStorage {
    dir: PathBuf,
    entries: RefCell<HashMap<String, String>>,
}

impl FileStorage {
    /// Storage in `dir`, loading whatever is already there.
    ///
    /// A directory that does not exist is created on first write rather than
    /// here: an application that never stores anything should not leave a
    /// directory behind.
    ///
    /// A file that cannot be read — corrupt, truncated by a kill mid-write,
    /// written by a newer version — is treated as empty rather than as an
    /// error. Losing settings is bad; refusing to start is worse, and there is
    /// no third option that does not involve asking a user about a file format.
    #[must_use]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let entries = fs::read_to_string(dir.join(FILE))
            .map(|text| decode(&text))
            .unwrap_or_default();
        Self {
            dir,
            entries: RefCell::new(entries),
        }
    }

    /// Storage in the platform's own data directory.
    #[must_use]
    pub fn platform() -> Self {
        Self::at(data_dir())
    }

    /// Storage inside an Android application's private directory.
    ///
    /// `internal_data_path` is the activity's `filesDir` — private to the app,
    /// backed up with it, and removed when it is uninstalled. Returns `None`
    /// when the platform did not supply one, which happens before the activity
    /// is attached.
    #[cfg(target_os = "android")]
    #[must_use]
    pub fn for_android(android: &crate::AndroidApp) -> Option<Self> {
        android.internal_data_path().map(Self::at)
    }

    /// Where this storage keeps its file.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Write the whole map out, atomically.
    ///
    /// Written to a temporary name and renamed over the real one, because
    /// `rename` within a directory is atomic on every filesystem this runs on.
    /// A process killed mid-write leaves the temporary file and the previous
    /// complete one, which is the outcome worth engineering for on a platform
    /// that kills processes as a matter of routine.
    fn flush(&self) -> Result<(), ServiceError> {
        fs::create_dir_all(&self.dir).map_err(|error| {
            ServiceError::failed(format!("creating {}: {error}", self.dir.display()))
        })?;

        let temporary = self.dir.join(TEMPORARY);
        let text = encode(&self.entries.borrow());
        fs::write(&temporary, text).map_err(|error| {
            ServiceError::failed(format!("writing {}: {error}", temporary.display()))
        })?;
        fs::rename(&temporary, self.dir.join(FILE))
            .map_err(|error| ServiceError::failed(format!("replacing {FILE}: {error}")))
    }
}

impl Storage for FileStorage {
    fn get(&self, key: &str) -> Result<Option<String>, ServiceError> {
        Ok(self.entries.borrow().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), ServiceError> {
        self.entries
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        self.flush()
    }

    fn remove(&self, key: &str) -> Result<(), ServiceError> {
        // Nothing to write if nothing was there — and no directory created
        // either, which is what keeps `remove` on a fresh install free.
        if self.entries.borrow_mut().remove(key).is_none() {
            return Ok(());
        }
        self.flush()
    }

    fn keys(&self) -> Result<Vec<String>, ServiceError> {
        Ok(self.entries.borrow().keys().cloned().collect())
    }

    fn clear(&self) -> Result<(), ServiceError> {
        self.entries.borrow_mut().clear();
        self.flush()
    }
}

/// The map as text: one `key\tvalue` line per entry.
///
/// Tab-separated rather than anything cleverer because the escape rules below
/// are the entire format, and a format small enough to hold in your head is one
/// that cannot disagree with its own parser.
fn encode(entries: &HashMap<String, String>) -> String {
    let mut text = String::new();
    for (key, value) in entries {
        text.push_str(&escape(key));
        text.push('\t');
        text.push_str(&escape(value));
        text.push('\n');
    }
    text
}

fn decode(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(key, value)| (unescape(key), unescape(value)))
        .collect()
}

/// Backslash, tab and newline, so a value containing any of them survives.
fn escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            // A trailing or unknown escape is kept verbatim rather than
            // dropped: this file is read by a later version of this code, and
            // silently eating characters it does not recognise is how a
            // settings file rots.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory that is removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("vieww-storage-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_value_survives_being_reopened() {
        let dir = TempDir::new("reopen");
        FileStorage::at(&dir.0).set("theme", "dark").expect("write");

        let reopened = FileStorage::at(&dir.0);
        assert_eq!(
            reopened.get("theme").expect("read"),
            Some("dark".to_owned()),
            "the whole point: this is what an in-memory store cannot do"
        );
    }

    #[test]
    fn tabs_and_newlines_in_a_value_survive_a_round_trip() {
        // The format is two characters wide, so this is the test that keeps it
        // honest — a token with a newline in it is not hypothetical.
        let dir = TempDir::new("escapes");
        let awkward = "line one\nline\ttwo\\three";
        FileStorage::at(&dir.0).set("k", awkward).expect("write");

        assert_eq!(
            FileStorage::at(&dir.0).get("k").expect("read"),
            Some(awkward.to_owned())
        );
    }

    #[test]
    fn a_key_with_a_tab_in_it_survives_too() {
        let dir = TempDir::new("tab-key");
        FileStorage::at(&dir.0).set("a\tb", "v").expect("write");
        assert_eq!(
            FileStorage::at(&dir.0).get("a\tb").expect("read"),
            Some("v".to_owned())
        );
    }

    #[test]
    fn a_corrupt_file_reads_as_empty_rather_than_failing_to_start() {
        let dir = TempDir::new("corrupt");
        fs::create_dir_all(&dir.0).expect("mkdir");
        fs::write(dir.0.join(FILE), "not\u{0}a\u{0}valid\u{0}file").expect("write");

        let storage = FileStorage::at(&dir.0);
        assert!(storage.get("anything").expect("read").is_none());
        assert!(
            storage.set("k", "v").is_ok(),
            "and it is still usable afterwards"
        );
    }

    #[test]
    fn removing_a_key_that_was_never_set_writes_nothing() {
        let dir = TempDir::new("absent");
        let storage = FileStorage::at(&dir.0);
        storage.remove("never").expect("remove");
        assert!(
            !dir.0.exists(),
            "an application that stores nothing should leave no directory"
        );
    }

    #[test]
    fn clearing_empties_the_file_as_well_as_the_map() {
        let dir = TempDir::new("clear");
        let storage = FileStorage::at(&dir.0);
        storage.set("a", "1").expect("write");
        storage.clear().expect("clear");

        assert!(FileStorage::at(&dir.0).keys().expect("read").is_empty());
    }

    #[test]
    fn no_temporary_file_is_left_behind_after_a_successful_write() {
        let dir = TempDir::new("atomic");
        FileStorage::at(&dir.0).set("k", "v").expect("write");
        assert!(
            !dir.0.join(TEMPORARY).exists(),
            "the temporary is renamed over the real file, not copied"
        );
    }

    #[test]
    fn the_data_dir_is_a_vieww_subdirectory_rather_than_a_shared_one() {
        // Writing settings straight into `~/.local/share` is how an application
        // collides with every other one on the machine.
        let dir = data_dir();
        assert!(
            dir.ends_with("vieww") || dir.ends_with("Documents"),
            "unexpected data directory: {}",
            dir.display()
        );
    }
}
