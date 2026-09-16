//! Where does a frame of this page go?
//!
//! ```console
//! cargo run --release -p viewwsite --example bench
//! ```
//!
//! Two numbers, because they have different fixes. The *pipeline* is build,
//! layout, paint and flatten — vieww's own work, and what a scroll makes it
//! redo. The *rasterise* is turning the resulting scene into pixels, which on
//! this backend happens for the whole window every time anything moves,
//! because `put_image_data` takes a whole buffer.
use std::time::Instant;

use vieww::foundation::{Color, Offset, ScrollEvent, Size};
use vieww::paint::native::NativeRenderer;
use vieww::FrameDriver;
use viewwsite::{root, Os};

const GROUND: Color = Color::rgb(0x10, 0x10, 0x12);

fn main() {
    for (label, w, h) in [
        ("desktop 1280x900", 1280.0_f32, 900.0_f32),
        ("phone 390x844", 390.0, 844.0),
    ] {
        let mut driver = FrameDriver::new(Size::new(w, h));
        let tree = root(&mut driver, Some(Os::Linux));
        driver.set_root(tree);
        for _ in 0..4 {
            driver.draw_frame();
        }

        let mut renderer = NativeRenderer::new();
        let (dw, dh) = (w as u32, h as u32);

        // Warm the caches the way a running page has them warm.
        for _ in 0..3 {
            let _ = renderer.render_to_pixels(driver.scene(), dw, dh, GROUND);
        }

        // What is actually in the frame, and what an empty one costs — the
        // difference between the two says whether the time is going on drawing
        // or on the fixed cost of clearing a window-sized buffer.
        let (_, report) = renderer
            .render_to_pixels(driver.scene(), dw, dh, GROUND)
            .unwrap();
        println!(
            "  scene: {} shapes, {} glyph runs ({} glyphs), {} clips, {} skipped",
            report.shapes, report.glyph_runs, report.glyphs, report.clips, report.skipped_commands
        );
        let t = Instant::now();
        for _ in 0..10 {
            let _ = renderer.render_to_pixels(&vieww::paint::Scene::new(), dw, dh, GROUND);
        }
        println!(
            "  empty {dw}x{dh} buffer: {:.2} ms",
            t.elapsed().as_secs_f64() * 100.0
        );

        let n = 30;
        let mut pipeline = 0.0_f64;
        let mut raster = 0.0_f64;
        let mut skipped = 0_usize;
        for i in 0..n as u32 {
            // A scroll every frame, which is the worst case and the one that
            // feels laggy: everything on screen moves, so nothing is reusable.
            let event = ScrollEvent::new(
                Offset::new(w / 2.0, h / 2.0),
                Offset::new(0.0, -8.0),
                std::time::Duration::from_millis(16 * u64::from(i)),
            );
            driver.handle_scroll(&event);

            let t0 = Instant::now();
            driver.draw_frame_at(std::time::Duration::from_millis(16 * u64::from(i)));
            pipeline += t0.elapsed().as_secs_f64() * 1000.0;

            // Cull to the window before rasterising: the page is 3400 px tall
            // and the window is 900, so three quarters of every glyph run in
            // the scene is off screen and would be clipped away a pixel at a
            // time. A command whose bounds do not touch the surface cannot
            // affect the output, so dropping it is free correctness-wise.
            let t1 = Instant::now();
            let mut visible =
                vieww::paint::Damage::new(vieww::foundation::Rect::new(0.0, 0.0, w, h));
            visible.add(vieww::foundation::Rect::new(0.0, 0.0, w, h));
            let (culled, _) = driver.scene().damage_cull(&visible);
            let r = renderer
                .render_to_pixels(&culled, dw, dh, GROUND)
                .unwrap()
                .1;
            raster += t1.elapsed().as_secs_f64() * 1000.0;
            skipped += driver.scene().len() - culled.len();
            let _ = r;
        }
        let n = n as f64;
        println!("  commands skipped per frame: {}", skipped / 30);
        println!(
            "{label}: pipeline {:.2} ms, rasterise {:.2} ms, total {:.2} ms  ({:.0} fps ceiling)",
            pipeline / n,
            raster / n,
            (pipeline + raster) / n,
            1000.0 / ((pipeline + raster) / n),
        );
    }
}
