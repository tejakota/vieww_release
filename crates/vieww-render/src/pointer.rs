//! Getting a pointer event to the render objects under it.
//!
//! # Why the route is captured once, on the down
//!
//! Only [`PointerPhase::Down`] hit tests. Every event after it goes to whatever
//! that hit test found, even once the finger has moved somewhere else entirely —
//! which is what makes dragging a scrollbar work after the finger has left it,
//! and what stops a button firing because a drag happened to end over it.
//!
//! Re-hit-testing each move is the intuitive implementation and it is wrong in
//! both directions at once: gestures start in the middle of themselves, and end
//! on whatever the finger drifted over.
//!
//! # One arena, not one per widget
//!
//! Every recogniser on the route contests the *same* arena, because that is the
//! entire point — a button and the list it sits in have to be compared against
//! each other. A dispatcher per render object would give each one its own arena,
//! each with one member, each of which always wins. Which is a way of writing
//! this that passes tests and disambiguates nothing.

use std::fmt;
use std::time::Duration;

use vieww_foundation::{Offset, PointerEvent, PointerId, PointerPhase};
use vieww_gestures::{
    drive, drive_tick, GestureArena, GestureRecognizer, Member, MemberId, Recognized,
};

use crate::{RenderId, RenderTree};
use vieww_foundation::FastMap;

/// A gesture, and the render object it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dispatched {
    /// The render object whose recogniser produced this.
    pub target: RenderId,
    pub gesture: Recognized,
    /// Where the pointer is in that object's own coordinates.
    pub local: Offset,
}

/// One recogniser on a live pointer's route.
struct Contestant {
    target: RenderId,
    /// The target's origin when the pointer went down, for converting global
    /// positions to local ones. Captured rather than looked up per event: the
    /// tree may have relaid out since, and a gesture in progress belongs to where
    /// it started.
    origin: Offset,
    member: MemberId,
    recognizer: Box<dyn GestureRecognizer>,
}

impl fmt::Debug for Contestant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Contestant")
            .field("target", &self.target)
            .field("member", &self.member)
            .field("recognizer", &self.recognizer.debug_name())
            .finish()
    }
}

/// Routes pointer events to the render objects that hit-tested under them.
#[derive(Debug, Default)]
pub struct PointerRouter {
    /// Live pointers and the recognisers contesting them.
    routes: FastMap<PointerId, Vec<Contestant>>,
    arena: GestureArena,
    next_member: u64,
}

impl PointerRouter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many pointers are currently down.
    #[must_use]
    pub fn live_pointers(&self) -> usize {
        self.routes.len()
    }

    /// The render objects on a live pointer's route, innermost first.
    #[must_use]
    pub fn route(&self, pointer: PointerId) -> Vec<RenderId> {
        self.routes
            .get(&pointer)
            .map(|route| {
                let mut targets: Vec<RenderId> =
                    route.iter().map(|contestant| contestant.target).collect();
                targets.dedup();
                targets
            })
            .unwrap_or_default()
    }

    /// `true` while some recogniser on some live route is waiting on a deadline.
    ///
    /// The reason a frame is worth asking for when nothing is animating and
    /// nothing has moved: a finger held still sends no events, so a long press —
    /// or the moment a press becomes visible — can only be reached by a frame
    /// going past. See [`GestureRecognizer::wants_tick`].
    #[must_use]
    pub fn wants_tick(&self) -> bool {
        self.routes.values().flatten().any(|contestant| {
            let recognizer: &dyn GestureRecognizer = contestant.recognizer.as_ref();
            recognizer.wants_tick()
        })
    }

    /// The arena, for inspection.
    #[must_use]
    pub const fn arena(&self) -> &GestureArena {
        &self.arena
    }

    /// Route one event, returning the gestures it produced.
    ///
    /// Delivering them is the caller's job — see
    /// [`FrameDriver::handle_pointer`](crate::FrameDriver::handle_pointer) — so
    /// that this stays a pure function of the tree and the event, and can be
    /// tested by reading what comes back.
    pub fn handle(&mut self, tree: &RenderTree, event: &PointerEvent) -> Vec<Dispatched> {
        if event.phase == PointerPhase::Down {
            self.open_route(tree, event);
        }

        let Some(route) = self.routes.get_mut(&event.pointer) else {
            // A move or an up for a pointer that hit nothing interested.
            return Vec::new();
        };

        let mut members: Vec<Member<'_>> = route
            .iter_mut()
            .map(|contestant| Member {
                id: contestant.member,
                recognizer: contestant.recognizer.as_mut(),
            })
            .collect();
        let mut sink = Vec::new();
        drive(&mut self.arena, &mut members, event, &mut sink);
        drop(members);

        let dispatched = attribute(route, &sink, event.position);
        if event.phase.is_terminal() {
            self.routes.remove(&event.pointer);
        }
        dispatched
    }

    /// Let time pass, for the recognisers that need it.
    ///
    /// Call once a frame with the frame's timestamp. A long press fires from here
    /// and from nowhere else.
    pub fn tick(&mut self, now: Duration) -> Vec<Dispatched> {
        let mut dispatched = Vec::new();
        for route in self.routes.values_mut() {
            let mut members: Vec<Member<'_>> = route
                .iter_mut()
                .map(|contestant| Member {
                    id: contestant.member,
                    recognizer: contestant.recognizer.as_mut(),
                })
                .collect();
            let mut sink = Vec::new();
            drive_tick(&mut self.arena, &mut members, now, &mut sink);
            drop(members);
            dispatched.extend(attribute(route, &sink, Offset::ZERO));
        }
        dispatched
    }

    /// Forget every live pointer.
    ///
    /// For a tree that was torn down under a finger: the `RenderId`s on the route
    /// are stale, and delivering to them would panic on a dead node.
    pub fn clear(&mut self) {
        for pointer in self.routes.keys().copied().collect::<Vec<_>>() {
            let _ = self.arena.cancel(pointer);
        }
        self.routes.clear();
    }

    /// Hit test, and build the contest for what was found.
    fn open_route(&mut self, tree: &RenderTree, event: &PointerEvent) {
        let hits = tree.hit_test(event.position);
        let mut contestants = Vec::new();

        // `bubble` runs from the target outwards, which is also the order members
        // must join in: a sweep awards the pointer to the first still waiting, and
        // that has to be the innermost.
        for entry in hits.bubble() {
            let Some(object) = tree.object(entry.id) else {
                continue;
            };
            let origin = tree.global_offset(entry.id);
            for recognizer in object.gesture_recognizers() {
                let member = MemberId(self.next_member);
                self.next_member += 1;
                contestants.push(Contestant {
                    target: entry.id,
                    origin,
                    member,
                    recognizer,
                });
            }
        }

        if !contestants.is_empty() {
            self.routes.insert(event.pointer, contestants);
        }
    }
}

/// Attach each gesture to the render object whose recogniser produced it.
fn attribute(
    route: &[Contestant],
    sink: &[(MemberId, Recognized)],
    position: Offset,
) -> Vec<Dispatched> {
    sink.iter()
        .filter_map(|&(member, gesture)| {
            let contestant = route.iter().find(|c| c.member == member)?;
            Some(Dispatched {
                target: contestant.target,
                gesture: localise(gesture, contestant.origin),
                local: position - contestant.origin,
            })
        })
        .collect()
}

/// Rewrite a gesture's `local` fields, which the gesture layer left as globals.
///
/// `vieww-gestures` has no tree and cannot know where a widget is, so it reports
/// global coordinates in both fields and this fills the local one in. A widget
/// that reads `local` and gets a global is a bug that only shows up once the
/// widget is not at the origin — which is to say, not in the first test anyone
/// writes.
fn localise(gesture: Recognized, origin: Offset) -> Recognized {
    match gesture {
        Recognized::TapDown(mut details) => {
            details.local = details.position - origin;
            Recognized::TapDown(details)
        }
        Recognized::Tap(mut details) => {
            details.local = details.position - origin;
            Recognized::Tap(details)
        }
        Recognized::DragStart(mut details) => {
            details.local = details.position - origin;
            Recognized::DragStart(details)
        }
        Recognized::DragUpdate(mut details) => {
            details.local = details.position - origin;
            Recognized::DragUpdate(details)
        }
        Recognized::DragEnd(mut details) => {
            details.local = details.position - origin;
            Recognized::DragEnd(details)
        }
        Recognized::LongPress(mut details) => {
            details.local = details.position - origin;
            Recognized::LongPress(details)
        }
        // Tap cancellations carry no position, and a scale's focal point is
        // deliberately global: a pinch spans two widgets as often as one.
        other => other,
    }
}
