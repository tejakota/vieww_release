//! Replace across the workspace: the plan, the refusals, and the write.
//!
//! Workspace *search* was real. `replace_one` and `replace_all` operated on one
//! buffer, so renaming a symbol across a project was manual. The reason this
//! took a preview step rather than a button is in `find::Plan`: there is no
//! undo across files.

use std::path::PathBuf;

use vieww_element::Runtime;
use viewwstudio::command::Command;
use viewwstudio::Studio;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("viewwstudio-replace-{name}-{}", std::process::id()));
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

fn looking_for(studio: &Studio, needle: &str, replacement: &str) {
    studio.find_query.set(needle.to_owned());
    studio.find_replacement.set(replacement.to_owned());
}

#[test]
fn a_plan_counts_every_file_and_every_match_before_writing_anything() {
    let dir = Dir::new("plan");
    let a = dir.file("a.rs", "let widget = 1; let widget = 2;\n");
    let b = dir.file("b.rs", "let widget = 3;\n");
    dir.file("c.rs", "nothing here\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "widget", "gadget");

    studio.plan_workspace_replace();

    let plan = studio.replace_plan.get().expect("planned");
    assert_eq!(plan.total(), 3);
    assert_eq!(plan.files.len(), 2);
    // And nothing has been written.
    assert!(std::fs::read_to_string(&a).unwrap().contains("widget"));
    assert!(std::fs::read_to_string(&b).unwrap().contains("widget"));
}

#[test]
fn confirming_writes_every_file_in_the_plan() {
    let dir = Dir::new("apply");
    let a = dir.file("a.rs", "let widget = 1; let widget = 2;\n");
    let b = dir.file("b.rs", "let widget = 3;\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "widget", "gadget");

    studio.plan_workspace_replace();
    studio.confirm_workspace_replace();

    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "let gadget = 1; let gadget = 2;\n"
    );
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "let gadget = 3;\n");
    assert!(studio.replace_plan.get().is_none(), "the dialog closed");
}

/// Forwards shifts every range after the first by the difference in length,
/// which is the classic way a replace-all quietly corrupts a file.
#[test]
fn replacing_with_a_longer_string_does_not_corrupt_the_later_matches() {
    let dir = Dir::new("longer");
    let a = dir.file("a.rs", "x x x x\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "x", "yyyy");

    studio.plan_workspace_replace();
    studio.confirm_workspace_replace();

    assert_eq!(
        std::fs::read_to_string(&a).unwrap(),
        "yyyy yyyy yyyy yyyy\n"
    );
}

#[test]
fn cancelling_writes_nothing() {
    let dir = Dir::new("cancel");
    let a = dir.file("a.rs", "let widget = 1;\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "widget", "gadget");

    studio.plan_workspace_replace();
    studio.cancel_workspace_replace();

    assert!(studio.replace_plan.get().is_none());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "let widget = 1;\n");
}

/// The answer to "why did three of my thirty files not change" has to arrive
/// with the offer, not afterwards.
#[test]
fn a_file_with_unsaved_edits_is_named_in_the_plan_and_left_alone() {
    let dir = Dir::new("dirty");
    let a = dir.file("a.rs", "let widget = 1;\n");
    dir.file("b.rs", "let widget = 2;\n");
    let studio = studio_in(&dir);
    studio.open_path(a.clone());
    studio.edit(vieww_foundation::TextEditingValue::new(
        "let widget = 1; // mine\n",
    ));
    looking_for(&studio, "widget", "gadget");

    studio.plan_workspace_replace();

    let plan = studio.replace_plan.get().expect("planned");
    assert_eq!(plan.skipped.len(), 1, "the dirty one is named: {plan:?}");
    assert!(plan.skipped[0].1.contains("unsaved"));

    studio.confirm_workspace_replace();
    // Untouched on disk, because the user's unsaved version is the live one.
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "let widget = 1;\n");
}

#[cfg(unix)]
#[test]
fn a_read_only_file_is_named_in_the_plan_rather_than_failing_during_it() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Dir::new("readonly");
    let locked = dir.file("locked.rs", "let widget = 1;\n");
    dir.file("open.rs", "let widget = 2;\n");
    let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
    permissions.set_mode(0o444);
    std::fs::set_permissions(&locked, permissions).unwrap();

    let studio = studio_in(&dir);
    looking_for(&studio, "widget", "gadget");
    studio.plan_workspace_replace();

    let plan = studio.replace_plan.get().expect("planned");
    assert!(
        plan.skipped
            .iter()
            .any(|(_, why)| why.contains("read-only")),
        "{plan:?}"
    );
}

#[test]
fn an_open_clean_buffer_is_reloaded_so_the_editor_is_not_showing_stale_text() {
    let dir = Dir::new("reload");
    let a = dir.file("a.rs", "let widget = 1;\n");
    let studio = studio_in(&dir);
    studio.open_path(a);
    looking_for(&studio, "widget", "gadget");

    studio.plan_workspace_replace();
    studio.confirm_workspace_replace();

    let shown = studio.active().expect("a buffer").value.text;
    assert!(shown.contains("gadget"), "the editor caught up: {shown:?}");
}

#[test]
fn replacing_with_no_query_says_so_rather_than_matching_everything() {
    let dir = Dir::new("empty");
    dir.file("a.rs", "anything\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "   ", "x");

    studio.run(Command::ReplaceInFiles);

    assert!(studio.replace_plan.get().is_none());
    assert!(studio
        .notice
        .get()
        .unwrap_or_default()
        .contains("find first"));
}

#[test]
fn replacing_with_no_workspace_says_what_is_missing() {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    looking_for(&studio, "x", "y");

    studio.run(Command::ReplaceInFiles);

    assert!(studio.replace_plan.get().is_none());
    let notice = studio.notice.get().unwrap_or_default();
    assert!(notice.contains("folder open"), "{notice}");
}

#[test]
fn no_matches_is_reported_and_raises_no_dialog() {
    let dir = Dir::new("nomatch");
    dir.file("a.rs", "nothing of interest\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "zebra", "giraffe");

    studio.plan_workspace_replace();

    assert!(studio.replace_plan.get().is_none());
    assert!(studio
        .notice
        .get()
        .unwrap_or_default()
        .contains("No matches"));
}

#[test]
fn the_plan_lists_files_relative_to_the_workspace_root() {
    let dir = Dir::new("relative");
    dir.file("a.rs", "widget\n");
    let studio = studio_in(&dir);
    looking_for(&studio, "widget", "gadget");
    studio.plan_workspace_replace();

    let plan = studio.replace_plan.get().expect("planned");
    let root = studio.root.get();
    let lines = plan.lines(root.as_deref());
    assert!(
        lines.iter().any(|line| line.starts_with("a.rs")),
        "{lines:?}"
    );
}
