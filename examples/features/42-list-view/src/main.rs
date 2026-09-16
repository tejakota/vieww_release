//! A virtualised list: a thousand rows, only the visible ones built.
//!
//! `ListView` asks for the rows the viewport can show and no others, so the
//! count is free — what costs is the window. The row index is drawn on each
//! row, which is what makes the scroll steps below legible: after a scroll the
//! numbers must have moved, and the row heights must not have.

use std::rc::Rc;

use vieww_element::Signal;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const ROWS: usize = 1000;
const ROW_HEIGHT: f32 = 44.0;
const LIMIT: f32 = ROW_HEIGHT * ROWS as f32 - 300.0;

#[derive(Debug)]
struct List {
    offset: Signal<f32>,
}

impl vieww_widget::Widget for List {
    fn debug_name(&self) -> &'static str {
        "List"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let offset = self.offset.get();
        let drag = {
            let held = self.offset.clone();
            Rc::new(move |drag: vieww_foundation::DragDetails| {
                held.set((held.peek() - drag.delta.dy).clamp(0.0, LIMIT));
            })
        };

        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(16.0))
            .child(
                Scrollable::vertical(offset)
                    .on_drag(drag)
                    .child(ListView::new(
                        ROWS,
                        ROW_HEIGHT,
                        Rc::new(|index| {
                            Container::new()
                                .color(if index % 2 == 0 {
                                    Color::rgb(240, 244, 249)
                                } else {
                                    Color::WHITE
                                })
                                .padding(EdgeInsets::symmetric(14.0, 12.0))
                                .child(
                                    Text::new(format!("row {index}"))
                                        .color(Color::rgb(23, 30, 42))
                                        .size(14.0),
                                )
                                .into()
                        }),
                    )),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(List);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch_with("42 — list view", Size::new(420.0, 420.0), |d| {
        let offset = d.elements().runtime().signal(0.0f32);
        // Deliberately not `feature_harness::set_page`: this example owns its
        // own `Scrollable`, and a page wrapper would hand it an unbounded
        // height — which is a viewport with no window to measure, so the list
        // inside it builds nothing and the sheet comes out blank.
        d.set_root(List {
            offset: offset.clone(),
        });

        [0.0f32, 220.0, 4400.0]
            .into_iter()
            .map(|at| {
                let offset = offset.clone();
                let step: Box<dyn Fn(&mut vieww_render::FrameDriver)> =
                    Box::new(move |_driver| offset.set(at));
                (format!("offset{at}"), 0, step)
            })
            .collect()
    })
}
