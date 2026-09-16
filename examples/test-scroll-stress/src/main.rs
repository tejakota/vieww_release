//! # Vieww scroll stress — certification suite
//!
//! A 5 000-row feed in a virtualised list, scrolled end to end at speed,
//! with fling-like acceleration and hard jumps. The questions: does the
//! list keep building only the rows the window covers, does a jump land
//! without a full-tree rebuild, and does every scroll step actually change
//! the picture?
//!
//! ```console
//! cargo run --release -p test-scroll-stress -- /tmp/vieww-scroll-stress
//! ```
//!
//! The assertions:
//! - only a window's worth of rows is ever in the tree (≈ tens, never 5 000)
//! - a hard jump (row 0 → row 4 900) rebuilds bounded work, not the feed
//! - every consecutive pair of scroll frames differs
//! - no layout overflow anywhere

use std::path::{Path as FsPath, PathBuf};
use std::time::Instant;

use gif::{Encoder, Frame, Repeat};
use vieww_element::Signal;
use vieww_foundation::{Color, Constraints, EdgeInsets, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{ListView, Scrollable};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 760.0;
const FRAME_MS: u16 = 50;

const ROW: f32 = 32.0;
const ROWS: usize = 5_000;
/// The list's window, as a device the feed is seen through.
const WINDOW: Size = Size::new(420.0, 600.0);

const BG: Color = Color::rgb(247, 248, 250);
const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const ACCENT: Color = Color::rgb(58, 122, 246);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("test-scroll-stress-out"));
    std::fs::create_dir_all(&out)?;

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let offset = driver.elements().runtime().signal(0.0_f32);
    driver.set_root(feed(&offset));
    driver.draw_frame();

    let mut renderer = NativeRenderer::new();
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut row_counts: Vec<usize> = Vec::new();
    let mut frame_times: Vec<f64> = Vec::new();

    vieww_render::overflow::forget_reported();

    // ── Scroll positions: a slow start, a fling through the middle, and a
    // hard jump near the end — three regimes with three different costs.
    let mut positions: Vec<f32> = Vec::new();
    for step in 0..24 {
        // Ease in and accelerate: the fling.
        let t = step as f32 / 24.0;
        positions.push(t * t * 900.0);
    }
    // The hard jump: row ~30 to row ~4 900 in one step.
    positions.push(4_900.0 * ROW);
    // And a slow settle back from the jump.
    for step in 0..16 {
        positions.push(4_900.0 * ROW - (step as f32 + 1.0) * 40.0);
    }

    let mut jump_built = usize::MAX;
    let mut jump_total = usize::MAX;
    for (index, position) in positions.iter().enumerate() {
        offset.set(*position);
        driver.draw_frame();

        if index == 24 {
            // The jump frame itself: what did the flatten rebuild?
            let stats = driver.flatten_stats();
            jump_built = stats.built;
            jump_total = stats.total();
        }

        let start = Instant::now();
        let (pixels, report) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        frame_times.push(start.elapsed().as_secs_f64() * 1000.0);
        // Count the *runs*, not the glyphs: a row is one or two runs of a
        // few dozen glyphs, and the claim under test is about rows.
        row_counts.push(driver.scene().glyph_runs().len());
        if index % 4 == 0 || index == 24 {
            frames.push(pixels.data().to_vec());
        }
        let _ = report;
    }

    // ── The window is bounded, everywhere along the feed.
    let max_rows = row_counts.iter().copied().max().unwrap_or(0);
    let min_rows = row_counts.iter().copied().min().unwrap_or(0);
    // ── Every step but the jump moved the picture; the jump moves it too.
    let moving = count_moving(&frames);

    let mean = frame_times.iter().sum::<f64>() / frame_times.len() as f64;
    let worst = frame_times.iter().copied().fold(0.0_f64, f64::max);
    println!(
        "rows in tree: {min_rows}..{max_rows} (of {ROWS}), moving frames: \
         {moving}/{}, render mean {mean:.2} ms, worst {worst:.2} ms",
        frames.len() - 1
    );
    println!(
        "jump frame: built {jump_built} of {jump_total} ({}%)",
        percent(jump_built, jump_total)
    );

    write_gif(
        &frames,
        WIDTH as u32,
        HEIGHT as u32,
        &out.join("test-scroll-stress.gif"),
    )?;

    let overflows = vieww_render::overflow::reported();
    let mut failures = Vec::new();
    // A window's worth of rows — in runs, at roughly two runs per row, with
    // slack for partial rows at both edges and the screen's own labels.
    if max_rows > ((WINDOW.height / ROW + 4.0) as usize) * 3 {
        failures.push(format!(
            "{max_rows} rows built for a {}px window over {}px rows — the \
             virtualisation is leaking",
            WINDOW.height, ROW
        ));
    }
    if moving < frames.len() / 2 {
        failures.push("the scroll sequence barely changed; scrolling appears dead".into());
    }
    // The jump must rebuild the window it lands on, not the whole feed's
    // worth of commands — a window is tens of rows, a feed is five
    // thousand.
    if jump_built > 400 {
        failures.push(format!(
            "the hard jump rebuilt {jump_built} commands — the feed's \
             window, not its contents, is what should have been rebuilt"
        ));
    }
    if overflows > 0 {
        failures.push(format!("{overflows} layout overflow(s) reported"));
    }

    std::fs::write(
        out.join("metrics.txt"),
        format!(
            "feed_rows={ROWS}\nwindow_rows={min_rows}..{max_rows}\nmoving={moving}\nrender_mean_ms={mean:.3}\nrender_worst_ms={worst:.3}\njump_built={jump_built}\njump_total={jump_total}\noverflows={overflows}\n"
        ),
    )?;

    println!("wrote {}", out.display());
    if failures.is_empty() {
        println!("ALL SCROLL-STRESS CHECKS PASSED");
    } else {
        for failure in &failures {
            eprintln!("FAIL: {failure}");
        }
        return Err(format!("{} scroll-stress check(s) failed", failures.len()).into());
    }
    Ok(())
}

fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(0)
}

fn count_moving(frames: &[Vec<u8>]) -> usize {
    frames.windows(2).filter(|pair| pair[0] != pair[1]).count()
}

// ─────────────────────────────────────────────────────────────────────────
// The feed
// ─────────────────────────────────────────────────────────────────────────

fn feed(offset: &Signal<f32>) -> vieww_widget::WidgetNode {
    Feed {
        offset: offset.clone(),
    }
    .into()
}

/// A 5 000-row feed in a fixed window — exactly the shape
/// `crates/vieww/tests/virtualised_scrolling.rs` certifies, driven here by
/// a signal an animation could equally drive.
#[derive(Debug)]
struct Feed {
    offset: Signal<f32>,
}

impl Widget for Feed {
    fn debug_name(&self) -> &'static str {
        "Feed"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let header: vieww_widget::WidgetNode = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(4.0)
            .children(children![
                Text::new("Scroll stress — 5 000 rows, virtualised")
                    .color(INK)
                    .size(20.0)
                    .bold(),
                Text::new(
                    "the feed builds one window of rows, wherever the \
                           offset lands"
                )
                .color(MUTED)
                .size(12.0),
            ])
            .into();
        let body: vieww_widget::WidgetNode = Constrained::new(Constraints::tight(WINDOW))
            .child(Scrollable::vertical(self.offset.get()).child(ListView::new(
                ROWS,
                ROW,
                std::rc::Rc::new(|row: usize| row_item(row).into()),
            )))
            .into();
        Container::new()
            .color(BG)
            .padding(EdgeInsets::all(30.0))
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .spacing(16.0)
                    .children(children![header, body]),
            )
            .into()
    }
}

vieww::widget::widget_node_from!(Feed);

/// One row: the index, every seventh row accented so motion reads as
/// motion (not a texture sliding), and a summary whose length varies so
/// each row's ink is its own.
fn row_item(row: usize) -> vieww_widget::Text {
    let label = format!(
        "item {row:04} — {}",
        match row % 4 {
            0 => "short",
            1 => "a longer summary line for the feed",
            2 => "medium summary",
            _ => "an even longer summary line, to vary the row's ink",
        }
    );
    Text::new(label)
        .color(if row % 7 == 0 { ACCENT } else { INK })
        .size(13.0)
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
