//! Implicit animation: change the properties, and the widget animates itself.
//!
//! There is no controller and no signal read in a build here. An
//! `AnimatedContainer` compares the properties it is given against the ones it
//! is showing and animates the difference — so a rebuild with a different
//! colour *is* the animation.
//!
//! The strip is taken across one such change: the step at 0 ms flips the
//! target, and the later shots are the frames between the two states rather
//! than the two states themselves.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use vieww::animation::Curve;
use vieww_element::Signal;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

#[derive(Debug)]
struct Morph {
    expanded: Signal<bool>,
}

impl vieww_widget::Widget for Morph {
    fn debug_name(&self) -> &'static str {
        "Morph"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let expanded = self.expanded.get();
        Container::new()
            .color(Color::rgb(247, 248, 250))
            .padding(EdgeInsets::all(24.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(12.0)
                    .children(children![
                        Text::new(if expanded { "expanded" } else { "collapsed" })
                            .color(Color::rgb(110, 122, 140))
                            .size(12.0),
                        AnimatedContainer::new()
                            .duration(Duration::from_millis(600))
                            .curve(Curve::FAST_OUT_SLOW_IN)
                            .size(
                                if expanded { 420.0 } else { 120.0 },
                                if expanded { 150.0 } else { 60.0 },
                            )
                            .color(if expanded {
                                Color::rgb(186, 100, 246)
                            } else {
                                Color::rgb(58, 122, 246)
                            }),
                    ]),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Morph);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch_with("38 — animated container", Size::new(520.0, 260.0), |d| {
        let expanded = d.elements().runtime().signal(false);
        feature_harness::set_page(
            d,
            Morph {
                expanded: expanded.clone(),
            },
        );

        // One flip, photographed on the way across: the interesting frames of an
        // implicit animation are the ones nobody wrote.
        let flipped = Rc::new(Cell::new(false));
        [
            ("collapsed", 0u64),
            ("start", 0),
            ("150ms", 150),
            ("350ms", 350),
            ("settled", 900),
        ]
        .into_iter()
        .map(|(label, at)| {
            let expanded = expanded.clone();
            let flipped = Rc::clone(&flipped);
            let first = label == "collapsed";
            let step: Box<dyn Fn(&mut vieww_render::FrameDriver)> = Box::new(move |_driver| {
                if !first && !flipped.replace(true) {
                    expanded.set(true);
                }
            });
            (label.to_owned(), at, step)
        })
        .collect()
    })
}
