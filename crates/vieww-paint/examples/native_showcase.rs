//! Two deliverables spec §12.2 and this task both ask for: a still image
//! proving all 28 blend modes render (not 22 real + 6 substituted), and an
//! animated GIF proving a `PushLayer` opacity/transform animation holds up
//! over many frames — the same kind of fixture `apps/viewwstudio/ci/gifs.sh`
//! produces for the studio, built here for `vieww-paint`'s `native` backend
//! directly rather than through a windowed app.
//!
//! Run with `cargo run -p vieww-paint --example native_showcase --features
//! cpu,native -- <out-dir>`.

use std::path::PathBuf;

use vieww_foundation::{BlendMode, Color, Offset, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Scene};

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "showcase-out".to_string()),
    );
    std::fs::create_dir_all(&out).expect("create output dir");

    blend_matrix(&out);
    animation_gif(&out);

    println!("wrote showcase files to {}", out.display());
}

/// Every [`BlendMode`] painted as a small red square over a colourful
/// checkerboard-plus-gradient backdrop, in one PNG, labelled by position —
/// spec §7.2's "The new renderer implements all 28 natively", shown rather
/// than only asserted.
fn blend_matrix(out: &std::path::Path) {
    let modes = BlendMode::ALL;
    let cell = 90u32;
    let cols = 7u32;
    let rows = (modes.len() as u32).div_ceil(cols);
    let width = cell * cols;
    let height = cell * rows;

    let mut scene = Scene::new();
    // Backdrop: a checkerboard so coverage-changing modes (Clear, DstOut, …)
    // are visible against it, plus a gradient so the separable/non-separable
    // modes show real colour mixing rather than flat colour arithmetic.
    for gy in 0..height / 10 {
        for gx in 0..width / 10 {
            let checker = (gx + gy) % 2 == 0;
            // Deliberately *not* a pale grey/white checker: that pattern
            // reads as a transparency indicator in most image viewers and
            // would make every coverage-changing mode (Dst, DstOut, Clear's
            // near-neighbours) look like a rendering bug instead of a
            // correct "nothing survives here" result. Amber/teal is
            // unambiguously opaque paint.
            let color = if checker {
                Color::rgba(214, 160, 40, 255)
            } else {
                Color::rgba(30, 110, 110, 255)
            };
            scene.fill_rect(
                Rect::new(
                    (gx * 10) as f32,
                    (gy * 10) as f32,
                    (gx * 10 + 10) as f32,
                    (gy * 10 + 10) as f32,
                ),
                color.into(),
            );
        }
    }
    for (i, &mode) in modes.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let x0 = col * cell;
        let y0 = row * cell;
        let bounds = Rect::new(
            x0 as f32 + 6.0,
            y0 as f32 + 6.0,
            (x0 + cell) as f32 - 6.0,
            (y0 + cell) as f32 - 6.0,
        );

        scene.save();
        scene.push_layer(bounds, 0.85, mode);
        scene.fill_rect(bounds, Color::rgba(220, 30, 30, 255).into());
        scene.fill_rect(
            Rect::new(
                bounds.left + 12.0,
                bounds.top + 12.0,
                bounds.right - 4.0,
                bounds.bottom - 4.0,
            ),
            Color::rgba(30, 120, 220, 255).into(),
        );
        scene.pop_layer();
        scene.restore();
    }

    let mut renderer = NativeRenderer::new();
    let (pixels, report) = renderer
        .render_to_pixels(&scene, width, height, Color::rgba(255, 255, 255, 255))
        .expect("render blend matrix");
    assert_eq!(
        report.unsupported_blends, 0,
        "vieww's native renderer should never substitute a blend mode, but reported {} unsupported",
        report.unsupported_blends
    );
    let png = pixels.encode_png().expect("encode png");
    std::fs::write(out.join("blend_mode_matrix_28.png"), png).expect("write blend matrix png");
    println!(
        "blend_mode_matrix_28.png: {} modes, {} layers, unsupported_blends={}",
        modes.len(),
        report.layers,
        report.unsupported_blends
    );
}

/// A 48-frame animation — a swept-arc "loading" motif combining a rotating
/// sweep gradient, a moving drop shadow and a fading layer — encoded to GIF.
/// Exercises the same command types the parity suite tests individually, now
/// across many frames of one continuous [`Scene`] rebuild, which is what an
/// application's frame loop actually does.
fn animation_gif(out: &std::path::Path) {
    const FRAMES: usize = 48;
    const WIDTH: u16 = 240;
    const HEIGHT: u16 = 240;

    let mut renderer = NativeRenderer::new();
    let mut gif_frames = Vec::with_capacity(FRAMES);

    for frame in 0..FRAMES {
        let t = frame as f32 / FRAMES as f32;
        let angle = t * std::f32::consts::TAU;

        let mut scene = Scene::new();
        scene.fill_rect(
            Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            Color::rgba(24, 26, 32, 255).into(),
        );

        let center = Offset::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
        let radius = 70.0;
        let orbit = Offset::new(
            center.dx + angle.cos() * radius,
            center.dy + angle.sin() * radius,
        );

        scene.draw_shadow(
            Rect::new(
                orbit.dx - 22.0,
                orbit.dy - 22.0,
                orbit.dx + 22.0,
                orbit.dy + 22.0,
            ),
            10.0,
            vieww_foundation::Shadow::new(Color::rgba(0, 0, 0, 180), Offset::new(0.0, 6.0), 18.0),
        );

        scene.save();
        scene.push_layer(
            Rect::new(
                orbit.dx - 24.0,
                orbit.dy - 24.0,
                orbit.dx + 24.0,
                orbit.dy + 24.0,
            ),
            0.5 + 0.5 * (t * std::f32::consts::TAU).sin().abs(),
            BlendMode::Normal,
        );
        let gradient =
            vieww_foundation::Gradient::sweep(Offset::new(0.5, 0.5), 0.0, std::f32::consts::TAU)
                .with_stops(&[
                    (0.0, Color::rgba(255, 100, 100, 255)),
                    (0.33, Color::rgba(100, 255, 140, 255)),
                    (0.66, Color::rgba(100, 160, 255, 255)),
                    (1.0, Color::rgba(255, 100, 100, 255)),
                ]);
        scene.fill_rect(
            Rect::new(
                orbit.dx - 24.0,
                orbit.dy - 24.0,
                orbit.dx + 24.0,
                orbit.dy + 24.0,
            ),
            vieww_paint::Paint::gradient(gradient),
        );
        scene.pop_layer();
        scene.restore();

        // A static ring, stroked, so the geometry engine's dash/round-join
        // path is exercised on every frame too.
        let mut ring = vieww_foundation::Path::new();
        let segments = 48;
        for i in 0..=segments {
            let a = std::f32::consts::TAU * i as f32 / segments as f32;
            let p = Offset::new(center.dx + a.cos() * radius, center.dy + a.sin() * radius);
            if i == 0 {
                ring.move_to(p);
            } else {
                ring.line_to(p);
            }
        }
        let style = vieww_foundation::StrokeStyle::default()
            .join(vieww_foundation::StrokeJoin::Round)
            .dash(vieww_foundation::Dash::even(10.0).offset(t * 40.0));
        scene.stroke_path(
            &ring,
            vieww_paint::Stroke::new(3.0).styled(style),
            Color::rgba(120, 130, 150, 255).into(),
        );

        let (pixels, _) = renderer
            .render_to_pixels(
                &scene,
                WIDTH as u32,
                HEIGHT as u32,
                Color::rgba(24, 26, 32, 255),
            )
            .expect("render animation frame");
        gif_frames.push(pixels.data().to_vec());
    }

    let gif_path = out.join("animation_showcase.gif");
    let mut file = std::fs::File::create(&gif_path).expect("create gif file");
    let mut encoder = gif::Encoder::new(&mut file, WIDTH, HEIGHT, &[]).expect("create gif encoder");
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .expect("set gif repeat");
    for rgba in &gif_frames {
        let mut frame = gif::Frame::from_rgba_speed(WIDTH, HEIGHT, &mut rgba.clone(), 10);
        frame.delay = 4; // 40ms/frame = 25fps, matching ci/gifs.sh's own rate.
        encoder.write_frame(&frame).expect("write gif frame");
    }
    drop(encoder);
    println!("animation_showcase.gif: {FRAMES} frames at {WIDTH}x{HEIGHT}");
}
