//! Easing: the shape of the journey from 0 to 1.
//!
//! # Why a curve is not a nicety
//!
//! A linear animation is the one thing nothing in the physical world does. A
//! drawer that slides open at a constant speed and then stops dead reads as
//! *broken* rather than as fast, because nothing with mass moves like that. Every
//! platform's motion vocabulary — emphasised easing, the
//! `easeInOut` — is a cubic bézier for this reason, and matching those curves is
//! most of what makes an animation feel native rather than merely present.
//!
//! # Why this is hand-written rather than a crate
//!
//! `docs/ROADMAP.md` suggests the `keyframe` crate for the easing math. What is
//! actually needed here is one function — a unit cubic bézier solved for *y* at a
//! given *x* — and the interesting part of it is the root-find, which is thirty
//! lines and completely specified by the CSS `cubic-bezier()` definition. Taking a
//! dependency for that would buy a keyframe-sequence API this crate does not use
//! and would still leave [`Tween`](crate::Tween) to be written, since `keyframe`'s
//! interpolation does not know about [`Color`](vieww_foundation::Color) or
//! [`EdgeInsets`](vieww_foundation::EdgeInsets). See `docs/DESIGN.md` §15.

use std::fmt;

/// Iterations of Newton-Raphson before falling back to bisection.
///
/// Four is enough for a well-behaved curve to reach f32 precision; the fallback
/// exists for the flat spots where the derivative is near zero and Newton's
/// method steps off into nothing.
const NEWTON_STEPS: u32 = 4;
const NEWTON_MIN_SLOPE: f32 = 0.001;
const BISECTION_STEPS: u32 = 24;

/// How a value's progress maps onto its animation's progress.
///
/// A curve is a function on the unit interval with `f(0) == 0` and `f(1) == 1`.
/// Everything between is where the character lives.
#[derive(Debug, Clone, Copy, Default)]
pub enum Curve {
    /// Constant speed. Correct for a spinner or a colour cycle, wrong for
    /// anything that starts or stops on screen.
    #[default]
    Linear,
    /// A cubic bézier with its endpoints pinned at `(0, 0)` and `(1, 1)`, given
    /// by its two control points — the same parameterisation as CSS's
    /// `cubic-bezier()` and this crate's `Cubic`.
    Cubic { x1: f32, y1: f32, x2: f32, y2: f32 },
    /// Any easing at all, as a function — see [`custom`](Self::custom).
    ///
    /// A stepped curve, a bounce, an elastic overshoot or a table lookup, none
    /// of which a cubic bézier can express. Construct it through `custom`
    /// rather than by literal: `flipped` is bookkeeping this type owns.
    Custom {
        /// Maps progress to eased progress.
        f: fn(f32) -> f32,
        /// Read the function backwards — see [`flipped`](Self::flipped).
        ///
        /// **Why a flag and not a composed function.** `flipped` has to return a
        /// `Curve`, and the flip of `f` is `|t| 1 - f(1 - t)` — a closure, which
        /// is not a `fn` pointer. Composing would need an allocation, and being
        /// allocation-free is the property this enum exists to protect. One
        /// `bool` buys the same thing for nothing.
        flipped: bool,
    },
}

impl Curve {
    /// Slow to start, slow to stop. The default for anything that moves both
    /// into and out of view.
    pub const EASE_IN_OUT: Self = Self::cubic(0.42, 0.0, 0.58, 1.0);
    /// Slow to start. For something entering — it has to accelerate from rest.
    pub const EASE_IN: Self = Self::cubic(0.42, 0.0, 1.0, 1.0);
    /// Slow to stop. For something leaving, or arriving at a rest position.
    pub const EASE_OUT: Self = Self::cubic(0.0, 0.0, 0.58, 1.0);
    /// CSS's `ease`, which is not symmetric: it starts gently and finishes long.
    pub const EASE: Self = Self::cubic(0.25, 0.1, 0.25, 1.0);
    /// The standard emphasised easing — leaves quickly, arrives slowly.
    pub const FAST_OUT_SLOW_IN: Self = Self::cubic(0.4, 0.0, 0.2, 1.0);

    /// A cubic bézier from its two control points.
    ///
    /// # Panics
    ///
    /// If either control point's *x* lies outside `0..=1`. Such a curve is not a
    /// function of *x* — it doubles back, so one progress value maps to several
    /// outputs and there is no answer to give. Const-evaluated for the
    /// associated constants above, so a bad one fails to compile.
    #[must_use]
    pub const fn cubic(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        assert!(
            x1 >= 0.0 && x1 <= 1.0 && x2 >= 0.0 && x2 <= 1.0,
            "a curve's control points must have x within 0..=1, or the curve \
             doubles back and is not a function of progress"
        );
        Self::Cubic { x1, y1, x2, y2 }
    }

    /// Any easing at all, from a function.
    ///
    /// For the curves a cubic bézier cannot express — a stepped curve, a bounce,
    /// an elastic overshoot, or anything sampled from a table.
    ///
    /// ```
    /// use vieww_animation::Curve;
    ///
    /// // Four discrete steps, which no bézier can do.
    /// let stepped = Curve::custom(|t: f32| (t * 4.0).floor() / 4.0);
    /// assert_eq!(stepped.transform(0.6), 0.5);
    /// assert_eq!(stepped.transform(1.0), 1.0);
    /// ```
    ///
    /// # The contract is yours to keep
    ///
    /// `f(0.0)` must be `0.0` and `f(1.0)` must be `1.0`. Neither is checked —
    /// checking would mean calling the function at construction, and a `const fn`
    /// cannot. A curve that breaks it makes an animation jump at one end.
    ///
    /// **It may leave the unit interval in between**, and that is the point of
    /// having it: an overshoot returning `1.2` is what a bounce *is*.
    /// [`Tween`](crate::Tween) extrapolates rather than clamping, so the
    /// overshoot survives all the way to the value.
    ///
    /// # Why `fn` and not a closure
    ///
    /// A bare function pointer keeps `Curve` `Copy`, comparable and
    /// allocation-free, which is why it is an enum rather than a trait object.
    /// The cost is that a curve cannot capture state — a table has to be a
    /// `static`, not a `Vec` — and that is the whole of what this does not cover.
    #[must_use]
    pub const fn custom(f: fn(f32) -> f32) -> Self {
        Self::Custom { f, flipped: false }
    }

    /// The eased value of `t`, which is clamped to `0..=1` first.
    ///
    /// Clamping rather than extrapolating: a bézier evaluated outside its
    /// interval says nothing useful, and the callers that legitimately go out of
    /// range — a spring overshooting its target — do not go through a curve at
    /// all. See [`AnimationController`](crate::AnimationController).
    #[must_use]
    pub fn transform(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Cubic { x1, y1, x2, y2 } => {
                // The endpoints are exact by definition; the solver would land
                // within a rounding error of them and look like a bug.
                if t == 0.0 || t == 1.0 {
                    return t;
                }
                bezier(y1, y2, solve_x(t, x1, x2))
            }
            // No endpoint short-circuit and no clamp of the result. The
            // short-circuit exists above because the *solver* lands a rounding
            // error away from an exact endpoint; a function has no solver, and
            // hiding a wrong endpoint would hide the implementor's bug. The
            // result is unclamped because an overshoot is the reason to be here.
            Self::Custom { f, flipped } => {
                if flipped {
                    1.0 - f(1.0 - t)
                } else {
                    f(t)
                }
            }
        }
    }

    /// The same curve read backwards: `flipped().transform(t) == 1 - transform(1 - t)`.
    ///
    /// What a reversing animation wants. An ease-in played backwards is not an
    /// ease-in — it is an ease-out, and reusing the forward curve is why a
    /// dismissal so often looks subtly wrong next to the entrance it undoes.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Linear => Self::Linear,
            Self::Cubic { x1, y1, x2, y2 } => Self::Cubic {
                x1: 1.0 - x2,
                y1: 1.0 - y2,
                x2: 1.0 - x1,
                y2: 1.0 - y1,
            },
            Self::Custom { f, flipped } => Self::Custom {
                f,
                flipped: !flipped,
            },
        }
    }
}

/// # Hand-written because a derive cannot compare a function pointer honestly
///
/// `#[derive(PartialEq)]` over [`Custom`](Self::Custom) compares `f` with `==`,
/// which rustc warns about (`unpredictable_function_pointer_comparisons`) and is
/// right to: the same function can have different addresses in different codegen
/// units, and distinct functions can share one after the linker merges
/// identical bodies.
///
/// [`std::ptr::fn_addr_eq`] is the sanctioned form of the same question. It
/// carries the same caveats — this is **identity, approximately** — and that is
/// all a `Curve` can offer, because comparing two easings by behaviour would
/// mean sampling them and two curves agreeing at every sample are still not the
/// same curve. `flipped` is compared normally: two references to one function,
/// read in opposite directions, are different curves.
impl PartialEq for Curve {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Linear, Self::Linear) => true,
            (
                Self::Cubic {
                    x1: ax1,
                    y1: ay1,
                    x2: ax2,
                    y2: ay2,
                },
                Self::Cubic {
                    x1: bx1,
                    y1: by1,
                    x2: bx2,
                    y2: by2,
                },
            ) => ax1 == bx1 && ay1 == by1 && ax2 == bx2 && ay2 == by2,
            (
                Self::Custom {
                    f: af,
                    flipped: aflipped,
                },
                Self::Custom {
                    f: bf,
                    flipped: bflipped,
                },
            ) => std::ptr::fn_addr_eq(*af, *bf) && aflipped == bflipped,
            _ => false,
        }
    }
}

impl fmt::Display for Curve {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linear => f.write_str("linear"),
            Self::Cubic { x1, y1, x2, y2 } => {
                write!(f, "cubic({x1}, {y1}, {x2}, {y2})")
            }
            Self::Custom { flipped, .. } => {
                // Nothing useful to print about a function pointer, and its
                // address would make this output differ between runs.
                f.write_str(if *flipped {
                    "custom (flipped)"
                } else {
                    "custom"
                })
            }
        }
    }
}

/// One coordinate of a unit cubic bézier at parameter `s`.
///
/// The first and last control points are 0 and 1, so the usual four-term form
/// collapses to this.
fn bezier(a: f32, b: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * s * a + 3.0 * inv * s * s * b + s * s * s
}

/// The derivative of [`bezier`] with respect to `s`.
fn bezier_slope(a: f32, b: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * a + 6.0 * inv * s * (b - a) + 3.0 * s * s * (1.0 - b)
}

/// The bézier parameter at which the curve's *x* equals `x`.
///
/// A bézier is parameterised by `s`, not by *x*, and the two are only equal for
/// a straight line — so asking "what is *y* when progress is `t`" means solving
/// for `s` first. Newton-Raphson converges in a few steps wherever the curve is
/// steep enough to have a direction; bisection finishes the flat spots, where
/// Newton would divide by nearly zero and leave the interval entirely.
fn solve_x(x: f32, x1: f32, x2: f32) -> f32 {
    /// Close enough that the difference is invisible at any pixel density.
    const TOLERANCE: f32 = 1e-6;

    let mut guess = x;
    for _ in 0..NEWTON_STEPS {
        let error = bezier(x1, x2, guess) - x;
        if error.abs() < TOLERANCE {
            return guess;
        }
        let slope = bezier_slope(x1, x2, guess);
        if slope.abs() < NEWTON_MIN_SLOPE {
            break;
        }
        guess -= error / slope;
        if !(0.0..=1.0).contains(&guess) {
            break;
        }
    }
    if (0.0..=1.0).contains(&guess) && (bezier(x1, x2, guess) - x).abs() < TOLERANCE {
        return guess;
    }

    // Newton stalled on a flat spot or stepped out of the interval. `x` is
    // monotonic in `s` for any curve `Curve::cubic` accepts, so halving the
    // interval always converges — just more slowly.
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    let mut guess = x;
    for _ in 0..BISECTION_STEPS {
        let value = bezier(x1, x2, guess);
        if (value - x).abs() < TOLERANCE {
            break;
        }
        if value < x {
            low = guess;
        } else {
            high = guess;
        }
        guess = (low + high) / 2.0;
    }
    guess
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples of a curve across the unit interval, endpoints included.
    fn samples(curve: Curve, count: u32) -> Vec<f32> {
        (0..=count)
            .map(|step| curve.transform(step as f32 / count as f32))
            .collect()
    }

    #[test]
    fn every_curve_starts_at_zero_and_ends_at_one() {
        for curve in [
            Curve::Linear,
            Curve::EASE,
            Curve::EASE_IN,
            Curve::EASE_OUT,
            Curve::EASE_IN_OUT,
            Curve::FAST_OUT_SLOW_IN,
        ] {
            assert_eq!(curve.transform(0.0), 0.0, "{curve}");
            assert_eq!(curve.transform(1.0), 1.0, "{curve}");
        }
    }

    #[test]
    fn every_curve_is_monotonic_so_an_animation_never_backs_up() {
        for curve in [
            Curve::EASE,
            Curve::EASE_IN,
            Curve::EASE_OUT,
            Curve::EASE_IN_OUT,
            Curve::FAST_OUT_SLOW_IN,
        ] {
            let mut previous = 0.0;
            for value in samples(curve, 200) {
                assert!(
                    value >= previous - 1e-4,
                    "{curve} went backwards: {previous} then {value}"
                );
                previous = value;
            }
        }
    }

    #[test]
    fn progress_outside_the_interval_is_clamped_rather_than_extrapolated() {
        assert_eq!(Curve::EASE_IN_OUT.transform(-2.0), 0.0);
        assert_eq!(Curve::EASE_IN_OUT.transform(4.0), 1.0);
        assert_eq!(Curve::Linear.transform(-0.5), 0.0);
    }

    /// Four discrete steps — the simplest thing a bézier cannot be.
    ///
    /// CSS's `steps(4, end)`: dividing by the step count rather than by one less
    /// is what lands `f(1.0)` on exactly `1.0`.
    fn stepped(t: f32) -> f32 {
        (t * 4.0).floor() / 4.0
    }

    /// Overshoots one before coming back, which is what a bounce *is*.
    fn overshooting(t: f32) -> f32 {
        if t >= 1.0 {
            1.0
        } else {
            1.0 - (1.0 - t).powi(2) * (1.0 - 3.0 * t)
        }
    }

    #[test]
    fn a_custom_curve_can_be_something_no_bezier_can() {
        // A bézier is continuous and a function of one solve; a step is neither.
        let curve = Curve::custom(stepped);
        assert_eq!(curve.transform(0.0), 0.0);
        assert_eq!(
            curve.transform(0.6),
            0.5,
            "held at the step rather than tracking progress"
        );
        assert_eq!(curve.transform(1.0), 1.0);
    }

    #[test]
    fn a_custom_curve_may_leave_the_unit_interval() {
        // **The reason this variant exists.** `transform` clamps its input and
        // must not clamp its output: an overshoot returning more than one is a
        // bounce, and clamping it here would flatten exactly the curves a bézier
        // could not express in the first place.
        let peak = samples(Curve::custom(overshooting), 200)
            .into_iter()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(peak > 1.0, "the overshoot survived: {peak}");
    }

    #[test]
    fn a_flipped_custom_curve_is_the_mirror_of_the_original() {
        // The property `flipped` promises, and the one a `bool` in the variant
        // exists to keep: the flip of `f` is `1 - f(1 - t)`, which is a closure
        // and so cannot be a `fn` pointer.
        let curve = Curve::custom(stepped);
        let flipped = curve.flipped();
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let expected = 1.0 - curve.transform(1.0 - t);
            assert!(
                (flipped.transform(t) - expected).abs() < 1e-6,
                "at {t}: {} vs {expected}",
                flipped.transform(t)
            );
        }
    }

    #[test]
    fn flipping_a_custom_curve_twice_returns_the_original() {
        let curve = Curve::custom(stepped);
        assert_eq!(curve.flipped().flipped(), curve);
    }

    #[test]
    fn a_custom_curve_still_starts_at_zero_and_ends_at_one() {
        // Not enforced at construction — a `const fn` cannot call its argument —
        // so it is the implementor's contract. Both shipped examples keep it.
        for curve in [Curve::custom(stepped), Curve::custom(overshooting)] {
            assert_eq!(curve.transform(0.0), 0.0, "{curve}");
            assert_eq!(curve.transform(1.0), 1.0, "{curve}");
            assert_eq!(curve.flipped().transform(0.0), 0.0, "{curve} flipped");
            assert_eq!(curve.flipped().transform(1.0), 1.0, "{curve} flipped");
        }
    }

    #[test]
    fn easing_in_starts_slower_than_linear_and_catches_up() {
        let quarter = Curve::EASE_IN.transform(0.25);
        assert!(
            quarter < 0.25 * 0.6,
            "an ease-in must visibly hold back at the start, not merely differ \
             from linear: {quarter}"
        );
        assert!(Curve::EASE_IN.transform(0.9) > 0.8, "and then arrive");
    }

    #[test]
    fn easing_out_leaves_immediately_and_arrives_slowly() {
        let quarter = Curve::EASE_OUT.transform(0.25);
        assert!(
            quarter > 0.35,
            "an ease-out must already be well ahead of linear a quarter of the \
             way through: {quarter}"
        );
        let last_tenth = 1.0 - Curve::EASE_OUT.transform(0.9);
        assert!(
            last_tenth < 0.05,
            "the last tenth of the time must cover very little ground: {last_tenth}"
        );
    }

    #[test]
    fn ease_in_out_is_symmetric_about_its_midpoint() {
        for step in 0..=50 {
            let t = step as f32 / 50.0;
            let forward = Curve::EASE_IN_OUT.transform(t);
            let backward = Curve::EASE_IN_OUT.transform(1.0 - t);
            assert!(
                (forward + backward - 1.0).abs() < 1e-3,
                "an ease-in-out that is not symmetric decelerates differently \
                 than it accelerated: f({t})={forward}, f({})={backward}",
                1.0 - t
            );
        }
        assert!((Curve::EASE_IN_OUT.transform(0.5) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn flipping_a_curve_plays_it_backwards() {
        let curve = Curve::EASE_IN;
        let flipped = curve.flipped();
        for step in 0..=20 {
            let t = step as f32 / 20.0;
            assert!(
                (flipped.transform(t) - (1.0 - curve.transform(1.0 - t))).abs() < 1e-3,
                "at {t}: {} vs {}",
                flipped.transform(t),
                1.0 - curve.transform(1.0 - t)
            );
        }
        assert!(
            (Curve::EASE_IN.flipped().transform(0.3) - Curve::EASE_OUT.transform(0.3)).abs() < 1e-6,
            "an ease-in read backwards is an ease-out"
        );
        assert_eq!(Curve::Linear.flipped(), Curve::Linear);
    }

    #[test]
    fn the_solver_inverts_x_rather_than_treating_the_parameter_as_time() {
        // A curve whose control points are far from the diagonal is where
        // "y at parameter t" and "y at progress t" differ most: reading the
        // parameter directly would give 0.5 here, and the answer is not 0.5.
        let curve = Curve::cubic(1.0, 0.0, 0.0, 1.0);
        let mid = curve.transform(0.5);
        assert!(
            (mid - 0.5).abs() < 1e-3,
            "this curve is antisymmetric, so its midpoint is 0.5: {mid}"
        );
        let quarter = curve.transform(0.25);
        assert!(
            quarter < 0.1,
            "and it must hold hard at the start: {quarter}"
        );
    }

    #[test]
    fn a_curve_with_a_flat_spot_still_resolves() {
        // Zero slope at both ends: Newton's method divides by nearly nothing
        // here, and without the bisection fallback this returns garbage.
        let curve = Curve::cubic(1.0, 0.0, 0.0, 1.0);
        for value in samples(curve, 100) {
            assert!(value.is_finite() && (0.0..=1.0).contains(&value), "{value}");
        }
    }

    #[test]
    #[should_panic(expected = "control points")]
    fn a_curve_that_doubles_back_is_rejected() {
        let _ = Curve::cubic(1.5, 0.0, 0.2, 1.0);
    }

    #[test]
    fn the_default_curve_is_linear() {
        assert_eq!(Curve::default(), Curve::Linear);
    }
}
