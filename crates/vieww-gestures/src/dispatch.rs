//! Running a set of recognisers against a pointer stream.
//!
//! # The order, which is the whole of it
//!
//! 1. on a `Down`, every recogniser that [`wants`](GestureRecognizer::wants) the
//!    pointer enters the arena;
//! 2. every recogniser sees the event and may state a disposition;
//! 3. multi-pointer recognisers may also claim *other* pointers' arenas;
//! 4. on an `Up`, the arena is **closed**, then the dispositions are applied,
//!    then it is **swept** — in that order.
//!
//! Sweeping before the recognisers have seen the up awards the pointer to
//! whichever member is innermost regardless of what actually happened, and
//! closing after the dispositions means a tap's `Accepted` cannot settle a
//! two-member contest. Both look completely correct in any test where only one
//! recogniser is registered, which is why `tests/disambiguation.rs` always
//! registers two.
//!
//! [`drive`] is this order, once, so that the render layer's router and the
//! standalone [`GestureDispatcher`] cannot drift apart on it.

use std::fmt;
use std::time::Duration;

use vieww_foundation::{PointerEvent, PointerId, PointerPhase};

use crate::arena::{ArenaOutcome, GestureArena, MemberId};
use crate::recognizer::{GestureRecognizer, Recognized};

/// One recogniser and the id it contests under.
pub struct Member<'a> {
    pub id: MemberId,
    pub recognizer: &'a mut dyn GestureRecognizer,
}

impl fmt::Debug for Member<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Member")
            .field("id", &self.id)
            .field("recognizer", &self.recognizer.debug_name())
            .finish()
    }
}

/// Run one pointer event through `members` and `arena`.
///
/// Gestures land in `sink` tagged with the member that produced them, so a
/// caller routing to several widgets knows which one to tell.
///
/// `members` must be in the order they should win a sweep: innermost first.
pub fn drive(
    arena: &mut GestureArena,
    members: &mut [Member<'_>],
    event: &PointerEvent,
    sink: &mut Vec<(MemberId, Recognized)>,
) {
    if event.phase == PointerPhase::Down {
        for member in members.iter() {
            if member.recognizer.wants(event) {
                arena.add(event.pointer, member.id);
            }
        }
    }

    let mut dispositions = Vec::new();
    let mut claims = Vec::new();
    for member in members.iter_mut() {
        if let Some(disposition) =
            tagged(sink, member.id, |out| member.recognizer.handle(event, out))
        {
            dispositions.push((event.pointer, member.id, disposition));
        }
        claims.clear();
        member.recognizer.drain_claims(&mut claims);
        for &(pointer, disposition) in &claims {
            dispositions.push((pointer, member.id, disposition));
        }
    }

    if event.phase == PointerPhase::Cancel {
        // Nobody wins a cancelled pointer, whatever they just said.
        let outcomes = arena.cancel(event.pointer);
        deliver(members, &outcomes, sink);
        return;
    }

    if event.phase == PointerPhase::Up {
        arena.close(event.pointer);
    }

    for (pointer, member, disposition) in dispositions {
        let outcomes = arena.resolve(pointer, member, disposition);
        deliver(members, &outcomes, sink);
    }

    if event.phase == PointerPhase::Up {
        let outcomes = arena.sweep(event.pointer);
        deliver(members, &outcomes, sink);
    }
}

/// Give every recogniser a chance to act on elapsed time.
pub fn drive_tick(
    arena: &mut GestureArena,
    members: &mut [Member<'_>],
    now: Duration,
    sink: &mut Vec<(MemberId, Recognized)>,
) {
    let mut dispositions = Vec::new();
    let mut claims = Vec::new();
    for member in members.iter_mut() {
        if let Some((pointer, disposition)) =
            tagged(sink, member.id, |out| member.recognizer.tick(now, out))
        {
            dispositions.push((pointer, member.id, disposition));
        }
        claims.clear();
        member.recognizer.drain_claims(&mut claims);
        for &(pointer, disposition) in &claims {
            dispositions.push((pointer, member.id, disposition));
        }
    }
    for (pointer, member, disposition) in dispositions {
        let outcomes = arena.resolve(pointer, member, disposition);
        deliver(members, &outcomes, sink);
    }
}

/// Run `f` with a plain sink, then tag whatever it produced with `id`.
///
/// Recognisers push bare gestures — they have no idea what member they are — so
/// attribution is recovered from how much the sink grew.
fn tagged<T>(
    sink: &mut Vec<(MemberId, Recognized)>,
    id: MemberId,
    f: impl FnOnce(&mut Vec<Recognized>) -> T,
) -> T {
    let mut local = Vec::new();
    let result = f(&mut local);
    sink.extend(local.into_iter().map(|gesture| (id, gesture)));
    result
}

fn deliver(
    members: &mut [Member<'_>],
    outcomes: &[ArenaOutcome],
    sink: &mut Vec<(MemberId, Recognized)>,
) {
    // Losers first. A recogniser that lost may need to retract something it
    // showed on the press — a button un-highlighting — and delivering that after
    // the winner's gesture puts the two in the wrong order on screen.
    for won in [false, true] {
        for outcome in outcomes.iter().filter(|outcome| outcome.won == won) {
            let Some(member) = members.iter_mut().find(|m| m.id == outcome.member) else {
                continue;
            };
            let id = member.id;
            tagged(sink, id, |out| {
                if won {
                    member.recognizer.win(outcome.pointer, out);
                } else {
                    member.recognizer.lose(outcome.pointer, out);
                }
            });
        }
    }
}

/// A fixed set of recognisers and the arena they contest in.
///
/// For a caller that has one set of gestures rather than a tree of them — a test,
/// or a single widget wired up by hand. The render layer routes by hit test
/// instead, and drives [`drive`] directly with the members it found.
#[derive(Debug, Default)]
pub struct GestureDispatcher {
    entries: Vec<Entry>,
    arena: GestureArena,
    next_id: u64,
}

struct Entry {
    id: MemberId,
    recognizer: Box<dyn GestureRecognizer>,
}

impl fmt::Debug for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .field("recognizer", &self.recognizer.debug_name())
            .finish()
    }
}

impl GestureDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a recogniser, returning the id it will contest under.
    ///
    /// Order matters: a sweep awards the pointer to the first member still
    /// waiting, so add the one that should win a still finger first.
    pub fn add(&mut self, recognizer: impl GestureRecognizer + 'static) -> MemberId {
        let id = MemberId(self.next_id);
        self.next_id += 1;
        self.entries.push(Entry {
            id,
            recognizer: Box::new(recognizer),
        });
        id
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The arena, for inspection.
    #[must_use]
    pub const fn arena(&self) -> &GestureArena {
        &self.arena
    }

    /// Feed one pointer event through every recogniser and the arena.
    pub fn handle(&mut self, event: &PointerEvent, sink: &mut Vec<Recognized>) {
        let mut tagged_sink = Vec::new();
        let mut members = members_of(&mut self.entries);
        drive(&mut self.arena, &mut members, event, &mut tagged_sink);
        drop(members);
        sink.extend(tagged_sink.into_iter().map(|(_, gesture)| gesture));
    }

    /// Give every recogniser a chance to act on elapsed time. Call once a frame.
    pub fn tick(&mut self, now: Duration, sink: &mut Vec<Recognized>) {
        let mut tagged_sink = Vec::new();
        let mut members = members_of(&mut self.entries);
        drive_tick(&mut self.arena, &mut members, now, &mut tagged_sink);
        drop(members);
        sink.extend(tagged_sink.into_iter().map(|(_, gesture)| gesture));
    }
}

/// Borrow every entry as a [`Member`].
///
/// A free function rather than a method, so that the borrow it takes is of
/// `entries` alone and the arena beside it stays reachable.
fn members_of(entries: &mut [Entry]) -> Vec<Member<'_>> {
    entries
        .iter_mut()
        .map(|entry| Member {
            id: entry.id,
            recognizer: entry.recognizer.as_mut(),
        })
        .collect()
}

/// Run a whole pointer sequence through a dispatcher.
#[must_use]
pub fn recognize(dispatcher: &mut GestureDispatcher, events: &[PointerEvent]) -> Vec<Recognized> {
    let mut sink = Vec::new();
    for event in events {
        dispatcher.handle(event, &mut sink);
    }
    sink
}

/// How many recognisers are still contesting `pointer`.
#[must_use]
pub fn contested(dispatcher: &GestureDispatcher, pointer: PointerId) -> usize {
    dispatcher.arena().members(pointer).len()
}
