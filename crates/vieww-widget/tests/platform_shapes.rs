//! The controls that look different on iOS and on Android, and do.
//!
//! # Why this file exists
//!
//! `ThemeData::adaptive` has chosen the platform's colours, metrics and motion
//! since it was written, and its own documentation was candid that this "does
//! not buy an iOS switch's shape, an iOS picker … and calling this
//! 'iOS support' would be a claim no screenshot supports."
//!
//! That mattered most one level up: `viewwstudio`'s preview pane simulates a
//! device — safe area, scroll physics, `TargetPlatform` — and its caption had
//! to say "widgets are not visually reskinned per platform", which is the one
//! sentence that stops a developer trusting the preview for the question they
//! opened it to answer.
//!
//! `ThemeData::platform` is now a field, and the controls where the two systems
//! genuinely draw different objects read it. This is the test that says so —
//! per control, on the property that is actually the tell, rather than on a
//! screenshot hash that would fail on every unrelated colour change.
//!
//! # What is deliberately *not* here
//!
//! Every other control in the catalogue. A text field, a card, a list row, a
//! badge, a tooltip: these differ in tokens, which `adaptive` already handles,
//! and inventing per-platform shapes for them would be two catalogues pretending
//! to be one.

use vieww_foundation::TargetPlatform;
use vieww_widget::prelude::*;
use vieww_widget::{
    Button, ButtonStyle, Checkbox, CircularProgress, Dialog, SegmentedControl, Slider, Switch,
    Theme, ThemeData,
};

/// Build `widget` under `platform`'s theme and return the tree as a dump.
fn under(platform: TargetPlatform, widget: impl Into<WidgetNode>) -> String {
    vieww_widget::debug_tree(Theme::new(ThemeData::adaptive(platform, false)).child(widget))
}

/// Both platforms, so a test can say "these differ" without naming which.
fn both(build: impl Fn(TargetPlatform) -> String) -> (String, String) {
    (build(TargetPlatform::IOS), build(TargetPlatform::Android))
}

/// The default follows the machine, and that is a decision with a cost.
///
/// # Both halves of it
///
/// An application that writes `ThemeData::light()` and nothing else has not
/// asked to look the same everywhere — it has not thought about it — and looking
/// native is the answer that is right more often. So `light()` and `dark()`
/// carry `TargetPlatform::current()`.
///
/// The cost is that a *test* written against `light()` cannot assert a shape:
/// the same line would pass on Linux and fail on a Mac. `with_platform` is what
/// pays it. Anything in this workspace that checks a shape names the platform
/// it means — this file does it in `under`, and `switch.rs`'s
/// `off_differs_from_on_in_shape_as_well_as_colour` is the one place inside the
/// crate that has to.
#[test]
fn the_default_theme_follows_the_host_and_can_be_pinned_off_it() {
    assert_eq!(
        ThemeData::light().platform,
        TargetPlatform::current(),
        "a default that ignored the machine would make every unbranded app \
         look foreign on one of the two phones"
    );
    assert_eq!(ThemeData::dark().platform, TargetPlatform::current());

    // And the way out, for a test that means a shape and for a branded app that
    // means one look everywhere.
    for platform in [TargetPlatform::IOS, TargetPlatform::Android] {
        assert_eq!(
            ThemeData::light().with_platform(platform).platform,
            platform
        );
    }
}

/// `with_platform` changes the shapes and nothing else.
///
/// The narrower promise is the point: `adaptive` moves colours, metrics, motion
/// and shapes together, and this moves one. A branded application pinning its
/// shapes must not silently lose the palette it chose.
#[test]
fn pinning_the_platform_leaves_the_colours_and_metrics_alone() {
    let base = ThemeData::light();
    let pinned = base.with_platform(TargetPlatform::IOS);

    assert_eq!(pinned.colors, base.colors);
    assert_eq!(pinned.metrics.touch_target, base.metrics.touch_target);
    assert_eq!(pinned.metrics.corner, base.metrics.corner);
    assert_eq!(pinned.motion.duration_short, base.motion.duration_short);

    // Where `adaptive` moves all of them, which is why the two are not
    // interchangeable.
    let adaptive = ThemeData::adaptive(TargetPlatform::IOS, false);
    assert_eq!(adaptive.platform, pinned.platform);
    assert_ne!(
        adaptive.metrics.touch_target, base.metrics.touch_target,
        "44 against 48 — `adaptive` is the one that takes the platform's numbers"
    );
}

#[test]
fn the_theme_carries_the_platform_it_was_built_for() {
    // The field everything below reads. Without it a control can know it is
    // wearing Apple's blue and still not know it is on Apple.
    assert_eq!(
        ThemeData::adaptive(TargetPlatform::IOS, false).platform,
        TargetPlatform::IOS
    );
    assert_eq!(
        ThemeData::adaptive(TargetPlatform::Android, true).platform,
        TargetPlatform::Android
    );
}

#[test]
fn a_switch_is_a_different_object_on_each_platform() {
    let (apple, android) = both(|platform| under(platform, Switch::new(true)));
    assert_ne!(
        apple, android,
        "the switch is the control people point at first"
    );
}

#[test]
fn a_slider_is_a_different_object_on_each_platform() {
    let (apple, android) = both(|platform| under(platform, Slider::new(0.5)));
    assert_ne!(apple, android);
}

#[test]
fn a_checkbox_is_round_on_apple_and_square_elsewhere() {
    let (apple, android) = both(|platform| under(platform, Checkbox::new(true)));
    assert_ne!(apple, android);
}

#[test]
fn a_filled_button_is_a_pill_on_material_and_a_rectangle_on_apple() {
    let (apple, android) =
        both(|platform| under(platform, Button::new("Done").style(ButtonStyle::Filled)));
    assert_ne!(apple, android);
}

#[test]
fn an_indeterminate_spinner_is_spokes_on_apple_and_an_arc_elsewhere() {
    let (apple, android) = both(|platform| under(platform, CircularProgress::indeterminate()));
    assert_ne!(apple, android);
}

#[test]
fn a_segmented_control_inverts_its_figure_and_ground_on_apple() {
    let (apple, android) =
        both(|platform| under(platform, SegmentedControl::new(["Day", "Week"], 0)));
    assert_ne!(apple, android);
}

#[test]
fn an_alert_stacks_its_buttons_on_apple_and_rows_them_elsewhere() {
    let (apple, android) = both(|platform| {
        under(
            platform,
            Dialog::new().title("Delete this file?").actions(children![
                Button::new("Cancel").style(ButtonStyle::Text),
                Button::new("Delete").style(ButtonStyle::Text),
            ]),
        )
    });
    assert_ne!(apple, android);
}

/// The other half of the promise, and the one that keeps this *one* catalogue.
///
/// A control with no platform-specific form must be identical under both, or
/// "the platform decides the shape" has quietly become "the platform decides
/// everything" and every future control has to be checked twice.
#[test]
fn a_control_with_no_platform_form_is_identical_under_both() {
    // Same theme, same platform difference available, nothing to say about it.
    let apple = under(TargetPlatform::IOS, Text::new("hello"));
    let android = under(TargetPlatform::Android, Text::new("hello"));
    assert_eq!(apple, android);
}
