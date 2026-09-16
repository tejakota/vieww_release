//! The exit test for memory pressure: caches that actually empty, and a
//! picture that does not change when they do.
//!
//! ```console
//! cargo test -p vieww --features native --test memory_pressure
//! ```
//!
//! # Why every assertion here is paired
//!
//! `crates/vieww/tests/incremental_scene.rs` makes the argument at length and
//! it applies exactly: a counter alone and a picture alone are each worthless,
//! in opposite directions.
//!
//! A **counter** test says the cache emptied. A `trim` that dropped the pixels
//! along with the cache would score perfectly — and be a black screen.
//!
//! A **pixel** test says the picture survived. A `trim` that was a no-op would
//! score perfectly — and be the bug this whole seam exists to prevent, silently
//! present on the one platform where it matters and nowhere a desktop
//! developer would ever see it.
//!
//! So each level asserts both: that the thing was released, *and* that the
//! frame after the release is byte-for-byte what the frame before it was. That
//! second property is the entire [`Trim`] contract.
//!
//! # Why vieww's own rasterizer
//!
//! It is the only renderer this crate ships, and it needs no display or
//! adapter, so this runs in a container.
//!
//! # No image cache here
//!
//! The old vello-based backends held a converted copy of every decoded image
//! (a `Pixmap`, hundreds of kilobytes of pure derived data) and a moderate
//! warning's whole design was releasing that large idle thing while keeping
//! the small hot font cache. Vieww's own rasterizer samples
//! [`vieww::foundation::Image`] data directly and holds no derived copy of its
//! own, so there is nothing there to trim — the tests below that used to
//! assert an image-cache release now assert what is actually real for this
//! renderer: its one cache, of derived glyph outlines.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Image, MemoryPressure, Rect, Size, Trim};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;

const WIDTH: u32 = 120;
const HEIGHT: u32 = 90;

/// A screen with text (which populates the shaping and font caches) and an
/// image (which populates the pixmap cache), so one frame fills all three.
#[derive(Debug)]
struct Screen {
    picture: Image,
}

impl Widget for Screen {
    fn debug_name(&self) -> &'static str {
        "Screen"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(children![
                Text::new("cache me").style(TextStyle::new(14.0)),
                vieww::widget::Image::new(self.picture.clone())
                    .width(32.0)
                    .height(32.0),
            ])
            .into()
    }
}

vieww::widget::widget_node_from!(Screen);

fn picture() -> Image {
    // A recognisable gradient rather than one flat colour, so a cache serving
    // the wrong entry shows up as different pixels rather than as nothing.
    let mut pixels = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16_u32 {
        for x in 0..16_u32 {
            pixels.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 128, 255]);
        }
    }
    Image::from_rgba8(pixels, 16, 16)
}

/// A driver settled on the screen above, and the renderer that drew it.
///
/// The renderer is returned rather than made per call because *it* is what
/// holds the image and font caches — a fresh one each time would be a cold
/// cache and nothing here would mean anything.
fn settled() -> (FrameDriver, NativeRenderer) {
    let mut driver = FrameDriver::new(Size::new(WIDTH as f32, HEIGHT as f32));
    driver.set_root(Screen { picture: picture() });
    driver.draw_frame();
    driver.draw_frame();

    let mut renderer = NativeRenderer::new();
    renderer
        .render_to_pixels(driver.scene(), WIDTH, HEIGHT, Color::WHITE)
        .expect("rasterising");
    (driver, renderer)
}

fn pixels(driver: &FrameDriver, renderer: &mut NativeRenderer) -> Vec<u8> {
    let (pixels, _) = renderer
        .render_to_pixels(driver.scene(), WIDTH, HEIGHT, Color::WHITE)
        .expect("rasterising");
    pixels.data().to_vec()
}

/// Nothing is released when nothing is wrong. The guard clause every `trim`
/// implementation opens with, asserted once so that a refactor that drops it
/// is caught somewhere other than a phone.
#[test]
fn no_pressure_releases_nothing() {
    let (mut driver, mut renderer) = settled();
    let fonts = renderer.cached_fonts();
    let shapes = driver.owner_mut().tree_mut().fonts_mut().retained_shapes();
    assert!(fonts > 0, "the fixture must actually fill the glyph cache");
    assert!(shapes > 0, "the fixture must actually fill the shape cache");

    renderer.trim(MemoryPressure::None);
    driver.trim_memory(MemoryPressure::None);

    assert_eq!(renderer.cached_fonts(), fonts);
    assert_eq!(
        driver.owner_mut().tree_mut().fonts_mut().retained_shapes(),
        shapes
    );
}

/// The moderate level's whole design: keep anything a visible frame would have
/// to recompute. Every text run on screen needs its font again on the very
/// next frame, so a moderate warning must not make that frame slow.
#[test]
fn a_moderate_warning_keeps_the_glyph_cache_warm() {
    let (_driver, mut renderer) = settled();
    assert!(renderer.cached_fonts() > 0);

    renderer.trim(MemoryPressure::Moderate);

    assert!(
        renderer.cached_fonts() > 0,
        "a moderate warning must not make the next frame's text slow"
    );
}

#[test]
fn a_critical_warning_releases_everything_the_renderer_holds() {
    let (_driver, mut renderer) = settled();

    renderer.trim(MemoryPressure::Critical);

    assert_eq!(
        renderer.cached_fonts(),
        0,
        "at critical the trade stops applying: a 40ms frame beats being killed"
    );
}

/// The driver half — the text caches and the retained scene, which are the
/// pieces a `FrameDriver` owns rather than the renderer.
/// The driver half — the text caches and the retained scene, which are the
/// pieces a `FrameDriver` owns rather than the renderer.
///
/// Asserted on `retained_shapes` (cache size) and **not** on `shape_count`,
/// which is cumulative work and does not go down when a cache is emptied. The
/// first draft of this test used `shape_count` and failed for that reason —
/// worth keeping in the record, because the two names are one word apart and
/// only one of them can answer this question.
#[test]
fn a_critical_warning_releases_the_shaped_paragraphs() {
    let (mut driver, _renderer) = settled();
    assert!(
        driver.owner_mut().tree_mut().fonts_mut().retained_shapes() > 0,
        "the fixture must actually shape something"
    );

    driver.trim_memory(MemoryPressure::Critical);

    assert_eq!(
        driver.owner_mut().tree_mut().fonts_mut().retained_shapes(),
        0,
        "shaped paragraphs are the largest derived thing the text layer holds"
    );
}

// The matching claim for the *text* caches — that a trimmed paragraph is
// genuinely re-shaped rather than quietly recovered — is a `vieww-text` unit
// test (`shaping_after_a_trim_is_real_work`), not one here. An idle frame never
// lays out, so it never reaches the shaper at all: a frame-level assertion
// about shaping would be measuring the layout dirty set, which is exactly the
// "test that could not fail" `TRACKER.md` records for checklist 6. The claim
// belongs at the layer that can actually drive it.

/// **The contract.** Trimming is only ever allowed to cost time, never to
/// change a pixel — that is what makes it safe to call from a signal nobody
/// scheduled, at any point between frames.
///
/// Runs every level, including `Backgrounded`, because the most aggressive one
/// is the one most likely to drop something load-bearing.
#[test]
fn trimming_at_any_level_draws_exactly_the_same_picture() {
    for level in [
        MemoryPressure::Moderate,
        MemoryPressure::Critical,
        MemoryPressure::Backgrounded,
    ] {
        let (mut driver, mut renderer) = settled();
        let before = pixels(&driver, &mut renderer);

        renderer.trim(level);
        driver.trim_memory(level);
        // A real application draws again after the warning; so does this.
        driver.draw_frame();
        let after = pixels(&driver, &mut renderer);

        assert_eq!(
            before, after,
            "{level} pressure changed what was drawn, which the Trim contract forbids"
        );
    }
}

/// The renderer's cache refills by itself afterwards. A trim that emptied a
/// cache permanently — by leaving a flag set, say — would pass every assertion
/// above and turn one memory warning into an application that is slow for ever.
///
/// Only the glyph cache is asserted, because only it is this renderer's own —
/// it is refilled by *drawing*. The driver's shape cache is refilled by
/// **laying out**, and an idle frame deliberately never lays out — see the
/// note above [`trimming_at_any_level_draws_exactly_the_same_picture`].
#[test]
fn the_renderers_cache_refills_after_a_trim() {
    let (mut driver, mut renderer) = settled();

    renderer.trim(MemoryPressure::Critical);
    driver.trim_memory(MemoryPressure::Critical);
    assert_eq!(renderer.cached_fonts(), 0);

    driver.draw_frame();
    let _ = pixels(&driver, &mut renderer);

    assert!(
        renderer.cached_fonts() > 0,
        "the next frame must repopulate the cache, not run cold for ever"
    );
}

/// The retained scene is dropped only at the serious levels, and the frame
/// after still produces the same commands. `Rect` is imported for this: the
/// scene's own bounds are the cheapest whole-scene equality check available.
#[test]
fn the_retained_scene_survives_a_moderate_warning_and_rebuilds_after_a_critical_one() {
    let (mut driver, _renderer) = settled();
    let bounds: Rect = driver.scene().bounds();
    assert!(!bounds.is_empty(), "the fixture must draw something");

    driver.trim_memory(MemoryPressure::Moderate);
    assert_eq!(
        driver.scene().bounds(),
        bounds,
        "a moderate warning keeps the retained scene"
    );

    driver.trim_memory(MemoryPressure::Backgrounded);
    driver.draw_frame();
    assert_eq!(
        driver.scene().bounds(),
        bounds,
        "the scene is rebuilt in full and comes back identical"
    );
}
