use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("29 — progress", Size::new(640.0, 200.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(16.0)
                        .children(children![
                            LinearProgress::new(0.2),
                            LinearProgress::new(0.6),
                            LinearProgress::new(0.95),
                        ]),
                ),
        );
    })
}
