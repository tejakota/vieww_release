use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;
use vieww_widget::RouteTransition;

fn screen_card() -> WidgetNode {
    Container::new()
        .color(Color::rgb(58, 122, 246))
        .padding(EdgeInsets::all(12.0))
        .child(Text::new("Screen B").color(Color::WHITE).size(15.0).bold())
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("06 — fade transition", Size::new(900.0, 240.0), |d| {
        let cell: Size = Size::new(150.0, 110.0);
        let cells: Vec<WidgetNode> = [0.0f32, 0.25, 0.5, 0.75, 1.0]
            .iter()
            .map(|&t| {
                Container::new()
                    .color(Color::rgb(233, 237, 243))
                    .radius(6.0)
                    .child(Clip::rounded(6.0).child(SizedBox::from_size(cell).child(
                        Stack::new().push(RouteTransition::Fade.wrap(screen_card(), t, cell)),
                    )))
                    .into()
            })
            .collect();
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(Flex::row().spacing(10.0).children(cells)),
        );
    })
}
