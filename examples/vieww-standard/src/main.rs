//! # The Vieww standard — Phase 4 contracts, measured
//!
//! "Superior" is only a claim if it is measurable. This runner takes the
//! twelve clauses of the Vieww standard and *measures* each one from a
//! real render workload — a cold first frame, an animated loop, a run of
//! steady frames under a counting allocator, the real GPU planner, a signal
//! flip, a pointer press, a memory-pressure trim, two shaping passes — then
//! feeds every number into
//! [`vieww_render_planner::QualityReport::check`], which has the final say.
//!
//! ```console
//! cargo run --release -p vieww-standard -- /tmp/vieww-standard
//! ```
//!
//! | contract | how it is measured here |
//! |---|---|
//! | startup | a *cold* renderer + driver + font store, to the first presented frame |
//! | frame budget | the worst frame of the animated loop |
//! | frame pacing | the p95 frame of the animated loop |
//! | animation latency | signal set → the first frame whose pixels moved, in vsync intervals |
//! | input latency | pointer press → the first frame whose pixels moved, in vsync intervals |
//! | steady allocations | a counting `#[global_allocator]`, over steady frames through the retained present path |
//! | clean-scene rebuilds | `FrameDriver::scene_rebuilds` across an unchanged frame |
//! | GPU completeness | `vieww_gpu::Planner` over every animated frame and the steady frame; `ScenePlan::unsupported` summed |
//! | pixel parity | the same scene, rendered twice, byte for byte |
//! | memory | a `Critical` trim, then a re-render, byte for byte |
//! | typography | the same paragraph shaped in two stores, run for run |
//! | accessibility | the audit of the standard screen's semantics tree |
//!
//! The output is `vieww-standard.txt` (the human report) and
//! `vieww-standard.json` (the machine one), and the exit code is the
//! verdict.

mod counting;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use vieww_accessibility::audit;
use vieww_element::Signal;
use vieww_foundation::{
    Color, EdgeInsets, MemoryPressure, Offset, PointerEvent, PointerId, Size, TextStyle,
};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_render_planner::{QualityContract, QualityObservation, QualityReport, RefreshRate};
use vieww_text::{FontStore, Paragraph, TextSpan};
use vieww_widget::prelude::*;

const WIDTH: f32 = 1120.0;
const HEIGHT: f32 = 760.0;
/// The vsync interval every latency is expressed in.
const VSYNC: Duration = Duration::from_millis(16);
/// Frames in the pacing loop.
const FRAMES: usize = 90;

const BG: Color = Color::rgb(10, 13, 20);
const INK: Color = Color::rgb(241, 245, 249);
const MUTED: Color = Color::rgb(151, 164, 184);
const ACCENT: Color = Color::rgb(104, 145, 255);
/// Steady frames measured under the counting allocator, after warm-up.
const STEADY_FRAMES: u64 = 60;

#[global_allocator]
static ALLOCATOR: counting::Counting = counting::Counting;

/// The screen every contract is measured against: dense text, a control
/// with a label (for the a11y audit), and an animated tile.
fn standard_screen(counter: &Signal<u32>, press: &Signal<u32>) -> WidgetNode {
    let header: WidgetNode = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            Text::new("The Vieww standard").color(INK).size(24.0).bold(),
            Text::new("eight contracts, measured from this very screen")
                .color(MUTED)
                .size(12.0),
        ])
        .into();
    let tile: WidgetNode = CounterTile {
        counter: counter.clone(),
        press: press.clone(),
    }
    .into();
    let counter_label: WidgetNode = CounterLabel {
        counter: counter.clone(),
    }
    .into();
    let press_label: WidgetNode = PressCount {
        press: press.clone(),
    }
    .into();
    let body: WidgetNode = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(18.0)
        .children(children![
            counter_label,
            press_label,
            Text::new(
                "The five boxing wizards jump quickly. \
                 视界框架文字排版设计，可变粗细演示。",
            )
            .color(INK)
            .size(14.0),
            tile,
        ])
        .into();
    Container::new()
        .color(BG)
        .padding(EdgeInsets::all(32.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(22.0)
                .children(children![header, body]),
        )
        .into()
}

/// The animated tile: its colour cycles with the counter, so every tick
/// moves pixels — and it is *pressable*, so the input-latency probe has a
/// real target that changes real state.
#[derive(Debug)]
struct CounterTile {
    counter: Signal<u32>,
    press: Signal<u32>,
}

impl Widget for CounterTile {
    fn debug_name(&self) -> &'static str {
        "CounterTile"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let phase = self.counter.get() % 4;
        let color = [
            Color::rgb(104, 145, 255),
            Color::rgb(73, 211, 210),
            Color::rgb(178, 112, 255),
            Color::rgb(94, 224, 163),
        ][phase as usize];
        let press = self.press.clone();
        Pressable::new(move |pressed: f32| {
            // The press sense is 0..1 — a *depth*, not a flag — so the wash
            // scales with it.
            let alpha = (pressed * 28.0).round() as u8;
            let wash = Color::rgba(255, 255, 255, alpha);
            Container::new()
                .decoration(
                    BoxDecoration::new()
                        .color(color)
                        .radius(10.0)
                        .shadow(vieww_foundation::Shadow::new(
                            Color::rgba(0, 0, 0, 80),
                            Offset::new(0.0, 5.0),
                            10.0,
                        ))
                        .border(vieww_foundation::Border::new(wash, 2.0)),
                )
                .padding(EdgeInsets::all(18.0))
                .child(Text::new("press me").color(Color::WHITE).size(13.0))
                .into()
        })
        .on_tap(move || {
            let next = press.get() + 1;
            press.set(next);
        })
        .into()
    }
}

vieww::widget::widget_node_from!(CounterTile);

/// "frames ticked: N", subscribed to the counter — a *label* built from the
/// signal rebuilds when the signal moves, where a root that read the signal
/// once would not.
#[derive(Debug)]
struct CounterLabel {
    counter: Signal<u32>,
}

impl Widget for CounterLabel {
    fn debug_name(&self) -> &'static str {
        "CounterLabel"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(format!("frames ticked: {}", self.counter.get()))
            .color(INK)
            .size(15.0)
            .into()
    }
}

vieww::widget::widget_node_from!(CounterLabel);

/// "presses: N", subscribed to the press signal — the visual half of the
/// input-latency probe: the frame that changes this label's pixels is the
/// frame the input reached the screen.
#[derive(Debug)]
struct PressCount {
    press: Signal<u32>,
}

impl Widget for PressCount {
    fn debug_name(&self) -> &'static str {
        "PressCount"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(format!("presses: {}", self.press.get()))
            .color(ACCENT)
            .size(15.0)
            .into()
    }
}

vieww::widget::widget_node_from!(PressCount);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("vieww-standard-out"));
    std::fs::create_dir_all(&out)?;

    // ── Startup: cold everything — driver, store, renderer — to the first
    // presented frame. Measured on the *embedded* font set, which is the
    // deterministic cold path a headless certification runs and what an
    // application presents its first frame from while the system font scan
    // runs behind it; the scan itself is measured separately below, because
    // a first frame that waits for a six-hundred-file directory walk is a
    // product decision, not a renderer property.
    let scan_start = Instant::now();
    drop(FontStore::with_system_fallback());
    let system_scan_ms = scan_start.elapsed().as_secs_f64() * 1000.0;

    let startup_start = Instant::now();
    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    driver.set_fonts(FontStore::embedded_only());
    let counter = driver.elements().runtime().signal(0_u32);
    let press = driver.elements().runtime().signal(0_u32);
    driver.set_root(standard_screen(&counter, &press));
    driver.draw_frame();
    let mut renderer = NativeRenderer::new();
    let (first_frame, _) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let startup_ms = startup_start.elapsed().as_secs_f64() * 1000.0;
    std::fs::write(out.join("first-frame.png"), first_frame.encode_png()?)?;

    // ── Frame budget and pacing: an animated loop, every frame different.
    let mut frame_times_ms: Vec<f64> = Vec::with_capacity(FRAMES);
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(FRAMES);
    // GPU completeness is measured on the same frames, through the real
    // planner a GPU backend executes — not assumed.
    let mut planner = vieww_gpu::Planner::new();
    let mut unsupported_gpu_commands = 0_u64;
    let mut unsupported_kinds = std::collections::BTreeMap::new();
    for frame in 1..=FRAMES {
        counter.set(frame as u32);
        driver.draw_frame_at(VSYNC * frame as u32);
        let start = Instant::now();
        let (pixels, _) =
            renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
        frame_times_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        frames.push(pixels.data().to_vec());
        let plan = planner.plan(driver.scene(), WIDTH, HEIGHT);
        for (kind, count) in &plan.unsupported {
            unsupported_gpu_commands += *count as u64;
            *unsked(&mut unsupported_kinds, kind.name()) += *count as u64;
        }
    }
    let mut sorted = frame_times_ms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let worst_frame_ms = sorted[sorted.len() - 1] as f32;
    let p95_frame_ms = sorted[(sorted.len() as f64 * 0.95) as usize] as f32;

    // ── Pixel parity: the same settled scene, rendered twice, byte for
    // byte. (The loop's last scene is reused untouched.)
    let (a, _) = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let (b, _) = renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let pixel_drift = if a.data() == b.data() { 0.0 } else { 1.0 };

    // ── Animation latency: the counter flips, and the count of frames
    // until the *pixels* move is the latency, in vsync intervals.
    let settled = frames.last().cloned().unwrap_or_default();
    counter.set(1_001);
    driver.draw_frame_at(VSYNC * (FRAMES as u32 + 1));
    let (after_tick, _) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let animation_latency_intervals = if after_tick.data() != settled.as_slice() {
        // The very next frame moved: one interval, the best a vsync'd
        // pipeline can do.
        1.0
    } else {
        99.0
    };

    // ── Input latency: a pointer press routed through the real dispatch,
    // the tappable tile, and the first frame that moves. The tap point comes
    // from the tree's own hit-testing — scan down the page for something
    // tappable rather than hand-computing coordinates the layout will
    // silently invalidate.
    let pressed_before = press.get();
    let mut tile_centre = None;
    for y in (40..HEIGHT as i32).step_by(20) {
        let candidate = Offset::new(64.0, y as f32);
        if driver.hit_test_identify(candidate).is_some() {
            // Confirm it is a real target that can change state: press and
            // release here, and keep the first one that ticks the signal.
            let candidate_time = VSYNC * (FRAMES as u32 + 2);
            let _ =
                driver.handle_pointer(&PointerEvent::down(PointerId(1), candidate, candidate_time));
            driver.draw_frame_at(candidate_time + VSYNC);
            let _ = driver.handle_pointer(&PointerEvent::up(
                PointerId(1),
                candidate,
                candidate_time + Duration::from_millis(40),
            ));
            if press.get() != pressed_before {
                tile_centre = Some(candidate);
                break;
            }
        }
    }
    let dispatched_a_press = tile_centre.is_some();
    if let Some(point) = tile_centre {
        println!("input probe: tapped a live target at {point:?}");
    }
    driver.draw_frame_at(VSYNC * (FRAMES as u32 + 3));
    let (after_press, _) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let input_latency_intervals = if dispatched_a_press && after_press.data() != after_tick.data() {
        1.0
    } else {
        99.0
    };
    if !dispatched_a_press {
        // Nothing on this screen is tappable through this synthesis, which
        // is itself a measurement problem: report it rather than silently
        // passing a latency nothing exercised.
        eprintln!("note: the synthesised press dispatched no state change");
    }

    // ── Memory: a Critical trim, then a re-render, byte for byte.
    driver.trim_memory(MemoryPressure::Critical);
    vieww_foundation::Trim::trim(&mut renderer, MemoryPressure::Critical);
    driver.draw_frame_at(VSYNC * (FRAMES as u32 + 4));
    let (after_trim, _) =
        renderer.render_to_pixels(driver.scene(), WIDTH as u32, HEIGHT as u32, BG)?;
    let trim_pixel_changes = count_differing_bytes(after_trim.data(), after_press.data()) as u64;

    // ── Typography: the same paragraph shaped in two independent stores.
    let text = "The five boxing wizards jump quickly. 视界框架文字排版设计，可变粗细演示。";
    let shape_one = shape_paragraph(text);
    let shape_two = shape_paragraph(text);
    let typography_drift_px = typographic_drift(&shape_one, &shape_two);

    // ── Accessibility: the audit of the standard screen's semantics.
    let findings = audit(&driver.semantics());
    let a11y_issues = findings.len() as u64;

    // ── Clean-scene rebuilds: an unchanged frame must not re-flatten.
    let rebuilds_before = driver.scene_rebuilds();
    driver.draw_frame_at(VSYNC * (FRAMES as u32 + 5));
    let clean_scene_rebuilds = driver.scene_rebuilds() - rebuilds_before;

    // ── Steady allocations: frames in which nothing changes, through the
    // path a window actually presents with — draw the frame, then repaint
    // only its damage into the renderer's retained buffer — counted by the
    // global allocator on this thread. Warm-up first, so caches that fill
    // once (glyph rasters, pools) are not mistaken for per-frame cost.
    if !counting::installed() {
        return Err(
            "steady_allocations cannot be measured: the counting global allocator is \
                    not installed in this binary (is `-C prefer-dynamic` set?). Refusing to \
                    report a zero nothing measured."
                .into(),
        );
    }
    let steady_base = FRAMES as u32 + 6;
    for k in 0..8 {
        driver.draw_frame_at(VSYNC * (steady_base + k));
        renderer.render_retained_in_place(
            driver.scene(),
            driver.damage(),
            WIDTH as u32,
            HEIGHT as u32,
            BG,
        )?;
    }
    if std::env::var_os("VIEWW_ALLOC_TRACE").is_some() {
        counting::trace(|| {
            driver.draw_frame_at(VSYNC * (steady_base + 7));
            eprintln!("=== render ===");
            let _ = renderer.render_retained_in_place(
                driver.scene(),
                driver.damage(),
                WIDTH as u32,
                HEIGHT as u32,
                BG,
            );
        });
    }
    let mut steady_error = None;
    let ((), steady_allocations) = counting::measure(|| {
        for k in 0..STEADY_FRAMES as u32 {
            driver.draw_frame_at(VSYNC * (steady_base + 8 + k));
            if let Err(error) = renderer.render_retained_in_place(
                driver.scene(),
                driver.damage(),
                WIDTH as u32,
                HEIGHT as u32,
                BG,
            ) {
                steady_error = Some(error);
                break;
            }
        }
    });
    if let Some(error) = steady_error {
        return Err(error.into());
    }
    let steady_matches = renderer.last_frame() == after_trim.data();

    // GPU completeness of the settled screen too — the frame a user looks at
    // longest.
    let steady_plan = planner.plan(driver.scene(), WIDTH, HEIGHT);
    for (kind, count) in &steady_plan.unsupported {
        unsupported_gpu_commands += *count as u64;
        *unsked(&mut unsupported_kinds, kind.name()) += *count as u64;
    }

    // ── The verdict.
    let observation = QualityObservation {
        worst_frame_ms,
        p95_frame_ms,
        startup_ms: startup_ms as f32,
        steady_allocations,
        clean_scene_rebuilds,
        unsupported_gpu_commands,
        pixel_drift,
        input_latency_intervals,
        animation_latency_intervals,
        trim_pixel_changes,
        a11y_issues,
        typography_drift_px,
    };
    let report = QualityReport::check(QualityContract::for_refresh(RefreshRate::Hz60), observation);

    println!("The Vieww standard — measured, not asserted\n");
    println!("  startup:                 {startup_ms:8.1} ms (cold driver + store + first frame)");
    println!("  (system font scan:       {system_scan_ms:8.1} ms, measured separately — not first-frame work)");
    println!("  frame budget (worst):    {worst_frame_ms:8.1} ms");
    println!("  frame pacing (p95):      {p95_frame_ms:8.1} ms");
    println!("  animation latency:       {animation_latency_intervals:8.1} intervals");
    println!("  input latency:           {input_latency_intervals:8.1} intervals");
    println!("  pixel parity drift:      {pixel_drift:8.3}");
    println!("  trim pixel changes:      {trim_pixel_changes:8}");
    println!("  typography drift:        {typography_drift_px:8.1} px");
    println!("  a11y findings:           {a11y_issues:8}");
    println!("  clean-scene rebuilds:    {clean_scene_rebuilds:8}");
    println!(
        "  steady allocations:      {steady_allocations:8} over {STEADY_FRAMES} frames (counting allocator; retained frame matches: {steady_matches})"
    );
    println!("  unsupported GPU cmds:    {unsupported_gpu_commands:8} {unsupported_kinds:?} (vieww_gpu::Planner, {} frames)", FRAMES + 1);
    println!();
    if report.passed {
        println!("  VERDICT: PASS — every clause of the standard held");
    } else {
        println!(
            "  VERDICT: FAIL — {} clause(s) violated:",
            report.violations.len()
        );
        for violation in &report.violations {
            println!("    - {violation}");
        }
    }

    std::fs::write(
        out.join("vieww-standard.json"),
        format!(
            "{{\n  \"startup_ms\": {startup_ms:.3},\n  \"worst_frame_ms\": {worst_frame_ms:.3},\n  \"p95_frame_ms\": {p95_frame_ms:.3},\n  \"animation_latency_intervals\": {animation_latency_intervals},\n  \"input_latency_intervals\": {input_latency_intervals},\n  \"pixel_drift\": {pixel_drift},\n  \"trim_pixel_changes\": {trim_pixel_changes},\n  \"typography_drift_px\": {typography_drift_px:.3},\n  \"a11y_issues\": {a11y_issues},\n  \"clean_scene_rebuilds\": {clean_scene_rebuilds},\n  \"steady_allocations\": {steady_allocations},\n  \"steady_frames\": {STEADY_FRAMES},\n  \"unsupported_gpu_commands\": {unsupported_gpu_commands},\n  \"allocation_counter_installed\": true,\n  \"passed\": {}\n}}\n",
            report.passed
        ),
    )?;
    std::fs::write(out.join("vieww-standard.txt"), format!("{report:#?}"))?;

    if report.passed {
        Ok(())
    } else {
        Err(format!(
            "the Vieww standard was violated: {}",
            report
                .violations
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
        .into())
    }
}

/// Shape the standard paragraph through a fresh store — the determinism
/// question is whether two independent stores agree, run for run.
fn shape_paragraph(text: &str) -> Paragraph {
    let mut store = FontStore::with_system_fallback();
    Paragraph::layout(
        &mut store,
        &[TextSpan::new(text.to_owned(), TextStyle::new(14.0))],
        f32::INFINITY,
    )
}

/// The maximum disagreement between two shapings, in pixels: run positions
/// compared pairwise. Zero is the only acceptable answer.
fn typographic_drift(a: &Paragraph, b: &Paragraph) -> f32 {
    let runs_a = a.runs();
    let runs_b = b.runs();
    if runs_a.len() != runs_b.len() {
        return f32::MAX;
    }
    let mut worst = 0.0_f32;
    for (run_a, run_b) in runs_a.iter().zip(runs_b.iter()) {
        if run_a.glyphs.len() != run_b.glyphs.len() {
            return f32::MAX;
        }
        for (glyph_a, glyph_b) in run_a.glyphs.iter().zip(run_b.glyphs.iter()) {
            worst = worst
                .max((glyph_a.offset.dx - glyph_b.offset.dx).abs())
                .max((glyph_a.offset.dy - glyph_b.offset.dy).abs());
        }
    }
    worst
}

fn unsked<'a>(
    map: &'a mut std::collections::BTreeMap<&'static str, u64>,
    key: &'static str,
) -> &'a mut u64 {
    map.entry(key).or_insert(0)
}

fn count_differing_bytes(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
}
