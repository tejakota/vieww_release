//! Every blend mode the framework has, drawn over the same two shapes.
//!
//! A grid of swatches: an orange square with a cyan circle over it, once per
//! [`BlendMode`]. That is the only honest way to show a blend model — the
//! names mean nothing until you have seen `ColorBurn` beside `Multiply` — and
//! it is also the check that caught two mismapped `(Mix, Compose)` pairs when
//! the three paint backends were converted.
//!
//! The three families are drawn as three blocks, because they answer different
//! questions: Porter-Duff decides *which region is covered*, the separable
//! modes decide *how the channels combine*, and the four non-separable ones
//! reach across channels and cannot be computed one at a time.
//!
//! # Six modes are named and not drawn
//!
//! `Clear`, `Src`, `SrcIn`, `DstIn`, `SrcOut` and `DstAtop` are in the model
//! and are **not honoured by the rasteriser** — their result differs from the
//! destination *outside* the source, and the compositor applies them across
//! the whole target rather than within the layer. Drawn here they would erase
//! the swatches above them; substituted, they would draw the same picture as
//! `Normal` and quietly claim to be something else.
//!
//! So they are listed by name in their own block instead, which is the honest
//! third option. `BlendMode::needs_isolated_compositing` carries the
//! measurement and `SceneReport::unsupported_blends` counts each substitution
//! at run time. **This example is how the defect was found**, which is the
//! argument for drawing a feature rather than only testing it.

use vieww_foundation::{BlendMode, Color, Size};
use vieww_widget::prelude::*;

const BACK: Color = Color::hex(0xF7_F7F7);
const INK: Color = Color::hex(0x30_3030);
const UNDER: Color = Color::hex(0xE8_7A_1E);
const OVER: Color = Color::hex(0x1E_9E_E8);

const SWATCH: f32 = 46.0;

/// One swatch: the orange square, with the cyan circle blended over it.
///
/// The circle is inside an `Opacity` at full alpha, which is what puts the
/// subtree on a layer of its own — the blend is a property of the *group*
/// against what is behind it, so there has to be a group.
fn swatch(mode: BlendMode) -> WidgetNode {
    Flex::column()
        .spacing(4.0)
        .push(
            Container::new().color(UNDER).radius(4.0).child(
                SizedBox::square(SWATCH).child(
                    Align::new(Alignment::CENTER).child(
                        Opacity::new(1.0).blend(mode).child(
                            Container::new()
                                .color(OVER)
                                .radius(SWATCH / 2.0)
                                .child(SizedBox::square(32.0)),
                        ),
                    ),
                ),
            ),
        )
        .push(Text::new(mode.name()).style(TextStyle::new(8.0).color(INK)))
        .into()
}

/// A titled block of swatches, wrapped at `per_row`.
fn family(title: &str, modes: &[BlendMode], per_row: usize) -> WidgetNode {
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .push(Text::new(title).style(TextStyle::new(11.0).color(INK)));

    for chunk in modes.chunks(per_row) {
        let mut row = Flex::row().spacing(8.0);
        for &mode in chunk {
            row = row.push(swatch(mode));
        }
        column = column.push(row);
    }
    column.into()
}

/// The modes the rasteriser cannot confine, listed rather than drawn.
fn named_only(modes: &[BlendMode]) -> WidgetNode {
    let names: Vec<&str> = modes.iter().map(|mode| mode.name()).collect();
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .push(
            Text::new("In the model, not honoured by the rasteriser")
                .style(TextStyle::new(11.0).color(INK)),
        )
        .push(
            Text::new(names.join(" \u{00B7} "))
                .style(TextStyle::new(9.0).color(Color::hex(0x8A_8A8A))),
        )
        .push(
            Text::new(
                "each is composited as Normal and counted in SceneReport::unsupported_blends",
            )
            .style(TextStyle::new(8.0).color(Color::hex(0xA0_A0A0))),
        )
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `Normal` is source-over, which `is_coverage` deliberately excludes — it
    // is the default rather than a coverage rule anybody chooses. It belongs
    // at the head of the Porter-Duff block all the same, because that is the
    // family it is the identity element of, and a grid without it gives the
    // eye nothing to judge the other twelve against.
    let coverage: Vec<BlendMode> = std::iter::once(BlendMode::Normal)
        .chain(
            BlendMode::ALL
                .iter()
                .copied()
                .filter(|m| m.is_coverage() && !m.needs_isolated_compositing()),
        )
        .collect();
    let unhonoured: Vec<BlendMode> = BlendMode::ALL
        .iter()
        .copied()
        .filter(|m| m.needs_isolated_compositing())
        .collect();
    let separable: Vec<BlendMode> = BlendMode::ALL
        .iter()
        .copied()
        .filter(|mode| {
            !mode.is_coverage() && !mode.is_non_separable() && *mode != BlendMode::Normal
        })
        .collect();
    let non_separable: Vec<BlendMode> = BlendMode::ALL
        .iter()
        .copied()
        .filter(|mode| mode.is_non_separable())
        .collect();

    feature_harness::launch("52 — blend modes", Size::new(620.0, 450.0), move |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(BACK)
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(16.0)
                        .push(family(
                            "Porter-Duff — which region is covered",
                            &coverage,
                            7,
                        ))
                        .push(family(
                            "Separable — how the channels combine",
                            &separable,
                            7,
                        ))
                        .push(family(
                            "Non-separable — reaching across channels",
                            &non_separable,
                            7,
                        ))
                        .push(named_only(&unhonoured)),
                ),
        );
    })
}
