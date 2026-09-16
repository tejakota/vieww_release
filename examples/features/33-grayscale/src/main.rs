use vieww_foundation::{Color, Image as Pixels, Size};
use vieww_widget::prelude::*;

fn sample(w: u32, h: u32) -> Pixels {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let check = ((x / 8) + (y / 8)) % 2 == 0;
            if check {
                v.extend_from_slice(&[60, 200, 60, 255]);
            } else {
                v.extend_from_slice(&[240, 80, 80, 255]);
            }
        }
    }
    Pixels::from_rgba8(v, w, h)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("33 — grayscale", Size::new(480.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    vieww_effects::FilterChain::new()
                        .filter(vieww_effects::Filter::grayscale())
                        .child(
                            SizedBox::from_size(Size::new(440.0, 200.0))
                                .child(Image::new(sample(96, 96)).fit(BoxFit::Fill)),
                        ),
                ),
        );
    })
}
