//! `native/linear.rs`'s opt-in linear-light compositing pipeline —
//! "Renderer v2" pillar B — exercised through a real [`Scene`] and a real
//! [`NativeRenderer`], not only through `blend_with_pipeline` in isolation
//! (that unit-level check lives in `native/linear.rs`'s own `tests`
//! module). This file checks the two things a caller actually needs to be
//! true: the default pipeline changes nothing about existing output, and
//! opting into `LinearLight` changes the *whole render*, not just one
//! function call.

use vieww_foundation::{BlendMode, Color, Rect};
use vieww_paint::native::{ColorPipeline, NativeRenderer};
use vieww_paint::{Canvas, Scene};

fn half_white_over_black_scene() -> Scene {
    let mut scene = Scene::new();
    scene.fill_rect(
        Rect::new(0.0, 0.0, 40.0, 40.0),
        Color::rgba(0, 0, 0, 255).into(),
    );
    scene.save();
    scene.push_layer(Rect::new(0.0, 0.0, 40.0, 40.0), 0.5, BlendMode::Normal);
    scene.fill_rect(
        Rect::new(0.0, 0.0, 40.0, 40.0),
        Color::rgba(255, 255, 255, 255).into(),
    );
    scene.pop_layer();
    scene.restore();
    scene
}

#[test]
fn the_default_renderer_is_byte_identical_to_a_renderer_with_gamma_space_named_explicitly() {
    let scene = half_white_over_black_scene();
    let (default_pixels, _) = NativeRenderer::new()
        .render_to_pixels(&scene, 40, 40, Color::WHITE)
        .expect("headless render");
    let (explicit_pixels, _) = NativeRenderer::with_color_pipeline(ColorPipeline::GammaSpace)
        .render_to_pixels(&scene, 40, 40, Color::WHITE)
        .expect("headless render");
    assert_eq!(default_pixels.data(), explicit_pixels.data());
}

#[test]
fn linear_light_end_to_end_produces_a_measurably_lighter_gray_than_the_default_pipeline() {
    let scene = half_white_over_black_scene();

    let (gamma_pixels, _) = NativeRenderer::new()
        .render_to_pixels(&scene, 40, 40, Color::WHITE)
        .expect("headless render");
    let (linear_pixels, _) = NativeRenderer::with_color_pipeline(ColorPipeline::LinearLight)
        .render_to_pixels(&scene, 40, 40, Color::WHITE)
        .expect("headless render");

    let center = gamma_pixels.pixel(20, 20);
    let center_linear = linear_pixels.pixel(20, 20);

    assert!(
        (120..=136).contains(&center.r),
        "gamma-space center should be close to the textbook encoded-0.5 gray: got {}",
        center.r
    );
    assert!(
        center_linear.r > center.r + 40,
        "linear-light center must be measurably lighter than gamma-space: gamma={}, linear={}",
        center.r,
        center_linear.r
    );
    // Both are still real, opaque gray pixels — the pipeline changes the
    // color science, not the shape or coverage of what got drawn.
    assert_eq!(center.a, 255);
    assert_eq!(center_linear.a, 255);
    assert_eq!(center.r, center.g);
    assert_eq!(center.g, center.b);
    assert_eq!(center_linear.r, center_linear.g);
    assert_eq!(center_linear.g, center_linear.b);
}
