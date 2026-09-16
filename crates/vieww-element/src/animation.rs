//! An animation whose value is a signal.
//!
//! # The join between two layers
//!
//! `vieww-animation` knows how a value moves over time and nothing about trees.
//! `vieww-element` knows which elements read which values. [`Animation`] is the
//! one type that needs both: it advances a controller each frame and writes the
//! result into a [`Signal`], so the elements that read it — and only those — are
//! marked pending. An animation therefore costs a rebuild of exactly the widgets
//! that display it, however large the rest of the tree is.
//!
//! That is the whole reason this lives here rather than in the animation crate:
//! nothing else about it needs the element layer.
//!
//! # Explicit, next to the implicit
//!
//! This is the controller you reach for when the animation is *yours* — a card
//! that follows a finger, a spinner, a staged entrance. When all you want is
//! "move to this new value smoothly", the widget layer's
//! [`AnimatedContainer`](vieww_widget::AnimatedContainer) does it with no
//! controller in sight.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{
    AnimationController, AnimationStatus, Curve, Lerp, Motion, Ticker, Tickers, Tween,
};

use crate::{Runtime, Signal};

/// A controller, a tween, and the signal their product is published to.
struct Driven<T: 'static> {
    controller: AnimationController,
    tween: Tween<T>,
    signal: Signal<T>,
}

impl<T: Lerp + 'static> Driven<T> {
    /// Publish where the animation currently is.
    fn publish(&self) {
        self.signal.set(self.tween.at(self.controller.value()));
    }
}

impl<T: Lerp + 'static> Ticker for Driven<T> {
    fn tick(&mut self, now: Duration) -> bool {
        let changed = self.controller.tick(now);
        if changed {
            self.publish();
        }
        changed
    }

    fn is_animating(&self) -> bool {
        self.controller.is_animating()
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Driven<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Animation")
            .field("value", &self.signal)
            .field("progress", &self.controller.value())
            .field("status", &self.controller.status())
            .finish()
    }
}

/// A value of type `T` that moves over time and rebuilds whoever reads it.
///
/// Cheap to clone — like a [`Signal`], every clone is a handle to the same
/// animation. Read it during a build with [`value`](Self::value) and the element
/// doing the reading is rebuilt for each frame of the animation and no other.
///
/// # Ownership, and why it stops on its own
///
/// [`attach`](Self::attach) registers the animation with the frame's tickers,
/// which hold it *weakly*. Drop the last handle — because the screen holding it
/// went away — and it stops being ticked, with nothing to unregister.
///
/// ```
/// use std::time::Duration;
/// use vieww_animation::{Tickers, Tween};
/// use vieww_element::{Animation, ElementTree};
///
/// let ms = Duration::from_millis;
/// let mut tree = ElementTree::new();
/// let mut tickers = Tickers::new();
///
/// let slide = Animation::new(tree.runtime(), Tween::new(0.0_f32, 100.0), ms(200));
/// slide.attach(&mut tickers);
///
/// slide.forward(ms(0));
/// tickers.advance(ms(100));
/// assert!((slide.peek() - 50.0).abs() < 0.01);
///
/// tickers.advance(ms(200));
/// assert_eq!(slide.peek(), 100.0);
/// assert!(!slide.is_animating());
/// ```
pub struct Animation<T: 'static> {
    inner: Rc<RefCell<Driven<T>>>,
    signal: Signal<T>,
}

impl<T: Lerp + 'static> Animation<T> {
    /// An animation over `tween`, taking `duration` to cross it, sitting at the
    /// start.
    ///
    /// The signal is created on `runtime`, so it must be the runtime of the tree
    /// that will display it — otherwise the write marks nothing.
    #[must_use]
    pub fn new(runtime: &Runtime, tween: Tween<T>, duration: Duration) -> Self {
        // Cloned rather than moved: `Lerp` is `Clone` rather than `Copy`, so
        // taking `begin` out would consume the tween this then stores.
        let signal = runtime.signal(tween.begin.clone());
        Self {
            inner: Rc::new(RefCell::new(Driven {
                controller: AnimationController::new(duration),
                tween,
                signal: signal.clone(),
            })),
            signal,
        }
    }

    /// Shape the timed motion with `curve`.
    #[must_use]
    pub fn curve(self, curve: Curve) -> Self {
        self.inner.borrow_mut().controller.set_curve(curve);
        self
    }

    /// Have `tickers` advance this animation once a frame.
    ///
    /// Held weakly, so this does not keep the animation alive; see the type's
    /// documentation.
    pub fn attach(&self, tickers: &mut Tickers) {
        tickers.add(&self.inner);
    }

    /// The current value, subscribing the element that is building.
    ///
    /// This is the read that makes an animation cost only what it moves: the
    /// elements that call it are the elements the next frame rebuilds.
    #[must_use]
    pub fn value(&self) -> T {
        self.signal.get()
    }

    /// The current value, without subscribing. For event handlers and tests.
    #[must_use]
    pub fn peek(&self) -> T {
        self.signal.peek()
    }

    /// The signal the value is published to, for handing to something that takes
    /// one.
    #[must_use]
    pub fn signal(&self) -> Signal<T> {
        self.signal.clone()
    }

    /// How far along the animation is: `0..=1` for timed motion, and wherever
    /// the simulation is for a physical one.
    #[must_use]
    pub fn progress(&self) -> f32 {
        self.inner.borrow().controller.value()
    }

    /// Where the animation is in its lifecycle.
    #[must_use]
    pub fn status(&self) -> AnimationStatus {
        self.inner.borrow().controller.status()
    }

    /// `true` while it still needs frames.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.inner.borrow().controller.is_animating()
    }

    /// How long a full traverse takes.
    #[must_use]
    pub fn duration(&self) -> Duration {
        self.inner.borrow().controller.duration()
    }

    /// Change how long a full traverse takes, from the next animation on.
    pub fn set_duration(&self, duration: Duration) {
        self.inner.borrow_mut().controller.set_duration(duration);
    }

    /// Run to the end of the tween.
    pub fn forward(&self, now: Duration) {
        self.inner.borrow_mut().controller.forward(now);
    }

    /// Run back to the beginning.
    pub fn reverse(&self, now: Duration) {
        self.inner.borrow_mut().controller.reverse(now);
    }

    /// Run to a point part way along, `0..=1`.
    pub fn animate_to(&self, progress: f32, now: Duration) {
        self.inner.borrow_mut().controller.animate_to(progress, now);
    }

    /// Settle to `progress` on a critically damped spring, carrying `velocity`
    /// in progress-units per second.
    ///
    /// The drag-release path: hand it the velocity the gesture reported, scaled
    /// into the same units the progress is in, and where it goes and how long it
    /// takes are the physics' business rather than a duration somebody picked.
    pub fn spring_to(&self, progress: f32, velocity: f32, now: Duration) {
        self.inner
            .borrow_mut()
            .controller
            .spring_to(progress, velocity, now);
    }

    /// Hand the progress to a simulation.
    pub fn animate_with(&self, motion: impl Into<Motion>, now: Duration) {
        self.inner.borrow_mut().controller.animate_with(motion, now);
    }

    /// Loop forever, ping-ponging if `reverse`.
    pub fn repeat(&self, reverse: bool, now: Duration) {
        self.inner.borrow_mut().controller.repeat(reverse, now);
    }

    /// Put the progress somewhere directly, stopping any motion, and publish it.
    ///
    /// What a drag does: while the finger is down it *is* the animation. The
    /// value reaches the signal immediately rather than at the next tick,
    /// because a finger that has already moved must not wait a frame to be
    /// followed.
    pub fn set_progress(&self, progress: f32) {
        let inner = self.inner.borrow_mut();
        let mut inner = inner;
        inner.controller.set_value(progress);
        inner.publish();
    }

    /// Stop where it is.
    pub fn stop(&self) {
        self.inner.borrow_mut().controller.stop();
    }
}

impl<T: 'static> Clone for Animation<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
            signal: self.signal.clone(),
        }
    }
}

impl<T: fmt::Debug + 'static> fmt::Debug for Animation<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.borrow().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use vieww_animation::Spring;
    use vieww_foundation::Color;

    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// An animation over 0..100 attached to a fresh set of tickers.
    fn harness() -> (Runtime, Tickers, Animation<f32>) {
        let runtime = Runtime::new();
        let mut tickers = Tickers::new();
        let animation = Animation::new(&runtime, Tween::new(0.0_f32, 100.0), ms(200));
        animation.attach(&mut tickers);
        (runtime, tickers, animation)
    }

    #[test]
    fn an_animation_publishes_its_value_to_a_signal_as_it_runs() {
        let (_runtime, mut tickers, slide) = harness();
        assert_eq!(slide.peek(), 0.0);

        slide.forward(ms(0));
        tickers.advance(ms(100));
        assert!((slide.peek() - 50.0).abs() < 0.01, "{}", slide.peek());

        tickers.advance(ms(200));
        assert_eq!(slide.peek(), 100.0);
        assert_eq!(slide.status(), AnimationStatus::Completed);
    }

    #[test]
    fn only_the_elements_that_read_the_value_are_marked_pending() {
        const READER: crate::ElementId = crate::ElementId::new(0, 0, 0);
        let (runtime, mut tickers, slide) = harness();

        runtime.push_tracking(READER);
        let _ = slide.value();
        runtime.pop_tracking();

        slide.forward(ms(0));
        tickers.advance(ms(50));

        assert_eq!(
            runtime.pending_ids(),
            vec![READER],
            "a frame of animation must cost a rebuild of what displays it and \
             nothing else"
        );
    }

    #[test]
    fn an_idle_animation_writes_nothing_and_marks_nobody_pending() {
        const READER: crate::ElementId = crate::ElementId::new(0, 0, 0);
        let (runtime, mut tickers, slide) = harness();

        runtime.push_tracking(READER);
        let _ = slide.value();
        runtime.pop_tracking();

        assert!(!tickers.advance(ms(50)));
        assert_eq!(
            runtime.pending_count(),
            0,
            "an animation nobody started must not keep the screen redrawing"
        );
    }

    #[test]
    fn a_drag_can_set_the_progress_and_be_followed_immediately() {
        let (_runtime, mut tickers, slide) = harness();
        slide.forward(ms(0));
        tickers.advance(ms(50));

        slide.set_progress(0.9);

        assert_eq!(
            slide.peek(),
            90.0,
            "a finger that has already moved must not wait a frame to be followed"
        );
        assert!(!slide.is_animating(), "and it takes the animation over");
    }

    #[test]
    fn releasing_into_a_spring_settles_the_value() {
        let (_runtime, mut tickers, slide) = harness();
        slide.set_progress(0.8);

        slide.spring_to(0.0, 0.0, ms(0));
        let mut now = ms(0);
        while tickers.advance(now) {
            now += ms(16);
            assert!(now < ms(10_000), "the settle never finished");
        }

        assert_eq!(slide.peek(), 0.0);
        assert!(
            now > ms(200),
            "and it took a visible amount of time: {now:?}"
        );
    }

    #[test]
    fn a_simulation_can_be_handed_over_directly() {
        let (_runtime, mut tickers, slide) = harness();
        slide.animate_with(Spring::settling(1.0, 0.0, 0.0), ms(0));

        tickers.advance(ms(50));
        let travelled = slide.peek();
        assert!(
            travelled < 100.0 && travelled > 0.0,
            "part way home: {travelled}"
        );
    }

    #[test]
    fn an_animation_works_over_any_lerp_type() {
        let runtime = Runtime::new();
        let mut tickers = Tickers::new();
        let tint = Animation::new(&runtime, Tween::new(Color::RED, Color::BLUE), ms(100));
        tint.attach(&mut tickers);

        tint.forward(ms(0));
        tickers.advance(ms(50));

        let mid = tint.peek();
        assert!(mid.r > 0 && mid.b > 0, "{mid}");
        tickers.advance(ms(100));
        assert_eq!(tint.peek(), Color::BLUE);
    }

    #[test]
    fn dropping_the_last_handle_stops_the_animation_being_ticked() {
        let runtime = Runtime::new();
        let mut tickers = Tickers::new();
        let slide = Animation::new(&runtime, Tween::new(0.0_f32, 100.0), ms(200));
        slide.attach(&mut tickers);
        slide.forward(ms(0));
        assert_eq!(tickers.len(), 1);

        drop(slide);

        assert!(
            !tickers.advance(ms(100)),
            "a screen that went away must stop costing frames without anything \
             having to remember to unregister it"
        );
        assert_eq!(tickers.len(), 0);
    }

    #[test]
    fn cloned_handles_drive_the_same_animation() {
        let (_runtime, mut tickers, slide) = harness();
        let handle = slide.clone();

        handle.forward(ms(0));
        tickers.advance(ms(200));

        assert_eq!(slide.peek(), 100.0);
        assert_eq!(tickers.len(), 1, "and there is still only one of it");
    }
}
