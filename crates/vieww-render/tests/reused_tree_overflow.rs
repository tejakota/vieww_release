//! Mounting unrelated screens into one tree reports the same overflows as
//! mounting each into a tree of its own.
//!
//! ```console
//! cargo test -p vieww-render --test reused_tree_overflow
//! ```
//!
//! # The claim this answers
//!
//! A long-running session that swapped between unrelated screens reported
//! layout overflows that the same screens did not report when each was built
//! fresh, and the reuse of the previous frame's tree was named as the cause.
//! If that is true it is a serious defect: `FrameDriver::set_root` reconciles
//! rather than rebuilding precisely so that a screen change is cheap, and a
//! reconcile that leaves geometry behind would make every application's
//! navigation report overflows for screens that fit.
//!
//! So the two are run side by side: one driver mounting every screen in turn,
//! and a fresh driver per screen, both at the same surface, with
//! `overflow::reported()` read per screen. Anything the reused tree reports
//! that the fresh one does not is the defect; the cycle is run twice so a
//! screen also gets mounted over *itself* and over a screen it has already
//! seen.

use vieww_foundation::{Color, EdgeInsets, Size};
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;

const SURFACE: Size = Size::new(400.0, 700.0);

/// A column of cards, comfortably inside the surface.
fn cards() -> WidgetNode {
    let mut children: Vec<WidgetNode> = Vec::new();
    for index in 0..4 {
        children.push(
            Container::new()
                .color(Color::rgba(240, 240, 245, 255))
                .padding(EdgeInsets::all(12.0))
                .child(Text::new(format!("Card {index}")))
                .into(),
        );
    }
    Flex::column().spacing(8.0).children(children).into()
}

/// A row of fixed-width tiles that fits exactly, which is where a stale
/// constraint would show first.
fn tiles() -> WidgetNode {
    let mut children: Vec<WidgetNode> = Vec::new();
    for _ in 0..4 {
        children.push(
            SizedBox::from_size(Size::new(90.0, 60.0))
                .child(Container::new().color(Color::rgba(200, 220, 255, 255)))
                .into(),
        );
    }
    Flex::row().spacing(8.0).children(children).into()
}

/// A scrollable list far longer than the surface — the case that must *not*
/// report, however often it is mounted.
fn list() -> WidgetNode {
    Scrollable::vertical(0.0)
        .viewport(SURFACE.height)
        .child(ListView::new(
            60,
            40.0,
            std::rc::Rc::new(|index| Text::new(format!("Row {index}")).into()),
        ))
        .into()
}

/// Text in a padded container, a different shape of tree again.
fn prose() -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::all(24.0))
        .child(Text::new(
            "The quick brown fox jumps over the lazy dog, and keeps going for \
             long enough to wrap across several lines of a narrow column.",
        ))
        .into()
}

type Screen = (&'static str, fn() -> WidgetNode);

const SCREENS: &[Screen] = &[
    ("cards", cards),
    ("tiles", tiles),
    ("list", list),
    ("prose", prose),
];

/// Mount `build` into `driver`, draw, and return what overflowed.
fn overflows_of(driver: &mut FrameDriver, build: fn() -> WidgetNode) -> usize {
    vieww_render::overflow::forget_reported();
    driver.set_root(build());
    driver.draw_frame();
    vieww_render::overflow::reported()
}

#[test]
fn a_reused_tree_reports_what_a_fresh_one_reports() {
    let mut reused = FrameDriver::new(SURFACE);

    for pass in 0..2 {
        for (name, build) in SCREENS {
            let mut fresh = FrameDriver::new(SURFACE);
            let expected = overflows_of(&mut fresh, *build);
            let actual = overflows_of(&mut reused, *build);
            assert_eq!(
                actual, expected,
                "pass {pass}, screen `{name}`: mounted into a tree that had \
                 already drawn other screens it reported {actual} overflow(s); \
                 mounted into a tree of its own, {expected}. A reconcile is \
                 keeping geometry from the screen before."
            );
        }
    }
}

/// The same, with a resize between screens — the state most likely to be
/// stale, and what a real session does when a window is dragged.
#[test]
fn a_reused_tree_survives_a_resize_between_screens() {
    let mut reused = FrameDriver::new(SURFACE);
    let sizes = [SURFACE, Size::new(320.0, 480.0), Size::new(900.0, 700.0)];

    for size in sizes {
        for (name, build) in SCREENS {
            let mut fresh = FrameDriver::new(size);
            let expected = overflows_of(&mut fresh, *build);

            reused.resize(size);
            let actual = overflows_of(&mut reused, *build);

            assert_eq!(
                actual, expected,
                "at {size:?}, screen `{name}`: {actual} overflow(s) in the \
                 reused tree against {expected} in a fresh one."
            );
        }
    }
}
