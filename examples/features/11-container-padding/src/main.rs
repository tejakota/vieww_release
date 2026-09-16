use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn row(pad: EdgeInsets, label: &str) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .children(children![
            Text::new(label).color(Color::rgb(110, 122, 140)).size(11.0),
            Container::new()
                .color(Color::rgb(58, 122, 246))
                .padding(pad)
                .child(
                    Container::new()
                        .color(Color::rgb(23, 30, 42))
                        .radius(2.0)
                        .child(SizedBox::from_size(Size::new(80.0, 50.0)))
                ),
        ])
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("11 — padding", Size::new(640.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::rgb(247, 248, 250))
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::row()
                        .spacing(24.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            row(EdgeInsets::ZERO, "zero"),
                            row(EdgeInsets::all(12.0), "all 12"),
                            row(EdgeInsets::symmetric(24.0, 8.0), "sym 24x8"),
                        ]),
                ),
        );
    })
}
