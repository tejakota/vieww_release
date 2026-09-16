//! A window you can drag a card around in, and the frame rate it ran at.
//!
//! ```console
//! # interactive: drag the green card, let go, watch it spring back
//! cargo run -p vieww-platform-winit --example hello --release
//!
//! # unattended: 600 frames, then print the report and exit
//! cargo run -p vieww-platform-winit --example hello --release -- --frames 600
//! ```
//!
//! # What this is for
//!
//! Phase 4's exit test asks for "a full-screen widget tree at 60fps on device".
//! Half of it was met headless — paint, rasterise, read the pixels back — and
//! the other half could not be attempted, because there was no window to be at
//! 60fps *in*. This is that window, and `--frames` makes the measurement
//! unattended so it can be recorded rather than eyeballed.
//!
//! The tree is deliberately the Phase 7 exit test's: a card that springs when
//! released, and a band that rebuilds on every single frame regardless. That
//! second part is the load-bearing half — a frame rate measured on a still
//! screen measures nothing, since the whole framework is built to make that
//! frame free.
//!
//! # Run it in release
//!
//! Debug builds of vello and of the layout pass are between ten and thirty
//! times slower, and a jank count from one is a report on `rustc -O0`.

use std::time::Duration;

use vieww_animation::{Spring, Tween};
use vieww_element::{Animation, Signal};
use vieww_foundation::{Axis, Color, EdgeInsets, Size};
use vieww_platform_winit::App;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 640.0,
    height: 480.0,
};
const CARD: f32 = 80.0;
/// How far the card can travel, in pixels.
const TRAVEL: f32 = SURFACE.width - CARD;

/// A band whose colour changes every frame, so the screen is never still.
///
/// Driven by an animation that repeats forever rather than by a counter someone
/// increments: that is the only thing in the framework that guarantees a frame
/// on every vsync, and a frame rate measured on a screen that is allowed to go
/// idle is a measurement of the idle path.
///
/// # The animation is held here, and that is not incidental
///
/// `Tickers` keeps a `Weak` to everything it drives, so an `Animation` stops the
/// moment the last strong handle goes — silently, with no panic and no warning,
/// leaving a window that renders one frame and then freezes. Holding it in the
/// widget that displays it is the ownership that makes that impossible.
#[derive(Debug)]
struct Ticker {
    /// Read for its value; held so it stays alive.
    pulse: Animation<f32>,
}

impl Widget for Ticker {
    fn debug_name(&self) -> &'static str {
        "Ticker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // The read is the subscription: the animation's write marks this
        // element and nothing else, which is what makes a per-frame rebuild
        // cost the band rather than the screen.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a tween bounded to 0..=255"
        )]
        let step = self.pulse.value().clamp(0.0, 255.0) as u8;
        RepaintBoundary::new()
            .child(
                ColoredBox::new(Color::rgb(step, 40, 255 - step))
                    .child(SizedBox::from_size(Size::new(SURFACE.width, 60.0))),
            )
            .into()
    }
}

widget_node_from!(Ticker);

/// The card, inset from the left by however far it has been dragged.
#[derive(Debug)]
struct Card {
    offset: Signal<f32>,
    /// Built once and cloned, so reconciliation skips it by pointer equality
    /// and the card *moves* rather than being repainted.
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
        Padding::new(EdgeInsets::only(self.offset.get(), 20.0, 0.0, 0.0))
            .child(self.content.clone())
            .into()
    }
}

widget_node_from!(Card);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames = frames_from_args();

    let mut app = App::new()
        .title("vieww — drag the card")
        .size(SURFACE)
        .background(Color::rgb(18, 18, 22));
    if let Some(frames) = frames {
        app = app.exit_after_frames(frames);
    }

    let report = app.run(|driver| {
        // 0 is home, 1 is hard against the right-hand edge. A drag and a spring
        // both drive this one number, which is why letting go can hand the
        // gesture's velocity straight to the physics.
        let slide = driver.animation(Tween::new(0.0_f32, TRAVEL), Duration::from_millis(250));
        let offset = slide.signal();

        // Never settles, so the framework never stops asking for frames.
        let pulse = driver.animation(Tween::new(0.0_f32, 255.0), Duration::from_secs(2));
        pulse.repeat(true, Duration::ZERO);

        let dragging = slide.clone();
        let released = slide.clone();

        let card = Card {
            offset,
            content: RepaintBoundary::new()
                .child(ColoredBox::new(Color::GREEN).child(SizedBox::square(CARD)))
                .into(),
        };

        // Left-aligned rather than centred: a `Flex` centres on its cross axis,
        // which would halve the card's travel and start it in the middle.
        let root = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(children![
                Ticker { pulse },
                GestureDetector::new()
                    .drag_axis(Axis::Horizontal)
                    .on_drag_start({
                        let slide = dragging.clone();
                        move |details| {
                            // The finger *is* the animation while it is down;
                            // whatever the spring was doing yields to it.
                            slide.set_progress(
                                (slide.progress() + details.delta.dx / TRAVEL).clamp(0.0, 1.0),
                            );
                        }
                    })
                    .on_drag_update(move |details| {
                        dragging.set_progress(
                            (dragging.progress() + details.delta.dx / TRAVEL).clamp(0.0, 1.0),
                        );
                    })
                    .on_drag_end(move |details| {
                        // Pixels a second into units of the whole travel, or
                        // the spring is launched hundreds of times too hard.
                        released.animate_with(
                            Spring::settling(
                                released.progress(),
                                0.0,
                                details.velocity.dx / TRAVEL,
                            ),
                            details.timestamp,
                        );
                    })
                    .child(card),
            ]);

        // `FrameDriver::set_root`, not `driver.elements().set_root`. The driver
        // keeps the root so it can re-publish it under an `Inherited<ViewMetrics>`
        // whenever the surface changes; going straight to the element tree leaves
        // it with no root to re-publish, and every safe area in the application
        // silently reads zero forever. This example would not notice — it draws
        // no `SafeArea` — which is exactly how the mistake got copied out of here
        // and onto a phone.
        driver.set_root(root);
    })?;

    println!("{report}");

    if frames.is_some() && !report.is_smooth() {
        eprintln!(
            "warning: {} of {} frames ran over the {:.2}ms budget",
            report.over_budget,
            report.frames,
            report.budget.as_secs_f64() * 1000.0
        );
    }
    Ok(())
}

/// `--frames N`, if it was passed.
fn frames_from_args() -> Option<u64> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|count| count.parse().ok());
        }
    }
    None
}
