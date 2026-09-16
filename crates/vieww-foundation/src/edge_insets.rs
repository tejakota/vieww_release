use std::fmt;

use crate::TextDirection;

/// Insets on the four sides of a box, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeInsets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl EdgeInsets {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    /// The same inset on all four sides.
    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    /// Horizontal insets on left/right, vertical on top/bottom.
    #[must_use]
    pub const fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }

    /// Only the sides given; the rest are zero.
    #[must_use]
    pub const fn only(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Total inset along the horizontal axis.
    #[must_use]
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    /// Total inset along the vertical axis.
    #[must_use]
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    /// `true` if every side is zero, in which case the owning render object can
    /// skip the inset entirely.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

/// Insets expressed against the *reading* direction rather than the screen.
///
/// `start` is the edge text begins at — left under [`TextDirection::Ltr`], right
/// under [`TextDirection::Rtl`] — and `end` is the one it runs towards. A label
/// padded 16 at the start and 8 at the end keeps that relationship in Arabic,
/// where [`EdgeInsets::only`] would mirror the layout and leave the padding
/// behind.
///
/// # Why this is a separate type rather than a flag on [`EdgeInsets`]
///
/// An `EdgeInsets` is consumed during layout, by render objects that have no
/// business knowing which way the user reads. Making the resolution lazy would
/// mean threading a [`TextDirection`] into every layout call to serve the small
/// minority of insets that are directional. Instead this resolves *once*, in the
/// widget layer where the ambient direction is already in hand, and everything
/// below the resolution stays physical and direction-free.
///
/// The consequence is deliberate and worth stating: **there is no way to read
/// `left` off this type.** [`Self::resolve`] is the only way out, so a caller
/// cannot accidentally treat a directional inset as a physical one — which is
/// the bug this type exists to make unwritable.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EdgeInsetsDirectional {
    /// The leading edge: left in LTR, right in RTL.
    pub start: f32,
    pub top: f32,
    /// The trailing edge: right in LTR, left in RTL.
    pub end: f32,
    pub bottom: f32,
}

impl EdgeInsetsDirectional {
    pub const ZERO: Self = Self {
        start: 0.0,
        top: 0.0,
        end: 0.0,
        bottom: 0.0,
    };

    /// The same inset on all four sides.
    ///
    /// Present for symmetry, and it resolves to [`EdgeInsets::all`] in both
    /// directions — a uniform inset has no handedness. Reach for the physical
    /// type when that is what you mean; this one is here so a call site that is
    /// directional everywhere else does not have to switch types for one field.
    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self {
            start: value,
            top: value,
            end: value,
            bottom: value,
        }
    }

    /// Insets along the reading axis on start/end, and vertically on top/bottom.
    #[must_use]
    pub const fn symmetric(along: f32, vertical: f32) -> Self {
        Self {
            start: along,
            top: vertical,
            end: along,
            bottom: vertical,
        }
    }

    /// Only the sides given; the rest are zero.
    #[must_use]
    pub const fn only(start: f32, top: f32, end: f32, bottom: f32) -> Self {
        Self {
            start,
            top,
            end,
            bottom,
        }
    }

    /// Which physical edges these land on, once the direction is known.
    #[must_use]
    pub const fn resolve(self, direction: TextDirection) -> EdgeInsets {
        let (left, right) = match direction {
            TextDirection::Ltr => (self.start, self.end),
            TextDirection::Rtl => (self.end, self.start),
        };
        EdgeInsets {
            left,
            top: self.top,
            right,
            bottom: self.bottom,
        }
    }

    /// Total inset along the reading axis.
    ///
    /// Direction-free on purpose: `start + end` is the same number whichever
    /// edge each lands on, and a caller sizing a box does not need to resolve to
    /// ask how much width the padding costs.
    #[must_use]
    pub fn along(self) -> f32 {
        self.start + self.end
    }

    /// Total inset along the vertical axis.
    #[must_use]
    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }

    /// `true` if every side is zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

impl fmt::Display for EdgeInsetsDirectional {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "EdgeInsetsDirectional(start: {}, top: {}, end: {}, bottom: {})",
            self.start, self.top, self.end, self.bottom
        )
    }
}

impl fmt::Display for EdgeInsets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.left == self.right && self.top == self.bottom {
            if self.left == self.top {
                return write!(f, "all({})", self.left);
            }
            return write!(f, "symmetric(h: {}, v: {})", self.left, self.top);
        }
        write!(
            f,
            "only(l: {}, t: {}, r: {}, b: {})",
            self.left, self.top, self.right, self.bottom
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_inset_is_on_the_left_when_reading_left_to_right() {
        let padding = EdgeInsetsDirectional::only(16.0, 4.0, 8.0, 2.0);
        let resolved = padding.resolve(TextDirection::Ltr);
        assert_eq!(resolved, EdgeInsets::only(16.0, 4.0, 8.0, 2.0));
    }

    #[test]
    fn the_same_inset_moves_to_the_right_in_arabic() {
        // The whole point of the type: the *relationship* to the text survives,
        // so a label indented from where reading begins stays indented.
        let padding = EdgeInsetsDirectional::only(16.0, 4.0, 8.0, 2.0);
        let resolved = padding.resolve(TextDirection::Rtl);
        assert_eq!(resolved.right, 16.0, "start is the right edge in RTL");
        assert_eq!(resolved.left, 8.0, "end is the left edge in RTL");
        assert_eq!(
            (resolved.top, resolved.bottom),
            (4.0, 2.0),
            "the vertical edges have no handedness and must not move"
        );
    }

    #[test]
    fn a_uniform_inset_resolves_the_same_in_both_directions() {
        let padding = EdgeInsetsDirectional::all(12.0);
        assert_eq!(
            padding.resolve(TextDirection::Ltr),
            padding.resolve(TextDirection::Rtl),
            "a uniform inset has no handedness to lose"
        );
        assert_eq!(padding.resolve(TextDirection::Ltr), EdgeInsets::all(12.0));
    }

    #[test]
    fn resolving_conserves_the_total_inset_along_each_axis() {
        // The property a layout depends on: flipping must move space, never
        // create or destroy it, or a screen would change width in Arabic.
        let padding = EdgeInsetsDirectional::only(16.0, 4.0, 8.0, 2.0);
        for direction in [TextDirection::Ltr, TextDirection::Rtl] {
            let resolved = padding.resolve(direction);
            assert_eq!(resolved.horizontal(), padding.along());
            assert_eq!(resolved.vertical(), padding.vertical());
        }
    }

    #[test]
    fn the_reading_axis_total_needs_no_direction_to_ask_for() {
        assert_eq!(EdgeInsetsDirectional::symmetric(10.0, 3.0).along(), 20.0);
        assert_eq!(EdgeInsetsDirectional::symmetric(10.0, 3.0).vertical(), 6.0);
    }

    #[test]
    fn zero_is_zero_whichever_way_it_is_read() {
        assert!(EdgeInsetsDirectional::ZERO.is_zero());
        assert_eq!(
            EdgeInsetsDirectional::ZERO.resolve(TextDirection::Rtl),
            EdgeInsets::ZERO
        );
        assert!(!EdgeInsetsDirectional::only(0.0, 0.0, 1.0, 0.0).is_zero());
    }
}
