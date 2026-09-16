//! Carrying a screen's state across a **remount**, which is what a live-reload
//! editor does on every save.
//!
//! # The case this is about
//!
//! `hot_reload_identity.rs` answers the neighbouring question — whether an
//! element can keep its state when the widget describing it is recompiled *and
//! declares the same reload identity*. That is the reload crate's path, and it
//! needs the guest to opt in.
//!
//! A studio previewing a screen has no such luxury. It compiles the buffer to a
//! `cdylib`, `dlopen`s it and mounts whatever comes back; the widgets are
//! ordinary types from a library that did not exist a second ago, they declare
//! nothing, and reconciliation correctly refuses to match a single one of them.
//! The whole subtree is torn down. Everything the developer had done to the
//! screen — the page they were on, the colour they had picked, how far they had
//! scrolled — went with it, and `viewwstudio`'s preview caption had to say so.
//!
//! `ElementTree::snapshot_states` and `restore_states` are the answer: take the
//! state off the old tree keyed by *where it sits*, and put it back into the new
//! one at the same place.
//!
//! # What is faked here, and what is not
//!
//! Only the recompile. `Before` and `After` below are two genuinely different
//! Rust types with different `TypeId`s — which is exactly what a rebuilt guest
//! library produces — and everything downstream is the real element tree.

use std::any::Any;

use vieww_element::ElementTree;
use vieww_widget::{BuildContext, ElementState, Flex, SizedBox, Widget, WidgetKind, WidgetNode};

/// A counter, of the kind a previewed screen has three of.
#[derive(Debug, Default)]
struct Count {
    value: u32,
}

impl ElementState for Count {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn snapshot(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn restore(&mut self, saved: &str) -> bool {
        match saved.parse() {
            Ok(value) => {
                self.value = value;
                true
            }
            // The contract: text this state cannot make sense of leaves it
            // alone. A screen edited between the snapshot and the restore can
            // hand a widget a value written by a different version of itself.
            Err(_) => false,
        }
    }
}

/// The widget before the edit.
#[derive(Debug)]
struct Before;

/// And after it — same name, same place, different type.
#[derive(Debug)]
struct After;

macro_rules! counter {
    ($name:ident) => {
        impl Widget for $name {
            fn debug_name(&self) -> &'static str {
                "Counter"
            }
            fn kind(&self) -> WidgetKind<'_> {
                WidgetKind::Composed
            }
            fn create_state(&self) -> Option<Box<dyn ElementState>> {
                Some(Box::new(Count::default()))
            }
            fn build(&self, _ctx: &BuildContext) -> WidgetNode {
                SizedBox::shrink().into()
            }
        }
        impl From<$name> for WidgetNode {
            fn from(widget: $name) -> Self {
                Self::new(widget)
            }
        }
    };
}

counter!(Before);
counter!(After);

/// A screen with two counters side by side, so the test can tell them apart.
fn screen(first: WidgetNode, second: WidgetNode) -> WidgetNode {
    Flex::column().push(first).push(second).into()
}

/// Set the `n`th counter's value, the way a tap handler would.
fn set(tree: &ElementTree, index: usize, value: u32) {
    let counters = tree.find_all("Counter");
    let state = counters[index].state().expect("a counter has state");
    state
        .borrow_mut()
        .as_any_mut()
        .downcast_mut::<Count>()
        .expect("a Count")
        .value = value;
}

fn read(tree: &ElementTree, index: usize) -> u32 {
    let counters = tree.find_all("Counter");
    counters[index]
        .state_as::<Count, _>(|count| count.value)
        .expect("a Count")
}

#[test]
fn state_survives_a_remount_onto_a_completely_different_type() {
    let mut tree = ElementTree::new();
    let root = tree.mount(screen(Before.into(), Before.into()));
    tree.rebuild_pending();

    set(&tree, 0, 7);
    set(&tree, 1, 42);

    let snapshot = tree.snapshot_states(root);
    assert_eq!(snapshot.len(), 2, "one entry per counter: {snapshot:?}");

    // The recompile. `After` shares nothing with `Before` but its shape and its
    // name, which is precisely what a rebuilt guest library shares.
    let root = tree.set_root(screen(After.into(), After.into()));
    tree.rebuild_pending();
    assert_eq!(read(&tree, 0), 0, "a fresh mount starts fresh");

    assert_eq!(tree.restore_states(root, &snapshot), 2);
    assert_eq!(read(&tree, 0), 7);
    assert_eq!(read(&tree, 1), 42, "and they do not swap places");
}

#[test]
fn a_restore_marks_the_tree_so_the_value_reaches_the_screen() {
    // A state written outside a build marks nothing on its own. A restore that
    // did not ask for a rebuild would put the value back and leave the previous
    // frame on screen — which looks exactly like the restore not working.
    let mut tree = ElementTree::new();
    let root = tree.mount(screen(Before.into(), Before.into()));
    tree.rebuild_pending();
    set(&tree, 0, 3);
    let snapshot = tree.snapshot_states(root);

    let root = tree.set_root(screen(After.into(), After.into()));
    tree.rebuild_pending();
    assert_eq!(tree.pending_count(), 0, "settled before the restore");

    tree.restore_states(root, &snapshot);
    assert!(
        tree.pending_count() > 0,
        "the restored value has to reach the next frame"
    );
}

#[test]
fn a_widget_that_moved_keeps_its_new_state_rather_than_somebody_elses() {
    // The edit that inserts a row above a counter. Its path changes, so its old
    // state no longer addresses it — and putting that state on whatever is now
    // at the old path would be worse than not restoring at all.
    let mut tree = ElementTree::new();
    let root = tree.mount(screen(Before.into(), Before.into()));
    tree.rebuild_pending();
    set(&tree, 0, 7);
    set(&tree, 1, 42);
    let snapshot = tree.snapshot_states(root);

    // Now there are three, and the two that existed have both shifted down.
    let root = tree.set_root(WidgetNode::from(
        Flex::column()
            .push(SizedBox::shrink())
            .push(After)
            .push(After),
    ));
    tree.rebuild_pending();
    let restored = tree.restore_states(root, &snapshot);

    assert_eq!(
        restored, 0,
        "no counter is where a counter used to be, so nothing is restored"
    );
    assert_eq!(read(&tree, 0), 0);
    assert_eq!(read(&tree, 1), 0);
}

#[test]
fn a_state_that_cannot_read_what_it_is_given_is_left_alone() {
    let mut tree = ElementTree::new();
    let root = tree.mount(screen(Before.into(), Before.into()));
    tree.rebuild_pending();
    set(&tree, 0, 5);

    // A real path — taken from the tree itself, so this tests the *contract*
    // rather than a key that happens not to match — carrying text written by a
    // version of the widget that stored its value differently.
    let snapshot = tree.snapshot_states(root);
    let nonsense: Vec<(String, String)> = snapshot
        .iter()
        .map(|(path, _)| (path.clone(), "{\"value\":9}".to_owned()))
        .collect();
    assert!(
        !nonsense.is_empty(),
        "the path has to exist for this to mean anything"
    );
    assert_eq!(tree.restore_states(root, &nonsense), 0);
    assert_eq!(read(&tree, 0), 5, "and the live value is untouched");
}

#[test]
fn a_state_with_nothing_to_say_is_not_in_the_snapshot() {
    // The default, and the reason a press highlight or a half-finished fade
    // does not survive a render: `snapshot` answers `None` unless a state opts
    // in, so the mechanism costs nothing for every state that should not use it.
    #[derive(Debug)]
    struct Ephemeral;
    impl ElementState for Ephemeral {
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn Any {
            self
        }
    }
    #[derive(Debug)]
    struct Transient;
    impl Widget for Transient {
        fn debug_name(&self) -> &'static str {
            "Transient"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn create_state(&self) -> Option<Box<dyn ElementState>> {
            Some(Box::new(Ephemeral))
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            SizedBox::shrink().into()
        }
    }
    impl From<Transient> for WidgetNode {
        fn from(widget: Transient) -> Self {
            Self::new(widget)
        }
    }

    let mut tree = ElementTree::new();
    let root = tree.mount(screen(Transient.into(), Before.into()));
    tree.rebuild_pending();
    let snapshot = tree.snapshot_states(root);
    assert_eq!(snapshot.len(), 1, "only the counter: {snapshot:?}");
}
