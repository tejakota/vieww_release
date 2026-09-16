use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("00 — rectangle", Size::new(480.0, 320.0), |d| {
        feature_harness::set_page(d, Container::new().color(Color::rgb(58, 122, 246)));
    })
}
