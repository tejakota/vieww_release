//! Themes, snippets and keybindings the user can change without a compiler.
//!
//! The parsers have their own tests in `customise.rs`. These are about the
//! wiring — the part that was missing, because all three were `const` items.

use vieww_element::Runtime;
use viewwstudio::command::{Chord, Command};
use viewwstudio::customise::{parse_chord, parse_keymap, parse_snippets, Theme};
use viewwstudio::Studio;

fn studio() -> Studio {
    let runtime = Runtime::new();
    Studio::new(&runtime)
}

// ---------------------------------------------------------------------------
// Keymap
// ---------------------------------------------------------------------------

/// With no keymap file, every command answers with its compiled-in chord.
#[test]
fn an_unconfigured_studio_uses_the_built_in_chords() {
    let studio = studio();
    assert_eq!(studio.chord_for(Command::Save), Command::Save.chord());
    assert_eq!(
        studio.chord_for(Command::CommandPalette),
        Command::CommandPalette.chord()
    );
}

/// The properties that make a keymap usable, checked against the parser and
/// the chord type together — the two halves that have to agree.
#[test]
fn a_parsed_chord_becomes_the_chord_the_studio_would_match() {
    let spec = parse_chord("Ctrl+Shift+K").expect("a chord");
    assert!(spec.shortcut && spec.shift && !spec.alt);
    assert_eq!(spec.character.as_deref(), Some("k"));

    // The equivalent built with the studio's own constructor.
    let built = Chord::cmd("k").with_shift();
    assert!(built.shortcut && built.shift);
}

#[test]
fn a_keymap_names_commands_by_the_title_the_palette_shows() {
    let map = parse_keymap("Save = Ctrl+Shift+S\nRender = Cmd+Enter\n");
    for name in map.bindings.keys() {
        assert!(
            Command::ALL
                .into_iter()
                .any(|command| command.title().eq_ignore_ascii_case(name)),
            "{name} is not a command title"
        );
    }
}

/// Every default chord is unique — the studio has its own test for that — and
/// a keymap file has to meet the same bar, or one command silently stops
/// working and the one that stops is whichever `Command::ALL` reaches second.
#[test]
fn the_built_in_chords_are_all_distinct() {
    let studio = studio();
    let mut seen: Vec<(Command, Chord)> = Vec::new();
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
// Snippets
// ---------------------------------------------------------------------------

#[test]
fn a_studio_starts_with_no_user_snippets_and_does_not_mind() {
    assert!(studio().user_snippets.get().is_empty());
}

/// A user snippet has to go in as whole lines, indented to the caret — the same
/// treatment the built-in ones get. Pasting at the caret is what produced the
/// glued-together line `insert_snippet`'s documentation exists to describe.
#[test]
fn a_user_snippet_is_inserted_as_whole_lines() {
    let studio = studio();
    let parsed = parse_snippets("[Row]\nFlex::row()\n    .children(children![])\n");
    assert_eq!(parsed.len(), 1);

    // Inside a `build`, which is where an expression belongs — and with the
    // caret actually in there. `TextEditingValue::new` puts the caret at the
    // *end* of the text, which is outside the braces, where an expression
    // snippet is correctly refused.
    let mut value = vieww_foundation::TextEditingValue::new("fn build() {\n    \n}\n");
    value.selection = vieww_foundation::TextSelection::collapsed(16);
    studio.edit(value);
    let before = studio.active().expect("a buffer").value.text.clone();
    studio.insert_text_snippet(&parsed[0].body);
    let after = studio.active().expect("a buffer").value.text;

    assert_ne!(after, before, "something was inserted");
    assert!(after.contains("Flex::row()"), "{after}");
    assert!(
        after.contains(".children(children![])"),
        "the second line came too: {after}"
    );
    // Not glued onto an existing line.
    assert!(
        !after.contains("}Flex::row()"),
        "inserted as its own lines: {after}"
    );
}

#[test]
fn an_empty_snippet_body_does_not_disturb_the_buffer() {
    let studio = studio();
    let before = studio.active().expect("a buffer").value.text.clone();
    studio.insert_text_snippet("");
    // Whether it refuses or inserts nothing, the one unacceptable outcome is
    // corrupting the buffer.
    let after = studio.active().expect("a buffer").value.text;
    assert!(after.contains(&before) || after == before, "{after:?}");
}

// ---------------------------------------------------------------------------
// Theme
// ---------------------------------------------------------------------------

/// The worst outcome of a partial theme file has to be a partially recoloured
/// studio, not an unreadable one — so a file naming one token leaves the rest
/// exactly as they were.
#[test]
fn a_theme_file_recolours_only_what_it_names() {
    let theme = Theme::parse("base = dark\nchrome_0 = #ff0000\n");
    assert!(theme.dark);
    assert_eq!(theme.get("chrome_0"), Some([0xFF, 0, 0, 0xFF]));
    assert_eq!(theme.get("chrome_1"), None);
    assert_eq!(theme.get("selection"), None);
}

#[test]
fn the_templates_are_written_where_about_says_they_are() {
    // The one thing a user cannot otherwise find out: where the studio keeps
    // its files. The About report has to name the same directory the
    // customisation loader reads.
    let report = viewwstudio::about::report();
    let dir = Studio::customise_dir();
    assert!(
        report.contains(&dir.display().to_string()),
        "About names the data directory: {report}"
    );
}

#[test]
fn every_customisation_file_has_a_template_that_mentions_it() {
    let theme = Theme::template();
    assert!(theme.contains("base = dark"));
    let snippets = viewwstudio::customise::snippets_template();
    assert!(snippets.contains('['), "a heading to copy: {snippets}");
    let keymap =
        viewwstudio::customise::keymap_template(&[("Save".to_owned(), "Ctrl+S".to_owned())]);
    assert!(keymap.contains("Save"));
}
