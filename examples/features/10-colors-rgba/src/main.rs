use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("10 — rgba", Size::new(480.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new().color(Color::WHITE).child(
                Stack::new()
                    .push(
                        Positioned::new().left(20.0).top(20.0).child(
                            Container::new()
                                .color(Color::rgba(255, 0, 0, 180))
                                .child(SizedBox::square(160.0)),
                        ),
                    )
                    .push(
                        Positioned::new().left(100.0).top(80.0).child(
                            Container::new()
                                .color(Color::rgba(0, 200, 0, 150))
                                .child(SizedBox::square(160.0)),
                        ),
                    )
                    .push(
                        Positioned::new().left(180.0).top(140.0).child(
                            Container::new()
                                .color(Color::rgba(0, 100, 255, 130))
                                .child(SizedBox::square(160.0)),
                        ),
                    ),
            ),
        );
    })
}
