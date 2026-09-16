//! Drawing something no widget covers, through `CustomPaint`.
//!
//! The painter is handed the size it will be painted at and returns
//! instructions. Everything here — the axis, the plotted line, the dots, the
//! faded band — is one list of `DrawInstruction`s, which is the escape hatch
//! for a chart or a diagram that is not worth a widget of its own.

use vieww_foundation::{Color, Offset, Rect, Size};
use vieww_widget::prelude::*;
use vieww_widget::DrawOnce;

const INK: Color = Color::rgb(23, 30, 42);
const ACCENT: Color = Color::rgb(58, 122, 246);
const CANVAS: Size = Size::new(600.0, 240.0);

/// A sine wave, its axis, and a band under it.
fn plot(size: Size) -> Vec<DrawInstruction> {
    let mid = size.height / 2.0;
    let mut out = vec![DrawInstruction::DrawLine {
        from: Offset::new(0.0, mid),
        to: Offset::new(size.width, mid),
        color: Color::rgb(203, 213, 225),
        width: 1.0,
    }];

    // The band first, so the line lands on top of it.
    let mut band = Vec::new();
    let steps = 60;
    for step in 0..steps {
        let t = step as f32 / steps as f32;
        let x = t * size.width;
        let y = mid - (t * std::f32::consts::TAU * 1.5).sin() * (mid - 20.0);
        band.push(DrawInstruction::FillRect {
            rect: Rect::from_origin_size(
                Offset::new(x, y.min(mid)),
                Size::new(size.width / steps as f32, (mid - y).abs()),
            ),
            color: ACCENT,
        });
    }
    out.push(DrawInstruction::Group {
        alpha: 0.18,
        instructions: band,
    });

    let mut previous: Option<Offset> = None;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let point = Offset::new(
            t * size.width,
            mid - (t * std::f32::consts::TAU * 1.5).sin() * (mid - 20.0),
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
        if step % 10 == 0 {
            out.push(DrawInstruction::FillCircle {
                center: point,
                radius: 4.0,
                color: INK,
            });
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("35 — custom paint", Size::new(660.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(30.0))
                .child(CustomPaint::sized(CANVAS, DrawOnce::new(plot))),
        );
    })
}
