//! N6: the edits an editor makes for you.
//!
//! Auto-indent, bracket auto-close and matching, comment toggling, and
//! indent/dedent. Everything here is a **pure function** from a buffer and a
//! selection to a new buffer and a new selection, which is what makes the
//! whole feature testable without a window, a font, or a frame.
//!
//! # The one rule that shapes all of it
//!
//! **A bracket inside a string or a comment is not a bracket.** Every function
//! in this file that looks at a `{` asks [`Code::at`] whether that byte is
//! really code first. Without it:
//!
//! * typing `"` inside `println!("hi")` closes a brace two lines up;
//! * `// }` un-indents the block it is documenting;
//! * bracket highlighting points at a `}` in a comment and blames the user.
//!
//! The scanner is deliberately small — Rust's own lexer this is not — but it
//! covers what appears in real source: line comments, block comments (nested,
//! which Rust allows and C does not), string literals with escapes, raw strings
//! with any number of hashes, char literals, and the lifetime tick that looks
//! exactly like an unterminated char literal (`&'a str`).
//!
//! # Offsets
//!
//! Byte offsets into a `&str`, like the rest of the studio's text handling, and
//! every one this file returns is on a character boundary. The tests include
//! multi-byte text for exactly that reason: an offset that splits a `é` is a
//! panic the moment the caret is drawn.

use std::ops::Range;

use crate::language::Language;

/// The result of an edit: the whole new buffer and where the selection lands.
///
/// The whole buffer rather than a splice, because that is the shape
/// [`Studio::edit`](crate::state::Studio::edit) already takes — a
/// `TextEditingValue` — and history coalescing works on it. Source files are
/// kilobytes; a clone per keystroke is not the cost worth complicating this to
/// avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub text: String,
    /// Anchor and focus, in bytes. Equal for a plain caret.
    pub selection: (usize, usize),
}

impl Edit {
    fn caret(text: String, at: usize) -> Self {
        Self {
            text,
            selection: (at, at),
        }
    }
}

/// The brackets that are auto-closed, and their partners.
const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
/// Quote characters, which close with themselves.
const QUOTES: [char; 2] = ['"', '\''];

/// Which bytes of a buffer are code, rather than string or comment.
///
/// Computed once per call over the whole buffer. That is O(n) for an operation
/// on one line, and it is the right trade: a scan that started at the caret
/// would have to guess whether it began inside a string, and the guess is
/// wrong exactly where it matters. A 10,000-line file is under a millisecond.
#[derive(Debug)]
pub struct Code {
    /// One flag per byte. `true` where the byte is code.
    flags: Vec<bool>,
}

impl Code {
    /// Scan `text`.
    #[must_use]
    pub fn scan(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut flags = vec![true; bytes.len()];
        let mut i = 0;

        while i < bytes.len() {
            match bytes[i] {
                b'/' if bytes.get(i + 1) == Some(&b'/') => {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        flags[i] = false;
                        i += 1;
                    }
                }
                b'/' if bytes.get(i + 1) == Some(&b'*') => {
                    // Rust's block comments nest, so a depth counter rather
                    // than a search for the first `*/`.
                    let mut depth = 0_u32;
                    while i < bytes.len() {
                        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                            depth += 1;
                            flags[i] = false;
                            flags[i + 1] = false;
                            i += 2;
                        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                            depth -= 1;
                            flags[i] = false;
                            flags[i + 1] = false;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        } else {
                            flags[i] = false;
                            i += 1;
                        }
                    }
                }
                b'r' if matches!(bytes.get(i + 1), Some(b'"' | b'#')) => {
                    if let Some(end) = raw_string_end(bytes, i) {
                        for flag in &mut flags[i..end] {
                            *flag = false;
                        }
                        i = end;
                    } else {
                        i += 1;
                    }
                }
                b'"' => {
                    let end = quoted_end(bytes, i, b'"');
                    for flag in &mut flags[i..end] {
                        *flag = false;
                    }
                    i = end;
                }
                b'\'' => {
                    // `'a` in `&'a str` is a lifetime, not an unterminated char
                    // literal, and treating it as one turns the rest of the
                    // file into a string.
                    if let Some(end) = char_literal_end(bytes, i) {
                        for flag in &mut flags[i..end] {
                            *flag = false;
                        }
                        i = end;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }

        Self { flags }
    }

    /// Whether the byte at `at` is code. Out of range answers `true`, so an
    /// offset at the very end of the buffer behaves like code rather than like
    /// an unterminated string.
    #[must_use]
    pub fn at(&self, at: usize) -> bool {
        self.flags.get(at).copied().unwrap_or(true)
    }
}

/// The end of a `"…"`, past the closing quote, or the end of the buffer.
fn quoted_end(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b if b == quote => return i + 1,
            b'\n' if quote == b'\'' => return i,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// The end of `r#"…"#`, counting the hashes on both sides.
fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let hash_start = i;
    while bytes.get(i) == Some(&b'#') {
        i += 1;
    }
    let hashes = i - hash_start;
    if bytes.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    while i < bytes.len() {
        if bytes[i] == b'"' && bytes[i + 1..].iter().take(hashes).all(|&b| b == b'#') {
            return Some((i + 1 + hashes).min(bytes.len()));
        }
        i += 1;
    }
    Some(bytes.len())
}

/// The end of `'x'`, or `None` when the tick is a lifetime.
///
/// # Why this is not "scan to the next quote"
///
/// It was, with a length cap to stop a lifetime swallowing the file, and the
/// cap was a guess that did not work. In `fn f<'a>(x: &'a str)` the scan from
/// the first tick reaches the *second* lifetime's tick nine bytes later, well
/// under any sane cap, and marks `'a>(x: &'` as a literal. Nothing in the test
/// suite noticed, because the braces that test looked at happened to fall
/// outside the mistaken span — the assertion was true for the wrong reason.
///
/// A char literal has a shape, so the shape is what is checked:
///
/// * `'\n'`, `'\u{1F600}'` — an escape, ending at the next unescaped quote;
/// * `'a'`, `'é'` — exactly one character, then a quote;
/// * anything else after the tick is a **lifetime**, and the tick is not a
///   literal at all.
fn char_literal_end(bytes: &[u8], start: usize) -> Option<usize> {
    let next = *bytes.get(start + 1)?;

    if next == b'\\' {
        let end = quoted_end(bytes, start, b'\'');
        return (end > start + 1 && bytes.get(end - 1) == Some(&b'\'')).then_some(end);
    }

    // One character — which may be several bytes — and then the closing quote.
    let width = match next {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        // A continuation byte cannot start a character; the input is not
        // sitting on a boundary, so this tick opens nothing.
        _ => return None,
    };
    (bytes.get(start + 1 + width) == Some(&b'\'')).then_some(start + 2 + width)
}

/// The byte range of the line containing `at`, without its line break.
#[must_use]
pub fn line_range(text: &str, at: usize) -> Range<usize> {
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let end = text[at..].find('\n').map_or(text.len(), |n| at + n);
    start..end
}

/// The leading whitespace of the line containing `at`.
#[must_use]
pub fn indent_of(text: &str, at: usize) -> &str {
    let line = line_range(text, at);
    let content = &text[line.clone()];
    let width = content
        .find(|c: char| !matches!(c, ' ' | '\t'))
        .unwrap_or(content.len());
    &content[..width]
}

/// One unit of indentation. Spaces, because the studio's own source is spaces
/// and a mixed file is worse than either choice.
pub const INDENT: &str = "    ";

/// What to do when Return is pressed at `caret`.
///
/// Three behaviours, in order of how surprising they are by their absence:
///
/// 1. **Keep the current indent.** Every editor does this; without it every
///    line after the first starts at column zero.
/// 2. **Add one level after an opener** — `{`, `(`, `[`, or a trailing `=>`.
/// 3. **Split a pair.** With the caret between `{` and `}`, Return produces the
///    open line, an indented empty line with the caret on it, and the closing
///    brace back at the original indent. This is the one that feels like magic
///    and is three lines of code.
///
/// A selection is replaced first, exactly as typing any character would.
#[must_use]
pub fn newline(text: &str, selection: (usize, usize)) -> Edit {
    let (start, end) = ordered(selection);
    let code = Code::scan(text);

    let indent = indent_of(text, start).to_string();
    let before = text[..start].trim_end_matches([' ', '\t']);
    let opens = before
        .chars()
        .next_back()
        .is_some_and(|c| matches!(c, '{' | '(' | '[') && code.at(before.len() - c.len_utf8()))
        || before.ends_with("=>");

    let deeper = format!("{indent}{INDENT}");

    // The closing partner sitting immediately after the caret, if any.
    let closes_here = text[end..]
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '}' | ')' | ']') && code.at(end));

    let mut out = String::with_capacity(text.len() + deeper.len() * 2 + 2);
    out.push_str(&text[..start]);

    let caret;
    if opens && closes_here {
        out.push('\n');
        out.push_str(&deeper);
        caret = out.len();
        out.push('\n');
        out.push_str(&indent);
        out.push_str(&text[end..]);
    } else {
        out.push('\n');
        out.push_str(if opens { &deeper } else { &indent });
        caret = out.len();
        out.push_str(&text[end..]);
    }

    Edit::caret(out, caret)
}

/// What to do when `typed` is a bracket or quote.
///
/// `None` means "nothing special, insert it normally" — the caller keeps its
/// ordinary path rather than this file having to reimplement plain insertion.
///
/// Three behaviours:
///
/// * **Surround.** With text selected, `(` wraps it rather than replacing it.
///   Replacing a selection with one bracket is the single most-cursed
///   auto-close behaviour, because it silently destroys what was selected.
/// * **Skip.** Typing `)` when `)` is already to the right moves over it,
///   rather than producing `))`.
/// * **Close.** Otherwise, insert the pair and sit between them — but only
///   where the next character is not a word character, so typing `(` before
///   `foo` does not produce `()foo`.
#[must_use]
pub fn typed_bracket(text: &str, selection: (usize, usize), typed: char) -> Option<Edit> {
    let (start, end) = ordered(selection);
    let code = Code::scan(text);

    let closer = PAIRS
        .iter()
        .find(|(open, _)| *open == typed)
        .map(|(_, close)| *close)
        .or_else(|| QUOTES.contains(&typed).then_some(typed));

    // Surround a selection.
    if start != end {
        if let Some(closer) = closer {
            let mut out = String::with_capacity(text.len() + 2);
            out.push_str(&text[..start]);
            out.push(typed);
            out.push_str(&text[start..end]);
            out.push(closer);
            out.push_str(&text[end..]);
            // Keep the selection on what was selected, now shifted by the
            // opener. Losing the selection here means a second wrap is
            // impossible without re-selecting.
            return Some(Edit {
                text: out,
                selection: (start + typed.len_utf8(), end + typed.len_utf8()),
            });
        }
        return None;
    }

    let next = text[start..].chars().next();

    // Skip over a closer that is already there.
    let is_closer = PAIRS.iter().any(|(_, close)| *close == typed) || QUOTES.contains(&typed);
    if is_closer && next == Some(typed) {
        // A bracket always skips. A **quote** only skips when the character to
        // its right is not code — that is, when it is the closing quote of the
        // string this caret is inside. Otherwise `"` before an unrelated `"` is
        // an ordinary insertion, and skipping would swallow it.
        let closing_something = matches!(typed, ')' | ']' | '}') || !code.at(start);
        if closing_something {
            return Some(Edit::caret(text.to_string(), start + typed.len_utf8()));
        }
    }

    let closer = closer?;

    // Do not auto-close in front of a word: `(` before `foo` should not make
    // `()foo`.
    if next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    // Nor inside a string or comment, where a `(` is just a character.
    if !code.at(start.saturating_sub(1)) && start > 0 {
        return None;
    }
    // A quote directly after a word is an apostrophe or a lifetime, not an
    // opening quote: `don` + `'` must not become `don''`.
    if QUOTES.contains(&typed) {
        let previous = text[..start].chars().next_back();
        if previous.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
    }

    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(&text[..start]);
    out.push(typed);
    out.push(closer);
    out.push_str(&text[start..]);
    Some(Edit::caret(out, start + typed.len_utf8()))
}

/// Backspace between an auto-closed pair deletes both.
///
/// `None` when the caret is not between a pair, so the caller does its normal
/// single-character delete.
#[must_use]
pub fn backspace_pair(text: &str, selection: (usize, usize)) -> Option<Edit> {
    let (start, end) = ordered(selection);
    if start != end || start == 0 {
        return None;
    }
    let before = text[..start].chars().next_back()?;
    let after = text[start..].chars().next()?;
    let paired = PAIRS
        .iter()
        .any(|(open, close)| *open == before && *close == after)
        || (QUOTES.contains(&before) && before == after);
    if !paired {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let at = start - before.len_utf8();
    out.push_str(&text[..at]);
    out.push_str(&text[start + after.len_utf8()..]);
    Some(Edit::caret(out, at))
}

/// The bracket matching the one at, or just before, `caret`.
///
/// Returns both offsets — the bracket found and its partner — because the
/// editor highlights the pair, not one half of it. `None` when the caret is not
/// on a bracket, or the bracket is unbalanced, which is the normal state of a
/// file being typed into and is not an error.
#[must_use]
pub fn match_bracket(text: &str, caret: usize) -> Option<(usize, usize)> {
    // **A caret from outside this string.** `text[..caret]` is a panic when the
    // caret is past the end or lands inside a character, and both happen for
    // ordinary reasons: a replace-all that shortened the file, a buffer whose
    // text was set wholesale, a file reloaded from disk after an outside edit.
    // The editor calls this on every build, so the panic took the whole code
    // pane down to the magenta placeholder for a caret that was merely stale —
    // a highlight that cannot be computed is worth nothing and costs nothing to
    // skip.
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let code = Code::scan(text);

    // Prefer the bracket *before* the caret, which is where it sits after
    // typing one.
    let before = text[..caret]
        .chars()
        .next_back()
        .map(|c| (caret - c.len_utf8(), c));
    let after = text[caret..].chars().next().map(|c| (caret, c));

    for (at, c) in [before, after].into_iter().flatten() {
        if !code.at(at) {
            continue;
        }
        if let Some(partner) = scan_for_partner(text, &code, at, c) {
            return Some((at, partner));
        }
    }
    None
}

fn scan_for_partner(text: &str, code: &Code, at: usize, bracket: char) -> Option<usize> {
    let forward = PAIRS.iter().find(|(open, _)| *open == bracket);
    let backward = PAIRS.iter().find(|(_, close)| *close == bracket);

    let (target, step): (char, isize) = match (forward, backward) {
        (Some((_, close)), _) => (*close, 1),
        (_, Some((open, _))) => (*open, -1),
        _ => return None,
    };

    let mut depth = 0_i32;
    let mut indices: Vec<(usize, char)> = text.char_indices().collect();
    if step < 0 {
        indices.reverse();
    }
    let from = indices.iter().position(|(i, _)| *i == at)?;

    for &(i, c) in &indices[from..] {
        if !code.at(i) {
            continue;
        }
        if c == bracket {
            depth += 1;
        } else if c == target {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Comment or uncomment every line the selection touches.
///
/// Uncomments only when **every** touched line is already commented, which is
/// the behaviour that makes the command its own inverse on a mixed block: a
/// first press comments the stragglers, a second uncomments all of it.
///
/// The marker goes at the **shallowest** indent among the touched lines rather
/// than at column zero, so commenting a nested block does not destroy the shape
/// of the code inside it.
///
/// # The marker comes from the language, not from Rust
///
/// This used to write `//` into whatever buffer was in front of it. In a
/// `Cargo.toml` or a `.json` that is not a comment — it is a syntax error
/// written into the user's file by a keystroke they use dozens of times an
/// hour, and the file fails to parse on the next load. The language decides:
/// [`Language::line_comment`] answers `Some("#")` for TOML, `Some("--")` for
/// SQL, and `None` for the languages that have no line comment at all — where
/// the honest edit is no edit, so this returns the text unchanged.
#[must_use]
pub fn toggle_comment(text: &str, language: Language, selection: (usize, usize)) -> Edit {
    let (start, end) = ordered(selection);

    // No line comment in this language: JSON and Markdown have none, and
    // inventing one corrupts the file. Return the input untouched, selection
    // and all, so the command is a visible no-op rather than a silent break.
    let Some(marker) = language.line_comment() else {
        return Edit {
            text: text.to_string(),
            selection: (start, end),
        };
    };

    let first = line_range(text, start).start;
    let last = line_range(text, end).end;

    let block = &text[first..last];
    let lines: Vec<&str> = block.split('\n').collect();

    let non_blank: Vec<&&str> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    let all_commented =
        !non_blank.is_empty() && non_blank.iter().all(|l| l.trim_start().starts_with(marker));

    let column = non_blank
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    let rebuilt: Vec<String> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                return (*line).to_string();
            }
            if all_commented {
                let at = line.find(marker).unwrap_or(0);
                let mut out = line[..at].to_string();
                // `// ` and `//` both uncomment, and the space only goes if it
                // is there — otherwise a second toggle eats a real character.
                let rest = &line[at + marker.len()..];
                out.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                out
            } else {
                let mut out = line[..column].to_string();
                out.push_str(marker);
                out.push(' ');
                out.push_str(&line[column..]);
                out
            }
        })
        .collect();

    let replacement = rebuilt.join("\n");
    let mut out = String::with_capacity(text.len() + lines.len() * 3);
    out.push_str(&text[..first]);
    out.push_str(&replacement);
    out.push_str(&text[last..]);

    // Keep the whole block selected so the command can be pressed twice.
    Edit {
        text: out,
        selection: (first, first + replacement.len()),
    }
}

/// Indent or outdent every line the selection touches — Tab and Shift-Tab.
///
/// With no selection, indenting inserts one unit at the caret like an ordinary
/// Tab; outdenting always works on the line.
#[must_use]
pub fn shift_indent(text: &str, selection: (usize, usize), deeper: bool) -> Edit {
    let (start, end) = ordered(selection);

    if start == end && deeper {
        let mut out = String::with_capacity(text.len() + INDENT.len());
        out.push_str(&text[..start]);
        out.push_str(INDENT);
        out.push_str(&text[start..]);
        return Edit::caret(out, start + INDENT.len());
    }

    let first = line_range(text, start).start;
    let last = line_range(text, end).end;
    let block = &text[first..last];

    let rebuilt: Vec<String> = block
        .split('\n')
        .map(|line| {
            if deeper {
                if line.trim().is_empty() {
                    line.to_string()
                } else {
                    format!("{INDENT}{line}")
                }
            } else {
                // Remove up to one indent's worth of leading whitespace,
                // accepting a tab as a whole unit.
                line.strip_prefix(INDENT)
                    .or_else(|| line.strip_prefix('\t'))
                    .map_or_else(
                        || line.trim_start_matches(' ').to_string(),
                        std::string::ToString::to_string,
                    )
            }
        })
        .collect();

    let replacement = rebuilt.join("\n");
    let mut out = String::with_capacity(text.len() + rebuilt.len() * INDENT.len());
    out.push_str(&text[..first]);
    out.push_str(&replacement);
    out.push_str(&text[last..]);

    Edit {
        text: out,
        selection: (first, first + replacement.len()),
    }
}

/// The comfort that applies to a change the editor has *already* made, if any.
///
/// # Why this reads a diff rather than a key event
///
/// The auto-indent and auto-close above want to run when a person types Return
/// or `(`. The obvious way to get them is to intercept the key — and it is the
/// wrong seam here. The studio's shortcut layer catches what the focused widget
/// *declined*, and a multi-line text field declines neither Return nor a
/// printable character; stealing them ahead of the field would break typing
/// itself, and IME composition, and paste.
///
/// The field's `on_changed` is the seam that already exists, and by then the
/// insertion has happened. So this asks the only question that matters: **was
/// this change one character being typed?** If it was, the comfort is applied
/// to the state *before* it, which is exactly what a key handler would have
/// done, and the result replaces the field's own edit.
///
/// `None` for everything else — a paste, a delete, a multi-character IME
/// commit, a caret move, or one of this module's own edits arriving back
/// through the same path. That last one is what keeps this from recursing.
#[must_use]
pub fn after_typing(before: &str, before_selection: (usize, usize), after: &str) -> Option<Edit> {
    let (start, end) = ordered(before_selection);
    if start > before.len() || end > before.len() {
        return None;
    }

    // The change must be exactly: the selection replaced by one character.
    // Reconstructing and comparing is cheaper to be sure of than a diff, and it
    // cannot mistake a two-character change for a one-character one.
    let typed = after.get(start..)?.chars().next()?;
    let mut expected = String::with_capacity(before.len() + typed.len_utf8());
    expected.push_str(&before[..start]);
    expected.push(typed);
    expected.push_str(&before[end..]);
    if expected != after {
        return None;
    }

    if typed == '\n' {
        return Some(newline(before, before_selection));
    }
    typed_bracket(before, before_selection, typed)
}

fn ordered((a, b): (usize, usize)) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

// ------------------------------------------------------------------ snippets

/// Where in a file a snippet is legal.
///
/// # Why this is on the snippet rather than worked out from the text
///
/// The library used to insert every entry as raw characters at the caret, and
/// two of its three entries were bare *expressions*. Clicking one on an empty
/// buffer put `Container::new()…` at item position and the buffer stopped
/// compiling on the spot — ``expected one of `!` or `::`, found `(` ``. The
/// position an entry needs is a property of the entry, known when it is
/// written; deriving it from the text would be guessing at something the
/// author already knew.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnippetKind {
    /// A whole item — `use`, `struct`, `impl`. Legal at the top level of a file.
    Item,
    /// An expression, legal only inside a body.
    Expression,
}

/// One entry in the widget library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snippet {
    pub name: &'static str,
    /// The file under `crates/` this comes from, so the row can say where to
    /// read the real thing rather than leaving the name to be searched for.
    pub source: &'static str,
    pub kind: SnippetKind,
    /// Written at zero indent. [`insert_snippet`] indents it to wherever it
    /// lands, so one body is right at the top level and six levels in.
    pub body: &'static str,
}

/// The widget library, in the order the sidebar lists it.
pub const SNIPPETS: [Snippet; 19] = [
    // ----- whole items: the shapes a file is made of ----------------------
    //
    // The library was eleven entries and ten of them were *expressions* — a
    // `Flex::row()` to drop inside a `build`. That is the half you need second.
    // The half you need first is the shape of a file: what `screen()` has to
    // look like for Render to find it, what an `impl Widget` needs, where state
    // lives. None of that was here, so a new user could fill a `build` they did
    // not know how to write.
    Snippet {
        name: "Screen entry point",
        source: "the contract Render looks for",
        kind: SnippetKind::Item,
        body: "/// The screen the preview mounts.\n///\n/// The name and the signature are the contract: Render looks for a\n/// `pub fn screen()` returning something that implements `Widget`.\npub fn screen() -> impl Widget {\n    Container::new()\n        .color(Color::hex(0xF7_F8FA))\n        .alignment(Alignment::CENTER)\n        .child(Text::new(\"Hello\"))\n}",
    },
    Snippet {
        name: "Widget with fields",
        source: "widget.rs",
        kind: SnippetKind::Item,
        body: "#[derive(Debug)]\npub struct Card {\n    pub title: String,\n    pub detail: String,\n}\n\nimpl Widget for Card {\n    fn debug_name(&self) -> &'static str {\n        \"Card\"\n    }\n\n    fn kind(&self) -> WidgetKind<'_> {\n        WidgetKind::Composed\n    }\n\n    fn build(&self, ctx: &BuildContext) -> WidgetNode {\n        let theme = ThemeData::of(ctx);\n\n        Container::new()\n            .color(theme.colors.surface_variant)\n            .radius(10.0)\n            .padding(EdgeInsets::all(12.0))\n            .child(\n                Flex::column()\n                    .cross_axis_alignment(CrossAxisAlignment::Start)\n                    .spacing(4.0)\n                    .children(children![\n                        Text::new(self.title.clone()),\n                        Text::new(self.detail.clone()),\n                    ]),\n            )\n            .into()\n    }\n}",
    },
    Snippet {
        name: "Widget with its own state",
        source: "widget.rs — create_state",
        kind: SnippetKind::Item,
        body: "/// A widget that remembers something across rebuilds.\n///\n/// State lives in the *element*, not the widget: the widget is rebuilt\n/// constantly and would forget. `create_state` is called once, on mount.\n#[derive(Debug)]\npub struct Counter;\n\n#[derive(Debug, Default)]\nstruct CounterState {\n    count: u32,\n}\n\nimpl ElementState for CounterState {\n    fn as_any(&self) -> &dyn std::any::Any {\n        self\n    }\n\n    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {\n        self\n    }\n}\n\nimpl Widget for Counter {\n    fn debug_name(&self) -> &'static str {\n        \"Counter\"\n    }\n\n    fn kind(&self) -> WidgetKind<'_> {\n        WidgetKind::Composed\n    }\n\n    fn create_state(&self) -> Option<Box<dyn ElementState>> {\n        Some(Box::new(CounterState::default()))\n    }\n\n    fn build(&self, ctx: &BuildContext) -> WidgetNode {\n        let count = ctx.state(|state: &CounterState| state.count).unwrap_or(0);\n\n        Text::new(format!(\"{count}\")).into()\n    }\n}",
    },
    Snippet {
        name: "Helper function",
        source: "a function that returns a node",
        kind: SnippetKind::Item,
        body: "/// One repeated piece, as a function.\n///\n/// The cheapest refactor in a screen: anything built twice becomes a `fn`\n/// returning a `WidgetNode`, and the `build` reads as an outline again.\nfn tile(color: Color, label: &str) -> WidgetNode {\n    Container::new()\n        .color(color)\n        .radius(8.0)\n        .padding(EdgeInsets::all(12.0))\n        .child(Text::new(label.to_owned()))\n        .into()\n}",
    },
    Snippet {
        name: "Tests for a widget",
        source: "widget::debug_tree",
        kind: SnippetKind::Item,
        body: "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    /// A widget test needs no window: `debug_tree` builds the tree and\n    /// prints it, which is enough to assert what a screen is made of.\n    #[test]\n    fn the_screen_has_its_text_in_it() {\n        let dump = vieww::widget::debug_tree(screen());\n        assert!(dump.contains(\"Text\"), \"{dump}\");\n    }\n}",
    },
    Snippet {
        name: "Widget shell",
        source: "widget.rs",
        kind: SnippetKind::Item,
        body: "#[derive(Debug)]\npub struct Screen;\n\nimpl Widget for Screen {\n    fn debug_name(&self) -> &'static str {\n        \"Screen\"\n    }\n\n    fn kind(&self) -> WidgetKind<'_> {\n        WidgetKind::Composed\n    }\n\n    fn build(&self, ctx: &BuildContext) -> WidgetNode {\n        let theme = ThemeData::of(ctx);\n\n        Container::new()\n            .color(theme.colors.surface)\n            .alignment(Alignment::CENTER)\n            .child(Text::new(\"Hello\"))\n            .into()\n    }\n}",
    },
    Snippet {
        name: "Centered text",
        source: "widgets/text.rs",
        kind: SnippetKind::Expression,
        body: "Container::new()\n    .alignment(Alignment::CENTER)\n    .child(Text::new(\"Hello\"))",
    },
    Snippet {
        name: "Button",
        source: "controls/button.rs",
        kind: SnippetKind::Expression,
        body: "Button::new(Text::new(\"Click\"))\n    .on_pressed(move || {})",
    },
    Snippet {
        name: "Flex column",
        source: "widgets/flex.rs",
        kind: SnippetKind::Expression,
        body: "Flex::column()\n    .cross_axis_alignment(CrossAxisAlignment::Start)\n    .spacing(8.0)\n    .children(children![])",
    },
    Snippet {
        name: "Flex row",
        source: "widgets/flex.rs",
        kind: SnippetKind::Expression,
        body: "Flex::row()\n    .main_axis_alignment(MainAxisAlignment::SpaceBetween)\n    .spacing(8.0)\n    .children(children![])",
    },
    Snippet {
        name: "Pressable",
        source: "widgets/pressable.rs",
        kind: SnippetKind::Expression,
        body: "Pressable::new(move |press: f32| {\n    Container::new()\n        .radius(8.0 + press * 2.0)\n        .color(theme.colors.surface_variant)\n        .into()\n})\n.on_tap(move || {})",
    },
    Snippet {
        name: "Stack + Positioned",
        source: "widgets/stack.rs",
        kind: SnippetKind::Expression,
        body: "Stack::new()\n    .alignment(Alignment::TOP_CENTER)\n    .children(children![\n        Positioned::fill().child(Container::new()),\n        Positioned::new().bottom(24.0).child(Container::new()),\n    ])",
    },
    Snippet {
        name: "ListView",
        source: "controls/list_view.rs",
        kind: SnippetKind::Expression,
        body: "ListView::new(items.len(), move |index| {\n    Text::new(items[index].clone()).into()\n})",
    },
    Snippet {
        name: "GridView",
        source: "controls/grid_view.rs",
        kind: SnippetKind::Expression,
        body: "GridView::new()\n    .columns(3)\n    .pitch(120.0)\n    .spacing(8.0)",
    },
    Snippet {
        name: "CircularProgress",
        source: "controls/progress.rs",
        kind: SnippetKind::Expression,
        body: "CircularProgress::indeterminate()",
    },
    Snippet {
        name: "Theme colours",
        source: "widgets/theme.rs",
        kind: SnippetKind::Expression,
        body: "let theme = ThemeData::of(ctx);\nlet colors = theme.colors;",
    },
    Snippet {
        name: "Entrance animation",
        source: "widgets/animated.rs",
        kind: SnippetKind::Expression,
        body: "Animated::new(1.0)\n    .from(0.0)\n    .duration(std::time::Duration::from_millis(400))\n    .build(|t| Opacity::new(t).child(Text::new(\"Hello\")).into())",
    },
    Snippet {
        name: "Safe area",
        source: "widgets/safe_area.rs",
        kind: SnippetKind::Expression,
        body: "SafeArea::new().child(\n    Container::new().padding(EdgeInsets::all(16.0)),\n)",
    },
    Snippet {
        name: "Semantics",
        source: "widgets/semantics.rs",
        kind: SnippetKind::Expression,
        body: "Semantics::new()\n    .role(Role::Button)\n    .label(\"Render\")\n    .child(child)",
    },
];

/// How deep in `{ }` the byte at `at` sits, counting only braces that are code.
///
/// Zero means item position — the top level of a file, where an expression is
/// a parse error and an `impl` is what belongs.
#[must_use]
pub fn brace_depth(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    let code = Code::scan(text);
    let mut depth = 0_usize;
    for (index, byte) in text.as_bytes()[..at].iter().enumerate() {
        if !code.at(index) {
            continue;
        }
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth
}

/// Every line of `body`, indented by `indent`. Blank lines stay blank.
///
/// Trailing whitespace on an otherwise-empty line is what `rustfmt` strips and
/// what a reviewer sees in a diff, so it is never written in the first place.
fn reindented(body: &str, indent: &str) -> String {
    let mut out = String::with_capacity(body.len() + indent.len() * 4);
    for (index, line) in body.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if !line.trim().is_empty() {
            out.push_str(indent);
            out.push_str(line);
        }
    }
    out
}

/// The edit that puts `snippet` into `text` at `selection`.
///
/// # Whole lines, always
///
/// A snippet is a construct, not a word, so it goes on lines of its own — the
/// caret's line if that line is blank, otherwise a new line under it. Inserting
/// at the exact caret offset is what glued four snippets into one
/// 388-character line in the recording this was written from, and no amount of
/// good intentions about where the user *should* have clicked fixes that.
///
/// The body is re-indented to the caret's line, so one body is right wherever
/// it lands, and the caret ends up after the last character inserted.
///
/// # Errors
///
/// Returns what to tell the user when the snippet cannot go where the caret
/// is: an expression at item position, or an item inside a body. The old
/// behaviour was to insert it anyway and let `rustc` report a parse error one
/// compile later — which tells the user that something is wrong, but not that
/// the studio did it to them.
pub fn insert_snippet(
    text: &str,
    selection: (usize, usize),
    snippet: &Snippet,
) -> Result<Edit, String> {
    insert_body(text, selection, snippet.name, snippet.kind, snippet.body)
}

/// The same, for a snippet that is not one of the built-in `&'static` ones.
///
/// # Why this split exists
///
/// [`Snippet`] holds `&'static str`, which is right for [`SNIPPETS`] — a
/// compiled-in array — and impossible for a snippet read out of the user's
/// snippets file at run time. Giving `Snippet` a lifetime parameter would
/// ripple through the sidebar, the palette and every test; splitting the
/// *body* of the insertion out costs one function and leaves both callers
/// exact.
///
/// # Errors
///
/// As [`insert_snippet`].
pub fn insert_body(
    text: &str,
    selection: (usize, usize),
    name: &str,
    kind: SnippetKind,
    body_text: &str,
) -> Result<Edit, String> {
    let (start, end) = ordered(selection);
    if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return Err("The caret is not in this buffer any more.".to_owned());
    }

    let depth = brace_depth(text, start);
    match kind {
        SnippetKind::Item if depth > 0 => {
            return Err(format!(
                "\u{201c}{name}\u{201d} is a whole item. Put the caret at the top level of the file, outside any braces."
            ));
        }
        SnippetKind::Expression if depth == 0 => {
            return Err(format!(
                "\u{201c}{name}\u{201d} is an expression and cannot go at the top level. Insert \u{201c}Widget shell\u{201d} first, then put the caret inside its `build`."
            ));
        }
        _ => {}
    }

    let line = line_range(text, start);
    let indent = indent_of(text, start).to_owned();
    let body = reindented(body_text, &indent);
    let line_is_blank = text[line.clone()].trim().is_empty();

    let mut out = String::with_capacity(text.len() + body.len() + indent.len() + 2);
    let caret;
    if line_is_blank {
        // The caret is on an empty line: the snippet *is* that line.
        out.push_str(&text[..line.start]);
        out.push_str(&body);
        caret = out.len();
        out.push_str(&text[line.end..]);
    } else {
        // Otherwise it goes under the caret's line, whole, leaving that line
        // exactly as it was. Replacing the selection would be the other
        // defensible choice, and is not what clicking a library card means.
        out.push_str(&text[..line.end]);
        out.push('\n');
        // An item wants a blank line above it; an expression inside a body does
        // not, and one there reads as a missing statement.
        if matches!(kind, SnippetKind::Item) {
            out.push('\n');
        }
        out.push_str(&body);
        caret = out.len();
        out.push_str(&text[line.end..]);
    }

    Ok(Edit::caret(out, caret))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write the caret as `|` and the selection as `|…|`. Reading an expected
    /// buffer with an offset written as a number beside it is how an off-by-one
    /// hides in a test that looks right.
    fn parse(marked: &str) -> (String, (usize, usize)) {
        let first = marked.find('|').expect("no caret in the fixture");
        let rest = &marked[first + 1..];
        match rest.find('|') {
            Some(second) => {
                let text = format!(
                    "{}{}{}",
                    &marked[..first],
                    &rest[..second],
                    &rest[second + 1..]
                );
                (text, (first, first + second))
            }
            None => {
                let text = format!("{}{}", &marked[..first], rest);
                (text, (first, first))
            }
        }
    }

    fn show(edit: &Edit) -> String {
        let (a, b) = edit.selection;
        if a == b {
            format!("{}|{}", &edit.text[..a], &edit.text[a..])
        } else {
            format!(
                "{}|{}|{}",
                &edit.text[..a],
                &edit.text[a..b],
                &edit.text[b..]
            )
        }
    }

    fn newline_of(marked: &str) -> String {
        let (text, selection) = parse(marked);
        show(&newline(&text, selection))
    }

    // ---- the scanner -------------------------------------------------------

    #[test]
    fn the_scanner_tells_code_from_everything_else() {
        let text = r#"let a = "in {string}"; // in }comment
let b = '}';"#;
        let code = Code::scan(text);
        let brace_in_string = text.find("{string}").unwrap();
        assert!(!code.at(brace_in_string), "a brace in a string is not code");
        let brace_in_comment = text.find("}comment").unwrap();
        assert!(
            !code.at(brace_in_comment),
            "a brace in a comment is not code"
        );
        let brace_in_char = text.rfind("'}'").unwrap() + 1;
        assert!(
            !code.at(brace_in_char),
            "a brace in a char literal is not code"
        );
        let real = text.find("let a").unwrap();
        assert!(code.at(real));
    }

    #[test]
    fn nested_block_comments_close_at_the_right_place() {
        let text = "a /* one /* two */ still */ b {}";
        let code = Code::scan(text);
        assert!(code.at(0), "before the comment");
        assert!(
            !code.at(text.find("still").unwrap()),
            "inside the outer comment"
        );
        assert!(
            code.at(text.find('{').unwrap()),
            "after both comments closed"
        );
    }

    #[test]
    fn raw_strings_swallow_their_hashes() {
        let text = r####"let s = r#"a "quoted" }brace"#; let t = 1;"####;
        let code = Code::scan(text);
        assert!(!code.at(text.find("}brace").unwrap()));
        assert!(code.at(text.find("let t").unwrap()), "the raw string ended");
    }

    /// `&'a str` looks exactly like the start of a char literal. Reading it as
    /// one turns the rest of the file into a string, and every bracket in it
    /// stops being code.
    #[test]
    fn a_lifetime_is_not_an_unterminated_char_literal() {
        let text = "fn f<'a>(x: &'a str) -> &'a str { x }";
        let code = Code::scan(text);
        assert!(code.at(text.find('{').unwrap()), "the body brace is code");
        assert!(
            code.at(text.rfind('}').unwrap()),
            "the closing brace is code"
        );

        // The assertion above is true even of an implementation that reads
        // `'a>(x: &'` as a char literal, because those braces are outside it.
        // This is the one that is not: every byte between the two lifetime
        // ticks is ordinary code.
        let between = text.find(">(x").unwrap();
        assert!(code.at(between), "the text between two lifetimes is code");
        assert!(
            code.at(text.find("str").unwrap()),
            "and so is the type after one"
        );
    }

    /// Char literals still have to work — including the escapes and the
    /// multi-byte characters that a "one byte then a quote" rule would miss.
    #[test]
    fn a_real_char_literal_is_still_a_char_literal() {
        for (text, inside) in [
            ("let c = '}';", "}"),
            ("let c = '\\'';", "\\'"),
            ("let c = '\\u{7d}';", "7d"),
            ("let c = 'é';", "é"),
            ("let c = '😀';", "😀"),
        ] {
            let code = Code::scan(text);
            let at = text.find(inside).unwrap();
            assert!(
                !code.at(at),
                "{inside} in {text} should be inside a literal"
            );
            assert!(code.at(text.find("let").unwrap()), "but `let` is code");
            let tail = text.rfind(';').unwrap();
            assert!(code.at(tail), "and the literal ended before the semicolon");
        }
    }

    // ---- newline -----------------------------------------------------------

    #[test]
    fn return_keeps_the_current_indent() {
        assert_eq!(newline_of("    let a = 1;|"), "    let a = 1;\n    |");
        assert_eq!(newline_of("no indent|"), "no indent\n|");
    }

    #[test]
    fn return_after_an_opener_goes_one_deeper() {
        assert_eq!(newline_of("    fn f() {|"), "    fn f() {\n        |");
        assert_eq!(newline_of("  match x {|"), "  match x {\n      |");
        assert_eq!(newline_of("    x => {|"), "    x => {\n        |");
        assert_eq!(newline_of("    Some(x) =>|"), "    Some(x) =>\n        |");
    }

    #[test]
    fn return_between_a_pair_puts_the_closer_on_its_own_line() {
        assert_eq!(
            newline_of("    fn f() {|}"),
            "    fn f() {\n        |\n    }"
        );
        assert_eq!(newline_of("v(|)"), "v(\n    |\n)");
    }

    /// A `{` inside a string is not an opener, so Return after it must not
    /// indent as though a block had been entered.
    #[test]
    fn a_brace_in_a_string_does_not_open_a_block() {
        // The caret is **inside** the string, directly after the brace, which
        // is the only position where the scanner is what decides. With the
        // caret after the closing quote the previous character is `"` and any
        // implementation gets it right.
        assert_eq!(newline_of("    let s = \"{|\""), "    let s = \"{\n    |\"");
    }

    #[test]
    fn return_replaces_a_selection_first() {
        assert_eq!(newline_of("    a|bc|d"), "    a\n    |d");
    }

    // ---- brackets ----------------------------------------------------------

    fn typed_of(marked: &str, c: char) -> Option<String> {
        let (text, selection) = parse(marked);
        typed_bracket(&text, selection, c).as_ref().map(show)
    }

    #[test]
    fn an_opener_brings_its_closer() {
        assert_eq!(typed_of("f|", '(').as_deref(), Some("f(|)"));
        assert_eq!(typed_of("a = |", '[').as_deref(), Some("a = [|]"));
        assert_eq!(typed_of("x |", '"').as_deref(), Some("x \"|\""));
    }

    #[test]
    fn a_selection_is_surrounded_not_replaced() {
        assert_eq!(typed_of("a |bcd| e", '(').as_deref(), Some("a (|bcd|) e"));
        assert_eq!(typed_of("a |bcd| e", '"').as_deref(), Some("a \"|bcd|\" e"));
    }

    #[test]
    fn typing_a_closer_that_is_already_there_steps_over_it() {
        assert_eq!(typed_of("f(a|)", ')').as_deref(), Some("f(a)|"));
    }

    #[test]
    fn nothing_is_auto_closed_in_front_of_a_word() {
        assert_eq!(typed_of("|foo", '('), None);
        assert_eq!(typed_of("|foo", '"'), None);
    }

    /// `don` + `'` is an apostrophe in prose and a lifetime in code. Neither
    /// wants a second quote.
    #[test]
    fn a_quote_after_a_word_is_not_an_opening_quote() {
        assert_eq!(typed_of("don|", '\''), None);
        assert_eq!(typed_of("fn f<|", '\'').as_deref(), Some("fn f<'|'"));
    }

    #[test]
    fn a_caret_from_outside_the_text_is_not_a_panic() {
        // A replace-all that shortened the file, or a buffer whose text was
        // set wholesale, leaves a caret past the end for one build. This used
        // to slice and panic, which took the whole code pane down to the error
        // placeholder — for a highlight nobody would have missed.
        // Clamped to the end rather than refused: the last bracket is still a
        // real pair and highlighting it is the right answer.
        assert_eq!(match_bracket("()", 99), Some((1, 0)));
        // And a caret inside a multi-byte character, which is the same panic
        // by a different route.
        assert_eq!(match_bracket("(é)", 2), None);
    }

    #[test]
    fn brackets_are_not_auto_closed_inside_a_string() {
        assert_eq!(typed_of(r#"let s = "hello |"#, '('), None);
    }

    #[test]
    fn backspace_between_a_pair_deletes_both() {
        let (text, selection) = parse("f(|)");
        assert_eq!(show(&backspace_pair(&text, selection).unwrap()), "f|");
        let (text, selection) = parse("f(a|)");
        assert_eq!(backspace_pair(&text, selection), None);
    }

    // ---- matching ----------------------------------------------------------

    #[test]
    fn a_bracket_finds_its_partner_in_both_directions() {
        let text = "fn f() { g(); }";
        let open = text.find('{').unwrap();
        let close = text.rfind('}').unwrap();
        assert_eq!(match_bracket(text, open), Some((open, close)));
        assert_eq!(match_bracket(text, close + 1), Some((close, open)));
    }

    #[test]
    fn matching_counts_depth() {
        let text = "a(b(c)d)e";
        let outer_open = 1;
        let outer_close = text.rfind(')').unwrap();
        assert_eq!(
            match_bracket(text, outer_open),
            Some((outer_open, outer_close))
        );
    }

    /// The case the scanner exists for: a `}` in a comment must not be the
    /// partner of a real `{`.
    #[test]
    fn a_bracket_in_a_comment_is_never_a_partner() {
        let text = "fn f() { // }\n    g();\n}";
        let open = text.find('{').unwrap();
        let real_close = text.rfind('}').unwrap();
        assert_eq!(match_bracket(text, open), Some((open, real_close)));
    }

    #[test]
    fn an_unbalanced_bracket_matches_nothing() {
        assert_eq!(match_bracket("fn f() {", 7), None);
        assert_eq!(match_bracket("plain text", 3), None);
    }

    // ---- comments ----------------------------------------------------------

    fn toggle_of(marked: &str) -> String {
        let (text, selection) = parse(marked);
        toggle_comment(&text, Language::Rust, selection).text
    }

    /// The gap this closes: `//` in a `Cargo.toml` is not a comment, it is a
    /// parse error written into the user's file by a keystroke.
    #[test]
    fn the_marker_comes_from_the_language() {
        let toml = "name = \"app\"";
        let commented = toggle_comment(toml, Language::Toml, (0, toml.len()));
        assert_eq!(commented.text, "# name = \"app\"");
        let back = toggle_comment(&commented.text, Language::Toml, (0, commented.text.len()));
        assert_eq!(back.text, toml);

        let sql = "select 1;";
        assert_eq!(
            toggle_comment(sql, Language::Sql, (0, sql.len())).text,
            "-- select 1;"
        );
    }

    /// A language with no line comment gets no edit at all, rather than a `//`
    /// that breaks the file.
    #[test]
    fn a_language_with_no_line_comment_is_a_no_op() {
        let json = "{\n  \"a\": 1\n}";
        let edit = toggle_comment(json, Language::Json, (0, json.len()));
        assert_eq!(edit.text, json);
        assert_eq!(edit.selection, (0, json.len()));
    }

    #[test]
    fn one_line_toggles_both_ways() {
        assert_eq!(toggle_of("    let a = 1;|"), "    // let a = 1;");
        assert_eq!(toggle_of("    // let a = 1;|"), "    let a = 1;");
    }

    /// The `//` goes at the shallowest indent of the block, not at column zero,
    /// so the shape of the code survives a round trip.
    #[test]
    fn a_block_is_commented_at_its_own_indent() {
        let source = "    |if x {\n        y();\n    }|";
        let commented = toggle_of(source);
        assert_eq!(commented, "    // if x {\n    //     y();\n    // }");

        // And back again, exactly.
        let (text, _) = parse(source);
        let round_trip = toggle_comment(&commented, Language::Rust, (0, commented.len()));
        assert_eq!(round_trip.text, text);
    }

    /// A half-commented block comments the rest rather than uncommenting the
    /// half — so pressing the key twice is comment, then uncomment.
    #[test]
    fn a_mixed_block_comments_before_it_uncomments() {
        let text = "// a\nb\n";
        let commented = toggle_comment(text, Language::Rust, (0, text.len()));
        assert_eq!(commented.text, "// // a\n// b\n");
        let back = toggle_comment(&commented.text, Language::Rust, (0, commented.text.len()));
        assert_eq!(back.text, text);
    }

    #[test]
    fn blank_lines_are_left_alone() {
        let text = "a\n\nb";
        assert_eq!(
            toggle_comment(text, Language::Rust, (0, text.len())).text,
            "// a\n\n// b"
        );
    }

    /// `//x` with no space must not lose the `x` when uncommented.
    #[test]
    fn uncommenting_only_eats_a_space_that_is_there() {
        assert_eq!(toggle_of("//x|"), "x");
        assert_eq!(toggle_of("// x|"), "x");
    }

    // ---- indent ------------------------------------------------------------

    #[test]
    fn tab_with_no_selection_inserts_one_level() {
        let (text, selection) = parse("ab|cd");
        assert_eq!(show(&shift_indent(&text, selection, true)), "ab    |cd");
    }

    #[test]
    fn a_selection_indents_and_outdents_by_line() {
        let text = "a\nb\nc";
        let deeper = shift_indent(text, (0, text.len()), true);
        assert_eq!(deeper.text, "    a\n    b\n    c");
        let back = shift_indent(&deeper.text, (0, deeper.text.len()), false);
        assert_eq!(back.text, text);
    }

    #[test]
    fn outdenting_an_unindented_line_does_nothing_to_it() {
        let text = "a\n  b";
        let out = shift_indent(text, (0, text.len()), false);
        assert_eq!(out.text, "a\nb", "a partial indent is removed entirely");
    }

    #[test]
    fn indenting_leaves_blank_lines_blank() {
        let text = "a\n\nb";
        assert_eq!(
            shift_indent(text, (0, text.len()), true).text,
            "    a\n\n    b"
        );
    }

    // ---- offsets -----------------------------------------------------------

    /// Every offset this file produces is used to slice a `&str`, and slicing
    /// one mid-character panics. Multi-byte text is not an edge case in a
    /// source file — it is every string literal anybody writes in.
    #[test]
    fn multibyte_text_never_produces_an_offset_mid_character() {
        let text = "    let s = \"héllo 😀\";\nlet t = 2;";
        for at in 0..=text.len() {
            if !text.is_char_boundary(at) {
                continue;
            }
            let edit = newline(text, (at, at));
            assert!(
                edit.text.is_char_boundary(edit.selection.0),
                "newline at {at} split a character"
            );
            let toggled = toggle_comment(text, Language::Rust, (at, at));
            assert!(toggled.text.is_char_boundary(toggled.selection.0));
            assert!(toggled.text.is_char_boundary(toggled.selection.1));
            // Must not panic.
            let _ = match_bracket(text, at);
            let _ = typed_bracket(text, (at, at), '(');
            let _ = backspace_pair(text, (at, at));
        }
    }

    // ---- the diff seam -----------------------------------------------------

    #[test]
    fn typing_a_newline_indents_as_return_would() {
        let before = "    let a = 1;";
        let after = "    let a = 1;\n";
        let edit = after_typing(before, (before.len(), before.len()), after).unwrap();
        assert_eq!(show(&edit), "    let a = 1;\n    |");
    }

    #[test]
    fn typing_an_opener_closes_it() {
        let edit = after_typing("f", (1, 1), "f(").unwrap();
        assert_eq!(show(&edit), "f(|)");
    }

    #[test]
    fn typing_over_a_selection_is_still_one_character() {
        // `abc` selected, `(` typed: the selection is surrounded, not lost.
        let edit = after_typing("a bcd e", (2, 5), "a ( e").unwrap();
        assert_eq!(show(&edit), "a (|bcd|) e");
    }

    /// Everything that is not one typed character must pass through untouched,
    /// or this seam would rewrite pastes, deletes and its own output.
    #[test]
    fn nothing_else_is_touched() {
        // A paste.
        assert_eq!(after_typing("a", (1, 1), "a(hello)"), None);
        // A delete.
        assert_eq!(after_typing("abc", (3, 3), "ab"), None);
        // A caret move: no change at all.
        assert_eq!(after_typing("abc", (3, 3), "abc"), None);
        // An ordinary character, which has no comfort attached.
        assert_eq!(after_typing("ab", (2, 2), "abc"), None);
        // This module's own output arriving back through the same path.
        let indented = newline("    x", (5, 5)).text;
        assert_eq!(after_typing("    x", (5, 5), &indented), None);
        // A selection replaced by more than one character.
        assert_eq!(after_typing("abcd", (1, 3), "aXYd"), None);
        // An out-of-range selection, which a stale caret can produce.
        assert_eq!(after_typing("ab", (9, 9), "ab("), None);
    }

    /// Typing `(` before a word does nothing special — and must therefore
    /// answer `None` here rather than an `Edit` that re-does the insertion,
    /// which would cost an undo step for every ordinary keystroke.
    #[test]
    fn a_comfort_that_declines_declines_all_the_way_up() {
        assert_eq!(after_typing("foo", (0, 0), "(foo"), None);
    }

    #[test]
    fn an_empty_buffer_is_not_a_special_case() {
        assert_eq!(newline("", (0, 0)).text, "\n");
        assert_eq!(toggle_comment("", Language::Rust, (0, 0)).text, "");
        assert_eq!(match_bracket("", 0), None);
        assert_eq!(backspace_pair("", (0, 0)), None);
        assert_eq!(shift_indent("", (0, 0), false).text, "");
    }

    // ------------------------------------------------------------- snippets

    fn snippet(name: &str) -> &'static Snippet {
        SNIPPETS
            .iter()
            .find(|s| s.name == name)
            .expect("no such snippet")
    }

    #[test]
    fn brace_depth_ignores_braces_in_strings_and_comments() {
        assert_eq!(brace_depth("", 0), 0);
        assert_eq!(brace_depth("fn f() {\n    x\n}", 12), 1);
        assert_eq!(brace_depth("fn f() {\n    x\n}", 16), 0);
        assert_eq!(
            brace_depth("let s = \"{\";\n", 13),
            0,
            "a brace in a string"
        );
        assert_eq!(brace_depth("// {\n", 5), 0, "a brace in a comment");
        assert_eq!(brace_depth("fn f() { if x { } }", 16), 2);
    }

    /// The bug this whole section exists for: clicking an expression card on an
    /// empty buffer used to produce `expected one of `!` or `::`, found `(``.
    #[test]
    fn an_expression_is_refused_at_item_position() {
        let error = insert_snippet("", (0, 0), snippet("Centered text"))
            .expect_err("an expression at the top level must be refused");
        assert!(error.contains("Widget shell"), "{error}");
    }

    #[test]
    fn an_item_is_refused_inside_a_body() {
        let text = "fn build() {\n    \n}";
        let error = insert_snippet(text, (17, 17), snippet("Widget shell"))
            .expect_err("an item inside a body must be refused");
        assert!(error.contains("top level"), "{error}");
    }

    #[test]
    fn an_item_lands_on_an_empty_buffer() {
        let edit = insert_snippet("", (0, 0), snippet("Widget shell")).expect("item at depth 0");
        assert!(edit
            .text
            .starts_with("#[derive(Debug)]\npub struct Screen;"));
        assert!(edit.text.contains("fn build(&self, ctx: &BuildContext)"));
        assert_eq!(edit.selection, (edit.text.len(), edit.text.len()));
    }

    /// The whole point. Four clicks used to give one 388-character line; they
    /// now give four separate, indented blocks.
    #[test]
    fn repeated_inserts_never_share_a_line() {
        let mut text = String::new();
        let mut caret = 0;
        for _ in 0..4 {
            let edit = insert_snippet(&text, (caret, caret), snippet("Widget shell"))
                .expect("item at depth 0");
            text = edit.text;
            caret = edit.selection.1;
        }
        assert_eq!(text.matches("pub struct Screen;").count(), 4);
        let longest = text.lines().map(str::len).max().unwrap_or(0);
        assert!(longest < 80, "longest line was {longest}: {text}");
    }

    #[test]
    fn an_expression_is_indented_to_where_it_lands() {
        // Caret on the blank line inside `build`, indented eight spaces.
        let text = "impl W {\n    fn build(&self) {\n        \n    }\n}";
        let caret = text.find("        \n").expect("the blank line") + 8;
        let edit = insert_snippet(text, (caret, caret), snippet("Centered text"))
            .expect("an expression inside a body");
        assert!(
            edit.text
                .contains("        Container::new()\n            .alignment"),
            "{}",
            edit.text
        );
        // And the line it replaced is gone rather than left blank above it.
        assert!(!edit.text.contains("\n        \n        Container"));
    }

    /// A caret in the middle of a line must not split that line.
    #[test]
    fn a_snippet_never_splits_the_line_the_caret_is_on() {
        let text = "impl W {\n    fn build(&self) {\n        let x = 1;\n    }\n}";
        let caret = text.find("let x").expect("the statement") + 3;
        let edit = insert_snippet(text, (caret, caret), snippet("Button"))
            .expect("an expression inside a body");
        assert!(edit.text.contains("        let x = 1;\n"), "{}", edit.text);
        assert!(
            edit.text
                .contains("\n        Button::new(Text::new(\"Click\"))"),
            "{}",
            edit.text
        );
    }

    /// Every body is written at zero indent and every line of it is either
    /// blank or starts with real content — otherwise `reindented` produces
    /// double indentation the first time somebody edits the catalogue.
    #[test]
    fn every_snippet_body_is_written_at_zero_indent() {
        for snippet in &SNIPPETS {
            let first = snippet.body.lines().next().unwrap_or_default();
            assert!(!first.starts_with(' '), "{} starts indented", snippet.name);
            assert!(
                !snippet.body.ends_with('\n'),
                "{} has a trailing newline",
                snippet.name
            );
            // Where to read more, and short enough for a 248-point sidebar —
            // `Text` has no truncation, so an over-long one does not clip, it
            // overflows. `widgets/flex.rs` is the form the mapping document and
            // the prototype both use; `widget.rs` sits at the root of
            // `vieww-widget/src`, so a directory is not something every entry
            // has to have.
            //
            // **Not every entry names a file any more.** The function-level
            // snippets are shapes rather than wrappers around one widget —
            // "the contract Render looks for", "a function that returns a
            // node" — and pointing those at a `crates/` path would be pointing
            // at the wrong thing. The line still has to say *something*, and
            // still has to fit.
            assert!(
                !snippet.source.trim().is_empty(),
                "{} has no source line",
                snippet.name
            );
            assert!(
                snippet.source.len() <= 32,
                "{}'s source path is too long for the sidebar: {}",
                snippet.name,
                snippet.source
            );
        }
    }

    /// The refusal message names "Widget shell" by name, so that entry has to
    /// exist and has to be an item.
    ///
    /// This once asserted there was exactly *one* item, back when the library
    /// was ten expressions and a single shell. That was the gap somebody
    /// reported: a person with a file open and a caret in it had nothing to
    /// insert but fragments. The invariant that actually matters is the one the
    /// message depends on, not the count.
    #[test]
    fn the_library_has_one_way_in() {
        let shell = SNIPPETS
            .iter()
            .find(|s| s.name == "Widget shell")
            .expect("the refusal message names this entry");
        assert_eq!(shell.kind, SnippetKind::Item);
    }
}
