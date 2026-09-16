//! A dropdown opened inside a scrolling row puts its list on the window.
//!
//! ```console
//! cargo test -p vieww --test overlay_escapes_its_parent
//! ```
//!
//! # Why this file exists
//!
//! `TRACKER.md` open item 2: *"the `Dropdown` not opening on screen."*
//! Reproduced on 2026-08-17 by running `examples/controls` under Xvfb and
//! clicking it — the menu beside it opened, and the dropdown did nothing at all.
//!
//! Two defects, and the first one hid the second.
//!
//! 1. **The whole window laid out to `NaN`.** `Menu` builds a `ModalBarrier`,
//!    which is a `SizedBox::expand`, and the dropdown built it inside a row —
//!    where the main axis is unbounded, so "be infinitely large" was granted.
//!    Fixed in `Constraints::finite_minimums`, which has its own tests.
//!
//! 2. **The list was built where it was decided rather than where it belongs.**
//!    `Menu` positions itself in *window* coordinates against `ViewMetrics`, and
//!    `Dropdown` handed it the constraints of a 220×49 box inside a scrollable.
//!    The panel landed hundreds of points outside its own parent and the
//!    scrollable clipped what was left.
//!
//! [`Overlay`] is the answer to the second, and to the class rather than the
//! instance: `Menu`, `Tooltip` and every scrim have the same shape.
//!
//! **These tests run a real frame.** `Dropdown`'s own nine unit tests all build
//! the widget and inspect the builder, so not one of them could see either
//! defect — the first only appears under layout and the second only under a
//! layout that is not the window's.

use std::rc::Rc;

use vieww::foundation::{Color, Constraints, Rect, Size};
use vieww::prelude::*;
use vieww::BuildContext;
use vieww_element::Signal;
use vieww_render::FrameDriver;
use vieww_widget::{widget_node_from, Dropdown, Overlay, Scrollable};

const SURFACE: Size = Size::new(880.0, 720.0);
/// How wide the closed control is, as `examples/controls` sizes it.
const CONTROL: f32 = 220.0;
const OPTIONS: [&str; 3] = ["Ireland", "Japan", "Peru"];

/// The signals a controlled dropdown needs, held above the tree as an
/// application holds them.
#[derive(Debug, Clone)]
struct State {
    open: Signal<bool>,
    anchor: Signal<Option<Rect>>,
    chosen: Signal<Option<usize>>,
    /// Whether the dropdown is in the tree at all, for the teardown test.
    mounted: Signal<bool>,
}

/// A dropdown in a row, in a column, in a scrollable — the shape it failed in.
#[derive(Debug)]
struct Page {
    state: State,
}

impl Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let state = self.state.clone();
        let toggle = state.open.clone();
        let anchor = state.anchor.clone();
        let chose = state.chosen.clone();
        let close = state.open.clone();

        let mut dropdown = Dropdown::new(OPTIONS, state.chosen.get())
            .placeholder("Choose a country")
            .open(state.open.get())
            .on_toggled(Rc::new(move |open| toggle.set(open)))
            .on_measured(Rc::new(move |rect| {
                if anchor.peek() != Some(rect) {
                    anchor.set(Some(rect));
                }
            }))
            .on_selected(Rc::new(move |index| {
                chose.set(Some(index));
                close.set(false);
            }));
        if let Some(rect) = state.anchor.get() {
            dropdown = dropdown.anchor(rect);
        }

        let row = Flex::row()
            .push(Constrained::new(Constraints::tight_for_width(CONTROL)).child(dropdown));

        let mut column = Flex::column()
            // Content above it, so the control is a long way down the window and
            // a list placed in the control's own space is somewhere obviously
            // wrong rather than coincidentally right.
            .push(SizedBox::height(300.0).child(ColoredBox::new(Color::rgb(10, 20, 30))));
        if self.state.mounted.get() {
            column = column.push(row);
        }
        column = column.push(SizedBox::height(900.0));

        Scrollable::vertical(0.0).child(column).into()
    }
}

widget_node_from!(Page);

/// A driver showing the page under an [`Overlay`], and its state.
fn app() -> (FrameDriver, State) {
    let mut driver = FrameDriver::new(SURFACE);
    let runtime = driver.elements().runtime().clone();
    let state = State {
        open: runtime.signal(false),
        anchor: runtime.signal(None),
        chosen: runtime.signal(None),
        mounted: runtime.signal(true),
    };
    driver.set_root(Overlay::new().child(Page {
        state: state.clone(),
    }));
    driver.draw_frame();
    (driver, state)
}

/// Every label the frame published, with the rectangle it was published at.
///
/// The semantics tree rather than the scene, because a `GlyphRun` carries glyph
/// **ids** and no text — and because it answers the better question anyway:
/// these bounds are what a screen reader draws its focus rectangle at and what
/// touch exploration hit tests against, so a list that is not here is not merely
/// invisible, it is unreachable.
fn labels(driver: &FrameDriver) -> Vec<(String, Rect)> {
    driver
        .semantics()
        .nodes()
        .iter()
        .filter_map(|node| node.label.clone().map(|label| (label, node.bounds)))
        .collect()
}

/// `true` if an option is published somewhere inside the window.
///
/// Inside, not merely present: the defect placed the panel hundreds of points
/// outside its own parent, so a test that only asked whether the node existed
/// would have passed on the broken build.
fn option_on_screen(driver: &FrameDriver, option: &str) -> bool {
    let window = Rect::new(0.0, 0.0, SURFACE.width, SURFACE.height);
    labels(driver).iter().any(|(label, bounds)| {
        label.contains(option)
            && bounds.width() > 0.0
            && bounds.height() > 0.0
            && bounds.left >= window.left
            && bounds.top >= window.top
            && bounds.right <= window.right
            && bounds.bottom <= window.bottom
    })
}

#[test]
fn an_open_dropdown_puts_its_options_on_the_window() {
    let (mut driver, state) = app();
    assert!(
        !option_on_screen(&driver, "Japan"),
        "the list is showing before anything opened it"
    );

    state.open.set(true);
    driver.draw_frame();

    for option in OPTIONS {
        assert!(
            option_on_screen(&driver, option),
            "'{option}' is not on the window. The list was built inside the \
             220-wide control it hangs off, where the scrollable clips it and \
             its window-coordinate placement means nothing. Drawn: {:?}",
            labels(&driver)
        );
    }
}

#[test]
fn the_list_arrives_on_the_frame_that_opened_it_rather_than_the_next_one() {
    // The half that makes the overlay worth having rather than merely correct.
    // An entry is contributed by a descendant *after* the overlay above it has
    // already built, so the naive version shows it one frame late — and one
    // frame of a control that visibly did nothing is exactly what the original
    // report looked like. `FrameSink::layout` settles it in place instead.
    let (mut driver, state) = app();
    state.open.set(true);
    driver.draw_frame();

    assert!(
        option_on_screen(&driver, "Japan"),
        "the list needed a second frame — `poll_states` did not see the \
         overlay's request, or the settle loop painted before it ran"
    );
}

#[test]
fn closing_it_takes_the_list_back_down() {
    let (mut driver, state) = app();
    state.open.set(true);
    driver.draw_frame();
    assert!(option_on_screen(&driver, "Japan"), "the starting point");

    state.open.set(false);
    driver.draw_frame();
    assert!(
        !option_on_screen(&driver, "Japan"),
        "the caller closed it and the list is still up — an entry that is only \
         withdrawn on dispose leaves a menu over an application that has \
         forgotten it: {:?}",
        labels(&driver)
    );
}

#[test]
fn a_dropdown_removed_from_the_tree_takes_its_open_list_with_it() {
    // The case a hand-written `hide` misses, and the reason `DropdownState`
    // exists: a picker scrolled out of a long list, or a screen popped, while
    // its menu is up. Nothing else in the tree knows the entry is there.
    let (mut driver, state) = app();
    state.open.set(true);
    driver.draw_frame();
    assert!(option_on_screen(&driver, "Japan"), "the starting point");

    state.mounted.set(false);
    driver.draw_frame();
    assert!(
        !option_on_screen(&driver, "Japan"),
        "the control is gone and its list is still on the window: {:?}",
        labels(&driver)
    );
}

#[test]
fn the_selection_that_arrives_is_the_one_that_was_ticked() {
    // The entry is a *builder*, not a captured node, so that a rebuild of the
    // dropdown reaches the list. A snapshot would keep whatever was current when
    // it opened, which is invisible until somebody changes the selection while
    // the list is up.
    let (mut driver, state) = app();
    state.open.set(true);
    driver.draw_frame();

    state.chosen.set(Some(2));
    driver.draw_frame();

    assert!(
        option_on_screen(&driver, "Peru"),
        "the list stopped being drawn when the selection changed under it"
    );
    assert_eq!(
        state.chosen.get(),
        Some(2),
        "and the application still owns the value"
    );
}

#[test]
fn the_closed_control_is_still_where_it_was() {
    // The overlay must not move the application. A `Stack` with `Expand` around
    // the whole tree is a real change to what the root is measured against, and
    // this is the guard on it.
    let (mut driver, state) = app();
    let closed = state
        .anchor
        .get()
        .expect("the control reported a rectangle");

    state.open.set(true);
    driver.draw_frame();

    assert_eq!(
        state.anchor.get(),
        Some(closed),
        "opening the list moved the control it hangs off"
    );
    assert!(
        (closed.width() - CONTROL).abs() < 0.5,
        "the control is {} wide rather than {CONTROL}",
        closed.width()
    );
}
