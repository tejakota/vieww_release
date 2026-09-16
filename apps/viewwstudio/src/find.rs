//! Find and replace over the active buffer.
//!
//! Plain literal search, deliberately: a regex engine is a dependency and a
//! whole second language for the user to get wrong, and what somebody editing
//! one screen file actually does is look for an identifier. Case sensitivity
//! and whole-word are the two switches that carry their weight; anything past
//! that is the point at which the answer is "open your editor".
//!
//! # Byte ranges, not line numbers
//!
//! A match is a byte range into the buffer, because that is what a selection
//! is: selecting a match means handing [`TextSelection`] the two offsets, and
//! anything that went through a line/column round-trip on the way would have to
//! come back. The line number the find bar shows is derived from the range when
//! it is needed, not carried alongside it where it could drift.

use vieww_foundation::{TextEditingValue, TextSelection};

/// What the find bar is looking for, and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub needle: String,
    pub case_sensitive: bool,
    /// Only match where the needle is not part of a longer identifier.
    pub whole_word: bool,
}

impl Query {
    #[must_use]
    pub fn new(needle: impl Into<String>) -> Self {
        Self {
            needle: needle.into(),
            case_sensitive: false,
            whole_word: false,
        }
    }

    /// Every match in `text`, as byte ranges, in order.
    ///
    /// Overlaps are not reported: the scan resumes after the match it just
    /// found, so searching `aa` in `aaaa` gives two matches rather than three.
    /// That is what "next match" has to mean for the button to walk the file.
    #[must_use]
    pub fn matches(&self, text: &str) -> Vec<(usize, usize)> {
        if self.needle.is_empty() {
            return Vec::new();
        }

        // Lowercased once for the whole scan rather than per candidate. The
        // offsets stay valid because `to_lowercase` on ASCII — which is what
        // Rust source is made of — is length-preserving, and the guard below
        // is what makes that claim rather than assuming it.
        let (haystack, needle) = if self.case_sensitive {
            (text.to_string(), self.needle.clone())
        } else {
            let lowered = text.to_lowercase();
            if lowered.len() == text.len() {
                (lowered, self.needle.to_lowercase())
            } else {
                // A locale where lowercasing changes byte length (ẞ → ss).
                // Fall back to an exact scan rather than report offsets into a
                // string that is no longer the buffer.
                (text.to_string(), self.needle.clone())
            }
        };

        let mut out = Vec::new();
        let mut from = 0usize;
        while let Some(found) = haystack[from..].find(&needle) {
            let start = from + found;
            let end = start + needle.len();
            if !self.whole_word || is_whole_word(&haystack, start, end) {
                out.push((start, end));
            }
            // Advance past this match. `max(start + 1)` guards a needle that
            // somehow measured zero after the transform above — an infinite
            // loop in a search box is the worst possible outcome here.
            from = end.max(start + 1);
            if from >= haystack.len() {
                break;
            }
        }
        out
    }

    /// The index of the first match at or after `offset`, wrapping.
    ///
    /// Wrapping rather than stopping at the end, because a find that goes quiet
    /// at the bottom of the file reads as "no more matches" when there are
    /// several above the caret.
    #[must_use]
    pub fn index_at_or_after(matches: &[(usize, usize)], offset: usize) -> Option<usize> {
        if matches.is_empty() {
            return None;
        }
        Some(
            matches
                .iter()
                .position(|(start, _)| *start >= offset)
                .unwrap_or(0),
        )
    }
}

/// Whether the range `start..end` stands alone rather than inside a longer
/// identifier. `_` counts as a word character, because `foo_bar` is one name.
fn is_whole_word(text: &str, start: usize, end: usize) -> bool {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let wordish = |c: char| c.is_alphanumeric() || c == '_';
    !before.is_some_and(wordish) && !after.is_some_and(wordish)
}

/// Select the match at `index`, leaving the caret at its end.
///
/// Returns the value the buffer should now hold. The caret goes to the *end* so
/// that pressing Find Next again moves forward rather than finding the same
/// match a second time.
#[must_use]
pub fn select(
    value: &TextEditingValue,
    matches: &[(usize, usize)],
    index: usize,
) -> TextEditingValue {
    let mut next = value.clone();
    if let Some(&(start, end)) = matches.get(index) {
        next.selection = TextSelection::new(start, end);
    }
    next
}

/// Replace the match at `index` with `replacement`.
///
/// Returns the new value and the offset the search should resume from, or
/// `None` when there is no such match.
#[must_use]
pub fn replace_one(
    value: &TextEditingValue,
    matches: &[(usize, usize)],
    index: usize,
    replacement: &str,
) -> Option<(TextEditingValue, usize)> {
    let &(start, end) = matches.get(index)?;
    let mut text = value.text.clone();
    text.replace_range(start..end, replacement);
    let after = start + replacement.len();
    Some((
        TextEditingValue {
            text,
            selection: TextSelection::collapsed(after),
            composing: None,
            // A replace puts the caret in one place, which is one caret.
            secondary: Vec::new(),
        },
        after,
    ))
}

/// Replace every match at once.
///
/// Applied back to front so that each replacement's offsets are still the
/// offsets the scan reported — replacing forwards shifts every range after the
/// first one by the difference in length, which is the classic way a
/// replace-all quietly corrupts a file.
#[must_use]
pub fn replace_all(
    value: &TextEditingValue,
    matches: &[(usize, usize)],
    replacement: &str,
) -> (TextEditingValue, usize) {
    let mut text = value.text.clone();
    for &(start, end) in matches.iter().rev() {
        text.replace_range(start..end, replacement);
    }
    let caret = text.len().min(value.selection.extent);
    (
        TextEditingValue {
            text,
            selection: TextSelection::collapsed(caret),
            composing: None,
            secondary: Vec::new(),
        },
        matches.len(),
    )
}

/// What a workspace-wide replace would do, before it does any of it.
///
/// # Why a preview and not just a replace
///
/// Workspace search was real and good — it walked every file, grouped hits by
/// path and reported its own truncation. `replace_one` and `replace_all`
/// existed beside it and operated on **one `TextEditingValue`**: a single
/// buffer. So renaming a symbol across a project was manual, file by file.
///
/// Adding "replace in all files" without a preview would be adding the single
/// most destructive button in the application. There is no undo across files —
/// the history is per buffer, by design — so an unreviewed replace-all is a
/// change to thirty files that cannot be taken back from inside the editor.
/// [`Plan`] is what the user says yes to: every file, and how many matches in
/// each, counted before a byte is written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Plan {
    /// Each file that would change, and how many matches are in it.
    pub files: Vec<(std::path::PathBuf, usize)>,
    /// Files that matched and would **not** be changed, with the reason.
    ///
    /// Refusals are part of the plan rather than surprises during it: a file
    /// that is read-only, or open with unsaved edits, is one the user has to
    /// know about *before* saying yes, because the answer to "why did 3 of my
    /// 30 files not change" has to arrive with the offer, not afterwards.
    pub skipped: Vec<(std::path::PathBuf, &'static str)>,
}

impl Plan {
    /// How many matches would be replaced.
    #[must_use]
    pub fn total(&self) -> usize {
        self.files.iter().map(|(_, count)| count).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// The plan as the confirmation dialog lists it.
    #[must_use]
    pub fn lines(&self, root: Option<&std::path::Path>) -> Vec<String> {
        let relative = |path: &std::path::Path| -> String {
            root.and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        };
        let mut out: Vec<String> = self
            .files
            .iter()
            .map(|(path, count)| format!("{}  ({count})", relative(path)))
            .collect();
        for (path, why) in &self.skipped {
            out.push(format!("{}  \u{2014} skipped, {why}", relative(path)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> TextEditingValue {
        TextEditingValue::new(text)
    }

    #[test]
    fn an_empty_needle_matches_nothing() {
        assert!(Query::new("").matches("anything at all").is_empty());
    }

    #[test]
    fn matches_do_not_overlap() {
        let found = Query::new("aa").matches("aaaa");
        assert_eq!(
            found,
            vec![(0, 2), (2, 4)],
            "two matches, not three — otherwise Find Next never leaves the first"
        );
    }

    #[test]
    fn case_insensitive_by_default_and_exact_when_asked() {
        let mut query = Query::new("screen");
        assert_eq!(query.matches("Screen screen SCREEN").len(), 3);
        query.case_sensitive = true;
        assert_eq!(query.matches("Screen screen SCREEN").len(), 1);
    }

    #[test]
    fn whole_word_does_not_match_inside_an_identifier() {
        let mut query = Query::new("screen");
        assert_eq!(query.matches("screen screen_size myscreen").len(), 3);
        query.whole_word = true;
        assert_eq!(
            query.matches("screen screen_size myscreen").len(),
            1,
            "an underscore keeps a name together"
        );
    }

    #[test]
    fn replace_all_applies_back_to_front() {
        let before = value("a a a");
        let matches = Query::new("a").matches(&before.text);
        let (after, count) = replace_all(&before, &matches, "bbb");
        assert_eq!(after.text, "bbb bbb bbb");
        assert_eq!(count, 3);
    }

    #[test]
    fn replacing_one_leaves_the_caret_past_what_it_wrote() {
        let before = value("foo bar foo");
        let matches = Query::new("foo").matches(&before.text);
        let (after, resume) = replace_one(&before, &matches, 0, "qux").expect("a first match");
        assert_eq!(after.text, "qux bar foo");
        assert_eq!(resume, 3);
        assert_eq!(after.selection.extent, 3);
    }

    #[test]
    fn selecting_a_match_selects_it_rather_than_placing_a_caret() {
        let before = value("one two three");
        let matches = Query::new("two").matches(&before.text);
        let after = select(&before, &matches, 0);
        assert_eq!(after.selection.start(), 4);
        assert_eq!(after.selection.end(), 7);
        assert_eq!(after.selection.range().slice(&after.text), "two");
    }

    #[test]
    fn find_next_wraps_at_the_end_of_the_file() {
        let matches = Query::new("x").matches("x  x  x");
        assert_eq!(Query::index_at_or_after(&matches, 0), Some(0));
        assert_eq!(Query::index_at_or_after(&matches, 4), Some(2));
        assert_eq!(
            Query::index_at_or_after(&matches, 99),
            Some(0),
            "past the last match, the next one is the first"
        );
    }

    #[test]
    fn a_needle_longer_than_the_text_finds_nothing_and_does_not_loop() {
        assert!(Query::new("enormous").matches("no").is_empty());
    }

    #[test]
    fn a_replacement_containing_the_needle_does_not_run_away() {
        let before = value("a a");
        let matches = Query::new("a").matches(&before.text);
        let (after, count) = replace_all(&before, &matches, "aa");
        assert_eq!(after.text, "aa aa", "the scan ran before the writes, once");
        assert_eq!(count, 2);
    }
}
