//! Paint order across a repaint boundary.
//!
//! ```console
//! cargo test -p vieww --test layer_paint_order
//! ```
//!
//! # Why this file exists
//!
//! Found by opening the menu in `examples/controls` after the page had been made
//! scrollable: the menu drew *under* the page it was covering. The two are
//! siblings in a `Stack`, the menu is second, and it still lost.
//!
//! The cause was in the compositor rather than in either widget.
//! `LayerTree::composite` put a layer's own recording down first and its child
//! layers on top of it — which is right for a layer *nested* inside the content
//! it covers, and wrong for one that is a sibling of content recorded after it.
//! A `Scrollable` is a repaint boundary, so its subtree went into a child layer;
//! everything painted after it stayed in the parent's recording; the child layer
//! was composited last and landed on top.
//!
//! # What fixed it
//!
//! Recording a boundary's subtree leaves a **hole** in its parent's command
//! list. `RenderTree::paint_boundary` now measures where that hole is, at the
//! one place it exists — `paint_into_layer`'s early return — and hands the index
//! straight to `LayerTree::set_slot` against the recording it was measured in.
//! `composite` interleaves: parent commands up to the slot, then the child, then
//! the rest.
//!
//! # A boundary inside a group is spliced into it, not left at the end
//!
//! An `Opacity` fades what its markers enclose *and* declares a `LayerEffect`,
//! so the fade reaches boundaries recorded elsewhere. A boundary inside the
//! group therefore meets the same declaration twice, and where it is placed
//! decides which half applies: outside the markers its own effect is right;
//! inside them the markers are, and its own would fade it a second time.
//!
//! It is spliced inside, and `composite` suppresses the duplicate. Inserting a
//! child's commands between two of its parent's cannot unbalance the stack —
//! the child's own composite is balanced — so the concern that argued for
//! leaving it at the end does not survive being looked at. Splicing is also the
//! better picture: the boundary and its faded siblings composite as *one* group,
//! which is what 50% opacity means to anyone who is not implementing it.

use vieww::foundation::{Color, Size};
use vieww::prelude::*;
use vieww::BuildContext;
use vieww_render::FrameDriver;

/// The scrolled page.
const UNDER: Color = Color::rgb(10, 20, 30);
/// The overlay that is supposed to cover it.
const OVER: Color = Color::rgb(200, 100, 50);
/// A third colour, for the cases that need to tell two overlays apart.
const OTHER: Color = Color::rgb(90, 160, 40);

/// The order the colours actually reach the scene in.
fn order(children: Vec<WidgetNode>) -> Vec<Color> {
    let mut driver = FrameDriver::new(Size::new(200.0, 200.0));
    driver.set_root(Stack::new().children(children));
    driver.draw_frame();
    colors(&driver)
}

/// The colours this file cares about, in the order the composited scene has them.
fn colors(driver: &FrameDriver) -> Vec<Color> {
    driver
        .scene()
        .fills()
        .into_iter()
        .map(|(_, paint)| paint.color)
        .filter(|color| [UNDER, OVER, OTHER].contains(color))
        .collect()
}

fn square(color: Color) -> WidgetNode {
    SizedBox::square(100.0).child(ColoredBox::new(color)).into()
}

/// A repaint boundary of its own — what a `Scrollable` is, without the physics.
fn scrolled(color: Color) -> WidgetNode {
    Scrollable::vertical(0.0).child(square(color)).into()
}

#[test]
fn an_overlay_covers_a_plain_sibling() {
    // The baseline, so that a failure below is read as being about layers
    // rather than about `Stack`.
    assert_eq!(
        order(vieww_widget::children![square(UNDER), square(OVER)]),
        vec![UNDER, OVER],
        "a later sibling paints over an earlier one"
    );
}

#[test]
fn an_overlay_covers_a_scrollable_sibling() {
    // The defect. The scrollable's subtree is a child layer; the overlay stays
    // in the parent's recording *after* the hole the scrollable left.
    assert_eq!(
        order(vieww_widget::children![scrolled(UNDER), square(OVER)]),
        vec![UNDER, OVER],
        "the scrollable is first in the tree, so it has to be first on the screen"
    );
}

#[test]
fn content_before_a_scrollable_still_goes_underneath_it() {
    // The other half, and the one a naive fix breaks: splicing the child in at
    // its slot must not drag it *below* what the parent painted first.
    assert_eq!(
        order(vieww_widget::children![square(OTHER), scrolled(UNDER)]),
        vec![OTHER, UNDER],
        "a boundary reached second composites above what was recorded first"
    );
}

#[test]
fn a_scrollable_lands_between_the_siblings_that_bracket_it() {
    assert_eq!(
        order(vieww_widget::children![
            square(OTHER),
            scrolled(UNDER),
            square(OVER)
        ]),
        vec![OTHER, UNDER, OVER],
        "both cuts at once — the parent's recording is split around the hole"
    );
}

#[test]
fn two_layers_keep_their_order_and_the_content_between_them() {
    // Two boundaries mean two holes, and the second slot is measured against a
    // recording the first has already been cut out of.
    assert_eq!(
        order(vieww_widget::children![
            scrolled(UNDER),
            square(OTHER),
            scrolled(OVER)
        ]),
        vec![UNDER, OTHER, OVER],
        "sibling boundaries interleave with the commands between them"
    );
}

#[test]
fn an_overlay_that_is_itself_a_layer_covers_a_scrollable() {
    // The old workaround. It has to keep working: `RepaintBoundary` around an
    // overlay is a reasonable thing to write for other reasons, and it now takes
    // the slot path rather than the child-order path.
    assert_eq!(
        order(vieww_widget::children![
            scrolled(UNDER),
            RepaintBoundary::new().child(square(OVER))
        ]),
        vec![UNDER, OVER],
        "two layers are still ordered against each other by the tree"
    );
}

#[test]
fn a_boundary_inside_a_faded_group_is_spliced_into_it() {
    // The case that decides whether the slot is taken inside an open group.
    //
    // `UNDER` is a boundary inside the `Opacity`; `OTHER` is an ordinary sibling
    // after it inside the same group; `OVER` is outside the group entirely.
    // Leaving the boundary at the end of its parent's recording — the shape this
    // started as — puts `UNDER` above `OVER`, which is the same defect this file
    // is about, one level in.
    assert_eq!(
        order(vieww_widget::children![
            Opacity::new(0.5).child(
                Stack::new().children(vieww_widget::children![scrolled(UNDER), square(OTHER)])
            ),
            square(OVER)
        ]),
        vec![UNDER, OTHER, OVER],
        "a boundary inside a group keeps its place in the group"
    );
}

#[test]
fn a_boundary_spliced_into_a_group_is_faded_once() {
    // The other half, and the one that makes the splice safe rather than merely
    // ordered: inside its parent's markers the child must not emit its own, or
    // the same 0.5 is applied twice and the subtree comes out at 0.25.
    let mut driver = FrameDriver::new(Size::new(200.0, 200.0));
    driver.set_root(Stack::new().children(vieww_widget::children![
        Opacity::new(0.5).child(Stack::new().children(vieww_widget::children![scrolled(UNDER)]))
    ]));
    driver.draw_frame();

    let groups = driver
        .scene()
        .commands()
        .iter()
        .filter(|command| matches!(command, vieww_paint::Command::PushLayer { .. }))
        .count();
    assert_eq!(
        groups, 1,
        "one declaration of opacity is one group, however many layers it spans"
    );
}

// --------------------------------------------------------------- second frames
//
// Everything above draws **one** frame, and a first frame is a full repaint by
// construction — `FrameDriver` starts with `full_repaint` set, because a surface
// that has never been drawn holds nothing to build on. So none of it says
// anything about damage.
//
// Damage is not a performance detail here. A presented surface skips
// repainting altogether when `Damage::is_clean`, so a perfectly composited scene
// whose frame reports nothing changed is **an unchanged window**. That is the
// gap the on-screen failure fell through: `examples/controls` still carries a
// `RepaintBoundary` around its menu because removing it drew nothing at all
// under Xvfb and lavapipe on 2026-08-16, at 78% CPU, while the eight tests above
// were green. Every one of them stops one frame before the frame that shows the
// overlay.

/// A scrolled page with an overlay that appears on demand and is **not** a
/// repaint boundary: `examples/controls` with the workaround taken out.
#[derive(Debug)]
struct Page {
    overlay: Signal<bool>,
    offset: Signal<f32>,
}

impl Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let page = Scrollable::vertical(self.offset.get()).child(square(UNDER));
        let mut stack = Stack::new().children(vieww_widget::children![page]);
        if self.overlay.get() {
            stack = stack.push(square(OVER));
        }
        stack.into()
    }
}

vieww::widget::widget_node_from!(Page);

/// A driver showing the page, with the signals that drive it.
fn page() -> (FrameDriver, Signal<bool>, Signal<f32>) {
    let mut driver = FrameDriver::new(Size::new(200.0, 200.0));
    let overlay = driver.elements().runtime().signal(false);
    let offset = driver.elements().runtime().signal(0.0_f32);
    driver.set_root(Page {
        overlay: overlay.clone(),
        offset: offset.clone(),
    });
    (driver, overlay, offset)
}

#[test]
fn the_frame_that_puts_an_overlay_up_damages_what_it_covers() {
    let (mut driver, overlay, _offset) = page();

    // Not an assertion about damage — this frame is the full repaint every first
    // frame is. It is here to give the next one a screen to differ from.
    driver.draw_frame();
    assert_eq!(
        colors(&driver),
        vec![UNDER],
        "the page alone, to start with"
    );

    overlay.set(true);
    driver.draw_frame();

    assert_eq!(
        colors(&driver),
        vec![UNDER, OVER],
        "the overlay composites above the page"
    );
    assert!(
        !driver.damage().is_clean(),
        "the overlay reached the scene and the frame reported nothing changed. \
         A backend that repaints only what is damaged draws none of it, and one \
         that skips presenting a clean frame leaves the previous frame on the \
         screen. Damage: {}",
        driver.damage()
    );
}

#[test]
fn an_overlay_stays_above_a_page_that_redraws_under_it() {
    let (mut driver, overlay, offset) = page();
    driver.draw_frame();
    overlay.set(true);
    driver.draw_frame();
    assert_eq!(colors(&driver), vec![UNDER, OVER], "the starting point");

    // The page changes and re-records; the overlay does not. A slot is an index
    // into a recording that is replaced wholesale on every repaint, so this is
    // the shape that goes wrong when one is kept a frame too long — an overlay
    // that drops behind its page on the frames the page happens to redraw.
    offset.set(40.0);
    driver.draw_frame();

    assert_eq!(
        colors(&driver),
        vec![UNDER, OVER],
        "the page redrew underneath and the overlay must still be above it"
    );
    assert!(
        !driver.damage().is_clean(),
        "the page scrolled and nothing was reported as changed: {}",
        driver.damage()
    );
}
