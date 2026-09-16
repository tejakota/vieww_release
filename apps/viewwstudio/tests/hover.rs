//! The activity bar's tooltip stays up while the pointer rests on the icon.
//!
//! # The reported defect
//!
//! "The text box on hover of icons on the left vertical tool bar flickers when
//! hovered." Reproduced here before it was fixed: with the pointer held
//! perfectly still on an icon, `hovered_view` alternated `Some(view)`, `None`,
//! `Some(view)`, `None` — once per frame, for as long as the pointer stayed
//! there.
//!
//! The loop was: the tooltip is placed above the icon and clamped downwards
//! when it does not fit, so near the top of the column it covered the icon it
//! was explaining. Its panel draws, so it takes the hit test; the icon was told
//! the pointer had left; `hovered_view` cleared; the tooltip came down; the
//! hover landed on the icon again. `IgnorePointer` around the tooltip layer is
//! the fix — see `ui::tooltip`.
//!
//! # Why the anchors rather than coordinates
//!
//! The bar publishes each button's rectangle through `Studio::view_anchors`
//! during paint. Hard-coding "31, 77" would make this a test about a layout
//! that is allowed to change; asking the bar where its buttons are makes it a
//! test about hover.

use std::time::Duration;

use vieww_foundation::{Offset, Size, TargetPlatform};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

/// How many frames the pointer rests on one icon. A flicker shows up on the
/// second frame; ten is enough to see it settle rather than to catch it
/// mid-transition.
const FRAMES: usize = 10;

fn shell() -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    (driver, studio)
}

#[test]
fn a_pointer_resting_on_an_activity_bar_icon_keeps_one_tooltip_up() {
    let (mut driver, studio) = shell();

    let anchors: Vec<vieww_foundation::Rect> =
        studio.view_anchors.borrow().iter().copied().collect();
    let mut checked = 0;

    for (index, anchor) in anchors.iter().enumerate() {
        if anchor.width() <= 0.0 || anchor.height() <= 0.0 {
            continue;
        }
        let centre = Offset::new(
            (anchor.left + anchor.right) / 2.0,
            (anchor.top + anchor.bottom) / 2.0,
        );

        let mut seen = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES {
            // Hover is delivered by the platform layer, not by pointer events —
            // this is the call a window makes for a mouse that has not moved.
            driver.handle_hover(Some(centre));
            driver.draw_frame_at(Duration::from_millis(16 * frame as u64));
            seen.push(studio.hovered_view.get());
        }

        let first = seen[0];
        assert!(
            first.is_some(),
            "button {index} at {centre:?} never took the hover at all"
        );
        assert!(
            seen.iter().all(|view| *view == first),
            "the tooltip for button {index} flickered: {seen:?}"
        );
        checked += 1;
    }

    assert!(
        checked >= 10,
        "only {checked} buttons were measured — the bar publishes anchors for \
         every view, so this test stopped testing what it says it does"
    );
}

/// The invariant underneath the flicker, asserted directly.
///
/// The alternation only *appears* when the card happens to overlap the icon —
/// which depends on the card's height, so on the text in it, the theme's font
/// and where the icon sits in the column. Shortening the message to the name
/// alone stops the current bar from overlapping and would let the flicker test
/// above pass with the fix removed, which is a test that has stopped guarding
/// anything.
///
/// So this asserts the property rather than the symptom: while the tooltip is
/// up, the hit test at the icon's centre reaches exactly what it reached before
/// the tooltip existed. A tooltip that can be hit fails this whatever its size.
#[test]
fn the_tooltip_is_never_what_the_pointer_finds() {
    let (mut driver, studio) = shell();
    let anchors: Vec<vieww_foundation::Rect> =
        studio.view_anchors.borrow().iter().copied().collect();

    for (index, anchor) in anchors.iter().enumerate() {
        if anchor.width() <= 0.0 {
            continue;
        }
        let centre = Offset::new(
            (anchor.left + anchor.right) / 2.0,
            (anchor.top + anchor.bottom) / 2.0,
        );

        // With nothing hovered, whatever the pointer finds at the icon's centre
        // is the icon.
        driver.handle_hover(None);
        driver.draw_frame_at(Duration::from_millis(1));
        let bare = path(&driver, centre);

        // Now the tooltip is up.
        driver.handle_hover(Some(centre));
        driver.draw_frame_at(Duration::from_millis(2));
        assert!(
            studio.hovered_view.get().is_some(),
            "button {index} did not raise a tooltip, so this proves nothing"
        );
        let with_tooltip = path(&driver, centre);

        assert_eq!(
            bare, with_tooltip,
            "the tooltip for button {index} changed what the pointer finds"
        );
    }
}

/// What a hit test at `point` lands on, outermost render object last.
fn path(driver: &FrameDriver, point: Offset) -> Vec<&'static str> {
    driver
        .hit_test(point)
        .entries()
        .iter()
        .filter_map(|entry| {
            driver
                .owner()
                .tree()
                .object(entry.id)
                .map(vieww_render::RenderObject::debug_name)
        })
        .collect()
}

/// Moving from one icon to the next hands the tooltip over rather than dropping
/// it: the neighbour's enter and the first one's leave can arrive in either
/// order, and the bar's `on_hover` is written to survive both.
#[test]
fn moving_between_two_icons_hands_the_tooltip_over() {
    let (mut driver, studio) = shell();
    let anchors: Vec<vieww_foundation::Rect> =
        studio.view_anchors.borrow().iter().copied().collect();
    let centre = |rect: vieww_foundation::Rect| {
        Offset::new(
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        )
    };

    driver.handle_hover(Some(centre(anchors[0])));
    driver.draw_frame_at(Duration::from_millis(16));
    let first = studio.hovered_view.get();

    driver.handle_hover(Some(centre(anchors[1])));
    driver.draw_frame_at(Duration::from_millis(32));
    let second = studio.hovered_view.get();

    assert!(first.is_some(), "the first icon took the hover");
    assert!(
        second.is_some() && second != first,
        "the second icon took it over: {first:?} -> {second:?}"
    );
}

/// Off the bar entirely, the tooltip goes away and stays away.
#[test]
fn leaving_the_bar_takes_the_tooltip_down() {
    let (mut driver, studio) = shell();
    let anchor = studio.view_anchors.borrow()[0];
    let centre = Offset::new(
        (anchor.left + anchor.right) / 2.0,
        (anchor.top + anchor.bottom) / 2.0,
    );

    driver.handle_hover(Some(centre));
    driver.draw_frame_at(Duration::from_millis(16));
    assert!(studio.hovered_view.get().is_some());

    for frame in 2..6 {
        driver.handle_hover(Some(Offset::new(700.0, 400.0)));
        driver.draw_frame_at(Duration::from_millis(16 * frame));
        assert_eq!(
            studio.hovered_view.get(),
            None,
            "the tooltip came back with the pointer in the middle of the editor"
        );
    }
}
