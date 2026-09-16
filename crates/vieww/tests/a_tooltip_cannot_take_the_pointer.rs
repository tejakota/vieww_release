//! A tooltip drawn *over* the control it explains still lets the control be hit.
//!
//! ```console
//! cargo test -p vieww --test a_tooltip_cannot_take_the_pointer
//! ```
//!
//! # Why this file exists
//!
//! Reported: the studio's activity-bar tooltip flickered at frame rate under a
//! pointer that was not moving. The tooltip is placed above the icon and clamped
//! downwards when it does not fit, so near the top of the column it landed on
//! the icon it was explaining. It has no gesture recogniser anywhere in it —
//! which was taken to mean it could not interfere — but its `DecoratedBox` and
//! `Text` are opaque to hit testing because they *draw*. The hover moved from
//! the icon to the tooltip, the icon was told the pointer had left, the tooltip
//! came down, the hover returned, and around again once per frame.
//!
//! The studio's own regression test (`apps/viewwstudio/tests/hover.rs`) pins the
//! symptom, but it can only see it while the card happens to overlap the icon —
//! which depends on the message, the font and where in the column the icon sits.
//! This pins the property instead, in the arrangement that produced the defect:
//! **a tooltip placed directly on top of a button does not take its input**.
//!
//! Delete the `IgnorePointer` in `ui::tooltip` and the two tests below fail; the
//! studio's do not necessarily, which is why both exist.
//!
//! # The other half of the same defect
//!
//! `Tooltip::height` measures **one line** and says so in its own doc comment:
//! a wrapped message is taller than the placement maths believes, and the card
//! overlaps its anchor by the difference. The studio was passing it two lines —
//! `"{title}\n{note}"` — so the overlap was not bad luck, it was arithmetic.
//! The tooltip now carries the name alone, which keeps it one line; this file
//! keeps the two-line arrangement, because the wrapper has to hold for a
//! message that wraps on its own.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{Offset, PointerEvent, PointerId, Rect, Size};
use vieww::prelude::*;
use vieww::BuildContext;
use vieww_render::FrameDriver;
use vieww_widget::{widget_node_from, Button, IgnorePointer, Tooltip};

const SURFACE: Size = Size::new(400.0, 300.0);
/// Where the button is, and — deliberately — where the tooltip is anchored, so
/// the card is drawn across the button rather than beside it.
const BUTTON: Rect = Rect {
    left: 40.0,
    top: 40.0,
    right: 200.0,
    bottom: 88.0,
};
/// A point inside the button, and inside the tooltip drawn over it.
const INSIDE: Offset = Offset { dx: 90.0, dy: 60.0 };

#[derive(Debug)]
struct Page {
    presses: Rc<Cell<u32>>,
    /// Whether the tooltip is wrapped the way the studio wraps it.
    ignoring: bool,
}

impl Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let presses = Rc::clone(&self.presses);
        let button = Positioned::new()
            .left(BUTTON.left)
            .top(BUTTON.top)
            .width(BUTTON.width())
            .height(BUTTON.height())
            .child(Button::new("Explorer").on_pressed(move || {
                presses.set(presses.get() + 1);
            }));

        // **Two lines, anchored near the top of the surface** — the studio's
        // exact arrangement before the fix. `Tooltip::height` measures one line
        // by its own admission ("a wrapped tooltip is taller than this says and
        // will overlap its anchor by the difference"), and placement clamps the
        // card to the top edge when it does not fit above. The card therefore
        // starts above the button and runs down across it.
        let card = Tooltip::new(
            "Explorer\nWhat the workspace holds, the files it is watching, and the \
             buffers open right now.",
        )
        .anchor(BUTTON);
        let card: WidgetNode = if self.ignoring {
            IgnorePointer::new().child(card).into()
        } else {
            card.into()
        };

        Stack::new().children(children![button, card]).into()
    }
}

widget_node_from!(Page);

fn app(ignoring: bool) -> (FrameDriver, Rc<Cell<u32>>) {
    let presses = Rc::new(Cell::new(0));
    let mut driver = FrameDriver::new(SURFACE);
    driver.set_root(Page {
        presses: Rc::clone(&presses),
        ignoring,
    });
    driver.draw_frame();
    (driver, presses)
}

fn tap(driver: &mut FrameDriver, at: Offset) {
    driver.handle_pointer(&PointerEvent::down(PointerId(1), at, Duration::ZERO));
    driver.draw_frame_at(Duration::from_millis(0));
    driver.draw_frame_at(Duration::from_millis(110));
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        at,
        Duration::from_millis(120),
    ));
    driver.draw_frame_at(Duration::from_millis(130));
}

#[test]
fn a_click_lands_on_the_button_under_the_tooltip() {
    let (mut driver, presses) = app(true);
    tap(&mut driver, INSIDE);
    assert_eq!(
        presses.get(),
        1,
        "the tooltip drawn over the button swallowed its click"
    );
}

#[test]
fn the_hover_stays_on_the_button_under_the_tooltip() {
    let (mut driver, _) = app(true);
    driver.handle_hover(Some(INSIDE));
    driver.draw_frame_at(Duration::from_millis(16));

    let names: Vec<&str> = driver
        .hit_test(INSIDE)
        .entries()
        .iter()
        .filter_map(|entry| {
            driver
                .owner()
                .tree()
                .object(entry.id)
                .map(vieww_render::RenderObject::debug_name)
        })
        .collect();

    assert!(
        names.contains(&"RenderGestureDetector"),
        "the pointer reached the tooltip instead of the button: {names:?}"
    );
}

/// The same arrangement without the wrapper, so the test above is known to be
/// measuring the wrapper rather than the geometry. This is the defect, pinned:
/// if this ever *stops* swallowing the click, the two tests above have become
/// tautologies and this file should be re-examined.
#[test]
fn without_the_wrapper_the_tooltip_takes_the_click() {
    let (mut driver, presses) = app(false);
    tap(&mut driver, INSIDE);
    assert_eq!(
        presses.get(),
        0,
        "an unwrapped tooltip over a button no longer intercepts input — the \
         hit-test rules changed, and `IgnorePointer` may no longer be what \
         keeps the studio's tooltip out of the way"
    );
}
