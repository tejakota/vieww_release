use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("16 — clip", Size::new(480.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Clip::rounded(24.0).child(
                        Container::new()
                            .color(Color::rgb(186, 100, 246))
                            .child(SizedBox::from_size(Size::new(440.0, 200.0))),
                    ),
                ),
        );
    })
}
