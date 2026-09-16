//! Every capability the `Painting` widget claims, in one picture.
//!
//! ```console
//! cargo run -p vector-probe -- shots/
//! ```
//!
//! Each panel exercises one thing a `Sketchbook` can record and the render
//! object has to translate. A panel that comes out blank is a translation that
//! was never written; a panel that comes out wrong is one that was written
//! wrong. Both are visible at a glance, which is the whole reason this is a
//! picture rather than an assertion on a display list.

use std::f32::consts::TAU;
use std::path::PathBuf;

use vieww::foundation::{
    parse_path_data, Color, Gradient, Offset, Path, Rect, Shadow, Size, Sketchbook, Transform,
};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;
use vieww::render::FrameDriver;
use vieww::widget::{PaintWith, Painting, WidgetNode};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 620.0;

const INK: Color = Color::rgb(232, 234, 240);
const MUTED: Color = Color::rgb(140, 145, 160);
const PAPER: Color = Color::rgb(16, 17, 21);
const CARD: Color = Color::rgb(27, 28, 34);
const ACCENT: Color = Color::rgb(91, 157, 249);
const VIOLET: Color = Color::rgb(167, 120, 246);

/// One labelled cell.
fn panel(title: &str, art: WidgetNode) -> WidgetNode {
    Container::new()
        .color(CARD)
        .radius(12.0)
        .padding(EdgeInsets::all(12.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(8.0)
                .children(children![
                    Text::new(title.to_owned()).size(11.0).color(MUTED),
                    Flexible::expanded(1).child(art),
                ]),
        )
        .into()
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shots".into())
        .into();
    std::fs::create_dir_all(&out).expect("creating the output directory");

    let grid = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .spacing(12.0)
        .children(children![
            Flexible::expanded(1).child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(12.0)
                    .children(children![
                        Flexible::expanded(1).child(panel("linear gradient", linear())),
                        Flexible::expanded(1).child(panel("radial + sweep", radial())),
                        Flexible::expanded(1).child(panel("strokes", strokes())),
                    ])
            ),
            Flexible::expanded(1).child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(12.0)
                    .children(children![
                        Flexible::expanded(1).child(panel("arcs and rings", arcs())),
                        Flexible::expanded(1).child(panel("svg path data", svg_path())),
                        Flexible::expanded(1).child(panel("shadow", shadow())),
                    ])
            ),
            Flexible::expanded(1).child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .spacing(12.0)
                    .children(children![
                        Flexible::expanded(1).child(panel("blurred layer", glow())),
                        Flexible::expanded(1).child(panel("clipped group", clipped())),
                        Flexible::expanded(1).child(panel("transformed group", spun())),
                    ])
            ),
        ]);

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    driver.set_root(
        Container::new()
            .color(PAPER)
            .padding(EdgeInsets::all(16.0))
            .child(grid),
    );
    driver.draw_frame();
    driver.draw_frame();

    let mut renderer = NativeRenderer::new();
    let (png, report) = renderer
        .render_to_png(driver.scene(), WIDTH as u32, HEIGHT as u32, PAPER)
        .expect("rasterising through vieww's own renderer");
    let path = out.join("vector-probe.png");
    std::fs::write(&path, png).expect("writing the PNG");
    println!(
        "{}: {} shapes, {} glyph runs, {} layers",
        path.display(),
        report.shapes,
        report.glyph_runs,
        report.layers
    );
}

fn linear() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        book.rrect(
            Rect::new(0.0, 0.0, size.width, size.height),
            10.0,
            Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0)).with_stops(&[
                (0.0, ACCENT),
                (0.55, VIOLET),
                (1.0, Color::rgb(240, 120, 170)),
            ]),
        );
    }))
    .into()
}

fn radial() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let half = size.width / 2.0;
        book.rrect(
            Rect::new(0.0, 0.0, half - 4.0, size.height),
            8.0,
            Gradient::radial_fill().with_stops(&[
                (0.0, Color::rgb(255, 255, 255)),
                (0.5, ACCENT),
                (1.0, Color::rgb(20, 30, 70)),
            ]),
        );
        book.rrect(
            Rect::new(half + 4.0, 0.0, size.width, size.height),
            8.0,
            Gradient::conic().with_stops(&[
                (0.0, ACCENT),
                (0.33, VIOLET),
                (0.66, Color::rgb(120, 230, 190)),
                (1.0, ACCENT),
            ]),
        );
    }))
    .into()
}

fn strokes() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        for (index, width) in [0.75_f32, 1.5, 3.0, 6.0].iter().enumerate() {
            let y = 10.0 + index as f32 * (size.height - 20.0) / 3.0;
            let mut wave = Path::new();
            wave.move_to(Offset::new(4.0, y));
            wave.cubic_to(
                Offset::new(size.width * 0.3, y - 18.0),
                Offset::new(size.width * 0.7, y + 18.0),
                Offset::new(size.width - 4.0, y),
            );
            book.stroke(wave, if index % 2 == 0 { ACCENT } else { VIOLET }, *width);
        }
        book.stroke_rrect(
            Rect::new(1.0, 1.0, size.width - 1.0, size.height - 1.0),
            8.0,
            MUTED,
            1.0,
        );
    }))
    .into()
}

fn arcs() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let center = Offset::new(size.width / 2.0, size.height / 2.0);
        let radius = size.height.min(size.width) / 2.0 - 6.0;
        book.ring(center, radius, 7.0, Color::rgb(45, 47, 56));
        book.arc(
            center,
            radius,
            7.0,
            -TAU / 4.0,
            TAU * 0.68,
            Gradient::conic().between(ACCENT, VIOLET),
        );
        book.circle(center, radius * 0.45, Color::rgb(35, 37, 45));
        book.arc(center, radius * 0.45, 3.0, TAU / 4.0, TAU * 0.3, VIOLET);
    }))
    .into()
}

fn svg_path() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        // A heart, entirely in cubics and arcs, parsed from `d` at paint time.
        let heart = parse_path_data(
            "M12 21.35 10.55 20.03C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 \
             4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 \
             11.54L12 21.35z",
        )
        .expect("a heart");
        let side = size.width.min(size.height) - 8.0;
        let fitted = heart.fitted(
            Rect::new(0.0, 0.0, 24.0, 24.0),
            Rect::from_origin_size(
                Offset::new((size.width - side) / 2.0, (size.height - side) / 2.0),
                Size::square(side),
            ),
        );
        book.fill(
            fitted,
            Gradient::vertical().between(Color::rgb(255, 120, 150), Color::rgb(200, 40, 110)),
        );
    }))
    .into()
}

fn shadow() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let card = Rect::new(14.0, 10.0, size.width - 14.0, size.height - 18.0);
        book.shadow(
            card,
            10.0,
            Shadow::new(Color::rgba(0, 0, 0, 190), Offset::new(0.0, 8.0), 20.0),
        );
        book.rrect(card, 10.0, Color::rgb(58, 60, 72));
        book.shadow(
            card,
            10.0,
            Shadow::inset(Color::rgba(255, 255, 255, 40), Offset::new(0.0, 1.0), 2.0),
        );
    }))
    .into()
}

fn glow() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let center = Offset::new(size.width / 2.0, size.height / 2.0);
        book.layer(0.9, 9.0, None, |inner| {
            inner.circle(center, size.height.min(size.width) / 3.0, ACCENT);
        });
        book.circle(center, size.height.min(size.width) / 5.0, Color::WHITE);
    }))
    .into()
}

fn clipped() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let center = Offset::new(size.width / 2.0, size.height / 2.0);
        let radius = size.height.min(size.width) / 2.0 - 4.0;
        let disc = Path::arc(center, radius, 0.0, TAU);
        book.layer(1.0, 0.0, Some(disc), |inner| {
            for index in 0..9 {
                let x = index as f32 * size.width / 8.0;
                inner.rect(
                    Rect::new(x, 0.0, x + size.width / 16.0, size.height),
                    if index % 2 == 0 { ACCENT } else { VIOLET },
                );
            }
        });
    }))
    .into()
}

fn spun() -> WidgetNode {
    Painting::new(PaintWith::new(|book: &mut Sketchbook, size: Size| {
        let center = Offset::new(size.width / 2.0, size.height / 2.0);
        for index in 0..6 {
            let angle = index as f32 * TAU / 12.0;
            book.transformed(Transform::rotate_around(center, angle), |inner| {
                inner.rrect(
                    Rect::new(
                        center.dx - 3.0,
                        center.dy - size.height / 2.0 + 6.0,
                        center.dx + 3.0,
                        center.dy,
                    ),
                    3.0,
                    Color::rgba(ACCENT.r, ACCENT.g, ACCENT.b, 70 + index as u8 * 30),
                );
            });
        }
        book.circle(center, 5.0, INK);
    }))
    .into()
}
