use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn tile(c: Color) -> WidgetNode {
    Container::new()
        .color(c)
        .radius(8.0)
        .child(SizedBox::square(80.0))
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("12 — flex row", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(Flex::row().spacing(16.0).children(children![
                    tile(Color::rgb(58, 122, 246)),
                    tile(Color::rgb(186, 100, 246)),
                    tile(Color::rgb(76, 187, 129)),
                    tile(Color::rgb(255, 180, 80)),
                ])),
        );
    })
}
