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
fn cell(label: &str, fit: BoxFit) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(label)
                .color(Color::rgb(23, 30, 42))
                .size(13.0)
                .bold(),
            Container::new()
                .color(Color::rgb(226, 232, 240))
                .radius(8.0)
                .child(
                    Clip::rounded(8.0).child(
                        SizedBox::from_size(Size::new(160.0, 110.0))
                            .child(Image::new(sample(64, 64)).fit(fit))
                    )
                ),
        ])
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("19 — box fit", Size::new(760.0, 240.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::row()
                        .spacing(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            cell("Fill", BoxFit::Fill),
                            cell("Contain", BoxFit::Contain),
                            cell("Cover", BoxFit::Cover),
                            cell("None", BoxFit::None),
                        ]),
                ),
        );
    })
}
