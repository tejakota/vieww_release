use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("23 — bar chart", Size::new(640.0, 280.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    BarChart::new(vec![15.0, 30.0, 20.0, 35.0, 25.0])
                        .color(Color::rgb(168, 85, 247))
                        .size(Size::new(600.0, 220.0)),
                ),
        );
    })
}
