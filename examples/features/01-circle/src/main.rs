//! A circle: a container with a corner radius of half its side.
//!
//! The `Center` is load-bearing. A `Container` takes every pixel its parent
//! offers whatever its child asks for, so without it this is a 480×320
//! rounded rectangle rather than a 160pt circle — the same code, and a
//! completely different picture.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("01 — circle", Size::new(480.0, 320.0), |d| {
        feature_harness::set_page(
            d,
            Container::new().color(Color::WHITE).child(
                Center::new().child(
                    Container::new()
                        .color(Color::rgb(186, 100, 246))
                        .radius(80.0)
                        .child(SizedBox::square(160.0)),
                ),
            ),
        );
    })
}
