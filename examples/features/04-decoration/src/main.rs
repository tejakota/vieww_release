//! Everything a `BoxDecoration` can put on a box: a gradient, a border, a
//! corner radius and a shadow.
//!
//! One row per feature, and each cell is otherwise identical — so a translator
//! that drops shadows, or a gradient that collapses to its first stop, shows up
//! as one cell looking like the flat one next to it.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const ACCENT: Color = Color::rgb(58, 122, 246);

fn cell(label: &str, decoration: BoxDecoration) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .children(children![
            Text::new(label).color(MUTED).size(12.0),
            DecoratedBox::new(decoration).child(SizedBox::from_size(Size::new(150.0, 96.0))),
        ])
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("04 — decoration", Size::new(760.0, 380.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(20.0)
                        .children(children![
                            Text::new("BoxDecoration").color(INK).size(18.0).bold(),
                            Flex::row()
                                .spacing(20.0)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(children![
                                    cell("color", BoxDecoration::new().color(ACCENT)),
                                    cell(
                                        "radius",
                                        BoxDecoration::new().color(ACCENT).radius(16.0),
                                    ),
                                    cell(
                                        "border",
                                        BoxDecoration::new()
                                            .color(Color::WHITE)
                                            .radius(16.0)
                                            .border(Border::new(ACCENT, 3.0)),
                                    ),
                                    cell(
                                        "gradient",
                                        BoxDecoration::new().radius(16.0).gradient(
                                            Gradient::vertical()
                                                .between(ACCENT, Color::rgb(186, 100, 246)),
                                        ),
                                    ),
                                ]),
                            Flex::row()
                                .spacing(20.0)
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .children(children![
                                    cell(
                                        "shadow",
                                        BoxDecoration::new()
                                            .color(Color::WHITE)
                                            .radius(16.0)
                                            .shadow(Shadow::new(
                                                Color::rgba(23, 30, 42, 60),
                                                Offset::new(0.0, 8.0),
                                                18.0,
                                            )),
                                    ),
                                    cell(
                                        "horizontal gradient",
                                        BoxDecoration::new().radius(16.0).gradient(
                                            Gradient::horizontal()
                                                .between(Color::rgb(76, 187, 129), ACCENT),
                                        ),
                                    ),
                                    cell(
                                        "all of it",
                                        BoxDecoration::new()
                                            .radius(16.0)
                                            .gradient(
                                                Gradient::vertical()
                                                    .between(ACCENT, Color::rgb(126, 87, 246)),
                                            )
                                            .border(Border::new(INK, 2.0))
                                            .shadow(Shadow::new(
                                                Color::rgba(23, 30, 42, 70),
                                                Offset::new(0.0, 10.0),
                                                20.0,
                                            )),
                                    ),
                                ]),
                        ]),
                ),
        );
    })
}
