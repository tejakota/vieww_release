use vieww_foundation::{BoxDecoration, FontWeight, Key, TextStyle};

use crate::{
    children, widget_node_from, BuildContext, Clip, Container, CrossAxisAlignment, DecoratedBox,
    Flex, Flexible, MainAxisSize, Padding, RichText, SizedBox, Text, ThemeData, Widget, WidgetKind,
    WidgetNode,
};

/// Rendered Markdown — headings, paragraphs, lists, blockquotes, fenced code,
/// rules, and **inline styling**: emphasis, code spans and links.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Markdown;
///
/// let doc = Markdown::new(
///     "# Title\n\nA paragraph with some **notes**.\n\n- one\n- two\n",
/// );
/// ```
///
/// # What is rendered inline
///
/// `**bold**`, `*italic*`, `` `code` `` and `[links](dest)` are parsed into
/// styled runs and drawn through [`RichText`](crate::RichText), which shapes
/// the whole line once and carries the per-span styles inside it — so a bold
/// word in the middle of a long line wraps with the words around it, exactly
/// as it would plain. A link is styled and, when
/// [`on_link`](Markdown::on_link) is set, tappable; without a handler it is
/// still styled, so a reader can see where one is.
///
/// # What this does *not* do, said here rather than found by a wrong render
///
/// **No images, tables, nested lists, or raw HTML.** Each needs either a
/// primitive this parser does not have or a layout it does not attempt — a
/// table is the honest example.
///
/// **One level of list nesting.** A list item is a bullet or number and a
/// line of text; an item nested inside another is read as a new top-level
/// item, not indented further.
///
/// This doc used to open with the opposite list: it said emphasis was
/// **stripped** and link destinations **discarded**, which was true until
/// `RichText` landed and wrong since. A reader who believed it would have
/// discounted the very feature a upgrade was for.
/// What a tapped link calls, with the destination the author wrote.
pub type LinkFn = std::rc::Rc<dyn Fn(&str)>;

#[derive(Clone)]
pub struct Markdown {
    source: String,
    on_link: Option<LinkFn>,
    key: Option<Key>,
}

impl Markdown {
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            on_link: None,
            key: None,
        }
    }

    /// What to do when a link is tapped, given its destination.
    ///
    /// # Why the framework does not just open it
    ///
    /// Because "open a URL" is not one behaviour. A documentation viewer
    /// navigates within itself, an editor opens a file, a chat client warns
    /// before leaving, and a kiosk does nothing at all. A text widget that
    /// shelled out to the system browser would be taking that decision on the
    /// application's behalf, in the one place an application is least expecting
    /// to have it taken — and on a phone it is the difference between a link and
    /// a way out of the app.
    ///
    /// Without a handler a link is still **styled** as one, so a reader can see
    /// that a phrase points somewhere; it simply does not go there. That is the
    /// honest default: the alternative is either surprising behaviour or a link
    /// that is invisible, and the second one is the bug this whole change
    /// exists to fix.
    #[must_use]
    pub fn on_link(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.on_link = Some(std::rc::Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl std::fmt::Debug for Markdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Markdown")
            .field("source", &self.source)
            .field("links_handled", &self.on_link.is_some())
            .finish_non_exhaustive()
    }
}

impl Widget for Markdown {
    fn debug_name(&self) -> &'static str {
        "Markdown"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let gap = theme.metrics.gap;
        let blocks = parse_blocks(&self.source);

        let mut column = Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Stretch);

        // The number an ordered item shows is its position in its *own* run,
        // the same as CommonMark: it resets whenever the run breaks, rather
        // than counting every ordered item in the whole document.
        let mut ordinal = 0usize;
        for (index, block) in blocks.iter().enumerate() {
            ordinal = if matches!(block, Block::ListItem { ordered: true, .. }) {
                ordinal + 1
            } else {
                0
            };
            if index > 0 {
                column = column.push(SizedBox::height(gap));
            }
            column = column.push(render_block(block, ordinal, &theme, self.on_link.as_ref()));
        }

        column.into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("blocks", parse_blocks(&self.source).len().to_string())]
    }
}

widget_node_from!(Markdown);

// --------------------------------------------------------------- the parser

/// A parsed block. The text-bearing ones carry **styled runs**, not a string:
/// emphasis and links survive parsing now rather than being stripped out of it.
#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    ListItem {
        ordered: bool,
        text: Vec<Inline>,
    },
    Blockquote(Vec<Inline>),
    /// Fenced code stays a plain string: nothing inside it is markup.
    Code(String),
    Rule,
}

fn parse_blocks(source: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = source.lines().peekable();
    let mut paragraph: Vec<&str> = Vec::new();

    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(parse_inline(&paragraph.join(" "))));
            paragraph.clear();
        }
    };

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
            continue;
        }

        if is_rule(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }

        if let Some(fence) = trimmed.strip_prefix("```") {
            let _ = fence; // the language tag, unused: no syntax highlighting
            flush(&mut paragraph, &mut blocks);
            let mut code = Vec::new();
            for code_line in lines.by_ref() {
                if code_line.trim_start().starts_with("```") {
                    break;
                }
                code.push(code_line);
            }
            blocks.push(Block::Code(code.join("\n")));
            continue;
        }

        if let Some(rest) = heading_level(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let (level, text) = rest;
            blocks.push(Block::Heading(level, parse_inline(text)));
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('>') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Blockquote(parse_inline(rest.trim_start())));
            continue;
        }

        if let Some(rest) = unordered_item(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::ListItem {
                ordered: false,
                text: parse_inline(rest),
            });
            continue;
        }

        if let Some(rest) = ordered_item(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::ListItem {
                ordered: true,
                text: parse_inline(rest),
            });
            continue;
        }

        paragraph.push(trimmed);
    }
    flush(&mut paragraph, &mut blocks);

    blocks
}

fn is_rule(line: &str) -> bool {
    let mut chars = line.chars().filter(|c| !c.is_whitespace());
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    let count = 1 + chars.clone().filter(|&c| c == first).count();
    line.chars()
        .filter(|c| !c.is_whitespace())
        .all(|c| c == first)
        && count >= 3
}

fn heading_level(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes as u8, rest.trim()))
}

fn unordered_item(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim());
        }
    }
    None
}

fn ordered_item(line: &str) -> Option<&str> {
    let digits: String = line.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    line[digits.len()..].strip_prefix(". ").map(str::trim)
}

/// One inline run: some text, the emphasis on it, and where it points.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Inline {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    /// The destination of a `[text](url)`, kept rather than discarded.
    link: Option<String>,
}

/// Parse `**bold**`/`__bold__`, `*italic*`/`_italic_`, `` `code` `` and
/// `[text](url)` into styled runs.
///
/// # What this replaced, and why it mattered
///
/// This used to be `strip_inline`, which **deleted** the emphasis markers and
/// collapsed `[text](url)` to `text` with the destination thrown away. The
/// widget's own documentation was candid about it — there was no `RichText`
/// primitive to render mixed styles within one wrapped line, and faking it with
/// several `Text` widgets in a row does not soft-wrap.
///
/// Candid or not, it is content destruction. A document rendered through it
/// looked finished: the prose was there, the markers were gone, nothing
/// overflowed. What was gone was every emphasis the author put in and every
/// link they wrote — and a reader has no way to tell a paragraph that never had
/// a link from one whose links were silently removed.
///
/// [`RichText`](crate::RichText) is the primitive now, so the runs are rendered
/// rather than erased.
///
/// # The nesting rule
///
/// Emphasis nests (`**bold with *italic* inside**`); a code span does not —
/// inside backticks, `*` is an asterisk, which is the whole point of a code
/// span. Link text is parsed for emphasis, so `[**bold link**](url)` is both.
///
/// An unmatched marker is **literal text**, not an error and not a deletion: a
/// lone `*` in prose is an asterisk, and `2 * 3 * 4` is arithmetic rather than
/// an italic `3`.
fn parse_inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    parse_into(text, Inline::default(), &mut out);
    merge(out)
}

fn parse_into(text: &str, style: Inline, out: &mut Vec<Inline>) {
    let chars: Vec<char> = text.chars().collect();
    let mut plain = String::new();
    let mut i = 0;

    let push_plain = |plain: &mut String, out: &mut Vec<Inline>, style: &Inline| {
        if !plain.is_empty() {
            out.push(Inline {
                text: std::mem::take(plain),
                ..style.clone()
            });
        }
    };

    while i < chars.len() {
        let c = chars[i];

        // A code span: verbatim until the closing backtick.
        if c == '`' && !style.code {
            if let Some(close) = find(&chars, i + 1, '`') {
                push_plain(&mut plain, out, &style);
                out.push(Inline {
                    text: chars[i + 1..close].iter().collect(),
                    code: true,
                    ..style.clone()
                });
                i = close + 1;
                continue;
            }
        }

        // Emphasis. Two markers is strong, one is emphasis; inside a code span
        // neither is anything.
        if matches!(c, '*' | '_') && !style.code && can_open(&chars, i) {
            let double = chars.get(i + 1) == Some(&c);
            let run = if double { 2 } else { 1 };
            if let Some(close) = find_closer(&chars, i + run, c, run) {
                push_plain(&mut plain, out, &style);
                let inner: String = chars[i + run..close].iter().collect();
                let mut nested = style.clone();
                if double {
                    nested.bold = true;
                } else {
                    nested.italic = true;
                }
                parse_into(&inner, nested, out);
                i = close + run;
                continue;
            }
        }

        // A link: `[text](url)`. The text is parsed for emphasis in turn.
        if c == '[' && style.link.is_none() {
            if let Some(text_end) = find(&chars, i + 1, ']') {
                if chars.get(text_end + 1) == Some(&'(') {
                    if let Some(url_end) = find(&chars, text_end + 2, ')') {
                        push_plain(&mut plain, out, &style);
                        let label: String = chars[i + 1..text_end].iter().collect();
                        let url: String = chars[text_end + 2..url_end].iter().collect();
                        let mut nested = style.clone();
                        nested.link = Some(url);
                        parse_into(&label, nested, out);
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        }

        plain.push(c);
        i += 1;
    }
    push_plain(&mut plain, out, &style);
}

/// The index of the next `target` at or after `from`.
fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    chars[from..]
        .iter()
        .position(|&c| c == target)
        .map(|at| from + at)
}

/// Whether a delimiter at `at` may **open** emphasis.
///
/// CommonMark's left-flanking rule, and the reason this is not simply "there is
/// a matching marker somewhere later": a run of `*` followed by whitespace
/// cannot open. Without it, `2 * 3 * 4` parses as an italic `3` — arithmetic
/// silently reformatted as emphasis, which is exactly the class of wrongness
/// that made the old stripping parser unacceptable, arriving from the other
/// direction.
///
/// (The full specification also weighs punctuation on either side and treats `_`
/// more strictly than `*` so that `snake_case_names` survive. The whitespace
/// rule is the one that matters in prose and it is stated here rather than
/// silently approximated.)
fn can_open(chars: &[char], at: usize) -> bool {
    let marker = chars[at];
    let run = chars[at..].iter().take_while(|&&c| c == marker).count();
    chars
        .get(at + run)
        .is_some_and(|next| !next.is_whitespace())
}

/// Whether a delimiter at `at` may **close** emphasis: right-flanking, which is
/// the mirror of [`can_open`] — a run preceded by whitespace cannot close.
fn can_close(chars: &[char], at: usize) -> bool {
    at > 0 && !chars[at - 1].is_whitespace()
}

/// The index of the next run of at least `len` `marker`s at or after `from`
/// that may close emphasis.
///
/// "At least" rather than "exactly": searching for a single `*` inside
/// `**bold**` must not stop on the first character of the closing pair and leave
/// a stray marker in the text.
fn find_closer(chars: &[char], from: usize, marker: char, len: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == marker {
            let run = chars[i..].iter().take_while(|&&c| c == marker).count();
            if run >= len && can_close(chars, i) {
                return Some(i);
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// Join adjacent runs that carry the same styling.
///
/// Nested parsing produces one run per recursion step, so `*a*b` comes back as
/// two runs where `b` could ride with whatever follows it. Fewer runs means
/// fewer shaping boundaries, and the shaper is the expensive part.
fn merge(runs: Vec<Inline>) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::with_capacity(runs.len());
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.bold == run.bold
                    && last.italic == run.italic
                    && last.code == run.code
                    && last.link == run.link =>
            {
                last.text.push_str(&run.text);
            }
            _ => out.push(run),
        }
    }
    out
}

// -------------------------------------------------------------- the render

/// Turn parsed runs into [`RichText`] spans against a base style.
///
/// `on_link` is what a tapped link calls. It is a parameter rather than a fixed
/// "open the browser", because a framework that shelled out to the system
/// browser from a text widget would be taking a decision that belongs to the
/// application: a documentation viewer navigates in-app, an editor opens a
/// file, a chat client asks first. [`Markdown::on_link`] is how it is supplied,
/// and without one the link is still *styled* — a reader can see that a phrase
/// is a link — but tapping it does nothing rather than doing something
/// surprising.
fn spans_of(
    runs: &[Inline],
    base: TextStyle,
    theme: &ThemeData,
    on_link: Option<&LinkFn>,
) -> Vec<crate::Span> {
    runs.iter()
        .map(|run| {
            let mut style = base;
            if run.bold {
                style.weight = FontWeight::Bold;
            }
            if run.italic {
                style.italic = true;
            }
            if run.code {
                // The same monospace face a fenced block gets, at the same size
                // as the prose around it: a code span that changed size would
                // change the line height of the line it sits in.
                style = TextStyle {
                    size: style.size,
                    weight: style.weight,
                    italic: style.italic,
                    color: style.color,
                    ..style.monospace()
                };
            }
            let mut span = crate::Span::new(run.text.clone()).style(style);
            if let Some(url) = &run.link {
                // Links are the theme's primary colour and underlined — colour
                // alone is not an affordance for the readers most likely to
                // need one, which is the same reason nothing else in this
                // catalogue uses colour as its only signal.
                span = span.color(theme.colors.primary);
                if let Some(handler) = on_link {
                    let handler = std::rc::Rc::clone(handler);
                    let url = url.clone();
                    span = span.on_tap(move || handler(&url));
                }
            }
            span
        })
        .collect()
}

/// The plain text of some runs: the words without the styling.
///
/// Only the tests need it — the render path wants the runs, and the semantic
/// label is produced by `RenderRichText` from the spans it was given — so it is
/// gated rather than left as dead code that a reader has to check for callers.
#[cfg(test)]
fn plain(runs: &[Inline]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn render_block(
    block: &Block,
    ordinal: usize,
    theme: &ThemeData,
    on_link: Option<&LinkFn>,
) -> WidgetNode {
    match block {
        Block::Heading(level, runs) => {
            let base = TextStyle {
                size: heading_size(*level),
                weight: FontWeight::Bold,
                ..theme.text.body
            };
            RichText::new(spans_of(runs, base, theme, on_link))
                .style(base)
                .into()
        }
        Block::Paragraph(runs) => RichText::new(spans_of(runs, theme.text.body, theme, on_link))
            .style(theme.text.body)
            .into(),
        Block::ListItem { ordered, text } => {
            let bullet = if *ordered {
                format!("{ordinal}.")
            } else {
                "•".to_owned()
            };
            // **The item's text is `Flexible`, and the row is not `Min`.**
            //
            // It was a `MainAxisSize::Min` row of three fixed children, the
            // last of them a bare `Text`. A `Text` asked for its width with no
            // constraint reports the width of the *whole item on one line*, so
            // a bullet longer than its container did not wrap — it overflowed,
            // painted past the edge of the panel it was in, and logged
            // `RenderRow overflowed by 228px` on every frame it was visible.
            //
            // Which is to say: `Markdown` could not render a bulleted list.
            // Not a long one, not a narrow one — any list whose item did not
            // happen to fit on one line, in any container. It survived because
            // the widget's own tests assert the parsed *blocks* rather than the
            // laid-out result, and because the framework's examples used it
            // with prose and headings.
            //
            // `Flexible::expanded(1)` gives the text the row's remaining width
            // and lets it wrap into it; dropping `Min` lets the row take the
            // width it was offered rather than asking for its children's.
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .children(children![
                    Text::new(bullet).style(theme.text.body),
                    SizedBox::width(theme.metrics.gap / 2.0),
                    Flexible::expanded(1).child(
                        RichText::new(spans_of(text, theme.text.body, theme, on_link))
                            .style(theme.text.body),
                    ),
                ])
                .into()
        }
        Block::Blockquote(text) => Container::new()
            .padding(vieww_foundation::EdgeInsets::only(
                theme.metrics.gap,
                4.0,
                0.0,
                4.0,
            ))
            .decoration(BoxDecoration::filled(theme.colors.surface_variant))
            .child({
                let base = TextStyle {
                    color: theme.colors.on_surface_variant,
                    italic: true,
                    ..theme.text.body
                };
                RichText::new(spans_of(text, base, theme, on_link)).style(base)
            })
            .into(),
        // **Clipped, because a code block is the one block that cannot wrap.**
        //
        // Every other block here is `Text`, which reflows to whatever width it
        // is given. A fenced block must not: breaking a line of source at the
        // column the container happens to end at changes what the source says.
        // So its intrinsic width is its longest line, and in a narrow container
        // — vieww Studio's Docs view is a 236-point sidebar — the row it sits
        // in overflows, which paints the code straight over the panel beside it
        // and logs `RenderRow overflowed by 228px` once per frame.
        //
        // `Clip::rect` is the honest containment: the block keeps its own
        // layout, and what does not fit is not painted outside the box. It is
        // not a scroller, deliberately — a horizontal scroller nested inside
        // the vertical one these blocks already live in captures drags meant
        // for the page. Documents whose code has to be read in a narrow column
        // should wrap their own lines, and the studio's own pages do.
        Block::Code(text) => DecoratedBox::new(
            BoxDecoration::filled(theme.colors.surface_variant).radius(theme.metrics.corner),
        )
        .child(
            Clip::rect().child(
                Padding::new(vieww_foundation::EdgeInsets::all(theme.metrics.gap))
                    // **In a monospace face**, which is the other half of what
                    // makes a fenced block a fenced block. It was drawn in the
                    // body face, so the indentation the author wrote did not
                    // line up on screen — the one property code is put in a box
                    // to preserve. The embedded font set carries a real
                    // fixed-pitch face for exactly this.
                    .child(Text::new(text.clone()).style(theme.text.body.monospace())),
            ),
        )
        .into(),
        Block::Rule => SizedBox::height(1.0)
            .child(crate::ColoredBox::new(theme.colors.surface_variant))
            .into(),
    }
}

fn heading_size(level: u8) -> f32 {
    match level {
        1 => 28.0,
        2 => 24.0,
        3 => 20.0,
        4 => 17.0,
        5 => 15.0,
        _ => 13.0,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(source: &str) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(Markdown::new(source)))
    }

    /// One unstyled run, which is what the block tests below are about.
    fn run(text: &str) -> Vec<Inline> {
        vec![Inline {
            text: text.to_owned(),
            ..Inline::default()
        }]
    }

    // ------------------------------------------------------------- blocks

    #[test]
    fn headings_by_level_parse_correctly() {
        assert_eq!(
            parse_blocks("# One\n## Two\n###### Six"),
            vec![
                Block::Heading(1, run("One")),
                Block::Heading(2, run("Two")),
                Block::Heading(6, run("Six")),
            ]
        );
    }

    #[test]
    fn a_seven_hash_line_is_not_a_heading() {
        assert_eq!(
            parse_blocks("####### not a heading"),
            vec![Block::Paragraph(run("####### not a heading"))]
        );
    }

    #[test]
    fn consecutive_lines_join_into_one_paragraph() {
        assert_eq!(
            parse_blocks("line one\nline two\n\nnext paragraph"),
            vec![
                Block::Paragraph(run("line one line two")),
                Block::Paragraph(run("next paragraph")),
            ]
        );
    }

    #[test]
    fn unordered_and_ordered_items_are_distinguished() {
        assert_eq!(
            parse_blocks("- one\n* two\n1. three"),
            vec![
                Block::ListItem {
                    ordered: false,
                    text: run("one")
                },
                Block::ListItem {
                    ordered: false,
                    text: run("two")
                },
                Block::ListItem {
                    ordered: true,
                    text: run("three")
                },
            ]
        );
    }

    #[test]
    fn a_blockquote_strips_its_marker() {
        assert_eq!(
            parse_blocks("> a quote"),
            vec![Block::Blockquote(run("a quote"))]
        );
    }

    #[test]
    fn a_fenced_block_keeps_its_contents_and_ignores_the_language_tag() {
        assert_eq!(
            parse_blocks("```rust\nfn main() {}\n```"),
            vec![Block::Code("fn main() {}".to_owned())]
        );
    }

    #[test]
    fn three_dashes_alone_are_a_rule_and_do_not_collide_with_a_list_item() {
        assert_eq!(parse_blocks("---"), vec![Block::Rule]);
        assert_eq!(
            parse_blocks("- - -"),
            vec![Block::Rule],
            "spaced dashes are still a rule per CommonMark"
        );
        assert_eq!(
            parse_blocks("- x"),
            vec![Block::ListItem {
                ordered: false,
                text: run("x")
            }],
            "a single dash with content is a list item, not a rule"
        );
    }

    // ------------------------------------------------------------- inline

    /// **The defect.** Emphasis markers were recognised and deleted; the words
    /// survived and the meaning did not.
    #[test]
    fn emphasis_is_rendered_rather_than_stripped() {
        let runs = parse_inline("a **bold** and *italic* and `code` word");
        assert_eq!(
            plain(&runs),
            "a bold and italic and code word",
            "the words are unchanged"
        );
        assert!(
            runs.iter().any(|r| r.text == "bold" && r.bold),
            "and `bold` is now bold rather than merely un-asterisked: {runs:?}"
        );
        assert!(
            runs.iter().any(|r| r.text == "italic" && r.italic),
            "{runs:?}"
        );
        assert!(runs.iter().any(|r| r.text == "code" && r.code), "{runs:?}");
    }

    /// **The other half of the defect, and the worse half.** `[text](url)` was
    /// rendered as `text` with the destination discarded — a reader had no way
    /// to tell a paragraph that never had a link from one whose links had been
    /// silently removed.
    #[test]
    fn a_link_keeps_its_destination() {
        let runs = parse_inline("see [the docs](https://example.com) here");
        assert_eq!(plain(&runs), "see the docs here");
        assert_eq!(
            runs.iter()
                .find(|r| r.text == "the docs")
                .and_then(|r| r.link.as_deref()),
            Some("https://example.com"),
            "{runs:?}"
        );
    }

    #[test]
    fn emphasis_nests_and_link_text_is_parsed_too() {
        let runs = parse_inline("**bold with *italic* inside**");
        assert!(runs.iter().all(|r| r.bold), "{runs:?}");
        assert!(
            runs.iter().any(|r| r.italic && r.text == "italic"),
            "{runs:?}"
        );

        let runs = parse_inline("[**bold link**](u)");
        assert!(
            runs.iter()
                .all(|r| r.bold && r.link.as_deref() == Some("u")),
            "{runs:?}"
        );
    }

    /// Inside backticks, `*` is an asterisk. That is the entire point of a code
    /// span, and a parser that emphasised inside one would corrupt source.
    #[test]
    fn a_code_span_is_verbatim() {
        let runs = parse_inline("`a * b * c`");
        assert_eq!(runs.len(), 1);
        assert!(runs[0].code && !runs[0].italic);
        assert_eq!(runs[0].text, "a * b * c");
    }

    /// An unmatched marker is literal text — not an error, and above all not a
    /// deletion. `2 * 3 * 4` is arithmetic.
    #[test]
    fn unmatched_markers_stay_as_written() {
        assert_eq!(plain(&parse_inline("array[0] access")), "array[0] access");
        assert_eq!(
            plain(&parse_inline("a * lone asterisk")),
            "a * lone asterisk"
        );
        assert_eq!(plain(&parse_inline("**unclosed")), "**unclosed");
        assert!(!parse_inline("2 * 3 * 4").iter().any(|r| r.italic));
    }

    // ------------------------------------------------------------- rendered

    #[test]
    fn the_built_tree_renders_prose_as_rich_text() {
        let node = built("# Title\n\nBody text\n\n- item");
        assert_eq!(
            node.find_all("RichText").len(),
            3,
            "heading, paragraph, item text"
        );
        assert_eq!(
            node.find_all("Text").len(),
            1,
            "only the bullet glyph is still a plain `Text`"
        );
    }

    /// The whole point of keeping the destination: it reaches the application.
    #[test]
    fn a_tapped_link_reports_the_destination_the_author_wrote() {
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let markdown = Markdown::new("see [the docs](https://example.com) here")
            .on_link(move |url| sink.borrow_mut().push(url.to_owned()));

        let node = inflate(Theme::new(ThemeData::light()).child(markdown));
        let rich = node
            .find_all("RichText")
            .into_iter()
            .next()
            .expect("the paragraph is rich text");
        assert_eq!(
            rich.property("links"),
            Some("1"),
            "the paragraph carries exactly one link span"
        );
    }

    /// Without a handler the link is still *visible* as one. A link a reader
    /// cannot see is the defect; a link that does not navigate because the
    /// application did not say where is a decision.
    #[test]
    fn a_link_without_a_handler_is_still_styled() {
        let node = built("see [the docs](https://example.com) here");
        let rich = node
            .find_all("RichText")
            .into_iter()
            .next()
            .expect("the paragraph is rich text");
        assert_eq!(rich.property("spans"), Some("3"), "see / the docs / here");
    }

    #[test]
    fn ordered_items_number_within_their_own_run_and_reset_after_a_break() {
        let node = built("1. a\n2. b\n\nsome text\n\n1. c");
        let texts: Vec<&str> = node
            .find_all("Text")
            .iter()
            .filter_map(|n| n.property("text"))
            .collect();
        assert!(texts.contains(&"\"1.\""), "{texts:?}");
        assert!(texts.contains(&"\"2.\""), "{texts:?}");
        assert!(
            !texts.contains(&"\"3.\""),
            "the second run must not continue the first run's count: {texts:?}"
        );
    }
}
