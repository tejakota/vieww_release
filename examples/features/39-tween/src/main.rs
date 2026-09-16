//! A tween, read as a colour.
//!
//! The square is centred rather than left to fill the window: a `Container`
//! with a fixed-size child still takes all the room its parent offers, so
//! "a 120pt square" needs something that shrink-wraps around it.

use std::time::Duration;

use vieww::animation::Tween;
use vieww_element::Animation;
use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

#[derive(Debug)]
struct TweenCell {
    fade: Animation<f32>,
}

impl vieww_widget::Widget for TweenCell {
    fn debug_name(&self) -> &'static str {
        "TweenCell"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let t = self.fade.value();
        let lerp = |from: f32, to: f32| (from + (to - from) * t) as u8;
        Container::new()
            .color(Color::WHITE)
            .child(
                Center::new().child(
                    Container::new()
                        .color(Color::rgb(lerp(58.0, 186.0), lerp(122.0, 100.0), 246))
                        .radius(12.0)
                        .child(SizedBox::square(120.0)),
                ),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(TweenCell);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("39 — tween", Size::new(480.0, 280.0), |d| {
        let fade = d.animation(Tween::new(0.0f32, 1.0), Duration::from_millis(2000));
        // Ping-ponged forever, and started on the first frame — see
        // `05-rectangle-animation` for why zero is the wrong timestamp.
        {
            let fade = fade.clone();
            feature_harness::on_first_frame(move |now| fade.repeat(true, now));
        }
        feature_harness::set_page(d, TweenCell { fade });
    })
}
