use std::time::Duration;
use vieww_element::{TimelineBuilder, TimelinePlayer};
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

#[derive(Debug)]
struct Row {
    opacity: vieww_element::Signal<f32>,
}
impl vieww_widget::Widget for Row {
    fn debug_name(&self) -> &'static str {
        "Row"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        Opacity::new(self.opacity.get().clamp(0.0, 1.0))
            .child(
                Container::new()
                    .color(Color::rgb(58, 122, 246))
                    .radius(4.0)
                    .child(SizedBox::from_size(Size::new(440.0, 24.0))),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Row);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("41 — timeline", Size::new(520.0, 480.0), |d| {
        let runtime = d.elements().runtime().clone();
        let rows: Vec<vieww_element::Signal<f32>> =
            (0..8).map(|_| runtime.signal(0.0f32)).collect();
        let reveal: TimelinePlayer = TimelineBuilder::new()
            // 150ms rather than 40: at 40 the whole cascade is over inside the
            // first sampled frame, and a strip of five identical finished lists
            // is not a picture of a stagger.
            .stagger(Duration::from_millis(150), |s| {
                rows.iter().fold(s, |b, r| b.action(r, 0.0, 1.0))
            })
            .build()
            .attach(d.tickers());
        reveal.play();
        let row_widgets: Vec<WidgetNode> = rows
            .iter()
            .map(|r| WidgetNode::from(Row { opacity: r.clone() }))
            .collect();
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(8.0)
                        .children(children![
                            Button::new("Reveal").on_pressed(move || reveal.play()),
                            Flex::column().spacing(6.0).children(row_widgets),
                        ]),
                ),
        );
    })
}
