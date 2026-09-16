//! Motion that is described by physics rather than by a duration.
//!
//! # Why these are not curves
//!
//! A curve needs to be told how long to take. That is fine for a value the
//! interface chose to change — a panel opening, a colour crossfading — and wrong
//! for anything continuing a gesture, because the finger already set the speed.
//! A card released mid-flick has a velocity, and the only honest answer to "how
//! long should the rest of the movement take" is "as long as that speed and this
//! spring imply". So a simulation is a function of time with *no* duration in it;
//! it reports its own [`is_done`](Simulation::is_done).
//!
//! # Closed form, not integration
//!
//! Every simulation here answers "where is it at time *t*" directly. Stepping a
//! velocity by a frame delta instead makes the result depend on frame rate — a
//! fling travels a different distance on a 60Hz and a 120Hz screen, and a dropped
//! frame shortens the throw. It also means a position can be sampled at whatever
//! moment the frame actually lands, rather than assuming it landed on time.
//!
//! # Where these came from
//!
//! [`Spring`] and [`Fling`] were written for `vieww-gestures`, which is where
//! their first caller lives — a scrollable's fling and its rubber-band settle.
//! They moved here in Phase 7 because an [`AnimationController`] wants the same
//! two simulations and a dependency from animation onto gestures would have been
//! backwards. `vieww_gestures` re-exports both, so scroll code is unchanged.
//!
//! [`AnimationController`]: crate::AnimationController

use std::fmt;
use std::time::Duration;

/// How far a fling may still travel before it is called stopped, in logical
/// pixels per second.
pub const MIN_FLING_VELOCITY: f32 = 50.0;

/// A position as a function of time, that knows when it has finished.
///
/// Implementable from outside this repository, and — since
/// [`Motion::Custom`] — **acceptable** from outside it too. Those are different
/// claims, and for a while only the first was true: the trait was public,
/// documented and implementable, and nothing in the framework took one. A public
/// trait that advertises an extension point which does not exist is worse than
/// no trait at all, because the discovery happens after the work.
///
/// `Debug` is required because [`Motion`] derives it, and `Motion` is stored in
/// an `AnimationController` that a widget prints when a tree is dumped.
pub trait Simulation: fmt::Debug {
    /// Where it is `elapsed` after it started.
    #[must_use]
    fn position(&self, elapsed: Duration) -> f32;

    /// How fast it is moving `elapsed` after it started, in units per second.
    #[must_use]
    fn velocity_at(&self, elapsed: Duration) -> f32;

    /// How long until it has run its course.
    ///
    /// Worth asking once and remembering: a spring has no closed form for it and
    /// answers by scanning, so a caller that asks every frame turns a constant
    /// into a per-frame cost. [`AnimationController`](crate::AnimationController)
    /// caches this when the simulation starts.
    #[must_use]
    fn duration(&self) -> Duration;

    /// `true` once it has run its course and further sampling would tell you
    /// nothing new.
    #[must_use]
    fn is_done(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration()
    }

    /// Where it comes to rest.
    ///
    /// Not always the same as sampling at [`duration`](Simulation::duration):
    /// "arrived" is a tolerance, and a settle that stops a quarter of a percent
    /// short leaves a card that came home sitting fractionally off. A
    /// simulation that knows its exact resting place says so here, and the
    /// controller lands on it.
    #[must_use]
    fn final_position(&self) -> f32 {
        self.position(self.duration())
    }
}

/// A fling in progress: position as a function of time since release.
///
/// The velocity decays exponentially, so position is its integral:
///
/// ```text
/// v(t) = v0 · dragᵗ
/// x(t) = x0 + v0 · (dragᵗ − 1) / ln(drag)
/// ```
///
/// which converges — a fling travels a finite distance and
/// [`duration`](Self::duration) says when it gets there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fling {
    start: f32,
    velocity: f32,
    drag: f32,
}

impl Fling {
    /// A fling released at `start` moving at `velocity` units per second, losing
    /// all but `drag` of its speed each second.
    #[must_use]
    pub const fn new(start: f32, velocity: f32, drag: f32) -> Self {
        Self {
            start,
            velocity,
            drag,
        }
    }

    /// Where the fling is `elapsed` after release.
    #[must_use]
    pub fn position(&self, elapsed: Duration) -> f32 {
        let t = elapsed.as_secs_f32();
        let decay = self.drag.powf(t);
        self.start + self.velocity * (decay - 1.0) / self.drag.ln()
    }

    /// How fast it is moving `elapsed` after release.
    #[must_use]
    pub fn velocity_at(&self, elapsed: Duration) -> f32 {
        self.velocity * self.drag.powf(elapsed.as_secs_f32())
    }

    /// Where it comes to rest.
    #[must_use]
    pub fn destination(&self) -> f32 {
        self.start - self.velocity / self.drag.ln()
    }

    /// How long until it is moving slower than [`MIN_FLING_VELOCITY`].
    ///
    /// Finite for any velocity: the decay is exponential, so the time to fall
    /// below a threshold is a logarithm, not a limit that is never reached.
    #[must_use]
    pub fn duration(&self) -> Duration {
        let speed = self.velocity.abs();
        if speed <= MIN_FLING_VELOCITY {
            return Duration::ZERO;
        }
        let seconds = (MIN_FLING_VELOCITY / speed).ln() / self.drag.ln();
        Duration::from_secs_f32(seconds.max(0.0))
    }

    /// `true` once the fling has run its course.
    #[must_use]
    pub fn is_done(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration()
    }
}

impl Simulation for Fling {
    fn position(&self, elapsed: Duration) -> f32 {
        Self::position(self, elapsed)
    }

    fn velocity_at(&self, elapsed: Duration) -> f32 {
        Self::velocity_at(self, elapsed)
    }

    fn duration(&self) -> Duration {
        Self::duration(self)
    }
}

/// A critically damped spring pulling a value back to a target.
///
/// Critically damped rather than under-damped, deliberately: an overscrolled list
/// that oscillates about its edge looks broken. This returns as fast as it can
/// without going past.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    start: f32,
    target: f32,
    velocity: f32,
    stiffness: f32,
}

impl Spring {
    /// How hard a spring pulls when nothing has said otherwise.
    ///
    /// Chosen to settle in a little over a third of a second, which is about the
    /// longest a released gesture can take to come to rest before the interface
    /// feels like it is thinking rather than responding.
    pub const DEFAULT_STIFFNESS: f32 = 12.0;

    #[must_use]
    pub const fn new(start: f32, target: f32, velocity: f32, stiffness: f32) -> Self {
        Self {
            start,
            target,
            velocity,
            stiffness,
        }
    }

    /// A spring at [`DEFAULT_STIFFNESS`](Self::DEFAULT_STIFFNESS).
    #[must_use]
    pub const fn settling(start: f32, target: f32, velocity: f32) -> Self {
        Self::new(start, target, velocity, Self::DEFAULT_STIFFNESS)
    }

    /// Where the spring is pulling to.
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.target
    }

    /// Where the spring is `elapsed` after it started.
    ///
    /// The closed form of a critically damped oscillator:
    /// `x(t) = target + (A + B·t)·e^(−ω·t)`.
    #[must_use]
    pub fn position(&self, elapsed: Duration) -> f32 {
        let t = elapsed.as_secs_f32();
        let w = self.stiffness;
        let a = self.start - self.target;
        let b = self.velocity + a * w;
        self.target + (a + b * t) * (-w * t).exp()
    }

    /// How fast it is moving `elapsed` after it started.
    ///
    /// The derivative of [`position`](Self::position): `(B − ω·(A + B·t))·e^(−ω·t)`.
    #[must_use]
    pub fn velocity_at(&self, elapsed: Duration) -> f32 {
        let t = elapsed.as_secs_f32();
        let w = self.stiffness;
        let a = self.start - self.target;
        let b = self.velocity + a * w;
        (b - w * (a + b * t)) * (-w * t).exp()
    }

    /// How far the spring travels away from its target before coming back.
    ///
    /// The scale everything about this spring is measured against. Both terms
    /// matter: a spring released far from its target is dominated by the
    /// distance, and one released *at* its target with speed — a flick that has
    /// to carry before it returns — is dominated by the velocity, whose peak
    /// excursion for a critically damped spring is `v/(ω·e)`.
    #[must_use]
    pub fn excursion(&self) -> f32 {
        let displacement = (self.start - self.target).abs();
        let from_velocity = if self.stiffness > 0.0 {
            self.velocity.abs() / (self.stiffness * std::f32::consts::E)
        } else {
            0.0
        };
        displacement.max(from_velocity)
    }

    /// How long until it has arrived and is staying there.
    ///
    /// # Why the tolerance is relative
    ///
    /// "Arrived" has to mean a fraction of the distance travelled, not a fixed
    /// number, because the same spring settles a scroll offset measured in
    /// hundreds of pixels and an animation value measured in *units of one*. An
    /// absolute half-pixel tolerance — which is what this had while it lived in
    /// the scroll physics, where pixels were the only unit — declares a 0..1
    /// animation finished before it has visibly started.
    #[must_use]
    pub fn duration(&self) -> Duration {
        /// A quarter of a percent of the distance travelled: below a pixel for a
        /// full-screen movement, and invisible for anything smaller.
        const TOLERANCE: f32 = 0.0025;
        /// However long the physics say, an animation that has not finished in
        /// this long has stopped being motion and become a bug.
        const CAP: f32 = 4.0;

        let tolerance = self.excursion() * TOLERANCE;
        if tolerance <= 0.0 {
            // Nowhere to go: no displacement and no speed.
            return Duration::ZERO;
        }

        // No closed form for "within a tolerance" on this curve, and a loop over
        // a handful of milliseconds is cheaper to read than a Lambert W. The
        // result is asked for once per animation, not once per frame — see
        // `Simulation::duration`.
        let mut t = 0.0_f32;
        while t < CAP {
            let elapsed = Duration::from_secs_f32(t);
            let arrived = (self.position(elapsed) - self.target).abs() < tolerance;
            // Being *at* the target is not being finished: a spring released
            // from rest at its target but moving — a flick that has to carry
            // before it comes back — passes through the target on frame one,
            // and stopping there would swallow the entire motion.
            let settled = self.velocity_at(elapsed).abs() < tolerance * self.stiffness;
            if arrived && settled {
                return elapsed;
            }
            t += 1.0 / 120.0;
        }
        Duration::from_secs_f32(CAP)
    }

    /// `true` once it has settled.
    #[must_use]
    pub fn is_done(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration()
    }
}

impl Simulation for Spring {
    fn position(&self, elapsed: Duration) -> f32 {
        Self::position(self, elapsed)
    }

    fn velocity_at(&self, elapsed: Duration) -> f32 {
        Self::velocity_at(self, elapsed)
    }

    fn duration(&self) -> Duration {
        Self::duration(self)
    }

    fn final_position(&self) -> f32 {
        self.target
    }
}

/// The simulations an [`AnimationController`](crate::AnimationController) can be
/// driven by.
///
/// An enum rather than a `Box<dyn Simulation>`: the two built in are `Copy`, and
/// a controller stays a plain value that can be cloned into a widget without an
/// allocation or a lifetime. [`Custom`](Self::Custom) keeps that property by
/// borrowing rather than owning.
#[derive(Debug, Clone, Copy)]
pub enum Motion {
    /// Decelerating from a release velocity. A throw.
    Fling(Fling),
    /// Pulled to a target. A settle.
    Spring(Spring),
    /// Any physics at all, written anywhere.
    ///
    /// ```
    /// use std::time::Duration;
    /// use vieww_animation::{Motion, Simulation};
    ///
    /// #[derive(Debug)]
    /// struct Instant;
    /// impl Simulation for Instant {
    ///     fn position(&self, _: Duration) -> f32 { 1.0 }
    ///     fn velocity_at(&self, _: Duration) -> f32 { 0.0 }
    ///     fn duration(&self) -> Duration { Duration::ZERO }
    /// }
    ///
    /// static INSTANT: Instant = Instant;
    /// let motion = Motion::Custom(&INSTANT);
    /// assert_eq!(motion.position(Duration::ZERO), 1.0);
    /// ```
    ///
    /// # Why `&'static` rather than `Box`
    ///
    /// Because `Copy` is the property this enum exists to protect — a
    /// `Box<dyn Simulation>` would make every `AnimationController` clone an
    /// allocation, and controllers are cloned into widgets on every build.
    ///
    /// The cost is that a custom simulation **cannot carry per-animation
    /// state**: it has to be a `static`, so its parameters are fixed at compile
    /// time. A spring whose stiffness comes from a config file does not fit, and
    /// the way to get one is to leak it once at startup — `Box::leak` gives a
    /// `&'static` — rather than to allocate per animation. That is the whole of
    /// what this does not cover.
    Custom(&'static dyn Simulation),
}

impl Motion {
    /// A critically damped settle from `start` to `target`, carrying `velocity`.
    #[must_use]
    pub const fn settling(start: f32, target: f32, velocity: f32) -> Self {
        Self::Spring(Spring::settling(start, target, velocity))
    }
}

/// # Two `Custom`s are equal when they are the same simulation
///
/// Hand-written because `&dyn Simulation` has no `PartialEq` and cannot be given
/// one: comparing behaviour would mean sampling, and two simulations that agree
/// at every sampled instant are still not the same object.
///
/// So identity, compared as a **thin** pointer — `from_ref(..).cast::<()>()`
/// discards the vtable, which is the part that may legitimately differ between
/// codegen units for one type. Comparing the wide pointer risks a false
/// *inequality*; this cannot, and equality of the data address is exactly the
/// question being asked.
impl PartialEq for Motion {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Fling(a), Self::Fling(b)) => a == b,
            (Self::Spring(a), Self::Spring(b)) => a == b,
            (Self::Custom(a), Self::Custom(b)) => {
                std::ptr::from_ref(*a).cast::<()>() == std::ptr::from_ref(*b).cast::<()>()
            }
            _ => false,
        }
    }
}

impl Simulation for Motion {
    fn position(&self, elapsed: Duration) -> f32 {
        match self {
            Self::Fling(fling) => fling.position(elapsed),
            Self::Spring(spring) => spring.position(elapsed),
            Self::Custom(simulation) => simulation.position(elapsed),
        }
    }

    fn velocity_at(&self, elapsed: Duration) -> f32 {
        match self {
            Self::Fling(fling) => fling.velocity_at(elapsed),
            Self::Spring(spring) => spring.velocity_at(elapsed),
            Self::Custom(simulation) => simulation.velocity_at(elapsed),
        }
    }

    fn duration(&self) -> Duration {
        match self {
            Self::Fling(fling) => fling.duration(),
            Self::Spring(spring) => spring.duration(),
            Self::Custom(simulation) => simulation.duration(),
        }
    }

    fn is_done(&self, elapsed: Duration) -> bool {
        match self {
            // Delegated rather than left to the default, because the default
            // asks `duration()` and a custom simulation is allowed to override
            // both. Falling through to `elapsed >= self.duration()` would
            // silently ignore an override of exactly this method.
            Self::Custom(simulation) => simulation.is_done(elapsed),
            _ => elapsed >= self.duration(),
        }
    }

    fn final_position(&self) -> f32 {
        match self {
            // A fling stops when it is *too slow to matter*, deliberately short
            // of where it would eventually converge, so where it stopped is
            // where it belongs. A spring knows its target exactly.
            Self::Fling(fling) => fling.position(fling.duration()),
            Self::Spring(spring) => spring.target(),
            Self::Custom(simulation) => simulation.final_position(),
        }
    }
}

impl From<Fling> for Motion {
    fn from(fling: Fling) -> Self {
        Self::Fling(fling)
    }
}

impl From<Spring> for Motion {
    fn from(spring: Spring) -> Self {
        Self::Spring(spring)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Android's scroller deceleration — the value the fling tests were written
    /// against when this lived in `vieww-gestures`.
    const DRAG: f32 = 0.000_015_5;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_fling_starts_where_it_was_released_and_moves_the_way_it_was_thrown() {
        let fling = Fling::new(100.0, -900.0, DRAG);

        assert!((fling.position(Duration::ZERO) - 100.0).abs() < 0.01);
        assert!(
            fling.position(ms(100)) < 100.0,
            "thrown negative, so it travels negative"
        );
    }

    #[test]
    fn a_fling_decelerates_rather_than_coasting() {
        let fling = Fling::new(0.0, 1000.0, DRAG);

        let first = fling.position(ms(100)) - fling.position(Duration::ZERO);
        let later = fling.position(ms(400)) - fling.position(ms(300));
        assert!(
            later < first * 0.9,
            "each 100ms must cover less ground than the last: {first} then {later}"
        );
    }

    #[test]
    fn a_fling_stops_somewhere_finite() {
        let fling = Fling::new(0.0, 4000.0, DRAG);

        let destination = fling.destination();
        assert!(
            destination.is_finite() && destination > 0.0,
            "{destination}"
        );
        assert!(
            (fling.position(ms(5000)) - destination).abs() < 1.0,
            "it converges on its destination rather than approaching forever"
        );
    }

    #[test]
    fn a_fling_has_a_finite_duration_and_is_slow_by_the_end_of_it() {
        let fling = Fling::new(0.0, 3000.0, DRAG);
        let duration = fling.duration();

        assert!(
            duration > Duration::ZERO && duration < ms(4000),
            "{duration:?}"
        );
        assert!(fling.velocity_at(duration).abs() <= MIN_FLING_VELOCITY + 1.0);
        assert!(fling.is_done(duration));
    }

    #[test]
    fn a_flick_too_slow_to_matter_is_already_done() {
        let fling = Fling::new(0.0, MIN_FLING_VELOCITY / 2.0, DRAG);
        assert_eq!(fling.duration(), Duration::ZERO);
        assert!(fling.is_done(Duration::ZERO));
    }

    #[test]
    fn a_spring_returns_to_its_target_without_going_past_it() {
        let spring = Spring::settling(80.0, 0.0, 0.0);

        let mut previous = spring.position(Duration::ZERO);
        for step in 1..=200 {
            let position = spring.position(Duration::from_secs_f32(step as f32 / 120.0));
            assert!(
                position >= -0.5,
                "critically damped means it must not overshoot: {position} at step {step}"
            );
            assert!(position <= previous + 0.01, "and must not turn back");
            previous = position;
        }
        assert!(previous.abs() < 1.0, "and it does arrive: {previous}");
    }

    #[test]
    fn a_spring_settles_in_a_reasonable_time() {
        let spring = Spring::settling(120.0, 0.0, 0.0);
        let duration = spring.duration();

        assert!(
            duration > Duration::ZERO && duration < ms(2000),
            "{duration:?}"
        );
        assert!((spring.position(duration)).abs() < 0.5);
        assert!(spring.is_done(duration));
    }

    #[test]
    fn a_spring_reports_the_velocity_its_position_is_actually_changing_at() {
        let spring = Spring::settling(100.0, 0.0, 0.0);
        assert_eq!(
            spring.velocity_at(Duration::ZERO),
            0.0,
            "released from rest, so it starts at rest — the pull has to build"
        );

        // A central difference of `position` has to agree with the analytic
        // derivative, or a fling handed off to a spring mid-flight would jump.
        for millis in [50, 120, 300] {
            let seconds = millis as f32 / 1000.0;
            let step = 0.0005;
            let before = spring.position(Duration::from_secs_f32(seconds - step));
            let after = spring.position(Duration::from_secs_f32(seconds + step));
            let measured = (after - before) / (2.0 * step);
            let reported = spring.velocity_at(ms(millis));
            assert!(
                (measured - reported).abs() < 1.0,
                "at {millis}ms: measured {measured}, reported {reported}"
            );
        }
    }

    #[test]
    fn a_springs_notion_of_arrived_scales_with_how_far_it_travelled() {
        // The same spring, in pixels and in units of one. An absolute tolerance
        // declares the second one finished before it has visibly moved.
        let pixels = Spring::settling(200.0, 0.0, 0.0);
        let units = Spring::settling(1.0, 0.0, 0.0);

        let ratio = units.duration().as_secs_f32() / pixels.duration().as_secs_f32();
        assert!(
            (ratio - 1.0).abs() < 0.2,
            "the same motion at two scales must take about the same time: \
             {:?} vs {:?}",
            pixels.duration(),
            units.duration()
        );
        assert!(units.duration() > ms(300), "{:?}", units.duration());
    }

    #[test]
    fn a_spring_released_at_its_target_is_not_finished_if_it_is_still_moving() {
        let thrown = Spring::settling(0.0, 0.0, 4.0);
        assert!(
            thrown.duration() > ms(100),
            "it starts *at* the target, so a position-only test calls it done \
             on frame one and swallows the whole motion: {:?}",
            thrown.duration()
        );

        let still = Spring::settling(0.0, 0.0, 0.0);
        assert_eq!(
            still.duration(),
            Duration::ZERO,
            "but one with nowhere to go and no speed really is done"
        );
    }

    #[test]
    fn a_spring_launched_with_speed_carries_it_before_turning_back() {
        // Released at the target but still moving away from it: the spring must
        // let it travel and then pull it back, which is what makes a rubber-band
        // release look continuous with the gesture.
        let spring = Spring::settling(0.0, 0.0, 300.0);

        let peak = spring.position(ms(80));
        assert!(peak > 5.0, "it has to keep going at first: {peak}");
        assert!(
            spring.position(spring.duration()).abs() < 0.5,
            "and still come home"
        );
    }

    #[test]
    fn a_motion_dispatches_to_whichever_simulation_it_holds() {
        let spring: Motion = Spring::settling(10.0, 0.0, 0.0).into();
        let fling: Motion = Fling::new(0.0, 1000.0, DRAG).into();

        assert!((spring.position(Duration::ZERO) - 10.0).abs() < 0.01);
        assert!(fling.position(ms(100)) > 0.0);
        assert!(!spring.is_done(Duration::ZERO));
        assert!(spring.is_done(ms(4000)));
    }

    /// Physics written outside this crate — a constant-speed slide, which
    /// neither `Fling` nor `Spring` can express.
    #[derive(Debug)]
    struct Slide {
        per_second: f32,
        seconds: f32,
    }

    impl Simulation for Slide {
        fn position(&self, elapsed: Duration) -> f32 {
            self.per_second * elapsed.as_secs_f32().min(self.seconds)
        }

        fn velocity_at(&self, elapsed: Duration) -> f32 {
            if elapsed.as_secs_f32() >= self.seconds {
                0.0
            } else {
                self.per_second
            }
        }

        fn duration(&self) -> Duration {
            Duration::from_secs_f32(self.seconds)
        }
    }

    static SLIDE: Slide = Slide {
        per_second: 100.0,
        seconds: 2.0,
    };

    #[test]
    fn a_simulation_written_elsewhere_can_actually_drive_a_motion() {
        // **The gap this closes.** `Simulation` was public, documented and
        // implementable, and nothing in the framework accepted one — so the work
        // of writing it could be done in full and then had nowhere to go.
        let motion = Motion::Custom(&SLIDE);

        assert_eq!(motion.position(ms(500)), 50.0);
        assert_eq!(motion.velocity_at(ms(500)), 100.0);
        assert_eq!(motion.duration(), Duration::from_secs(2));
        assert!(!motion.is_done(ms(500)));
        assert!(motion.is_done(ms(2500)));
    }

    #[test]
    fn a_custom_motions_resting_place_is_its_own_to_report() {
        // The hook that exists because "arrived" is a tolerance: a simulation
        // that knows exactly where it stops says so, rather than being sampled
        // at its duration and landing fractionally short.
        assert_eq!(Motion::Custom(&SLIDE).final_position(), 200.0);
    }

    #[test]
    fn two_custom_motions_are_equal_when_they_are_the_same_simulation() {
        // Identity, because behaviour cannot be compared without sampling and
        // two simulations agreeing at every sample are still not one object.
        static OTHER: Slide = Slide {
            per_second: 100.0,
            seconds: 2.0,
        };

        assert_eq!(Motion::Custom(&SLIDE), Motion::Custom(&SLIDE));
        assert_ne!(
            Motion::Custom(&SLIDE),
            Motion::Custom(&OTHER),
            "identical parameters, different simulations"
        );
    }

    #[test]
    fn a_custom_motion_is_never_equal_to_a_built_in_one() {
        let spring: Motion = Spring::settling(10.0, 0.0, 0.0).into();
        assert_ne!(Motion::Custom(&SLIDE), spring);
    }
}
