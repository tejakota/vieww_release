//! A virtualised list keeps building the rows the window is actually over.
//!
//! ```console
//! cargo test -p vieww --test virtualised_scrolling
//! ```
//!
//! # Why this file exists
//!
//! Reported as "`examples/data`'s feed goes blank when scrolled hard", and
//! carried on the open list for two sessions as *possibly* the scroll
//! dead-travel defect. It was neither that nor that example.
//!
//! `ListView` takes its window from the [`ScrollMetrics`] a `Scrollable`
//! publishes, and `ScrollMetrics::viewport` was whatever the caller passed to
//! `Scrollable::viewport` — a value **nothing measured**, despite the field's own
//! doc saying it was measured during layout. No call site in this repository
//! passed one. So `visible()` was `None` for every scrollable in the framework,
//! every `ListView` inside one took its "nothing has measured a window yet"
//! fallback of twelve rows, and the fallback is indistinguishable from working
//! until you scroll: rows `0..14` are built forever, the viewport translates them
//! by the offset, and at a few hundred points they leave the window. The list is
//! then blank while still reporting its full height and its full row count.
//!
//! Every test that covered virtualisation declared a viewport and inflated the
//! tree without laying it out, so all of them agreed with each other and none of
//! them could see it. **These run a real frame and declare nothing**, which is
//! what an application does.
//!
//! `Scrollable` now reads its window from the constraints layout hands it, one
//! level above the `Viewport` — the viewport gives its child an unbounded main
//! axis on purpose, so measuring inside it reads infinity.

use std::rc::Rc;

use vieww::foundation::{Constraints, Offset, Size};
use vieww::prelude::*;
use vieww::BuildContext;
use vieww_element::Signal;
use vieww_render::FrameDriver;
use vieww_widget::{widget_node_from, ListView, Scrollable};

/// The window the list is seen through, and the surface around it.
const WINDOW: Size = Size::new(320.0, 150.0);
const SURFACE: Size = Size::new(400.0, 300.0);
/// How tall each row is.
const ROW: f32 = 32.0;
/// Enough rows that a full build would be obvious and a wrong window fatal.
const ROWS: usize = 5_000;

/// A feed in a fixed-size box, exactly as `examples/data` builds one — and, the
/// point of the file, **without declaring a viewport**.
#[derive(Debug)]
struct Feed {
    offset: Signal<f32>,
}

impl Widget for Feed {
    fn debug_name(&self) -> &'static str {
        "Feed"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Constrained::new(Constraints::tight(WINDOW))
            .child(Scrollable::vertical(self.offset.get()).child(ListView::new(
                ROWS,
                ROW,
                Rc::new(|row: usize| Text::new(format!("item {row}")).into()),
            )))
            .into()
    }
}

widget_node_from!(Feed);

/// A driver showing the feed, and the signal that scrolls it.
fn feed() -> (FrameDriver, Signal<f32>) {
    let mut driver = FrameDriver::new(SURFACE);
    let offset = driver.elements().runtime().signal(0.0_f32);
    driver.set_root(Feed {
        offset: offset.clone(),
    });
    driver.draw_frame();
    (driver, offset)
}

/// Where every row's text landed, in the surface's own pixels.
fn row_positions(driver: &FrameDriver) -> Vec<Offset> {
    driver
        .scene()
        .glyph_runs()
        .into_iter()
        .map(|(offset, _)| offset)
        .collect()
}

/// How many rows land inside the window rather than off the top of it.
fn rows_in_view(driver: &FrameDriver) -> usize {
    row_positions(driver)
        .iter()
        .filter(|offset| offset.dy >= -ROW && offset.dy <= WINDOW.height)
        .count()
}

#[test]
fn a_scrollable_publishes_the_window_it_was_measured_at_without_being_told() {
    let (driver, _offset) = feed();
    assert!(
        rows_in_view(&driver) > 0,
        "nothing on screen on the very first frame"
    );
}

#[test]
fn the_rows_on_screen_are_still_on_screen_after_scrolling_a_long_way() {
    let (mut driver, offset) = feed();

    // Well past the fourteen rows the old fallback built, and past the point
    // where the report came from.
    for far in [300.0, 1_000.0, 5_000.0, 50_000.0] {
        offset.set(far);
        driver.draw_frame();
        assert!(
            rows_in_view(&driver) > 0,
            "the feed went blank at offset {far} — the list is building rows the \
             window is no longer over. Rows landed at {:?}",
            row_positions(&driver)
        );
    }
}

#[test]
fn scrolling_does_not_start_building_the_whole_list() {
    // The other half: a window that tracks the offset must not do it by
    // building everything above it. Both halves in one file, because a fix for
    // either one alone passes the other's test.
    let (mut driver, offset) = feed();
    offset.set(50_000.0);
    driver.draw_frame();

    let built = row_positions(&driver).len();
    assert!(
        built <= 16,
        "five thousand rows cost {built} built rows — a window of {} at {ROW} a \
         row is about five, plus overscan at both ends",
        WINDOW.height
    );
}

#[test]
fn the_rows_built_are_the_rows_the_offset_names() {
    // Position alone would pass for a list that built row 0 and placed it
    // correctly by accident, so this pins *which* rows arrive: at offset 1600
    // the window starts at row 50, and row 50's text must be the one at the top
    // of the window rather than fifty rows above it.
    let (mut driver, offset) = feed();
    offset.set(1_600.0);
    driver.draw_frame();

    let top = row_positions(&driver)
        .into_iter()
        .filter(|offset| offset.dy >= -ROW)
        .map(|offset| offset.dy)
        .fold(f32::INFINITY, f32::min);

    assert!(
        top.is_finite() && top > -ROW && top < ROW,
        "the first row in the window landed at {top}, which is not within a row \
         of the top of it — the list built somewhere else and the viewport \
         translated it out of sight"
    );
}
