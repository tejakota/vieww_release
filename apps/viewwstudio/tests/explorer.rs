//! The file operations that were written, tested, and called by nothing.
//!
//! `file_tree::rename` and `delete_to_trash` had **zero callers** before the
//! context menu existed. These tests are about the wire, not the syscalls: the
//! syscalls were always fine.

use std::path::PathBuf;

use vieww_element::Runtime;
use viewwstudio::state::{NameKind, TabScope};
use viewwstudio::Studio;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "viewwstudio-explorer-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("write");
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn studio_in(dir: &Dir) -> Studio {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.open_workspace(&dir.0);
    studio
}

// ---------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------

/// The separator check is the important one. A "name" containing a `/` is a
/// path, and letting one through turns "rename this file" into "move it
/// somewhere the user did not look at" — including out of the workspace.
#[test]
fn a_name_that_is_really_a_path_is_refused() {
    assert!(Studio::name_error("main.rs").is_none());
    assert!(Studio::name_error("../escape.rs").is_some());
    assert!(Studio::name_error("nested/file.rs").is_some());
    assert!(Studio::name_error("back\\slash.rs").is_some());
    assert!(Studio::name_error("").is_some());
    assert!(Studio::name_error("   ").is_some());
    assert!(Studio::name_error(".").is_some());
    assert!(Studio::name_error("..").is_some());
}

#[test]
fn the_prompt_validates_as_you_type_rather_than_after_the_click() {
    let dir = Dir::new("validate");
    let studio = studio_in(&dir);
    studio.ask_for_name(NameKind::NewFile, &dir.0);

    studio.set_prompt_name("a/b.rs".to_owned());
    assert!(studio.name_prompt.get().expect("open").error.is_some());

    studio.set_prompt_name("ab.rs".to_owned());
    assert!(studio.name_prompt.get().expect("open").error.is_none());
}

#[test]
fn renaming_pre_fills_the_current_name_and_creating_does_not() {
    let dir = Dir::new("prefill");
    let file = dir.file("thing.rs", "fn main() {}\n");
    let studio = studio_in(&dir);

    studio.ask_for_name(NameKind::Rename, &file);
    assert_eq!(studio.name_prompt.get().expect("open").text, "thing.rs");

    // Empty on purpose: a pre-filled name is one people accept without
    // reading, and a project full of `untitled.rs` is what that produces.
    studio.ask_for_name(NameKind::NewFile, &dir.0);
    assert_eq!(studio.name_prompt.get().expect("open").text, "");
}

/// "New File" belongs *inside* a folder and *beside* a file. Getting this
/// backwards is the single most annoying thing a file tree can do.
#[test]
fn a_new_file_lands_beside_a_file_and_inside_a_folder() {
    let dir = Dir::new("container");
    let file = dir.file("existing.rs", "");
    std::fs::create_dir_all(dir.0.join("sub")).expect("subdir");
    let studio = studio_in(&dir);

    studio.ask_for_name(NameKind::NewFile, &file);
    assert_eq!(studio.name_prompt.get().expect("open").target, dir.0);

    studio.ask_for_name(NameKind::NewFile, &dir.0.join("sub"));
    assert_eq!(
        studio.name_prompt.get().expect("open").target,
        dir.0.join("sub")
    );
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

#[test]
fn renaming_moves_the_file() {
    let dir = Dir::new("rename");
    let file = dir.file("old.rs", "fn main() {}\n");
    let studio = studio_in(&dir);

    studio.ask_for_name(NameKind::Rename, &file);
    studio.set_prompt_name("new.rs".to_owned());
    studio.confirm_name_prompt();

    assert!(!file.exists(), "the old name is gone");
    assert!(dir.0.join("new.rs").exists(), "and the new one is there");
    assert!(studio.name_prompt.get().is_none(), "the prompt closed");
}

/// A rename that leaves the open tab pointing at a path that no longer exists
/// turns the next save into a file the user did not ask for, under the old
/// name, beside the new one.
#[test]
fn the_open_buffer_follows_the_rename() {
    let dir = Dir::new("follow");
    let file = dir.file("before.rs", "fn main() {}\n");
    let studio = studio_in(&dir);
    studio.open_path(file.clone());

    studio.ask_for_name(
        NameKind::Rename,
        &std::fs::canonicalize(&file).expect("real"),
    );
    studio.set_prompt_name("after.rs".to_owned());
    studio.confirm_name_prompt();

    let buffer = studio
        .buffers
        .get()
        .iter()
        .find(|b| b.name == "after.rs")
        .cloned()
        .expect("the tab was renamed too");
    assert!(buffer.path.expect("has a path").ends_with("after.rs"));
}

#[test]
fn renaming_onto_something_that_exists_is_refused_and_says_so() {
    let dir = Dir::new("collide");
    let one = dir.file("one.rs", "");
    dir.file("two.rs", "");
    let studio = studio_in(&dir);

    studio.ask_for_name(NameKind::Rename, &one);
    studio.set_prompt_name("two.rs".to_owned());
    studio.confirm_name_prompt();

    let prompt = studio.name_prompt.get().expect("still open");
    assert!(prompt.error.is_some(), "refused with a reason");
    assert!(one.exists(), "and nothing moved");
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// To trash, not `unlink`. That is what makes saying yes survivable.
#[test]
fn deleting_moves_the_file_into_the_workspace_trash() {
    let dir = Dir::new("trash");
    let file = dir.file("doomed.rs", "fn main() {}\n");
    let studio = studio_in(&dir);

    studio.ask_to_delete(std::fs::canonicalize(&file).expect("real"));
    assert_eq!(studio.pending_delete_name().as_deref(), Some("doomed.rs"));
    studio.confirm_delete();

    assert!(!file.exists(), "gone from where it was");
    let trash = dir.0.join(".viewwstudio-trash");
    assert!(trash.is_dir(), "and moved into the trash, not unlinked");
    let found = std::fs::read_dir(&trash)
        .expect("trash readable")
        .filter_map(Result::ok)
        .any(|stamp| stamp.path().join("doomed.rs").exists());
    assert!(found, "the file is in there");
}

/// Leaving it open means the next save recreates the file the user just
/// deleted.
#[test]
fn deleting_closes_the_tab_that_was_over_it() {
    let dir = Dir::new("closetab");
    let file = dir.file("doomed.rs", "fn main() {}\n");
    let studio = studio_in(&dir);
    studio.open_path(file.clone());
    let real = std::fs::canonicalize(&file).expect("real");
    assert!(studio
        .buffers
        .get()
        .iter()
        .any(|b| b.path.as_deref() == Some(real.as_path())));

    studio.ask_to_delete(real.clone());
    studio.confirm_delete();

    assert!(!studio
        .buffers
        .get()
        .iter()
        .any(|b| b.path.as_deref() == Some(real.as_path())));
    assert!(
        !studio.buffers.get().is_empty(),
        "the editor is never empty"
    );
}

#[test]
fn cancelling_a_delete_leaves_the_file_alone() {
    let dir = Dir::new("cancel");
    let file = dir.file("safe.rs", "");
    let studio = studio_in(&dir);
    studio.ask_to_delete(file.clone());
    studio.cancel_delete();
    assert!(studio.pending_delete_name().is_none());
    assert!(file.exists());
}

// ---------------------------------------------------------------------------
// Bulk tab closing
// ---------------------------------------------------------------------------

/// A bulk close is one click from an accident, and the accident it would
/// otherwise cause is losing work that was never named in a dialog.
#[test]
fn a_bulk_close_never_takes_a_tab_with_unsaved_edits() {
    let dir = Dir::new("bulk");
    let a = dir.file("a.rs", "a\n");
    let b = dir.file("b.rs", "b\n");
    let c = dir.file("c.rs", "c\n");
    let studio = studio_in(&dir);
    for path in [&a, &b, &c] {
        studio.open_path(path.clone());
    }
    // Dirty the tab that is open, then move to a different one — so the
    // unsaved buffer is squarely *in* the scope of "close others". With it
    // active, the scope would exclude it and the test would prove nothing.
    studio.edit(vieww_foundation::TextEditingValue::new("edited"));
    studio.active_buffer.set(0);
    let dirty_before = studio.buffers.get().iter().filter(|b| b.dirty).count();
    assert!(dirty_before > 0, "the fixture has something to protect");

    studio.close_tabs(TabScope::Others);

    let dirty_after = studio.buffers.get().iter().filter(|b| b.dirty).count();
    assert_eq!(dirty_after, dirty_before, "nothing dirty was closed");
    let notice = studio.notice.get().unwrap_or_default();
    assert!(
        notice.contains("unsaved"),
        "and the user was told what was kept back: {notice}"
    );
}

#[test]
fn closing_to_the_right_leaves_everything_to_the_left() {
    let dir = Dir::new("right");
    for name in ["a.rs", "b.rs", "c.rs"] {
        dir.file(name, "x\n");
    }
    let studio = studio_in(&dir);
    let all = studio.buffers.get().len();
    assert!(all >= 3, "the walk opened them: {all}");
    studio.active_buffer.set(0);

    studio.close_tabs(TabScope::ToTheRight);

    assert_eq!(studio.buffers.get().len(), 1);
    assert_eq!(studio.active_buffer.get(), 0);
}

#[test]
fn closing_everything_still_leaves_an_editor() {
    let dir = Dir::new("empty");
    dir.file("only.rs", "x\n");
    let studio = studio_in(&dir);
    studio.close_tabs(TabScope::Saved);
    assert_eq!(studio.buffers.get().len(), 1);
    assert!(studio.buffers.get()[0].path.is_none(), "a scratch");
}

// ---------------------------------------------------------------------------
// Drag and drop
// ---------------------------------------------------------------------------

/// The framework has delivered dropped files since before the studio existed
/// and nothing here had ever read them.
#[test]
fn dropping_files_opens_them_as_tabs() {
    let dir = Dir::new("drop");
    let a = dir.file("one.rs", "1\n");
    let b = dir.file("two.rs", "2\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    let before = studio.buffers.get().len();

    studio.accept_dropped(&[a, b]);

    assert_eq!(studio.buffers.get().len(), before + 2);
}

#[test]
fn dropping_a_folder_opens_it_as_the_workspace() {
    let dir = Dir::new("dropdir");
    dir.file("main.rs", "fn main() {}\n");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    assert!(studio.root.get().is_none());

    studio.accept_dropped(std::slice::from_ref(&dir.0));

    assert!(
        studio.root.get().is_some(),
        "the folder became the workspace"
    );
}

/// Picking the first would be a guess about which project the user meant.
#[test]
fn dropping_two_folders_asks_rather_than_guessing() {
    let one = Dir::new("two-a");
    let two = Dir::new("two-b");
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);

    studio.accept_dropped(&[one.0.clone(), two.0.clone()]);

    assert!(studio.root.get().is_none(), "neither was chosen");
    let notice = studio.notice.get().unwrap_or_default();
    assert!(notice.contains("one at a time"), "{notice}");
}

#[test]
fn dropping_nothing_does_nothing() {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    studio.accept_dropped(&[]);
    assert!(studio.notice.get().is_none());
}

// ---------------------------------------------------------------------------
// Welcome
// ---------------------------------------------------------------------------

#[test]
fn a_studio_with_nothing_to_come_back_to_opens_on_the_welcome() {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    assert!(studio.should_welcome(), "a scratch studio, first launch");
}

#[test]
fn a_studio_that_restored_a_workspace_opens_on_the_work() {
    let dir = Dir::new("restored");
    dir.file("main.rs", "fn main() {}\n");
    let studio = studio_in(&dir);
    assert!(
        !studio.should_welcome(),
        "putting a welcome in front of someone mid-project is how they stop reading them"
    );
}

// ---------------------------------------------------------------------------
// About
// ---------------------------------------------------------------------------

#[test]
fn the_about_report_names_the_version_and_where_the_settings_live() {
    let report = viewwstudio::about::report();
    assert!(report.contains(viewwstudio::about::VERSION));
    assert!(report.contains("Data"));
}
