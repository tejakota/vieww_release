use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("03 — text box", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .radius(8.0)
                .padding(EdgeInsets::symmetric(16.0, 12.0))
                .child(
                    Text::new("you can edit this placeholder")
                        .color(Color::rgb(110, 122, 140))
                        .size(16.0),
                ),
        );
    })
}
