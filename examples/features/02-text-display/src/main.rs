use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("02 — text", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::rgb(247, 248, 250))
                .padding(EdgeInsets::all(24.0))
                .child(
                    Text::new("the quick brown fox jumps over the lazy dog")
                        .color(Color::rgb(23, 30, 42))
                        .size(28.0)
                        .bold(),
                ),
        );
    })
}
