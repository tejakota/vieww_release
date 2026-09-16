//! Scene rendering, checked at the pixels: [`Scene`] built by hand, rendered
//! through [`vieww_paint::native::NativeRenderer`] — this crate's one and
//! only renderer — and asserted on the actual colours that landed, with a PNG
//! written to `target/tmp/native_parity/` for a person to look at directly.
//!
//! # Why this file used to be named `native_parity`
//!
//! It began as a side-by-side comparison: the same [`Scene`] rendered through
//! `vello_cpu` and through this renderer, diffed pixel by pixel, to prove the
//! two agreed before this renderer became the default. That vello-based
//! backend is now gone entirely — see `docs/RENDERER-MIGRATION.md` — so there
//! is nothing left to compare against. What remains is the fixture set and
//! the PNG-per-scene habit, repointed at real assertions about what this
//! renderer actually draws rather than at agreement with a backend that no
//! longer exists. The filename stayed so `git log` keeps this file's history
//! attached to it; a fresh reader should read this doc comment, not the name.

use std::path::PathBuf;

use vieww_foundation::{
    BlendMode, Color, Gradient, GradientStop, Image, Offset, Path, Rect, Shadow, StrokeCap,
    StrokeJoin, StrokeStyle, TextStyle,
};
use vieww_paint::native::NativeRenderer;
use vieww_paint::{Canvas, Paint, Scene, Stroke};

/// Under `target/` (cargo's per-package scratch directory for integration
/// tests), never in the source tree: a test run must not dirty the checkout,
/// and `ci/check/release-clean-check.sh` fails a tree with generated output in
/// it — which made the release gate's packaging step fail after its own tests.
fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("native_parity");
    std::fs::create_dir_all(&dir).expect("create native_parity output dir");
    dir
}

fn save_png(name: &str, data: &[u8]) {
    let path = output_dir().join(format!("{name}.png"));
    std::fs::write(&path, data).unwrap_or_else(|e| panic!("writing {path:?}: {e}"));
}

/// Render `scene` through this crate's renderer, write a PNG to
/// `target/tmp/native_parity/<name>.png` for visual inspection, and hand back the
/// straight-alpha RGBA8 pixels plus the report.
fn render(
    name: &str,
    scene: &Scene,
    width: u32,
    height: u32,
    base: Color,
) -> (
    vieww_paint::native::Pixels,
    vieww_paint::native::SceneReport,
) {
    let mut renderer = NativeRenderer::new();
    let (pixels, report) = renderer
        .render_to_pixels(scene, width, height, base)
        .expect("vieww's own rasterizer needs no display");
    save_png(name, &pixels.encode_png().expect("encode png"));
    eprintln!(
        "{name}: shapes={} images={} glyph_runs={} layers={} unsupported_blends={}",
        report.shapes, report.images, report.glyph_runs, report.layers, report.unsupported_blends,
    );
    (pixels, report)
}

/// What a colour at `alpha` over `under` should composite to, per channel —
/// the same straight-alpha-over formula `Color::over` implements, spelled out
/// so a test asserting against it is not just re-running the code under test.
fn expected_over(over: u8, under: u8, alpha: u8) -> u8 {
    let a = f32::from(alpha) / 255.0;
    let value = f32::from(over).mul_add(a, f32::from(under) * (1.0 - a));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a channel, 0..=255 by construction"
    )]
    let rounded = (value + 0.5) as u8;
    rounded
}

#[test]
fn solid_axis_aligned_fill_composites_exactly() {
    let mut scene = Scene::new();
    scene.fill_rect(
        Rect::new(20.0, 20.0, 200.0, 150.0),
        Color::rgba(220, 40, 40, 255).into(),
    );
    scene.fill_rect(
        Rect::new(60.0, 60.0, 260.0, 190.0),
        Color::rgba(40, 120, 220, 200).into(),
    );
    let (pixels, _) = render("solid_fill", &scene, 320, 240, Color::WHITE);

    // Well inside the first rect, above where the second overlaps it: opaque
    // red, untouched.
    let red_only = pixels.pixel(30, 30);
    assert_eq!(
        (red_only.r, red_only.g, red_only.b, red_only.a),
        (220, 40, 40, 255),
        "an opaque axis-aligned fill must land bit-exact, not just close"
    );

    // Where the translucent blue rect overlaps the red one: a straight-alpha
    // composite of blue-at-200 over red. Red covers x in [20,200), y in
    // [20,150); blue covers x in [60,260), y in [60,190) — (100,100) sits
    // well inside both, clear of the half-open edges either rect stops at.
    let overlap = pixels.pixel(100, 100);
    assert_eq!(
        (overlap.r, overlap.g, overlap.b, overlap.a),
        (
            expected_over(40, 220, 200),
            expected_over(120, 40, 200),
            expected_over(220, 40, 200),
            255,
        ),
        "a translucent fill over an opaque one must composite by the straight-alpha formula"
    );

    // Outside both rects: the white base, untouched.
    let base = pixels.pixel(300, 230);
    assert_eq!((base.r, base.g, base.b, base.a), (255, 255, 255, 255));
}

#[test]
fn linear_gradient_fill_varies_along_its_axis() {
    let mut scene = Scene::new();
    let gradient = Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0)).with_stops(&[
        (0.0, Color::rgba(255, 0, 0, 255)),
        (0.5, Color::rgba(0, 255, 0, 255)),
        (1.0, Color::rgba(0, 0, 255, 255)),
    ]);
    scene.fill_rect(
        Rect::new(10.0, 10.0, 300.0, 220.0),
        Paint::gradient(gradient),
    );
    let _ = GradientStop::new(0.0, Color::WHITE); // keep the import honest
    let (pixels, report) = render(
        "linear_gradient",
        &scene,
        320,
        240,
        Color::rgba(250, 250, 250, 255),
    );

    assert_eq!(report.shapes, 1);
    // Near the start stop (red) and near the end stop (blue) must actually
    // differ — a gradient that silently fell back to a flat fill would not.
    let near_start = pixels.pixel(20, 20);
    let near_end = pixels.pixel(295, 215);
    assert!(
        near_start.r > near_end.r && near_end.b > near_start.b,
        "the gradient must move from red toward blue along its axis: \
         start={near_start:?} end={near_end:?}"
    );
}

#[test]
fn stroked_rounded_path_inks_its_outline() {
    let mut scene = Scene::new();
    let mut path = Path::new();
    path.move_to(Offset::new(40.0, 40.0));
    path.line_to(Offset::new(260.0, 40.0));
    path.cubic_to(
        Offset::new(300.0, 40.0),
        Offset::new(300.0, 80.0),
        Offset::new(260.0, 120.0),
    );
    path.line_to(Offset::new(60.0, 160.0));
    path.close();
    let style = StrokeStyle::default()
        .cap(StrokeCap::Round)
        .join(StrokeJoin::Round);
    scene.stroke_path(
        &path,
        Stroke::new(10.0).styled(style),
        Color::rgba(20, 20, 20, 255).into(),
    );
    let (pixels, _) = render("stroked_path", &scene, 320, 240, Color::WHITE);

    // On the top edge of the path, the stroke must be dark ink; far from any
    // edge, the page must still be white.
    let on_stroke = pixels.pixel(150, 40);
    assert!(
        on_stroke.r < 100,
        "the stroke must ink its own path: {on_stroke:?}"
    );
    let untouched = pixels.pixel(10, 220);
    assert_eq!((untouched.r, untouched.g, untouched.b), (255, 255, 255));
}

#[test]
fn drop_shadow_darkens_beneath_the_offset() {
    let mut scene = Scene::new();
    scene.draw_shadow(
        Rect::new(80.0, 60.0, 240.0, 160.0),
        16.0,
        Shadow::new(Color::rgba(0, 0, 0, 160), Offset::new(6.0, 10.0), 24.0),
    );
    scene.fill_rect(
        Rect::new(80.0, 60.0, 240.0, 160.0),
        Color::rgba(250, 250, 245, 255).into(),
    );
    let (pixels, _) = render(
        "drop_shadow",
        &scene,
        320,
        240,
        Color::rgba(235, 235, 235, 255),
    );

    // Below-right of the rect, in the shadow's offset direction, must be
    // darker than the flat grey page far from any shape.
    let in_shadow = pixels.pixel(160, 172);
    let page = pixels.pixel(300, 20);
    assert!(
        in_shadow.r < page.r,
        "the shadow must darken the page beneath its offset: shadow={in_shadow:?} page={page:?}"
    );
    // And the rect itself must still read as its own near-white fill, not
    // swallowed by the shadow it casts.
    let on_rect = pixels.pixel(160, 110);
    assert!(
        on_rect.r > 200,
        "the rect's own fill must stay light: {on_rect:?}"
    );
}

#[test]
fn straight_alpha_image_renders_its_checker() {
    let mut scene = Scene::new();
    let mut pixels_in = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32u32 {
        for x in 0..32u32 {
            let checker = ((x / 4) + (y / 4)) % 2 == 0;
            if checker {
                pixels_in.extend_from_slice(&[230, 90, 20, 255]);
            } else {
                pixels_in.extend_from_slice(&[20, 90, 230, 140]);
            }
        }
    }
    let image = Image::from_rgba8(pixels_in, 32, 32);
    scene.draw_image(Rect::new(40.0, 40.0, 280.0, 200.0), &image);
    let (pixels, report) = render(
        "image_checker",
        &scene,
        320,
        240,
        Color::rgba(245, 245, 245, 255),
    );

    assert_eq!(report.images, 1);
    // The opaque orange squares and the translucent blue-over-page squares
    // must read as visibly different colours — a sampler that ignored alpha
    // would make every square the same flat colour.
    //
    // The image scales from 32×32 source pixels to a 240×160 destination
    // (7.5x horizontally, 5x vertically), so each 4×4-source-pixel checker
    // cell lands on a 30×20-destination-pixel cell. Sampling each cell's
    // centre — rather than an edge — keeps this clear of interpolation
    // blending between neighbouring cells: cell (0,0) is checker-true
    // (opaque orange) and centres on dest-local (15,10); cell (1,0) is
    // checker-false (translucent blue) and centres on dest-local (45,10).
    // The rect starts at (40,40), so that is (55,50) and (85,50).
    let opaque_square = pixels.pixel(55, 50);
    let translucent_square = pixels.pixel(85, 50);
    assert!(
        opaque_square.r > translucent_square.r,
        "opaque orange and page-blended translucent blue must differ: \
         opaque={opaque_square:?} translucent={translucent_square:?}"
    );
}

#[test]
fn glyph_run_inks_the_page() {
    use vieww_text::{FontStore, Paragraph, TextSpan};

    let mut fonts = FontStore::embedded_only();
    let spans = [TextSpan::new("vieww", TextStyle::new(48.0))];
    let mut run = Paragraph::layout(&mut fonts, &spans, f32::INFINITY)
        .runs()
        .first()
        .cloned()
        .expect("a shaped run");
    run.color = Color::rgba(20, 20, 20, 255);
    run.origin = Offset::new(20.0, 70.0);

    let mut scene = Scene::new();
    scene.draw_glyphs(&run);
    let (pixels, report) = render("glyph_run", &scene, 320, 120, Color::WHITE);

    assert!(report.glyphs > 0, "the run must have reached the renderer");
    let inked = (0..pixels.height())
        .flat_map(|y| (0..pixels.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| pixels.pixel(x, y).r < 200)
        .count();
    assert!(
        inked > 50,
        "shaped text must ink a real number of pixels: {inked}"
    );
}

/// The concrete non-separable/isolation case (spec §7.2, §1.1 audit finding
/// #3): `SrcIn` must mask the group's own content against what was already
/// underneath it, not paint over that backdrop unconditionally — a renderer
/// that substituted `SrcIn` with `Normal` could not tell the difference on an
/// opaque backdrop, so this fixture puts a *shaped, partially transparent*
/// backdrop under the group specifically so the two policies would disagree
/// if this renderer got it wrong.
#[test]
fn isolated_blend_mode_masks_against_its_own_backdrop() {
    let mut scene = Scene::new();
    // A marker in the corner, far from the blended group — proof nothing
    // leaks outside the group's own bounds.
    scene.fill_rect(
        Rect::new(4.0, 4.0, 24.0, 24.0),
        Color::rgba(10, 200, 10, 255).into(),
    );

    // The backdrop the group will be masked against: a diamond, not the full
    // bounds rect, so `SrcIn`'s mask is visibly narrower than an unconditional
    // fill would be.
    let mut diamond = Path::new();
    diamond.move_to(Offset::new(160.0, 90.0));
    diamond.line_to(Offset::new(220.0, 140.0));
    diamond.line_to(Offset::new(160.0, 190.0));
    diamond.line_to(Offset::new(100.0, 140.0));
    diamond.close();
    scene.fill_path(&diamond, Color::rgba(40, 40, 220, 255).into());

    scene.save();
    scene.push_layer(Rect::new(90.0, 80.0, 230.0, 200.0), 1.0, BlendMode::SrcIn);
    scene.fill_rect(
        Rect::new(90.0, 80.0, 230.0, 200.0),
        Color::rgba(230, 30, 30, 255).into(),
    );
    scene.pop_layer();
    scene.restore();

    // A transparent base, deliberately: Porter-Duff `SrcIn` masks against the
    // *destination's alpha*, not its colour, so the backdrop needs a
    // genuinely transparent surround (not just a differently-coloured one)
    // for `SrcIn` to mean anything here — with an opaque base under the
    // diamond, every policy gives the same alpha (1.0) everywhere.
    let (pixels, _) = render("isolated_blend_srcin", &scene, 320, 240, Color::TRANSPARENT);

    let marker = pixels.pixel(12, 12);
    assert!(
        marker.g > 150 && marker.r < 100,
        "the corner marker must be untouched by the blended group: {marker:?}"
    );
    let inside_diamond = pixels.pixel(160, 140);
    assert!(
        inside_diamond.r > 150,
        "the diamond's centre should be masked red: {inside_diamond:?}"
    );
    let corner_of_layer_bounds_outside_diamond = pixels.pixel(95, 85);
    assert!(
        corner_of_layer_bounds_outside_diamond.r < 100,
        "SrcIn must not paint red where the diamond backdrop was not, but got {corner_of_layer_bounds_outside_diamond:?}"
    );
}
