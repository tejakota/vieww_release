//! `winit` input events as [`PointerEvent`]s.
//!
//! Phase 6 built the pointer pipeline against a shape it defined and left the
//! platform side of it open; this is that side. It is deliberately a plain
//! struct with a method per event rather than anything that touches a window, so
//! the translation can be tested without opening one — which is most of what
//! there is to get wrong here.
//!
//! # What a platform does not give you
//!
//! Three things the framework's own model assumes, that `winit` does not supply:
//!
//! - **A press has no position.** `MouseInput` says a button changed; where the
//!   cursor was is the business of the last `CursorMoved`. So the last position
//!   is kept, and a press before any movement is dropped rather than invented at
//!   the origin.
//! - **A move has no previous position.** `PointerEvent::delta` is movement
//!   since that pointer's last event, so each live pointer's position is kept
//!   and differenced.
//! - **An event has no timestamp.** `winit` reports events, not when the device
//!   produced them, so the clock is read on arrival. That is exactly the error
//!   [`PointerEvent::timestamp`] warns about — a fling measured from arrival
//!   times is wrong by however long the queue was — and it is the best any
//!   `winit` backend can do. A platform that does expose device timestamps
//!   should use them and skip this.

use std::collections::HashMap;
use std::time::Duration;

use vieww_foundation::{
    Offset, PointerButton, PointerDeviceKind, PointerEvent, PointerId, PointerPhase, ScrollEvent,
};

use crate::scale::Scale;

/// The mouse's pointer id.
///
/// Fixed, and reserved: touch ids are allocated from 1 upwards, so a finger can
/// never be mistaken for the cursor. `winit`'s own touch ids are not usable
/// directly — they are whatever the platform hands out, and on Android that is a
/// small integer that starts at zero.
const MOUSE: PointerId = PointerId(0);

/// How far one wheel notch scrolls, in logical pixels.
///
/// A line, roughly, and a made-up number in the sense that every toolkit makes
/// up its own: `winit` reports notches and nothing under it knows what a line
/// means on this display. Chosen to match what a three-notch flick does in a
/// browser, because that is what a user's hand is calibrated to.
const LINE_HEIGHT: f32 = 40.0;

/// What a platform said a scroll was worth.
///
/// Two units rather than one because trackpads and wheels genuinely differ: a
/// trackpad reports the pixels a finger moved and a wheel reports detents, and
/// converting the second to the first is a decision only a platform can make.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelDelta {
    /// Wheel notches, horizontal then vertical.
    Lines(f32, f32),
    /// Logical pixels, horizontal then vertical.
    Pixels(f32, f32),
}

/// Turns a window's raw input into the events the gesture layer understands.
///
/// One per window. Holds the state `winit` does not: where each live pointer
/// was, and whether the mouse button is down.
#[derive(Debug)]
pub struct PointerTranslator {
    scale: Scale,
    /// Where the cursor was when we last heard, in logical pixels. `None` before
    /// the first `CursorMoved`, and after the cursor leaves the window.
    cursor: Option<Offset>,
    /// `true` between a mouse-button press and its release.
    pressed: bool,
    /// Which button that press was, so the matching release can be recognised.
    held: PointerButton,
    /// Each live pointer's last position, for computing `delta`.
    previous: HashMap<PointerId, Offset>,
    /// `winit`'s touch id to ours. Cleared per finger on lift, so a platform
    /// that reuses ids does not inherit a stale position.
    touches: HashMap<u64, PointerId>,
    /// The next touch id to hand out. Never 0 — that is the mouse.
    next_touch: u64,
}

impl PointerTranslator {
    /// A translator for a window at the given scale.
    #[must_use]
    pub fn new(scale: Scale) -> Self {
        Self {
            scale,
            cursor: None,
            pressed: false,
            held: PointerButton::Primary,
            previous: HashMap::new(),
            touches: HashMap::new(),
            next_touch: 1,
        }
    }

    /// Follow a window whose display changed.
    pub const fn set_scale(&mut self, scale: Scale) {
        self.scale = scale;
    }

    /// How many pointers are currently down.
    #[must_use]
    pub fn live_pointers(&self) -> usize {
        self.previous.len()
    }

    /// The cursor moved to a physical position.
    ///
    /// Produces a pointer event only while a button is held: feeding moves to
    /// the arena with nothing pressed would start drags nobody asked for. A
    /// cursor that is merely *over* something is [`hover`](Self::hover)'s
    /// business, which is a different question answered by a different hit test.
    pub fn cursor_moved(&mut self, physical: (f64, f64), now: Duration) -> Option<PointerEvent> {
        let position = self.logical(physical);
        self.cursor = Some(position);
        self.pressed
            .then(|| self.moved(MOUSE, position, now, PointerDeviceKind::Mouse))
    }

    /// Where the cursor is, or `None` when it is not in the window.
    ///
    /// Reported separately from the pointer stream, and unconditionally: hover
    /// is tracked while a button is held too, because a drag that leaves a
    /// control should not leave it looking hovered.
    #[must_use]
    pub const fn hover(&self) -> Option<Offset> {
        self.cursor
    }

    /// A mouse button went down or came up.
    ///
    /// `None` if the cursor has never reported a position — a press we cannot
    /// place is a press we cannot hit test, and guessing the origin would
    /// deliver it to whatever happens to be in the corner.
    ///
    /// One button at a time: a second button pressed while the first is held is
    /// ignored rather than opening a second pointer. Both would share the mouse's
    /// single id, and the arena keys everything on that.
    pub fn mouse_button(
        &mut self,
        button: PointerButton,
        down: bool,
        now: Duration,
    ) -> Option<PointerEvent> {
        let position = self.cursor?;
        if down == self.pressed {
            return None;
        }
        if down {
            self.pressed = true;
            self.held = button;
            self.previous.insert(MOUSE, position);
            return Some(
                PointerEvent::down(MOUSE, position, now)
                    .with_kind(PointerDeviceKind::Mouse)
                    .with_button(button),
            );
        }
        if button != self.held {
            // The release of a button whose press we ignored.
            return None;
        }
        self.pressed = false;
        self.previous.remove(&MOUSE);
        Some(
            PointerEvent::up(MOUSE, position, now)
                .with_kind(PointerDeviceKind::Mouse)
                .with_button(button),
        )
    }

    /// A wheel notch or a trackpad scroll, as a movement of the content.
    ///
    /// `None` before the cursor has reported a position: a scroll is hit tested
    /// where the cursor is, and there is nowhere to send one that has no place.
    ///
    /// `lines` are converted at `LINE_HEIGHT` because only a platform knows how
    /// far a notch is meant to move; pixel deltas from a trackpad arrive already
    /// in the right unit and pass through.
    pub fn scroll(&self, delta: WheelDelta, now: Duration) -> Option<ScrollEvent> {
        let position = self.cursor?;
        let pixels = match delta {
            WheelDelta::Lines(x, y) => Offset::new(x * LINE_HEIGHT, y * LINE_HEIGHT),
            WheelDelta::Pixels(x, y) => Offset::new(x, y),
        };
        // **Not negated, and it used to be.** `ScrollEvent::delta` is documented
        // as finger-equivalent — the movement a *drag* would have made — so that
        // `Scrollable` can hand a wheel notch to the same handler as a drag
        // "without anything downstream negating one of them". Negating here put
        // it in the opposite convention to `DragDetails::delta`, and the two
        // then disagreed by a sign the whole way down.
        //
        // What that looked like: winit reports a wheel turned *toward* the user
        // as a negative `LineDelta`, the negation made it positive, and
        // `ScrollPosition::apply_drag` reads a positive delta as a finger moving
        // *down* — which scrolls back towards the start. So the wheel ran
        // backwards everywhere, and on any list sitting at offset 0 it did
        // nothing at all, which is how it went unnoticed: the failure looks
        // exactly like a wheel that is not wired up.
        //
        // Nothing pinned this. The one wheel test asserts the handler is reached
        // and passes its delta through unchanged, which is true of either sign.
        Some(ScrollEvent::new(position, pixels, now))
    }

    /// The cursor left the window.
    ///
    /// A button held across the boundary is cancelled, not released: the pointer
    /// is gone and we will never see where it came up, so a tap must not fire.
    pub fn cursor_left(&mut self, now: Duration) -> Option<PointerEvent> {
        let position = self.cursor.take();
        if !self.pressed {
            return None;
        }
        self.pressed = false;
        self.previous.remove(&MOUSE);
        Some(PointerEvent::cancel(MOUSE, position?, now).with_kind(PointerDeviceKind::Mouse))
    }

    /// A finger touched, moved, lifted or was taken away.
    ///
    /// `id` is `winit`'s, and is translated to one of ours — see `MOUSE`.
    ///
    /// `pressure` is `winit`'s own `Force::normalized()` (already reduced to a
    /// plain `0.0..=1.0` before it reaches here — see this type's `pressure`
    /// field for
    /// why this file never imports `winit::event::Force` itself). Not every
    /// platform `winit` runs on reports it (its own doc: "Only available on
    /// iOS 9.0+, Windows 8+, Web, and Android"), which is exactly why the
    /// parameter is an `Option` all the way through to
    /// [`PointerEvent::pressure`] rather than defaulted to some number.
    pub fn touch(
        &mut self,
        id: u64,
        phase: TouchPhase,
        physical: (f64, f64),
        now: Duration,
        pressure: Option<f32>,
    ) -> Option<PointerEvent> {
        let position = self.logical(physical);

        match phase {
            TouchPhase::Started => {
                let pointer = PointerId(self.next_touch);
                self.next_touch += 1;
                self.touches.insert(id, pointer);
                self.previous.insert(pointer, position);
                Some(Self::pressure(
                    PointerEvent::down(pointer, position, now),
                    pressure,
                ))
            }
            // A move for a finger we never saw go down is not ours to route —
            // it happens when a touch was in progress as the window gained
            // focus, and inventing a `Down` for it would hit test against a
            // position the finger has already left.
            TouchPhase::Moved => {
                let pointer = *self.touches.get(&id)?;
                let event = self.moved(pointer, position, now, PointerDeviceKind::Touch);
                Some(Self::pressure(event, pressure))
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let pointer = self.touches.remove(&id)?;
                self.previous.remove(&pointer);
                let event = if matches!(phase, TouchPhase::Ended) {
                    PointerEvent::up(pointer, position, now)
                } else {
                    PointerEvent::cancel(pointer, position, now)
                };
                Some(Self::pressure(event, pressure))
            }
        }
    }

    /// Stamp `pressure` onto `event`, if the platform reported one.
    ///
    /// Kept as one line rather than repeated at each of `touch`'s four call
    /// sites, and its own function rather than inlined `Option` plumbing so
    /// adding a fifth phase later cannot forget it — the exact shape of bug
    /// [`with_modifiers`](vieww_foundation::PointerEvent::with_modifiers)'s own
    /// doc describes for keys.
    fn pressure(event: PointerEvent, pressure: Option<f32>) -> PointerEvent {
        match pressure {
            Some(pressure) => event.with_pressure(pressure),
            None => event,
        }
    }

    /// Every live pointer, cancelled.
    ///
    /// What a suspend or a lost window owes the gesture layer: the pointers are
    /// not coming back, and a recogniser still waiting on one holds an arena
    /// entry open forever. `Cancel` rather than `Up` because nothing was
    /// completed — see [`PointerPhase::Cancel`].
    pub fn cancel_all(&mut self, now: Duration) -> Vec<PointerEvent> {
        let mut events: Vec<_> = self
            .previous
            .drain()
            .map(|(pointer, position)| {
                let kind = if pointer == MOUSE {
                    PointerDeviceKind::Mouse
                } else {
                    PointerDeviceKind::Touch
                };
                PointerEvent::cancel(pointer, position, now).with_kind(kind)
            })
            .collect();
        // A `HashMap` drains in whatever order it likes, and a caller replaying
        // these wants them reproducible.
        events.sort_by_key(|event| event.pointer);

        self.pressed = false;
        self.touches.clear();
        events
    }

    fn moved(
        &mut self,
        pointer: PointerId,
        position: Offset,
        now: Duration,
        kind: PointerDeviceKind,
    ) -> PointerEvent {
        let previous = self.previous.insert(pointer, position).unwrap_or(position);
        let mut event = PointerEvent::moved(pointer, previous, position, now);
        event.kind = kind;
        debug_assert_eq!(event.phase, PointerPhase::Move);
        event
    }

    fn logical(&self, physical: (f64, f64)) -> Offset {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "f32 is the geometry scalar; §4"
        )]
        let physical = Offset::new(physical.0 as f32, physical.1 as f32);
        self.scale.to_logical(physical)
    }
}

/// Where a finger is in its lifetime, as `winit` reports it.
///
/// Mirrored rather than re-exported so the tests below — and the translation
/// itself — do not need a window, an event loop, or `winit` in scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

impl From<winit::event::TouchPhase> for TouchPhase {
    fn from(phase: winit::event::TouchPhase) -> Self {
        match phase {
            winit::event::TouchPhase::Started => Self::Started,
            winit::event::TouchPhase::Moved => Self::Moved,
            winit::event::TouchPhase::Ended => Self::Ended,
            winit::event::TouchPhase::Cancelled => Self::Cancelled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn translator() -> PointerTranslator {
        PointerTranslator::new(Scale::ONE)
    }

    #[test]
    fn a_press_before_any_movement_is_dropped_rather_than_placed_at_the_origin() {
        let mut translator = translator();
        assert!(
            translator
                .mouse_button(PointerButton::Primary, true, ms(0))
                .is_none(),
            "a press we cannot place would hit test the top-left corner"
        );
        assert_eq!(translator.live_pointers(), 0);
    }

    #[test]
    fn a_press_is_placed_where_the_cursor_last_was() {
        let mut translator = translator();
        assert!(
            translator.cursor_moved((40.0, 30.0), ms(0)).is_none(),
            "a hover with nothing pressed is not a pointer event"
        );

        let down = translator
            .mouse_button(PointerButton::Primary, true, ms(1))
            .expect("a down");
        assert_eq!(down.phase, PointerPhase::Down);
        assert_eq!(down.position, Offset::new(40.0, 30.0));
        assert_eq!(down.kind, PointerDeviceKind::Mouse);
        assert_eq!(down.delta, Offset::ZERO);
    }

    #[test]
    fn a_drag_carries_the_movement_since_the_last_event_not_since_the_press() {
        let mut translator = translator();
        translator.cursor_moved((0.0, 0.0), ms(0));
        translator.mouse_button(PointerButton::Primary, true, ms(1));

        let first = translator.cursor_moved((10.0, 0.0), ms(2)).expect("a move");
        assert_eq!(first.delta, Offset::new(10.0, 0.0));

        let second = translator.cursor_moved((25.0, 0.0), ms(3)).expect("a move");
        assert_eq!(
            second.delta,
            Offset::new(15.0, 0.0),
            "delta is per event; a delta from the press would double-count"
        );
        assert_eq!(second.position, Offset::new(25.0, 0.0));
    }

    #[test]
    fn a_repeated_press_or_release_produces_nothing() {
        let mut translator = translator();
        translator.cursor_moved((5.0, 5.0), ms(0));

        assert!(translator
            .mouse_button(PointerButton::Primary, true, ms(1))
            .is_some());
        assert!(
            translator
                .mouse_button(PointerButton::Primary, true, ms(2))
                .is_none(),
            "two downs for one press would open two arena entries"
        );
        assert!(translator
            .mouse_button(PointerButton::Primary, false, ms(3))
            .is_some());
        assert!(translator
            .mouse_button(PointerButton::Primary, false, ms(4))
            .is_none());
    }

    #[test]
    fn a_button_held_out_of_the_window_cancels_rather_than_taps() {
        let mut translator = translator();
        translator.cursor_moved((5.0, 5.0), ms(0));
        translator.mouse_button(PointerButton::Primary, true, ms(1));

        let left = translator.cursor_left(ms(2)).expect("a cancel");
        assert_eq!(
            left.phase,
            PointerPhase::Cancel,
            "an up here would fire a tap the user never completed"
        );
        assert_eq!(translator.live_pointers(), 0);
    }

    #[test]
    fn leaving_the_window_with_nothing_pressed_is_silent() {
        let mut translator = translator();
        translator.cursor_moved((5.0, 5.0), ms(0));
        assert!(translator.cursor_left(ms(1)).is_none());
    }

    #[test]
    fn two_fingers_are_two_pointers_and_neither_is_the_mouse() {
        let mut translator = translator();

        let first = translator
            .touch(7, TouchPhase::Started, (10.0, 10.0), ms(0), None)
            .expect("a down");
        let second = translator
            .touch(9, TouchPhase::Started, (80.0, 10.0), ms(1), None)
            .expect("a down");

        assert_ne!(first.pointer, second.pointer);
        assert_ne!(first.pointer, MOUSE);
        assert_ne!(second.pointer, MOUSE);
        assert_eq!(translator.live_pointers(), 2);

        let moved = translator
            .touch(9, TouchPhase::Moved, (85.0, 10.0), ms(2), None)
            .expect("a move");
        assert_eq!(
            moved.pointer, second.pointer,
            "the second finger, not the first"
        );
        assert_eq!(moved.delta, Offset::new(5.0, 0.0));
    }

    /// `winit` reports touch force on iOS/Windows/Web/Android but not
    /// everywhere — the whole reason [`PointerEvent::pressure`] is an
    /// `Option`. Both cases must reach the recogniser layer intact: a real
    /// reading stamped, and no reading left honestly absent rather than
    /// defaulted to some number nobody measured.
    #[test]
    fn touch_pressure_is_carried_through_when_the_platform_reports_it() {
        let mut translator = translator();

        let with_pressure = translator
            .touch(1, TouchPhase::Started, (10.0, 10.0), ms(0), Some(0.75))
            .expect("a down");
        assert_eq!(with_pressure.pressure, Some(0.75));

        let without_pressure = translator
            .touch(2, TouchPhase::Started, (20.0, 20.0), ms(1), None)
            .expect("a down");
        assert_eq!(
            without_pressure.pressure, None,
            "a platform that reports no force must not be defaulted to one"
        );

        let moved = translator
            .touch(1, TouchPhase::Moved, (12.0, 10.0), ms(2), Some(0.9))
            .expect("a move");
        assert_eq!(
            moved.pressure,
            Some(0.9),
            "pressure updates on every phase, not only Down"
        );
    }

    #[test]
    fn a_reused_platform_touch_id_is_a_new_pointer() {
        let mut translator = translator();
        let first = translator
            .touch(0, TouchPhase::Started, (10.0, 10.0), ms(0), None)
            .expect("a down");
        translator.touch(0, TouchPhase::Ended, (10.0, 10.0), ms(1), None);

        let second = translator
            .touch(0, TouchPhase::Started, (90.0, 90.0), ms(2), None)
            .expect("a second down");

        assert_ne!(
            first.pointer, second.pointer,
            "the same id reused is a different finger, and sharing a PointerId \
             would make the arena treat them as one sequence"
        );
        // The delta would be 80,80 if the first finger's position had survived.
        let moved = translator
            .touch(0, TouchPhase::Moved, (95.0, 90.0), ms(3), None)
            .expect("a move");
        assert_eq!(moved.delta, Offset::new(5.0, 0.0));
    }

    #[test]
    fn a_move_for_a_finger_we_never_saw_go_down_is_ignored() {
        let mut translator = translator();
        assert!(translator
            .touch(3, TouchPhase::Moved, (10.0, 10.0), ms(0), None)
            .is_none());
        assert!(translator
            .touch(3, TouchPhase::Ended, (10.0, 10.0), ms(1), None)
            .is_none());
    }

    #[test]
    fn a_cancelled_touch_is_not_an_up() {
        let mut translator = translator();
        translator.touch(1, TouchPhase::Started, (10.0, 10.0), ms(0), None);
        let ended = translator
            .touch(1, TouchPhase::Cancelled, (10.0, 10.0), ms(1), None)
            .expect("a cancel");
        assert_eq!(ended.phase, PointerPhase::Cancel);
    }

    #[test]
    fn suspending_cancels_every_live_pointer_exactly_once() {
        let mut translator = translator();
        translator.cursor_moved((5.0, 5.0), ms(0));
        translator.mouse_button(PointerButton::Primary, true, ms(1));
        translator.touch(4, TouchPhase::Started, (20.0, 20.0), ms(2), None);
        translator.touch(5, TouchPhase::Started, (30.0, 30.0), ms(3), None);

        let cancelled = translator.cancel_all(ms(4));

        assert_eq!(cancelled.len(), 3);
        assert!(cancelled
            .iter()
            .all(|event| event.phase == PointerPhase::Cancel));
        assert_eq!(
            cancelled
                .iter()
                .map(|event| event.pointer)
                .collect::<Vec<_>>(),
            [MOUSE, PointerId(1), PointerId(2)],
            "sorted, so a caller replaying them gets the same order every run"
        );
        assert_eq!(translator.live_pointers(), 0);
        assert!(
            translator.cancel_all(ms(5)).is_empty(),
            "cancelling twice must not cancel twice"
        );
    }

    #[test]
    fn positions_arrive_in_logical_pixels_whatever_the_display_does() {
        let mut translator = PointerTranslator::new(Scale::new(3.0));
        translator.cursor_moved((300.0, 150.0), ms(0));
        let down = translator
            .mouse_button(PointerButton::Primary, true, ms(1))
            .expect("a down");

        assert_eq!(
            down.position,
            Offset::new(100.0, 50.0),
            "the tree was laid out in logical pixels, so the hit test must be too"
        );
    }
}
