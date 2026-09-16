//! The Phase 9 exit test: a real settings screen, built entirely from the
//! widget library.
//!
//! ```console
//! cargo test -p vieww --test settings_screen
//! ```
//!
//! # What the roadmap asks for
//!
//! > a real, non-trivial screen (e.g. a settings page with a scroll list,
//! > toggles, and navigation to a detail view) built entirely from your widget
//! > library, running smoothly on-device.
//!
//! All of it is here except the last two words, and those are stated rather than
//! faked: this runs the whole pipeline — widgets, elements, layout, hit testing,
//! gestures, painting — on a headless surface. "On-device" is `--example hello`
//! on a phone, and Phase 8's own status says the phone has never run a window.
//!
//! What *is* asserted is everything a settings screen is made of:
//!
//! - a list of two hundred rows that costs a dozen widgets, not two hundred;
//! - a drag that scrolls it, and a tap that does not scroll it — through the
//!   real gesture arena, in the same tree, at the same time;
//! - toggles that flip when tapped, on the row the finger actually hit;
//! - a row that pushes a detail screen, a back button that pops it, and the
//!   scroll position still where it was left;
//! - a dialog that covers the screen and takes the taps that miss it;
//! - and a screen reader that can tell all of these apart.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{EdgeInsets, Offset, PointerEvent, PointerId, Size};
use vieww::gestures::ScrollPhysics;
use vieww::prelude::*;
use vieww::{FrameDriver, NavigatorController, Role, ScrollController};

const SURFACE: Size = Size {
    width: 400.0,
    height: 600.0,
};
const ROW: f32 = 60.0;
const ROWS: usize = 200;
const POINTER: PointerId = PointerId(1);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// Everything the application knows, in signals.
#[derive(Clone)]
struct State {
    nav: NavigatorController,
    scroll: ScrollController,
    /// Which rows are switched on.
    toggles: Signal<Vec<bool>>,
    brightness: Signal<f32>,
    /// The dialog is a route, so "is it open" is the navigator's business — this
    /// only records that its action ran.
    reset: Rc<RefCell<bool>>,
}

/// One row of the settings list: a label, and a switch on the right.
fn toggle_row(state: &State, index: usize) -> WidgetNode {
    let toggles = state.toggles.clone();
    let on = toggles.get().get(index).copied().unwrap_or(false);

    let write = toggles.clone();
    let switch = Switch::new(on)
        .label(format!("Setting {index}"))
        .on_changed(Rc::new(move |next| {
            let mut values = write.peek();
            values[index] = next;
            write.set(values);
        }));

    Padding::new(EdgeInsets::symmetric(16.0, 0.0))
        .child(
            SizedBox::from_size(Size::new(SURFACE.width, ROW)).child(
                Flex::row()
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![Text::new(format!("Setting {index}")), switch]),
            ),
        )
        .into()
}

/// The row that leads somewhere: tapping it pushes the detail screen.
fn detail_row(state: &State) -> WidgetNode {
    let push = state.nav.on_push(detail_route(state));

    GestureDetector::new()
        .on_tap(move |_| push())
        .child(
            SizedBox::from_size(Size::new(SURFACE.width, ROW)).child(
                Padding::new(EdgeInsets::symmetric(16.0, 0.0)).child(
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(children![
                            Text::new("Display"),
                            Icon::new(icons::chevron_right()).size(20.0),
                        ]),
                ),
            ),
        )
        .into()
}

/// The home screen: a scrollable, virtualised list of rows.
fn home_route(state: &State) -> Route {
    let state = state.clone();
    Route::new(
        "settings",
        Rc::new(move |_| {
            let rows = state.clone();
            Scrollable::vertical(state.scroll.offset())
                .viewport(SURFACE.height)
                .on_drag(state.scroll.on_drag())
                .on_drag_end(state.scroll.on_drag_end())
                .on_extents(state.scroll.on_extents())
                .child(ListView::new(
                    ROWS,
                    ROW,
                    Rc::new(move |index| {
                        // Row zero leads to the detail screen; the rest are
                        // toggles. A list that is all one thing would not prove
                        // that the right row got the tap.
                        if index == 0 {
                            detail_row(&rows)
                        } else {
                            toggle_row(&rows, index)
                        }
                    }),
                ))
                .into()
        }),
    )
}

/// The detail screen: a back button and a slider.
fn detail_route(state: &State) -> Route {
    let state = state.clone();
    Route::new(
        "display",
        Rc::new(move |_| {
            let back = state.nav.on_pop();
            let brightness = state.brightness.clone();
            let write = brightness.clone();

            // Opaque, and it says so by painting a background: a route that
            // covers another has to actually cover it.
            ColoredBox::new(Color::WHITE)
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            Button::new("Back").on_pressed(move || back()),
                            Text::new("Brightness"),
                            Slider::new(brightness.get())
                                .label("Brightness")
                                .on_changed(Rc::new(move |next| write.set(next))),
                        ]),
                )
                .into()
        }),
    )
}

/// The confirmation dialog, pushed as a modal route.
fn reset_route(state: &State) -> Route {
    let state = state.clone();
    Route::modal(
        "reset",
        Rc::new(move |_| {
            let dismiss = state.nav.on_pop();
            let confirm_nav = state.nav.clone();
            let confirmed = Rc::clone(&state.reset);

            Dialog::new()
                .title("Reset all settings?")
                .content(Text::new("This cannot be undone."))
                .actions(children![Button::new("Reset").on_pressed(move || {
                    *confirmed.borrow_mut() = true;
                    confirm_nav.pop();
                })])
                .on_dismiss(move || dismiss())
                .into()
        }),
    )
}

struct Harness {
    driver: FrameDriver,
    state: State,
}

impl Harness {
    fn new() -> Self {
        let mut driver = FrameDriver::new(SURFACE);
        let runtime = driver.elements().runtime().clone();

        let scroll = ScrollController::new(&runtime, ScrollPhysics::android());
        scroll.attach(driver.tickers());

        let state = State {
            nav: NavigatorController::new(
                &runtime,
                // A placeholder, replaced below: the home route needs the state
                // that is being built around it, which is the one knot in
                // wiring a navigator up.
                Route::new("bootstrap", Rc::new(|_| SizedBox::shrink().into())),
            ),
            scroll,
            toggles: runtime.signal(vec![false; ROWS]),
            brightness: runtime.signal(0.5),
            reset: Rc::new(RefCell::new(false)),
        };
        state.nav.replace(home_route(&state));

        let app = App {
            nav: state.nav.clone(),
        };
        driver.set_root(app);
        driver.draw_frame();
        // A second frame: the first one is what *measures* the viewport, and the
        // extents it reports are what the list needs to virtualise properly.
        driver.draw_frame();

        Self { driver, state }
    }

    fn frame(&mut self) {
        self.driver.draw_frame();
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

    fn semantic_roles(&self) -> Vec<Role> {
        self.driver
            .semantics()
            .nodes()
            .iter()
            .map(|node| node.role)
            .collect()
    }

    /// Everything a screen reader would read out.
    fn labels(&self) -> Vec<String> {
        self.driver
            .semantics()
            .nodes()
            .iter()
            .filter_map(|node| node.label.clone())
            .collect()
    }

    /// The centre of the first node a screen reader would call a switch.
    fn first_switch_centre(&self) -> Offset {
        let semantics = self.driver.semantics();
        let node = semantics
            .nodes()
            .iter()
            .filter(|node| node.role == Role::Switch)
            .min_by(|a, b| a.bounds.top.total_cmp(&b.bounds.top))
            .expect("the list has switches in it");
        Offset::new(
            node.bounds.left + node.bounds.width() / 2.0,
            node.bounds.top + node.bounds.height() / 2.0,
        )
    }

    fn toggles_on(&self) -> usize {
        self.state.toggles.peek().iter().filter(|on| **on).count()
    }

    fn current_screen(&self) -> Option<String> {
        self.state.nav.current_name()
    }
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

// --------------------------------------------------- the list, and what it costs

#[test]
fn two_hundred_rows_cost_a_screenful_of_widgets() {
    let harness = Harness::new();

    // Every switch on screen is one row. Ten rows fit in 600px, plus overscan
    // and the row that leads to the detail screen.
    let switches = harness
        .semantic_roles()
        .iter()
        .filter(|role| **role == Role::Switch)
        .count();

    assert!(
        (8..=16).contains(&switches),
        "a virtualised list of {ROWS} rows built {switches} switches"
    );
}

#[test]
fn a_drag_scrolls_the_list() {
    let mut harness = Harness::new();
    assert_eq!(harness.state.scroll.peek(), 0.0);

    // Up the screen by 200px: the content moves the same distance the finger did.
    harness.drag(Offset::new(200.0, 400.0), Offset::new(200.0, 200.0), 8);

    let offset = harness.state.scroll.peek();
    assert!(
        offset > 150.0,
        "a 200px drag should scroll about that far, not {offset}"
    );
}

#[test]
fn scrolling_builds_the_rows_that_came_into_view() {
    let mut harness = Harness::new();
    harness.drag(Offset::new(200.0, 500.0), Offset::new(200.0, 100.0), 10);
    harness.frame();

    // The list is somewhere in the middle now, so the first switch on screen is
    // no longer at the top of the surface's coordinate space by accident — what
    // matters is that there are still only a screenful of them.
    let switches = harness
        .semantic_roles()
        .iter()
        .filter(|role| **role == Role::Switch)
        .count();
    assert!(
        (8..=16).contains(&switches),
        "scrolling must not accumulate rows: {switches}"
    );
}

// ------------------------------------------------------------------- the toggles

#[test]
fn a_tap_flips_the_switch_it_landed_on() {
    let mut harness = Harness::new();
    assert_eq!(harness.toggles_on(), 0);

    let switch = harness.first_switch_centre();
    harness.tap(switch);

    assert_eq!(
        harness.toggles_on(),
        1,
        "exactly the switch under the finger, and only it"
    );
}

#[test]
fn a_tap_that_drifts_two_pixels_still_flips_the_switch() {
    let mut harness = Harness::new();
    let switch = harness.first_switch_centre();

    // A finger never holds perfectly still. Two pixels is not a scroll.
    harness.drag(switch, Offset::new(switch.dx, switch.dy + 2.0), 2);

    assert_eq!(harness.toggles_on(), 1);
    assert_eq!(
        harness.state.scroll.peek(),
        0.0,
        "and it certainly is not a scroll"
    );
}

#[test]
fn a_drag_across_a_switch_scrolls_instead_of_flipping_it() {
    let mut harness = Harness::new();
    let switch = harness.first_switch_centre();

    harness.drag(switch, Offset::new(switch.dx, switch.dy - 120.0), 8);

    assert_eq!(
        harness.toggles_on(),
        0,
        "a drag that began on a switch must not switch it"
    );
    assert!(harness.state.scroll.peek() > 80.0);
}

// ---------------------------------------------------------------- the navigation

#[test]
fn a_row_pushes_a_detail_screen_and_back_returns() {
    let mut harness = Harness::new();
    assert_eq!(harness.current_screen().as_deref(), Some("settings"));

    // The first row is the one that leads somewhere.
    harness.tap(Offset::new(200.0, ROW / 2.0));
    assert_eq!(harness.current_screen().as_deref(), Some("display"));

    // The detail screen has a slider on it; the settings list does not.
    assert!(harness.semantic_roles().contains(&Role::Slider));

    // "Back" is the only button on the detail screen.
    let back = {
        let semantics = harness.driver.semantics();
        let node = semantics
            .nodes()
            .iter()
            .find(|node| node.role == Role::Button)
            .expect("a back button")
            .clone();
        Offset::new(
            node.bounds.left + node.bounds.width() / 2.0,
            node.bounds.top + node.bounds.height() / 2.0,
        )
    };
    harness.tap(back);

    assert_eq!(harness.current_screen().as_deref(), Some("settings"));
}

#[test]
fn the_list_is_where_it_was_left_after_coming_back() {
    let mut harness = Harness::new();
    harness.drag(Offset::new(200.0, 500.0), Offset::new(200.0, 300.0), 8);
    let scrolled_to = harness.state.scroll.peek();
    assert!(scrolled_to > 0.0);

    harness.state.nav.push(detail_route(&harness.state));
    harness.frame();
    harness.state.nav.pop();
    harness.frame();

    assert_eq!(
        harness.state.scroll.peek(),
        scrolled_to,
        "a screen that comes back must come back where it was"
    );
}

// -------------------------------------------------------------------- the dialog

#[test]
fn a_dialog_covers_the_screen_and_its_action_runs() {
    let mut harness = Harness::new();
    harness.state.nav.push(reset_route(&harness.state));
    harness.frame();

    assert_eq!(harness.current_screen().as_deref(), Some("reset"));
    assert!(!*harness.state.reset.borrow());

    // The dialog's only button is the last one a screen reader would find, and
    // the settings list behind it has none.
    let confirm = {
        let semantics = harness.driver.semantics();
        let node = semantics
            .nodes()
            .iter()
            .find(|node| node.role == Role::Button)
            .expect("the dialog's action")
            .clone();
        Offset::new(
            node.bounds.left + node.bounds.width() / 2.0,
            node.bounds.top + node.bounds.height() / 2.0,
        )
    };
    harness.tap(confirm);

    assert!(*harness.state.reset.borrow(), "the action ran");
    assert_eq!(
        harness.current_screen().as_deref(),
        Some("settings"),
        "and it dismissed itself"
    );
}

#[test]
fn a_tap_that_misses_the_dialog_does_not_reach_the_list_behind_it() {
    let mut harness = Harness::new();
    harness.state.nav.push(reset_route(&harness.state));
    harness.frame();

    // The top-left corner is scrim, and behind it is the row that navigates.
    harness.tap(Offset::new(8.0, 8.0));

    assert_eq!(
        harness.current_screen().as_deref(),
        Some("settings"),
        "tapping the scrim dismisses the dialog"
    );
    assert_eq!(
        harness.toggles_on(),
        0,
        "and nothing behind it was pressed on the way"
    );
}

/// The other half of the test above, for the other input device.
///
/// **This is the assertion the barrier did not previously survive.** A tap that
/// misses the dialog is eaten by the scrim, which is why the list behind it was
/// safe from a finger. A screen reader is not a finger: it walks the semantics
/// tree rather than the pixels, and `handle_semantic_action` dispatches by id,
/// so nothing about the scrim was in its way. A user could swipe onto a switch
/// behind the dialog and flip it.
///
/// `Navigator` could not close this on its own. It excludes everything below the
/// top **opaque** route, and a dialog is deliberately not opaque — the screen
/// behind it is meant to show through. Widening that to `!is_opaque()` would
/// silence a whole screen behind a *snackbar*, which is also a modal route and
/// covers nothing. So the marker belongs on `ModalBarrier`, which is the thing
/// that actually covers something, and `Snackbar` builds none.
#[test]
fn a_screen_reader_cannot_reach_the_list_behind_the_dialog() {
    let mut harness = Harness::new();
    assert!(
        harness.semantic_roles().contains(&Role::Switch),
        "the switches are readable before the dialog goes up"
    );

    harness.state.nav.push(reset_route(&harness.state));
    harness.frame();

    let roles = harness.semantic_roles();
    assert!(
        !roles.contains(&Role::Switch),
        "every switch is behind the scrim and must be out of the semantics \
         tree: {roles:?}"
    );
    assert!(
        !roles.contains(&Role::ScrollView),
        "so is the list that holds them: {roles:?}"
    );
}

/// And the dialog itself is still readable, which is the half a careless
/// implementation breaks.
///
/// The barrier is the *first* child of the dialog's stack and the dialog's
/// surface is the second, so a block scoped to "everything at this level" rather
/// than "everything painted before me" would silence the dialog it belongs to —
/// leaving a modal a screen reader can neither answer nor escape, which is
/// worse than the bug being fixed.
#[test]
fn the_dialog_is_readable_over_the_screen_it_silences() {
    let mut harness = Harness::new();
    harness.state.nav.push(reset_route(&harness.state));
    harness.frame();

    let labels = harness.labels();
    assert!(
        labels.iter().any(|label| label == "Reset all settings?"),
        "the dialog announces itself: {labels:?}"
    );
    assert!(
        labels.iter().any(|label| label == "Reset"),
        "and its action is reachable: {labels:?}"
    );
    assert!(
        harness.semantic_roles().contains(&Role::Button),
        "as a button, not as text"
    );
}

/// Dismissing it gives the screen back.
///
/// A block that outlived its barrier would be the same defect pointing the other
/// way: a settings screen that never speaks again after its first dialog.
#[test]
fn the_list_is_readable_again_once_the_dialog_goes() {
    let mut harness = Harness::new();
    harness.state.nav.push(reset_route(&harness.state));
    harness.frame();
    assert!(!harness.semantic_roles().contains(&Role::Switch));

    // The top-left corner is scrim; tapping it dismisses.
    harness.tap(Offset::new(8.0, 8.0));
    // One more frame than the tap draws: a pop leaves the dialog in the
    // navigator's `leaving` slot on its way out, and the question here is about
    // the screen it uncovers rather than about the frame it is uncovered on.
    harness.frame();

    assert_eq!(harness.current_screen().as_deref(), Some("settings"));
    assert!(
        harness.semantic_roles().contains(&Role::Switch),
        "the switches come back with the screen: {:?}",
        harness.semantic_roles()
    );
}

// ------------------------------------------------------------- what it announces

#[test]
fn a_screen_reader_can_tell_the_parts_of_the_screen_apart() {
    let harness = Harness::new();
    let roles = harness.semantic_roles();

    assert!(roles.contains(&Role::ScrollView), "a scrollable region");
    assert!(roles.contains(&Role::Switch), "switches, not buttons");
    assert!(roles.contains(&Role::Label), "the rows' text");
}

#[test]
fn a_switch_reports_its_state_and_its_name() {
    let mut harness = Harness::new();
    let switch = harness.first_switch_centre();
    harness.tap(switch);

    let semantics = harness.driver.semantics();
    let flipped = semantics
        .nodes()
        .iter()
        .find(|node| node.role == Role::Switch && node.toggled == Some(true))
        .expect("the switch that was tapped reports itself as on");

    assert!(
        flipped
            .label
            .as_deref()
            .is_some_and(|label| label.starts_with("Setting")),
        "a switch a screen reader cannot name is a switch nobody can use: {:?}",
        flipped.label
    );
}
