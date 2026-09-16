//! A hands-on showcase of the motion system.
//!
//! ```console
//! cargo run -p animation-showcase --features gpu
//! ```
//!
//! Four panels, one per motion primitive:
//!
//! | panel | demonstrates |
//! |---|---|
//! | Springs | interruptible springs, expressive vs. standard presets |
//! | Timeline | staggered list reveal |
//! | Drag | physics-driven drag with snap points |
//! | Charts | CustomPaint-based line and bar charts |

use std::time::Duration;

use vieww_animation::SpringPreset;
use vieww_element::{PhysicsDrag, Runtime, Signal, TimelineBuilder, TimelinePlayer};
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;
use vieww_widget::{BarChart, LineChart};

const INK: Color = Color::rgb(23, 30, 42);
const PAPER: Color = Color::rgb(247, 248, 250);
const ACCENT: Color = Color::rgb(58, 122, 246);
const EXPRESSIVE_COLOR: Color = Color::rgb(186, 100, 246);
const STANDARD_COLOR: Color = Color::rgb(76, 187, 129);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = vieww_platform_winit::App::new()
        .title("vieww — motion showcase")
        .size(Size::new(800.0, 600.0))
        .background(PAPER);

    let report = app.run(|driver| {
        let runtime = driver.elements().runtime().clone();

        // ── Springs panel ─────────────────────────────────────────────
        // Two boxes, same target, different presets. Tap to animate.
        let expressive_pos = runtime.signal(0.0f32);
        let standard_pos = runtime.signal(0.0f32);

        // ── Timeline panel ────────────────────────────────────────────
        // A list of rows that stagger-reveals.
        let row_opacities: Vec<Signal<f32>> = (0..8).map(|_| runtime.signal(0.0f32)).collect();

        // ── Drag panel ────────────────────────────────────────────────
        let drag = PhysicsDrag::new(
            &runtime,
            0.0,
            vec![0.0, 100.0, 200.0, 300.0, 400.0],
            SpringPreset::Expressive,
        );

        // **Attached, not played.** `Timeline::attach` registers the timeline
        // with the driver's `Tickers` here — the one place a `&mut` to them
        // exists — and returns a `TimelinePlayer` that starts it later.
        //
        // This used to be `play(driver.tickers())` and a comment explaining
        // that the "Reveal" button had to be deleted, because `Timeline::play`
        // needs `&mut Tickers` at the moment it fires and a widget callback has
        // no route to them. The button is back below; the API grew the route.
        let reveal = TimelineBuilder::new()
            // `StaggerBuilder::action` consumes and returns the builder, so the
            // rows fold through it — a `for` loop would evaluate to `()`.
            .stagger(Duration::from_millis(40), |s| {
                row_opacities
                    .iter()
                    .fold(s, |builder, row| builder.action(row, 0.0, 1.0))
            })
            .build()
            .attach(driver.tickers());

        // Played once at start-up so the screen is not blank on arrival, and
        // playable again from the button in `timeline_panel`.
        reveal.play();

        driver.set_root(showcase_screen(
            expressive_pos.clone(),
            standard_pos.clone(),
            &row_opacities,
            drag.position_signal(),
            reveal,
            runtime,
        ));
    })?;

    println!("{report}");
    Ok(())
}

fn showcase_screen(
    expressive: Signal<f32>,
    standard: Signal<f32>,
    rows: &[Signal<f32>],
    drag_position: Signal<f32>,
    reveal: TimelinePlayer,
    runtime: Runtime,
) -> WidgetNode {
    let content = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(24.0)
        .push(
            Text::new("vieww motion showcase")
                .color(INK)
                .size(24.0)
                .bold(),
        )
        .push(springs_panel(expressive, standard, &runtime))
        .push(timeline_panel(rows, reveal))
        .push(drag_panel(drag_position))
        .push(charts_panel());

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(32.0))
        .child(content)
        .into()
}

fn panel(title: &str, content: impl Into<WidgetNode>) -> WidgetNode {
    Container::new()
        .color(Color::WHITE)
        .radius(12.0)
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .children(children![
                    Text::new(title).color(INK).size(18.0).bold(),
                    content.into(),
                ]),
        )
        .into()
}

// ── Springs panel ──────────────────────────────────────────────────────────

fn springs_panel(expressive: Signal<f32>, standard: Signal<f32>, _runtime: &Runtime) -> WidgetNode {
    panel(
        "Springs — tap to toggle (expressive bounces, standard does not)",
        Flex::column().spacing(24.0).children(children![
            spring_track("expressive (damping 0.8)", expressive, EXPRESSIVE_COLOR),
            spring_track("standard (damping 1.0)", standard, STANDARD_COLOR),
        ]),
    )
}

fn spring_track(_label: &str, position: Signal<f32>, color: Color) -> WidgetNode {
    let pos = position.clone();
    let pos_for_tap = position.clone();

    Pressable::new(move |_press| {
        // Map 0-100 to 0-600px.
        let x = pos.get() * 6.0;
        Stack::new()
            .push(
                Container::new()
                    .color(Color::rgb(226, 232, 240))
                    .radius(2.0)
                    .child(SizedBox::from_size(Size::new(600.0, 4.0))),
            )
            .push(
                Positioned::new().left(x).top(-10.0).child(
                    Container::new()
                        .color(color)
                        .radius(12.0)
                        .child(SizedBox::square(24.0)),
                ),
            )
            .into()
    })
    .on_tap(move || {
        // Toggle between 0 and 100.
        let current = pos_for_tap.peek();
        let target = if current < 50.0 { 100.0 } else { 0.0 };
        pos_for_tap.set(target);
    })
    .into()
}

// ── Timeline panel ─────────────────────────────────────────────────────────

fn timeline_panel(rows: &[Signal<f32>], reveal: TimelinePlayer) -> WidgetNode {
    // **One widget per row, and each reads its own signal inside `build`.**
    //
    // This used to be `Opacity::new(opacity.get())` right here, in a function
    // called while the tree was being *constructed* — before `set_root`, and so
    // outside any build. `Runtime::track` records a read only while a build is
    // on the tracking stack, so those reads subscribed nothing: the timeline
    // wrote eight signals every frame and not one element rebuilt. The panel
    // was inert and looked like a panel that had simply been given no
    // animation.
    let row_widgets: Vec<WidgetNode> = rows
        .iter()
        .map(|opacity| {
            WidgetNode::from(RevealRow {
                opacity: opacity.clone(),
            })
        })
        .collect();

    panel(
        "Timeline — staggered reveal, from a button",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(12.0)
            .children(children![
                // The control that could not exist before `Timeline::attach`.
                Button::new("Reveal").on_pressed(move || reveal.play()),
                Flex::column().spacing(6.0).children(row_widgets),
            ]),
    )
}

/// One row of the staggered reveal.
///
/// A widget rather than an inline `Opacity` for one reason: `build` is where a
/// signal read subscribes. See the note in [`timeline_panel`].
#[derive(Debug)]
struct RevealRow {
    opacity: Signal<f32>,
}

impl Widget for RevealRow {
    fn debug_name(&self) -> &'static str {
        "RevealRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Opacity::new(self.opacity.get().clamp(0.0, 1.0))
            .child(
                Container::new()
                    .color(ACCENT)
                    .radius(4.0)
                    .child(SizedBox::from_size(Size::new(500.0, 28.0))),
            )
            .into()
    }
}

vieww_widget::widget_node_from!(RevealRow);

// ── Drag panel ─────────────────────────────────────────────────────────────

fn drag_panel(position: Signal<f32>) -> WidgetNode {
    panel(
        "Physics drag — the dot follows a spring between snap points",
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(16.0)
            .children(children![
                Text::new("drag the handle — release to snap")
                    .color(Color::rgb(110, 122, 140))
                    .size(14.0),
                Stack::new()
                    .push(
                        // The track.
                        Container::new()
                            .color(Color::rgb(226, 232, 240))
                            .radius(2.0)
                            .child(SizedBox::from_size(Size::new(400.0, 4.0))),
                    )
                    // Snap point markers at 0, 100, 200, 300, 400.
                    .push(
                        Positioned::new().left(0.0).top(-4.0).child(
                            Container::new()
                                .color(Color::rgb(148, 163, 184))
                                .radius(2.0)
                                .child(SizedBox::square(8.0))
                        ),
                    )
                    .push(
                        Positioned::new().left(100.0).top(-4.0).child(
                            Container::new()
                                .color(Color::rgb(148, 163, 184))
                                .radius(2.0)
                                .child(SizedBox::square(8.0))
                        ),
                    )
                    .push(
                        Positioned::new().left(200.0).top(-4.0).child(
                            Container::new()
                                .color(Color::rgb(148, 163, 184))
                                .radius(2.0)
                                .child(SizedBox::square(8.0))
                        ),
                    )
                    .push(
                        Positioned::new().left(300.0).top(-4.0).child(
                            Container::new()
                                .color(Color::rgb(148, 163, 184))
                                .radius(2.0)
                                .child(SizedBox::square(8.0))
                        ),
                    )
                    .push(
                        Positioned::new().left(400.0).top(-4.0).child(
                            Container::new()
                                .color(Color::rgb(148, 163, 184))
                                .radius(2.0)
                                .child(SizedBox::square(8.0))
                        ),
                    )
                    // The handle, following the drag signal.
                    //
                    // Read inside `DragHandle::build`, not here. `pos.get()` at
                    // this point is a read outside any build and subscribes
                    // nothing — the handle would never have moved. See
                    // `timeline_panel`.
                    .push(DragHandle { position }),
            ]),
    )
}

/// The dot that follows the physics drag.
#[derive(Debug)]
struct DragHandle {
    position: Signal<f32>,
}

impl Widget for DragHandle {
    fn debug_name(&self) -> &'static str {
        "DragHandle"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Positioned::new()
            .left(self.position.get())
            .top(-10.0)
            .child(
                Container::new()
                    .color(ACCENT)
                    .radius(14.0)
                    .child(SizedBox::square(28.0)),
            )
            .into()
    }
}

vieww_widget::widget_node_from!(DragHandle);

// ── Charts panel ───────────────────────────────────────────────────────────

fn charts_panel() -> WidgetNode {
    panel(
        "Charts — CustomPaint",
        Flex::column().spacing(24.0).children(children![
            LineChart::new(vec![10.0, 25.0, 15.0, 30.0, 20.0, 35.0, 25.0, 40.0])
                .color(ACCENT)
                .line_width(2.0)
                .show_dots(true)
                .size(Size::new(600.0, 150.0)),
            BarChart::new(vec![15.0, 30.0, 20.0, 35.0, 25.0])
                .color(Color::rgb(168, 85, 247))
                .size(Size::new(600.0, 120.0)),
        ]),
    )
}
