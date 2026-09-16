//! The Phase 4 exit test: a widget tree, all the way to real pixels.
//!
//! Requires the `native` feature:
//!
//! ```console
//! cargo test -p vieww --features native --test paint_to_pixels
//! ```
//!
//! # What this can and cannot prove
//!
//! The roadmap's Phase 4 exit criterion asks for an on-screen frame on a real
//! device at 60fps. Half of that is reachable here and half is not: there is no
//! window, no display and no device in a test process. What *is* reachable is
//! everything up to the swapchain — build, layout, paint, translate, rasterise
//! and read the pixels back to check that the colours landed where layout said
//! they would. The on-device half is recorded as pending in `docs/ROADMAP.md`,
//! the same way §8's iOS device testing is.
//!
//! Vieww's own rasterizer needs no graphics adapter, so this runs unconditionally
//! on a CI box with no GPU at all.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Size};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;

const SIDE: u32 = 120;

fn driver() -> FrameDriver {
    FrameDriver::new(Size::square(SIDE as f32))
}

#[test]
fn a_widget_tree_reaches_the_right_pixels() {
    let mut renderer = NativeRenderer::new();

    let mut driver = driver();
    // A grey ground with a blue child inset by 10 on every side. The root is laid
    // out tight to the surface, so the padding is the only grey left visible —
    // the child fills everything inside it, since a `SizedBox` under tight
    // constraints cannot choose to be smaller.
    driver.elements().set_root(
        Container::new()
            .color(Color::hex(0x80_8080))
            .padding(EdgeInsets::all(10.0))
            .child(ColoredBox::new(Color::BLUE).child(SizedBox::square(40.0))),
    );
    driver.draw_frame();

    let (pixels, report) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");

    assert_eq!(report.shapes, 2, "the ground and the child");
    assert_eq!(
        pixels.pixel(SIDE / 2, SIDE / 2),
        Color::rgba(0, 0, 255, 255),
        "the middle is the blue child"
    );
    assert_eq!(
        pixels.pixel(5, 5),
        Color::rgba(128, 128, 128, 255),
        "the 10px padding is the only ground still visible"
    );
    assert_eq!(
        pixels.pixel(SIDE - 5, SIDE - 5),
        Color::rgba(128, 128, 128, 255),
        "padding on the far side too"
    );
}

#[test]
fn layout_decides_where_the_pixels_go() {
    let mut renderer = NativeRenderer::new();

    // Two 30x30 boxes in a row: red then green, so the boundary is at x=30. The
    // row is tight to the 120-tall surface and centres its children on the cross
    // axis, so both sit in the band y=45..75.
    let mut driver = driver();
    driver.elements().set_root(
        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .children(children![
                ColoredBox::new(Color::RED).child(SizedBox::square(30.0)),
                ColoredBox::new(Color::GREEN).child(SizedBox::square(30.0)),
            ]),
    );
    driver.draw_frame();

    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");

    let middle = SIDE / 2;
    assert_eq!(
        pixels.pixel(15, middle),
        Color::rgba(255, 0, 0, 255),
        "the first child, at the start of the main axis"
    );
    assert_eq!(
        pixels.pixel(45, middle),
        Color::rgba(0, 255, 0, 255),
        "the second child, placed after the first by the row rather than by itself"
    );
    assert_eq!(
        pixels.pixel(15, 5),
        Color::rgba(255, 255, 255, 255),
        "above the centred band, so nothing was painted"
    );
}

#[test]
fn a_rebuild_changes_the_pixels() {
    let mut renderer = NativeRenderer::new();

    let mut driver = driver();
    driver
        .elements()
        .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(40.0)));
    driver.draw_frame();

    let (before, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");
    assert_eq!(before.pixel(20, 20), Color::rgba(255, 0, 0, 255));

    driver
        .elements()
        .set_root(ColoredBox::new(Color::GREEN).child(SizedBox::square(40.0)));
    driver.draw_frame();

    let (after, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");
    assert_eq!(
        after.pixel(20, 20),
        Color::rgba(0, 255, 0, 255),
        "the whole pipeline ran again and the pixels followed"
    );
    assert!(
        !driver.damage().is_clean(),
        "and the change was reported as damage"
    );
}

#[test]
fn a_scheduler_produces_one_frame_of_pixels_per_vsync() {
    use std::time::Duration;

    let mut renderer = NativeRenderer::new();

    let mut scheduler = FrameScheduler::sixty_hz();
    let mut driver = driver();
    driver
        .elements()
        .set_root(ColoredBox::new(Color::BLUE).child(SizedBox::square(50.0)));

    // Several changes, one vsync: one frame.
    scheduler.request_frame();
    scheduler.request_frame();
    let stats = scheduler
        .pulse(Duration::from_millis(16), &mut driver, Duration::default)
        .expect("a frame was requested");
    assert_eq!(stats.number, 1);

    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), SIDE, SIDE, Color::WHITE)
        .expect("render");
    assert_eq!(pixels.pixel(25, 25), Color::rgba(0, 0, 255, 255));

    // No further request, so no further frame.
    assert!(scheduler
        .pulse(Duration::from_millis(32), &mut driver, Duration::default)
        .is_none());
    assert_eq!(scheduler.frame_count(), 1);
}
