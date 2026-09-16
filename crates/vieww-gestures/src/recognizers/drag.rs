use std::collections::HashMap;
use std::time::Duration;

use vieww_foundation::{
    Axis, DragDetails, Modifiers, Offset, PointerEvent, PointerId, PointerPhase,
};

use crate::arena::GestureDisposition;
use crate::recognizer::{GestureRecognizer, Recognized, Sink};
use crate::velocity::VelocityTracker;

/// A press that moved far enough to be a drag.
///
/// # Why it accepts rather than waiting
///
/// Once a finger has travelled past the slop radius the gesture is no longer
/// ambiguous, and holding the arena open past that point costs the user the first
/// frames of their scroll. So passing slop accepts outright — which is also what
/// makes a drag beat a button it started on, since certainty outranks being
/// innermost.
///
/// # Axis restriction
///
/// A vertical list inside a horizontal pager needs both to be recognisers and
/// only one to win. [`Axis`]-restricted drags compare movement along their own
/// axis against the slop, so the one the finger is actually going wins and the
/// other keeps waiting. A drag with no axis takes any direction.
#[derive(Debug, Default)]
pub struct DragRecognizer {
    axis: Option<Axis>,
    pending: HashMap<PointerId, Pending>,
}

#[derive(Debug)]
struct Pending {
    origin: Offset,
    position: Offset,
    /// The timestamp of the most recent event for this pointer, so that a
    /// `DragStart` — which the arena, not an event, triggers — can still be
    /// dated.
    timestamp: Duration,
    slop: f32,
    velocity: VelocityTracker,
    /// `true` once slop was passed and the arena was asked for the pointer.
    claimed: bool,
    /// `true` once the arena granted it and `DragStart` was emitted.
    active: bool,
    /// The modifiers from this pointer's most recent event, so a `DragStart` —
    /// which the arena triggers, not an event — can still report them.
    modifiers: Modifiers,
}

impl DragRecognizer {
    /// A drag in any direction.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A drag that only recognises movement along `axis`.
    #[must_use]
    pub fn along(axis: Axis) -> Self {
        Self {
            axis: Some(axis),
            pending: HashMap::new(),
        }
    }

    /// The axis this recogniser is restricted to, if any.
    #[must_use]
    pub const fn axis(&self) -> Option<Axis> {
        self.axis
    }

    /// How far the pointer has travelled in the direction that matters.
    fn travel(&self, from_origin: Offset) -> f32 {
        match self.axis {
            Some(axis) => from_origin.along(axis).abs(),
            None => from_origin.distance(),
        }
    }

    /// Movement projected onto this recogniser's axis.
    ///
    /// An axis-locked drag reports only the component it owns, so a list being
    /// scrolled with a slightly diagonal finger does not drift sideways.
    fn project(&self, delta: Offset) -> Offset {
        match self.axis {
            Some(Axis::Horizontal) => Offset::new(delta.dx, 0.0),
            Some(Axis::Vertical) => Offset::new(0.0, delta.dy),
            None => delta,
        }
    }
}

impl GestureRecognizer for DragRecognizer {
    fn debug_name(&self) -> &'static str {
        "DragRecognizer"
    }

    fn wants(&self, event: &PointerEvent) -> bool {
        // The primary button only. A right-drag is not a drag, and a
        // middle-click held is not a long press — both are gestures an
        // application defines for itself if it wants them.
        event.is_primary()
    }

    fn handle(&mut self, event: &PointerEvent, sink: Sink<'_>) -> Option<GestureDisposition> {
        match event.phase {
            PointerPhase::Down => {
                let mut velocity = VelocityTracker::new();
                velocity.add(event.timestamp, event.position);
                self.pending.insert(
                    event.pointer,
                    Pending {
                        origin: event.position,
                        position: event.position,
                        timestamp: event.timestamp,
                        slop: event.kind.touch_slop(),
                        velocity,
                        claimed: false,
                        active: false,
                        modifiers: event.modifiers,
                    },
                );
                None
            }
            PointerPhase::Move => {
                let claim = {
                    let pending = self.pending.get_mut(&event.pointer)?;
                    pending.position = event.position;
                    pending.timestamp = event.timestamp;
                    pending.velocity.add(event.timestamp, event.position);
                    // Followed through the drag rather than fixed at the start:
                    // ⌥ pressed or released mid-drag is a real thing to react
                    // to, and a copy-drag that only counts the key held at the
                    // first pixel is one nobody can start deliberately.
                    pending.modifiers = event.modifiers;

                    if pending.active {
                        sink.push(Recognized::DragUpdate(DragDetails {
                            position: event.position,
                            local: event.position,
                            delta: self.project(event.delta),
                            velocity: Offset::ZERO,
                            timestamp: event.timestamp,
                            modifiers: event.modifiers,
                        }));
                        return None;
                    }
                    !pending.claimed
                };

                let pending = self.pending.get(&event.pointer)?;
                let travelled = self.travel(pending.position - pending.origin);
                if claim && travelled > pending.slop {
                    self.pending
                        .get_mut(&event.pointer)
                        .expect("just read")
                        .claimed = true;
                    return Some(GestureDisposition::Accepted);
                }
                None
            }
            PointerPhase::Up => {
                let pending = self.pending.remove(&event.pointer)?;
                if pending.active {
                    sink.push(Recognized::DragEnd(DragDetails {
                        position: event.position,
                        local: event.position,
                        delta: Offset::ZERO,
                        velocity: self.project(pending.velocity.velocity()),
                        timestamp: event.timestamp,
                        modifiers: event.modifiers,
                    }));
                }
                Some(GestureDisposition::Rejected)
            }
            PointerPhase::Cancel => {
                let pending = self.pending.remove(&event.pointer)?;
                if pending.active {
                    // A cancelled drag ends where it is, with no velocity. Flinging
                    // on a cancel would scroll the list as the app backgrounds.
                    sink.push(Recognized::DragEnd(DragDetails {
                        position: event.position,
                        local: event.position,
                        delta: Offset::ZERO,
                        velocity: Offset::ZERO,
                        timestamp: event.timestamp,
                        modifiers: event.modifiers,
                    }));
                }
                Some(GestureDisposition::Rejected)
            }
        }
    }

    fn win(&mut self, pointer: PointerId, sink: Sink<'_>) {
        let Some(pending) = self.pending.get(&pointer) else {
            return;
        };
        if pending.active {
            return;
        }
        let (position, travelled) = (pending.position, pending.position - pending.origin);
        let timestamp = pending.timestamp;
        let modifiers = pending.modifiers;
        self.pending.get_mut(&pointer).expect("just read").active = true;
        sink.push(Recognized::DragStart(DragDetails {
            position,
            local: position,
            // The movement that got us here is reported with the start, so a
            // scroll does not visibly lag the finger by one slop radius.
            delta: self.project(travelled),
            velocity: Offset::ZERO,
            timestamp,
            modifiers,
        }));
    }

    fn lose(&mut self, pointer: PointerId, _sink: Sink<'_>) {
        self.pending.remove(&pointer);
    }
}
