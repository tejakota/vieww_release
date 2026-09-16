use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn row(label: &str, m: vieww_widget::Motion) -> WidgetNode {
    Container::new()
        .color(Color::WHITE)
        .radius(10.0)
        .padding(EdgeInsets::all(16.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(6.0)
                .children(children![
                    Text::new(label)
                        .color(Color::rgb(23, 30, 42))
                        .size(15.0)
                        .bold(),
                    Text::new(format!(
                        "spatial damping: {:.2}",
                        m.spatial_default.spec().damping_ratio
                    ))
                    .color(Color::rgb(110, 122, 140))
                    .size(12.0),
                    Text::new(format!(
                        "effects damping: {:.2}",
                        m.effects_default.spec().damping_ratio
                    ))
                    .color(Color::rgb(110, 122, 140))
                    .size(12.0),
                ]),
        )
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("46 — motion tokens", Size::new(640.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::rgb(247, 248, 250))
                .padding(EdgeInsets::all(20.0))
                .child(Flex::column().spacing(12.0).children(children![
                    row("Motion::standard()", vieww_widget::Motion::standard()),
                    row("Motion::expressive()", vieww_widget::Motion::expressive()),
                ])),
        );
    })
}
