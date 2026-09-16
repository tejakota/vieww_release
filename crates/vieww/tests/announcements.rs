//! Transient widgets announce themselves, without anybody remembering to.
//!
//! ```console
//! cargo test -p vieww --test announcements
//! ```
//!
//! # What `docs/AIMS.md` asks for
//!
//! §J: *"A snackbar that appears and disappears without being announced is
//! invisible to a screen reader… **Announcement should be part of what a
//! transient widget is**, not a property an application remembers to set."*
//!
//! The second sentence is the testable one, and it is what most of this file is
//! about. It is easy to build a snackbar that announces when the application
//! calls `announce()`; the claim here is stronger — the announcement is derived
//! from the semantics tree by [`SemanticsTree::announcements`], so it happens on
//! **every** path that mounts the widget, including paths written later by
//! somebody who has never read this file.
//!
//! So the tests below never call anything announcement-shaped. They mount a
//! widget the ordinary way and then ask the driver what a screen reader would
//! now say.
//!
//! # What is not claimed
//!
//! No cross-framework claim. The house rule is that an upstream comparison
//! needs a reading taken and dated, and none was — see `TRACKER.md`.

use vieww::foundation::Size;
use vieww::prelude::*;
use vieww::{FrameDriver, Liveness};

const SURFACE: Size = Size {
    width: 400.0,
    height: 600.0,
};

fn screen(overlay: Option<WidgetNode>) -> WidgetNode {
    let mut stack = Stack::new()
        .fit(StackFit::Expand)
        .children(children![Button::new("Send").on_pressed(|| {})]);
    if let Some(overlay) = overlay {
        stack = stack.push(overlay);
    }
    Theme::new(ThemeData::light()).child(stack).into()
}

/// A driver that has already drawn the plain screen and reported whatever that
/// first frame had to say — so every assertion below is about what *changed*.
fn settled() -> FrameDriver {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(screen(None));
    driver.draw_frame();
    let _ = driver.take_announcements();
    driver
}

fn show(driver: &mut FrameDriver, overlay: impl Into<WidgetNode>) -> Vec<String> {
    driver.set_root(screen(Some(overlay.into())));
    driver.draw_frame();
    driver
        .take_announcements()
        .into_iter()
        .map(|announcement| announcement.text)
        .collect()
}

#[test]
fn an_ordinary_screen_has_nothing_to_say() {
    let mut driver = settled();
    driver.draw_frame();
    assert!(
        driver.take_announcements().is_empty(),
        "a screen reader that read every button out as the user scrolled past \
         would be unusable; `Liveness::Off` has to be the default"
    );
}

#[test]
fn a_snackbar_announces_itself_the_moment_it_mounts() {
    let mut driver = settled();
    let said = show(&mut driver, Snackbar::new("Message sent"));
    assert!(
        said.iter().any(|text| text.contains("Message sent")),
        "nothing in this test asked for an announcement — the snackbar was \
         mounted the ordinary way. got: {said:?}"
    );
}

#[test]
fn a_snackbar_interrupts_and_a_tooltip_does_not() {
    let mut driver = settled();
    driver.set_root(screen(Some(Snackbar::new("Deleted").into())));
    driver.draw_frame();
    let snackbar = driver.take_announcements();
    assert_eq!(
        snackbar.first().map(|announcement| announcement.liveness),
        Some(Liveness::Assertive),
        "a snackbar often carries the only chance to undo something, so it \
         cuts across what the user is doing"
    );

    let mut driver = settled();
    driver.set_root(screen(Some(Tooltip::new("Copies the link").into())));
    driver.draw_frame();
    let tooltip = driver.take_announcements();
    assert_eq!(
        tooltip.first().map(|announcement| announcement.liveness),
        Some(Liveness::Polite),
        "a tooltip is an explanation somebody went looking for, not news. got: \
         {tooltip:?}"
    );
}

#[test]
fn an_inline_error_announces_itself() {
    let mut driver = settled();
    let said = show(
        &mut driver,
        InlineError::new(icons::add(), "Enter a valid email address"),
    );
    assert!(
        said.iter().any(|text| text.contains("valid email")),
        "a validation error nobody is told about is a form that silently \
         refuses to submit. got: {said:?}"
    );
}

#[test]
fn a_snackbar_that_stays_put_is_not_read_out_again() {
    let mut driver = settled();
    assert!(!show(&mut driver, Snackbar::new("Message sent")).is_empty());

    driver.draw_frame();
    assert!(
        driver.take_announcements().is_empty(),
        "the announcement is a diff, not a poll — repeating it every frame is \
         the failure mode of every `aria-live` region ever shipped"
    );
}

#[test]
fn a_new_message_in_the_same_snackbar_is_read_out() {
    let mut driver = settled();
    let _ = show(&mut driver, Snackbar::new("Message sent"));

    let said = show(&mut driver, Snackbar::new("Message failed to send"));
    assert!(
        said.iter().any(|text| text.contains("failed")),
        "a snackbar replaced by a different snackbar is usually the same \
         element reconciled with new words — the case an id comparison misses \
         and a user most needs to hear. got: {said:?}"
    );
}

#[test]
fn a_dismissed_snackbar_says_nothing_on_the_way_out() {
    let mut driver = settled();
    let _ = show(&mut driver, Snackbar::new("Message sent"));

    driver.set_root(screen(None));
    driver.draw_frame();
    assert!(
        driver.take_announcements().is_empty(),
        "appearing is news; going away is not. A screen reader narrating every \
         dismissal is reading the user their own history"
    );
}

/// The mechanism is open to a control written outside this repository, which is
/// §A's rule reaching accessibility: a third party naming the `alert` role is
/// live for the same reason `Snackbar` is, with no vieww change.
#[test]
fn a_third_partys_alert_is_live_without_any_framework_change() {
    let mut driver = settled();
    let said = show(
        &mut driver,
        Semantics::container("Connection lost")
            .role(SemanticRole::Custom("alert"))
            .child(SizedBox::from_size(Size::new(10.0, 10.0))),
    );
    assert!(
        said.iter().any(|text| text.contains("Connection lost")),
        "the liveness comes from the role, so it is not a list of vieww's own \
         widgets. got: {said:?}"
    );
}

/// Turning liveness off is possible and deliberately awkward — the point is that
/// silence has to be *asked* for, where in every other framework it is what you
/// get by forgetting.
#[test]
fn silence_has_to_be_asked_for() {
    let mut driver = settled();
    let said = show(
        &mut driver,
        Semantics::container("Connection lost")
            .role(SemanticRole::Custom("alert"))
            .live(SemanticLiveness::Off)
            .child(SizedBox::from_size(Size::new(10.0, 10.0))),
    );
    assert!(said.is_empty(), "got: {said:?}");
}
