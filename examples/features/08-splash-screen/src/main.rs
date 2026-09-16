//! A splash mark that fades up and rises into place.
//!
//! Two properties off one animation — opacity and a translation — which is the
//! usual shape of an entrance: one clock, several things read from it.
//!
//! The animation is owned by the widget for the reason spelled out in
//! `05-rectangle-animation`: `attach` holds it weakly.

use std::time::Duration;

use vieww::animation::Tween;
use vieww_element::Animation;
use vieww_foundation::{Color, Offset, Size};
use vieww_widget::prelude::*;

#[derive(Debug)]
struct Splash {
    entrance: Animation<f32>,
}

impl vieww_widget::Widget for Splash {
    fn debug_name(&self) -> &'static str {
        "Splash"
    }
    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }
    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let t = self.entrance.value();
        let rise = (1.0 - t) * 50.0;
        Container::new()
            .color(Color::rgb(23, 30, 42))
            .child(
                Stack::new().push(
                    Positioned::new().left(180.0).top(140.0).child(
                        Opacity::new(t.clamp(0.0, 1.0)).child(
                            Transformed::translate(Offset::new(0.0, rise)).child(
                                Container::new()
                                    .color(Color::rgb(58, 122, 246))
                                    .radius(24.0)
                                    .child(SizedBox::square(120.0)),
                            ),
                        ),
                    ),
                ),
            )
            .into()
    }
}
vieww_widget::widget_node_from!(Splash);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("08 — splash", Size::new(480.0, 360.0), |d| {
        let entrance = d.animation(Tween::new(0.0f32, 1.0), Duration::from_millis(1400));
        // An entrance plays once in a real application. Here it ping-pongs, so
        // that opening the window at any moment shows the entrance rather than
        // whatever it ended on — and it starts on the first frame, not at
        // timestamp zero. See `05-rectangle-animation`.
        {
            let entrance = entrance.clone();
            feature_harness::on_first_frame(move |now| entrance.repeat(true, now));
        }
        feature_harness::set_page(d, Splash { entrance });
    })
}
