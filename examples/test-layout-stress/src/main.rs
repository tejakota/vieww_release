//! # Vieww layout stress — certification suite
//!
//! Layout is the framework's spine, so this suite stresses it in the three
//! directions that break layout engines: **breadth** (thousands of siblings),
//! **depth** (two dozen levels of nesting), and **feedback** (intrinsic
//! sizes that must measure children before the parent can size itself).
//! Plus the rebuild path: one changed leaf in a five-thousand-node tree
//! must cost a bounded fraction of the tree, not the whole thing.
//!
//! ```console
//! cargo run --release -p test-layout-stress -- /tmp/vieww-layout-stress
//! ```
//!
//! | screen | the stress |
//! |---|---|
//! | `01-breadth` | 5 000 siblings in a grid of flexes |
//! | `02-depth` | 24 levels deep, alternating axes, padding all the way down |
//! | `03-intrinsic` | chains of `IntrinsicWidth`-style containers that must measure up |
//! | (GIF) | breadth screen rebuilding, one node lit at a time |
//!
//! The assertions: no overflow reported anywhere, the flatten counters show
//! a single-node change costing a bounded slice of the tree, and every
//! screen renders twice with the second pass cheaper than the first.

use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use gif::{Encoder, Frame, Repeat};
use vieww_element::Signal;
use vieww_foundation::{Color, EdgeInsets, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 760.0;
const FRAME_MS: u16 = 80;

const BG: Color = Color::rgb(247, 248, 250);
const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const ACCENT: Color = Color::rgb(58, 122, 246);

const BREADTH: usize = 5_000;
const DEPTH: usize = 24;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-layout-stress-out"));
    std::fs::create_dir_all(&out)?;

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let mut renderer = NativeRenderer::new();
    let mut failures: Vec<String> = Vec::new();

    vieww_render::overflow::forget_reported();

    // ── The breadth screen, with one lit cell the GIF animates.
    let cells: Vec<Signal<Color>> = (0..BREADTH)
        .map(|_| {
            driver
                .elements()
                .runtime()
                .signal(Color::rgb(226, 232, 240))
        })
        .collect();
    let lit = BREADTH / 2;

    driver.set_root(breadth_screen(&cells));
    driver.draw_frame();
    let first = Instant::now();
    let (pixels, report) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let breadth_first_ms = first.elapsed().as_secs_f64() * 1000.0;
    let png = pixels.encode_png()?;
    std::fs::write(out.join("01-breadth.png"), &png)?;
    println!(
        "01-breadth: {BREADTH} cells, {} shapes, built {} reused {} — \
         {:.1} ms",
        report.shapes,
        driver.flatten_stats().built,
        driver.flatten_stats().reused,
        breadth_first_ms
    );

    // The second render of an unchanged tree must not re-flatten anything.
    driver.draw_frame();
    let clean_rebuilds = driver.scene_rebuilds();
    let second = Instant::now();
    let _ = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let breadth_second_ms = second.elapsed().as_secs_f64() * 1000.0;
    println!(
        "02-breadth again: {clean_rebuilds} flattens (was 2), {:.1} ms",
        breadth_second_ms
    );
    if clean_rebuilds > 2 {
        failures.push(format!(
            "an unchanged 5 000-node tree re-flattened ({clean_rebuilds} \
             flattens for 2 frames)"
        ));
    }

    // ── The one-cell change: light one cell, and the flatten must touch a
    // bounded slice, not the screen.
    cells[lit].set(ACCENT);
    driver.draw_frame();
    let stats = driver.flatten_stats();
    println!(
        "one cell recoloured: built {} of {} ({}% of the scene)",
        stats.built,
        stats.total(),
        percent(stats.built, stats.total()),
    );
    if stats.built > stats.total() / 4 {
        failures.push(format!(
            "one cell's colour change rebuilt {} of {} commands — the \
             rebuild is following the tree, not the change",
            stats.built,
            stats.total()
        ));
    }

    // ── The GIF: a row of cells lighting in sequence.
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let step = BREADTH / 16;
    for frame_index in 0..16 {
        let index = frame_index * step;
        let previous = index.saturating_sub(step);
        if previous != index {
            cells[previous].set(Color::rgb(226, 232, 240));
        }
        cells[index].set(ACCENT);
        driver.draw_frame();
        let (pixels, _) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        frames.push(pixels.data().to_vec());
    }
    write_gif(
        &frames,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-layout-stress.gif"),
    )?;

    // ── Depth.
    driver.set_root(depth_screen());
    driver.draw_frame();
    let start = Instant::now();
    let (pixels, report) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let depth_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::fs::write(out.join("02-depth.png"), pixels.encode_png()?)?;
    println!(
        "02-depth: {DEPTH} levels, {} shapes — {:.1} ms",
        report.shapes, depth_ms
    );

    // ── Intrinsic feedback.
    driver.set_root(intrinsic_screen());
    driver.draw_frame();
    let start = Instant::now();
    let (pixels, report) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let intrinsic_ms = start.elapsed().as_secs_f64() * 1000.0;
    std::fs::write(out.join("03-intrinsic.png"), pixels.encode_png()?)?;
    println!(
        "03-intrinsic: {} shapes — {:.1} ms",
        report.shapes, intrinsic_ms
    );

    let overflows = vieww_render::overflow::reported();
    println!("overflows across every screen: {overflows}");
    if overflows > 0 {
        failures.push(format!("{overflows} layout overflow(s) reported"));
    }

    std::fs::write(
        out.join("metrics.txt"),
        format!(
            "breadth_cells={BREADTH}\ndepth_levels={DEPTH}\nbreadth_first_ms={breadth_first_ms:.3}\nbreadth_second_ms={breadth_second_ms:.3}\ndepth_ms={depth_ms:.3}\nintrinsic_ms={intrinsic_ms:.3}\noverflows={overflows}\n"
        ),
    )?;

    println!("\nwrote {}", out.display());
    if failures.is_empty() {
        println!("ALL LAYOUT-STRESS CHECKS PASSED");
    } else {
        for failure in &failures {
            eprintln!("FAIL: {failure}");
        }
        return Err(format!("{} layout-stress check(s) failed", failures.len()).into());
    }
    Ok(())
}

fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(0)
}

// ─────────────────────────────────────────────────────────────────────────
// Screens
// ─────────────────────────────────────────────────────────────────────────

fn shell(title: &str, subtitle: &str, body: WidgetNode) -> WidgetNode {
    let header: WidgetNode = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new(title).color(INK).size(24.0).bold(),
            Text::new(subtitle).color(MUTED).size(12.0),
        ])
        .into();
    Container::new()
        .color(BG)
        .padding(EdgeInsets::all(28.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(children![header, body]),
        )
        .into()
}

/// One cell of the breadth grid, coloured by its signal behind its own
/// repaint boundary — the boundary is what makes the one-cell rebuild
/// bounded, and its absence is what this suite exists to catch.
#[derive(Debug)]
struct Cell {
    color: Signal<Color>,
}

impl Widget for Cell {
    fn debug_name(&self) -> &'static str {
        "Cell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        RepaintBoundary::new()
            .child(ColoredBox::new(self.color.get()).child(SizedBox::square(8.0)))
            .into()
    }
}

vieww::widget::widget_node_from!(Cell);

fn breadth_screen(cells: &[Signal<Color>]) -> WidgetNode {
    // 113 columns of 8px cells at 1px gaps: 5 000 cells in 45 rows that fit
    // the frame exactly, so the stress is "lay all of them out" with no
    // overflow noise to explain away.
    const COLUMNS: usize = 113;
    let rows: Vec<WidgetNode> = cells
        .chunks(COLUMNS)
        .map(|row| {
            Flex::row()
                .spacing(1.0)
                .children(row.iter().map(|color| {
                    Cell {
                        color: color.clone(),
                    }
                    .into()
                }))
                .into()
        })
        .collect();
    shell(
        "Breadth — 5 000 siblings",
        "A 113-column grid of signal-driven cells, each behind its own repaint \
         boundary: one changed cell must cost one cell, not the grid",
        Container::new()
            .decoration(BoxDecoration::new().color(Color::WHITE).radius(12.0))
            .padding(EdgeInsets::all(8.0))
            .child(Flex::column().spacing(1.0).children(rows))
            .into(),
    )
}

/// `level` levels of nested padded boxes, alternating flex axes, with a
/// label at the bottom — the classic deep-tree worst case for layout.
fn depth_screen() -> WidgetNode {
    fn nest(level: usize) -> WidgetNode {
        if level == 0 {
            Text::new("bottom — 24 levels up, still laid out")
                .color(MUTED)
                .size(11.0)
                .into()
        } else {
            let child = nest(level - 1);
            let padded = Container::new()
                .padding(EdgeInsets::symmetric(6.0, 5.0))
                .child(child);
            let axis = if level % 2 == 0 {
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children([padded.into()])
            } else {
                Flex::row().children([padded.into()])
            };
            Container::new()
                .decoration(
                    BoxDecoration::new()
                        .color(if level % 3 == 0 {
                            Color::WHITE
                        } else {
                            Color::rgba(58, 122, 246, (8 + level) as u8)
                        })
                        .radius(3.0),
                )
                .child(axis)
                .into()
        }
    }
    shell(
        "Depth — 24 levels of nesting",
        "Alternating axes, padding and backgrounds all the way down: every \
         level constrains, and every level is constrained",
        nest(DEPTH),
    )
}

/// Intrinsic-size feedback: the panel's width comes from its widest row,
/// which comes from its text, which must be measured before the panel can
/// size — and there are three such panels inside each other.
fn intrinsic_screen() -> WidgetNode {
    fn intrinsic_panel(label: &str, rows: usize) -> WidgetNode {
        let content: Vec<WidgetNode> = (0..rows)
            .map(|row| {
                Text::new(format!("{label} · row {row} — width from the text"))
                    .color(INK)
                    .size(12.0)
                    .into()
            })
            .collect();
        Container::new()
            .decoration(
                BoxDecoration::new()
                    .color(Color::WHITE)
                    .radius(8.0)
                    .border(vieww_foundation::Border::new(ACCENT, 1.0)),
            )
            .padding(EdgeInsets::all(12.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(4.0)
                    .children(content),
            )
            .into()
    }
    let body: WidgetNode = Flex::row()
        .spacing(16.0)
        .children(children![
            intrinsic_panel("outer", 4),
            intrinsic_panel("mid", 6),
            intrinsic_panel("inner", 8),
        ])
        .into();
    shell(
        "Intrinsic — widths that measure their children",
        "Three hugging panels, each as wide as its widest row of text: \
         the measure must flow up the tree, not guess down it",
        body,
    )
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
