//! Text on the GPU, against the CPU rasterizer that decides what text looks
//! like.
//!
//! # Why this file exists separately from `vulkan_scene.rs`
//!
//! Because the claim is different, and so is the tolerance it can be held to.
//!
//! `vulkan_scene.rs` compares two *different* rasterizations of the same
//! shapes — analytic scanline coverage on one side, `lyon` triangulation and
//! hardware sampling on the other — so it asserts that interiors agree and
//! allows the edges to disagree. That is the honest bound for two independent
//! implementations, and it is why that file's own doc says asking for exact
//! equality "would mean pinning both to one implementation forever."
//!
//! Text is not that situation. There is **one** glyph rasterizer in this
//! workspace, and the GPU path samples the coverage it produced — see
//! `vieww_paint::native::glyph_coverage`'s module doc for why a second one
//! would have made this suite unable to catch anything. So the only difference
//! between the two pictures is that the atlas holds coverage as `u8` where the
//! compositor holds it as `f32`, which moves a composited channel by at most
//! one step of 1/255.
//!
//! That makes the assertion here much stronger than the shape suite's:
//! **every pixel, within 1/255, including the antialiased rim of every glyph.**
//! A mirrored frame, a glyph placed a pixel off, a wrong atlas patch, a colour
//! applied before coverage instead of after — none of those can hide inside a
//! bound that tight, and all of them can hide inside "interiors match".
//!
//! `#[ignore]`d like every Vulkan suite here: needs a loader and an ICD
//! (`lavapipe`, or real hardware).
//! `cargo test -p vieww-hal --features vulkan -- --ignored`

#![cfg(feature = "vulkan")]

use vieww_foundation::{Color, Offset, Rect, TextStyle, Transform};
use vieww_gpu::Planner;
use vieww_hal::vulkan::{SceneRenderer, VulkanDevice};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Clip, Command, Paint, Scene};
use vieww_text::{FontStore, Paragraph, TextSpan};

const WIDTH: u32 = 220;
const HEIGHT: u32 = 80;

/// See `vulkan_scene.rs`'s `ONE_DEVICE_AT_A_TIME` for why this is here: Mesa's
/// software rasterizer faults when several devices in one process are used
/// concurrently, and this file creates devices exactly like that one does.
static ONE_DEVICE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

type Exclusive = std::sync::MutexGuard<'static, ()>;

fn try_renderer() -> Option<(Exclusive, VulkanDevice, SceneRenderer)> {
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
    Some((guard, device, renderer))
}

/// Real shaped text, not hand-picked glyph ids.
///
/// Going through `Paragraph` means the runs carry the origins, offsets,
/// advances and font the real text pipeline produces — including the
/// fractional baseline positions that are the whole reason the glyph cache
/// keys on sub-pixel phase. Hand-assembling a `GlyphRun` with round numbers
/// would test the one case where phase cannot go wrong.
///
/// The fonts are the embedded ones, not the system's. This file compares the
/// GPU with the CPU rasterizer, and which face a system scan resolves "sans
/// serif" to is not part of that claim. On the macOS GPU runner it resolved to
/// a system face that *both* rasterizers drew as nothing, so four tests failed
/// with blank frames on each side while the GPU path itself was never
/// exercised.
fn text_scene(text: &str, at: Offset, color: Color) -> Scene {
    text_scene_in(&mut FontStore::embedded_only(), text, at, color)
}

/// [`text_scene`], sharing a caller's font store.
///
/// # Why any test comparing two scenes must use this
///
/// `FontData` compares by `Rc` identity, and `FontStore`'s own doc says that
/// caching the bytes it hands out "is not an optimisation, it is a correctness
/// requirement" — a fresh `Rc` per frame would make damage tracking see every
/// text run as changed. The glyph atlas keys on `FontData::id`, so it inherits
/// that invariant exactly: two stores loading *the same file* produce two font
/// identities and therefore two full sets of atlas patches.
///
/// That is correct behaviour, and it is also a trap for a test. The first draft
/// of `two_colours_share_one_atlas_patch` built a store per scene and then
/// asserted the patches were shared; it failed with 8 packed glyphs against an
/// expected 4, and the renderer was right both times. An application holds one
/// store, so a test that wants to say anything about atlas reuse has to as
/// well.
fn text_scene_in(store: &mut FontStore, text: &str, at: Offset, color: Color) -> Scene {
    let style = TextStyle {
        color,
        size: 17.0,
        ..TextStyle::default()
    };
    let paragraph = Paragraph::layout(store, &[TextSpan::new(text, style)], 1_000.0);

    let mut scene = Scene::default();
    for run in paragraph.runs() {
        let mut run = run.clone();
        run.origin = Offset::new(run.origin.dx + at.dx, run.origin.dy + at.dy);
        run.color = color;
        scene.push_command(Command::DrawGlyphs {
            run,
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
    }
    assert!(
        !scene.commands().is_empty(),
        "the test needs actual glyphs to compare"
    );
    scene
}

fn cpu(scene: &Scene, clear: Color) -> Vec<u8> {
    let mut renderer = NativeRenderer::new();
    let (pixels, _report) = renderer
        .render_to_pixels(scene, WIDTH, HEIGHT, clear)
        .expect("the CPU rasterizer is the oracle and must not fail");
    pixels.data().to_vec()
}

/// Compare two RGBA8 buffers channel by channel.
///
/// Returns the worst difference found and where, rather than asserting inside:
/// a caller that knows what it is testing writes a better message than a
/// helper can, and the worst-case number is the thing worth putting in it.
fn worst_difference(gpu: &[u8], reference: &[u8]) -> (u8, usize) {
    assert_eq!(gpu.len(), reference.len(), "same size buffers");
    let mut worst = 0u8;
    let mut worst_at = 0usize;
    for (i, (g, c)) in gpu.iter().zip(reference).enumerate() {
        let delta = g.abs_diff(*c);
        if delta > worst {
            worst = delta;
            worst_at = i;
        }
    }
    (worst, worst_at)
}

fn assert_matches_cpu(gpu: &[u8], reference: &[u8], what: &str) {
    let (worst, at) = worst_difference(gpu, reference);
    let pixel = at / 4;
    assert!(
        worst <= 1,
        "{what}: worst channel difference {worst} at pixel ({}, {}) — the \
         atlas rounds coverage to u8, which is worth at most 1. Anything \
         larger is a placement, orientation or colour bug, not rounding. \
         gpu={:?} cpu={:?}",
        pixel % WIDTH as usize,
        pixel / WIDTH as usize,
        &gpu[at / 4 * 4..at / 4 * 4 + 4],
        &reference[at / 4 * 4..at / 4 * 4 + 4],
    );
}

/// How many pixels differ from the clear colour — a rendering that drew
/// nothing at all is the failure mode a tolerance-based test is blind to,
/// because two blank images agree perfectly.
fn inked_pixels(pixels: &[u8], clear: Color) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| p[0] != clear.r || p[1] != clear.g || p[2] != clear.b || p[3] != clear.a)
        .count()
}

#[test]
#[ignore = "needs a Vulkan ICD"]
fn a_line_of_text_matches_the_cpu_rasterizer() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let scene = text_scene(
        "Hamburgefonstiv 123",
        Offset::new(8.0, 40.0),
        Color::rgba(20, 20, 20, 255),
    );

    let mut planner = Planner::new();
    let plan = planner.plan(
        &scene,
        f32::from(u16::try_from(WIDTH).unwrap()),
        f32::from(u16::try_from(HEIGHT).unwrap()),
    );

    assert!(
        plan.is_complete(),
        "text must now be plannable — {:?}",
        plan.unsupported
    );
    assert!(
        !planner.atlas().is_empty(),
        "the glyphs must have been packed into the atlas"
    );

    let gpu = renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .expect("a complete plan must render");
    let reference = cpu(&scene, clear);

    // Before comparing: both must have actually drawn something. Two blank
    // frames match perfectly and prove nothing, and "the GPU drew no text"
    // is precisely the failure this whole file exists to detect.
    let inked = inked_pixels(&reference, clear);
    assert!(inked > 200, "the CPU reference drew only {inked} pixels");
    assert!(
        inked_pixels(&gpu, clear) > 200,
        "the GPU drew {} inked pixels against the CPU's {inked} — the text is \
         missing, not merely different",
        inked_pixels(&gpu, clear)
    );

    assert_matches_cpu(&gpu, &reference, "a line of text");
}

/// The asymmetry test, in the spirit of `vulkan_mesh_smoke`'s: text near the
/// top of a tall frame is the single most sensitive thing to a Y flip, and a
/// flip is the defect this workspace has already shipped once in a shader and
/// found twice in review.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn text_near_the_top_edge_is_not_vertically_mirrored() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let scene = text_scene("TOP", Offset::new(6.0, 16.0), Color::rgba(0, 0, 0, 255));

    let mut planner = Planner::new();
    let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    let gpu = renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .expect("a complete plan must render");
    let reference = cpu(&scene, clear);

    assert_matches_cpu(&gpu, &reference, "text near the top edge");

    // And say the thing directly, so this test still fails for the right
    // reason if the tolerance above is ever loosened: the ink is in the top
    // half of the frame, and a mirrored frame puts it in the bottom half.
    let half = (HEIGHT / 2) as usize * WIDTH as usize * 4;
    let top = inked_pixels(&gpu[..half], clear);
    let bottom = inked_pixels(&gpu[half..], clear);
    assert!(
        top > 0 && bottom == 0,
        "text drawn at y=16 of an {HEIGHT}px frame must be entirely in the \
         top half: {top} inked pixels above the midline, {bottom} below"
    );
}

/// Text and shapes in one scene, which is what every real screen is.
///
/// The property under test is the one the shared white texel buys: a fill and
/// a glyph go through the same pipeline, in one batch, and the fill is
/// unaffected by the coverage multiply.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn a_label_on_a_panel_is_one_batch_and_still_matches() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);

    let mut scene = Scene::default();
    scene.push_command(Command::FillRect {
        rect: Rect::new(4.0, 8.0, 210.0, 68.0),
        paint: Paint::solid(Color::rgba(40, 90, 200, 255)),
        transform: Transform::IDENTITY,
        clip: Clip::NONE,
    });
    for command in text_scene(
        "Label on a panel",
        Offset::new(14.0, 44.0),
        Color::rgba(255, 255, 255, 255),
    )
    .commands()
    {
        scene.push_command(command.clone());
    }

    let mut planner = Planner::new();
    let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    assert!(plan.is_complete(), "{:?}", plan.unsupported);
    assert_eq!(
        plan.draw_call_count(),
        1,
        "a panel and the label on it share a scissor, so they must share a \
         draw call — this is the entire reason solid geometry samples a \
         reserved full-coverage texel instead of using a second pipeline"
    );

    let gpu = renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .expect("a complete plan must render");
    assert_matches_cpu(&gpu, &cpu(&scene, clear), "a label on a panel");
}

/// A second frame must not re-upload, and must draw the same thing.
///
/// The atlas is the one piece of state that survives a frame, so it is the one
/// piece that can go stale. A renderer that re-uploaded every frame would pass
/// a single-frame test and cost a texture upload per frame forever; one that
/// never re-uploaded would pass this and break the moment a new glyph appeared,
/// which is why the third frame introduces one.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn a_repeated_frame_reuses_the_atlas_and_a_new_glyph_grows_it() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let mut planner = Planner::new();

    let mut store = FontStore::embedded_only();
    let first_scene = text_scene_in(
        &mut store,
        "abc",
        Offset::new(10.0, 40.0),
        Color::rgba(0, 0, 0, 255),
    );
    let first = planner.plan(&first_scene, WIDTH as f32, HEIGHT as f32);
    let version_after_first = planner.atlas().version();
    let glyphs_after_first = planner.atlas().len();
    assert!(
        glyphs_after_first >= 3,
        "three distinct letters were packed"
    );

    let second = planner.plan(&first_scene, WIDTH as f32, HEIGHT as f32);
    assert_eq!(
        planner.atlas().version(),
        version_after_first,
        "the same text at the same position rasterises no new glyph, so the \
         atlas must be untouched and the backend must upload nothing"
    );
    assert_eq!(
        first.vertices, second.vertices,
        "and the plan itself must be identical, or the atlas coordinates \
         moved under a frame that did not change"
    );

    let third_scene = text_scene_in(
        &mut store,
        "abcxyz",
        Offset::new(10.0, 40.0),
        Color::rgba(0, 0, 0, 255),
    );
    let third = planner.plan(&third_scene, WIDTH as f32, HEIGHT as f32);
    assert!(
        planner.atlas().version() > version_after_first,
        "three new letters must change the atlas"
    );
    assert_eq!(
        planner.atlas().len(),
        glyphs_after_first + 3,
        "exactly the three new letters are packed — `abc` was already there, \
         and one font store means one font identity"
    );

    // The frame after a growth still has to be right — this is where a
    // renderer that cached the old image view or forgot to rewrite its
    // descriptor draws a screen of garbage.
    let gpu = renderer
        .render_planned(&planner, &third, WIDTH, HEIGHT, clear)
        .expect("a complete plan must render");
    assert_matches_cpu(
        &gpu,
        &cpu(&third_scene, clear),
        "the frame after a new glyph",
    );
}

/// Colour must be applied *through* coverage, not before it.
///
/// Red text and blue text share every atlas patch — the atlas holds coverage,
/// not colour. A renderer that baked colour into the atlas, or that sampled
/// the wrong channel, passes a single-colour test and fails this one.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn two_colours_share_one_atlas_patch() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let mut planner = Planner::new();

    let mut store = FontStore::embedded_only();
    let red_scene = text_scene_in(
        &mut store,
        "same",
        Offset::new(10.0, 40.0),
        Color::rgba(220, 30, 30, 255),
    );
    let red_plan = planner.plan(&red_scene, WIDTH as f32, HEIGHT as f32);
    let packed = planner.atlas().len();

    let blue_scene = text_scene_in(
        &mut store,
        "same",
        Offset::new(10.0, 40.0),
        Color::rgba(30, 30, 220, 255),
    );
    let blue_plan = planner.plan(&blue_scene, WIDTH as f32, HEIGHT as f32);
    assert_eq!(
        planner.atlas().len(),
        packed,
        "the same glyphs in a different colour must reuse the same patches"
    );

    let red = renderer
        .render_planned(&planner, &red_plan, WIDTH, HEIGHT, clear)
        .expect("renders");
    assert_matches_cpu(&red, &cpu(&red_scene, clear), "red text");

    let blue = renderer
        .render_planned(&planner, &blue_plan, WIDTH, HEIGHT, clear)
        .expect("renders");
    assert_matches_cpu(&blue, &cpu(&blue_scene, clear), "blue text");

    assert_ne!(red, blue, "the two colours must not produce the same frame");
}

/// Translucent text: coverage and the paint's own alpha both apply.
///
/// `color.a * coverage` is one multiply in the shader and the place a
/// premultiply mistake hides — it looks correct at alpha 1.0, which is what
/// every other test in this file uses.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn half_transparent_text_matches_the_cpu() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let scene = text_scene("faded", Offset::new(10.0, 40.0), Color::rgba(0, 0, 0, 128));

    let mut planner = Planner::new();
    let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    let gpu = renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .expect("renders");
    assert_matches_cpu(&gpu, &cpu(&scene, clear), "half-transparent text");
}

/// A clip that cuts a line of text in half must cut it in the same place.
#[test]
#[ignore = "needs a Vulkan ICD"]
fn a_clip_cuts_text_where_the_cpu_cuts_it() {
    let Some((_lock, _device, mut renderer)) = try_renderer() else {
        return;
    };
    let clear = Color::rgba(255, 255, 255, 255);
    let mut clip = Clip::NONE;
    clip.add_rect(Rect::new(0.0, 0.0, 90.0, HEIGHT as f32));

    let mut scene = Scene::default();
    for command in text_scene(
        "clipped in half",
        Offset::new(8.0, 40.0),
        Color::rgba(0, 0, 0, 255),
    )
    .commands()
    {
        let Command::DrawGlyphs { run, transform, .. } = command else {
            continue;
        };
        scene.push_command(Command::DrawGlyphs {
            run: run.clone(),
            transform: *transform,
            clip: clip.clone(),
        });
    }

    let mut planner = Planner::new();
    let plan = planner.plan(&scene, WIDTH as f32, HEIGHT as f32);
    let gpu = renderer
        .render_planned(&planner, &plan, WIDTH, HEIGHT, clear)
        .expect("renders");
    assert_matches_cpu(&gpu, &cpu(&scene, clear), "clipped text");

    // And the clip did something, so the comparison above is not vacuous.
    let right_of_clip: usize = gpu
        .chunks_exact(4)
        .enumerate()
        .filter(|(i, p)| i % WIDTH as usize >= 90 && p[0] != 255)
        .count();
    assert_eq!(right_of_clip, 0, "nothing may be drawn past the clip");
}
