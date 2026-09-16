//! A header whose height, colour and title size are functions of scroll
//! position — the `animation-timeline: scroll()` case.
//!
//! # Driven by scrolling, not by a clock
//!
//! `ScrollTimeline` is pull-based: it maps an offset onto `0..=1` and the
//! application tells it where the scroll got to. So there is nothing here for a
//! frame to advance, and a still at offset 0 is a picture of an untouched page.
//! The window scrolls with a wheel or a drag; the shots are taken at five
//! offsets through [`feature_harness::Steps`], which is the same call the drag
//! handler makes.

use std::rc::Rc;

use vieww_element::scroll_anim::ScrollTimeline;
use vieww_element::Signal;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const RANGE: f32 = 240.0;
const ROWS: usize = 12;

#[derive(Debug)]
struct Page {
    offset: Signal<f32>,
    timeline: Rc<ScrollTimeline>,
}

impl vieww_widget::Widget for Page {
    fn debug_name(&self) -> &'static str {
        "Page"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let t = self.timeline.progress().get();
        let offset = self.offset.get();

        let scroll = {
            let offset = self.offset.clone();
            let timeline = Rc::clone(&self.timeline);
            Rc::new(move |drag: vieww_foundation::DragDetails| {
                let next = (offset.peek() - drag.delta.dy).clamp(0.0, RANGE);
                offset.set(next);
                timeline.update(next);
            })
        };

        let rows: Vec<WidgetNode> = (0..ROWS)
            .map(|row| {
                Container::new()
                    .color(if row % 2 == 0 {
                        Color::rgb(226, 232, 240)
                    } else {
                        Color::rgb(237, 241, 247)
                    })
                    .radius(4.0)
                    .child(SizedBox::from_size(Size::new(440.0, 28.0)))
                    .into()
            })
            .collect();

        Container::new()
            .color(Color::WHITE)
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(children![
                        // Every one of these is a function of `t`: a header that
                        // only moved would prove less than one that also
                        // recolours and retypesets.
                        Container::new()
                            .color(Color::rgb(
                                (58.0 + 100.0 * t) as u8,
                                (122.0 - 40.0 * t) as u8,
                                (246.0 - 60.0 * t) as u8,
                            ))
                            .padding(EdgeInsets::all(16.0))
                            .child(
                                SizedBox::from_size(Size::new(440.0, 140.0 - 90.0 * t)).child(
                                    Text::new("scroll-driven header")
                                        .color(Color::WHITE)
                                        .size(26.0 - 10.0 * t)
                                        .bold(),
                                ),
                            ),
                        Flexible::new(1).child(
                            Scrollable::vertical(offset).on_drag(scroll).child(
                                Container::new()
                                    .padding(EdgeInsets::all(20.0))
                                    .child(Flex::column().spacing(8.0).children(rows),),
                            ),
                        ),
                    ]),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Page);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch_with("43 — scroll driven", Size::new(520.0, 420.0), |d| {
        let runtime = d.elements().runtime().clone();
        let timeline = Rc::new(ScrollTimeline::new(&runtime, 0.0, RANGE));
        let offset = runtime.signal(0.0f32);
        // Deliberately not `feature_harness::set_page`: this example owns its
        // own `Scrollable`, and a page wrapper would hand it an unbounded
        // height — which is a viewport with no window to measure, so the list
        // inside it builds nothing and the sheet comes out blank.
        d.set_root(Page {
            offset: offset.clone(),
            timeline: Rc::clone(&timeline),
        });

        [0.0f32, 60.0, 120.0, 180.0, 240.0]
            .into_iter()
            .map(|at| {
                let offset = offset.clone();
                let timeline = Rc::clone(&timeline);
                let step: Box<dyn Fn(&mut vieww_render::FrameDriver)> = Box::new(move |_driver| {
                    offset.set(at);
                    timeline.update(at);
                });
                (format!("offset{at}"), 0, step)
            })
            .collect()
    })
}
