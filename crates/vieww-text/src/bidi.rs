//! Bidirectional text: the Unicode Bidi Algorithm (UAX #9), basic level.
//!
//! # The problem
//!
//! A line of text can contain both left-to-right (English) and
//! right-to-left (Arabic, Hebrew) runs. The *logical* order (reading
//! order) and the *visual* order (left-to-right on screen) are different:
//!
//! ```text
//! Logical:  H I   A R A B I C   W O R D
//! Visual:   H I   D R A W   C I B A R A     ← reversed
//! ```
//!
//! No — that's wrong. The Arabic word's *characters* are stored in
//! logical order (first letter first), but rendered right-to-left. When
//! mixed with LTR text:
//!
//! ```text
//! Logical:  "hello" "سلام" "world"
//! Visual:   "hello" [ملاس reversed] "world"
//! ```
//!
//! The Arabic run is reversed as a unit; the surrounding English runs
//! are not. The algorithm determines which runs are RTL and reverses
//! them.
//!
//! # This implementation
//!
//! The full UAX #9 algorithm has ~70 rules. This implements the subset
//! that covers the common cases:
//!
//! - LTR text with embedded RTL runs (English with Arabic/Hebrew words)
//! - RTL text with embedded LTR runs (Arabic with English words/numbers)
//! - Numbers inside RTL text (numbers stay LTR)
//!
//! Not handled: nested bidi (RTL inside LTR inside RTL), explicit
//! embedding controls (LRE/RLE/PDF), isolate markers (LRI/RLI/FSI/PDI).
//! These are rare in UI text; when they appear, the text is treated as
//! if the controls were absent.

/// The resolved direction of a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
    /// Neutral: space, punctuation. Takes the direction of its context.
    Neutral,
}

/// The paragraph's base direction.
///
/// Determined by the first strong directional character, or set
/// explicitly (e.g. from the app locale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParagraphDirection {
    LeftToRight,
    RightToLeft,
}

/// A run of text with a single resolved direction.
#[derive(Debug, Clone, PartialEq)]
pub struct BidiRun {
    /// The byte range in the source text.
    pub range: std::ops::Range<usize>,
    /// The resolved direction for rendering.
    pub direction: Direction,
}

/// Determine the base direction of a paragraph.
///
/// The first strong directional character (an Arabic/Hebrew letter or a
/// Latin letter) sets the base. If none is found, LTR (the Unicode
/// default).
#[must_use]
pub fn paragraph_direction(text: &str) -> ParagraphDirection {
    for ch in text.chars() {
        if is_strong_rtl(ch) {
            return ParagraphDirection::RightToLeft;
        }
        if is_strong_ltr(ch) {
            return ParagraphDirection::LeftToRight;
        }
    }
    ParagraphDirection::LeftToRight
}

/// Split text into runs of consistent direction.
///
/// The returned runs are in *logical* order (reading order). The renderer
/// reverses the run order (not the characters within runs) for RTL
/// paragraphs.
#[must_use]
pub fn resolve_bidi(text: &str, base: ParagraphDirection) -> Vec<BidiRun> {
    if text.is_empty() {
        return Vec::new();
    }

    let base_dir = match base {
        ParagraphDirection::LeftToRight => Direction::LeftToRight,
        ParagraphDirection::RightToLeft => Direction::RightToLeft,
    };

    // Pass 1 — classify every character, leaving neutrals unresolved.
    let mut classes: Vec<(usize, usize, Direction)> = Vec::new();
    for (offset, ch) in text.char_indices() {
        let end = offset + ch.len_utf8();
        let dir = if is_strong_rtl(ch) {
            Direction::RightToLeft
        } else if is_strong_ltr(ch) || is_number(ch) {
            // Numbers are weak LTR: they run LTR inside RTL text, which is why
            // "١٢٣" and "123" both read left-to-right in an Arabic sentence.
            Direction::LeftToRight
        } else {
            Direction::Neutral
        };
        classes.push((offset, end, dir));
    }

    // Pass 2 — UAX #9 rule N1/N2. A neutral span takes the direction of the
    // runs on both sides when they agree, and the paragraph direction when
    // they differ or when the span touches the start or end of the text.
    //
    // The earlier implementation instead let a neutral inherit whatever run
    // preceded it, which merged the space after an Arabic word into the
    // following digit run: "العدد 123 هنا" produced an LTR run of "123 "
    // rather than a clean "123", so the digits and the space that separates
    // them from the Arabic were treated as one left-to-right unit.
    let mut i = 0;
    while i < classes.len() {
        if classes[i].2 != Direction::Neutral {
            i += 1;
            continue;
        }

        let span_start = i;
        while i < classes.len() && classes[i].2 == Direction::Neutral {
            i += 1;
        }
        let span_end = i;

        let before = classes[..span_start]
            .iter()
            .rev()
            .find_map(|c| (c.2 != Direction::Neutral).then_some(c.2));
        let after = classes[span_end..]
            .iter()
            .find_map(|c| (c.2 != Direction::Neutral).then_some(c.2));

        let resolved = match (before, after) {
            (Some(a), Some(b)) if a == b => a,
            _ => base_dir,
        };

        for entry in &mut classes[span_start..span_end] {
            entry.2 = resolved;
        }
    }

    // Pass 3 — coalesce neighbouring characters of equal direction.
    let mut runs: Vec<BidiRun> = Vec::new();
    for (start, end, dir) in classes {
        match runs.last_mut() {
            Some(run) if run.direction == dir => run.range.end = end,
            _ => runs.push(BidiRun {
                range: start..end,
                direction: dir,
            }),
        }
    }

    runs
}

/// Whether a character is a strong RTL character (Arabic, Hebrew).
///
/// Public because the real path — `runs_of`, through this module — picks a
/// run's direction with it, and a private copy anywhere else would be a second
/// table to keep in sync. (`shaping_improved`, the module that first made this
/// public, was deleted on 2026-08-30; see the tombstone in `lib.rs`.)
#[must_use]
pub fn is_strong_rtl(ch: char) -> bool {
    matches!(ch as u32,
        0x0590..=0x05FF |    // Hebrew
        0x0600..=0x06FF |    // Arabic
        0x0700..=0x074F |    // Syriac
        0x0750..=0x077F |    // Arabic Supplement
        0x08A0..=0x08FF |    // Arabic Extended-A
        0xFB1D..=0xFDFF |    // Arabic Presentation Forms-A
        0xFE70..=0xFEFF      // Arabic Presentation Forms-B
    )
}

/// Whether a character is a strong LTR character (Latin, Greek, Cyrillic).
#[must_use]
pub fn is_strong_ltr(ch: char) -> bool {
    matches!(ch as u32,
        0x0041..=0x005A |    // Latin uppercase
        0x0061..=0x007A |    // Latin lowercase
        0x00C0..=0x024F |    // Latin Extended
        0x0370..=0x03FF |    // Greek
        0x0400..=0x04FF |    // Cyrillic
        0x4E00..=0x9FFF      // CJK (treated as LTR for line direction)
    )
}

/// Whether a character is a digit or number.
fn is_number(ch: char) -> bool {
    ch.is_ascii_digit()
        || matches!(ch as u32,
            0x0660..=0x0669 |    // Arabic-Indic digits
            0x06F0..=0x06F9      // Extended Arabic-Indic digits
        )
}

/// Reorder runs for visual display.
///
/// For an RTL paragraph, the runs are reversed (the first logical run
/// appears rightmost). For LTR, they stay in order.
#[must_use]
pub fn visual_order(runs: &[BidiRun], base: ParagraphDirection) -> Vec<BidiRun> {
    match base {
        ParagraphDirection::LeftToRight => runs.to_vec(),
        ParagraphDirection::RightToLeft => {
            let mut reversed = runs.to_vec();
            reversed.reverse();
            reversed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_english_is_ltr() {
        let runs = resolve_bidi("hello world", ParagraphDirection::LeftToRight);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].direction, Direction::LeftToRight);
    }

    #[test]
    fn pure_arabic_is_rtl() {
        let runs = resolve_bidi("مرحبا", ParagraphDirection::RightToLeft);
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|r| r.direction == Direction::RightToLeft));
    }

    #[test]
    fn mixed_text_has_multiple_runs() {
        // English with an embedded Arabic word.
        let text = "hello مرحبا world";
        let runs = resolve_bidi(text, ParagraphDirection::LeftToRight);
        assert!(
            runs.len() >= 2,
            "at least an LTR and an RTL run, got {} runs",
            runs.len()
        );

        let has_ltr = runs.iter().any(|r| r.direction == Direction::LeftToRight);
        let has_rtl = runs.iter().any(|r| r.direction == Direction::RightToLeft);
        assert!(has_ltr, "there is an LTR run");
        assert!(has_rtl, "there is an RTL run");
    }

    #[test]
    fn numbers_in_rtl_text_stay_ltr() {
        // Arabic text with an embedded number.
        let text = "العدد 123 هنا";
        let runs = resolve_bidi(text, ParagraphDirection::RightToLeft);
        // The number run should be LTR.
        let number_run = runs
            .iter()
            .find(|r| text[r.range.clone()].chars().all(|c| c.is_ascii_digit()));
        assert!(
            number_run.is_some_and(|r| r.direction == Direction::LeftToRight),
            "digits stay LTR in RTL text"
        );
    }

    #[test]
    fn paragraph_direction_from_first_strong_char() {
        assert_eq!(
            paragraph_direction("hello"),
            ParagraphDirection::LeftToRight
        );
        assert_eq!(
            paragraph_direction("مرحبا"),
            ParagraphDirection::RightToLeft
        );
        assert_eq!(
            paragraph_direction("123"),
            ParagraphDirection::LeftToRight,
            "neutral default is LTR"
        );
    }

    #[test]
    fn rtl_paragraph_reverses_run_order() {
        let runs = vec![
            BidiRun {
                range: 0..5,
                direction: Direction::LeftToRight,
            },
            BidiRun {
                range: 5..10,
                direction: Direction::RightToLeft,
            },
        ];
        let visual = visual_order(&runs, ParagraphDirection::RightToLeft);
        assert_eq!(
            visual[0].range,
            5..10,
            "the second logical run is first visually"
        );
    }
}
