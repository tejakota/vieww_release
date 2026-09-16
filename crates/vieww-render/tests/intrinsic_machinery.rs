//! The intrinsic pass's own machinery, at the level it lives on.
//!
//! ```console
//! cargo test -p vieww-render --test intrinsic_machinery
//! ```
//!
//! `crates/vieww/tests/intrinsic_sizing.rs` asserts what a *widget* tree gets
//! out of `RenderObject::intrinsic`: that a row's cells agree on a height, that
//! an unmeasurable subtree is transparent rather than collapsed. Everything it
//! covers goes through a widget, and a widget cannot reach most of what
//! `RenderTree::intrinsic` guards against.
//!
//! This file drives the render tree by hand and pins the guards themselves:
//!
//! 1. **Reentrancy.** `RenderTree::intrinsic` takes the object out of its slot
//!    before running it, so a node that is asked *while it is answering* finds
//!    an empty slot and gets `None`. That is `NEXT.md` item 11 — deliberate,
//!    and until now untested because nothing a widget tree can build produces a
//!    cycle. The branch is what stands between a future reparenting or
//!    developer-written render object and an unbounded recursion, so the answer
//!    is pinned here before something reaches it. Same reasoning, and the same
//!    shape, as `a_provision_under_a_moved_chain_is_a_different_object` in
//!    `crates/vieww-element/tests/inherited_ancestry.rs`.
//! 2. **The clamp.** A render object that returns a negative or non-finite
//!    number is a bug, and the tree converts it to `0.0` so that the bug is a
//!    layout that looks wrong rather than a `NaN` that propagates into every
//!    size above it.
//! 3. **The cache.** Answered questions are remembered, the memory is dropped
//!    when it is invalidated, and the keying treats `-0.0` and `0.0` as the one
//!    question they are.
//!
//! The cap on the cache and the key function are unit-tested inside
//! `src/intrinsics.rs` instead, because both `CACHE_CAP` and
//! `IntrinsicQuery::key` are private and asserting on a copy of the number
//! would pin nothing.
//!
//! ## Reaching a node's own id from inside `intrinsic`
//!
//! `IntrinsicCtx` deliberately exposes children and nothing else — no `id()`,
//! no parent, no way to name yourself. So the probes below are handed their
//! target's `RenderId` through an `Rc<Cell<_>>` filled in after insertion, and
//! call `IntrinsicCtx::child_intrinsic` with it. That method does not verify
//! that the id it is given is actually a child, which is what makes the test
//! possible without adding a public API purely so that a test can exist.

use std::cell::Cell;
use std::rc::Rc;

use vieww_foundation::{Constraints, Size};
use vieww_render::{IntrinsicCtx, IntrinsicQuery, LayoutCtx, RenderId, RenderObject, RenderTree};

// ------------------------------------------------------------------- probes

/// A leaf that answers a declared number and counts how many times it is asked.
///
/// The counter is what turns "the cache works" into a measurement rather than a
/// claim, the same way `crates/vieww/tests/intrinsic_sizing.rs` does it: a
/// cache that quietly stopped working would still produce the right sizes, and
/// only the count would notice.
#[derive(Debug)]
struct Leaf {
    answer: Option<f32>,
    asked: Rc<Cell<u32>>,
}

impl Leaf {
    fn new(answer: f32) -> (Box<Self>, Rc<Cell<u32>>) {
        let asked = Rc::new(Cell::new(0));
        let object = Box::new(Self {
            answer: Some(answer),
            asked: Rc::clone(&asked),
        });
        (object, asked)
    }
}

impl RenderObject for Leaf {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.smallest()
    }

    fn intrinsic(&self, _ctx: &mut IntrinsicCtx<'_>, _query: IntrinsicQuery) -> Option<f32> {
        self.asked.set(self.asked.get() + 1);
        self.answer
    }

    fn debug_name(&self) -> &'static str {
        "Leaf"
    }
}

/// Asks whatever id it has been pointed at, and falls back when that id cannot
/// answer.
///
/// Pointing it at *itself* is the direct reentrant case; pointing two of them
/// at each other is the cycle. The `fallback` is what makes the difference
/// between "the cycle was refused" and "the cycle was refused *and the refusal
/// was not remembered*" observable — see
/// `a_reentrant_refusal_is_not_remembered_as_this_nodes_answer`.
#[derive(Debug)]
struct Asker {
    target: Rc<Cell<Option<RenderId>>>,
    fallback: Option<f32>,
    asked: Rc<Cell<u32>>,
}

/// What `Asker::new` hands back: the object to insert, the cell to point it at
/// once its target has an id, and its call counter.
///
/// The id has to be filled in afterwards because a node cannot know its own id
/// — or its sibling's — until the tree has assigned one.
type Probe = (Box<Asker>, Rc<Cell<Option<RenderId>>>, Rc<Cell<u32>>);

impl Asker {
    fn new(fallback: Option<f32>) -> Probe {
        let target = Rc::new(Cell::new(None));
        let asked = Rc::new(Cell::new(0));
        let object = Box::new(Self {
            target: Rc::clone(&target),
            fallback,
            asked: Rc::clone(&asked),
        });
        (object, target, asked)
    }
}

impl RenderObject for Asker {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.smallest()
    }

    fn intrinsic(&self, ctx: &mut IntrinsicCtx<'_>, query: IntrinsicQuery) -> Option<f32> {
        self.asked.set(self.asked.get() + 1);
        let target = self.target.get().expect("probe was never pointed anywhere");
        ctx.child_intrinsic(target, query).or(self.fallback)
    }

    fn debug_name(&self) -> &'static str {
        "Asker"
    }
}

// -------------------------------------------------------------- reentrancy

/// **`NEXT.md` item 11.** A render object that asks the tree about *itself*
/// while it is answering gets `None`, and the query returns.
///
/// The whole intrinsic pass is recursive and unmemoised on the way down: a
/// container answers by asking its children. Nothing checks depth, so the only
/// thing standing between a cycle and a blown stack is that
/// `RenderTree::intrinsic` takes the object out of its slot before running it —
/// a node asked again finds the slot empty and stops. Without that branch this
/// test does not fail, it aborts the process.
///
/// `None` rather than a panic is the deliberate half: an intrinsic query is
/// advisory, every caller of it already has a "cannot measure" path (see the
/// module docs on `Option`, not zero), and a framework that kills the app
/// because a custom render object measured itself is worse than one that lays
/// out the way it did before the feature existed.
#[test]
fn a_reentrant_query_answers_nothing_rather_than_recurring_forever() {
    let mut tree = RenderTree::new();
    let (probe, target, asked) = Asker::new(None);
    let id = tree.insert(None, probe);
    target.set(Some(id));

    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width()),
        None,
        "a node that asks itself cannot be measured, and says so"
    );
    assert_eq!(
        asked.get(),
        1,
        "and it ran exactly once: the reentrant ask was refused by the tree \
         before the object was entered a second time, which is what bounds the \
         recursion"
    );
}

/// The same guard through a genuine two-node cycle: A asks B, B asks A.
///
/// Worth its own test because the direct case could be caught by a cheaper
/// check — comparing the id against the node being asked — and that check would
/// do nothing here. What actually terminates the walk is the empty slot, which
/// is a property of the whole in-flight chain rather than of one hop, and only
/// a cycle longer than one link distinguishes the two.
#[test]
fn a_cycle_between_two_render_objects_terminates_with_nothing() {
    let mut tree = RenderTree::new();
    let (first, first_target, first_asked) = Asker::new(None);
    let (second, second_target, second_asked) = Asker::new(None);

    let a = tree.insert(None, first);
    let b = tree.insert(Some(a), second);
    first_target.set(Some(b));
    second_target.set(Some(a));

    assert_eq!(
        tree.intrinsic(a, IntrinsicQuery::max_height()),
        None,
        "A cannot answer because B cannot, and B cannot because A is already \
         answering — the cycle resolves to `None` instead of unwinding the stack"
    );
    assert_eq!(
        (first_asked.get(), second_asked.get()),
        (1, 1),
        "each object was entered once; a second entry anywhere would mean the \
         chain was going around again"
    );
}

/// The refusal handed to a reentrant asker is **not** written into the cache of
/// the node that refused it.
///
/// This is the subtle half of the branch and the one that would survive a
/// careless refactor. `RenderTree::intrinsic` returns from the reentrancy arm
/// *before* it touches the cache; if instead it cached the `None` on the way
/// out, that entry would be the first match for the key and the node's real
/// answer — computed moments later by the outer call — would be shadowed by it
/// for the rest of the layout pass. A cycle anywhere in a subtree would then
/// silently zero out a node that is perfectly measurable.
///
/// Here B falls back to 7 when A refuses, so A's real answer is 7. Asking A
/// again has to still say 7.
#[test]
fn a_reentrant_refusal_is_not_remembered_as_this_nodes_answer() {
    let mut tree = RenderTree::new();
    let (first, first_target, _) = Asker::new(None);
    let (second, second_target, _) = Asker::new(Some(7.0));

    let a = tree.insert(None, first);
    let b = tree.insert(Some(a), second);
    first_target.set(Some(b));
    second_target.set(Some(a));

    let query = IntrinsicQuery::max_width();
    assert_eq!(
        tree.intrinsic(a, query),
        Some(7.0),
        "B's fallback survives the cycle and becomes A's answer"
    );
    assert_eq!(
        tree.intrinsic(a, query),
        Some(7.0),
        "and asking A again still gets 7 — a cached `None` from the reentrant \
         arm would shadow the real answer here and report an unmeasurable node"
    );
}

// ------------------------------------------------------------ the NaN cross

/// A `NaN` cross extent is refused before the object is ever consulted.
///
/// A degenerate constraint — a subtraction of two infinities, a division by a
/// zero extent — produces `NaN`, and `NaN != NaN`, so an answer computed at one
/// `NaN` is not an answer to the next question that happens to carry one. The
/// key function returns `None` for it so nothing is looked up and nothing is
/// stored; the query then short-circuits, which is why the object is not run at
/// all.
#[test]
fn a_nan_cross_extent_answers_nothing_without_running_the_object() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width().across(f32::NAN)),
        None,
        "there is no meaningful answer to \"how wide at a height of NaN\""
    );
    assert_eq!(
        asked.get(),
        0,
        "and the object was never asked: the question is rejected at the key, \
         above the object"
    );
}

/// A `NaN` question does not poison the node for the next real one.
///
/// The failure this guards is not the `NaN` query itself — that one is expected
/// to be useless — it is the node afterwards. If the `NaN` were keyed to
/// anything at all (a sentinel, `to_bits`, `0`), its `None` would sit in the
/// cache and the *next* legitimate query on the same node would read it back
/// and report an unmeasurable subtree. One degenerate constraint would silently
/// disable intrinsics for that node for the rest of the pass.
#[test]
fn a_nan_query_does_not_poison_a_later_valid_query_on_the_same_node() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width().across(f32::NAN)),
        None
    );
    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width().across(50.0)),
        Some(30.0),
        "the real question afterwards is answered normally"
    );
    assert_eq!(
        asked.get(),
        1,
        "asked exactly once — for the real question, and never for the NaN"
    );
}

// -------------------------------------------------------------- the clamp

/// A negative answer becomes zero.
///
/// A size is not a signed quantity, and a render object that arrives at one by
/// subtracting padding from a child that turned out smaller than the padding
/// has a bug. Letting the number through produces a parent laid out smaller
/// than its own child — content drawn outside the box that is supposed to
/// contain it, with the symptom appearing wherever that box happens to be
/// painted rather than where the arithmetic went wrong. Clamped in the tree so
/// that every implementation does not have to remember.
#[test]
fn a_negative_answer_is_clamped_to_zero() {
    let mut tree = RenderTree::new();
    let (leaf, _) = Leaf::new(-5.0);
    let id = tree.insert(None, leaf);

    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width()),
        Some(0.0),
        "-5 is not a width; the smallest thing it can honestly mean is 0"
    );
}

/// An ordinary answer is passed through untouched.
///
/// The control for the two clamp tests around it. A clamp implemented as
/// "return zero when in doubt" would pass both of them and destroy every real
/// measurement in the framework, so the pass-through needs asserting next to
/// them rather than assumed from the sizing tests in another crate.
#[test]
fn a_finite_positive_answer_survives_the_clamp_unchanged() {
    let mut tree = RenderTree::new();
    let (leaf, _) = Leaf::new(42.5);
    let id = tree.insert(None, leaf);

    assert_eq!(tree.intrinsic(id, IntrinsicQuery::max_width()), Some(42.5));
}

/// Every non-finite answer becomes zero — both infinities and `NaN`.
///
/// Each of the three is a different bug and they fail differently if they get
/// out. `INFINITY` is the one a render object reaches by reporting the
/// unbounded constraint it was handed instead of measuring; it propagates into
/// a parent's sum and produces an infinite box. `NaN` is worse, because it
/// survives every comparison as `false` — `max`, `min`, and the constraint
/// clamps all quietly do the wrong thing with it, and it spreads through
/// arithmetic to every size computed above the node. Neither may leave this
/// function.
///
/// Zero rather than `None` is a deliberate choice worth stating: the object
/// claimed it could answer, and the tree corrects the number rather than
/// silently converting a broken implementation into an unmeasurable one, which
/// would be the harder bug to find.
#[test]
fn a_non_finite_answer_is_clamped_to_zero() {
    for broken in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
        let mut tree = RenderTree::new();
        let (leaf, _) = Leaf::new(broken);
        let id = tree.insert(None, leaf);

        assert_eq!(
            tree.intrinsic(id, IntrinsicQuery::max_width()),
            Some(0.0),
            "{broken} is not a size and must not reach a parent's arithmetic"
        );
    }
}

// -------------------------------------------------------------- the cache

/// The same question twice runs the object once.
///
/// **Counted, not asserted in prose.** This is the property the whole cache
/// exists for: a container answers by asking every child, so a node reachable
/// by k paths from the node being queried is asked k times without memoisation,
/// and in a tree of containers that is exponential in depth. Sizes would still
/// come out right with the cache broken — only the count notices, which is why
/// this test counts.
#[test]
fn a_repeated_question_is_answered_from_the_cache() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    let query = IntrinsicQuery::max_width().across(50.0);
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(asked.get(), 1, "asked once, answered three times");
}

/// A *different* question is not answered from the first one's entry.
///
/// The other half of the cache being correct, and the one a too-eager
/// implementation gets wrong: a cache keyed on the node alone, or on the axis
/// alone, would hand a min query the max answer. Both numbers below come from
/// the same object, so only the keying distinguishes them.
#[test]
fn a_different_question_is_not_answered_from_another_ones_entry() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    let query = IntrinsicQuery::max_width();
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(tree.intrinsic(id, IntrinsicQuery::min_width()), Some(30.0));
    assert_eq!(tree.intrinsic(id, IntrinsicQuery::max_height()), Some(30.0));
    assert_eq!(tree.intrinsic(id, query.across(50.0)), Some(30.0));
    assert_eq!(
        asked.get(),
        4,
        "four distinct questions, four runs — a cache that matched any of these \
         against another would be returning an answer to a question nobody asked"
    );
}

/// `-0.0` and `0.0` are the same available extent, so they are one entry.
///
/// A cross extent of `-0.0` is what a subtraction of two equal numbers produces
/// on the way down — a padding taking its insets off a zero-width slot — and it
/// means exactly what `0.0` means. `to_bits` disagrees with itself here, so the
/// key normalises the two together; without that, a subtree alternating between
/// the two spellings would miss the cache on every query and pay the full walk
/// each time, which is the cost the cache exists to remove.
#[test]
fn a_negative_zero_cross_extent_is_the_same_question_as_a_positive_zero() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width().across(0.0)),
        Some(30.0)
    );
    assert_eq!(
        tree.intrinsic(id, IntrinsicQuery::max_width().across(-0.0)),
        Some(30.0)
    );
    assert_eq!(
        asked.get(),
        1,
        "one question asked two ways; two runs here would mean the cache is \
         keyed on a bit pattern rather than on the extent it stands for"
    );
}

/// A refusal is remembered too.
///
/// `None` is a legitimate answer — the default `RenderObject::intrinsic`
/// returns it, so most of a real tree does — and it costs exactly as much to
/// recompute as a number does, because reaching it means walking the whole
/// subtree to find the one node that could not answer. Caching only `Some`
/// would leave the cache doing nothing at all for precisely the trees it is
/// most needed on.
#[test]
fn a_refusal_is_cached_like_any_other_answer() {
    let mut tree = RenderTree::new();
    let asked = Rc::new(Cell::new(0));
    let id = tree.insert(
        None,
        Box::new(Leaf {
            answer: None,
            asked: Rc::clone(&asked),
        }),
    );

    let query = IntrinsicQuery::max_width();
    assert_eq!(tree.intrinsic(id, query), None);
    assert_eq!(tree.intrinsic(id, query), None);
    assert_eq!(
        asked.get(),
        1,
        "the walk that found no answer is not repeated"
    );
}

/// A removed node answers nothing instead of panicking.
///
/// The liveness check is the first line of `RenderTree::intrinsic` and it is
/// not decoration: the id would otherwise reach `node()`, which asserts on the
/// generation and panics. Stale ids are ordinary during a sync — a parent
/// holding a child list across the frame in which the child was removed is the
/// normal shape — and an advisory query is not a reason to bring the frame
/// down.
#[test]
fn a_removed_node_answers_nothing() {
    let mut tree = RenderTree::new();
    let (parent, _) = Leaf::new(10.0);
    let (child, asked) = Leaf::new(30.0);
    let root = tree.insert(None, parent);
    let id = tree.insert(Some(root), child);

    tree.remove(id);

    assert_eq!(tree.intrinsic(id, IntrinsicQuery::max_width()), None);
    assert_eq!(asked.get(), 0, "there is nothing left to ask");
}

// ------------------------------------------------------------ invalidation

/// Clearing the cache makes the next question recompute.
///
/// An intrinsic answer is a pure function of the subtree's configuration, so
/// the thing that invalidates it is the thing that already invalidates layout.
/// `clear_intrinsics` is the tree-wide form, for a change the tree cannot see —
/// fonts above all, since a different face changes every text measurement at
/// once. A clear that did not actually drop entries would leave the whole tree
/// sized against a font it is no longer drawing.
#[test]
fn clearing_the_intrinsic_cache_makes_the_next_question_recompute() {
    let mut tree = RenderTree::new();
    let (leaf, asked) = Leaf::new(30.0);
    let id = tree.insert(None, leaf);

    let query = IntrinsicQuery::max_width();
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(asked.get(), 1);

    tree.clear_intrinsics();

    assert_eq!(tree.intrinsic(id, query), Some(30.0));
    assert_eq!(
        asked.get(),
        2,
        "the answer was dropped and recomputed; still 1 here would mean a \
         font change leaves every cached measurement in place"
    );
}

/// Marking a node for layout drops the cached answers on it and above it.
///
/// The path that actually runs in a frame: `mark_needs_layout` is what a render
/// object calls when its configuration changes, and an answer computed from the
/// old configuration is wrong the moment it does. The sweep goes *past* the
/// relayout boundary on purpose — an ancestor's size may be pinned by tight
/// constraints while its intrinsic answer still depends on what changed below
/// it — so the parent here has to be invalidated as well as the child.
#[test]
fn marking_a_node_for_layout_drops_the_cached_answers_above_it() {
    let mut tree = RenderTree::new();
    let (parent, parent_target, parent_asked) = Asker::new(None);
    let (child, child_asked) = Leaf::new(30.0);

    let root = tree.insert(None, parent);
    let id = tree.insert(Some(root), child);
    parent_target.set(Some(id));

    let query = IntrinsicQuery::max_width();
    assert_eq!(tree.intrinsic(root, query), Some(30.0));
    assert_eq!(tree.intrinsic(root, query), Some(30.0));
    assert_eq!((parent_asked.get(), child_asked.get()), (1, 1));

    tree.mark_needs_layout(id);

    assert_eq!(tree.intrinsic(root, query), Some(30.0));
    assert_eq!(
        (parent_asked.get(), child_asked.get()),
        (2, 2),
        "both recomputed: the child because its own configuration changed, the \
         parent because its answer was derived from the child's"
    );
}
