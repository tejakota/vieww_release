//! Phase 2 exit test, made watchable.
//!
//! ```text
//! cargo run -p vieww --example counter
//! ```
//!
//! Two counters sit side by side, each reading its own signal, next to a label
//! that reads nothing. Incrementing one of them shows the point of the whole
//! element layer: **one element rebuilds, and the rest of the tree is not
//! touched** — not diffed, not visited, not rebuilt.
//!
//! `builds=` in the dump is the receipt.

use vieww::prelude::*;
use vieww::{widget, BuildContext, WidgetNode};

/// A composed widget that reads a signal during `build`, which is what
/// subscribes its element to that signal.
#[derive(Debug)]
struct Counter {
    label: &'static str,
    count: Signal<i32>,
}

impl Counter {
    fn new(label: &'static str, count: &Signal<i32>) -> Self {
        Self {
            label,
            count: count.clone(),
        }
    }
}

#[widget]
impl Counter {
    fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
        // Reading the signal here is the subscription. Nothing else registers
        // it; there is no dependency array to keep in sync.
        let count = self.count.get();

        Flex::row().children(children![
            Text::new(self.label),
            SizedBox::width(8.0),
            Text::new(count.to_string()).bold(),
        ])
    }
}

/// Reads nothing, so nothing can ever pending it.
#[derive(Debug)]
struct Legend;

#[widget]
impl Legend {
    fn build(&self, _ctx: &BuildContext) -> impl Into<WidgetNode> {
        Text::new("(this label reads no signal)").size(11.0)
    }
}

fn report(tree: &ElementTree, heading: &str) {
    println!(
        "── {heading} {}",
        "─".repeat(56_usize.saturating_sub(heading.len()))
    );
    println!("{}\n", tree.debug_tree());
}

fn main() {
    let mut tree = ElementTree::new();

    // Signals live outside the tree. Widgets hold handles to them, which is how
    // a widget stays immutable while the value it shows does not.
    let left = tree.runtime().signal(0);
    let right = tree.runtime().signal(0);

    tree.mount(
        Container::new()
            .padding(EdgeInsets::all(16.0))
            .child(Flex::column().children(children![
                Counter::new("left ", &left),
                Counter::new("right", &right),
                Legend,
            ])),
    );

    report(&tree, "after mount: everything built once");

    // A write from outside the tree. Note what does *not* happen: no rebuild.
    left.set(1);
    println!(
        "left.set(1)  ->  {} element(s) marked pending, 0 rebuilt so far\n",
        tree.pending_count()
    );

    let rebuilt = tree.rebuild_pending();
    report(
        &tree,
        &format!("after rebuild_pending(): {rebuilt} element(s) rebuilt"),
    );

    // Ten writes, one rebuild — the scheduler collapses them.
    for value in 2..=11 {
        left.set(value);
    }
    let rebuilt = tree.rebuild_pending();
    println!("10 more writes to `left`  ->  {rebuilt} rebuild(s)\n");

    // And the other signal drives its own element, independently.
    right.set(99);
    tree.rebuild_pending();
    report(&tree, "after touching `right` too");

    let counters: Vec<u32> = tree
        .find_all("Counter")
        .iter()
        .map(|element| element.build_count())
        .collect();
    let legend = tree.find("Legend").unwrap().build_count();

    println!("Counter build counts: {counters:?}");
    println!("Legend  build count : {legend}");
    println!(
        "\n{} elements in the tree; the Legend built once and was never revisited.",
        tree.len()
    );
}
