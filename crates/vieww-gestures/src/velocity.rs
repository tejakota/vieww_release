//! Estimating how fast a pointer was moving when it left the screen.
//!
//! # Why not just the last delta
//!
//! A fling's whole character comes from this number, and the obvious estimate —
//! the last move divided by the time since the one before — is dominated by
//! noise. Touch hardware reports jitter of a pixel or two per sample, and at
//! 120Hz that is a several-hundred-pixels-per-second error on the one sample that
//! decides the throw. Worse, the last sample before release is systematically
//! *slow*: fingers decelerate as they lift.
//!
//! So velocity is fitted over the recent samples instead of read off the last
//! one. A least-squares line through position against time within a short horizon
//! averages the jitter out while still tracking a genuine change of speed.

use std::time::Duration;

use vieww_foundation::Offset;

/// How far back samples are still considered.
///
/// Long enough that jitter averages out, short enough that a finger which
/// stopped dead before lifting reports a stop rather than the speed it had a
/// moment earlier. 100ms is the figure Android's platform uses.
pub const HORIZON: Duration = Duration::from_millis(100);

/// How many samples to keep. At 120Hz, 100ms is 12 of them.
const CAPACITY: usize = 20;

/// The largest speed a fling may claim, in logical pixels per second.
///
/// A sample pair arriving in a fraction of a millisecond can fit an arbitrarily
/// steep line, and an 80000px/s fling is not a gesture, it is a division by
/// something close to zero.
pub const MAX_VELOCITY: f32 = 8000.0;

#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Duration,
    position: Offset,
}

/// Collects pointer positions and fits a velocity through them.
#[derive(Debug, Default)]
pub struct VelocityTracker {
    samples: Vec<Sample>,
}

impl VelocityTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record where the pointer was, and when.
    pub fn add(&mut self, at: Duration, position: Offset) {
        if self.samples.len() == CAPACITY {
            self.samples.remove(0);
        }
        self.samples.push(Sample { at, position });
    }

    /// Forget everything. Call when a pointer ends.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// How many samples are being kept.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Pixels per second, fitted over the samples inside [`HORIZON`].
    ///
    /// Returns zero when there is nothing to fit — one sample, or several that
    /// all arrived at the same instant. Zero is the right answer there: an
    /// unknown velocity must not become an invented fling.
    #[must_use]
    pub fn velocity(&self) -> Offset {
        let Some(last) = self.samples.last() else {
            return Offset::ZERO;
        };

        let recent: Vec<&Sample> = self
            .samples
            .iter()
            .filter(|sample| last.at.saturating_sub(sample.at) <= HORIZON)
            .collect();
        if recent.len() < 2 {
            return Offset::ZERO;
        }

        // Times relative to the newest sample, so the numbers stay small however
        // long the app has been running. An f32 holding milliseconds since boot
        // loses the resolution this fit depends on.
        let seconds = |sample: &Sample| -(last.at.saturating_sub(sample.at).as_secs_f32());

        let count = recent.len() as f32;
        let mean_t: f32 = recent.iter().map(|s| seconds(s)).sum::<f32>() / count;
        let mut variance = 0.0;
        let mut covariance = Offset::ZERO;
        for sample in &recent {
            let dt = seconds(sample) - mean_t;
            variance += dt * dt;
            covariance = covariance + sample.position.scale(dt);
        }

        if variance <= f32::EPSILON {
            // Every sample landed at the same instant, so the line through them
            // is vertical and its slope is not a number.
            return Offset::ZERO;
        }

        let velocity = covariance.scale(1.0 / variance);
        clamp(velocity)
    }
}

/// Hold a velocity to a plausible magnitude, keeping its direction.
///
/// Clamped as a vector rather than per axis, so a diagonal fling is not bent
/// towards whichever axis happened to exceed the limit first.
#[must_use]
fn clamp(velocity: Offset) -> Offset {
    let speed = velocity.distance();
    if speed > MAX_VELOCITY {
        velocity.scale(MAX_VELOCITY / speed)
    } else {
        velocity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// A pointer moving at a constant `pixels_per_second` along x, sampled every
    /// 10ms for `count` samples.
    fn steady(tracker: &mut VelocityTracker, pixels_per_second: f32, count: u64) {
        for step in 0..count {
            let seconds = step as f32 * 0.01;
            tracker.add(ms(step * 10), Offset::new(pixels_per_second * seconds, 0.0));
        }
    }

    #[test]
    fn a_steady_drag_reports_the_speed_it_was_going() {
        let mut tracker = VelocityTracker::new();
        steady(&mut tracker, 500.0, 10);

        let velocity = tracker.velocity();
        assert!(
            (velocity.dx - 500.0).abs() < 1.0,
            "expected about 500px/s, got {velocity}"
        );
        assert!(velocity.dy.abs() < 1.0);
    }

    #[test]
    fn jitter_on_the_last_sample_does_not_dominate() {
        let mut clean = VelocityTracker::new();
        steady(&mut clean, 400.0, 10);

        let mut noisy = VelocityTracker::new();
        steady(&mut noisy, 400.0, 9);
        // One sample two pixels off, which is ordinary touch noise. Read as a
        // last-delta it is 200px/s of error.
        noisy.add(ms(90), Offset::new(400.0 * 0.09 + 2.0, 0.0));

        let difference = (noisy.velocity().dx - clean.velocity().dx).abs();
        assert!(
            difference < 60.0,
            "a 2px blip moved the estimate by {difference}px/s — a fit over the \
             recent samples is the whole reason this is not the last delta"
        );
    }

    #[test]
    fn samples_older_than_the_horizon_are_ignored() {
        let mut tracker = VelocityTracker::new();
        // Moving fast, long ago.
        tracker.add(ms(0), Offset::new(0.0, 0.0));
        tracker.add(ms(10), Offset::new(100.0, 0.0));
        // Then slowly, recently.
        for step in 0..5 {
            tracker.add(ms(500 + step * 10), Offset::new(100.0 + step as f32, 0.0));
        }

        let velocity = tracker.velocity();
        assert!(
            velocity.dx < 400.0,
            "the old fast samples are outside the horizon: {velocity}"
        );
    }

    #[test]
    fn a_finger_that_stopped_before_lifting_reports_a_stop() {
        let mut tracker = VelocityTracker::new();
        steady(&mut tracker, 1000.0, 5);
        // Held still for the rest of the horizon.
        for step in 0..8 {
            tracker.add(ms(50 + step * 10), Offset::new(40.0, 0.0));
        }

        assert!(
            tracker.velocity().dx.abs() < 200.0,
            "lifting after a pause must not fling: {}",
            tracker.velocity()
        );
    }

    #[test]
    fn one_sample_is_not_a_velocity() {
        let mut tracker = VelocityTracker::new();
        tracker.add(ms(0), Offset::new(10.0, 10.0));

        assert_eq!(
            tracker.velocity(),
            Offset::ZERO,
            "an unknown velocity must not become an invented fling"
        );
    }

    #[test]
    fn samples_at_the_same_instant_do_not_divide_by_zero() {
        let mut tracker = VelocityTracker::new();
        tracker.add(ms(5), Offset::new(0.0, 0.0));
        tracker.add(ms(5), Offset::new(50.0, 0.0));

        assert_eq!(tracker.velocity(), Offset::ZERO);
    }

    #[test]
    fn velocity_is_clamped_as_a_vector_so_a_diagonal_keeps_its_direction() {
        let mut tracker = VelocityTracker::new();
        tracker.add(ms(0), Offset::ZERO);
        tracker.add(ms(1), Offset::new(1000.0, 1000.0));

        let velocity = tracker.velocity();
        assert!(velocity.distance() <= MAX_VELOCITY + 1.0, "{velocity}");
        assert!(
            (velocity.dx - velocity.dy).abs() < 1.0,
            "clamping per axis would bend a diagonal fling: {velocity}"
        );
    }

    #[test]
    fn the_sample_buffer_is_bounded() {
        let mut tracker = VelocityTracker::new();
        for step in 0..200 {
            tracker.add(ms(step), Offset::new(step as f32, 0.0));
        }
        assert!(tracker.len() <= CAPACITY);
    }
}
