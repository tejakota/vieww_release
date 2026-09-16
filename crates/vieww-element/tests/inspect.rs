//! Finding what rebuilds too much.
//!
//! ```console
//! cargo test -p vieww-element --test inspect
//! ```
//!
//! Phase 10. `debug_tree` already dumps the whole tree with a build count on
//! every node; what it cannot do is answer "what is rebuilding *now*" on a
//! screen with four hundred elements in it. That is what these rank.

use vieww_element::{ElementTree, Signal};
use vieww_widget::prelude::*;

/// Rebuilds whenever its signal is written.
#[derive(Debug, Clone)]
struct Reader {
    label: Signal<String>,
}

impl Reader {
    fn new(label: &Signal<String>) -> Self {
        Self {
            label: label.clone(),
        }
    }
}

impl Widget for Reader {
    fn debug_name(&self) -> &'static str {
        "Reader"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(self.label.get()).into()
    }
}

widget_node_from!(Reader);

/// A tree with two independently-driven readers, already mounted and marked, so
/// each test starts from a clean measurement window.
fn two_readers() -> (ElementTree, Signal<String>, Signal<String>) {
    let mut tree = ElementTree::new();
    let busy = tree.runtime().signal(String::from("0"));
    let quiet = tree.runtime().signal(String::from("0"));

    tree.mount(Flex::column().children(children![
        Reader::new(&busy),
        Reader::new(&quiet),
        Text::new("a leaf that never builds"),
    ]));
    tree.mark_builds();

    (tree, busy, quiet)
}

/// Write `signal` `times` times, rebuilding after each.
fn churn(tree: &mut ElementTree, signal: &Signal<String>, times: u32) {
    for round in 0..times {
        signal.set(round.to_string());
        tree.rebuild_pending();
    }
}

#[test]
fn the_element_that_rebuilds_most_comes_first() {
    let (mut tree, busy, quiet) = two_readers();
    churn(&mut tree, &busy, 5);
    churn(&mut tree, &quiet, 1);

    let hot = tree.hotspots(10);
    assert_eq!(hot.len(), 2, "two readers rebuilt, got {hot:?}");
    assert_eq!(hot[0].recent, 5);
    assert_eq!(hot[1].recent, 1);
}

#[test]
fn an_element_that_never_builds_is_not_in_the_list_at_all() {
    // A render leaf has no `build`, so it is not a rebuild problem and would be
    // pure noise in a ranking of them.
    let (mut tree, busy, _quiet) = two_readers();
    churn(&mut tree, &busy, 3);

    assert!(
        tree.hotspots(10)
            .iter()
            .all(|hotspot| hotspot.widget_name != "Text"),
        "got {:?}",
        tree.hotspots(10)
    );
}

#[test]
fn nothing_rebuilding_is_an_empty_list_rather_than_a_list_of_zeroes() {
    let (tree, _busy, _quiet) = two_readers();
    assert!(tree.hotspots(10).is_empty());
    assert_eq!(tree.builds_since_mark(), 0);
}

#[test]
fn marking_starts_a_new_window_and_forgets_the_old_one() {
    let (mut tree, busy, quiet) = two_readers();
    churn(&mut tree, &busy, 5);
    assert_eq!(tree.hotspots(10)[0].recent, 5);

    // A new second begins. What was busy a moment ago is not the question any
    // more.
    tree.mark_builds();
    churn(&mut tree, &quiet, 2);

    let hot = tree.hotspots(10);
    assert_eq!(hot.len(), 1, "only the reader that rebuilt in this window");
    assert_eq!(hot[0].recent, 2);
}

#[test]
fn the_lifetime_count_survives_a_mark_even_though_the_window_does_not() {
    let (mut tree, busy, _quiet) = two_readers();
    churn(&mut tree, &busy, 4);
    tree.mark_builds();
    churn(&mut tree, &busy, 1);

    let hot = tree.hotspots(10);
    assert_eq!(hot[0].recent, 1, "one rebuild in this window");
    assert_eq!(
        hot[0].builds, 6,
        "but six over its life: one to mount, four, then one more"
    );
}

#[test]
fn the_limit_is_respected() {
    let (mut tree, busy, quiet) = two_readers();
    churn(&mut tree, &busy, 5);
    churn(&mut tree, &quiet, 3);

    assert_eq!(tree.hotspots(1).len(), 1);
    assert_eq!(tree.hotspots(1)[0].recent, 5, "and it is the busiest one");
}

#[test]
fn the_order_is_stable_across_calls() {
    // A ranking that reshuffled between frames would be unreadable in exactly
    // the situation it exists for — watching it while poking at the app.
    let (mut tree, busy, quiet) = two_readers();
    churn(&mut tree, &busy, 3);
    churn(&mut tree, &quiet, 3);

    let first = tree.hotspots(10);
    assert_eq!(first, tree.hotspots(10));
    assert_eq!(first.len(), 2, "and the tie did not drop either of them");
}

#[test]
fn without_any_mark_the_window_is_the_whole_session() {
    // A caller that never marks should still get a sensible answer rather than
    // an empty one, so the ceremony is optional.
    let mut tree = ElementTree::new();
    let label = tree.runtime().signal(String::from("0"));
    tree.mount(Reader::new(&label));
    churn(&mut tree, &label, 2);

    let hot = tree.hotspots(10);
    assert_eq!(hot[0].recent, hot[0].builds);
    assert_eq!(hot[0].builds, 3, "one mount plus two rebuilds");
}

#[test]
fn the_total_is_the_sum_of_what_rebuilt() {
    let (mut tree, busy, quiet) = two_readers();
    churn(&mut tree, &busy, 5);
    churn(&mut tree, &quiet, 2);

    assert_eq!(tree.builds_since_mark(), 7);
}

#[test]
fn a_hotspot_names_the_widget_and_addresses_the_element() {
    let (mut tree, busy, _quiet) = two_readers();
    churn(&mut tree, &busy, 2);

    let hot = tree.hotspots(1);
    assert_eq!(hot[0].widget_name, "Reader");
    assert!(
        tree.is_alive(hot[0].element),
        "the id addresses something you can then go and look at"
    );
}
