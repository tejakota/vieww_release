//! Real GPU workload for Vieww's Vulkan execution path.
//!
//! This is intentionally different from `examples/fixtures`: `fixtures`
//! validates the shipped CPU rasterizer. This package sends real `ScenePlan`s
//! into `vieww_hal::vulkan::SceneRenderer`, reads pixels back, compares every
//! frame against `NativeRenderer`, and writes the sequence as a GIF so a
//! reviewer can inspect actual GPU output.
//!
//! The animated scene uses every GPU capability at once: gradients (linear,
//! radial, sweep), images (magnified, rotated, minified through mips), outer
//! and inset shadows, rectangular and rounded clips, group opacity, non-normal
//! blend modes, a layer blur, a colour matrix and a backdrop blur.
//!
//! What it measures, rather than asserts by construction:
//!
//! - `unsupported_gpu_commands` — summed from every plan's
//!   `ScenePlan::unsupported`, not written as a literal.
//! - `parity_*` — per-frame disagreement with the CPU renderer.
//!
//! And it keeps the honesty rule the first version of this probe established:
//! **incomplete GPU work must become an explicit capability result, not a
//! visually incorrect approximation.** The remaining gaps are resource limits,
//! so the negative probes exercise those.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gif::{Encoder, Frame, Repeat};
use vieww_foundation::{
    BlendMode, Color, Gradient, Image, ImageFilter, Offset, Path, Rect, Shadow, Transform,
};
use vieww_gpu::{plan_scene, Planner, Unsupported, MAX_TARGET_DEPTH};
use vieww_hal::vulkan::{SceneRenderer, VulkanDevice};
use vieww_hal::Device;
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Clip, Command, Paint, Scene};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;
const FRAMES: usize = 24;
const FRAME_DELAY_CS: u16 = 5;
/// A per-channel difference above this counts as a disagreeing pixel.
const PARITY_TOLERANCE: u8 = 8;
/// The fraction of disagreeing pixels a frame may have. Non-zero only because
/// the GPU fills rotated/curved *geometry* without edge antialiasing (masks
/// and clips are exact); an executor bug moves far more pixels than this.
const PARITY_MAX_FRACTION: f64 = 0.02;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gpu-test-out".into())
        .into();
    std::fs::create_dir_all(&out)?;

    let device = match VulkanDevice::new() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("test-gpu-work: no Vulkan device: {error}");
            std::process::exit(2);
        }
    };

    println!("adapter.name={}", device.info().name);
    println!("adapter.type={}", device.info().device_type);
    println!("adapter.backend={}", device.info().backend);

    let mut renderer = SceneRenderer::new(&device)?;
    let mut cpu = NativeRenderer::new();
    let gif_path = out.join("gpu-scene.gif");
    let mut encoder = Encoder::new(
        std::fs::File::create(&gif_path)?,
        WIDTH as u16,
        HEIGHT as u16,
        &[],
    )?;
    encoder.set_repeat(Repeat::Infinite)?;

    let mut planner = Planner::default();
    let mut frame_0: Option<Vec<u8>> = None;
    let mut changed = 0usize;
    let mut total_triangles = 0usize;
    let mut total_draw_calls = 0usize;
    let mut total_offscreen = 0usize;
    let mut unsupported_total = 0usize;
    let mut total_gpu = Duration::ZERO;
    let mut worst_gpu = Duration::ZERO;
    let mut worst_fraction = 0.0f64;
    let mut worst_diff = 0u8;

    for i in 0..FRAMES {
        let progress = i as f32 / (FRAMES - 1) as f32;
        let scene = gpu_scene(progress);
        let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
        unsupported_total += plan.unsupported.values().sum::<usize>();
        total_triangles += plan.triangle_count();
        total_draw_calls += plan.draw_call_count();
        total_offscreen += plan.offscreen_step_count();

        let start = Instant::now();
        let pixels =
            renderer.render_planned(&planner, &plan, WIDTH, HEIGHT, Color::rgb(8, 11, 18))?;
        let elapsed = start.elapsed();
        total_gpu += elapsed;
        worst_gpu = worst_gpu.max(elapsed);

        let (reference, _) = cpu.render_to_pixels(&scene, WIDTH, HEIGHT, Color::rgb(8, 11, 18))?;
        let (over, max) = disagreement(&pixels, reference.data());
        let fraction = over as f64 / f64::from(WIDTH * HEIGHT);
        if fraction > worst_fraction || i == 0 {
            // Evidence for the number: the worst frame on both renderers.
            std::fs::write(
                out.join("parity-worst-gpu.png"),
                vieww_paint::native::Pixels::from_rgba8(pixels.clone(), WIDTH, HEIGHT)
                    .encode_png()?,
            )?;
            std::fs::write(out.join("parity-worst-cpu.png"), reference.encode_png()?)?;
            std::fs::write(
                out.join("parity-worst-frame.txt"),
                format!("frame={i}\nfraction={fraction:.5}\n"),
            )?;
        }
        worst_fraction = worst_fraction.max(fraction);
        worst_diff = worst_diff.max(max);

        if let Some(first) = &frame_0 {
            if pixels != *first {
                changed += 1;
            }
        } else {
            frame_0 = Some(pixels.clone());
            changed += 1;
        }

        let mut rgba = pixels;
        let mut frame = Frame::from_rgba_speed(WIDTH as u16, HEIGHT as u16, &mut rgba, 10);
        frame.delay = FRAME_DELAY_CS;
        encoder.write_frame(&frame)?;
    }

    let metrics = format!(
        "frames={FRAMES}\n\
         changed_from_first={changed}\n\
         mean_triangles={:.3}\n\
         mean_draw_calls={:.3}\n\
         mean_offscreen_steps={:.3}\n\
         gpu_render_mean_ms={:.3}\n\
         gpu_render_worst_ms={:.3}\n\
         parity_tolerance={PARITY_TOLERANCE}\n\
         parity_worst_fraction={worst_fraction:.5}\n\
         parity_worst_channel_diff={worst_diff}\n\
         unsupported_gpu_commands={unsupported_total}\n",
        total_triangles as f64 / FRAMES as f64,
        total_draw_calls as f64 / FRAMES as f64,
        total_offscreen as f64 / FRAMES as f64,
        total_gpu.as_secs_f64() * 1000.0 / FRAMES as f64,
        worst_gpu.as_secs_f64() * 1000.0,
    );
    print!("{metrics}");
    println!("gif={}", gif_path.display());
    std::fs::write(out.join("metrics.txt"), &metrics)?;

    if unsupported_total != 0 {
        return Err(
            format!("the showcase plan had {unsupported_total} unsupported commands").into(),
        );
    }
    if worst_fraction > PARITY_MAX_FRACTION {
        return Err(format!(
            "GPU/CPU parity: {:.2}% of a frame differs by more than {PARITY_TOLERANCE}",
            worst_fraction * 100.0
        )
        .into());
    }
    if changed < FRAMES / 2 {
        return Err(format!("GPU sequence moved too little: {changed}/{FRAMES}").into());
    }

    // Resize and reuse: same renderer, a larger target.
    let resize_scene = gpu_scene(0.5);
    let resize_plan = planner.plan(&resize_scene, (WIDTH * 2) as f32, (HEIGHT * 2) as f32);
    let pixels = renderer.render_planned(
        &planner,
        &resize_plan,
        WIDTH * 2,
        HEIGHT * 2,
        Color::rgb(8, 11, 18),
    )?;
    assert_eq!(pixels.len(), (WIDTH * 2 * HEIGHT * 2 * 4) as usize);
    println!("resize=PASS");

    // Positive capability probes: every one of these was a reported gap.
    for (name, scene) in capability_probes() {
        let mut planner = Planner::default();
        let plan = planner.plan(&scene, 320.0, 200.0);
        if !plan.is_complete() {
            return Err(format!("{name} should be GPU-complete: {:?}", plan.unsupported).into());
        }
        renderer.render_planned(&planner, &plan, 320, 200, Color::WHITE)?;
        println!("supported.{name}=PASS");
    }

    // Negative probes: resource limits are still reported, never flattened.
    let mut deep = Scene::default();
    for _ in 0..MAX_TARGET_DEPTH + 2 {
        push_layer(
            &mut deep,
            0.9,
            BlendMode::Normal,
            ImageFilter::NONE,
            Clip::NONE,
        );
    }
    fill_rect(
        &mut deep,
        Rect::new(0.0, 0.0, 10.0, 10.0),
        Color::RED,
        Transform::IDENTITY,
        Clip::NONE,
    );
    for _ in 0..MAX_TARGET_DEPTH + 2 {
        deep.push_command(Command::PopLayer);
    }
    expect_gap(
        "layer-depth-limit",
        &Planner::default().plan(&deep, 100.0, 100.0),
        Unsupported::Layer,
    )?;
    let mut shaped = Scene::default();
    fill_rect(
        &mut shaped,
        Rect::new(0.0, 0.0, 50.0, 50.0),
        Color::RED,
        Transform::IDENTITY,
        rounded(Rect::new(0.0, 0.0, 50.0, 50.0), 10.0),
    );
    expect_gap(
        "shaped-clip-without-atlas",
        &plan_scene(&shaped, 100.0, 100.0),
        Unsupported::ShapedClip,
    )?;

    Ok(())
}

/// Pixels whose worst channel differs by more than the tolerance, and the
/// worst channel difference overall.
fn disagreement(gpu: &[u8], cpu: &[u8]) -> (usize, u8) {
    let mut over = 0;
    let mut max = 0;
    for (g, c) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)) {
        let d = (0..4).map(|k| g[k].abs_diff(c[k])).max().unwrap_or(0);
        max = max.max(d);
        if d > PARITY_TOLERANCE {
            over += 1;
        }
    }
    (over, max)
}

fn expect_gap(name: &str, plan: &vieww_gpu::ScenePlan, kind: Unsupported) -> Result<(), String> {
    if plan.unsupported.get(&kind) != Some(&1) {
        return Err(format!(
            "{name}: expected exactly one {kind:?} gap, got {:?}",
            plan.unsupported
        ));
    }
    println!("unsupported.{name}=HONESTLY_REPORTED");
    Ok(())
}

fn rounded(rect: Rect, radius: f32) -> Clip {
    let mut clip = Clip::NONE;
    clip.add_path(Path::rounded_rect(rect, radius));
    clip
}

fn push_layer(scene: &mut Scene, alpha: f32, blend: BlendMode, filter: ImageFilter, clip: Clip) {
    push_layer_in(
        scene,
        Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
        alpha,
        blend,
        filter,
        clip,
    );
}

fn push_layer_in(
    scene: &mut Scene,
    bounds: Rect,
    alpha: f32,
    blend: BlendMode,
    filter: ImageFilter,
    clip: Clip,
) {
    scene.push_command(Command::PushLayer {
        bounds,
        alpha,
        blend,
        clip,
        filter,
    });
}

fn fill_rect(scene: &mut Scene, rect: Rect, color: Color, transform: Transform, clip: Clip) {
    scene.push_command(Command::FillRect {
        rect,
        paint: Paint::solid(color),
        transform,
        clip,
    });
}

fn photo(w: u32, h: u32, phase: u32) -> Image {
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let stripe = ((x + phase) / 6 + y / 6) % 2;
            pixels.extend_from_slice(&[
                (60 + x * 180 / w) as u8,
                (40 + y * 180 / h) as u8,
                if stripe == 0 { 200 } else { 90 },
                255,
            ]);
        }
    }
    Image::from_rgba8(pixels, w, h)
}

#[allow(clippy::too_many_lines)]
fn gpu_scene(t: f32) -> Scene {
    let mut scene = Scene::default();
    let wave = (t * std::f32::consts::TAU).sin();
    let pulse = 0.5 + 0.5 * wave;
    let drift = 36.0 * wave;

    // Card shadow, then the card: a diagonal gradient plate.
    scene.push_command(Command::DrawShadow {
        rect: Rect::new(36.0, 34.0, 604.0, 366.0),
        radius: 24.0,
        shadow: Shadow::new(Color::rgba(0, 0, 0, 120), Offset::new(0.0, 10.0), 18.0),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    let card = Rect::new(36.0, 34.0, 604.0, 366.0);
    scene.push_command(Command::FillRect {
        rect: card,
        paint: Paint::gradient(
            Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0))
                .between(Color::rgba(20, 28, 45, 255), Color::rgba(44, 58, 96, 255))
                .with_dither(),
        ),
        transform: Transform::IDENTITY,
        clip: rounded(card, 24.0),
    });

    // A photograph behind glass: a minified image, clipped to a rounded rect.
    let photo_rect = Rect::new(66.0, 64.0, 346.0, 250.0);
    scene.push_command(Command::DrawImage {
        rect: photo_rect,
        image: photo(560, 372, (t * 60.0) as u32),
        transform: Transform::IDENTITY,
        clip: rounded(photo_rect, 18.0),
    });
    // Frosted glass over the lower half of the photo: backdrop blur + tint.
    let glass = Rect::new(66.0, 170.0 + 10.0 * wave, 346.0, 250.0);
    push_layer_in(
        &mut scene,
        glass,
        1.0,
        BlendMode::Normal,
        ImageFilter::backdrop_blur(6.0),
        rounded(glass, 18.0),
    );
    fill_rect(
        &mut scene,
        glass,
        Color::rgba(255, 255, 255, 60),
        Transform::IDENTITY,
        Clip::NONE,
    );
    fill_rect(
        &mut scene,
        Rect::new(86.0, glass.top + 20.0, 206.0, glass.top + 34.0),
        Color::rgba(255, 255, 255, 230),
        Transform::IDENTITY,
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);

    // Moving tiles in a translucent group, inside a rectangular clip.
    let mut clip = Clip::NONE;
    clip.add_rect(Rect::new(366.0, 64.0, 574.0, 250.0));
    push_layer_in(
        &mut scene,
        Rect::new(366.0, 64.0, 574.0, 250.0),
        0.55 + 0.4 * pulse,
        BlendMode::Normal,
        ImageFilter::NONE,
        Clip::NONE,
    );
    for row in 0..3 {
        for col in 0..3 {
            let x = 376.0 + col as f32 * 66.0 + drift * (row as f32 * 0.2);
            let y = 74.0 + row as f32 * 60.0;
            let c = Color::rgba(
                (70.0 + 90.0 * pulse + col as f32 * 20.0) as u8,
                (130 + row * 30) as u8,
                230,
                255,
            );
            fill_rect(
                &mut scene,
                Rect::new(x, y, x + 58.0, y + 52.0),
                c,
                Transform::IDENTITY,
                clip.clone(),
            );
        }
    }
    scene.push_command(Command::PopLayer);

    // A radial "light" multiplied over the tiles.
    push_layer_in(
        &mut scene,
        Rect::new(366.0, 64.0, 574.0, 250.0),
        1.0,
        BlendMode::Multiply,
        ImageFilter::NONE,
        Clip::NONE,
    );
    scene.push_command(Command::FillRect {
        rect: Rect::new(366.0, 64.0, 574.0, 250.0),
        paint: Paint::gradient(
            Gradient::radial(Offset::new(0.3 + 0.4 * pulse, 0.4), 0.8).between(
                Color::rgba(255, 250, 230, 255),
                Color::rgba(90, 70, 140, 255),
            ),
        ),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    scene.push_command(Command::PopLayer);

    // An inset "well" with a sweep-gradient dial in it.
    let well = Rect::new(66.0, 276.0, 346.0, 340.0);
    fill_rect(
        &mut scene,
        well,
        Color::rgba(14, 18, 30, 255),
        Transform::IDENTITY,
        rounded(well, 16.0),
    );
    scene.push_command(Command::DrawShadow {
        rect: well,
        radius: 16.0,
        shadow: Shadow::inset(Color::rgba(0, 0, 0, 200), Offset::new(0.0, 4.0), 12.0),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    let dial = Rect::new(90.0, 284.0, 138.0, 332.0);
    let mut dial_clip = Clip::NONE;
    dial_clip.add_path(Path::rounded_rect(dial, 24.0));
    scene.push_command(Command::FillRect {
        rect: dial,
        paint: Paint::gradient(
            Gradient::sweep(Offset::new(0.5, 0.5), 0.0, 0.5 + 5.5 * pulse).between(
                Color::rgba(80, 220, 160, 255),
                Color::rgba(40, 120, 255, 255),
            ),
        ),
        transform: Transform::IDENTITY,
        clip: dial_clip,
    });

    // A blurred, desaturated glow behind a rotated thumbnail with its own shadow.
    push_layer_in(
        &mut scene,
        Rect::new(366.0, 260.0, 604.0, 366.0),
        0.8,
        BlendMode::Screen,
        ImageFilter::blur(5.0),
        Clip::NONE,
    );
    fill_rect(
        &mut scene,
        Rect::new(420.0 + drift, 290.0, 520.0 + drift, 330.0),
        Color::rgba(255, 170, 60, 255),
        Transform::IDENTITY,
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    let spin = Transform::rotate(0.25 * wave).then(Transform::translate(Offset::new(470.0, 312.0)));
    scene.push_command(Command::DrawShadow {
        rect: Rect::new(-40.0, -26.0, 40.0, 26.0),
        radius: 8.0,
        shadow: Shadow::new(Color::rgba(0, 0, 0, 150), Offset::new(0.0, 6.0), 10.0),
        transform: spin,
        clip: Clip::NONE,
    });
    push_layer_in(
        &mut scene,
        Rect::new(366.0, 260.0, 604.0, 366.0),
        1.0,
        BlendMode::Normal,
        ImageFilter::color(vieww_foundation::saturation_matrix(0.3 + 0.7 * pulse)),
        Clip::NONE,
    );
    scene.push_command(Command::DrawImage {
        rect: Rect::new(-40.0, -26.0, 40.0, 26.0),
        image: photo(40, 26, 0),
        transform: spin,
        clip: Clip::NONE,
    });
    scene.push_command(Command::PopLayer);

    scene
}

fn capability_probes() -> Vec<(&'static str, Scene)> {
    let r = Rect::new(20.0, 20.0, 180.0, 120.0);
    let one = |f: &dyn Fn(&mut Scene)| {
        let mut s = Scene::default();
        fill_rect(
            &mut s,
            Rect::new(0.0, 0.0, 320.0, 200.0),
            Color::rgba(200, 210, 220, 255),
            Transform::IDENTITY,
            Clip::NONE,
        );
        f(&mut s);
        s
    };
    let gradient = |g: Gradient| {
        one(&move |s: &mut Scene| {
            s.push_command(Command::FillRect {
                rect: r,
                paint: Paint::gradient(g.between(Color::RED, Color::BLUE)),
                transform: Transform::IDENTITY,
                clip: Clip::NONE,
            });
        })
    };
    let shadow = |shadow: Shadow, transform: Transform| {
        one(&move |s: &mut Scene| {
            s.push_command(Command::DrawShadow {
                rect: r,
                radius: 14.0,
                shadow,
                transform,
                clip: Clip::NONE,
            });
        })
    };
    let layer = |alpha: f32, blend: BlendMode, filter: ImageFilter, clip: Clip| {
        one(&move |s: &mut Scene| {
            push_layer_in(
                s,
                Rect::new(0.0, 0.0, 320.0, 200.0),
                alpha,
                blend,
                filter,
                clip.clone(),
            );
            fill_rect(
                s,
                r,
                Color::rgba(30, 140, 90, 255),
                Transform::IDENTITY,
                Clip::NONE,
            );
            s.push_command(Command::PopLayer);
        })
    };
    vec![
        ("gradient-linear", gradient(Gradient::horizontal())),
        (
            "gradient-radial",
            gradient(Gradient::radial(Offset::new(0.5, 0.5), 0.5)),
        ),
        (
            "gradient-sweep",
            gradient(Gradient::sweep(Offset::new(0.5, 0.5), 0.0, 6.0)),
        ),
        (
            "image-rotated-minified",
            one(&|s: &mut Scene| {
                s.push_command(Command::DrawImage {
                    rect: Rect::new(-40.0, -30.0, 40.0, 30.0),
                    image: photo(320, 240, 0),
                    transform: Transform::rotate(0.4)
                        .then(Transform::translate(Offset::new(160.0, 100.0))),
                    clip: Clip::NONE,
                });
            }),
        ),
        (
            "shadow-outer",
            shadow(
                Shadow::new(Color::BLACK, Offset::new(0.0, 4.0), 8.0),
                Transform::IDENTITY,
            ),
        ),
        (
            "shadow-inset",
            shadow(
                Shadow::inset(Color::BLACK, Offset::new(0.0, 4.0), 8.0),
                Transform::IDENTITY,
            ),
        ),
        (
            "shadow-rotated",
            shadow(
                Shadow::new(Color::BLACK, Offset::new(0.0, 4.0), 8.0),
                Transform::rotate(0.3),
            ),
        ),
        (
            "shaped-clip",
            one(&|s: &mut Scene| {
                fill_rect(s, r, Color::BLACK, Transform::IDENTITY, rounded(r, 30.0))
            }),
        ),
        (
            "layer-opacity",
            layer(0.5, BlendMode::Normal, ImageFilter::NONE, Clip::NONE),
        ),
        (
            "layer-blend-luminosity",
            layer(1.0, BlendMode::Luminosity, ImageFilter::NONE, Clip::NONE),
        ),
        (
            "layer-blur",
            layer(1.0, BlendMode::Normal, ImageFilter::blur(6.0), Clip::NONE),
        ),
        (
            "layer-color-matrix",
            layer(
                1.0,
                BlendMode::Normal,
                ImageFilter::color(vieww_foundation::sepia_matrix()),
                Clip::NONE,
            ),
        ),
        (
            "layer-backdrop-blur",
            layer(
                1.0,
                BlendMode::Normal,
                ImageFilter::backdrop_blur(4.0),
                rounded(r, 12.0),
            ),
        ),
    ]
}
