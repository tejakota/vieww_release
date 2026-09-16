use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("17 — sized box", Size::new(480.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(Flex::row().spacing(16.0).children(children![
                        Container::new()
                            .color(Color::rgb(58, 122, 246))
                            .child(SizedBox::from_size(Size::new(120.0, 80.0))),
                        Container::new()
                            .color(Color::rgb(186, 100, 246))
                            .child(SizedBox::square(80.0)),
                        Container::new()
                            .color(Color::rgb(76, 187, 129))
                            .child(SizedBox::from_size(Size::new(60.0, 140.0))),
                    ])),
        );
    })
}
