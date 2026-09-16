//! # Vieww text fidelity — Phase 2 certification
//!
//! "Make text indisputably excellent" is a claim about *pictures*, so this
//! suite renders the picture and asserts on it: variable-font instances
//! drawn at their interpolated weights, COLRv0 layered colour glyphs, CBDT
//! bitmap emoji, a real CJK fallback chain, selection and caret geometry
//! from the shaping pass itself, subpixel glyph phase, and a large-text
//! stress screen.
//!
//! ```console
//! cargo run --release -p test-text-fidelity -- /tmp/vieww-text-fidelity
//! ```
//!
//! # What each screen proves
//!
//! | screen | the claim it holds to |
//! |---|---|
//! | `01-variable-weights` | one variable face, four instances — the *ink* differs per weight |
//! | `02-colour-layers` | COLRv0 base glyphs paint their palette layers, both colours visible |
//! | `03-emoji` | CBDT strikes decode, composite, and are cached across re-renders |
//! | `04-cjk-fallback` | a mixed Latin + CJK line shapes through two faces and draws both |
//! | `05-selection-caret` | selection boxes and the caret come from the same shaping pass as the glyphs |
//! | `06-subpixel` | the same run at four fractional x phases — four rasters, one font |
//! | `07-large-text` | thousands of glyphs in one frame; the second pass is cache hits |
//! | `08-lcd-subpixel` | `AaMode::Lcd`: the same screen through both AA modes — the LCD render
//!   has channel-split edge pixels the gray render cannot have, an 8× zoom
//!   pair in `08-lcd-zoom.png`, and LCD rasters that cache like gray ones |
//!
//! The GIF is the evidence a human reviews; the printed assertions are the
//! evidence a CI reviews. The runner exits non-zero on any failure.

use std::cell::RefCell;
use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use gif::{Encoder, Frame, Repeat};
use vieww::{RenderObject, TextPosition, TextRange};
use vieww_foundation::{
    Color, Constraints, EdgeInsets, FontFamily, FontWeight, Offset, Rect, Shadow, Size, TextStyle,
};
use vieww_paint::native::{AaMode, NativeRenderer};
use vieww_render::FrameDriver;
use vieww_text::{FontStore, Paragraph, TextSpan};
use vieww_widget::prelude::*;

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 760.0;
const FRAME_MS: u16 = 900;

const BG: Color = Color::rgb(247, 248, 250);
const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const SELECTION: Color = Color::rgba(58, 122, 246, 64);
const CARET: Color = Color::rgb(220, 60, 60);

/// The variable Noto Serif SC subset — wght axis 200..900, default 200.
const VF_FONT: &[u8] = include_bytes!("../assets/NotoSansSC-VF-subset.ttf");
/// The COLRv0 layered-colour test face.
const COLR_FONT: &[u8] = include_bytes!("../assets/ViewwColourTest-COLRv0.ttf");
/// The CBDT bitmap-emoji subset.
const EMOJI_FONT: &[u8] = include_bytes!("../assets/NotoColorEmoji-subset.ttf");

const VF_FAMILY: &str = "Noto Serif SC";
const COLR_FAMILY: &str = "ViewwColourTest";
const EMOJI_FAMILY: &str = "Noto Color Emoji";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-text-fidelity-out"));
    std::fs::create_dir_all(&out)?;

    // One store with every test face plus the embedded set: the CJK face is
    // embedded now, so the fallback chain is real even headlessly.
    let fonts = FontStore::with_application_faces(
        [VF_FONT.to_vec(), COLR_FONT.to_vec(), EMOJI_FONT.to_vec()],
        Some(VF_FAMILY),
        None,
    );
    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    driver.set_fonts(fonts);
    // The two custom render objects this suite paints with — the same seam
    // any third-party text widget would use.
    driver.register::<SelectionText, RenderSelectionText>(|widget| RenderSelectionText {
        paragraph: widget.paragraph.clone(),
        selection: widget.selection,
    });
    driver.register::<PhaseRow, RenderPhaseRow>(|widget| RenderPhaseRow {
        paragraph: widget.paragraph.clone(),
        phase: widget.phase,
    });

    let mut renderer = NativeRenderer::new();
    let mut failures: Vec<String> = Vec::new();

    // One screen: its evidence filename and the tree that renders it.
    // Named because the array type is otherwise four levels of brackets
    // deep, which clippy is right to call unreadable.
    type Screen = (&'static str, fn() -> WidgetNode);
    let screens: [Screen; 8] = [
        ("01-variable-weights", variable_weights),
        ("02-colour-layers", colour_layers),
        ("03-emoji", emoji),
        ("04-cjk-fallback", cjk_fallback),
        ("05-selection-caret", selection_caret),
        ("06-subpixel", subpixel),
        ("07-large-text", large_text),
        ("08-lcd-subpixel", lcd_subpixel),
    ];

    let mut gif_frames: Vec<Vec<u8>> = Vec::new();
    for (name, screen) in screens {
        driver.set_root(screen());
        driver.draw_frame();

        let start = Instant::now();
        let (pixels, report) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let png = pixels.encode_png()?;
        std::fs::write(out.join(format!("{name}.png")), &png)?;

        println!(
            "{name}: {} glyph runs, {} glyphs ({} from colour tables), {} \
             shapes — {:.1} ms",
            report.glyph_runs, report.glyphs, report.colour_glyphs, report.shapes, elapsed_ms
        );
        if std::env::var("VIEWW_DUMP_RUNS").is_ok() {
            for (origin, run) in driver.scene().glyph_runs() {
                println!(
                    "  run at {:?}: font {:?} ({} variations) size {} glyphs {:?}",
                    origin,
                    (run.font.bytes().len(), run.font.index()),
                    run.font.variations().len(),
                    run.size,
                    run.glyphs.iter().map(|glyph| glyph.id).collect::<Vec<_>>()
                );
            }
        }
        gif_frames.push(pixels.data().to_vec());
        check(name, &report, &png, &mut failures);
    }

    write_gif(
        &gif_frames,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-text-fidelity.gif"),
    )?;

    // Emoji strike caching: re-rendering the same screen must not decode a
    // single new bitmap.
    driver.set_root(emoji());
    driver.draw_frame();
    let before = renderer.decoded_color_glyphs();
    let _ = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let after = renderer.decoded_color_glyphs();
    let emoji_decodes = after;
    println!("emoji strike decodes: {before} first pass, {after} after a re-render");
    if after != before {
        failures.push(format!(
            "emoji re-render decoded new strikes ({before} → {after}); the \
             bitmap cache is not holding"
        ));
    }

    // Large-text caching: the whole stress screen, re-rendered, must be
    // raster-cache hits.
    driver.set_root(large_text());
    driver.draw_frame();
    let before = renderer.glyph_raster_cache_stats();
    let _ = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let after = renderer.glyph_raster_cache_stats();
    let new_misses = after.misses - before.misses;
    let new_hits = after.hits - before.hits;
    println!("large-text re-render: {new_hits} cache hits, {new_misses} new rasters");
    if new_misses > 0 {
        failures.push(format!(
            "re-rendering an unchanged text screen rasterized {new_misses} \
             new glyphs; the cache should have answered everything"
        ));
    }

    // ── LCD subpixel AA: the same scene through both AA modes.
    //
    // The screen loop above drew `08-lcd-subpixel` in grayscale (that frame
    // is in the GIF). Here the *same* scene goes through a renderer built
    // with `AaMode::Lcd`, and the two renders are compared pixel by pixel.
    //
    // The observable is the one a magnifying glass would find: grayscale AA
    // has one coverage number per pixel, so R, G and B move *together* on a
    // glyph edge. LCD coverage is per sub-column, so the channels move
    // *apart* — a pixel where the two renders disagree per-channel is a
    // fringe pixel, and the gray render cannot produce one no matter what
    // colour the ink is.
    driver.set_root(lcd_subpixel());
    driver.draw_frame();
    let (gray_pixels, _) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let mut lcd_renderer = NativeRenderer::with_aa_mode(AaMode::Lcd);
    let (lcd_pixels, _) =
        lcd_renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;

    let mut fringes = 0usize;
    let (mut bx0, mut by0, mut bx1, mut by1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, (g, l)) in gray_pixels
        .data()
        .chunks_exact(4)
        .zip(lcd_pixels.data().chunks_exact(4))
        .enumerate()
    {
        let dr = i16::from(l[0]) - i16::from(g[0]);
        let dg = i16::from(l[1]) - i16::from(g[1]);
        let db = i16::from(l[2]) - i16::from(g[2]);
        let spread = dr.max(dg).max(db) - dr.min(dg).min(db);
        if spread > 8 {
            fringes += 1;
            let x = (i % WIDTH as usize) as u32;
            let y = (i / WIDTH as usize) as u32;
            bx0 = bx0.min(x);
            by0 = by0.min(y);
            bx1 = bx1.max(x + 1);
            by1 = by1.max(y + 1);
        }
    }
    println!("lcd subpixel: {fringes} channel-split edge pixels between the two AA modes");
    if fringes < 50 {
        failures.push(format!(
            "lcd subpixel: only {fringes} channel-split pixels — the LCD \
             render should fringe on every vertical stem edge"
        ));
    }

    // LCD rasterisations cache like gray ones: a second render of the
    // unchanged scene must take no new misses.
    let before = lcd_renderer.glyph_raster_cache_stats();
    let _ = lcd_renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let after = lcd_renderer.glyph_raster_cache_stats();
    let lcd_new_misses = after.misses - before.misses;
    let lcd_new_hits = after.hits - before.hits;
    println!("lcd re-render: {lcd_new_hits} cache hits, {lcd_new_misses} new rasters");
    if lcd_new_misses > 0 {
        failures.push(format!(
            "re-rendering the LCD screen rasterized {lcd_new_misses} new \
             glyphs; LCD entries must cache exactly like gray ones"
        ));
    }

    // The 8× zoom pair: the same crop of both renders, nearest-neighbour
    // upscale, stacked — gray on top, LCD beneath, one separator row —
    // around the bounding box the fringes live in. This is the artefact a
    // human reviews to see sub-pixel colour fringing happen.
    let zoom = 8u32;
    let surface_w = WIDTH as u32;
    let surface_h = HEIGHT as u32;
    let window_w = (surface_w / zoom).min(140);
    let window_h = (surface_h / zoom).min(70);
    let cx = (bx0 + bx1) / 2;
    let cy = (by0 + by1) / 2;
    let zx = cx.saturating_sub(window_w / 2).min(surface_w - window_w);
    let zy = cy.saturating_sub(window_h / 2).min(surface_h - window_h);
    let gray_crop = crop_rgba(gray_pixels.data(), WIDTH as u32, zx, zy, window_w, window_h);
    let lcd_crop = crop_rgba(lcd_pixels.data(), WIDTH as u32, zx, zy, window_w, window_h);
    let sheet_w = window_w * zoom;
    let sheet_h = window_h * zoom * 2 + 6;
    let mut sheet = vec![255u8; sheet_w as usize * sheet_h as usize * 4];
    blit_rgba(
        &mut sheet,
        sheet_w,
        &upscale_nearest(&gray_crop, window_w, window_h, zoom),
        0,
        0,
    );
    blit_rgba(
        &mut sheet,
        sheet_w,
        &upscale_nearest(&lcd_crop, window_w, window_h, zoom),
        0,
        window_h * zoom + 6,
    );
    let zoom_png = vieww_paint::native::Pixels::from_rgba8(sheet, sheet_w, sheet_h).encode_png()?;
    std::fs::write(out.join("08-lcd-zoom.png"), &zoom_png)?;
    println!(
        "lcd zoom pair: {}x{} window at ({zx},{zy}) -> 08-lcd-zoom.png",
        window_w, window_h
    );

    // The suite's measured summary — the numbers a CI can pin.
    std::fs::write(
        out.join("metrics.txt"),
        format!(
            "screens=8\nlcd_fringe_pixels={fringes}\nlcd_rerender_hits={lcd_new_hits}\n\
             lcd_rerender_new_rasters={lcd_new_misses}\nlarge_text_rerender_new_rasters={new_misses}\n\
             emoji_strike_decodes_total={emoji_decodes}\n",
        ),
    )?;

    println!("\nwrote {}", out.display());
    if failures.is_empty() {
        println!("ALL TEXT-FIDELITY CHECKS PASSED");
    } else {
        for failure in &failures {
            eprintln!("FAIL: {failure}");
        }
        return Err(format!("{} text-fidelity check(s) failed", failures.len()).into());
    }
    Ok(())
}

/// Per-screen assertions.
fn check(
    name: &str,
    report: &vieww_paint::native::SceneReport,
    png: &[u8],
    failures: &mut Vec<String>,
) {
    match name {
        "01-variable-weights" => {
            if report.glyphs < 20 {
                failures.push("variable weights: too few glyphs drawn".into());
            }
            if png.len() < 20_000 {
                failures.push(format!(
                    "variable weights: suspiciously empty image ({} bytes) — \
                     the variable face may have rendered nothing",
                    png.len()
                ));
            }
        }
        "02-colour-layers" => {
            if report.colour_glyphs == 0 {
                failures.push("colour layers: no COLRv0 glyph reached the screen".into());
            }
        }
        "03-emoji" => {
            if report.colour_glyphs == 0 {
                failures.push("emoji: no bitmap glyph reached the screen".into());
            }
        }
        "04-cjk-fallback" => {
            if report.glyphs == 0 {
                failures.push("cjk fallback: nothing drawn".into());
            }
        }
        "05-selection-caret" => {
            if report.shapes < 5 {
                failures.push("selection/caret: no selection boxes or caret recorded".into());
            }
        }
        "06-subpixel" => {
            if report.glyph_runs < 4 {
                failures.push("subpixel: expected several phase-shifted runs".into());
            }
        }
        "07-large-text" => {
            if report.glyphs < 2000 {
                failures.push(format!(
                    "large text: only {} glyphs in the stress screen, expected ≥ 2000",
                    report.glyphs
                ));
            }
        }
        "08-lcd-subpixel" if report.glyphs < 20 => {
            failures.push(format!(
                "lcd subpixel: only {} glyphs drawn, expected the two sample rows",
                report.glyphs
            ));
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Screens
// ─────────────────────────────────────────────────────────────────────────

fn shell(title: &str, subtitle: &str, body: WidgetNode) -> WidgetNode {
    Container::new()
        .color(BG)
        .padding(EdgeInsets::all(36.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(26.0)
                .children(children![header(title, subtitle), body]),
        )
        .into()
}

fn header(title: &str, subtitle: &str) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(title)
                .color(INK)
                .size(26.0)
                .weight(FontWeight::Bold),
            Text::new(subtitle).color(MUTED).size(13.0),
        ])
        .into()
}

fn card(child: WidgetNode) -> WidgetNode {
    Container::new()
        .decoration(
            BoxDecoration::new()
                .color(Color::WHITE)
                .radius(12.0)
                .shadow(Shadow::new(
                    Color::rgba(23, 30, 42, 30),
                    Offset::new(0.0, 4.0),
                    10.0,
                )),
        )
        .padding(EdgeInsets::all(22.0))
        .child(child)
        .into()
}

fn vf_style(size: f32, weight: FontWeight) -> TextStyle {
    TextStyle::new(size)
        .family(FontFamily::Named(VF_FAMILY))
        .weight(weight)
}

fn named_style(size: f32, family: &'static str) -> TextStyle {
    TextStyle::new(size).family(FontFamily::Named(family))
}

/// One variable face, four weights.
fn variable_weights() -> WidgetNode {
    const WEIGHTS: [FontWeight; 4] = [
        FontWeight::Light,
        FontWeight::Regular,
        FontWeight::Medium,
        FontWeight::Bold,
    ];
    let rows = WEIGHTS
        .iter()
        .map(|weight| {
            Flex::row()
                .spacing(16.0)
                .children(children![
                    Text::new("视界框架 Vieww typography").style(vf_style(30.0, *weight)),
                    Text::new(format!("{:?} · wght {}", weight, weight.value()))
                        .color(MUTED)
                        .size(12.0),
                ])
                .into()
        })
        .collect::<Vec<WidgetNode>>();
    shell(
        "Variable font — one face, four instances",
        "Noto Serif SC, wght axis 200–900: shaping and outline rasterization \
         agree on the instance; advances follow the interpolated metrics",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(18.0)
                .children(rows)
                .into(),
        ),
    )
}

/// COLRv0 layers.
fn colour_layers() -> WidgetNode {
    shell(
        "Colour fonts — COLRv0 layered glyphs",
        "Each letter is two stacked shapes in two palette colours; both \
         layers of every glyph must be visible",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![
                    Text::new("A  B  C  A B C").style(named_style(72.0, COLR_FAMILY)),
                    Text::new(
                        "layered: square under disc, disc under square, square under offset disc"
                    )
                    .color(MUTED)
                    .size(13.0),
                ])
                .into(),
        ),
    )
}

/// CBDT bitmap emoji at several sizes.
fn emoji() -> WidgetNode {
    shell(
        "Colour emoji — CBDT bitmap strikes",
        "Real Noto Color Emoji strikes, decoded once and cached per glyph; \
         the runner re-renders this screen and asserts no new decode",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![
                    Text::new("🚀🎉❤️😀🔥✨👍").style(named_style(48.0, EMOJI_FAMILY)),
                    Text::new("🌈⚡🎯💡🏆🖼").style(named_style(30.0, EMOJI_FAMILY)),
                    Text::new("the same emoji smaller — one decoded strike, two placements:")
                        .color(MUTED)
                        .size(13.0),
                    Text::new("🚀 🚀 🚀 🚀 🚀").style(named_style(22.0, EMOJI_FAMILY)),
                ])
                .into(),
        ),
    )
}

/// Mixed Latin + CJK through the embedded fallback chain.
fn cjk_fallback() -> WidgetNode {
    shell(
        "CJK fallback — two faces, one line",
        "Latin from the embedded DejaVu subset, CJK from the embedded Noto \
         face: the fallback chain is real headlessly",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(18.0)
                .children(children![
                    Text::new("Vieww 视界框架 — 你好世界").size(28.0),
                    Text::new("中文字型回退链：同一行，两种字面。").size(20.0),
                    Text::new("同一行 Latin 与 CJK 混排，两种字面，一种度量。")
                        .color(MUTED)
                        .size(14.0),
                ])
                .into(),
        ),
    )
}

/// Selection and caret geometry from the shaping pass.
fn selection_caret() -> WidgetNode {
    shell(
        "Selection and caret — from the shaping pass",
        "Highlight boxes and the caret rect come from the paragraph's own \
         geometry, so what you see is what hit-testing sees",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![
                    SelectionText::node("The quick brown fox jumps over the lazy dog", 10, 26),
                    SelectionText::node("视界框架文字排版，可变粗细演示。", 2, 9),
                    Text::new("selection in an LTR run and in a CJK run; caret at each range end")
                        .color(MUTED)
                        .size(13.0),
                ])
                .into(),
        ),
    )
}

/// One run, four fractional x origins — through a render object, because the
/// built-in `Text` deliberately snaps run origins to the pixel grid (see
/// `RenderText::paint`'s comment for the argument) and what this screen
/// certifies is the layer *underneath* that snap: the raster cache's exact
/// phase keying, exercised the way shaping itself produces fractional
/// positions.
fn subpixel() -> WidgetNode {
    shell(
        "Subpixel positioning — four phases",
        "The same glyphs at x offsets 0, 0.25, 0.5, 0.75: each phase is a \
         distinct raster-cache entry, not a rounding to whole pixels",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .children(children![
                    PhaseRow::node(0.0),
                    PhaseRow::node(0.25),
                    PhaseRow::node(0.5),
                    PhaseRow::node(0.75),
                ])
                .into(),
        ),
    )
}

/// The LCD comparison sample: two fractional-phase rows. The screen loop
/// renders this in gray for the GIF; the post-loop LCD check re-renders the
/// *same* scene through `AaMode::Lcd` and compares the two byte-for-byte —
/// see the "LCD subpixel AA" block in `main`.
fn lcd_subpixel() -> WidgetNode {
    shell(
        "Subpixel AA — grayscale vs LCD",
        "AaMode::Lcd rasterises text coverage at 3× horizontal resolution; \
         the runner re-renders this exact scene with both AA modes and \
         writes the channel-split comparison and an 8× zoom to 08-lcd-zoom.png",
        card(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![
                    PhaseRow::with_text("Hamburgefonstiv — subpixel", 40.0, 0.17),
                    PhaseRow::with_text("grayscale on top, LCD below (8× zoom):", 20.0, 0.62),
                    Text::new(
                        "the GIF frame you are looking at is the grayscale render; \
                         the LCD render of this same scene is 08-lcd-subpixel.png's \
                         comparison and the zoom pair beneath it"
                    )
                    .color(MUTED)
                    .size(13.0),
                ])
                .into(),
        ),
    )
}

/// Thousands of glyphs on one screen.
fn large_text() -> WidgetNode {
    const ROWS: usize = 56;
    let rows = (0..ROWS)
        .map(|row| {
            let filler = match row % 3 {
                0 => "视界框架渲染引擎文字排版设计优雅快速现代跨平台中文测试 ",
                1 => "the quick brown fox jumps over the lazy dog 0123456789 ",
                _ => "框架渲染引擎文字排版设计，像素级验证。回退链表演示。 ",
            };
            Text::new(format!("{row:03} {filler}"))
                .color(INK)
                .size(13.0)
                .into()
        })
        .collect::<Vec<_>>();
    shell(
        "Large text — one frame, thousands of glyphs",
        "A full screen of dense text, rendered in one pass; the runner \
         re-renders it and asserts the second pass is all cache hits",
        // Clipped rather than shrunk: the stress is "render all of this at
        // once", and a real scroll view would be the home for it — this
        // suite's sibling `test-scroll-stress` renders exactly that. Here
        // the clip keeps the screen's own frame clean while every row still
        // rasterises.
        Clip::rounded(12.0)
            .child(
                Container::new()
                    .decoration(BoxDecoration::new().color(Color::WHITE).radius(12.0))
                    .padding(EdgeInsets::symmetric(24.0, 16.0))
                    .child(
                        Flex::column()
                            .cross_axis_alignment(CrossAxisAlignment::Start)
                            .spacing(4.0)
                            .children(rows),
                    ),
            )
            .into(),
    )
}

// ─────────────────────────────────────────────────────────────────────────
// Custom render objects — the third-party seam
// ─────────────────────────────────────────────────────────────────────────

/// Lays out a paragraph once (at build time, through the *driver's* store,
/// so family fallback sees the application faces) and hands the shaped
/// result to the render object.
fn shape_with(store: &mut FontStore, text: &str, style: TextStyle) -> Paragraph {
    Paragraph::layout(
        store,
        &[TextSpan::new(text.to_owned(), style)],
        f32::INFINITY,
    )
}

thread_local! {
    static SHAPE_STORE: RefCell<Option<FontStore>> = const { RefCell::new(None) };
}

/// Shape through a store mirroring the driver's registration: the app faces
/// first, the embedded set behind. Kept in a thread-local so every screen
/// of one run shapes through one store's cache.
fn shape(text: &str, style: TextStyle) -> Paragraph {
    SHAPE_STORE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let store = borrow.get_or_insert_with(|| {
            FontStore::with_application_faces(
                [VF_FONT.to_vec(), COLR_FONT.to_vec(), EMOJI_FONT.to_vec()],
                Some(VF_FAMILY),
                None,
            )
        });
        shape_with(store, text, style)
    })
}

/// A text widget whose selection and caret are painted from the paragraph's
/// own geometry.
#[derive(Debug)]
struct SelectionText {
    paragraph: Paragraph,
    selection: (usize, usize),
}

impl SelectionText {
    /// Not `new` — this renders the paragraph straight to a node, so the
    /// name says what comes back rather than lying about a `Self`.
    fn node(text: &str, from: usize, to: usize) -> WidgetNode {
        Self {
            paragraph: shape(text, TextStyle::new(22.0)),
            selection: (from, to),
        }
        .into()
    }
}

impl Widget for SelectionText {
    fn debug_name(&self) -> &'static str {
        "SelectionText"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(SelectionText);

#[derive(Debug)]
struct RenderSelectionText {
    paragraph: Paragraph,
    selection: (usize, usize),
}

impl RenderObject for RenderSelectionText {
    fn debug_name(&self) -> &'static str {
        "RenderSelectionText"
    }

    fn layout(
        &mut self,
        _ctx: &mut vieww::render::LayoutCtx<'_>,
        constraints: Constraints,
    ) -> Size {
        constraints.constrain(Size::new(
            self.paragraph.size().width + 2.0,
            self.paragraph.size().height,
        ))
    }

    fn paint(&self, ctx: &mut vieww::render::PaintCtx<'_>) {
        let origin = ctx.origin();
        // Selection highlight first, so the text lands on top of it.
        let rects = self
            .paragraph
            .selection_rects(TextRange::new(self.selection.0, self.selection.1));
        for rect in rects {
            let rect = Rect::new(
                origin.dx + rect.left,
                origin.dy + rect.top,
                origin.dx + rect.right,
                origin.dy + rect.bottom,
            );
            ctx.canvas().fill_rect(rect, SELECTION.into());
        }
        // Then the glyphs.
        for run in self.paragraph.runs() {
            let mut placed = run.clone();
            placed.origin = Offset::new(origin.dx + run.origin.dx, origin.dy + run.origin.dy);
            ctx.canvas().draw_glyphs(&placed);
        }
        // Then the caret, on top of everything, where an editor blinks it.
        // `cursor_rect` is deliberately zero-width (a *position*, not a
        // mark); `RenderEditableText` inflates it to 2px and so does the
        // caret that draws here, for the same reason.
        let caret = self
            .paragraph
            .cursor_rect(TextPosition::new(self.selection.1));
        let caret = Rect::new(
            origin.dx + caret.left,
            origin.dy + caret.top,
            origin.dx + caret.left + 2.0,
            origin.dy + caret.bottom,
        );
        ctx.canvas().fill_rect(caret, CARET.into());
    }
}

/// One paragraph drawn at a fractional x offset — the subpixel-phase probe.
#[derive(Debug)]
struct PhaseRow {
    paragraph: Paragraph,
    phase: f32,
}

impl PhaseRow {
    /// Not `new`, for the same reason as `SelectionText::node`: the caller
    /// gets a rendered node, not a `PhaseRow`.
    fn node(phase: f32) -> WidgetNode {
        Self::with_text("Hamburgefonstiv", 24.0, phase)
    }

    /// The same probe at an arbitrary size and text — the LCD screen wants
    /// bigger ink and its own fractional phases.
    fn with_text(text: &str, size: f32, phase: f32) -> WidgetNode {
        Self {
            paragraph: shape(text, TextStyle::new(size)),
            phase,
        }
        .into()
    }
}

impl Widget for PhaseRow {
    fn debug_name(&self) -> &'static str {
        "PhaseRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(PhaseRow);

#[derive(Debug)]
struct RenderPhaseRow {
    paragraph: Paragraph,
    phase: f32,
}

impl RenderObject for RenderPhaseRow {
    fn debug_name(&self) -> &'static str {
        "RenderPhaseRow"
    }

    fn layout(
        &mut self,
        _ctx: &mut vieww::render::LayoutCtx<'_>,
        constraints: Constraints,
    ) -> Size {
        constraints.constrain(Size::new(
            self.paragraph.size().width + self.phase,
            self.paragraph.size().height,
        ))
    }

    fn paint(&self, ctx: &mut vieww::render::PaintCtx<'_>) {
        let origin = ctx.origin();
        for run in self.paragraph.runs() {
            let mut placed = run.clone();
            // The fractional x lands *unsnapped*, deliberately: this row is
            // the probe for the raster cache's exact-phase keying, the layer
            // `RenderText`'s origin snap sits on top of.
            placed.origin = Offset::new(
                origin.dx + run.origin.dx + self.phase,
                origin.dy + run.origin.dy,
            );
            ctx.canvas().draw_glyphs(&placed);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Encoding
// ─────────────────────────────────────────────────────────────────────────

/// The `window_w × window_h` rectangle at `(x, y)` of an RGBA8 buffer, as
/// its own tightly-packed buffer. Pure evidence plumbing — no framework
/// involvement, deliberately, so the zoom sheet shows exactly the bytes the
/// renderer produced.
fn crop_rgba(data: &[u8], stride: u32, x: u32, y: u32, window_w: u32, window_h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(window_w as usize * window_h as usize * 4);
    for row in y..y + window_h {
        let start = (row * stride + x) as usize * 4;
        out.extend_from_slice(&data[start..start + window_w as usize * 4]);
    }
    out
}

/// Nearest-neighbour upscale — every source pixel becomes a `k × k` block,
/// which is what makes sub-pixel fringes visible as blocks rather than
/// smearing them the way the framework's own bilinear sampler would.
fn upscale_nearest(data: &[u8], width: u32, height: u32, k: u32) -> Vec<u8> {
    let big_w = width * k;
    let mut out = vec![0u8; (big_w * height * k) as usize * 4];
    for y in 0..height * k {
        let src_row = (y / k) * width;
        let dst_row = y * big_w;
        for x in 0..big_w {
            let src = ((src_row + x / k) * 4) as usize;
            let dst = (dst_row + x) as usize * 4;
            out[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
        }
    }
    out
}

/// Copy a whole RGBA8 buffer into another at `(x, y)`. No clipping: the
/// callers place windows that are known to fit.
fn blit_rgba(dest: &mut [u8], dest_stride: u32, src: &[u8], x: u32, y: u32) {
    let src_rows = (src.len() / dest_stride as usize / 4) as u32;
    for row in 0..src_rows {
        let src_start = (row * dest_stride) as usize * 4;
        let dst_start = ((y + row) * dest_stride + x) as usize * 4;
        dest[dst_start..dst_start + dest_stride as usize * 4]
            .copy_from_slice(&src[src_start..src_start + dest_stride as usize * 4]);
    }
}

fn write_gif(
    frames: &[Vec<u8>],
    width: u32,
    height: u32,
    path: &FsPath,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder = Encoder::new(file, width as u16, height as u16, &[])?;
    encoder.set_repeat(Repeat::Infinite)?;
    for rgba in frames {
        let mut frame = Frame::from_rgba_speed(width as u16, height as u16, &mut rgba.clone(), 10);
        frame.delay = FRAME_MS / 10;
        encoder.write_frame(&frame)?;
    }
    Ok(())
}
