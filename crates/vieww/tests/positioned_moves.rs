//! A `Positioned` that is rebuilt at a new offset has to move.
//!
//! ```console
//! cargo test -p vieww --test positioned_moves
//! ```
//!
//! Found from an application whose segmented control slid its selection off the
//! right-hand edge of its own track: the widget was rebuilt with the correct
//! geometry — the builder was instrumented and printed the right number — and
//! the pixels stayed where they were.

use vieww::foundation::{Color, Rect, Size};
use vieww::paint::Command;
use vieww::prelude::*;
use vieww::FrameDriver;

const SURFACE: Size = Size {
    width: 400.0,
    height: 200.0,
};

fn tree(left: f32, width: f32) -> WidgetNode {
    Stack::new()
        .children(children![Positioned::new()
            .left(left)
            .top(10.0)
            .width(width)
            .height(20.0)
            .child(Container::new().color(Color::RED))])
        .into()
}

fn red(driver: &FrameDriver) -> Option<Rect> {
    driver
        .scene()
        .commands()
        .iter()
        .find_map(|command| match command {
            Command::FillRect {
                rect,
                paint,
                transform,
                ..
            } if paint.color == Color::RED => Some(transform.apply_rect(*rect)),
            _ => None,
        })
}

#[test]
fn rebuilding_a_positioned_at_a_new_offset_moves_it() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(tree(10.0, 100.0));
    driver.draw_frame();
    assert_eq!(
        red(&driver),
        Some(Rect::new(10.0, 10.0, 110.0, 30.0)),
        "the first frame is the control"
    );

    driver.set_root(tree(200.0, 50.0));
    driver.draw_frame();
    assert_eq!(
        red(&driver),
        Some(Rect::new(200.0, 10.0, 250.0, 30.0)),
        "the widget was rebuilt with a new left and a new width and the pixels \
         did not move"
    );
}

#[test]
fn rebuilding_a_positioned_at_the_same_offset_leaves_it_alone() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(tree(10.0, 100.0));
    driver.draw_frame();
    driver.set_root(tree(10.0, 100.0));
    driver.draw_frame();
    assert_eq!(red(&driver), Some(Rect::new(10.0, 10.0, 110.0, 30.0)));
}

/// The same defect, in the other widget that carries parent data.
///
/// `Flexible`'s factor is read by the `Flex` above it, and `RenderFlexible` is
/// layout-transparent for exactly the same reason `RenderPositioned` is — so
/// changing a factor marked a node that could not act on it and stopped there.
/// Nobody had reported it, which is the point of writing it down: the two
/// widgets share a shape, so they share the bug.
#[test]
fn rebuilding_a_flexible_with_a_new_factor_resizes_it() {
    fn row(first: u16) -> WidgetNode {
        Flex::row()
            .children(children![
                Flexible::expanded(first).child(Container::new().color(Color::RED)),
                Flexible::expanded(1).child(Container::new().color(Color::BLUE)),
            ])
            .into()
    }

    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(row(1));
    driver.draw_frame();
    let even = red(&driver).expect("the red half drew");
    assert!(
        (even.width() - SURFACE.width / 2.0).abs() < 0.5,
        "1:1 is half the row, and this is the control: {even:?}"
    );

    driver.set_root(row(3));
    driver.draw_frame();
    let uneven = red(&driver).expect("the red part drew");
    assert!(
        (uneven.width() - SURFACE.width * 0.75).abs() < 0.5,
        "3:1 is three quarters of the row; the factor changed and the layout \
         did not: {uneven:?}"
    );
}
