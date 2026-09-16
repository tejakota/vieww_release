//! `CrossAxisAlignment::Baseline`, beside the alignment it replaces.
//!
//! The whole feature is a few pixels, and a few pixels is exactly what a
//! picture is for. Two rows of the same three children — a 30pt number, an
//! 13pt label and an icon — one centred and one on a baseline. In the centred
//! row the digits ride high above the label; in the baselined row they sit on
//! one line, and the icon, which has no baseline at all, falls back to the top
//! rather than to a fabricated zero.

use vieww_foundation::{Color, IconData, Size};
use vieww_widget::prelude::*;

const INK: Color = Color::hex(0x1A_1A1A);
const MUTED: Color = Color::hex(0x6E_6E6E);
const RULE: Color = Color::hex(0xD0_47_4E);

/// A downward chevron, in the 24-unit grid `Icon` scales from.
fn chevron() -> IconData {
    IconData::square24(
        vieww_foundation::parse_path_data("M6 9 L12 15 L18 9").expect("a literal path"),
    )
}

/// The same three children every time, so the only variable is the alignment.
fn row(alignment: CrossAxisAlignment) -> WidgetNode {
    Flex::row()
        .cross_axis_alignment(alignment)
        .spacing(10.0)
        .push(Text::new("128").style(TextStyle::new(30.0).color(INK)))
        .push(Text::new("messages").style(TextStyle::new(13.0).color(MUTED)))
        .push(Icon::new(chevron()).size(16.0).color(MUTED))
        .into()
}

/// A hairline through the row, at the height the baselined digits sit on.
///
/// Drawn rather than described, because "the letters are on one line" is a
/// claim a ruler settles and prose does not. The two rows are the same height,
/// so one offset serves both — which is itself the thing being shown: baseline
/// alignment makes a row *taller* than its tallest child when two type sizes
/// are in it, because it has to hold the deepest ascent over the deepest
/// descent.
fn labelled(caption: &str, alignment: CrossAxisAlignment) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .push(Text::new(caption).style(TextStyle::new(11.0).color(RULE)))
        .push(
            Container::new()
                .color(Color::hex(0xF4_F4F4))
                .radius(6.0)
                .padding(EdgeInsets::all(12.0))
                .child(row(alignment)),
        )
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("50 — baseline alignment", Size::new(420.0, 260.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(18.0)
                        .push(labelled(
                            "Center — the boxes line up",
                            CrossAxisAlignment::Center,
                        ))
                        .push(labelled(
                            "Baseline — the letters line up",
                            CrossAxisAlignment::Baseline,
                        )),
                ),
        );
    })
}
