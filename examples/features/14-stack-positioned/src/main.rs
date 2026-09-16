use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn dot(x: f32, y: f32, c: Color) -> WidgetNode {
    Positioned::new()
        .left(x)
        .top(y)
        .child(
            Container::new()
                .color(c)
                .radius(20.0)
                .child(SizedBox::square(40.0)),
        )
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("14 — stack", Size::new(480.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new().color(Color::rgb(247, 248, 250)).child(
                Stack::new()
                    .push(dot(40.0, 40.0, Color::rgb(58, 122, 246)))
                    .push(dot(200.0, 80.0, Color::rgb(186, 100, 246)))
                    .push(dot(120.0, 180.0, Color::rgb(76, 187, 129)))
                    .push(dot(340.0, 220.0, Color::rgb(255, 180, 80))),
            ),
        );
    })
}
