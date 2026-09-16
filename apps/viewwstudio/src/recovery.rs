//! Crash recovery: the unsaved text, on disk, before the process dies.
//!
//! # The gap this closes
//!
//! The studio had a quit dialog and nothing else. Close the window with
//! unsaved work and it asked; **lose the process any other way and the work was
//! gone.** `main.rs` said as much in the past tense — "before this, clicking
//! the window's close button on a modified buffer lost it: no dialog, no
//! autosave, no recovery file" — while only the dialog had actually been
//! written. A layout panic in a previewed screen (which the studio exists to
//! provoke), an OOM kill, a power cut, a `kill -9` on a hung build: each of
//! them took every unsaved buffer with it, and there was nothing on disk to
//! come back to.
//!
//! Every editor a developer has used recovers from this. This is that.
//!
//! # What is written, and what is deliberately not
//!
//! One directory, `recovery/` beside the settings file, holding one file per
//! **dirty** buffer. A clean buffer is already on disk under its own name and
//! copying it would be writing the same bytes twice.
//!
//! Each file begins with a single header line naming where the text came
//! from — an absolute path, or `scratch` for a buffer that has never been
//! saved — followed by a blank line and then the text verbatim. A header rather
//! than a separate manifest because a manifest is a second file that can
//! disagree with the first: the thing that names the text and the text itself
//! survive or are lost together.
//!
//! The file **name** is a hash of the origin, so the same buffer overwrites its
//! own recovery file rather than accumulating one per save tick, and two files
//! called `main.rs` in different projects do not collide.
//!
//! # Why the whole text and not a diff
//!
//! A diff needs the original to apply against, and the case this exists for is
//! the one where the original may also have moved. The text is what the user
//! had; storing anything less means recovery that sometimes works.
//!
//! # When it is cleared
//!
//! On a clean shutdown, and after a recovery has been offered and taken. A
//! recovery directory that outlives the crash it describes is a studio that
//! offers to restore yesterday's work every morning.

use std::path::{Path, PathBuf};

/// One buffer's unsaved text, as it was written out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// Where the text belongs, or `None` for a scratch buffer.
    pub origin: Option<PathBuf>,
    /// What to call it in the message. The file name, or `scratch`.
    pub name: String,
    pub text: String,
}

/// The directory recovery files are written to.
#[must_use]
pub fn dir() -> PathBuf {
    crate::about::data_dir().join("recovery")
}

/// The file name a given origin is written under.
///
/// A hash rather than the path with separators replaced: a path can be longer
/// than a file name is allowed to be, and two different paths can flatten to
/// the same string. FNV-1a because it is eight lines and this is a file name,
/// not a security boundary.
fn file_name(origin: Option<&Path>) -> String {
    let key = origin.map_or_else(
        || "scratch".to_owned(),
        |p| p.to_string_lossy().into_owned(),
    );
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.recovery")
}

/// Write one buffer's unsaved text.
///
/// # Errors
///
/// Whatever the write failed with. Reported by the caller into the Output
/// panel rather than raised: a recovery file that could not be written is worth
/// saying out loud and is not worth interrupting an edit for.
pub fn write(origin: Option<&Path>, text: &str) -> std::io::Result<()> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let header = origin.map_or_else(
        || "scratch".to_owned(),
        |path| path.to_string_lossy().into_owned(),
    );
    std::fs::write(dir.join(file_name(origin)), format!("{header}\n\n{text}"))
}

/// Forget the recovery file for one origin — what a save does.
pub fn forget(origin: Option<&Path>) {
    std::fs::remove_file(dir().join(file_name(origin))).ok();
}

/// Drop the whole directory. A clean shutdown, or a recovery that was taken.
pub fn clear() {
    std::fs::remove_dir_all(dir()).ok();
}

/// Everything a previous run left behind.
///
/// Empty on the ordinary launch, which is every launch that was not preceded by
/// a crash. Files that cannot be read or that have no header are skipped rather
/// than reported: the point of this directory is to be salvage, and salvage
/// that refuses to open because one file in it is damaged is not salvage.
#[must_use]
pub fn pending() -> Vec<Recovered> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("recovery") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(recovered) = parse(&contents) else {
            continue;
        };
        out.push(recovered);
    }
    // A stable order so the message reads the same twice.
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Split a recovery file into its origin and its text.
///
/// Separate from the read so the format is testable without a data directory.
#[must_use]
pub fn parse(contents: &str) -> Option<Recovered> {
    let (header, text) = contents.split_once("\n\n")?;
    let header = header.trim();
    if header.is_empty() {
        return None;
    }
    if header == "scratch" {
        return Some(Recovered {
            origin: None,
            name: "scratch".to_owned(),
            text: text.to_owned(),
        });
    }
    let origin = PathBuf::from(header);
    let name = origin
        .file_name()
        .map_or_else(|| header.to_owned(), |n| n.to_string_lossy().into_owned());
    Some(Recovered {
        origin: Some(origin),
        name,
        text: text.to_owned(),
    })
}

/// Whether a recovered text is worth offering.
///
/// It is not, if the file on disk already holds it — which happens when the
/// studio was killed *after* a save but before the recovery file was cleared.
/// Offering to restore text somebody already has is how a recovery prompt
/// becomes something people dismiss without reading.
#[must_use]
pub fn differs_from_disk(recovered: &Recovered) -> bool {
    let Some(origin) = recovered.origin.as_ref() else {
        // A scratch buffer has no disk to compare against, but an empty one is
        // not work anybody wants back.
        return !recovered.text.trim().is_empty();
    };
    std::fs::read_to_string(origin).is_ok_and(|on_disk| on_disk != recovered.text)
        || !origin.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_round_trips_through_its_own_format() {
        let written = format!("{}\n\n{}", "/p/src/main.rs", "fn main() {}\n");
        let parsed = parse(&written).expect("a recovery file");
        assert_eq!(parsed.origin, Some(PathBuf::from("/p/src/main.rs")));
        assert_eq!(parsed.name, "main.rs");
        assert_eq!(parsed.text, "fn main() {}\n");
    }

    #[test]
    fn a_scratch_buffer_says_so_rather_than_inventing_a_path() {
        let parsed = parse("scratch\n\nhello").expect("a recovery file");
        assert_eq!(parsed.origin, None);
        assert_eq!(parsed.name, "scratch");
        assert_eq!(parsed.text, "hello");
    }

    #[test]
    fn text_containing_a_blank_line_survives_intact() {
        // The header is split on the *first* blank line, so a body full of them
        // must come back whole — this is the failure that would silently
        // truncate somebody's recovered file at its first paragraph break.
        let body = "one\n\ntwo\n\nthree\n";
        let parsed = parse(&format!("/p/a.rs\n\n{body}")).expect("a recovery file");
        assert_eq!(parsed.text, body);
    }

    #[test]
    fn a_damaged_file_is_skipped_rather_than_guessed_at() {
        assert_eq!(parse("no header, no blank line"), None);
        assert_eq!(parse("\n\nbody with an empty header"), None);
    }

    #[test]
    fn two_paths_ending_in_the_same_name_get_different_files() {
        let one = file_name(Some(Path::new("/a/src/main.rs")));
        let two = file_name(Some(Path::new("/b/src/main.rs")));
        assert_ne!(one, two);
        assert_eq!(one, file_name(Some(Path::new("/a/src/main.rs"))), "stable");
    }

    #[test]
    fn an_empty_scratch_recovery_is_not_worth_offering() {
        assert!(!differs_from_disk(&Recovered {
            origin: None,
            name: "scratch".to_owned(),
            text: "   \n".to_owned(),
        }));
        assert!(differs_from_disk(&Recovered {
            origin: None,
            name: "scratch".to_owned(),
            text: "real work".to_owned(),
        }));
    }

    #[test]
    fn a_recovery_matching_the_file_on_disk_is_not_offered() {
        let dir = std::env::temp_dir().join("viewwstudio-recovery-test");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("same.rs");
        std::fs::write(&path, "fn main() {}\n").expect("write");

        let same = Recovered {
            origin: Some(path.clone()),
            name: "same.rs".to_owned(),
            text: "fn main() {}\n".to_owned(),
        };
        assert!(!differs_from_disk(&same), "already saved");

        let newer = Recovered {
            text: "fn main() { changed() }\n".to_owned(),
            ..same
        };
        assert!(differs_from_disk(&newer));
        std::fs::remove_dir_all(&dir).ok();
    }
}
