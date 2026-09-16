//! A window's raw input, all the way to pixels moving.
//!
//! ```console
//! cargo test -p vieww-platform-winit --test window_to_gesture
//! ```
//!
//! # What this is for
//!
//! The unit tests in `input.rs` prove the translation is right in isolation, and
//! the tests above this crate prove the pipeline is right given a
//! [`PointerEvent`]. Neither proves the two are joined, and "joined" is the only
//! thing a platform bridge is *for*. This drives the whole of it — a window's
//! coordinates in, a scene with the card somewhere new out — with no window,
//! because everything that needed one has already been separated out.
//!
//! The one thing not covered here is the swapchain, which cannot be had without
//! a display. `examples/hello.rs` is where that is exercised, and it reports the
//! frame rate when it is.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use vieww_element::Signal;
use vieww_foundation::{Axis, Color, EdgeInsets, Offset, PointerButton, Rect, Size};
use vieww_paint::Damage;
use vieww_platform_winit::{Physical, PointerTranslator, Scale, TouchPhase};
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 200.0,
    height: 200.0,
};
const CARD: f32 = 40.0;
/// Where the card sits before anything touches it.
const HOME: Rect = Rect {
    left: 0.0,
    top: 0.0,
    right: CARD,
    bottom: CARD,
};

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A card inset from the left by however far it has been dragged.
#[derive(Debug)]
struct Card {
    offset: Signal<f32>,
}

impl Widget for Card {
    fn debug_name(&self) -> &'static str {
        "Card"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Padding::new(EdgeInsets::only(self.offset.get(), 0.0, 0.0, 0.0))
            .child(ColoredBox::new(Color::GREEN).child(SizedBox::square(CARD)))
            .into()
    }
}

widget_node_from!(Card);

/// A driver with a draggable card in it, and the drag total the handlers saw.
struct Harness {
    driver: FrameDriver,
    input: PointerTranslator,
    /// What the gesture layer reported, in logical pixels. Distinct from the
    /// card's position: this is what the *handler* was told.
    dragged: Rc<Cell<f32>>,
    taps: Rc<Cell<u32>>,
}

impl Harness {
    fn at_scale(scale: Scale) -> Self {
        let mut driver = FrameDriver::new(SURFACE);
        let offset = driver.elements().runtime().signal(0.0_f32);
        let dragged = Rc::new(Cell::new(0.0));
        let taps = Rc::new(Cell::new(0));

        let root = {
            let (moving, reported) = (offset.clone(), Rc::clone(&dragged));
            let tapped = Rc::clone(&taps);
            GestureDetector::new()
                .drag_axis(Axis::Horizontal)
                .on_tap(move |_| tapped.set(tapped.get() + 1))
                .on_drag_start(move |details| {
                    reported.set(reported.get() + details.delta.dx);
                    moving.update(|at| *at = (*at + details.delta.dx).max(0.0));
                })
                .on_drag_update({
                    let (moving, reported) = (offset.clone(), Rc::clone(&dragged));
                    move |details| {
                        reported.set(reported.get() + details.delta.dx);
                        moving.update(|at| *at = (*at + details.delta.dx).max(0.0));
                    }
                })
                .child(Card { offset })
        };

        // Inside a column aligned to the start of its cross axis, so the card
        // is 40x40 rather than stretched to the surface: the root's constraints
        // are tight, and a `ColoredBox` handed tight constraints fills them.
        let root = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(children![root]);

        driver.elements().set_root(root);
        driver.draw_frame();

        Self {
            driver,
            input: PointerTranslator::new(scale),
            dragged,
            taps,
        }
    }

    fn new() -> Self {
        Self::at_scale(Scale::ONE)
    }

    /// Feed one translated event in, exactly as the event loop does.
    fn feed(&mut self, event: Option<vieww_foundation::PointerEvent>) {
        if let Some(event) = event {
            self.driver.handle_pointer(&event);
        }
    }

    fn frame(&mut self, now: Duration) {
        self.driver.draw_frame_at(now);
    }

    /// Where the card is drawn, according to the scene the backend would get.
    fn card(&self) -> Rect {
        self.driver
            .scene()
            .fills()
            .into_iter()
            .find(|(_, paint)| paint.color == Color::GREEN)
            .map(|(bounds, _)| bounds)
            .expect("the card is always drawn")
    }
}

#[test]
fn a_window_drag_moves_the_card_it_started_on() {
    let mut harness = Harness::new();
    assert_eq!(harness.card(), HOME);

    // Exactly the sequence `winit` delivers: a position, a press, positions,
    // a release. Physical pixels throughout, because that is what a window
    // reports.
    let event = harness.input.cursor_moved((10.0, 20.0), ms(0));
    harness.feed(event);
    let event = harness
        .input
        .mouse_button(PointerButton::Primary, true, ms(10));
    harness.feed(event);

    for (step, x) in [30.0, 50.0, 70.0, 90.0].into_iter().enumerate() {
        let event = harness
            .input
            .cursor_moved((x, 20.0), ms(20 + step as u64 * 10));
        harness.feed(event);
    }

    let event = harness
        .input
        .mouse_button(PointerButton::Primary, false, ms(70));
    harness.feed(event);
    harness.frame(ms(80));

    assert!(
        harness.dragged.get() > 0.0,
        "the drag reported no movement at all"
    );
    let card = harness.card();
    assert!(
        card.left > HOME.left,
        "the card should have moved right, and is at {card:?}"
    );
    assert_eq!(
        card.width(),
        CARD,
        "a drag moves the card, it does not size it"
    );
    assert_eq!(
        harness.taps.get(),
        0,
        "a drag that travelled 80 pixels must not also fire a tap"
    );
}

#[test]
fn a_press_and_release_without_movement_is_a_tap() {
    let mut harness = Harness::new();

    let event = harness.input.cursor_moved((10.0, 10.0), ms(0));
    harness.feed(event);
    let event = harness
        .input
        .mouse_button(PointerButton::Primary, true, ms(10));
    harness.feed(event);
    let event = harness
        .input
        .mouse_button(PointerButton::Primary, false, ms(60));
    harness.feed(event);
    harness.frame(ms(70));

    assert_eq!(harness.taps.get(), 1);
    assert_eq!(harness.card(), HOME, "a tap does not move anything");
}

#[test]
fn the_same_gesture_on_a_hidpi_display_moves_the_card_the_same_distance() {
    let mut one = Harness::new();
    let mut three = Harness::at_scale(Scale::new(3.0));

    // The same *logical* gesture, reported by two different displays: the
    // 3x window reports every coordinate three times larger.
    for (harness, factor) in [(&mut one, 1.0), (&mut three, 3.0)] {
        let event = harness
            .input
            .cursor_moved((10.0 * factor, 20.0 * factor), ms(0));
        harness.feed(event);
        let event = harness
            .input
            .mouse_button(PointerButton::Primary, true, ms(10));
        harness.feed(event);
        for (step, x) in [30.0, 50.0, 70.0].into_iter().enumerate() {
            let event = harness
                .input
                .cursor_moved((x * factor, 20.0 * factor), ms(20 + step as u64 * 10));
            harness.feed(event);
        }
        let event = harness
            .input
            .mouse_button(PointerButton::Primary, false, ms(60));
        harness.feed(event);
        harness.frame(ms(70));
    }

    assert_eq!(
        one.card(),
        three.card(),
        "a denser display must not make a drag travel three times as far"
    );
    assert!(one.card().left > HOME.left, "and both actually moved");
}

#[test]
fn a_suspend_mid_drag_cancels_rather_than_completing_the_gesture() {
    let mut harness = Harness::new();

    let event = harness.input.cursor_moved((10.0, 10.0), ms(0));
    harness.feed(event);
    let event = harness
        .input
        .mouse_button(PointerButton::Primary, true, ms(10));
    harness.feed(event);

    // What `ApplicationHandler::suspended` does: every live pointer is told it
    // is gone, while the tree can still hear about it.
    for event in harness.input.cancel_all(ms(20)) {
        harness.driver.handle_pointer(&event);
    }
    harness.frame(ms(30));

    assert_eq!(
        harness.taps.get(),
        0,
        "a backgrounded application must not fire the tap the user never finished"
    );
    assert_eq!(
        harness.input.live_pointers(),
        0,
        "a pointer left live would hold its arena entry open forever"
    );
}

#[test]
fn a_touch_drives_the_same_pipeline_a_mouse_does() {
    let mut harness = Harness::new();

    let event = harness
        .input
        .touch(0, TouchPhase::Started, (10.0, 20.0), ms(0), None);
    harness.feed(event);
    for (step, x) in [40.0, 70.0, 100.0].into_iter().enumerate() {
        let event = harness.input.touch(
            0,
            TouchPhase::Moved,
            (x, 20.0),
            ms(10 + step as u64 * 10),
            None,
        );
        harness.feed(event);
    }
    let event = harness
        .input
        .touch(0, TouchPhase::Ended, (100.0, 20.0), ms(40), None);
    harness.feed(event);
    harness.frame(ms(50));

    assert!(
        harness.card().left > HOME.left,
        "a finger and a cursor reach the same render object or only one of them was ever tested"
    );
}

#[test]
fn what_the_backend_rasterises_is_the_frame_in_the_windows_own_pixels() {
    let mut harness = Harness::at_scale(Scale::new(2.0));

    let event = harness.input.cursor_moved((20.0, 40.0), ms(0));
    harness.feed(event);
    let event = harness
        .input
        .mouse_button(PointerButton::Primary, true, ms(10));
    harness.feed(event);
    let event = harness.input.cursor_moved((120.0, 40.0), ms(20));
    harness.feed(event);
    harness.frame(ms(30));

    let logical = harness.card();
    let surface = Rect::new(0.0, 0.0, SURFACE.width * 2.0, SURFACE.height * 2.0);
    let physical = Physical::new(
        harness.driver.scene(),
        harness.driver.damage(),
        Scale::new(2.0),
        surface,
    );

    let drawn = physical
        .scene()
        .fills()
        .into_iter()
        .find(|(_, paint)| paint.color == Color::GREEN)
        .map(|(bounds, _)| bounds)
        .expect("the card survives the conversion");

    // The scene is handed over *as described*, in logical pixels, and the
    // transform beside it is what puts it in the swapchain's. Rewriting the
    // scene is what this used to do, and it copied every command in the frame
    // on every display that was not exactly 1:1.
    assert_eq!(drawn, logical, "the frame is not rewritten");
    assert_eq!(
        physical.root().apply_rect(drawn),
        Rect::new(
            logical.left * 2.0,
            logical.top * 2.0,
            logical.right * 2.0,
            logical.bottom * 2.0
        ),
        "and what the 2x swapchain ends up rasterising is in its own pixels"
    );
    assert_eq!(physical.damage().surface(), surface);
}

#[test]
fn an_idle_frame_damages_nothing_so_the_window_presents_nothing() {
    let mut harness = Harness::new();

    // Two frames with no input between them. The first still carries the
    // startup damage; the second has nothing to say.
    harness.frame(ms(10));
    harness.frame(ms(20));

    assert!(
        harness.driver.damage().is_clean(),
        "an idle frame that reports damage makes an idle window cost a repaint"
    );

    let physical = Physical::new(
        harness.driver.scene(),
        harness.driver.damage(),
        Scale::ONE,
        Rect::from_origin_size(Offset::ZERO, SURFACE),
    );
    assert!(physical.damage().is_clean());
    assert_eq!(
        physical.damage().repaint_regions(),
        Damage::new(Rect::from_origin_size(Offset::ZERO, SURFACE)).repaint_regions(),
    );
}
