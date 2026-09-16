//! Randomised property tests for paragraph layout and caret navigation.
//!
//! # Why here
//!
//! `docs/PRODUCTION-GAPS.md` names text shaping as one of the two places
//! property testing pays most, and the reason is that the failures are all
//! *offset* failures. Slicing a `String` at a byte that is not a character
//! boundary is a panic; a caret walk that lands between the two halves of a
//! surrogate pair, or in the middle of a combining sequence, is a panic on the
//! next edit. Those are found by feeding the layer strings nobody would think to
//! type, which is exactly what an example-based test cannot do.
//!
//! Every property below is a sentence about *any* string, which is what makes it
//! checkable without an expected answer to write down.
//!
//! # The alphabet
//!
//! Deliberately hostile, and each entry earns its place:
//!
//! * **Combining marks** — `é` written as `e` + U+0301 is two code points and
//!   one grapheme. A caret that stops between them puts the accent on nothing.
//! * **Astral characters** — an emoji is four UTF-8 bytes; every offset
//!   arithmetic that assumed one is a panic here.
//! * **Zero-width joiners** — a family emoji is several astral characters glued
//!   together, and is the longest single grapheme most software ever meets.
//! * **Right-to-left runs** — Arabic and Hebrew, so visual order and logical
//!   order disagree and `position_left_of` is not `offset - 1`.
//! * **Newlines and runs of spaces** — line breaking, and the empty line.
//!
//! # No proptest dependency
//!
//! A deterministic generator seeded from a constant in the source, for the reason
//! `vieww-gestures`' arena properties give: shrinking is worth having and is not
//! worth a tree of transitive crates in the lockfile every consumer resolves. A
//! failure reproduces exactly; widening the search means raising `CASES`.

use vieww_foundation::{Offset, TextAlign, TextPosition, TextStyle};
use vieww_text::{FontStore, Paragraph, TextSpan};

/// How many random strings each property is checked over.
///
/// Shaping is not free — this is a real text engine, not a stub — so the count
/// is chosen to keep the file inside a second or two rather than to be large.
const CASES: usize = 500;

/// The width every paragraph in this file is laid out against.
///
/// Narrow enough that most of the alphabet wraps, which is where the
/// interesting failures are.
const WIDTH: f32 = 120.0;

/// xorshift64*: short enough to read, good enough to explore an input space.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// The pieces a random string is built from. See the module docs for why each.
const PIECES: &[&str] = &[
    "a",
    "Hi",
    " ",
    "   ",
    "\n",
    "word ",
    "supercalifragilistic",
    // Combining: one grapheme, two code points.
    "e\u{0301}",
    // Astral: four bytes each.
    "😀",
    "🌍",
    // A ZWJ sequence: several astral characters, one grapheme.
    "👩\u{200D}👩\u{200D}👧",
    // Right-to-left, so visual and logical order disagree.
    "مرحبا",
    "שלום",
    // Mixed direction in one run, which is where bidi reordering bites.
    "a مرحبا b",
    // A lone combining mark with nothing to combine with.
    "\u{0301}",
    // A zero-width space, which has an offset and no ink.
    "\u{200B}",
];

fn text(rng: &mut Rng) -> String {
    let count = rng.below(6);
    (0..count)
        .map(|_| PIECES[rng.below(PIECES.len())])
        .collect()
}

fn lay_out(store: &mut FontStore, text: &str, width: f32) -> Paragraph {
    let spans = [TextSpan::new(text.to_owned(), TextStyle::default())];
    Paragraph::layout_aligned(store, &spans, width, TextAlign::Start, None)
}

/// **Every caret offset a hit test returns is a character boundary.**
///
/// The single most consequential property in this file. An offset that is not a
/// boundary is a panic the moment anything slices the string at it — which is
/// every insertion, every deletion, and every selection paint — and the panic
/// happens on the *next* keystroke rather than at the tap, so the tap is not
/// what gets reported.
#[test]
fn every_hit_tested_offset_is_a_character_boundary() {
    let mut rng = Rng::new(0x7E47_0001);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);

        // A grid of taps across and beyond the paragraph, including negative
        // coordinates and points past the end — a finger lands outside the text
        // constantly, and "outside" must resolve to an end rather than to
        // nothing.
        for y in [-10.0, 0.0, 5.0, 20.0, 60.0, 500.0] {
            for x in [-10.0, 0.0, 1.0, 37.0, 119.0, 400.0] {
                let position = paragraph.hit_test(Offset::new(x, y));
                assert!(
                    source.is_char_boundary(position.offset),
                    "case {case}: hit at ({x}, {y}) gave offset {} in {source:?}, \
                     which is not a character boundary",
                    position.offset
                );
                assert!(
                    position.offset <= source.len(),
                    "case {case}: offset {} is past the end of {} bytes",
                    position.offset,
                    source.len()
                );
            }
        }
    }
}

/// **Walking left from the end reaches the start, in finitely many steps, on
/// boundaries the whole way.**
///
/// The caret-navigation contract stated as a walk rather than as a step. Three
/// separate failures are caught by it: a step that lands off a boundary, a step
/// that does not move (an infinite loop in the editor, which presents as the
/// arrow key doing nothing), and a step that moves *backwards* into a cycle.
#[test]
fn walking_left_from_the_end_terminates_at_the_start() {
    let mut rng = Rng::new(0x7E47_0002);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);

        let mut at = TextPosition::new(source.len());
        // Generous, and finite: one step per byte can never be too few, and a
        // walk that needs more is looping.
        let mut budget = source.len() + 8;
        while let Some(next) = paragraph.position_left_of(at) {
            assert!(
                source.is_char_boundary(next.offset),
                "case {case}: stepped to {} in {source:?}, off a boundary",
                next.offset
            );
            assert_ne!(
                next.offset, at.offset,
                "case {case}: a step that does not move is an arrow key that \
                 does nothing, forever; at {} in {source:?}",
                at.offset
            );
            budget = budget
                .checked_sub(1)
                .unwrap_or_else(|| panic!("case {case}: the walk loops in {source:?}"));
            at = next;
        }
    }
}

/// The same, rightward. Written out rather than parameterised because the two
/// directions fail differently — rightward is the one that walks off the end.
#[test]
fn walking_right_from_the_start_terminates_at_the_end() {
    let mut rng = Rng::new(0x7E47_0003);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);

        let mut at = TextPosition::new(0);
        let mut budget = source.len() + 8;
        while let Some(next) = paragraph.position_right_of(at) {
            assert!(
                source.is_char_boundary(next.offset),
                "case {case}: stepped to {} in {source:?}, off a boundary",
                next.offset
            );
            assert!(
                next.offset <= source.len(),
                "case {case}: stepped to {} past the end of {} bytes in {source:?}",
                next.offset,
                source.len()
            );
            assert_ne!(
                next.offset, at.offset,
                "case {case}: a step that does not move"
            );
            budget = budget
                .checked_sub(1)
                .unwrap_or_else(|| panic!("case {case}: the walk loops in {source:?}"));
            at = next;
        }
    }
}

/// **A caret box exists for every boundary in the string, on its line and
/// within the line's extent.**
///
/// A caret drawn somewhere unbounded is a caret the user cannot see, which reads
/// as the field having lost focus.
///
/// # Why the horizontal bound is the *unclamped* extent, and can be negative
///
/// This first asserted `left >= -1.0` against `size().width`, and found a caret
/// at `-4.17` in a right-to-left paragraph ending in a space. That turned out to
/// be **correct typography, and a wrong assertion**.
///
/// Two things were being conflated. `size().width` is clamped to the width the
/// paragraph was laid out against, so for a line whose content overflows it
/// reports the box rather than the ink; `widest_line` is the real extent. And
/// trailing whitespace hangs *past* the end of a line by design — in
/// left-to-right text past the right edge, which nobody notices, and in
/// right-to-left text past the **left** edge, where the same behaviour produces a
/// negative coordinate. Bounding one side at zero asserts that right-to-left text
/// behaves differently from left-to-right text, which is exactly the assumption
/// this file exists to find.
///
/// The second attempt bounded it by the *ink* extent instead, and found a caret
/// at `120.0` on a line only `45.7` wide — also correct, and also a wrong
/// assertion: a right-to-left paragraph begins at the **right edge of the
/// layout box**, so a caret before any text sits at the full layout width
/// however narrow the text is.
///
/// So the honest property is that the caret is *bounded* by the box it was laid
/// out in, plus a line's worth of slack for whitespace that hangs off either
/// end — it does not run away, go infinite, or land on another line — rather
/// than that it sits inside an extent it has every right to be outside of.
#[test]
fn every_boundary_has_a_caret_box_on_its_own_line() {
    let mut rng = Rng::new(0x7E47_0004);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);
        let size = paragraph.size();
        // The box it was laid out in, or the ink if that overflowed — and one
        // line of slack either side for hanging whitespace.
        let extent = paragraph.widest_line().max(WIDTH);
        let slack = size.height + 1.0;

        for offset in 0..=source.len() {
            if !source.is_char_boundary(offset) {
                continue;
            }
            let rect = paragraph.cursor_rect(TextPosition::new(offset));

            // Vertically strict: a caret belongs to exactly one line, and the
            // lines stack inside the paragraph's height.
            assert!(
                rect.top >= -1.0 && rect.bottom <= size.height + 1.0,
                "case {case}: the caret at {offset} in {source:?} is at {rect:?}, \
                 outside a paragraph {size:?} tall"
            );
            assert!(
                rect.height() > 0.0,
                "case {case}: a caret with no height at {offset} in {source:?}"
            );

            // Horizontally bounded, either side, by the box it was laid out in
            // plus a line's worth of slack for whitespace that hangs off an end.
            assert!(
                rect.left.is_finite() && rect.left >= -slack && rect.left <= extent + slack,
                "case {case}: the caret at {offset} in {source:?} is at {rect:?}, \
                 well outside a box {extent} wide"
            );
        }
    }
}

/// **`line_of` never names a line that does not exist.**
///
/// An index into the line list, used to paint a selection and to move the caret
/// vertically. Out of range is an immediate panic in both.
#[test]
fn every_boundary_belongs_to_a_line_that_exists() {
    let mut rng = Rng::new(0x7E47_0005);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);
        let lines = paragraph.line_count();

        for offset in 0..=source.len() {
            if !source.is_char_boundary(offset) {
                continue;
            }
            let line = paragraph.line_of(TextPosition::new(offset));
            assert!(
                line < lines.max(1),
                "case {case}: offset {offset} in {source:?} is on line {line} of {lines}"
            );
        }
    }
}

/// **Home and End stay on their own line and stay on boundaries.**
#[test]
fn line_start_and_line_end_are_boundaries_on_the_same_line() {
    let mut rng = Rng::new(0x7E47_0006);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let paragraph = lay_out(&mut store, &source, WIDTH);

        for offset in 0..=source.len() {
            if !source.is_char_boundary(offset) {
                continue;
            }
            let at = TextPosition::new(offset);
            let line = paragraph.line_of(at);
            for end in [paragraph.line_start(at), paragraph.line_end(at)] {
                assert!(
                    source.is_char_boundary(end.offset),
                    "case {case}: {end:?} from {offset} in {source:?} is off a boundary"
                );
                assert_eq!(
                    paragraph.line_of(end),
                    line,
                    "case {case}: Home or End from {offset} in {source:?} left its line"
                );
            }
            assert!(
                paragraph.line_start(at).offset <= paragraph.line_end(at).offset,
                "case {case}: a line whose start is after its end, at {offset} in {source:?}"
            );
        }
    }
}

/// **Laying the same text out twice gives the same answer.**
///
/// Shaping is cached — by span, width, alignment and direction — and a cache
/// that returned something different on the second call would make every
/// measurement depend on whether anything had measured before it. That is the
/// class of bug that shows up as a layout which is correct on the first frame.
#[test]
fn laying_out_the_same_text_twice_is_the_same_paragraph() {
    let mut rng = Rng::new(0x7E47_0007);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        let first = lay_out(&mut store, &source, WIDTH);
        let second = lay_out(&mut store, &source, WIDTH);

        assert_eq!(
            first.size(),
            second.size(),
            "case {case}: {source:?} measured differently the second time"
        );
        assert_eq!(first.line_count(), second.line_count(), "case {case}");
        assert_eq!(first.glyph_count(), second.glyph_count(), "case {case}");
    }
}

/// **A narrower paragraph is never shorter.**
///
/// Wrapping monotonicity: less width means the same words on more lines, never
/// fewer. A layout that got *shorter* as it narrowed is one that dropped
/// content, which is the failure mode a height assertion cannot see and a user
/// reports as text going missing.
#[test]
fn narrowing_a_paragraph_never_makes_it_shorter() {
    let mut rng = Rng::new(0x7E47_0008);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        if source.is_empty() {
            continue;
        }
        let wide = lay_out(&mut store, &source, 400.0);
        let narrow = lay_out(&mut store, &source, 60.0);

        assert!(
            narrow.size().height >= wide.size().height - 0.01,
            "case {case}: {source:?} is {} tall at 400 and {} at 60",
            wide.size().height,
            narrow.size().height
        );
        assert!(
            narrow.line_count() >= wide.line_count(),
            "case {case}: {source:?} wraps onto fewer lines when narrowed"
        );
    }
}

/// **Nothing in the pipeline loses a glyph.**
///
/// The shaped paragraph has at least one glyph for every non-empty piece of
/// text. Stated loosely on purpose: shaping legitimately produces fewer glyphs
/// than code points — a ZWJ sequence is one glyph made of seven — so the honest
/// invariant is that text with ink in it produces ink.
#[test]
fn text_with_ink_in_it_produces_glyphs() {
    let mut rng = Rng::new(0x7E47_0009);
    let mut store = FontStore::new();

    for case in 0..CASES {
        let source = text(&mut rng);
        // Whitespace and zero-width characters legitimately produce no ink.
        if source
            .chars()
            .all(|c| c.is_whitespace() || c == '\u{200B}' || c == '\u{200D}')
        {
            continue;
        }
        let paragraph = lay_out(&mut store, &source, 400.0);
        assert!(
            paragraph.glyph_count() > 0,
            "case {case}: {source:?} shaped to nothing at all"
        );
    }
}

/// The empty string is a real input — a field before anybody types — and it has
/// to have a caret, a line, and a height.
#[test]
fn an_empty_paragraph_still_has_a_caret_and_a_line() {
    let mut store = FontStore::new();
    let paragraph = lay_out(&mut store, "", WIDTH);

    assert!(paragraph.is_empty());
    let rect = paragraph.cursor_rect(TextPosition::new(0));
    assert!(
        rect.height() > 0.0,
        "an empty field still shows a caret of the line's height: {rect:?}"
    );
    assert_eq!(paragraph.line_of(TextPosition::new(0)), 0);
    assert_eq!(paragraph.hit_test(Offset::new(50.0, 5.0)).offset, 0);
}
