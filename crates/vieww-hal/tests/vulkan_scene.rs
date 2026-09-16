//! `SceneRenderer` against a real Vulkan device, and against the CPU
//! rasterizer that is this workspace's reference for what a scene looks like.
//!
//! # Why the comparison is with `NativeRenderer` and not with a stored image
//!
//! A golden PNG says "this is what it looked like the day somebody approved
//! it". The question worth answering about a *second* renderer is different:
//! does it agree with the first one? `vieww_paint::NativeRenderer` is the
//! renderer this framework ships, has the larger test suite behind it, and is
//! what every fixture in `fixtures-out/` was drawn with — so it is the oracle,
//! and the GPU path's job is to match it.
//!
//! # Why the tolerance is not zero
//!
//! Two rasterizers agreeing pixel-for-pixel is not a reasonable requirement
//! and asking for it would mean pinning both to one implementation forever.
//! The CPU path computes analytic coverage per scanline; the GPU path
//! triangulates with `lyon` and lets the hardware sample. They disagree by
//! design at exactly one place — the fractional pixels along a shape's edge —
//! and nowhere else.
//!
//! So the assertion is shaped to say that: **interiors must match closely**,
//! and the disagreement must stay on the boundary. A regression that
//! misplaces a shape, drops it, or paints it the wrong colour moves interior
//! pixels and fails, while a change to either rasterizer's antialiasing does
//! not — which is the distinction a parity test is for.
//!
//! `#[ignore]`d for the same reason the other Vulkan suites here are: this
//! needs a loader and an ICD (`lavapipe`, or real hardware). Run with
//! `cargo test -p vieww-hal --features vulkan -- --ignored`.

#![cfg(feature = "vulkan")]

use vieww_foundation::{Color, Offset, Path, Rect, Transform};
use vieww_gpu::{plan_scene, Unsupported};
use vieww_hal::vulkan::{SceneRenderer, VulkanDevice};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Clip, Command, Paint, Scene, Stroke};

const WIDTH: u32 = 120;
const HEIGHT: u32 = 90;

/// One Vulkan device at a time, for the whole file.
///
/// # This is a real observation, not defensive habit
///
/// These tests each build their own `VulkanDevice`, and `libtest` runs them on
/// as many threads as the machine has. Once this suite began creating images
/// and samplers — the glyph atlas — running it multi-threaded started to
/// **SIGSEGV inside `libvulkan_lvp.so`**, reproducibly, on about half of runs.
/// Every frame in the backtrace at the fault is lavapipe's own; the deepest
/// frame belonging to this workspace is an ordinary `wait_for_fences`, and the
/// same eight tests pass every time with `--test-threads=1`.
///
/// So: concurrent devices on Mesa's software rasterizer, not a data race in
/// anything here — nothing in this crate shares a Vulkan object between
/// threads, and there is no `unsafe` in the tests at all. The serialisation is
/// therefore in the *tests*, where the constraint actually is, rather than
/// worked around with synchronisation inside `SceneRenderer` that would slow
/// down a real driver to accommodate a software one.
///
/// It is written here rather than in `TRACKER.md` alone because the next
/// person to add a test to this file will add it without the guard, watch CI
/// crash in a C library, and have no way to know why.
static ONE_DEVICE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The guard is returned, not dropped here: it has to be held for as long as
/// the device lives, and a `let _ = ...` at the top of a function would release
/// it immediately.
type Exclusive = std::sync::MutexGuard<'static, ()>;

fn try_renderer() -> Option<(Exclusive, VulkanDevice, SceneRenderer)> {
    // A test that fails while holding the lock poisons it. Recovering rather
    // than propagating is deliberate: the useful failure is the assertion that
    // actually failed, and letting every subsequent test fail with
    // `PoisonError` instead buries it.
    let guard = ONE_DEVICE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let device = match VulkanDevice::new() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("skipping: no Vulkan device available ({error})");
            return None;
        }
    };
    let renderer = SceneRenderer::new(&device).expect("the pipeline should build");
    // The renderer holds a cloned device handle and the device must outlive
    // it, so both are returned and dropped together by the caller.
    Some((guard, device, renderer))
}

fn fill_rect(scene: &mut Scene, rect: Rect, color: Color) {
    scene.push_command(Command::FillRect {
        rect,
        paint: Paint::solid(color),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
}

/// The CPU rasterizer's answer for the same scene.
fn on_cpu(scene: &Scene, base: Color) -> Vec<u8> {
    let mut renderer = NativeRenderer::new();
    let (pixels, _) = renderer
        .render_to_pixels(scene, WIDTH, HEIGHT, base)
        .expect("the CPU rasterizer should render this scene");
    pixels.data().to_vec()
}

/// Count pixels where `gpu` and `cpu` disagree, excluding pixels that sit on
/// a shape's antialiased edge.
///
/// # The mistake this function was written wrong once to make
///
/// The first version decided "is this pixel on an edge?" by asking whether
/// any *neighbouring pixel also disagreed between the two images*. That is
/// circular, and it fails in exactly the case a parity test exists to catch:
/// when the GPU path is broadly wrong — a swapped colour channel, say — every
/// pixel of every shape disagrees, so every pixel has a disagreeing
/// neighbour, so every pixel is classified as an edge, and the function
/// returns zero. It was confirmed by making that mutation deliberately: three
/// parity tests passed with red and blue swapped in the planner.
///
/// An edge is a property of the *picture*, not of the diff. So the question
/// asked here is whether the **reference** image has a coverage transition at
/// this pixel — a neighbour in `cpu` that differs from `cpu`'s own value —
/// which is true along shape boundaries and false in flat interiors no matter
/// what the GPU produced.
fn interior_mismatches(gpu: &[u8], cpu: &[u8], tolerance: u8) -> usize {
    assert_eq!(gpu.len(), cpu.len(), "same dimensions");
    let w = WIDTH as usize;
    let h = HEIGHT as usize;
    let channel = |buffer: &[u8], index: usize, c: usize| buffer[index * 4 + c];
    let differs = |index: usize| {
        (0..4).any(|c| channel(gpu, index, c).abs_diff(channel(cpu, index, c)) > tolerance)
    };
    // A transition in the reference itself: this is where antialiasing lives,
    // and the one place two rasterizers are allowed to disagree.
    let reference_edge = |x: usize, y: usize| {
        let index = y * w + x;
        (-1i64..=1).any(|dy| {
            (-1i64..=1).any(|dx| {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    return false;
                }
                let neighbour = ny as usize * w + nx as usize;
                (0..4).any(|c| channel(cpu, index, c).abs_diff(channel(cpu, neighbour, c)) > 8)
            })
        })
    };

    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            if differs(y * w + x) && !reference_edge(x, y) {
                count += 1;
            }
        }
    }
    count
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn an_empty_scene_is_the_clear_colour_exactly() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(20, 30, 40, 255);
    let plan = plan_scene(&Scene::default(), WIDTH as f32, HEIGHT as f32);
    assert!(plan.is_complete(), "an empty scene has nothing unsupported");

    let pixels = renderer
        .render(&plan, WIDTH, HEIGHT, clear)
        .expect("render should succeed");

    // Exactly, not approximately: nothing was drawn, so no rasterization
    // happened and there is no coverage to disagree about.
    for pixel in pixels.chunks_exact(4) {
        assert_eq!(pixel, [20, 30, 40, 255], "every pixel is the clear colour");
    }
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn one_rectangle_lands_where_the_geometry_says() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let mut scene = Scene::default();
    fill_rect(
        &mut scene,
        Rect::new(20.0, 20.0, 80.0, 60.0),
        Color::rgba(255, 0, 0, 255),
    );
    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    let pixels = renderer
        .render(&plan, WIDTH, HEIGHT, Color::rgba(0, 0, 0, 255))
        .expect("render should succeed");

    let at = |x: usize, y: usize| {
        let i = (y * WIDTH as usize + x) * 4;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };

    assert_eq!(at(50, 40), [255, 0, 0, 255], "inside the rectangle");
    assert_eq!(at(5, 5), [0, 0, 0, 255], "outside it, top-left");
    assert_eq!(at(100, 80), [0, 0, 0, 255], "outside it, bottom-right");
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn overlapping_translucent_fills_composite_like_the_cpu_rasterizer() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let base = Color::rgba(255, 255, 255, 255);
    let mut scene = Scene::default();
    fill_rect(
        &mut scene,
        Rect::new(10.0, 10.0, 70.0, 60.0),
        Color::rgba(200, 40, 40, 255),
    );
    // Half-transparent, and overlapping the first: this is the case where a
    // wrong blend factor produces a picture that still looks plausible.
    fill_rect(
        &mut scene,
        Rect::new(40.0, 30.0, 110.0, 80.0),
        Color::rgba(40, 80, 220, 128),
    );

    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    assert!(plan.is_complete());
    assert_eq!(
        plan.draw_call_count(),
        1,
        "two unclipped fills share a scissor, so they batch into one draw"
    );

    let gpu = renderer
        .render(&plan, WIDTH, HEIGHT, base)
        .expect("render should succeed");
    let cpu = on_cpu(&scene, base);

    let mismatches = interior_mismatches(&gpu, &cpu, 2);
    assert_eq!(
        mismatches, 0,
        "GPU and CPU must agree away from shape edges; {mismatches} interior pixels differed"
    );
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn a_transformed_path_and_a_stroke_match_the_cpu_rasterizer() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let base = Color::rgba(12, 12, 16, 255);
    let mut scene = Scene::default();

    let mut triangle = Path::new();
    triangle.move_to(Offset::new(10.0, 10.0));
    triangle.line_to(Offset::new(60.0, 15.0));
    triangle.line_to(Offset::new(30.0, 55.0));
    triangle.close();
    scene.push_command(Command::FillPath {
        path: triangle.clone(),
        paint: Paint::solid(Color::rgba(90, 200, 120, 255)),
        // A translation, so the planner's transform handling is exercised
        // rather than skipped by the identity fast path.
        transform: Transform::translate(Offset::new(24.0, 12.0)),
        clip: Clip::NONE,
    });

    let mut line = Path::new();
    line.move_to(Offset::new(8.0, 78.0));
    line.line_to(Offset::new(112.0, 66.0));
    scene.push_command(Command::StrokePath {
        path: line,
        stroke: Stroke::new(6.0),
        paint: Paint::solid(Color::rgba(240, 190, 60, 255)),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });

    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    assert!(plan.is_complete(), "{:?}", plan.unsupported);
    assert!(plan.triangle_count() >= 3);

    let gpu = renderer
        .render(&plan, WIDTH, HEIGHT, base)
        .expect("render should succeed");
    let cpu = on_cpu(&scene, base);

    let mismatches = interior_mismatches(&gpu, &cpu, 2);
    assert_eq!(
        mismatches, 0,
        "a transformed fill and a stroke must land where the CPU puts them; \
         {mismatches} interior pixels differed"
    );
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn a_rectangular_clip_becomes_a_scissor_and_cuts_the_same_way() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let base = Color::rgba(255, 255, 255, 255);
    let mut clip = Clip::NONE;
    clip.add_rect(Rect::new(30.0, 30.0, 70.0, 70.0));

    let mut scene = Scene::default();
    scene.push_command(Command::FillRect {
        rect: Rect::new(0.0, 0.0, 120.0, 90.0),
        paint: Paint::solid(Color::rgba(30, 120, 200, 255)),
        transform: Transform::IDENTITY,
        clip,
    });

    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    assert!(plan.is_complete(), "a rectangular clip is expressible");
    assert_eq!(plan.runs.len(), 1);
    assert!(
        plan.runs[0].scissor.is_some(),
        "the clip became a scissor rather than being dropped"
    );

    let gpu = renderer
        .render(&plan, WIDTH, HEIGHT, base)
        .expect("render should succeed");
    let cpu = on_cpu(&scene, base);
    assert_eq!(
        interior_mismatches(&gpu, &cpu, 2),
        0,
        "the scissor must cut where the CPU clip cuts"
    );
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn the_renderer_refuses_a_plan_it_cannot_draw() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    // A shaped clip: expressible by the CPU rasterizer, not by a scissor.
    let mut clip = Clip::NONE;
    clip.add_rect(Rect::new(10.0, 10.0, 90.0, 70.0));
    let mut rounded = Path::new();
    rounded.move_to(Offset::new(20.0, 20.0));
    rounded.cubic_to(
        Offset::new(60.0, 10.0),
        Offset::new(80.0, 40.0),
        Offset::new(40.0, 60.0),
    );
    rounded.close();
    clip.add_path(rounded);

    let mut scene = Scene::default();
    scene.push_command(Command::FillRect {
        rect: Rect::new(0.0, 0.0, 120.0, 90.0),
        paint: Paint::solid(Color::rgba(200, 30, 30, 255)),
        transform: Transform::IDENTITY,
        clip,
    });

    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    assert!(!plan.is_complete());
    assert_eq!(plan.unsupported.get(&Unsupported::ShapedClip), Some(&1));

    let refused = renderer.render(&plan, WIDTH, HEIGHT, Color::WHITE);
    assert!(
        refused.is_err(),
        "an incomplete plan must be refused, not partly drawn — see the module doc"
    );
    let message = refused.unwrap_err().to_string();
    assert!(
        message.contains("shaped-clip"),
        "the error should name what stopped it, got: {message}"
    );

    // And the escape hatch still works, for tools that want the partial view.
    assert!(renderer
        .render_incomplete(&plan, WIDTH, HEIGHT, Color::WHITE)
        .is_ok());
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn the_renderer_is_reusable_across_frames_and_sizes() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let mut scene = Scene::default();
    fill_rect(
        &mut scene,
        Rect::new(5.0, 5.0, 40.0, 40.0),
        Color::rgba(255, 255, 0, 255),
    );
    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);

    // Several frames at one size: the whole point of the persistent pipeline.
    for _ in 0..8 {
        let pixels = renderer
            .render(&plan, WIDTH, HEIGHT, Color::rgba(0, 0, 0, 255))
            .expect("every frame should render");
        assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
    }

    // Then a resize, which must rebuild only the size-dependent objects.
    let bigger = renderer
        .render(&plan, WIDTH * 2, HEIGHT * 2, Color::rgba(0, 0, 0, 255))
        .expect("a resize should not break the renderer");
    assert_eq!(bigger.len(), (WIDTH * 2 * HEIGHT * 2 * 4) as usize);

    // And back down again, which is the case a naive cache gets wrong.
    let smaller = renderer
        .render(&plan, WIDTH / 2, HEIGHT / 2, Color::rgba(0, 0, 0, 255))
        .expect("shrinking should work too");
    assert_eq!(smaller.len(), (WIDTH / 2 * (HEIGHT / 2) * 4) as usize);
}

#[test]
#[ignore = "needs a Vulkan loader + ICD (lavapipe or a real GPU); run with `cargo test --features vulkan -- --ignored`"]
fn many_shapes_stay_one_draw_call_and_still_match_the_cpu() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let base = Color::rgba(250, 250, 250, 255);
    let mut scene = Scene::default();
    // A grid of differently-coloured tiles: the case per-vertex colour exists
    // for, and the one a per-draw uniform would turn into 48 draw calls.
    for row in 0..6 {
        for column in 0..8 {
            let x = column as f32 * 15.0;
            let y = row as f32 * 15.0;
            fill_rect(
                &mut scene,
                Rect::new(x + 1.0, y + 1.0, x + 14.0, y + 14.0),
                Color::rgba(
                    (column * 30) as u8,
                    (row * 40) as u8,
                    ((column + row) * 20) as u8,
                    255,
                ),
            );
        }
    }

    let plan = plan_scene(&scene, WIDTH as f32, HEIGHT as f32);
    assert_eq!(
        plan.draw_call_count(),
        1,
        "48 tiles, one draw call — this is the batching claim, checked"
    );
    assert_eq!(plan.triangle_count(), 96, "two triangles per tile");

    let gpu = renderer
        .render(&plan, WIDTH, HEIGHT, base)
        .expect("render should succeed");
    let cpu = on_cpu(&scene, base);
    assert_eq!(
        interior_mismatches(&gpu, &cpu, 2),
        0,
        "batching must not change the picture"
    );
}
