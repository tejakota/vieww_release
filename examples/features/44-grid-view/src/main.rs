//! A virtualised grid — `ListView`'s two-dimensional counterpart.
//!
//! Columns are fixed and the row pitch comes from the item extent plus the
//! spacing, so a tile's index and its position are the same fact stated twice.
//! Scrolling by exactly one row's pitch must therefore move the numbers by
//! exactly one row.

use std::rc::Rc;

use vieww_element::Signal;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const TILES: usize = 240;
const COLUMNS: usize = 4;
const EXTENT: f32 = 88.0;

fn tint(index: usize) -> Color {
    const PALETTE: [Color; 4] = [
        Color::rgb(58, 122, 246),
        Color::rgb(186, 100, 246),
        Color::rgb(76, 187, 129),
        Color::rgb(255, 180, 80),
    ];
    PALETTE[index % PALETTE.len()]
}

#[derive(Debug)]
struct Gallery {
    offset: Signal<f32>,
}

impl vieww_widget::Widget for Gallery {
    fn debug_name(&self) -> &'static str {
        "Gallery"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let offset = self.offset.get();
        let drag = {
            let held = self.offset.clone();
            Rc::new(move |drag: vieww_foundation::DragDetails| {
                held.set((held.peek() - drag.delta.dy).max(0.0));
            })
        };

        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(16.0))
            .child(
                Scrollable::vertical(offset).on_drag(drag).child(
                    GridView::new(
                        TILES,
                        COLUMNS,
                        EXTENT,
                        Rc::new(|index| {
                            Container::new()
                                .color(tint(index))
                                .radius(10.0)
                                .padding(EdgeInsets::all(8.0))
                                .child(
                                    Text::new(format!("{index}"))
                                        .color(Color::WHITE)
                                        .size(13.0)
                                        .bold(),
                                )
                                .into()
                        }),
                    )
                    .spacing(10.0, 10.0),
                ),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Gallery);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch_with("44 — grid view", Size::new(480.0, 420.0), |d| {
        let offset = d.elements().runtime().signal(0.0f32);
        // Deliberately not `feature_harness::set_page`: this example owns its
        // own `Scrollable`, and a page wrapper would hand it an unbounded
        // height — which is a viewport with no window to measure, so the list
        // inside it builds nothing and the sheet comes out blank.
        d.set_root(Gallery {
            offset: offset.clone(),
        });

        [0.0f32, 98.0, 980.0]
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
