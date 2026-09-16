//! An overlay over a scrollable, all the way to pixels, on the second frame.
//!
//! ```console
//! cargo test -p vieww --features native --test overlay_to_pixels
//! ```
//!
//! # Why this file exists
//!
//! `TRACKER.md` carried this as open item 1 for two sessions: *"the compositor
//! fix is shipped and does not work on a real screen."* `efa6610` gave every
//! child layer a slot so `LayerTree::composite` splices a boundary back where
//! the paint pass left the hole for it, and `layer_paint_order.rs` covers the
//! flattener with ten tests — but the complaint against it ended
//! **"no test in the workspace sees it"**, because every one of those tests
//! reads `driver.scene()` and stops there. The scene was never rasterised.
//!
//! That is the gap, and it is the one that matters: the reported symptom was
//! *nothing drawn at all*, which a correct scene produces perfectly well if a
//! persistent surface skips repainting an unchanged region — a correct scene
//! with empty damage leaves the previous frame's pixels exactly as they were.
//!
//! So this asks the question in pixels, on the **second** frame. A first frame
//! is a full repaint by construction and cannot exercise damage at all, which is
//! why the eight original tests could not have caught this whatever they
//! asserted.
//!
//! # What was actually wrong
//!
//! Nothing, on this tree. Re-measured on 2026-08-17 under Xvfb with lavapipe —
//! the same rig the 2026-08-16 report used — by removing the `RepaintBoundary`
//! workaround from `examples/controls` and opening the menu by hand: the menu
//! draws, the scrim is right, scrolling under it behaves, choosing an item
//! closes it and the page redraws, and the process sits at 2.6% CPU rather than
//! the reported 78%. The workaround is gone from that example.
//!
//! A defect that cannot be reproduced is not a defect that has been fixed, and
//! the honest reading is that it was closed by something between the two dates
//! rather than by anybody deciding to. **This file is the difference**: the
//! claim now has a test rather than a memory of a screen.
//!
//! Vieww's own rasterizer needs no graphics adapter, so this runs
//! unconditionally, driving [`vieww_test_harness::Persistent`] — the same
//! damage-composited-onto-the-previous-frame model a real presented surface
//! uses — the way `paint_to_pixels.rs`'s sibling tests use a fresh render.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Size};
use vieww::prelude::*;
use vieww::BuildContext;
use vieww_element::Signal;
use vieww_render::FrameDriver;
use vieww_test_harness::Persistent;
use vieww_widget::widget_node_from;

/// The scrolled page underneath. A colour no blend of the others produces.
const UNDER: Color = Color::rgb(10, 20, 30);
/// The overlay that has to cover it.
const OVER: Color = Color::rgb(200, 100, 50);

const SURFACE: Size = Size::new(200.0, 200.0);

/// `Persistent::frame`'s pixels are `(r, g, b, a)` tuples; a `Color` reads the
/// same channels for a straightforward comparison.
fn rgba(c: Color) -> (u8, u8, u8, u8) {
    (c.r, c.g, c.b, c.a)
}

/// `examples/controls`, reduced to the two widgets the report is about: a
/// scrollable page — which is a repaint boundary, and therefore a layer — with a
/// **non-boundary** overlay pushed over it as a later sibling.
#[derive(Debug)]
struct Page {
    overlay: Signal<bool>,
}

impl Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let page = Scrollable::vertical(0.0)
            .child(SizedBox::from_size(Size::new(200.0, 400.0)).child(ColoredBox::new(UNDER)));
        let mut stack = Stack::new().children(vieww_widget::children![page]);
        if self.overlay.get() {
            stack = stack.push(SizedBox::from_size(SURFACE).child(ColoredBox::new(OVER)));
        }
        stack.into()
    }
}

widget_node_from!(Page);

#[test]
fn an_overlay_that_appears_on_a_later_frame_reaches_the_screen() {
    let mut driver = FrameDriver::new(SURFACE);
    let overlay = driver.elements().runtime().signal(false);
    driver.set_root(Page {
        overlay: overlay.clone(),
    });

    // Frame one: the page alone. A full repaint by construction — it is here to
    // give the next frame a screen to differ from, which is the whole mechanism
    // the report was about.
    driver.draw_frame();
    let mut persistent = Persistent::new(&driver, SURFACE, Color::BLACK);
    assert_eq!(
        persistent.frame().at(100, 100),
        rgba(UNDER),
        "the page did not reach the screen at all"
    );

    // Frame two: the overlay appears. This is the frame that was reported as
    // drawing nothing.
    overlay.set(true);
    driver.draw_frame();
    assert!(
        !driver.damage().is_clean(),
        "the overlay is in the scene and the frame reported nothing changed — a \
         backend that repaints only what is damaged draws none of it, and one \
         that skips presenting a clean frame leaves the previous frame up. \
         Damage: {}",
        driver.damage()
    );
    persistent.advance(&driver);
    assert_eq!(
        persistent.frame().at(100, 100),
        rgba(OVER),
        "the overlay is above the page in the composited scene but the page is \
         what landed in the pixels — the flattener and the backend disagree, \
         which is exactly what the workaround in `examples/controls` was for"
    );
}

#[test]
fn the_page_comes_back_when_the_overlay_goes_away() {
    // The other direction, and the one a damage bug fails at rather than a paint
    // order bug: removing the overlay has to damage the pixels it was covering,
    // or its colour stays on the screen with nothing in the tree drawing it.
    let mut driver = FrameDriver::new(SURFACE);
    let overlay = driver.elements().runtime().signal(true);
    driver.set_root(Page {
        overlay: overlay.clone(),
    });

    driver.draw_frame();
    let mut persistent = Persistent::new(&driver, SURFACE, Color::BLACK);
    assert_eq!(
        persistent.frame().at(100, 100),
        rgba(OVER),
        "the overlay was up on the first frame"
    );

    overlay.set(false);
    driver.draw_frame();
    persistent.advance(&driver);
    assert_eq!(
        persistent.frame().at(100, 100),
        rgba(UNDER),
        "the overlay was dismissed and its pixels were never repainted — the \
         frame did not damage what it stopped covering"
    );
}
