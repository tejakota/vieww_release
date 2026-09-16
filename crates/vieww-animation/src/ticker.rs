//! Who gets told that a frame happened.
//!
//! # Weakly held, on purpose
//!
//! [`Tickers`] keeps a [`Weak`] handle to everything it drives, so an animation
//! stops being ticked the moment the last real owner drops it. The alternative —
//! a strong reference plus an `unregister` call — makes a leak the *default*
//! outcome of forgetting one line, and the thing that gets forgotten is the
//! teardown path of a screen that was closed, which is exactly where nobody
//! looks. Here, a controller owned by an element that unmounts is gone from the
//! registry by the next frame, without anything having to remember it.
//!
//! # Not a subscription system
//!
//! A ticker is asked "did your value change" once per frame and answers `true` or
//! `false`. It does not push to listeners. That keeps the reactive graph the
//! element layer's business — an animation's value reaches the tree by being
//! written into a signal, which marks exactly the elements that read it pending —
//! and it keeps this crate free of any notion of a tree at all.

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::Duration;

use crate::AnimationController;

/// Something a frame can advance.
pub trait Ticker {
    /// Advance to `now`; `true` if the value may have changed.
    fn tick(&mut self, now: Duration) -> bool;

    /// `true` while this still needs frames.
    fn is_animating(&self) -> bool;

    /// The earliest time a frame would change this ticker's value.
    ///
    /// `None` — the default — means **as soon as possible**: an animation
    /// sampled from a curve produces a different value on every frame, so the
    /// only honest answer is "the next one". A ticker that changes on a
    /// *schedule* rather than continuously can say so, and the event loop can
    /// then sleep until then instead of spinning.
    ///
    /// # The defect this exists for
    ///
    /// A caret blinks twice a second and answers [`is_animating`] `true` for as
    /// long as there is a caret. With no deadline to offer, "something is
    /// animating" was the whole of the loop's information, so an idle editor
    /// window with a caret in it **never slept**: one core at 100%,
    /// indefinitely, with nothing on screen changing but a caret. Measured on
    /// an untouched window and pinned by `viewwstudio`'s `tests/idle_cost.rs`.
    ///
    /// Answering `Some(next_toggle)` turns that into two frames a second.
    ///
    /// [`is_animating`]: Ticker::is_animating
    fn next_deadline(&self, _now: Duration) -> Option<Duration> {
        None
    }

    /// Arrive at the end **now**, because the person asked for less motion.
    ///
    /// Called by [`Tickers::advance`] in place of [`tick`](Ticker::tick) when
    /// [`Accessibility::reduce_motion`](vieww_foundation::Accessibility::reduce_motion)
    /// is set. Returns the same `bool` as `tick`: whether the value changed.
    ///
    /// # Why this is a settle rather than a shorter duration
    ///
    /// "Reduce motion" is not "the same journey, faster". A spring scaled to a
    /// tenth of its duration still travels its whole arc, and it is the *travel*
    /// — the parallax, the zoom, the slide across the viewport — that provokes
    /// vestibular symptoms, not the number of milliseconds it takes. Every
    /// platform's own implementation replaces the motion with a cut or a
    /// cross-fade for the same reason. So the value lands on its target on the
    /// first frame, the animation reports settled, and the frame loop goes back
    /// to sleep.
    ///
    /// # Why the default keeps ticking
    ///
    /// A `Ticker` outside this crate might be driving something that is not
    /// motion at all — a caret blink, a polling clock, a progress readout — and
    /// silently freezing it would be a worse bug than the one being fixed. The
    /// default is therefore *unchanged behaviour*, and the three tickers the
    /// framework ships that genuinely animate position override it.
    fn settle(&mut self, now: Duration) -> bool {
        self.tick(now)
    }
}

impl Ticker for AnimationController {
    fn tick(&mut self, now: Duration) -> bool {
        Self::tick(self, now)
    }

    fn is_animating(&self) -> bool {
        Self::is_animating(self)
    }

    fn settle(&mut self, now: Duration) -> bool {
        Self::settle(self, now)
    }
}

/// Everything being advanced by the frame scheduler.
///
/// The frame driver owns one of these and advances it during the frame's
/// *animate* phase — first in the frame, so that whatever an animation changes is
/// picked up by the *same* frame's build rather than trailing it by one.
#[derive(Default)]
pub struct Tickers {
    entries: Vec<Weak<RefCell<dyn Ticker>>>,
    /// Whether the person has asked their operating system for less movement.
    ///
    /// **Here rather than in a context, and consulted at tick time rather than
    /// at build time.** `Widget::create_state` takes no `BuildContext`, so an
    /// `AnimationController` is constructed where nothing ambient can be read —
    /// which is the structural reason the framework published this preference
    /// for two releases without honouring any of its own animations. Making it
    /// available at *construction* means giving state creation a context: a
    /// change to a trait every stateful widget implements.
    ///
    /// A registry that every framework animation already passes through once
    /// per frame does not need any of that. The frame driver publishes the
    /// preference here from `set_accessibility`, `advance` settles instead of
    /// stepping, and no widget opts in or even knows.
    reduce_motion: bool,
}

impl Tickers {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            reduce_motion: false,
        }
    }

    /// Publish the person's reduced-motion preference.
    ///
    /// Returns `true` if it changed. The frame driver calls this from
    /// `set_accessibility`; nothing else should.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) -> bool {
        let changed = self.reduce_motion != reduce_motion;
        self.reduce_motion = reduce_motion;
        changed
    }

    /// Whether motion is currently being suppressed.
    #[must_use]
    pub const fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    /// Drive `ticker` from now on, for as long as somebody else still holds it.
    pub fn add<T: Ticker + 'static>(&mut self, ticker: &Rc<RefCell<T>>) {
        self.entries
            .push(Rc::downgrade(ticker) as Weak<RefCell<dyn Ticker>>);
    }

    /// How many live tickers are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.strong_count() > 0)
            .count()
    }

    /// `true` if nothing live is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// When the next frame is owed, if every animating ticker can say.
    ///
    /// `None` has two meanings and the caller wants the same thing for both:
    /// nothing is animating (so there is no deadline), or something is
    /// animating that cannot name one (so the deadline is "now"). The caller
    /// pairs this with [`is_animating`](Self::is_animating), which separates
    /// them.
    ///
    /// A single ticker with no deadline sinks the whole answer, which is
    /// correct: the loop has to be awake for the soonest of them, and one that
    /// needs every frame needs every frame.
    #[must_use]
    pub fn next_deadline(&self, now: Duration) -> Option<Duration> {
        let mut soonest: Option<Duration> = None;
        for entry in &self.entries {
            let Some(ticker) = entry.upgrade() else {
                continue;
            };
            let ticker = ticker.borrow();
            if !ticker.is_animating() {
                continue;
            }
            let deadline = ticker.next_deadline(now)?;
            soonest = Some(soonest.map_or(deadline, |held: Duration| held.min(deadline)));
        }
        soonest
    }

    /// `true` if anything registered still needs frames.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.entries.iter().any(|entry| {
            entry
                .upgrade()
                .is_some_and(|ticker| ticker.borrow().is_animating())
        })
    }

    /// Advance everything to `now`, dropping whatever has been released.
    ///
    /// Returns `true` if any ticker reported a change, which is what tells the
    /// frame it has work to do.
    pub fn advance(&mut self, now: Duration) -> bool {
        let mut changed = false;
        // Read out before the closure: a `&self` field cannot be read inside a
        // closure that already borrows `self.entries` mutably.
        let reduce_motion = self.reduce_motion;
        self.entries.retain(|entry| match entry.upgrade() {
            Some(ticker) => {
                // Borrowed for the tick and released before the next one: a
                // ticker whose value lands in a signal can pending an element that
                // holds another ticker, and holding a borrow across that would
                // panic at a distance from the cause.
                let mut ticker = ticker.borrow_mut();
                changed |= if reduce_motion {
                    ticker.settle(now)
                } else {
                    ticker.tick(now)
                };
                true
            }
            None => false,
        });
        changed
    }

    /// Forget everything, live or not. For tearing a screen down.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl fmt::Debug for Tickers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tickers")
            .field("live", &self.len())
            .field("animating", &self.is_animating())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn controller() -> Rc<RefCell<AnimationController>> {
        Rc::new(RefCell::new(AnimationController::new(ms(100))))
    }

    #[test]
    fn a_registered_controller_is_advanced_by_the_frame() {
        let mut tickers = Tickers::new();
        let fade = controller();
        tickers.add(&fade);
        fade.borrow_mut().forward(ms(0));

        assert!(tickers.advance(ms(50)));
        assert!((fade.borrow().value() - 0.5).abs() < 0.01);
    }

    #[test]
    fn an_idle_registry_reports_no_work() {
        let mut tickers = Tickers::new();
        let fade = controller();
        tickers.add(&fade);

        assert!(
            !tickers.advance(ms(50)),
            "a registered but idle controller must not keep the display awake"
        );
        assert!(!tickers.is_animating());
    }

    #[test]
    fn dropping_a_controller_unregisters_it() {
        let mut tickers = Tickers::new();
        let fade = controller();
        tickers.add(&fade);
        assert_eq!(tickers.len(), 1);

        drop(fade);

        assert_eq!(
            tickers.len(),
            0,
            "an animation whose owner went away must not have to be unregistered \
             by hand — that is the line everybody forgets"
        );
        assert!(!tickers.advance(ms(50)));
    }

    #[test]
    fn advancing_prunes_the_dead_entries_rather_than_growing_forever() {
        let mut tickers = Tickers::new();
        for _ in 0..100 {
            let short_lived = controller();
            tickers.add(&short_lived);
        }
        assert_eq!(tickers.entries.len(), 100, "all still in the vec");

        tickers.advance(ms(1));

        assert_eq!(tickers.entries.len(), 0, "and none after a frame");
        assert!(tickers.is_empty());
    }

    #[test]
    fn several_animations_run_at_once_and_report_together() {
        let mut tickers = Tickers::new();
        let (a, b) = (controller(), controller());
        tickers.add(&a);
        tickers.add(&b);

        a.borrow_mut().forward(ms(0));
        assert!(tickers.is_animating());
        assert!(tickers.advance(ms(50)));
        assert!((a.borrow().value() - 0.5).abs() < 0.01);
        assert_eq!(b.borrow().value(), 0.0, "b was never started");

        assert!(tickers.advance(ms(200)), "a finishes on this one");
        assert!(
            !tickers.advance(ms(300)),
            "and then everything is quiet again"
        );
        assert!(!tickers.is_animating());
    }
}

#[cfg(test)]
mod reduced_motion_tests {
    use super::*;
    use crate::{SpringAnimation, SpringPreset};

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// **The regression test for the accessibility gap.**
    ///
    /// `reduce_motion` was published by the foundation crate and honoured by
    /// none of the framework's own animations, so a person who had asked their
    /// operating system for less movement received every spring, route slide
    /// and press wash at full travel. On iOS that is an App Review rejection
    /// risk for vestibular safety; for the people the setting exists for it is a
    /// medical problem, not a preference.
    #[test]
    fn a_spring_lands_at_once_when_motion_is_reduced() {
        let spring = Rc::new(RefCell::new(SpringAnimation::new(
            0.0,
            SpringPreset::Standard,
        )));
        spring.borrow_mut().retarget(1.0);

        let mut tickers = Tickers::new();
        tickers.add(&spring);
        tickers.set_reduce_motion(true);

        tickers.advance(ms(0));

        assert!(
            (spring.borrow().value() - 1.0).abs() < f32::EPSILON,
            "the spring has to be at its target on the first frame, not on its way: {}",
            spring.borrow().value()
        );
        assert!(
            !spring.borrow().is_animating(),
            "and it has to report settled, so the frame loop goes back to sleep"
        );
    }

    /// The suppression must be exactly conditional — a system with the setting
    /// off is unchanged, which is what stops this being a behaviour regression
    /// for everybody else.
    #[test]
    fn the_same_spring_travels_normally_when_it_is_not() {
        let spring = Rc::new(RefCell::new(SpringAnimation::new(
            0.0,
            SpringPreset::Standard,
        )));
        spring.borrow_mut().retarget(1.0);

        let mut tickers = Tickers::new();
        tickers.add(&spring);
        assert!(!tickers.reduce_motion());

        tickers.advance(ms(0));
        tickers.advance(ms(16));

        let value = spring.borrow().value();
        assert!(
            value > 0.0 && value < 1.0,
            "one frame in, a real spring is on its way rather than arrived: {value}"
        );
    }

    /// A controller running a finite animation lands; **a repeat keeps going**.
    /// A spinner frozen at one value is not a reduced animation, it is a
    /// broken one, and every platform's own reduced-motion mode leaves progress
    /// indicators running.
    #[test]
    fn a_repeat_is_not_frozen_by_reduced_motion() {
        let finite = controller();
        finite.borrow_mut().forward(ms(0));
        let spinner = controller();
        spinner.borrow_mut().repeat(false, ms(0));

        let mut tickers = Tickers::new();
        tickers.add(&finite);
        tickers.add(&spinner);
        tickers.set_reduce_motion(true);
        tickers.advance(ms(0));

        assert!(
            (finite.borrow().value() - 1.0).abs() < f32::EPSILON,
            "the finite run is at its destination"
        );
        assert!(
            !finite.borrow().is_animating(),
            "and has stopped asking for frames"
        );
        assert!(
            spinner.borrow().is_animating(),
            "the repeat is still running — a stopped spinner reads as a hang"
        );
    }

    /// A ticker the framework does not own keeps ticking, because it may not be
    /// motion at all — a caret blink, a clock, a progress readout.
    #[test]
    fn an_unknown_ticker_is_left_alone() {
        #[derive(Default)]
        struct Counter {
            ticks: u32,
        }
        impl Ticker for Counter {
            fn tick(&mut self, _now: Duration) -> bool {
                self.ticks += 1;
                true
            }
            fn is_animating(&self) -> bool {
                true
            }
        }

        let counter = Rc::new(RefCell::new(Counter::default()));
        let mut tickers = Tickers::new();
        tickers.add(&counter);
        tickers.set_reduce_motion(true);
        tickers.advance(ms(0));
        tickers.advance(ms(16));

        assert_eq!(
            counter.borrow().ticks,
            2,
            "the default `settle` is `tick`: freezing something that is not \
             motion would be a worse bug than the one being fixed"
        );
    }

    fn controller() -> Rc<RefCell<AnimationController>> {
        Rc::new(RefCell::new(AnimationController::new(ms(300))))
    }
}
