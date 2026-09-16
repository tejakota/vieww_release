use std::collections::HashMap;
use std::time::Duration;

use vieww_foundation::{
    LongPressDetails, Offset, PointerEvent, PointerId, PointerPhase, LONG_PRESS_TIMEOUT,
};

use crate::arena::GestureDisposition;
use crate::recognizer::{GestureRecognizer, Recognized, Sink};

/// A press held still for long enough.
///
/// The only recogniser whose gesture is triggered by *nothing happening*, which
/// is why [`GestureRecognizer::tick`] exists: every other transition here is
/// driven by an incoming pointer event, and a finger resting on the screen
/// produces none.
#[derive(Debug)]
pub struct LongPressRecognizer {
    timeout: Duration,
    pending: HashMap<PointerId, Pending>,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    down_at: Duration,
    origin: Offset,
    slop: f32,
    /// Set once the timeout has been reached and the arena asked for the
    /// pointer, so the tick does not ask again every frame the finger stays down.
    claimed: bool,
}

impl Default for LongPressRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl LongPressRecognizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            timeout: LONG_PRESS_TIMEOUT,
            pending: HashMap::new(),
        }
    }

    /// A long press with a non-standard hold time.
    #[must_use]
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            pending: HashMap::new(),
        }
    }
}

impl GestureRecognizer for LongPressRecognizer {
    fn debug_name(&self) -> &'static str {
        "LongPressRecognizer"
    }

    fn wants(&self, event: &PointerEvent) -> bool {
        // The primary button only. A right-drag is not a drag, and a
        // middle-click held is not a long press — both are gestures an
        // application defines for itself if it wants them.
        event.is_primary()
    }

    fn handle(&mut self, event: &PointerEvent, _sink: Sink<'_>) -> Option<GestureDisposition> {
        match event.phase {
            PointerPhase::Down => {
                self.pending.insert(
                    event.pointer,
                    Pending {
                        down_at: event.timestamp,
                        origin: event.position,
                        slop: event.kind.touch_slop(),
                        claimed: false,
                    },
                );
                None
            }
            PointerPhase::Move => {
                let pending = self.pending.get(&event.pointer)?;
                if pending.claimed {
                    // Already won; a finger that wanders after the press has
                    // registered does not un-register it.
                    return None;
                }
                let strayed = (event.position - pending.origin).distance_squared()
                    > pending.slop * pending.slop;
                if strayed {
                    self.pending.remove(&event.pointer);
                    return Some(GestureDisposition::Rejected);
                }
                None
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                // Lifting before the timeout is a tap or nothing. Lifting after
                // it means this already won and there is nothing more to say.
                let claimed = self
                    .pending
                    .remove(&event.pointer)
                    .is_some_and(|p| p.claimed);
                (!claimed).then_some(GestureDisposition::Rejected)
            }
        }
    }

    fn tick(&mut self, now: Duration, _sink: Sink<'_>) -> Option<(PointerId, GestureDisposition)> {
        let ready = self.pending.iter().find_map(|(&pointer, pending)| {
            (!pending.claimed && now.saturating_sub(pending.down_at) >= self.timeout)
                .then_some(pointer)
        })?;
        self.pending.get_mut(&ready).expect("just found").claimed = true;
        Some((ready, GestureDisposition::Accepted))
    }

    fn wants_tick(&self) -> bool {
        // Only until the press has fired. After that the finger can rest as long
        // as it likes and nothing here is waiting on the clock.
        self.pending.values().any(|pending| !pending.claimed)
    }

    fn win(&mut self, pointer: PointerId, sink: Sink<'_>) {
        // Read rather than removed: the press is still down, and a `Move` after
        // this has to find the entry to know it must not reject.
        if let Some(pending) = self.pending.get(&pointer) {
            sink.push(Recognized::LongPress(LongPressDetails {
                position: pending.origin,
                local: pending.origin,
            }));
        }
    }

    fn lose(&mut self, pointer: PointerId, _sink: Sink<'_>) {
        self.pending.remove(&pointer);
    }
}
