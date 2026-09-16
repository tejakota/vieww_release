//! What the arena is actually for: two recognisers that both want the same
//! finger, and only one of them may have it.
//!
//! Every test here has two recognisers registered. That is deliberate — a
//! recogniser tested alone always wins, so a suite of single-recogniser tests
//! passes whatever the arena does, including nothing at all.

use std::time::Duration;

use vieww_foundation::{Axis, Offset, PointerEvent, PointerId, PointerPhase, PRESS_TIMEOUT};
use vieww_gestures::{
    recognize, DragRecognizer, GestureDispatcher, GestureRecognizer, LongPressRecognizer,
    Recognized, ScaleRecognizer, TapRecognizer,
};

const POINTER: PointerId = PointerId(1);
const SECOND: PointerId = PointerId(2);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn at(x: f32, y: f32) -> Offset {
    Offset::new(x, y)
}

/// A tap racing a drag — the button-inside-a-list case.
fn tap_vs_drag() -> GestureDispatcher {
    let mut dispatcher = GestureDispatcher::new();
    // The tap is added first, so it is the innermost member and wins a sweep.
    dispatcher.add(TapRecognizer::new());
    dispatcher.add(DragRecognizer::new());
    dispatcher
}

/// A press-move-release sequence, sampled every 16ms.
fn stroke(from: Offset, to: Offset, steps: u32, start: Duration) -> Vec<PointerEvent> {
    let mut events = vec![PointerEvent::down(POINTER, from, start)];
    events.extend(stroke_after(from, to, steps, start));
    events
}

/// The same, without the press — for a test that has to do something between
/// the finger landing and the finger moving.
fn stroke_after(from: Offset, to: Offset, steps: u32, start: Duration) -> Vec<PointerEvent> {
    let mut events = Vec::new();
    let mut previous = from;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let position = at(
            from.dx + (to.dx - from.dx) * t,
            from.dy + (to.dy - from.dy) * t,
        );
        events.push(PointerEvent::moved(
            POINTER,
            previous,
            position,
            start + ms(u64::from(step) * 16),
        ));
        previous = position;
    }
    events.push(PointerEvent::up(
        POINTER,
        to,
        start + ms(u64::from(steps) * 16 + 16),
    ));
    events
}

fn taps(gestures: &[Recognized]) -> usize {
    gestures
        .iter()
        .filter(|g| matches!(g, Recognized::Tap(_)))
        .count()
}

/// Where the first gesture matching `pred` landed in the stream. Order is the
/// assertion in half these tests — a retraction after the winner's gesture is
/// the wrong retraction.
fn position(gestures: &[Recognized], pred: impl Fn(&Recognized) -> bool) -> Option<usize> {
    gestures.iter().position(pred)
}

fn drag_starts(gestures: &[Recognized]) -> usize {
    gestures
        .iter()
        .filter(|g| matches!(g, Recognized::DragStart(_)))
        .count()
}

// ------------------------------------------------- the two roadmap criteria

#[test]
fn a_tap_that_moves_two_pixels_is_still_a_tap() {
    let mut dispatcher = tap_vs_drag();
    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(50.0, 50.0), at(52.0, 50.0), 2, ms(0)),
    );

    assert_eq!(
        taps(&gestures),
        1,
        "a finger never holds perfectly still; 2px must not cancel a tap: \
         {gestures:?}"
    );
    assert_eq!(drag_starts(&gestures), 0);
}

#[test]
fn a_drag_of_twenty_pixels_does_not_also_fire_a_tap() {
    let mut dispatcher = tap_vs_drag();
    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(50.0, 50.0), at(50.0, 90.0), 8, ms(0)),
    );

    assert_eq!(
        taps(&gestures),
        0,
        "flicking a list must not also press what was under the finger: \
         {gestures:?}"
    );
    assert_eq!(drag_starts(&gestures), 1);
    assert!(
        gestures.iter().any(|g| matches!(g, Recognized::DragEnd(_))),
        "and the drag has to end: {gestures:?}"
    );
}

// ------------------------------------------------------------------ the rest

#[test]
fn a_losing_tap_is_told_to_retract_what_it_showed_on_the_press() {
    let mut dispatcher = tap_vs_drag();
    let mut gestures = Vec::new();

    // Held still long enough to have lit something up, and only then dragged
    // away. Without the hold there is nothing on screen to retract, which is the
    // companion test below.
    dispatcher.handle(
        &PointerEvent::down(POINTER, at(0.0, 0.0), ms(0)),
        &mut gestures,
    );
    dispatcher.tick(PRESS_TIMEOUT, &mut gestures);
    for event in &stroke_after(at(0.0, 0.0), at(0.0, 60.0), 6, PRESS_TIMEOUT) {
        dispatcher.handle(event, &mut gestures);
    }

    let down = position(&gestures, |g| matches!(g, Recognized::TapDown(_)));
    let cancel = position(&gestures, |g| matches!(g, Recognized::TapCancel));
    let start = position(&gestures, |g| matches!(g, Recognized::DragStart(_)));

    assert!(
        down.is_some(),
        "the press was held long enough to show: {gestures:?}"
    );
    assert!(
        cancel.is_some(),
        "a button highlighted on the press has to \
        un-highlight when the list takes the gesture: {gestures:?}"
    );
    assert!(down < cancel, "and in that order: {gestures:?}");
    assert!(
        cancel < start,
        "and it has to be told before the winner's gesture arrives: {gestures:?}"
    );
}

#[test]
fn a_flick_never_highlights_what_it_passes_over() {
    let mut dispatcher = tap_vs_drag();
    // No tick, because a real flick gives the screen no time for one: the finger
    // is already moving 16ms in, long before `PRESS_TIMEOUT`.
    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(0.0, 0.0), at(0.0, 60.0), 6, ms(0)),
    );

    assert!(
        !gestures
            .iter()
            .any(|g| matches!(g, Recognized::TapDown(_) | Recognized::TapCancel)),
        "a press that was never shown must not be retracted either — a row that \
         lights up and goes out again under a scrolling finger is the artefact \
         PRESS_TIMEOUT exists to prevent: {gestures:?}"
    );
    assert_eq!(drag_starts(&gestures), 1);
}

#[test]
fn a_tap_faster_than_the_press_timeout_still_reports_its_press() {
    let mut dispatcher = tap_vs_drag();
    // Down and up inside 40ms: no tick ever falls in the gap, so the press can
    // only be reported by the win itself. A control that got a bare `Tap` with
    // no `TapDown` before it would have to special-case the fast tap.
    let gestures = recognize(
        &mut dispatcher,
        &[
            PointerEvent::down(POINTER, at(10.0, 10.0), ms(0)),
            PointerEvent::up(POINTER, at(10.0, 10.0), ms(40)),
        ],
    );

    let down = position(&gestures, |g| matches!(g, Recognized::TapDown(_)));
    let tap = position(&gestures, |g| matches!(g, Recognized::Tap(_)));
    assert!(down.is_some(), "{gestures:?}");
    assert!(down < tap, "the press has to come first: {gestures:?}");
    assert_eq!(taps(&gestures), 1);
}

#[test]
fn a_press_is_reported_once_however_many_frames_go_past() {
    let mut dispatcher = tap_vs_drag();
    let mut gestures = Vec::new();

    dispatcher.handle(
        &PointerEvent::down(POINTER, at(10.0, 10.0), ms(0)),
        &mut gestures,
    );
    for frame in 0..20 {
        dispatcher.tick(ms(frame * 16), &mut gestures);
    }

    assert_eq!(
        gestures
            .iter()
            .filter(|g| matches!(g, Recognized::TapDown(_)))
            .count(),
        1,
        "a held finger ticks every frame; the press is one event: {gestures:?}"
    );
}

#[test]
fn a_pointer_with_no_deadline_left_stops_asking_for_frames() {
    let mut tap = TapRecognizer::new();
    let mut sink = Vec::new();

    assert!(!tap.wants_tick(), "nothing is down");
    tap.handle(&PointerEvent::down(POINTER, at(0.0, 0.0), ms(0)), &mut sink);
    assert!(
        tap.wants_tick(),
        "the press deadline can only be reached by a frame going past"
    );

    tap.tick(PRESS_TIMEOUT, &mut sink);
    assert!(
        !tap.wants_tick(),
        "and once it has been reported, a finger may rest for a minute \
         without costing sixty frames a second"
    );
}

#[test]
fn the_drag_start_carries_the_movement_that_earned_it() {
    let mut dispatcher = tap_vs_drag();
    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(0.0, 0.0), at(0.0, 40.0), 4, ms(0)),
    );

    let Some(Recognized::DragStart(details)) = gestures
        .iter()
        .find(|g| matches!(g, Recognized::DragStart(_)))
    else {
        panic!("no drag start in {gestures:?}");
    };
    assert!(
        details.delta.dy > 0.0,
        "swallowing the slop distance makes a scroll lag the finger by 18px on \
         every touch: {details:?}"
    );
}

#[test]
fn a_cancelled_pointer_fires_nothing() {
    let mut dispatcher = tap_vs_drag();
    let gestures = recognize(
        &mut dispatcher,
        &[
            PointerEvent::down(POINTER, at(10.0, 10.0), ms(0)),
            PointerEvent::cancel(POINTER, at(10.0, 10.0), ms(16)),
        ],
    );

    assert_eq!(
        taps(&gestures),
        0,
        "the platform took the touch away — a call arrived, the app \
         backgrounded — and no gesture happened: {gestures:?}"
    );
    assert_eq!(drag_starts(&gestures), 0);
}

#[test]
fn a_cancelled_drag_ends_without_a_fling() {
    let mut dispatcher = tap_vs_drag();
    let mut sink = Vec::new();
    for event in &stroke(at(0.0, 0.0), at(0.0, 60.0), 6, ms(0))[..5] {
        dispatcher.handle(event, &mut sink);
    }
    dispatcher.handle(
        &PointerEvent::cancel(POINTER, at(0.0, 50.0), ms(100)),
        &mut sink,
    );

    let Some(Recognized::DragEnd(details)) = sink
        .iter()
        .rev()
        .find(|g| matches!(g, Recognized::DragEnd(_)))
    else {
        panic!("a drag in progress has to end even when cancelled: {sink:?}");
    };
    assert_eq!(
        details.velocity,
        Offset::ZERO,
        "flinging on a cancel scrolls the list as the app backgrounds"
    );
}

#[test]
fn a_drag_end_reports_the_speed_the_finger_was_going() {
    let mut dispatcher = tap_vs_drag();
    // 400px over 400ms is a steady 1000px/s.
    let mut events = vec![PointerEvent::down(POINTER, at(0.0, 0.0), ms(0))];
    let mut previous = at(0.0, 0.0);
    for step in 1..=40 {
        let position = at(0.0, step as f32 * 10.0);
        events.push(PointerEvent::moved(
            POINTER,
            previous,
            position,
            ms(step * 10),
        ));
        previous = position;
    }
    events.push(PointerEvent::up(POINTER, previous, ms(410)));

    let gestures = recognize(&mut dispatcher, &events);
    let Some(Recognized::DragEnd(details)) = gestures
        .iter()
        .find(|g| matches!(g, Recognized::DragEnd(_)))
    else {
        panic!("no drag end in {gestures:?}");
    };
    assert!(
        (details.velocity.dy - 1000.0).abs() < 100.0,
        "expected about 1000px/s, got {}",
        details.velocity
    );
}

#[test]
fn an_axis_locked_drag_ignores_movement_across_it() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(TapRecognizer::new());
    dispatcher.add(DragRecognizer::along(Axis::Vertical));

    // 40px sideways, well past the slop radius, but not on the drag's axis.
    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(0.0, 0.0), at(40.0, 0.0), 4, ms(0)),
    );

    assert_eq!(
        drag_starts(&gestures),
        0,
        "a vertical list must not scroll because the finger went sideways: \
         {gestures:?}"
    );
}

#[test]
fn an_axis_locked_drag_reports_only_its_own_axis() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(DragRecognizer::along(Axis::Vertical));

    let gestures = recognize(
        &mut dispatcher,
        &stroke(at(0.0, 0.0), at(30.0, 60.0), 6, ms(0)),
    );

    for gesture in &gestures {
        if let Recognized::DragStart(details) | Recognized::DragUpdate(details) = gesture {
            assert_eq!(
                details.delta.dx, 0.0,
                "a slightly diagonal finger must not drift the list sideways: \
                 {details:?}"
            );
        }
    }
}

#[test]
fn a_long_press_beats_a_tap_that_was_waiting() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(TapRecognizer::new());
    dispatcher.add(LongPressRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(&PointerEvent::down(POINTER, at(5.0, 5.0), ms(0)), &mut sink);
    // Nothing else happens; only the clock moves.
    dispatcher.tick(ms(600), &mut sink);

    assert!(
        sink.iter().any(|g| matches!(g, Recognized::LongPress(_))),
        "a long press is triggered by nothing happening: {sink:?}"
    );

    dispatcher.handle(&PointerEvent::up(POINTER, at(5.0, 5.0), ms(700)), &mut sink);
    assert_eq!(
        taps(&sink),
        0,
        "and lifting afterwards must not also fire a tap: {sink:?}"
    );
}

#[test]
fn a_press_released_before_the_timeout_is_a_tap_and_not_a_long_press() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(TapRecognizer::new());
    dispatcher.add(LongPressRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(&PointerEvent::down(POINTER, at(5.0, 5.0), ms(0)), &mut sink);
    dispatcher.tick(ms(100), &mut sink);
    dispatcher.handle(&PointerEvent::up(POINTER, at(5.0, 5.0), ms(120)), &mut sink);

    assert_eq!(taps(&sink), 1, "{sink:?}");
    assert!(!sink.iter().any(|g| matches!(g, Recognized::LongPress(_))));
}

#[test]
fn a_long_press_that_wanders_before_firing_is_abandoned() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(LongPressRecognizer::new());
    dispatcher.add(DragRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(&PointerEvent::down(POINTER, at(0.0, 0.0), ms(0)), &mut sink);
    dispatcher.handle(
        &PointerEvent::moved(POINTER, at(0.0, 0.0), at(0.0, 40.0), ms(100)),
        &mut sink,
    );
    dispatcher.tick(ms(600), &mut sink);

    assert!(
        !sink.iter().any(|g| matches!(g, Recognized::LongPress(_))),
        "the finger left; holding it somewhere else is not the same press: {sink:?}"
    );
}

#[test]
fn a_second_finger_starts_a_pinch_and_the_drag_stands_down() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(DragRecognizer::new());
    dispatcher.add(ScaleRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(
        &PointerEvent::down(POINTER, at(100.0, 100.0), ms(0)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::down(SECOND, at(200.0, 100.0), ms(16)),
        &mut sink,
    );

    assert!(
        sink.iter().any(|g| matches!(g, Recognized::ScaleStart(_))),
        "two fingers is what makes a pinch unambiguous: {sink:?}"
    );
}

#[test]
fn spreading_the_fingers_scales_up_and_pinching_scales_down() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(ScaleRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(
        &PointerEvent::down(POINTER, at(100.0, 100.0), ms(0)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::down(SECOND, at(200.0, 100.0), ms(16)),
        &mut sink,
    );
    // Spread from 100px apart to 200px apart.
    dispatcher.handle(
        &PointerEvent::moved(SECOND, at(200.0, 100.0), at(300.0, 100.0), ms(32)),
        &mut sink,
    );

    let Some(Recognized::ScaleUpdate(details)) = sink
        .iter()
        .rev()
        .find(|g| matches!(g, Recognized::ScaleUpdate(_)))
    else {
        panic!("no scale update in {sink:?}");
    };
    assert!(
        (details.scale - 2.0).abs() < 0.01,
        "spread doubled, so scale should be 2: {details:?}"
    );
    assert!(
        details.focal.dx > 150.0,
        "and the focal point follows the fingers: {details:?}"
    );
}

#[test]
fn lifting_one_finger_ends_the_pinch() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(ScaleRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(
        &PointerEvent::down(POINTER, at(100.0, 100.0), ms(0)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::down(SECOND, at(200.0, 100.0), ms(16)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::moved(SECOND, at(200.0, 100.0), at(260.0, 100.0), ms(32)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::up(SECOND, at(260.0, 100.0), ms(48)),
        &mut sink,
    );

    assert!(
        sink.iter().any(|g| matches!(g, Recognized::ScaleEnd(_))),
        "{sink:?}"
    );
}

#[test]
fn two_fingers_in_the_same_place_do_not_divide_by_zero() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(ScaleRecognizer::new());

    let mut sink = Vec::new();
    dispatcher.handle(
        &PointerEvent::down(POINTER, at(50.0, 50.0), ms(0)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::down(SECOND, at(50.0, 50.0), ms(16)),
        &mut sink,
    );
    dispatcher.handle(
        &PointerEvent::moved(SECOND, at(50.0, 50.0), at(150.0, 50.0), ms(32)),
        &mut sink,
    );

    for gesture in &sink {
        if let Recognized::ScaleUpdate(details) | Recognized::ScaleStart(details) = gesture {
            assert!(details.scale.is_finite(), "{details:?}");
        }
    }
}

#[test]
fn two_fingers_on_different_widgets_are_disambiguated_independently() {
    let mut dispatcher = tap_vs_drag();
    let mut sink = Vec::new();

    dispatcher.handle(&PointerEvent::down(POINTER, at(0.0, 0.0), ms(0)), &mut sink);
    dispatcher.handle(
        &PointerEvent::down(SECOND, at(0.0, 200.0), ms(4)),
        &mut sink,
    );
    // The first finger drags; the second stays put and lifts.
    dispatcher.handle(
        &PointerEvent::moved(POINTER, at(0.0, 0.0), at(0.0, 60.0), ms(50)),
        &mut sink,
    );
    dispatcher.handle(&PointerEvent::up(SECOND, at(0.0, 200.0), ms(60)), &mut sink);

    assert_eq!(
        taps(&sink),
        1,
        "the still finger is still a tap even while the other one drags: {sink:?}"
    );
}

#[test]
fn a_press_held_too_long_is_no_longer_a_tap_even_without_a_long_press_recogniser() {
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(TapRecognizer::new());

    let gestures = recognize(
        &mut dispatcher,
        &[
            PointerEvent::down(POINTER, at(5.0, 5.0), ms(0)),
            PointerEvent::up(POINTER, at(5.0, 5.0), ms(900)),
        ],
    );

    assert_eq!(
        taps(&gestures),
        0,
        "a press held for most of a second and then released is not a tap: \
         {gestures:?}"
    );
}

#[test]
fn a_mouse_is_held_to_a_much_tighter_slop_than_a_finger() {
    let mut dispatcher = tap_vs_drag();
    let gestures = recognize(
        &mut dispatcher,
        &[
            PointerEvent::down(POINTER, at(0.0, 0.0), ms(0))
                .with_kind(vieww_foundation::PointerDeviceKind::Mouse),
            PointerEvent::moved(POINTER, at(0.0, 0.0), at(0.0, 6.0), ms(16))
                .with_kind(vieww_foundation::PointerDeviceKind::Mouse),
            PointerEvent::up(POINTER, at(0.0, 6.0), ms(32))
                .with_kind(vieww_foundation::PointerDeviceKind::Mouse),
        ],
    );

    assert_eq!(
        drag_starts(&gestures),
        1,
        "6px is nothing for a finger and a deliberate drag for a mouse: \
         {gestures:?}"
    );
}

#[test]
fn a_pointer_that_never_arrives_leaves_no_contest_behind() {
    let mut dispatcher = tap_vs_drag();
    let _ = recognize(
        &mut dispatcher,
        &stroke(at(0.0, 0.0), at(0.0, 60.0), 6, ms(0)),
    );

    assert!(
        dispatcher.arena().is_empty(),
        "a settled pointer must not leak an arena entry: {:?}",
        dispatcher.arena()
    );
}

#[test]
fn the_phase_of_the_last_event_decides_nothing_on_its_own() {
    // A regression guard for the ordering the dispatcher documents: sweeping
    // before the recognisers have seen the up awards the pointer to whoever is
    // innermost regardless of what happened, which looks right whenever only one
    // recogniser is registered.
    let mut dispatcher = GestureDispatcher::new();
    dispatcher.add(TapRecognizer::new());
    dispatcher.add(DragRecognizer::new());

    let mut sink = Vec::new();
    for event in &stroke(at(0.0, 0.0), at(0.0, 80.0), 8, ms(0)) {
        assert!(
            event.phase != PointerPhase::Up || !sink.is_empty(),
            "the drag should have started well before the up"
        );
        dispatcher.handle(event, &mut sink);
    }

    assert_eq!(taps(&sink), 0, "{sink:?}");
}
