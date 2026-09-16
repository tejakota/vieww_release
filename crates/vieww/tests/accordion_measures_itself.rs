//! The accordion reveals to its body's own height, with nobody supplying it.
//!
//! ```console
//! cargo test -p vieww --test accordion_measures_itself
//! ```
//!
//! `Accordion::content_height` used to be a **required** parameter, and both
//! `docs/PRODUCTION-GAPS.md` and `NEXT.md` cited it as the visible edge of the
//! missing intrinsic-size pass: a framework asking the application how tall the
//! framework's own text is.
//!
//! # The fix is not the intrinsic pass, and that is the interesting part
//!
//! The stated reasoning was that revealing to a natural height needs the body
//! measured *before* it is shown. It does not. The body is laid out normally
//! every frame; what the reveal needs is to be a **fraction of whatever the
//! body chose**, which is [`Align::height_factor`] and one layout pass. The
//! intrinsic pass is for the other shape — where a parent's own constraint
//! depends on a child's natural size, which a reveal's does not.
//!
//! So the two land together but they are separate mechanisms, and this suite
//! only exercises the reveal. `intrinsic_sizing.rs` covers the other.
//!
//! # Why the assertion is a measured box
//!
//! The old parameter's failure mode was silent: a caller's number and the
//! body's real height drift apart when the content changes, and the symptom is
//! a body clipped a few points short or a gap under it. Nothing in a widget
//! tree dump shows that. So the assertion is the laid-out height of the revealed
//! region against the laid-out height of the body itself, and the test that
//! matters is the one where the body changes size and nobody is told.

use vieww::foundation::Size;
use vieww::{FrameDriver, RenderObject};

const SURFACE: Size = Size {
    width: 300.0,
    height: 600.0,
};

/// Every laid-out size for the named render object, in tree order.
fn sizes_of(driver: &FrameDriver, debug_name: &str) -> Vec<Size> {
    let tree = driver.owner().tree();
    tree.ids()
        .into_iter()
        .filter(|&id| tree.object(id).map(RenderObject::debug_name) == Some(debug_name))
        .map(|id| tree.size(id))
        .collect()
}

/// The revealed region: the `RenderAlign` the accordion puts under its `Clip`.
///
/// Identified by being the one with a height factor set, which no other `Align`
/// in this tree has — the accordion is the only thing here that reveals.
fn revealed_height(driver: &FrameDriver) -> f32 {
    let tree = driver.owner().tree();
    let mut found = None;
    for id in tree.ids() {
        let Some(object) = tree.object(id) else {
            continue;
        };
        let any: &dyn std::any::Any = object;
        let Some(align) = any.downcast_ref::<vieww::render::RenderAlign>() else {
            continue;
        };
        if align.height_factor.is_some() {
            assert!(
                found.is_none(),
                "more than one Align with a height factor; this test wants the \
                 accordion's and there should be exactly one"
            );
            found = Some(tree.size(id).height);
        }
    }
    found.expect("the accordion builds an Align with a height factor")
}

/// An accordion holding `body`, driven to a settled state.
fn drive(body_height: f32, expanded: bool) -> FrameDriver {
    use vieww::prelude::*;

    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(
        Theme::new(ThemeData::light()).child(
            Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .children(children![Accordion::new(
                    Text::new("Header"),
                    // A box rather than text, so the number under test is one
                    // this test declared rather than one the font decided.
                    SizedBox::from_size(Size::new(120.0, body_height))
                        .child(ColoredBox::new(Color::rgb(1, 2, 3))),
                )
                .expanded(expanded)]),
        ),
    );

    // The reveal is animated, so the settled state is several frames away. A
    // simulated clock rather than sleeping: the answer must not depend on how
    // busy the machine is.
    for step in 0..90 {
        driver.draw_frame_at(std::time::Duration::from_millis(step * 16));
    }
    driver
}

#[test]
fn an_open_accordion_reveals_exactly_its_bodys_height() {
    for body_height in [40.0, 137.0, 260.0] {
        let driver = drive(body_height, true);

        assert!(
            sizes_of(&driver, "RenderColoredBox")
                .iter()
                .any(|size| (size.height - body_height).abs() < 0.5),
            "the body is laid out at the {body_height} it declared"
        );
        let revealed = revealed_height(&driver);
        assert!(
            (revealed - body_height).abs() < 0.5,
            "a fully open accordion reveals {revealed:.1} for a body of \
             {body_height} — and nothing told it that number"
        );
    }
}

#[test]
fn a_closed_accordion_reveals_nothing() {
    let driver = drive(137.0, false);
    let revealed = revealed_height(&driver);
    assert!(
        revealed.abs() < 0.5,
        "a closed accordion takes no vertical space for its body; it took \
         {revealed:.1}"
    );
}

/// **The assertion the old API could not make.**
///
/// The body's height changes and nobody updates a constant, because there is no
/// longer a constant to update. Under `content_height` this is precisely the
/// case that broke: the caller's number stayed at the old value and the body
/// was clipped or floated.
#[test]
fn a_body_that_changes_height_reveals_to_the_new_one() {
    let short = revealed_height(&drive(50.0, true));
    let tall = revealed_height(&drive(200.0, true));

    assert!(
        (short - 50.0).abs() < 0.5 && (tall - 200.0).abs() < 0.5,
        "the reveal follows the body: got {short:.1} and {tall:.1} for bodies \
         of 50 and 200"
    );
}
