//! What the studio does when it is used, checked without a window.
//!
//! `tests/shell.rs` asks whether a region is *there*. This file asks whether
//! pressing a thing *does* the thing — the undo stack, the shortcut table, the
//! find bar, the command dispatcher, the tab strip.
//!
//! Every one of these drives [`Studio`] directly rather than through a
//! `FrameDriver`, and that is the point of the studio holding all of its state
//! above the tree: a keystroke is a function call on a value, so thirty
//! shortcuts cost thirty microseconds instead of thirty windows.
//!
//! The one thing that genuinely needs a driver — that a key travelling through
//! the real focus/bubble path reaches the shortcut layer — is the last test
//! here, and it is deliberately only one: it proves the wiring, and the other
//! forty prove the behaviour.

use std::time::Duration;

use vieww_element::Runtime;
use vieww_foundation::{
    KeyEvent, LogicalKey, Modifiers, NamedKey, Size, TargetPlatform, TextSelection,
};
use vieww_render::FrameDriver;
use viewwstudio::buffer::{Buffer, Workspace};
use viewwstudio::command::{Chord, Command, Menu};
use viewwstudio::state::{PaletteEntry, PanelTab, Platform, PreviewState, Studio, View};
use viewwstudio::Shell;

/// A studio holding one scratch file, with Linux keyboard conventions so the
/// assertions can name `Ctrl` rather than branching on the runner.
fn studio() -> (Runtime, Studio) {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    (runtime, studio)
}

/// The same, holding `files` as `(name, text)` pairs.
fn studio_with(files: &[(&str, &str)]) -> (Runtime, Studio) {
    let runtime = Runtime::new();
    let workspace = Workspace::of_buffers(
        files
            .iter()
            .map(|(name, text)| Buffer::scratch(*name, *text))
            .collect(),
    );
    let studio = Studio::with_workspace(&runtime, workspace).with_host(TargetPlatform::Linux);
    (runtime, studio)
}

/// Type `text` into the active buffer one character at a time, as the editor's
/// `on_changed` would.
fn type_text(studio: &Studio, text: &str) {
    for character in text.chars() {
        let mut value = studio.active().expect("a buffer").value;
        value.insert(&character.to_string());
        studio.edit(value);
    }
}

/// Replace the active buffer's whole text in one go.
fn set_text(studio: &Studio, text: &str) {
    let mut value = studio.active().expect("a buffer").value;
    value.text = text.to_string();
    value.selection = TextSelection::collapsed(0);
    studio.edit(value);
}

fn ctrl(character: &str) -> KeyEvent {
    KeyEvent::character(character, Duration::ZERO).with_modifiers(Modifiers::CONTROL)
}

fn ctrl_shift(character: &str) -> KeyEvent {
    KeyEvent::character(character, Duration::ZERO)
        .with_modifiers(Modifiers::CONTROL.union(Modifiers::SHIFT))
}

/// Send a key to the shortcut layer's dispatcher, as the render object would.
fn press(studio: &Studio, event: &KeyEvent) -> bool {
    viewwstudio::ui::shortcuts::dispatch(studio, event, studio.host)
}

// ===================== undo and redo ====================================

#[test]
fn undo_takes_back_a_word_and_redo_puts_it_back() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    type_text(&studio, "hello");
    assert_eq!(studio.active().unwrap().value.text, "hello");

    press(&studio, &ctrl("z"));
    assert_eq!(
        studio.active().unwrap().value.text,
        "",
        "five keystrokes come back as one undo"
    );

    press(&studio, &ctrl_shift("z"));
    assert_eq!(studio.active().unwrap().value.text, "hello");
}

#[test]
fn each_buffer_keeps_its_own_history() {
    let (_runtime, studio) = studio_with(&[("a.rs", ""), ("b.rs", "")]);

    type_text(&studio, "aaa");
    studio.active_buffer.set(1);
    type_text(&studio, "bbb");

    // Undo on b.rs must not reach into a.rs.
    press(&studio, &ctrl("z"));
    assert_eq!(studio.buffers.get()[1].value.text, "");
    assert_eq!(
        studio.buffers.get()[0].value.text,
        "aaa",
        "an undo stack owned by the editor would have taken this instead"
    );

    studio.active_buffer.set(0);
    press(&studio, &ctrl("z"));
    assert_eq!(studio.buffers.get()[0].value.text, "");
}

#[test]
fn undo_with_nothing_to_undo_does_not_clear_the_buffer() {
    let (_runtime, studio) = studio_with(&[("a.rs", "important")]);
    press(&studio, &ctrl("z"));
    assert_eq!(
        studio.active().unwrap().value.text,
        "important",
        "the worst possible reading of an empty history"
    );
}

#[test]
fn undo_is_offered_only_when_it_would_do_something() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    assert!(!studio.can_run(Command::Undo));
    assert!(!studio.can_run(Command::Redo));

    type_text(&studio, "x");
    assert!(studio.can_run(Command::Undo));
    assert!(!studio.can_run(Command::Redo));

    studio.run(Command::Undo);
    assert!(studio.can_run(Command::Redo));
}

#[test]
fn the_caret_readout_follows_an_undo() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    type_text(&studio, "one\ntwo");
    assert_eq!(studio.caret.get(), (2, 4));

    // Every step back, the readout has to be the buffer's own answer. A status
    // bar that keeps the position from before an undo is reporting a document
    // that is no longer showing — and the bug is invisible until somebody
    // clicks a diagnostic and lands in the wrong place.
    while studio.can_run(Command::Undo) {
        studio.run(Command::Undo);
        assert_eq!(
            studio.caret.get(),
            studio.active().unwrap().caret(),
            "the readout and the buffer disagreed"
        );
    }
    assert_eq!(
        studio.caret.get(),
        (1, 1),
        "back at the top of an empty file"
    );
}

// ===================== the shortcut table ===============================

#[test]
fn a_plain_character_is_left_for_the_editor() {
    let (_runtime, studio) = studio();
    assert!(
        !press(&studio, &KeyEvent::character("s", Duration::ZERO)),
        "typing `s` must reach the buffer rather than saving the file"
    );
}

#[test]
fn the_shortcut_layer_consumes_what_it_handles() {
    let (_runtime, studio) = studio();
    assert!(
        press(&studio, &ctrl("j")),
        "an unconsumed shortcut falls through and the editor inserts a control \
         character"
    );
}

#[test]
fn a_chord_whose_command_can_do_nothing_is_still_consumed() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    assert!(!studio.can_run(Command::Undo));
    assert!(
        press(&studio, &ctrl("z")),
        "letting ⌘Z through to the buffer would insert U+001A"
    );
}

#[test]
fn the_view_shortcuts_move_the_panel_and_the_sidebar() {
    let (_runtime, studio) = studio();

    let panel = studio.panel_open.get();
    press(&studio, &ctrl("j"));
    assert_eq!(studio.panel_open.get(), !panel);

    let width = studio.sidebar_width.get();
    press(&studio, &ctrl("b"));
    assert_eq!(studio.sidebar_width.get(), 0.0, "collapsed");
    press(&studio, &ctrl("b"));
    assert!(studio.sidebar_width.get() > 0.0, "and back");
    let _ = width;
}

#[test]
fn the_platform_shortcuts_switch_the_preview_and_mark_it_dirty() {
    let (_runtime, studio) = studio();
    studio.dirty.set(false);

    press(&studio, &ctrl("2"));
    assert_eq!(studio.platform.get(), Platform::Android);
    assert!(
        studio.dirty.get(),
        "a platform switch is an edit — plan §4.7 — and the Render dot has to \
         say so"
    );

    press(&studio, &ctrl("3"));
    assert_eq!(studio.platform.get(), Platform::Desktop);
}

#[test]
fn every_chord_in_the_table_reaches_the_command_it_names() {
    // Not a spot check: the whole table, so a chord added without a command —
    // or a command whose chord collides with another's — fails here rather
    // than in somebody's hands.
    for command in Command::ALL {
        let Some(chord) = command.chord() else {
            continue;
        };
        let event = synthesise(&chord);
        assert_eq!(
            Command::for_key(&event, TargetPlatform::Linux),
            Some(command),
            "{} does not reach {command:?}",
            chord.describe(TargetPlatform::Linux)
        );
    }
}

/// The key event a chord describes.
fn synthesise(chord: &Chord) -> KeyEvent {
    let mut modifiers = Modifiers::NONE;
    if chord.shortcut {
        modifiers = modifiers.union(Modifiers::CONTROL);
    }
    if chord.shift {
        modifiers = modifiers.union(Modifiers::SHIFT);
    }
    if chord.alt {
        modifiers = modifiers.union(Modifiers::ALT);
    }
    KeyEvent {
        key: chord.key.clone(),
        state: vieww_foundation::KeyState::Down,
        repeat: false,
        modifiers,
        timestamp: Duration::ZERO,
    }
}

// ===================== find and replace =================================

#[test]
fn find_walks_the_matches_and_wraps() {
    let (_runtime, studio) = studio_with(&[("a.rs", "foo bar foo baz foo")]);
    studio.find_query.set("foo".into());
    studio.find_open.set(true);

    assert_eq!(studio.find_matches().len(), 3);

    studio.run(Command::FindNext);
    assert_eq!(studio.find_index.get(), 1);
    studio.run(Command::FindNext);
    assert_eq!(studio.find_index.get(), 2);
    studio.run(Command::FindNext);
    assert_eq!(
        studio.find_index.get(),
        0,
        "a find that goes quiet at the bottom reads as no more matches"
    );

    studio.run(Command::FindPrevious);
    assert_eq!(studio.find_index.get(), 2);
}

#[test]
fn finding_selects_the_match_in_the_editor() {
    let (_runtime, studio) = studio_with(&[("a.rs", "one two three")]);
    studio.find_query.set("two".into());
    studio.open_find(false);

    let value = studio.active().unwrap().value;
    assert_eq!(value.selection.start(), 4);
    assert_eq!(value.selection.end(), 7);
    assert_eq!(
        value.selection.range().slice(&value.text),
        "two",
        "a find that only moves the caret leaves the user hunting for the match"
    );
}

#[test]
fn opening_find_seeds_the_query_from_the_selection() {
    let (_runtime, studio) = studio_with(&[("a.rs", "alpha beta")]);
    let mut value = studio.active().unwrap().value;
    value.selection = TextSelection::new(6, 10);
    studio.edit(value);

    studio.run(Command::Find);
    assert_eq!(studio.find_query.get(), "beta");
}

#[test]
fn replace_all_is_one_undo() {
    let (_runtime, studio) = studio_with(&[("a.rs", "x x x")]);
    studio.find_query.set("x".into());
    studio.find_replacement.set("yy".into());
    studio.replace_all();

    assert_eq!(studio.active().unwrap().value.text, "yy yy yy");
    studio.run(Command::Undo);
    assert_eq!(
        studio.active().unwrap().value.text,
        "x x x",
        "a replace-all somebody cannot take back is a replace-all nobody presses"
    );
}

#[test]
fn replace_one_leaves_the_rest_alone() {
    let (_runtime, studio) = studio_with(&[("a.rs", "cat cat cat")]);
    studio.find_query.set("cat".into());
    studio.find_replacement.set("dog".into());
    studio.find_index.set(1);
    studio.replace_current();
    assert_eq!(
        studio.active().unwrap().value.text,
        "cat dog cat",
        "the middle one, and only the middle one"
    );
}

#[test]
fn the_case_switch_changes_what_is_found() {
    let (_runtime, studio) = studio_with(&[("a.rs", "Screen screen")]);
    studio.find_query.set("screen".into());
    assert_eq!(studio.find_matches().len(), 2);

    studio.find_case_sensitive.set(true);
    assert_eq!(studio.find_matches().len(), 1);
}

#[test]
fn escape_shuts_one_thing_at_a_time() {
    let (_runtime, studio) = studio();
    studio.find_open.set(true);
    studio.palette_open.set(true);

    press(&studio, &KeyEvent::named(NamedKey::Escape, Duration::ZERO));
    assert!(!studio.palette_open.get(), "the innermost goes first");
    assert!(studio.find_open.get(), "and the one behind it stays");

    press(&studio, &KeyEvent::named(NamedKey::Escape, Duration::ZERO));
    assert!(!studio.find_open.get());
}

// ===================== the palette ======================================

#[test]
fn the_palette_filters_and_runs_what_it_shows() {
    let (_runtime, studio) = studio();
    press(&studio, &ctrl("p"));
    assert!(studio.palette_open.get());

    // `>` is the command prefix. The palette has four modes now and bare text
    // is the file one, so a query without it filters files and finds no
    // command at all — which is the behaviour, not a regression.
    studio.palette_query.set(">panel".into());
    let results = studio.palette_results();
    assert!(results.contains(&PaletteEntry::Command(Command::TogglePanel)));

    let panel = studio.panel_open.get();
    studio.palette_index.set(
        results
            .iter()
            .position(|entry| *entry == PaletteEntry::Command(Command::TogglePanel))
            .expect("in the list"),
    );
    studio.palette_accept();

    assert_eq!(studio.panel_open.get(), !panel);
    assert!(
        !studio.palette_open.get(),
        "a palette that stays open over what it just did covers the result"
    );
}

#[test]
fn the_arrows_move_the_palette_highlight_and_wrap() {
    let (_runtime, studio) = studio();
    // Through the command, so the palette opens on commands. Setting
    // `palette_open` by hand opens on the default mode — files — and a scratch
    // workspace has no tree, so the list is empty and there is nothing for an
    // arrow to move between.
    studio.run(Command::CommandPalette);
    let count = studio.palette_results().len();
    assert!(
        count > 1,
        "the command list is what the arrows move through"
    );

    press(
        &studio,
        &KeyEvent::named(NamedKey::ArrowDown, Duration::ZERO),
    );
    assert_eq!(studio.palette_index.get(), 1);

    press(&studio, &KeyEvent::named(NamedKey::ArrowUp, Duration::ZERO));
    press(&studio, &KeyEvent::named(NamedKey::ArrowUp, Duration::ZERO));
    assert_eq!(
        studio.palette_index.get(),
        count - 1,
        "wrapped past the top"
    );
}

#[test]
fn the_palette_owns_enter_while_it_is_open() {
    let (_runtime, studio) = studio();
    let enter = KeyEvent::named(NamedKey::Enter, Duration::ZERO);
    assert!(
        !press(&studio, &enter),
        "a shut palette leaves Enter for the editor to make a newline with"
    );

    studio.palette_open.set(true);
    assert!(
        press(&studio, &enter),
        "an open one runs the highlighted row"
    );
}

// ===================== buffers ==========================================

#[test]
fn new_file_never_reuses_a_name() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    studio.run(Command::NewFile);
    studio.run(Command::NewFile);

    let names: Vec<String> = studio
        .buffers
        .get()
        .iter()
        .map(|b| b.name.clone())
        .collect();
    assert_eq!(names, ["a.rs", "untitled-1.rs", "untitled-2.rs"]);
    assert_eq!(
        studio.active_buffer.get(),
        2,
        "a new file you have to go and find is a new file nobody uses"
    );
}

#[test]
fn the_last_buffer_closes_and_the_pane_says_so() {
    // **This test used to assert the opposite**, on the reasoning that "a
    // studio with no buffer has an empty pane and no way back". The first half
    // was true and the second half is what changed: the pane is a real empty
    // state now — "No file open", with New file and Open folder on it — so
    // refusing to close the last tab is no longer protecting anybody. It was
    // reported as a bug in exactly those words: the last tab would not close.
    let (_runtime, studio) = studio_with(&[("only.rs", "")]);
    assert!(studio.can_run(Command::CloseTab));

    studio.run(Command::CloseTab);

    assert!(
        studio.buffers.get().is_empty(),
        "the last tab did not close"
    );
    assert!(
        !studio.can_run(Command::CloseTab),
        "and now there is nothing to close"
    );

    // The way back, which is what makes closing it safe.
    studio.run(Command::NewFile);
    assert_eq!(studio.buffers.get().len(), 1);
}

#[test]
fn closing_a_tab_takes_its_diagnostics_with_it() {
    let (_runtime, studio) = studio_with(&[("a.rs", ""), ("b.rs", "")]);
    studio.diagnostics.set(std::rc::Rc::new(vec![
        diagnostic("a.rs"),
        diagnostic("b.rs"),
    ]));

    studio.active_buffer.set(0);
    studio.run(Command::CloseTab);

    let left: Vec<String> = studio
        .diagnostics
        .get()
        .iter()
        .map(|d| d.file.clone())
        .collect();
    assert_eq!(
        left,
        ["b.rs"],
        "a problem pointing at a file that is not open jumps nowhere"
    );
}

fn diagnostic(file: &str) -> viewwstudio::state::Diagnostic {
    viewwstudio::state::Diagnostic {
        file: file.to_string(),
        severity: viewwstudio::state::Severity::Error,
        code: "E0000".into(),
        message: "something".into(),
        help: None,
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
    }
}

#[test]
fn the_tab_shortcuts_wrap_in_both_directions() {
    let (_runtime, studio) = studio_with(&[("a.rs", ""), ("b.rs", ""), ("c.rs", "")]);
    assert_eq!(studio.active_buffer.get(), 0);

    press(&studio, &ctrl("]"));
    assert_eq!(studio.active_buffer.get(), 1);

    press(&studio, &ctrl("["));
    press(&studio, &ctrl("["));
    assert_eq!(studio.active_buffer.get(), 2, "wrapped backwards past zero");
}

#[test]
fn saving_a_buffer_that_was_never_on_disk_asks_where_to_put_it() {
    // **This used to assert a failure message**, and reporting the failure
    // where the user could see it was the right fix *at the time*: it beat
    // writing "could not save" to a stderr nobody running a GUI is reading.
    // But the failure itself was the bug. A buffer with no path is not a save
    // that cannot happen, it is a save that does not know where to go — so it
    // asks, and the answer is a file on disk.
    //
    // With no folder open there is still nowhere to put it, and that case is
    // covered in `tests/saving.rs` along with the writing itself.
    let (_runtime, studio) = studio_with(&[("scratch.rs", "x")]);
    studio.root.set(Some(std::env::temp_dir()));
    set_text(&studio, "changed");

    assert!(
        studio.can_run(Command::Save),
        "Save is greyed out on a buffer that has never been written"
    );
    studio.run(Command::Save);

    let prompt = studio.name_prompt.get().expect("Save asks for a name");
    assert_eq!(prompt.kind, viewwstudio::state::NameKind::SaveAs);
    assert!(
        studio
            .output
            .get()
            .iter()
            .all(|line| !line.contains("could not save")),
        "it reported a failure instead of asking"
    );
}

#[test]
fn a_real_file_round_trips_through_save_and_revert() {
    let directory = std::env::temp_dir().join("viewwstudio-interaction-save");
    std::fs::create_dir_all(&directory).expect("a temp directory");
    let path = directory.join("screen.rs");
    std::fs::write(&path, "original").expect("write");

    let runtime = Runtime::new();
    let workspace = Workspace::open(&directory);
    let studio = Studio::with_workspace(&runtime, workspace).with_host(TargetPlatform::Linux);
    assert_eq!(studio.active().unwrap().value.text, "original");

    set_text(&studio, "edited");
    studio.run(Command::Save);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
    assert!(
        !studio.active().unwrap().dirty,
        "a saved buffer stops showing the unsaved dot"
    );

    set_text(&studio, "edited again");
    studio.run(Command::RevertFile);
    assert_eq!(studio.active().unwrap().value.text, "edited");
    assert!(
        !studio.can_run(Command::Undo),
        "the entries behind a revert describe a document that is gone"
    );

    std::fs::remove_dir_all(&directory).ok();
}

// ===================== the compile state machine =========================

#[test]
fn render_without_a_toolchain_says_why_rather_than_going_quiet() {
    let (_runtime, studio) = studio();
    studio.run(Command::Render);

    assert_eq!(studio.preview.get(), PreviewState::Failed);
    assert!(
        studio
            .output
            .get()
            .iter()
            .any(|l| l.contains("no usable rustc") || l.contains("was built without a toolchain")),
        "output was {:?}",
        studio.output.get()
    );
    assert!(
        !studio.can_run(Command::Render),
        "the Render button has to explain itself rather than be pressable and \
         do nothing"
    );
}

#[test]
fn polling_with_no_job_running_is_a_no_op() {
    let (_runtime, studio) = studio();
    assert!(
        !studio.poll_compile(),
        "the hook runs every frame and almost every frame there is nothing there"
    );
    assert!(!studio.is_compiling());
}

#[test]
fn cancel_is_only_offered_while_something_is_running() {
    let (_runtime, studio) = studio();
    assert!(!studio.can_run(Command::CancelRender));
}

#[test]
fn the_status_bar_shows_no_render_time_until_there_has_been_a_render() {
    let (_runtime, studio) = studio();
    assert_eq!(
        studio.last_render.get(),
        None,
        "this cell used to read `1.92s` from a literal on a studio that had \
         never compiled anything"
    );
    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(!dump.contains("1.92s"));
}

/// The bug this pins: while a build is running, the bottom-left status light
/// used to keep showing the last preview's state ("Failed" red, "Rendered"
/// green) — silently, because nothing in the studio's signal set connected
/// `build_state` to the status bar. A user pressing Build would see "Failed"
/// stay lit through the whole build, which reads as "the build failed" rather
/// than "the build is running". The fix overrides the preview state with the
/// build's own status while `build_state` is `Running`.
#[test]
fn the_status_bar_shows_building_while_a_build_is_running() {
    use viewwstudio::builds;
    let (_runtime, studio) = studio();

    // Mark a build as in flight, the way `start_build` would.
    studio
        .build_state
        .set(builds::State::Running(builds::Kind::Build));

    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });

    // The status bar's word for the build's running state. "Building…" is
    // what `StatusBar::build` writes when `build_state.busy()` is true.
    assert!(
        dump.contains("Building…"),
        "the status bar must say Building… while a build is running, got: {dump}"
    );
}

/// **The defect a user reported, pinned.**
///
/// Press Render on `src/main.rs` — which has no `screen()`, so the preview
/// fails. Then press Build, and wait five and a half minutes. The Output panel
/// ends with `Build finished in 336.8s`; the light in the bottom-left corner
/// is red and reads `Failed`. Both cells are telling the truth about their own
/// pipeline, and together they say the build failed. Everybody who saw it read
/// it that way, because the build is what they had just sat through.
///
/// The old contract — "when the build ends the status bar returns to the
/// preview's own state" — is what produced it, and this test used to assert
/// exactly that. The contract now is **the most recent thing wins**, tracked
/// by `Studio::activity`, and the words name their pipeline so that even a
/// wrong arbitration cannot be ambiguous.
#[test]
fn a_finished_build_outranks_a_stale_failed_render() {
    use viewwstudio::builds;
    let (_runtime, studio) = studio();

    // A render failed a while ago and nothing has cleared it.
    studio.set_preview(PreviewState::Failed);
    let before = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        before.contains("Render failed"),
        "with no build in the session the light is the preview's, and says so: {before}"
    );

    // Now a build runs and succeeds.
    studio
        .build_state
        .set(builds::State::Running(builds::Kind::Build));
    studio.activity.set(viewwstudio::state::Activity::Build);
    let during = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(during.contains("Building…"), "{during}");

    studio.build_state.set(builds::State::Succeeded {
        kind: builds::Kind::Build,
        warnings: 0,
    });
    let after = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        !after.contains("Building…"),
        "Building… must not stick after the build ends: {after}"
    );
    assert!(
        after.contains("Build succeeded"),
        "the build is the most recent thing that happened, so it is what the \
         light reports — this is the whole bug: {after}"
    );
    assert!(
        !after.contains("Render failed"),
        "and the render's five-minute-old verdict is not what the corner of \
         the window shouts: {after}"
    );
}

/// The other direction: a render *after* a build is what the light reports.
///
/// Without this the fix above would be a different stuck cell — one that said
/// "Build succeeded" over a render that had just failed.
#[test]
fn a_render_after_a_build_takes_the_light_back() {
    use viewwstudio::builds;
    let (_runtime, studio) = studio();

    studio.build_state.set(builds::State::Succeeded {
        kind: builds::Kind::Build,
        warnings: 0,
    });
    studio.activity.set(viewwstudio::state::Activity::Build);
    assert!(vieww_widget::debug_tree(Shell {
        studio: studio.clone()
    })
    .contains("Build succeeded"));

    studio.set_preview(PreviewState::Failed);
    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        dump.contains("Render failed") && !dump.contains("Build succeeded"),
        "the newest verdict is the one shown, in both directions: {dump}"
    );
}

/// A build in flight beats everything, including a render that just failed.
///
/// The one case where recency is *not* the rule: an operation still running is
/// more interesting than any finished one, so it holds the light.
#[test]
fn a_running_build_outranks_even_a_newer_render_verdict() {
    use viewwstudio::builds;
    let (_runtime, studio) = studio();

    studio
        .build_state
        .set(builds::State::Running(builds::Kind::BuildAndRun));
    // A background auto-render finishes and fails while the build runs, which
    // sets `activity` back to `Preview`.
    studio.set_preview(PreviewState::Failed);

    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        dump.contains("Building…"),
        "a build still running holds the light: {dump}"
    );
}

/// A cancelled build is not a failed one.
#[test]
fn a_cancelled_build_does_not_read_as_a_failure() {
    use viewwstudio::builds;
    let (_runtime, studio) = studio();

    studio.build_state.set(builds::State::Failed {
        kind: builds::Kind::Build,
        status: viewwstudio::task::Status::Cancelled,
    });
    studio.activity.set(viewwstudio::state::Activity::Build);

    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        dump.contains("Build cancelled") && !dump.contains("Build failed"),
        "stopping a build on purpose is not a failure to report: {dump}"
    );
}

// ===================== menus and views ==================================

#[test]
fn running_a_command_shuts_the_menu_it_came_from() {
    let (_runtime, studio) = studio();
    studio.menu_open.set(Some(Menu::View));
    studio.run(Command::TogglePanel);
    assert_eq!(
        studio.menu_open.get(),
        None,
        "a menu left hanging over the thing it just changed"
    );
}

#[test]
fn show_problems_moves_the_sidebar_and_the_panel_together() {
    let (_runtime, studio) = studio();
    studio.view.set(View::Explorer);
    studio.panel_open.set(false);

    studio.run(Command::ShowProblems);
    assert_eq!(studio.view.get(), View::Problems);
    assert_eq!(studio.panel_tab.get(), PanelTab::Problems);
    assert!(studio.panel_open.get());
}

// ===================== the wiring, once =================================

#[test]
fn a_key_reaches_the_shortcut_layer_through_the_real_focus_path() {
    // The one end-to-end test. Everything above drives `Studio` directly, which
    // is fast and proves the behaviour; this proves the *plumbing* — that the
    // layer is registered, that it takes focus, and that a key travelling the
    // framework's own focus-and-bubble path arrives at it.
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);

    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    viewwstudio::seed_focus(&mut driver);

    assert!(
        driver.focused().is_some(),
        "with nothing focused there is nothing for a key to bubble out from, \
         and no shortcut would ever arrive"
    );

    let before = studio.panel_open.get();
    let handled = driver.handle_key(&ctrl("j"));
    assert!(handled, "Ctrl+J was not consumed by anything in the tree");
    assert_eq!(
        studio.panel_open.get(),
        !before,
        "the key reached the layer but the layer did not act on it"
    );
}

#[test]
fn an_ordinary_character_is_not_swallowed_by_the_layer() {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell { studio });
    driver.draw_frame();
    viewwstudio::seed_focus(&mut driver);

    let event = KeyEvent {
        key: LogicalKey::Character("q".into()),
        state: vieww_foundation::KeyState::Down,
        repeat: false,
        modifiers: Modifiers::NONE,
        timestamp: Duration::ZERO,
    };
    assert!(
        !driver.handle_key(&event),
        "the shortcut layer must decline a plain letter so it can reach a field"
    );
}

// ===================== what the preview claims ===========================

#[test]
fn a_failed_first_render_does_not_claim_a_previous_one() {
    // The pane's one promise is that it never blanks (plan §2.2): a failed
    // compile keeps the last good frame and says so. When there has *never*
    // been a good frame there is nothing to keep, and the badge must not appear
    // — a dimmed empty phone under "showing last successful render" is the pane
    // asserting a render that never happened.
    let (_runtime, studio) = studio();
    studio.run(Command::Render);
    assert_eq!(studio.preview.get(), PreviewState::Failed);
    assert!(
        studio.preview_screen.get().is_none(),
        "nothing has ever compiled"
    );

    let dump = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });
    assert!(
        !dump.contains("showing last successful render"),
        "there was no last successful render to show"
    );
    assert!(
        dump.contains("No render yet"),
        "and the frame says so plainly instead"
    );
}

#[test]
fn the_stale_badge_appears_once_there_is_something_to_be_stale() {
    // The other half: with a screen mounted, a later failure *does* earn the
    // badge, because now the frame on screen really is older than the buffer.
    // Checked through `PreviewState::is_stale` plus the screen, which is the
    // pair the pane reads — mounting a real `Preview` needs a compiled dylib
    // and belongs in `tests/pipeline.rs`.
    assert!(
        PreviewState::Failed.is_stale(),
        "Failed is the state that means the frame is behind the buffer"
    );
    for state in [
        PreviewState::Empty,
        PreviewState::Compiling,
        PreviewState::Rendered,
    ] {
        assert!(!state.is_stale(), "{state:?} is not a stale frame");
    }
}

#[test]
fn the_status_bar_does_not_put_the_frameworks_name_on_the_studios_version() {
    let (_runtime, studio) = studio();
    let dump = vieww_widget::debug_tree(Shell { studio });
    assert!(
        !dump.contains(&format!("vieww {}", env!("CARGO_PKG_VERSION"))),
        "this cell reads the studio's own CARGO_PKG_VERSION; labelling it \
         `vieww` claims a framework version it never asked for"
    );
    assert!(dump.contains(&format!("studio {}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn a_long_diagnostic_does_not_run_off_the_edge_of_the_sidebar() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    studio.view.set(View::Problems);
    studio
        .diagnostics
        .set(std::rc::Rc::new(vec![viewwstudio::state::Diagnostic {
            file: "a.rs".into(),
            severity: viewwstudio::state::Severity::Error,
            code: "timeout".into(),
            message: "rustc was still running after 10s and was killed".into(),
            help: None,
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        }]));

    // The *sidebar* only. The bottom Problems panel is full-width and shows the
    // message whole, which is right — the narrow column is the one that was
    // cutting text off mid-word with nothing to say it had.
    let sidebar = vieww_widget::debug_tree(viewwstudio::ui::sidebar::Sidebar {
        studio: studio.clone(),
    });
    assert!(
        !sidebar.contains("rustc was still running after 10s and was killed"),
        "the untruncated message is wider than a 248-point column, and what \
         reached the screen was `…after 10s and w`"
    );
    assert!(
        sidebar.contains('…'),
        "a cut message has to admit it was cut"
    );

    // And the panel, which has the room, still says the whole thing.
    let panel = vieww_widget::debug_tree(viewwstudio::ui::panel::Panel { studio });
    assert!(panel.contains("rustc was still running after 10s and was killed"));
}

// ---------------------------------------------------------------------------
// N6 — the editor's own edits, driven through `Studio::edit` exactly as the
// text field's `on_changed` drives them.
// ---------------------------------------------------------------------------

/// Type one character at the caret, producing the value the field would.
fn type_char(studio: &Studio, character: char) {
    let mut value = studio.active().expect("a buffer").value;
    value.insert(&character.to_string());
    studio.edit(value);
}

fn text_of(studio: &Studio) -> String {
    studio.active().expect("a buffer").value.text
}

fn caret_of(studio: &Studio) -> usize {
    studio.active().expect("a buffer").value.selection.extent
}

#[test]
fn typing_an_opening_bracket_brings_its_closer() {
    let (_runtime, studio) = studio_with(&[("a.rs", "f")]);
    // The field starts with the caret at the end of the text.
    type_char(&studio, '(');
    assert_eq!(text_of(&studio), "f()");
    assert_eq!(caret_of(&studio), 2, "the caret sits between the pair");
}

#[test]
fn return_keeps_the_indent_of_the_line_it_left() {
    let (_runtime, studio) = studio_with(&[("a.rs", "fn f() {\n    let a = 1;")]);
    type_char(&studio, '\n');
    assert_eq!(text_of(&studio), "fn f() {\n    let a = 1;\n    ");
    assert_eq!(caret_of(&studio), text_of(&studio).len());
}

#[test]
fn return_after_an_opener_goes_one_level_deeper() {
    let (_runtime, studio) = studio_with(&[("a.rs", "fn f() {")]);
    type_char(&studio, '\n');
    assert_eq!(text_of(&studio), "fn f() {\n    ");
}

/// The comforts are a setting, and turning them off has to actually turn them
/// off — an editor that rewrites what you typed and will not stop is worse than
/// one that never did.
#[test]
fn the_comforts_can_be_turned_off() {
    let (_runtime, studio) = studio_with(&[("a.rs", "f")]);
    studio.comforts.set(false);
    type_char(&studio, '(');
    assert_eq!(text_of(&studio), "f(", "no closer when the setting is off");
}

/// Ordinary typing must cost nothing and change nothing about the text.
#[test]
fn ordinary_characters_pass_straight_through() {
    let (_runtime, studio) = studio_with(&[("a.rs", "")]);
    for character in "let a = 1;".chars() {
        type_char(&studio, character);
    }
    assert_eq!(text_of(&studio), "let a = 1;");
}

#[test]
fn toggle_comment_comments_and_uncomments_the_line() {
    let (_runtime, studio) = studio_with(&[("a.rs", "    let a = 1;")]);
    studio.run(Command::ToggleComment);
    assert_eq!(text_of(&studio), "    // let a = 1;");
    studio.run(Command::ToggleComment);
    assert_eq!(text_of(&studio), "    let a = 1;");
}

#[test]
fn indent_and_outdent_move_the_line_by_one_level() {
    let (_runtime, studio) = studio_with(&[("a.rs", "let a = 1;")]);
    // A selection covering the line, as Tab with a selection would have.
    let mut value = studio.active().unwrap().value;
    value.selection = TextSelection {
        base: 0,
        extent: value.text.len(),
        affinity: value.selection.affinity,
    };
    studio.edit(value);

    studio.run(Command::Indent);
    assert_eq!(text_of(&studio), "    let a = 1;");
    studio.run(Command::Outdent);
    assert_eq!(text_of(&studio), "let a = 1;");
}

/// Every comfort has to be one undo step, or undo stops landing where a person
/// expects and the feature becomes a nuisance.
#[test]
fn a_comfort_is_undoable_as_one_step() {
    let (_runtime, studio) = studio_with(&[("a.rs", "f")]);
    type_char(&studio, '(');
    assert_eq!(text_of(&studio), "f()");
    studio.run(Command::Undo);
    assert_eq!(text_of(&studio), "f");
}

// ---------------------------------------------------------------------------
// N3/N4 — building, and refusing to.
// ---------------------------------------------------------------------------

/// The preview works on a single file; a build needs a project. Pressing Build
/// on a scratch studio has to explain that rather than doing nothing.
#[test]
fn build_is_refused_with_a_reason_when_no_folder_is_open() {
    let (_runtime, studio) = studio();
    assert!(studio.build_refusal().is_some());
    assert!(
        !studio.can_run(Command::Build),
        "and the menu item is greyed"
    );

    studio.run(Command::Build);

    let output = studio.output.get();
    assert!(
        output.iter().any(|line| line.contains("Cargo project")),
        "the refusal has to reach the panel: {output:?}"
    );
    assert!(
        studio.panel_open.get(),
        "and the panel has to be open to read it"
    );
    assert_eq!(studio.panel_tab.get(), PanelTab::Output);
}

/// Cancel is only offered while something is running, so it can never look like
/// the way to stop a build that is not happening.
#[test]
fn cancel_build_is_only_available_while_a_build_runs() {
    let (_runtime, studio) = studio();
    assert!(!studio.can_run(Command::CancelBuild));
}

#[test]
fn the_trash_button_empties_the_output_panel() {
    let (_runtime, studio) = studio();
    studio.append_output(vec!["something happened".into()]);
    assert!(studio.can_run(Command::ClearOutput));

    studio.run(Command::ClearOutput);
    assert!(studio.output.get().is_empty());
    assert!(
        !studio.can_run(Command::ClearOutput),
        "and there is nothing left to clear"
    );
}

/// Plan 2 §4.3: preview renders a widget, build produces an application. The
/// clearest way for the UI never to blur them is for them to be different
/// commands in different menus with different chords.
#[test]
fn preview_and_build_are_never_the_same_control() {
    assert_ne!(Command::Render.menu(), Command::Build.menu());
    assert_ne!(Command::Render.chord(), Command::Build.chord());
    assert_eq!(Command::Render.menu(), Menu::Render);
    assert_eq!(Command::Build.menu(), Menu::Build);
}
