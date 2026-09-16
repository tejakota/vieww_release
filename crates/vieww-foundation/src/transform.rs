use std::fmt;

use crate::{Offset, Rect};

/// A 2D affine transform.
///
/// Stored as the six meaningful entries of a 3x3 matrix whose last row is
/// always `[0, 0, 1]`:
///
/// ```text
/// | a  c  tx |
/// | b  d  ty |
/// | 0  0  1  |
/// ```
///
/// Affine is enough for everything the paint layer does — translate, scale,
/// rotate, skew, and any composition of them. Perspective would need the full
/// 3x3 and is deliberately out of scope: it would make hit testing
/// non-invertible in the general case, and nothing in the widget layer asks
/// for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Transform {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    #[must_use]
    pub const fn new(a: f32, b: f32, c: f32, d: f32, tx: f32, ty: f32) -> Self {
        Self { a, b, c, d, tx, ty }
    }

    /// A pure translation — by far the most common transform in a UI, since
    /// every child placement is one.
    #[must_use]
    pub const fn translate(offset: Offset) -> Self {
        Self::new(1.0, 0.0, 0.0, 1.0, offset.dx, offset.dy)
    }

    /// A scale about the origin.
    #[must_use]
    pub const fn scale(sx: f32, sy: f32) -> Self {
        Self::new(sx, 0.0, 0.0, sy, 0.0, 0.0)
    }

    /// A rotation about the origin, in radians, clockwise in screen
    /// coordinates (y grows downward).
    #[must_use]
    pub fn rotate(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self::new(cos, sin, -sin, cos, 0.0, 0.0)
    }

    /// `self` followed by `other`.
    ///
    /// Order matters and is easy to get backwards: the result applies `self`
    /// first. `a.then(b)` transforms a point as `b(a(point))`.
    #[must_use]
    pub fn then(self, other: Self) -> Self {
        Self::new(
            other.a * self.a + other.c * self.b,
            other.b * self.a + other.d * self.b,
            other.a * self.c + other.c * self.d,
            other.b * self.c + other.d * self.d,
            other.a * self.tx + other.c * self.ty + other.tx,
            other.b * self.tx + other.d * self.ty + other.ty,
        )
    }

    /// Rotate by `radians` about `center` rather than about the origin.
    ///
    /// Spelled out, this is `translate(-center).then(rotate).then(translate(center))`,
    /// and the order of those three is the thing everybody gets backwards —
    /// including, on its first outing, `examples/vector-probe`, whose spokes
    /// scattered across three neighbouring panels rather than fanning around a
    /// hub. Getting it wrong does not fail; it draws somewhere else.
    #[must_use]
    pub fn rotate_around(center: Offset, radians: f32) -> Self {
        Self::translate(Offset::new(-center.dx, -center.dy))
            .then(Self::rotate(radians))
            .then(Self::translate(center))
    }

    /// Scale by `sx`/`sy` about `center` rather than about the origin.
    ///
    /// The same trap as [`rotate_around`](Self::rotate_around), and the one a
    /// press-scale animation walks into: a control scaled about the origin
    /// slides towards the top-left as it shrinks.
    #[must_use]
    pub fn scale_around(center: Offset, sx: f32, sy: f32) -> Self {
        Self::translate(Offset::new(-center.dx, -center.dy))
            .then(Self::scale(sx, sy))
            .then(Self::translate(center))
    }

    /// Apply this transform to a point.
    #[must_use]
    pub fn apply(self, point: Offset) -> Offset {
        Offset::new(
            self.a * point.dx + self.c * point.dy + self.tx,
            self.b * point.dx + self.d * point.dy + self.ty,
        )
    }

    /// The axis-aligned bounding box of `rect` after transforming.
    ///
    /// All four corners are transformed and re-bounded, because under rotation
    /// or skew the transformed rectangle is not axis-aligned and taking only
    /// two corners would under-report the area — which, for damage tracking,
    /// means leaving stale pixels on screen.
    #[must_use]
    pub fn apply_rect(self, rect: Rect) -> Rect {
        let corners = [
            self.apply(Offset::new(rect.left, rect.top)),
            self.apply(Offset::new(rect.right, rect.top)),
            self.apply(Offset::new(rect.left, rect.bottom)),
            self.apply(Offset::new(rect.right, rect.bottom)),
        ];
        let mut left = f32::INFINITY;
        let mut top = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for corner in corners {
            left = left.min(corner.dx);
            top = top.min(corner.dy);
            right = right.max(corner.dx);
            bottom = bottom.max(corner.dy);
        }
        Rect::new(left, top, right, bottom)
    }

    /// The determinant. Zero means the transform collapses everything onto a
    /// line or a point, and cannot be inverted.
    #[must_use]
    pub fn determinant(self) -> f32 {
        self.a * self.d - self.b * self.c
    }

    /// The inverse, or `None` if this transform is degenerate.
    ///
    /// Hit testing needs this: a pointer arrives in screen coordinates and has
    /// to be pushed back through every transform on the way down.
    #[must_use]
    pub fn invert(self) -> Option<Self> {
        let det = self.determinant();
        if det.abs() < f32::EPSILON {
            return None;
        }
        let inv = 1.0 / det;
        Some(Self::new(
            self.d * inv,
            -self.b * inv,
            -self.c * inv,
            self.a * inv,
            (self.c * self.ty - self.d * self.tx) * inv,
            (self.b * self.tx - self.a * self.ty) * inv,
        ))
    }

    /// `true` if this is a translation and nothing else.
    ///
    /// Worth special-casing: almost every transform in a UI tree is one, and a
    /// translation-only chain lets damage rectangles stay axis-aligned.
    #[must_use]
    pub fn is_translation(self) -> bool {
        (self.a - 1.0).abs() < f32::EPSILON
            && self.b.abs() < f32::EPSILON
            && self.c.abs() < f32::EPSILON
            && (self.d - 1.0).abs() < f32::EPSILON
    }

    /// `true` if this transform changes nothing.
    #[must_use]
    pub fn is_identity(self) -> bool {
        self.is_translation() && self.tx.abs() < f32::EPSILON && self.ty.abs() < f32::EPSILON
    }

    /// The six entries in the column-major order most GPU APIs expect.
    #[must_use]
    pub const fn to_array(self) -> [f32; 6] {
        [self.a, self.b, self.c, self.d, self.tx, self.ty]
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl fmt::Display for Transform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_identity() {
            return f.write_str("identity");
        }
        if self.is_translation() {
            return write!(f, "translate({}, {})", self.tx, self.ty);
        }
        write!(
            f,
            "[{} {} {} {} {} {}]",
            self.a, self.b, self.c, self.d, self.tx, self.ty
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_leaves_points_alone() {
        let point = Offset::new(3.0, 4.0);
        assert_eq!(Transform::IDENTITY.apply(point), point);
        assert!(Transform::IDENTITY.is_identity());
    }

    #[test]
    fn composition_applies_self_first() {
        let move_right = Transform::translate(Offset::new(10.0, 0.0));
        let double = Transform::scale(2.0, 2.0);

        // Translate then scale: (1,0) -> (11,0) -> (22,0).
        assert_eq!(
            move_right.then(double).apply(Offset::new(1.0, 0.0)),
            Offset::new(22.0, 0.0)
        );
        // Scale then translate: (1,0) -> (2,0) -> (12,0).
        assert_eq!(
            double.then(move_right).apply(Offset::new(1.0, 0.0)),
            Offset::new(12.0, 0.0)
        );
    }

    #[test]
    fn inverting_round_trips_a_point() {
        let transform = Transform::translate(Offset::new(5.0, -3.0))
            .then(Transform::scale(2.0, 4.0))
            .then(Transform::rotate(0.7));
        let inverse = transform.invert().expect("not degenerate");

        let point = Offset::new(11.0, 17.0);
        let round_tripped = inverse.apply(transform.apply(point));
        assert!((round_tripped.dx - point.dx).abs() < 1e-3);
        assert!((round_tripped.dy - point.dy).abs() < 1e-3);
    }

    #[test]
    fn a_collapsing_transform_has_no_inverse() {
        assert!(Transform::scale(0.0, 1.0).invert().is_none());
    }

    #[test]
    fn rotating_a_rect_bounds_all_four_corners() {
        let rect = Rect::new(0.0, 0.0, 10.0, 2.0);
        let bounds = Transform::rotate(std::f32::consts::FRAC_PI_2).apply_rect(rect);

        // A quarter turn swaps the extents; taking only two corners would have
        // produced a rectangle of the wrong shape.
        assert!((bounds.width() - 2.0).abs() < 1e-4, "{bounds}");
        assert!((bounds.height() - 10.0).abs() < 1e-4, "{bounds}");
    }

    #[test]
    fn translations_are_recognised() {
        assert!(Transform::translate(Offset::new(4.0, 5.0)).is_translation());
        assert!(!Transform::scale(2.0, 2.0).is_translation());
    }
}

#[cfg(test)]
mod around_tests {
    use super::*;

    #[test]
    fn rotating_around_a_centre_leaves_the_centre_alone() {
        let centre = Offset::new(40.0, 25.0);
        let moved = Transform::rotate_around(centre, std::f32::consts::FRAC_PI_3).apply(centre);
        assert!((moved.dx - centre.dx).abs() < 0.001, "{moved:?}");
        assert!((moved.dy - centre.dy).abs() < 0.001, "{moved:?}");
    }

    #[test]
    fn a_quarter_turn_puts_the_point_above_beside() {
        let centre = Offset::new(10.0, 10.0);
        let above = Offset::new(10.0, 0.0);
        let moved = Transform::rotate_around(centre, std::f32::consts::FRAC_PI_2).apply(above);
        // Y-down, so a positive quarter turn takes "above" to "right of".
        assert!((moved.dx - 20.0).abs() < 0.001, "{moved:?}");
        assert!((moved.dy - 10.0).abs() < 0.001, "{moved:?}");
    }

    #[test]
    fn scaling_around_a_centre_leaves_the_centre_alone() {
        let centre = Offset::new(7.0, 3.0);
        let moved = Transform::scale_around(centre, 0.5, 0.5).apply(centre);
        assert!((moved.dx - centre.dx).abs() < 0.001);
        assert!((moved.dy - centre.dy).abs() < 0.001);
    }
}
