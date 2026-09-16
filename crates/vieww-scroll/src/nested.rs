//! Coordinating a drag between two nested scroll positions.
//!
//! # The problem
//!
//! A list of comments inside a page, or a collapsing header above a list, is
//! two [`ScrollPosition`]s stacked in the same gesture's path. Handing every
//! drag delta to just the one directly under the finger is wrong in both
//! directions: a comment list that eats the whole gesture never lets the page
//! itself scroll once the list is exhausted, and a page that eats it first
//! never lets the list scroll at all. Something has to decide, per delta, how
//! much goes to which — that something is this module, and deciding it is
//! pure arithmetic over two [`ScrollPosition`]s, no tree required.
//!
//! # Two orders, for two different real layouts
//!
//! - [`NestedScrollOrder::InnerFirst`] — the common case: a sub-list inside a
//!   page (comments under a post, a chat thread in a panel). The inner list
//!   scrolls normally; only once it is already at the edge in the drag's
//!   direction does the *remainder* of the delta reach the outer scroll.
//! - [`NestedScrollOrder::OuterFirst`] — a collapsing header: the outer
//!   position (the header's own collapse progress) absorbs the drag first,
//!   and only once it is fully collapsed does the inner content start
//!   scrolling. This is what makes a list not move at all until its header
//!   has finished shrinking above it.
//!
//! # Where the split is exact, and where it blends
//!
//! The handoff point is "has the position doing the absorbing reached an
//! edge in this delta's direction" — computed as offset before versus after
//! calling [`ScrollPosition::apply_drag`], which resists a boundary rather
//! than reporting one under `Overscroll::Bounce`. Give the position that
//! absorbs *first* in whichever [`NestedScrollOrder`] is chosen
//! `Overscroll::Clamp` physics for a clean, exact handoff at the edge; a
//! `Bounce` position instead blends — some of the delta stretches it and the
//! rest still reaches the sibling in the same gesture, which is a reasonable
//! feel but not the sharp handoff a `Clamp` position gives. Documented rather
//! than hidden, because "which physics on which position" changes the feel
//! measurably and silently.
//!
//! # A note on sign
//!
//! `delta` follows [`ScrollPosition::apply_drag`]'s own convention exactly
//! (this module never flips it): a *negative* delta is what scrolls the
//! offset forward, into the content, because `apply_drag` computes
//! `raw -= delta`. `drag`'s tests below drive both directions so the
//! handoff's symmetry is checked, not just its common case.

use vieww_gestures::ScrollPosition;

/// Which of the two nested positions absorbs a drag delta first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NestedScrollOrder {
    /// The inner content scrolls first; the outer only receives what the
    /// inner could not absorb.
    InnerFirst,
    /// The outer scrolls first (a collapsing header); the inner only
    /// receives what the outer could not absorb.
    OuterFirst,
}

/// Splits one drag delta between an inner and an outer [`ScrollPosition`]
/// according to a [`NestedScrollOrder`].
///
/// Stateless by design — the two positions already hold all the state that
/// matters (their own offsets, their own physics), so this is a policy
/// applied fresh to whatever they currently are rather than something that
/// could itself drift out of sync with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestedScroll {
    order: NestedScrollOrder,
}

impl NestedScroll {
    #[must_use]
    pub const fn new(order: NestedScrollOrder) -> Self {
        Self { order }
    }

    /// Apply one drag `delta` across `inner` and `outer`, absorbing it in
    /// whichever order this coordinator was built with. Always returns
    /// `(applied_to_inner, applied_to_outer)` in that fixed order — not the
    /// absorption order — so a caller never has to remember which config it
    /// built this with just to read the result back correctly. The two
    /// values sum to `delta` exactly, unless *both* positions are already
    /// pinned at an edge in this delta's direction, in which case the
    /// leftover has nowhere to go and neither position moves.
    pub fn drag(
        &self,
        inner: &mut ScrollPosition,
        outer: &mut ScrollPosition,
        delta: f32,
    ) -> (f32, f32) {
        match self.order {
            NestedScrollOrder::InnerFirst => {
                let to_inner = Self::apply_and_measure(inner, delta);
                let to_outer = Self::apply_remainder(outer, delta, to_inner);
                (to_inner, to_outer)
            }
            NestedScrollOrder::OuterFirst => {
                let to_outer = Self::apply_and_measure(outer, delta);
                let to_inner = Self::apply_remainder(inner, delta, to_outer);
                (to_inner, to_outer)
            }
        }
    }

    /// Apply whatever of `delta` the first position in the order left
    /// unconsumed to `position`, and report how much of it that position
    /// took. `0.0` without touching `position` at all when the first
    /// position already absorbed everything — a `ScrollPosition` at rest is
    /// not itself a no-op internally (it still clears any running
    /// animation), so skipping the call is what actually changes for a
    /// caller mid-fling on the second position: `apply_drag` cancels the
    /// fling. Only touching it when there is something left to give
    /// preserves that.
    fn apply_remainder(position: &mut ScrollPosition, delta: f32, already_consumed: f32) -> f32 {
        let remainder = delta - already_consumed;
        if remainder.abs() > f32::EPSILON {
            Self::apply_and_measure(position, remainder)
        } else {
            0.0
        }
    }

    /// Apply `delta` to `position` and report how much of it actually moved
    /// the offset, in the same finger-delta units `delta` is measured in.
    ///
    /// `apply_drag`'s own convention is `raw -= delta` — offset decreases as
    /// `delta` increases — so the finger-delta consumed is `before - after`
    /// rather than `after - before`.
    fn apply_and_measure(position: &mut ScrollPosition, delta: f32) -> f32 {
        let before = position.offset();
        position.apply_drag(delta);
        before - position.offset()
    }
}

#[cfg(test)]
mod tests {
    use vieww_gestures::ScrollPhysics;

    use super::*;

    /// `Clamp` physics on both, so the handoff below is exact rather than
    /// blended — see this module's own doc for why that matters to a test
    /// that wants precise numbers.
    fn position(content: f32) -> ScrollPosition {
        ScrollPosition::new(100.0, content, ScrollPhysics::android())
    }

    #[test]
    fn inner_first_keeps_the_whole_delta_while_inner_has_room() {
        let mut inner = position(500.0); // max_offset = 400
        let mut outer = position(300.0); // max_offset = 200
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -30.0);

        assert_eq!(to_inner, -30.0);
        assert_eq!(to_outer, 0.0);
        assert_eq!(inner.offset(), 30.0);
        assert_eq!(
            outer.offset(),
            0.0,
            "the outer must not move while the inner has room"
        );
    }

    /// The handoff itself: once the inner is pinned at its max, the drag's
    /// remainder — not the whole delta again — reaches the outer.
    #[test]
    fn inner_first_hands_the_remainder_to_the_outer_once_inner_is_pinned() {
        let mut inner = position(150.0); // max_offset = 50
        let mut outer = position(300.0); // max_offset = 200
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        // Scrolls the inner to its very end and nothing reaches the outer.
        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -50.0);
        assert_eq!((to_inner, to_outer), (-50.0, 0.0));
        assert_eq!(inner.offset(), 50.0);
        assert_eq!(outer.offset(), 0.0);

        // A further 20 of drag: the inner is pinned (nothing more to
        // absorb), so all of it reaches the outer.
        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -20.0);
        assert_eq!(to_inner, 0.0, "the inner is already at its edge");
        assert_eq!(to_outer, -20.0);
        assert_eq!(outer.offset(), 20.0);
    }

    /// A single drag that both finishes the inner and overflows into the
    /// outer within the same call — the case a naive "all or nothing" split
    /// gets wrong by dropping the leftover instead of forwarding it.
    #[test]
    fn one_drag_can_finish_the_inner_and_overflow_into_the_outer_in_the_same_call() {
        let mut inner = position(150.0); // max_offset = 50
        let mut outer = position(300.0); // max_offset = 200
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        // 70 of drag: 50 finishes the inner, the remaining 20 must reach the
        // outer in this same call.
        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -70.0);
        assert_eq!(to_inner, -50.0);
        assert_eq!(to_outer, -20.0);
        assert_eq!(inner.offset(), 50.0);
        assert_eq!(outer.offset(), 20.0);
    }

    #[test]
    fn outer_first_reverses_which_position_absorbs_first() {
        let mut inner = position(300.0); // max_offset = 200
        let mut outer = position(150.0); // max_offset = 50
        let coordinator = NestedScroll::new(NestedScrollOrder::OuterFirst);

        // 70 of drag: 50 finishes the (now-first) outer, 20 overflows to the
        // inner — but the return order is always (inner, outer).
        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -70.0);
        assert_eq!(to_outer, -50.0);
        assert_eq!(to_inner, -20.0);
        assert_eq!(outer.offset(), 50.0);
        assert_eq!(inner.offset(), 20.0);
    }

    /// The handoff has to work in both directions: once the inner is pinned
    /// at its *start*, a "scroll back" delta it cannot absorb must still
    /// reach the outer, exactly as it does at the far end.
    #[test]
    fn the_handoff_is_symmetric_in_the_other_direction() {
        let mut inner = position(150.0); // max_offset = 50, starts at 0 (pinned)
        let mut outer = position(300.0); // max_offset = 200
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        // Move the outer partway down first so there is somewhere for a
        // "scroll back" handoff to land.
        outer.apply_drag(-100.0);
        assert_eq!(outer.offset(), 100.0);

        // The inner is already at offset 0 and cannot go lower — a "scroll
        // back" (positive) delta must pass straight through to the outer.
        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, 30.0);
        assert_eq!(to_inner, 0.0);
        assert_eq!(to_outer, 30.0);
        assert_eq!(outer.offset(), 70.0);
    }

    #[test]
    fn a_delta_within_the_inners_range_never_touches_the_outer() {
        let mut inner = position(500.0);
        let mut outer = position(500.0);
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        let (_, to_outer) = coordinator.drag(&mut inner, &mut outer, -10.0);
        assert_eq!(to_outer, 0.0);
        assert_eq!(outer.offset(), 0.0);
    }

    /// Both pinned: the leftover has nowhere to go, and neither position is
    /// disturbed.
    #[test]
    fn both_pinned_at_the_same_edge_absorbs_nothing() {
        let mut inner = position(150.0); // max_offset = 50
        let mut outer = position(150.0); // max_offset = 50
        let coordinator = NestedScroll::new(NestedScrollOrder::InnerFirst);

        coordinator.drag(&mut inner, &mut outer, -50.0);
        coordinator.drag(&mut inner, &mut outer, -50.0); // outer now also pinned
        assert_eq!(inner.offset(), 50.0);
        assert_eq!(outer.offset(), 50.0);

        let (to_inner, to_outer) = coordinator.drag(&mut inner, &mut outer, -20.0);
        assert_eq!((to_inner, to_outer), (0.0, 0.0));
        assert_eq!(inner.offset(), 50.0);
        assert_eq!(outer.offset(), 50.0);
    }
}
