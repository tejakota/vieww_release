//! What a frame of the studio actually costs, phase by phase.
//!
//! ```console
//! cargo run --release -p viewwstudio --example bench
//! cargo run --release -p viewwstudio --example bench -- --frames=400 --json
//! ```
//!
//! # Why this exists before any optimisation does
//!
//! "Make it faster" has exactly one honest starting point, and it is a number.
//! Without one, optimisation is a sequence of plausible-sounding edits with no
//! way to tell the ones that helped from the ones that made it slower, and the
//! usual outcome is a codebase that is harder to read and the same speed.
//!
//! So this measures the four things a frame is made of, separately, because
//! they have completely different fixes:
//!
//! * **build** — running `Widget::build` down the dirty part of the tree.
//!   Expensive when widgets allocate or format strings during build.
//! * **layout** — constraints down, sizes up. Expensive when the tree is deep
//!   or something re-measures text.
//! * **paint** — walking render objects into a `Scene`. Expensive when it is
//!   done for things that did not change.
//! * **raster** — the scene into pixels. Measured through the CPU backend
//!   because that is the one that runs everywhere, including here.
//!
//! `draw_frame_at` does the first three together, so they are separated by
//! measuring the driver's own phase clocks where it exposes them and the whole
//! otherwise. What matters for optimisation work is that the *same* number is
//! produced before and after a change, on the same machine, from the same
//! frames — not that it matches any other profiler.
//!
//! # The two numbers that matter most
//!
//! **Idle** is the cost of a frame where nothing changed. The studio is a
//! window somebody leaves open all day; a non-zero idle cost is a battery
//! complaint. `tests/idle_cost.rs` asserts the *shape* of this — that an idle
//! frame asks for no repaint — and this puts a time on it.
//!
//! **Typing** is the cost of a frame where one character was inserted into the
//! buffer. That is the interaction with the tightest budget in the whole
//! application, because it happens on every keystroke and the eye is merciless
//! about latency between a key and a glyph.

use std::time::{Duration, Instant};

use vieww_foundation::{Color, Size};
use vieww_paint::{FrameInfo, FrameSink};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

/// A window big enough to have real work in it.
const WINDOW: Size = Size {
    width: 1440.0,
    height: 900.0,
};

/// One measured phase.
struct Stat {
    what: &'static str,
    samples: Vec<Duration>,
}

impl Stat {
    fn new(what: &'static str) -> Self {
        Self {
            what,
            samples: Vec::new(),
        }
    }

    fn push(&mut self, taken: Duration) {
        self.samples.push(taken);
    }

    /// The median, which is the number to quote.
    ///
    /// Not the mean: a run of a few hundred frames on a shared machine reliably
    /// contains a handful that were descheduled, and a mean that a scheduling
    /// hiccup can move by 30% is a mean that reports optimisations that did not
    /// happen. The p95 is printed beside it because a frame budget is about the
    /// slow frames, not the typical one.
    fn median(&mut self) -> Duration {
        self.samples.sort_unstable();
        self.samples
            .get(self.samples.len() / 2)
            .copied()
            .unwrap_or_default()
    }

    fn p95(&mut self) -> Duration {
        self.samples.sort_unstable();
        let at = self.samples.len() * 95 / 100;
        self.samples
            .get(at.min(self.samples.len().saturating_sub(1)))
            .copied()
            .unwrap_or_default()
    }
}

fn micros(taken: Duration) -> f64 {
    taken.as_secs_f64() * 1e6
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("--frames="))
        .and_then(|n| n.parse().ok())
        .unwrap_or(200);
    let json = args.iter().any(|a| a == "--json");
    // The histogram `ChildIds`' inline capacity is chosen from. Off by default
    // because it answers a question that is settled until somebody changes the
    // shape of a screen, and on demand because a constant argued from a number
    // needs the number to still be produceable. See `vieww_render::ChildIds`.
    let fanout = args.iter().any(|a| a == "--fanout");
    // Which phases to run. Rasterising is two hundred milliseconds a frame on
    // the CPU backend and a *profiler* pointed at the default run sees almost
    // nothing else — the first callgrind pass over this was 40% vello and 0.4%
    // anything in this workspace. `--only=typing` is what a profile is taken
    // through.
    let only = args
        .iter()
        .find_map(|a| a.strip_prefix("--only="))
        .unwrap_or("all");
    let wants = |phase: &str| only == "all" || only == phase;

    if cfg!(debug_assertions) {
        eprintln!(
            "warning: this is a debug build and every number below is several \
             times what a shipped studio costs. Use --release."
        );
    }

    let mut driver = FrameDriver::new(WINDOW);
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime);
    studio.note_window_size(WINDOW);
    studio.splash.set(false);
    driver.set_root(Shell {
        studio: studio.clone(),
    });

    // Warm: the first frame builds the whole tree, faults in the font atlas and
    // fills every cache in the workspace. Including it in the samples would
    // measure the cold start once and hide the steady state forever.
    for step in 0..20 {
        driver.draw_frame_at(Duration::from_millis(step));
    }

    let mut idle = Stat::new("idle frame");
    let mut dirty = Stat::new("frame after a state change");
    let mut typing = Stat::new("frame after a keystroke");
    let mut raster = Stat::new("rasterise (CPU)");
    let mut edit_only = Stat::new("  of which: Studio::edit");
    let mut frame_only = Stat::new("  of which: build+layout+paint");
    let mut build = Stat::new("    build");
    let mut layout = Stat::new("    layout");
    let mut paint = Stat::new("    paint");

    // ---------------------------------------------------------------- idle
    for step in 0..(if wants("idle") { frames } else { 0 }) {
        let now = Duration::from_millis(1000 + step as u64);
        let at = Instant::now();
        driver.draw_frame_at(now);
        idle.push(at.elapsed());
    }

    // ------------------------------------------------- a plain state change
    //
    // Toggling the bottom panel is a real interaction that dirties a large
    // subtree without touching the buffer, which separates "rebuild a region"
    // from "re-highlight a file".
    for step in 0..(if wants("dirty") { frames } else { 0 }) {
        let now = Duration::from_millis(2000 + step as u64 * 4);
        studio.panel_open.set(step % 2 == 0);
        let at = Instant::now();
        driver.draw_frame_at(now);
        dirty.push(at.elapsed());
    }

    // -------------------------------------------------------------- typing
    //
    // The tightest budget in the application. One character in, one frame out.
    //
    // `Studio::edit` is the one entry point every edit goes through, so this is
    // the same path a keypress takes: text in, re-highlight, tab dot, caret
    // readout, frame out.
    let start = studio
        .active()
        .map(|buffer| buffer.value.text.clone())
        .unwrap_or_default();
    for step in 0..(if wants("typing") { frames } else { 0 }) {
        let now = Duration::from_millis(6000 + step as u64 * 4);
        let mut text = start.clone();
        #[expect(clippy::cast_possible_truncation, reason = "step % 26 is 0..26")]
        text.insert(0, char::from(b'a' + (step % 26) as u8));
        let value = vieww_foundation::TextEditingValue {
            text,
            selection: vieww_foundation::TextSelection::collapsed(1),
            composing: None,
            secondary: Vec::new(),
        };
        let at = Instant::now();
        studio.edit(value);
        let edited = at.elapsed();

        // The four phases, run by hand rather than through `draw_frame_at`, so
        // each one can be timed. This is exactly what that method does — see
        // its body — and the point of splitting it here is that "a keystroke
        // costs five milliseconds" is not an actionable sentence, while "four
        // of the five are in layout" is.
        let frame = FrameInfo {
            number: 0,
            timestamp: now,
            delta: Duration::ZERO,
        };
        let phase = Instant::now();
        FrameSink::animate(&mut driver, &frame);
        let animated = phase.elapsed();
        let phase = Instant::now();
        FrameSink::build(&mut driver, &frame);
        build.push(phase.elapsed());
        let phase = Instant::now();
        FrameSink::layout(&mut driver, &frame);
        layout.push(phase.elapsed());
        let phase = Instant::now();
        FrameSink::paint(&mut driver, &frame);
        paint.push(phase.elapsed());
        let phase = Instant::now();
        FrameSink::composite(&mut driver, &frame);
        let composited = phase.elapsed();
        let _ = (animated, composited);

        let whole = at.elapsed();
        edit_only.push(edited);
        frame_only.push(whole - edited);
        typing.push(whole);
    }

    // **The same keystroke with syntax highlighting off.**
    //
    // Not a setting anyone would ship with — it is a *subtraction*. The
    // difference between this line and the one above is what the highlighter
    // costs per keystroke, which no flat profile answers directly because the
    // parse, the span building and the text shaping are three different places
    // in the call graph.
    let mut unlit = Stat::new("    (typing, highlighting off)");
    if wants("typing") {
        studio.highlight_enabled.set(false);
        driver.draw_frame_at(Duration::from_millis(8000));
        for step in 0..frames {
            let now = Duration::from_millis(8100 + step as u64 * 4);
            let mut text = start.clone();
            #[expect(clippy::cast_possible_truncation, reason = "step % 26 is 0..26")]
            text.insert(0, char::from(b'a' + (step % 26) as u8));
            studio.edit(vieww_foundation::TextEditingValue {
                text,
                selection: vieww_foundation::TextSelection::collapsed(1),
                composing: None,
                secondary: Vec::new(),
            });
            let at = Instant::now();
            driver.draw_frame_at(now);
            unlit.push(at.elapsed());
        }
        studio.highlight_enabled.set(true);
        driver.draw_frame_at(Duration::from_millis(8900));
    }

    // **How much of the tree a keystroke actually rebuilds.**
    //
    // The number that tells an optimiser where to look. A frame that costs
    // three milliseconds because it rebuilt four thousand elements is a
    // dirty-tracking problem; the same three milliseconds spread over forty is
    // a problem inside those forty. `ElementTree::hotspots` names the widgets.
    driver.elements().mark_builds();
    {
        let mut text = start.clone();
        text.insert(0, 'Z');
        studio.edit(vieww_foundation::TextEditingValue {
            text,
            selection: vieww_foundation::TextSelection::collapsed(1),
            composing: None,
            secondary: Vec::new(),
        });
        driver.draw_frame_at(Duration::from_millis(9000));
    }
    let rebuilt = driver.elements().builds_since_mark();
    let total = driver.elements().len();

    // ------------------------------------------------------------ rasterise
    //
    // Fewer samples: this is milliseconds a frame, where the other three are
    // microseconds. It is here because it is the only measure of "how much is
    // in the scene" that works without a display, and it needs no adapter to
    // run at all.
    let mut cpu = vieww_paint::native::NativeRenderer::new();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a window size"
    )]
    let (width, height) = (WINDOW.width as u32, WINDOW.height as u32);
    for _ in 0..(if wants("raster") {
        (frames / 20).max(3)
    } else {
        0
    }) {
        let at = Instant::now();
        let _ = cpu
            .render_to_pixels(driver.scene(), width, height, Color::BLACK)
            .expect("rasterising");
        raster.push(at.elapsed());
    }

    let report = if wants("raster") {
        cpu.render_to_pixels(driver.scene(), width, height, Color::BLACK)
            .expect("rasterising")
            .1
    } else {
        vieww_paint::native::SceneReport::default()
    };

    let mut stats = [
        idle, dirty, typing, edit_only, frame_only, build, layout, paint, unlit, raster,
    ];
    if json {
        println!("{{");
        for (index, stat) in stats.iter_mut().enumerate() {
            let comma = if index + 1 == 10 { "" } else { "," };
            let (median, p95) = (micros(stat.median()), micros(stat.p95()));
            println!(
                "  \"{}\": {{ \"median_us\": {median:.1}, \"p95_us\": {p95:.1} }}{comma}",
                stat.what
            );
        }
        println!("}}");
    } else {
        println!("{frames} frames, {}×{} window\n", width, height);
        println!("{:<32}{:>12}{:>12}", "", "median", "p95");
        for stat in &mut stats {
            let (median, p95) = (micros(stat.median()), micros(stat.p95()));
            println!("{:<32}{median:>10.1}µs{p95:>10.1}µs", stat.what);
        }
        println!(
            "\nscene: {} shapes, {} glyph runs, {} layers, {} commands skipped",
            report.shapes, report.glyph_runs, report.layers, report.skipped_commands
        );
        println!("tree:  {total} elements, {rebuilt} rebuilt by one keystroke");
        if fanout {
            // Eight buckets: 0..=6 children, then everything else. `ChildIds`
            // spills past four, so the interesting boundary is inside the range
            // rather than at its end.
            let hist = driver.owner().tree().child_fanout(8);
            let nodes: usize = hist.iter().sum();
            let branching: usize = hist[1..].iter().sum();
            println!("       {nodes} render nodes, {branching} of them with children");
            let mut cumulative = 0usize;
            for (children, &count) in hist.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                cumulative += count;
                let label = if children + 1 == hist.len() {
                    format!("{children}+")
                } else {
                    children.to_string()
                };
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "node counts are thousands, and this is a percentage"
                )]
                let (share, running) = (
                    100.0 * count as f64 / nodes as f64,
                    100.0 * cumulative as f64 / nodes as f64,
                );
                println!("       {label:>3} children: {count:5}  {share:5.1}%  ({running:5.1}% cumulative)");
            }
            // The number the constant is actually chosen against: of the nodes
            // that would allocate with a bare `Vec`, how many fit inline.
            for capacity in [1usize, 2, 4, 6] {
                let fits: usize = hist[1..=capacity.min(hist.len() - 1)].iter().sum();
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "node counts are thousands, and this is a percentage"
                )]
                let share = 100.0 * fits as f64 / branching as f64;
                println!("       inline {capacity}: {fits} of {branching} allocations removed ({share:.1}%)");
            }
        }
        if !json {
            for hotspot in driver.elements().hotspots(8) {
                println!("       {hotspot:?}");
            }
        }
    }
}
