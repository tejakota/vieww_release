//! The renderer section's pictures, drawn by the renderer they are about.
//!
//! # Why these are generated and not screenshots of the test suite
//!
//! The page shipped two images out of `examples/fixtures`: a checkerboard with
//! twenty-eight blend modes stamped on it, and one frame of the animation
//! showcase. Both are *correctness* pictures. A checkerboard exists so a human
//! can see premultiplication go wrong; the harsh red-orange-blue is there
//! because it makes an error obvious, which is the opposite of what a product
//! page wants. Putting them on the page said "here is our QA output".
//!
//! These are the same claims made properly. Everything in them is a vieww
//! widget tree — gradients, blend modes, real shadows, a Gaussian blur on a
//! group — rasterised by `vieww_paint::native`, which is exactly the sentence
//! the section next to them is trying to make.
//!
//! ```console
//! cargo run -p viewwsite --example figures
//! ```
//!
//! Writes into `apps/viewwsite/assets/`, which `build-site.sh` copies.

use std::f32::consts::PI;
use std::path::PathBuf;

use vieww::foundation::{BlendMode, Color, EdgeInsets, Gradient, Offset, Shadow, Size};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;
use vieww::FrameDriver;

/// The page's own ground, so a figure sits on the card rather than on a hole.
const GROUND: Color = Color::rgb(0x0B, 0x0A, 0x09);
const INK: Color = Color::rgb(0xF8, 0xF4, 0xF2);
const ACCENT: Color = Color::rgb(0xB4, 0x91, 0xFF);

/// 16:10, and big enough that the page can show it at 2x.
const W: f32 = 1600.0;
const H: f32 = 1000.0;

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "apps/viewwsite/assets".into())
        .into();
    std::fs::create_dir_all(&out).expect("creating the output directory");

    for (name, tree) in [
        ("figure-blend.png", compositing()),
        ("figure-depth.png", depth()),
        ("figure-easing.png", easing()),
    ] {
        let mut driver = FrameDriver::new(Size::new(W, H));
        driver.set_fonts(viewwsite::fonts());
        driver.set_root(tree);
        for _ in 0..3 {
            driver.draw_frame();
        }
        let mut renderer = NativeRenderer::new();
        let (png, report) = renderer
            .render_to_png(driver.scene(), W as u32, H as u32, GROUND)
            .expect("rasterising through vieww's own renderer");
        std::fs::write(out.join(name), png).expect("writing the PNG");
        println!(
            "{name}: {} shapes, {} glyph runs, {} clips",
            report.shapes, report.glyph_runs, report.clips
        );
    }
}

/// A soft disc of colour — the shape everything here is built from.
fn orb(size: f32, inner: Color, outer: Color) -> WidgetNode {
    Container::new()
        .radius(size)
        .gradient(Gradient::radial(Offset::new(0.5, 0.5), 0.5).with_stops(&[
            (0.0, inner),
            (0.62, mix(inner, outer, 0.55)),
            (1.0, outer),
        ]))
        .child(SizedBox::from_size(Size::new(size, size)))
        .into()
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
    Color::rgba(f(a.r, b.r), f(a.g, b.g), f(a.b, b.b), f(a.a, b.a))
}

fn clear() -> Color {
    Color::rgba(0, 0, 0, 0)
}

fn at(x: f32, y: f32, child: impl Into<WidgetNode>) -> WidgetNode {
    Positioned::new().left(x).top(y).child(child).into()
}

/// The one piece of type in these pictures: a curve's name, under its plot.
fn label(text: &str, size: f32, color: Color) -> WidgetNode {
    Text::new(text.to_string())
        .style(
            vieww::foundation::TextStyle::new(size)
                .family(vieww::foundation::FontFamily::Named("Geist Mono"))
                .color(color),
        )
        .into()
}

/// **Compositing.** Three luminous discs over a dark ground, each through a
/// different blend mode, plus the mode's name.
///
/// It is the same claim the checkerboard made — twenty-eight modes, one pass,
/// premultiplied correctly — made with an image somebody would want to look at.
fn compositing() -> WidgetNode {
    let disc = |x: f32, y: f32, size: f32, inner: Color, mode: BlendMode| -> WidgetNode {
        at(
            x,
            y,
            Opacity::new(0.92)
                .blend(mode)
                .child(orb(size, inner, clear())),
        )
    };
    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            // The ground: a wide, slow ramp, so the modes have something with
            // tone in it to work against rather than flat black.
            Container::new().gradient(
                Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0)).with_stops(&[
                    (0.0, Color::rgb(0x14, 0x10, 0x22)),
                    (0.55, Color::rgb(0x0E, 0x0C, 0x14)),
                    (1.0, Color::rgb(0x1A, 0x12, 0x10)),
                ])
            ),
            disc(
                120.0,
                130.0,
                560.0,
                Color::rgb(0x7E, 0x5C, 0xE8),
                BlendMode::Screen
            ),
            disc(
                470.0,
                300.0,
                620.0,
                Color::rgb(0x2F, 0xBF, 0xAE),
                BlendMode::Screen
            ),
            disc(
                830.0,
                90.0,
                640.0,
                Color::rgb(0xFF, 0x9F, 0xB4),
                BlendMode::Screen
            ),
            disc(
                620.0,
                40.0,
                400.0,
                Color::rgb(0xB4, 0x91, 0xFF),
                BlendMode::SoftLight
            ),
            disc(
                230.0,
                470.0,
                540.0,
                Color::rgb(0xFE, 0xB2, 0x63),
                BlendMode::Overlay
            ),
        ])
        .into()
}

/// **Depth.** Real shadows, a real Gaussian blur, and a rim of light — the
/// three things that make a surface read as a surface.
fn depth() -> WidgetNode {
    let card = |w: f32, h: f32, tint: Color, alpha: u8| -> WidgetNode {
        Container::new()
            .radius(28.0)
            .color(Color::rgba(tint.r, tint.g, tint.b, alpha))
            .gradient(Gradient::vertical().between(
                Color::rgba(255, 255, 255, 26),
                Color::rgba(255, 255, 255, 4),
            ))
            .border(vieww::foundation::Border::new(
                Color::rgba(255, 255, 255, 38),
                1.5,
            ))
            .shadow(Shadow::new(
                Color::rgba(0, 0, 0, 150),
                Offset::new(0.0, 34.0),
                70.0,
            ))
            .child(SizedBox::from_size(Size::new(w, h)))
            .into()
    };
    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            Container::new().gradient(
                Gradient::linear(Offset::new(0.0, 0.0), Offset::new(0.3, 1.0)).with_stops(&[
                    (0.0, Color::rgb(0x12, 0x0F, 0x1C)),
                    (1.0, Color::rgb(0x0B, 0x0A, 0x09)),
                ])
            ),
            // A blurred group behind the glass — the thing the cards are
            // translucent *over*.
            at(
                300.0,
                180.0,
                Filtered::blur(58.0).child(Stack::new().fit(StackFit::Loose).children(children![
                    at(0.0, 0.0, orb(520.0, Color::rgb(0x7E, 0x5C, 0xE8), clear())),
                    at(
                        340.0,
                        160.0,
                        orb(560.0, Color::rgb(0x2F, 0xBF, 0xAE), clear())
                    ),
                ]))
            ),
            at(
                190.0,
                250.0,
                card(560.0, 340.0, Color::rgb(0x2B, 0x25, 0x33), 150)
            ),
            at(
                430.0,
                380.0,
                card(620.0, 380.0, Color::rgb(0x24, 0x22, 0x2E), 165)
            ),
            at(
                760.0,
                190.0,
                card(520.0, 320.0, Color::rgb(0x2E, 0x28, 0x38), 140)
            ),
        ])
        .into()
}

/// **Easing.** The curves themselves, plotted — the most honest picture an
/// animation system can offer, and the one the dashed circle was trying to be.
fn easing() -> WidgetNode {
    /// One plotted curve, as a run of short segments.
    fn curve(name: &str, tint: Color, f: fn(f32) -> f32) -> WidgetNode {
        const PLOT: f32 = 380.0;
        // A spring passes 1.0 on the way back. Plotting 0..=1 clipped the
        // overshoot flat, which is the one part of the curve worth showing.
        const HEAD: f32 = 1.18;
        const STEPS: usize = 64;
        let mut dots: Vec<WidgetNode> = Vec::with_capacity(STEPS + 1);
        for i in 0..=STEPS {
            let t = i as f32 / STEPS as f32;
            let v = f(t).clamp(0.0, HEAD);
            dots.push(at(
                t * PLOT - 3.0,
                (1.0 - v / HEAD) * PLOT - 3.5,
                Container::new()
                    .color(tint)
                    .radius(999.0)
                    .child(SizedBox::from_size(Size::new(7.0, 7.0))),
            ));
        }
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(18.0)
            .children(children![
                Container::new()
                    .radius(20.0)
                    .color(Color::rgba(255, 255, 255, 8))
                    .border(vieww::foundation::Border::new(
                        Color::rgba(255, 255, 255, 20),
                        1.0
                    ))
                    .padding(EdgeInsets::all(26.0))
                    .child(
                        SizedBox::from_size(Size::new(PLOT, PLOT)).child(
                            Stack::new().fit(StackFit::Expand).children(
                                std::iter::once(
                                    // the linear reference, faint
                                    at(
                                        0.0,
                                        (1.0 - 1.0 / HEAD) * PLOT,
                                        Container::new()
                                            .color(Color::rgba(255, 255, 255, 14))
                                            .child(SizedBox::from_size(Size::new(PLOT, 1.0)))
                                    )
                                )
                                .chain(dots)
                                .collect::<Vec<_>>()
                            )
                        )
                    ),
                label(name, 19.0, INK),
            ])
            .into()
    }

    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            Container::new().gradient(
                Gradient::linear(Offset::new(1.0, 0.0), Offset::new(0.0, 1.0)).with_stops(&[
                    (0.0, Color::rgb(0x12, 0x10, 0x18)),
                    (1.0, Color::rgb(0x0B, 0x0A, 0x09)),
                ])
            ),
            at(
                92.0,
                220.0,
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(56.0)
                    .children(children![
                        curve("ease out cubic", ACCENT, |t| 1.0 - (1.0 - t).powi(3)),
                        curve(
                            "ease in out",
                            Color::rgb(0x2F, 0xBF, 0xAE),
                            |t| if t < 0.5 {
                                4.0 * t * t * t
                            } else {
                                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                            }
                        ),
                        curve("spring", Color::rgb(0xFE, 0xB2, 0x63), |t| {
                            1.0 - (-7.0 * t).exp() * (t * PI * 2.6).cos()
                        }),
                    ])
            ),
        ])
        .into()
}
