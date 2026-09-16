//! Themes, snippets and keybindings the user can change without a compiler.
//!
//! # The finding
//!
//! Three things in the studio could not be personalised at all, and the reason
//! they could not is the same in each case: they were `const` items.
//!
//! * **Two themes.** `StudioTheme::dark()` and `::light()` are constructors,
//!   and the derivation from a single `ColorScheme` is elegant — no value typed
//!   twice. There was no third, no theme file, no import, and no high-contrast
//!   option. The Tokens view could nudge a value and export it, and nothing
//!   read an exported set back in.
//! * **Eleven snippets.** `const SNIPPETS: [Snippet; 11]`. For a studio whose
//!   purpose is authoring widgets, the library of widget starting points being
//!   uneditable is a pointed limitation.
//! * **Eighty-plus compiled-in chords.** `Command::chord` is a `match`. The
//!   architecture is right — a `Chord` carries meaning rather than a platform's
//!   rendering of it, and a test enforces uniqueness — and there was no path
//!   for a user to change one. Anyone arriving from Vim, Emacs or a JetBrains
//!   keymap had no recourse.
//!
//! Taken with the packaging gap, that closed the loop: users could not
//! configure the studio, and could not rebuild it either, because rebuilding
//! needed the framework checkout.
//!
//! # One format, three files
//!
//! Everything here is the `key = value` format [`crate::settings`] already
//! uses, for the same reason: these are files people edit by hand, so nothing
//! in the parsers may fail. An unreadable line is skipped, an unknown key is
//! ignored, a bad value leaves its default — and, for the keymap, an override
//! that would collide with an existing chord is **refused and reported**
//! rather than silently shadowing a shortcut the user still expects to work.
//!
//! ```text
//! <data_dir>/theme.txt      colours, over the built-in dark or light
//! <data_dir>/snippets.txt   extra entries for the Snippets view
//! <data_dir>/keymap.txt     chord overrides, by command name
//! ```

use std::collections::BTreeMap;

use crate::settings::Record;

/// The file names, under [`crate::about::data_dir`].
pub const THEME_FILE: &str = "theme.txt";
pub const SNIPPETS_FILE: &str = "snippets.txt";
pub const KEYMAP_FILE: &str = "keymap.txt";

// ---------------------------------------------------------------------------
// Themes
// ---------------------------------------------------------------------------

/// Colour overrides, by token name, over one of the built-in themes.
///
/// An *override*, not a whole theme. A format that required every colour would
/// be a format nobody finishes filling in, and a half-filled one would produce
/// a studio with black text on a black panel. Starting from a built-in theme
/// and changing what you care about means the worst outcome of a partial file
/// is a partially recoloured studio that is still legible.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Which built-in to start from: `true` for dark.
    pub dark: bool,
    /// Token name to colour.
    pub colors: BTreeMap<String, [u8; 4]>,
}

impl Theme {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let record = Record::parse(text);
        let mut colors = BTreeMap::new();
        for key in TOKENS {
            if let Some(value) = record.get(key) {
                if let Some(color) = parse_hex(value) {
                    colors.insert((*key).to_owned(), color);
                }
                // A value that is not a colour is skipped, leaving the built-in
                // one. The alternative is a studio that will not start because
                // somebody typed `chrome_0 = darkish`.
            }
        }
        Self {
            // `base = light` starts from the light theme. Anything else, or
            // nothing, starts from dark — which is what the studio has always
            // opened as.
            dark: record.get("base") != Some("light"),
            colors,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// A colour by token name.
    #[must_use]
    pub fn get(&self, token: &str) -> Option<[u8; 4]> {
        self.colors.get(token).copied()
    }

    /// A starting point somebody can edit, with every token listed and
    /// commented out.
    ///
    /// Written on demand rather than shipped: a file full of the current
    /// theme's values is the difference between "themes are supported" and
    /// "themes are supported and here is how".
    #[must_use]
    pub fn template() -> String {
        let mut out = String::from(
            "# vieww Studio theme. Start from the dark or the light built-in\n\
             # and change only what you care about — anything not named here\n\
             # keeps its built-in value.\n\
             base = dark\n\n",
        );
        for token in TOKENS {
            out.push_str(&format!("# {token} = #000000\n"));
        }
        out
    }
}

/// Every colour a theme file may name.
///
/// The chrome depths and the two editor colours: the values `StudioTheme` holds
/// itself, rather than the whole `ColorScheme` beneath it. Overriding the
/// scheme is a larger surface and a much easier way to produce something
/// unreadable; this is the set the Tokens view already exposes.
pub const TOKENS: &[&str] = &[
    "window",
    "chrome_0",
    "chrome_1",
    "chrome_2",
    "chrome_3",
    "line",
    "selection",
    "caret",
    "gutter",
    "highlight",
];

/// `#rgb`, `#rrggbb` or `#rrggbbaa`, as RGBA bytes.
#[must_use]
pub fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let text = text.trim().strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(text.get(at..at + 2)?, 16).ok();
    match text.len() {
        3 => {
            // `#abc` is `#aabbcc`, the shorthand every stylesheet uses.
            let mut out = [0u8, 0, 0, 0xFF];
            for (index, character) in text.chars().enumerate() {
                let value = character.to_digit(16)?;
                #[expect(clippy::cast_possible_truncation, reason = "a hex digit is 0..=15")]
                let value = value as u8;
                out[index] = value * 17;
            }
            Some(out)
        }
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 0xFF]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Snippets
// ---------------------------------------------------------------------------

/// A user-defined snippet: a name, and the lines it inserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSnippet {
    pub name: String,
    pub body: String,
}

/// Parse a snippets file.
///
/// The format is a heading line and the lines under it, because a snippet is
/// **several lines of code** and `key = value` cannot hold one:
///
/// ```text
/// [Centred column]
/// Flex::column()
///     .main_axis_alignment(MainAxisAlignment::Center)
///     .children(children![])
/// ```
///
/// Leading and trailing blank lines around a body are dropped; the indentation
/// inside it is kept exactly, because that is the whole content. A heading with
/// no body is skipped rather than inserted as an empty snippet.
#[must_use]
pub fn parse_snippets(text: &str) -> Vec<UserSnippet> {
    let mut out: Vec<UserSnippet> = Vec::new();
    let mut name: Option<String> = None;
    let mut body: Vec<&str> = Vec::new();

    let finish = |name: &mut Option<String>, body: &mut Vec<&str>, out: &mut Vec<UserSnippet>| {
        let Some(name) = name.take() else { return };
        while body.first().is_some_and(|line| line.trim().is_empty()) {
            body.remove(0);
        }
        while body.last().is_some_and(|line| line.trim().is_empty()) {
            body.pop();
        }
        if !body.is_empty() {
            out.push(UserSnippet {
                name,
                body: body.join("\n"),
            });
        }
        body.clear();
    };

    for line in text.lines() {
        let trimmed = line.trim();
        // A comment only counts outside a body: `#` is a legal character in
        // Rust source, and a snippet whose first line is `#[derive(Debug)]`
        // must not have that line eaten.
        if name.is_none() && (trimmed.is_empty() || trimmed.starts_with('#')) {
            continue;
        }
        if let Some(heading) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            finish(&mut name, &mut body, &mut out);
            let heading = heading.trim();
            if !heading.is_empty() {
                name = Some(heading.to_owned());
            }
            continue;
        }
        if name.is_some() {
            body.push(line);
        }
    }
    finish(&mut name, &mut body, &mut out);
    out
}

/// A snippets file somebody can start from.
#[must_use]
pub fn snippets_template() -> String {
    "# vieww Studio snippets. A heading in [brackets], then the lines it\n\
     # inserts. Indentation inside a snippet is kept exactly as written.\n\
     \n\
     [Centred column]\n\
     Flex::column()\n\
     \x20   .main_axis_alignment(MainAxisAlignment::Center)\n\
     \x20   .children(children![])\n"
        .to_owned()
}

// ---------------------------------------------------------------------------
// Keymap
// ---------------------------------------------------------------------------

/// A chord, as a keymap file spells it: `Ctrl+Shift+K`, `Cmd+Alt+P`, `Escape`.
///
/// Parsed into the parts of a [`crate::command::Chord`] rather than into one
/// directly, so this module stays free of the widget types and can be tested
/// by the std-only harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordSpec {
    pub shortcut: bool,
    pub shift: bool,
    pub alt: bool,
    /// A single character, lower-cased.
    pub character: Option<String>,
    /// A named key: `Enter`, `Escape`, `Tab`, `Space`.
    pub named: Option<String>,
}

/// Parse `Ctrl+Shift+K` and friends.
///
/// `Ctrl`, `Cmd`, `Command`, `Meta` and `Super` all mean the same thing — the
/// **shortcut modifier**, which is Command on Apple and Control everywhere
/// else. That is what `Chord::shortcut` means, and spelling it five ways is
/// what people will type; refusing four of them would be pedantry about a
/// distinction the studio deliberately does not make.
#[must_use]
pub fn parse_chord(text: &str) -> Option<ChordSpec> {
    let mut spec = ChordSpec {
        shortcut: false,
        shift: false,
        alt: false,
        character: None,
        named: None,
    };
    let mut key_seen = false;
    for part in text.split('+') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" | "meta" | "super" => spec.shortcut = true,
            "shift" => spec.shift = true,
            "alt" | "opt" | "option" => spec.alt = true,
            other => {
                // Two keys in one chord is a typo, not a chord.
                if key_seen {
                    return None;
                }
                key_seen = true;
                match other {
                    "enter" | "return" => spec.named = Some("Enter".to_owned()),
                    "escape" | "esc" => spec.named = Some("Escape".to_owned()),
                    "tab" => spec.named = Some("Tab".to_owned()),
                    "space" => spec.named = Some("Space".to_owned()),
                    "backspace" => spec.named = Some("Backspace".to_owned()),
                    "delete" | "del" => spec.named = Some("Delete".to_owned()),
                    single if single.chars().count() == 1 => {
                        spec.character = Some(single.to_owned());
                    }
                    _ => return None,
                }
            }
        }
    }
    key_seen.then_some(spec)
}

/// A parsed keymap: command name to chord, plus the lines that were refused.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub bindings: BTreeMap<String, ChordSpec>,
    /// Lines that could not be used, and why. Reported rather than dropped:
    /// a keymap file where one line silently did nothing is a file the user
    /// keeps editing without understanding why.
    pub problems: Vec<String>,
}

/// Parse a keymap file: `Command Label = Ctrl+Shift+K`.
///
/// The key is the command's **label** — what the palette and the menus call it
/// — because that is the name the user can see. An id would be stabler and
/// would require them to know it.
///
/// A chord of `none` unbinds the command, which is the only way to free a
/// shortcut whose default is in the way.
#[must_use]
pub fn parse_keymap(text: &str) -> Keymap {
    let mut map = Keymap::default();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, chord)) = line.split_once('=') else {
            map.problems
                .push(format!("line {}: no \"=\" in {line:?}", number + 1));
            continue;
        };
        let name = name.trim();
        let chord = chord.trim();
        if name.is_empty() {
            continue;
        }
        if chord.eq_ignore_ascii_case("none") {
            map.bindings.insert(
                name.to_owned(),
                ChordSpec {
                    shortcut: false,
                    shift: false,
                    alt: false,
                    character: None,
                    named: None,
                },
            );
            continue;
        }
        match parse_chord(chord) {
            Some(spec) => {
                // **The collision check this module's own header promised.**
                // It said an override "that would collide with an existing
                // chord is refused and reported rather than silently shadowing
                // a shortcut the user still expects to work", and then did no
                // such thing: two commands bound to ⌘S both landed in the map,
                // one of them won inside `Shortcuts`, and the user's first
                // binding disappeared with nothing said about it.
                //
                // Refused, not overwritten — the first binding for a chord is
                // the one that stands. A file is read top to bottom, so the
                // binding a person wrote first is the one they were thinking
                // about, and "the later line silently wins" is the behaviour
                // that made this worth reporting in the first place.
                if let Some((claimed, _)) = map
                    .bindings
                    .iter()
                    .find(|(other, existing)| **existing == spec && other.as_str() != name)
                {
                    map.problems.push(format!(
                        "line {}: {chord:?} is already bound to {claimed:?}, so {name:?} keeps its default",
                        number + 1
                    ));
                    continue;
                }
                map.bindings.insert(name.to_owned(), spec);
            }
            None => map
                .problems
                .push(format!("line {}: {chord:?} is not a chord", number + 1)),
        }
    }
    map
}

/// A keymap file somebody can start from.
#[must_use]
pub fn keymap_template(current: &[(String, String)]) -> String {
    let mut out = String::from(
        "# vieww Studio keymap. The name on the left is what the command\n\
         # palette calls the command. \"none\" unbinds it.\n\
         #\n\
         # Ctrl, Cmd, Command, Meta and Super all mean the same thing: the\n\
         # shortcut modifier, which is Command on a Mac and Control elsewhere.\n\
         #\n\
         # Every command and its current chord, commented out:\n\n",
    );
    for (name, chord) in current {
        out.push_str(&format!("# {name} = {chord}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- colours -------------------------------------------------------

    #[test]
    fn hex_colours_in_every_length() {
        assert_eq!(parse_hex("#000000"), Some([0, 0, 0, 0xFF]));
        assert_eq!(parse_hex("#ffffff"), Some([0xFF, 0xFF, 0xFF, 0xFF]));
        assert_eq!(parse_hex("#5B9DF9"), Some([0x5B, 0x9D, 0xF9, 0xFF]));
        assert_eq!(parse_hex("#12345678"), Some([0x12, 0x34, 0x56, 0x78]));
        assert_eq!(parse_hex("  #abc  "), Some([0xAA, 0xBB, 0xCC, 0xFF]));
    }

    #[test]
    fn a_value_that_is_not_a_colour_is_none_rather_than_a_panic() {
        for bad in ["", "#", "red", "#12345", "#gggggg", "123456"] {
            assert_eq!(parse_hex(bad), None, "{bad:?}");
        }
    }

    /// The worst outcome of a partial theme file has to be a partially
    /// recoloured studio, not an unreadable one.
    #[test]
    fn a_theme_overrides_only_what_it_names() {
        let theme = Theme::parse("base = light\nchrome_0 = #101010\n");
        assert!(!theme.dark);
        assert_eq!(theme.get("chrome_0"), Some([0x10, 0x10, 0x10, 0xFF]));
        assert_eq!(theme.get("chrome_1"), None, "left at the built-in");
    }

    #[test]
    fn a_bad_colour_leaves_that_token_alone_and_keeps_the_rest() {
        let theme = Theme::parse("chrome_0 = darkish\nchrome_1 = #202020\n");
        assert_eq!(theme.get("chrome_0"), None);
        assert_eq!(theme.get("chrome_1"), Some([0x20, 0x20, 0x20, 0xFF]));
    }

    #[test]
    fn an_unknown_token_is_ignored() {
        assert!(Theme::parse("wallpaper = #ffffff").is_empty());
    }

    #[test]
    fn the_theme_template_names_every_token() {
        let template = Theme::template();
        for token in TOKENS {
            assert!(
                template.contains(token),
                "{token} missing from the template"
            );
        }
    }

    // ----- snippets ------------------------------------------------------

    #[test]
    fn a_snippet_is_a_heading_and_the_lines_under_it() {
        let parsed = parse_snippets("[Row]\nFlex::row()\n    .children(children![])\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Row");
        assert_eq!(parsed[0].body, "Flex::row()\n    .children(children![])");
    }

    #[test]
    fn several_snippets_in_one_file() {
        let parsed = parse_snippets("[One]\na\n\n[Two]\nb\n");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].body, "a");
        assert_eq!(parsed[1].body, "b");
    }

    /// `#` is a legal character in Rust source, and a snippet whose first line
    /// is `#[derive(Debug)]` must not have that line eaten as a comment.
    #[test]
    fn a_hash_inside_a_body_is_code_and_not_a_comment() {
        let parsed = parse_snippets("# a real comment\n[Derive]\n#[derive(Debug)]\nstruct S;\n");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].body, "#[derive(Debug)]\nstruct S;");
    }

    #[test]
    fn indentation_inside_a_snippet_is_kept_exactly() {
        let parsed = parse_snippets("[Nested]\nouter {\n        deep\n}\n");
        assert!(
            parsed[0].body.contains("\n        deep"),
            "{:?}",
            parsed[0].body
        );
    }

    #[test]
    fn a_heading_with_no_body_is_skipped_rather_than_inserted_empty() {
        assert!(parse_snippets("[Empty]\n\n[Also empty]\n").is_empty());
    }

    #[test]
    fn an_empty_file_is_no_snippets() {
        assert!(parse_snippets("").is_empty());
        assert!(parse_snippets("# only comments\n").is_empty());
    }

    // ----- chords --------------------------------------------------------

    #[test]
    fn chords_parse_the_way_people_write_them() {
        let ctrl_k = parse_chord("Ctrl+K").expect("a chord");
        assert!(ctrl_k.shortcut);
        assert_eq!(ctrl_k.character.as_deref(), Some("k"));

        let all = parse_chord("Cmd+Shift+Alt+P").expect("a chord");
        assert!(all.shortcut && all.shift && all.alt);
        assert_eq!(all.character.as_deref(), Some("p"));

        assert_eq!(
            parse_chord("Escape").expect("a chord").named.as_deref(),
            Some("Escape")
        );
        assert_eq!(
            parse_chord("Ctrl+Enter").expect("a chord").named.as_deref(),
            Some("Enter")
        );
    }

    /// Five spellings of one modifier, because that is what people will type
    /// and the studio deliberately does not distinguish them.
    #[test]
    fn every_spelling_of_the_shortcut_modifier_works() {
        for spelling in ["Ctrl", "Control", "Cmd", "Command", "Meta", "Super"] {
            let chord = parse_chord(&format!("{spelling}+S")).expect(spelling);
            assert!(chord.shortcut, "{spelling}");
        }
    }

    #[test]
    fn case_and_spacing_do_not_matter() {
        assert_eq!(parse_chord("ctrl+shift+k"), parse_chord("Ctrl + Shift + K"));
    }

    #[test]
    fn a_chord_with_no_key_or_two_keys_is_refused() {
        assert_eq!(parse_chord("Ctrl+Shift"), None, "modifiers are not a chord");
        assert_eq!(parse_chord("Ctrl+A+B"), None, "two keys is a typo");
        assert_eq!(parse_chord(""), None);
        assert_eq!(parse_chord("Ctrl+Wibble"), None);
    }

    // ----- keymap --------------------------------------------------------

    #[test]
    fn a_keymap_binds_by_the_name_the_palette_shows() {
        let map = parse_keymap("Save = Ctrl+Shift+S\nRender = Cmd+Enter\n");
        assert_eq!(map.bindings.len(), 2);
        assert!(map.problems.is_empty());
        assert!(map.bindings["Save"].shift);
    }

    /// The only way to free a shortcut whose default is in the way.
    #[test]
    fn none_unbinds() {
        let map = parse_keymap("Zen Mode = none\n");
        let spec = &map.bindings["Zen Mode"];
        assert!(spec.character.is_none() && spec.named.is_none());
    }

    /// A keymap file where one line silently did nothing is a file the user
    /// keeps editing without understanding why.
    #[test]
    fn a_line_that_cannot_be_used_is_reported_rather_than_dropped() {
        let map = parse_keymap("Save = Ctrl+Wibble\nno equals here\nRender = Cmd+Enter\n");
        assert_eq!(map.bindings.len(), 1, "the good line still applied");
        assert_eq!(map.problems.len(), 2, "{:?}", map.problems);
        assert!(map.problems[0].contains("line 1"));
        assert!(map.problems[1].contains("line 2"));
    }

    #[test]
    fn comments_and_blank_lines_are_not_problems() {
        let map = parse_keymap("# a comment\n\n   \nSave = Ctrl+S\n");
        assert!(map.problems.is_empty(), "{:?}", map.problems);
    }

    #[test]
    fn the_keymap_template_lists_what_is_bound_now() {
        let template = keymap_template(&[("Save".to_owned(), "Ctrl+S".to_owned())]);
        assert!(template.contains("# Save = Ctrl+S"));
    }

    #[test]
    fn two_commands_on_one_chord_is_reported_and_the_first_stands() {
        let map = parse_keymap("Save = Cmd+S\nSave All = Cmd+S\n");
        assert_eq!(
            map.bindings.get("Save").and_then(|c| c.character.clone()),
            Some("s".to_owned())
        );
        assert!(
            !map.bindings.contains_key("Save All"),
            "the colliding binding is refused, not applied"
        );
        assert_eq!(map.problems.len(), 1, "{:?}", map.problems);
        assert!(
            map.problems[0].contains("already bound to") && map.problems[0].contains("Save"),
            "{:?}",
            map.problems
        );
    }

    #[test]
    fn rebinding_the_same_command_twice_is_not_a_collision_with_itself() {
        let map = parse_keymap("Save = Cmd+S\nSave = Cmd+W\n");
        assert!(map.problems.is_empty(), "{:?}", map.problems);
        assert_eq!(
            map.bindings.get("Save").and_then(|c| c.character.clone()),
            Some("w".to_owned())
        );
    }

    #[test]
    fn unbinding_two_commands_is_not_a_collision() {
        // Every "none" parses to the same empty spec, and two commands with no
        // chord do not shadow each other.
        let map = parse_keymap("Save = none\nSave All = none\n");
        assert!(map.problems.is_empty(), "{:?}", map.problems);
        assert_eq!(map.bindings.len(), 2);
    }
}
