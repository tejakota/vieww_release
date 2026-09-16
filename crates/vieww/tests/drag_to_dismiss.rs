//! The Phase 7 exit test: a card dragged sideways and released, which either
//! springs home or is thrown off the screen — while an unrelated part of the
//! tree rebuilds on every one of those frames.
//!
//! ```console
//! cargo test -p vieww --test drag_to_dismiss
//! ```
//!
//! # What the roadmap asks for
//!
//! > a spring-animated drag-to-dismiss card that feels physically correct (not
//! > linear) and runs at 60fps while a rebuild happens elsewhere in the tree
//! > simultaneously.
//!
//! Three claims, and each is asserted rather than described:
//!
//! - **spring-animated**: the release hands the gesture's velocity to a
//!   `Spring`, and how long the motion takes is the physics' answer rather than
//!   a duration anybody picked.
//! - **physically correct**: the card accelerates from rest, reaches its
//!   greatest speed somewhere in the middle, decelerates into its target and
//!   does not overshoot. A linear tween fails every one of those.
//! - **60fps with a concurrent rebuild**: a sibling subtree is rebuilt on every
//!   frame of the animation, and the frames are timed against a real clock and
//!   a 60Hz budget.
//!
//! # Why the card moves with padding
//!
//! There is no transform or opacity render object yet — Phase 4 built the layer
//! tree, not a full property set — so "move the card" is a left inset that
//! changes. That is a layout change rather than a paint one, which makes the
//! test *harder* than a transform would: every frame re-lays-out the row the
//! card sits in, and the assertions about layer re-recording below still hold.

use std::rc::Rc;
use std::time::{Duration, Instant};

use vieww::foundation::{Axis, Color, EdgeInsets, Offset, PointerEvent, PointerId, Size};
use vieww::prelude::*;
use vieww::{Animation, FrameDriver, FrameScheduler, Spring};

const SURFACE: f32 = 300.0;
const CARD: f32 = 60.0;
/// Past this fraction of the screen, letting go dismisses rather than returns.
const DISMISS_AT: f32 = 0.4;
/// A flick faster than this dismisses however short it was, in pixels a second.
const FLICK: f32 = 600.0;
const POINTER: PointerId = PointerId(1);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn at(x: f32, y: f32) -> Offset {
    Offset::new(x, y)
}

/// A band whose colour comes from a signal, standing in for the rest of an
/// application: something that rebuilds while the card is moving and has nothing
/// to do with it.
#[derive(Debug)]
struct Ticker {
    frame: Signal<u32>,
}

impl Widget for Ticker {
    fn debug_name(&self) -> &'static str {
        "Ticker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Reading the signal here is what subscribes this element to it, so
        // bumping the frame counter rebuilds exactly this and nothing else.
        let step = (self.frame.get() % 255) as u8;
        RepaintBoundary::new()
            .child(
                ColoredBox::new(Color::rgb(step, 0, 255 - step))
                    .child(SizedBox::from_size(Size::new(SURFACE, 40.0))),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Ticker);

/// The card itself: a fixed subtree behind a repaint boundary, inset from the
/// left by however far the dismissal has got.
///
/// Composed, and reading the offset signal in its own `build`, which is the
/// whole point: that read subscribes *this* element, so the animate phase's
/// write marks this element and nothing else, and the same frame's build shows
/// the new position. A test harness that re-supplied the whole tree each frame
/// instead would work — and would prove nothing about the cost of a frame.
#[derive(Debug)]
struct Card {
    offset: Signal<f32>,
    /// Built once and cloned, so reconciliation skips the subtree by pointer
    /// equality and the boundary's layer is never re-recorded — the card
    /// *moves*, and moving is a composite, not a repaint.
    content: WidgetNode,
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
            .child(self.content.clone())
            .into()
    }
}

vieww::widget::widget_node_from!(Card);

/// The screen: the ticker band, and under it the card at its current offset.
fn screen(frame: &Signal<u32>, offset: &Signal<f32>, handlers: &Handlers) -> WidgetNode {
    let (start, update, end) = (
        Rc::clone(&handlers.start),
        Rc::clone(&handlers.update),
        Rc::clone(&handlers.end),
    );

    let card = Card {
        offset: offset.clone(),
        content: RepaintBoundary::new()
            .child(ColoredBox::new(Color::GREEN).child(SizedBox::square(CARD)))
            .into(),
    };

    // Left-aligned, not centred: a `Flex` centres on its cross axis by default,
    // which would halve the card's travel and put it at x=120 to start with.
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(children![
            Ticker {
                frame: frame.clone()
            },
            GestureDetector::new()
                .drag_axis(Axis::Horizontal)
                .on_drag_start(move |details| start(details.delta.dx, 0.0, details.timestamp))
                .on_drag_update(move |details| update(details.delta.dx, 0.0, details.timestamp))
                .on_drag_end(move |details| end(0.0, details.velocity.dx, details.timestamp))
                .child(card),
        ])
        .into()
}

/// The three drag handlers, boxed so the widget tree can be rebuilt without
/// rebuilding them.
struct Handlers {
    start: Rc<dyn Fn(f32, f32, Duration)>,
    update: Rc<dyn Fn(f32, f32, Duration)>,
    end: Rc<dyn Fn(f32, f32, Duration)>,
}

struct Harness {
    driver: FrameDriver,
    scheduler: FrameScheduler,
    /// The card's left inset, in pixels. Written by the animation, read by the
    /// tree.
    offset: Signal<f32>,
    /// How far along the dismissal the card is: 0 at home, 1 off the edge.
    slide: Animation<f32>,
    frame: Signal<u32>,
    handlers: Rc<Handlers>,
    /// Vsync timestamps handed to the driver, so the test's notion of time and
    /// the animation's are the same one.
    now: Duration,
}

impl Harness {
    fn new() -> Self {
        let mut driver = FrameDriver::new(Size::square(SURFACE));

        // 0 is home, 1 is off the right-hand edge. Everything about the
        // dismissal is expressed in this one number, which is what lets a drag
        // and a spring drive the same motion.
        let slide = driver.animation(Tween::new(0.0_f32, SURFACE), ms(250));
        let offset = slide.signal();
        let frame = driver.elements().runtime().signal(0_u32);

        let dragging = slide.clone();
        let start_of_drag = slide.clone();
        let released = slide.clone();
        let handlers = Rc::new(Handlers {
            start: Rc::new(move |dx, _, _| {
                // While the finger is down it *is* the animation: whatever the
                // spring was doing yields to it rather than fighting it.
                start_of_drag.set_progress((start_of_drag.progress() + dx / SURFACE).max(0.0));
            }),
            update: Rc::new(move |dx, _, _| {
                dragging.set_progress((dragging.progress() + dx / SURFACE).max(0.0));
            }),
            end: Rc::new(move |_, velocity, now| {
                // Distance *or* speed: a card flicked hard from near home is
                // dismissed, and one dragged slowly most of the way across and
                // let go is too. Anything else comes back.
                let progress = released.progress();
                let dismissed = progress > DISMISS_AT || velocity > FLICK;
                let target = if dismissed { 1.0 } else { 0.0 };
                // The gesture reports pixels a second; the animation is in units
                // of the whole screen, so the velocity has to be scaled into the
                // same units or the spring is launched 300 times too hard.
                released.animate_with(Spring::settling(progress, target, velocity / SURFACE), now);
            }),
        });

        let mut harness = Self {
            driver,
            scheduler: FrameScheduler::sixty_hz(),
            offset,
            slide,
            frame,
            handlers,
            now: Duration::ZERO,
        };
        // Mounted once. Everything after this reaches the tree through signals,
        // which is what makes the per-frame cost proportional to what moved.
        let root = screen(&harness.frame, &harness.offset, &harness.handlers);
        harness.driver.elements().set_root(root);
        harness.frame_at(ms(0), || ms(0));
        harness
    }

    /// One vsync, with the unrelated subtree changing on it.
    ///
    /// `elapsed` is the cost clock the scheduler measures phases against, kept
    /// separate from the vsync timestamp exactly as `FrameScheduler::pulse`
    /// intends: one is when the frame is *for*, the other is what it *cost*.
    fn frame_at(&mut self, now: Duration, elapsed: impl FnMut() -> Duration) {
        self.now = now;
        // The concurrent rebuild: something else on screen changing on the same
        // frames the card is animating on.
        self.frame.update(|frame| *frame += 1);
        self.scheduler.request_frame();
        self.driver.drive(&mut self.scheduler, now, elapsed);
    }

    /// Run frames at 60Hz until nothing is animating, collecting where the card
    /// was on each one.
    fn run_until_still(&mut self) -> Vec<f32> {
        let start = Instant::now();
        let mut positions = vec![self.card_x()];
        while self.driver.is_animating() {
            let now = self.now + ms(16);
            self.frame_at(now, || start.elapsed());
            positions.push(self.card_x());
            assert!(
                positions.len() < 600,
                "the animation never settled: {positions:?}"
            );
        }
        positions
    }

    /// How far the card has been moved, in pixels.
    fn card_x(&self) -> f32 {
        self.offset.peek()
    }

    /// Where the card actually is in the painted frame, or `None` once it has
    /// left the screen — which is what being dismissed looks like from the
    /// outside.
    fn painted_x(&self) -> Option<f32> {
        self.driver
            .scene()
            .fills()
            .iter()
            .find(|(rect, paint)| paint.color == Color::GREEN && rect.width() > 0.0)
            .map(|(rect, _)| rect.left)
    }

    fn send(&mut self, event: &PointerEvent) {
        self.driver.handle_pointer(event);
        let now = self.now + ms(16);
        self.frame_at(now, || ms(0));
    }

    /// A press, a series of moves and a release, over `duration`.
    fn swipe(&mut self, from: Offset, to: Offset, steps: u32, duration: Duration) {
        let base = self.now;
        self.send(&PointerEvent::down(POINTER, from, base));
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
                base + duration.mul_f32(t),
            ));
            previous = position;
        }
        self.send(&PointerEvent::up(POINTER, to, base + duration));
    }

    /// How many times each layer has been re-recorded, root first and then its
    /// children in composite order: the page, the ticker band, the card.
    fn paint_counts(&self) -> Vec<u32> {
        fn collect(layers: &vieww::LayerTree, id: vieww::LayerId, out: &mut Vec<u32>) {
            out.push(layers.layer(id).paint_count());
            for &child in layers.layer(id).children() {
                collect(layers, child, out);
            }
        }

        let layers = self.driver.layers();
        let mut counts = Vec::new();
        if let Some(root) = layers.root() {
            collect(layers, root, &mut counts);
        }
        counts
    }
}

// ------------------------------------------------------------ dragging itself

#[test]
fn the_card_follows_the_finger() {
    let mut harness = Harness::new();
    assert_eq!(harness.card_x(), 0.0);

    // The card row starts below the 40px band.
    harness.swipe(at(30.0, 70.0), at(110.0, 70.0), 8, ms(200));

    assert!(
        harness.card_x() > 60.0,
        "the card has to travel with the finger, slop included: {}",
        harness.card_x()
    );
    assert_eq!(
        harness.painted_x(),
        Some(harness.card_x()),
        "and the pixels have to be where the offset says, or every assertion \
         below is about a number nobody can see"
    );
}

// ------------------------------------------------------ the spring, and its feel

#[test]
fn a_short_drag_released_springs_the_card_home() {
    let mut harness = Harness::new();

    // A fifth of the way across, slowly enough not to count as a flick.
    harness.swipe(at(30.0, 70.0), at(85.0, 70.0), 10, ms(400));
    assert!(harness.card_x() > 30.0, "{}", harness.card_x());
    assert!(
        harness.driver.is_animating(),
        "letting go has to start the spring"
    );

    let positions = harness.run_until_still();

    assert_eq!(harness.card_x(), 0.0, "and it arrives exactly home");
    assert!(
        positions.len() > 8,
        "a settle that resolves in three frames is a jump, not a spring: {} frames",
        positions.len()
    );
}

#[test]
fn the_return_is_a_spring_rather_than_a_linear_slide() {
    let mut harness = Harness::new();
    harness.swipe(at(30.0, 70.0), at(90.0, 70.0), 10, ms(400));
    let positions = harness.run_until_still();

    // Frame-to-frame movement *homeward*: a linear tween's is constant, a
    // spring's builds from rest and then decays. Measured from the furthest
    // point, because a release still carrying the finger's velocity legitimately
    // travels away from home for a frame or two first.
    let furthest = positions
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(index, _)| index)
        .expect("the card moved");
    let steps: Vec<f32> = positions[furthest..]
        .windows(2)
        .map(|p| p[0] - p[1])
        .collect();
    let fastest = steps
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(index, _)| index)
        .expect("the card moved");

    assert!(
        fastest > 0 && fastest < steps.len() - 1,
        "a spring released from rest is at its fastest somewhere in the middle; \
         a linear slide is fastest on frame one and every frame after: {steps:?}"
    );
    assert!(
        steps.last().copied().expect("frames") < steps[fastest] * 0.5,
        "and it must visibly decelerate into its target: {steps:?}"
    );
    assert!(
        positions.iter().all(|&x| x >= -0.01),
        "critically damped means it must not shoot past home and come back: \
         {positions:?}"
    );
}

#[test]
fn a_drag_past_the_threshold_dismisses_the_card_off_the_screen() {
    let mut harness = Harness::new();

    // Most of the way across, slowly: distance alone is enough.
    harness.swipe(at(20.0, 70.0), at(200.0, 70.0), 16, ms(600));
    let positions = harness.run_until_still();

    assert_eq!(
        harness.card_x(),
        SURFACE,
        "a dismissed card carries on off the edge rather than returning: \
         {positions:?}"
    );
    assert!(
        harness.painted_x().is_none(),
        "and it is gone from the painted frame, not merely far right"
    );
    assert!(
        positions.windows(2).all(|p| p[1] >= p[0] - 0.01),
        "and goes there without doubling back: {positions:?}"
    );
}

#[test]
fn a_fast_flick_dismisses_a_card_that_barely_moved() {
    let mut harness = Harness::new();

    // 60px in 50ms is 1200px/s — twice the flick threshold, but only a fifth of
    // the way across, so distance alone would bring it home.
    harness.swipe(at(20.0, 70.0), at(80.0, 70.0), 4, ms(50));
    harness.run_until_still();

    assert_eq!(
        harness.card_x(),
        SURFACE,
        "the speed of a gesture is part of what it meant, not noise on top of \
         where it ended"
    );
}

#[test]
fn a_release_carries_the_gestures_velocity_into_the_spring() {
    let mut slow = Harness::new();
    slow.swipe(at(20.0, 70.0), at(180.0, 70.0), 12, ms(600));
    let slow_frames = slow.run_until_still().len();

    let mut fast = Harness::new();
    fast.swipe(at(20.0, 70.0), at(180.0, 70.0), 12, ms(120));
    let fast_frames = fast.run_until_still().len();

    assert_eq!(slow.card_x(), SURFACE);
    assert_eq!(fast.card_x(), SURFACE);
    assert!(
        fast_frames < slow_frames,
        "the same swipe thrown harder has to finish sooner, or the release \
         velocity is being dropped: {fast_frames} frames vs {slow_frames}"
    );
}

// --------------------------------------- 60fps, with the rest of the tree busy

#[test]
fn the_animation_runs_at_sixty_frames_a_second_while_something_else_rebuilds() {
    let mut harness = Harness::new();
    let builds_before = harness
        .driver
        .elements()
        .find("Ticker")
        .expect("mounted")
        .build_count();

    harness.swipe(at(20.0, 70.0), at(90.0, 70.0), 8, ms(300));
    let started = Instant::now();
    let frames = harness.run_until_still().len();
    let taken = started.elapsed();

    let builds_after = harness
        .driver
        .elements()
        .find("Ticker")
        .expect("mounted")
        .build_count();
    assert!(
        builds_after >= builds_before + frames as u32 - 1,
        "the unrelated subtree has to have rebuilt on every frame of the \
         animation, or this proves nothing about doing both at once: \
         {builds_before} then {builds_after} over {frames} frames"
    );

    // Real wall time against a real 60Hz budget. There is roughly two orders of
    // magnitude of headroom here on a development machine — 0.12ms a frame
    // against 16.67ms — so this is not a flaky timing assertion, it is a
    // regression alarm for the day a frame starts costing what the screen costs
    // rather than what changed.
    let budget = harness.scheduler.budget() * frames as u32;
    assert!(
        taken < budget,
        "{frames} frames of animation with a concurrent rebuild took {taken:?}, \
         which is over the {budget:?} that 60fps allows"
    );
}

#[test]
fn a_sliding_card_does_not_re_record_its_own_layer_or_the_page() {
    let mut harness = Harness::new();
    harness.swipe(at(20.0, 70.0), at(90.0, 70.0), 8, ms(300));

    let before = harness.paint_counts();
    assert_eq!(before.len(), 3, "the page, the ticker band, and the card");
    harness.run_until_still();
    let after = harness.paint_counts();

    assert_eq!(
        after[2], before[2],
        "a card that only *moves* re-records nothing: its layer is composited \
         at a new offset, which is the entire reason repaint boundaries exist. \
         {before:?} then {after:?}"
    );
    assert!(
        after[1] > before[1],
        "while the band that really did change its colour re-recorded on every \
         frame, so this is not measuring a screen where nothing happened: \
         {before:?} then {after:?}"
    );
}

#[test]
fn nothing_asks_for_frames_once_the_card_has_settled() {
    let mut harness = Harness::new();
    harness.swipe(at(20.0, 70.0), at(90.0, 70.0), 8, ms(300));
    harness.run_until_still();

    assert!(!harness.driver.is_animating());
    assert!(
        !harness.slide.is_animating(),
        "an animation that has arrived must stop asking for frames, or an idle \
         screen burns the battery at 60fps"
    );
}
