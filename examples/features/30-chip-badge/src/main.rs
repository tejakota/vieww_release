use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("30 — chip & badge", Size::new(640.0, 200.0), |d| {
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
                            Chip::new("inbox"),
                            // The badge sits over its child's trailing corner, so the child is
                            // padded out of its way — a badge that covers the last two
                            // letters of the word it is counting is not a demonstration.
                            Badge::new(
                                Container::new()
                                    .padding(EdgeInsets::only(0.0, 6.0, 22.0, 0.0))
                                    .child(
                                        Text::new("unread")
                                            .color(Color::rgb(23, 30, 42))
                                            .size(13.0)
                                    ),
                            )
                            .count(12),
                        ]),
                ),
        );
    })
}
