//! A rectangle sliding across the window on a timed tween.
//!
//! # The animation is held by the widget, on purpose
//!
//! `Animation::attach` registers the animation **weakly**, so a frame only
//! advances one that something else still owns. An animation created in
//! `main` and dropped there is therefore not an animation that runs slowly —
//! it is one that never moves at all, which looks exactly like a broken tween.
//! Keeping it in the widget that reads it is the smallest way to say "this
//! lives as long as the thing it moves".

use std::time::Duration;

use vieww::animation::Tween;
use vieww_element::Animation;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

#[derive(Debug)]
struct Slide {
    /// Kept so the frame driver's weak registration stays alive.
    slide: Animation<f32>,
}

impl vieww_widget::Widget for Slide {
    fn debug_name(&self) -> &'static str {
        "Slide"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        // Reading the animation here is what subscribes this element to it, so
        // one moving rectangle rebuilds one element rather than the screen.
        let x = self.slide.value();
        Container::new()
            .color(Color::rgb(247, 248, 250))
            .child(
                Stack::new().push(
                    Positioned::new().left(x).top(90.0).child(
                        Container::new()
                            .color(Color::rgb(58, 122, 246))
                            .radius(8.0)
                            .child(SizedBox::from_size(Size::new(80.0, 60.0))),
                    ),
                ),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Slide);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("05 — rectangle animation", Size::new(640.0, 240.0), |d| {
        let slide = d.animation(Tween::new(0.0f32, 480.0), Duration::from_millis(2000));
        // Started on the first frame rather than at timestamp zero, and looped:
        // a window costs a surface and a first shader compile before it draws,
        // so a one-shot begun at zero can be over before anything is on screen —
        // which looks like an animation that does not run at all.
        {
            let slide = slide.clone();
            feature_harness::on_first_frame(move |now| slide.repeat(true, now));
        }
        feature_harness::set_page(d, Slide { slide });
    })
}
