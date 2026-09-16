//! Deciding which recogniser gets a pointer.
//!
//! # The problem
//!
//! A tap on a button inside a scrollable list is ambiguous at the moment the
//! finger lands, and stays ambiguous for as long as the finger is still. Whether
//! it was a tap or the beginning of a scroll is not knowable until either the
//! finger moves far enough or it lifts. Deciding early is what produces the two
//! classic bugs: a list that will not scroll because every drag starts on a
//! button, and buttons that will not press because the list claimed the touch.
//!
//! # The resolution
//!
//! Every recogniser that hit-tested under the pointer joins an *arena* for it,
//! and the arena holds the ambiguity until it can be resolved one of three ways:
//!
//! - a member **accepts**, declaring it is certain — a drag that has passed the
//!   slop threshold knows it is a drag, and wins immediately;
//! - members **reject** until one is left, and the last one standing wins by
//!   default — this is how a tap wins when nothing moved;
//! - the pointer lifts and the arena is **swept**, which awards it to the member
//!   that has been waiting longest.
//!
//! The order matters and is not arbitrary. Sweeping picks the *first* member,
//! and members are added innermost-first as the hit test unwinds, so a button
//! beats the list it sits in — while a drag, which accepts outright, beats the
//! button because certainty outranks position.
//!
//! This is the classic gesture-arena algorithm. It is worth taking rather than reinventing: the
//! failure modes are subtle, they only appear under a real finger, and every one
//! of them has already been found.

use std::collections::HashMap;
use std::fmt;

use vieww_foundation::PointerId;

/// Identifies a recogniser within an arena.
///
/// Opaque and assigned by whoever owns the recognisers, so this crate needs to
/// know nothing about render objects or widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MemberId(pub u64);

impl fmt::Display for MemberId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "m{}", self.0)
    }
}

/// What a recogniser tells the arena about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureDisposition {
    /// "I am certain this is my gesture." Wins immediately, and every other
    /// member of that arena loses.
    Accepted,
    /// "This is not my gesture." Bows out; if one member remains, it wins.
    Rejected,
}

/// What the arena decided about one member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaOutcome {
    pub pointer: PointerId,
    pub member: MemberId,
    /// `true` for the single winner, `false` for everyone else.
    pub won: bool,
}

#[derive(Debug)]
struct Contest {
    /// In join order, which is hit-test order: innermost first.
    members: Vec<MemberId>,
    /// Members that have already been told an outcome, and may not re-enter.
    ///
    /// # Why re-entry has to be refused
    ///
    /// A recogniser that rejected has *stood down* — it has run whatever it does
    /// on losing, and it is not watching this pointer any more. Letting it back
    /// in means it can be awarded the gesture afterwards: told it lost, then
    /// told it won, for one touch. A drag that fires its cancel path and then
    /// its start path leaves a scroll position moving after the finger is gone.
    ///
    /// And re-entry is not exotic. Members are added as pointer events are
    /// routed, so a recogniser rebuilt between a move and the up is re-added by
    /// the ordinary path.
    ///
    /// Found by `tests/arena_properties.rs`.
    decided: Vec<MemberId>,
    /// `true` once the pointer is up, so no new member may join. The contest can
    /// still be open — a sweep is what closes it.
    closed: bool,
    resolved: bool,
}

/// One arena per live pointer.
///
/// Pure bookkeeping: it never calls a recogniser, it returns what it decided and
/// lets the caller deliver that. Keeping it that way is what makes the
/// disambiguation testable without a tree, a frame or a recogniser in sight.
#[derive(Debug, Default)]
pub struct GestureArena {
    contests: HashMap<PointerId, Contest>,
}

impl GestureArena {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many pointers currently have an **unresolved** contest.
    ///
    /// A pointer whose gesture has already been awarded does not count, even
    /// though a tombstone for it is still held until the pointer lifts — see
    /// [`add`](Self::add). "How many contests are in progress" is the question
    /// a caller is asking, and a decided one is not in progress.
    #[must_use]
    pub fn len(&self) -> usize {
        self.contests
            .values()
            .filter(|contest| !contest.resolved)
            .count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many pointers this arena is holding anything at all for, decided or
    /// not.
    ///
    /// The number that has to reach zero for the arena to have leaked nothing.
    /// A tombstone is removed by the sweep or cancel that ends its pointer, so
    /// in a running application this tracks the fingers on the glass.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.contests.len()
    }

    /// The members still contesting a pointer, in join order.
    #[must_use]
    pub fn members(&self, pointer: PointerId) -> &[MemberId] {
        self.contests
            .get(&pointer)
            .map_or(&[], |contest| &contest.members)
    }

    /// Enter a recogniser into the contest for `pointer`.
    ///
    /// Ignored once the pointer is up **or the gesture has been awarded**: a
    /// recogniser that was not there for the down has no claim on a gesture that
    /// already happened.
    ///
    /// # The hole this guard used to have
    ///
    /// The check was here from the start and could not fire, because resolving a
    /// contest **removed it from the map** — so a later `add` for the same
    /// pointer found no entry, created a fresh one with `resolved: false`, and
    /// opened a second contest on a pointer whose gesture had already been
    /// awarded. That pointer could then produce a *second* winner, which is
    /// precisely what the arena exists to make impossible: one touch, one
    /// gesture.
    ///
    /// It is reachable whenever a recogniser registers part-way through a
    /// pointer's life — a rebuild between the down and the up, which is ordinary
    /// in a list that is loading. A decided contest is therefore kept as a
    /// tombstone with `resolved: true` and no members, and removed by the
    /// [`sweep`](Self::sweep) or [`cancel`](Self::cancel) that ends the pointer.
    /// Both already returned nothing for a resolved contest, so neither needed
    /// changing.
    ///
    /// Found by `tests/arena_properties.rs`, which asserts "at most one member
    /// ever wins" against randomised orderings rather than the handful anybody
    /// thought to write down.
    pub fn add(&mut self, pointer: PointerId, member: MemberId) {
        let contest = self.contests.entry(pointer).or_insert_with(|| Contest {
            members: Vec::new(),
            decided: Vec::new(),
            closed: false,
            resolved: false,
        });
        if contest.closed || contest.resolved || contest.decided.contains(&member) {
            return;
        }
        if !contest.members.contains(&member) {
            contest.members.push(member);
        }
    }

    /// Stop new members joining, without deciding anything.
    ///
    /// Called when the pointer lifts. The contest may still resolve after this —
    /// a tap accepts on the up event, and the sweep that follows is what settles
    /// the case where nobody did.
    pub fn close(&mut self, pointer: PointerId) {
        if let Some(contest) = self.contests.get_mut(&pointer) {
            contest.closed = true;
        }
    }

    /// Record a member's disposition, resolving the contest if that settles it.
    ///
    /// Returns the outcomes to deliver, which is empty until something is
    /// actually decided.
    pub fn resolve(
        &mut self,
        pointer: PointerId,
        member: MemberId,
        disposition: GestureDisposition,
    ) -> Vec<ArenaOutcome> {
        let Some(contest) = self.contests.get_mut(&pointer) else {
            return Vec::new();
        };
        if contest.resolved || !contest.members.contains(&member) {
            return Vec::new();
        }

        match disposition {
            GestureDisposition::Accepted => {
                let members = std::mem::take(&mut contest.members);
                contest.decided.extend(members.iter().copied());
                // A tombstone, not a removal — see `add`. The entry stays with
                // `resolved: true` so a recogniser joining later is refused
                // rather than starting a second contest on a decided pointer.
                contest.resolved = true;
                members
                    .into_iter()
                    .map(|id| ArenaOutcome {
                        pointer,
                        member: id,
                        won: id == member,
                    })
                    .collect()
            }
            GestureDisposition::Rejected => {
                contest.members.retain(|&id| id != member);
                contest.decided.push(member);
                let mut outcomes = vec![ArenaOutcome {
                    pointer,
                    member,
                    won: false,
                }];

                // A single survivor wins by default, but only once nobody else
                // can join. Before the pointer is up, more recognisers may still
                // arrive — awarding it now would hand the gesture to whichever
                // one happened to be registered first.
                if contest.closed && contest.members.len() == 1 {
                    let winner = contest.members.remove(0);
                    contest.decided.push(winner);
                    contest.resolved = true;
                    outcomes.push(ArenaOutcome {
                        pointer,
                        member: winner,
                        won: true,
                    });
                } else if contest.members.is_empty() {
                    // Everybody bowed out and nobody won. Still a tombstone: the
                    // pointer is spoken for until it lifts, and a recogniser
                    // arriving now would be contesting a gesture that is over.
                    contest.resolved = true;
                }
                outcomes
            }
        }
    }

    /// Award the contest to whoever has waited longest, and end it.
    ///
    /// Called after the pointer is up and every recogniser has seen it. The
    /// *first* member wins, and since members join as the hit test unwinds from
    /// the target outwards, that is the innermost one — a button inside a list
    /// beats the list.
    pub fn sweep(&mut self, pointer: PointerId) -> Vec<ArenaOutcome> {
        let Some(contest) = self.contests.remove(&pointer) else {
            return Vec::new();
        };
        if contest.resolved {
            return Vec::new();
        }
        contest
            .members
            .iter()
            .enumerate()
            .map(|(index, &member)| ArenaOutcome {
                pointer,
                member,
                won: index == 0,
            })
            .collect()
    }

    /// Abandon a contest with nobody winning.
    ///
    /// For a cancelled pointer: the platform took the touch away, so no gesture
    /// happened and every member must be told it lost.
    pub fn cancel(&mut self, pointer: PointerId) -> Vec<ArenaOutcome> {
        let Some(contest) = self.contests.remove(&pointer) else {
            return Vec::new();
        };
        // A tombstone has no members left to tell, so this is empty for a
        // contest that already resolved — which is correct: they were told when
        // it did.
        contest
            .members
            .iter()
            .map(|&member| ArenaOutcome {
                pointer,
                member,
                won: false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINTER: PointerId = PointerId(1);
    const BUTTON: MemberId = MemberId(1);
    const LIST: MemberId = MemberId(2);

    fn winners(outcomes: &[ArenaOutcome]) -> Vec<MemberId> {
        outcomes
            .iter()
            .filter(|outcome| outcome.won)
            .map(|outcome| outcome.member)
            .collect()
    }

    fn losers(outcomes: &[ArenaOutcome]) -> Vec<MemberId> {
        outcomes
            .iter()
            .filter(|outcome| !outcome.won)
            .map(|outcome| outcome.member)
            .collect()
    }

    #[test]
    fn accepting_wins_immediately_and_everyone_else_loses() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);

        let outcomes = arena.resolve(POINTER, LIST, GestureDisposition::Accepted);

        assert_eq!(winners(&outcomes), vec![LIST]);
        assert_eq!(losers(&outcomes), vec![BUTTON]);
        assert!(arena.is_empty(), "the contest is over");
    }

    #[test]
    fn a_drag_beats_a_button_it_started_on() {
        // The whole point of the arena, stated as the case it exists for.
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);

        // The finger moved past the slop threshold, so the list is now certain.
        let outcomes = arena.resolve(POINTER, LIST, GestureDisposition::Accepted);

        assert_eq!(
            winners(&outcomes),
            vec![LIST],
            "certainty outranks being innermost"
        );
    }

    #[test]
    fn a_button_beats_the_list_it_sits_in_when_nothing_moved() {
        let mut arena = GestureArena::new();
        // Members join as the hit test unwinds, so the innermost is first.
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);

        arena.close(POINTER);
        let outcomes = arena.sweep(POINTER);

        assert_eq!(
            winners(&outcomes),
            vec![BUTTON],
            "a sweep awards the pointer to the innermost waiting member"
        );
        assert_eq!(losers(&outcomes), vec![LIST]);
    }

    #[test]
    fn the_last_member_standing_wins_once_the_pointer_is_up() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);
        arena.close(POINTER);

        let outcomes = arena.resolve(POINTER, BUTTON, GestureDisposition::Rejected);

        assert_eq!(winners(&outcomes), vec![LIST]);
        assert!(arena.is_empty());
    }

    #[test]
    fn a_lone_survivor_does_not_win_while_others_can_still_join() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);

        // Still down: the hit test may not have finished adding members.
        let outcomes = arena.resolve(POINTER, BUTTON, GestureDisposition::Rejected);

        assert!(
            winners(&outcomes).is_empty(),
            "awarding now would hand the gesture to whoever registered first"
        );
        assert_eq!(arena.members(POINTER), [LIST]);
    }

    #[test]
    fn a_member_that_arrives_after_the_pointer_is_up_is_ignored() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.close(POINTER);
        arena.add(POINTER, LIST);

        assert_eq!(
            arena.members(POINTER),
            [BUTTON],
            "a recogniser that was not there for the down has no claim"
        );
    }

    #[test]
    fn cancelling_ends_the_contest_with_no_winner() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);

        let outcomes = arena.cancel(POINTER);

        assert!(
            winners(&outcomes).is_empty(),
            "the platform took the touch away, so no gesture happened"
        );
        assert_eq!(losers(&outcomes).len(), 2, "and both are told so");
        assert!(arena.is_empty());
    }

    #[test]
    fn resolving_a_settled_contest_twice_decides_nothing_more() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, LIST);
        arena.resolve(POINTER, LIST, GestureDisposition::Accepted);

        assert!(
            arena
                .resolve(POINTER, BUTTON, GestureDisposition::Rejected)
                .is_empty(),
            "a recogniser told it lost may still be finishing its own bookkeeping"
        );
        assert!(arena.sweep(POINTER).is_empty());
    }

    #[test]
    fn each_pointer_gets_its_own_contest() {
        let mut arena = GestureArena::new();
        let second = PointerId(2);
        arena.add(POINTER, BUTTON);
        arena.add(second, LIST);

        arena.resolve(POINTER, BUTTON, GestureDisposition::Accepted);

        assert_eq!(
            arena.members(second),
            [LIST],
            "two fingers are two independent sequences that happen to interleave"
        );
    }

    #[test]
    fn a_member_only_joins_once() {
        let mut arena = GestureArena::new();
        arena.add(POINTER, BUTTON);
        arena.add(POINTER, BUTTON);

        assert_eq!(arena.members(POINTER), [BUTTON]);
    }
}
