//! What one real Studio frame costs, with no window and no GPU.
//!
//! The whole shell — activity bar, file tree, tab strip, the editor with its
//! syntax colouring, the panels, the status bar — built, laid out and
//! rasterised through the same `NativeRenderer` a window presents through.
//! This is the measurement that says whether the application is usable, and
//! it needs no display to take, which means it can be taken in CI.

use std::time::Instant;

use vieww_foundation::Color;
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

fn main() {
    let size = viewwstudio::WINDOW;
    let mut driver = FrameDriver::new(size);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    driver.set_root(Shell {
        studio: Studio::new(&runtime),
    });
    driver.draw_frame();

    let (w, h) = (size.width as u32, size.height as u32);
    let mut renderer = NativeRenderer::new();
    let (_, report) = renderer
        .render_to_pixels(driver.scene(), w, h, Color::BLACK)
        .expect("first frame");

    let mut best = f64::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        renderer
            .render_to_pixels(driver.scene(), w, h, Color::BLACK)
            .expect("timed frame");
        best = best.min(start.elapsed().as_secs_f64());
    }

    println!(
        "studio shell at {w}x{h}: {} commands ({} shapes, {} glyph runs / {} glyphs, \
         {} clips, {} shadows, {} layers)",
        driver.scene().len(),
        report.shapes,
        report.glyph_runs,
        report.glyphs,
        report.clips,
        report.shadows,
        report.layers,
    );
    println!(
        "one full-window frame: {:.2} ms  ({:.0} fps)",
        best * 1000.0,
        1.0 / best
    );

    let (png, _) = renderer
        .render_to_png(driver.scene(), w, h, Color::BLACK)
        .expect("encoding");
    std::fs::write("studio-frame.png", png).expect("writing the shot");
    println!("wrote studio-frame.png");
}
