//! `native/residency.rs`'s `ResidencyCache`, exercised through the one place
//! it is actually wired in: `native/glyph.rs`'s per-font glyph-outline
//! cache. `crates/vieww-paint/src/native/residency.rs`'s own unit tests
//! cover the cache in isolation; this file is the integration half —
//! proving eviction really happens during real rendering, and that a glyph
//! whose outline was evicted and re-derived draws exactly the pixels it drew
//! the first time. "Renderer v2" pillar A's dependency-graph tests
//! (`crates/vieww-paint/src/graph.rs`) and this pillar's residency tests
//! both live in this crate, on either side of the same principle: an
//! internal restructuring is only real work once it is checked against
//! actual rendered output, not only against itself.

use vieww_foundation::{Color, Offset, TextStyle};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Scene};
use vieww_text::{FontStore, Paragraph, TextSpan};

/// Shape and rasterize `text`. `fonts` is a `&mut` passed in rather than
/// created fresh here on purpose: `native/glyph.rs`'s cache is keyed by
/// `FontData::id()`, which is stable for one loaded font but not
/// guaranteed to be the same value across two separate `FontStore`s that
/// happen to load the same font bytes — a fresh `FontStore` per call would
/// silently defeat the very cache this file exists to exercise, by giving
/// every call a "new" font it had never seen before.
fn render(
    renderer: &mut NativeRenderer,
    fonts: &mut FontStore,
    text: &str,
) -> vieww_paint::native::Pixels {
    let spans = [TextSpan::new(text, TextStyle::new(24.0))];
    let mut run = Paragraph::layout(fonts, &spans, f32::INFINITY)
        .runs()
        .first()
        .cloned()
        .expect("a shaped run");
    run.color = Color::rgba(20, 20, 20, 255);
    run.origin = Offset::new(10.0, 40.0);

    let mut scene = Scene::new();
    scene.draw_glyphs(&run);
    let (pixels, report) = renderer
        .render_to_pixels(&scene, 400, 80, Color::WHITE)
        .expect("vieww's own rasterizer needs no display");
    assert!(report.glyphs > 0, "the run must have reached the renderer");
    pixels
}

/// Draw the same text twice with nothing in between: the second draw must
/// reuse everything and produce the same pixels.
///
/// # Why this asserts on the *raster* cache rather than the outline cache
///
/// It used to assert that the second draw registered glyph-*outline* hits, and
/// that was the right assertion when the outline was the only thing kept: every
/// frame re-derived, re-placed, re-flattened and re-rasterised each glyph, and
/// an outline hit was the only reuse there was to observe.
///
/// `native/glyph_raster.rs` now keeps the finished coverage, so the second draw
/// never reaches the outline cache at all — which is the entire point of it, and
/// which made "the outline cache registered a hit" fail for the best possible
/// reason. The property worth asserting is unchanged and is asserted here:
/// nothing about the glyph is recomputed, and the pixels are identical. It is
/// simply observed one layer up, where the reuse now happens.
#[test]
fn a_repeated_draw_is_a_cache_hit_and_produces_identical_pixels() {
    let mut renderer = NativeRenderer::new();
    let mut fonts = FontStore::embedded_only();
    let first = render(&mut renderer, &mut fonts, "vieww");
    let outlines_after_first = renderer.glyph_outline_cache_stats();
    let rasters_after_first = renderer.glyph_raster_cache_stats();
    assert!(
        rasters_after_first.misses > 0,
        "the first draw must rasterise every glyph at least once"
    );
    assert!(
        outlines_after_first.misses > 0,
        "the first draw must derive every outline at least once"
    );

    let second = render(&mut renderer, &mut fonts, "vieww");
    let outlines_after_second = renderer.glyph_outline_cache_stats();
    let rasters_after_second = renderer.glyph_raster_cache_stats();
    assert_eq!(
        rasters_after_second.misses, rasters_after_first.misses,
        "a repeated draw of the same text must not re-rasterise any glyph"
    );
    assert!(
        rasters_after_second.hits > rasters_after_first.hits,
        "a repeated draw must register raster-cache hits"
    );
    assert_eq!(
        outlines_after_second.misses, outlines_after_first.misses,
        "and must not re-derive any outline either"
    );
    assert_eq!(
        first.data(),
        second.data(),
        "identical scenes must produce identical pixels"
    );
}

/// Draw enough distinct, never-repeated text that the glyph-outline cache's
/// budget is exceeded and it starts evicting — then draw the very first
/// string again and check the re-derived outline still inks exactly the
/// pixels it inked the first time. This is `ResidencyCache`'s whole point,
/// checked against a real renderer rather than only against itself: an
/// eviction must be invisible to what gets drawn.
#[test]
fn a_glyph_outline_evicted_under_budget_pressure_redraws_identically_once_re_derived() {
    // A deliberately tiny budget — real content easily has enough distinct
    // glyphs to exceed the built-in default eventually, but a test should
    // not depend on "eventually"; this makes eviction happen after only a
    // few distinct glyphs, deterministically.
    let mut renderer = NativeRenderer::with_glyph_outline_budget_bytes(256);
    let mut fonts = FontStore::embedded_only();
    let baseline = render(&mut renderer, &mut fonts, "The quick brown fox");

    // Enough distinct long strings, each with its own glyphs, to push the
    // per-font residency cache well past its budget and force real
    // evictions — not just fill it once.
    let filler_sentences = [
        "jumps over the lazy dog while zebras question everything",
        "Pack my box with five dozen liquor jugs quickly today",
        "Sphinx of black quartz judge my vow before nightfall",
        "Waltz nymph for quick jigs vex Bud in the hallway",
        "How vexingly quick daft zebras jump over lazy foxes",
        "Grumpy wizards make toxic brew for the evil queen",
        "Crazy Fredrick bought many very exquisite opal jewels",
        "Amazingly few discotheques provide jukeboxes for extra cash",
    ];
    for sentence in filler_sentences {
        render(&mut renderer, &mut fonts, sentence);
    }

    let stats = renderer.glyph_outline_cache_stats();
    assert!(
        stats.evictions > 0,
        "enough distinct glyphs must have been drawn to exceed the budget and evict something: {stats:?}"
    );

    // **The raster cache has to be dropped for this test to still test
    // anything.** With it warm, redrawing the baseline string is answered from
    // finished coverage and the outline cache is never consulted — so the
    // re-derivation this test exists to check would not happen, and the
    // assertion below would pass without exercising it. `Trim` at the
    // everything level is the supported way to drop both caches, and dropping
    // both makes the check stronger rather than weaker: every stage of the
    // glyph pipeline is recomputed from the font bytes, and the pixels still
    // have to match.
    vieww_foundation::Trim::trim(&mut renderer, vieww_foundation::MemoryPressure::Critical);
    let before_redraw = renderer.glyph_outline_cache_stats();

    let redrawn = render(&mut renderer, &mut fonts, "The quick brown fox");
    assert!(
        renderer.glyph_outline_cache_stats().misses > before_redraw.misses,
        "the redraw must actually re-derive outlines, or this test proves nothing"
    );
    assert_eq!(
        baseline.data(),
        redrawn.data(),
        "a re-derived (possibly evicted-and-recomputed) outline must ink the same pixels as the first draw"
    );
}
