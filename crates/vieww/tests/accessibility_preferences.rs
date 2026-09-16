//! The OS accessibility settings, from `FrameDriver` down to laid-out text.
//!
//! ```console
//! cargo test -p vieww --test accessibility_preferences
//! ```
//!
//! # Why this measures geometry rather than reading the value back
//!
//! Asserting that `driver.accessibility().text_scale == 2.0` would pass for an
//! implementation that stores the number and never applies it — which is
//! precisely the failure mode this whole item exists to avoid, and precisely
//! the one nobody notices, because the developer writing the feature does not
//! have Large Text turned on.
//!
//! So every assertion here is about the **size of something on screen**.

use vieww::foundation::{Accessibility, Size};
use vieww::prelude::*;

const SURFACE: Size = Size {
    width: 400.0,
    height: 300.0,
};

/// A screen whose only content is one line of text, so the tree's measured
/// height *is* the text's height.
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
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_size(MainAxisSize::Min)
            .children(children![Text::new("Hello").style(TextStyle::new(10.0))])
            .into()
    }
}

vieww::widget::widget_node_from!(Screen);

/// The height of the drawn text, via the scene's own bounds.
///
/// The scene is what a backend rasterises, so this is the size the person
/// actually sees rather than a number from a layout struct that a paint pass
/// might ignore.
fn drawn_height(accessibility: Accessibility) -> f32 {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_accessibility(accessibility);
    driver.set_root(Screen);
    driver.draw_frame();
    driver.scene().bounds().height()
}

#[test]
fn the_default_preferences_change_nothing() {
    let plain = drawn_height(Accessibility::default());
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    assert_eq!(
        driver.scene().bounds().height(),
        plain,
        "a driver that was never told anything must match one told the defaults"
    );
}

/// The claim, on pixels: doubling the OS text size makes the text on screen
/// meaningfully bigger.
///
/// Asserted as a ratio band rather than an exact figure, because the exact
/// height depends on the embedded font's ascent and descent, and pinning it
/// would make this test fail on a font update for no reason anyone cares
/// about. The band is still far tighter than "unchanged" — a no-op
/// implementation scores 1.0 and fails.
#[test]
fn a_larger_text_scale_makes_the_text_on_screen_larger() {
    let plain = drawn_height(Accessibility::default());
    let doubled = drawn_height(Accessibility::new(false, false, 2.0));

    assert!(plain > 0.0, "the fixture must draw some text");
    let ratio = doubled / plain;
    assert!(
        (1.7..=2.3).contains(&ratio),
        "doubling the text scale should roughly double the drawn height, \
         got {plain} -> {doubled} (ratio {ratio})"
    );
}

#[test]
fn a_smaller_text_scale_makes_the_text_on_screen_smaller() {
    let plain = drawn_height(Accessibility::default());
    let small = drawn_height(Accessibility::new(false, false, 0.5));

    assert!(
        small < plain,
        "halving the text scale should shrink the drawn height, \
         got {plain} -> {small}"
    );
}

/// Publishing is a republish, so it reaches a tree that is *already mounted* —
/// which is the case that matters, because a person can change this from
/// Control Centre while the application is on screen. An implementation that
/// only read the value at mount would pass every test above and fail this one.
#[test]
fn changing_the_preferences_while_mounted_reaches_the_tree() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();
    let before = driver.scene().bounds().height();

    let changed = driver.set_accessibility(Accessibility::new(false, false, 2.0));
    driver.draw_frame();
    let after = driver.scene().bounds().height();

    assert!(changed, "a real change must report itself as one");
    assert!(
        after > before,
        "the already-mounted tree did not pick up the new text scale: \
         {before} -> {after}"
    );
}

/// Setting the same preferences twice must not republish — a republish is a
/// full reconciliation, and one per frame would defeat every relayout boundary
/// in the tree. The same property `set_capture` has.
#[test]
fn setting_the_same_preferences_again_is_not_a_republish() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.draw_frame();

    let prefs = Accessibility::new(true, true, 1.5);
    assert!(driver.set_accessibility(prefs), "the first set is a change");
    assert!(
        !driver.set_accessibility(prefs),
        "setting an identical value must be a no-op"
    );
}

/// `reduce_motion` and `high_contrast` are published rather than applied by the
/// framework today — see `Accessibility`'s module docs for exactly why. What
/// *is* guaranteed is that they arrive, so application code can read them.
///
/// This is deliberately a weak test, and it is weak in the honest direction: it
/// pins what is actually true rather than implying the framework acts on them.
#[test]
fn the_unapplied_preferences_still_reach_the_driver() {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Screen);
    driver.set_accessibility(Accessibility::new(true, true, 1.0));

    let prefs = driver.accessibility();
    assert!(prefs.reduce_motion);
    assert!(prefs.high_contrast);
    // And the helper an application uses to honour it.
    assert_eq!(
        prefs.motion(std::time::Duration::from_millis(300)),
        std::time::Duration::ZERO
    );
}

// ---------------------------------------------------------------- damage cost

/// `FrameStats` now reports what a frame *cost in pixels* beside what it cost
/// in milliseconds. Lives in this file rather than its own because it needs
/// the same one-widget fixture.
mod damage_cost {
    use super::{Screen, SURFACE};
    use std::time::Duration;
    use vieww::paint::FrameScheduler;
    use vieww::prelude::*;

    fn driven() -> (
        FrameDriver,
        FrameScheduler,
        Option<vieww::paint::FrameStats>,
    ) {
        let mut driver = FrameDriver::new(SURFACE);
        let mut scheduler = FrameScheduler::new(60.0);
        driver.set_root(Screen);
        scheduler.request_frame();
        let stats = driver.drive(&mut scheduler, Duration::ZERO, || Duration::ZERO);
        (driver, scheduler, stats)
    }

    /// The first frame draws everything, so it must report a non-zero area.
    ///
    /// This is the assertion that fails if the annotation reads the damage
    /// *before* the pulse rather than after it — which is what the first
    /// version of `FrameDriver::drive` did, reporting zero damage for the
    /// first frame of every application. See the comment there.
    #[test]
    fn the_first_frame_reports_the_pixels_it_actually_painted() {
        let (_driver, _scheduler, stats) = driven();
        let stats = stats.expect("a requested frame produces stats");

        assert!(
            stats.damage_area > 0.0,
            "a frame that drew the whole screen reported no damaged pixels"
        );
        assert!(stats.damage_regions > 0);
    }

    /// And the fraction is a fraction — the form a performance overlay wants.
    #[test]
    fn the_damaged_fraction_is_a_proportion_of_the_surface() {
        let (_driver, _scheduler, stats) = driven();
        let stats = stats.expect("stats");

        let fraction = stats.damage_fraction(SURFACE).expect("a real surface");
        assert!(
            (0.0..=1.0).contains(&fraction),
            "a fraction outside 0..=1: {fraction}"
        );
        assert!(fraction > 0.0);
    }

    /// A zero-area surface has no denominator, and asking for a percentage of
    /// nothing should not produce an infinity that then reaches a log line.
    #[test]
    fn a_zero_sized_surface_has_no_fraction_rather_than_an_infinite_one() {
        let (_driver, _scheduler, stats) = driven();
        let stats = stats.expect("stats");
        assert_eq!(stats.damage_fraction(Size::new(0.0, 0.0)), None);
    }

    /// The scheduler's stored copy agrees with the returned one, so a caller
    /// reading `last_stats` does not see a different number from a caller
    /// reading the return value.
    #[test]
    fn the_stored_stats_carry_the_damage_too() {
        let (_driver, scheduler, stats) = driven();
        let returned = stats.expect("stats");
        let stored = scheduler.last_stats().expect("stored");

        assert_eq!(stored.damage_area, returned.damage_area);
        assert_eq!(stored.damage_regions, returned.damage_regions);
    }
}
