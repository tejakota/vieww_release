//! What does a frame cost *before it draws anything*?
//!
//! Every fixture in the gallery has a floor under it: allocating the root
//! buffer, filling it with the background, and converting the finished float
//! buffer to RGBA8 all happen whether the scene has one command or a thousand.
//! If that floor is a large fraction of a frame, no amount of work on the
//! rasteriser moves the number — and the fix is a different fix (reuse the
//! buffer, convert only what changed) than the one everyone reaches for.

use std::time::Instant;

use vieww_foundation::{Color, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Paint, Scene};

fn bench(label: &str, scene: &Scene, w: u32, h: u32, runs: usize) {
    let mut renderer = NativeRenderer::new();
    let _ = renderer
        .render_to_pixels(scene, w, h, Color::WHITE)
        .unwrap();
    let mut best = f64::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        let _ = renderer
            .render_to_pixels(scene, w, h, Color::WHITE)
            .unwrap();
        best = best.min(start.elapsed().as_secs_f64());
    }
    println!("{label:<44} {:>9.2} ms", best * 1000.0);
}

fn main() {
    for (w, h) in [(560u32, 360u32), (840, 560), (1366, 679)] {
        println!("\n--- {w}x{h} ({} kpx) ---", w * h / 1000);

        let empty = Scene::new();
        bench("empty scene (the floor)", &empty, w, h, 8);

        let mut one = Scene::new();
        one.fill_rect(Rect::new(10.0, 10.0, 60.0, 60.0), Paint::solid(Color::RED));
        bench("one small rect", &one, w, h, 8);

        let mut full = Scene::new();
        full.fill_rect(
            Rect::new(0.0, 0.0, w as f32, h as f32),
            Paint::solid(Color::rgb(30, 40, 60)),
        );
        bench("one full-surface rect", &full, w, h, 8);

        let mut overdraw = Scene::new();
        for i in 0..8 {
            overdraw.fill_rect(
                Rect::new(0.0, 0.0, w as f32, h as f32),
                Paint::solid(Color::rgba(30, 40, 60, 200 - i * 10)),
            );
        }
        bench("eight full-surface rects (overdraw)", &overdraw, w, h, 8);
    }
}
