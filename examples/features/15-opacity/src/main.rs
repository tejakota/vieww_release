use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn cell(a: f32) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .children(children![
            Text::new(format!("alpha {a}"))
                .color(Color::rgb(110, 122, 140))
                .size(11.0),
            Opacity::new(a).child(
                Container::new()
                    .color(Color::rgb(58, 122, 246))
                    .radius(8.0)
                    .child(SizedBox::square(80.0))
            ),
        ])
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("15 — opacity", Size::new(640.0, 220.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::row()
                        .spacing(20.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![cell(1.0), cell(0.7), cell(0.4), cell(0.15),]),
                ),
        );
    })
}
