//! Rasterise the launch splash, headless, at whatever moments you ask for.
//!
//! The splash is two and a half seconds of an application nobody can screenshot
//! by hand at the right instant, and its first version was broken in a way
//! nothing but a picture would have shown — it was *in the tree* on every
//! launch and drew nothing at all. This is how it is reviewed: the same shell,
//! the same widget, the frame clock supplied rather than read off the wall, so
//! a given millisecond always produces the same PNG.
//!
//! ```console
//! cargo run -p viewwstudio --example splash_probe -- out/ [--small|--tiny] [--every=40]
//! ```
//!
//! `--every=40` walks the whole animation at 25 frames a second, which is what
//! to feed to a GIF or a WebP encoder. Without it you get the dozen moments
//! worth looking at one at a time.

use std::path::PathBuf;

use vieww_foundation::Size;
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio, Workspace, WINDOW};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args
        .next()
        .map_or_else(|| PathBuf::from("splash"), PathBuf::from);
    let rest: Vec<String> = args.collect();
    let small = rest.iter().any(|a| a == "--small");
    std::fs::create_dir_all(&out).expect("creating the output directory");

    let tiny = rest.iter().any(|a| a == "--tiny");
    let window = if tiny {
        Size::new(560.0, 360.0)
    } else if small {
        Size::new(1366.0, 679.0)
    } else {
        WINDOW
    };

    let mut driver = FrameDriver::new(window);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/screens"));
    let studio = Studio::with_workspace(&runtime, Workspace::open(&root));
    studio.note_window_size(window);
    studio.splash.set(true);
    driver.set_root(Shell {
        studio: studio.clone(),
    });

    let mut cpu = vieww_paint::native::NativeRenderer::new();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window size"
    )]
    let (width, height) = (window.width as u32, window.height as u32);

    // Deterministic: the splash's clock is the frame's own timestamp, through
    // `Animated`, so the same millisecond always rasterises the same picture.
    // `--every=40` walks the whole thing at 25fps, which is what proves it is
    // an animation rather than a picture.
    let every: Option<u64> = rest
        .iter()
        .find_map(|a| a.strip_prefix("--every="))
        .and_then(|n| n.parse().ok());
    let frames: Vec<u64> = match every {
        Some(step) => (0..=2500).step_by(step as usize).collect(),
        None => vec![
            0, 150, 300, 500, 700, 900, 1200, 1600, 2100, 2300, 2400, 2500,
        ],
    };

    for ms in frames {
        driver.draw_frame_at(std::time::Duration::from_millis(ms));
        let background = viewwstudio::StudioTheme::dark().window;
        let (png, _) = cpu
            .render_to_png(driver.scene(), width, height, background)
            .expect("rasterising");
        let path = out.join(format!("splash-{ms:04}.png"));
        std::fs::write(&path, png).expect("writing the PNG");
        println!("{}", path.display());
    }
}
