//! The Phase 5 exit test: a paragraph with mixed styles, wrapped, on real pixels.
//!
//! ```console
//! cargo test -p vieww --features native --test text_to_pixels
//! ```
//!
//! # What this covers, and what it does not
//!
//! The roadmap asks for four things: a paragraph with mixed bold/italic runs
//! wrapping correctly at a width, a blinking cursor placed at a tapped position,
//! and an IME composition on-device.
//!
//! The first is met here end to end — shaped by `cosmic-text`, wrapped, laid out
//! by the render tree, rasterised by vieww's own rasterizer, and read back.
//! Bidirectional text is covered too, which the roadmap lists separately and
//! which is the part that would have been expensive to retrofit.
//!
//! Cursor placement and IME are **not** here. Both need a text *editing* model,
//! and IME additionally needs the platform bridges from Phase 8 — the roadmap
//! flags that dependency itself. Recorded as pending in `docs/ROADMAP.md` rather
//! than quietly counted as done.
//!
//! Vieww's own rasterizer needs no graphics adapter, so this runs unconditionally.

#![cfg(feature = "native")]

use vieww::foundation::{Color, Size, TextAlign, TextDirection, TextStyle};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;
use vieww::text::{FontStore, Paragraph, TextSpan};

const WIDTH: u32 = 260;
const HEIGHT: u32 = 160;

/// How many pixels are darker than the white background.
fn inked(pixels: &vieww::paint::native::Pixels) -> usize {
    (0..pixels.height())
        .flat_map(|y| (0..pixels.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| pixels.pixel(x, y).r < 200)
        .count()
}

/// The inked pixels' bounding box, as `(left, top, right, bottom)`.
fn ink_bounds(pixels: &vieww::paint::native::Pixels) -> (u32, u32, u32, u32) {
    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..pixels.height() {
        for x in 0..pixels.width() {
            if pixels.pixel(x, y).r < 200 {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
    }
    (l, t, r, b)
}

#[test]
fn exit_test_a_mixed_style_paragraph_wraps_and_reaches_pixels() {
    let mut renderer = NativeRenderer::new();

    // "plain bold italic" with each word in its own style, narrow enough to wrap.
    let mut fonts = FontStore::embedded_only();
    let spans = [
        TextSpan::new("Regular text then ", TextStyle::new(20.0)),
        TextSpan::new("bold words ", TextStyle::new(20.0).bold()),
        TextSpan::new("then italic ones", TextStyle::new(20.0).italic(true)),
    ];

    let unwrapped = Paragraph::layout(&mut fonts, &spans, f32::INFINITY);
    let wrapped = Paragraph::layout(&mut fonts, &spans, 240.0);

    assert_eq!(unwrapped.line_count(), 1, "unbounded fits on one line");
    assert!(
        wrapped.line_count() > 1,
        "240px must force a break: {} lines",
        wrapped.line_count()
    );
    assert!(
        wrapped.size().width <= 240.0,
        "wrapped text must fit the width it was given: {}",
        wrapped.size().width
    );
    assert!(
        wrapped.runs().len() >= 3,
        "three styles means at least three runs: {}",
        wrapped.runs().len()
    );

    // Draw it.
    let mut scene = vieww::Scene::new();
    for run in wrapped.runs() {
        let mut placed = run.clone();
        placed.origin = vieww::foundation::Offset::new(run.origin.dx + 6.0, run.origin.dy + 6.0);
        scene.draw_glyphs(&placed);
    }

    let (pixels, report) = renderer
        .render_to_pixels(&scene, WIDTH, HEIGHT, Color::WHITE)
        .expect("render");

    // A space shapes to a real, positioned glyph — `wrapped.glyph_count()`
    // counts it — but it carries no outline, so vieww's own rasterizer skips
    // drawing it rather than rasterising an empty mask. `report.glyphs`
    // therefore undercounts `glyph_count()` by exactly the shaped whitespace,
    // which is what this checks instead of exact equality.
    let expected_inked_glyphs: usize = spans
        .iter()
        .map(|span| span.text.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    assert_eq!(
        report.glyphs, expected_inked_glyphs,
        "every glyph with an outline reached the backend"
    );
    assert!(
        inked(&pixels) > 200,
        "text must ink pixels: {}",
        inked(&pixels)
    );

    let (left, top, right, bottom) = ink_bounds(&pixels);
    assert!(
        right <= 246,
        "ink stays within the wrap width: right={right}"
    );
    assert!(
        left >= 5,
        "and starts at the offset it was placed at: left={left}"
    );
    assert!(
        bottom - top > 20,
        "wrapped text occupies more than one line of pixels: {top}..{bottom}"
    );
}

#[test]
fn bold_inks_more_than_regular_at_the_same_size() {
    let mut renderer = NativeRenderer::new();

    let mut fonts = FontStore::embedded_only();
    let draw = |renderer: &mut NativeRenderer, style: TextStyle, fonts: &mut FontStore| {
        let spans = [TextSpan::new("Handgloves", style)];
        let paragraph = Paragraph::layout(fonts, &spans, f32::INFINITY);
        let mut scene = vieww::Scene::new();
        for run in paragraph.runs() {
            let mut placed = run.clone();
            placed.origin =
                vieww::foundation::Offset::new(run.origin.dx + 5.0, run.origin.dy + 5.0);
            scene.draw_glyphs(&placed);
        }
        let (pixels, _) = renderer
            .render_to_pixels(&scene, WIDTH, HEIGHT, Color::WHITE)
            .expect("render");
        inked(&pixels)
    };

    let regular = draw(&mut renderer, TextStyle::new(24.0), &mut fonts);
    let bold = draw(&mut renderer, TextStyle::new(24.0).bold(), &mut fonts);

    assert!(
        bold > regular,
        "a real bold face lays down more ink: {bold} vs {regular}"
    );
}

#[test]
fn right_to_left_text_inks_the_right_hand_side() {
    let mut renderer = NativeRenderer::new();

    // "shalom" in Hebrew, right-aligned in a wide surface. An RTL paragraph must
    // sit against the trailing edge, which is the visible consequence of getting
    // the base direction right.
    let mut fonts = FontStore::embedded_only();
    let spans = [TextSpan::new("שלום", TextStyle::new(28.0))];
    let paragraph = Paragraph::layout_aligned(
        &mut fonts,
        &spans,
        f32::from(u16::try_from(WIDTH).expect("fits")),
        TextAlign::Start,
        None,
    );

    assert_eq!(
        paragraph.direction(),
        TextDirection::Rtl,
        "Hebrew must be detected as RTL without being told"
    );

    let mut scene = vieww::Scene::new();
    for run in paragraph.runs() {
        let mut placed = run.clone();
        placed.origin = vieww::foundation::Offset::new(run.origin.dx, run.origin.dy + 8.0);
        scene.draw_glyphs(&placed);
    }

    let (pixels, report) = renderer
        .render_to_pixels(&scene, WIDTH, HEIGHT, Color::WHITE)
        .expect("render");

    assert!(report.glyphs > 0, "the embedded font must cover Hebrew");
    let (left, _, right, _) = ink_bounds(&pixels);
    assert!(
        left > WIDTH / 2,
        "RTL text with Start alignment sits on the right: ink spans {left}..{right}"
    );
}

#[test]
fn a_text_widget_reaches_pixels_through_the_whole_pipeline() {
    let mut renderer = NativeRenderer::new();

    // Widget -> element -> render -> paint -> pixels, with no manual shaping.
    let mut driver = FrameDriver::new(Size::new(
        f32::from(u16::try_from(WIDTH).expect("fits")),
        60.0,
    ));
    driver.elements().set_root(
        Container::new()
            .color(Color::WHITE)
            .padding(EdgeInsets::all(8.0))
            .child(Text::new("Hello, world").size(24.0).bold()),
    );
    driver.draw_frame();

    let runs = driver.scene().glyph_runs();
    assert!(!runs.is_empty(), "the Text widget must produce glyphs");

    let (pixels, report) = renderer
        .render_to_pixels(driver.scene(), WIDTH, 60, Color::WHITE)
        .expect("render");

    assert!(report.glyphs >= 10, "'Hello, world' is 12 characters");
    assert!(
        inked(&pixels) > 100,
        "and they must actually ink: {}",
        inked(&pixels)
    );

    let (left, top, ..) = ink_bounds(&pixels);
    assert!(left >= 7, "inside the 8px padding: left={left}");
    assert!(top >= 7, "and below it: top={top}");
}
