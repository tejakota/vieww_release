use vieww_foundation::{Color, Size, TextDirection};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("22 — RTL", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Directionality::new(TextDirection::Rtl).child(
                        Container::new()
                            .color(Color::rgb(237, 241, 247))
                            .radius(6.0)
                            .padding(EdgeInsets::all(12.0))
                            .child(
                                Text::new("שלום עולם — مرحبا بالعالم")
                                    .color(Color::rgb(23, 30, 42))
                                    .size(22.0),
                            ),
                    ),
                ),
        );
    })
}
