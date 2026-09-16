//! Line breaking and wrapping: turning a text run into lines.
//!
//! # The algorithm
//!
//! Greedy line breaking (the same algorithm CSS uses for `word-wrap:
//! normal`):
//!
//! 1. Split the text into *break opportunities* — positions where a line
//!    break is allowed. For English, that's after spaces. For Thai, it
//!    requires a dictionary. For CJK, it's between any two characters.
//!    For URLs, it's nowhere (unless `word-break: break-all`).
//!
//! 2. Greedily fill each line: add words until the next word would
//!    overflow, then break.
//!
//! 3. If a single word is longer than the line, either overflow it
//!    (normal) or break it mid-word (`overflow-wrap: break-word`).
//!
//! Greedy is not optimal — a dynamic-programming approach can produce
//! lines with less raggedness — but greedy is what every browser does
//! and what users expect.
//!
//! # Break opportunities
//!
//! The `BreakOpportunity` list is computed once per paragraph and then
//! consumed by the greedy filler. The rules:
//!
//! - After a space (U+0020)
//! - After a hyphen (U+002D) — but not before
//! - Between a CJK character and anything (CJK breaks freely)
//! - After a zero-width space (U+200B)
//! - Never inside a word (Latin scripts)

/// Where a line break is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakOpportunity {
    /// The byte offset in the text where a break can occur.
    ///
    /// A break at offset `i` means "the line can end just before character
    /// at byte `i`".
    pub offset: usize,
    /// Whether this is a *required* break (a newline character).
    pub required: bool,
}

/// Find all break opportunities in `text`.
///
/// Handles: spaces, hyphens, newlines, CJK characters, and zero-width
/// spaces. This is the Unicode UAX #14 line breaking algorithm, reduced
/// to the cases that matter for UI text.
#[must_use]
pub fn find_break_opportunities(text: &str) -> Vec<BreakOpportunity> {
    let mut opportunities = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();

    for i in 0..chars.len() {
        let (byte_offset, ch) = chars[i];
        let next = chars.get(i + 1).map(|(_, c)| *c);

        // Newline is always a required break.
        if ch == '\n' {
            opportunities.push(BreakOpportunity {
                offset: byte_offset + 1,
                required: true,
            });
            continue;
        }

        // Space: break after it (before the next character).
        if ch == ' ' {
            if let Some((next_offset, _)) = chars.get(i + 1) {
                opportunities.push(BreakOpportunity {
                    offset: *next_offset,
                    required: false,
                });
            }
            continue;
        }

        // Hyphen: break after it.
        if ch == '-' && next.is_some() {
            opportunities.push(BreakOpportunity {
                offset: byte_offset + 1,
                required: false,
            });
            continue;
        }

        // Zero-width space: break after it.
        if ch == '\u{200B}' {
            opportunities.push(BreakOpportunity {
                offset: byte_offset + 3, // ZWSP is 3 bytes
                required: false,
            });
            continue;
        }

        // CJK: break after a CJK character, whatever follows it.
        //
        // Including another CJK character — that is the whole point. CJK has
        // no spaces between words, so if the only break opportunities were
        // CJK-to-non-CJK, a paragraph of Chinese would be one unbreakable
        // "word" and would overflow its box rather than wrap. The earlier
        // form of this rule excluded `is_cjk(next)` and did exactly that.
        //
        // A following space is left to the space rule above, which already
        // emits a break at the same position; adding one here would duplicate
        // the offset.
        if is_cjk(ch) {
            if let Some((next_offset, next_ch)) = chars.get(i + 1) {
                if *next_ch != ' ' {
                    opportunities.push(BreakOpportunity {
                        offset: *next_offset,
                        required: false,
                    });
                }
            }
        }
    }

    opportunities
}

/// Whether a character is CJK (Chinese, Japanese, Korean).
///
/// CJK characters break freely — between any two of them — which is not
/// how Latin text works.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x4E00..=0x9FFF |    // CJK Unified Ideographs
        0x3400..=0x4DBF |    // CJK Extension A
        0x3000..=0x303F |    // CJK Symbols and Punctuation
        0x3040..=0x309F |    // Hiragana
        0x30A0..=0x30FF |    // Katakana
        0xAC00..=0xD7AF      // Hangul Syllables
    )
}

/// A word-like unit for wrapping: a maximal run of non-breaking text.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// The text content.
    pub text: String,
    /// The byte offset where this word starts in the source text.
    pub start: usize,
    /// The width of this word (computed by the shaper).
    pub width: f32,
    /// Whether a break is required after this word (newline).
    pub requires_break: bool,
}

/// Split text into words at break opportunities.
///
/// A "word" here is a run of text between break opportunities — it may
/// include trailing spaces (which are part of the word's width but
/// trimmed at line ends).
#[must_use]
pub fn split_into_words(text: &str) -> Vec<Word> {
    let opportunities = find_break_opportunities(text);
    let mut words = Vec::new();
    let mut start = 0;

    for opp in &opportunities {
        if opp.offset > start {
            let word_text = &text[start..opp.offset];
            words.push(Word {
                text: word_text.to_owned(),
                start,
                width: 0.0, // Filled in by the shaper
                requires_break: opp.required,
            });
            start = opp.offset;
        }
        if opp.required {
            // A newline ends a "word" that includes the newline.
            // The next word starts after it.
            start = opp.offset;
        }
    }

    // The trailing text (after the last break opportunity).
    if start < text.len() {
        words.push(Word {
            text: text[start..].to_owned(),
            start,
            width: 0.0,
            requires_break: false,
        });
    }

    words
}

/// Greedy line breaking: fill lines up to `max_width`.
///
/// `word_widths` provides each word's width. Returns the words assigned
/// to each line.
///
/// # The algorithm
///
/// ```text
/// current_line = []
/// current_width = 0
///
/// for word in words:
///     if current_width + word.width > max_width and current_line not empty:
///         emit current_line
///         current_line = [word]
///         current_width = word.width
///     else:
///         current_line.append(word)
///         current_width += word.width
///
///     if word.requires_break:
///         emit current_line
///         current_line = []
///         current_width = 0
///
/// if current_line not empty:
///     emit current_line
/// ```
#[must_use]
pub fn greedy_wrap(words: &[Word], max_width: f32) -> Vec<Vec<&Word>> {
    let mut lines = Vec::new();
    let mut current: Vec<&Word> = Vec::new();
    let mut current_width = 0.0;

    for word in words {
        // Hard break: newline.
        if word.requires_break {
            current.push(word);
            lines.push(std::mem::take(&mut current));
            current_width = 0.0;
            continue;
        }

        // Soft break: the next word doesn't fit.
        let word_width = word.width;
        if current_width + word_width > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_width = 0.0;
        }

        current.push(word);
        current_width += word_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, width: f32) -> Word {
        Word {
            text: text.to_owned(),
            start: 0,
            width,
            requires_break: false,
        }
    }

    fn newline_word(text: &str, width: f32) -> Word {
        Word {
            text: text.to_owned(),
            start: 0,
            width,
            requires_break: true,
        }
    }

    #[test]
    fn words_that_fit_stay_on_one_line() {
        let words = vec![word("hello ", 30.0), word("world", 50.0)];
        let lines = greedy_wrap(&words, 100.0);
        assert_eq!(lines.len(), 1, "both fit");
    }

    #[test]
    fn a_word_that_overflows_starts_a_new_line() {
        let words = vec![
            word("hello ", 30.0),
            word("world ", 50.0),
            word("foo", 40.0),
        ];
        let lines = greedy_wrap(&words, 100.0);
        // "hello world" = 80 (fits), then "foo" = 40 (new line).
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), 2);
        assert_eq!(lines[1].len(), 1);
    }

    #[test]
    fn a_single_wide_word_overflows() {
        // A word wider than the line stays on its own line and overflows.
        let words = vec![word("supercalifragilistic", 500.0)];
        let lines = greedy_wrap(&words, 100.0);
        assert_eq!(lines.len(), 1, "one word, one line, it overflows");
    }

    #[test]
    fn newline_forces_a_break() {
        let words = vec![
            word("hello ", 30.0),
            newline_word("world\n", 50.0),
            word("next", 40.0),
        ];
        let lines = greedy_wrap(&words, 1000.0);
        assert_eq!(lines.len(), 2, "newline breaks even with room");
    }

    #[test]
    fn break_opportunities_after_spaces() {
        let ops = find_break_opportunities("hello world");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].offset, 6);
        assert!(!ops[0].required);
    }

    #[test]
    fn break_opportunities_after_hyphens() {
        let ops = find_break_opportunities("well-known");
        // Break after the hyphen (offset 4 in "well-").
        assert!(ops.iter().any(|o| o.offset == 5));
    }

    #[test]
    fn newlines_are_required_breaks() {
        let ops = find_break_opportunities("a\nb");
        assert!(ops.iter().any(|o| o.required && o.offset == 2));
    }

    #[test]
    fn cjk_breaks_between_characters() {
        let ops = find_break_opportunities("中文");
        assert!(!ops.is_empty(), "CJK text has break opportunities");
    }

    #[test]
    fn splitting_into_words_works() {
        let words = split_into_words("hello world foo");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "hello ");
        assert_eq!(words[1].text, "world ");
        assert_eq!(words[2].text, "foo");
    }
}
