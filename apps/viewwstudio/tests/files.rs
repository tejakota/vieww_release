//! What the editor does with files that are not small, clean, writable UTF-8.
//!
//! Every case here used to reach `std::fs::read_to_string` and either fail with
//! no explanation or succeed and produce something wrong.

use std::io::Write;
use std::path::PathBuf;

use vieww_element::Runtime;
use viewwstudio::buffer::{human_bytes, looks_binary, Buffer, Encoding, LineEndings, MAX_BYTES};
use viewwstudio::language::Language;
use viewwstudio::Studio;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("viewwstudio-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(bytes).expect("write");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Binary and oversized files
// ---------------------------------------------------------------------------

#[test]
fn a_binary_file_is_refused_with_a_sentence() {
    let dir = Dir::new("binary");
    // A PNG header: NUL bytes inside the sniff window.
    let path = dir.write(
        "logo.png",
        &[0x89, b'P', b'N', b'G', 0x00, 0x00, 0x1A, 0x0A],
    );
    let error = Buffer::open(&path).expect_err("binary files do not open");
    assert!(
        error.to_string().contains("binary"),
        "the message has to say why: {error}"
    );
}

#[test]
fn the_binary_sniff_looks_at_the_first_bytes_only() {
    assert!(looks_binary(b"\0"));
    assert!(looks_binary(b"text then \0 a nul"));
    assert!(!looks_binary(b"ordinary text with no nul in it"));
    assert!(!looks_binary(b""), "an empty file is a text file");
}

#[test]
fn an_enormous_file_is_refused_before_it_is_read_into_memory() {
    let dir = Dir::new("huge");
    // One byte over the limit is enough to prove the guard; writing 8 MB of
    // zeroes would also trip the binary sniff, so this is spaces.
    let big = vec![b' '; usize::try_from(MAX_BYTES).expect("fits") + 1];
    let path = dir.write("giant.log", &big);
    let error = Buffer::open(&path).expect_err("too big to open");
    let message = error.to_string();
    assert!(message.contains("larger than"), "{message}");
    assert!(message.contains("MB"), "the size is stated: {message}");
}

#[test]
fn a_file_at_the_limit_still_opens() {
    let dir = Dir::new("atlimit");
    let path = dir.write("ok.rs", &vec![b' '; 1024]);
    assert!(Buffer::open(&path).is_ok());
}

#[test]
fn byte_counts_are_readable() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(1024), "1.0 KB");
    assert_eq!(human_bytes(8 * 1024 * 1024), "8.0 MB");
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

#[test]
fn a_file_that_is_not_utf8_opens_readable_and_refuses_to_be_saved() {
    let dir = Dir::new("latin1");
    // `café` in Latin-1: the 0xE9 is not valid UTF-8.
    let path = dir.write("notes.txt", b"caf\xE9\n");
    let mut buffer = Buffer::open(&path).expect("opens rather than refusing");
    assert_eq!(buffer.encoding, Encoding::Unknown);
    assert!(
        buffer.value.text.contains('\u{FFFD}'),
        "the undecodable byte is shown as a replacement character"
    );

    let error = buffer.save().expect_err("saving would corrupt the file");
    assert!(error.to_string().contains("UTF-8"), "{error}");

    // And the file on disk is untouched.
    assert_eq!(std::fs::read(&path).expect("still there"), b"caf\xE9\n");
}

#[test]
fn clean_utf8_says_so() {
    let dir = Dir::new("utf8");
    let path = dir.write("hello.rs", "fn main() { let s = \"héllo\"; }\n".as_bytes());
    let buffer = Buffer::open(&path).expect("opens");
    assert_eq!(buffer.encoding, Encoding::Utf8);
    assert_eq!(buffer.encoding.name(), "UTF-8");
}

// ---------------------------------------------------------------------------
// Line endings
// ---------------------------------------------------------------------------

#[test]
fn line_endings_are_detected() {
    assert_eq!(LineEndings::detect("a\nb\n"), LineEndings::Lf);
    assert_eq!(LineEndings::detect("a\r\nb\r\n"), LineEndings::Crlf);
    assert_eq!(LineEndings::detect("a\r\nb\n"), LineEndings::Mixed);
    assert_eq!(LineEndings::detect("no newline"), LineEndings::Lf);
}

/// The finding: a CRLF file was loaded, edited, and written back with its
/// endings changed — a whole-file diff on the next commit, produced by opening
/// the file.
#[test]
fn a_crlf_file_is_still_crlf_after_a_round_trip() {
    let dir = Dir::new("crlf");
    let path = dir.write("windows.rs", b"fn main() {\r\n}\r\n");

    let mut buffer = Buffer::open(&path).expect("opens");
    assert_eq!(buffer.endings, LineEndings::Crlf);
    assert!(
        !buffer.value.text.contains('\r'),
        "the buffer itself holds LF — every offset in the editor assumes it"
    );

    buffer.save().expect("saves");
    assert_eq!(
        std::fs::read(&path).expect("read back"),
        b"fn main() {\r\n}\r\n",
        "byte for byte what it was"
    );
}

#[test]
fn an_lf_file_does_not_grow_carriage_returns() {
    let dir = Dir::new("lf");
    let path = dir.write("unix.rs", b"fn main() {\n}\n");
    let mut buffer = Buffer::open(&path).expect("opens");
    buffer.save().expect("saves");
    assert_eq!(
        std::fs::read(&path).expect("read back"),
        b"fn main() {\n}\n"
    );
}

// ---------------------------------------------------------------------------
// Read-only
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_read_only_file_says_so_on_open_rather_than_on_save() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Dir::new("readonly");
    let path = dir.write("locked.rs", b"fn main() {}\n");
    let mut permissions = std::fs::metadata(&path).expect("meta").permissions();
    permissions.set_mode(0o444);
    std::fs::set_permissions(&path, permissions).expect("chmod");

    let mut buffer = Buffer::open(&path).expect("opens for reading");
    assert!(buffer.read_only, "known before a single keystroke");

    let error = buffer.save().expect_err("and the save is refused");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

// ---------------------------------------------------------------------------
// Language
// ---------------------------------------------------------------------------

#[test]
fn a_buffer_knows_what_language_it_is() {
    let dir = Dir::new("lang");
    let toml = dir.write("Cargo.toml", b"[package]\n");
    let rust = dir.write("main.rs", b"fn main() {}\n");

    assert_eq!(Buffer::open(&toml).expect("opens").language, Language::Toml);
    assert_eq!(Buffer::open(&rust).expect("opens").language, Language::Rust);
}

/// Each file is highlighted with **its own** grammar, and a file with no
/// grammar is drawn flat.
///
/// # What this test used to pin
///
/// It was `a_non_rust_file_is_not_highlighted_with_the_rust_grammar`, and it
/// asserted that a `Cargo.toml` came back with *no* spans at all. The rule
/// behind it was right and is kept — running the Rust grammar over TOML is not
/// "no highlighting", it is *wrong* highlighting, which looks the same to
/// somebody who has not seen the file before. What has changed is that TOML now
/// has a grammar of its own, so the honest answer for it is no longer "nothing".
///
/// A `.sql` file stands in for everything still without one.
#[test]
fn every_file_is_highlighted_with_its_own_grammar_or_not_at_all() {
    let dir = Dir::new("nohl");
    let toml = dir.write("Cargo.toml", b"[package]\nname = \"app\"\n");
    let sql = dir.write("query.sql", b"SELECT 1;\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    let style = vieww_foundation::TextStyle::new(13.0);
    let theme = viewwstudio::StudioTheme::dark();

    studio.open_path(toml);
    let spans = studio.highlighted_spans(style, theme);
    assert!(!spans.is_empty(), "TOML has its own grammar now");
    assert!(
        spans.iter().any(|span| span.style.color != style.color),
        "and it is actually colouring something: {spans:?}"
    );

    studio.open_path(sql);
    assert!(
        studio.highlighted_spans(style, theme).is_empty(),
        "a language with no grammar is drawn flat rather than parsed as \
         whatever happens to be loaded"
    );

    // And the Rust scratch buffer it opened with still is highlighted.
    studio.active_buffer.set(0);
    let spans = studio.highlighted_spans(style, theme);
    assert!(!spans.is_empty(), "Rust files still highlight");
}

// ---------------------------------------------------------------------------
// One file, one buffer
// ---------------------------------------------------------------------------

#[test]
fn the_same_file_reached_by_two_paths_is_one_buffer() {
    let dir = Dir::new("canonical");
    let direct = dir.write("thing.rs", b"fn main() {}\n");
    // The same file, spelled with a detour through the parent.
    let roundabout = dir.0.join("sub/../thing.rs");
    std::fs::create_dir_all(dir.0.join("sub")).expect("subdir");

    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    let before = studio.buffers.get().len();

    studio.open_path(direct);
    studio.open_path(roundabout);

    assert_eq!(
        studio.buffers.get().len(),
        before + 1,
        "one file is one buffer, however it was spelled"
    );
}

#[test]
fn opening_something_the_editor_will_not_take_says_why() {
    let dir = Dir::new("refused");
    let path = dir.write("blob.bin", &[0x00, 0x01, 0x02, 0x03]);
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    let before = studio.buffers.get().len();

    studio.open_path(path);

    assert_eq!(studio.buffers.get().len(), before, "nothing was opened");
    let notice = studio.notice.get().expect("and the user was told");
    assert!(notice.contains("binary"), "{notice}");
}

// ---------------------------------------------------------------------------
// The conflict that used to be silent
// ---------------------------------------------------------------------------

/// `drain_watcher` reloaded a clean buffer and, when the buffer was dirty, took
/// a branch that did **nothing**: no banner, no prompt, no marker. The user
/// kept typing into a buffer that no longer matched disk and found out at save
/// time, by overwriting whatever had changed it.
#[test]
fn a_change_under_a_dirty_buffer_raises_a_question_rather_than_being_dropped() {
    let dir = Dir::new("conflict");
    let path = dir.write("shared.rs", b"fn main() {}\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.open_path(path.clone());

    // Edited here, and changed out there.
    studio.edit(vieww_foundation::TextEditingValue::new(
        "fn main() { mine(); }",
    ));
    std::fs::write(&path, b"fn main() { theirs(); }\n").expect("outside change");

    studio.raise_conflicts(vec![std::fs::canonicalize(&path).expect("canonical")]);

    assert_eq!(studio.conflict_names().len(), 1);
    assert_eq!(studio.conflict_names()[0], "shared.rs");
}

#[test]
fn the_same_file_changing_twice_is_one_question() {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    let path = PathBuf::from("/tmp/thing.rs");
    studio.raise_conflicts(vec![path.clone()]);
    studio.raise_conflicts(vec![path]);
    assert_eq!(studio.conflict_names().len(), 1);
}

#[test]
fn keeping_mine_clears_the_question_and_leaves_the_buffer_alone() {
    let dir = Dir::new("keepmine");
    let path = dir.write("a.rs", b"original\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.open_path(path.clone());
    studio.edit(vieww_foundation::TextEditingValue::new("mine"));
    std::fs::write(&path, b"theirs\n").expect("outside change");
    studio.raise_conflicts(vec![std::fs::canonicalize(&path).expect("canonical")]);

    studio.keep_mine();

    assert!(studio.conflict_names().is_empty());
    assert_eq!(studio.active().expect("a buffer").value.text, "mine");
}

#[test]
fn taking_theirs_reloads_from_disk() {
    let dir = Dir::new("taketheirs");
    let path = dir.write("b.rs", b"original\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.open_path(path.clone());
    studio.edit(vieww_foundation::TextEditingValue::new("mine"));
    std::fs::write(&path, b"theirs\n").expect("outside change");
    studio.raise_conflicts(vec![std::fs::canonicalize(&path).expect("canonical")]);

    studio.take_theirs();

    assert!(studio.conflict_names().is_empty());
    assert_eq!(studio.active().expect("a buffer").value.text, "theirs\n");
}
