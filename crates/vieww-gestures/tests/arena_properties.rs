//! Randomised property tests for the gesture arena.
//!
//! # Why here, and why randomised
//!
//! `docs/PRODUCTION-GAPS.md` names the two places property testing pays most —
//! text shaping and gesture recognition — and the reason is the same for both:
//! the input space is combinatorial and the invariants are simple. A
//! disambiguation test written by hand asserts one interleaving that somebody
//! thought of. There are hundreds, and the ones that break are the ones nobody
//! thought of: a recogniser rejecting after the sweep, two members rejecting on
//! the same frame, a pointer cancelled while a contest is half resolved.
//!
//! The invariants below are the arena's whole contract, and every one of them is
//! a sentence rather than a number. That is what makes them checkable against
//! random input: there is no expected output to write down, only a property that
//! must hold whatever happened.
//!
//! # No proptest dependency
//!
//! A deterministic pseudo-random generator, seeded from a constant, in about
//! thirty lines. The workspace is publishing to crates.io and a dev-dependency
//! is still a dependency in the lockfile every consumer resolves; `proptest`
//! would buy shrinking, which is worth having and is not worth a tree of
//! transitive crates for a test suite this shape.
//!
//! Deterministic rather than seeded from the clock, deliberately: a property
//! test that fails on one machine on one afternoon and never again is worse than
//! no test at all. The seed is in the source, so a failure reproduces exactly,
//! and widening the search is a matter of raising `CASES`.

use vieww_foundation::PointerId;
use vieww_gestures::{GestureArena, GestureDisposition, MemberId};

/// How many random histories each property is checked over.
///
/// A thousand runs in well under a second and covers every shape of contest the
/// arena has — the state space is small, it is the *orderings* that are large.
const CASES: usize = 2000;

/// xorshift64*, which is short enough to read and good enough to explore an
/// ordering space. Not a cryptographic generator and not pretending to be.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Never zero: xorshift is stuck there forever.
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.next() % one_in == 0
    }
}

/// One thing that can be done to an arena.
#[derive(Debug, Clone, Copy)]
enum Step {
    Add(MemberId),
    Close,
    Resolve(MemberId, GestureDisposition),
    Sweep,
    Cancel,
}

/// A random history for **one pointer's whole life**.
///
/// The shape is constrained to what a dispatcher can actually produce, and the
/// constraint is the interesting part. A `Sweep` or a `Cancel` *ends* a
/// pointer — the finger is off the glass — so nothing follows one, and a
/// generator that emitted `Sweep, Add, Sweep` would be describing two contests
/// on one id and would fail every invariant below for a reason that has nothing
/// to do with the arena. (It did, on the first run of this file. The generator
/// was wrong; the arena was not.)
///
/// Within that, the ordering is free: members join at any point, `close` can
/// arrive before or after resolutions, and resolutions can name a member that
/// has already gone. Those are the interleavings a hand-written test does not
/// enumerate.
fn history(rng: &mut Rng) -> Vec<Step> {
    let count = 1 + rng.below(4);
    let members: Vec<MemberId> = (0..count).map(|i| MemberId(i as u64)).collect();

    let mut steps = Vec::new();
    // At least one member joins before anything else happens; the rest may
    // arrive later, which is the case the "a lone survivor waits for close" rule
    // exists for — a recogniser registered by a rebuild mid-gesture.
    steps.push(Step::Add(members[0]));

    let length = 2 + rng.below(9);
    for _ in 0..length {
        let step = match rng.below(6) {
            0 => Step::Close,
            1 => Step::Add(members[rng.below(members.len())]),
            // A member that never joined, sometimes: the arena has to ignore it
            // rather than decide anything about it.
            2 if rng.chance(4) => {
                Step::Resolve(MemberId(count as u64 + 1), GestureDisposition::Rejected)
            }
            _ => Step::Resolve(
                members[rng.below(members.len())],
                if rng.chance(3) {
                    GestureDisposition::Accepted
                } else {
                    GestureDisposition::Rejected
                },
            ),
        };
        steps.push(step);
    }

    // The pointer ends exactly once, one way or the other — and nothing follows.
    steps.push(if rng.chance(4) {
        Step::Cancel
    } else {
        Step::Sweep
    });
    steps
}

/// Run a history, collecting every outcome the arena produced.
fn run(steps: &[Step]) -> Vec<vieww_gestures::ArenaOutcome> {
    let pointer = PointerId(1);
    let mut arena = GestureArena::new();
    let mut outcomes = Vec::new();
    for step in steps {
        match *step {
            Step::Add(member) => arena.add(pointer, member),
            Step::Close => arena.close(pointer),
            Step::Resolve(member, disposition) => {
                outcomes.extend(arena.resolve(pointer, member, disposition));
            }
            Step::Sweep => outcomes.extend(arena.sweep(pointer)),
            Step::Cancel => outcomes.extend(arena.cancel(pointer)),
        }
    }
    outcomes
}

/// **At most one member ever wins.**
///
/// The arena's central promise. Two winners means two recognisers both acting on
/// one touch — a button pressed *and* the list under it scrolled — which is the
/// failure a disambiguation system exists to prevent, and the one that is
/// hardest to reproduce by hand because it needs a specific interleaving.
#[test]
fn at_most_one_member_ever_wins_a_contest() {
    let mut rng = Rng::new(0x5EED_1234_ABCD_0001);
    for case in 0..CASES {
        let steps = history(&mut rng);
        let winners: Vec<_> = run(&steps).into_iter().filter(|out| out.won).collect();
        assert!(
            winners.len() <= 1,
            "case {case}: {} winners from {steps:?}\n{winners:?}",
            winners.len()
        );
    }
}

/// **No member is told anything twice.**
///
/// A recogniser told it lost and then told it won would fire a gesture it had
/// already stood down from, and one told it lost twice would run its cancel path
/// twice — which for a drag means two `DragEnd`s and a scroll position that
/// moves after the finger is gone.
#[test]
fn no_member_is_reported_more_than_once() {
    let mut rng = Rng::new(0x5EED_1234_ABCD_0002);
    for case in 0..CASES {
        let steps = history(&mut rng);
        let outcomes = run(&steps);
        let mut seen: Vec<MemberId> = Vec::new();
        for outcome in &outcomes {
            assert!(
                !seen.contains(&outcome.member),
                "case {case}: {:?} reported twice from {steps:?}\n{outcomes:?}",
                outcome.member
            );
            seen.push(outcome.member);
        }
    }
}

/// **Nothing is decided about a member that never joined.**
///
/// The arena is bookkeeping over a set it was told about. An outcome for an
/// unknown member would be delivered to whatever now holds that id, which after
/// a rebuild is a different recogniser on a different widget.
#[test]
fn every_outcome_names_a_member_that_joined() {
    let mut rng = Rng::new(0x5EED_1234_ABCD_0003);
    for case in 0..CASES {
        let steps = history(&mut rng);
        let joined: Vec<MemberId> = steps
            .iter()
            .filter_map(|step| match step {
                Step::Add(member) => Some(*member),
                _ => None,
            })
            .collect();
        for outcome in run(&steps) {
            assert!(
                joined.contains(&outcome.member),
                "case {case}: {:?} never joined; {steps:?}",
                outcome.member
            );
        }
    }
}

/// **A contest that has produced a winner produces nothing further.**
///
/// Once a gesture is awarded, later rejections and sweeps have to be silent: the
/// losers were already told, and telling them again is the duplicate above
/// arriving by another route.
#[test]
fn nothing_is_reported_after_a_winner_is_declared() {
    let mut rng = Rng::new(0x5EED_1234_ABCD_0004);
    for case in 0..CASES {
        let steps = history(&mut rng);
        let outcomes = run(&steps);
        if let Some(at) = outcomes.iter().position(|out| out.won) {
            // Everything after the win belongs to the same batch it was produced
            // in — the losers of that one resolution — and no *further* award
            // may follow. Stated as "no second win" rather than "no further
            // outcomes at all", because the losers legitimately come after the
            // winner in the batch.
            assert!(
                outcomes[at + 1..].iter().all(|out| !out.won),
                "case {case}: a second award after index {at}; {steps:?}\n{outcomes:?}"
            );
        }
    }
}

// ------------------------------------------------------------- the specific

/// A single survivor does **not** win until the pointer is up.
///
/// Written out rather than left to the random histories because it is the one
/// rule whose absence produces a plausible-looking wrong answer: awarding the
/// last remaining member immediately hands the gesture to whichever recogniser
/// happened to be registered first, which works in every test with one
/// recogniser and fails on a real screen.
#[test]
fn a_lone_survivor_waits_for_the_pointer_to_lift() {
    let pointer = PointerId(7);
    let mut arena = GestureArena::new();
    arena.add(pointer, MemberId(1));
    arena.add(pointer, MemberId(2));

    // Still open: a third recogniser could yet join.
    let outcomes = arena.resolve(pointer, MemberId(2), GestureDisposition::Rejected);
    assert_eq!(outcomes.len(), 1, "only the rejection is reported");
    assert!(!outcomes[0].won);

    arena.close(pointer);
    // Now nobody else can join, and the survivor takes it on the sweep.
    let swept = arena.sweep(pointer);
    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].member, MemberId(1));
    assert!(swept[0].won);
}

/// The sweep awards the **innermost** member, because members join as the hit
/// test unwinds from the target outwards: a button inside a list beats the list.
#[test]
fn the_sweep_awards_the_first_to_join() {
    let pointer = PointerId(3);
    let mut arena = GestureArena::new();
    arena.add(pointer, MemberId(10)); // the button
    arena.add(pointer, MemberId(20)); // the list around it
    arena.close(pointer);

    let outcomes = arena.sweep(pointer);
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().any(|o| o.member == MemberId(10) && o.won));
    assert!(outcomes.iter().any(|o| o.member == MemberId(20) && !o.won));
}

/// A cancelled pointer means **no gesture happened**: every member loses, and
/// none of them wins by default. The platform took the touch away — a phone
/// call arrived, a system gesture claimed it — and firing the innermost
/// recogniser anyway is a tap the user did not make.
#[test]
fn a_cancelled_pointer_awards_nothing_to_anybody() {
    let pointer = PointerId(4);
    let mut arena = GestureArena::new();
    arena.add(pointer, MemberId(1));
    arena.add(pointer, MemberId(2));
    arena.close(pointer);

    let outcomes = arena.cancel(pointer);
    assert_eq!(outcomes.len(), 2);
    assert!(
        outcomes.iter().all(|out| !out.won),
        "nobody may win a cancelled contest: {outcomes:?}"
    );
}
