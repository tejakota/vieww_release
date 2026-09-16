use std::collections::HashMap;
use std::time::Duration;

use vieww_foundation::{
    Modifiers, Offset, PointerButton, PointerEvent, PointerId, PointerPhase, TapDetails,
    PRESS_TIMEOUT, TAP_TIMEOUT,
};

use crate::arena::GestureDisposition;
use crate::recognizer::{GestureRecognizer, Recognized, Sink};

/// A press and release in roughly the same place.
///
/// # Why it does not win on `Up`
///
/// Lifting the finger makes a tap *possible*, not certain: a drag that has
/// already accepted owns the pointer, and a tap that fired on `Up` regardless
/// would produce the bug where flicking a list also presses whatever was under
/// the finger. So `Up` accepts — a request — and the tap is emitted only if the
/// arena grants it.
///
/// # When the press is reported
///
/// [`Recognized::TapDown`] is what a control highlights on, and it is emitted at
/// the *later* of two moments and only once:
///
/// - [`PRESS_TIMEOUT`] after the `Down`, from [`tick`](GestureRecognizer::tick) —
///   long enough that a finger which was really starting a scroll has already
///   moved and been rejected without ever having lit anything up;
/// - immediately on [`win`](GestureRecognizer::win), for the tap that came and
///   went faster than that, which would otherwise show no press at all.
///
/// Whichever fires, exactly one [`Tap`](Recognized::Tap) or
/// [`TapCancel`](Recognized::TapCancel) follows, and neither is ever emitted
/// without a `TapDown` before it.
///
/// The tick is the reason a live pointer is a reason to ask for a frame: a
/// finger resting on the screen produces no events at all, so nothing else would
/// ever bring `now` forward. Same dependency as [`LongPressRecognizer`].
///
/// [`LongPressRecognizer`]: crate::LongPressRecognizer
#[derive(Debug, Default)]
pub struct TapRecognizer {
    /// Which button this listens for. A separate recogniser per button rather
    /// than one that reports which: they are different gestures with different
    /// handlers, and a widget that wanted only one would otherwise have to
    /// filter in its callback — after the arena had already been contested on
    /// its behalf.
    button: PointerButton,
    pending: HashMap<PointerId, Pending>,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    down_at: Duration,
    origin: Offset,
    slop: f32,
    /// Set once the finger has strayed too far or held too long. The pointer is
    /// kept rather than dropped, so a later `Up` is not mistaken for a new press.
    dead: bool,
    /// Whether [`Recognized::TapDown`] has gone out for this pointer, which is
    /// the same question as whether there is anything for a cancel to retract.
    down_reported: bool,
    /// The modifiers held when the finger went down.
    ///
    /// From the *press*, not the release, for the same reason `down_at` is: a
    /// ⇧-click is decided by what was held when the click started, and letting
    /// go of Shift before the button comes up does not turn it into a plain one.
    modifiers: Modifiers,
}

impl Pending {
    /// Emit the press if it has not gone out yet, and remember that it has.
    ///
    /// Only the primary button has a press to report. A right-click does not
    /// highlight anything on the way down on any platform, and emitting one here
    /// would drive the press state of whatever the cursor happened to be over.
    fn report_down(&mut self, button: PointerButton, sink: Sink<'_>) {
        if self.down_reported || button != PointerButton::Primary {
            return;
        }
        self.down_reported = true;
        sink.push(Recognized::TapDown(self.details(button)));
    }

    /// Retract the press, if there was one to retract.
    fn cancel(self, sink: Sink<'_>) {
        if self.down_reported {
            sink.push(Recognized::TapCancel);
        }
    }

    /// Where the press happened. `local` is filled in by whoever has a tree.
    const fn details(self, button: PointerButton) -> TapDetails {
        TapDetails {
            position: self.origin,
            local: self.origin,
            button,
            // The press, not the release. A repeat click is counted from when
            // the finger arrived, which is also the instant the caret moved.
            timestamp: self.down_at,
            modifiers: self.modifiers,
        }
    }
}

impl TapRecognizer {
    /// A tap with the primary button, which is every ordinary tap.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A tap with some other button — a right-click, say.
    #[must_use]
    pub fn for_button(button: PointerButton) -> Self {
        Self {
            button,
            pending: HashMap::new(),
        }
    }

    /// How many pointers this recogniser is currently watching.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.pending.len()
    }
}

impl GestureRecognizer for TapRecognizer {
    fn debug_name(&self) -> &'static str {
        "TapRecognizer"
    }

    fn wants(&self, event: &PointerEvent) -> bool {
        // Declined here rather than joined and rejected, so that a right-click
        // does not sit in the arena keeping a real contest undecided.
        event.button == self.button
    }

    fn handle(&mut self, event: &PointerEvent, sink: Sink<'_>) -> Option<GestureDisposition> {
        match event.phase {
            PointerPhase::Down => {
                self.pending.insert(
                    event.pointer,
                    Pending {
                        down_at: event.timestamp,
                        origin: event.position,
                        slop: event.kind.touch_slop(),
                        dead: false,
                        down_reported: false,
                        modifiers: event.modifiers,
                    },
                );
                None
            }
            PointerPhase::Move => {
                let pending = self.pending.get_mut(&event.pointer)?;
                if pending.dead {
                    return None;
                }
                let strayed = (event.position - pending.origin).distance_squared()
                    > pending.slop * pending.slop;
                let held = event.timestamp.saturating_sub(pending.down_at) > TAP_TIMEOUT;
                if strayed || held {
                    pending.dead = true;
                    pending.cancel(sink);
                    return Some(GestureDisposition::Rejected);
                }
                None
            }
            PointerPhase::Up => {
                let pending = self.pending.get(&event.pointer)?;
                if pending.dead {
                    return None;
                }
                if event.timestamp.saturating_sub(pending.down_at) > TAP_TIMEOUT {
                    // A press held this long is a long press, whatever happens
                    // when it lifts. Without this, a long press also fires a tap.
                    let pending = self.pending.remove(&event.pointer).expect("just read");
                    pending.cancel(sink);
                    return Some(GestureDisposition::Rejected);
                }
                Some(GestureDisposition::Accepted)
            }
            PointerPhase::Cancel => {
                if let Some(pending) = self.pending.remove(&event.pointer) {
                    if !pending.dead {
                        pending.cancel(sink);
                    }
                }
                Some(GestureDisposition::Rejected)
            }
        }
    }

    fn tick(&mut self, now: Duration, sink: Sink<'_>) -> Option<(PointerId, GestureDisposition)> {
        for pending in self.pending.values_mut() {
            if !pending.dead && now.saturating_sub(pending.down_at) >= PRESS_TIMEOUT {
                pending.report_down(self.button, sink);
            }
        }
        // Time alone never settles a tap: the finger has to lift. This says
        // nothing to the arena, and a press that outlives `TAP_TIMEOUT` is left
        // showing until it does — the alternative un-highlights under a finger
        // that is still down, half a second before the long press it is becoming.
        None
    }

    fn wants_tick(&self) -> bool {
        // A secondary tap reports no press, so it is waiting on no deadline and
        // must not hold the frame loop open.
        self.button == PointerButton::Primary
            && self
                .pending
                .values()
                .any(|pending| !pending.dead && !pending.down_reported)
    }

    fn win(&mut self, pointer: PointerId, sink: Sink<'_>) {
        if let Some(mut pending) = self.pending.remove(&pointer) {
            // A tap quicker than `PRESS_TIMEOUT` has had no tick, and would
            // otherwise produce a `Tap` with no press before it — which every
            // reader of these would have to special-case.
            pending.report_down(self.button, sink);
            sink.push(Recognized::Tap(pending.details(self.button)));
        }
    }

    fn lose(&mut self, pointer: PointerId, sink: Sink<'_>) {
        if let Some(pending) = self.pending.remove(&pointer) {
            if !pending.dead {
                // Something else won — a drag, most likely. Anything that
                // highlighted itself on the press has to be told to stop.
                pending.cancel(sink);
            }
        }
    }
}

#[cfg(test)]
mod modifier_tests {
    use super::*;
    use crate::recognizer::Recognized;
    use vieww_foundation::{Modifiers, PointerDeviceKind};

    fn down(modifiers: Modifiers) -> PointerEvent {
        let mut event = PointerEvent::down(PointerId(1), Offset::new(4.0, 4.0), Duration::ZERO);
        event.kind = PointerDeviceKind::Mouse;
        event.modifiers = modifiers;
        event
    }

    /// The gap that kept ⇧-click and ⌘-click out of every application built on
    /// vieww: a tap arrived with no record of what was held, and nothing could
    /// go back and ask.
    #[test]
    fn a_tap_carries_the_modifiers_that_were_held_at_the_press() {
        let mut recognizer = TapRecognizer::new();
        let mut out = Vec::new();
        recognizer.handle(&down(Modifiers::SHIFT), &mut out);
        // The press goes out on the tick that passes `PRESS_TIMEOUT`, or on the
        // win for a tap quicker than that. This drives the first.
        recognizer.tick(PRESS_TIMEOUT, &mut out);

        let reported = out.iter().find_map(|event| match event {
            Recognized::TapDown(details) => Some(details.modifiers),
            _ => None,
        });
        assert_eq!(reported, Some(Modifiers::SHIFT));
    }

    #[test]
    fn a_plain_press_reports_none() {
        let mut recognizer = TapRecognizer::new();
        let mut out = Vec::new();
        recognizer.handle(&down(Modifiers::NONE), &mut out);
        recognizer.tick(PRESS_TIMEOUT, &mut out);
        let reported = out.iter().find_map(|event| match event {
            Recognized::TapDown(details) => Some(details.modifiers),
            _ => None,
        });
        assert_eq!(reported, Some(Modifiers::NONE));
    }
}
