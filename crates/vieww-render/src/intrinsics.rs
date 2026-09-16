//! Asking a subtree how big it *wants* to be, before deciding how big it gets.
//!
//! # Why this exists, and what it costs
//!
//! [`RenderObject::layout`](crate::RenderObject::layout) answers "given these
//! constraints, what size do you take". That is enough for almost everything
//! and it is why the framework got this far without an intrinsic pass. It is
//! not enough for the shape where a parent's *own* constraint depends on a
//! child's natural size:
//!
//! - a row whose children should all be as tall as the tallest one, where the
//!   tallest one's height depends on the width it is given;
//! - a column that should be exactly as wide as its widest line of text;
//! - an accordion that animates open to the height its content will occupy, at
//!   a moment when the content is not laid out at all.
//!
//! Every one of those was, until now, pushed onto the caller as a number. The
//! public API said so out loud: `Accordion::content_height` was a **required**
//! parameter and `ListView::variable` needed a caller-supplied estimator. An
//! API that asks the application to measure the framework's own text is an
//! admission that the framework cannot.
//!
//! The cost is real and is why this is opt-in rather than automatic. An
//! intrinsic query walks the subtree **without** laying it out, and a parent
//! that asks two questions of `n` children before laying them out has turned a
//! single-pass O(n) layout into a multi-pass one. The long-standing rule applies
//! unchanged: intrinsics are for the cases above, not for general use, and the
//! per-node cache below is what stops a nested query going exponential.
//!
//! # `Option`, not zero
//!
//! [`RenderObject::intrinsic`](crate::RenderObject::intrinsic) returns
//! `Option<f32>`, and the default is `None` — *"I do not know."* The
//! alternative — a `0.0` default — is a known trap: a custom render
//! object that forgets to override it does not fail, it silently reports that
//! it wants no space, and the bug surfaces as a collapsed row somewhere else
//! entirely.
//!
//! `None` propagates. A container that cannot measure one child cannot answer
//! for itself, so it returns `None` too, and the widget that asked
//! ([`IntrinsicWidth`](vieww_widget::IntrinsicWidth),
//! [`Accordion`](vieww_widget::Accordion)) falls back to the behaviour it had
//! before this module existed. Nothing collapses, nothing lies, and adding an
//! implementation to one more render object strictly improves the answer.

use std::fmt;

use vieww_foundation::Axis;

use crate::{RenderId, RenderObject, RenderTree};

/// Which end of a render object's range is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Extremum {
    /// The smallest extent at which the object can be laid out without its
    /// content being clipped or overflowing.
    ///
    /// For text, the width of the longest unbreakable word. For a row, the sum
    /// of its children's minimums — a row cannot be narrower than its parts.
    Min,
    /// The extent at which the object would stop benefiting from more room.
    ///
    /// For text, the width of the whole string on one line. This is the one
    /// almost every caller wants: "how big would you like to be".
    Max,
}

/// One intrinsic question: an axis, an end of the range, and what is known
/// about the other axis.
///
/// # Why `cross` is an `Option`
///
/// "How wide do you want to be" and "how wide do you want to be if you are 40
/// tall" are different questions with different answers for anything that
/// wraps. Some toolkits pass a sentinel `infinity` for "unconstrained",
/// which then has to be checked for at every arithmetic site. `None` says the
/// same thing and cannot be added to something by accident.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntrinsicQuery {
    /// The axis whose extent is being asked about.
    pub axis: Axis,
    /// Which end of the range.
    pub extremum: Extremum,
    /// The extent available on the *other* axis, if the caller knows it.
    pub cross: Option<f32>,
}

impl IntrinsicQuery {
    /// The widest this object would like to be, given unlimited height.
    #[must_use]
    pub const fn max_width() -> Self {
        Self {
            axis: Axis::Horizontal,
            extremum: Extremum::Max,
            cross: None,
        }
    }

    /// The narrowest this object can be without overflowing.
    #[must_use]
    pub const fn min_width() -> Self {
        Self {
            axis: Axis::Horizontal,
            extremum: Extremum::Min,
            cross: None,
        }
    }

    /// The tallest this object would like to be.
    #[must_use]
    pub const fn max_height() -> Self {
        Self {
            axis: Axis::Vertical,
            extremum: Extremum::Max,
            cross: None,
        }
    }

    /// The shortest this object can be without overflowing.
    #[must_use]
    pub const fn min_height() -> Self {
        Self {
            axis: Axis::Vertical,
            extremum: Extremum::Min,
            cross: None,
        }
    }

    /// The same question, with the other axis pinned to `extent`.
    ///
    /// This is the form that matters for anything that wraps: the height a
    /// paragraph wants is a function of the width it is given, and asking
    /// without saying so gets the single-line answer.
    #[must_use]
    pub const fn across(mut self, extent: f32) -> Self {
        self.cross = Some(extent);
        self
    }

    /// A key that is stable under `f32` bit patterns, for the cache.
    ///
    /// Bit equality rather than `==` on purpose: two queries that differ only
    /// in `-0.0` versus `0.0` are the same question, and `NaN` — which a
    /// degenerate constraint can produce — must not match itself, because a
    /// cached `NaN` answer would be wrong for every later query.
    fn key(self) -> Option<(Axis, Extremum, u32)> {
        let cross = self.cross.unwrap_or(f32::NEG_INFINITY);
        if cross.is_nan() {
            return None;
        }
        // `+0.0` and `-0.0` are the same available extent.
        let bits = if cross == 0.0 { 0 } else { cross.to_bits() };
        Some((self.axis, self.extremum, bits))
    }
}

/// A node's answers, remembered for the duration of one layout pass.
///
/// # Why a `Vec` and not a `HashMap`
///
/// A node is asked at most a handful of distinct questions — in practice one or
/// two — so a linear scan over four entries beats hashing, and the whole
/// structure is one allocation that most nodes never make. It is capped, and a
/// node that somehow exceeds the cap simply stops caching rather than growing:
/// an unbounded per-node cache on a tree of a hundred thousand nodes is a leak
/// wearing an optimisation's name.
#[derive(Default)]
pub(crate) struct IntrinsicCache {
    entries: Vec<((Axis, Extremum, u32), Option<f32>)>,
}

/// Past this many distinct questions for one node, stop remembering.
///
/// Four covers min/max on both axes at one cross extent, which is every query
/// any render object in this crate makes.
const CACHE_CAP: usize = 8;

impl IntrinsicCache {
    fn get(&self, key: (Axis, Extremum, u32)) -> Option<Option<f32>> {
        self.entries
            .iter()
            .find(|(entry, _)| *entry == key)
            .map(|(_, value)| *value)
    }

    fn insert(&mut self, key: (Axis, Extremum, u32), value: Option<f32>) {
        if self.entries.len() < CACHE_CAP {
            self.entries.push((key, value));
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl fmt::Debug for IntrinsicCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntrinsicCache")
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// Handed to [`RenderObject::intrinsic`](crate::RenderObject::intrinsic).
///
/// Deliberately narrower than
/// [`LayoutCtx`](crate::LayoutCtx): it can reach children and fonts, and it
/// cannot lay anything out or place anything. An intrinsic query that laid its
/// children out would leave them sized against constraints their parent never
/// chose, and the corruption would surface a frame later somewhere else.
pub struct IntrinsicCtx<'a> {
    pub(crate) tree: &'a mut RenderTree,
    pub(crate) id: RenderId,
}

impl IntrinsicCtx<'_> {
    /// This object's children, in paint order.
    #[must_use]
    pub fn children(&self) -> &[RenderId] {
        self.tree.children(self.id)
    }

    /// This object's children, copied out — for asking each one a question,
    /// which needs the tree back.
    #[must_use]
    pub fn children_owned(&self) -> Vec<RenderId> {
        self.tree.children(self.id).to_vec()
    }

    /// How many children this object has.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.tree.children(self.id).len()
    }

    /// Ask a child an intrinsic question.
    ///
    /// `None` means the child cannot answer, and a parent that cannot measure
    /// one child almost always cannot answer for itself either — see the module
    /// documentation on why that propagates rather than defaulting to zero.
    pub fn child_intrinsic(&mut self, child: RenderId, query: IntrinsicQuery) -> Option<f32> {
        self.tree.intrinsic(child, query)
    }

    /// Ask the single child, when there is exactly one.
    ///
    /// The shape every proxy render object needs, and worth having as one
    /// method because getting it wrong — measuring the first of several — is
    /// the kind of mistake that only shows up on a subtree nobody screenshotted.
    pub fn only_child_intrinsic(&mut self, query: IntrinsicQuery) -> Option<f32> {
        let children = self.children_owned();
        match children.as_slice() {
            [child] => self.child_intrinsic(*child, query),
            _ => None,
        }
    }

    /// A child's flex factor, if it declared one.
    #[must_use]
    pub fn child_flex(&self, child: RenderId) -> Option<vieww_widget::FlexFactor> {
        self.tree.object(child).and_then(RenderObject::flex)
    }

    /// A child's stack position, if it declared one.
    #[must_use]
    pub fn child_stack_position(&self, child: RenderId) -> Option<vieww_widget::StackPosition> {
        self.tree
            .object(child)
            .and_then(RenderObject::stack_position)
    }

    /// The fonts to measure text against.
    pub fn fonts_mut(&mut self) -> &mut vieww_text::FontStore {
        self.tree.fonts_mut()
    }
}

impl fmt::Debug for IntrinsicCtx<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IntrinsicCtx")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Sum a series of answers, giving up if any one of them is unknown.
///
/// The shape a row needs for its width and a column for its height. Written
/// once because "unknown plus known is unknown" is exactly the rule that gets
/// quietly dropped when each container implements it by hand.
#[must_use]
pub fn sum(answers: impl IntoIterator<Item = Option<f32>>) -> Option<f32> {
    answers
        .into_iter()
        .try_fold(0.0_f32, |total, answer| Some(total + answer?))
}

/// The largest of a series of answers, giving up if any one is unknown.
///
/// The shape a row needs for its *height* and a stack for both axes. Unknown
/// poisons the maximum for the same reason it poisons the sum: an unmeasurable
/// child could be the tallest one, and skipping it reports a box that does not
/// fit its own contents.
#[must_use]
pub fn largest(answers: impl IntoIterator<Item = Option<f32>>) -> Option<f32> {
    let mut iter = answers.into_iter();
    let mut best = iter.next()??;
    for answer in iter {
        best = best.max(answer?);
    }
    Some(best)
}

impl RenderTree {
    /// Ask a node an intrinsic question, using and filling the per-node cache.
    ///
    /// # Why the cache is not optional
    ///
    /// A container answers by asking each child, and a child that is itself a
    /// container asks each of *its* children. A row of columns of rows queried
    /// on both axes is 2^depth walks without memoisation, and the tree that
    /// makes that expensive — deep, wide, text at the leaves — is exactly the
    /// tree an application has. Every serious layout engine caches for the same reason.
    ///
    /// Entries live until [`clear_intrinsics`](Self::clear_intrinsics) drops
    /// them, which `mark_needs_layout` does on the way up: an answer is a pure
    /// function of the subtree's configuration, so the thing that invalidates
    /// it is the thing that already invalidates layout.
    pub fn intrinsic(&mut self, id: RenderId, query: IntrinsicQuery) -> Option<f32> {
        if !self.is_alive(id) {
            return None;
        }
        let key = query.key()?;
        if let Some(cached) = self.intrinsic_cache(id).get(key) {
            return cached;
        }

        // Taken out for the same reason `layout` takes it out: the context
        // below borrows the tree mutably to reach the children.
        let Some(object) = self.take_object(id) else {
            // Already borrowed — a render object asking itself, directly or
            // through a cycle. `None` rather than a panic: an intrinsic query
            // is advisory, and a caller that gets `None` falls back to
            // behaviour that works.
            return None;
        };

        let mut ctx = IntrinsicCtx { tree: self, id };
        let answer = object.intrinsic(&mut ctx, query);
        self.put_object(id, object);

        // A negative answer is a bug in the render object rather than a size,
        // and letting it through produces a parent that is smaller than its
        // own child. Clamped here so every implementation does not have to.
        let answer = answer.map(|value| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        });

        self.intrinsic_cache_mut(id).insert(key, answer);
        answer
    }
}

/// The `intrinsic` implementation for a render object that is transparent to
/// layout: one child, handed this object's constraints unchanged, and this
/// object's size is whatever the child chose.
///
/// Every one of those answers the same way — "ask the child" — and writing it
/// out fifteen times is fifteen chances to ask the *first* of several children
/// or to forget the method exists. `only_child_intrinsic` returns `None` for
/// anything that is not exactly one child, so the macro is also the check.
macro_rules! pass_through_intrinsic {
    () => {
        fn intrinsic(
            &self,
            ctx: &mut $crate::IntrinsicCtx<'_>,
            query: $crate::IntrinsicQuery,
        ) -> Option<f32> {
            ctx.only_child_intrinsic(query)
        }
    };
}

pub(crate) use pass_through_intrinsic;

#[cfg(test)]
mod tests {
    //! The two pieces of this module that are private, and so cannot be
    //! reached from `tests/intrinsic_machinery.rs`: the cache key and the cap.
    //!
    //! Everything else about the intrinsic pass is asserted there, through the
    //! public tree, because that is the level a caller experiences it at. These
    //! two are here for a specific reason: `CACHE_CAP` is a number, and an
    //! integration test that wanted to exceed it would have to write its own
    //! copy of the number — which stops being a test of the cap the first time
    //! somebody changes one of the two copies.

    use std::cell::Cell;
    use std::rc::Rc;

    use vieww_foundation::{Constraints, Size};

    use super::*;
    use crate::LayoutCtx;

    /// Answers the cross extent it was asked at, and counts being asked.
    ///
    /// Answering *the question* rather than a constant is what makes a wrong
    /// cache hit visible: a cache that returned one query's entry for another
    /// query's key would produce a number that does not match the cross extent
    /// it was asked with, and a probe answering a constant would agree with it.
    #[derive(Debug)]
    struct Echo(Rc<Cell<u32>>);

    impl RenderObject for Echo {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            constraints.smallest()
        }

        fn intrinsic(&self, _ctx: &mut IntrinsicCtx<'_>, query: IntrinsicQuery) -> Option<f32> {
            self.0.set(self.0.get() + 1);
            query.cross
        }

        fn debug_name(&self) -> &'static str {
            "Echo"
        }
    }

    /// A `NaN` cross extent has no key at all, so it can neither be looked up
    /// nor stored.
    ///
    /// `NaN != NaN`, so an answer computed at one `NaN` is not an answer to any
    /// later question — including the next one that also carries a `NaN`. Every
    /// other float would be keyed by `to_bits`, which *would* match a later
    /// `NaN` with the same payload and hand back a number derived from a
    /// different degenerate constraint. The `None` here is what stops that
    /// before the cache is ever touched.
    #[test]
    fn a_nan_cross_extent_has_no_cache_key() {
        assert_eq!(IntrinsicQuery::max_width().across(f32::NAN).key(), None);
        assert_eq!(IntrinsicQuery::min_height().across(f32::NAN).key(), None);
    }

    /// `-0.0` and `0.0` are one key.
    ///
    /// They compare equal and mean the same available extent, but their bit
    /// patterns differ, so a key built straight from `to_bits` would split one
    /// question into two entries — filling the cap twice as fast and missing on
    /// every alternation. `-0.0` is not exotic: it is what subtracting equal
    /// insets from an extent produces on the way down a real tree.
    #[test]
    fn a_negative_zero_and_a_positive_zero_cross_extent_key_the_same() {
        assert_eq!(
            IntrinsicQuery::max_width().across(0.0).key(),
            IntrinsicQuery::max_width().across(-0.0).key(),
        );
    }

    /// "I do not know the other axis" is its own question, distinct from
    /// knowing it is zero.
    ///
    /// The two have genuinely different answers for anything that wraps — an
    /// unconstrained paragraph is one line, a paragraph in zero width is one
    /// word per line — so collapsing them would return the single-line width
    /// for a query that asked about a zero-width slot. `None` is encoded as
    /// `-inf` precisely because no caller can pass `-inf` as a real extent.
    #[test]
    fn an_absent_cross_extent_is_a_key_of_its_own() {
        let unknown = IntrinsicQuery::max_width();
        assert_ne!(unknown.key(), unknown.across(0.0).key());
        assert_eq!(
            unknown.key(),
            unknown.across(f32::NEG_INFINITY).key(),
            "documenting the encoding: `-inf` is the sentinel, and it is \
             unreachable as a real extent because the clamp above rejects \
             non-finite answers"
        );
    }

    /// Past the cap the cache stops taking entries rather than growing.
    ///
    /// An unbounded per-node cache is a leak wearing an optimisation's name: it
    /// is one `Vec` per node, on a tree that can be a hundred thousand nodes,
    /// retained until something invalidates layout. Bounded means the worst
    /// case is a node paying for recomputation, which is slow; unbounded means
    /// the worst case is memory, which is fatal.
    #[test]
    fn the_cache_stops_taking_entries_at_the_cap() {
        let mut cache = IntrinsicCache::default();
        assert!(cache.is_empty());

        for index in 0..u32::try_from(CACHE_CAP).unwrap() + 4 {
            cache.insert((Axis::Horizontal, Extremum::Max, index), Some(index as f32));
        }

        assert_eq!(cache.entries.len(), CACHE_CAP, "and no more than that");
        assert_eq!(
            cache.get((Axis::Horizontal, Extremum::Max, 0)),
            Some(Some(0.0)),
            "the entries it did take are still readable"
        );
        assert_eq!(
            cache.get((
                Axis::Horizontal,
                Extremum::Max,
                u32::try_from(CACHE_CAP).unwrap()
            )),
            None,
            "the ones past the cap were dropped, not stored under a wrong key"
        );

        cache.clear();
        assert!(
            cache.is_empty(),
            "and clearing releases the allocation's contents"
        );
    }

    /// **The property that matters about the cap.** A node asked more distinct
    /// questions than it can remember recomputes; it never answers wrongly.
    ///
    /// This is the whole justification for capping rather than growing. Dropping
    /// an entry costs a walk of the subtree, which is the cost the cache was
    /// added to avoid — a slow frame, and one that shows up in a profile. The
    /// unacceptable failure would be a cap implemented by *overwriting* an
    /// arbitrary entry's value, or by evicting a key while leaving its slot
    /// matchable, either of which turns "too many questions" into a subtree laid
    /// out against another question's answer, with nothing to see in a profile
    /// and no obvious place to look.
    ///
    /// Written against `CACHE_CAP` rather than a copy of it, so raising the cap
    /// keeps testing the same property instead of quietly testing nothing.
    #[test]
    fn a_node_past_the_cache_cap_recomputes_rather_than_answering_wrongly() {
        let asked = Rc::new(Cell::new(0));
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(Echo(Rc::clone(&asked))));

        // One more distinct cross extent than the node can remember. Every
        // answer must equal the extent it was asked at, cached or not.
        let extents: Vec<f32> = (1..=CACHE_CAP + 1).map(|step| step as f32).collect();
        for &extent in &extents {
            assert_eq!(
                tree.intrinsic(id, IntrinsicQuery::max_width().across(extent)),
                Some(extent),
            );
        }
        let after_first_pass = asked.get();
        assert_eq!(after_first_pass, u32::try_from(extents.len()).unwrap());

        for &extent in &extents {
            assert_eq!(
                tree.intrinsic(id, IntrinsicQuery::max_width().across(extent)),
                Some(extent),
                "every question still gets its own answer on the second pass — \
                 this is the assertion a wrong eviction breaks",
            );
        }

        assert_eq!(
            asked.get() - after_first_pass,
            1,
            "exactly one recomputation on the second pass: the first \
             `CACHE_CAP` questions were remembered, and the one that did not \
             fit degraded to a fresh walk rather than to a stale entry",
        );
    }
}
