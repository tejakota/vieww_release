//! Where does a frame's time actually go? A controlled, headless probe.
//!
//! Renders the same command count under different clip shapes and reports
//! wall-clock per frame, so "the CPU rasterizer is slow" can be separated
//! from "one specific per-command cost is quadratic in the scene".

use std::time::Instant;

use vieww_foundation::{Color, Path, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Paint, Scene};

const W: u32 = 1366;
const H: u32 = 679;

fn card_grid(scene: &mut Scene, n: usize) {
    for i in 0..n {
        let x = (i % 40) as f32 * 34.0;
        let y = (i / 40) as f32 * 26.0;
        scene.fill_rect(
            Rect::new(x + 2.0, y + 2.0, x + 30.0, y + 22.0),
            Paint::solid(Color::rgb(40 + (i % 200) as u8, 80, 160)),
        );
    }
}

fn time<F: FnMut(&mut Scene)>(label: &str, n: usize, mut build: F) {
    let mut scene = Scene::new();
    build(&mut scene);
    let mut renderer = NativeRenderer::new();
    // warm
    let _ = renderer
        .render_to_pixels(&scene, W, H, Color::WHITE)
        .unwrap();
    let start = Instant::now();
    let (_pixels, report) = renderer
        .render_to_pixels(&scene, W, H, Color::WHITE)
        .unwrap();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{label:<46} n={n:<5} cmds={:<5} {ms:>9.1} ms  ({:.3} ms/cmd)",
        report.translated_commands,
        ms / report.translated_commands.max(1) as f64
    );
}

fn main() {
    println!("surface {W}x{H}\n");

    for n in [200usize, 400, 800, 1600] {
        time("A no clip at all", n, |s| card_grid(s, n));
    }
    println!();

    for n in [200usize, 400, 800, 1600] {
        time("B rect clip (bounds only, no shapes)", n, |s| {
            s.save();
            s.clip_rect(Rect::new(0.0, 0.0, W as f32, H as f32));
            card_grid(s, n);
            s.restore();
        });
    }
    println!();

    for n in [200usize, 400, 800, 1600] {
        time("C one rounded clip over the whole window", n, |s| {
            s.save();
            s.clip_rrect(Rect::new(0.0, 0.0, W as f32, H as f32), 12.0);
            card_grid(s, n);
            s.restore();
        });
    }
    println!();

    for n in [200usize, 400, 800, 1600] {
        time("D two nested rounded clips (panel in panel)", n, |s| {
            s.save();
            s.clip_rrect(Rect::new(0.0, 0.0, W as f32, H as f32), 12.0);
            s.clip_rrect(
                Rect::new(40.0, 40.0, W as f32 - 40.0, H as f32 - 40.0),
                10.0,
            );
            card_grid(s, n);
            s.restore();
        });
    }
    println!();

    // A small rounded clip: same shape count, far fewer pixels in the mask.
    for n in [200usize, 400, 800, 1600] {
        time("E one SMALL rounded clip (200x200)", n, |s| {
            s.save();
            s.clip_rrect(Rect::new(0.0, 0.0, 200.0, 200.0), 12.0);
            card_grid(s, n);
            s.restore();
        });
    }
    println!();

    // Correctness probe: two disjoint rounded clips should intersect to
    // nothing. Union-vs-intersection shows up as ink where there should be
    // none.
    let mut scene = Scene::new();
    scene.save();
    scene.clip_path(&Path::rounded_rect(Rect::new(0.0, 0.0, 100.0, 100.0), 8.0));
    scene.clip_path(&Path::rounded_rect(
        Rect::new(300.0, 300.0, 400.0, 400.0),
        8.0,
    ));
    scene.fill_rect(Rect::new(0.0, 0.0, 500.0, 500.0), Paint::solid(Color::RED));
    scene.restore();
    let mut renderer = NativeRenderer::new();
    let (pixels, _) = renderer
        .render_to_pixels(&scene, 500, 500, Color::WHITE)
        .unwrap();
    let data = pixels.data();
    let at = |x: usize, y: usize| {
        let i = (y * 500 + x) * 4;
        (data[i], data[i + 1], data[i + 2])
    };
    println!("clip intersection probe (two disjoint rounded clips, both should cull everything):");
    println!(
        "  inside clip A (50,50)   = {:?}   (white = correct)",
        at(50, 50)
    );
    println!(
        "  inside clip B (350,350) = {:?}   (white = correct)",
        at(350, 350)
    );
}
