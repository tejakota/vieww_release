//! Files dragged in from outside, end to end through the tree.
//!
//! ```console
//! cargo test -p vieww --test file_drop
//! ```
//!
//! # What this pins
//!
//! The feature is deliberately **window-level** — `winit` 0.30 reports a dropped
//! path and no position, so a per-widget hit-tested drop target cannot be built
//! honestly on it. `vieww_foundation::file_drop` carries the full argument,
//! including why hit-testing with the last known cursor position was refused.
//!
//! What is testable, and what these assert, is the half that does exist: the
//! drag state reaching a zone through the tree with no application code in
//! between, and a multi-file drop arriving as one delivery.

use vieww::foundation::{DroppedFiles, FileDrag, Size};
use vieww::prelude::*;
use vieww::FrameDriver;

const SURFACE: Size = Size {
    width: 400.0,
    height: 400.0,
};

/// A screen with a drop zone in it, and nothing else.
#[derive(Debug)]
struct Screen;

impl Widget for Screen {
    fn debug_name(&self) -> &'static str {
        "Screen"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::light())
            .child(
                FileDropZone::new()
                    .label("Attach a document")
                    .child(Text::new("Drop a file here")),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Screen);

#[test]
fn a_zone_learns_about_a_hover_with_no_application_code_in_between() {
    // The property that makes this a framework feature rather than a recipe:
    // the platform tells the driver, the driver publishes above the tree, and
    // the zone reads it. Nothing in `Screen` is wired to anything.
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();
    assert_eq!(driver.file_drag(), FileDrag::Idle);

    assert!(
        driver.set_file_drag(FileDrag::Hovering),
        "the first hover is a change"
    );
    driver.draw_frame();
    assert_eq!(driver.file_drag(), FileDrag::Hovering);
}

#[test]
fn a_repeated_hover_is_not_a_change() {
    // The platform sends one hover event *per file*, so repeats are the normal
    // case. Treating each as a change would ask for a frame per file dragged.
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    assert!(driver.set_file_drag(FileDrag::Hovering));
    assert!(
        !driver.set_file_drag(FileDrag::Hovering),
        "a second file hovering is not a second state change"
    );
}

#[test]
fn a_multi_file_drop_arrives_as_one_delivery() {
    // **The gap this closes.** The platform reports one event per file; an
    // application that saw four would upload four times.
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    driver.push_dropped_file("a.png".into());
    driver.push_dropped_file("b.png".into());
    driver.push_dropped_file("c.png".into());

    let dropped = driver.take_dropped_files();
    assert_eq!(dropped.len(), 3);
    assert_eq!(
        dropped.paths()[0]
            .file_name()
            .and_then(|name| name.to_str()),
        Some("a.png"),
        "order is the platform's own"
    );
}

#[test]
fn taking_the_files_leaves_nothing_for_the_next_drop() {
    // A take rather than a read: a drop happens once, and the next drop must not
    // carry the previous one's files.
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    driver.push_dropped_file("first.txt".into());
    assert_eq!(driver.take_dropped_files().len(), 1);
    assert_eq!(
        driver.take_dropped_files(),
        DroppedFiles::default(),
        "the second take saw the first take's files"
    );
}

#[test]
fn the_zone_is_announced_so_it_is_not_only_a_dashed_outline() {
    // Drag-and-drop is the least accessible interaction there is. A zone that
    // exists only as a border is invisible to anybody who cannot see it.
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    let semantics = driver.semantics();
    assert!(
        semantics
            .nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some("Attach a document")),
        "the drop zone does not announce itself: {}",
        semantics.describe()
    );
}
