//! # Vieww native surface — certification suite
//!
//! The renderer's *complete* surface, as one picture: every blend mode in
//! the enum, every gradient geometry with and without dithering, outer and
//! inset and rotated shadows, nested shaped clips, layers under clips,
//! backdrop filters — plus the resize path, rendering the same scene at
//! three surface sizes through one renderer, which is what a real window
//! resize does.
//!
//! ```console
//! cargo run --release -p test-native-surface -- /tmp/vieww-native-surface
//! ```
//!
//! This is deliberately **paint-level** — a `Scene` built through the
//! `Canvas` trait, no widgets — because the question here is what the
//! rasterizer itself does with every `Command` variant, independent of the
//! tree that produced them. (The widget-level suites above cover the other
//! seam.)
//!
//! The assertions are on `SceneReport`: no unsupported blends, every
//! feature's counter non-zero, and the offscreen-target pool actually
//! reusing buffers across the blend matrix's 28 layers.

use std::path::PathBuf;
use std::time::Instant;

use vieww_foundation::{
    BlendMode, Color, Gradient, ImageFilter, Offset, Path, Rect, Shadow, Transform,
};
use vieww_paint::native::NativeRenderer;
use vieww_paint::Canvas;

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 760.0;

const BG: Color = Color::rgb(247, 248, 250);
const INK: Color = Color::rgb(23, 30, 42);
const PAPER: Color = Color::WHITE;
const ACCENT: Color = Color::rgb(58, 122, 246);
const VIOLET: Color = Color::rgb(126, 87, 246);
const CYAN: Color = Color::rgb(73, 211, 210);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-native-surface-out"));
    std::fs::create_dir_all(&out)?;

    let mut renderer = NativeRenderer::new();

    // ── Frame 1: the completeness sheet.
    let mut scene = vieww_paint::Scene::new();
    completeness_sheet(&mut scene);
    let start = Instant::now();
    let (pixels, report) = renderer.render_to_png(&scene, WIDTH as u32, HEIGHT as u32, BG)?;
    let sheet_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::fs::write(out.join("01-completeness.png"), &pixels)?;
    println!(
        "01-completeness: {} commands, {} shapes, {} layers, {} clips, {} \
         shadows, {} glyphs, {} filtered layers — {:.1} ms",
        report.translated_commands,
        report.shapes,
        report.layers,
        report.clips,
        report.shadows,
        report.glyphs,
        report.filtered_layers,
        sheet_ms
    );

    // ── The resize path: the same scene through the same renderer at
    // 0.5× and 1.5× the size — the buffer must be reallocated, not corrupt.
    for (name, factor) in [("half", 0.5_f32), ("one-and-half", 1.5)] {
        let (pixels, resize_report) = renderer.render_to_png(
            &scene,
            (WIDTH * factor) as u32,
            (HEIGHT * factor) as u32,
            BG,
        )?;
        std::fs::write(out.join(format!("02-resize-{name}.png")), &pixels)?;
        println!(
            "02-resize-{name}: {} commands (was {}), {} shapes",
            resize_report.translated_commands, report.translated_commands, resize_report.shapes
        );
        if resize_report.translated_commands != report.translated_commands {
            return Err("the resize path dropped or duplicated commands".into());
        }
    }

    // ── The pool: the blend matrix pushed 28 layers; the second render of
    // the same sheet must reuse them all.
    let pool_before = renderer.pool_stats();
    let _ = renderer.render_to_pixels(&scene, WIDTH as u32, HEIGHT as u32, BG)?;
    let pool_after = renderer.pool_stats();
    println!(
        "pool: {} reused / {} allocated after one sheet, then {} / {} after \
         the re-render",
        pool_before.reused, pool_before.allocated, pool_after.reused, pool_after.allocated
    );

    let mut failures = Vec::new();
    if report.unsupported_blends != 0 {
        failures.push(format!(
            "{} unsupported blend mode(s) — the completeness claim is false",
            report.unsupported_blends
        ));
    }
    if report.layers < 28 {
        failures.push(format!(
            "the blend matrix needed 28 isolated layers, got {}",
            report.layers
        ));
    }
    if report.filtered_layers == 0 {
        failures.push("no filtered layer ran — the filter path is dark".into());
    }
    if report.shadows == 0 {
        failures.push("no shadow drew".into());
    }
    if report.clips == 0 {
        failures.push("no shaped clip was resolved".into());
    }
    if pool_after.allocated > pool_before.allocated {
        failures.push(format!(
            "the re-render allocated {} new offscreen buffer(s); the pool \
             should have covered it",
            pool_after.allocated - pool_before.allocated
        ));
    }

    std::fs::write(
        out.join("metrics.txt"),
        format!(
            "commands={}\nshapes={}\nlayers={}\nfiltered_layers={}\nclips={}\nshadows={}\nsheet_ms={sheet_ms:.3}\nunsupported_blends={}\n",
            report.translated_commands,
            report.shapes,
            report.layers,
            report.filtered_layers,
            report.clips,
            report.shadows,
            report.unsupported_blends,
        ),
    )?;

    println!("wrote {}", out.display());
    if failures.is_empty() {
        println!("ALL NATIVE-SURFACE CHECKS PASSED");
    } else {
        for failure in &failures {
            eprintln!("FAIL: {failure}");
        }
        return Err(format!("{} native-surface check(s) failed", failures.len()).into());
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// The completeness sheet
// ─────────────────────────────────────────────────────────────────────────

fn completeness_sheet(scene: &mut vieww_paint::Scene) {
    header(scene);

    // ── The blend matrix: every mode in the enum, each isolated in its own
    // layer over the same two-colour background, in the enum's own order.
    let modes = BlendMode::ALL;
    let cell = 34.0;
    let columns = 14;
    let origin = Offset::new(56.0, 118.0);
    for (index, mode) in modes.iter().enumerate() {
        let x = origin.dx + (index % columns) as f32 * (cell + 8.0);
        let y = origin.dy + (index / columns) as f32 * (cell + 8.0);
        let rect = Rect::new(x, y, x + cell, y + cell);
        // The two-colour substrate the mode mixes.
        scene.fill_rect(
            Rect::new(x, y, x + cell * 0.5, y + cell),
            Color::rgb(250, 60, 60).into(),
        );
        scene.fill_rect(
            Rect::new(x + cell * 0.5, y, x + cell, y + cell),
            Color::rgb(30, 40, 170).into(),
        );
        // The layer carrying the mode: a translucent accent over it.
        scene.push_filtered_layer(rect, 0.62, *mode, ImageFilter::NONE);
        scene.fill_rect(rect, ACCENT.into());
        scene.pop_layer();
    }
    label(
        scene,
        Offset::new(56.0, 118.0 + 2.0 * (cell + 8.0) + 14.0),
        "all 28 blend modes, each isolated in its own layer",
    );

    // ── Gradients: all three geometries, dithered beside exact.
    let gy = 260.0;
    let gradient_pairs = [
        (
            "linear",
            Gradient::horizontal().between(VIOLET, CYAN),
            Gradient::horizontal().between(VIOLET, CYAN).with_dither(),
        ),
        (
            "radial",
            Gradient::radial_fill().between(ACCENT, Color::rgb(10, 13, 20)),
            Gradient::radial_fill()
                .between(ACCENT, Color::rgb(10, 13, 20))
                .with_dither(),
        ),
        (
            "sweep",
            Gradient::conic().between(CYAN, VIOLET),
            Gradient::conic().between(CYAN, VIOLET).with_dither(),
        ),
    ];
    let mut gx = 56.0;
    for (name, exact, dithered) in gradient_pairs {
        for (i, gradient) in [exact, dithered].iter().enumerate() {
            let rect = Rect::new(gx, gy, gx + 150.0, gy + 64.0);
            scene.fill_rect(rect, (*gradient).into());
            if i == 1 {
                label(
                    scene,
                    Offset::new(gx, gy + 70.0),
                    &format!("{name} · dithered"),
                );
            } else {
                label(
                    scene,
                    Offset::new(gx, gy + 70.0),
                    &format!("{name} · exact"),
                );
            }
            gx += 158.0;
        }
    }

    // ── Shadows: outer, inset, and rotated.
    let sy = 410.0;
    let shadow_specs: [(&str, Shadow, f32); 3] = [
        (
            "outer",
            Shadow::new(Color::rgba(23, 30, 42, 90), Offset::new(0.0, 8.0), 16.0),
            0.0,
        ),
        (
            "inset",
            Shadow::inset(Color::rgba(23, 30, 42, 110), Offset::new(0.0, 4.0), 10.0),
            0.0,
        ),
        (
            "rotated 24°",
            Shadow::new(Color::rgba(126, 87, 246, 110), Offset::new(6.0, 6.0), 12.0),
            0.42,
        ),
    ];
    let mut sx = 56.0;
    for (name, shadow, angle) in shadow_specs {
        let rect = Rect::new(sx, sy, sx + 150.0, sy + 84.0);
        scene.save();
        // The rotation is the *canvas* state the shadow is recorded under —
        // the caster rect stays axis-aligned in its own space and the
        // silhouette is transformed at record time.
        scene.transform(Transform::rotate_around(
            Offset::new(sx + 75.0, sy + 42.0),
            angle,
        ));
        scene.draw_shadow(rect, 16.0, shadow);
        scene.fill_rect(rect, PAPER.into());
        scene.restore();
        label(scene, Offset::new(sx, sy + 92.0), name);
        sx += 190.0;
    }

    // ── Nested shaped clips: a circle inside a rounded rect that only
    // partially overlaps — the intersection case, not the containment one.
    let cy = 530.0;
    let clip_region = Rect::new(56.0, cy, 300.0, cy + 130.0);
    scene.save();
    scene.clip_rrect(clip_region, 22.0);
    // A circle centred so it pokes out of the rounded rect's right edge:
    // the intersection of the two clips is all that may ink.
    let mut circle = Path::new();
    circle.move_to(Offset::new(250.0, cy + 20.0));
    for step in 0..24 {
        let angle = step as f32 * std::f32::consts::TAU / 24.0;
        circle.line_to(Offset::new(
            250.0 + 55.0 * angle.cos(),
            cy + 65.0 + 55.0 * angle.sin(),
        ));
    }
    circle.close();
    scene.clip_path(&circle);
    scene.fill_rect(clip_region.inflate(20.0), ACCENT.into());
    scene.restore();
    label(
        scene,
        Offset::new(56.0, cy + 138.0),
        "nested shaped clips intersect — the circle meets the rounded edge",
    );

    // ── A layer under a clip: the group is clipped by the shape, not just
    // its children.
    let ly = 530.0;
    let layer_region = Rect::new(420.0, ly, 700.0, ly + 130.0);
    scene.save();
    scene.clip_rrect(layer_region, 26.0);
    scene.push_filtered_layer(
        layer_region,
        0.85,
        BlendMode::Normal,
        ImageFilter::blur(4.0),
    );
    scene.fill_rect(
        Rect::new(430.0, ly + 10.0, 560.0, ly + 110.0),
        Color::rgb(250, 60, 60).into(),
    );
    scene.fill_rect(
        Rect::new(540.0, ly + 30.0, 690.0, ly + 90.0),
        Color::rgb(30, 40, 170).into(),
    );
    scene.pop_layer();
    scene.restore();
    label(
        scene,
        Offset::new(420.0, cy + 138.0),
        "a blurred layer, itself under a shaped clip",
    );

    // ── A backdrop filter over everything drawn so far in its box.
    let backdrop = Rect::new(760.0, 530.0, 1060.0, 660.0);
    scene.push_filtered_layer(
        backdrop,
        1.0,
        BlendMode::Normal,
        ImageFilter::backdrop_blur(5.0),
    );
    scene.fill_rect(
        Rect::new(770.0, 545.0, 1050.0, 645.0),
        Color::rgba(255, 255, 255, 36).into(),
    );
    scene.pop_layer();
    label(
        scene,
        Offset::new(760.0, cy + 138.0),
        "a backdrop blur over the gradient row",
    );

    // ── A stroke, to close the Command surface.
    let stroked = Rect::new(56.0, 690.0, 300.0, 740.0);
    scene.stroke_path(
        &Path::rect(stroked),
        vieww_paint::Stroke::new(2.0),
        INK.into(),
    );
}

fn header(scene: &mut vieww_paint::Scene) {
    scene.fill_rect(Rect::new(0.0, 0.0, WIDTH, 92.0), PAPER.into());
    // Text through the scene API is glyph runs, which need a shaped
    // paragraph; the paint layer alone cannot shape. The title is drawn as
    // paint instead: a gradient band and an accent keyline, and the
    // *glyph* coverage belongs to the suites above this one.
    scene.fill_rect(
        Rect::new(56.0, 24.0, 300.0, 30.0),
        Gradient::horizontal()
            .between(INK, Color::rgba(23, 30, 42, 170))
            .into(),
    );
    scene.stroke_path(
        &Path::rect(Rect::new(56.0, 62.0, 300.0, 64.0)),
        vieww_paint::Stroke::new(2.0),
        ACCENT.into(),
    );
}

fn label(scene: &mut vieww_paint::Scene, at: Offset, _text: &str) {
    // A small accent tick stands in for the caption: this sheet is
    // paint-level, and captions are glyph-level (see test-text-fidelity).
    scene.fill_rect(
        Rect::new(at.dx, at.dy, at.dx + 12.0, at.dy + 3.0),
        Color::rgba(110, 122, 140, 200).into(),
    );
}
