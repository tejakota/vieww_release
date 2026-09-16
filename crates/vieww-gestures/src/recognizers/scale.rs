use std::collections::HashMap;

use vieww_foundation::{Offset, PointerEvent, PointerId, PointerPhase, ScaleDetails};

use crate::arena::GestureDisposition;
use crate::recognizer::{GestureRecognizer, Recognized, Sink};

/// A two-finger pinch.
///
/// # Why this one is different
///
/// Every other recogniser here decides about one pointer in isolation. A pinch
/// does not exist until there are two, and neither finger alone means anything —
/// so this joins the arena for *each* pointer and stays undecided until a second
/// one arrives. That is also why it cannot claim early: with one finger down, a
/// pinch and a drag are indistinguishable, and claiming would break every drag
/// that shares a widget with a zoom.
///
/// Rotation is deliberately not recognised. It needs the angle between the
/// pointers, which is easy, and a policy for when a rotation is intended rather
/// than incidental wobble, which is not — and nothing in the framework consumes
/// it yet.
#[derive(Debug, Default)]
pub struct ScaleRecognizer {
    /// Live pointers and where they are, in the order they arrived.
    pointers: Vec<(PointerId, Offset)>,
    /// Set once two fingers were seen, so a `Down` cannot restart a gesture
    /// mid-pinch.
    started: Option<Started>,
    /// Which pointers this recogniser has won, so it knows when it is entitled
    /// to act.
    won: HashMap<PointerId, bool>,
    /// Arenas to claim on the next drain — see
    /// [`GestureRecognizer::drain_claims`].
    claims: Vec<(PointerId, GestureDisposition)>,
}

#[derive(Debug, Clone, Copy)]
struct Started {
    /// Distance between the two pointers when the pinch began. Never zero.
    initial_spread: f32,
    focal: Offset,
    emitted: bool,
}

impl ScaleRecognizer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many pointers are currently down on this recogniser.
    #[must_use]
    pub fn pointer_count(&self) -> usize {
        self.pointers.len()
    }

    fn position(&self, pointer: PointerId) -> Option<Offset> {
        self.pointers
            .iter()
            .find(|(id, _)| *id == pointer)
            .map(|(_, position)| *position)
    }

    /// The midpoint and separation of the first two pointers.
    fn geometry(&self) -> Option<(Offset, f32)> {
        let [(_, a), (_, b)] = self.pointers.get(..2)? else {
            return None;
        };
        let focal = Offset::new((a.dx + b.dx) / 2.0, (a.dy + b.dy) / 2.0);
        Some((focal, (*b - *a).distance()))
    }

    /// `true` once the arena has granted every pointer in the pinch.
    fn owns_the_pinch(&self) -> bool {
        self.pointers.len() >= 2
            && self
                .pointers
                .iter()
                .take(2)
                .all(|(id, _)| self.won.get(id).copied().unwrap_or(false))
    }

    fn emit(&mut self, sink: Sink<'_>) {
        let (Some((focal, spread)), Some(started)) = (self.geometry(), self.started) else {
            return;
        };
        if !self.owns_the_pinch() {
            return;
        }

        let details = ScaleDetails {
            focal,
            scale: spread / started.initial_spread,
            focal_delta: focal - started.focal,
        };
        if started.emitted {
            sink.push(Recognized::ScaleUpdate(details));
        } else {
            sink.push(Recognized::ScaleStart(ScaleDetails {
                scale: 1.0,
                focal_delta: Offset::ZERO,
                ..details
            }));
            self.started = Some(Started {
                emitted: true,
                ..started
            });
        }
    }
}

impl GestureRecognizer for ScaleRecognizer {
    fn debug_name(&self) -> &'static str {
        "ScaleRecognizer"
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
                if self.position(event.pointer).is_none() {
                    self.pointers.push((event.pointer, event.position));
                }
                if self.pointers.len() == 2 && self.started.is_none() {
                    let (focal, spread) = self.geometry()?;
                    if spread <= f32::EPSILON {
                        // Two fingers in exactly the same place have no spread to
                        // scale against, and dividing by it gives infinity.
                        return None;
                    }
                    self.started = Some(Started {
                        initial_spread: spread,
                        focal,
                        emitted: false,
                    });
                    // A second finger is what makes this unambiguous. Nothing
                    // else on screen wants two pointers at once.
                    //
                    // Both are claimed, not just the one that just landed: the
                    // first finger's arena is still open and still contains a
                    // drag that would otherwise go on believing it might win.
                    self.claims = self
                        .pointers
                        .iter()
                        .map(|(id, _)| (*id, GestureDisposition::Accepted))
                        .collect();
                }
                None
            }
            PointerPhase::Move => {
                let slot = self
                    .pointers
                    .iter_mut()
                    .find(|(id, _)| *id == event.pointer)?;
                slot.1 = event.position;
                self.emit(sink);
                None
            }
            PointerPhase::Up | PointerPhase::Cancel => {
                let held = self.position(event.pointer).is_some();
                self.pointers.retain(|(id, _)| *id != event.pointer);
                self.won.remove(&event.pointer);

                if held && self.pointers.len() < 2 {
                    if let Some(started) = self.started.take() {
                        if started.emitted {
                            sink.push(Recognized::ScaleEnd(ScaleDetails {
                                focal: started.focal,
                                scale: 1.0,
                                focal_delta: Offset::ZERO,
                            }));
                        }
                    }
                }
                Some(GestureDisposition::Rejected)
            }
        }
    }

    fn drain_claims(&mut self, out: &mut Vec<(PointerId, GestureDisposition)>) {
        out.append(&mut self.claims);
    }

    fn win(&mut self, pointer: PointerId, sink: Sink<'_>) {
        self.won.insert(pointer, true);
        self.emit(sink);
    }

    fn lose(&mut self, pointer: PointerId, _sink: Sink<'_>) {
        self.pointers.retain(|(id, _)| *id != pointer);
        self.won.remove(&pointer);
        if self.pointers.len() < 2 {
            self.started = None;
        }
    }
}
