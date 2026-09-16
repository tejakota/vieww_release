//! WCAG 2.x contrast-ratio math.
//!
//! # Why this reuses `Color::to_linear` instead of its own sRGB decode
//!
//! The WCAG relative-luminance formula is defined over *linearised* channels,
//! and `vieww-foundation` already carries a correct, gamut-aware linearisation
//! in [`Color::to_linear`] — including the exact piecewise threshold between the
//! linear segment near black and the power curve above it, which is the part of
//! this formula every hand-rolled reimplementation gets subtly wrong at the
//! boundary. Reimplementing it here would be a second copy of that threshold,
//! liable to drift from the first one the day either is touched. This module
//! contributes exactly one thing on top: the WCAG luminance weights and the
//! contrast-ratio and pass/fail arithmetic built from them.
//!
//! `to_linear` also folds a wide-gamut colour into linear sRGB before decoding,
//! so a [`Color::p3`] value gets a luminance computed against the same
//! primaries a plain sRGB screen would show it in — which is the honest answer
//! for "how much of this can a person actually see", not a mathematically
//! separate one for each gamut.

use vieww_foundation::Color;

/// The WCAG relative luminance of a colour, `0.0` (black) to `1.0` (white).
///
/// `L = 0.2126*R + 0.7152*G + 0.0722*B`, over the **linear-light** channels
/// from [`Color::to_linear`] — not the encoded 0..=255 values, which is the
/// mistake this formula invites if the decode step is skipped. Alpha is not
/// part of the definition: a translucent colour has no luminance of its own
/// until it is composited onto something, and [`Color::over`] is the place to
/// do that compositing before calling this.
///
/// # Reference
///
/// WCAG 2.1 §1.4.3, "Relative Luminance" definition.
#[must_use]
pub fn relative_luminance(color: Color) -> f32 {
    let [r, g, b, _a] = color.to_linear();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// The WCAG contrast ratio between two colours, from `1.0` (identical) to
/// `21.0` (black against white).
///
/// `(L1 + 0.05) / (L2 + 0.05)`, where `L1` is the **lighter** of the two
/// relative luminances — which is what makes this symmetric in its arguments:
/// `contrast_ratio(a, b) == contrast_ratio(b, a)` always, because the formula
/// itself, not the call site, decides which one plays which role.
///
/// # What this does not account for
///
/// This is a ratio between two *opaque* colours. If either argument is
/// translucent, composite it over its real background with [`Color::over`]
/// first — this function does not do that for you, because it does not know
/// what the background is.
///
/// # Reference
///
/// WCAG 2.1 §1.4.3, "Contrast Ratio" definition.
#[must_use]
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (l1, l2) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Which published WCAG conformance level a contrast check is judged against.
///
/// Only the two levels that define a contrast threshold at all — WCAG's `A`
/// level has none. See [`passes`] for the actual numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WcagLevel {
    /// Success Criterion 1.4.3, Minimum Contrast.
    Aa,
    /// Success Criterion 1.4.6, Enhanced Contrast.
    Aaa,
}

/// Whether a measured contrast ratio clears the published WCAG threshold for
/// `level` at this text size.
///
/// # The four thresholds
///
/// | | normal text | large text |
/// |---|---|---|
/// | AA | 4.5:1 | 3.0:1 |
/// | AAA | 7.0:1 | 4.5:1 |
///
/// # What "large text" means, precisely, and why this function does not decide it
///
/// WCAG defines large text as **18pt (24px) or larger, or 14pt (~18.67px) or
/// larger and bold** — a rule stated in points because that is how the
/// standard is written, converted here at the usual 1pt = 1.333px. Whether a
/// given run of text meets that bar depends on its resolved font size *and*
/// weight *and* the platform's own point-to-pixel convention, none of which
/// this crate has visibility into: `vieww-accessibility` depends on
/// `vieww-foundation`, `vieww-render` and `vieww-widget`, and text sizing is
/// decided inside the render tree's text-layout path, several layers away from
/// a colour pair. Rather than approximate that boundary — and risk telling a
/// caller their 17.9px bold label is "large" when a screen's actual rendering
/// rounds it down — `large_text` is a plain `bool` the caller supplies, having
/// already resolved the real size and weight against the table above.
#[must_use]
pub fn passes(ratio: f32, level: WcagLevel, large_text: bool) -> bool {
    let threshold = match (level, large_text) {
        (WcagLevel::Aa, false) => 4.5,
        (WcagLevel::Aa, true) => 3.0,
        (WcagLevel::Aaa, false) => 7.0,
        (WcagLevel::Aaa, true) => 4.5,
    };
    ratio >= threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ends of the scale, hand-verifiable from the formula itself:
    /// identical colours have identical luminance, so the ratio collapses to
    /// `(L + 0.05) / (L + 0.05) = 1.0`; black (`L = 0`) against white
    /// (`L = 1`) gives `(1.0 + 0.05) / (0.0 + 0.05) = 21.0`.
    #[test]
    fn black_against_white_is_exactly_twenty_one_to_one() {
        let ratio = contrast_ratio(Color::BLACK, Color::WHITE);
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn identical_colours_have_a_ratio_of_exactly_one() {
        for color in [
            Color::BLACK,
            Color::WHITE,
            Color::RED,
            Color::hex(0x77_7680),
        ] {
            let ratio = contrast_ratio(color, color);
            assert!((ratio - 1.0).abs() < 1e-4, "{color}: got {ratio}");
        }
    }

    /// A hand-computed third point, independent of the implementation: mid-grey
    /// `#808080` against white.
    ///
    /// `0x80 / 255 = 0.501960...`, which is above the `0.04045` linear
    /// threshold, so `L = ((0.501960 + 0.055) / 1.055) ^ 2.4 = 0.215861...`
    /// (all three channels equal, so the WCAG weights sum to exactly that one
    /// value). White has `L = 1.0`. Ratio = `(1.0 + 0.05) / (0.215861 + 0.05)
    /// = 1.05 / 0.265861 = 3.9494...`.
    #[test]
    fn mid_gray_against_white_matches_a_hand_computed_value() {
        let ratio = contrast_ratio(Color::hex(0x80_8080), Color::WHITE);
        assert!((ratio - 3.9494).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn contrast_ratio_is_symmetric() {
        let pairs = [
            (Color::BLACK, Color::WHITE),
            (Color::hex(0x2C_62EA), Color::WHITE),
            (Color::hex(0x80_8080), Color::hex(0x40_4040)),
            (Color::RED, Color::GREEN),
        ];
        for (a, b) in pairs {
            let forward = contrast_ratio(a, b);
            let backward = contrast_ratio(b, a);
            assert!(
                (forward - backward).abs() < 1e-6,
                "{a} vs {b}: {forward} != {backward}"
            );
        }
    }

    #[test]
    fn aa_normal_text_threshold_is_four_point_five() {
        assert!(!passes(4.49, WcagLevel::Aa, false));
        assert!(passes(4.5, WcagLevel::Aa, false));
        assert!(passes(4.51, WcagLevel::Aa, false));
    }

    #[test]
    fn aa_large_text_threshold_is_three() {
        assert!(!passes(2.99, WcagLevel::Aa, true));
        assert!(passes(3.0, WcagLevel::Aa, true));
    }

    #[test]
    fn aaa_normal_text_threshold_is_seven() {
        assert!(!passes(6.99, WcagLevel::Aaa, false));
        assert!(passes(7.0, WcagLevel::Aaa, false));
    }

    #[test]
    fn aaa_large_text_threshold_is_four_point_five() {
        assert!(!passes(4.49, WcagLevel::Aaa, true));
        assert!(passes(4.5, WcagLevel::Aaa, true));
    }

    /// A ratio that clears AAA clears AA too, at the same text size — the
    /// thresholds are supposed to nest, and a bug that swapped two entries in
    /// the table would break exactly this.
    #[test]
    fn every_passing_aaa_ratio_also_passes_aa() {
        for large_text in [false, true] {
            for ratio in [3.0, 4.5, 7.0, 12.0, 21.0] {
                if passes(ratio, WcagLevel::Aaa, large_text) {
                    assert!(
                        passes(ratio, WcagLevel::Aa, large_text),
                        "ratio {ratio} (large_text={large_text}) passed AAA but not AA"
                    );
                }
            }
        }
    }
}
