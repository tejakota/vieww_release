//! Tier 4 — interaction: what one changed thing actually costs.
//!
//! # The question the other tiers cannot answer
//!
//! Every other fixture measures a frame drawn from nothing. That is the right
//! measurement for "can this interface be drawn at all", and it is the wrong
//! one for "does this interface feel alive", because an application almost
//! never draws from nothing. It changes one row's colour, opens one panel,
//! moves one caret — and then repaints.
//!
//! So each fixture here builds a screen, settles it, changes exactly one
//! thing, and reports three numbers: how much of the surface the framework
//! says was damaged, what a full repaint costs, and what a *retained* repaint
//! costs — the previous frame's pixels kept, only the damaged regions cleared
//! and redrawn.
//!
//! It also asserts, every run, that the two produce **the same bytes**. A
//! partial repaint that is fast and subtly wrong is worse than a slow one,
//! and the fault it produces — one stale rectangle after one interaction —
//! is close to invisible in review. `vieww-paint`'s own
//! `tests/retained_equivalence.rs` covers the geometric edge cases; this
//! covers them at the scale of a real screen, where the clip masks, the
//! shadows and the layer tree are all in play at once.

use std::time::Instant;

use vieww_element::Signal;
use vieww_foundation::{Color, EdgeInsets, Offset, Shadow, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Clip, RepaintBoundary};

use crate::primitives::{ACCENT, INK, MUTED, PAPER};

const SIZE: Size = Size::new(840.0, 560.0);

/// One list row, reading its own colour signal.
///
/// Behind a `RepaintBoundary` at the call site, which is what lets the layer
/// tree re-record this row alone instead of the list — the framework half of
/// the same idea the retained surface implements in pixels.
#[derive(Debug)]
struct Row {
    label: String,
    tint: Signal<Color>,
}

impl Widget for Row {
    fn debug_name(&self) -> &'static str {
        "Row"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .color(self.tint.get())
            .padding(EdgeInsets::all(12.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(10.0)
                    .push(Container::new().color(ACCENT).radius(5.0).size(20.0, 20.0))
                    .push(Text::new(self.label.clone()).color(INK).size(13.0)),
            )
            .into()
    }
}

widget_node_from!(Row);

/// Ten rows, not twelve.
///
/// Twelve came to 594px of column in the 520px this screen's padding leaves —
/// a 74px overflow on every run, with the bottom row and a half painting past
/// the card that is supposed to clip them, in the fixture whose entire subject
/// is that a *partial* repaint reproduces a full one exactly. Ten leaves 10px
/// of headroom, and both damage scenarios below (rows 4 and 9) still fit.
const ROWS: usize = 10;
const RESTING: Color = Color::rgb(255, 255, 255);
const LIT: Color = Color::rgb(219, 233, 255);

fn screen(tints: &[Signal<Color>]) -> WidgetNode {
    let mut list = Flex::column().cross_axis_alignment(CrossAxisAlignment::Start);
    for (i, tint) in tints.iter().enumerate() {
        list = list.push(RepaintBoundary::new().child(Row {
            label: format!("Row {} — a selectable item in a scrolling list", i + 1),
            tint: tint.clone(),
        }));
    }

    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(14.0)
                .push(Text::new("Selection").color(INK).size(20.0).bold())
                .push(
                    Text::new("One row lights up. Nothing else moves.")
                        .color(MUTED)
                        .size(12.0),
                )
                .push(
                    // The rounded, elevated card the list lives in: a clip and
                    // a shadow, both of which a region repaint has to resolve
                    // exactly as a full one would.
                    Container::new()
                        .color(Color::WHITE)
                        .radius(14.0)
                        .shadow(Shadow::new(
                            Color::rgba(20, 30, 60, 30),
                            Offset::new(0.0, 4.0),
                            18.0,
                        ))
                        .child(Clip::rounded(14.0).child(list)),
                ),
        )
        .into()
}

/// Build the screen, settle it, light one row, and report what that cost.
pub(crate) fn run() {
    vieww_render::overflow::forget_reported();
    let (w, h) = (SIZE.width as u32, SIZE.height as u32);
    let mut driver = FrameDriver::new(SIZE);
    let runtime = driver.elements().runtime().clone();
    let tints: Vec<Signal<Color>> = (0..ROWS).map(|_| runtime.signal(RESTING)).collect();
    driver.elements().set_root(screen(&tints));

    // Two frames to settle, so the third describes the interaction rather
    // than the start-up.
    driver.draw_frame();
    driver.draw_frame();

    let mut renderer = NativeRenderer::new();
    renderer
        .render_to_pixels(driver.scene(), w, h, PAPER)
        .expect("settle");

    println!(
        "\n{:<30} {:>9} {:>11} {:>11} {:>9}  identical?",
        "interaction", "damage", "full frame", "retained", "speed-up"
    );
    println!("{}", "-".repeat(96));

    for (label, lit) in [
        ("light row 4", Some(3usize)),
        ("move the light to row 9", Some(8usize)),
        ("clear the selection", None),
        ("nothing changes at all", None),
    ] {
        for tint in &tints {
            tint.set(RESTING);
        }
        if let Some(i) = lit {
            tints[i].set(LIT);
        }
        driver.draw_frame();

        let damage = driver.damage().clone();
        let ratio = damage.covered_area() / (SIZE.width * SIZE.height);

        // Full: a renderer that has seen nothing, drawing this scene whole.
        let mut fresh = NativeRenderer::new();
        fresh
            .render_to_pixels(driver.scene(), w, h, PAPER)
            .expect("warm");
        let start = Instant::now();
        let (complete, _) = fresh
            .render_to_pixels(driver.scene(), w, h, PAPER)
            .expect("full");
        let full_ms = start.elapsed().as_secs_f64() * 1000.0;

        // Retained: the renderer that has been following along, given only
        // what changed.
        let start = Instant::now();
        let (incremental, _) = renderer
            .render_retained(driver.scene(), &damage, w, h, PAPER)
            .expect("retained");
        let retained_ms = start.elapsed().as_secs_f64() * 1000.0;

        let identical = incremental.data() == complete.data();
        println!(
            "{label:<30} {:>8.2}% {:>9.2}ms {:>9.2}ms {:>8.1}x  {}",
            ratio * 100.0,
            full_ms,
            retained_ms,
            full_ms / retained_ms.max(1e-6),
            if identical { "yes" } else { "NO — MISMATCH" }
        );
        assert!(
            identical,
            "{label}: a retained repaint did not match a full one"
        );
    }

    report_overflows("the list interaction screen");
}

/// The same question, asked of the screen the still-fixture gallery flags as
/// over budget.
///
/// # Why this exists
///
/// `23-editor-glass` renders a **full** frame in about 25 ms against a 16.7 ms
/// budget, and the gallery says so in red on every run. That is a true fact
/// about a full repaint and it was, for as long as it was the only number,
/// silently standing in for a claim nobody had checked: that the screen cannot
/// hold 60 Hz.
///
/// A live window does not repaint a glass screen from nothing when a menu item
/// highlights. It repaints the damage. So this builds that exact screen, puts
/// one signal-driven control over it, changes the control, and measures both —
/// which turns "over budget" from an open question into a measured one, in
/// either direction. The full-repaint number stays in the table because it is
/// the right alarm for a *first* frame, a resize and a theme change, all of
/// which really do redraw everything.
///
/// The equality assertion is the same one the list scenario makes and matters
/// more here: this screen has a blurred backdrop layer, real shadows and a
/// clip stack, and those are exactly the things a region repaint gets subtly
/// wrong — a shadow blurred against the edge of a damage rectangle instead of
/// the edge of the window is a few wrong pixels nobody sees in review.
pub(crate) fn run_glass() {
    vieww_render::overflow::forget_reported();
    const GLASS: Size = Size::new(1366.0, 679.0);
    let (w, h) = (GLASS.width as u32, GLASS.height as u32);

    let mut driver = FrameDriver::new(GLASS);
    let runtime = driver.elements().runtime().clone();
    let tint = runtime.signal(Color::rgba(58, 122, 246, 60));

    driver
        .elements()
        .set_root(Stack::new().fit(StackFit::Expand).children(children![
                crate::screens::editor_glass(),
                // One highlighted row of the palette, on its own repaint
                // boundary — the smallest real interaction this screen has.
                Positioned::new().left(400.0).top(140.0).child(
                    RepaintBoundary::new().child(HighlightRow {
                        tint: tint.clone(),
                    }),
                ),
            ]));
    driver.draw_frame();
    driver.draw_frame();

    let background = crate::primitives::PAPER;
    let mut renderer = NativeRenderer::new();
    renderer
        .render_to_pixels(driver.scene(), w, h, background)
        .expect("settle");

    for (label, colour) in [
        ("glass: highlight moves", Color::rgba(58, 122, 246, 140)),
        ("glass: highlight clears", Color::TRANSPARENT),
    ] {
        tint.set(colour);
        driver.draw_frame();

        let damage = driver.damage().clone();
        let ratio = damage.covered_area() / (GLASS.width * GLASS.height);

        let mut fresh = NativeRenderer::new();
        fresh
            .render_to_pixels(driver.scene(), w, h, background)
            .expect("warm");
        let start = Instant::now();
        let (complete, _) = fresh
            .render_to_pixels(driver.scene(), w, h, background)
            .expect("full");
        let full_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        let (incremental, _) = renderer
            .render_retained(driver.scene(), &damage, w, h, background)
            .expect("retained");
        let retained_ms = start.elapsed().as_secs_f64() * 1000.0;

        let identical = incremental.data() == complete.data();
        let verdict = if retained_ms <= 16.667 {
            "within 60Hz"
        } else {
            "OVER even retained"
        };
        println!(
            "{label:<30} {:>8.2}% {:>9.2}ms {:>9.2}ms {:>8.1}x  {}  {verdict}",
            ratio * 100.0,
            full_ms,
            retained_ms,
            full_ms / retained_ms.max(1e-6),
            if identical { "yes" } else { "NO — MISMATCH" }
        );
        assert!(
            identical,
            "{label}: a retained repaint did not match a full one"
        );
    }

    report_overflows("the glass interaction screen");
}

/// Say whether the scene just built laid out cleanly, and fail the run if not.
///
/// The still gallery gained this gate first (`runner.rs`); the interaction
/// suites build their own trees and so needed their own. Same reason in both
/// places: `vieww_render::overflow` reports to stderr, and stderr in the middle
/// of a tool that prints a table scrolls past looking like progress.
fn report_overflows(what: &str) {
    let count = vieww_render::overflow::reported();
    assert_eq!(
        count, 0,
        "{what} reported {count} layout overflow(s) — see the `vieww:` lines \
         above. A screen this gallery presents as a reference cannot be one \
         whose own layout does not fit."
    );
}

/// A palette row whose tint is a signal — the one thing that changes in
/// [`run_glass`].
#[derive(Debug)]
struct HighlightRow {
    tint: Signal<Color>,
}

impl Widget for HighlightRow {
    fn debug_name(&self) -> &'static str {
        "HighlightRow"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Container::new()
            .color(self.tint.get())
            .radius(6.0)
            .padding(EdgeInsets::all(10.0))
            .child(
                Text::new("Render: Run and Watch")
                    .color(Color::rgb(232, 234, 240))
                    .size(13.0),
            )
            .into()
    }
}

widget_node_from!(HighlightRow);
