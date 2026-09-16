use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn swatch(c: Color) -> WidgetNode {
    Container::new()
        .color(c)
        .radius(8.0)
        .child(SizedBox::square(80.0))
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("09 — rgb", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(Flex::row().spacing(12.0).children(children![
                    swatch(Color::rgb(255, 0, 0)),
                    swatch(Color::rgb(0, 200, 0)),
                    swatch(Color::rgb(0, 100, 255)),
                    swatch(Color::rgb(255, 200, 0)),
                    swatch(Color::rgb(200, 0, 200)),
                    swatch(Color::rgb(0, 0, 0)),
                ])),
        );
    })
}
