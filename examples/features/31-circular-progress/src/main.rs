//! Circular progress, determinate and indeterminate, next to a skeleton.
//!
//! The indeterminate ring animates **on its own** — nothing in this file
//! starts it. That is why the shot is a strip: if the frames are identical,
//! its ticker is not being advanced, and only a moving picture can say so.
//!
//! The `Skeleton` next to it is deliberately **static**: the library ships a
//! plain placeholder rather than a shimmer, because the looping animation a
//! shimmer needs does not exist yet and a faked one would be the wrong kind of
//! finished (see the widget's own docs). It is here so that the difference
//! between "animates itself" and "does not" is on the same sheet.

use vieww_foundation::{Color, Size};
use vieww_widget::prelude::*;

const MUTED: Color = Color::rgb(110, 122, 140);

fn labelled(label: &str, child: impl Into<WidgetNode>) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(10.0)
        .children(children![
            Text::new(label).color(MUTED).size(12.0),
            child.into(),
        ])
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("31 — circular progress", Size::new(700.0, 220.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::row()
                        .spacing(28.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            labelled("0.25", CircularProgress::new(0.25)),
                            labelled("0.6", CircularProgress::new(0.6)),
                            labelled("0.95", CircularProgress::new(0.95)),
                            labelled("indeterminate", CircularProgress::indeterminate()),
                            labelled(
                                "skeleton",
                                Skeleton::new().width(140.0).height(64.0).corner(8.0),
                            ),
                        ]),
                ),
        );
    })
}
