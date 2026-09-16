//! The completion list, from the studio's side.
//!
//! The transport and the parsing have their own tests in `lsp.rs`. These are
//! about the state machine around them — which is where a completion list is
//! usually wrong: it shows a stale list, or it inserts over the wrong range.

use vieww_element::Runtime;
use viewwstudio::command::Command;
use viewwstudio::lsp::Completion;
use viewwstudio::Studio;

fn studio() -> Studio {
    let runtime = Runtime::new();
    Studio::new(&runtime)
}

fn item(label: &str, insert: &str) -> Completion {
    Completion {
        label: label.to_owned(),
        insert: insert.to_owned(),
        detail: "fn".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// The word under the caret
// ---------------------------------------------------------------------------

#[test]
fn the_word_at_the_caret_is_the_identifier_it_is_inside() {
    assert_eq!(Studio::word_at_caret("let value = 1;", 7), 4..9);
    assert_eq!(
        Studio::word_at_caret("let value = 1;", 4),
        4..9,
        "at the start"
    );
    assert_eq!(
        Studio::word_at_caret("let value = 1;", 9),
        4..9,
        "at the end"
    );
}

/// Empty when the caret is not in one, which is what makes "complete here"
/// mean "offer everything" rather than "offer nothing".
#[test]
fn a_caret_between_words_selects_nothing() {
    // Offset 10 is the `=`, with a space on either side — no identifier
    // character on either hand. (Offset 3 would be the end of `let`, which is
    // very much inside a word.)
    let range = Studio::word_at_caret("let value = 1;", 10);
    assert!(range.is_empty(), "{range:?}");
}

#[test]
fn underscores_and_digits_are_part_of_an_identifier() {
    assert_eq!(Studio::word_at_caret("my_var2 = 1", 4), 0..7);
}

#[test]
fn an_offset_past_the_end_is_clamped() {
    let range = Studio::word_at_caret("abc", 99);
    assert!(range.end <= 3, "{range:?}");
}

#[test]
fn an_empty_document_has_no_word() {
    assert!(Studio::word_at_caret("", 0).is_empty());
}

// ---------------------------------------------------------------------------
// Moving and accepting
// ---------------------------------------------------------------------------

/// `move_completion` on a closed list must be a no-op, not a panic — the arrow
/// keys reach it on every frame the list is shut.
#[test]
fn moving_a_closed_list_does_nothing() {
    let studio = studio();
    studio.move_completion(1);
    studio.move_completion(-1);
    studio.accept_completion();
    assert!(studio.completion.get().is_none());
}

#[test]
fn the_highlight_wraps_in_both_directions() {
    let studio = studio();
    studio.open_completion_for_test(
        vec![
            item("one", "one"),
            item("two", "two"),
            item("three", "three"),
        ],
        0..0,
    );

    studio.move_completion(1);
    assert_eq!(studio.completion.get().expect("open").index, 1);

    studio.move_completion(-1);
    assert_eq!(studio.completion.get().expect("open").index, 0);

    studio.move_completion(-1);
    assert_eq!(
        studio.completion.get().expect("open").index,
        2,
        "up from the first wraps to the last"
    );

    studio.move_completion(1);
    assert_eq!(studio.completion.get().expect("open").index, 0, "and back");
}

#[test]
fn accepting_replaces_the_word_under_the_caret() {
    let studio = studio();
    studio.edit(vieww_foundation::TextEditingValue::new("let x = va"));
    studio.open_completion_for_test(vec![item("value", "value")], 8..10);

    studio.accept_completion();

    assert_eq!(
        studio.active().expect("a buffer").value.text,
        "let x = value"
    );
    assert!(studio.completion.get().is_none(), "the list closed");
}

#[test]
fn accepting_puts_the_caret_after_what_it_inserted() {
    let studio = studio();
    studio.edit(vieww_foundation::TextEditingValue::new("va;"));
    studio.open_completion_for_test(vec![item("value", "value")], 0..2);

    studio.accept_completion();

    let value = studio.active().expect("a buffer").value;
    assert_eq!(value.text, "value;");
    assert_eq!(value.selection.extent, 5, "just before the semicolon");
}

/// The range was recorded when the list opened, and the buffer can have shrunk
/// since — a reload, an undo, a replace-all. Slicing a string at an offset it
/// no longer has is a panic in the middle of typing.
#[test]
fn accepting_after_the_buffer_shrank_does_not_panic() {
    let studio = studio();
    studio.edit(vieww_foundation::TextEditingValue::new("a longer document"));
    studio.open_completion_for_test(vec![item("value", "value")], 10..16);

    studio.edit(vieww_foundation::TextEditingValue::new("tiny"));
    studio.accept_completion();

    // Whatever it did, it did not panic and it did not corrupt the buffer.
    assert!(!studio.active().expect("a buffer").value.text.is_empty());
}

#[test]
fn accepting_across_multi_byte_text_does_not_panic() {
    let studio = studio();
    studio.edit(vieww_foundation::TextEditingValue::new("héllo wörld"));
    // A range that starts and ends on real boundaries.
    studio.open_completion_for_test(vec![item("world", "world")], 7..13);
    studio.accept_completion();
    assert!(studio
        .active()
        .expect("a buffer")
        .value
        .text
        .contains("world"));
}

#[test]
fn closing_forgets_both_the_list_and_the_request_it_was_waiting_for() {
    let studio = studio();
    studio.open_completion_for_test(vec![item("one", "one")], 0..0);
    studio.close_completion();
    assert!(studio.completion.get().is_none());
}

/// Escape closes the completion list before anything else, because it is the
/// most recently opened thing on screen whenever it is open.
#[test]
fn escape_closes_the_completion_list_first() {
    let studio = studio();
    studio.palette_open.set(true);
    studio.open_completion_for_test(vec![item("one", "one")], 0..0);

    studio.escape();

    assert!(studio.completion.get().is_none(), "the list went");
    assert!(studio.palette_open.get(), "and the palette stayed");
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn completing_with_no_analyzer_says_so_rather_than_doing_nothing() {
    let studio = studio();
    studio.run(Command::Complete);
    let notice = studio.notice.get().unwrap_or_default();
    assert!(
        notice.contains("rust-analyzer") || notice.contains("save this buffer"),
        "{notice}"
    );
}

#[test]
fn hovering_with_no_analyzer_says_so_too() {
    let studio = studio();
    studio.run(Command::Hover);
    assert!(studio.notice.get().is_some());
}

#[test]
fn both_commands_are_in_the_palette() {
    for command in [Command::Complete, Command::Hover] {
        assert!(
            Command::ALL.contains(&command),
            "{} is not reachable from the palette",
            command.title()
        );
        assert!(!command.title().is_empty());
    }
}

/// Every chord in the table is still unique after two more were added.
#[test]
fn the_new_chords_do_not_collide() {
    let studio = studio();
    let mut seen: Vec<(Command, viewwstudio::command::Chord)> = Vec::new();
    for command in Command::ALL {
        let Some(chord) = studio.chord_for(command) else {
            continue;
        };
        if let Some((other, _)) = seen.iter().find(|(_, existing)| *existing == chord) {
            panic!(
                "{} and {} both want {}",
                command.title(),
                other.title(),
                chord.describe(studio.host)
            );
        }
        seen.push((command, chord));
    }
}

// ---------------------------------------------------------------------------
// The Run box
// ---------------------------------------------------------------------------

use viewwstudio::state::split_command;

/// Whitespace-separated with double quotes grouping, and *not* a shell.
#[test]
fn a_command_line_splits_the_way_the_docs_say_it_does() {
    assert_eq!(
        split_command("cargo test --lib"),
        Some(vec!["cargo".into(), "test".into(), "--lib".into()])
    );
    assert_eq!(
        split_command("ls \"my folder\""),
        Some(vec!["ls".into(), "my folder".into()]),
        "quotes group, so a path with a space is one argument"
    );
    assert_eq!(
        split_command("   spaced   out  "),
        Some(vec!["spaced".into(), "out".into()])
    );
    assert_eq!(split_command(""), Some(Vec::new()));
}

/// Guessing at unbalanced quotes produces a command the user did not write.
#[test]
fn an_unbalanced_quote_is_refused_rather_than_guessed_at() {
    assert_eq!(split_command("ls \"unfinished"), None);
}

/// An empty quoted string is an argument — `--message=""` is a real thing to
/// pass, and dropping it changes the command.
#[test]
fn an_empty_quoted_argument_survives() {
    assert_eq!(
        split_command("git commit -m \"\""),
        Some(vec![
            "git".into(),
            "commit".into(),
            "-m".into(),
            String::new()
        ])
    );
}

#[test]
fn running_with_no_workspace_says_what_is_missing() {
    let studio = studio();
    studio.run_command("cargo test");
    let notice = studio.notice.get().unwrap_or_default();
    assert!(notice.contains("folder open"), "{notice}");
    assert!(
        studio.run_history.get().is_empty(),
        "and nothing was recorded"
    );
}

#[test]
fn an_empty_command_does_nothing_at_all() {
    let studio = studio();
    studio.run_command("   ");
    assert!(studio.notice.get().is_none());
    assert!(studio.run_history.get().is_empty());
}

#[test]
fn the_run_tab_is_reachable() {
    assert!(Command::ALL.contains(&Command::ShowRun));
    studio().run(Command::ShowRun);
}
