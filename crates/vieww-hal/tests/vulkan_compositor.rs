//! The GPU compositor against `NativeRenderer`, feature by feature.
//!
//! Every test here draws one scene through `vieww_gpu::Planner` +
//! `SceneRenderer` on a real Vulkan device and through the CPU rasterizer, and
//! compares the two **over every pixel**, not only interiors.
//!
//! # Why whole-frame comparison is possible here
//!
//! `vulkan_scene.rs` excludes antialiased edges because the GPU triangulates
//! fills without coverage AA while the CPU computes analytic coverage. These
//! scenes avoid that one source of disagreement on purpose: filled geometry is
//! pixel-aligned, so both rasterizers cover exactly the same pixels, and every
//! curved edge comes from a *mask* — a rounded clip, a shadow silhouette —
//! which the GPU takes from the CPU rasterizer through `GpuSeam`. What remains
//! to disagree about is exactly what these features added: offscreen targets,
//! group opacity, 28 blend modes, blur and colour-matrix passes, backdrop
//! sampling, mask multiplies, shadow blur/inset/tint, per-fragment gradients
//! and image sampling. A mistake in any of those moves whole regions of pixels
//! and fails; a one-step rounding difference does not.
//!
//! `#[ignore]`d like the other Vulkan suites: run with
//! `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`.

#![cfg(feature = "vulkan")]

use vieww_foundation::{
    BlendMode, Color, Gradient, Image, ImageFilter, Offset, Path, Rect, Shadow, Transform,
};
use vieww_gpu::{Planner, Step};
use vieww_hal::vulkan::{SceneRenderer, VulkanDevice};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Clip, Command, Paint, Scene};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 96;

/// See `vulkan_scene.rs`: lavapipe crashes with concurrent devices.
static ONE_DEVICE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Gpu {
    _lock: std::sync::MutexGuard<'static, ()>,
    renderer: SceneRenderer,
    _device: VulkanDevice,
}

fn gpu() -> Option<Gpu> {
    let lock = ONE_DEVICE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let device = match VulkanDevice::new() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("skipping: no Vulkan device available ({error})");
            return None;
        }
    };
    let renderer = SceneRenderer::new(&device).expect("the pipelines should build");
    Some(Gpu {
        _lock: lock,
        renderer,
        _device: device,
    })
}

/// Render on both, and assert the frames agree to within `tolerance` per
/// channel on every pixel except at most `allowed` of them.
fn assert_parity(name: &str, scene: &Scene, clear: Color, tolerance: u8, allowed: usize) {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let mut planner = Planner::new();
    let plan = planner.plan(scene, WIDTH as f32, HEIGHT as f32);
    assert!(
        plan.is_complete(),
        "{name}: plan has gaps {:?}",
        plan.unsupported
    );
    let gpu_pixels = gpu
        .renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .unwrap_or_else(|e| panic!("{name}: GPU render failed: {e}"));
    let (cpu, _) = NativeRenderer::new()
        .render_to_pixels(scene, WIDTH, HEIGHT, clear)
        .expect("CPU render");
    let cpu_pixels = cpu.data();

    let mut worst = (0u8, 0usize, 0usize, [0u8; 4], [0u8; 4]);
    let mut over = 0usize;
    for (i, (g, c)) in gpu_pixels
        .chunks_exact(4)
        .zip(cpu_pixels.chunks_exact(4))
        .enumerate()
    {
        let d = (0..4).map(|k| g[k].abs_diff(c[k])).max().unwrap_or(0);
        if d > tolerance {
            over += 1;
        }
        if d > worst.0 {
            let x = i % WIDTH as usize;
            let y = i / WIDTH as usize;
            worst = (d, x, y, [g[0], g[1], g[2], g[3]], [c[0], c[1], c[2], c[3]]);
        }
    }
    eprintln!(
        "{name}: max channel diff {} at ({}, {}) gpu={:?} cpu={:?}; {over} px over {tolerance}",
        worst.0, worst.1, worst.2, worst.3, worst.4
    );
    if std::env::var_os("VIEWW_PARITY_DUMP").is_some() {
        dump(name, &gpu_pixels, cpu_pixels);
    }
    assert!(
        over <= allowed,
        "{name}: {over} pixels differ by more than {tolerance} (allowed {allowed}); worst {} at ({}, {}) gpu={:?} cpu={:?}",
        worst.0,
        worst.1,
        worst.2,
        worst.3,
        worst.4
    );
}

/// Side-by-side PPM (GPU | CPU) for eyeballing a failure.
fn dump(name: &str, gpu: &[u8], cpu: &[u8]) {
    let mut out = format!("P6\n{} {}\n255\n", WIDTH * 2, HEIGHT).into_bytes();
    for y in 0..HEIGHT as usize {
        for buffer in [gpu, cpu] {
            for x in 0..WIDTH as usize {
                let i = (y * WIDTH as usize + x) * 4;
                out.extend_from_slice(&buffer[i..i + 3]);
            }
        }
    }
    let _ = std::fs::write(std::env::temp_dir().join(format!("parity-{name}.ppm")), out);
}

fn fill(scene: &mut Scene, rect: Rect, color: Color, clip: Clip) {
    scene.push_command(Command::FillRect {
        rect,
        paint: Paint::solid(color),
        transform: Transform::IDENTITY,
        clip,
    });
}

fn backdrop(scene: &mut Scene) {
    // A pixel-aligned checker of saturated colours: something for blend modes,
    // blurs and backdrop filters to visibly act on.
    for row in 0..6 {
        for col in 0..8 {
            let color = match (row + col) % 4 {
                0 => Color::rgba(230, 60, 40, 255),
                1 => Color::rgba(40, 170, 90, 255),
                2 => Color::rgba(50, 90, 220, 255),
                _ => Color::rgba(240, 220, 60, 255),
            };
            let x = col as f32 * 16.0;
            let y = row as f32 * 16.0;
            fill(
                scene,
                Rect::new(x, y, x + 16.0, y + 16.0),
                color,
                Clip::NONE,
            );
        }
    }
}

fn push_layer(
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

fn rounded_clip(rect: Rect, radius: f32) -> Clip {
    let mut clip = Clip::NONE;
    clip.add_path(Path::rounded_rect(rect, radius));
    clip
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn group_opacity_composites_the_group_not_each_child() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(8.0, 8.0, 120.0, 88.0),
        0.5,
        BlendMode::Normal,
        ImageFilter::NONE,
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(16.0, 16.0, 72.0, 64.0),
        Color::rgba(255, 255, 255, 255),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(48.0, 32.0, 112.0, 80.0),
        Color::rgba(0, 0, 0, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    assert_parity("group-opacity", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn all_twenty_eight_blend_modes_match() {
    use BlendMode as B;
    let modes = [
        B::Normal,
        B::Clear,
        B::Src,
        B::Dst,
        B::DstOver,
        B::SrcIn,
        B::DstIn,
        B::SrcOut,
        B::DstOut,
        B::SrcAtop,
        B::DstAtop,
        B::Xor,
        B::Plus,
        B::Multiply,
        B::Screen,
        B::Overlay,
        B::Darken,
        B::Lighten,
        B::ColorDodge,
        B::ColorBurn,
        B::HardLight,
        B::SoftLight,
        B::Difference,
        B::Exclusion,
        B::Hue,
        B::Saturation,
        B::Color,
        B::Luminosity,
    ];
    for mode in modes {
        let mut scene = Scene::default();
        backdrop(&mut scene);
        // A translucent backdrop region too, so modes that use dst alpha have
        // something other than 1.0 to read.
        fill(
            &mut scene,
            Rect::new(0.0, 64.0, 128.0, 96.0),
            Color::rgba(90, 20, 160, 0),
            Clip::NONE,
        );
        push_layer(
            &mut scene,
            Rect::new(12.0, 12.0, 116.0, 84.0),
            0.85,
            mode,
            ImageFilter::NONE,
            Clip::NONE,
        );
        fill(
            &mut scene,
            Rect::new(20.0, 20.0, 76.0, 60.0),
            Color::rgba(250, 140, 30, 255),
            Clip::NONE,
        );
        fill(
            &mut scene,
            Rect::new(52.0, 36.0, 108.0, 76.0),
            Color::rgba(30, 200, 230, 150),
            Clip::NONE,
        );
        scene.push_command(Command::PopLayer);
        assert_parity(
            &format!("blend-{mode:?}"),
            &scene,
            Color::rgba(128, 128, 128, 200),
            2,
            0,
        );
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_blurred_layer_matches_the_three_box_blur() {
    let mut scene = Scene::default();
    fill(
        &mut scene,
        Rect::new(0.0, 0.0, 128.0, 96.0),
        Color::rgba(245, 245, 250, 255),
        Clip::NONE,
    );
    push_layer(
        &mut scene,
        Rect::new(0.0, 0.0, 128.0, 96.0),
        1.0,
        BlendMode::Normal,
        ImageFilter::blur(4.0),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(32.0, 24.0, 96.0, 72.0),
        Color::rgba(20, 60, 200, 255),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(40.0, 40.0, 56.0, 56.0),
        Color::rgba(250, 250, 0, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    assert_parity("layer-blur", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_colour_matrix_layer_matches() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(8.0, 8.0, 120.0, 88.0),
        0.9,
        BlendMode::Normal,
        ImageFilter::color(vieww_foundation::grayscale_matrix()),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(16.0, 16.0, 112.0, 80.0),
        Color::rgba(220, 40, 120, 200),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    assert_parity("color-matrix", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_backdrop_blur_blurs_what_is_behind_and_keeps_the_foreground_sharp() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(16.0, 16.0, 112.0, 80.0),
        1.0,
        BlendMode::Normal,
        ImageFilter::backdrop_blur(3.0),
        rounded_clip(Rect::new(16.0, 16.0, 112.0, 80.0), 14.0),
    );
    // A glass tint and a sharp foreground chip on top of the blurred backdrop.
    fill(
        &mut scene,
        Rect::new(16.0, 16.0, 112.0, 80.0),
        Color::rgba(255, 255, 255, 90),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(40.0, 40.0, 88.0, 56.0),
        Color::rgba(10, 10, 10, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    assert_parity("backdrop-blur", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn shaped_clips_on_draws_and_on_layers_match() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    // A rounded clip on a plain fill (per-draw mask).
    fill(
        &mut scene,
        Rect::new(8.0, 8.0, 60.0, 60.0),
        Color::rgba(20, 20, 20, 230),
        rounded_clip(Rect::new(8.0, 8.0, 60.0, 60.0), 20.0),
    );
    // Two nested shaped clips: a rounded rect intersected with a circle.
    let mut nested = rounded_clip(Rect::new(64.0, 8.0, 124.0, 68.0), 8.0);
    let mut circle = Path::new();
    circle.move_to(Offset::new(124.0, 38.0));
    for step in 1..=48 {
        let angle = step as f32 / 48.0 * std::f32::consts::TAU;
        circle.line_to(Offset::new(
            94.0 + 30.0 * angle.cos(),
            38.0 + 30.0 * angle.sin(),
        ));
    }
    circle.close();
    nested.add_path(circle);
    fill(
        &mut scene,
        Rect::new(64.0, 8.0, 124.0, 68.0),
        Color::rgba(250, 250, 250, 255),
        nested,
    );
    // A rounded clip on a translucent layer (mask at pop).
    push_layer(
        &mut scene,
        Rect::new(20.0, 60.0, 108.0, 92.0),
        0.75,
        BlendMode::Normal,
        ImageFilter::NONE,
        rounded_clip(Rect::new(20.0, 60.0, 108.0, 92.0), 16.0),
    );
    fill(
        &mut scene,
        Rect::new(0.0, 56.0, 128.0, 96.0),
        Color::rgba(0, 120, 255, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    assert_parity("shaped-clips", &scene, Color::WHITE, 2, 0);
}

fn shadow_scene(shadow: Shadow, transform: Transform, clip: Clip) -> Scene {
    let mut scene = Scene::default();
    fill(
        &mut scene,
        Rect::new(0.0, 0.0, 128.0, 96.0),
        Color::rgba(236, 238, 242, 255),
        Clip::NONE,
    );
    scene.push_command(Command::DrawShadow {
        rect: Rect::new(36.0, 26.0, 92.0, 70.0),
        radius: 10.0,
        shadow,
        transform,
        clip,
    });
    scene
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn every_shadow_variant_matches() {
    let dark = Color::rgba(10, 20, 40, 140);
    let rotate = Transform::rotate_around(Offset::new(64.0, 48.0), 0.35);
    let scale = Transform::scale_around(Offset::new(64.0, 48.0), 1.2, 0.8);
    let cases = [
        (
            "shadow-outer",
            Shadow::new(dark, Offset::new(0.0, 6.0), 16.0),
            Transform::IDENTITY,
            Clip::NONE,
        ),
        (
            "shadow-outer-spread",
            Shadow::new(dark, Offset::new(3.0, 3.0), 8.0).spread(4.0),
            Transform::IDENTITY,
            Clip::NONE,
        ),
        (
            "shadow-inset",
            Shadow::inset(dark, Offset::new(0.0, 4.0), 12.0),
            Transform::IDENTITY,
            Clip::NONE,
        ),
        (
            "shadow-rotated",
            Shadow::new(dark, Offset::new(0.0, 6.0), 14.0),
            rotate,
            Clip::NONE,
        ),
        (
            "shadow-inset-rotated",
            Shadow::inset(dark, Offset::new(2.0, 2.0), 10.0),
            rotate,
            Clip::NONE,
        ),
        (
            "shadow-scaled",
            Shadow::new(dark, Offset::new(0.0, 4.0), 12.0),
            scale,
            Clip::NONE,
        ),
        (
            "shadow-shaped-clip",
            Shadow::new(dark, Offset::new(0.0, 8.0), 20.0),
            Transform::IDENTITY,
            rounded_clip(Rect::new(20.0, 20.0, 108.0, 90.0), 24.0),
        ),
    ];
    for (name, shadow, transform, clip) in cases {
        assert_parity(
            name,
            &shadow_scene(shadow, transform, clip),
            Color::WHITE,
            2,
            0,
        );
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_shadow_inside_a_translucent_layer_matches() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(4.0, 4.0, 124.0, 92.0),
        0.8,
        BlendMode::Normal,
        ImageFilter::NONE,
        Clip::NONE,
    );
    scene.push_command(Command::DrawShadow {
        rect: Rect::new(30.0, 24.0, 98.0, 72.0),
        radius: 12.0,
        shadow: Shadow::new(Color::rgba(0, 0, 0, 180), Offset::new(0.0, 6.0), 18.0),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    fill(
        &mut scene,
        Rect::new(30.0, 24.0, 98.0, 72.0),
        Color::rgba(255, 255, 255, 255),
        rounded_clip(Rect::new(30.0, 24.0, 98.0, 72.0), 12.0),
    );
    scene.push_command(Command::PopLayer);
    assert_parity("shadow-in-layer", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn linear_radial_and_sweep_gradients_match_per_pixel() {
    let stops = [
        (0.0, Color::rgba(255, 40, 40, 255)),
        (0.35, Color::rgba(250, 220, 40, 200)),
        (0.7, Color::rgba(40, 200, 120, 255)),
        (1.0, Color::rgba(40, 80, 240, 120)),
    ];
    let gradients = [
        ("gradient-linear", Gradient::horizontal()),
        (
            "gradient-diagonal",
            Gradient::linear(Offset::new(0.1, 0.0), Offset::new(0.9, 1.0)),
        ),
        (
            "gradient-radial",
            Gradient::radial(Offset::new(0.4, 0.5), 0.6),
        ),
        (
            "gradient-sweep",
            Gradient::sweep(Offset::new(0.5, 0.5), 0.3, 5.5),
        ),
    ];
    for (name, gradient) in gradients {
        for dither in [false, true] {
            let gradient = gradient.with_stops(&stops);
            let gradient = if dither {
                gradient.with_dither()
            } else {
                gradient
            };
            let mut scene = Scene::default();
            scene.push_command(Command::FillRect {
                rect: Rect::new(8.0, 8.0, 120.0, 88.0),
                paint: Paint::gradient(gradient),
                transform: Transform::IDENTITY,
                clip: Clip::NONE,
            });
            // The same gradient under a rotation, clipped to a rounded shape.
            scene.push_command(Command::FillRect {
                rect: Rect::new(-20.0, -14.0, 20.0, 14.0),
                paint: Paint::gradient(gradient),
                transform: Transform::rotate(0.6)
                    .then(Transform::translate(Offset::new(64.0, 48.0))),
                clip: rounded_clip(Rect::new(40.0, 28.0, 88.0, 68.0), 12.0),
            });
            // Rotated geometry has non-aligned edges; allow those pixels.
            assert_parity(
                &format!("{name}-dither-{dither}"),
                &scene,
                Color::WHITE,
                2,
                140,
            );
        }
    }
}

fn test_image(w: u32, h: u32) -> Image {
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&[
                (x * 255 / w.max(1)) as u8,
                (y * 255 / h.max(1)) as u8,
                (((x / 3) + (y / 3)) % 2 * 200) as u8,
                if (x + y) % 7 == 0 { 90 } else { 255 },
            ]);
        }
    }
    Image::from_rgba8(pixels, w, h)
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn images_magnified_rotated_and_minified_match() {
    let cases = [
        (
            "image-magnified",
            test_image(12, 9),
            Rect::new(10.0, 10.0, 118.0, 86.0),
            Transform::IDENTITY,
            0,
        ),
        (
            "image-rotated",
            test_image(20, 20),
            Rect::new(-30.0, -30.0, 30.0, 30.0),
            Transform::rotate(0.5).then(Transform::translate(Offset::new(64.0, 48.0))),
            // A rotated quad's edge pixels are decided by two rasterization
            // rules; its interior is not.
            120,
        ),
        (
            "image-minified",
            test_image(256, 192),
            Rect::new(16.0, 12.0, 112.0, 84.0),
            Transform::IDENTITY,
            0,
        ),
        (
            "image-minified-far",
            test_image(300, 300),
            Rect::new(40.0, 20.0, 70.0, 50.0),
            Transform::IDENTITY,
            0,
        ),
    ];
    for (name, image, rect, transform, allowed) in cases {
        let mut scene = Scene::default();
        fill(
            &mut scene,
            Rect::new(0.0, 0.0, 128.0, 96.0),
            Color::rgba(30, 30, 30, 255),
            Clip::NONE,
        );
        scene.push_command(Command::DrawImage {
            rect,
            image,
            transform,
            clip: Clip::NONE,
        });
        assert_parity(name, &scene, Color::WHITE, 2, allowed);
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn nested_layers_with_mixed_effects_match() {
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(0.0, 0.0, 128.0, 96.0),
        0.9,
        BlendMode::Multiply,
        ImageFilter::NONE,
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(8.0, 8.0, 120.0, 88.0),
        Color::rgba(250, 230, 200, 255),
        Clip::NONE,
    );
    push_layer(
        &mut scene,
        Rect::new(16.0, 16.0, 112.0, 80.0),
        0.7,
        BlendMode::Screen,
        ImageFilter::blur(2.0),
        rounded_clip(Rect::new(16.0, 16.0, 112.0, 80.0), 10.0),
    );
    fill(
        &mut scene,
        Rect::new(24.0, 24.0, 64.0, 72.0),
        Color::rgba(0, 90, 200, 255),
        Clip::NONE,
    );
    push_layer(
        &mut scene,
        Rect::new(48.0, 32.0, 104.0, 72.0),
        1.0,
        BlendMode::Difference,
        ImageFilter::NONE,
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(48.0, 32.0, 104.0, 72.0),
        Color::rgba(255, 255, 255, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    scene.push_command(Command::PopLayer);
    scene.push_command(Command::PopLayer);
    assert_parity("nested-layers", &scene, Color::WHITE, 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_transparent_clear_reads_back_straight_alpha_like_the_cpu() {
    let mut scene = Scene::default();
    fill(
        &mut scene,
        Rect::new(10.0, 10.0, 60.0, 60.0),
        Color::rgba(200, 100, 50, 128),
        Clip::NONE,
    );
    fill(
        &mut scene,
        Rect::new(30.0, 30.0, 90.0, 80.0),
        Color::rgba(20, 140, 220, 77),
        Clip::NONE,
    );
    assert_parity("transparent-clear", &scene, Color::rgba(0, 0, 0, 0), 2, 0);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_steady_frame_uploads_nothing_new_and_reuses_every_target() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let mut scene = Scene::default();
    backdrop(&mut scene);
    push_layer(
        &mut scene,
        Rect::new(8.0, 8.0, 120.0, 88.0),
        0.6,
        BlendMode::Overlay,
        ImageFilter::blur(3.0),
        rounded_clip(Rect::new(8.0, 8.0, 120.0, 88.0), 12.0),
    );
    fill(
        &mut scene,
        Rect::new(20.0, 20.0, 100.0, 70.0),
        Color::rgba(255, 255, 255, 255),
        Clip::NONE,
    );
    scene.push_command(Command::PopLayer);
    let mut planner = Planner::new();
    let first_plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    let first = gpu
        .renderer
        .render_planned(&planner, &first_plan, WIDTH, HEIGHT, Color::WHITE)
        .unwrap();
    let versions = (
        planner.mask_atlas().version(),
        planner.ramp_atlas().version(),
        planner.atlas().version(),
    );
    let second_plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    let second = gpu
        .renderer
        .render_planned(&planner, &second_plan, WIDTH, HEIGHT, Color::WHITE)
        .unwrap();
    assert_eq!(first_plan, second_plan, "the same scene plans identically");
    assert_eq!(first, second, "and renders identically");
    assert_eq!(
        versions,
        (
            planner.mask_atlas().version(),
            planner.ramp_atlas().version(),
            planner.atlas().version()
        ),
        "a steady frame changes no atlas"
    );
    assert!(first_plan
        .steps
        .iter()
        .any(|s| matches!(s, Step::PopLayer { .. })));
}

/// One renderer, two planners: the second planner's atlases must be uploaded
/// even though, counted per atlas, they have been changed the same number of
/// times as the first's. The fixture census found this — gradients in the
/// previous screen's colours.
#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_renderer_shared_between_planners_never_draws_a_stale_atlas() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    let scene_with = |from: Color, to: Color, radius: f32| {
        let mut scene = Scene::default();
        scene.push_command(Command::FillRect {
            rect: Rect::new(8.0, 8.0, 120.0, 88.0),
            paint: Paint::gradient(Gradient::radial(Offset::new(0.5, 0.5), 0.5).between(from, to)),
            transform: Transform::IDENTITY,
            clip: rounded_clip(Rect::new(8.0, 8.0, 120.0, 88.0), radius),
        });
        scene
    };
    for (i, scene) in [
        scene_with(Color::RED, Color::BLUE, 10.0),
        scene_with(
            Color::rgba(255, 210, 90, 255),
            Color::rgba(120, 60, 220, 255),
            40.0,
        ),
    ]
    .iter()
    .enumerate()
    {
        let mut planner = Planner::new();
        let plan = planner.plan(scene, WIDTH as f32, HEIGHT as f32);
        let pixels = gpu
            .renderer
            .render_planned(&planner, &plan, WIDTH, HEIGHT, Color::WHITE)
            .unwrap();
        let (cpu, _) = NativeRenderer::new()
            .render_to_pixels(scene, WIDTH, HEIGHT, Color::WHITE)
            .unwrap();
        let over = pixels
            .chunks_exact(4)
            .zip(cpu.data().chunks_exact(4))
            .filter(|(g, c)| (0..4).any(|k| g[k].abs_diff(c[k]) > 2))
            .count();
        assert_eq!(
            over, 0,
            "planner {i}: {over} pixels drawn against a stale atlas"
        );
    }
}

/// Overlapping subpaths fill under **non-zero** winding, as the display list
/// specifies. The tessellator defaulted to even-odd, which punched holes in
/// icons built from overlapping pieces (the census's `10-icon-grid`).
#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn overlapping_subpaths_fill_under_non_zero_winding() {
    let mut path = Path::new();
    for rect in [
        Rect::new(16.0, 16.0, 80.0, 64.0),
        Rect::new(48.0, 32.0, 112.0, 80.0),
    ] {
        path.move_to(Offset::new(rect.left, rect.top));
        path.line_to(Offset::new(rect.right, rect.top));
        path.line_to(Offset::new(rect.right, rect.bottom));
        path.line_to(Offset::new(rect.left, rect.bottom));
        path.close();
    }
    let mut scene = Scene::default();
    scene.push_command(Command::FillPath {
        path,
        paint: Paint::solid(Color::rgba(240, 150, 60, 255)),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    assert_parity("non-zero-winding", &scene, Color::WHITE, 2, 0);
}

/// Strokes are expanded in local space and then transformed, and dashes are
/// applied — both as the CPU stroker does. Pixel-aligned so the comparison is
/// whole-frame: a 2 px line at 2x scale is 4 device pixels, dashes 8 on / 4 off.
#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn a_dashed_stroke_under_a_scale_matches_width_and_pattern() {
    let mut line = Path::new();
    line.move_to(Offset::new(4.0, 10.0));
    line.line_to(Offset::new(60.0, 10.0));
    let mut scene = Scene::default();
    scene.push_command(Command::StrokePath {
        path: line.clone(),
        stroke: vieww_paint::Stroke::new(2.0).dash(vieww_foundation::Dash::new(vec![4.0, 2.0])),
        paint: Paint::solid(Color::rgba(20, 40, 200, 255)),
        transform: Transform::scale(2.0, 2.0),
        clip: Clip::NONE,
    });
    // And an undashed one at 1x, for the width alone.
    scene.push_command(Command::StrokePath {
        path: line,
        stroke: vieww_paint::Stroke::new(4.0),
        paint: Paint::solid(Color::rgba(200, 40, 20, 255)),
        transform: Transform::translate(Offset::new(0.0, 40.0)),
        clip: Clip::NONE,
    });
    assert_parity("dashed-scaled-stroke", &scene, Color::WHITE, 2, 0);
}

/// A sharp miter join bevels at the same limit on both renderers. lyon's
/// limit is expressed differently from the CPU stroker's (and SVG's); passed
/// through unchanged, the GPU's spike was several pixels taller.
#[test]
#[ignore = "needs a Vulkan loader + ICD; run with `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`"]
fn sharp_miter_joins_reach_as_far_as_the_cpu_allows() {
    let Some(mut gpu) = gpu() else {
        return;
    };
    // Half-width 2. The SVG miter ratio is 1/sin(half the apex angle):
    //   within-both:     ~1.2  — mitred on both renderers
    //   between-limits:  ~5.7  — bevelled by the CPU (limit 4); an unscaled
    //                            lyon limit mitred it, ~9 px taller
    //   beyond-both:     ~11.5 — bevelled on both
    for (name, half_span, rise) in [
        ("within-both", 24.0, 14.0),
        ("between-limits", 8.0, 45.0),
        ("beyond-both", 4.0, 46.0),
    ] {
        let mut path = Path::new();
        path.move_to(Offset::new(64.0 - half_span, 80.0));
        path.line_to(Offset::new(64.0, 80.0 - rise));
        path.line_to(Offset::new(64.0 + half_span, 80.0));
        let mut scene = Scene::default();
        scene.push_command(Command::StrokePath {
            path,
            stroke: vieww_paint::Stroke::new(4.0),
            paint: Paint::solid(Color::BLACK),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        let mut planner = Planner::new();
        let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
        let pixels = gpu
            .renderer
            .render_planned(&planner, &plan, WIDTH, HEIGHT, Color::WHITE)
            .unwrap();
        let (cpu, _) = NativeRenderer::new()
            .render_to_pixels(&scene, WIDTH, HEIGHT, Color::WHITE)
            .unwrap();
        let top = |buffer: &[u8]| {
            (0..HEIGHT as usize)
                .find(|y| (0..WIDTH as usize).any(|x| buffer[(y * WIDTH as usize + x) * 4] < 128))
                .unwrap_or(HEIGHT as usize)
        };
        let (g, c) = (top(&pixels), top(cpu.data()));
        assert!(
            g.abs_diff(c) <= 1,
            "{name}: the join's tip is at row {g} on the GPU and row {c} on the CPU"
        );
    }
}
