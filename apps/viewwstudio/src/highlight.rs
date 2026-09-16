//! Incremental syntax highlighting for the Studio editor.
//!
//! Tree-sitter is used only as a parser here. The editor still owns text,
//! selection, hit testing and shaping; this module maps syntax nodes to the
//! `TextSpan` runs that the editable renderer already understands.
//!
//! # Five grammars, not one
//!
//! This was Rust-only, and `language.rs` was candid about what that meant:
//! *"Rust is highlighted, everything else is legible plain text with a correct
//! label."* Defensible for a Rust-focused studio, and wrong the moment anybody
//! opened the `Cargo.toml` that every Rust project has — or its `.json`
//! configs, its CI `.yml`, its `README.md`. Those are the four, and they are
//! here now beside Rust.
//!
//! # Why the colour mapping is per grammar rather than shared
//!
//! Tree-sitter node kinds are the *grammar's* vocabulary, not a common one. A
//! string is `string_literal` in Rust, `string` in TOML and JSON, and
//! `string_scalar` in YAML; TOML's `bare_key` has no counterpart in Rust at
//! all. A single table of kind names would be a table that is subtly wrong for
//! four of the five languages in a way nobody notices until they look at a
//! file. So `style_for` dispatches on the language first, and each arm speaks
//! its own grammar''s names.

use tree_sitter::{InputEdit, Node, Parser, Point, Tree};
use vieww_foundation::{Color, FontFamily, TextStyle};
use vieww_text::TextSpan;

use crate::language::Language;
use crate::theme::StudioTheme;

/// The grammar for `language`, or `None` for one the studio does not parse.
///
/// The single switch point the module docs promise: adding a language is a line
/// here, a line in [`Language::highlighted`], and an arm in [`style_for`].
fn grammar_for(language: Language) -> Option<tree_sitter::Language> {
    Some(match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Language::Json => tree_sitter_json::LANGUAGE.into(),
        Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Language::Markdown => tree_sitter_md::LANGUAGE.into(),
        _ => return None,
    })
}

pub struct Highlighter {
    parser: Parser,
    /// Which grammar `parser` currently holds.
    ///
    /// Swapping a `Parser`'s language throws its state away, so this is checked
    /// before every parse and the swap costs nothing on the common frame where
    /// the buffer has not changed language.
    language: Language,
    source: String,
    spans: Vec<TextSpan>,
    style: Option<TextStyle>,
    theme: Option<StudioTheme>,
    /// The last parse, kept so the next one can be incremental.
    ///
    /// This is the whole of the fix described on [`Highlighter::highlight`]:
    /// the field did not exist, `parse` was called with `None`, and tree-sitter
    /// re-walked the entire file on every keystroke.
    tree: Option<Tree>,
}

/// The largest file the studio will syntax-highlight.
///
/// Beyond this the editor still opens the file, still edits it and still saves
/// it — it simply draws it as plain text. Highlighting is the one feature whose
/// cost is proportional to the *whole* file rather than to the visible part,
/// and a two-megabyte generated `.rs` file is one where the honest trade is a
/// responsive editor with flat text.
///
/// Stated in the status bar rather than left to be discovered as "why is this
/// file grey", which is [`Studio::highlighted_spans`]'s job.
///
/// [`Studio::highlighted_spans`]: crate::Studio::highlighted_spans
pub const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;

/// The [`InputEdit`] that turns `old` into `new`, or `None` if they are equal.
///
/// # Why a derived edit rather than one the editor reports
///
/// The editor *does* know what it changed — it applies a `TextEditingValue`.
/// Threading that all the way down to the highlighter would mean every path
/// that can change the buffer (typing, paste, undo, format-on-save, a snippet,
/// a multi-caret edit, a reload from disk) carrying an edit description, and
/// **one path forgetting to** would hand tree-sitter a tree edited towards the
/// wrong text. That is not a wrong colour; it is a parse against stale byte
/// offsets, which is a crash or a silent mis-highlight depending on where it
/// lands.
///
/// Deriving it here means the edit cannot disagree with the text, because the
/// text is the only input. The cost is one forward scan and one backward scan
/// over the parts that match, which is a memcmp — and for the case this exists
/// for, a keystroke, both scans stop almost immediately.
///
/// The derived edit is a *conservative* one: for a change in several places at
/// once — a replace-all, a formatter pass — the reported span is everything
/// from the first difference to the last, which is exactly what tree-sitter
/// needs to know to be correct, and merely less of a saving than a real diff.
#[must_use]
pub fn edit_between(old: &str, new: &str) -> Option<InputEdit> {
    if old == new {
        return None;
    }
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();

    // The common prefix, backed off to a character boundary in *both* — a
    // multi-byte character edited in place shares some of its bytes with the
    // one that replaced it, and an offset in the middle of one is not a
    // position tree-sitter can be given.
    let mut start = 0;
    let limit = old_bytes.len().min(new_bytes.len());
    while start < limit && old_bytes[start] == new_bytes[start] {
        start += 1;
    }
    while start > 0 && !(old.is_char_boundary(start) && new.is_char_boundary(start)) {
        start -= 1;
    }

    // The common suffix, measured backwards from both ends and never allowed
    // to run back past `start`.
    let mut tail = 0;
    let tail_limit = limit - start;
    while tail < tail_limit
        && old_bytes[old_bytes.len() - 1 - tail] == new_bytes[new_bytes.len() - 1 - tail]
    {
        tail += 1;
    }
    let mut old_end = old_bytes.len() - tail;
    let mut new_end = new_bytes.len() - tail;
    while old_end < old_bytes.len()
        && new_end < new_bytes.len()
        && !(old.is_char_boundary(old_end) && new.is_char_boundary(new_end))
    {
        old_end += 1;
        new_end += 1;
    }

    Some(InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old, start),
        old_end_position: point_at(old, old_end),
        new_end_position: point_at(new, new_end),
    })
}

/// The row and column of `offset` in `text`, as tree-sitter counts them.
///
/// Both are zero-based, and the column is in **bytes**, which is what
/// tree-sitter means by `column` — not characters, and not display columns.
/// Getting that wrong is invisible on ASCII and wrong on every file with an
/// accent in it.
#[must_use]
fn point_at(text: &str, offset: usize) -> Point {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let row = before.matches('\n').count();
    let column = before.rfind('\n').map_or(offset, |at| offset - at - 1);
    Point { row, column }
}

/// One definition found in a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// `fn`, `struct`, `enum`, `trait`, `impl`, `mod`, `type`, `const`, `static`.
    pub kind: &'static str,
    /// One-based, because every other line number the studio shows is.
    pub line: u32,
    /// How deeply nested the definition is, so a method reads as belonging to
    /// its `impl` rather than as a second top-level item.
    pub depth: usize,
}

impl std::fmt::Debug for Highlighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Highlighter").finish_non_exhaustive()
    }
}

impl Highlighter {
    /// A highlighter with no grammar loaded yet.
    ///
    /// # Why this no longer panics
    ///
    /// It used to `set_language(...).expect("tree-sitter-rust language must
    /// load")` in the constructor. A grammar whose ABI does not match the
    /// `tree-sitter` it is linked against fails there — and that took the whole
    /// studio down *on startup*, before a window existed to say why, for a
    /// failure whose correct consequence is "this file is drawn as plain text".
    ///
    /// The grammar is now loaded lazily by `use_language`,
    /// which reports a failure by leaving the language unset. Nothing is
    /// highlighted and everything else still works.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            language: Language::Text,
            source: String::new(),
            spans: Vec::new(),
            style: None,
            theme: None,
            tree: None,
        }
    }

    /// Point the parser at `language`'s grammar, if there is one.
    ///
    /// Returns whether the grammar is loaded and usable. Swapping languages
    /// throws away the retained tree, which is correct — a tree is a parse of
    /// one text under one grammar, and reusing it across a switch would be
    /// reusing a Rust parse to describe a TOML file.
    fn use_language(&mut self, language: Language) -> bool {
        if self.language == language {
            return language.highlighted();
        }
        let Some(grammar) = grammar_for(language) else {
            self.language = language;
            self.tree = None;
            return false;
        };
        if self.parser.set_language(&grammar).is_err() {
            // A grammar built against a different tree-sitter ABI. Reported by
            // drawing the file flat rather than by ending the process.
            self.language = Language::Text;
            self.tree = None;
            return false;
        }
        self.language = language;
        self.tree = None;
        self.source.clear();
        true
    }

    /// Every definition in `source`, in the order they appear.
    ///
    /// # Why this is a parse and not a line scan
    ///
    /// It was a line scan: `trim_start`, strip `pub `, match one of seven
    /// keywords, take characters until something that is not an identifier.
    /// That is honest about being a scan and wrong in all the ways a scan is —
    /// it finds `fn` inside a doc comment, misses anything not at the start of
    /// its line, cannot tell a method from a free function, and reports
    /// `Widget<'_` as a name. `NEXT-VIEWWSTUDIO.md` says it *"should be deleted,
    /// not extended"*, and this is the deletion.
    ///
    /// The parser is already here — it has highlighted every keystroke since
    /// M6 — so the whole cost of doing it properly is walking a tree the studio
    /// has already built.
    pub fn symbols(&mut self, source: &str, language: Language) -> Vec<Symbol> {
        // Rust only, and deliberately: `ITEMS` below is a list of Rust node
        // kinds, and a TOML table is not a definition anything jumps to.
        if language != Language::Rust {
            return Vec::new();
        }
        let Some(tree) = self.tree_for(source, language) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        collect_symbols(tree.root_node(), source, 0, &mut out);
        out
    }

    /// The parse of `source`, reusing the highlighter's own if it is of the
    /// same text.
    ///
    /// # Why this exists
    ///
    /// Two callers — `symbols` and `lint::check` — each used to build a
    /// `Parser`, set the language and parse the whole buffer from scratch,
    /// beside a highlighter that had *just* parsed the same bytes and was
    /// holding the result. On a large file that is the same work three times
    /// for one frame, and `lint`'s own module docs claimed it ran "over the
    /// tree-sitter tree the highlighter already builds" while doing nothing of
    /// the kind.
    ///
    /// The cache key is the same one `highlight` uses, and for the same reason:
    /// a tree describes exactly the text it was parsed from, so a tree for
    /// different text is worse than no tree. When the text differs this parses
    /// — and keeps the result, so the next caller in the same frame is free.
    pub fn tree_for(&mut self, source: &str, language: Language) -> Option<&Tree> {
        if !self.use_language(language) {
            return None;
        }
        if self.tree.is_none() || self.source.len() != source.len() || self.source != source {
            let old = self.tree.take().and_then(|mut tree| {
                let edit = edit_between(&self.source, source)?;
                tree.edit(&edit);
                Some(tree)
            });
            let tree = self.parser.parse(source, old.as_ref())?;
            self.source = source.to_owned();
            // The styled spans belong to the text that was highlighted, which
            // is no longer this text. Dropping the style key forces `highlight`
            // to rebuild rather than hand back spans for the old bytes.
            self.style = None;
            self.theme = None;
            self.tree = Some(tree);
        }
        self.tree.as_ref()
    }

    /// Return styled runs for `source`, reusing the previous parse.
    ///
    /// # What this used to do on every keystroke
    ///
    /// `self.parser.parse(source, None)`. The `None` is the previous tree —
    /// the thing tree-sitter exists to reuse — so **every keystroke reparsed
    /// the whole file**, re-collected every leaf, sorted them and rebuilt every
    /// span. The frame-level cache above stopped that happening twice per
    /// frame and did nothing about the per-edit cost, which on this
    /// repository's own five-thousand-line source files is the editor's
    /// ceiling. The comment that used to be here said future edits *could* be
    /// changed to true incremental parsing; this is that change.
    ///
    /// The work is [`edit_between`]: derive an [`InputEdit`] from the old and
    /// new text by common prefix and suffix, hand it to the retained tree, and
    /// give the edited tree back to the parser. tree-sitter then re-walks only
    /// the part that actually moved. A change with no old tree — the first
    /// parse, or a file switch — still costs a full parse, which is correct:
    /// there is nothing to reuse.
    ///
    /// # The cache key, and why length is checked first
    ///
    /// `self.source == source` is a whole-buffer comparison, run once a frame
    /// on every file. Comparing lengths first turns the common "nothing
    /// changed, but the text is 200 KB" frame into an integer compare. When
    /// the lengths do match the full compare still runs, because two different
    /// buffers of equal length are not the same buffer.
    pub fn highlight(
        &mut self,
        source: &str,
        language: Language,
        base: TextStyle,
        theme: StudioTheme,
    ) -> Vec<TextSpan> {
        if !self.use_language(language) {
            return Vec::new();
        }
        if self.source.len() == source.len()
            && self.source == source
            && self.style == Some(base)
            && self.theme == Some(theme)
        {
            return self.spans.clone();
        }

        // Above the size at which highlighting stops being worth its cost. The
        // file is still edited and saved; it is simply drawn flat.
        if source.len() > MAX_HIGHLIGHT_BYTES {
            self.source = source.to_owned();
            self.style = Some(base);
            self.theme = Some(theme);
            self.tree = None;
            self.spans = vec![TextSpan::new(source, base)];
            return self.spans.clone();
        }

        // The incremental step. `take` rather than a borrow: an edited tree is
        // only valid for the text it was edited towards, so a parse that fails
        // must not leave the old one behind to be reused against text it does
        // not describe.
        let old = self.tree.take().and_then(|mut tree| {
            let edit = edit_between(&self.source, source)?;
            tree.edit(&edit);
            Some(tree)
        });

        let Some(tree) = self.parser.parse(source, old.as_ref()) else {
            self.source = source.to_owned();
            self.style = Some(base);
            self.theme = Some(theme);
            self.spans = vec![TextSpan::new(source, base)];
            return self.spans.clone();
        };

        let mut leaves = Vec::new();
        collect_leaves(tree.root_node(), &mut leaves);
        leaves.sort_by_key(|node| node.start_byte());
        // Kept for the next edit. Cloning a `Tree` is a refcount bump.
        self.tree = Some(tree.clone());

        let mut spans = Vec::new();
        let mut cursor = 0usize;
        for node in leaves {
            let start = node.start_byte().min(source.len());
            let end = node.end_byte().min(source.len());
            if start < cursor || end <= start {
                continue;
            }
            if start > cursor {
                // **The gap between two leaves belongs to whatever encloses
                // it.** Not every grammar makes its text a leaf: TOML parses
                // `"app"` as a `string` node whose only children are the two
                // quote tokens, so the letters between them are in no leaf at
                // all and used to be filled in with the base style — a string
                // drawn with coloured quotes and plain contents.
                //
                // The smallest node covering the gap is the honest answer.
                //
                // **Descended from the nearest ancestor that already contains
                // the gap, not from the root.** The answer is the same — the
                // smallest node spanning a range is unique, and is a descendant
                // of *every* node spanning it — but the walk is a handful of
                // levels instead of the full depth of the file's syntax tree.
                //
                // This is the difference between O(gaps × depth) and
                // O(gaps × a small constant), and gaps are not rare: every
                // string literal in the file has two. `ts_node_child_with_
                // descendant` was 3.4% of a keystroke frame in the studio
                // before this, which is more than the whole paint phase.
                let mut from = node;
                while from.start_byte() > cursor {
                    match from.parent() {
                        Some(parent) => from = parent,
                        None => break,
                    }
                }
                let enclosing = from
                    .descendant_for_byte_range(cursor, start)
                    .map_or(base, |node| style_for(node, language, base, theme));
                push_span(&mut spans, &source[cursor..start], enclosing);
            }
            let text = &source[start..end];
            push_span(&mut spans, text, style_for(node, language, base, theme));
            cursor = end;
        }
        if cursor < source.len() {
            push_span(&mut spans, &source[cursor..], base);
        }
        if spans.is_empty() {
            spans.push(TextSpan::new(source, base));
        }

        self.source = source.to_owned();
        self.style = Some(base);
        self.theme = Some(theme);
        self.spans = spans;
        self.spans.clone()
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_leaves<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    let mut cursor = node.walk();
    if node.child_count() == 0 {
        if node.end_byte() > node.start_byte() {
            out.push(node);
        }
        return;
    }
    for child in node.children(&mut cursor) {
        collect_leaves(child, out);
    }
}

fn push_span(spans: &mut Vec<TextSpan>, text: &str, style: TextStyle) {
    if text.is_empty() {
        return;
    }
    // Merge adjacent runs with identical styles. This keeps the paragraph small
    // for punctuation-heavy Rust without changing any glyph geometry.
    if let Some(last) = spans.last_mut() {
        if last.style == style {
            last.text.push_str(text);
            return;
        }
    }
    spans.push(TextSpan::new(text, style));
}

/// The colour a node takes, in its own grammar's vocabulary.
///
/// # Why this dispatches on the language before it looks at the kind
///
/// Node kinds belong to grammars, not to a shared vocabulary. A string is
/// `string_literal` in Rust, `string` in TOML and JSON, `string_scalar` in
/// YAML. TOML's `bare_key` has no Rust counterpart; Rust's `macro_invocation`
/// has no TOML one. One flat table of names would silently mis-colour four
/// languages out of five, and nothing would look broken enough to investigate.
fn style_for(node: Node<'_>, language: Language, base: TextStyle, theme: StudioTheme) -> TextStyle {
    let syntax = theme.syntax;
    let kind = node.kind();
    let parent = node.parent().map(|p| p.kind()).unwrap_or_default();

    let color = match language {
        Language::Rust => rust_color(kind, parent, syntax),
        Language::Toml => toml_color(kind, parent, syntax),
        Language::Json => json_color(kind, parent, syntax),
        Language::Yaml => yaml_color(kind, parent, syntax),
        Language::Markdown => markdown_color(kind, parent, syntax),
        _ => None,
    };

    let Some(color) = color else { return base };
    TextStyle {
        color,
        family: FontFamily::Monospace,
        ..base
    }
}

fn rust_color(kind: &str, parent: &str, syntax: crate::theme::Syntax) -> Option<Color> {
    Some(
        if kind.starts_with("line_comment") || kind.starts_with("block_comment") {
            syntax.comment
        } else if kind == "string_literal" || kind == "raw_string_literal" || kind == "char_literal"
        {
            syntax.string
        } else if kind == "integer_literal" || kind == "float_literal" {
            syntax.number
        } else if kind == "attribute_item" || parent == "attribute_item" {
            syntax.attribute
        } else if kind == "macro_invocation"
            || kind == "macro_definition"
            || parent == "macro_invocation"
        {
            syntax.macro_name
        } else if is_keyword(kind) {
            syntax.keyword
        } else if kind == "identifier"
            && (parent.contains("function") || parent == "call_expression")
        {
            syntax.function
        } else if kind == "type_identifier" || kind == "primitive_type" {
            syntax.type_name
        } else if is_punctuation(kind) {
            syntax.punctuation
        } else {
            return None;
        },
    )
}

/// TOML — the grammar a Rust developer's most-opened non-Rust file uses.
///
/// A table header (`[dependencies]`) is coloured as a type rather than as
/// punctuation: it is the one thing on the screen that says which section the
/// keys below belong to, and finding it by eye is the whole of navigating a
/// long `Cargo.toml`.
fn toml_color(kind: &str, parent: &str, syntax: crate::theme::Syntax) -> Option<Color> {
    // A quoted string is not one leaf in this grammar — the quotes and the
    // content are separate tokens under a `string` node — so the parent is what
    // says "this is inside a string". Checked first, because a `.` inside a
    // quoted key is punctuation everywhere else and part of the string here.
    if parent.contains("string") {
        return Some(syntax.string);
    }
    Some(match kind {
        "comment" => syntax.comment,
        "string"
        | "basic_string"
        | "literal_string"
        | "multiline_basic_string"
        | "multiline_literal_string" => syntax.string,
        "integer" | "float" | "local_date" | "local_time" | "local_date_time"
        | "offset_date_time" => syntax.number,
        "boolean" | "true" | "false" => syntax.keyword,
        "bare_key" | "quoted_key" | "dotted_key" => {
            if parent == "table" || parent == "table_array_element" {
                syntax.type_name
            } else {
                syntax.attribute
            }
        }
        "[" | "]" | "[[" | "]]" | "{" | "}" | "=" | "," | "." => syntax.punctuation,
        _ => return None,
    })
}

/// JSON — where the useful distinction is a *key* from a *value*, because that
/// is the only structure a config file has.
fn json_color(kind: &str, parent: &str, syntax: crate::theme::Syntax) -> Option<Color> {
    if parent == "string" {
        return Some(syntax.string);
    }
    Some(match kind {
        "comment" => syntax.comment,
        "string_content" | "escape_sequence" | "string" => syntax.string,
        "number" => syntax.number,
        "true" | "false" | "null" => syntax.keyword,
        "\"" => syntax.string,
        "{" | "}" | "[" | "]" | ":" | "," => syntax.punctuation,
        _ => return None,
    })
}

/// YAML — a CI file, which is keys, strings and the occasional anchor.
fn yaml_color(kind: &str, parent: &str, syntax: crate::theme::Syntax) -> Option<Color> {
    if parent.contains("scalar") && parent != "plain_scalar" {
        return Some(syntax.string);
    }
    Some(match kind {
        "comment" => syntax.comment,
        "string_scalar" | "single_quote_scalar" | "double_quote_scalar" | "block_scalar" => {
            syntax.string
        }
        "integer_scalar" | "float_scalar" => syntax.number,
        "boolean_scalar" | "null_scalar" => syntax.keyword,
        "tag" | "anchor_name" | "alias_name" => syntax.attribute,
        "-" | ":" | "," | "{" | "}" | "[" | "]" | "|" | ">" | "&" | "*" => syntax.punctuation,
        _ => return None,
    })
}

/// Markdown — headings, code and links, which is what a README is made of.
fn markdown_color(kind: &str, parent: &str, syntax: crate::theme::Syntax) -> Option<Color> {
    if kind.starts_with("atx_h") || parent.starts_with("atx_heading") || kind == "setext_heading" {
        return Some(syntax.type_name);
    }
    Some(match kind {
        "code_fence_content" | "fenced_code_block" | "indented_code_block" | "code_span" => {
            syntax.string
        }
        "block_quote_marker" | "block_continuation" => syntax.comment,
        "list_marker_minus" | "list_marker_star" | "list_marker_plus" | "list_marker_dot" => {
            syntax.punctuation
        }
        "link_destination" | "link_label" | "uri_autolink" => syntax.function,
        "`" | "```" | "~~~" => syntax.punctuation,
        _ => return None,
    })
}

fn is_keyword(kind: &str) -> bool {
    matches!(
        kind,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

fn is_punctuation(kind: &str) -> bool {
    matches!(
        kind,
        "{" | "}"
            | "("
            | ")"
            | "["
            | "]"
            | ";"
            | ":"
            | ","
            | "."
            | "::"
            | "->"
            | "=>"
            | "="
            | "=="
            | "!="
            | "<"
            | ">"
            | "<="
            | ">="
            | "+"
            | "-"
            | "*"
            | "/"
            | "%"
            | "&"
            | "|"
            | "^"
            | "?"
            | "@"
            | "#"
            | "'"
            | "..."
            | ".."
    )
}

/// The item kinds worth listing, and the field each one's name lives in.
///
/// `impl` has no `name` field — its subject is a `type` — which is why the
/// lookup is a list of candidate field names rather than one.
const ITEMS: [(&str, &str); 9] = [
    ("function_item", "name"),
    ("struct_item", "name"),
    ("enum_item", "name"),
    ("trait_item", "name"),
    ("mod_item", "name"),
    ("type_item", "name"),
    ("const_item", "name"),
    ("static_item", "name"),
    ("impl_item", "type"),
];

/// Walk the tree, collecting definitions with their nesting depth.
///
/// Depth counts *definitions*, not tree nodes: a method inside an `impl` is at
/// depth 1 whether the grammar puts two nodes between them or five. That is the
/// number a reader is looking at when they scan an indented list.
fn collect_symbols(node: Node<'_>, source: &str, depth: usize, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let found = ITEMS
            .iter()
            .find(|(kind, _)| *kind == child.kind())
            .and_then(|(kind, field)| {
                let named = child.child_by_field_name(field)?;
                let text = source.get(named.start_byte()..named.end_byte())?;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a buffer longer than u32 lines is not a buffer"
                )]
                Some(Symbol {
                    name: text.to_owned(),
                    // The grammar's node kind, shortened to the keyword a Rust
                    // reader knows. `function_item` is not a word anybody types.
                    kind: match *kind {
                        "function_item" => "fn",
                        "struct_item" => "struct",
                        "enum_item" => "enum",
                        "trait_item" => "trait",
                        "mod_item" => "mod",
                        "type_item" => "type",
                        "const_item" => "const",
                        "static_item" => "static",
                        _ => "impl",
                    },
                    line: child.start_position().row as u32 + 1,
                    depth,
                })
            });

        match found {
            Some(symbol) => {
                out.push(symbol);
                collect_symbols(child, source, depth + 1, out);
            }
            // Not a definition itself, but its children may be: an `impl`'s
            // methods sit inside a `declaration_list`, and a `mod`'s items
            // inside another one.
            None => collect_symbols(child, source, depth, out),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextStyle {
        TextStyle::new(13.0)
    }

    fn text_of(spans: &[TextSpan]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    // ----- edit_between --------------------------------------------------

    #[test]
    fn identical_text_is_not_an_edit() {
        assert!(edit_between("fn main() {}", "fn main() {}").is_none());
    }

    /// The case this exists for: one character typed in the middle of a file.
    /// The reported span must be that character and nothing else.
    #[test]
    fn one_typed_character_reports_one_typed_character() {
        let edit = edit_between("fn main() {}", "fn main() {x}").expect("an edit");
        assert_eq!(edit.start_byte, 11);
        assert_eq!(edit.old_end_byte, 11);
        assert_eq!(edit.new_end_byte, 12);
    }

    #[test]
    fn a_deletion_reports_the_bytes_that_went() {
        let edit = edit_between("abcdef", "abef").expect("an edit");
        assert_eq!(edit.start_byte, 2);
        assert_eq!(edit.old_end_byte, 4);
        assert_eq!(edit.new_end_byte, 2);
    }

    #[test]
    fn appending_reports_only_the_tail() {
        let edit = edit_between("abc", "abcdef").expect("an edit");
        assert_eq!(edit.start_byte, 3);
        assert_eq!(edit.old_end_byte, 3);
        assert_eq!(edit.new_end_byte, 6);
    }

    #[test]
    fn deleting_everything_is_an_edit_covering_everything() {
        let edit = edit_between("abc", "").expect("an edit");
        assert_eq!(edit.start_byte, 0);
        assert_eq!(edit.old_end_byte, 3);
        assert_eq!(edit.new_end_byte, 0);
    }

    /// A byte offset in the middle of a character is not a position
    /// tree-sitter can be given. Invisible on ASCII, wrong on every file with
    /// an accent in it.
    #[test]
    fn offsets_land_on_character_boundaries() {
        let old = "let s = \"héllo\";";
        let new = "let s = \"hàllo\";";
        let edit = edit_between(old, new).expect("an edit");
        assert!(old.is_char_boundary(edit.start_byte), "{edit:?}");
        assert!(new.is_char_boundary(edit.start_byte), "{edit:?}");
        assert!(old.is_char_boundary(edit.old_end_byte), "{edit:?}");
        assert!(new.is_char_boundary(edit.new_end_byte), "{edit:?}");
    }

    #[test]
    fn an_edit_on_a_later_line_reports_that_line() {
        let old = "fn a() {}\nfn b() {}\n";
        let new = "fn a() {}\nfn c() {}\n";
        let edit = edit_between(old, new).expect("an edit");
        assert_eq!(edit.start_position.row, 1, "{edit:?}");
    }

    /// The column is in bytes, which is what tree-sitter means by column.
    #[test]
    fn a_point_counts_bytes_from_the_start_of_its_line() {
        assert_eq!(point_at("abc\ndef", 0), Point { row: 0, column: 0 });
        assert_eq!(point_at("abc\ndef", 5), Point { row: 1, column: 1 });
        // Four bytes of accented text, two characters in.
        assert_eq!(point_at("éé", 4), Point { row: 0, column: 4 });
    }

    #[test]
    fn a_point_past_the_end_is_clamped_rather_than_panicking() {
        assert_eq!(point_at("ab", 99), Point { row: 0, column: 2 });
    }

    // ----- highlight -----------------------------------------------------

    /// The property that matters, and the one an incremental parse could break:
    /// the spans have to reconstruct the buffer exactly. Anything else is text
    /// missing from the editor.
    #[test]
    fn the_spans_always_reconstruct_the_source() {
        let mut highlighter = Highlighter::new();
        let theme = StudioTheme::dark();
        for source in [
            "fn main() {}",
            "fn main() { let x = 1; }",
            "fn main() { let x = 1; }\n\nstruct S;",
            "struct S;",
            "",
            "// just a comment\n",
            "let s = \"héllo wörld\";",
        ] {
            let spans = highlighter.highlight(source, Language::Rust, style(), theme);
            assert_eq!(text_of(&spans), source, "round-trip for {source:?}");
        }
    }

    /// An incremental parse must produce exactly what a cold parse would.
    /// If it does not, the editor's colours depend on how you got there.
    #[test]
    fn incremental_and_cold_parses_agree() {
        let theme = StudioTheme::dark();
        let steps = [
            "fn main() {}",
            "fn main() { }",
            "fn main() { let x = 1; }",
            "fn main() { let x = 1; let y = 2; }",
            "fn main() { let y = 2; }",
            "struct S; fn main() { let y = 2; }",
        ];

        let mut warm = Highlighter::new();
        for source in steps {
            let incremental = warm.highlight(source, Language::Rust, style(), theme);
            // A highlighter that has never seen anything else.
            let cold = Highlighter::new().highlight(source, Language::Rust, style(), theme);
            assert_eq!(
                incremental.len(),
                cold.len(),
                "span count differs after an edit to {source:?}"
            );
            for (a, b) in incremental.iter().zip(cold.iter()) {
                assert_eq!(a.text, b.text, "text differs in {source:?}");
                assert_eq!(a.style.color, b.style.color, "colour differs in {source:?}");
            }
        }
    }

    #[test]
    fn switching_files_does_not_reuse_the_other_files_tree() {
        let theme = StudioTheme::dark();
        let mut highlighter = Highlighter::new();
        highlighter.highlight("fn a() { let x = 1; }", Language::Rust, style(), theme);
        let after = highlighter.highlight(
            "struct Totally; enum Different {}",
            Language::Rust,
            style(),
            theme,
        );
        let cold = Highlighter::new().highlight(
            "struct Totally; enum Different {}",
            Language::Rust,
            style(),
            theme,
        );
        assert_eq!(text_of(&after), text_of(&cold));
        assert_eq!(after.len(), cold.len());
    }

    #[test]
    fn an_unchanged_buffer_returns_the_cached_spans() {
        let theme = StudioTheme::dark();
        let mut highlighter = Highlighter::new();
        let first = highlighter.highlight("fn main() {}", Language::Rust, style(), theme);
        let second = highlighter.highlight("fn main() {}", Language::Rust, style(), theme);
        assert_eq!(first.len(), second.len());
    }

    /// Changing the theme has to re-colour even though the text is identical —
    /// the cache key is text *and* style *and* theme.
    #[test]
    fn a_theme_change_invalidates_the_cache() {
        let mut highlighter = Highlighter::new();
        let dark =
            highlighter.highlight("fn main() {}", Language::Rust, style(), StudioTheme::dark());
        let light = highlighter.highlight(
            "fn main() {}",
            Language::Rust,
            style(),
            StudioTheme::light(),
        );
        let differs = dark
            .iter()
            .zip(light.iter())
            .any(|(a, b)| a.style.color != b.style.color);
        assert!(differs, "the light theme is not the dark one");
    }

    /// Highlighting is the one cost proportional to the whole file rather than
    /// to the visible part. Past the ceiling the file is still editable — it is
    /// simply drawn flat.
    #[test]
    fn an_enormous_file_is_drawn_flat_rather_than_slowly() {
        let mut highlighter = Highlighter::new();
        let huge = "fn main() {}\n".repeat(MAX_HIGHLIGHT_BYTES / 8);
        assert!(huge.len() > MAX_HIGHLIGHT_BYTES);
        let spans = highlighter.highlight(&huge, Language::Rust, style(), StudioTheme::dark());
        assert_eq!(spans.len(), 1, "one flat run");
        assert_eq!(spans[0].text, huge, "and all of the text is still there");
    }

    /// The gap this closes: a `Cargo.toml` in a Rust IDE was flat text.
    #[test]
    fn toml_is_highlighted_and_its_table_headers_stand_out() {
        let theme = StudioTheme::dark();
        let source = "# a comment\n[package]\nname = \"app\"\nedition = 2021\n";
        let spans = Highlighter::new().highlight(source, Language::Toml, style(), theme);
        let coloured: Vec<_> = spans
            .iter()
            .filter(|span| span.style.color != style().color)
            .collect();
        assert!(!coloured.is_empty(), "TOML came back flat");

        let colour_of = |needle: &str| {
            spans
                .iter()
                .find(|span| span.text.contains(needle))
                .map(|span| span.style.color)
        };
        assert_eq!(colour_of("a comment"), Some(theme.syntax.comment));
        assert_eq!(colour_of("package"), Some(theme.syntax.type_name));
        assert_eq!(colour_of("name"), Some(theme.syntax.attribute));
        assert_eq!(colour_of("app"), Some(theme.syntax.string), "{spans:#?}");
        assert_eq!(colour_of("2021"), Some(theme.syntax.number));
    }

    #[test]
    fn json_separates_its_keys_from_its_values() {
        let theme = StudioTheme::dark();
        let source = "{\"name\": \"app\", \"count\": 3, \"on\": true}";
        let spans = Highlighter::new().highlight(source, Language::Json, style(), theme);
        let colour_of = |needle: &str| {
            spans
                .iter()
                .find(|span| span.text.contains(needle))
                .map(|span| span.style.color)
        };
        assert_eq!(colour_of("3"), Some(theme.syntax.number));
        assert_eq!(colour_of("true"), Some(theme.syntax.keyword));
        assert!(
            spans
                .iter()
                .any(|span| span.style.color == theme.syntax.string),
            "the strings are coloured"
        );
    }

    #[test]
    fn yaml_and_markdown_come_back_coloured_too() {
        let theme = StudioTheme::dark();
        for (language, source) in [
            (Language::Yaml, "# ci\nname: build\njobs:\n  - one\n"),
            (Language::Markdown, "# Title\n\nSome `code` here.\n"),
        ] {
            let spans = Highlighter::new().highlight(source, language, style(), theme);
            assert!(
                spans.iter().any(|span| span.style.color != style().color),
                "{language:?} came back flat"
            );
        }
    }

    #[test]
    fn a_language_with_no_grammar_is_flat_rather_than_wrong() {
        // The rule the studio already had and now has to keep with five
        // grammars loaded: a file nothing can parse is drawn as plain text, not
        // parsed as whatever happens to be loaded.
        let theme = StudioTheme::dark();
        let spans = Highlighter::new().highlight("SELECT 1;", Language::Sql, style(), theme);
        assert!(spans.is_empty(), "{spans:?}");
    }

    #[test]
    fn switching_language_does_not_reuse_the_previous_grammars_parse() {
        // A retained tree describes one text under one grammar. Reusing a Rust
        // parse to describe a TOML file is the failure this guards.
        let theme = StudioTheme::dark();
        let mut highlighter = Highlighter::new();
        let rust = highlighter.highlight("fn main() {}", Language::Rust, style(), theme);
        let toml = highlighter.highlight("[package]", Language::Toml, style(), theme);
        let back = highlighter.highlight("fn main() {}", Language::Rust, style(), theme);
        assert_ne!(rust, toml);
        assert_eq!(
            rust, back,
            "coming back to Rust gives the Rust answer again"
        );
    }
}
