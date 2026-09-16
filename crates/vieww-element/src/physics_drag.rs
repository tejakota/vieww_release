//! A physics-driven drag with snap points.
//!
//! # What it is
//!
//! The engine behind pickers, carousels, and swipeable panels: drag
//! freely, release, and the position springs to the nearest stop with
//! the release velocity carried through. A fast flick skips stops; a
//! slow drag settles on the adjacent one.
//!
//! # Why here and not vieww-gestures
//!
//! The drag's position is a [`Signal`] — it needs the reactive layer,
//! which is this crate. `vieww-gestures` sits below us and cannot name
//! `Signal`. The velocity tracking is simple enough to implement inline
//! (a sliding window with a least-squares fit) that pulling in a
//! dependency would cost more than it saves.
//!
//! # The two speeds of a drag
//!
//! **While dragging:** the position tracks the finger exactly, no
//! spring in the way. A spring that fights the finger is the classic
//! "laggy scroll" feel and no amount of tuning fixes it.
//!
//! **On release:** the spring takes over, starting from the finger's
//! velocity and pulling toward the nearest snap point.

use std::time::Duration;

use vieww_animation::{SpringAnimation as Spring, SpringPreset, Ticker};

use crate::{Runtime, Signal};

/// Below this velocity (units/second), a release is "at rest".
const VELOCITY_EPSILON: f32 = 1.0;

/// A physics-driven drag over snap points.
///
/// # Examples
///
/// ```ignore
/// let runtime = Runtime::new();
/// let mut drag = PhysicsDrag::new(
///     &runtime,
///     0.0,
///     vec![0.0, 100.0, 200.0],
///     SpringPreset::Expressive,
/// );
///
/// drag.drag_begin();
/// drag.drag_to(90.0);
/// drag.drag_end(150.0); // velocity toward the second snap
///
/// assert_eq!(drag.target(), 100.0);
/// ```
#[derive(Debug)]
pub struct PhysicsDrag {
    /// The position, as a signal the UI reads.
    position: Signal<f32>,
    /// The spring that takes over on release.
    spring: Spring,
    /// Where the spring is heading (the chosen snap point).
    target: Signal<f32>,
    /// Candidate snap positions, sorted.
    snap_points: Vec<f32>,
    /// `true` while the finger is down.
    dragging: bool,
    /// Position when the drag began.
    drag_start_position: f32,
    /// Last pointer position, for delta computation.
    last_pointer: f32,
    /// Recent pointer positions with timestamps, for velocity estimation.
    history: Vec<(Duration, f32)>,
    /// The last tick time, for the spring.
    last_tick: Option<Duration>,
}

impl PhysicsDrag {
    /// Create a drag over `snap_points` starting at `initial`.
    ///
    /// The snap points need not include the initial position; the first
    /// release will spring to the nearest one.
    #[must_use]
    pub fn new(
        runtime: &Runtime,
        initial: f32,
        snap_points: Vec<f32>,
        preset: SpringPreset,
    ) -> Self {
        let mut sorted = snap_points;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let position = runtime.signal(initial);
        let target = runtime.signal(initial);

        let mut spring = Spring::new(initial, preset);
        spring.jump(initial);

        Self {
            position,
            spring,
            target,
            snap_points: sorted,
            dragging: false,
            drag_start_position: initial,
            last_pointer: initial,
            history: Vec::new(),
            last_tick: None,
        }
    }

    /// The signal holding the current position.
    ///
    /// Read this in a widget's `build` to follow the drag reactively.
    #[must_use]
    pub fn position_signal(&self) -> Signal<f32> {
        self.position.clone()
    }

    /// The current position, without subscribing.
    #[must_use]
    pub fn position(&self) -> f32 {
        self.position.peek()
    }

    /// The snap point the spring is heading toward.
    #[must_use]
    pub fn target(&self) -> f32 {
        self.target.peek()
    }

    /// The finger went down at `position`.
    ///
    /// Cancels any in-flight spring: the user has taken manual control.
    pub fn drag_begin(&mut self) {
        self.dragging = true;
        self.drag_start_position = self.position.peek();
        self.last_pointer = self.position.peek();
        self.history.clear();
    }

    /// The finger moved. `position` is the new absolute position.
    ///
    /// During a drag the position tracks directly — no spring, no smoothing.
    pub fn drag_to(&mut self, position: f32) {
        if !self.dragging {
            return;
        }
        self.last_pointer = position;
        self.position.set(position);
    }

    /// The finger moved by `delta`. Convenience for delta-based input.
    pub fn drag_by(&mut self, delta: f32) {
        if !self.dragging {
            return;
        }
        let new_pos = self.last_pointer + delta;
        self.drag_to(new_pos);
    }

    /// Record a pointer sample with a timestamp, for velocity estimation.
    ///
    /// Call this alongside `drag_to` or `drag_by` if you have access to
    /// a frame timestamp.
    pub fn record_sample(&mut self, at: Duration) {
        if self.dragging {
            self.history.push((at, self.last_pointer));
            // Keep the last ~16 samples (~267ms at 60fps).
            if self.history.len() > 16 {
                self.history.remove(0);
            }
        }
    }

    /// The finger lifted, moving at `velocity` units/second.
    ///
    /// The velocity decides how far the fling carries: the snap point
    /// is chosen based on both position and velocity.
    pub fn drag_end(&mut self, velocity: f32) {
        if !self.dragging {
            return;
        }
        self.dragging = false;

        let current = self.position.peek();
        let target = self.choose_snap_point(current, velocity);

        self.target.set(target);

        // The spring starts from where the finger left it, with the
        // finger's velocity.
        let spec = vieww_animation::SpringSpec::new(380.0)
            .damping_ratio(0.9)
            .initial_velocity(velocity);
        self.spring = Spring::with_spec(current, spec);
        self.spring.retarget(target);
    }

    /// The finger lifted. Estimate velocity from the recorded samples.
    pub fn drag_end_tracked(&mut self) {
        let v = self.estimate_velocity();
        self.drag_end(v);
    }

    /// The gesture was cancelled.
    ///
    /// Springs back to the current target rather than holding position.
    pub fn drag_cancel(&mut self) {
        self.dragging = false;

        // Move the spring to where the finger actually left the handle before
        // retargeting it. During a drag only `position` moves — `drag_to`
        // writes the signal and deliberately leaves the spring alone — so the
        // spring is still sitting whereever it settled last time. Retargeting
        // it without this `jump` asks a spring already at its target to travel
        // to that same target, which is no animation at all: the handle stays
        // stranded under the finger and never springs back. `drag_end` gets
        // this right by rebuilding the spring from `current`.
        let current = self.position.peek();
        let target = self.target.peek();
        self.spring.jump(current);
        self.spring.retarget(target);
    }

    /// Estimate velocity from the sample history (least-squares fit).
    #[must_use]
    pub fn estimate_velocity(&self) -> f32 {
        if self.history.len() < 2 {
            return 0.0;
        }

        let n = self.history.len() as f32;
        let t0 = self.history[0].0;
        let mut sum_t = 0.0;
        let mut sum_x = 0.0;
        let mut sum_tt = 0.0;
        let mut sum_tx = 0.0;

        for (at, pos) in &self.history {
            let t = at.saturating_sub(t0).as_secs_f32();
            sum_t += t;
            sum_x += *pos;
            sum_tt += t * t;
            sum_tx += t * *pos;
        }

        let denom = n * sum_tt - sum_t * sum_t;
        if denom.abs() < f32::EPSILON {
            return 0.0;
        }

        (n * sum_tx - sum_t * sum_x) / denom
    }

    /// Choose the snap point to settle on, given release position and velocity.
    ///
    /// The rule: velocity projects the position forward by ~250ms of
    /// decaying motion, and the nearest snap point to the projected
    /// position wins. A velocity near zero is the plain "nearest snap
    /// point to where I am".
    fn choose_snap_point(&self, position: f32, velocity: f32) -> f32 {
        if self.snap_points.is_empty() {
            return position;
        }

        // Project forward by how far the momentum would carry.
        let velocity = if velocity.abs() < VELOCITY_EPSILON {
            0.0
        } else {
            velocity
        };
        let projected = position + velocity * 0.25;

        self.snap_points
            .iter()
            .copied()
            .min_by(|a, b| {
                (a - projected)
                    .abs()
                    .partial_cmp(&(b - projected).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(position)
    }
}

impl Ticker for PhysicsDrag {
    fn tick(&mut self, now: Duration) -> bool {
        // During a drag the spring is not running; the position is set
        // directly by drag_to/drag_by. The ticker stays "animating"
        // because the position may change at any moment.
        if self.dragging {
            self.last_tick = Some(now);
            return true;
        }

        let Some(last) = self.last_tick else {
            self.last_tick = Some(now);
            return self.spring.is_animating();
        };

        let _ = now.saturating_sub(last); // The Spring handles its own dt.
        self.last_tick = Some(now);

        if self.spring.tick(now) {
            self.position.set(self.spring.value());
            true
        } else {
            // Ensure the final value is written.
            self.position.set(self.spring.target());
            false
        }
    }

    fn is_animating(&self) -> bool {
        self.dragging || self.spring.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIXTY_FPS: Duration = Duration::from_millis(16);

    fn runtime() -> crate::Runtime {
        crate::Runtime::new()
    }

    #[test]
    fn a_slow_release_snaps_to_the_nearest_point() {
        let rt = runtime();
        let mut drag =
            PhysicsDrag::new(&rt, 0.0, vec![0.0, 100.0, 200.0], SpringPreset::Expressive);

        drag.drag_begin();
        drag.drag_to(95.0);
        drag.drag_end(0.0); // released with no velocity

        assert_eq!(drag.target(), 100.0, "nearest to 95");
    }

    #[test]
    fn a_fast_release_carries_past_the_nearest_point() {
        let rt = runtime();
        let mut drag =
            PhysicsDrag::new(&rt, 0.0, vec![0.0, 100.0, 200.0], SpringPreset::Expressive);

        drag.drag_begin();
        drag.drag_to(95.0);
        drag.drag_end(400.0); // flicking hard

        // Projected: 95 + 400*0.25 = 195, nearest is 200.
        assert_eq!(drag.target(), 200.0, "velocity carried past 100");
    }

    #[test]
    fn a_release_with_negative_velocity_goes_backward() {
        let rt = runtime();
        let mut drag = PhysicsDrag::new(
            &rt,
            100.0,
            vec![0.0, 100.0, 200.0],
            SpringPreset::Expressive,
        );

        drag.drag_begin();
        drag.drag_to(105.0);
        drag.drag_end(-400.0); // flicking back

        // Projected: 105 - 400*0.25 = 5, nearest is 0.
        assert_eq!(drag.target(), 0.0);
    }

    #[test]
    fn drag_cancel_springs_back_to_target() {
        let rt = runtime();
        let mut drag = PhysicsDrag::new(&rt, 0.0, vec![0.0, 100.0], SpringPreset::Standard);

        drag.drag_begin();
        drag.drag_to(50.0);
        drag.drag_cancel();

        assert!(!drag.dragging);
        assert!(drag.is_animating(), "spring is pulling back");
    }

    #[test]
    fn the_drag_ticks_until_settled() {
        let rt = runtime();
        let mut drag = PhysicsDrag::new(&rt, 0.0, vec![0.0, 100.0], SpringPreset::Standard);

        drag.drag_begin();
        drag.drag_to(50.0);
        drag.drag_end(100.0); // hard flick toward 100

        let mut t = Duration::ZERO;
        let mut still_running = false;
        for _ in 0..120 {
            t += SIXTY_FPS;
            if drag.tick(t) {
                still_running = true;
            } else {
                still_running = false;
                break;
            }
        }

        assert!(!still_running, "the drag settled");
        assert!(
            (drag.position() - 100.0).abs() < 0.1,
            "arrived at the snap point: {}",
            drag.position()
        );
    }

    #[test]
    fn drag_by_accumulates() {
        let rt = runtime();
        let mut drag = PhysicsDrag::new(&rt, 0.0, vec![0.0, 100.0], SpringPreset::Standard);

        drag.drag_begin();
        drag.drag_by(10.0);
        drag.drag_by(10.0);
        drag.drag_by(10.0);

        assert!((drag.position() - 30.0).abs() < 0.001);
    }
}
