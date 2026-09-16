use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;
use vieww_widget::RouteTransition;

fn card() -> WidgetNode {
    Container::new()
        .color(Color::rgb(186, 100, 246))
        .padding(EdgeInsets::all(12.0))
        .child(Text::new("B").color(Color::WHITE).size(15.0).bold())
        .into()
}
fn film(label: &str, t: f32, rt: RouteTransition, surface: Size) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .children(children![
            Text::new(format!("{label} t={t}"))
                .color(Color::rgb(110, 122, 140))
                .size(11.0),
            Container::new()
                .color(Color::rgb(233, 237, 243))
                .radius(6.0)
                .child(Clip::rounded(6.0).child(
                    SizedBox::from_size(surface).child(Stack::new().push(rt.wrap(
                        card(),
                        t,
                        surface
                    )))
                )),
        ])
        .into()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("07 — slide transitions", Size::new(980.0, 360.0), |d| {
        let s = Size::new(150.0, 110.0);
        let ts = [0.0f32, 0.25, 0.5, 0.75, 1.0];
        let row_a: Vec<WidgetNode> = ts
            .iter()
            .map(|&t| film("SlideFromEnd", t, RouteTransition::SlideFromEnd, s))
            .collect();
        let row_b: Vec<WidgetNode> = ts
            .iter()
            .map(|&t| film("SlideFromBottom", t, RouteTransition::SlideFromBottom, s))
            .collect();
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(Flex::column().spacing(20.0).children(children![
                    Flex::row().spacing(10.0).children(row_a),
                    Flex::row().spacing(10.0).children(row_b),
                ])),
        );
    })
}
