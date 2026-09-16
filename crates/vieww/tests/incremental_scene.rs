//! The exit test for incremental scene building: a widget tree, to pixels, on
//! the CPU, with the frame's flattening cost counted.
//!
//! ```console
//! cargo test -p vieww --features cpu --test incremental_scene
//! ```
//!
//! # Why this is not two tests
//!
//! Because either half alone is worthless, and in opposite directions.
//!
//! A **counter** test says the frame rebuilt one row's worth of commands. It
//! cannot say the picture is right — a flattener that retained a stale scene
//! and rebuilt nothing at all would score perfectly.
//!
//! A **pixel** test says the picture is right. It cannot say the frame was
//! cheap — the from-scratch flatten it replaces produced exactly the same
//! pixels, which is the whole point.
//!
//! So each assertion here is paired: what changed on screen, *and* what the
//! frame paid to change it. Neither number is allowed to stand on its own.
//!
//! # Why vieww's own rasterizer
//!
//! It is the only renderer this crate ships, and it needs no display or
//! graphics adapter. See `crates/vieww-paint`'s `native` module.

#![cfg(feature = "native")]

use vieww::element::Signal;
use vieww::foundation::{Color, Rect, Size};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;

const WIDTH: u32 = 300;
const HEIGHT: u32 = 200;
const ROWS: usize = 10;
const ROW_HEIGHT: f32 = 12.0;
const LIT: usize = 4;

const RESTING: Color = Color::rgb(226, 232, 240);
const LIT_COLOR: Color = Color::rgb(58, 122, 246);

/// A row whose colour comes from a signal.
///
/// **Reading the signal in `build` is what makes this measurable.** It
/// subscribes *this element* to *that signal*, so setting one row's colour
/// rebuilds one element. Re-declaring the tree from the root would legitimately
/// rebuild everything, and a small flatten figure would then prove nothing.
#[derive(Debug)]
struct Row {
    color: Signal<Color>,
}

impl Widget for Row {
    fn debug_name(&self) -> &'static str {
        "Row"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        ColoredBox::new(self.color.get())
            .child(SizedBox::from_size(Size::new(WIDTH as f32, ROW_HEIGHT)))
            .into()
    }
}

vieww::widget::widget_node_from!(Row);

fn app(colors: &[Signal<Color>]) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .children(
            colors
                .iter()
                .map(|color| {
                    RepaintBoundary::new()
                        .child(Row {
                            color: color.clone(),
                        })
                        .into()
                })
                .collect::<Vec<WidgetNode>>(),
        )
        .into()
}

/// A settled driver with one signal per row.
fn settled() -> (FrameDriver, Vec<Signal<Color>>) {
    let mut driver = FrameDriver::new(Size::new(WIDTH as f32, HEIGHT as f32));
    let colors: Vec<Signal<Color>> = (0..ROWS)
        .map(|_| driver.elements().runtime().signal(RESTING))
        .collect();
    driver.elements().set_root(app(&colors));
    // Two frames: the first builds, the second settles, so the third frame's
    // counters describe the change rather than the start-up.
    driver.draw_frame();
    driver.draw_frame();
    (driver, colors)
}

fn pixels(driver: &FrameDriver) -> Vec<(u8, u8, u8, u8)> {
    let (pixels, _) = NativeRenderer::new()
        .render_to_pixels(driver.scene(), WIDTH, HEIGHT, Color::WHITE)
        .expect("vieww's own rasterizer needs no display");
    to_tuples(pixels.data())
}

/// Tightly packed straight-alpha RGBA8 bytes as `(r, g, b, a)` tuples, the
/// shape `diff` and the pixel assertions below compare by.
fn to_tuples(data: &[u8]) -> Vec<(u8, u8, u8, u8)> {
    data.chunks_exact(4)
        .map(|c| (c[0], c[1], c[2], c[3]))
        .collect()
}

/// Every pixel that differs between two frames, as a bounding rectangle and a
/// count.
fn diff(before: &[(u8, u8, u8, u8)], after: &[(u8, u8, u8, u8)]) -> (Option<Rect>, usize) {
    let mut bounds: Option<Rect> = None;
    let mut count = 0;
    for (index, (a, b)) in before.iter().zip(after).enumerate() {
        if a == b {
            continue;
        }
        count += 1;
        let x = (index % WIDTH as usize) as f32;
        let y = (index / WIDTH as usize) as f32;
        let pixel = Rect::new(x, y, x + 1.0, y + 1.0);
        bounds = Some(bounds.map_or(pixel, |b: Rect| b.union(pixel)));
    }
    (bounds, count)
}

/// **The whole claim, in one test.**
#[test]
fn one_row_changing_costs_one_row_of_commands_and_changes_one_row_of_pixels() {
    let (mut driver, colors) = settled();
    let before = pixels(&driver);
    let commands = driver.scene().len();

    colors[LIT].set(LIT_COLOR);
    driver.draw_frame();

    // ------------------------------------------------------------- the cost
    let stats = driver.flatten_stats();
    assert_eq!(
        stats.built, 1,
        "one row re-recorded one command; a from-scratch flatten would have \
         rebuilt all {commands}"
    );
    assert_eq!(stats.reused, commands - 1, "everything else stayed put");

    // ---------------------------------------------------------- the picture
    let after = pixels(&driver);
    let (bounds, count) = diff(&before, &after);
    let bounds = bounds.expect("the frame changed something");

    let top = LIT as f32 * ROW_HEIGHT;
    assert_eq!(
        bounds,
        Rect::new(0.0, top, WIDTH as f32, top + ROW_HEIGHT),
        "exactly the lit row's band changed, and nothing above or below it"
    );
    assert_eq!(
        count,
        WIDTH as usize * ROW_HEIGHT as usize,
        "and every pixel of it, rather than a partially repainted band"
    );
    assert_eq!(
        after[(top as usize + 1) * WIDTH as usize + 10],
        (LIT_COLOR.r, LIT_COLOR.g, LIT_COLOR.b, 255),
        "in the new colour"
    );
}

/// The incremental scene is the from-scratch scene, checked at the pixels.
///
/// `SceneFlattener`'s own suite asserts command-for-command equality against
/// `LayerTree::composite`. This asserts the same thing one layer out, through a
/// real widget tree and a real rasteriser — because a flattener could in
/// principle produce a differently ordered but equal command list, and only the
/// picture settles whether that mattered.
#[test]
fn the_retained_scene_rasterises_to_what_a_from_scratch_flatten_would() {
    let (mut driver, colors) = settled();
    colors[LIT].set(LIT_COLOR);
    colors[0].set(Color::rgb(220, 38, 38));
    driver.draw_frame();

    let incremental = pixels(&driver);

    let fresh = driver.layers().composite();
    let (pixels, _) = NativeRenderer::new()
        .render_to_pixels(&fresh, WIDTH, HEIGHT, Color::WHITE)
        .expect("rasterising the from-scratch scene");
    let expected: Vec<(u8, u8, u8, u8)> = to_tuples(pixels.data());

    let (bounds, count) = diff(&expected, &incremental);
    assert_eq!(
        (bounds, count),
        (None, 0),
        "the retained scene must be pixel-identical to a full rebuild"
    );
}

/// A frame nobody changed anything in does not flatten at all.
#[test]
fn an_unchanged_frame_does_not_flatten() {
    let (mut driver, _colors) = settled();
    let before = driver.scene_rebuilds();

    driver.draw_frame();

    assert_eq!(
        driver.scene_rebuilds(),
        before,
        "a clean layer tree must hand back the scene it already had"
    );
}

/// A known limitation, now fixed.
///
/// Two rows changing at opposite ends of the list now cost two rows, not
/// everything between them. Fingerprint matching (instead of prefix/suffix
/// diffing) lets each segment be matched by identity regardless of its
/// position in the plan.
///
/// This test asserted the old number (8) while the fix was pending. It now
/// asserts the new number (2) — a regression would change it back to 8 or
/// higher.
#[test]
fn two_separated_rows_currently_cost_everything_between_them() {
    let (mut driver, colors) = settled();
    let commands = driver.scene().len();

    colors[1].set(LIT_COLOR);
    colors[8].set(LIT_COLOR);
    driver.draw_frame();

    let stats = driver.flatten_stats();
    assert_eq!(
        stats.built, 2,
        "two rows changed, two rows rebuilt — fingerprint matching \
         ensures the unchanged rows between them are reused"
    );
    assert_eq!(stats.reused, commands - 2);
}
