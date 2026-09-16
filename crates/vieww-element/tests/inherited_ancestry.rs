//! The seam under inherited dependency tracking: what happens to a provider's
//! `Provision` when the scope chain **above** it moves.
//!
//! Written because `NEXT.md` (2026-08-18) named it as the untested edge:
//! *"a `Provision` is dropped and rebuilt when the scope chain above it moves;
//! that is correct and untested at the seam. The case to write is a provider
//! that gains or loses an ancestor provider mid-session."*
//!
//! # What the seam turned out to be
//!
//! The worry was a surviving reader stranded on a discarded provision: when
//! `InheritedScope::parent_is` fails, `ElementTree::child_scope` throws the
//! provision away and mints a fresh one, and every reader registered on the old
//! one is registered on an object nobody will publish into again.
//!
//! **That state is not reachable from a widget tree.** Every way of inserting
//! or removing a provider is a type change in some slot, `WidgetNode::can_update`
//! compares `TypeId`, and a type change unmounts the subtree — so the readers
//! do not survive to be stranded. The guard is real and the branch it guards is
//! currently dead by construction, which is a different and more useful thing to
//! know than "it works".
//!
//! It is one change away from being live: reload-mode identity already compares
//! type *paths* rather than `TypeId`s, and `docs/PRODUCTION-GAPS.md` §5.2 names
//! `Inherited` as the piece of hot reload that needs its own answer. It needs
//! this branch. `a_provision_under_a_moved_chain_is_a_different_object`, at the
//! bottom of this file, exercises it at the scope level so the answer is
//! written down before something reaches it.
//!
//! # How these tests are written
//!
//! Every test asserts **two** things: that the value on screen is right, and
//! that the number of rebuilds it took is right. The first alone would pass for
//! a tree that rebuilt everything — which is exactly the defect the provision
//! exists to prevent, and exactly what a correctness-only test misses. This is
//! `docs/AIMS.md`'s rule: where a claim is about cost, the test has to count.
//!
//! Two things had to be held still for the counts to mean anything, and both
//! are properties of the framework rather than of the tests:
//!
//! - **The child node is one `Rc`, cloned into every frame.** That is the
//!   situation the provision exists for — `Scrollable` republishes
//!   `ScrollMetrics` every frame while handing back the child node it was
//!   given, and it is that node's `ptr_eq` holding which lets the subtree be
//!   skipped. Rebuilding the body each frame would defeat the early-out and
//!   measure nothing.
//! - **`Provision::republish` compares allocation identity, not `PartialEq`.**
//!   A freshly boxed `Theme("dark")` each frame *is* a republish and correctly
//!   marks every `Theme` reader. So a test that wants one value to move holds
//!   the other's `Rc` still.

use std::rc::Rc;

use vieww_element::ElementTree;
use vieww_widget::prelude::*;
use vieww_widget::Inherited;

// ----------------------------------------------------------------- the values

/// An outer, rarely-changing value. Stands in for `Theme`.
#[derive(Debug, PartialEq, Eq)]
struct Theme(&'static str);

/// An inner, frequently-republished value. Stands in for `ScrollMetrics`.
#[derive(Debug, PartialEq)]
struct Metrics(i32);

// ---------------------------------------------------------------- the readers

/// Reads `Metrics` only. The element a republish is supposed to mark.
#[derive(Debug)]
struct MetricsReader;

impl Widget for MetricsReader {
    fn debug_name(&self) -> &'static str {
        "MetricsReader"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let metrics = ctx
            .inherit::<Metrics>()
            .expect("Metrics is published above every use of this widget");
        Text::new(metrics.0.to_string()).into()
    }
}

widget_node_from!(MetricsReader);

/// Reads `Theme` only. Sits *below* the `Metrics` provider, so a republish of
/// `Metrics` must not touch it — that is the whole claim of dependency
/// tracking, and the ancestry churn in these tests is what could break it.
#[derive(Debug)]
struct ThemeReader;

impl Widget for ThemeReader {
    fn debug_name(&self) -> &'static str {
        "ThemeReader"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // `None` is a legitimate state here: half these tests run with no
        // `Theme` provider at all, and "the ancestor went away" has to be
        // observable rather than a panic.
        let theme = ctx.inherit::<Theme>().map_or("none", |theme| theme.0);
        Text::new(theme).into()
    }
}

widget_node_from!(ThemeReader);

/// Reads nothing. Nothing in this file should ever rebuild it after mount, and
/// it is the control that makes every rebuild count below meaningful: a tree
/// that rebuilt indiscriminately would move this number too.
#[derive(Debug)]
struct Inert;

impl Widget for Inert {
    fn debug_name(&self) -> &'static str {
        "Inert"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new("inert").into()
    }
}

widget_node_from!(Inert);

// ------------------------------------------------------------------- the trees

/// The body under test, identical in every arrangement: one reader of each
/// value plus one reader of neither.
///
/// Built **once** and cloned by `Rc` into every frame. That is not a shortcut —
/// it is the situation the provision exists for. `Scrollable` republishes
/// `ScrollMetrics` every frame while handing back the same child node it was
/// given, and it is precisely that node's `ptr_eq` holding which lets the
/// subtree be skipped. A test that rebuilt the body widgets each frame would
/// defeat the early-out itself and measure nothing about provisions.
fn body() -> WidgetNode {
    Flex::column()
        .children(vec![MetricsReader.into(), ThemeReader.into(), Inert.into()])
        .into()
}

/// `Metrics` published with no `Theme` above it.
fn without_theme(body: &WidgetNode, metrics: &Rc<Metrics>) -> WidgetNode {
    Inherited::shared(Rc::clone(metrics), body.clone())
        .key("metrics")
        .into()
}

/// The one `Metrics` allocation a frame publishes.
///
/// Every value here is a fresh `Rc`, because a *changed* value is what a
/// republish means; the tests that want no change reuse one of these.
fn metrics(value: i32) -> Rc<Metrics> {
    Rc::new(Metrics(value))
}

/// `Metrics` published underneath a `Theme` provider.
///
/// The `Metrics` provider carries the same key in both arrangements, so
/// reconciliation reuses the same element and the provision genuinely has to
/// survive — or genuinely has to be replaced — rather than the whole subtree
/// being unmounted and remounted, which would prove nothing.
///
/// `Theme` is passed as an `Rc` rather than a value because
/// `Provision::republish` compares **allocation identity**, not `PartialEq`:
/// handing it a freshly boxed `Theme("dark")` each frame is a republish and
/// correctly marks every `Theme` reader. That is the documented contract and
/// not what these tests are about, so the allocation is held still and only
/// `Metrics` is allowed to move.
fn with_theme(body: &WidgetNode, theme: &Rc<Theme>, metrics: &Rc<Metrics>) -> WidgetNode {
    Inherited::shared(Rc::clone(theme), without_theme(body, metrics))
        .key("theme")
        .into()
}

/// The one `Theme` allocation a test reuses across frames.
fn theme(name: &'static str) -> Rc<Theme> {
    Rc::new(Theme(name))
}

/// One frame: hand the tree a new root, then build whatever that marked.
///
/// Both halves matter. `set_root` reconciles and *marks* readers; the values
/// they read do not reach them until `rebuild_pending` runs. Splitting them is
/// what the real frame does, and folding them together in the tests would hide
/// a provision that marked nobody behind a subtree that rebuilt anyway.
fn frame(tree: &mut ElementTree, root: WidgetNode) {
    tree.set_root(root);
    tree.rebuild_pending();
}

// ------------------------------------------------------------------- reading

fn text_under(tree: &ElementTree, reader: &str) -> String {
    let element = tree
        .find(reader)
        .unwrap_or_else(|| panic!("{reader} is in the tree"));
    let child = *element
        .children()
        .first()
        .unwrap_or_else(|| panic!("{reader} built exactly one child"));
    tree.get(child)
        .unwrap_or_else(|| panic!("{reader}'s child is live"))
        .widget()
        .downcast_ref::<Text>()
        .unwrap_or_else(|| panic!("{reader} builds a Text"))
        .data()
        .to_owned()
}

fn build_count(tree: &ElementTree, name: &str) -> u32 {
    tree.find(name)
        .unwrap_or_else(|| panic!("{name} is in the tree"))
        .build_count()
}

/// Every reader's build count in one shot, so an assertion names all three and
/// a stray rebuild anywhere cannot hide behind an assertion that only looked at
/// the one element the test was about.
fn counts(tree: &ElementTree) -> (u32, u32, u32) {
    (
        build_count(tree, "MetricsReader"),
        build_count(tree, "ThemeReader"),
        build_count(tree, "Inert"),
    )
}

// ------------------------------------------------------------------- the tests

/// The baseline the seam tests are measured against: with the chain still, a
/// republish rebuilds the one element that read the value.
///
/// Asserted first so that a failure *here* is not misread as a failure of the
/// ancestry cases below.
#[test]
fn a_republish_with_a_still_chain_rebuilds_only_the_reader() {
    let body = body();
    let mut tree = ElementTree::new();
    frame(&mut tree, without_theme(&body, &metrics(1)));
    let before = counts(&tree);

    frame(&mut tree, without_theme(&body, &metrics(2)));

    assert_eq!(text_under(&tree, "MetricsReader"), "2");
    assert_eq!(
        counts(&tree),
        (before.0 + 1, before.1, before.2),
        "a republish must reach its reader and nobody else"
    );
}

/// **The provider gains an ancestor provider** — and what that actually costs.
///
/// The result is not what `NEXT.md` assumed, and the difference is the finding:
/// inserting a provider above **replaces** the element in that slot, because
/// `WidgetNode::can_update` compares `TypeId`, and an `Inherited<Theme>` is not
/// an `Inherited<Metrics>`. Everything below it is unmounted and remounted.
///
/// So the provision is not "dropped and rebuilt while its readers live on" —
/// the readers do not live on either. That is a *stronger* guarantee than the
/// one the seam worried about, and it is the reason the seam is safe today.
/// It is also a real cost with nothing else asserting it: every signal, scroll
/// offset and half-finished animation below the insertion point is lost.
/// Pinned here so a future change to `can_update` — reparenting, or the
/// hot-reload identity work that `docs/PRODUCTION-GAPS.md` §5.2 describes,
/// which already names `Inherited` as needing its own answer — cannot quietly
/// move it in either direction without a test going red.
#[test]
fn gaining_an_ancestor_provider_remounts_the_subtree_and_tracking_is_exact_after() {
    let body = body();
    let dark = theme("dark");
    let mut tree = ElementTree::new();
    frame(&mut tree, without_theme(&body, &metrics(1)));
    let before_insert = tree.find("MetricsReader").expect("mounted").id();

    // --- the chain above the Metrics provider changes.
    frame(&mut tree, with_theme(&body, &dark, &metrics(1)));

    assert_ne!(
        tree.find("MetricsReader").expect("still mounted").id(),
        before_insert,
        "inserting a provider above is a type change in that slot, so the \
         subtree is remounted rather than reconciled. If this ever starts \
         passing by identity, the provision seam becomes live and \
         `InheritedScope::parent_is` starts carrying real weight — see the \
         unit test at the bottom of this file"
    );
    assert_eq!(text_under(&tree, "ThemeReader"), "dark");
    assert_eq!(text_under(&tree, "MetricsReader"), "1");

    // --- and now the part that matters: is tracking live on the new tree?
    let before = counts(&tree);
    frame(&mut tree, with_theme(&body, &dark, &metrics(2)));

    assert_eq!(
        text_under(&tree, "MetricsReader"),
        "2",
        "a republish after the chain moved must reach the reader"
    );
    assert_eq!(
        counts(&tree),
        (before.0 + 1, before.1, before.2),
        "and must reach only the reader: registration on the fresh provision \
         has to be exact, not merely present"
    );
}

/// **The provider loses its ancestor provider.** The mirror case, same shape.
///
/// The second failure mode here is the one worth naming: `ThemeReader` was
/// registered on a provision that is now gone from the chain entirely, and it
/// must observe the *absence* rather than the last value it saw.
#[test]
fn losing_an_ancestor_provider_remounts_the_subtree_and_tracking_is_exact_after() {
    let body = body();
    let dark = theme("dark");
    let mut tree = ElementTree::new();
    frame(&mut tree, with_theme(&body, &dark, &metrics(1)));
    assert_eq!(text_under(&tree, "ThemeReader"), "dark");

    // --- the ancestor provider is removed.
    frame(&mut tree, without_theme(&body, &metrics(1)));

    assert_eq!(
        text_under(&tree, "ThemeReader"),
        "none",
        "a reader whose provider was removed must see the absence, not the \
         last value it cached — this is the assertion a stranded registration \
         would fail"
    );

    let before = counts(&tree);
    frame(&mut tree, without_theme(&body, &metrics(2)));

    assert_eq!(text_under(&tree, "MetricsReader"), "2");
    assert_eq!(
        counts(&tree),
        (before.0 + 1, before.1, before.2),
        "tracking must survive losing an ancestor as exactly as it survives \
         gaining one"
    );
}

/// The chain moves back and forth repeatedly, and tracking stays exact
/// throughout.
///
/// A single transition can pass by luck — one stale registration among two is
/// still one correct one. Ten transitions with a counted republish after each
/// cannot: a provision that leaked a registration, or a reader that failed to
/// re-register, diverges within a cycle or two.
#[test]
fn tracking_stays_exact_across_repeated_ancestry_changes() {
    let body = body();
    let dark = theme("dark");
    let mut tree = ElementTree::new();
    frame(&mut tree, without_theme(&body, &metrics(0)));

    let mut value = 0;
    for round in 0..10 {
        let themed = round % 2 == 0;
        let root = |value: i32| {
            if themed {
                with_theme(&body, &dark, &metrics(value))
            } else {
                without_theme(&body, &metrics(value))
            }
        };

        // --- move the chain.
        value += 1;
        frame(&mut tree, root(value));
        assert_eq!(
            text_under(&tree, "MetricsReader"),
            value.to_string(),
            "round {round}: the value must arrive on the frame the chain moved"
        );
        assert_eq!(
            text_under(&tree, "ThemeReader"),
            if themed { "dark" } else { "none" },
            "round {round}: the outer value must track its provider appearing \
             and disappearing"
        );

        // --- then republish with the chain still, and count.
        let before = counts(&tree);
        value += 1;
        frame(&mut tree, root(value));

        assert_eq!(
            text_under(&tree, "MetricsReader"),
            value.to_string(),
            "round {round}: the reader must still be reachable after {round} \
             ancestry changes"
        );
        assert_eq!(
            counts(&tree),
            (before.0 + 1, before.1, before.2),
            "round {round}: exactly one rebuild — a registration leaked by an \
             earlier round would show up here as a second one, and a lost \
             registration as none at all"
        );
    }
}

/// Changing the *outer* value must not rebuild the inner provider's readers.
///
/// The provisions are independent, and this is what makes them worth having:
/// `Theme` is the value that changes once, `Metrics` the one that changes every
/// frame, and neither should be able to drag the other's readers along.
#[test]
fn an_outer_republish_does_not_reach_an_inner_providers_readers() {
    let body = body();
    let mut tree = ElementTree::new();
    let still = metrics(1);
    frame(&mut tree, with_theme(&body, &theme("dark"), &still));
    let before = counts(&tree);

    // Only the outer allocation moves. The inner one is the same `Rc`, so
    // `Provision::republish` short-circuits and the Metrics reader is never
    // marked — which is the claim.
    frame(&mut tree, with_theme(&body, &theme("light"), &still));

    assert_eq!(text_under(&tree, "ThemeReader"), "light");
    assert_eq!(
        counts(&tree),
        (before.0, before.1 + 1, before.2),
        "the Metrics reader sits below the Theme provider but does not read \
         Theme; publishing Theme must not cost it a rebuild"
    );
}

/// The value is republished but is the *same allocation*, so nobody rebuilds.
///
/// `Provision::republish` short-circuits on `Rc::ptr_eq`, and that early-out has
/// to keep working across an ancestry change too — it is the one that makes an
/// idle frame free.
#[test]
fn a_republish_of_the_same_allocation_rebuilds_nobody() {
    let body = body();
    let shared = metrics(7);
    let dark = theme("dark");
    let root = |themed: bool| -> WidgetNode {
        if themed {
            with_theme(&body, &dark, &shared)
        } else {
            without_theme(&body, &shared)
        }
    };

    let mut tree = ElementTree::new();
    frame(&mut tree, root(false));

    // --- move the chain, which necessarily rebuilds the subtree once.
    frame(&mut tree, root(true));
    let before = counts(&tree);

    // --- now republish the identical `Rc` with the chain still.
    frame(&mut tree, root(true));

    assert_eq!(
        counts(&tree),
        before,
        "an unchanged value must cost nothing, including on the frame after \
         the provision was replaced"
    );
}

/// The readers list does not grow without bound as a reader re-registers.
///
/// `Provision::readers` is drained on every republish precisely so a long-lived
/// provider cannot accumulate one entry per frame. Ancestry churn adds a second
/// way for entries to pile up — a fresh provision inherits nothing, but a
/// *surviving* one must still not double-count a reader that reads twice.
#[test]
fn a_reader_that_reads_every_frame_is_registered_once() {
    let body = body();
    let mut tree = ElementTree::new();
    frame(&mut tree, without_theme(&body, &metrics(0)));

    let mut seen = Vec::new();
    for value in 1..=25 {
        frame(&mut tree, without_theme(&body, &metrics(value)));
        seen.push(build_count(&tree, "MetricsReader"));
    }

    let steps: Vec<u32> = seen.windows(2).map(|pair| pair[1] - pair[0]).collect();
    assert!(
        steps.iter().all(|&step| step == 1),
        "every republish must cost the reader exactly one rebuild; got steps \
         {steps:?}, which means registrations are accumulating"
    );
}

// -------------------------------------------------------- the seam, directly

/// What `InheritedScope::parent_is` is actually guarding, exercised at the
/// scope level because the element tree cannot currently reach this state.
///
/// The two tests above establish why: every way of changing a provider's
/// ancestry in a widget tree is a type change in some slot, and a type change
/// remounts. So `child_scope`'s "the chain above moved" branch is, today,
/// **unreachable from any application**. That is worth knowing precisely and
/// worth not deleting:
///
/// - It is one `can_update` change away from being reachable. Reload-mode
///   identity (`node.rs`, `#[cfg(feature = "hot-reload")]`) already compares
///   type *paths* rather than `TypeId`s, and `docs/PRODUCTION-GAPS.md` §5.2
///   names `Inherited` as the part that needs its own answer. It needs this
///   branch.
/// - Any future reparenting that keeps the element alive reaches it directly.
///
/// This test asserts the branch does the right thing when it is reached: a
/// provision minted under a different chain is a different object, and readers
/// registered on the old one are not marked by the new one. That is the
/// behaviour the element tree relies on being *safe* (it re-registers everyone
/// by remounting); it is the behaviour a reparenting implementation would have
/// to compensate for deliberately.
#[test]
fn a_provision_under_a_moved_chain_is_a_different_object() {
    use vieww_widget::{InheritedScope, Provision};

    let outer_a = InheritedScope::new().push(Rc::new(Theme("a")));
    let outer_b = InheritedScope::new().push(Rc::new(Theme("b")));

    let provision = Provision::new(metrics(1));
    let under_a = outer_a.push_provision(Rc::clone(&provision));

    assert!(
        under_a.parent_is(&outer_a),
        "a scope pushed onto `outer_a` reports `outer_a` as its parent"
    );
    assert!(
        !under_a.parent_is(&outer_b),
        "and reports a different chain as not its parent — this false is the \
         whole signal `child_scope` acts on"
    );

    // A reader registers against the provision while it is under `outer_a`.
    let reader = 7_u64;
    assert_eq!(under_a.get_for::<Metrics>(Some(reader)).unwrap().0, 1);

    // The chain moves, so a fresh provision is minted — this is exactly what
    // `child_scope` does when `parent_is` fails.
    let replacement = Provision::new(metrics(2));
    let under_b = outer_b.push_provision(Rc::clone(&replacement));

    assert_eq!(under_b.get::<Metrics>().unwrap().0, 2);
    assert!(
        replacement.republish(metrics(3)).is_empty(),
        "the replacement has no readers yet: the reader registered on the old \
         provision is invisible to it. An element tree that did not remount \
         the subtree here would leave that reader stale for ever"
    );
    assert_eq!(
        provision.republish(metrics(4)),
        vec![reader],
        "and the old provision still holds the registration, pointing at an \
         element nothing will ever publish into again"
    );
}
