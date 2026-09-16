use std::fmt;
use std::ops::{Add, Sub};

/// A displacement in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Offset {
    pub dx: f32,
    pub dy: f32,
}

impl Offset {
    pub const ZERO: Self = Self { dx: 0.0, dy: 0.0 };

    #[must_use]
    pub const fn new(dx: f32, dy: f32) -> Self {
        Self { dx, dy }
    }

    /// Straight-line length.
    #[must_use]
    pub fn distance(self) -> f32 {
        self.dx.hypot(self.dy)
    }

    /// Length squared, for comparing against a threshold without a square root.
    ///
    /// Gesture recognition compares distances constantly — every move event
    /// against a slop radius — and never needs the distance itself.
    #[must_use]
    pub fn distance_squared(self) -> f32 {
        self.dx.mul_add(self.dx, self.dy * self.dy)
    }

    /// Scale both components.
    #[must_use]
    pub fn scale(self, factor: f32) -> Self {
        Self::new(self.dx * factor, self.dy * factor)
    }

    /// This offset's component along `axis`.
    #[must_use]
    pub fn along(self, axis: crate::Axis) -> f32 {
        match axis {
            crate::Axis::Horizontal => self.dx,
            crate::Axis::Vertical => self.dy,
        }
    }
}

impl Add for Offset {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.dx + rhs.dx, self.dy + rhs.dy)
    }
}

impl Sub for Offset {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.dx - rhs.dx, self.dy - rhs.dy)
    }
}

impl fmt::Display for Offset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", self.dx, self.dy)
    }
}

/// A width/height pair in logical pixels.
///
/// Either component may be [`f32::INFINITY`], which is how an unbounded axis is
/// spelled during layout.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };

    /// A size that is infinite on both axes.
    pub const INFINITE: Self = Self {
        width: f32::INFINITY,
        height: f32::INFINITY,
    };

    #[must_use]
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// A square of the given side length.
    #[must_use]
    pub const fn square(side: f32) -> Self {
        Self::new(side, side)
    }

    /// `true` if both components are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.width.is_finite() && self.height.is_finite()
    }

    /// The rectangle of this size positioned at `origin`.
    #[must_use]
    pub fn at(self, origin: Offset) -> Rect {
        Rect::from_origin_size(origin, self)
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} x {}", self.width, self.height)
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Rect {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };

    #[must_use]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[must_use]
    pub fn from_origin_size(origin: Offset, size: Size) -> Self {
        Self::new(
            origin.dx,
            origin.dy,
            origin.dx + size.width,
            origin.dy + size.height,
        )
    }

    #[must_use]
    pub fn width(self) -> f32 {
        self.right - self.left
    }

    #[must_use]
    pub fn height(self) -> f32 {
        self.bottom - self.top
    }

    #[must_use]
    pub fn size(self) -> Size {
        Size::new(self.width(), self.height())
    }

    #[must_use]
    pub fn origin(self) -> Offset {
        Offset::new(self.left, self.top)
    }

    /// `true` if `point` is inside this rectangle, left/top inclusive and
    /// right/bottom exclusive.
    ///
    /// The half-open convention matters for hit testing: adjacent rectangles
    /// must not both claim the pixel on their shared edge.
    #[must_use]
    pub fn contains(self, point: Offset) -> bool {
        point.dx >= self.left
            && point.dx < self.right
            && point.dy >= self.top
            && point.dy < self.bottom
    }

    /// This rectangle moved by `offset`.
    #[must_use]
    pub fn translate(self, offset: Offset) -> Self {
        Self::new(
            self.left + offset.dx,
            self.top + offset.dy,
            self.right + offset.dx,
            self.bottom + offset.dy,
        )
    }

    /// `true` if this rectangle encloses no area.
    ///
    /// Inverted rectangles (`right < left`) count as empty rather than as
    /// negative area, so they cannot contribute to a [`union`](Self::union).
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// The enclosed area, or zero if [empty](Self::is_empty).
    #[must_use]
    pub fn area(self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            self.width() * self.height()
        }
    }

    /// The smallest rectangle containing both.
    ///
    /// An empty rectangle contributes nothing, so folding `union` over a list
    /// starting from [`Rect::ZERO`] gives the list's bounds.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self::new(
            self.left.min(other.left),
            self.top.min(other.top),
            self.right.max(other.right),
            self.bottom.max(other.bottom),
        )
    }

    /// The overlap of the two, or [`Rect::ZERO`] if they are disjoint.
    ///
    /// Disjoint pairs collapse to `ZERO` rather than to an inverted rectangle,
    /// so the result is always usable without a further emptiness check.
    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let result = Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        if result.is_empty() {
            Self::ZERO
        } else {
            result
        }
    }

    /// `true` if the two share any area.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        !self.intersect(other).is_empty()
    }

    /// This rectangle grown by `delta` on every side.
    ///
    /// Damage tracking uses this: an antialiased edge tints pixels a fraction
    /// outside the geometric bounds, so a repaint region has to be a little
    /// larger than the shape that marked it pending.
    #[must_use]
    pub fn inflate(self, delta: f32) -> Self {
        Self::new(
            self.left - delta,
            self.top - delta,
            self.right + delta,
            self.bottom + delta,
        )
    }

    /// The smallest rectangle on the integer pixel grid that contains this one.
    ///
    /// Repaint regions must land on whole pixels — a scissor rectangle cannot
    /// be half a pixel wide, and rounding inward would leave a stale seam.
    #[must_use]
    pub fn round_out(self) -> Self {
        if self.is_empty() {
            return Self::ZERO;
        }
        Self::new(
            self.left.floor(),
            self.top.floor(),
            self.right.ceil(),
            self.bottom.ceil(),
        )
    }
}

impl fmt::Display for Rect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Rect({}, {}, {}, {})",
            self.left, self.top, self.right, self.bottom
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_half_open() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(rect.contains(Offset::new(0.0, 0.0)));
        assert!(rect.contains(Offset::new(9.99, 9.99)));
        assert!(!rect.contains(Offset::new(10.0, 5.0)));
        assert!(!rect.contains(Offset::new(5.0, 10.0)));
    }

    #[test]
    fn adjacent_rects_do_not_both_claim_the_shared_edge() {
        let left = Rect::new(0.0, 0.0, 10.0, 10.0);
        let right = Rect::new(10.0, 0.0, 20.0, 10.0);
        let seam = Offset::new(10.0, 5.0);
        assert!(!left.contains(seam));
        assert!(right.contains(seam));
    }

    #[test]
    fn union_ignores_empty_rects_so_it_can_be_folded_from_zero() {
        let a = Rect::new(10.0, 10.0, 20.0, 20.0);
        assert_eq!(Rect::ZERO.union(a), a);
        assert_eq!(a.union(Rect::ZERO), a);
        assert_eq!(
            a.union(Rect::new(0.0, 15.0, 5.0, 40.0)),
            Rect::new(0.0, 10.0, 20.0, 40.0)
        );
    }

    #[test]
    fn disjoint_rects_intersect_to_zero_not_to_an_inverted_rect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(50.0, 50.0, 60.0, 60.0);
        assert_eq!(a.intersect(b), Rect::ZERO);
        assert!(!a.overlaps(b));
    }

    #[test]
    fn touching_rects_do_not_overlap() {
        let left = Rect::new(0.0, 0.0, 10.0, 10.0);
        let right = Rect::new(10.0, 0.0, 20.0, 10.0);
        assert!(!left.overlaps(right), "a shared edge encloses no area");
    }

    #[test]
    fn round_out_never_shrinks_a_rect() {
        let rect = Rect::new(0.3, 9.7, 10.2, 20.1);
        assert_eq!(rect.round_out(), Rect::new(0.0, 9.0, 11.0, 21.0));
    }

    #[test]
    fn area_of_an_inverted_rect_is_zero_rather_than_positive() {
        // Both extents are flipped, so a naive width * height would give +4.
        assert_eq!(Rect::new(10.0, 10.0, 8.0, 8.0).area(), 0.0);
    }

    #[test]
    fn rect_round_trips_through_origin_and_size() {
        let origin = Offset::new(3.0, 4.0);
        let size = Size::new(10.0, 20.0);
        let rect = Rect::from_origin_size(origin, size);
        assert_eq!(rect.origin(), origin);
        assert_eq!(rect.size(), size);
    }
}
