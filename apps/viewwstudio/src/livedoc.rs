//! `live.rs` — the file in the user's workspace that the Live Preview draws.
//!
//! # Why the live preview reads a file at all
//!
//! It used to be a fixed demo compiled into the studio: three screens of
//! invented content that were the same in every project. That answers "what
//! does a whole app flow feel like in this frame" and nothing about the app the
//! person is actually building, and it was reported as exactly that — *"is it a
//! static one or does it live preview any workspace that I'm working on?"*
//!
//! So the screens come from `live.rs` at the workspace root. The studio parses
//! it and draws it; it is **not** compiled and not linked into the app, which is
//! what makes the preview instant and what lets it update *while the file is
//! being typed*. `Render` remains the thing that compiles real Rust, one screen
//! at a time. The two answer different questions and now they say so.
//!
//! No `live.rs`, no live preview — and the studio says which file is missing and
//! offers to write one, rather than quietly showing a demo that belongs to
//! nobody.
//!
//! # The format
//!
//! A `live!` block of lines, each a keyword and its quoted arguments:
//!
//! ```text
//! live! {
//!     screen "Home" {
//!         title "Inbox"
//!         row "Design Review" "3 new comments"
//!         button "Open Settings" -> "Settings"
//!     }
//!     screen "Settings" {
//!         title "Settings"
//!         toggle "Notifications"
//!         counter "Badge count"
//!     }
//!     nav "Home" "Settings"
//! }
//! ```
//!
//! It is a macro invocation, so an editor highlights it as Rust and `rustfmt`
//! leaves it alone. It is deliberately **not** a language: there are no
//! expressions, no conditionals and no loops, because everything it cannot say
//! is a thing `Render` says properly with real code. What it can say is the
//! shape of a flow — screens, the rows on them, and the buttons between them —
//! which is what a preview that bypasses the compiler can honestly show.
//!
//! # Why a hand-written scanner and not tree-sitter
//!
//! The workspace already links `tree-sitter-rust` for highlighting, so parsing
//! this with it was the obvious move. The format is one keyword and some string
//! literals per line: a scanner is thirty lines, reports the line number a
//! mistake is on for free, and cannot be broken by a `live!` body that is not
//! valid Rust — which is the common case while somebody is typing one.

use std::fmt;
use std::path::{Path, PathBuf};

/// The file the Live Preview reads, relative to the workspace root.
pub const FILE: &str = "live.rs";

/// What a new project gets, and what "Create live.rs" writes.
///
/// Content rather than lorem ipsum: the point of the template is that pressing
/// Live immediately shows something that looks like an app, so the first edit
/// is a change to something working rather than a blank screen.
///
/// # The `mount` line names a file that exists
///
/// It used to say `mount "card_grid.rs"`, which is one of **this studio's own
/// sample screens** and is not in any project the scaffold writes. So the
/// second screen of every new project's flow showed a grey box reading
/// `card_grid.rs` — "Open this file and press Render once to place it here" —
/// naming a file the user does not have and cannot open. An instruction that
/// cannot be followed is worse than no placeholder at all.
///
/// `src/screens/home.rs` is what [`crate::scaffold`] writes and what the studio
/// opens the project on, so the instruction is one keystroke from done: press
/// Render on the file already in front of you and it appears in the flow. That
/// is also the clearest possible demonstration of what `mount` is *for*.
pub const TEMPLATE: &str = r#"// live.rs — vieww Studio's Live Preview reads this file.
//
// It is NOT compiled and not part of your app: the studio parses it and draws
// it, which is why it updates as you type and why it cannot run any logic. Use
// it to sketch a flow — the screens, what is on them, and the way between them.
// Press Render to see real code instead; that one compiles.
//
// Delete this file and the Live Preview turns off.

live! {
    screen "Home" {
        title "Inbox"
        row "Design Review" "3 new comments on the card layout"
        row "Sprint Planning" "5 tickets moved to In Progress"
        row "Release Notes" "Draft for v0.3 is ready to review"
        button "Open settings" -> "Settings"
    }

    screen "Detail" {
        title "Design Review"
        text "Rows, text, buttons, a switch and a counter are all this file can say by itself."
        // `mount` drops one of your own screens in, exactly as it last rendered.
        // Press Render on src/screens/home.rs once to fill this in — it is the
        // file this project opens with.
        mount "src/screens/home.rs"
        button "Back" -> "Home"
    }

    screen "Settings" {
        title "Settings"
        toggle "Notifications"
        counter "Badge count"
        text "Changes here are a sketch, not state your app keeps."
        button "Done" -> "Home"
    }

    nav "Home" "Detail" "Settings"
}
"#;

/// One thing on a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveItem {
    /// A list row: a title, and a line under it.
    Row { title: String, detail: String },
    /// A paragraph.
    Text(String),
    /// A button that goes to another screen by name.
    Button { label: String, target: String },
    /// A labelled switch. Its state is the preview's, not the app's.
    Toggle(String),
    /// A labelled number with − and + beside it.
    Counter(String),
    /// **The user's own screen, compiled.** `mount "screens/card_grid.rs"`
    /// places the widget tree that file's `screen()` builds inside the flow.
    ///
    /// This is what keeps the preview about the app being written rather than
    /// about this file's small vocabulary: the rows and buttons sketch the
    /// journey, and a `mount` drops the real thing — the user's components,
    /// their spacing, their colours — into the middle of it.
    ///
    /// It is the one item that needs the compiler, and it does not run it: it
    /// shows the screen as of the last time that file was rendered, and says so
    /// when it has not been. Pressing Render on the file once is what makes it
    /// appear, and every later Render updates it in place.
    Mount(String),
}

/// One screen: the name the navigation and buttons use, its heading, and what
/// is on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveScreen {
    pub name: String,
    pub title: String,
    pub items: Vec<LiveItem>,
}

/// A parsed `live.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveDoc {
    pub screens: Vec<LiveScreen>,
    /// The names along the bottom bar, in order. Empty means no bar.
    pub nav: Vec<String>,
}

impl LiveDoc {
    /// The screen at `index`, clamped — the route survives an edit that deletes
    /// the screen it was pointing at.
    #[must_use]
    pub fn screen(&self, index: usize) -> Option<&LiveScreen> {
        if self.screens.is_empty() {
            return None;
        }
        self.screens.get(index.min(self.screens.len() - 1))
    }

    /// Which screen a button's target names.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.screens.iter().position(|screen| screen.name == name)
    }
}

/// What went wrong, and on which line.
///
/// One error rather than a list: the parser stops at the first thing it cannot
/// read, because everything after a mistake in a nested block is a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveError {
    /// One-based, so it matches the editor's gutter and `jump_to`.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for LiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "live.rs line {}: {}", self.line, self.message)
    }
}

/// Where `live.rs` is for a workspace.
#[must_use]
pub fn path_in(root: &Path) -> PathBuf {
    root.join(FILE)
}

/// Read the quoted strings on one line, in order.
///
/// Escapes are deliberately not handled: a `\"` inside a preview label is an
/// edge case nobody has, and pretending to support it invites the file to grow
/// into a language. A stray quote produces an unterminated string, which is
/// reported with its line number.
fn strings(line: &str) -> Result<Vec<String>, &'static str> {
    let mut out = Vec::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        let mut closed = false;
        for c in chars.by_ref() {
            if c == '"' {
                closed = true;
                break;
            }
            value.push(c);
        }
        if !closed {
            return Err("a string is missing its closing quote");
        }
        out.push(value);
    }
    Ok(out)
}

/// Parse a `live.rs`.
///
/// # Errors
///
/// The first line that cannot be read, with its number.
#[expect(
    clippy::too_many_lines,
    reason = "one match over the format's keywords, which is clearer whole than split across helpers"
)]
pub fn parse(source: &str) -> Result<LiveDoc, LiveError> {
    let mut doc = LiveDoc::default();
    let mut screen: Option<LiveScreen> = None;
    let mut in_block = false;

    for (index, raw) in source.lines().enumerate() {
        let line = raw.trim();
        let number = index + 1;
        let fail = |message: &str| LiveError {
            line: number,
            message: message.to_owned(),
        };

        // Comments and blank lines, and the block's own braces.
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if !in_block {
            if line.starts_with("live!") {
                in_block = true;
            }
            continue;
        }
        if line == "}" {
            match screen.take() {
                // Closing a screen.
                Some(done) => doc.screens.push(done),
                // Closing the `live!` block itself.
                None => in_block = false,
            }
            continue;
        }

        let keyword = line.split_whitespace().next().unwrap_or_default();
        let args = strings(line).map_err(fail)?;

        match keyword {
            "screen" => {
                if screen.is_some() {
                    return Err(fail("a screen cannot be opened inside another screen"));
                }
                let name = args
                    .first()
                    .ok_or_else(|| fail("screen needs a name: screen \"Home\" {"))?;
                if !line.ends_with('{') {
                    return Err(fail("screen needs an opening brace: screen \"Home\" {"));
                }
                screen = Some(LiveScreen {
                    name: name.clone(),
                    title: name.clone(),
                    items: Vec::new(),
                });
            }
            "title" | "row" | "text" | "button" | "toggle" | "counter" | "mount" => {
                let Some(open) = screen.as_mut() else {
                    return Err(fail(&format!("{keyword} has to be inside a screen")));
                };
                match keyword {
                    "title" => {
                        open.title = args
                            .first()
                            .ok_or_else(|| fail("title needs text: title \"Inbox\""))?
                            .clone();
                    }
                    "row" => {
                        let title = args
                            .first()
                            .ok_or_else(|| fail("row needs a title: row \"Name\" \"detail\""))?
                            .clone();
                        open.items.push(LiveItem::Row {
                            title,
                            detail: args.get(1).cloned().unwrap_or_default(),
                        });
                    }
                    "text" => open.items.push(LiveItem::Text(
                        args.first()
                            .ok_or_else(|| fail("text needs something to say"))?
                            .clone(),
                    )),
                    "button" => {
                        let label = args
                            .first()
                            .ok_or_else(|| {
                                fail("button needs a label: button \"Go\" -> \"Screen\"")
                            })?
                            .clone();
                        let target = args
                            .get(1)
                            .ok_or_else(|| {
                                fail("button needs a screen to go to: button \"Go\" -> \"Screen\"")
                            })?
                            .clone();
                        open.items.push(LiveItem::Button { label, target });
                    }
                    "mount" => open.items.push(LiveItem::Mount(
                        args.first()
                            .ok_or_else(|| {
                                fail("mount needs a file: mount \"screens/card_grid.rs\"")
                            })?
                            .clone(),
                    )),
                    "toggle" => open.items.push(LiveItem::Toggle(
                        args.first()
                            .ok_or_else(|| fail("toggle needs a label"))?
                            .clone(),
                    )),
                    _ => open.items.push(LiveItem::Counter(
                        args.first()
                            .ok_or_else(|| fail("counter needs a label"))?
                            .clone(),
                    )),
                }
            }
            "nav" => {
                if screen.is_some() {
                    return Err(fail("nav belongs outside a screen"));
                }
                doc.nav = args;
            }
            other => {
                return Err(fail(&format!(
                    "{other} is not one of screen, title, row, text, button, toggle, counter, \
                     mount, nav"
                )))
            }
        }
    }

    if screen.is_some() {
        return Err(LiveError {
            line: source.lines().count().max(1),
            message: "a screen is missing its closing brace".to_owned(),
        });
    }
    if doc.screens.is_empty() {
        return Err(LiveError {
            line: 1,
            message: "no screens — a live preview needs at least one `screen \"Name\" { … }`"
                .to_owned(),
        });
    }

    // A button pointing at a screen that does not exist is the mistake this
    // format makes easiest, and the one whose symptom (a button that does
    // nothing) is hardest to read. Named here instead.
    for screen in &doc.screens {
        for item in &screen.items {
            if let LiveItem::Button { label, target } = item {
                if doc.index_of(target).is_none() {
                    return Err(LiveError {
                        line: line_of(source, target),
                        message: format!(
                            "\"{label}\" goes to \"{target}\", which is not a screen in this file"
                        ),
                    });
                }
            }
        }
    }

    Ok(doc)
}

/// The first line mentioning `needle`, for an error that is about a value
/// rather than about a line.
fn line_of(source: &str, needle: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map_or(1, |index| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses() {
        let doc = parse(TEMPLATE).expect("the file every new project gets");
        assert_eq!(doc.screens.len(), 3);
        assert_eq!(doc.nav, vec!["Home", "Detail", "Settings"]);
        assert_eq!(doc.screens[0].title, "Inbox");
        assert!(matches!(doc.screens[0].items[0], LiveItem::Row { .. }));
    }

    #[test]
    fn a_button_reaches_a_screen_by_name() {
        let doc = parse(TEMPLATE).unwrap();
        let LiveItem::Button { target, .. } = doc.screens[0].items.last().unwrap() else {
            panic!("the last item is the button")
        };
        assert_eq!(doc.index_of(target), Some(2));
    }

    #[test]
    fn a_button_to_nowhere_is_named_with_its_line() {
        let source = "live! {\n  screen \"Home\" {\n    button \"Go\" -> \"Typo\"\n  }\n}\n";
        let error = parse(source).expect_err("refused");
        assert_eq!(error.line, 3);
        assert!(error.message.contains("Typo"), "{}", error.message);
    }

    #[test]
    fn an_unknown_keyword_says_what_is_allowed() {
        let source = "live! {\n  screen \"Home\" {\n    heading \"Nope\"\n  }\n}\n";
        let error = parse(source).expect_err("refused");
        assert_eq!(error.line, 3);
        assert!(
            error.message.contains("screen, title, row"),
            "{}",
            error.message
        );
    }

    #[test]
    fn an_unterminated_string_is_reported_where_it_starts() {
        let source = "live! {\n  screen \"Home\" {\n    row \"Missing\n  }\n}\n";
        let error = parse(source).expect_err("refused");
        assert_eq!(error.line, 3);
    }

    #[test]
    fn a_file_with_no_screens_is_not_a_preview() {
        assert!(parse("live! {\n}\n").is_err());
        assert!(parse("// nothing here\n").is_err());
    }

    /// The half-typed file: this is what the parser sees on most keystrokes, and
    /// it has to fail with a line number rather than panic or hang.
    #[test]
    fn every_prefix_of_the_template_either_parses_or_reports_a_line() {
        for end in 0..TEMPLATE.len() {
            if !TEMPLATE.is_char_boundary(end) {
                continue;
            }
            match parse(&TEMPLATE[..end]) {
                Ok(doc) => assert!(!doc.screens.is_empty()),
                Err(error) => assert!(error.line >= 1),
            }
        }
    }

    #[test]
    fn the_route_survives_a_screen_being_deleted() {
        let doc = parse(TEMPLATE).unwrap();
        assert_eq!(
            doc.screen(99).map(|s| s.name.clone()),
            Some("Settings".to_owned())
        );
        assert!(LiveDoc::default().screen(0).is_none());
    }
}
