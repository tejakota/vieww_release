//! Where do the Studio's milliseconds actually go?
//!
//! Renders the real shell's scene, then renders deliberately mutilated copies
//! of it — the same commands with the clips stripped, with the glyphs
//! removed, with the shadows removed — and reports the difference. Nothing
//! here is a benchmark of a primitive in isolation; it is the *application's
//! own scene*, minus one thing at a time, which is the only attribution that
//! accounts for how the pieces interact.

use std::time::Instant;

use vieww_foundation::{Color, Rect};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Command, Scene};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

fn time(label: &str, scene: &Scene, w: u32, h: u32) -> f64 {
    let mut renderer = NativeRenderer::new();
    let _ = renderer
        .render_to_pixels(scene, w, h, Color::BLACK)
        .unwrap();
    let mut best = f64::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        let _ = renderer
            .render_to_pixels(scene, w, h, Color::BLACK)
            .unwrap();
        best = best.min(start.elapsed().as_secs_f64());
    }
    println!(
        "{label:<40} {:>9.2} ms   ({} commands)",
        best * 1000.0,
        scene.len()
    );
    best
}

/// A copy of `scene` keeping only the commands `keep` accepts.
fn filtered(scene: &Scene, keep: impl Fn(&Command) -> bool) -> Scene {
    let mut out = Scene::new();
    for command in scene.commands() {
        // Layer markers always travel in pairs; dropping one half of a pair
        // makes an unbalanced scene the renderer rejects outright.
        let is_layer = matches!(command, Command::PushLayer { .. } | Command::PopLayer);
        if is_layer || keep(command) {
            out.push_command(command.clone());
        }
    }
    out
}

fn main() {
    let size = viewwstudio::WINDOW;
    let (w, h) = (size.width as u32, size.height as u32);
    let mut driver = FrameDriver::new(size);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    driver.set_root(Shell {
        studio: Studio::new(&runtime),
    });
    driver.draw_frame();
    let scene = driver.scene().clone();

    println!("studio shell at {w}x{h}, {} commands\n", scene.len());
    let full = time("everything (the real frame)", &scene, w, h);

    let no_glyphs = filtered(&scene, |c| !matches!(c, Command::DrawGlyphs { .. }));
    let a = time("without text", &no_glyphs, w, h);

    let no_shadows = filtered(&scene, |c| !matches!(c, Command::DrawShadow { .. }));
    let b = time("without shadows", &no_shadows, w, h);

    let only_glyphs = filtered(&scene, |c| matches!(c, Command::DrawGlyphs { .. }));
    let d = time("text alone", &only_glyphs, w, h);

    let empty = Scene::new();
    let floor = time("nothing at all (the floor)", &empty, w, h);

    let shapes_only = filtered(&scene, |c| {
        matches!(
            c,
            Command::FillRect { .. } | Command::FillPath { .. } | Command::StrokePath { .. }
        )
    });
    let e = time("shapes alone", &shapes_only, w, h);

    println!(
        "\n  floor (buffer + readback)      {:>7.1} ms",
        floor * 1000.0
    );
    println!(
        "  text                           {:>7.1} ms",
        (full - a) * 1000.0
    );
    println!(
        "  shadows                        {:>7.1} ms",
        (full - b) * 1000.0
    );
    println!("  shapes alone (incl. floor)     {:>7.1} ms", e * 1000.0);
    println!("  text alone (incl. floor)       {:>7.1} ms", d * 1000.0);

    // How much ink does the frame actually lay down? A frame is expensive
    // because of *area*, not because of command count, and a screen built out
    // of stacked full-width panels can paint its own pixels many times over.
    let mut ink = 0.0f64;
    for command in scene.commands() {
        let b = command.bounds();
        if b.width() > 0.0 && b.height() > 0.0 {
            ink += f64::from(b.width() * b.height());
        }
    }
    let surface_px = f64::from(size.width * size.height);
    println!(
        "\n  ink: {:.1} megapixels over a {:.1} megapixel window — {:.1}x overdraw",
        ink / 1e6,
        surface_px / 1e6,
        ink / surface_px
    );

    // How much of that overdraw is *recoverable*? A command is invisible if
    // something opaque, drawn later, covers it completely. The cheap test —
    // "contained in one single later opaque rectangle" — is what an occlusion
    // pass would implement first, so measuring it before writing it says
    // whether writing it is worth anything.
    //
    // Only root-level commands count as coverers: anything inside a layer may
    // be composited with an alpha or a blend mode, and "opaque" stops meaning
    // opaque the moment either is in play.
    let commands = scene.commands();
    let mut opaque_rects: Vec<(usize, Rect)> = Vec::new();
    let mut depth = 0i32;
    for (i, command) in commands.iter().enumerate() {
        match command {
            Command::PushLayer { .. } => depth += 1,
            Command::PopLayer => depth -= 1,
            Command::FillRect {
                rect,
                paint,
                transform,
                clip,
            } if depth == 0 => {
                let axis_aligned = transform.b == 0.0 && transform.c == 0.0;
                if paint.gradient.is_none()
                    && paint.color.a == 255
                    && axis_aligned
                    && clip.shapes().is_empty()
                {
                    let mut covered = transform.apply_rect(*rect);
                    if let Some(bounds) = clip.bounds() {
                        covered = covered.intersect(bounds);
                    }
                    if !covered.is_empty() {
                        opaque_rects.push((i, covered));
                    }
                }
            }
            _ => {}
        }
    }

    let contains = |outer: Rect, inner: Rect| {
        outer.left <= inner.left
            && outer.top <= inner.top
            && outer.right >= inner.right
            && outer.bottom >= inner.bottom
    };

    let mut hidden = 0usize;
    let mut hidden_ink = 0.0f64;
    for (i, command) in commands.iter().enumerate() {
        let b = command.bounds();
        if b.width() <= 0.0 || b.height() <= 0.0 {
            continue;
        }
        if opaque_rects
            .iter()
            .any(|(j, rect)| *j > i && contains(*rect, b))
        {
            hidden += 1;
            hidden_ink += f64::from(b.width() * b.height());
        }
    }

    println!(
        "  occlusion: {} opaque root fills; {hidden} of {} commands are fully covered \
         by a later one ({:.1} of {:.1} megapixels of ink, {:.0}%)",
        opaque_rects.len(),
        commands.len(),
        hidden_ink / 1e6,
        ink / 1e6,
        hidden_ink / ink * 100.0
    );

    // How many clips are shape clips (needing a rasterised mask) rather than
    // plain rectangles (which cost nothing but a tighter bounding box)?
    let mut rect_clips = 0usize;
    let mut shape_clips = 0usize;
    let mut clip_area = 0.0f64;
    for command in scene.commands() {
        let clip = command.clip();
        if clip.is_none() {
            continue;
        }
        if clip.shapes().is_empty() {
            rect_clips += 1;
        } else {
            shape_clips += 1;
            if let Some(bounds) = clip.bounds() {
                clip_area += f64::from(bounds.width() * bounds.height());
            }
        }
    }
    println!(
        "\nclips: {rect_clips} rectangular (free), {shape_clips} shape clips \
         needing a mask, {:.1} megapixels of mask if each is rasterised once",
        clip_area / 1e6
    );
}
