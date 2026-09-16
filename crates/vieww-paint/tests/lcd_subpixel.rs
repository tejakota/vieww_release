//! `AaMode::Lcd`, exercised the only way that counts: through the renderer,
//! against real shaped text, comparing actual output bytes.
//!
//! The coverage-level identities — an LCD mask's channels averaging to the
//! gray coverage, an equal-channel LCD composite being the gray composite —
//! are unit-tested beside the code that establishes them
//! (`native/geometry/fill.rs`, `native/target.rs`). This file is the
//! integration half: that the *renderer* routes text to the LCD path when its
//! documented conditions hold, that it falls back to grayscale when they do
//! not, and that the fallback is observable in the output rather than assumed
//! from the call graph.
//!
//! The observable throughout is the same one a human eye would use: a pixel
//! whose R, G and B differ on the edge of neutral-coloured text over a
//! neutral background. Grayscale antialiasing cannot produce such a pixel —
//! one coverage number scaled into three identical channels. LCD subpixel
//! coverage produces them on exactly the vertical edges it exists to sharpen,
//! and nowhere it does not.

use vieww_foundation::{BlendMode, Color, Offset, Rect, TextStyle};
use vieww_paint::native::{AaMode, NativeRenderer};
use vieww_paint::{Canvas, Scene};
use vieww_text::{FontStore, Paragraph, TextSpan};

/// Shape `text` and return the scene that draws it — one run, neutral ink,
/// positioned over a white background. `size` is large enough that stems are
/// several pixels wide and their edges land at arbitrary sub-pixel phases.
fn text_scene(fonts: &mut FontStore, text: &str, size: f32) -> Scene {
    let spans = [TextSpan::new(text, TextStyle::new(size))];
    let mut run = Paragraph::layout(fonts, &spans, f32::INFINITY)
        .runs()
        .first()
        .cloned()
        .expect("a shaped run");
    run.color = Color::rgb(30, 30, 30);
    run.origin = Offset::new(12.0, 64.0);
    let mut scene = Scene::new();
    scene.draw_glyphs(&run);
    scene
}

/// How many pixels have visibly split channels — R, G or B differing by more
/// than one step, so a rounding difference in one channel cannot pass as a
/// fringe. On neutral text over a neutral background this is 0 under gray
/// antialiasing by construction, and non-zero under LCD exactly on stem
/// edges.
fn fringe_pixels(pixels: &vieww_paint::native::Pixels) -> usize {
    let data = pixels.data();
    data.chunks_exact(4)
        .filter(|&px| {
            let spread = px[0].max(px[1]).max(px[2]) - px[0].min(px[1]).min(px[2]);
            spread > 1
        })
        .count()
}

/// The headline behaviour: LCD text has channel-split edge pixels that gray
/// text cannot have, over the same background, from the same shaped run.
#[test]
fn lcd_text_carries_subpixel_fringes_and_gray_text_does_not() {
    let mut fonts = FontStore::embedded_only();
    let scene = text_scene(&mut fonts, "vieww hamburgefonstiv", 28.0);

    let gray = NativeRenderer::new()
        .render_to_pixels(&scene, 640, 100, Color::WHITE)
        .expect("gray render")
        .0;
    let lcd = NativeRenderer::with_aa_mode(AaMode::Lcd)
        .render_to_pixels(&scene, 640, 100, Color::WHITE)
        .expect("lcd render")
        .0;

    let gray_fringes = fringe_pixels(&gray);
    let lcd_fringes = fringe_pixels(&lcd);
    assert_eq!(
        gray_fringes, 0,
        "one coverage number cannot split channels — gray text must be neutral"
    );
    assert!(
        lcd_fringes > 20,
        "LCD text should fringe on many stem edges, found {lcd_fringes}"
    );
}

/// The fallback the `AaMode` documentation promises, observed rather than
/// assumed: text drawn *inside a layer* by an LCD renderer comes out gray,
/// because per-channel blending is only defined over the opaque root.
#[test]
fn lcd_mode_falls_back_to_gray_inside_layers() {
    let mut fonts = FontStore::embedded_only();
    let spans = [TextSpan::new("layered text", TextStyle::new(28.0))];
    let mut run = Paragraph::layout(&mut fonts, &spans, f32::INFINITY)
        .runs()
        .first()
        .cloned()
        .expect("a shaped run");
    run.color = Color::rgb(30, 30, 30);
    run.origin = Offset::new(12.0, 64.0);

    let mut scene = Scene::new();
    scene.push_layer(Rect::new(0.0, 0.0, 640.0, 100.0), 1.0, BlendMode::Normal);
    scene.draw_glyphs(&run);
    scene.pop_layer();

    let lcd = NativeRenderer::with_aa_mode(AaMode::Lcd)
        .render_to_pixels(&scene, 640, 100, Color::WHITE)
        .expect("lcd render of a layered scene")
        .0;
    assert_eq!(
        fringe_pixels(&lcd),
        0,
        "a layer is not an opaque surface — its glyphs must stay grayscale"
    );
}

/// The second fallback: LCD over a *translucent* frame background is
/// grayscale too, because the root is not opaque and the fringes would leak
/// through whatever the compositor puts underneath.
#[test]
fn lcd_mode_falls_back_to_gray_over_a_transparent_background() {
    let mut fonts = FontStore::embedded_only();
    let scene = text_scene(&mut fonts, "ghosted text", 28.0);
    let translucent = Color::rgba(255, 255, 255, 128);

    let lcd = NativeRenderer::with_aa_mode(AaMode::Lcd)
        .render_to_pixels(&scene, 640, 100, translucent)
        .expect("lcd render over a translucent root")
        .0;
    assert_eq!(
        fringe_pixels(&lcd),
        0,
        "a translucent root cannot absorb per-channel fringes — text must stay gray"
    );
}

/// LCD rasterisations are cached like gray ones: a second render of the same
/// unmoved text takes no new misses. This is the property that makes the
/// three-coverage-per-pixel cost a *miss-path* cost, paid once per distinct
/// glyph at a distinct phase rather than per frame.
#[test]
fn lcd_glyphs_cache_between_frames() {
    let mut fonts = FontStore::embedded_only();
    let scene = text_scene(&mut fonts, "twice drawn", 28.0);
    let mut renderer = NativeRenderer::with_aa_mode(AaMode::Lcd);

    let _ = renderer
        .render_to_pixels(&scene, 640, 100, Color::WHITE)
        .expect("first lcd render");
    let after_first = renderer.glyph_raster_cache_stats();
    assert!(after_first.misses > 0, "the first render must rasterise");

    let _ = renderer
        .render_to_pixels(&scene, 640, 100, Color::WHITE)
        .expect("second lcd render");
    let after_second = renderer.glyph_raster_cache_stats();
    assert_eq!(
        after_second.misses, after_first.misses,
        "unchanged text must not re-rasterise"
    );
    assert_eq!(
        after_second.hits - after_first.hits,
        after_first.misses,
        "every glyph of the second frame should be a hit"
    );
}

/// The mode is a property of the renderer, and the default is the one every
/// existing golden image was blessed under.
#[test]
fn the_aa_mode_default_is_gray_and_the_builder_sets_it() {
    assert_eq!(NativeRenderer::new().aa_mode(), AaMode::Gray);
    assert_eq!(
        NativeRenderer::with_aa_mode(AaMode::Lcd).aa_mode(),
        AaMode::Lcd
    );
}
