//! The thing that turns frames into a moving number.
//!
//! # One number, and why that is enough
//!
//! An [`AnimationController`] animates a single `f32`. Everything richer — a
//! colour, a position, a set of insets, several of them at once — is that number
//! put through a [`Tween`](crate::Tween). Keeping the controller scalar means
//! there is exactly one place that understands time, direction, curves and
//! completion, and adding an animatable type costs a [`Lerp`](crate::Lerp) impl
//! rather than a new controller.
//!
//! # Nothing here reads a clock
//!
//! Every method that starts or advances an animation takes `now`. The frame
//! scheduler supplies it from the vsync timestamp, which means an animation
//! samples where it should be *at the moment this frame will be shown* rather
//! than where it was when the CPU got round to it — and it means every behaviour
//! in this module is testable to the microsecond instead of by sleeping.

use std::time::Duration;

use crate::{Curve, Motion, Simulation};

/// Where an animation is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationStatus {
    /// At the start, not moving.
    Dismissed,
    /// Moving towards the end.
    Forward,
    /// Moving back towards the start.
    Reverse,
    /// At the end, not moving.
    Completed,
}

impl AnimationStatus {
    /// `true` while the value is still changing.
    #[must_use]
    pub const fn is_animating(self) -> bool {
        matches!(self, Self::Forward | Self::Reverse)
    }
}

/// What is currently moving the value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drive {
    /// From one value to another over a fixed time, shaped by the curve.
    Timed {
        from: f32,
        to: f32,
        started: Duration,
        duration: Duration,
        /// `Some(reverse)` to run forever, ping-ponging when `reverse`.
        repeat: Option<bool>,
    },
    /// Wherever the physics says, for as long as the physics says.
    Simulated {
        motion: Motion,
        started: Duration,
        /// Asked of the simulation once, because a spring answers by scanning.
        duration: Duration,
    },
}

/// A value moving from 0 to 1 — or wherever a simulation takes it — driven by
/// the frame scheduler.
///
/// ```
/// use std::time::Duration;
/// use vieww_animation::{AnimationController, AnimationStatus, Curve};
///
/// let ms = Duration::from_millis;
/// let mut fade = AnimationController::new(ms(300)).curve(Curve::EASE_IN_OUT);
/// assert_eq!(fade.status(), AnimationStatus::Dismissed);
///
/// fade.forward(ms(0));
/// fade.tick(ms(150));
/// assert!((fade.value() - 0.5).abs() < 0.01, "half a symmetric curve is half way");
///
/// fade.tick(ms(300));
/// assert_eq!(fade.value(), 1.0);
/// assert_eq!(fade.status(), AnimationStatus::Completed);
/// assert!(!fade.is_animating(), "and it stops asking for frames");
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimationController {
    value: f32,
    duration: Duration,
    curve: Curve,
    status: AnimationStatus,
    drive: Option<Drive>,
}

impl AnimationController {
    /// A controller at 0 that takes `duration` to travel the whole 0..1 range.
    #[must_use]
    pub const fn new(duration: Duration) -> Self {
        Self {
            value: 0.0,
            duration,
            curve: Curve::Linear,
            status: AnimationStatus::Dismissed,
            drive: None,
        }
    }

    /// Shape the timed motion with `curve`.
    #[must_use]
    pub const fn curve(mut self, curve: Curve) -> Self {
        self.curve = curve;
        self
    }

    /// Start at `value` rather than at 0.
    #[must_use]
    pub const fn starting_at(mut self, value: f32) -> Self {
        self.value = value;
        self
    }

    /// The current value: `0..=1` for timed motion, and wherever the simulation
    /// is for a physical one — a spring overshoots, and flattening that here
    /// would defeat the point of using one.
    #[must_use]
    pub const fn value(&self) -> f32 {
        self.value
    }

    /// Where it is in its lifecycle.
    #[must_use]
    pub const fn status(&self) -> AnimationStatus {
        self.status
    }

    /// `true` while this controller still needs frames.
    #[must_use]
    pub const fn is_animating(&self) -> bool {
        self.drive.is_some()
    }

    /// How long a full 0..1 traverse takes.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Change how long a full traverse takes. Applies to the *next* animation,
    /// not to one already running.
    pub const fn set_duration(&mut self, duration: Duration) {
        self.duration = duration;
    }

    /// Change the easing. Applies to the *next* animation.
    pub const fn set_curve(&mut self, curve: Curve) {
        self.curve = curve;
    }

    /// Run to 1.
    pub fn forward(&mut self, now: Duration) {
        self.animate_to(1.0, now);
    }

    /// Run back to 0.
    pub fn reverse(&mut self, now: Duration) {
        self.animate_to(0.0, now);
    }

    /// Run from wherever it is to `target`.
    ///
    /// # Why the duration shrinks
    ///
    /// The time taken is [`duration`](Self::duration) scaled by the distance
    /// left, so reversing a half-open drawer takes half as long as reversing a
    /// fully open one. A fixed duration regardless of distance is the classic
    /// way to make a quick tap-tap on a toggle feel broken: the second animation
    /// crawls across a tiny distance while the finger has already moved on.
    pub fn animate_to(&mut self, target: f32, now: Duration) {
        let distance = (target - self.value).abs();
        if distance <= f32::EPSILON {
            self.value = target;
            self.drive = None;
            self.status = Self::resting_status(target);
            return;
        }
        self.status = if target > self.value {
            AnimationStatus::Forward
        } else {
            AnimationStatus::Reverse
        };
        self.drive = Some(Drive::Timed {
            from: self.value,
            to: target,
            started: now,
            duration: self.duration.mul_f32(distance.clamp(0.0, 1.0)),
            repeat: None,
        });
    }

    /// Loop forever between 0 and 1, ping-ponging if `reverse`.
    ///
    /// For a spinner or a pulse — something with no end state to arrive at. It
    /// never completes, so whatever started it is responsible for
    /// [`stop`](Self::stop)ping it; an element that unmounts drops its
    /// controller, and a dropped controller is pruned from the tickers.
    pub fn repeat(&mut self, reverse: bool, now: Duration) {
        self.value = 0.0;
        self.status = AnimationStatus::Forward;
        self.drive = Some(Drive::Timed {
            from: 0.0,
            to: 1.0,
            started: now,
            duration: self.duration,
            repeat: Some(reverse),
        });
    }

    /// Hand the value to a simulation — a spring settling, a fling decaying.
    ///
    /// This is the drag-release path: the gesture supplies the release velocity,
    /// and how long the motion takes is a consequence of the physics rather than
    /// something the interface picked. The curve is not applied; a simulation is
    /// its own curve.
    pub fn animate_with(&mut self, motion: impl Into<Motion>, now: Duration) {
        let motion = motion.into();
        let duration = motion.duration();
        self.value = motion.position(Duration::ZERO);
        self.status = if motion.position(duration) >= self.value {
            AnimationStatus::Forward
        } else {
            AnimationStatus::Reverse
        };
        self.drive = Some(Drive::Simulated {
            motion,
            started: now,
            duration,
        });
    }

    /// Critically damped settle from here to `target`, carrying `velocity` in
    /// value-units per second.
    pub fn spring_to(&mut self, target: f32, velocity: f32, now: Duration) {
        self.animate_with(crate::Spring::settling(self.value, target, velocity), now);
    }

    /// Put the value somewhere directly, stopping whatever was moving it.
    ///
    /// What a drag does: while a finger is down it *is* the animation, and any
    /// controller-driven motion has to yield to it rather than fight it.
    pub fn set_value(&mut self, value: f32) {
        self.drive = None;
        self.status = Self::resting_status(value);
        self.value = value;
    }

    /// Stop where it is, keeping the value.
    pub const fn stop(&mut self) {
        self.drive = None;
    }

    /// Stop and return to 0.
    pub fn reset(&mut self) {
        self.set_value(0.0);
    }

    /// Land on wherever this animation was going, without travelling.
    ///
    /// The reduced-motion path — see [`Ticker::settle`](crate::Ticker::settle).
    /// A finite run lands on its destination and stops. A **repeat** does not:
    /// an indefinite loop has no destination, and a spinner or a pulse frozen
    /// at one value is not a reduced animation, it is a broken one. Those keep
    /// running, which is what every platform's own reduced-motion mode does
    /// with a progress indicator.
    pub fn settle(&mut self, now: Duration) -> bool {
        match self.drive {
            Some(Drive::Timed {
                to, repeat: None, ..
            }) => {
                let moved = (self.value - to).abs() > f32::EPSILON;
                self.value = to;
                self.drive = None;
                self.status = Self::resting_status(to);
                moved || self.status != AnimationStatus::Dismissed
            }
            // A simulation has no declared destination to jump to — it stops
            // where the physics stops — so it is asked for its own end state
            // rather than guessed at. `duration` is what the simulation
            // answered when it started, so sampling there is its resting value.
            Some(Drive::Simulated { motion, .. }) => {
                let end = motion.final_position();
                let moved = (self.value - end).abs() > f32::EPSILON;
                self.value = end;
                self.drive = None;
                self.status = Self::resting_status(end);
                moved
            }
            // A repeat, or nothing running at all.
            _ => self.tick(now),
        }
    }

    /// Advance to `now`.
    ///
    /// Returns `true` if the value may have changed and whatever displays it
    /// needs to be rebuilt. The tick that *finishes* an animation returns `true`
    /// as well — the final value still has to reach the screen — and the one
    /// after it returns `false`, which is how an idle interface stops costing
    /// frames.
    pub fn tick(&mut self, now: Duration) -> bool {
        let Some(drive) = self.drive else {
            return false;
        };

        match drive {
            Drive::Timed {
                from,
                to,
                started,
                duration,
                repeat,
            } => {
                let elapsed = now.saturating_sub(started);
                if duration.is_zero() {
                    // Nothing to interpolate over, and a repeat of it would be
                    // an infinite number of cycles per frame. Land, and stop.
                    self.value = to;
                    self.drive = None;
                    self.status = Self::resting_status(to);
                    return true;
                }
                let cycles = elapsed.as_secs_f32() / duration.as_secs_f32();

                match repeat {
                    // Modulo rather than restarting on each completion: a repeat
                    // that re-bases its start time drifts by whatever the frame
                    // overshot by, every cycle, and a run of dropped frames
                    // would stretch a spinner instead of skipping ahead in it.
                    Some(ping_pong) => {
                        let turn = cycles.floor();
                        let phase = cycles - turn;
                        let t = if ping_pong && (turn as i64) % 2 == 1 {
                            1.0 - phase
                        } else {
                            phase
                        };
                        self.value = from + (to - from) * self.curve.transform(t);
                    }
                    None if cycles >= 1.0 => {
                        self.value = to;
                        self.drive = None;
                        self.status = Self::resting_status(to);
                    }
                    None => {
                        self.value = from + (to - from) * self.curve.transform(cycles);
                    }
                }
            }
            Drive::Simulated {
                motion,
                started,
                duration,
            } => {
                let elapsed = now.saturating_sub(started);
                if elapsed >= duration {
                    self.value = motion.final_position();
                    self.drive = None;
                    self.status = Self::resting_status(self.value);
                } else {
                    self.value = motion.position(elapsed);
                }
            }
        }
        true
    }

    /// The status of a value that has stopped moving.
    ///
    /// Anything strictly inside the range counts as `Forward`: it is neither at
    /// the start nor at the end, and "dismissed" would be a lie about a card
    /// left half open.
    const fn resting_status(value: f32) -> AnimationStatus {
        if value <= 0.0 {
            AnimationStatus::Dismissed
        } else if value >= 1.0 {
            AnimationStatus::Completed
        } else {
            AnimationStatus::Forward
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fling, Spring};

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn controller() -> AnimationController {
        AnimationController::new(ms(200))
    }

    /// Run a controller at 60Hz from `now` until it stops, collecting values.
    fn run(controller: &mut AnimationController, mut now: Duration) -> Vec<f32> {
        let mut values = vec![controller.value()];
        while controller.tick(now) {
            values.push(controller.value());
            now += ms(16);
            assert!(now < ms(30_000), "animation never finished");
        }
        values
    }

    #[test]
    fn a_fresh_controller_is_dismissed_and_costs_no_frames() {
        let mut controller = controller();

        assert_eq!(controller.value(), 0.0);
        assert_eq!(controller.status(), AnimationStatus::Dismissed);
        assert!(!controller.is_animating());
        assert!(
            !controller.tick(ms(1000)),
            "ticking an idle controller must report no work, or an idle screen \
             keeps asking for frames forever"
        );
    }

    #[test]
    fn running_forward_crosses_the_range_over_the_duration() {
        let mut controller = controller();
        controller.forward(ms(0));

        assert_eq!(controller.status(), AnimationStatus::Forward);
        controller.tick(ms(100));
        assert!(
            (controller.value() - 0.5).abs() < 0.01,
            "{}",
            controller.value()
        );

        controller.tick(ms(200));
        assert_eq!(controller.value(), 1.0);
        assert_eq!(controller.status(), AnimationStatus::Completed);
    }

    #[test]
    fn the_tick_that_finishes_an_animation_still_reports_a_change() {
        let mut controller = controller();
        controller.forward(ms(0));

        assert!(
            controller.tick(ms(500)),
            "the frame that lands on the end has to draw the end, so it must \
             report work even though the animation is over"
        );
        assert!(
            !controller.tick(ms(516)),
            "and the one after it must not, or the animation never stops"
        );
    }

    #[test]
    fn a_curve_changes_the_shape_without_changing_the_endpoints() {
        let mut eased = AnimationController::new(ms(200)).curve(Curve::EASE_IN);
        let mut linear = controller();
        eased.forward(ms(0));
        linear.forward(ms(0));

        eased.tick(ms(50));
        linear.tick(ms(50));
        assert!(
            eased.value() < linear.value(),
            "an ease-in must be behind linear a quarter of the way through: \
             {} vs {}",
            eased.value(),
            linear.value()
        );

        eased.tick(ms(200));
        linear.tick(ms(200));
        assert_eq!(eased.value(), linear.value(), "and they arrive together");
    }

    #[test]
    fn reversing_from_half_way_takes_half_as_long() {
        let mut controller = controller();
        controller.forward(ms(0));
        controller.tick(ms(100));
        assert!((controller.value() - 0.5).abs() < 0.01);

        controller.reverse(ms(100));
        assert_eq!(controller.status(), AnimationStatus::Reverse);
        controller.tick(ms(200));

        assert_eq!(
            controller.value(),
            0.0,
            "a fixed duration regardless of distance makes a quick toggle crawl"
        );
        assert_eq!(controller.status(), AnimationStatus::Dismissed);
    }

    #[test]
    fn animating_to_where_it_already_is_does_nothing_at_all() {
        let mut controller = controller();
        controller.forward(ms(0));
        controller.tick(ms(500));

        controller.forward(ms(500));
        assert!(
            !controller.is_animating(),
            "a no-op animation must not book a frame"
        );
        assert_eq!(controller.status(), AnimationStatus::Completed);
    }

    #[test]
    fn a_dropped_frame_skips_ahead_rather_than_stretching_the_animation() {
        let mut controller = controller();
        controller.forward(ms(0));

        // The frame at 100ms never happened; the next one is at 150ms.
        controller.tick(ms(150));

        assert!(
            (controller.value() - 0.75).abs() < 0.01,
            "the value is a function of the timestamp, not of how many ticks \
             have gone by: {}",
            controller.value()
        );
    }

    #[test]
    fn a_repeat_never_finishes_and_never_drifts() {
        let mut controller = controller();
        controller.repeat(false, ms(0));

        assert!(controller.tick(ms(100)));
        assert!((controller.value() - 0.5).abs() < 0.01);
        // Two and a half cycles later, exactly: no accumulated overshoot.
        assert!(controller.tick(ms(500)));
        assert!(
            (controller.value() - 0.5).abs() < 0.01,
            "{}",
            controller.value()
        );
        assert!(
            controller.is_animating(),
            "a repeat has no end to arrive at"
        );

        controller.stop();
        assert!(!controller.is_animating());
    }

    #[test]
    fn a_ping_pong_repeat_comes_back_on_the_odd_cycles() {
        let mut controller = controller();
        controller.repeat(true, ms(0));

        controller.tick(ms(100));
        assert!((controller.value() - 0.5).abs() < 0.01);
        // 300ms is half way through the second cycle, which runs backwards.
        controller.tick(ms(300));
        assert!(
            (controller.value() - 0.5).abs() < 0.01,
            "{}",
            controller.value()
        );
        controller.tick(ms(250));
        assert!(
            (controller.value() - 0.75).abs() < 0.01,
            "a quarter into the reverse leg is three quarters of the way up: {}",
            controller.value()
        );
    }

    #[test]
    fn setting_the_value_stops_whatever_was_moving_it() {
        let mut controller = controller();
        controller.forward(ms(0));

        controller.set_value(0.3);
        assert!(
            !controller.is_animating(),
            "a finger taking hold of a card must win against the animation \
             it interrupted"
        );
        assert_eq!(controller.value(), 0.3);
        assert_eq!(controller.status(), AnimationStatus::Forward);
    }

    #[test]
    fn a_spring_settles_at_its_target_and_stops_asking_for_frames() {
        let mut controller = controller().starting_at(0.6);
        controller.spring_to(0.0, 0.0, ms(0));

        let values = run(&mut controller, ms(0));

        assert!(!controller.is_animating());
        assert!(controller.value().abs() < 0.01, "{}", controller.value());
        assert_eq!(controller.status(), AnimationStatus::Dismissed);
        assert!(
            values.len() > 10,
            "a settle that resolves in three frames is a jump: {} frames",
            values.len()
        );
    }

    #[test]
    fn a_spring_does_not_move_at_a_constant_speed() {
        let mut controller = controller().starting_at(1.0);
        controller.spring_to(0.0, 0.0, ms(0));
        let values = run(&mut controller, ms(0));

        let middle = values.len() / 4;
        let early = values[middle] - values[middle + 1];
        let late = values[values.len() - 3] - values[values.len() - 2];
        assert!(
            early > late * 3.0,
            "a spring has to visibly decelerate, or it is a linear tween in a \
             costume: {early} then {late}"
        );
    }

    #[test]
    fn a_simulation_may_take_the_value_outside_the_unit_range() {
        let mut controller = controller();
        // Released at rest but moving hard: it has to travel past the target
        // before the spring pulls it back, which is the whole reason a gesture
        // hands its velocity over.
        controller.animate_with(Spring::settling(0.0, 0.0, 4.0), ms(0));
        let values = run(&mut controller, ms(0));

        let peak = values.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            peak > 0.1,
            "a released flick must carry past where it was let go: {peak}"
        );
        assert!(controller.value().abs() < 0.01, "and then come home");
    }

    #[test]
    fn a_fling_can_drive_a_controller_too() {
        let mut controller = controller();
        // A fling's "slow enough to stop" threshold is `MIN_FLING_VELOCITY`,
        // which is in pixels per second — so a fling driving a 0..1 controller
        // has to be given a velocity on that scale and the value scaled back by
        // whatever tween reads it. This is the case a spring is usually the
        // better fit for; the test is here because the controller must not care
        // which simulation it was handed.
        controller.animate_with(Fling::new(0.0, 900.0, 0.000_015_5), ms(0));

        let values = run(&mut controller, ms(0));
        assert!(controller.value() > 10.0, "{}", controller.value());
        assert!(values.windows(2).all(|pair| pair[1] >= pair[0] - 1e-4));
    }

    #[test]
    fn a_zero_duration_animation_lands_on_the_first_tick() {
        let mut controller = AnimationController::new(Duration::ZERO);
        controller.forward(ms(0));

        assert!(controller.tick(ms(0)));
        assert_eq!(controller.value(), 1.0);
        assert!(!controller.is_animating(), "and does not spin");
    }

    #[test]
    fn a_tick_from_before_the_start_is_treated_as_the_start() {
        let mut controller = controller();
        controller.forward(ms(1000));

        // A vsync timestamp that predates the animation — a clock that went
        // backwards, or an event handled with a stale timestamp.
        controller.tick(ms(500));
        assert_eq!(controller.value(), 0.0, "and not a wild extrapolation");
    }
}
