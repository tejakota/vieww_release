use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("25 — button & pressable", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::row()
                        .spacing(20.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            Button::new("Tap me").on_pressed(move || { /* action */ }),
                            Pressable::new(move |_p| {
                                Container::new()
                                    .color(Color::rgb(58, 122, 246))
                                    .radius(8.0)
                                    .padding(EdgeInsets::symmetric(16.0, 10.0))
                                    .child(Text::new("hold me").color(Color::WHITE).size(14.0))
                                    .into()
                            }),
                        ]),
                ),
        );
    })
}
