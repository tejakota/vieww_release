//! Three ways to get from red to green, and why the default is the wrong one.
//!
//! Each strip is thirty-one steps of the same two endpoints, mixed three ways:
//!
//! - **`lerp`** mixes the *encoded* bytes. It is what every framework does by
//!   default and what a caller expects `lerp` to mean, and it is wrong in a
//!   visible way — the middle goes muddy and dark, because the sRGB transfer
//!   function is not linear in light.
//! - **`lerp_linear`** decodes to linear light, mixes, re-encodes. Physically
//!   right; the middle stays bright.
//! - **`lerp_oklab`** mixes in Oklab, which is perceptually uniform: equal
//!   steps *look* equal, which the other two do not manage.
//!
//! The fourth strip is the gamut: the same three hues in sRGB and in Display
//! P3. On an sRGB screen the two rows will look identical, which is correct
//! and is the point — the *model* can now name a colour outside sRGB, and what
//! a given screen does with it is the screen's business.

use vieww_foundation::{Color, ColorSpace, Size};
use vieww_widget::prelude::*;

const INK: Color = Color::hex(0x1A_1A1A);
const STEPS: usize = 31;
const CELL: f32 = 14.0;

const FROM: Color = Color::hex(0xE0_1B_1B);
const TO: Color = Color::hex(0x12_A0_4A);

/// One strip of `STEPS` cells, coloured by `mix`.
fn strip(caption: &str, mix: impl Fn(f32) -> Color) -> WidgetNode {
    let mut row = Flex::row();
    for step in 0..STEPS {
        #[expect(
            clippy::cast_precision_loss,
            reason = "STEPS is 31; the ratio is exact in f32"
        )]
        let t = step as f32 / (STEPS - 1) as f32;
        row = row.push(
            Container::new()
                .color(mix(t))
                .child(SizedBox::from_size(Size::new(CELL, 28.0))),
        );
    }

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .push(Text::new(caption).style(TextStyle::new(11.0).color(INK)))
        .push(row)
        .into()
}

/// The same three hues, said in two colour spaces.
fn gamut() -> WidgetNode {
    let hues = [
        ("red", (224_u8, 27_u8, 27_u8)),
        ("green", (18, 160, 74)),
        ("blue", (30, 90, 232)),
    ];

    let mut srgb = Flex::row().spacing(6.0);
    let mut p3 = Flex::row().spacing(6.0);
    for (_, (r, g, b)) in hues {
        srgb = srgb.push(
            Container::new()
                .color(Color::rgb(r, g, b))
                .radius(4.0)
                .child(SizedBox::from_size(Size::new(64.0, 28.0))),
        );
        // The same numbers, read as P3 primaries rather than sRGB ones — a
        // genuinely different colour, converted back into sRGB for this
        // screen by `converted_to`.
        p3 = p3.push(
            Container::new()
                .color(Color::p3(r, g, b).converted_to(ColorSpace::Srgb))
                .radius(4.0)
                .child(SizedBox::from_size(Size::new(64.0, 28.0))),
        );
    }

    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(4.0)
        .push(
            Text::new("gamut — sRGB above, the same numbers as Display P3 below")
                .style(TextStyle::new(11.0).color(INK)),
        )
        .push(srgb)
        .push(p3)
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("53 — wide-gamut colour", Size::new(520.0, 380.0), |d| {
        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(24.0))
                .child(
                    Flex::column()
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .spacing(14.0)
                        .push(strip("lerp — encoded bytes, and a muddy middle", |t| {
                            FROM.lerp(TO, t)
                        }))
                        .push(strip("lerp_linear — mixed in linear light", |t| {
                            FROM.lerp_linear(TO, t)
                        }))
                        .push(strip("lerp_oklab — perceptually even steps", |t| {
                            FROM.lerp_oklab(TO, t)
                        }))
                        .push(gamut()),
                ),
        );
    })
}
