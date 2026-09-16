//! What a recogniser is, and what it is allowed to say.

use std::fmt::Debug;
use std::time::Duration;

use vieww_foundation::{
    DragDetails, LongPressDetails, PointerEvent, PointerId, ScaleDetails, TapDetails,
};

use crate::arena::GestureDisposition;

/// A gesture a recogniser decided had happened.
///
/// The `local` field of each detail is left at the global position here — this
/// crate has no tree and cannot know where a widget is. Whoever routes these to a
/// widget fills it in, since it is the one that did the hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Recognized {
    /// A press that has lasted long enough to be worth showing, and might still
    /// become a tap. Exactly one of [`Tap`](Self::Tap) or
    /// [`TapCancel`](Self::TapCancel) follows it.
    ///
    /// Deliberately *not* emitted on the `Down` itself — see
    /// [`PRESS_TIMEOUT`](vieww_foundation::PRESS_TIMEOUT) for why, and
    /// [`TapRecognizer`](crate::TapRecognizer) for when it arrives early.
    TapDown(TapDetails),
    /// A press and release with no significant movement.
    Tap(TapDetails),
    /// A press that ended without becoming a tap — the finger moved away, or
    /// another recogniser won. Buttons use this to un-highlight.
    ///
    /// Only ever follows a [`TapDown`](Self::TapDown): there is nothing to
    /// retract otherwise, and a widget that had to tolerate a bare cancel would
    /// need a flag for whether it was pressed at all.
    TapCancel,
    DragStart(DragDetails),
    DragUpdate(DragDetails),
    /// Carries the release velocity, which is what a fling is simulated from.
    DragEnd(DragDetails),
    LongPress(LongPressDetails),
    ScaleStart(ScaleDetails),
    ScaleUpdate(ScaleDetails),
    ScaleEnd(ScaleDetails),
}

/// Somewhere to put gestures as they are recognised.
///
/// A sink rather than a return value: one pointer event can produce a
/// `DragUpdate` *and* settle the arena, and a recogniser winning can flush
/// several updates it had been holding. Returning them would mean allocating a
/// vector per event, sixty times a second, almost always empty.
pub type Sink<'a> = &'a mut Vec<Recognized>;

/// Turns a stream of pointer events into a gesture, or bows out.
///
/// # The contract
///
/// A recogniser sees **every** event for a pointer it entered the arena for,
/// including after it has lost — it has to know when to stop. It says what it
/// thinks by returning a [`GestureDisposition`], and it must not emit the gesture
/// itself at that moment: `Accepted` is a request, and only [`win`](Self::win)
/// confirms it. A recogniser that emits on `Up` rather than on `win` fires taps
/// that a competing drag should have swallowed.
pub trait GestureRecognizer: Debug {
    fn debug_name(&self) -> &'static str;

    /// Whether this recogniser wants the pointer at all.
    ///
    /// Called on `Down` before entering the arena. A drag recogniser restricted
    /// to one axis still wants every pointer; a recogniser that only handles a
    /// mouse can decline a finger here rather than joining and rejecting.
    fn wants(&self, event: &PointerEvent) -> bool {
        let _ = event;
        true
    }

    /// Handle one pointer event.
    ///
    /// Returns what to tell the arena, or `None` to stay undecided.
    fn handle(&mut self, event: &PointerEvent, sink: Sink<'_>) -> Option<GestureDisposition>;

    /// Dispositions for pointers *other* than the one just handled.
    ///
    /// Drained after every [`handle`](Self::handle) and [`tick`](Self::tick).
    /// Returning a disposition from those covers the single-pointer case, which
    /// is every gesture here except one: a pinch becomes certain when the second
    /// finger lands, and must then claim the *first* finger's arena as well —
    /// which nothing is going to ask it about, because that pointer produced no
    /// event.
    ///
    /// Without this a multi-pointer recogniser wins only the pointer it was
    /// asked about, and then waits forever for the rest.
    fn drain_claims(&mut self, out: &mut Vec<(PointerId, GestureDisposition)>) {
        let _ = out;
    }

    /// Called once per frame, so a recogniser can act on the passage of time.
    ///
    /// A long press fires without any further pointer event, which nothing else
    /// in this design can express: every other transition is driven by input.
    fn tick(&mut self, now: Duration, sink: Sink<'_>) -> Option<(PointerId, GestureDisposition)> {
        let _ = (now, sink);
        None
    }

    /// Whether this recogniser is still waiting on a deadline.
    ///
    /// A finger resting on the screen sends nothing, so the only thing that can
    /// advance [`tick`](Self::tick) is a frame — and an event loop only produces
    /// frames it is asked for. This is that request, and it is deliberately
    /// narrow: a recogniser whose deadlines have all passed says `false`, and a
    /// press held for a minute costs two frames rather than sixty a second.
    ///
    /// Defaults to `false`. Overriding it is not optional for a recogniser that
    /// uses `tick` — without it the deadline is reachable in a test that draws
    /// frames in a row and unreachable in a real window, which is the shape of
    /// bug that survives a whole test suite.
    fn wants_tick(&self) -> bool {
        false
    }

    /// This recogniser won the arena for `pointer`. Emit the gesture now.
    fn win(&mut self, pointer: PointerId, sink: Sink<'_>) {
        let _ = (pointer, sink);
    }

    /// This recogniser lost. Drop any state for `pointer` and emit nothing that
    /// implies the gesture happened.
    fn lose(&mut self, pointer: PointerId, sink: Sink<'_>) {
        let _ = (pointer, sink);
    }
}
