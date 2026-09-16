use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("24 — line chart", Size::new(640.0, 280.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    LineChart::new(vec![10.0, 25.0, 15.0, 30.0, 20.0, 35.0, 25.0, 40.0])
                        .color(Color::rgb(58, 122, 246))
                        .line_width(2.0)
                        .show_dots(true)
                        .size(Size::new(600.0, 220.0)),
                ),
        );
    })
}
