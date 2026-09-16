//! Does a `Transformed` inside a `Stack` actually move its child?
//!
//! Written to settle a specific question rather than to cover a feature. An
//! application built a screen-transition stack as
//! `Stack -> Positioned::fill -> Transformed::translate -> screen` and got
//! every screen painted at the stack's origin, on top of each other, with no
//! travel at all. Either the composition is wrong or the framework is, and
//! reading the render objects did not say which.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Offset, Size};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;

const SIDE: u32 = 120;

/// The control: a transform with no stack around it.
#[test]
fn a_transform_moves_its_child() {
    let mut renderer = NativeRenderer::new();

    let mut driver = FrameDriver::new(Size::square(SIDE as f32));
    driver.elements().set_root(
        Transformed::translate(Offset::new(60.0, 0.0))
            .child(ColoredBox::new(Color::BLUE).child(SizedBox::square(40.0))),
    );
    driver.draw_frame();

    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");

    assert_eq!(
        pixels.pixel(20, 20),
        Color::rgba(255, 255, 255, 255),
        "the child left its original position"
    );
    assert_eq!(
        pixels.pixel(80, 20),
        Color::rgba(0, 0, 255, 255),
        "and arrived 60 to the right"
    );
}

/// The case the application actually built.
#[test]
fn a_transform_inside_a_filled_stack_slot_moves_its_child() {
    let mut renderer = NativeRenderer::new();

    let mut driver = FrameDriver::new(Size::square(SIDE as f32));
    driver
        .elements()
        .set_root(Stack::new().fit(StackFit::Expand).children(children![
            // Left where it is.
            Positioned::fill().child(
                Transformed::translate(Offset::new(0.0, 0.0))
                    .child(ColoredBox::new(Color::RED).child(SizedBox::square(40.0))),
            ),
            // Pushed right by half the surface.
            Positioned::fill().child(
                Transformed::translate(Offset::new(60.0, 0.0))
                    .child(ColoredBox::new(Color::BLUE).child(SizedBox::square(40.0))),
            ),
        ]));
    driver.draw_frame();

    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");

    // If the transform is honoured these are two different colours. If it is
    // not, the blue is painted over the red at the origin and both reads are
    // blue — which is exactly the "two screens in the same place" symptom.
    assert_eq!(
        pixels.pixel(20, 20),
        Color::rgba(255, 0, 0, 255),
        "the untranslated child is still at the origin"
    );
    assert_eq!(
        pixels.pixel(80, 20),
        Color::rgba(0, 0, 255, 255),
        "the translated child moved, rather than painting on top of the other"
    );
}
