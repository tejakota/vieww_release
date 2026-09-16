//! Interpolation: what "half way between these two values" means, per type.
//!
//! # Why a trait and not a function per type
//!
//! An [`AnimationController`](crate::AnimationController) produces one number.
//! Everything an animation actually changes — a colour, an offset, a set of
//! insets — is some other type, and the mapping from the one to the other is the
//! [`Tween`]. Making that a trait is what lets the implicit animation widgets
//! animate a heterogeneous bag of properties with one code path instead of one
//! branch per property.
//!
//! # Unclamped, deliberately
//!
//! [`Tween::at`] extrapolates outside `0..=1`, because the values it is fed do
//! go outside it: a spring settling into place overshoots by design, and a tween
//! that clamped would flatten exactly the part of the motion the spring was
//! chosen for. The `Lerp` impls are responsible for staying *representable* under
//! extrapolation — a colour channel saturates rather than wrapping, and a size
//! stops at zero rather than going negative.

use vieww_foundation::{Alignment, Color, EdgeInsets, Offset, Rect, Size};

/// A value that can be interpolated with another of its own type.
///
/// # `Clone`, not `Copy`
///
/// It was `Copy`, which quietly excluded everything that owns anything — a
/// `String` being typed out a character at a time, a `Vec` of points, a
/// [`Path`](vieww_foundation::Path) morphing into another. None of those are
/// exotic and none of them could be animated at all.
///
/// `Clone` costs an allocation per frame for the types that need one and
/// **nothing** for the types that do not: a `Copy` type's `clone` is a move, and
/// every impl below is unchanged. The bound was the only thing in the way.
pub trait Lerp: Clone {
    /// This value at `t == 0`, `other` at `t == 1`, and the straight line
    /// between them in between — extended past both ends for `t` outside
    /// `0..=1`.
    #[must_use]
    fn lerp(self, other: Self, t: f32) -> Self;
}

/// The scalar case, and the one every other impl is built out of.
impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        // `self + (other - self) * t` rather than `self * (1 - t) + other * t`:
        // the former is exact at `t == 0`, which is what stops an animation
        // that never started from twitching by a rounding error.
        self + (other - self) * t
    }
}

impl Lerp for Offset {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.dx.lerp(other.dx, t), self.dy.lerp(other.dy, t))
    }
}

impl Lerp for Size {
    fn lerp(self, other: Self, t: f32) -> Self {
        // Clamped at zero: a size is a measurement, and an overshooting spring
        // asking for a negative one would put a render object into constraints
        // it has no defined behaviour for.
        Self::new(
            self.width.lerp(other.width, t).max(0.0),
            self.height.lerp(other.height, t).max(0.0),
        )
    }
}

impl Lerp for Rect {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.left.lerp(other.left, t),
            self.top.lerp(other.top, t),
            self.right.lerp(other.right, t),
            self.bottom.lerp(other.bottom, t),
        )
    }
}

impl Lerp for EdgeInsets {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self::only(
            self.left.lerp(other.left, t),
            self.top.lerp(other.top, t),
            self.right.lerp(other.right, t),
            self.bottom.lerp(other.bottom, t),
        )
    }
}

impl Lerp for Alignment {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(self.x.lerp(other.x, t), self.y.lerp(other.y, t))
    }
}

impl Lerp for Color {
    /// Channel-wise in sRGB, saturating at the ends of the byte range.
    ///
    /// Not gamma-correct — a mid-point between two saturated colours passes
    /// through a darker place than a physicist would like. It is what CSS and
    /// every UI toolkit in wide use do, and matching them matters
    /// more here than matching the physics: a designer's gradient stops are
    /// chosen against this behaviour.
    fn lerp(self, other: Self, t: f32) -> Self {
        fn channel(from: u8, to: u8, t: f32) -> u8 {
            let value = f32::from(from).lerp(f32::from(to), t);
            // Saturating rather than wrapping: an overshoot past white must look
            // like white, not like black.
            value.round().clamp(0.0, 255.0) as u8
        }
        Self::rgba(
            channel(self.r, other.r, t),
            channel(self.g, other.g, t),
            channel(self.b, other.b, t),
            channel(self.a, other.a, t),
        )
    }
}

/// A pair of values and the line between them.
///
/// ```
/// use vieww_animation::{Curve, Lerp, Tween};
/// use vieww_foundation::Color;
///
/// let fade = Tween::new(Color::RED, Color::BLUE);
/// assert_eq!(fade.at(0.0), Color::RED);
/// assert_eq!(fade.at(1.0), Color::BLUE);
///
/// // Curves compose by transforming the parameter, not the endpoints.
/// let eased = fade.at(Curve::EASE_IN_OUT.transform(0.25));
/// assert!(eased.r > 128, "a quarter of the way through an ease-in-out is early");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tween<T> {
    /// The value at `t == 0`.
    pub begin: T,
    /// The value at `t == 1`.
    pub end: T,
}

impl<T: Lerp> Tween<T> {
    /// A tween from `begin` to `end`.
    #[must_use]
    pub const fn new(begin: T, end: T) -> Self {
        Self { begin, end }
    }

    /// A tween that does not move. For a property that is animated in one branch
    /// of a widget and static in another.
    ///
    /// Not `const` since [`Lerp`] widened to `Clone`: one value has to become
    /// two, and `Clone::clone` cannot run at compile time. `new` still is.
    #[must_use]
    pub fn constant(value: T) -> Self {
        Self {
            begin: value.clone(),
            end: value,
        }
    }

    /// The value at `t`, extrapolated if `t` is outside `0..=1`.
    ///
    /// Both ends are cloned, because `lerp` consumes them and this borrows. For
    /// a `Copy` type that is a move and costs nothing; for an owning one it is
    /// the allocation that made the type animatable in the first place.
    #[must_use]
    pub fn at(&self, t: f32) -> T {
        self.begin.clone().lerp(self.end.clone(), t)
    }

    /// The same tween, run the other way.
    #[must_use]
    pub fn reversed(&self) -> Self {
        Self {
            begin: self.end.clone(),
            end: self.begin.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tween_hits_its_endpoints_exactly() {
        let tween = Tween::new(10.0_f32, 20.0);
        assert_eq!(tween.at(0.0), 10.0);
        assert_eq!(tween.at(1.0), 20.0);
        assert_eq!(tween.at(0.5), 15.0);
    }

    #[test]
    fn a_tween_extrapolates_because_springs_overshoot() {
        let tween = Tween::new(0.0_f32, 100.0);
        assert!(
            (tween.at(1.2) - 120.0).abs() < 1e-3,
            "clamping here would flatten exactly the part of a spring's motion \
             it was chosen for: {}",
            tween.at(1.2)
        );
        assert!((tween.at(-0.1) + 10.0).abs() < 1e-3);
    }

    /// A string revealed a character at a time — the classic thing `Copy` shut
    /// out, and the reason the bound widened.
    #[derive(Debug, Clone, PartialEq)]
    struct Typed(String);

    impl Lerp for Typed {
        fn lerp(self, other: Self, t: f32) -> Self {
            // The endpoints are the values themselves, not a character count
            // that happens to reach them — the same rule `f32::lerp` follows,
            // and what makes `reversed().at(0.0)` the string it started from.
            let t = t.clamp(0.0, 1.0);
            if t <= 0.0 {
                return self;
            }
            if t >= 1.0 {
                return other;
            }
            let chars: Vec<char> = other.0.chars().collect();
            let shown = (chars.len() as f32 * t).round() as usize;
            Self(chars[..shown.min(chars.len())].iter().collect())
        }
    }

    #[test]
    fn a_type_that_owns_something_can_be_tweened() {
        // **The gap this closes.** `Lerp: Copy` excluded every owning type —
        // a `String`, a `Vec`, a `Path` — so none of them could be animated at
        // all, however ordinary the animation.
        let tween = Tween::new(Typed(String::new()), Typed("hello".into()));

        assert_eq!(tween.at(0.0), Typed(String::new()));
        assert_eq!(tween.at(0.6), Typed("hel".into()));
        assert_eq!(tween.at(1.0), Typed("hello".into()));
    }

    #[test]
    fn a_tween_over_an_owning_type_is_still_usable_twice() {
        // `at` borrows and clones rather than consuming, so a tween outlives
        // being read — which is what an animation ticking every frame needs.
        let tween = Tween::new(Typed(String::new()), Typed("hi".into()));
        assert_eq!(tween.at(1.0), tween.at(1.0));
        assert_eq!(tween.reversed().at(0.0), Typed("hi".into()));
    }

    #[test]
    fn a_reversed_tween_swaps_its_ends() {
        let tween = Tween::new(2.0_f32, 8.0).reversed();
        assert_eq!(tween.at(0.0), 8.0);
        assert_eq!(tween.at(1.0), 2.0);
    }

    #[test]
    fn a_constant_tween_never_moves() {
        let tween = Tween::constant(Offset::new(3.0, 4.0));
        assert_eq!(tween.at(0.0), tween.at(1.0));
        assert_eq!(tween.at(2.5), Offset::new(3.0, 4.0));
    }

    #[test]
    fn colours_interpolate_per_channel_including_alpha() {
        let tween = Tween::new(Color::rgba(0, 0, 0, 0), Color::rgba(255, 100, 50, 255));
        let mid = tween.at(0.5);
        assert_eq!(mid, Color::rgba(128, 50, 25, 128));
    }

    #[test]
    fn a_colour_pushed_past_its_endpoint_saturates_rather_than_wrapping() {
        let tween = Tween::new(Color::rgb(200, 200, 200), Color::rgb(255, 255, 255));
        let overshoot = tween.at(3.0);
        assert_eq!(
            overshoot,
            Color::WHITE,
            "wrapping a channel would make an overshoot past white flash black"
        );
        assert_eq!(tween.at(-3.0), Color::rgb(35, 35, 35));
    }

    #[test]
    fn a_size_never_goes_negative_under_an_overshoot() {
        let tween = Tween::new(Size::new(100.0, 50.0), Size::new(40.0, 5.0));
        assert_eq!(
            tween.at(1.5),
            Size::new(10.0, 0.0),
            "the width still has room to shrink into; the height does not, and \
             a negative one would put a render object into constraints it has \
             no defined behaviour for"
        );
    }

    #[test]
    fn geometry_interpolates_component_wise() {
        assert_eq!(
            Tween::new(Offset::new(0.0, 10.0), Offset::new(10.0, 0.0)).at(0.5),
            Offset::new(5.0, 5.0)
        );
        assert_eq!(
            Tween::new(EdgeInsets::all(0.0), EdgeInsets::all(20.0)).at(0.25),
            EdgeInsets::all(5.0)
        );
        assert_eq!(
            Tween::new(Alignment::TOP_LEFT, Alignment::BOTTOM_RIGHT).at(0.5),
            Alignment::CENTER
        );
        assert_eq!(
            Tween::new(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Rect::new(10.0, 10.0, 20.0, 20.0)
            )
            .at(0.5),
            Rect::new(5.0, 5.0, 15.0, 15.0)
        );
    }

    #[test]
    fn interpolation_is_exact_at_the_start_even_with_awkward_numbers() {
        // `a*(1-t) + b*t` is not exactly `a` at `t == 0` for every pair of
        // floats, and an animation that has not started must not move a pixel.
        let tween = Tween::new(0.1_f32 + 0.2, 1e9);
        assert_eq!(tween.at(0.0), 0.1_f32 + 0.2);
    }
}
