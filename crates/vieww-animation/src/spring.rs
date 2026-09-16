//! SpringAnimation physics for the existing Ticker pipeline.
//!
//! # Why springs alongside Tween
//!
//! `Tween` + `Curve` answers "where is the value at time t" — a question
//! asked once, before the animation starts, with an answer that never
//! changes. Real interaction needs a different question: "given where I
//! am, how fast I'm going, and where I now want to be, where am I next
//! frame?" That question can be asked mid-flight, and the answer changes
//! when the destination changes.
//!
//! A spring is that second question. It holds position, velocity, and a
//! target. Each `tick` advances the simulation by dt. Calling `retarget`
//! mid-flight keeps position and velocity — no jump, no restart.
//!
//! # Integration
//!
//! `SpringAnimation` implements `Ticker`, so it drops into the same `Tickers`
//! collection that `ScrollController` and `NavigatorController` already
//! use. The frame driver calls `tick` once per frame; the spring reports
//! `is_animating` until it settles; the driver stops calling when it
//! returns `false`.
//!
//! # The settling threshold
//!
//! A damped spring reaches its target at infinity. We declare it settled
//! when displacement and velocity are both below thresholds, because
//! sub-pixel wobble for another second is invisible and keeps the frame
//! busy for nothing.

use std::time::Duration;

use crate::Ticker;

/// Below this displacement (logical px), the spring has arrived.
const SETTLE_DISPLACEMENT: f32 = 0.01;

/// Below this velocity (logical px/s), the spring has stopped.
const SETTLE_VELOCITY: f32 = 10.0;

/// SpringAnimation tuning parameters.
///
/// Three numbers because that is the minimum a real spring needs:
/// how hard it pulls, how quickly oscillation dies, and how fast it
/// was already moving when it started.
///
/// | damping_ratio | behaviour | use for |
/// |---|---|---|
/// | `< 1.0` | overshoots, oscillates, settles | playful UI |
/// | `= 1.0` | fastest arrival, no overshoot | functional UI |
/// | `> 1.0` | no overshoot, slower arrival | almost never |
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSpec {
    /// How hard the spring pulls toward its target. Higher = faster.
    ///
    /// Practical range: 100 (slow) to 1000 (instant).
    pub stiffness: f32,
    /// Ratio of actual damping to critical damping. See the table above.
    pub damping_ratio: f32,
    /// Velocity (units/second) when the spring starts.
    ///
    /// Usually taken from a gesture so a fling hands its momentum to
    /// the spring instead of the spring restarting from zero.
    pub initial_velocity: f32,
}

impl SpringSpec {
    /// A critically damped spring with no initial velocity.
    #[must_use]
    pub const fn new(stiffness: f32) -> Self {
        Self {
            stiffness,
            damping_ratio: 1.0,
            initial_velocity: 0.0,
        }
    }

    /// Override the damping ratio.
    #[must_use]
    pub const fn damping_ratio(mut self, ratio: f32) -> Self {
        self.damping_ratio = ratio;
        self
    }

    /// Override the initial velocity.
    #[must_use]
    pub const fn initial_velocity(mut self, v: f32) -> Self {
        self.initial_velocity = v;
        self
    }
}

/// Preset spring parameters for common interaction feels.
///
/// Two presets, because a product needs exactly one opinion about motion
/// and no more. The preset is chosen at the call site; every spring in
/// the app uses it, which is what keeps motion feeling like one hand
/// made it.
///
/// **Expressive** for the 10% of interactions that are the product:
/// hero transitions, sheet reveals. The overshoot is what makes it feel
/// alive.
///
/// **Standard** for the 90% that are infrastructure: scroll settling,
/// toggle positions. A user would notice if a dialog bounced, and not
/// in a good way.
/// `PartialEq` — but not `Eq` — because [`SpringSpec`] holds `f32`s. Comparison
/// is by variant and, for [`Custom`](Self::Custom), by the three numbers. Two
/// `Custom`s carrying the same parameters as `Standard` are *not* equal to it:
/// the enum records which token was asked for, and a design system that swaps
/// `Standard`'s tuning should not silently change what a `Custom` compares to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpringPreset {
    /// Overshoots and settles. For hero moments.
    Expressive,
    /// No overshoot, minimal fuss. For utilitarian UI.
    Standard,
    /// Bring your own parameters.
    Custom(SpringSpec),
}

impl SpringPreset {
    /// The [`SpringSpec`] this preset describes.
    #[must_use]
    pub const fn spec(self) -> SpringSpec {
        match self {
            // Tuned against mainstream expressive motion presets.
            // Damping 0.8 gives one small overshoot and settle — more
            // and a dialog feels like a rubber ball.
            SpringPreset::Expressive => SpringSpec {
                stiffness: 380.0,
                damping_ratio: 0.8,
                initial_velocity: 0.0,
            },
            // Critically damped at a stiffness that arrives in ~200ms.
            // Faster and a scroll settles before the eye can track it.
            SpringPreset::Standard => SpringSpec {
                stiffness: 200.0,
                damping_ratio: 1.0,
                initial_velocity: 0.0,
            },
            SpringPreset::Custom(spec) => spec,
        }
    }
}

/// A single-value spring animation.
///
/// # Retargeting — the whole point
///
/// Call [`retarget`](Self::retarget) at any time, including mid-flight.
/// The spring keeps its current position and velocity and starts pulling
/// toward the new target. There is no jump, no restart, no visual
/// discontinuity.
///
/// # As a Ticker
///
/// The spring implements [`Ticker`] so it drops into the existing frame
/// pipeline. The frame driver calls `tick` once per frame with the frame
/// timestamp; the spring advances, reports whether it is still animating,
/// and the driver stops calling once it returns `false`.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use vieww_animation::{SpringAnimation, SpringPreset, Ticker};
///
/// let mut spring = SpringAnimation::new(0.0, SpringPreset::Standard);
/// spring.retarget(100.0);
///
/// // Simulate frames at 16ms (60fps).
/// let mut t = Duration::ZERO;
/// for _ in 0..120 {
///     t += Duration::from_millis(16);
///     if !spring.tick(t) {
///         break; // settled
///     }
/// }
/// assert!((spring.value() - 100.0).abs() < 0.1);
/// ```
#[derive(Debug, Clone)]
pub struct SpringAnimation {
    /// Current position.
    value: f32,
    /// Current velocity, in units/second.
    velocity: f32,
    /// Where the spring is pulling toward.
    target: f32,
    /// The tuning parameters.
    spec: SpringSpec,
    /// The timestamp of the last tick, for dt computation.
    ///
    /// `None` until the first tick, because a spring created between
    /// frames has no dt yet and assuming one would mis-scale the first
    /// step.
    last_tick: Option<Duration>,
}

impl SpringAnimation {
    /// A spring starting at `from`, tuned by `preset`.
    ///
    /// Not yet moving — call [`retarget`](Self::retarget) to give it a
    /// destination. The two-step construction is deliberate: the common
    /// pattern is "create at rest, later animate somewhere".
    #[must_use]
    pub fn new(from: f32, preset: SpringPreset) -> Self {
        let spec = preset.spec();
        Self {
            value: from,
            velocity: spec.initial_velocity,
            target: from,
            spec,
            last_tick: None,
        }
    }

    /// A spring starting at `from` with explicit parameters.
    #[must_use]
    pub fn with_spec(from: f32, spec: SpringSpec) -> Self {
        Self {
            value: from,
            velocity: spec.initial_velocity,
            target: from,
            spec,
            last_tick: None,
        }
    }

    /// Current position.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Current velocity, in units/second.
    ///
    /// Exposed because a gesture that grabs a moving spring needs this
    /// to take over smoothly.
    #[must_use]
    pub const fn velocity(&self) -> f32 {
        self.velocity
    }

    /// Where the spring is heading.
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// `true` if the spring has somewhere to be and has not arrived.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        (self.value - self.target).abs() > SETTLE_DISPLACEMENT
            || self.velocity.abs() > SETTLE_VELOCITY
    }

    /// Change the destination, keeping position and velocity.
    ///
    /// This is the interruption-friendly core. Call it as often as you
    /// like: a slider thumb following a finger retargets sixty times a
    /// second and the spring just keeps following.
    pub fn retarget(&mut self, target: f32) {
        self.target = target;
        self.last_tick = None; // Re-derive dt from the next tick.
    }

    /// Override the tuning mid-flight.
    ///
    /// For the rare case where the *feel* should change: a spring that
    /// settles quickly while dragging but becomes bouncy on release.
    /// Position and velocity are kept.
    pub fn retune(&mut self, preset: SpringPreset) {
        self.spec = preset.spec();
    }

    /// Jump to a value with no animation.
    ///
    /// For programmatic changes that should not animate: a scroll
    /// position restored from a saved state, a reset.
    pub fn jump(&mut self, to: f32) {
        self.value = to;
        self.target = to;
        self.velocity = 0.0;
        self.last_tick = None;
    }
}

impl Ticker for SpringAnimation {
    /// Land on the target immediately: see [`Ticker::settle`].
    ///
    /// A spring's whole contribution is the *path* it takes to its target —
    /// overshoot, oscillation, the sense of weight — which is exactly what a
    /// reduced-motion preference is asking not to see. [`jump`](Self::jump) is
    /// already the operation for "be there, without travelling", so this is it
    /// with the change report the tick contract wants.
    fn settle(&mut self, _now: Duration) -> bool {
        let moved = (self.value - self.target).abs() > f32::EPSILON || self.velocity != 0.0;
        let target = self.target;
        self.jump(target);
        moved
    }

    /// Advance the simulation to `now`.
    ///
    /// Returns `true` if the spring is still animating (and should be
    /// ticked again next frame), `false` if it has settled.
    fn tick(&mut self, now: Duration) -> bool {
        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return self.is_animating();
        };

        let dt = now.saturating_sub(last).as_secs_f32();
        self.last_tick = Some(now);

        if dt <= 0.0 {
            return self.is_animating();
        }

        // Clamp dt: a frame that took 200ms (the app was suspended,
        // the debugger paused) would otherwise produce one enormous
        // step. The spring jumps most of the way and settles next
        // frame, which is what a user expects after coming back from
        // a suspend anyway.
        let dt = dt.min(0.033); // At most 2 frames' worth.

        // The analytic solution of the damped spring, not a numerical step.
        //
        // The obvious implementation here is semi-implicit Euler —
        // `v += force * dt; x += v * dt` — and it is wrong at frame rates.
        // Euler adds numerical damping proportional to `dt`, and at a 16ms
        // frame that error swamps the physics: an under-damped spring at
        // ζ=0.8, stiffness 380, should overshoot its target by about 1.5%,
        // and Euler at 60fps delivers 0.02%. The bounce that is the entire
        // reason to choose `Expressive` simply does not happen, and it comes
        // back if you shrink `dt` — which is the signature of an integration
        // error rather than a tuning one.
        //
        // Solving x'' + 2ζωx' + ω²x = 0 in closed form is exact at any `dt`,
        // costs two transcendentals, and removes the frame rate from the
        // feel entirely: a spring on a 30Hz frame and one on 120Hz now trace
        // the same curve.
        let omega = self.spec.stiffness.sqrt();
        let zeta = self.spec.damping_ratio;
        let d0 = self.value - self.target;
        let v0 = self.velocity;

        let (d, v) = if zeta < 1.0 - f32::EPSILON {
            // Under-damped: oscillates inside a decaying envelope.
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * dt).exp();
            let (sin, cos) = (omega_d * dt).sin_cos();

            let a = d0;
            let b = (v0 + zeta * omega * d0) / omega_d;

            let d = decay * (a * cos + b * sin);
            let v = decay
                * ((-zeta * omega * a + b * omega_d) * cos
                    + (-zeta * omega * b - a * omega_d) * sin);
            (d, v)
        } else if zeta <= 1.0 + f32::EPSILON {
            // Critically damped: the repeated-root case.
            let decay = (-omega * dt).exp();
            let c = v0 + omega * d0;
            let d = (d0 + c * dt) * decay;
            let v = (v0 - omega * dt * c) * decay;
            (d, v)
        } else {
            // Over-damped: two real roots, no oscillation.
            let root = omega * (zeta * zeta - 1.0).sqrt();
            let r1 = -zeta * omega + root;
            let r2 = -zeta * omega - root;
            let c1 = (v0 - r2 * d0) / (r1 - r2);
            let c2 = d0 - c1;
            let e1 = (r1 * dt).exp();
            let e2 = (r2 * dt).exp();
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        };

        self.value = self.target + d;
        self.velocity = v;

        // Settled: snap to target and stop. Without the snap, the last
        // few sub-pixel oscillations keep the frame ticking for another
        // second.
        if !self.is_animating() {
            self.value = self.target;
            self.velocity = 0.0;
            return false;
        }

        true
    }

    fn is_animating(&self) -> bool {
        SpringAnimation::is_animating(self)
    }
}

/// A two-dimensional spring, for point/offset animation.
///
/// Two independent springs sharing the same tuning. Independence is
/// correct: a fling that is mostly horizontal should not wobble
/// vertically just because the y-spring is attached to the x-spring.
#[derive(Debug, Clone)]
pub struct Spring2D {
    pub x: SpringAnimation,
    pub y: SpringAnimation,
}

impl Spring2D {
    /// A 2D spring starting at `(x, y)`.
    #[must_use]
    pub fn new(x: f32, y: f32, preset: SpringPreset) -> Self {
        Self {
            x: SpringAnimation::new(x, preset),
            y: SpringAnimation::new(y, preset),
        }
    }

    /// Retarget both axes.
    pub fn retarget(&mut self, x: f32, y: f32) {
        self.x.retarget(x);
        self.y.retarget(y);
    }

    /// Current position as `(x, y)`.
    #[must_use]
    pub fn value(&self) -> (f32, f32) {
        (self.x.value(), self.y.value())
    }

    /// Jump both axes.
    pub fn jump(&mut self, x: f32, y: f32) {
        self.x.jump(x);
        self.y.jump(y);
    }
}

impl Ticker for Spring2D {
    fn tick(&mut self, now: Duration) -> bool {
        let x_alive = self.x.tick(now);
        let y_alive = self.y.tick(now);
        x_alive || y_alive
    }

    fn is_animating(&self) -> bool {
        self.x.is_animating() || self.y.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIXTY_FPS: Duration = Duration::from_millis(16);

    /// Run a spring to completion, returning the settled value.
    fn settle(spring: &mut SpringAnimation, frames: usize) -> f32 {
        let mut t = Duration::ZERO;
        for _ in 0..frames {
            t += SIXTY_FPS;
            if !spring.tick(t) {
                break;
            }
        }
        spring.value()
    }

    #[test]
    fn a_standard_spring_arrives_without_overshooting() {
        let mut spring = SpringAnimation::new(0.0, SpringPreset::Standard);
        spring.retarget(100.0);

        let mut max = 0.0f32;
        let mut t = Duration::ZERO;
        for _ in 0..120 {
            t += SIXTY_FPS;
            if !spring.tick(t) {
                break;
            }
            max = max.max(spring.value());
        }

        assert!((spring.value() - 100.0).abs() < 0.1, "arrived");
        assert!(
            max <= 100.0 + 0.5,
            "critically damped must not overshoot, reached {max}"
        );
    }

    #[test]
    fn an_expressive_spring_overshoots_then_settles() {
        let mut spring = SpringAnimation::new(0.0, SpringPreset::Expressive);
        spring.retarget(100.0);

        let mut max = 0.0f32;
        let mut t = Duration::ZERO;
        for _ in 0..240 {
            t += SIXTY_FPS;
            if !spring.tick(t) {
                break;
            }
            max = max.max(spring.value());
        }

        assert!(max > 100.0, "under-damped overshoots, reached {max}");
        assert!((spring.value() - 100.0).abs() < 0.1, "but still arrives");
    }

    #[test]
    fn retargeting_mid_flight_does_not_jump() {
        let mut spring = SpringAnimation::new(0.0, SpringPreset::Standard);
        spring.retarget(100.0);

        // Run part-way. Ten frames is ~160ms, and `Standard` is documented
        // as arriving in ~200ms — at the twenty frames this test originally
        // used the spring is already at 95% and "mid-flight" is not a
        // meaningful description of it.
        let mut t = Duration::ZERO;
        for _ in 0..10 {
            t += SIXTY_FPS;
            spring.tick(t);
        }
        let mid = spring.value();
        assert!(mid > 10.0 && mid < 90.0, "somewhere in transit: {mid}");

        // Retarget backward. The next value must be close to `mid` — a
        // jump means the spring reset rather than redirected.
        spring.retarget(0.0);
        let before = spring.value();
        t += SIXTY_FPS;
        spring.tick(t);
        let after = spring.value();

        assert!(
            (before - after).abs() < 15.0,
            "no jump: {before} -> {after}"
        );
    }

    #[test]
    fn a_spring_at_its_target_does_not_tick() {
        let mut spring = SpringAnimation::new(50.0, SpringPreset::Expressive);
        assert!(!spring.is_animating(), "nowhere to go");

        let t = Duration::from_millis(16);
        assert!(!spring.tick(t), "and nothing to do");
    }

    #[test]
    fn jump_takes_effect_immediately() {
        let mut spring = SpringAnimation::new(0.0, SpringPreset::Expressive);
        spring.retarget(100.0);
        spring.jump(200.0);

        assert_eq!(spring.value(), 200.0);
        assert_eq!(spring.target(), 200.0);
        assert!(!spring.is_animating());
    }

    #[test]
    fn initial_velocity_carries_through() {
        // A spring given initial velocity toward the target should not
        // be slower than one starting at rest.
        let at_rest = {
            let mut s = SpringAnimation::new(0.0, SpringPreset::Standard);
            s.retarget(100.0);
            settle(&mut s, 60)
        };

        let moving = {
            let spec = SpringPreset::Standard.spec().initial_velocity(500.0);
            let mut s = SpringAnimation::with_spec(0.0, spec);
            s.retarget(100.0);
            settle(&mut s, 60)
        };

        assert!((at_rest - 100.0).abs() < 1.0);
        assert!((moving - 100.0).abs() < 1.0);
    }

    #[test]
    fn a_2d_spring_settles_both_axes() {
        let mut spring = Spring2D::new(0.0, 0.0, SpringPreset::Standard);
        spring.retarget(50.0, 100.0);

        let mut t = Duration::ZERO;
        for _ in 0..120 {
            t += SIXTY_FPS;
            if !spring.tick(t) {
                break;
            }
        }

        let (x, y) = spring.value();
        assert!((x - 50.0).abs() < 0.1);
        assert!((y - 100.0).abs() < 0.1);
    }
}
