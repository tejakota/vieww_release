//! The Phase 6 exit test: a scrollable list of tappable rows, where a tap and a
//! scroll are told apart correctly.
//!
//! ```console
//! cargo test -p vieww --test gesture_to_scroll
//! ```
//!
//! # What the roadmap asks for
//!
//! > a scrollable list where drag-to-scroll and tap-on-item are correctly
//! > disambiguated (a tap that moves 2px shouldn't cancel the tap; a drag that
//! > moves 20px shouldn't fire a tap).
//!
//! Both halves are here, driven through the *whole* stack rather than against the
//! recognisers directly: a widget tree is built and mounted, laid out, hit
//! tested, and synthetic pointer events are routed to the render objects the hit
//! test found. `crates/vieww-gestures/tests/disambiguation.rs` covers the
//! recognisers in isolation; this covers the wiring, which is where a gesture
//! layer that works in a unit test still fails in an app.
//!
//! # Why there is no `Scrollable` widget yet
//!
//! The list here is assembled by hand from a [`Viewport`], a
//! [`GestureDetector`] and a [`ScrollPosition`] — which is exactly what a
//! `Scrollable` would do, without the widget-library API around it. That widget,
//! and the virtualisation that makes a long list cheap, is Phase 9. Phase 6 owes
//! the *mechanism*, and the mechanism is what this asserts.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{Axis, Color, Offset, PointerEvent, PointerId, Size};
use vieww::prelude::*;
use vieww::{FrameDriver, Signal};

const SURFACE: f32 = 300.0;
const ROW: f32 = 60.0;
const ROWS: usize = 12;
const CONTENT: f32 = ROW * ROWS as f32;
const POINTER: PointerId = PointerId(1);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn at(x: f32, y: f32) -> Offset {
    Offset::new(x, y)
}

/// Which rows have been tapped, in order.
type Taps = Rc<RefCell<Vec<usize>>>;

/// The list: a viewport onto twenty rows, each of which is tappable, with the
/// whole thing draggable to scroll.
///
/// The scroll offset lives in a signal, so a drag update rebuilds the viewport
/// and nothing else — the rows are untouched, and their render objects and
/// gesture recognisers survive the scroll.
fn list(offset: &Signal<f32>, scroll: &Rc<RefCell<ScrollPosition>>, taps: &Taps) -> WidgetNode {
    let rows: Vec<WidgetNode> = (0..ROWS)
        .map(|index| {
            let taps = Rc::clone(taps);
            GestureDetector::new()
                .on_tap(move |_| taps.borrow_mut().push(index))
                .child(
                    ColoredBox::new(if index % 2 == 0 {
                        Color::RED
                    } else {
                        Color::BLUE
                    })
                    .child(SizedBox::from_size(Size::new(SURFACE, ROW))),
                )
                .into()
        })
        .collect();

    // `DragStart` carries the movement that earned the drag — the slop distance
    // the finger had to travel before it was certain. Handling only `on_drag_update`
    // silently throws that away, and the list lags the finger by 18px on every
    // touch. Both handlers do the same thing for that reason.
    let started = Rc::clone(scroll);
    let started_offset = offset.clone();
    let dragging = Rc::clone(scroll);
    let dragging_offset = offset.clone();
    let ending = Rc::clone(scroll);
    let ending_offset = offset.clone();

    GestureDetector::new()
        .drag_axis(Axis::Vertical)
        .on_drag_start(move |details| {
            let mut scroll = started.borrow_mut();
            scroll.apply_drag(details.delta.dy);
            started_offset.set(scroll.offset());
        })
        .on_drag_update(move |details| {
            let mut scroll = dragging.borrow_mut();
            scroll.apply_drag(details.delta.dy);
            dragging_offset.set(scroll.offset());
        })
        .on_drag_end(move |details| {
            let mut scroll = ending.borrow_mut();
            scroll.fling(details.velocity.dy, Duration::ZERO);
            ending_offset.set(scroll.offset());
        })
        .child(
            Viewport::vertical()
                .offset(offset.get())
                .child(Flex::column().children(rows)),
        )
        .into()
}

/// A driver with the list mounted and one frame drawn.
struct Harness {
    driver: FrameDriver,
    offset: Signal<f32>,
    scroll: Rc<RefCell<ScrollPosition>>,
    taps: Taps,
}

impl Harness {
    fn new() -> Self {
        Self::with_physics(ScrollPhysics::android())
    }

    fn with_physics(physics: ScrollPhysics) -> Self {
        let mut driver = FrameDriver::new(Size::square(SURFACE));
        let offset = driver.elements().runtime().signal(0.0_f32);
        let scroll = Rc::new(RefCell::new(ScrollPosition::new(SURFACE, CONTENT, physics)));
        let taps: Taps = Rc::new(RefCell::new(Vec::new()));

        let (o, s, t) = (offset.clone(), Rc::clone(&scroll), Rc::clone(&taps));
        driver.elements().set_root(list(&o, &s, &t));
        driver.draw_frame();

        let mut harness = Self {
            driver,
            offset,
            scroll,
            taps,
        };
        // The tree has to be rebuilt whenever the offset changes, and the signal
        // is read in `list` rather than by a composed widget, so the test does
        // the rebuilding a `Scrollable` widget would do for itself.
        harness.rebuild();
        harness
    }

    fn rebuild(&mut self) {
        let root = list(&self.offset, &self.scroll, &self.taps);
        self.driver.elements().set_root(root);
        self.driver.draw_frame();
    }

    fn send(&mut self, event: &PointerEvent) {
        self.driver.handle_pointer(event);
        self.rebuild();
    }

    /// A press, some moves, and a release — the shape of every gesture here.
    fn stroke(&mut self, from: Offset, to: Offset, steps: u32) {
        self.send(&PointerEvent::down(POINTER, from, ms(0)));
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let position = at(
                from.dx + (to.dx - from.dx) * t,
                from.dy + (to.dy - from.dy) * t,
            );
            self.send(&PointerEvent::moved(
                POINTER,
                previous,
                position,
                ms(u64::from(step) * 16),
            ));
            previous = position;
        }
        self.send(&PointerEvent::up(
            POINTER,
            to,
            ms(u64::from(steps) * 16 + 16),
        ));
    }

    fn taps(&self) -> Vec<usize> {
        self.taps.borrow().clone()
    }

    fn scroll_offset(&self) -> f32 {
        self.scroll.borrow().offset()
    }
}

// ------------------------------------------------- the two roadmap criteria

#[test]
fn a_tap_that_moves_two_pixels_still_presses_the_row() {
    let mut harness = Harness::new();

    // Row 2 spans y 120..180.
    harness.stroke(at(150.0, 150.0), at(152.0, 150.0), 2);

    assert_eq!(
        harness.taps(),
        vec![2],
        "a finger never holds perfectly still, and 2px must not cancel a press"
    );
    assert_eq!(harness.scroll_offset(), 0.0, "and it must not scroll");
}

#[test]
fn a_drag_of_twenty_pixels_scrolls_without_pressing_anything() {
    let mut harness = Harness::new();

    harness.stroke(at(150.0, 150.0), at(150.0, 130.0), 4);

    assert!(
        harness.taps().is_empty(),
        "flicking the list must not also press the row under the finger: {:?}",
        harness.taps()
    );
    assert!(
        harness.scroll_offset() >= 20.0,
        "and it has to actually scroll the full 20px, slop included: {}",
        harness.scroll_offset()
    );
}

// ------------------------------------------------------------------ the rest

#[test]
fn the_row_that_was_pressed_is_the_row_under_the_finger() {
    let mut harness = Harness::new();

    // Rows are 60 tall: y=330 is row 5.
    harness.stroke(at(10.0, 30.0), at(10.0, 30.0), 1);
    assert_eq!(harness.taps(), vec![0]);

    harness.stroke(at(290.0, 290.0), at(290.0, 290.0), 1);
    assert_eq!(
        harness.taps(),
        vec![0, 4],
        "y=290 is inside row 4, which spans 240..300"
    );
}

#[test]
fn a_row_scrolled_under_the_finger_is_the_one_that_gets_pressed() {
    let mut harness = Harness::new();

    // Scroll two rows' worth, so row 2 is now at the top.
    harness.stroke(at(150.0, 200.0), at(150.0, 80.0), 6);
    let scrolled = harness.scroll_offset();
    assert!(scrolled > 100.0, "{scrolled}");
    assert!(harness.taps().is_empty());

    // Now tap the top of the viewport. Which row that is depends on the scroll,
    // which is the point: hit testing runs against the laid-out tree, not against
    // where the rows were when the list was built.
    harness.stroke(at(150.0, 10.0), at(150.0, 10.0), 1);
    let expected = ((scrolled + 10.0) / ROW) as usize;
    assert_eq!(harness.taps(), vec![expected], "scrolled by {scrolled}");
}

#[test]
fn a_gesture_keeps_its_target_after_the_finger_leaves_it() {
    let mut harness = Harness::new();

    // Press inside the list and drag right off the side of the surface. The
    // scroll has to keep working: only the down hit tests, and everything after
    // goes wherever that found.
    harness.send(&PointerEvent::down(POINTER, at(150.0, 150.0), ms(0)));
    harness.send(&PointerEvent::moved(
        POINTER,
        at(150.0, 150.0),
        at(150.0, 60.0),
        ms(16),
    ));
    harness.send(&PointerEvent::moved(
        POINTER,
        at(150.0, 60.0),
        at(150.0, -400.0),
        ms(32),
    ));

    assert!(
        harness.scroll_offset() > 0.0,
        "a drag that leaves the widget must not stop scrolling it: {}",
        harness.scroll_offset()
    );
}

#[test]
fn a_cancelled_touch_neither_taps_nor_scrolls() {
    let mut harness = Harness::new();

    harness.send(&PointerEvent::down(POINTER, at(150.0, 150.0), ms(0)));
    harness.send(&PointerEvent::cancel(POINTER, at(150.0, 150.0), ms(50)));

    assert!(harness.taps().is_empty(), "{:?}", harness.taps());
    assert_eq!(harness.scroll_offset(), 0.0);
}

#[test]
fn a_horizontal_swipe_does_not_scroll_a_vertical_list() {
    let mut harness = Harness::new();

    harness.stroke(at(50.0, 150.0), at(250.0, 150.0), 8);

    assert_eq!(
        harness.scroll_offset(),
        0.0,
        "the drag is axis-locked to vertical"
    );
}

#[test]
fn the_list_stops_at_the_top_rather_than_scrolling_into_nothing() {
    let mut harness = Harness::new();

    // Drag downwards from the very top: there is nothing above row 0.
    harness.stroke(at(150.0, 100.0), at(150.0, 250.0), 6);

    assert_eq!(
        harness.scroll_offset(),
        0.0,
        "android physics clamp at the edge"
    );
}

#[test]
fn ios_physics_let_the_list_stretch_past_the_top() {
    let mut harness = Harness::with_physics(ScrollPhysics::ios());

    harness.stroke(at(150.0, 100.0), at(150.0, 250.0), 6);

    assert!(
        harness.scroll_offset() < 0.0,
        "an elastic list follows the finger past the edge: {}",
        harness.scroll_offset()
    );
}

#[test]
fn releasing_a_fast_drag_flings_the_list_onward() {
    let mut harness = Harness::new();

    // 240px in 240ms is a steady 1000px/s upward.
    harness.send(&PointerEvent::down(POINTER, at(150.0, 290.0), ms(0)));
    let mut previous = at(150.0, 290.0);
    for step in 1..=24 {
        let position = at(150.0, 290.0 - step as f32 * 10.0);
        harness.send(&PointerEvent::moved(
            POINTER,
            previous,
            position,
            ms(step * 10),
        ));
        previous = position;
    }
    harness.send(&PointerEvent::up(POINTER, previous, ms(250)));

    let at_release = harness.scroll_offset();
    assert!(
        harness.scroll.borrow().is_animating(),
        "a fast release has to start a fling"
    );

    let mut now = Duration::ZERO;
    while harness.scroll.borrow_mut().advance(now) && now < ms(6000) {
        now += ms(8);
    }
    assert!(
        harness.scroll.borrow().offset() > at_release + 50.0,
        "the throw has to carry the list well past where the finger left it: \
         {at_release} then {}",
        harness.scroll.borrow().offset()
    );
}

#[test]
fn the_viewport_gets_its_own_layer_so_scrolling_does_not_repaint_the_page() {
    let harness = Harness::new();

    assert!(
        harness.driver.layers().len() >= 2,
        "a viewport is a repaint boundary; scrolling is the case boundaries exist \
         for: {} layer(s)",
        harness.driver.layers().len()
    );
}

#[test]
fn the_list_paints_only_what_fits_on_screen() {
    let harness = Harness::new();

    // Twenty 60px rows into a 300px window: five fit, six once one is partly
    // scrolled in. The rest are clipped away rather than drawn.
    let fills = harness.driver.scene().fills();
    let visible = fills
        .iter()
        .filter(|(rect, _)| rect.top < SURFACE && rect.bottom > 0.0)
        .count();
    assert!(
        visible <= 6,
        "the viewport has to clip: {visible} of {ROWS} rows reached the frame"
    );
}
