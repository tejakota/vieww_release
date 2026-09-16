//! The hot-reload thesis, tested with no dynamic loading at all.
//!
//! `docs/HOT-RELOAD.md` §4 concluded that nothing else about hot reload can
//! start until one question is answered:
//!
//! > Build a tree, swap the root for widgets of a deliberately different Rust
//! > type with the same identity, and assert that state survives. That test is
//! > the whole thesis, and it needs no `dlopen`.
//!
//! This is that test. If it fails, the architecture does not carry dynamic
//! library reloading either, and the honest answer becomes "vieww has no hot
//! reload story" — reached here for the price of a test file rather than after
//! building a loader.
//!
//! # What it is simulating
//!
//! A rebuilt guest library defines `Counter` again. It is the *same* widget to
//! a person and to the type path, and a **different type** to `Any`, because a
//! [`TypeId`](std::any::TypeId) carries a compilation-session component. Inside
//! one binary two types cannot share a path, so the two versions below say the
//! path explicitly through `WidgetNode::with_reload_id`. That is the only thing
//! faked; everything downstream is the real element tree.

#![cfg(feature = "hot-reload")]

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use vieww_element::{ElementId, ElementTree};
use vieww_foundation::Key;
use vieww_widget::{BuildContext, ElementState, SizedBox, Widget, WidgetKind, WidgetNode};

/// The path both builds of the widget report — what a guest's `Counter` would
/// return from `type_name` before and after an edit.
const COUNTER: &str = "app::screens::Counter";

/// The thing a reload exists to preserve.
///
/// A scroll offset, a half-finished animation and a signal subscription are all
/// this shape: state the element owns, which a teardown destroys and a reload
/// must not.
#[derive(Debug)]
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
}

/// `Counter`, as the first build compiled it.
///
/// The `Option<Key>` is not decoration: reload identity replaces only half of
/// `can_update`, and the key half has to keep working exactly as before.
#[derive(Debug)]
struct CounterV1(Option<Key>);

/// `Counter` after an edit — a genuinely different Rust type, exactly as a
/// rebuilt guest library produces. Nothing about it is contrived except that it
/// has to be spelled with a different name to exist alongside the first.
#[derive(Debug)]
struct CounterV2(Option<Key>);

macro_rules! counter {
    ($name:ty) => {
        impl Widget for $name {
            fn debug_name(&self) -> &'static str {
                "Counter"
            }

            fn key(&self) -> Option<&Key> {
                self.0.as_ref()
            }

            fn kind(&self) -> WidgetKind<'_> {
                WidgetKind::Composed
            }

            fn create_state(&self) -> Option<Box<dyn ElementState>> {
                Some(Box::new(Count { value: 0 }))
            }

            fn build(&self, _ctx: &BuildContext) -> WidgetNode {
                SizedBox::shrink().into()
            }
        }
    };
}

counter!(CounterV1);
counter!(CounterV2);

/// The widget as build *n* produced it.
fn before_edit() -> WidgetNode {
    WidgetNode::with_reload_id(CounterV1(None), COUNTER)
}

/// The same widget as build *n+1* produced it.
fn after_edit() -> WidgetNode {
    WidgetNode::with_reload_id(CounterV2(None), COUNTER)
}

/// Mount a counter and run it forward to `value`.
fn mounted_at(value: u32) -> (ElementTree, ElementId, Rc<RefCell<dyn ElementState>>) {
    let mut tree = ElementTree::new();
    let root = tree.mount(before_edit());
    {
        let state = tree
            .get(root)
            .expect("just mounted")
            .state()
            .expect("a Counter carries state");
        state
            .borrow_mut()
            .as_any_mut()
            .downcast_mut::<Count>()
            .expect("its own state type")
            .value = value;
    }
    let handle = Rc::clone(tree.get(root).unwrap().state().unwrap());
    (tree, root, handle)
}

#[test]
fn element_state_survives_a_recompile() {
    // **The thesis.** Two different Rust types, one path, and the element is
    // reconfigured rather than torn down — so the count it was holding is still
    // there afterwards.
    let (mut tree, root, before) = mounted_at(7);

    let reloaded = tree.set_root(after_edit());

    assert_eq!(
        reloaded, root,
        "the element was reused rather than remounted"
    );
    let after = tree
        .get(root)
        .expect("still alive")
        .state()
        .expect("still stateful");
    assert!(
        Rc::ptr_eq(&before, after),
        "the very same state allocation, not an equal one"
    );
    assert_eq!(
        tree.get(root)
            .unwrap()
            .state_as::<Count, _>(|count| count.value),
        Some(7),
        "and the value it was holding"
    );
}

#[test]
fn the_rule_still_discriminates_rather_than_matching_everything() {
    // Reload identity is *weaker* than `TypeId`, not absent. A widget that is
    // genuinely something else still loses its state — otherwise this would not
    // be reconciliation, it would be a tree that never tears anything down.
    let (mut tree, _root, before) = mounted_at(7);

    let replaced = tree.set_root(WidgetNode::with_reload_id(
        CounterV2(None),
        "app::screens::Other",
    ));

    let after = tree
        .get(replaced)
        .expect("something was mounted")
        .state()
        .expect("the replacement is stateful too");
    assert!(
        !Rc::ptr_eq(&before, after),
        "a different widget must not inherit the old element's state"
    );
    assert_eq!(
        tree.get(replaced)
            .unwrap()
            .state_as::<Count, _>(|count| count.value),
        Some(0),
        "the replacement starts fresh"
    );
}

#[test]
fn a_key_still_separates_two_of_the_same_widget() {
    // The other half of `can_update` is untouched by any of this, and it has to
    // stay that way: a keyed list reorders by key, and a reload that ignored
    // keys would shuffle every row's state into the wrong row.
    let (mut tree, root, before) = mounted_at(7);

    let rekeyed = tree.set_root(WidgetNode::with_reload_id(
        CounterV2(Some(Key::from("second"))),
        COUNTER,
    ));

    assert_ne!(
        rekeyed, root,
        "a key appearing is a different widget, reload or not"
    );
    let after = tree.get(rekeyed).unwrap().state().unwrap();
    assert!(!Rc::ptr_eq(&before, after));
}
