use vieww_foundation::{Color, Size, TargetPlatform};
use vieww_widget::prelude::*;

fn row_of(data: vieww_widget::ThemeData) -> WidgetNode {
    Theme::new(data)
        .child(
            Container::new()
                .color(data.colors.surface)
                .radius(data.metrics.corner)
                .padding(EdgeInsets::all(16.0))
                .child(Flex::row().spacing(12.0).children(children![
                    Button::new("Primary"),
                    Chip::new("Chip"),
                    Badge::new(Text::new("in").color(data.colors.on_surface).size(13.0)).count(9),
                ])),
        )
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("21 — theme", Size::new(720.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::rgb(247, 248, 250))
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::row()
                        .spacing(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            row_of(vieww_widget::ThemeData::light()),
                            row_of(vieww_widget::ThemeData::dark()),
                            row_of(vieww_widget::ThemeData::adaptive(TargetPlatform::IOS, true)),
                        ]),
                ),
        );
    })
}
