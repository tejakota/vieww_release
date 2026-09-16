//! The native half of the web certification: the same scene, the same two
//! states, rasterised on the host.
//!
//! ```console
//! cargo run --release -p test-web --example baseline -- /tmp/web-cert
//! ```
//!
//! For each state (`taps = 0`, `taps = 1`) this writes:
//!
//! * `native-<state>.png` — for the eye, and for the evidence directory;
//! * `native-<state>.rgba` — the raw straight-alpha RGBA8 buffer, which is
//!   what the browser canvas is read back as, so the comparison is bytes
//!   against bytes with no encoder in between.
//!
//! # Why `draw_frame` twice
//!
//! The first frame mounts and lays out; a tree that settles over more than
//! one pass (a `Stack` with positioned children can) finishes on the second.
//! The wasm side's `requestAnimationFrame` loop runs exactly the same
//! pipeline per frame until `scene_rebuilds` stops moving, so the settled
//! frame is what the canvas holds when the browser half is captured — and
//! this is the settled frame.

use std::path::PathBuf;

use test_web::{screen, BG, HEIGHT, WIDTH};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;
use vieww_foundation::Size;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-web-out"));
    std::fs::create_dir_all(&out)?;

    let states = [("baseline", 0_i32), ("tapped", 1_i32)];
    for (name, taps) in states {
        let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
        let runtime = driver.elements().runtime().clone();
        let signal = runtime.signal(taps);
        driver.elements().set_root(screen(signal));
        driver.draw_frame();
        driver.draw_frame();

        let mut renderer = NativeRenderer::new();
        let (png, _report) =
            renderer.render_to_png(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        std::fs::write(out.join(format!("native-{name}.png")), &png)?;

        let (pixels, _report) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        std::fs::write(out.join(format!("native-{name}.rgba")), pixels.data())?;

        println!("native-{name}: {} bytes of RGBA", pixels.data().len());
    }
    Ok(())
}
