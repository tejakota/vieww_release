//! The framework showcase: every feature, one app.
//!
//! ```console
//! cargo run -p full-showcase
//! ```
//!
//! Renders to a PNG through vieww's own rasterizer — no display, no GPU
//! adapter, no window. This is the smoke test: if this renders, every
//! subsystem composes.

use std::path::PathBuf;

use vieww::prelude::*;
use vieww_foundation::{Color, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::Carousel;
use vieww_widget::ColorPicker;
use vieww_widget::Grid;
use vieww_widget::{BarChart, LineChart};

const WIDTH: f32 = 720.0;
// Sized to the content: the frame driver logs a `RenderColumn overflowed`
// warning when the children do not fit, and the screenshot then shows a
// clipped panel. Content measured 1594 into 1552 of space at 1600.
const HEIGHT: f32 = 1720.0;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const PAPER: Color = Color::rgb(247, 248, 250);
const ACCENT: Color = Color::rgb(58, 122, 246);

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shots".into())
        .into();
    std::fs::create_dir_all(&out).expect("creating the output directory");

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let mut renderer = NativeRenderer::new();

    driver.elements().set_root(screen());
    driver.draw_frame();

    let (png, report) = renderer
        .render_to_png(driver.scene(), WIDTH as u32, HEIGHT as u32, Color::WHITE)
        .expect("rasterising");

    let path = out.join("full-showcase.png");
    std::fs::write(&path, png).expect("writing the PNG");

    println!(
        "wrote {}: {} shapes, {} glyph runs ({} glyphs), {} clips, {} shadows, {} layers",
        path.display(),
        report.shapes,
        report.glyph_runs,
        report.glyphs,
        report.clips,
        report.shadows,
        report.layers
    );
}

fn screen() -> WidgetNode {
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(24.0)
                .children(children![
                    Text::new("vieww — full framework showcase")
                        .color(INK)
                        .size(24.0)
                        .bold(),
                    Text::new("charts, colour picker, grid, carousel, morphing — one screenshot")
                        .color(MUTED)
                        .size(13.0),
                    section(
                        "Charts",
                        children![
                            LineChart::new(vec![10.0, 25.0, 15.0, 30.0, 20.0, 35.0, 25.0])
                                .color(ACCENT)
                                .line_width(2.0)
                                .show_dots(true)
                                .size(Size::new(600.0, 140.0)),
                            BarChart::new(vec![15.0, 30.0, 20.0, 35.0, 25.0])
                                .color(Color::rgb(168, 85, 247))
                                .size(Size::new(600.0, 100.0)),
                        ]
                    ),
                    section("Colour Picker", children![ColorPicker::new(ACCENT),]),
                    section(
                        "Grid (3 columns)",
                        children![Grid::columns(3).gap(12.0).children(vec![
                            grid_card("One"),
                            grid_card("Two"),
                            grid_card("Three"),
                            grid_card("Four"),
                            grid_card("Five"),
                            grid_card("Six"),
                        ]),]
                    ),
                    section(
                        "Carousel",
                        children![Carousel::new(600.0, 16.0).viewport_height(200.0).children(
                            vec![
                                carousel_card("First"),
                                carousel_card("Second"),
                                carousel_card("Third"),
                                carousel_card("Fourth"),
                            ]
                        ),]
                    ),
                    section(
                        "Shape Morphing",
                        children![
                            // Circle → square at t=0.5 (a squircle).
                            vieww_widget::ShapeMorph::new(
                                vieww_widget::Circle,
                                vieww_widget::RoundedRectangle { radius: 0.0 },
                            )
                            .progress(0.5)
                            .size(Size::new(120.0, 120.0))
                            .child(
                                Container::new()
                                    .color(Color::rgb(186, 100, 246))
                                    .child(SizedBox::square(120.0)),
                            ),
                        ]
                    ),
                ]),
        )
        .into()
}

fn section(title: &str, children: Vec<WidgetNode>) -> WidgetNode {
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(16.0)
        .push(Text::new(title).color(INK).size(18.0).bold());

    for child in children {
        column = column.push(child);
    }

    Container::new()
        .color(Color::WHITE)
        .radius(12.0)
        .padding(EdgeInsets::all(20.0))
        .child(column)
        .into()
}

fn grid_card(label: &str) -> WidgetNode {
    Container::new()
        .color(Color::rgb(241, 245, 249))
        .radius(8.0)
        .padding(EdgeInsets::all(16.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(8.0)
                .children(children![
                    Container::new()
                        .color(ACCENT)
                        .radius(6.0)
                        .child(SizedBox::square(32.0)),
                    Text::new(label).color(INK).size(14.0),
                    Text::new("A card in the grid").color(MUTED).size(12.0),
                ]),
        )
        .into()
}

fn carousel_card(label: &str) -> WidgetNode {
    Container::new()
        .color(Color::rgb(241, 245, 249))
        .radius(12.0)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(12.0)
                .children(children![
                    Container::new()
                        .color(ACCENT)
                        .radius(8.0)
                        .child(SizedBox::from_size(Size::new(80.0, 80.0))),
                    Text::new(label).color(INK).size(18.0).bold(),
                    Text::new("A card in the carousel").color(MUTED).size(13.0),
                ]),
        )
        .into()
}
