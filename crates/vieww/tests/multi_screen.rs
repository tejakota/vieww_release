//! Multi-screen navigation: two ways forward, one destination.
//!
//! ```console
//! cargo test -p vieww --test multi_screen
//! ```
//!
//! # What this is for
//!
//! `settings_screen.rs` already pushes a detail view from a row and pops it from
//! a back button, which proves the navigator works. What it does not cover is
//! the thing an application actually ships: **more than one way to get to the
//! same screen**, and specifically the two a phone user expects — a control you
//! press, and a page you slide.
//!
//! Those two travel through completely different machinery. A tap is one
//! recogniser claiming a pointer that never moved; a slide is a drag recogniser
//! accumulating deltas and deciding at release. They meet only at
//! `NavigatorController::push`, and until this test nothing checked that they
//! meet *there* rather than at two subtly different stacks.
//!
//! # The icons are checked, not just drawn
//!
//! Navigation assertions say nothing about what is on screen: a page that
//! painted no icons, or painted them transparent, or laid them out at zero size
//! would pass every one of them. So each page carries a progress row of one tick
//! per page reached plus the chevrons that lead somewhere, and
//! `the_icons_are_painted_as_paths` counts `Command::FillPath` in the scene —
//! which is what an `Icon` emits and a `ColoredBox` does not.
//!
//! # The gesture arena is the interesting part
//!
//! The icon sits **inside** the region that listens for the slide. Every tap on
//! it is therefore also the start of a possible drag, and the two recognisers
//! are in the arena together. A framework that resolved that wrongly would
//! either swallow the tap (the icon stops working) or fire both (one press,
//! two pages). `a_press_on_the_icon_turns_exactly_one_page` is the check on
//! that, and it is the one most likely to catch a regression.

use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{Color, IconData, Offset, PointerEvent, PointerId, Size, TextDirection};
use vieww::paint::Command;
use vieww::prelude::*;
use vieww::{icons, FrameDriver, Navigator, NavigatorController, Route};

const SURFACE: Size = Size {
    width: 400.0,
    height: 600.0,
};
const POINTER: PointerId = PointerId(1);

/// How far a finger travels sideways before it counts as a page turn.
///
/// A fifth of the screen. Small enough that a deliberate swipe always makes it,
/// large enough that the sideways wobble in a vertical scroll does not.
const SWIPE: f32 = 80.0;

/// How many pages the flow has. The last one has nowhere further to go, which is
/// its own check — a forward gesture there must not push a fourth.
const PAGES: usize = 3;

/// What a screen reader calls the two chevrons, and how the test finds them.
///
/// Located by label rather than by position for the same reason
/// `device_tests.rs` does it: a target accessibility cannot find is one no test
/// should be able to find either.
///
/// **The page number is in the label, and that is not cosmetic.** More than one
/// screen is live at a time — `Navigator` keeps the one below the top drawable
/// so a transition has something to reveal — so "is there a forward chevron"
/// cannot be answered without saying *whose*. A plain label would make a test
/// looking at page 3 find page 2's chevron and call it page 3's.
fn next_label(page: usize) -> String {
    format!("Next, from page {page}")
}

fn back_label(page: usize) -> String {
    format!("Back, from page {page}")
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// Everything the application knows.
#[derive(Clone)]
struct State {
    nav: NavigatorController,
    /// Sideways distance covered by the drag in progress.
    ///
    /// A signal rather than a plain cell because the decision is made at
    /// release, and `DragDetails::delta` is *movement since the last update* —
    /// zero at both start and end. Something has to add them up, and in a real
    /// application that something is state.
    swipe: Signal<f32>,
}

/// Go to page `from + 1`, if there is one.
fn forward(state: &State, from: usize) {
    if from < PAGES {
        state.nav.push(page(state, from + 1));
    }
}

/// A pressable icon: the shape, a hit target, and a name for a screen reader.
///
/// The label is on the **button**, not on the `Icon`. An `Icon` publishes
/// semantics only when it is given a name, and naming both would make a screen
/// reader stop twice and say the same thing — see `RenderIcon::semantics`.
fn icon_button(label: String, icon: IconData, on_tap: impl Fn() + 'static) -> WidgetNode {
    Semantics::button(label)
        .child(
            GestureDetector::new().on_tap(move |_| on_tap()).child(
                ColoredBox::new(Color::rgb(20, 90, 200)).child(
                    SizedBox::from_size(Size::new(48.0, 48.0))
                        .child(Icon::new(icon).color(Color::WHITE)),
                ),
            ),
        )
        .into()
}

/// One page: a progress row, a title, and the two chevrons that apply here.
///
/// # The icons are load-bearing, not decoration
///
/// An earlier version of this test drew one chevron on a flat `ColoredBox` and
/// asserted nothing about either. That checks the navigator and *nothing about
/// what is on screen* — a page that painted no icons at all, or painted them
/// with a transparent colour, or laid them out at zero size, passed every
/// assertion. `RenderIcon::paint` emits a `Command::FillPath` and a
/// `ColoredBox` emits a `FillRect`, so counting paths is what tells a rendered
/// icon apart from a coloured rectangle.
///
/// The progress row is **one tick per page reached**, so the number of icons on
/// screen is a fact about where you are rather than a constant. That is what
/// makes `the_progress_row_grows_as_pages_are_reached` able to fail.
fn page(state: &State, number: usize) -> Route {
    let state = state.clone();
    Route::new(
        format!("page-{number}"),
        Rc::new(move |_| {
            let tapped = state.clone();
            let popped = state.clone();
            let started = state.swipe.clone();
            let updated = state.swipe.clone();
            let released = state.clone();

            // Each page a different shade, so a screenshot of this flow shows
            // which one is up without reading anything.
            let shade = 30 + u8::try_from(number).unwrap_or(0) * 25;

            // One tick per page reached so far.
            let progress: Vec<WidgetNode> = (0..number)
                .map(|_| {
                    SizedBox::from_size(Size::new(24.0, 24.0))
                        .child(Icon::new(icons::check()).color(Color::WHITE))
                        .into()
                })
                .collect();

            // Only the chevrons that lead somewhere. A back arrow on the first
            // page and a forward arrow on the last are both controls that
            // announce themselves to a screen reader and then do nothing, which
            // is worse than their absence.
            let mut chevrons: Vec<WidgetNode> = Vec::new();
            if number > 1 {
                chevrons.push(icon_button(
                    back_label(number),
                    icons::chevron_back(TextDirection::Ltr),
                    move || {
                        popped.nav.pop();
                    },
                ));
            }
            if number < PAGES {
                chevrons.push(icon_button(
                    next_label(number),
                    icons::chevron_forward(TextDirection::Ltr),
                    move || forward(&tapped, number),
                ));
            }

            GestureDetector::new()
                .on_drag_start(move |_| started.set(0.0))
                .on_drag_update(move |drag| updated.set(updated.peek() + drag.delta.dx))
                .on_drag_end(move |_| {
                    let travelled = released.swipe.peek();
                    // Leftward is forward in a left-to-right reading order: the
                    // next page comes in from the right edge, so the finger
                    // pulls it in.
                    if travelled <= -SWIPE {
                        forward(&released, number);
                    } else if travelled >= SWIPE {
                        released.nav.pop();
                    }
                    // Anything shorter is not a page turn, and leaving the
                    // accumulator pending would let two half-swipes add up into
                    // one.
                    released.swipe.set(0.0);
                })
                .child(
                    ColoredBox::new(Color::rgb(shade, shade, shade)).child(
                        Flex::column()
                            .main_axis_alignment(MainAxisAlignment::Center)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                Flex::row()
                                    .main_axis_alignment(MainAxisAlignment::Center)
                                    .children(progress),
                                Text::new(format!("Page {number}")),
                                Flex::row()
                                    .main_axis_alignment(MainAxisAlignment::Center)
                                    .children(chevrons),
                            ]),
                    ),
                )
                .into()
        }),
    )
}

/// The root: a navigator over whatever the controller currently holds.
#[derive(Debug)]
struct App {
    nav: NavigatorController,
}

impl Widget for App {
    fn debug_name(&self) -> &'static str {
        "App"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::light())
            .child(Navigator::new(self.nav.routes()))
            .into()
    }
}

vieww::widget::widget_node_from!(App);

struct Harness {
    driver: FrameDriver,
    state: State,
    /// The clock every frame is drawn at.
    ///
    /// **`FrameDriver::draw_frame` runs at `Duration::ZERO`, every time.** A
    /// loop of it therefore draws a thousand frames at the same instant, and a
    /// route transition driven by a ticker never advances past t=0 — the test
    /// would sit for ever on a half-finished push and read the semantics of two
    /// pages at once. `draw_frame_at` with a clock that moves is what makes
    /// "let the animation finish" mean anything here.
    clock: Duration,
}

impl Harness {
    fn new() -> Self {
        let mut driver = FrameDriver::new(SURFACE);
        let runtime = driver.elements().runtime().clone();

        let state = State {
            nav: NavigatorController::new(
                &runtime,
                // Replaced immediately: the home route needs the state being
                // built around it. Same knot `settings_screen.rs` ties.
                Route::new("bootstrap", Rc::new(|_| SizedBox::shrink().into())),
            ),
            swipe: runtime.signal(0.0),
        };
        state.nav.replace(page(&state, 1));
        // Without this the route transition never advances, so a pushed page
        // stays at t=0 for ever and nothing it contains is hit-testable.
        state.nav.attach(driver.tickers());

        let app = App {
            nav: state.nav.clone(),
        };
        driver.set_root(app);

        let mut harness = Self {
            driver,
            state,
            clock: Duration::ZERO,
        };
        // Two frames before anything is asked: the first measures, the second
        // is the one laid out against those measurements.
        harness.frame();
        harness.frame();
        harness
    }

    fn frame(&mut self) {
        self.clock += ms(16);
        self.driver.draw_frame_at(self.clock);
    }

    /// Run enough frames for a route transition to finish.
    ///
    /// Asserting mid-transition is the classic flake here: `depth` changes on
    /// the push, but what is *on screen* is two pages crossfading, and both
    /// sets of semantics are live.
    ///
    /// 40 frames is 640ms against a `ROUTE_DURATION` of 220ms — deliberately
    /// several times over, because the cost of too many frames is microseconds
    /// and the cost of too few is a test that fails on a slower curve.
    fn settle(&mut self) {
        for _ in 0..40 {
            self.frame();
        }
    }

    fn tap(&mut self, at: Offset) {
        self.driver
            .handle_pointer(&PointerEvent::down(POINTER, at, ms(0)));
        self.frame();
        self.driver
            .handle_pointer(&PointerEvent::up(POINTER, at, ms(50)));
        self.frame();
    }

    /// A press, a run of moves and a release — a real finger, not a teleport.
    fn drag(&mut self, from: Offset, to: Offset, steps: u32) {
        self.driver
            .handle_pointer(&PointerEvent::down(POINTER, from, ms(0)));
        self.frame();

        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let at = Offset::new(
                from.dx + (to.dx - from.dx) * t,
                from.dy + (to.dy - from.dy) * t,
            );
            self.driver.handle_pointer(&PointerEvent::moved(
                POINTER,
                previous,
                at,
                ms(u64::from(step) * 16),
            ));
            self.frame();
            previous = at;
        }

        self.driver.handle_pointer(&PointerEvent::up(
            POINTER,
            to,
            ms(u64::from(steps) * 16 + 16),
        ));
        self.frame();
    }

    /// Slide the page sideways by `dx`, at the vertical middle.
    ///
    /// Deliberately below the icon's row so the two gestures are told apart by
    /// *what they do*, not by where they start — except in
    /// `a_slide_starting_on_the_icon_still_turns_the_page`, which starts on it
    /// on purpose.
    fn slide(&mut self, dx: f32) {
        let y = SURFACE.height - 80.0;
        let from = Offset::new(SURFACE.width / 2.0, y);
        self.drag(from, Offset::new(from.dx + dx, y), 10);
    }

    fn current(&self) -> Option<String> {
        self.state.nav.current_name()
    }

    fn depth(&self) -> usize {
        self.state.nav.depth()
    }

    /// The centre of the node a screen reader would reach by `label`.
    ///
    /// Panics rather than returning an `Option`: a test that silently skipped
    /// its own action would pass while checking nothing.
    fn centre_of(&self, label: &str) -> Offset {
        let semantics = self.driver.semantics();
        let node = semantics
            .nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no control labelled {label:?} on this page"));
        let rect = node.bounds;
        Offset::new(
            rect.origin().dx + rect.size().width / 2.0,
            rect.origin().dy + rect.size().height / 2.0,
        )
    }

    /// The forward chevron belonging to `page`.
    fn icon_centre_on(&self, page: usize) -> Offset {
        self.centre_of(&next_label(page))
    }

    fn has_control(&self, label: &str) -> bool {
        self.driver
            .semantics()
            .nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some(label))
    }

    /// How many filled paths the last frame rasterised.
    ///
    /// **This is what tells a rendered icon apart from a coloured box.**
    /// `RenderIcon::paint` fills a *path* — the fitted outline of the glyph —
    /// while `ColoredBox` fills a *rect*, and text goes out as glyph runs. So a
    /// page that forgot its icons, or drew them transparent (`RenderIcon::paint`
    /// returns early), or laid them out at zero size, differs from a working one
    /// exactly here and nowhere in the navigation state.
    fn painted_paths(&self) -> usize {
        self.driver
            .scene()
            .commands()
            .iter()
            .filter(|command| matches!(command, Command::FillPath { .. }))
            .count()
    }

    /// Everything a screen reader would read out, in tree order.
    fn labels(&self) -> Vec<String> {
        self.driver
            .semantics()
            .nodes()
            .iter()
            .filter_map(|node| node.label.clone())
            .collect()
    }
}

// ------------------------------------------------------------------ the flow

#[test]
fn the_app_opens_on_the_first_page() {
    let harness = Harness::new();

    assert_eq!(harness.current().as_deref(), Some("page-1"));
    assert_eq!(harness.depth(), 1, "nothing has been pushed yet");
    assert!(
        harness.labels().iter().any(|label| label == "Page 1"),
        "the first page should be the one on screen: {:?}",
        harness.labels()
    );
}

#[test]
fn the_icon_goes_to_the_next_screen() {
    let mut harness = Harness::new();

    let icon = harness.icon_centre_on(1);
    harness.tap(icon);
    harness.settle();

    assert_eq!(harness.current().as_deref(), Some("page-2"));
    assert_eq!(harness.depth(), 2);
}

#[test]
fn a_slide_goes_to_the_next_screen() {
    let mut harness = Harness::new();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();

    assert_eq!(harness.current().as_deref(), Some("page-2"));
    assert_eq!(harness.depth(), 2);
}

/// The headline: two different gestures, one destination.
///
/// Compared by what is **on screen** rather than by the route name, because the
/// names are what the two paths have in common by construction — both call
/// `forward`. Reading the semantics instead asks the question a user would: did
/// I end up looking at the same thing?
#[test]
fn the_icon_and_the_slide_arrive_at_the_same_screen() {
    let mut by_icon = Harness::new();
    let icon = by_icon.icon_centre_on(1);
    by_icon.tap(icon);
    by_icon.settle();

    let mut by_slide = Harness::new();
    by_slide.slide(-(SWIPE + 40.0));
    by_slide.settle();

    assert_eq!(by_icon.current(), by_slide.current());
    assert_eq!(by_icon.depth(), by_slide.depth());
    assert_eq!(
        by_icon.labels(),
        by_slide.labels(),
        "the two ways forward should leave the user looking at the same screen"
    );
}

#[test]
fn a_slide_back_returns_to_the_previous_screen() {
    let mut harness = Harness::new();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();
    assert_eq!(harness.current().as_deref(), Some("page-2"));

    harness.slide(SWIPE + 40.0);
    harness.settle();

    assert_eq!(harness.current().as_deref(), Some("page-1"));
    assert_eq!(
        harness.depth(),
        1,
        "the pushed page is gone, not stacked on"
    );
}

#[test]
fn a_slide_that_does_not_travel_far_enough_stays_put() {
    let mut harness = Harness::new();

    harness.slide(-(SWIPE - 20.0));
    harness.settle();

    assert_eq!(
        harness.current().as_deref(),
        Some("page-1"),
        "a short drag is a wobble, not a page turn"
    );
    assert_eq!(harness.depth(), 1);
}

// ---------------------------------------------------------------- the icons

/// The icons reach the scene as filled paths, not as the boxes behind them.
///
/// Page 1 carries one progress tick and one forward chevron. If either were
/// missing, transparent, or laid out at zero size this is the only assertion in
/// the file that would notice — every navigation check would still pass.
#[test]
fn the_icons_are_painted_as_paths() {
    let harness = Harness::new();

    let paths = harness.painted_paths();
    assert!(
        paths >= 2,
        "page 1 should paint a tick and a chevron, got {paths} filled path(s)"
    );
}

/// One more tick per page reached.
///
/// A count that *changes* is what makes this able to fail. A fixed row of icons
/// would be satisfied by a page that painted the same thing regardless of where
/// the user actually was.
#[test]
fn the_progress_row_grows_as_pages_are_reached() {
    let mut harness = Harness::new();
    let first = harness.painted_paths();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();
    let second = harness.painted_paths();

    assert!(
        second > first,
        "page 2 carries two ticks and two chevrons where page 1 had one of each, \
         so the second screen must rasterise more than the first's {first}: got \
         {second}"
    );
}

/// The back chevron is a third way to navigate, and it pops.
#[test]
fn the_back_icon_returns_to_the_previous_screen() {
    let mut harness = Harness::new();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();
    assert_eq!(harness.current().as_deref(), Some("page-2"));

    let back = harness.centre_of(&back_label(2));
    harness.tap(back);
    harness.settle();

    assert_eq!(harness.current().as_deref(), Some("page-1"));
    assert_eq!(harness.depth(), 1);
}

/// A control that announces itself and then does nothing is worse than no
/// control, so the chevrons that lead nowhere are absent rather than inert.
///
/// Asked per page, because more than one page is live: see `next_label`.
#[test]
fn the_chevrons_that_lead_nowhere_are_not_there() {
    let mut harness = Harness::new();

    assert!(
        !harness.has_control(&back_label(1)),
        "nothing is under the first page to go back to"
    );
    assert!(harness.has_control(&next_label(1)));

    for _ in 1..PAGES {
        harness.slide(-(SWIPE + 40.0));
        harness.settle();
    }

    assert_eq!(harness.current().as_deref(), Some("page-3"));
    assert!(
        !harness.has_control(&next_label(3)),
        "the last page has nowhere further to go"
    );
    assert!(harness.has_control(&back_label(3)));
}

/// A screen you cannot see is a screen a screen reader must not reach.
///
/// **This is the assertion that found a real defect**, on 2026-08-14. `Navigator`
/// keeps the route one below the top *onstage* on purpose — it is what a push
/// slides over and a pop reveals — and `Offstage` was the only thing that
/// pruned the semantics tree. So on a stack three deep, page 2's chevron stayed
/// published while page 3 was showing, and `handle_semantic_action` dispatches
/// by id rather than by hit test: TalkBack could swipe to a button on a screen
/// the user could not see and activate it. Touch was never affected, because the
/// opaque screen on top absorbs it, which is exactly why nothing had noticed.
///
/// The fix is `ExcludeSemantics` — draw it, do not announce it.
#[test]
fn a_covered_screen_is_not_reachable_by_a_screen_reader() {
    let mut harness = Harness::new();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();
    assert_eq!(harness.current().as_deref(), Some("page-2"));

    // Page 1 is under one opaque route: still drawn, and no longer readable.
    assert!(
        !harness.has_control(&next_label(1)),
        "page 1 is covered, so its forward chevron must be out of the \
         semantics tree: {:?}",
        harness.labels()
    );
    assert!(
        harness.has_control(&next_label(2)),
        "page 2 is the one on top"
    );

    harness.slide(-(SWIPE + 40.0));
    harness.settle();

    // And now page 2 is covered too, which is the case that has to keep
    // painting — it is one below the top, so it is not offstage.
    assert!(
        !harness.has_control(&next_label(2)),
        "page 2 is covered by page 3: {:?}",
        harness.labels()
    );
    assert!(harness.has_control(&back_label(3)));
}

/// Hiding it from a screen reader must not stop it being drawn.
///
/// The other half of the fix, and the one a careless version breaks: reaching
/// for `Offstage` here would have pruned the semantics tree *and* stopped the
/// route painting, so a push would slide the new screen over a blank surface.
/// The transition would look broken and every navigation test would still pass.
#[test]
fn a_covered_screen_is_still_painted() {
    let mut harness = Harness::new();
    let alone = harness.painted_paths();

    harness.slide(-(SWIPE + 40.0));
    harness.settle();

    assert!(
        harness.painted_paths() > alone,
        "page 2's icons and page 1's should both be rasterised while page 1 is \
         one below the top: {alone} path(s) before, {} after",
        harness.painted_paths()
    );
}

// -------------------------------------------------------------- the arena

/// One press on the icon must turn exactly one page.
///
/// The icon is inside the slide region, so a press on it enters the arena with
/// both a tap recogniser and a drag recogniser interested. Firing both would
/// push two pages from one press — and it would look like a working app right
/// up until somebody counted.
#[test]
fn a_press_on_the_icon_turns_exactly_one_page() {
    let mut harness = Harness::new();

    let icon = harness.icon_centre_on(1);
    harness.tap(icon);
    harness.settle();

    assert_eq!(harness.depth(), 2, "one press, one page");
    assert_eq!(harness.current().as_deref(), Some("page-2"));
}

/// A slide that *starts* on the icon is still a slide.
///
/// The opposite resolution of the same arena: the finger moved, so the drag
/// should win and the tap should be cancelled. If the tap fired too, this ends
/// up two pages on.
#[test]
fn a_slide_starting_on_the_icon_still_turns_one_page() {
    let mut harness = Harness::new();

    let icon = harness.icon_centre_on(1);
    harness.drag(icon, Offset::new(icon.dx - (SWIPE + 60.0), icon.dy), 10);
    harness.settle();

    assert_eq!(
        harness.depth(),
        2,
        "a drag from the icon is one page turn, not two"
    );
}

// ---------------------------------------------------------------- the edges

#[test]
fn the_last_page_has_nowhere_further_to_go() {
    let mut harness = Harness::new();

    for _ in 1..PAGES {
        harness.slide(-(SWIPE + 40.0));
        harness.settle();
    }
    assert_eq!(harness.current().as_deref(), Some("page-3"));

    harness.slide(-(SWIPE + 40.0));
    harness.settle();

    assert_eq!(
        harness.depth(),
        PAGES,
        "the flow ends at {PAGES} pages rather than growing a fourth"
    );
    assert_eq!(harness.current().as_deref(), Some("page-3"));
}

#[test]
fn sliding_back_from_the_first_page_does_nothing() {
    let mut harness = Harness::new();

    harness.slide(SWIPE + 40.0);
    harness.settle();

    assert_eq!(harness.current().as_deref(), Some("page-1"));
    assert_eq!(harness.depth(), 1, "there is nothing under the home route");
}
