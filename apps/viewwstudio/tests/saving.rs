//! Saving a file that has never been on disk, and closing the last tab.
//!
//! Two reported bugs, both of them the same shape: a command that was gated on
//! a condition that had nothing to do with whether it could be carried out.
//!
//! * **Save** required `path.is_some()`, so a `File > New File` buffer — the
//!   one kind that exists only in memory — could be typed into, filled from a
//!   snippet, and never written. The disabled reason said "open a folder
//!   first", which was not the problem and would not have fixed it.
//! * **Close Tab** required two open buffers, so the editor could never be
//!   empty and the last file could not be put away.

use std::path::PathBuf;

use vieww_element::Runtime;
use viewwstudio::command::Command;
use viewwstudio::state::{NameKind, Studio};

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("viewwstudio-saving-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        // Canonical, because `open_workspace` canonicalizes its root: on macOS
        // the temp directory is `/var/folders/…`, a symlink to
        // `/private/var/folders/…`, and every path the studio reports back is
        // the second spelling. Comparing against the first failed only there.
        Self(std::fs::canonicalize(&path).unwrap_or(path))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A studio with `dir` open as its workspace.
fn studio_in(dir: &Dir) -> (Runtime, Studio) {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.open_workspace(&dir.0);
    (runtime, studio)
}

/// Type `text` into the active buffer, as the field would.
fn type_into(studio: &Studio, text: &str) {
    let mut value = studio.active().expect("a buffer").value;
    value.text = text.to_owned();
    value.selection = vieww_foundation::TextSelection::collapsed(text.len());
    studio.edit(value);
}

// ---------------------------------------------------------------------------
// Saving a new file
// ---------------------------------------------------------------------------

#[test]
fn a_new_file_can_be_saved() {
    let dir = Dir::new("new-file");
    let (_runtime, studio) = studio_in(&dir);

    studio.run(Command::NewFile);
    type_into(&studio, "use vieww::prelude::*;\n");

    // The gate that was wrong. Dirty is the whole condition: a buffer with no
    // path is saved *differently*, not never.
    assert!(
        studio.can_run(Command::Save),
        "Save is greyed out on a new file, which is the bug"
    );

    studio.run(Command::Save);

    // Save on a pathless buffer asks where it should go rather than failing.
    let prompt = studio.name_prompt.get().expect("Save asks for a name");
    assert_eq!(prompt.kind, NameKind::SaveAs);
    assert!(
        !prompt.text.is_empty(),
        "pre-filled with the name the tab already shows: {prompt:?}"
    );

    studio.set_prompt_name("screen.rs".to_owned());
    studio.confirm_name_prompt();

    let written = dir.0.join("screen.rs");
    assert!(
        written.is_file(),
        "nothing was written to {}",
        written.display()
    );
    assert_eq!(
        std::fs::read_to_string(&written).expect("read back"),
        "use vieww::prelude::*;\n"
    );

    let buffer = studio.active().expect("still open");
    assert_eq!(buffer.path.as_deref(), Some(written.as_path()));
    assert_eq!(buffer.name, "screen.rs");
    assert!(!buffer.dirty, "saved and still marked unsaved");
}

#[test]
fn saving_a_new_file_keeps_its_undo_history() {
    // The reason this binds the existing buffer to a path rather than writing
    // the file and opening it: an open-and-replace would throw away everything
    // the user has typed since the buffer appeared, which is the entire
    // lifetime of a new file.
    let dir = Dir::new("history");
    let (_runtime, studio) = studio_in(&dir);

    studio.run(Command::NewFile);
    type_into(&studio, "first");
    type_into(&studio, "first second");
    studio.run(Command::Save);
    studio.set_prompt_name("kept.rs".to_owned());
    studio.confirm_name_prompt();

    assert!(
        studio.active().expect("a buffer").history.can_undo(),
        "the undo stack did not survive the save"
    );
}

#[test]
fn a_name_already_taken_is_refused_rather_than_overwritten() {
    let dir = Dir::new("collision");
    std::fs::write(dir.0.join("taken.rs"), "// somebody else's work\n").expect("seed");
    let (_runtime, studio) = studio_in(&dir);

    studio.run(Command::NewFile);
    type_into(&studio, "mine\n");
    studio.run(Command::Save);
    studio.set_prompt_name("taken.rs".to_owned());
    studio.confirm_name_prompt();

    assert_eq!(
        std::fs::read_to_string(dir.0.join("taken.rs")).expect("read"),
        "// somebody else's work\n",
        "the existing file was overwritten"
    );
    let prompt = studio.name_prompt.get().expect("the prompt stays open");
    assert!(prompt.error.is_some(), "and says why: {prompt:?}");
}

#[test]
fn a_saved_new_file_becomes_an_ordinary_save() {
    // Once it has a path, ⌘S is the plain write again — no second prompt.
    let dir = Dir::new("second-save");
    let (_runtime, studio) = studio_in(&dir);

    studio.run(Command::NewFile);
    type_into(&studio, "one\n");
    studio.run(Command::Save);
    studio.set_prompt_name("twice.rs".to_owned());
    studio.confirm_name_prompt();

    type_into(&studio, "two\n");
    studio.run(Command::Save);

    assert!(studio.name_prompt.get().is_none(), "asked for a name twice");
    assert_eq!(
        std::fs::read_to_string(dir.0.join("twice.rs")).expect("read"),
        "two\n"
    );
}

#[test]
fn with_no_folder_open_it_says_what_to_do() {
    // There is no platform file dialog in vieww, so there is nowhere honest to
    // put an unnamed buffer. What matters is that the studio says which of the
    // two things to do rather than silently doing nothing — which is what it
    // did before.
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.root.set(None);

    studio.run(Command::NewFile);
    type_into(&studio, "nowhere to go\n");
    studio.run(Command::Save);

    assert!(studio.name_prompt.get().is_none());
    let notice = studio.notice.get().expect("a message");
    assert!(
        notice.contains("Open a folder"),
        "the message does not say what to do: {notice:?}"
    );
}

// ---------------------------------------------------------------------------
// Closing the last tab
// ---------------------------------------------------------------------------

#[test]
fn the_last_tab_closes_and_leaves_an_empty_editor() {
    let dir = Dir::new("last-tab");
    std::fs::write(dir.0.join("only.rs"), "fn main() {}\n").expect("seed");
    let (_runtime, studio) = studio_in(&dir);
    studio.open_path(dir.0.join("only.rs"));

    // Everything but one.
    while studio.buffers.get().len() > 1 {
        studio.run(Command::CloseTab);
    }
    assert_eq!(studio.buffers.get().len(), 1);

    assert!(
        studio.can_run(Command::CloseTab),
        "Close Tab is greyed on the last tab, which is the bug"
    );
    studio.run(Command::CloseTab);

    assert!(
        studio.buffers.get().is_empty(),
        "the last tab did not close"
    );
    assert!(studio.active().is_none());
    // And the command is now genuinely unavailable, for the right reason.
    assert!(!studio.can_run(Command::CloseTab));
}

#[test]
fn an_empty_editor_can_be_filled_again() {
    // Closing everything must not be a dead end: the pane offers New file and
    // Open folder, and both have to work from nothing.
    let dir = Dir::new("refill");
    let (_runtime, studio) = studio_in(&dir);

    while !studio.buffers.get().is_empty() {
        studio.run(Command::CloseTab);
    }
    assert!(studio.active().is_none());

    studio.run(Command::NewFile);
    assert_eq!(studio.buffers.get().len(), 1, "New file did nothing");
    assert!(studio.active().is_some());
}

#[test]
fn closing_the_last_tab_drops_its_diagnostics() {
    // Diagnostics point at a file by name, and a problem list about a file
    // nobody has open is a list that navigates to nothing when clicked.
    let dir = Dir::new("diagnostics");
    std::fs::write(dir.0.join("broken.rs"), "fn main() { oops }\n").expect("seed");
    let (_runtime, studio) = studio_in(&dir);
    studio.open_path(dir.0.join("broken.rs"));

    let name = studio.active().expect("a buffer").name.clone();
    studio
        .diagnostics
        .set(std::rc::Rc::new(vec![viewwstudio::state::Diagnostic {
            file: name,
            severity: viewwstudio::state::Severity::Error,
            code: "E0425".to_owned(),
            message: "cannot find value `oops`".to_owned(),
            help: None,
            line: 1,
            column: 13,
            end_line: 1,
            end_column: 17,
        }]));

    while !studio.buffers.get().is_empty() {
        studio.run(Command::CloseTab);
    }

    assert!(
        studio.diagnostics.get().is_empty(),
        "diagnostics outlived the file they were about"
    );
}
