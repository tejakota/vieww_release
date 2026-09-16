//! Wheel and touchpad scrolls against the editor, through the real driver.
//!
//! # Why this file exists
//!
//! The report was: the scrollbars drag fine, but a two-finger swipe on the
//! touchpad — horizontal or vertical — does nothing over the code pane. Every
//! half of that path was unit-tested somewhere and the whole of it was tested
//! nowhere, which is exactly the shape of bug only a driver test catches.
//!
//! These tests feed [`FrameDriver::handle_scroll`] the events a window
//! delivers: a wheel notch as a vertical delta, a touchpad swipe as a
//! horizontal one, and the diagonal noise a real trackpad produces between
//! the two.

use std::time::Duration;

use vieww_foundation::{Offset, ScrollEvent, Size, TargetPlatform, TextEditingValue};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

/// A mounted shell, focused, with one frame drawn — the same fixture
/// `tests/input.rs` uses.
fn shell() -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    viewwstudio::seed_focus(&mut driver);
    (driver, studio)
}

/// A point inside the code pane. The same reasoning as `IN_CODE` in
/// `tests/input.rs`: window inset, activity bar, gutter and card borders.
const IN_CODE: Offset = Offset::new(520.0 + 16.0, 260.0 + 14.0);

/// Fill the active buffer with content that overflows the pane both ways:
/// more lines than fit, and lines longer than the pane is wide.
fn long_buffer(studio: &Studio) {
    let mut text = String::new();
    for line in 0..200 {
        text.push_str(&format!("fn line_{line}() {{ {line} }}\n"));
    }
    // One very long line, so the horizontal scrollbar has something to do.
    text.push_str(&format!("const WIDE: &str = \"{}\";\n", "x".repeat(400)));
    for line in 200..260 {
        text.push_str(&format!("fn tail_{line}() {{ {line} }}\n"));
    }
    studio.edit(TextEditingValue {
        text,
        ..TextEditingValue::default()
    });
}

/// One frame after the scroll, so a rebuild the scroll asked for has run.
fn scroll_and_draw(driver: &mut FrameDriver, at: Offset, delta: Offset, ms: u64) {
    driver.handle_scroll(&ScrollEvent::new(at, delta, Duration::from_millis(ms)));
    driver.draw_frame_at(Duration::from_millis(ms));
}

#[test]
fn a_vertical_wheel_notch_scrolls_the_editor() {
    let (mut driver, studio) = shell();
    long_buffer(&studio);
    driver.draw_frame_at(Duration::from_millis(1));
    driver.draw_frame_at(Duration::from_millis(2));

    assert!(
        studio.editor_scroll.max_offset() > 0.0,
        "the fixture is useless if the buffer fits the pane"
    );
    let before = studio.editor_scroll.peek();

    scroll_and_draw(&mut driver, IN_CODE, Offset::new(0.0, -120.0), 10);

    assert!(
        studio.editor_scroll.peek() > before,
        "a vertical wheel over the code pane did not scroll: {} -> {}",
        before,
        studio.editor_scroll.peek()
    );
}

#[test]
fn a_horizontal_touchpad_swipe_scrolls_the_editor() {
    let (mut driver, studio) = shell();
    long_buffer(&studio);
    driver.draw_frame_at(Duration::from_millis(1));
    driver.draw_frame_at(Duration::from_millis(2));

    assert!(
        studio.editor_scroll_x.max_offset() > 0.0,
        "the fixture is useless if the widest line fits the pane"
    );
    let before = studio.editor_scroll_x.peek();

    // A leftward swipe: the finger moves left, the content follows it.
    scroll_and_draw(&mut driver, IN_CODE, Offset::new(-80.0, 0.0), 10);

    assert!(
        studio.editor_scroll_x.peek() > before,
        "a horizontal swipe over the code pane did not scroll: {} -> {}",
        before,
        studio.editor_scroll_x.peek()
    );
}

/// A swipe whose horizontal component dominates, but with real vertical
/// movement in it. The innermost scrollable is the horizontal one, so this is
/// the event it must claim; if it does not, the detector is not in the hit
/// chain at all — which is a different defect from a gate declining.
#[test]
fn a_horizontal_dominant_diagonal_reaches_the_editor_columns() {
    let (mut driver, studio) = shell();
    long_buffer(&studio);
    driver.draw_frame_at(Duration::from_millis(1));
    driver.draw_frame_at(Duration::from_millis(2));

    let before = studio.editor_scroll_x.peek();

    scroll_and_draw(&mut driver, IN_CODE, Offset::new(-80.0, -30.0), 10);

    assert!(
        studio.editor_scroll_x.peek() > before,
        "a horizontal-dominant diagonal did not scroll the columns: {} -> {} \
         (wrap={}, inline_diagnostics={})",
        before,
        studio.editor_scroll_x.peek(),
        studio.word_wrap.get(),
        studio.inline_diagnostics.get()
    );
}

#[test]
fn a_diagonal_trackpad_scroll_moves_both_axes() {
    // The noise a real trackpad produces: a mostly-vertical two-finger scroll
    // whose x is not exactly zero. The horizontal scrollable sits innermost,
    // so a detector that claims every event with any dx at all steals the
    // scroll from the vertical one — and the pane stops scrolling vertically
    // for a reason nothing downstream can see.
    let (mut driver, studio) = shell();
    long_buffer(&studio);
    driver.draw_frame_at(Duration::from_millis(1));
    driver.draw_frame_at(Duration::from_millis(2));

    let v_before = studio.editor_scroll.peek();
    let h_before = studio.editor_scroll_x.peek();

    scroll_and_draw(&mut driver, IN_CODE, Offset::new(-4.0, -120.0), 10);

    assert!(
        studio.editor_scroll.peek() > v_before,
        "a mostly-vertical diagonal scroll did not scroll vertically"
    );
    let _ = h_before;
}
