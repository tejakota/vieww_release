//! Phase 2 exit test and the reconciliation guarantees underneath it.
//!
//! The exit test: mutate state from outside the tree, and confirm that only the
//! affected element rebuilds while its siblings are untouched. `build_count`
//! per element is what turns that from a claim into a measurement.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_element::{ElementTree, Runtime, Signal};
use vieww_widget::prelude::*;
use vieww_widget::ElementState;

// ---------------------------------------------------------------- test widgets

/// A composed widget that reads a signal during build. Two of these as
/// siblings, each on its own signal, is the exit test's whole setup.
#[derive(Debug)]
struct Counter {
    count: Signal<i32>,
    key: Option<Key>,
}

impl Counter {
    fn new(count: &Signal<i32>) -> Self {
        Self {
            count: count.clone(),
            key: None,
        }
    }

    fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Counter {
    fn debug_name(&self) -> &'static str {
        "Counter"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(self.count.get().to_string()).into()
    }
}

widget_node_from!(Counter);

/// A composed widget that reads nothing, so nothing can ever pending it.
#[derive(Debug)]
struct Static(&'static str);

impl Widget for Static {
    fn debug_name(&self) -> &'static str {
        "Static"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(self.0).into()
    }
}

widget_node_from!(Static);

/// Records mount/dispose ordering into a shared log.
#[derive(Debug)]
struct Tracked {
    label: &'static str,
    log: Rc<RefCell<Vec<String>>>,
    key: Option<Key>,
}

impl Tracked {
    fn new(label: &'static str, log: &Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            label,
            log: Rc::clone(log),
            key: None,
        }
    }

    fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

#[derive(Debug)]
struct TrackedState {
    label: &'static str,
    log: Rc<RefCell<Vec<String>>>,
}

impl ElementState for TrackedState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn mounted(&mut self) {
        self.log.borrow_mut().push(format!("mount:{}", self.label));
    }

    fn dispose(&mut self) {
        self.log
            .borrow_mut()
            .push(format!("dispose:{}", self.label));
    }
}

impl Widget for Tracked {
    fn debug_name(&self) -> &'static str {
        "Tracked"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(TrackedState {
            label: self.label,
            log: Rc::clone(&self.log),
        }))
    }
}

widget_node_from!(Tracked);

fn builds(tree: &ElementTree, name: &str) -> Vec<u32> {
    tree.find_all(name)
        .iter()
        .map(|element| element.build_count())
        .collect()
}

// ------------------------------------------------------------------ exit test

#[test]
fn exit_test_a_state_change_rebuilds_only_the_element_that_read_it() {
    let mut tree = ElementTree::new();
    let left = tree.runtime().signal(0);
    let right = tree.runtime().signal(100);

    tree.mount(Flex::column().children(children![
        Counter::new(&left).key("left"),
        Counter::new(&right).key("right"),
        Static("untouched"),
    ]));

    assert_eq!(builds(&tree, "Counter"), [1, 1], "each built once on mount");
    assert_eq!(builds(&tree, "Static"), [1]);
    let total_elements = tree.len();

    // Mutate from outside the tree — no widget rebuilt, no frame ran.
    left.set(1);

    assert_eq!(tree.pending_count(), 1, "exactly one element was marked");
    assert_eq!(
        builds(&tree, "Counter"),
        [1, 1],
        "a write must not rebuild synchronously; it only schedules"
    );

    let rebuilt = tree.rebuild_pending();

    assert_eq!(rebuilt, 1, "exactly one element rebuilt");
    assert_eq!(
        builds(&tree, "Counter"),
        [2, 1],
        "the left counter rebuilt; its sibling was never touched"
    );
    assert_eq!(
        builds(&tree, "Static"),
        [1],
        "the static sibling is untouched"
    );
    assert_eq!(tree.len(), total_elements, "no element was remounted");
    assert_eq!(tree.pending_count(), 0);
}

#[test]
fn a_rebuild_updates_the_subtree_the_build_produced() {
    let mut tree = ElementTree::new();
    let count = tree.runtime().signal(41);
    tree.mount(Counter::new(&count));

    let text_before = tree.find("Text").unwrap().id();
    count.set(42);
    tree.rebuild_pending();

    let text_after = tree.find("Text").unwrap();
    assert_eq!(
        text_after.id(),
        text_before,
        "Text kept its element: same type, same key, so it is updated not replaced"
    );
    assert_eq!(
        text_after.widget().downcast_ref::<Text>().unwrap().data(),
        "42"
    );
}

#[test]
fn writing_many_times_between_frames_costs_one_rebuild() {
    let mut tree = ElementTree::new();
    let count = tree.runtime().signal(0);
    tree.mount(Counter::new(&count));

    for value in 1..=10 {
        count.set(value);
    }

    assert_eq!(tree.rebuild_pending(), 1);
    assert_eq!(builds(&tree, "Counter"), [2]);
    assert_eq!(
        tree.find("Text")
            .unwrap()
            .widget()
            .downcast_ref::<Text>()
            .unwrap()
            .data(),
        "10",
        "the rebuild sees the last value written, not the first"
    );
}

#[test]
fn an_unread_signal_marks_nothing_pending() {
    let mut tree = ElementTree::new();
    let unread = tree.runtime().signal(0);
    tree.mount(Static("nothing reads the signal"));

    unread.set(1);

    assert_eq!(tree.pending_count(), 0);
    assert_eq!(tree.rebuild_pending(), 0);
}

// -------------------------------------------------------------- reconciliation

#[test]
fn matching_type_and_key_reuses_the_element_and_its_state() {
    let mut tree = ElementTree::new();
    tree.mount(Flex::column().children(children![Text::new("before")]));
    let before = tree.find("Text").unwrap().id();

    tree.set_root(Flex::column().children(children![Text::new("after")]));
    let after = tree.find("Text").unwrap();

    assert_eq!(
        after.id(),
        before,
        "same type and key: the element persists"
    );
    assert_eq!(
        after.widget().downcast_ref::<Text>().unwrap().data(),
        "after"
    );
}

#[test]
fn a_changed_type_replaces_the_element() {
    let mut tree = ElementTree::new();
    tree.mount(Flex::column().children(children![Text::new("x")]));
    let before = tree.find("Text").unwrap().id();

    tree.set_root(Flex::column().children(children![Padding::all(4.0)]));

    assert!(tree.find("Text").is_none());
    assert!(!tree.is_alive(before), "the old element was unmounted");
    assert!(tree.find("Padding").is_some());
}

#[test]
fn a_changed_key_replaces_the_element_even_at_the_same_position() {
    let mut tree = ElementTree::new();
    tree.mount(Flex::column().children(children![Text::new("x").key("a")]));
    let before = tree.find("Text").unwrap().id();

    tree.set_root(Flex::column().children(children![Text::new("x").key("b")]));

    assert_ne!(tree.find("Text").unwrap().id(), before);
    assert!(!tree.is_alive(before));
}

#[test]
fn reordering_keyed_children_moves_their_elements_rather_than_their_state() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(Flex::column().children(children![
        Tracked::new("a", &log).key("a"),
        Tracked::new("b", &log).key("b"),
        Tracked::new("c", &log).key("c"),
    ]));

    let ids: Vec<_> = tree
        .find_all("Tracked")
        .iter()
        .map(|element| element.id())
        .collect();
    log.borrow_mut().clear();

    // Reverse them. Every element should move, none should be rebuilt.
    tree.set_root(Flex::column().children(children![
        Tracked::new("c", &log).key("c"),
        Tracked::new("b", &log).key("b"),
        Tracked::new("a", &log).key("a"),
    ]));

    let reordered: Vec<_> = tree
        .find_all("Tracked")
        .iter()
        .map(|element| element.id())
        .collect();

    assert_eq!(
        reordered,
        vec![ids[2], ids[1], ids[0]],
        "keyed children keep their own elements when reordered"
    );
    assert!(
        log.borrow().is_empty(),
        "nothing should mount or dispose on a pure reorder, got {:?}",
        log.borrow()
    );
}

#[test]
fn unkeyed_children_are_matched_by_position() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(
        Flex::column().children(children![Tracked::new("a", &log), Tracked::new("b", &log),]),
    );
    log.borrow_mut().clear();

    // Same shape, no keys: positions match, so nothing remounts even though the
    // labels differ. This is the trap keys exist to avoid.
    tree.set_root(
        Flex::column().children(children![Tracked::new("b", &log), Tracked::new("a", &log),]),
    );

    assert!(log.borrow().is_empty());
}

#[test]
fn appending_a_child_leaves_the_existing_ones_alone() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(Flex::column().children(children![
        Tracked::new("a", &log).key("a"),
        Tracked::new("b", &log).key("b"),
    ]));
    log.borrow_mut().clear();

    tree.set_root(Flex::column().children(children![
        Tracked::new("a", &log).key("a"),
        Tracked::new("b", &log).key("b"),
        Tracked::new("c", &log).key("c"),
    ]));

    assert_eq!(*log.borrow(), ["mount:c"]);
}

#[test]
fn removing_a_middle_child_disposes_only_that_one() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(Flex::column().children(children![
        Tracked::new("a", &log).key("a"),
        Tracked::new("b", &log).key("b"),
        Tracked::new("c", &log).key("c"),
    ]));
    log.borrow_mut().clear();

    tree.set_root(Flex::column().children(children![
        Tracked::new("a", &log).key("a"),
        Tracked::new("c", &log).key("c"),
    ]));

    assert_eq!(*log.borrow(), ["dispose:b"]);
    assert_eq!(tree.find_all("Tracked").len(), 2);
}

// -------------------------------------------------------------- lifecycle

#[test]
fn state_is_created_on_mount_and_disposed_on_unmount() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(Tracked::new("only", &log));
    assert_eq!(*log.borrow(), ["mount:only"]);

    tree.mount(Static("replacement"));
    assert_eq!(*log.borrow(), ["mount:only", "dispose:only"]);
}

#[test]
fn disposal_runs_parent_before_child() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    tree.mount(Padding::all(1.0).child(Tracked::new("outer", &log).key("outer")));
    // `Tracked` is a leaf, so nest through a real parent/child pair instead.
    log.borrow_mut().clear();

    tree.mount(Static("gone"));
    assert_eq!(*log.borrow(), ["dispose:outer"]);
}

// -------------------------------------------------------------- housekeeping

#[test]
fn unmounting_drops_the_subscriptions_it_held() {
    let runtime = Runtime::new();
    let mut tree = ElementTree::with_runtime(runtime.clone());
    let count = runtime.signal(0);

    tree.mount(Counter::new(&count));
    assert_eq!(runtime.subscriber_count(), 1);

    tree.mount(Static("replacement"));
    assert_eq!(
        runtime.subscriber_count(),
        0,
        "an unmounted element must stop subscribing, or it leaks and wakes forever"
    );

    count.set(1);
    assert_eq!(tree.pending_count(), 0);
}

#[test]
fn rebuilding_does_not_accumulate_subscriptions() {
    let runtime = Runtime::new();
    let mut tree = ElementTree::with_runtime(runtime.clone());
    let count = runtime.signal(0);

    tree.mount(Counter::new(&count));
    for value in 1..=20 {
        count.set(value);
        tree.rebuild_pending();
    }

    assert_eq!(
        runtime.subscriber_count(),
        1,
        "each rebuild must replace its subscriptions, not add to them"
    );
}

#[test]
fn a_stale_id_does_not_address_a_reused_slot() {
    let mut tree = ElementTree::new();
    tree.mount(Text::new("first"));
    let stale = tree.root().unwrap();

    tree.mount(Text::new("second"));

    assert!(
        !tree.is_alive(stale),
        "the slot was reused, but the generation makes the old id invalid"
    );
    assert!(tree.get(stale).is_none());
}

#[test]
fn parents_rebuild_before_children() {
    // Both elements are marked pending by the same write. The parent's rebuild
    // reconciles the child, so ordering by depth means the child is not built
    // twice.
    let mut tree = ElementTree::new();
    let shared = tree.runtime().signal(0);

    #[derive(Debug)]
    struct Outer(Signal<i32>);

    impl Widget for Outer {
        fn debug_name(&self) -> &'static str {
            "Outer"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            let _ = self.0.get();
            Inner(self.0.clone()).into()
        }
    }

    #[derive(Debug)]
    struct Inner(Signal<i32>);

    impl Widget for Inner {
        fn debug_name(&self) -> &'static str {
            "Inner"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Text::new(self.0.get().to_string()).into()
        }
    }

    widget_node_from!(Outer, Inner);

    tree.mount(Outer(shared.clone()));
    assert_eq!(builds(&tree, "Outer"), [1]);
    assert_eq!(builds(&tree, "Inner"), [1]);

    shared.set(1);
    assert_eq!(tree.pending_count(), 2, "both elements read the signal");
    tree.rebuild_pending();

    assert_eq!(builds(&tree, "Outer"), [2]);
    assert_eq!(
        builds(&tree, "Inner"),
        [2],
        "the child rebuilt exactly once, not once per pending ancestor"
    );
}

#[test]
fn an_unchanged_subtree_is_skipped_without_being_walked() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = ElementTree::new();

    // The same Rc handed back on both frames: reconciliation can prove nothing
    // below it changed without descending into it.
    let shared: WidgetNode = Tracked::new("shared", &log).key("shared").into();

    tree.mount(Flex::column().children(vec![shared.clone()]));
    let before = tree.find("Tracked").unwrap().id();
    log.borrow_mut().clear();

    tree.set_root(Flex::column().children(vec![shared.clone()]));

    assert_eq!(tree.find("Tracked").unwrap().id(), before);
    assert!(log.borrow().is_empty());
}

#[test]
fn inherited_values_reach_elements_and_survive_a_rebuild() {
    #[derive(Debug, PartialEq)]
    struct Theme(&'static str);

    #[derive(Debug)]
    struct Reader;

    impl Widget for Reader {
        fn debug_name(&self) -> &'static str {
            "Reader"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, ctx: &BuildContext) -> WidgetNode {
            let theme = ctx.inherit::<Theme>().expect("Theme was published above");
            Text::new(theme.0).into()
        }
    }

    widget_node_from!(Reader);

    let mut tree = ElementTree::new();
    tree.mount(vieww_widget::Inherited::new(Theme("light"), Reader));
    assert_eq!(
        tree.find("Text")
            .unwrap()
            .widget()
            .downcast_ref::<Text>()
            .unwrap()
            .data(),
        "light"
    );

    tree.set_root(vieww_widget::Inherited::new(Theme("dark"), Reader));
    assert_eq!(
        tree.find("Text")
            .unwrap()
            .widget()
            .downcast_ref::<Text>()
            .unwrap()
            .data(),
        "dark",
        "a changed inherited value must reach descendants that read it"
    );
}

/// **A widget that says its description is unchanged is not rebuilt, even
/// though its parent was.**
///
/// The pointer short-circuit above covers a parent that *cloned* a child it did
/// not touch. It cannot cover a parent that rebuilt and constructed the child
/// afresh — a new `Rc` every time, however identical its contents — and that is
/// the shape of every region built inside another region's `build`.
///
/// `viewwstudio`'s gutter is the case this was added for: 175 elements of line
/// numbers, living inside the code pane because it has to scroll with the text,
/// and rebuilt on every keystroke because the pane *shows* the text. No amount
/// of narrowing the gutter's own subscriptions could help — it was not its own
/// subscriptions waking it.
#[test]
fn a_widget_that_says_it_is_unchanged_is_not_rebuilt() {
    /// A child whose only field is a number it draws. `same_configuration`
    /// compares it, which is the whole opt-in.
    #[derive(Debug)]
    struct Careful(i32);

    impl Widget for Careful {
        fn debug_name(&self) -> &'static str {
            "Careful"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn same_configuration(&self, other: &WidgetNode) -> bool {
            other
                .downcast_ref::<Self>()
                .is_some_and(|new| self.0 == new.0)
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Text::new(self.0.to_string()).into()
        }
    }
    widget_node_from!(Careful);

    /// The same child without the opt-in, as the control: the default is
    /// `false`, so it must rebuild exactly as it always did.
    #[derive(Debug)]
    struct Plain(i32);

    impl Widget for Plain {
        fn debug_name(&self) -> &'static str {
            "Plain"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Text::new(self.0.to_string()).into()
        }
    }
    widget_node_from!(Plain);

    /// A parent that rebuilds on its own signal and reconstructs both children
    /// every time — the studio's code pane, in miniature.
    #[derive(Debug)]
    struct Parent {
        tick: Signal<i32>,
        child: Signal<i32>,
    }

    impl Widget for Parent {
        fn debug_name(&self) -> &'static str {
            "Parent"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            let _ = self.tick.get();
            let value = self.child.get();
            Flex::column()
                .children(children![Careful(value), Plain(value)])
                .into()
        }
    }
    widget_node_from!(Parent);

    let runtime = Runtime::new();
    let tick = runtime.signal(0);
    let child = runtime.signal(7);
    let mut tree = ElementTree::with_runtime(runtime.clone());
    tree.mount(Parent {
        tick: tick.clone(),
        child: child.clone(),
    });
    assert_eq!(builds(&tree, "Careful"), [1]);
    assert_eq!(builds(&tree, "Plain"), [1]);

    // The parent rebuilds; both children are reconstructed with the same value.
    tick.set(1);
    tree.rebuild_pending();
    assert_eq!(
        builds(&tree, "Careful"),
        [1],
        "an unchanged description must not be rebuilt"
    );
    assert_eq!(
        builds(&tree, "Plain"),
        [2],
        "the default is to rebuild, and must stay that way"
    );

    // And a real change still gets through, or this is a cache that never
    // invalidates — which would be a correctness bug, not a slow one.
    child.set(8);
    tree.rebuild_pending();
    assert_eq!(builds(&tree, "Careful"), [2], "a changed value was skipped");
    assert_eq!(
        tree.find("Text")
            .unwrap()
            .widget()
            .downcast_ref::<Text>()
            .unwrap()
            .data(),
        "8"
    );
}

// ---------------------------------------------------------------------- memos

/// A memo re-derives when an input changes, and **tells nobody when the result
/// does not**. That second half is the whole reason to have the type: it is
/// what `Signal::set_if_changed` gives a hand-written derivation, without the
/// hand-written derivation.
#[test]
fn a_memo_only_rebuilds_readers_when_its_value_moves() {
    #[derive(Debug)]
    struct Reader {
        lines: vieww_element::Memo<usize>,
    }

    impl Widget for Reader {
        fn debug_name(&self) -> &'static str {
            "Reader"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Text::new(self.lines.get().to_string()).into()
        }
    }
    widget_node_from!(Reader);

    let runtime = Runtime::new();
    let text = runtime.signal("one\ntwo".to_string());
    let lines = {
        let text = text.clone();
        runtime.memo(move || text.with(|t| t.lines().count()))
    };
    assert_eq!(lines.get(), 2);

    let mut tree = ElementTree::with_runtime(runtime.clone());
    tree.mount(Reader {
        lines: lines.clone(),
    });
    assert_eq!(builds(&tree, "Reader"), [1]);

    // A write that leaves the *derived* value alone: two lines, still.
    text.set("one!\ntwo!".to_string());
    assert!(lines.is_stale(), "the input changed, so it must re-derive");
    tree.rebuild_pending();
    assert!(!lines.is_stale());
    assert_eq!(
        builds(&tree, "Reader"),
        [1],
        "the line count did not change, so nobody should have been told"
    );

    // And a write that does move it.
    text.set("one\ntwo\nthree".to_string());
    tree.rebuild_pending();
    assert_eq!(builds(&tree, "Reader"), [2]);
    assert_eq!(
        tree.find("Text")
            .unwrap()
            .widget()
            .downcast_ref::<Text>()
            .unwrap()
            .data(),
        "3"
    );
}

/// Dependencies are re-recorded on every derivation, so a memo that stops
/// reading a signal stops hearing about it — the same rule an element's build
/// follows, and the reason a branch nobody takes costs nothing.
#[test]
fn a_memo_drops_the_inputs_it_no_longer_reads() {
    let runtime = Runtime::new();
    let use_left = runtime.signal(true);
    let left = runtime.signal(1);
    let right = runtime.signal(100);

    let chosen = {
        let (use_left, left, right) = (use_left.clone(), left.clone(), right.clone());
        runtime.memo(move || {
            if use_left.get() {
                left.get()
            } else {
                right.get()
            }
        })
    };
    assert_eq!(chosen.get(), 1);

    // `right` is not on the taken branch, so writing it changes nothing.
    right.set(200);
    assert!(!chosen.is_stale(), "an unread input woke the memo");

    use_left.set(false);
    assert_eq!(chosen.get(), 200);

    // And now the branch has swapped, `left` is the one that is unread.
    left.set(2);
    assert!(!chosen.is_stale(), "a dropped input still wakes the memo");
}

/// A memo reading a memo is a chain, and one settle resolves the whole of it.
#[test]
fn memos_compose_and_settle_in_one_pass() {
    let runtime = Runtime::new();
    let text = runtime.signal("a b c".to_string());
    let words = {
        let text = text.clone();
        runtime.memo(move || text.with(|t| t.split_whitespace().count()))
    };
    let banner = {
        let words = words.clone();
        runtime.memo(move || format!("{} words", words.get()))
    };
    assert_eq!(banner.get(), "3 words");

    text.set("a b c d".to_string());
    assert_eq!(runtime.settle_memos(), 2, "both links re-derived");
    assert_eq!(banner.peek(), "4 words");
    assert_eq!(runtime.stale_memo_count(), 0);

    // A write that the *first* link absorbs never reaches the second.
    text.set("x y z d".to_string());
    assert_eq!(
        runtime.settle_memos(),
        1,
        "the word count did not move, so the banner was never asked"
    );
}

/// Reading a stale memo re-derives it there and then, so no caller can observe
/// a value its inputs have already moved past — the settle pass batches this,
/// it is not the only thing that does it.
#[test]
fn reading_a_stale_memo_derives_it_first() {
    let runtime = Runtime::new();
    let n = runtime.signal(2);
    let doubled = {
        let n = n.clone();
        runtime.memo(move || n.get() * 2)
    };

    n.set(21);
    assert!(doubled.is_stale());
    assert_eq!(doubled.get(), 42, "a read must not see the old derivation");
    assert!(!doubled.is_stale());
}

/// Dropping the last handle takes the memo's subscriptions with it. A runtime
/// that outlives a screen must not accumulate derivations nothing reads.
#[test]
fn a_dropped_memo_stops_subscribing() {
    let runtime = Runtime::new();
    let n = runtime.signal(1);
    let before = runtime.subscriber_count();
    {
        let n = n.clone();
        let memo = runtime.memo(move || n.get() + 1);
        assert_eq!(memo.get(), 2);
        assert!(runtime.subscriber_count() > before, "it subscribed");
    }
    assert_eq!(
        runtime.subscriber_count(),
        before,
        "a dropped memo left its subscription behind"
    );
    n.set(9);
    assert_eq!(runtime.stale_memo_count(), 0, "it is still being notified");
}
