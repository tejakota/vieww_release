//! Does a light rounded rect fading over a dark stage leave a bright rim?
//!
//! ```console
//! cargo run --release -p vieww-paint --features native --example edge_probe
//! ```
//!
//! # The question this settles
//!
//! The launch film cross-fades light product surfaces — an app screen, a
//! desktop window body — over a near-black stage, and reviewers of the master
//! reported a pale edge left behind around those surfaces mid-transition. That
//! has two possible authors and they want different fixes:
//!
//! * **the film**, if the artifact is a consequence of animating opacity on a
//!   light surface over a dark ground — in which case the fix is to transform
//!   the geometry instead and never partially composite the edge; or
//! * **this crate**, if compositing a light rounded rect at partial alpha over
//!   a darker ground can produce a pixel *brighter than the fill itself* — a
//!   double-composite or a premultiplication error, which would affect every
//!   Vieww application that draws a light card on a dark surface, not just a
//!   film.
//!
//! The test is the second one, because it is decidable. Over a ground darker
//! than the fill, correct source-over compositing gives
//!
//! ```text
//! result = a·fill + (1 - a)·ground        (per channel, a = alpha × coverage)
//! ```
//!
//! which is a weighted average of two values the fill is the larger of. So
//! **no pixel may ever exceed the fill**, at any alpha, at any coverage, at any
//! subpixel offset. One that does is a renderer bug. One that does not means
//! the rim is authored, and the film is where it gets fixed.
//!
//! Both ways of asking for partial opacity are probed, because they take
//! different paths: `push_alpha` (a group opacity, which composites an
//! offscreen) and a paint whose own colour carries the alpha (composited
//! directly). A double-composite bug lives in the first and not the second, so
//! reporting them separately says where to look if this ever does fail.

use vieww_foundation::{BlendMode, Color, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Paint, Scene};

const W: u32 = 320;
const H: u32 = 220;

/// The film's stage: near-black, not black, which is the case that matters —
/// a rim against pure black could be hidden by clamping.
const STAGE: Color = Color::rgb(10, 11, 13);
/// The app surface the film fades: an off-white panel.
const SURFACE: Color = Color::rgb(242, 242, 240);
const RADIUS: f32 = 18.0;

/// Rec. 709 luminance, on raw sRGB bytes.
///
/// Not linearised on purpose: the question is whether a *stored* pixel comes
/// out brighter than the stored fill, and both sides of that comparison are in
/// the same encoding. Converting both would not change which is larger.
fn luma(c: Color) -> f32 {
    0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)
}

/// How partial opacity was asked for.
#[derive(Clone, Copy)]
enum Mode {
    /// `push_alpha` — a group opacity, composited through an offscreen layer.
    GroupAlpha,
    /// Alpha carried on the paint's own colour, composited directly.
    PaintAlpha,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::GroupAlpha => "push_alpha (group)",
            Self::PaintAlpha => "paint alpha    ",
        }
    }
}

/// Draw the surface once, at `alpha`, offset by `dx`/`dy` subpixels.
fn render(renderer: &mut NativeRenderer, mode: Mode, alpha: f32, dx: f32, dy: f32) -> Vec<Color> {
    let mut scene = Scene::new();
    let rect = Rect::new(60.0 + dx, 40.0 + dy, 260.0 + dx, 180.0 + dy);

    match mode {
        Mode::GroupAlpha => {
            scene.push_layer(
                Rect::new(0.0, 0.0, W as f32, H as f32),
                alpha,
                BlendMode::Normal,
            );
            scene.fill_rrect(rect, RADIUS, Paint::solid(SURFACE));
            scene.pop_layer();
        }
        Mode::PaintAlpha => {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let a = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            let faded = Color::rgba(SURFACE.r, SURFACE.g, SURFACE.b, a);
            scene.fill_rrect(rect, RADIUS, Paint::solid(faded));
        }
    }

    let (pixels, _) = renderer
        .render_to_pixels(&scene, W, H, STAGE)
        .expect("rasterising the probe scene");

    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| pixels.pixel(x, y))
        .collect()
}

fn main() {
    let mut renderer = NativeRenderer::new();
    let stage_luma = luma(STAGE);
    let surface_luma = luma(SURFACE);

    println!("stage {stage_luma:.1}  surface {surface_luma:.1}  (Rec.709 luma, sRGB bytes)\n");
    println!("A pixel brighter than the composited fill is a renderer bug.");
    println!("'over' is the worst excess found, in luma units.\n");
    println!("  mode                 alpha   offset   expected  brightest    over");

    let mut worst_over = f32::MIN;
    let mut failures = 0usize;

    for mode in [Mode::GroupAlpha, Mode::PaintAlpha] {
        for alpha in [0.05_f32, 0.15, 0.3, 0.5, 0.7, 0.85, 0.95, 1.0] {
            for (dx, dy) in [(0.0_f32, 0.0_f32), (0.25, 0.5), (0.5, 0.5), (0.75, 0.25)] {
                let pixels = render(&mut renderer, mode, alpha, dx, dy);

                // What a correct composite of the fill at this alpha produces.
                // Every pixel in the image is the same formula with a coverage
                // factor between 0 and 1, so this is the ceiling for all of
                // them — interior included, since interior coverage is 1.
                let expected = alpha * surface_luma + (1.0 - alpha) * stage_luma;
                let brightest = pixels.iter().copied().map(luma).fold(f32::MIN, f32::max);

                // A little slack for the renderer's own rounding: a channel
                // rounded up by one byte moves luma by at most 1.0.
                let over = brightest - expected;
                let bug = over > 1.5;
                failures += usize::from(bug);
                worst_over = worst_over.max(over);

                println!(
                    "  {}  {alpha:>5.2}   {dx:.2},{dy:.2}   {expected:>8.1}  {brightest:>9.1}  {over:>+6.1} {}",
                    mode.name(),
                    if bug { "  <-- BUG" } else { "" }
                );
            }
        }
    }

    println!();
    if failures == 0 {
        println!(
            "PASS — {} configurations, none brighter than its composited fill \
             (worst excess {worst_over:+.1}).",
            8 * 4 * 2
        );
        println!(
            "The rasteriser composites light-over-dark correctly. A pale edge in \
             the film is authored, not rendered."
        );
    } else {
        println!(
            "FAIL — {failures} configuration(s) produced a pixel brighter than the \
             fill (worst {worst_over:+.1})."
        );
        println!("This is a renderer bug and affects every light surface on a dark ground.");
        std::process::exit(1);
    }
}
