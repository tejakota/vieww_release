//! Contrast scanning over a `vieww-widget` [`ColorScheme`]/[`ThemeData`].
//!
//! # Why this checks named pairs rather than every colour against every other
//!
//! A [`ColorScheme`] is not a bag of colours — it is a set of *roles*, each
//! with a documented partner it is meant to sit under text or an icon on:
//! `on_primary` is written on `primary`, never on `surface`. Checking every
//! combination would produce numbers for pairings nothing in the framework
//! ever draws, and would bury the pairings that matter under ones that do not
//! mean anything. So this module reads the same pairing `ColorScheme`'s own
//! field docs already state and checks exactly those.
//!
//! # What running this over the built-in schemes actually found
//!
//! [`ColorScheme::light`] and [`ColorScheme::dark`] cleared AA on every pair
//! from the start. [`ColorScheme::apple_light`] and [`ColorScheme::apple_dark`]
//! did **not**: white text on `systemBlue`, on `systemRed`/`systemGreen`, and
//! the hairline `outline` grey each landed under their threshold — seven
//! failing pairs across the two schemes.
//!
//! That was a genuine property of Apple's default palette rather than a bug in
//! this scanner, and for a while the tests below pinned the failures rather
//! than hiding them. Pinning them was the wrong resting place: these are
//! schemes the framework *ships*, so the measurement was documenting, in
//! precise detail, contrast that every application choosing the Apple look
//! inherited. Both schemes now carry Apple's own published increased-contrast
//! colours and pass. See `ColorScheme::apple_light`'s doc for the numbers and
//! the reasoning, including why the dark scheme's fix is a black label rather
//! than a darker fill.

use vieww_foundation::Color;
use vieww_widget::{ColorScheme, ThemeData};

use crate::contrast::{contrast_ratio, passes, WcagLevel};

/// The minimum non-text contrast WCAG 1.4.11 asks for on a UI component's
/// outline against the surface it sits on.
///
/// Distinct from the 4.5:1 text threshold [`passes`] applies at
/// [`WcagLevel::Aa`]: an outline is a shape, not a glyph, and WCAG gives shapes
/// a lower bar than text — 3:1 is the number Success Criterion 1.4.11 states
/// for "Non-text Contrast".
pub const MIN_NON_TEXT_CONTRAST: f32 = 3.0;

/// One named foreground/background pair that failed its required contrast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeContrastFailure {
    /// The pair's name, as `ColorScheme`'s own field docs name it — e.g.
    /// `"on_primary / primary"`.
    pub pair: &'static str,
    /// The role drawn on top — text or an icon.
    pub foreground: Color,
    /// The role it was drawn on.
    pub background: Color,
    /// What [`contrast_ratio`] actually measured.
    pub ratio: f32,
    /// What the pair needed to clear — 4.5 for a text pair at AA, 3.0 for the
    /// non-text outline pair. See [`MIN_NON_TEXT_CONTRAST`].
    pub required: f32,
}

/// Every text/background pair a scheme is judged on, and the one non-text
/// pair, in the order they are reported.
///
/// # Why `on_surface_variant` is judged against `surface_variant`, not `surface`
///
/// `ColorScheme::surface_variant`'s own doc calls it "a raised or recessed area
/// of the same background", and `on_surface_variant` is its label colour — a
/// chip's caption sits on the chip, not on the screen behind it. Judging it
/// against `surface` would answer a question nobody draws.
///
/// # What is deliberately not scanned
///
/// `outline` is checked against `surface` at the *non-text* 3:1 threshold
/// (WCAG 1.4.11), since it is a divider and a field border, never a glyph.
/// Nothing in `ColorScheme` pairs a colour with more than one background in
/// practice, so there is nothing else to add without inventing a pairing the
/// framework does not draw.
fn text_pairs(colors: &ColorScheme) -> [(&'static str, Color, Color); 5] {
    [
        ("on_primary / primary", colors.on_primary, colors.primary),
        ("on_surface / surface", colors.on_surface, colors.surface),
        (
            "on_surface_variant / surface_variant",
            colors.on_surface_variant,
            colors.surface_variant,
        ),
        ("on_error / error", colors.on_error, colors.error),
        ("on_success / success", colors.on_success, colors.success),
    ]
}

/// The result of scanning a scheme's named pairs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeAuditReport {
    /// Every pair that failed, empty if none did.
    pub failures: Vec<ThemeContrastFailure>,
}

impl ThemeAuditReport {
    /// `true` if every checked pair cleared its required contrast.
    #[must_use]
    pub fn passes(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Scan every documented foreground/background pair in `colors` and report
/// which ones fail AA normal-text contrast (4.5:1), plus the `outline` pair at
/// the non-text 3:1 threshold.
///
/// `large_text` is not a parameter here: a `ColorScheme`'s roles are used at
/// whatever size the widget drawing them chooses, so there is no one text size
/// to judge the *scheme* at. Checking against the stricter normal-text
/// threshold is the conservative choice — a pair that clears 4.5:1 clears
/// large text's 3.0:1 automatically, so nothing that would look fine gets
/// flagged, and a caller who knows a specific label is large text can call
/// [`contrast_ratio`] and [`passes`] directly for that pair.
#[must_use]
pub fn audit_color_scheme(colors: &ColorScheme) -> ThemeAuditReport {
    let mut failures = Vec::new();
    for (pair, foreground, background) in text_pairs(colors) {
        let ratio = contrast_ratio(foreground, background);
        if !passes(ratio, WcagLevel::Aa, false) {
            failures.push(ThemeContrastFailure {
                pair,
                foreground,
                background,
                ratio,
                required: 4.5,
            });
        }
    }
    let outline_ratio = contrast_ratio(colors.outline, colors.surface);
    if outline_ratio < MIN_NON_TEXT_CONTRAST {
        failures.push(ThemeContrastFailure {
            pair: "outline / surface",
            foreground: colors.outline,
            background: colors.surface,
            ratio: outline_ratio,
            required: MIN_NON_TEXT_CONTRAST,
        });
    }
    ThemeAuditReport { failures }
}

/// [`audit_color_scheme`] over a whole theme's [`ThemeData::colors`].
#[must_use]
pub fn audit_theme(theme: &ThemeData) -> ThemeAuditReport {
    audit_color_scheme(&theme.colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The framework's own built-in light and dark schemes, both checked
    /// honestly rather than tuned to pass: if either had a real failure this
    /// test would show it, not hide it.
    #[test]
    fn the_built_in_light_and_dark_schemes_pass_every_pair() {
        for (name, scheme) in [
            ("light", ColorScheme::light()),
            ("dark", ColorScheme::dark()),
        ] {
            let report = audit_color_scheme(&scheme);
            assert!(
                report.passes(),
                "{name} scheme failed: {:#?}",
                report.failures
            );
        }
    }

    /// Apple's built-in schemes, held to the same bar as vieww's own.
    ///
    /// # What this test used to assert, and why it changed
    ///
    /// It used to pin a *list of failures* — four pairs in `apple_light`,
    /// three in `apple_dark` — on the reasoning that Apple's default system
    /// colours are tuned for how a filled iOS control looks rather than for
    /// WCAG's white-on-solid-fill arithmetic, and that recording the real
    /// numbers was more honest than a looser assertion.
    ///
    /// The first half of that was true and the conclusion did not follow. A
    /// scheme this framework *ships as a default* is not an observation about
    /// Apple's palette; it is the contrast an application gets for choosing
    /// nothing. Pinning the failure documented the problem exactly and left
    /// every application built on the Apple schemes below AA.
    ///
    /// Both schemes now use Apple's own published increased-contrast variants
    /// — see `ColorScheme::apple_light` and `apple_dark` for which colours
    /// moved and why the dark scheme's fix is a black label rather than a
    /// darker fill. So this asserts a clean pass, and it is a real one: the
    /// arithmetic below is untouched and no threshold was relaxed to get it.
    #[test]
    fn the_apple_schemes_pass_every_pair() {
        for (name, scheme) in [
            ("apple_light", ColorScheme::apple_light()),
            ("apple_dark", ColorScheme::apple_dark()),
        ] {
            let report = audit_color_scheme(&scheme);
            assert!(
                report.passes(),
                "{name} scheme failed: {:#?}",
                report.failures
            );
        }
    }

    /// The margin each Apple pair clears its threshold by.
    ///
    /// Pinned, because "passes" is a cliff: a colour nudged later for looks
    /// could land at 4.51:1, keep the test above green, and be one rounding
    /// away from failing. These are the ratios measured when the schemes were
    /// retuned, and a change that moves one materially has to come here and
    /// say so.
    #[test]
    fn the_apple_pairs_clear_their_thresholds_with_room() {
        let light = ColorScheme::apple_light();
        assert!(contrast_ratio(light.on_primary, light.primary) > 7.0);
        assert!(contrast_ratio(light.on_error, light.error) > 5.0);
        assert!(contrast_ratio(light.on_success, light.success) > 5.0);
        assert!(contrast_ratio(light.outline, light.surface) > 3.0);

        let dark = ColorScheme::apple_dark();
        assert!(contrast_ratio(dark.on_primary, dark.primary) > 7.0);
        assert!(contrast_ratio(dark.on_error, dark.error) > 7.0);
        assert!(contrast_ratio(dark.on_success, dark.success) > 7.0);
        assert!(contrast_ratio(dark.outline, dark.surface) > 3.0);
    }

    /// A synthetic scheme with one deliberately bad pair: light grey text on a
    /// near-white surface, the classic low-contrast mistake. Every other pair
    /// is copied from the real light scheme so this is a targeted defect, not
    /// a wholesale bad palette.
    #[test]
    fn a_light_gray_on_white_pair_is_caught() {
        let mut colors = ColorScheme::light();
        colors.on_surface = Color::hex(0xCC_CCCC);
        colors.surface = Color::WHITE;

        let report = audit_color_scheme(&colors);
        assert!(!report.passes());
        let failure = report
            .failures
            .iter()
            .find(|f| f.pair == "on_surface / surface")
            .expect("the bad pair should have been reported");
        assert!(failure.ratio < 4.5, "{}", failure.ratio);
        assert_eq!(failure.required, 4.5);
    }

    /// A scheme where nothing else is wrong reports zero failures — the
    /// no-false-positive half of the same claim.
    #[test]
    fn an_untouched_scheme_reports_nothing_wrong_with_the_pair_that_was_not_touched() {
        let colors = ColorScheme::light();
        let report = audit_color_scheme(&colors);
        assert!(report.passes(), "{:#?}", report.failures);
    }

    #[test]
    fn audit_theme_delegates_to_the_schemes_colors() {
        let theme = ThemeData::light();
        let from_theme = audit_theme(&theme);
        let from_scheme = audit_color_scheme(&theme.colors);
        assert_eq!(from_theme, from_scheme);
    }

    /// An outline driven low enough to fail the non-text 3:1 bar specifically
    /// — distinct from the 4.5:1 text pairs, and reported with its own
    /// `required` value so a caller can tell which threshold was in play.
    #[test]
    fn a_washed_out_outline_fails_the_non_text_threshold_specifically() {
        let mut colors = ColorScheme::light();
        colors.outline = colors.surface.lerp(Color::hex(0xF0_F0F0), 0.5);

        let report = audit_color_scheme(&colors);
        let failure = report
            .failures
            .iter()
            .find(|f| f.pair == "outline / surface")
            .expect("a washed-out outline should fail the non-text check");
        assert_eq!(failure.required, MIN_NON_TEXT_CONTRAST);
        assert!(failure.ratio < MIN_NON_TEXT_CONTRAST);
    }
}
