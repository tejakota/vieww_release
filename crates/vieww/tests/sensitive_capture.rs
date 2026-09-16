//! Masked sensitive views, end to end.
//!
//! ```console
//! cargo test -p vieww --test sensitive_capture
//! ```
//!
//! # What `docs/AIMS.md` asks for
//!
//! §D: *"A framework that knows which subtree is sensitive can suppress it from
//! screenshots, from the recents thumbnail, and from the semantics tree in **one
//! declaration**."*
//!
//! The load-bearing words are "one declaration". Three suppressions that an
//! application has to remember separately is what every other framework already
//! offers, and it is why applications ship having handled the screenshot and not
//! the task-switcher thumbnail. So the assertions below are deliberately all
//! about *the same* `Sensitive::new()` call: the pixels, the semantics tree and
//! the hit testing all change together, and no application code runs in between.
//!
//! # What is not claimed
//!
//! No cross-framework claim. The house rule is that an upstream comparison
//! needs a reading taken and dated, and none was — see `TRACKER.md`. What is
//! tested here is vieww's behaviour.

use vieww::foundation::{Capture, Color, Offset, Rect, Size};
use vieww::prelude::*;
use vieww::FrameDriver;

const SURFACE: Size = Size {
    width: 200.0,
    height: 100.0,
};

/// The secret is red, the page behind it is white, and the mask is black — so
/// "did the mask work" is a question about one pixel rather than about a hash.
const SECRET: Color = Color::RED;
const PAGE: Color = Color::WHITE;
const MASK: Color = Color::BLACK;

/// One declaration, and everything below is about what it does.
fn app() -> WidgetNode {
    ColoredBox::new(PAGE)
        .child(
            Sensitive::new()
                .cover(MASK)
                .label("Balance, hidden while the screen is being recorded")
                .child(
                    SizedBox::from_size(Size::new(200.0, 100.0))
                        .child(ColoredBox::new(SECRET).child(Text::new("12402.11"))),
                ),
        )
        .into()
}

fn driver() -> FrameDriver {
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Theme::new(ThemeData::light()).child(app()));
    driver.draw_frame();
    driver
}

/// The topmost solid fill covering the middle of the surface, and the rectangle
/// it covered.
///
/// Read off the scene rather than off a rasterised buffer, so this runs on a CI
/// box with no graphics adapter — the question here is what was recorded to be
/// drawn, and a GPU would only confirm that the rasteriser honoured it.
/// `paint_to_pixels.rs` is where that half is checked.
fn topmost(driver: &mut FrameDriver) -> (Rect, Color) {
    driver.draw_frame();
    let middle = Offset::new(SURFACE.width / 2.0, SURFACE.height / 2.0);
    driver
        .scene()
        .fills()
        .into_iter()
        .rfind(|(rect, paint)| rect.contains(middle) && !paint.color.is_transparent())
        .map(|(rect, paint)| (rect, paint.color))
        .expect("something is painted at the centre of a mounted surface")
}

fn centre(driver: &mut FrameDriver) -> Color {
    topmost(driver).1
}

#[test]
fn the_secret_is_on_screen_while_only_the_user_is_looking() {
    let mut driver = driver();
    assert_eq!(
        driver.capture(),
        Capture::Screen,
        "a surface nobody has said anything about is one the user is looking at"
    );
    assert_eq!(
        centre(&mut driver),
        SECRET,
        "masking is a state, not a permanent exclusion — a balance you can \
         never see is not a feature"
    );
}

#[test]
fn a_recording_covers_the_pixels_without_the_application_being_asked() {
    let mut driver = driver();
    assert!(
        driver.set_capture(Capture::Recorded),
        "this changed something"
    );
    assert_eq!(
        centre(&mut driver),
        MASK,
        "no application code ran between the notification and the frame; that \
         is the whole point of publishing the state rather than firing an event"
    );
}

#[test]
fn the_same_declaration_hides_it_from_a_screen_reader() {
    let mut driver = driver();
    let before = driver.semantics().describe();
    assert!(
        before.contains("12402.11"),
        "the balance reads normally when nobody is recording. got:\n{before}"
    );

    driver.set_capture(Capture::Recorded);
    driver.draw_frame();
    let after = driver.semantics().describe();
    assert!(
        !after.contains("12402.11"),
        "a screen reader is a second way to read the pixels, and a screen \
         recording usually carries audio. got:\n{after}"
    );
    assert!(
        after.contains("hidden while the screen is being recorded"),
        "and it says something is here rather than going silent — silence \
         reads as an application bug. got:\n{after}"
    );
}

#[test]
fn unmasking_restores_everything_the_mask_took_away() {
    let mut driver = driver();
    driver.set_capture(Capture::Recorded);
    assert_eq!(centre(&mut driver), MASK);

    assert!(driver.set_capture(Capture::Screen));
    assert_eq!(
        centre(&mut driver),
        SECRET,
        "the screenshot is over; the balance comes back"
    );
    assert!(driver.semantics().describe().contains("12402.11"));
}

#[test]
fn a_redundant_notification_costs_nothing() {
    let mut driver = driver();
    assert!(driver.set_capture(Capture::Recorded));
    assert!(
        !driver.set_capture(Capture::Recorded),
        "platforms deliver these more than once, and a re-publish is a full \
         reconciliation — saying so lets a caller tell a real change from noise"
    );
}

/// The masked subtree keeps its geometry, so the mask is not itself visible as
/// a reflow in the frame it is meant to be invisible in.
#[test]
fn the_mask_lands_exactly_where_the_secret_was() {
    let mut driver = driver();
    let (secret, colour) = topmost(&mut driver);
    assert_eq!(colour, SECRET);

    driver.set_capture(Capture::Recorded);
    let (mask, colour) = topmost(&mut driver);
    assert_eq!(colour, MASK);

    assert_eq!(
        mask, secret,
        "the sensitive subtree is still laid out while it is hidden. A mask \
         that reflowed would leak the shape of what it hid, and would be \
         visible to the user for the frame in which it happened"
    );
}
