//! A three-page flow you can walk through two ways: press the icon, or slide.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example screens --release
//! ```
//!
//! # What this is for
//!
//! `crates/vieww/tests/multi_screen.rs` asserts all of this headlessly — that
//! the icon and the slide arrive at the same screen, that one press turns
//! exactly one page, that a short drag turns none. What a test cannot do is show
//! that the *transition* looks right, and a page turn is one of the few things
//! in a UI where being correct and looking correct are genuinely different: a
//! stack that pushes the right route while the animation jumps, snaps back, or
//! runs backwards passes every assertion and feels broken.
//!
//! So this is the same flow with a window around it. **Slide left** to go
//! forward, **slide right** to go back, or press a chevron.
//!
//! Each page carries a row of ticks — one per page reached — and only the
//! chevrons that lead somewhere: no back arrow on the first page, no forward
//! arrow on the last. That is the same tree `multi_screen.rs` counts filled
//! paths in, so what the test asserts and what you can see here are the same
//! thing rather than two drawings that drifted apart.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower, and a
//! transition judged by eye in one is a judgement of `rustc -O0`.

use std::rc::Rc;

use vieww_element::{NavigatorController, Signal};
use vieww_foundation::{Color, Size, TextDirection};
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 420.0,
    height: 720.0,
};

/// How far a finger travels sideways before it counts as a page turn.
///
/// The same number `multi_screen.rs` uses, and for the same reason: far enough
/// that the sideways wobble in a vertical gesture is not a page turn.
const SWIPE: f32 = 80.0;
const PAGES: usize = 3;
const NEXT_LABEL: &str = "Next page";
const BACK_LABEL: &str = "Previous page";

#[derive(Clone)]
struct State {
    nav: NavigatorController,
    /// Sideways distance covered by the drag in progress.
    ///
    /// `DragDetails::delta` is movement since the last update and is zero at
    /// both ends, so the total only exists if something adds it up.
    swipe: Signal<f32>,
}

fn forward(state: &State, from: usize) {
    if from < PAGES {
        state.nav.push(page(state, from + 1));
    }
}

/// A pressable icon: the shape, a hit target, and a name for a screen reader.
///
/// The label goes on the button, not the `Icon` — an `Icon` publishes semantics
/// only when named, and naming both makes a screen reader say it twice.
fn icon_button(label: &'static str, icon: IconData, on_tap: impl Fn() + 'static) -> WidgetNode {
    Semantics::button(label)
        .child(
            GestureDetector::new().on_tap(move |_| on_tap()).child(
                ColoredBox::new(Color::rgb(20, 90, 200)).child(
                    SizedBox::from_size(Size::new(56.0, 56.0))
                        .child(Icon::new(icon).color(Color::WHITE)),
                ),
            ),
        )
        .into()
}

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

            // Each page a different shade, so which one is up is obvious at a
            // glance and a transition is visible as a wipe rather than guessed.
            let shade = 30 + u8::try_from(number).unwrap_or(0) * 25;

            // One tick per page reached — the same progress row
            // `multi_screen.rs` counts, so what the test asserts and what a
            // person sees are the same thing.
            let progress: Vec<WidgetNode> = (0..number)
                .map(|_| {
                    SizedBox::from_size(Size::new(28.0, 28.0))
                        .child(Icon::new(icons::check()).color(Color::WHITE))
                        .into()
                })
                .collect();

            // Only the chevrons that lead somewhere.
            let mut chevrons: Vec<WidgetNode> = Vec::new();
            if number > 1 {
                chevrons.push(icon_button(
                    BACK_LABEL,
                    icons::chevron_back(TextDirection::Ltr),
                    move || {
                        popped.nav.pop();
                    },
                ));
            }
            if number < PAGES {
                chevrons.push(icon_button(
                    NEXT_LABEL,
                    icons::chevron_forward(TextDirection::Ltr),
                    move || forward(&tapped, number),
                ));
            }

            GestureDetector::new()
                .on_drag_start(move |_| started.set(0.0))
                .on_drag_update(move |drag| updated.set(updated.peek() + drag.delta.dx))
                .on_drag_end(move |_| {
                    let travelled = released.swipe.peek();
                    if travelled <= -SWIPE {
                        forward(&released, number);
                    } else if travelled >= SWIPE {
                        released.nav.pop();
                    }
                    released.swipe.set(0.0);
                })
                .child(
                    ColoredBox::new(Color::rgb(shade, shade, shade + 20)).child(
                        Flex::column()
                            .main_axis_alignment(MainAxisAlignment::Center)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .children(children![
                                Flex::row()
                                    .main_axis_alignment(MainAxisAlignment::Center)
                                    .children(progress),
                                Text::new(format!("Page {number} of {PAGES}")),
                                Text::new("slide left or right, or press a chevron"),
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
struct Flow {
    nav: NavigatorController,
}

impl Widget for Flow {
    fn debug_name(&self) -> &'static str {
        "Flow"
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

widget_node_from!(Flow);

fn main() {
    // Bound before `run` rather than chained into it: the closure is twenty
    // lines, and chaining pushes the whole body two levels right for no reason.
    let app = App::new().title("vieww — screens").size(SURFACE);

    let result = app.run(|driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        let state = State {
            nav: NavigatorController::new(
                &runtime,
                Route::new("bootstrap", Rc::new(|_| SizedBox::shrink().into())),
            ),
            swipe: runtime.signal(0.0),
        };
        state.nav.replace(page(&state, 1));
        // Without this the transition never advances and a pushed page sits at
        // t=0 for ever — visible as a page that arrives already there.
        state.nav.attach(driver.tickers());

        driver.set_root(Flow {
            nav: state.nav.clone(),
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("screens failed: {error}"),
    }
}
