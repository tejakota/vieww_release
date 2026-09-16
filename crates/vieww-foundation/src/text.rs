//! Text styling, shared by the widget layer that describes it and the paint
//! layer that draws it.
//!
//! Lives in `foundation` rather than `vieww-widget` because the paint layer
//! needs it and must not depend on widgets — `docs/DESIGN.md` §7.
//!
//! Deliberately thin. A family, a size, a weight, an italic flag, a line box
//! and a tracking value is what a type scale has to carry and what a run has to
//! say. Fallback chains and OpenType feature settings are still the shaper's
//! business rather than this module's — a `TextStyle` that could turn on
//! discretionary ligatures would be a style nobody could compare cheaply.

use crate::Color;

/// How heavy a font is drawn.
///
/// `Hash` because a font weight is part of what identifies a shaped paragraph,
/// and the text shape cache keys on one. Without it that cache hashed the
/// weight by *formatting it into a `String`* — which is the sort of thing that
/// looks harmless in a derive list and turns up in a profile as fifteen hundred
/// allocations a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontWeight {
    Light,
    #[default]
    Regular,
    Medium,
    Bold,
}

impl FontWeight {
    /// The CSS-style numeric weight, which is what font files are indexed by.
    #[must_use]
    pub const fn value(self) -> u16 {
        match self {
            Self::Light => 300,
            Self::Regular => 400,
            Self::Medium => 500,
            Self::Bold => 700,
        }
    }
}

/// Which typeface a run is set in.
///
/// # Why a small enum and not a `String`
///
/// [`TextStyle`] is `Copy` and every one of its builders is a `const fn`, so a
/// type scale is a table of constants rather than a table of allocations. A
/// `String` family would end that, and the cost would be paid by every style in
/// every theme to serve the rare case of a family name discovered at runtime.
///
/// [`Named`](Self::Named) takes a `&'static str` instead, which covers the case
/// that actually occurs — an application registers a font and names it with a
/// literal. A name genuinely not known until runtime can be `String::leak`ed:
/// a handful of bytes, once, for a font that will be loaded for the life of the
/// process anyway.
///
/// # The three generic families
///
/// Resolved by the font database rather than by this enum, because what
/// `SansSerif` means is the platform's answer and on a headless build it is the
/// embedded face. A framework that hard-codes "Helvetica" is wrong on every
/// machine that has not got it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontFamily {
    /// The platform's default UI face. What text gets if nothing asks.
    #[default]
    SansSerif,
    Serif,
    /// A fixed-pitch face.
    ///
    /// The one that earns its place in a UI toolkit rather than a word
    /// processor: a column of figures that *changes* — a size, a countdown, a
    /// percentage — jitters horizontally on a proportional face, because `1` is
    /// narrower than `8`. Setting the figures monospace is the fix; doing it
    /// with weight and letter-spacing, which is what a framework without this
    /// forces, is not.
    Monospace,
    /// A family registered under this exact name.
    ///
    /// Falls back to [`SansSerif`](Self::SansSerif) if nothing by that name is
    /// loaded. A missing font is not worth refusing to draw over, and the
    /// substitution is visible.
    Named(&'static str),
}

/// How a run of text should be drawn.
///
/// Deliberately small, and `Copy`: a type scale is a table of `const` values,
/// which is what makes a theme cheap to pass around and impossible to mutate by
/// accident. Fallback chains and OpenType features are still the shaper's
/// business rather than this struct's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub color: Color,
    /// The typeface. See [`FontFamily`].
    pub family: FontFamily,
    pub size: f32,
    pub weight: FontWeight,
    pub italic: bool,
    /// The line box, as a multiple of [`size`](Self::size).
    ///
    /// A multiple rather than a pixel height, so that one number in a type scale
    /// keeps its proportions when the scale is resized — which is what a user
    /// enlarging text at the OS level does to every size at once.
    ///
    /// Below `1.0` lines overlap. That is allowed rather than clamped: tight
    /// display type is a real thing, and a toolkit that refuses it is one people
    /// work around with negative padding.
    pub line_height: f32,
    /// Extra space after every glyph, in logical pixels.
    ///
    /// Applied by the shaper, so it lands in the advances a caret walks and in
    /// the width a line wraps against — not added to a measurement afterwards,
    /// which is how spacing and hit testing get out of step.
    ///
    /// Negative tightens. Small capitals and all-caps labels are the cases that
    /// need it, which is why it is on the style rather than on the widget.
    pub letter_spacing: f32,
}

impl TextStyle {
    #[must_use]
    pub const fn new(size: f32) -> Self {
        Self {
            color: Color::BLACK,
            family: FontFamily::SansSerif,
            size,
            weight: FontWeight::Regular,
            italic: false,
            line_height: Self::NORMAL_LINE_HEIGHT,
            letter_spacing: 0.0,
        }
    }

    /// The line box every style starts with, as a multiple of the size.
    ///
    /// 1.2 is the long-standing typographic default and what most UI toolkits
    /// use; a font's own ascent plus descent is usually tighter than comfortable
    /// to read.
    pub const NORMAL_LINE_HEIGHT: f32 = 1.2;

    /// The line box this style asks for, in logical pixels.
    #[must_use]
    pub fn line_extent(self) -> f32 {
        self.size * self.line_height
    }

    /// The line box, as a multiple of the size.
    #[must_use]
    pub const fn line_height(mut self, multiple: f32) -> Self {
        self.line_height = multiple;
        self
    }

    /// Extra space after every glyph, in logical pixels. Negative tightens.
    #[must_use]
    pub const fn letter_spacing(mut self, spacing: f32) -> Self {
        self.letter_spacing = spacing;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// Set in this typeface. See [`FontFamily`].
    ///
    /// ```
    /// use vieww_foundation::{FontFamily, TextStyle};
    /// let figures = TextStyle::new(28.0).family(FontFamily::Monospace);
    /// ```
    #[must_use]
    pub const fn family(mut self, family: FontFamily) -> Self {
        self.family = family;
        self
    }

    /// Shorthand for [`FontFamily::Monospace`], which is the family a UI asks
    /// for most often after the default.
    #[must_use]
    pub const fn monospace(self) -> Self {
        self.family(FontFamily::Monospace)
    }

    #[must_use]
    pub const fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    #[must_use]
    pub const fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    #[must_use]
    pub const fn italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        Self::new(14.0)
    }
}

/// The base direction of a paragraph.
///
/// Bidirectional text is laid out per-run, and the direction of each run is
/// derived from the characters in it — so this is *not* "the direction of the
/// text". It is the direction to fall back on where the characters do not decide,
/// which matters more often than it sounds: a paragraph that is empty, or holds
/// only digits and punctuation, has no inherent direction, and neither does the
/// question of which edge [`TextAlign::Start`] means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDirection {
    /// Left-to-right: Latin, Cyrillic, Greek, most scripts.
    #[default]
    Ltr,
    /// Right-to-left: Arabic, Hebrew, Farsi.
    Rtl,
}

impl TextDirection {
    #[must_use]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// How lines are positioned within a paragraph's width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// The leading edge, whichever that is for the base
    /// [`TextDirection`] — left for LTR, right for RTL.
    ///
    /// The default, and almost always the right choice: hard-coding `Left` is how
    /// an interface ends up looking broken in Arabic.
    #[default]
    Start,
    /// The trailing edge.
    End,
    /// Centred.
    Center,
    /// The left edge regardless of direction. For content that is inherently
    /// left-aligned, such as code.
    Left,
    /// The right edge regardless of direction. For content that is inherently
    /// right-aligned, such as a column of figures.
    Right,
}

impl TextAlign {
    /// Resolve a direction-relative alignment into a concrete edge.
    ///
    /// Returns the fraction of the leftover space to place before the line: 0.0
    /// for left, 0.5 for centred, 1.0 for right.
    #[must_use]
    pub const fn leading_fraction(self, direction: TextDirection) -> f32 {
        match self {
            Self::Left => 0.0,
            Self::Right => 1.0,
            Self::Center => 0.5,
            Self::Start => match direction {
                TextDirection::Ltr => 0.0,
                TextDirection::Rtl => 1.0,
            },
            Self::End => match direction {
                TextDirection::Ltr => 1.0,
                TextDirection::Rtl => 0.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_end_follow_the_base_direction() {
        assert_eq!(TextAlign::Start.leading_fraction(TextDirection::Ltr), 0.0);
        assert_eq!(TextAlign::Start.leading_fraction(TextDirection::Rtl), 1.0);
        assert_eq!(TextAlign::End.leading_fraction(TextDirection::Ltr), 1.0);
        assert_eq!(TextAlign::End.leading_fraction(TextDirection::Rtl), 0.0);
    }

    #[test]
    fn left_and_right_ignore_the_base_direction() {
        for direction in [TextDirection::Ltr, TextDirection::Rtl] {
            assert_eq!(TextAlign::Left.leading_fraction(direction), 0.0);
            assert_eq!(TextAlign::Right.leading_fraction(direction), 1.0);
            assert_eq!(TextAlign::Center.leading_fraction(direction), 0.5);
        }
    }

    #[test]
    fn the_default_alignment_is_direction_relative() {
        assert_eq!(
            TextAlign::default(),
            TextAlign::Start,
            "hard-coding Left is how an interface breaks in Arabic"
        );
    }
}
