use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn bar(w: f32, c: Color) -> WidgetNode {
    Container::new()
        .color(c)
        .radius(4.0)
        .child(SizedBox::from_size(Size::new(w, 24.0)))
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("13 — flex column", Size::new(640.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(12.0)
                        .children(children![
                            bar(480.0, Color::rgb(58, 122, 246)),
                            bar(360.0, Color::rgb(186, 100, 246)),
                            bar(280.0, Color::rgb(76, 187, 129)),
                            bar(180.0, Color::rgb(255, 180, 80)),
                        ]),
                ),
        );
    })
}
