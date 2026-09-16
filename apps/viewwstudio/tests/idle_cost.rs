//! What an untouched window costs.
//!
//! The framework's whole idle story is one sentence in
//! `vieww-platform-winit`'s `windows.rs`: *"An idle vieww application sleeps in
//! `ControlFlow::Wait` and costs no CPU."* That is true of the framework and
//! **not** true of this application, and the difference is worth a test rather
//! than a paragraph, because nothing else in the suite can see it: every other
//! test here draws frames in a row on purpose, which is exactly the shape that
//! makes a permanently-animating tree indistinguishable from a settled one.

use vieww_foundation::Size;
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

/// A shell with `attach` applied to it, after enough frames to settle.
fn settled(attach: impl FnOnce(&Studio, &mut FrameDriver)) -> FrameDriver {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    attach(&studio, &mut driver);
    driver.set_root(Shell { studio });
    // More than enough for anything transient to finish.
    for _ in 0..8 {
        driver.draw_frame();
    }
    driver
}

/// Everything in the studio except the caret settles.
///
/// This is the control for the test below: without it, "the blink never
/// settles" could equally be "something in the shell never settles and the
/// blink is incidental".
#[test]
fn the_shell_settles_when_nothing_is_blinking() {
    let driver = settled(|_studio, _driver| {});
    assert!(
        !driver.needs_frame(),
        "the studio shell asks for frames for ever even with no caret blinking \
         — something other than the blink is animating, and the idle cost \
         below is not the blink's alone"
    );
}

/// **Closed.** This was the suite's one `#[ignore]`d test.
///
/// # What it pinned
///
/// `Blink::is_animating` answers `true` unconditionally — a caret blinks for as
/// long as there is one. That answer is read by `FrameDriver::needs_frame`,
/// which was read by `next_action` as a *standing state*, so `about_to_wait`
/// reached `ControlFlow::Poll` on every iteration and the loop never slept.
/// Measured on an untouched window: **one core at 100%, indefinitely**, with
/// nothing on screen changing but a caret twice a second.
///
/// The claim in `is_animating`'s doc was right and the mechanism was wrong. A
/// blink does not need *every* frame; it needs **a frame at its next toggle**,
/// which is a deadline and not an animation. The pointer router already drew
/// that distinction for the same reason — `PointerRouter::wants_tick`'s docs
/// note that a finger held for a minute costs "the two frames its deadlines
/// fall on rather than sixty a second" — and could only afford to spin because
/// a live gesture is transient. A caret is not.
///
/// # What closed it
///
/// `Ticker::next_deadline`, minimised across live tickers by `Tickers`,
/// surfaced as `FrameDriver::frame_deadline`, returned as a third
/// `LoopAction::SleepUntil`, and mapped to winit's `ControlFlow::WaitUntil`.
///
/// # Why the assertion changed shape
///
/// `needs_frame` still answers `true`, and that is correct: the caret **is**
/// animating, and a loop that treated it as settled would freeze it — the other
/// of the two failure modes this change had to avoid. What is no longer true is
/// that the loop has to be awake for it. So the question is now the one that
/// was always being asked underneath: *when* is the next frame owed, and is it
/// far enough away to sleep through.
#[test]
fn an_untouched_window_sleeps_between_caret_blinks() {
    let driver = settled(|studio, driver| studio.attach_blink(driver.tickers()));
    assert!(
        driver.needs_frame(),
        "a caret is still animating — a loop that thought otherwise would freeze it"
    );

    let now = std::time::Duration::ZERO;
    let deadline = driver
        .frame_deadline(now)
        .expect("the blink is the only thing animating, and it knows its next toggle");
    assert!(
        deadline > now,
        "the next toggle is in the future, so the loop has something to sleep against"
    );
    assert_eq!(
        deadline,
        viewwstudio::caret::BLINK,
        "the first toggle is one interval after the frame that started the phase"
    );
}

/// And the decision the event loop actually makes, through the same function a
/// real window calls.
///
/// `next_action` is `about_to_wait`'s body with the platform types removed —
/// see its module docs — so this is the loop, not a model of it.
#[test]
fn the_event_loop_answers_sleep_until_rather_than_draw() {
    use vieww_render::{next_action, LoopAction};

    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.attach_blink(driver.tickers());
    driver.set_root(Shell { studio });
    for _ in 0..8 {
        driver.draw_frame();
    }

    let mut scheduler = vieww_paint::FrameScheduler::sixty_hz();
    let action = next_action(
        &mut scheduler,
        Some(&driver),
        true,
        std::time::Duration::ZERO,
    );
    assert!(
        matches!(action, LoopAction::SleepUntil(_)),
        "an idle window with a caret in it used to answer Draw for ever: {action:?}"
    );
    assert!(
        !scheduler.is_frame_requested(),
        "and it must not have asked for the frame it just declined to draw —          that request is what would turn the sleep back into a spin"
    );
}

/// A frame somebody asked for is never deferred.
///
/// The failure this guards is the deadline swallowing real work: a keystroke
/// requests a frame, and if `SleepUntil` won over that request the character
/// would appear at the caret's next blink instead of immediately.
#[test]
fn a_requested_frame_is_drawn_now_even_with_a_deadline_pending() {
    use vieww_render::{next_action, LoopAction};

    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.attach_blink(driver.tickers());
    driver.set_root(Shell { studio });
    for _ in 0..8 {
        driver.draw_frame();
    }

    let mut scheduler = vieww_paint::FrameScheduler::sixty_hz();
    scheduler.request_frame();
    assert_eq!(
        next_action(
            &mut scheduler,
            Some(&driver),
            true,
            std::time::Duration::ZERO
        ),
        LoopAction::Draw
    );
}
