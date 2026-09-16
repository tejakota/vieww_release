//! What a keystroke actually costs to rasterise — damaged region, not the
//! whole window.
//!
//! ```console
//! cargo run --release -p viewwstudio --example raster_real
//! ```
//!
//! # Why the existing bench's raster number is not the shipping one
//!
//! `examples/bench.rs` rasterises `driver.scene()` over the full 1440×900
//! surface, every sample. That is the right measure for "how much is in this
//! scene", which is what it is there for — but it is not what a frame costs,
//! because the platform does not repaint the window on a keystroke. It repaints
//! the damage: `FrameDriver::damage_culled_scene` drops every command outside
//! the changed region, and `NativeSurface::present_damaged` uploads only those
//! rows.
//!
//! Quoting the full-surface figure as the cost of a frame overstates it by
//! whatever ratio the damage happens to be, and in a text editor that ratio is
//! large — one line of a file changed, and the other eight hundred did not.
//! This measures both and prints the ratio, so the honest number is the one in
//! front of you.
//!
//! # This is the CPU rasteriser, and that is not a fallback
//!
//! Worth stating plainly, because the name invites the opposite assumption:
//! `vieww-platform-winit` rasterises with `vieww_paint::native::NativeRenderer`
//! and uses `vieww-hal` to *present* the result. There is no
//! `vieww_paint::gpu` module in this tree — several comments refer to a
//! `GpuRenderer` that does not exist yet. So this is not the slow path; on
//! desktop today it is the only path, and its cost is the frame's cost.

use std::time::{Duration, Instant};

use vieww_foundation::{Color, Offset, Size, Transform};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

const WINDOW: Size = Size {
    width: 1440.0,
    height: 900.0,
};

fn median(samples: &mut [Duration]) -> f64 {
    samples.sort_unstable();
    samples
        .get(samples.len() / 2)
        .copied()
        .unwrap_or_default()
        .as_secs_f64()
        * 1e3
}

fn main() {
    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(WINDOW);
    studio.splash.set(false);
    driver.set_root(Shell {
        studio: studio.clone(),
    });

    // Settle, so nothing below is measuring first-frame work.
    for step in 0..40 {
        driver.draw_frame_at(Duration::from_millis(step * 16));
    }

    let mut renderer = vieww_paint::native::NativeRenderer::new();
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (full_w, full_h) = (WINDOW.width as u32, WINDOW.height as u32);

    let mut full_samples = Vec::new();
    let mut damaged_samples = Vec::new();
    let mut ratios = Vec::new();
    let mut last_damage = None;

    let start = studio
        .active()
        .map(|buffer| buffer.value.text.clone())
        .unwrap_or_default();

    let mut now = 1_000u64;
    for i in 0..24u32 {
        // One character, the interaction with the tightest budget there is.
        // Seeded exactly as `bench.rs` does it, so the two are comparable.
        let mut text = start.clone();
        // `try_from` rather than an `as` cast behind `#[expect]`: newer clippy
        // proves `i % 26` fits a `u8` and then fails the unfulfilled expectation.
        text.insert(0, char::from(b'a' + u8::try_from(i % 26).unwrap_or(0)));
        studio.edit(vieww_foundation::TextEditingValue {
            text,
            selection: vieww_foundation::TextSelection::collapsed(1),
            composing: None,
            secondary: Vec::new(),
        });
        now += 16;
        driver.draw_frame_at(Duration::from_millis(now));

        let damage = driver.damage().clone();
        let bounds = damage.bounds();
        let (culled, _) = driver.damage_culled_scene();

        // The damaged rectangle, rendered into a buffer its own size — which is
        // what presenting damage does. Translated so the region's top-left is
        // the buffer's origin.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (dw, dh) = (
            bounds.width().ceil().max(1.0) as u32,
            bounds.height().ceil().max(1.0) as u32,
        );
        let mut moved = vieww_paint::Scene::new();
        moved.append(
            &culled,
            Transform::translate(Offset::new(-bounds.left, -bounds.top)),
        );

        let at = Instant::now();
        let _ = renderer
            .render_to_pixels(&moved, dw, dh, Color::BLACK)
            .expect("rasterising the damaged region");
        damaged_samples.push(at.elapsed());

        let at = Instant::now();
        let _ = renderer
            .render_to_pixels(driver.scene(), full_w, full_h, Color::BLACK)
            .expect("rasterising the full surface");
        full_samples.push(at.elapsed());

        ratios
            .push(f64::from(dw) * f64::from(dh) / (f64::from(full_w) * f64::from(full_h)) * 100.0);
        last_damage = Some((dw, dh, damage.regions().len()));
    }

    let full = median(&mut full_samples);
    let damaged = median(&mut damaged_samples);
    let area = ratios.iter().sum::<f64>() / ratios.len() as f64;

    println!("rasterising one keystroke, {full_w}×{full_h} window\n");
    if let Some((dw, dh, regions)) = last_damage {
        println!("  damaged region            {dw}×{dh} px, {regions} region(s)");
        println!("  as a share of the window  {area:.1}%\n");
    }
    println!("                              median");
    println!("  full surface            {full:>9.2} ms   (what bench.rs reports)");
    println!("  damaged region only     {damaged:>9.2} ms   (what a frame repaints)");
    if damaged > 0.0 {
        println!(
            "\n  the full-surface figure overstates a keystroke by {:.1}×",
            full / damaged
        );
    }
}
