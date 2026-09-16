use std::fmt;

use crate::{Offset, Size, TextDirection};

/// A point within a box, expressed in a `-1.0 ..= 1.0` coordinate space where
/// `(-1, -1)` is the top-left corner, `(0, 0)` the centre and `(1, 1)` the
/// bottom-right.
///
/// The fractional space is what makes an alignment reusable across boxes of
/// different sizes — the same `Alignment` positions a 10px child and a 400px
/// child correctly without either knowing the other's size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Alignment {
    pub x: f32,
    pub y: f32,
}

impl Alignment {
    pub const TOP_LEFT: Self = Self::new(-1.0, -1.0);
    pub const TOP_CENTER: Self = Self::new(0.0, -1.0);
    pub const TOP_RIGHT: Self = Self::new(1.0, -1.0);
    pub const CENTER_LEFT: Self = Self::new(-1.0, 0.0);
    pub const CENTER: Self = Self::new(0.0, 0.0);
    pub const CENTER_RIGHT: Self = Self::new(1.0, 0.0);
    pub const BOTTOM_LEFT: Self = Self::new(-1.0, 1.0);
    pub const BOTTOM_CENTER: Self = Self::new(0.0, 1.0);
    pub const BOTTOM_RIGHT: Self = Self::new(1.0, 1.0);

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// The offset at which a `child`-sized box sits inside a `container`-sized
    /// box under this alignment.
    ///
    /// A child larger than its container yields a negative offset, i.e. it
    /// overflows symmetrically rather than being clamped — clipping is the
    /// parent's decision, not the alignment's.
    #[must_use]
    pub fn inscribe(self, child: Size, container: Size) -> Offset {
        let free_x = container.width - child.width;
        let free_y = container.height - child.height;
        Offset::new(free_x * (self.x + 1.0) / 2.0, free_y * (self.y + 1.0) / 2.0)
    }
}

/// An [`Alignment`] whose horizontal axis runs from the start of the reading
/// direction to its end, rather than from left to right.
///
/// `x` is `-1.0` at the leading edge and `+1.0` at the trailing one, so
/// [`Self::CENTER_START`] is the left edge in Latin and the right edge in
/// Arabic. `y` is unchanged, because vertical has no handedness — every script
/// this targets runs top to bottom.
///
/// Same trade as [`EdgeInsetsDirectional`](crate::EdgeInsetsDirectional), for
/// the same reason: it resolves once in the widget layer, and everything below
/// keeps taking a physical [`Alignment`] that needs no direction to interpret.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AlignmentDirectional {
    /// `-1.0` at the leading edge, `+1.0` at the trailing edge.
    pub x: f32,
    pub y: f32,
}

impl AlignmentDirectional {
    pub const TOP_START: Self = Self::new(-1.0, -1.0);
    pub const TOP_END: Self = Self::new(1.0, -1.0);
    pub const CENTER_START: Self = Self::new(-1.0, 0.0);
    pub const CENTER: Self = Self::new(0.0, 0.0);
    pub const CENTER_END: Self = Self::new(1.0, 0.0);
    pub const BOTTOM_START: Self = Self::new(-1.0, 1.0);
    pub const BOTTOM_END: Self = Self::new(1.0, 1.0);

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Where this lands on screen, once the direction is known.
    ///
    /// The flip is a negation rather than a swap of named constants, so it holds
    /// for the arbitrary values `new` admits — `x: -0.5` is a quarter of the way
    /// in from the leading edge in both directions, which is the property a
    /// caller interpolating an alignment depends on.
    #[must_use]
    pub const fn resolve(self, direction: TextDirection) -> Alignment {
        let x = match direction {
            TextDirection::Ltr => self.x,
            TextDirection::Rtl => -self.x,
        };
        Alignment { x, y: self.y }
    }
}

impl fmt::Display for AlignmentDirectional {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match (self.x, self.y) {
            (-1.0, -1.0) => "topStart",
            (1.0, -1.0) => "topEnd",
            (-1.0, 0.0) => "centerStart",
            (0.0, 0.0) => "center",
            (1.0, 0.0) => "centerEnd",
            (-1.0, 1.0) => "bottomStart",
            (1.0, 1.0) => "bottomEnd",
            _ => return write!(f, "AlignmentDirectional({}, {})", self.x, self.y),
        };
        f.write_str(name)
    }
}

impl fmt::Display for Alignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match (self.x, self.y) {
            (-1.0, -1.0) => "topLeft",
            (0.0, -1.0) => "topCenter",
            (1.0, -1.0) => "topRight",
            (-1.0, 0.0) => "centerLeft",
            (0.0, 0.0) => "center",
            (1.0, 0.0) => "centerRight",
            (-1.0, 1.0) => "bottomLeft",
            (0.0, 1.0) => "bottomCenter",
            (1.0, 1.0) => "bottomRight",
            _ => return write!(f, "Alignment({}, {})", self.x, self.y),
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER: Size = Size::new(100.0, 100.0);
    const CHILD: Size = Size::new(20.0, 20.0);

    #[test]
    fn corners_and_center_inscribe_where_expected() {
        assert_eq!(Alignment::TOP_LEFT.inscribe(CHILD, CONTAINER), Offset::ZERO);
        assert_eq!(
            Alignment::CENTER.inscribe(CHILD, CONTAINER),
            Offset::new(40.0, 40.0)
        );
        assert_eq!(
            Alignment::BOTTOM_RIGHT.inscribe(CHILD, CONTAINER),
            Offset::new(80.0, 80.0)
        );
    }

    #[test]
    fn oversized_child_overflows_symmetrically_rather_than_clamping() {
        let oversized = Size::new(140.0, 100.0);
        assert_eq!(
            Alignment::CENTER.inscribe(oversized, CONTAINER),
            Offset::new(-20.0, 0.0)
        );
    }
}

#[cfg(test)]
mod directional_tests {
    use super::*;

    const CONTAINER: Size = Size::new(100.0, 100.0);
    const CHILD: Size = Size::new(20.0, 20.0);

    #[test]
    fn the_start_edge_is_the_left_one_in_latin_and_the_right_one_in_arabic() {
        let start = AlignmentDirectional::CENTER_START;
        assert_eq!(start.resolve(TextDirection::Ltr), Alignment::CENTER_LEFT);
        assert_eq!(start.resolve(TextDirection::Rtl), Alignment::CENTER_RIGHT);
    }

    #[test]
    fn the_end_edge_mirrors_too() {
        let end = AlignmentDirectional::TOP_END;
        assert_eq!(end.resolve(TextDirection::Ltr), Alignment::TOP_RIGHT);
        assert_eq!(end.resolve(TextDirection::Rtl), Alignment::TOP_LEFT);
    }

    #[test]
    fn the_vertical_axis_never_moves() {
        // Every script this framework targets runs top to bottom, so a flip
        // that touched `y` would be a bug rather than a feature.
        for alignment in [
            AlignmentDirectional::TOP_START,
            AlignmentDirectional::BOTTOM_END,
            AlignmentDirectional::new(0.25, -0.75),
        ] {
            assert_eq!(alignment.resolve(TextDirection::Rtl).y, alignment.y);
        }
    }

    #[test]
    fn centre_is_the_same_point_in_both_directions() {
        assert_eq!(
            AlignmentDirectional::CENTER.resolve(TextDirection::Ltr),
            AlignmentDirectional::CENTER.resolve(TextDirection::Rtl)
        );
    }

    #[test]
    fn an_arbitrary_fraction_flips_rather_than_snapping_to_an_edge() {
        // The reason `resolve` negates instead of matching named constants: an
        // alignment being animated passes through values with no name.
        let quarter_in = AlignmentDirectional::new(-0.5, 0.0);
        assert_eq!(quarter_in.resolve(TextDirection::Ltr).x, -0.5);
        assert_eq!(quarter_in.resolve(TextDirection::Rtl).x, 0.5);
    }

    #[test]
    fn a_resolved_alignment_inscribes_a_mirrored_offset() {
        // End to end: the same directional alignment puts a child against
        // opposite screen edges, and both are flush.
        let ltr = AlignmentDirectional::CENTER_START
            .resolve(TextDirection::Ltr)
            .inscribe(CHILD, CONTAINER);
        let rtl = AlignmentDirectional::CENTER_START
            .resolve(TextDirection::Rtl)
            .inscribe(CHILD, CONTAINER);
        assert_eq!(ltr.dx, 0.0, "flush against the left in LTR");
        assert_eq!(rtl.dx, 80.0, "flush against the right in RTL");
        assert_eq!(ltr.dy, rtl.dy, "the vertical placement is untouched");
    }
}
