//! N6's last mark: code folding, as a **view over the buffer**.
//!
//! # The problem, stated exactly
//!
//! A `TextField` is handed a `TextEditingValue` and reports a new one back. It
//! has no notion of a hidden line, and giving it one would mean a second
//! coordinate system inside the render object — every offset, every hit test,
//! every decoration range doubled. So folding happens *above* the field: the
//! studio hands it text with the folded regions removed, and turns what comes
//! back into an edit of the real buffer.
//!
//! That crossing is the whole of this module, and it is the part that can lose
//! somebody's work if it is wrong. It is therefore done at **line
//! granularity**, never at byte granularity:
//!
//! * [`project`] removes whole lines and records which source line each
//!   surviving line came from.
//! * [`unproject`] compares the edited view against the projection it was made
//!   from **as lists of lines**, finds the run that differs, and splices that
//!   run back into the source at the lines it corresponds to.
//!
//! A line-level splice cannot silently drop a folded region: the lines the
//! splice does not touch are copied through byte for byte, and any fold whose
//! header was inside the replaced run is reported as no longer valid so the
//! caller can drop it. There is no offset arithmetic across a hidden region to
//! get wrong, because there is none at all.
//!
//! # What is foldable
//!
//! Balanced `{}` runs spanning more than one line, plus runs of `//` comment
//! lines. Braces rather than a parse because the studio's parser is
//! `tree-sitter` and this has to work on a buffer that does not compile —
//! which is most of the time an editor is open. Strings, character literals
//! and comments are skipped so a `{` inside `"{"` does not open a region.
//!
//! Nothing here is specific to Rust beyond the comment marker, and nothing at
//! all is specific to a platform.

use std::collections::BTreeSet;

/// A run of lines that can be folded away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Region {
    /// The line the marker goes on. Stays visible when the region is folded —
    /// a fold that hid its own header would hide the name of what was folded.
    pub header: usize,
    /// The last line of the region, inclusive. Hidden when folded, along with
    /// everything between it and the header.
    pub last: usize,
}

impl Region {
    /// How many lines disappear when this is folded.
    #[must_use]
    pub const fn hidden(&self) -> usize {
        self.last - self.header
    }
}

/// Which regions are currently folded, named by their header line.
///
/// A `BTreeSet` so the order is the file's order, which is what both the
/// projection and every "fold all" style operation want.
pub type Folds = BTreeSet<usize>;

/// What the editor is actually showing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct View {
    /// The text handed to the field.
    pub text: String,
    /// The source line each visible line came from, in order.
    ///
    /// This is what keeps the gutter honest: row *n* of the pane is labelled
    /// `lines[n] + 1`, so a folded file's gutter reads 1, 2, 9, 10 — the
    /// numbers that actually exist — rather than being renumbered.
    pub lines: Vec<usize>,
}

impl View {
    /// Whether anything is hidden.
    #[must_use]
    pub fn is_complete(&self, source: &str) -> bool {
        self.lines.len() == source.split('\n').count()
    }
}

/// Every foldable region in `text`, outermost first, then by position.
///
/// # Why the closing line is included and the header is not
///
/// Folding `fn f() {` should leave that line on screen with a marker and hide
/// everything down to and including its `}`. Hiding the header would hide the
/// signature, which is the one line worth keeping; leaving the `}` visible
/// would leave a dangling brace under a collapsed body.
#[must_use]
pub fn regions(text: &str) -> Vec<Region> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut regions = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut scan = ScanState::default();

    for (index, line) in lines.iter().enumerate() {
        for (depth_open, depth_close) in [scan_braces(line, &mut scan)] {
            for _ in 0..depth_close {
                if let Some(start) = open.pop() {
                    if index > start {
                        regions.push(Region {
                            header: start,
                            last: index,
                        });
                    }
                }
            }
            for _ in 0..depth_open {
                open.push(index);
            }
        }
    }

    regions.extend(comment_regions(&lines));
    // Outermost first: a caller folding everything wants the big regions
    // recorded before the ones inside them, and `project` skips a header that
    // is already hidden.
    regions.sort_by_key(|region| (region.header, std::cmp::Reverse(region.last)));
    regions
}

/// What a scan carries from one line to the next.
///
/// Block comments and raw strings both span lines, so neither can be tracked
/// inside a single line's loop. Ordinary strings and character literals cannot
/// span a line in Rust and are therefore reset per line, which is also what
/// keeps one unbalanced quote from mis-reading the rest of the file.
#[derive(Debug, Default, Clone, Copy)]
struct ScanState {
    in_block_comment: bool,
    /// `Some(n)` inside `r` + n × `#` + `"`, waiting for `"` + n × `#`.
    ///
    /// **The gap this closes.** The scanner skipped `"…"` and `'…'` so that a
    /// `{` inside a string did not open a fold, and did not know about
    /// `r#"…"#` at all. A raw string is exactly where a Rust file keeps braces
    /// that are not code — a snippet in a doc test, a template, a generated
    /// shader — so the one construct most likely to contain a stray `{` was the
    /// one construct the scanner read as code. The result was a fold arrow on a
    /// line with no block under it, and folding it hid lines belonging to no
    /// region at all.
    raw_hashes: Option<usize>,
}

/// How many braces a line opens and closes, ignoring quoted and commented text.
///
/// Returns `(opened, closed)` — closes first when applied, because a line like
/// `} else {` closes one region and opens another.
fn scan_braces(line: &str, state: &mut ScanState) -> (usize, usize) {
    let mut opened = 0;
    let mut closed = 0;
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;

    while at < chars.len() {
        let ch = chars[at];

        // A raw string ends at `"` followed by exactly the hashes it opened
        // with, and nothing inside it — not a backslash, not a `"` with too few
        // hashes after it — means anything.
        if let Some(hashes) = state.raw_hashes {
            if ch == '"'
                && chars[at + 1..]
                    .iter()
                    .take(hashes)
                    .filter(|c| **c == '#')
                    .count()
                    == hashes
            {
                state.raw_hashes = None;
                at += 1 + hashes;
                continue;
            }
            at += 1;
            continue;
        }

        if state.in_block_comment {
            if ch == '*' && chars.get(at + 1) == Some(&'/') {
                state.in_block_comment = false;
                at += 2;
                continue;
            }
            at += 1;
            continue;
        }

        if escaped {
            escaped = false;
            at += 1;
            continue;
        }

        // `r"`, `r#"`, `r##"` … and the byte-string forms `br"`, `br#"`.
        // Recognised before the quote rules below, because the `"` that starts
        // a raw string must not be read as the start of an ordinary one.
        if !in_string && !in_char && matches!(ch, 'r' | 'b') {
            let starts_here = ch == 'r' || chars.get(at + 1) == Some(&'r');
            let after_r = at + usize::from(ch == 'b') + 1;
            let is_word_char = at > 0 && (chars[at - 1].is_alphanumeric() || chars[at - 1] == '_');
            if starts_here && !is_word_char {
                let hashes = chars[after_r..].iter().take_while(|c| **c == '#').count();
                if chars.get(after_r + hashes) == Some(&'"') {
                    state.raw_hashes = Some(hashes);
                    at = after_r + hashes + 1;
                    continue;
                }
            }
        }

        match ch {
            '\\' if in_string || in_char => escaped = true,
            '"' if !in_char => in_string = !in_string,
            '\'' if !in_string => in_char = !in_char,
            _ if in_string || in_char => {}
            '/' if chars.get(at + 1) == Some(&'/') => break,
            '/' if chars.get(at + 1) == Some(&'*') => {
                state.in_block_comment = true;
                at += 2;
                continue;
            }
            '{' => opened += 1,
            '}' => {
                if opened > 0 {
                    // `{ }` on one line is not a region, and must not close an
                    // enclosing one.
                    opened -= 1;
                } else {
                    closed += 1;
                }
            }
            _ => {}
        }
        at += 1;
    }
    (opened, closed)
}

/// Runs of two or more consecutive `//` lines.
///
/// Doc comments above an item are routinely longer than the item, and being
/// able to collapse them is most of what folding is for in a well-documented
/// file — which this workspace's own source is.
fn comment_regions(lines: &[&str]) -> Vec<Region> {
    let mut regions = Vec::new();
    let mut start: Option<usize> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("//") {
            start.get_or_insert(index);
        } else if let Some(first) = start.take() {
            if index - 1 > first {
                regions.push(Region {
                    header: first,
                    last: index - 1,
                });
            }
        }
    }
    if let Some(first) = start {
        if lines.len() - 1 > first {
            regions.push(Region {
                header: first,
                last: lines.len() - 1,
            });
        }
    }
    regions
}

/// The text to show, given what is folded.
///
/// A fold whose header is itself hidden inside another fold is skipped rather
/// than applied twice — nesting is normal, and folding an outer region should
/// not depend on what is folded inside it.
#[must_use]
pub fn project(text: &str, folds: &Folds) -> View {
    let source: Vec<&str> = text.split('\n').collect();
    let all = regions(text);

    // header -> last, for the folds that name a real region.
    let mut hide_from: Vec<(usize, usize)> = folds
        .iter()
        .filter_map(|&header| {
            all.iter()
                .filter(|region| region.header == header)
                .map(|region| (header, region.last))
                // The largest region with that header: `fn f() {` may open one
                // region, and folding it should take the whole body.
                .max_by_key(|&(_, last)| last)
        })
        .collect();
    hide_from.sort_unstable();

    let mut lines = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        lines.push(index);
        if let Some(&(_, last)) = hide_from.iter().find(|&&(header, _)| header == index) {
            index = last + 1;
        } else {
            index += 1;
        }
    }

    View {
        text: lines
            .iter()
            .map(|&line| source[line])
            .collect::<Vec<_>>()
            .join("\n"),
        lines,
    }
}

/// What an edit to a projection did to the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unprojected {
    /// The buffer's new full text.
    pub text: String,
    /// The source lines the edit replaced, as a half-open range. Every fold
    /// whose header falls inside it is no longer meaningful and the caller
    /// drops it.
    pub replaced: std::ops::Range<usize>,
    /// How many lines the replacement is, so a caller can shift the folds
    /// after it rather than dropping them too.
    pub replacement_lines: usize,
}

/// Turn an edit of `view.text` into an edit of `source`.
///
/// # The algorithm, and why it is this one
///
/// Both texts are split into lines. The common prefix and the common suffix
/// are found; whatever is between them is what changed. Those bounds are
/// translated through [`View::lines`] into a span of *source* lines, and the
/// changed run is spliced in.
///
/// Everything outside the splice is copied byte for byte, including every
/// hidden line — which is the property that matters. No offset is computed
/// across a hidden region, so there is no arithmetic to get wrong, and an edit
/// far from any fold cannot disturb one.
///
/// Returns `None` when `edited` and the view are identical, which is not an
/// edit and should not produce one.
#[must_use]
pub fn unproject(source: &str, view: &View, edited: &str) -> Option<Unprojected> {
    if edited == view.text {
        return None;
    }
    let old: Vec<&str> = view.text.split('\n').collect();
    let new: Vec<&str> = edited.split('\n').collect();

    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let source_lines: Vec<&str> = source.split('\n').collect();
    // The first source line the change touches. `view.lines[prefix]` when the
    // prefix stops inside the view; the end of the file when the change is an
    // append.
    let from = view
        .lines
        .get(prefix)
        .copied()
        .unwrap_or(source_lines.len());
    // One past the last source line it touches. The view line at
    // `old.len() - suffix` is the first *unchanged* trailing line, and every
    // source line before it — hidden ones included — belongs to the run.
    let to = view
        .lines
        .get(old.len() - suffix)
        .copied()
        .unwrap_or(source_lines.len());

    let replacement = &new[prefix..new.len() - suffix];
    let mut rebuilt: Vec<&str> = Vec::with_capacity(source_lines.len());
    rebuilt.extend_from_slice(&source_lines[..from.min(source_lines.len())]);
    rebuilt.extend_from_slice(replacement);
    if to < source_lines.len() {
        rebuilt.extend_from_slice(&source_lines[to..]);
    }

    Some(Unprojected {
        text: rebuilt.join("\n"),
        replaced: from..to,
        replacement_lines: replacement.len(),
    })
}

/// The folds that survive a splice, shifted to their new lines.
///
/// A fold whose header was inside the replaced run is gone: the code it named
/// no longer exists in the form it was folded in, and keeping the fold would
/// hide lines the user just typed.
#[must_use]
pub fn surviving(folds: &Folds, edit: &Unprojected) -> Folds {
    let removed = edit.replaced.end - edit.replaced.start;
    folds
        .iter()
        .filter_map(|&header| {
            if header < edit.replaced.start {
                Some(header)
            } else if header < edit.replaced.end {
                None
            } else {
                // Shifted by however much the splice grew or shrank the file.
                Some(header + edit.replacement_lines - removed)
            }
        })
        .collect()
}

/// The offset in `source` that a view offset corresponds to.
///
/// Line and column, translated through the map — the same crossing the text
/// makes, so a caret cannot land in a hidden region.
#[must_use]
pub fn source_offset(source: &str, view: &View, offset: usize) -> usize {
    let offset = offset.min(view.text.len());
    let before = &view.text[..floor_boundary(&view.text, offset)];
    let line = before.matches('\n').count();
    let column = before.rsplit('\n').next().unwrap_or_default().len();

    let Some(&source_line) = view.lines.get(line) else {
        return source.len();
    };
    let start: usize = source
        .split('\n')
        .take(source_line)
        .map(|l| l.len() + 1)
        .sum();
    let width = source.split('\n').nth(source_line).map_or(0, |l| l.len());
    (start + column.min(width)).min(source.len())
}

/// The offset in `view.text` that a source offset corresponds to.
///
/// The inverse of [`source_offset`], with one asymmetry that is the point: a
/// source offset inside a *hidden* region has no position in the view, and
/// answers with the start of the header line that hides it. A caret in code
/// nobody can see would be a caret nobody can find.
#[must_use]
pub fn view_offset(source: &str, view: &View, offset: usize) -> usize {
    let offset = offset.min(source.len());
    let before = &source[..floor_boundary(source, offset)];
    let line = before.matches('\n').count();
    let column = before.rsplit('\n').next().unwrap_or_default().len();

    // The visible line this source line is on, or — when it is hidden — the
    // last visible line above it, which is its fold's header.
    let (row, exact) = match view.lines.binary_search(&line) {
        Ok(row) => (row, true),
        Err(0) => (0, false),
        Err(next) => (next - 1, false),
    };

    let start: usize = view.text.split('\n').take(row).map(|l| l.len() + 1).sum();
    if !exact {
        return start.min(view.text.len());
    }
    let width = view.text.split('\n').nth(row).map_or(0, str::len);
    (start + column.min(width)).min(view.text.len())
}

/// Whether the source line containing `offset` is on screen.
#[must_use]
pub fn is_visible(source: &str, view: &View, offset: usize) -> bool {
    let offset = offset.min(source.len());
    let line = source[..floor_boundary(source, offset)]
        .matches('\n')
        .count();
    view.lines.binary_search(&line).is_ok()
}

/// A source range as a view range, or `None` when either end is hidden.
///
/// # The rule, and why it is this one
///
/// `NEXT-VIEWWSTUDIO.md`: *"a source range becomes a view range whenever both
/// ends are visible and is dropped when either is not."* A range with one end
/// inside a fold has no honest position — [`view_offset`] answers with the
/// header line, which is right for a *caret* (a caret nobody can find is worse
/// than one on the header) and wrong for a *mark*, where it would draw a
/// squiggle under a line that is not the line with the problem.
///
/// Dropping is the conservative half of the same argument the editor already
/// makes for the whole set: drawing a mark under the wrong token is not a step
/// towards drawing it under the right one.
#[must_use]
pub fn project_range(
    source: &str,
    view: &View,
    start: usize,
    end: usize,
) -> Option<(usize, usize)> {
    if !is_visible(source, view, start) || !is_visible(source, view, end.min(source.len())) {
        return None;
    }
    let mapped = (
        view_offset(source, view, start),
        view_offset(source, view, end),
    );
    (mapped.0 <= mapped.1).then_some(mapped)
}

/// Every visible line, as `(source range, view range)`.
///
/// What a caller projecting *runs* — syntax spans — walks, rather than mapping
/// each run's ends one at a time: a run can cross a fold boundary, and the
/// pieces on either side land in different places.
///
/// Both ranges exclude the line break, so a caller re-joining them puts the
/// `\n` back itself and does not have to reason about the last line.
#[must_use]
pub fn line_map(
    source: &str,
    view: &View,
) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let mut source_starts = Vec::with_capacity(view.lines.len());
    let mut at = 0;
    for line in source.split('\n') {
        source_starts.push(at..at + line.len());
        at += line.len() + 1;
    }

    let mut out = Vec::with_capacity(view.lines.len());
    let mut view_at = 0;
    for (row, line) in view.text.split('\n').enumerate() {
        let view_range = view_at..view_at + line.len();
        view_at += line.len() + 1;
        let Some(&source_line) = view.lines.get(row) else {
            continue;
        };
        let Some(source_range) = source_starts.get(source_line) else {
            continue;
        };
        out.push((source_range.clone(), view_range));
    }
    out
}

/// The largest offset at or below `at` that starts a character.
fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folds(lines: &[usize]) -> Folds {
        lines.iter().copied().collect()
    }

    const SAMPLE: &str = "\
fn main() {
    let x = 1;
    if x > 0 {
        println!(\"{x}\");
    }
}
struct S;";

    #[test]
    fn a_brace_run_is_a_region_from_its_header_to_its_close() {
        let found = regions(SAMPLE);
        assert!(
            found.contains(&Region { header: 0, last: 5 }),
            "the function: {found:?}"
        );
        assert!(
            found.contains(&Region { header: 2, last: 4 }),
            "and the `if` inside it: {found:?}"
        );
    }

    #[test]
    fn a_brace_inside_a_string_or_a_comment_opens_nothing() {
        // `println!("{x}")` above already proves the string case is not
        // *counted*; this proves it does not open a region of its own.
        let text = "let a = \"{\";\nlet b = 1;\n// {\nlet c = 2;";
        assert!(
            regions(text).is_empty(),
            "a brace in a string or a comment is text: {:?}",
            regions(text)
        );
    }

    #[test]
    fn a_region_that_opens_and_closes_on_one_line_is_not_foldable() {
        // Nothing to hide, and a marker beside it would be a control that
        // does nothing.
        assert!(regions("fn f() { 1 }\nfn g() { 2 }").is_empty());
    }

    #[test]
    fn a_run_of_comment_lines_is_foldable_and_a_single_one_is_not() {
        let text = "/// one\n/// two\n/// three\nfn f();\n// alone\nfn g();";
        let found = regions(text);
        assert!(found.contains(&Region { header: 0, last: 2 }));
        assert!(
            !found.iter().any(|r| r.header == 4),
            "one comment line has nothing to collapse: {found:?}"
        );
    }

    #[test]
    fn folding_hides_the_body_and_keeps_the_header() {
        let view = project(SAMPLE, &folds(&[0]));
        assert_eq!(view.text, "fn main() {\nstruct S;");
        assert_eq!(view.lines, vec![0, 6], "the gutter still reads 1 and 7");
        assert!(!view.is_complete(SAMPLE));
    }

    #[test]
    fn folding_nothing_is_the_whole_file() {
        let view = project(SAMPLE, &Folds::new());
        assert_eq!(view.text, SAMPLE);
        assert!(view.is_complete(SAMPLE));
    }

    #[test]
    fn a_fold_inside_a_fold_is_not_applied_twice() {
        // Folding the `if` and then the function must give the same thing as
        // folding the function alone: its header is hidden, so it cannot hide
        // anything itself.
        assert_eq!(
            project(SAMPLE, &folds(&[0, 2])),
            project(SAMPLE, &folds(&[0]))
        );
    }

    #[test]
    fn a_fold_naming_a_line_that_is_not_a_region_is_ignored() {
        // The set is a set of line numbers, and lines move. One that no longer
        // opens anything must not eat the rest of the file.
        assert_eq!(project(SAMPLE, &folds(&[1])).text, SAMPLE);
    }

    #[test]
    fn an_edit_far_from_a_fold_leaves_every_hidden_line_alone() {
        // The property the whole module exists for.
        let view = project(SAMPLE, &folds(&[0]));
        let edited = "fn main() {\nstruct Renamed;";
        let result = unproject(SAMPLE, &view, edited).expect("an edit");
        assert_eq!(
            result.text,
            SAMPLE.replace("struct S;", "struct Renamed;"),
            "the folded body came through byte for byte"
        );
        assert_eq!(result.replaced, 6..7);
        assert_eq!(surviving(&folds(&[0]), &result), folds(&[0]));
    }

    #[test]
    fn typing_inside_a_visible_line_edits_that_source_line() {
        let view = project(SAMPLE, &Folds::new());
        let edited = SAMPLE.replace("let x = 1;", "let x = 42;");
        let result = unproject(SAMPLE, &view, &edited).expect("an edit");
        assert_eq!(result.text, edited);
        assert_eq!(result.replaced, 1..2);
    }

    #[test]
    fn adding_a_line_shifts_the_folds_below_it() {
        // A fold is a line number, so anything inserted above one moves it.
        let text = "a\nfn f() {\n    body();\n}";
        let view = project(text, &Folds::new());
        let edited = "a\nNEW\nfn f() {\n    body();\n}";
        let result = unproject(text, &view, edited).expect("an edit");
        assert_eq!(result.text, edited);
        assert_eq!(surviving(&folds(&[1]), &result), folds(&[2]));
    }

    #[test]
    fn deleting_a_folded_region_drops_its_fold() {
        // Selecting the collapsed line and deleting it removes the whole
        // region — which is what every editor does, and the fold that named it
        // must not survive to hide something else.
        let view = project(SAMPLE, &folds(&[0]));
        let edited = "struct S;";
        let result = unproject(SAMPLE, &view, edited).expect("an edit");
        assert_eq!(result.text, "struct S;");
        assert_eq!(
            result.replaced,
            0..6,
            "the hidden lines went with the header"
        );
        assert!(surviving(&folds(&[0]), &result).is_empty());
    }

    #[test]
    fn an_identical_text_is_not_an_edit() {
        let view = project(SAMPLE, &folds(&[0]));
        assert_eq!(unproject(SAMPLE, &view, &view.text), None);
    }

    #[test]
    fn appending_past_the_end_of_the_view_lands_at_the_end_of_the_source() {
        let view = project(SAMPLE, &folds(&[0]));
        let edited = format!("{}\ntrailing", view.text);
        let result = unproject(SAMPLE, &view, &edited).expect("an edit");
        assert_eq!(result.text, format!("{SAMPLE}\ntrailing"));
    }

    #[test]
    fn a_caret_in_the_view_maps_to_the_line_it_is_really_on() {
        let view = project(SAMPLE, &folds(&[0]));
        // Start of the second visible line, which is source line 6.
        let at = view.text.find("struct").expect("in the view");
        let mapped = source_offset(SAMPLE, &view, at);
        assert!(
            SAMPLE[mapped..].starts_with("struct S;"),
            "mapped to {mapped}, which is {:?}",
            &SAMPLE[mapped..]
        );
    }

    #[test]
    fn a_caret_past_the_end_of_a_short_line_is_clamped_to_it() {
        // Column 40 of a five-character line is not a position; the end of
        // that line is.
        let view = project("short\nlonger line here", &Folds::new());
        let mapped = source_offset("short\nlonger line here", &view, 4);
        assert_eq!(mapped, 4);
    }

    #[test]
    fn a_multibyte_caret_does_not_split_a_character() {
        // The offsets are bytes; the boundaries are characters. A caret
        // reported mid-character must floor rather than panic on a slice.
        let text = "héllo\nworld";
        let view = project(text, &Folds::new());
        for offset in 0..=text.len() {
            let mapped = source_offset(text, &view, offset);
            assert!(text.is_char_boundary(mapped), "{offset} -> {mapped}");
        }
    }
    #[test]
    fn a_source_caret_maps_into_the_view_and_back() {
        let view = project(SAMPLE, &folds(&[0]));
        let at = SAMPLE.find("struct").expect("in the source");
        let in_view = view_offset(SAMPLE, &view, at);
        assert!(view.text[in_view..].starts_with("struct S;"));
        assert_eq!(source_offset(SAMPLE, &view, in_view), at, "and back again");
    }

    #[test]
    fn a_caret_inside_a_hidden_region_lands_on_the_header_that_hides_it() {
        // Not clamped to the end of the file and not dropped: the header is
        // the line the user can actually see the code on.
        let view = project(SAMPLE, &folds(&[0]));
        let inside = SAMPLE.find("println").expect("in the source");
        assert_eq!(
            view_offset(SAMPLE, &view, inside),
            0,
            "source line 3 is hidden by the fold whose header is line 0"
        );
    }

    #[test]
    fn view_offset_survives_every_byte_of_a_multibyte_file() {
        let text = "héllo\nwörld\nthird";
        let view = project(text, &Folds::new());
        for offset in 0..=text.len() {
            let mapped = view_offset(text, &view, offset);
            assert!(view.text.is_char_boundary(mapped), "{offset} -> {mapped}");
        }
    }
}

#[cfg(test)]
mod projection_tests {
    use super::*;

    const SOURCE: &str = "fn a() {\n    one\n    two\n}\nfn b() {\n    three\n}\n";

    fn folded_at(header: usize) -> View {
        let mut folds = Folds::new();
        folds.insert(header);
        project(SOURCE, &folds)
    }

    #[test]
    fn a_range_wholly_visible_maps_through() {
        let view = folded_at(0);
        // `fn b` is on source line 4 and still visible.
        let start = SOURCE.find("fn b").expect("the second function");
        let mapped = project_range(SOURCE, &view, start, start + 4).expect("visible");
        assert_eq!(&view.text[mapped.0..mapped.1], "fn b");
    }

    /// The rule the whole feature turns on: a mark with either end inside a
    /// fold is dropped rather than drawn somewhere plausible and wrong.
    #[test]
    fn a_range_inside_a_fold_is_dropped() {
        let view = folded_at(0);
        let hidden = SOURCE.find("one").expect("a hidden line");
        assert_eq!(project_range(SOURCE, &view, hidden, hidden + 3), None);
    }

    #[test]
    fn a_range_straddling_a_fold_is_dropped() {
        let view = folded_at(0);
        let start = SOURCE.find("fn a").expect("the header");
        let end = SOURCE.find("two").expect("a hidden line") + 3;
        assert_eq!(project_range(SOURCE, &view, start, end), None);
    }

    #[test]
    fn the_line_map_pairs_every_visible_line_with_its_source() {
        let view = folded_at(0);
        let map = line_map(SOURCE, &view);
        assert_eq!(
            map.len(),
            view.text.split('\n').count().min(view.lines.len())
        );
        for (source_range, view_range) in &map {
            assert_eq!(
                &SOURCE[source_range.clone()],
                &view.text[view_range.clone()],
                "a mapped line does not hold the same text"
            );
        }
    }

    #[test]
    fn an_unfolded_view_maps_everything_to_itself() {
        let view = project(SOURCE, &Folds::new());
        for (source_range, view_range) in line_map(SOURCE, &view) {
            assert_eq!(source_range, view_range);
        }
        assert_eq!(
            project_range(SOURCE, &view, 3, 7),
            Some((3, 7)),
            "with nothing folded a range is its own projection"
        );
    }

    #[test]
    fn a_brace_inside_a_raw_string_does_not_open_a_region() {
        // The gap: the scanner knew `"…"` and `'…'` and not `r#"…"#`, so the
        // one place a Rust file keeps braces that are not code was read as
        // code — a fold arrow with no block under it.
        let text = "let snippet = r#\"fn f() {\"#;\nlet after = 1;\nlet more = 2;\n";
        assert!(regions(text).is_empty(), "{:?}", regions(text));
    }

    #[test]
    fn a_raw_string_spanning_lines_hides_its_braces_on_every_line() {
        let text = "let s = r#\"\n{\n{\n\"#;\nfn real() {\n    ok();\n}\n";
        let found = regions(text);
        assert_eq!(
            found,
            vec![Region { header: 4, last: 6 }],
            "only the real fn folds: {found:?}"
        );
    }

    #[test]
    fn every_raw_string_form_is_recognised() {
        for opener in ["r\"", "r#\"", "r##\"", "br\"", "br#\""] {
            let closer = opener.trim_start_matches(['b', 'r']);
            let closer: String = format!("\"{}", &closer[..closer.len() - 1]);
            let text = format!("let s = {opener}{{{closer};\nlet t = 1;\n");
            assert!(
                regions(&text).is_empty(),
                "{opener} left a region: {:?}",
                regions(&text)
            );
        }
    }

    #[test]
    fn an_identifier_ending_in_r_does_not_start_a_raw_string() {
        // `for` and `char` end in a letter before a quote often enough that a
        // naive check turns the rest of the file into a string.
        let text = "fn f() {\n    let c = 'a';\n    other();\n}\n";
        assert_eq!(regions(text), vec![Region { header: 0, last: 3 }]);
    }
}
