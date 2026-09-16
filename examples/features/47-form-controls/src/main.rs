//! The value-bearing controls, each shown at more than one value.
//!
//! Every one of these is **controlled**: it is told its value and reports the
//! change it was asked for. Nothing here holds state of its own, which is why
//! a row of them at four different values is a fair picture of the widget
//! rather than four copies of its default.

use std::rc::Rc;

use vieww_element::Signal;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);

fn row(label: &str, child: impl Into<WidgetNode>) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .children(children![
            Text::new(label).color(MUTED).size(12.0),
            child.into(),
        ])
        .into()
}

#[derive(Debug)]
struct Form {
    volume: Signal<f32>,
    tab: Signal<usize>,
}

impl vieww_widget::Widget for Form {
    fn debug_name(&self) -> &'static str {
        "Form"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let volume = self.volume.get();
        let tab = self.tab.get();

        let on_volume = {
            let held = self.volume.clone();
            Rc::new(move |value: f32| held.set(value))
        };
        let on_tab = {
            let held = self.tab.clone();
            Rc::new(move |index: usize| held.set(index))
        };

        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(24.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(20.0)
                    .children(children![
                        Text::new("controlled inputs").color(INK).size(18.0).bold(),
                        row(
                            "Slider — value, range and divisions",
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .spacing(14.0)
                                .children(children![
                                    SizedBox::from_size(Size::new(360.0, 40.0)).child(
                                        Slider::new(volume).range(0.0, 1.0).on_changed(on_volume),
                                    ),
                                    SizedBox::from_size(Size::new(360.0, 40.0))
                                        .child(Slider::new(0.5).range(0.0, 1.0).divisions(4)),
                                ]),
                        ),
                        row(
                            "SegmentedControl",
                            SegmentedControl::new(["Day", "Week", "Month"], tab)
                                .on_selected(on_tab),
                        ),
                        row(
                            "TextField",
                            SizedBox::from_size(Size::new(360.0, 44.0))
                                .child(TextField::text("hello@example.com").show_cursor(true)),
                        ),
                    ]),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Form);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch_with("47 — form controls", Size::new(560.0, 420.0), |d| {
        let runtime = d.elements().runtime().clone();
        let volume = runtime.signal(0.25f32);
        let tab = runtime.signal(0usize);
        feature_harness::set_page(
            d,
            Form {
                volume: volume.clone(),
                tab: tab.clone(),
            },
        );

        [(0.25f32, 0usize), (0.7, 1), (1.0, 2)]
            .into_iter()
            .map(|(level, index)| {
                let volume = volume.clone();
                let tab = tab.clone();
                let step: Box<dyn Fn(&mut vieww_render::FrameDriver)> = Box::new(move |_driver| {
                    volume.set(level);
                    tab.set(index);
                });
                (format!("volume{level}"), 0, step)
            })
            .collect()
    })
}
