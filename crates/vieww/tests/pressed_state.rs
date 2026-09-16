//! A finger going down on a control, and the control noticing — through the
//! whole pipeline, with a clock.
//!
//! ```console
//! cargo test -p vieww --test pressed_state
//! ```
//!
//! # Why this is not part of `settings_screen`
//!
//! That test drives every frame at time zero, deliberately: nothing in it
//! depends on the clock, and pinning the clock keeps it from depending on one by
//! accident. A press is the opposite — it is a deadline
//! ([`PRESS_TIMEOUT`](vieww::foundation::PRESS_TIMEOUT)) followed by a fade
//! ([`CONTROL_DURATION`](vieww::CONTROL_DURATION)), so the frames here
//! carry real timestamps and the gaps between them are the thing under test.
//!
//! What is asserted is the chain end to end, because each link is invisible from
//! the next: the recogniser holding the press back, the frame ticking it out,
//! the handler writing an `ElementState`, that state booking itself a frame, and
//! the builder being called again on each one with a new fraction.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{Color, Offset, PointerEvent, PointerId, Size, PRESS_TIMEOUT};
use vieww::prelude::*;
use vieww::{FrameDriver, CONTROL_DURATION};

const SURFACE: Size = Size {
    width: 200.0,
    height: 200.0,
};
const POINTER: PointerId = PointerId(1);
const MIDDLE: Offset = Offset {
    dx: 100.0,
    dy: 100.0,
};

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A driver showing one full-surface pressable, and the record of what its
/// builder was asked to build.
struct Screen {
    driver: FrameDriver,
    /// Every fraction the builder was called with, in order. The mechanism under
    /// test is "was it rebuilt, and with what" — asserting that directly beats
    /// reading colours back out of the scene, which would also pass if the
    /// rebuild happened for some unrelated reason.
    built: Rc<RefCell<Vec<f32>>>,
    taps: Rc<RefCell<usize>>,
}

impl Screen {
    fn new() -> Self {
        let mut driver = FrameDriver::new(SURFACE);
        let built = Rc::new(RefCell::new(Vec::new()));
        let taps = Rc::new(RefCell::new(0));

        let record = Rc::clone(&built);
        let counter = Rc::clone(&taps);
        driver.set_root(
            Pressable::new(move |press| {
                record.borrow_mut().push(press);
                ColoredBox::new(if press > 0.5 {
                    Color::BLUE
                } else {
                    Color::WHITE
                })
                .into()
            })
            .on_tap(move || *counter.borrow_mut() += 1),
        );
        driver.draw_frame_at(ms(0));

        Self {
            driver,
            built,
            taps,
        }
    }

    fn frame(&mut self, at: Duration) {
        self.driver.draw_frame_at(at);
    }

    /// Run frames every 16ms from `from` until `until`, as a display would.
    fn frames_until(&mut self, from: Duration, until: Duration) {
        let mut at = from;
        while at <= until {
            self.frame(at);
            at += ms(16);
        }
    }

    fn down(&mut self, at: Duration) {
        self.driver
            .handle_pointer(&PointerEvent::down(POINTER, MIDDLE, at));
    }

    fn up(&mut self, at: Duration) {
        self.driver
            .handle_pointer(&PointerEvent::up(POINTER, MIDDLE, at));
    }

    /// How pressed the last build was told it was.
    fn press(&self) -> f32 {
        self.built.borrow().last().copied().unwrap_or(0.0)
    }

    fn builds(&self) -> usize {
        self.built.borrow().len()
    }
}

#[test]
fn a_press_shows_only_after_the_finger_has_declared_itself() {
    let mut screen = Screen::new();
    assert_eq!(screen.press(), 0.0, "nothing has touched it");

    screen.down(ms(0));
    screen.frame(ms(0));
    assert_eq!(
        screen.press(),
        0.0,
        "the frame the finger landed on must not light it up — that is the \
         flash under a scrolling finger PRESS_TIMEOUT exists to prevent"
    );

    screen.frame(ms(48));
    assert_eq!(screen.press(), 0.0, "three frames in, still too early");

    // The deadline reports the press; the fade it starts is what shows it, and
    // that is the *next* frame's business — a handler runs in the build phase
    // and animations advance in the phase before it.
    screen.frames_until(PRESS_TIMEOUT, PRESS_TIMEOUT + CONTROL_DURATION + ms(32));
    assert!(
        (screen.press() - 1.0).abs() < 0.01,
        "and a finger held past the fade is fully pressed: {}",
        screen.press()
    );
}

#[test]
fn the_press_arrives_gradually_rather_than_all_at_once() {
    let mut screen = Screen::new();
    screen.down(ms(0));
    screen.frames_until(ms(0), PRESS_TIMEOUT + CONTROL_DURATION);

    let seen = screen.built.borrow().clone();
    let partial = seen.iter().filter(|&&p| p > 0.01 && p < 0.99).count();
    assert!(
        partial >= 2,
        "a highlight that snapped on would show no intermediate frames at all, \
         and this is the assertion that a fade is a fade: {seen:?}"
    );
}

#[test]
fn lifting_the_finger_releases_the_press_and_runs_the_handler() {
    let mut screen = Screen::new();

    screen.down(ms(0));
    screen.frames_until(ms(0), PRESS_TIMEOUT + CONTROL_DURATION + ms(32));
    assert!(screen.press() > 0.9);

    let lifted = PRESS_TIMEOUT + CONTROL_DURATION + ms(48);
    screen.up(lifted);
    screen.frames_until(lifted, lifted + CONTROL_DURATION + ms(32));

    assert!(
        screen.press() < 0.01,
        "the highlight goes with the finger: {}",
        screen.press()
    );
    assert_eq!(*screen.taps.borrow(), 1, "and the tap still happened");
}

#[test]
fn a_tap_too_quick_for_a_frame_runs_without_ever_looking_pressed() {
    let mut screen = Screen::new();
    let before = screen.builds();

    // Down and up with no frame in between, which is what a fast tap on a 60Hz
    // screen actually is. The gesture layer still reports the press — it has to,
    // or a `Tap` would arrive with nothing before it — and the widget layer nets
    // the two out, because a fade that starts and ends between two frames is
    // work for something nobody could have seen.
    screen.down(ms(0));
    screen.up(ms(40));
    screen.frames_until(ms(48), ms(48) + CONTROL_DURATION);

    assert_eq!(screen.press(), 0.0);
    assert_eq!(*screen.taps.borrow(), 1, "the tap still happened");
    assert_eq!(screen.builds(), before, "and cost no rebuild");
}

#[test]
fn a_press_below_the_deadline_costs_no_rebuilds() {
    let mut screen = Screen::new();
    let before = screen.builds();

    screen.down(ms(0));
    screen.frame(ms(16));
    screen.frame(ms(32));
    screen.up(ms(40));
    screen.frame(ms(48));

    assert_eq!(
        screen.builds(),
        before,
        "a press this short changes nothing on screen, and rebuilding for it \
         would be a rebuild per row a scrolling finger passes over"
    );
}

#[test]
fn a_settled_control_asks_for_no_more_frames() {
    let mut screen = Screen::new();

    assert!(
        !screen.driver.needs_frame(),
        "an idle window costs no CPU, which is the whole point of asking"
    );

    screen.down(ms(0));
    assert!(
        screen.driver.needs_frame(),
        "the press deadline is reachable only by a frame going past — this is \
         what was missing, and why a long press could not fire in a real window"
    );

    // Held, past both the deadline and the fade it starts.
    screen.frames_until(PRESS_TIMEOUT, PRESS_TIMEOUT + CONTROL_DURATION + ms(32));
    assert!(
        !screen.driver.needs_frame(),
        "once the deadline has passed and the fade has finished it stops: a \
         finger resting on the screen must not hold the app at sixty frames a \
         second"
    );

    let lifted = PRESS_TIMEOUT + CONTROL_DURATION + ms(48);
    screen.up(lifted);
    screen.frames_until(lifted, lifted + CONTROL_DURATION + ms(32));
    assert!(!screen.driver.needs_frame());
}
