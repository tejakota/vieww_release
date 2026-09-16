//! Cutting a string to a *width*, rather than to a character count.
//!
//! # The thing this replaces
//!
//! Every surface that has to fit a name into a fixed slot reaches for the same
//! shortcut first: keep twenty-four characters and add an ellipsis. It is one
//! line, it never panics, and it is wrong in both directions — `WWWWWWWW` at
//! twenty-four characters is half again as wide as `illillillil` at the same
//! count, so the same rule either overflows the slot or leaves a third of it
//! empty depending on what the user happened to name their file. The studio's
//! tab strip carried a comment admitting exactly this, and the reason given was
//! that there was no way to ask for an ellipsis at a pixel boundary.
//!
//! This is that way. It lives here rather than in the widget layer because a
//! pixel boundary is a question only the shaper can answer: the same string in
//! the same nominal size is a different width in a different font, at a
//! different weight, after a fallback for a script the first font did not cover.
//!
//! # Cost
//!
//! A binary search over character boundaries — about six shaping passes for a
//! sixty-character name, all of them hitting [`crate::shape_cache`] on every
//! frame after the first. It runs in layout, and only when the text does not
//! already fit.

use vieww_foundation::{TextAlign, TextDirection, TextStyle};

use crate::{FontStore, Paragraph, TextOverflow, TextSpan};

/// The character an elision leaves behind.
const ELLIPSIS: char = '\u{2026}';

/// `text` cut to `max_width`, or `None` if it already fits.
///
/// `mode` decides where the cut goes: [`TextOverflow::Ellipsis`] takes it off
/// the end, [`TextOverflow::EllipsisMiddle`] out of the middle. Any other mode
/// returns `None` — those do not replace text, and a caller that hands one over
/// is asking for the string unchanged.
///
/// The result is guaranteed to measure no wider than `max_width` **unless even
/// a bare ellipsis does not fit**, in which case the ellipsis is returned
/// anyway: something the reader can recognise as "there was more here" beats an
/// empty slot, and a slot that narrow is a layout bug this function cannot fix.
#[must_use]
pub fn elide_to_width(
    store: &mut FontStore,
    text: &str,
    style: &TextStyle,
    align: TextAlign,
    direction: Option<TextDirection>,
    max_width: f32,
    mode: TextOverflow,
) -> Option<String> {
    if !mode.elides() || !max_width.is_finite() || max_width <= 0.0 {
        return None;
    }

    let mut measure = |candidate: &str| -> f32 {
        let spans = [TextSpan::new(candidate.to_owned(), *style)];
        Paragraph::layout_aligned(store, &spans, f32::INFINITY, align, direction).widest_line()
    };

    // Measured on one line: an elision is a single-line answer, and asking at
    // `max_width` would wrap instead of overflowing and always look like it fit.
    if measure(text) <= max_width {
        return None;
    }

    let characters: Vec<char> = text.chars().collect();
    // How many of the original characters survive. Monotone in the width of the
    // result — every extra character is at least as wide — so a binary search
    // finds the largest that still fits, and does it in six shapes rather than
    // sixty.
    let mut low = 0_usize;
    let mut high = characters.len();
    let mut best = build(&characters, 0, mode);

    while low <= high {
        let keep = usize::midpoint(low, high);
        let candidate = build(&characters, keep, mode);
        if measure(&candidate) <= max_width {
            best = candidate;
            low = keep + 1;
        } else if keep == 0 {
            break;
        } else {
            high = keep - 1;
        }
    }

    Some(best)
}

/// `keep` of the original characters, plus the ellipsis, arranged per `mode`.
fn build(characters: &[char], keep: usize, mode: TextOverflow) -> String {
    let keep = keep.min(characters.len());
    match mode {
        TextOverflow::EllipsisMiddle => {
            // The head gets the odd character. A name is more often
            // distinguished by how it starts than by how it ends, and the tail
            // is already carrying the extension.
            let head = keep.div_ceil(2);
            let tail = keep - head;
            let mut out: String = characters[..head].iter().collect();
            out.push(ELLIPSIS);
            out.extend(&characters[characters.len() - tail..]);
            out
        }
        _ => {
            let mut out: String = characters[..keep].iter().collect();
            out.push(ELLIPSIS);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> FontStore {
        FontStore::embedded_only()
    }

    fn width(store: &mut FontStore, text: &str, style: &TextStyle) -> f32 {
        let spans = [TextSpan::new(text.to_owned(), *style)];
        Paragraph::layout_aligned(store, &spans, f32::INFINITY, TextAlign::Start, None)
            .widest_line()
    }

    fn elide(
        store: &mut FontStore,
        text: &str,
        max_width: f32,
        mode: TextOverflow,
    ) -> Option<String> {
        elide_to_width(
            store,
            text,
            &TextStyle::new(12.5),
            TextAlign::Start,
            None,
            max_width,
            mode,
        )
    }

    #[test]
    fn text_that_fits_is_left_alone() {
        let mut store = store();
        let style = TextStyle::new(12.5);
        let room = width(&mut store, "main.rs", &style) + 10.0;
        assert_eq!(
            elide(&mut store, "main.rs", room, TextOverflow::Ellipsis),
            None
        );
    }

    #[test]
    fn what_comes_back_fits_the_width_it_was_given() {
        let mut store = store();
        let style = TextStyle::new(12.5);
        let long = "a_very_long_module_name_indeed.rs";
        let room = width(&mut store, long, &style) / 2.0;

        for mode in [TextOverflow::Ellipsis, TextOverflow::EllipsisMiddle] {
            let cut = elide(&mut store, long, room, mode).expect("it did not fit");
            assert!(
                width(&mut store, &cut, &style) <= room,
                "{mode:?} returned {cut:?}, which is still wider than {room}"
            );
            assert!(cut.contains(ELLIPSIS), "{mode:?} said nothing was removed");
        }
    }

    #[test]
    fn a_wide_string_keeps_fewer_characters_than_a_narrow_one() {
        // The whole point: the answer is a width, not a count. Same character
        // count in, different character count out.
        let mut store = store();
        let wide = elide(&mut store, &"W".repeat(40), 120.0, TextOverflow::Ellipsis)
            .expect("it did not fit");
        let narrow = elide(&mut store, &"i".repeat(40), 120.0, TextOverflow::Ellipsis)
            .expect("it did not fit");
        assert!(
            wide.chars().count() < narrow.chars().count(),
            "W is wider than i, so fewer of them fit: {wide:?} vs {narrow:?}"
        );
    }

    #[test]
    fn the_middle_cut_keeps_both_ends() {
        let mut store = store();
        let style = TextStyle::new(12.5);
        let name = "login_screen_controller.rs";
        let room = width(&mut store, name, &style) * 0.6;
        let cut =
            elide(&mut store, name, room, TextOverflow::EllipsisMiddle).expect("it did not fit");

        assert!(cut.starts_with('l'), "the head survives: {cut:?}");
        assert!(cut.ends_with(".rs"), "and the extension does: {cut:?}");
    }

    #[test]
    fn a_slot_too_narrow_for_anything_still_says_something_was_cut() {
        let mut store = store();
        let cut = elide(&mut store, "settings_form.rs", 0.5, TextOverflow::Ellipsis)
            .expect("it did not fit");
        assert_eq!(cut, "\u{2026}");
    }

    #[test]
    fn a_multi_byte_name_is_cut_on_a_character_boundary() {
        // The failure this replaces is a panic, not a wrong pixel: slicing a
        // `String` at byte 24 lands mid-codepoint on any name that is not ASCII.
        let mut store = store();
        let name = "ラーメン屋さんの画面.rs";
        let cut =
            elide(&mut store, name, 40.0, TextOverflow::EllipsisMiddle).expect("it did not fit");
        assert!(name.starts_with(cut.chars().next().into_iter().collect::<String>().as_str()));
    }

    #[test]
    fn modes_that_do_not_replace_text_return_nothing() {
        let mut store = store();
        assert_eq!(
            elide(&mut store, &"W".repeat(40), 20.0, TextOverflow::Clip),
            None
        );
        assert_eq!(
            elide(&mut store, &"W".repeat(40), 20.0, TextOverflow::Visible),
            None
        );
        assert_eq!(
            elide(&mut store, &"W".repeat(40), 20.0, TextOverflow::Fade),
            None
        );
    }
}
