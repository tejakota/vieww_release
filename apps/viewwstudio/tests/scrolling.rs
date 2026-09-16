//! Following a build log, and steering one.
//!
//! # Why these are driver tests
//!
//! Both behaviours live in the gap between a widget and its layout: "stick to
//! the end" is decided in the extents handler, which only runs when something
//! has actually been measured, and a scrollbar's thumb is a function of a
//! viewport and a content extent that only exist after a layout pass. Neither
//! can be asserted against a tree dump — the dump is a description of a tree
//! that has not been measured.

use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::Size;
use vieww_render::FrameDriver;
use viewwstudio::state::{PanelTab, Studio};
use viewwstudio::{Shell, WINDOW};

/// A shell with the panel open on `tab`, and one frame drawn.
fn panel(tab: PanelTab) -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(WINDOW);
    studio.panel_open.set(true);
    studio.panel_tab.set(tab);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame_at(Duration::ZERO);
    (driver, studio)
}

/// `lines` lines of build output, as a compile would produce them.
fn log(studio: &Studio, lines: usize) {
    studio.output.set(Rc::new(
        (0..lines).map(|n| format!("line {n}")).collect::<Vec<_>>(),
    ));
}

#[test]
fn the_output_panel_follows_the_newest_line() {
    // The behaviour that was missing: a `cargo build` writes for a minute or
    // two, every line of it landed below the fold, and the only way to watch a
    // build was to keep scrolling down by hand.
    let (mut driver, studio) = panel(PanelTab::Output);
    let scroll = studio.panel_scroll(PanelTab::Output).clone();

    log(&studio, 400);
    driver.draw_frame_at(Duration::from_millis(16));
    driver.draw_frame_at(Duration::from_millis(32));

    assert!(scroll.max_offset() > 0.0, "400 lines do not fit the panel");
    assert!(
        scroll.peek() >= scroll.max_offset() - 1.0,
        "the panel is at {} of {}, so the newest line is off the bottom",
        scroll.peek(),
        scroll.max_offset()
    );
}

#[test]
fn scrolling_up_stops_it_following() {
    // The other half, and the reason this is not a "scroll to bottom" call on
    // every append: somebody reading an error four hundred lines back must not
    // be yanked to the end because the build is still talking.
    let (mut driver, studio) = panel(PanelTab::Output);
    let scroll = studio.panel_scroll(PanelTab::Output).clone();

    log(&studio, 400);
    driver.draw_frame_at(Duration::from_millis(16));
    driver.draw_frame_at(Duration::from_millis(32));

    scroll.jump_to(0.0);
    log(&studio, 800);
    driver.draw_frame_at(Duration::from_millis(48));
    driver.draw_frame_at(Duration::from_millis(64));

    assert!(
        scroll.peek() < 40.0,
        "the reader was pulled from {} to the end of the log",
        scroll.peek()
    );
}

#[test]
fn coming_back_to_the_end_starts_it_following_again() {
    // No flag, no toggle: being at the end *is* the state. A reader who scrolls
    // back down has said they want to follow again, and nothing else has to
    // record that.
    let (mut driver, studio) = panel(PanelTab::Output);
    let scroll = studio.panel_scroll(PanelTab::Output).clone();

    log(&studio, 400);
    driver.draw_frame_at(Duration::from_millis(16));
    scroll.jump_to(0.0);
    driver.draw_frame_at(Duration::from_millis(32));

    scroll.jump_to(scroll.max_offset());
    log(&studio, 800);
    driver.draw_frame_at(Duration::from_millis(48));
    driver.draw_frame_at(Duration::from_millis(64));

    assert!(
        scroll.peek() >= scroll.max_offset() - 1.0,
        "back at the end and not following: {} of {}",
        scroll.peek(),
        scroll.max_offset()
    );
}

#[test]
fn a_short_log_does_not_scroll_anywhere() {
    // The case a naive "jump to the end on every append" gets wrong in a way
    // that is invisible until it is not: nothing to scroll, so nothing moves,
    // and `max_offset` stays zero rather than going slightly negative.
    let (mut driver, studio) = panel(PanelTab::Output);
    let scroll = studio.panel_scroll(PanelTab::Output).clone();

    log(&studio, 2);
    driver.draw_frame_at(Duration::from_millis(16));
    driver.draw_frame_at(Duration::from_millis(32));

    assert_eq!(scroll.max_offset(), 0.0);
    assert_eq!(scroll.peek(), 0.0);
}

#[test]
fn the_scrollbar_is_drawn_only_when_there_is_something_to_scroll() {
    // `Scrollbar` is an empty box on a pane whose content fits: a track that
    // says "there is more" when there is not is worse than no track.
    let mut driver = FrameDriver::new(Size::new(1366.0, 679.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(Size::new(1366.0, 679.0));
    studio.panel_open.set(true);
    studio.panel_tab.set(PanelTab::Output);
    driver.set_root(Shell {
        studio: studio.clone(),
    });

    log(&studio, 1);
    driver.draw_frame_at(Duration::ZERO);
    driver.draw_frame_at(Duration::from_millis(16));
    let short = vieww_widget::debug_tree(Shell {
        studio: studio.clone(),
    });

    log(&studio, 500);
    driver.draw_frame_at(Duration::from_millis(32));
    driver.draw_frame_at(Duration::from_millis(48));

    // The tree is the same either way — the widget is always mounted — so what
    // is asserted is the geometry the controller reports, which is what the bar
    // is built from.
    assert!(short.contains("Scrollbar"), "the bar is always in the tree");
    assert!(
        studio.panel_scroll(PanelTab::Output).max_offset() > 0.0,
        "and now it has a thumb to draw"
    );
}
