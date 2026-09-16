//! Animation through the element tree: state that survives a rebuild, notices a
//! new description, and is advanced by a frame.
//!
//! ```console
//! cargo test -p vieww-element --test animation
//! ```
//!
//! `vieww-animation` tests the motion itself and `vieww-widget` tests the
//! implicit container's state machine in isolation. What is only testable here
//! is the *wiring*: that reconciliation reaches the state at all, that the state
//! outlives the widget that described it, and that a tick marks the right
//! element pending.

use std::time::Duration;

use vieww_element::ElementTree;
use vieww_foundation::Color;
use vieww_widget::prelude::*;
use vieww_widget::AnimatedContainerState;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A container animating linearly, so every assertion is a value rather than a
/// tolerance.
fn swatch(color: Color, width: f32) -> AnimatedContainer {
    AnimatedContainer::new()
        .duration(ms(200))
        .curve(Curve::Linear)
        .color(color)
        .width(width)
        .child(SizedBox::square(10.0))
}

/// What the animated container is showing right now.
fn shown(tree: &ElementTree) -> (Color, f32) {
    tree.find("AnimatedContainer")
        .expect("mounted")
        .state_as(|state: &AnimatedContainerState| {
            let props = state.current();
            (
                props.color.expect("colour is set"),
                props.width.expect("width is set"),
            )
        })
        .expect("an AnimatedContainer's element carries its animation")
}

#[test]
fn mounting_creates_the_state_and_shows_the_first_description_as_given() {
    let mut tree = ElementTree::new();
    tree.mount(swatch(Color::RED, 100.0));

    assert_eq!(tree.stateful_count(), 1);
    assert_eq!(shown(&tree), (Color::RED, 100.0));
    assert!(
        !tree.has_animating_states(),
        "the first description is where it starts, not something to animate to"
    );
}

#[test]
fn a_new_description_animates_over_frames_rather_than_applying() {
    let mut tree = ElementTree::new();
    tree.mount(swatch(Color::RED, 100.0));

    tree.set_root(swatch(Color::BLUE, 200.0));
    assert!(
        tree.has_animating_states(),
        "reconciliation has to reach the state, or an implicit animation is \
         just a slow assignment"
    );
    assert_eq!(
        shown(&tree).1,
        100.0,
        "and nothing has moved yet — no frame has run"
    );

    tree.tick_states(ms(0));
    tree.tick_states(ms(100));
    assert_eq!(shown(&tree).1, 150.0);

    tree.tick_states(ms(200));
    assert_eq!(shown(&tree), (Color::BLUE, 200.0));
    assert!(!tree.has_animating_states());
}

#[test]
fn a_tick_marks_the_animating_element_pending_and_the_rebuild_shows_the_new_value() {
    let mut tree = ElementTree::new();
    tree.mount(swatch(Color::RED, 100.0));
    tree.set_root(swatch(Color::RED, 200.0));
    tree.rebuild_pending();

    let before = tree
        .find("AnimatedContainer")
        .expect("mounted")
        .build_count();

    tree.tick_states(ms(0));
    tree.tick_states(ms(100));
    assert_eq!(
        tree.pending_count(),
        1,
        "a frame of animation marks exactly the element that is animating"
    );

    tree.rebuild_pending();
    let element = tree.find("AnimatedContainer").expect("mounted");
    assert!(element.build_count() > before, "and it rebuilt");

    // The container it built is the one carrying the interpolated width.
    let container = tree.find("Container").expect("built a container");
    let width = container
        .widget()
        .debug_properties()
        .into_iter()
        .find(|(name, _)| *name == "width")
        .map(|(_, value)| value);
    assert_eq!(width.as_deref(), Some("150"));
}

#[test]
fn the_state_survives_the_rebuilds_that_drive_it() {
    let mut tree = ElementTree::new();
    let id = tree.mount(swatch(Color::RED, 0.0));

    for step in 0..5 {
        tree.set_root(swatch(Color::RED, 100.0));
        tree.tick_states(ms(step * 20));
        tree.rebuild_pending();
    }

    assert_eq!(
        tree.root(),
        Some(id),
        "the element must be reused across every one of those, or the animation \
         restarts from scratch each frame and never arrives"
    );
    let width = shown(&tree).1;
    assert!(width > 0.0 && width < 100.0, "part way along: {width}");
}

#[test]
fn unmounting_takes_the_state_with_it() {
    let mut tree = ElementTree::new();
    tree.mount(swatch(Color::RED, 100.0));
    tree.set_root(swatch(Color::BLUE, 200.0));
    assert_eq!(tree.stateful_count(), 1);

    tree.set_root(SizedBox::square(10.0));

    assert_eq!(
        tree.stateful_count(),
        0,
        "a different widget type replaces it"
    );
    assert!(
        !tree.has_animating_states(),
        "an animation on a screen that is gone must not keep the display awake"
    );
    assert!(!tree.tick_states(ms(500)));
}

#[test]
fn a_keyed_container_that_changes_key_starts_over_rather_than_moving() {
    let mut tree = ElementTree::new();
    tree.mount(swatch(Color::RED, 100.0).key("first"));

    tree.set_root(swatch(Color::BLUE, 200.0).key("second"));

    assert_eq!(
        shown(&tree),
        (Color::BLUE, 200.0),
        "a new key is a different thing in the same place, so its animation \
         starts at its own values rather than travelling from the old ones"
    );
    assert!(!tree.has_animating_states());
}

#[test]
fn an_element_with_no_state_costs_nothing_per_frame() {
    let mut tree = ElementTree::new();
    tree.mount(Flex::column().children(children![
        SizedBox::square(10.0),
        ColoredBox::new(Color::RED).child(SizedBox::square(10.0)),
    ]));

    assert_eq!(tree.stateful_count(), 0);
    assert!(
        !tree.tick_states(ms(16)),
        "a tree with nothing animating in it must report no work whatever is \
         mounted"
    );
    assert_eq!(tree.pending_count(), 0);
}
