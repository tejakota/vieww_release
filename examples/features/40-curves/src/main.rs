//! What each easing curve actually does, plotted.
//!
//! `Curve::transform` maps progress to eased progress, so a curve can be drawn
//! without animating anything: sample it across `0..=1` and the shape is the
//! answer. Linear is the straight diagonal every other cell is compared
//! against — if they all look like it, `transform` is returning its input.

use vieww::animation::Curve;
use vieww_foundation::{Color, Offset, Size};
use vieww_widget::prelude::*;
use vieww_widget::DrawOnce;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const ACCENT: Color = Color::rgb(58, 122, 246);
const PLOT: Size = Size::new(150.0, 150.0);

fn cell(label: &str, curve: Curve) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .children(children![
            Text::new(label).color(INK).size(12.0).bold(),
            Container::new()
                .color(Color::rgb(247, 248, 250))
                .radius(8.0)
                .child(CustomPaint::sized(
                    PLOT,
                    DrawOnce::new(move |size: Size| {
                        let mut out = vec![
                            // The diagonal is linear, drawn under every curve so
                            // the eased one can be read against it.
                            DrawInstruction::DrawLine {
                                from: Offset::new(0.0, size.height),
                                to: Offset::new(size.width, 0.0),
                                color: Color::rgb(214, 221, 231),
                                width: 1.0,
                            },
                        ];
                        let steps = 48;
                        let mut previous: Option<Offset> = None;
                        for step in 0..=steps {
                            #[allow(clippy::cast_precision_loss)]
                            let t = step as f32 / steps as f32;
                            let point = Offset::new(
                                t * size.width,
                                size.height - curve.transform(t) * size.height,
                            );
                            if let Some(from) = previous {
                                out.push(DrawInstruction::DrawLine {
                                    from,
                                    to: point,
                                    color: ACCENT,
                                    width: 2.5,
                                });
                            }
                            previous = Some(point);
                        }
                        out
                    }),
                )),
        ])
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("40 — curves", Size::new(880.0, 260.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(10.0)
                        .children(children![
                            Text::new("eased progress against linear")
                                .color(MUTED)
                                .size(12.0),
                            Flex::row()
                                .spacing(16.0)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(children![
                                    cell("Linear", Curve::Linear),
                                    cell("EASE_IN", Curve::EASE_IN),
                                    cell("EASE_OUT", Curve::EASE_OUT),
                                    cell("EASE_IN_OUT", Curve::EASE_IN_OUT),
                                    cell("FAST_OUT_SLOW_IN", Curve::FAST_OUT_SLOW_IN),
                                ]),
                        ]),
                ),
        );
    })
}
