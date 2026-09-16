//! Horizontal scroll through nested scrollables, at the framework level.
//!
//! # Why this file exists
//!
//! The report was: the editor's horizontal scrollbar drags fine, but a
//! touchpad swipe — horizontal or vertical — does nothing over the code pane.
//! The framework half of that path (a horizontal `Scrollable` inside a
//! vertical one, fed wheel events through a real driver) is what these pin;
//! the studio half lives in `apps/viewwstudio/tests/scroll_gestures.rs`.

use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::{DragDetails, Offset, ScrollEvent, Size};
use vieww_render::FrameDriver;

use vieww_widget::{Scrollable, SizedBox, Text, WidgetNode};

/// The point the probes scroll at: inside the content in every fixture here.
const AT: Offset = Offset::new(200.0, 50.0);

fn swipe(driver: &mut FrameDriver) {
    driver.handle_scroll(&ScrollEvent::new(
        AT,
        Offset::new(-80.0, 0.0),
        Duration::from_millis(5),
    ));
    driver.draw_frame_at(Duration::from_millis(6));
}

/// A wide, tall block: something to scroll in both readings of the word, so
/// `AT` always lands on content.
fn wide_block() -> WidgetNode {
    WidgetNode::from(SizedBox::from_size(Size::new(1200.0, 200.0)).child(Text::new("wide")))
}

#[test]
fn a_horizontal_scrollable_answers_a_horizontal_wheel() {
    let mut driver = FrameDriver::new(Size::new(400.0, 300.0));

    let calls = Rc::new(std::cell::Cell::new(0));
    let seen = Rc::clone(&calls);

    driver.set_root(WidgetNode::from(
        Scrollable::horizontal(0.0)
            .on_drag(Rc::new(move |_: DragDetails| seen.set(seen.get() + 1)))
            .child(wide_block()),
    ));
    driver.draw_frame();
    swipe(&mut driver);

    assert_eq!(calls.get(), 1, "the handler fired once, for the one swipe");
}

/// The editor's shape: the horizontal scrollable sits inside the vertical one,
/// so it is the innermost detector on every wheel.
#[test]
fn a_horizontal_scrollable_inside_a_vertical_one_answers_a_horizontal_wheel() {
    let mut driver = FrameDriver::new(Size::new(400.0, 300.0));

    let calls = Rc::new(std::cell::Cell::new(0));
    let seen = Rc::clone(&calls);

    let inner = Scrollable::horizontal(0.0)
        .on_drag(Rc::new(move |_: DragDetails| seen.set(seen.get() + 1)))
        .child(wide_block());

    driver.set_root(WidgetNode::from(
        Scrollable::vertical(0.0)
            .on_drag(Rc::new(|_: DragDetails| {}))
            .child(WidgetNode::from(inner)),
    ));
    driver.draw_frame();
    swipe(&mut driver);

    assert_eq!(
        calls.get(),
        1,
        "the inner handler fired; the outer vertical one stayed out of the way"
    );
}
