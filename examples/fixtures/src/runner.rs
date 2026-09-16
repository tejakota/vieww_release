//! The thing that turns a fixture into a picture and a number.
//!
//! # Why a runner and not `main` doing it inline
//!
//! Every fixture in this gallery has to be measured the *same* way or the
//! numbers cannot be compared, and comparing them is the entire point: a
//! rendering cost is only ever pathological *relative* to a simpler scene
//! that does nearly the same work. The clip bug this gallery was built after
//! looked like "the studio is slow" for as long as nobody had a picture of
//! four hundred rectangles with and without a rounded corner around them,
//! rendered by the same code, timed on the same machine.
//!
//! # One renderer, reused
//!
//! Each fixture gets its own [`NativeRenderer`] but renders through it more
//! than once: the first frame is a warm-up, and the reported time is a later
//! one. That is deliberate and it is the honest number for a *window*, which
//! is what this is a model of — a window keeps its renderer, and therefore
//! its glyph outlines and its clip masks, between frames. Timing the very
//! first frame would report font parsing as if it happened every frame.

use std::path::Path;
use std::time::{Duration, Instant};

use vieww_foundation::{Color, Size};
use vieww_paint::native::{NativeRenderer, SceneReport};
use vieww_render::FrameDriver;
use vieww_widget::WidgetNode;

/// How many times a still fixture is rendered before the timed run, and how
/// many timed runs are taken. Small: this is a gallery, not a benchmark
/// suite, and the costs it exists to find differ by orders of magnitude
/// rather than by percent.
const WARMUP: usize = 1;
const RUNS: usize = 3;

/// One entry in the gallery.
pub(crate) struct Fixture {
    /// `tier/name`, which is also the PNG's filename.
    pub name: &'static str,
    /// What this fixture is for — printed beside its numbers so a row in the
    /// report explains itself.
    pub about: &'static str,
    pub size: Size,
    pub background: Color,
    pub build: fn() -> WidgetNode,
}

/// What one fixture cost, once rendered.
pub(crate) struct Measured {
    pub name: &'static str,
    pub about: &'static str,
    pub size: Size,
    /// Best of [`RUNS`] — the floor, which is the number a regression moves.
    pub best: Duration,
    /// Worst of [`RUNS`], to show when a fixture is noisy rather than slow.
    pub worst: Duration,
    pub report: SceneReport,
    /// Commands the widget tree produced. Cost per command is the ratio worth
    /// looking at; a scene that is slow because it is *large* is a different
    /// problem from one that is slow per command.
    pub commands: usize,
    /// Layout overflows this fixture's own tree reported while building.
    ///
    /// `vieww_render::overflow::reported` has existed the whole time, with a
    /// doc comment saying a number "is what turns that from something a
    /// reviewer might notice into something a script can refuse" — and nothing
    /// in this gallery read it. So the gallery printed overflow warnings on
    /// stderr, between the pictures, on every run, and they scrolled past
    /// looking exactly like the progress output the same doc warns about.
    ///
    /// Counted per fixture rather than for the whole run because "something
    /// overflowed" is not actionable and "31-stagger overflowed" is.
    pub overflows: usize,
}

impl Measured {
    /// Microseconds of render time per recorded command.
    #[must_use]
    pub(crate) fn per_command_us(&self) -> f64 {
        self.best.as_secs_f64() * 1e6 / self.commands.max(1) as f64
    }

    /// How many of these frames fit in 16.67 ms. Below 1.0 the fixture cannot
    /// hold 60 Hz on this machine by itself, let alone with a real
    /// application's other work beside it.
    #[must_use]
    pub(crate) fn frames_in_budget(&self) -> f64 {
        0.016_667 / self.best.as_secs_f64().max(1e-9)
    }
}

/// Build, render, time and write one fixture.
pub(crate) fn run(fixture: &Fixture, out: &Path) -> Measured {
    // Attributed to this fixture: cleared before its tree is built, read after.
    vieww_render::overflow::forget_reported();

    let mut driver = FrameDriver::new(fixture.size);
    driver.elements().set_root((fixture.build)());
    driver.draw_frame();
    let overflows = vieww_render::overflow::reported();

    let (width, height) = (fixture.size.width as u32, fixture.size.height as u32);
    let mut renderer = NativeRenderer::new();

    for _ in 0..WARMUP {
        renderer
            .render_to_pixels(driver.scene(), width, height, fixture.background)
            .expect("warm-up render");
    }

    let mut best = Duration::MAX;
    let mut worst = Duration::ZERO;
    let mut report = SceneReport::default();
    for _ in 0..RUNS {
        let start = Instant::now();
        let (_pixels, this) = renderer
            .render_to_pixels(driver.scene(), width, height, fixture.background)
            .expect("timed render");
        let elapsed = start.elapsed();
        best = best.min(elapsed);
        worst = worst.max(elapsed);
        report = this;
    }

    let (png, _) = renderer
        .render_to_png(driver.scene(), width, height, fixture.background)
        .expect("encoding the fixture");
    let path = out.join(format!("{}.png", fixture.name.replace('/', "-")));
    std::fs::write(&path, png).expect("writing the fixture PNG");

    Measured {
        name: fixture.name,
        about: fixture.about,
        size: fixture.size,
        best,
        worst,
        report,
        commands: driver.scene().len(),
        overflows,
    }
}

/// The report table — one line per fixture, widest cost last so the eye lands
/// on the outlier.
pub(crate) fn print_table(measured: &[Measured]) {
    // **`worst` is printed, and it is not decoration.**
    //
    // `best` is the floor and it is the right number to compare across a
    // change. But a floor with a ceiling three times higher above it is not a
    // measurement of this code, it is a measurement of what else the machine
    // was doing — and a table that shows only the floor gives no way to tell a
    // real regression from a noisy afternoon. Both fields were already being
    // computed and neither was being shown; `worst` was carrying a doc comment
    // saying it existed "to show when a fixture is noisy rather than slow"
    // while being dead code.
    println!(
        "\n{:<34} {:>9} {:>7} {:>9} {:>9} {:>9} {:>8}  what it covers",
        "fixture", "size", "cmds", "best", "worst", "us/cmd", "fps"
    );
    println!("{}", "-".repeat(136));
    for m in measured {
        let flag = if m.frames_in_budget() < 1.0 {
            "  <-- OVER 16.7ms"
        } else if m.worst.as_secs_f64() > m.best.as_secs_f64() * 2.0 {
            "  (noisy: worst is over twice best)"
        } else {
            ""
        };
        println!(
            "{:<34} {:>4}x{:<4} {:>7} {:>8.2}ms {:>8.2}ms {:>9.1} {:>8.0}  {}{}",
            m.name,
            m.size.width as u32,
            m.size.height as u32,
            m.commands,
            m.best.as_secs_f64() * 1000.0,
            m.worst.as_secs_f64() * 1000.0,
            m.per_command_us(),
            m.frames_in_budget(),
            m.about,
            flag
        );
    }

    let total: f64 = measured.iter().map(|m| m.best.as_secs_f64()).sum();
    let over: Vec<&str> = measured
        .iter()
        .filter(|m| m.frames_in_budget() < 1.0)
        .map(|m| m.name)
        .collect();
    println!("{}", "-".repeat(136));
    println!(
        "{} fixtures, {:.1} ms of rendering in total",
        measured.len(),
        total * 1000.0
    );
    // What the gallery actually asked the renderer to draw, summed. The other
    // half of `Measured` that was being computed and thrown away — and the
    // number that says whether a change to the *fixtures* moved a timing,
    // rather than a change to the renderer.
    let shapes: usize = measured.iter().map(|m| m.report.shapes).sum();
    let glyphs: usize = measured.iter().map(|m| m.report.glyphs).sum();
    let layers: usize = measured.iter().map(|m| m.report.layers).sum();
    let shadows: usize = measured.iter().map(|m| m.report.shadows).sum();
    let skipped: usize = measured.iter().map(|m| m.report.skipped_commands).sum();
    println!(
        "drawn across the gallery: {shapes} shapes, {glyphs} glyphs, \
         {layers} layers, {shadows} shadows ({skipped} commands culled)"
    );
    if over.is_empty() {
        println!("every fixture renders inside a 60 Hz frame budget");
    } else {
        println!("over budget: {}", over.join(", "));
    }

    let overflowing: Vec<String> = measured
        .iter()
        .filter(|m| m.overflows > 0)
        .map(|m| format!("{} ({})", m.name, m.overflows))
        .collect();
    if overflowing.is_empty() {
        println!("no fixture's layout overflows");
    } else {
        println!(
            "LAYOUT OVERFLOW in {}: a fixture whose own layout does not fit is \
             not a picture of what this framework does, it is a picture of a \
             bug — fix the fixture",
            overflowing.join(", ")
        );
    }
}

/// How many fixtures reported a layout overflow.
///
/// Returned so `main` can exit non-zero: see [`Measured::overflows`] for why a
/// printed warning was not enough.
#[must_use]
pub(crate) fn total_overflows(measured: &[Measured]) -> usize {
    measured.iter().map(|m| m.overflows).sum()
}
