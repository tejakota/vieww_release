//! What one full-window blurred layer costs, on its own.
//!
//! ```console
//! cargo run --release -p vieww-paint --features native --example blur_bench
//! ```
//!
//! # Why this is separate from the fixture gallery
//!
//! `examples/fixtures`' `23-editor-glass` and `33-sheet` are both blur-bound,
//! and both are also a hundred-odd other commands — so a change to the blur
//! kernel moves them by a fraction of their total and disappears into the noise
//! floor. This is the blur and almost nothing else: one filtered layer over the
//! whole surface, sixty solid fills inside it to give the kernel something to
//! spread, and no text, no shadows and no clips.
//!
//! # Read more than one run of it
//!
//! The spread between runs of the *same binary* on an ordinary shared machine
//! is comfortably wider than a 20% change in the kernel. A single before and a
//! single after is not a measurement. `native/effects.rs`'s `vertical` records
//! what happened when that was tried here: one sample each said the change made
//! things *slower*, six runs each said it was 26% faster, and the first answer
//! was nearly acted on. Take the best of several runs on both sides, and
//! interleave them if the machine is doing anything else.

use std::time::Instant;

use vieww_foundation::{BlendMode, Color, ImageFilter, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Paint, Scene};

/// A window-sized surface: the case that matters, since a glass panel or a
/// modal backdrop is usually the whole window.
const W: u32 = 1366;
const H: u32 = 768;

/// Large enough that the three box passes have a real radius to run at —
/// `radius = (sqrt(4σ² + 1) - 1) / 2`, so about 18 pixels a pass and 54 of
/// total reach.
const SIGMA: f32 = 18.0;

fn main() {
    let mut scene = Scene::new();
    scene.push_filtered_layer(
        Rect::new(0.0, 0.0, W as f32, H as f32),
        1.0,
        BlendMode::Normal,
        ImageFilter::blur(SIGMA),
    );
    // Content for the blur to have an opinion about. Solid rectangles rather
    // than anything expensive: the point is to measure the filter, not what is
    // underneath it.
    for i in 0..60 {
        let x = (i % 10) as f32 * 130.0;
        let y = (i / 10) as f32 * 120.0;
        scene.fill_rect(
            Rect::new(x + 10.0, y + 10.0, x + 120.0, y + 110.0),
            Paint::solid(Color::rgb(60, 120, 220)),
        );
    }
    scene.pop_layer();

    let mut renderer = NativeRenderer::new();
    // Warm: the first render allocates the root buffer and the layer pool's
    // first offscreen, neither of which happens again.
    renderer
        .render_to_pixels(&scene, W, H, Color::WHITE)
        .expect("the first render");

    let mut best = f64::MAX;
    for _ in 0..12 {
        let at = Instant::now();
        renderer
            .render_to_pixels(&scene, W, H, Color::WHITE)
            .expect("a timed render");
        best = best.min(at.elapsed().as_secs_f64());
    }
    println!(
        "one full-window blurred layer at {W}x{H}, sigma {SIGMA}: {:.2} ms",
        best * 1000.0
    );
}
