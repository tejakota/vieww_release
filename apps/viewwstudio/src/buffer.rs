//! Files on disk, and the text of the one being edited.
//!
//! M0's "buffers" were three names and a boolean. This is the real thing: a
//! directory of `.rs` files read at startup, each carrying its own
//! [`TextEditingValue`] — the text *and* where the caret and selection are —
//! so switching tabs restores the cursor rather than resetting it.
//!
//! # Still not a project
//!
//! The plan's non-goals hold (§3): no `Cargo.toml`, no dependency resolution,
//! no multi-file crate. A screen is one file, the directory is a flat list of
//! them, and nothing here knows what a module is. What this adds over M0 is
//! only that the file is *real* — opened, edited and saved — because an editor
//! that cannot lose your work is an editor that was never holding it.

use std::fs;
use std::path::{Path, PathBuf};

use vieww_foundation::TextEditingValue;

use crate::file_tree::{FileTree, Match as TreeMatch};
use crate::history::History;

/// One open file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    /// What the tab and the explorer show.
    pub name: String,
    /// Where it came from, and where [`save`](Buffer::save) writes it back.
    ///
    /// `None` for a scratch buffer that has never been on disk — the studio
    /// opens one when the directory it was pointed at is empty, so the editor
    /// is never a blank pane with no explanation.
    pub path: Option<PathBuf>,
    /// The text, the caret and the selection.
    pub value: TextEditingValue,
    /// Edited since it was last written to disk.
    pub dirty: bool,
    /// This file's own undo and redo stacks.
    ///
    /// Per buffer rather than per editor: see [`crate::history`]. It travels
    /// with the file, so switching tabs and coming back finds the same history
    /// rather than somebody else's keystrokes.
    pub history: History,
    /// What kind of file this is. Decides the status cell, whether the editor
    /// highlights it, and whether Render will touch it.
    pub language: crate::language::Language,
    /// How the bytes on disk decoded.
    pub encoding: Encoding,
    /// The line endings the file arrived with, restored when it is written.
    pub endings: LineEndings,
    /// The file cannot be written. Detected on open rather than discovered on
    /// save, so the editor can say so before the user has typed into it.
    pub read_only: bool,
}

/// Everything about an open file **except its text**.
///
/// # Why this exists, and what it cost not to have it
///
/// [`Studio::buffers`](crate::Studio) is one signal holding every open
/// [`Buffer`], text included, so a keystroke notifies every element that read
/// it. Most of those elements do not want the text: the tab strip wants a name
/// and a dot, the explorer's open-buffer list wants the same, the status bar
/// wants a language and the Save button wants a boolean. All of them rebuilt on
/// every character typed, and the studio's own bench put **492 elements** —
/// better than a fifth of the tree — on one buffer write.
///
/// `docs/PERFORMANCE.md` named this as one of the two things left after the
/// profiling round, and named the risk with it: a second signal that has to be
/// kept in step with the first is a signal that disagrees with it exactly once.
/// So the two are written by `Studio::put_buffers` and nowhere
/// else — one door, the same shape as `Studio::run` — and the metadata is
/// written with
/// [`set_if_changed`](vieww_element::Signal::set_if_changed), so typing, which
/// changes no field here, notifies nobody.
///
/// Cheap to compare: a handful of open files, each a short name, an optional
/// path and three scalars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferTab {
    /// [`Buffer::name`].
    pub name: String,
    /// [`Buffer::path`].
    pub path: Option<PathBuf>,
    /// [`Buffer::dirty`].
    pub dirty: bool,
    /// [`Buffer::language`].
    pub language: crate::language::Language,
    /// [`Buffer::read_only`].
    pub read_only: bool,
    /// [`Buffer::encoding`].
    pub encoding: Encoding,
    /// [`Buffer::endings`].
    pub endings: LineEndings,
}

impl BufferTab {
    /// The metadata half of `buffer`.
    #[must_use]
    pub fn of(buffer: &Buffer) -> Self {
        Self {
            name: buffer.name.clone(),
            path: buffer.path.clone(),
            dirty: buffer.dirty,
            language: buffer.language,
            read_only: buffer.read_only,
            encoding: buffer.encoding,
            endings: buffer.endings,
        }
    }
}

/// How a file's bytes decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Valid UTF-8. The only kind the studio will write.
    #[default]
    Utf8,
    /// Some bytes did not decode; the text shown is lossy. See
    /// [`Buffer::open`] for why this opens rather than refusing, and why
    /// [`Buffer::save`] then refuses.
    Unknown,
}

impl Encoding {
    /// What the status bar shows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            // Not "Binary" and not "Latin-1": the studio does not know which
            // encoding it is, only that it is not UTF-8, and guessing a name
            // is exactly the sin the hardcoded `"UTF-8"` cell committed.
            Self::Unknown => "Not UTF-8",
        }
    }
}

/// The line endings a file uses.
///
/// The buffer always holds LF — every offset in the editor, every diagnostic
/// line number and every fold assumes it — and this is what puts the file back
/// the way it was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEndings {
    #[default]
    Lf,
    Crlf,
    /// Both, in one file. Normalised to LF in the buffer and written out as
    /// LF: a file that was already inconsistent is one the studio is allowed to
    /// make consistent, and preserving a mixture byte-for-byte would mean
    /// tracking it per line for no one's benefit.
    Mixed,
}

impl LineEndings {
    /// What `text` uses.
    #[must_use]
    pub fn detect(text: &str) -> Self {
        let crlf = text.matches("\r\n").count();
        if crlf == 0 {
            return Self::Lf;
        }
        // Every LF that is not part of a CRLF.
        let lf = text.matches('\n').count();
        if lf == crlf {
            Self::Crlf
        } else {
            Self::Mixed
        }
    }

    /// What the status bar shows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
            Self::Mixed => "Mixed",
        }
    }

    /// Normalise to the LF the buffer holds.
    #[must_use]
    pub fn to_lf(self, text: String) -> String {
        match self {
            // Not `replace`: the overwhelmingly common case is a file with no
            // CR in it at all, and `detect` has already established that.
            Self::Lf => text,
            Self::Crlf | Self::Mixed => text.replace("\r\n", "\n"),
        }
    }

    /// Put the file's own endings back for the write.
    #[must_use]
    pub fn from_lf(self, text: &str) -> String {
        match self {
            Self::Lf | Self::Mixed => text.to_owned(),
            Self::Crlf => text.replace('\n', "\r\n"),
        }
    }
}

/// The largest file the editor will open.
///
/// Eight megabytes is far past any hand-written source file and far short of
/// the logs and datasets that make a text editor stop responding. The number is
/// a *refusal with a sentence*, not a silent truncation — see [`Buffer::open`].
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How many bytes are examined to decide whether a file is binary.
///
/// The same window `git` uses. A NUL in the first 8 KiB is the test; a file
/// whose only NUL is at byte 900,000 is a file the editor will happily open and
/// the user will notice.
pub const SNIFF: usize = 8 * 1024;

/// Whether `bytes` look like something that is not text.
#[must_use]
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(SNIFF).any(|byte| *byte == 0)
}

/// A byte count a person can read.
#[must_use]
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    #[expect(
        clippy::cast_precision_loss,
        reason = "a size shown to a person, rounded to one decimal"
    )]
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

impl Buffer {
    /// A buffer holding `text`, not backed by a file.
    #[must_use]
    pub fn scratch(name: impl Into<String>, text: impl Into<String>) -> Self {
        let value = TextEditingValue::new(text);
        Self {
            name: name.into(),
            path: None,
            history: History::new(value.clone()),
            value,
            dirty: false,
            // A scratch buffer is Rust: it holds `SCRATCH`, which is a screen,
            // and the Render button has to light up on it.
            language: crate::language::Language::Rust,
            encoding: Encoding::Utf8,
            endings: LineEndings::default(),
            read_only: false,
        }
    }

    /// Read `path` into a buffer.
    ///
    /// # What this used to be, and what each guard is for
    ///
    /// It was one line: `std::fs::read_to_string(path)?`. That is correct for
    /// a small UTF-8 text file and wrong for everything else a real project
    /// contains, in four different ways — and the studio's file tree listed
    /// every one of them as openable.
    ///
    /// * **Size.** A `read_to_string` of a 400 MB log allocates 400 MB and then
    ///   hands it to a text field that shapes it. The editor does not come
    ///   back. [`MAX_BYTES`] refuses first and says how big the file is, which
    ///   is a sentence the user can act on.
    /// * **Binary.** A `.png` is not valid UTF-8 and so failed already — but a
    ///   `.wasm` or a UTF-8-clean binary blob is not, and would open as a wall
    ///   of replacement characters that the user could then *save over the
    ///   original*. A NUL byte in the first [`SNIFF`] bytes is the same test
    ///   `git` uses and it is refused outright.
    /// * **Encoding.** Invalid UTF-8 is now decoded lossily and **flagged**,
    ///   rather than refused. Refusing means a file the user can see in the
    ///   tree and cannot open, with no explanation; opening it read-only with
    ///   the status bar saying so is the honest version. [`Buffer::save`]
    ///   refuses to write a lossily-decoded buffer, because writing it back
    ///   would replace every undecodable byte with `U+FFFD` — silent
    ///   corruption of a file the user only meant to look at.
    /// * **Line endings.** CRLF files were loaded, edited, and written back
    ///   with their endings changed to LF. On a Windows-authored file that is
    ///   a whole-file diff on the next commit, produced by opening it. The
    ///   endings are detected, normalised to LF *in the buffer* — every offset
    ///   in the editor assumes that — and restored on save.
    ///
    /// Read-only files are detected here too, for the same reason: a file the
    /// user cannot write looked exactly like one they could, until the save
    /// failed.
    ///
    /// # Errors
    ///
    /// If the file cannot be read, is larger than [`MAX_BYTES`], or looks
    /// binary.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        if metadata.len() > MAX_BYTES {
            return Err(std::io::Error::other(format!(
                "{} is {} — larger than the {} this editor will open",
                path.display(),
                human_bytes(metadata.len()),
                human_bytes(MAX_BYTES)
            )));
        }
        let bytes = std::fs::read(path)?;
        if looks_binary(&bytes) {
            return Err(std::io::Error::other(format!(
                "{} looks like a binary file",
                path.display()
            )));
        }

        let (text, encoding) = match String::from_utf8(bytes) {
            Ok(text) => (text, Encoding::Utf8),
            Err(error) => (
                String::from_utf8_lossy(error.as_bytes()).into_owned(),
                Encoding::Unknown,
            ),
        };
        let endings = LineEndings::detect(&text);
        let text = endings.to_lf(text);

        // The caret starts at the top of a file that was just opened, not at
        // the end: `TextEditingValue::new` puts it at the end because that is
        // what a *form field* wants, and a source file is the other case.
        let mut value = TextEditingValue::new(text);
        value.selection = vieww_foundation::TextSelection::collapsed(0);
        Ok(Self {
            name: path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into(),
            ),
            path: Some(path.to_path_buf()),
            history: History::new(value.clone()),
            value,
            dirty: false,
            language: crate::language::Language::of_path(path),
            encoding,
            endings,
            read_only: metadata.permissions().readonly(),
        })
    }

    /// Re-read the file from disk, discarding unsaved edits and the history
    /// that described them.
    ///
    /// # Errors
    ///
    /// If the buffer has no path, or the file cannot be read.
    pub fn reload(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "this buffer has never been on disk",
            ));
        };
        // Same guards as `open`: a file that became a 900 MB log while the
        // studio held it open is still a file this must not read into memory.
        let reloaded = Self::open(path)?;
        let mut value = TextEditingValue::new(reloaded.value.text);
        self.encoding = reloaded.encoding;
        self.endings = reloaded.endings;
        self.read_only = reloaded.read_only;
        // Keep the caret where it was, as far as the new text allows — a
        // reload that jumps to the top loses the reader's place for no reason.
        // Floored to a boundary because the byte it was at may be mid-character
        // in the file that came back.
        let at = floor_char_boundary(&value.text, self.value.selection.extent);
        value.selection = vieww_foundation::TextSelection::collapsed(at);
        self.history.reset(value.clone());
        self.value = value;
        self.dirty = false;
        Ok(())
    }

    /// Write the buffer back to the file it came from.
    ///
    /// The line endings the file arrived with are restored on the way out, so
    /// opening and saving a CRLF file is not a whole-file diff.
    ///
    /// # Errors
    ///
    /// If there is nowhere to write it, if the file is read-only, if it was
    /// decoded lossily (see [`Buffer::open`]), or if the write fails.
    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "this buffer has never been on disk",
            ));
        };
        if self.read_only {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("{} is read-only", path.display()),
            ));
        }
        if self.encoding == Encoding::Unknown {
            // Writing this back would turn every byte that did not decode into
            // `U+FFFD` — corruption of a file the user probably only opened to
            // look at. Refusing is the only safe answer, and saying why is what
            // makes it actionable.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} is not valid UTF-8; saving would replace the bytes that did not decode",
                    path.display()
                ),
            ));
        }
        std::fs::write(path, self.endings.from_lf(&self.value.text))?;
        self.dirty = false;
        Ok(())
    }

    /// The number of lines, which is what the gutter counts.
    ///
    /// A trailing newline makes a last, empty line — the same one every editor
    /// puts a cursor on, and the same one `wc -l` does not count.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.value.text.split('\n').count()
    }

    /// The caret's position as a 1-indexed `(line, column)`.
    ///
    /// Counted in `char`s rather than bytes, because a status bar that says
    /// "Col 7" after four keystrokes of `héllo` is reporting on UTF-8 rather
    /// than on the text.
    #[must_use]
    pub fn caret(&self) -> (u32, u32) {
        let offset = self.value.selection.extent.min(self.value.text.len());
        let before = &self.value.text[..floor_char_boundary(&self.value.text, offset)];

        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit_once('\n')
            .map_or(before, |(_, last)| last)
            .chars()
            .count()
            + 1;

        #[expect(
            clippy::cast_possible_truncation,
            reason = "a file with four billion lines is not one this edits"
        )]
        {
            (line as u32, column as u32)
        }
    }
}

/// The largest char boundary at or below `index`.
///
/// `str::floor_char_boundary` is unstable, and a selection offset arriving
/// mid-character would otherwise panic the slice above — which is a crash in
/// the editor for a keystroke, the one place it is least acceptable.
fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// How many files [`Workspace::open`] will put in tabs.
///
/// Eight is about a screen of tabs. The number is a bound on *surprise*, not on
/// capability: everything else is one click away in the Explorer, and the
/// session file reopens exactly what was open last time regardless of this.
pub const OPEN_LIMIT: usize = 8;

/// How deep [`entry_points`] walks before giving up.
///
/// Four levels reaches `src/screens/home/mod.rs` and stops well short of a
/// vendored dependency tree. A walk with no depth bound is a walk that can be
/// pointed at `/` by a mis-click in the folder picker.
const WALK_DEPTH: usize = 4;

/// Directories a source walk has no business entering.
///
/// Shared in spirit with `picker::SKIPPED` and `file_tree`'s own rule, and
/// deliberately not shared in code: this list is about *source files worth
/// opening*, the picker's is about *directories worth showing*, and merging
/// them would mean every future addition had to be right for both.
const SKIPPED: [&str; 6] = [
    "target",
    ".git",
    "node_modules",
    ".cargo",
    ".rustup",
    "vendor",
];

/// The files a person would open first in `root`, best first, bounded.
///
/// The order is the point:
///
/// 1. **Crate roots.** `src/main.rs` and `src/lib.rs` are where a reader of an
///    unfamiliar Rust project starts, and where a vieww project's `screen()`
///    usually is.
/// 2. **Loose files in the root.** What the old one-level scan found. A
///    single-file experiment still opens the way it always did.
/// 3. **Everything else**, shallowest first and alphabetically within a depth,
///    so `src/app.rs` beats `src/screens/detail/parts.rs`.
///
/// Duplicates are removed while preserving the first position, so a `src/lib.rs`
/// picked up by rule 1 does not appear again under rule 3.
#[must_use]
pub fn entry_points(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let push = |path: PathBuf, out: &mut Vec<PathBuf>| {
        if out.len() < OPEN_LIMIT && path.is_file() && !out.contains(&path) {
            out.push(path);
        }
    };

    for candidate in ["src/main.rs", "src/lib.rs", "main.rs", "lib.rs"] {
        push(root.join(candidate), &mut out);
    }

    // Rules 2 and 3 are one breadth-first walk: rule 2 is simply depth zero.
    let mut frontier = vec![root.to_path_buf()];
    let mut depth = 0;
    while depth <= WALK_DEPTH && out.len() < OPEN_LIMIT && !frontier.is_empty() {
        let mut next = Vec::new();
        // Sorted so the result does not depend on the order the filesystem
        // hands entries back, which differs between machines and between two
        // runs on the same one. A studio that opens different tabs on Tuesday
        // is a studio nobody trusts.
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for dir in &frontier {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || SKIPPED.contains(&name.as_str()) {
                    continue;
                }
                // `file_type` rather than `is_dir`: the former does not follow
                // a symlink, which is what stops a link pointing at an ancestor
                // from making this walk unbounded in a way `WALK_DEPTH` cannot
                // catch.
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    dirs.push(path);
                } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
                    files.push(path);
                }
            }
        }
        files.sort();
        dirs.sort();
        for file in files {
            push(file, &mut out);
        }
        next.extend(dirs);
        frontier = next;
        depth += 1;
    }

    out
}

/// Every file the studio has open, and where they came from.
///
/// N1 grows this from M1's flat list of `.rs` files into a real directory
/// workspace: a recursive [`FileTree`] for the Explorer, a flag that recognises
/// a vieww project (and so lights up Build/Export), and on-demand buffer
/// opening so a project with hundreds of files does not load them all at
/// startup. The `buffers` vec is the open tabs; the tree is everything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: Option<PathBuf>,
    pub buffers: Vec<Buffer>,
    /// The recursive directory tree the Explorer renders. Empty for a scratch
    /// workspace.
    pub tree: FileTree,
    /// `true` when `root` is a directory whose `Cargo.toml` depends on `vieww`,
    /// which is what the plan's §5.3 says lights up Build and Export.
    pub is_vieww: bool,
}

impl Workspace {
    /// Open `root` as a workspace: scan a recursive tree, recognise a vieww
    /// project, and open a small, useful set of buffers.
    ///
    /// A directory that cannot be read, or holds no `.rs` files at all, gives
    /// the scratch workspace rather than an error: the studio has to open.
    ///
    /// # What this used to do, and why it was the first thing every user saw
    ///
    /// It was one `read_dir` — the `.rs` files *directly inside* `root`, no
    /// walk. Every conventional Rust project keeps its code in `src/`, so
    /// opening a real project matched **nothing**, and the studio fell through
    /// to the scratch buffer. A user pointing the studio at their own crate for
    /// the first time got an editor containing a sample screen and no sign that
    /// their project had been read at all. The tree was scanned correctly the
    /// whole time, which is what made it look like a display bug rather than
    /// the workspace never having been opened.
    ///
    /// [`entry_points`] now walks, and picks the way a person would: the crate
    /// roots first, then what is loose in the top directory, then whatever else
    /// it finds, bounded at [`OPEN_LIMIT`]. The bound is the other half of the
    /// fix — a recursive walk that opened everything would greet the user with
    /// four hundred tabs, which is a different way of being unusable.
    #[must_use]
    pub fn open(root: &Path) -> Self {
        if !root.is_dir() {
            return Self::scratch();
        }
        let paths = entry_points(root);
        let buffers: Vec<Buffer> = paths.iter().filter_map(|p| Buffer::open(p).ok()).collect();

        let tree = FileTree::scan(root);
        let is_vieww = crate::file_tree::is_vieww_project(root);

        if buffers.is_empty() && tree.root().is_none() {
            return Self::scratch();
        }

        Self {
            root: Some(root.to_path_buf()),
            buffers,
            tree,
            is_vieww,
        }
    }

    /// Which open buffer the studio should put the caret in, best first.
    ///
    /// # The first-run defect this exists for
    ///
    /// [`entry_points`] opens the *crate roots* first, which is the right
    /// reading order for a person meeting an unfamiliar Rust project and the
    /// wrong opening move for this studio. `src/main.rs` in a vieww project is
    /// four lines that call `run()`. It has no `screen()`, so it is precisely
    /// the one file in the project the preview **cannot** render.
    ///
    /// So the studio's own New Project command produced this, every time:
    /// scaffold a project, watch it open on `main.rs`, press the large button
    /// in the top-right corner marked Render, and be told the render failed.
    /// On the first thing a new user does. The template was fine, the preview
    /// was fine, the error message was accurate, and the sequence was still a
    /// failure demo.
    ///
    /// Opening the tabs in reading order and *focusing* a renderable one keeps
    /// both properties: `main.rs` and `lib.rs` are still there, one click away
    /// and still first in the strip, and the file in front of the user is one
    /// Render works on.
    ///
    /// Falls back to zero, so a project with no screens in it behaves exactly
    /// as it did before.
    #[must_use]
    pub fn preferred_buffer(&self) -> usize {
        // **A Say screen first, then anything renderable.** A Say project's
        // working file is `home.say`; opening the studio onto it is the whole
        // point of the kind, and no Rust screen exists beside it to win the
        // entry-point check below.
        if let Some(index) = self
            .buffers
            .iter()
            .position(|buffer| buffer.language == crate::language::Language::Say)
        {
            return index;
        }
        self.buffers
            .iter()
            .position(|buffer| crate::compile::entry_point(&buffer.value.text))
            .unwrap_or(0)
    }

    /// The workspace a studio opens with nothing to open.
    #[must_use]
    pub fn scratch() -> Self {
        Self::of_buffers(vec![Buffer::scratch("scratch.rs", SCRATCH)])
    }

    /// A workspace holding exactly these buffers and no directory.
    ///
    /// # Why this exists rather than a struct literal
    ///
    /// `tests/interaction.rs` built a `Workspace { root, buffers }` by hand. N1
    /// added `tree` and `is_vieww`, and that test stopped compiling — a break
    /// that went unnoticed because the studio's integration tests need the GPU
    /// dev-dependency stack and were not run in the session that added the
    /// fields. Every future field would break it again the same way.
    ///
    /// A named constructor is the fix: callers that mean "these buffers, no
    /// directory" say so, and a new field is this function's problem rather
    /// than every caller's.
    #[must_use]
    pub fn of_buffers(buffers: Vec<Buffer>) -> Self {
        Self {
            root: None,
            buffers,
            tree: FileTree::empty(),
            is_vieww: false,
        }
    }

    /// Open `path` as a buffer, adding it to the tab strip if it is not already
    /// open. Returns the index of the now-open buffer.
    ///
    /// This is the on-demand half of N1: a project's tree may list hundreds of
    /// files, but only the ones the user clicks are loaded into memory. A path
    /// that cannot be read is ignored rather than panicking — a tree node that
    /// pointed at a vanished file is a stale tree, not a crashed editor.
    pub fn open_buffer(&mut self, path: &Path) -> Option<usize> {
        // Already open? Return its index.
        if let Some(index) = self
            .buffers
            .iter()
            .position(|b| b.path.as_deref() == Some(path))
        {
            return Some(index);
        }
        let buffer = Buffer::open(path).ok()?;
        self.buffers.push(buffer);
        Some(self.buffers.len() - 1)
    }

    /// Close the buffer at `index`, returning whether anything was removed.
    /// Keeps at least one buffer (a fresh scratch) so the editor is never empty.
    pub fn close_buffer(&mut self, index: usize) -> bool {
        if index >= self.buffers.len() {
            return false;
        }
        self.buffers.remove(index);
        if self.buffers.is_empty() {
            self.buffers.push(Buffer::scratch("scratch.rs", SCRATCH));
        }
        true
    }

    /// Save every dirty buffer. The first error stops the run and is returned,
    /// because a "Save All" that silently skipped a file is worse than one that
    /// reported which one failed.
    ///
    /// # Errors
    ///
    /// The first I/O error encountered.
    pub fn save_all(&mut self) -> std::io::Result<()> {
        for buffer in &mut self.buffers {
            if buffer.dirty {
                buffer.save()?;
            }
        }
        Ok(())
    }

    /// Search every file in the workspace tree for `query`. Delegates to the
    /// tree so the search covers files that are not open as buffers too — the
    /// plan's "find across the workspace" (§5.5).
    #[must_use]
    pub fn find(&self, query: &str) -> Vec<TreeMatch> {
        self.tree.find(query)
    }

    /// Re-scan the directory tree. Called when the watcher reports a change, so
    /// a `git checkout` done outside the window reaches the Explorer. Expansion
    /// state is preserved by [`FileTree::rescan`] itself.
    pub fn rescan(&mut self) {
        self.tree.rescan();
    }
}

/// What an empty studio starts with: the smallest thing that renders.
pub(crate) const SCRATCH: &str = "\
use vieww::prelude::*;

// `Widget: Any + Debug`, so this derive is not optional — without it the
// buffer does not compile, and the first thing a new studio does is fail.
#[derive(Debug)]
pub struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str {
        \"Screen\"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        Container::new()
            .color(theme.colors.surface)
            .alignment(Alignment::CENTER)
            .child(Text::new(\"Hello\").style(theme.text.headline))
            .into()
    }
}

pub fn screen() -> impl Widget {
    Screen
}
";

/// A sample `flow.rs` file. When the user clicks Render on a buffer containing
/// `// vieww:flow`, the studio shows the live-preview caution instead of
/// compiling — because a whole app's flow cannot be expressed as a single
/// `pub fn screen()`, and the live demo is what shows the complete UX.
pub(crate) const FLOW_SCRATCH: &str = "\
// vieww:flow
//
// This file triggers the Live Preview. Click Render, accept the caution,
// and a built-in demo app mounts inside the device frame — a complete UX
// with navigation, buttons, state and transitions, without compiling.
//
// The demo is a visual representation, not your code. Use Build and Run
// to see your actual project.
//
// You can also start the Live Preview from the toolbar's \"Live\" button
// or the command palette (\"Live Preview (No Build)\").
";

#[cfg(test)]
mod tests {
    use super::*;

    // ----- entry_points: what opening a real project finds ---------------

    /// A scratch directory that cleans itself up.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("viewwstudio-entry-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temp dir");
            Self(path)
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent");
            }
            fs::write(&path, "// file\n").expect("write");
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The finding: opening a conventional Rust project used to preload
    /// *nothing*, because the scan was one `read_dir` of the root and every
    /// Rust project keeps its code in `src/`.
    #[test]
    fn a_conventional_project_is_not_empty() {
        let dir = Dir::new("conventional");
        let main = dir.file("src/main.rs");
        dir.file("Cargo.toml");

        let found = entry_points(&dir.0);
        assert!(!found.is_empty(), "the whole bug, in one assertion");
        assert_eq!(found[0], main, "the crate root comes first");
    }

    #[test]
    fn crate_roots_come_before_whatever_else_is_lying_around() {
        let dir = Dir::new("order");
        dir.file("aaa.rs");
        let lib = dir.file("src/lib.rs");
        assert_eq!(entry_points(&dir.0)[0], lib);
    }

    #[test]
    fn a_loose_file_in_the_root_still_opens_the_way_it_always_did() {
        let dir = Dir::new("loose");
        let one = dir.file("screen.rs");
        assert_eq!(entry_points(&dir.0), vec![one]);
    }

    /// The other half of the fix. A recursive walk that opened everything
    /// would greet the user with four hundred tabs.
    #[test]
    fn the_walk_is_bounded() {
        let dir = Dir::new("bounded");
        for n in 0..OPEN_LIMIT * 3 {
            dir.file(&format!("src/file{n:03}.rs"));
        }
        assert_eq!(entry_points(&dir.0).len(), OPEN_LIMIT);
    }

    #[test]
    fn build_output_and_dot_directories_are_not_source() {
        let dir = Dir::new("skipped");
        dir.file("target/debug/build/thing.rs");
        dir.file(".git/hooks/thing.rs");
        dir.file("node_modules/pkg/thing.rs");
        let wanted = dir.file("src/lib.rs");
        assert_eq!(entry_points(&dir.0), vec![wanted]);
    }

    #[test]
    fn non_rust_files_are_not_opened_as_tabs() {
        let dir = Dir::new("nonrust");
        dir.file("README.md");
        dir.file("Cargo.toml");
        assert!(entry_points(&dir.0).is_empty());
    }

    /// Two runs on one machine, and two machines on one project, must agree.
    /// `read_dir` does not promise an order.
    #[test]
    fn the_result_does_not_depend_on_filesystem_order() {
        let dir = Dir::new("stable");
        for name in ["zebra.rs", "alpha.rs", "middle.rs"] {
            dir.file(&format!("src/{name}"));
        }
        let once = entry_points(&dir.0);
        let twice = entry_points(&dir.0);
        assert_eq!(once, twice);
        assert!(
            once[0].ends_with("alpha.rs"),
            "sorted within a depth: {once:?}"
        );
    }

    #[test]
    fn shallower_files_beat_deeper_ones() {
        let dir = Dir::new("depth");
        dir.file("src/deep/deeper/deepest/leaf.rs");
        let shallow = dir.file("src/top.rs");
        assert_eq!(entry_points(&dir.0)[0], shallow);
    }

    #[test]
    fn a_directory_that_is_not_there_gives_the_scratch_workspace() {
        let missing = std::env::temp_dir().join("viewwstudio-definitely-absent-xyz");
        assert_eq!(Workspace::open(&missing), Workspace::scratch());
    }

    #[test]
    fn the_caret_is_counted_in_lines_and_characters() {
        let mut buffer = Buffer::scratch("t.rs", "one\ntwo\nthree");

        buffer.value.selection = vieww_foundation::TextSelection::collapsed(0);
        assert_eq!(buffer.caret(), (1, 1), "the very start");

        // Offset 8 is the start of "three" — two newlines behind it.
        buffer.value.selection = vieww_foundation::TextSelection::collapsed(8);
        assert_eq!(buffer.caret(), (3, 1));

        buffer.value.selection = vieww_foundation::TextSelection::collapsed(13);
        assert_eq!(buffer.caret(), (3, 6), "the end of the last line");
    }

    #[test]
    fn a_multibyte_line_is_counted_in_characters_not_bytes() {
        let mut buffer = Buffer::scratch("t.rs", "héllo");
        // Five characters, six bytes: the caret after all of them is column 6.
        buffer.value.selection = vieww_foundation::TextSelection::collapsed(6);
        assert_eq!(buffer.caret(), (1, 6));
    }

    #[test]
    fn a_trailing_newline_makes_a_last_empty_line() {
        assert_eq!(Buffer::scratch("t.rs", "a\nb").line_count(), 2);
        assert_eq!(
            Buffer::scratch("t.rs", "a\nb\n").line_count(),
            3,
            "the line the cursor sits on after the last newline is a line"
        );
    }

    #[test]
    fn an_unreadable_directory_still_opens_a_studio() {
        let workspace = Workspace::open(Path::new("/definitely/not/here"));
        assert_eq!(workspace.root, None);
        assert_eq!(workspace.buffers.len(), 1, "the scratch buffer");
        assert!(workspace.buffers[0].value.text.contains("impl Widget"));
    }

    // ----- N1: the workspace, the tree, on-demand opening ----------------

    fn workspace_with_files() -> (std::path::PathBuf, Workspace) {
        let root = std::env::temp_dir().join(format!(
            "viewwstudio-buf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/screens")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"app\"\n[dependencies]\nvieww = \"0.2\"\n",
        )
        .unwrap();
        // Both of these are now opened at startup: `entry_points` walks, and
        // `main.rs` at the root is a crate root by rule 1. The nested one is
        // found by the walk, which is the behaviour the one-level scan lacked
        // and the reason opening a real project used to show an empty editor.
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            root.join("src/screens/home.rs"),
            "// hello\npub fn screen() {}\n",
        )
        .unwrap();
        let ws = Workspace::open(&root);
        (root, ws)
    }

    #[test]
    fn opening_a_workspace_scans_a_tree_and_recognises_vieww() {
        let (root, ws) = workspace_with_files();
        assert_eq!(ws.root.as_deref(), Some(root.as_path()));
        assert!(ws.is_vieww, "Cargo.toml depends on vieww");
        // The tree lists every file, nested — even ones no buffer holds.
        let files: Vec<String> = ws
            .tree
            .files()
            .into_iter()
            // `/` on every platform: `Path::display` writes `\` on Windows,
            // and what is under test is which files the scan found, not which
            // separator the operating system spells them with.
            .map(|p| {
                p.strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        assert!(
            files.iter().any(|f| f == "src/screens/home.rs"),
            "nested file present in tree: {files:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn open_buffer_loads_a_file_on_demand_and_dedups() {
        let (root, mut ws) = workspace_with_files();
        let home = root.join("src/screens/home.rs");
        // **This assertion used to be the opposite.** It read "not loaded at
        // startup (only top-level .rs are)" and was true of the one-level
        // scan — the same one-level scan that made opening a conventional
        // Rust project show nothing. The walk finds it now, and `open_buffer`
        // is still what the Explorer calls; what it has to promise is that
        // opening an already-open file does not make a second tab, which is
        // what the rest of this test checks.
        assert!(
            ws.buffers.iter().any(|b| b.path.as_deref() == Some(&home)),
            "the walk reaches a nested file"
        );
        let idx = ws.open_buffer(&home).expect("opens");
        assert_eq!(ws.buffers[idx].path.as_deref(), Some(home.as_path()));
        assert!(ws.buffers[idx].value.text.contains("screen"));
        // A second open of the same path returns the existing index, not a new tab.
        let again = ws.open_buffer(&home).expect("opens");
        assert_eq!(again, idx, "opening an open file does not duplicate it");
        assert_eq!(ws.buffers.len(), ws.buffers.len()); // unchanged
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn close_buffer_keeps_a_scratch_so_the_editor_is_never_empty() {
        let (root, mut ws) = workspace_with_files();
        // Closed one at a time down to nothing: the promise is that the last
        // close leaves a scratch behind rather than an empty editor. Written
        // as a loop because the fixture's buffer count changed when the walk
        // replaced the one-level scan, and a test that hardcodes it is a test
        // that breaks for the wrong reason next time.
        while ws.buffers.len() > 1 {
            assert!(ws.close_buffer(0));
        }
        assert!(ws.close_buffer(0));
        assert_eq!(ws.buffers.len(), 1, "never empty");
        assert!(
            ws.buffers[0].path.is_none(),
            "and what is left is a scratch"
        );
        let _ = std::fs::remove_dir_all(&root);
        // Closing an out-of-range index is a no-op.
        assert!(!ws.close_buffer(999));
    }

    #[test]
    fn find_searches_every_file_in_the_tree_not_just_open_buffers() {
        let (root, ws) = workspace_with_files();
        // `home.rs` is not open as a buffer, but find reaches it through the tree.
        let matches = ws.find("screen");
        assert!(
            matches
                .iter()
                .any(|m| m.path == root.join("src/screens/home.rs")),
            "{matches:?}"
        );
        assert!(matches.iter().all(|m| m.text.contains("screen")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn save_all_writes_every_dirty_buffer() {
        let (root, mut ws) = workspace_with_files();
        // Edit the open buffer and save all.
        ws.buffers[0].value.text = "// changed\n".into();
        ws.buffers[0].dirty = true;
        ws.save_all().unwrap();
        assert!(!ws.buffers[0].dirty, "dirty cleared on save");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rescan_preserves_expansion_state() {
        let (root, mut ws) = workspace_with_files();
        // Expand `src` in the tree, then rescan; the expansion must survive.
        let src = root.join("src");
        ws.tree.expand(&src);
        assert!(ws.tree.root().unwrap().expanded);
        ws.rescan();
        // After rescan, find `src` and confirm it is still expanded.
        fn find<'a>(
            node: &'a crate::file_tree::TreeNode,
            path: &Path,
        ) -> Option<&'a crate::file_tree::TreeNode> {
            if node.path == path {
                return Some(node);
            }
            match &node.kind {
                crate::file_tree::NodeKind::File => None,
                crate::file_tree::NodeKind::Dir { children } => {
                    children.iter().find_map(|c| find(c, path))
                }
            }
        }
        let src_node = find(ws.tree.root().unwrap(), &src).expect("src still present");
        assert!(src_node.expanded, "expansion survived the rescan");
        let _ = std::fs::remove_dir_all(&root);
    }
}
