use vieww_foundation::{Color, Offset, Size};
use vieww_widget::prelude::*;

fn box_of(c: Color) -> WidgetNode {
    Container::new()
        .color(c)
        .radius(8.0)
        .child(SizedBox::square(80.0))
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("20 — transform", Size::new(480.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Stack::new().push(box_of(Color::rgb(186, 100, 246))).push(
                        Transformed::translate(Offset::new(80.0, 60.0))
                            .child(box_of(Color::rgb(58, 122, 246))),
                    ),
                ),
        );
    })
}
