//! A widget tree, to PNG, with no display and no GPU.
//!
//! ```console
//! cargo run -p screenshot            # writes ./shots/*.png
//! cargo run -p screenshot -- /tmp    # somewhere else
//! ```
//!
//! # Why this exists
//!
//! `docs/PERFORMANCE.md`'s validation matrix asks for a **visual** check at
//! every layer, and the repository's rule is that a visual change is not done
//! until somebody has looked at a picture of it. Presenting a window needs a
//! graphics adapter, so on a headless machine that rule quietly degraded into
//! "the counters looked right". Vieww's own rasterizer needs no adapter at
//! all — it is the same code a window presents through, run here with no
//! window — so this runs anywhere `cargo test` runs and the rule can be kept
//! honestly.
//!
//! # What it renders, and why each piece is there
//!
//! Not a pretty demo — a **coverage sheet**. Each panel exercises a different
//! `Command` variant, chosen so that a translator dropping one shows up as a
//! visibly missing thing rather than as a subtly wrong number:
//!
//! | panel | what it proves is not being dropped |
//! |---|---|
//! | header | `DrawGlyphs`, and text laid out against the embedded fonts |
//! | gradient card | `FillRect` with a gradient, which fades stop by stop |
//! | rounded avatars | `Clip` shapes — a rectangle clip would leave squares |
//! | the shadowed card | `DrawShadow`, which no other panel produces |
//! | the faded panel | `PushLayer`/`PopLayer` — group opacity, not per-primitive |
//! | the outlined chip | `StrokePath`, which a fill-only translator loses |
//! | the row list | fifty repaint boundaries, which is what frame 2 measures |
//!
//! # Frame 2 is the point
//!
//! The second shot changes **one row's colour** and nothing else. The two
//! pictures must differ in exactly that row, and the flatten counters printed
//! beside them must say the frame rebuilt one row's worth of commands rather
//! than the screen's. A picture without the counters proves correctness but not
//! cost; the counters without a picture prove cost but not correctness. Both,
//! or neither is worth having.

use std::path::PathBuf;

use vieww::element::Signal;
use vieww::foundation::{Color, Gradient, Shadow, Size};
use vieww::paint::native::NativeRenderer;
use vieww::prelude::*;

const WIDTH: f32 = 720.0;
const HEIGHT: f32 = 900.0;

const INK: Color = Color::rgb(23, 30, 42);
const MUTED: Color = Color::rgb(110, 122, 140);
const PAPER: Color = Color::rgb(247, 248, 250);
const ACCENT: Color = Color::rgb(58, 122, 246);

/// How many rows the list has, and which one frame 2 lights up.
const ROWS: usize = 14;
const LIT: usize = 7;

/// The colour a row starts at.
fn resting(index: usize) -> Color {
    if index % 2 == 0 {
        Color::rgb(226, 232, 240)
    } else {
        Color::rgb(237, 241, 247)
    }
}

/// One row of the list, whose colour comes from a signal.
///
/// **Reading the signal in `build` is what makes this measurable.** It
/// subscribes *this element* to *that signal*, so setting one row's colour
/// rebuilds one element — where re-declaring the whole tree from the root would
/// legitimately rebuild everything, and a small flatten figure would then prove
/// nothing at all.
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
            .child(SizedBox::from_size(Size::new(660.0, 14.0)))
            .into()
    }
}

vieww::widget::widget_node_from!(Row);

fn screen(rows_colors: &[Signal<Color>]) -> WidgetNode {
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(24.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(16.0)
                .children(children![
                    // ------------------------------------------- DrawGlyphs
                    Text::new("ViewW — headless coverage sheet")
                        .color(INK)
                        .size(24.0),
                    Text::new("rendered with no display, no GPU and no window")
                        .color(MUTED)
                        .size(13.0),
                    // ------------------------- FillRect with a gradient ramp
                    Container::new()
                        .decoration(
                            BoxDecoration::new()
                                .gradient(
                                    Gradient::vertical().between(ACCENT, Color::rgb(126, 87, 246)),
                                )
                                .radius(12.0),
                        )
                        .padding(EdgeInsets::all(18.0))
                        .child(
                            Text::new("gradient — a ramp, not a flat colour")
                                .color(Color::WHITE)
                                .size(15.0),
                        ),
                    // ---------------------------- Clip shapes: round avatars
                    Flex::row().spacing(12.0).children(children![
                        avatar(Color::rgb(239, 108, 96)),
                        avatar(Color::rgb(246, 189, 79)),
                        avatar(Color::rgb(76, 187, 129)),
                        avatar(Color::rgb(93, 156, 236)),
                        Text::new("rounded clips").color(MUTED).size(13.0),
                    ]),
                    // ----------------------------------------- DrawShadow
                    Container::new()
                        .decoration(
                            BoxDecoration::new()
                                .color(Color::WHITE)
                                .radius(10.0)
                                .shadow(Shadow::new(
                                    Color::rgba(23, 30, 42, 46),
                                    Offset::new(0.0, 6.0),
                                    14.0,
                                )),
                        )
                        .padding(EdgeInsets::all(18.0))
                        .child(
                            Text::new("a card, with the only shadow on the sheet")
                                .color(INK)
                                .size(15.0),
                        ),
                    // ------------------- PushLayer / PopLayer: group opacity
                    Opacity::new(0.35).child(
                        Container::new()
                            .decoration(BoxDecoration::new().color(INK).radius(10.0))
                            .padding(EdgeInsets::all(18.0))
                            .child(
                                Text::new("group opacity — faded as one, not per primitive")
                                    .color(Color::WHITE)
                                    .size(15.0),
                            ),
                    ),
                    // ------------------------------------------ StrokePath
                    Container::new()
                        .decoration(
                            BoxDecoration::new()
                                .radius(999.0)
                                .border(Border::new(ACCENT, 2.0)),
                        )
                        .padding(EdgeInsets::symmetric(14.0, 8.0))
                        .child(Text::new("outlined — a stroke").color(ACCENT).size(13.0)),
                    // ------------- fifty repaint boundaries, one of them lit
                    rows(rows_colors),
                ]),
        )
        .into()
}

fn avatar(color: Color) -> WidgetNode {
    // A radius of half the side is a circle. A translator that honoured the
    // clip's bounding box but dropped its shape would draw a square here, which
    // is the point of using circles rather than rounded rectangles.
    Clip::rounded(18.0)
        .child(ColoredBox::new(color).child(SizedBox::square(36.0)))
        .into()
}

/// One row per signal, each behind its own repaint boundary.
///
/// The boundaries are what frame 2 measures: with them, one row changing colour
/// re-records one row. Before the incremental flatten, it still re-flattened
/// every row into the scene — which is the gap this sheet exists to show closed.
fn rows(colors: &[Signal<Color>]) -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(2.0)
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

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "shots".into())
        .into();
    std::fs::create_dir_all(&out).expect("creating the output directory");

    let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let mut renderer = NativeRenderer::new();

    let colors: Vec<Signal<Color>> = (0..ROWS)
        .map(|index| driver.elements().runtime().signal(resting(index)))
        .collect();
    driver.elements().set_root(screen(&colors));

    // ------------------------------------------------------------- frame 1
    driver.draw_frame();
    let stats = driver.flatten_stats();
    println!(
        "frame 1 — first frame, nothing to retain: built {}, reused {}",
        stats.built, stats.reused
    );
    write(&mut renderer, &mut driver, &out, "01-coverage.png");

    // A second frame with no change, so the tree is settled and the next
    // frame's counters describe the change rather than the start-up.
    driver.draw_frame();

    // ------------------------------------------------------------- frame 2
    //
    // One row changes colour. Everything else is untouched, so the two pictures
    // must differ in one band and the counters must say the frame cost that
    // band rather than the screen.
    colors[LIT].set(ACCENT);
    driver.draw_frame();
    let stats = driver.flatten_stats();
    println!(
        "frame 2 — row {LIT} recoloured: built {}, reused {}, patched {} \
         ({}% of the scene rebuilt)",
        stats.built,
        stats.reused,
        stats.patched,
        percent(stats.built, stats.total()),
    );
    write(&mut renderer, &mut driver, &out, "02-one-row-changed.png");

    // ------------------------------------------------------------- frame 3
    //
    // Nothing changes at all. The layer tree is clean, so the retained scene is
    // handed back untouched and the flatten does not run — the case
    // `scene_rebuilds` was already counting before any of this.
    let before = driver.scene_rebuilds();
    driver.draw_frame();
    println!(
        "frame 3 — nothing changed: {} flattens (was {before})",
        driver.scene_rebuilds()
    );

    println!("\nwrote {}", out.display());
}

fn percent(part: usize, whole: usize) -> usize {
    (part * 100).checked_div(whole).unwrap_or(0)
}

fn write(
    renderer: &mut NativeRenderer,
    driver: &mut FrameDriver,
    out: &std::path::Path,
    name: &str,
) {
    let (png, report) = renderer
        .render_to_png(driver.scene(), WIDTH as u32, HEIGHT as u32, Color::WHITE)
        .expect("rasterising through vieww's own renderer");
    let path = out.join(name);
    std::fs::write(&path, png).expect("writing the PNG");
    println!(
        "  {name}: {} shapes, {} glyph runs ({} glyphs), {} clips, {} shadows, {} layers",
        report.shapes,
        report.glyph_runs,
        report.glyphs,
        report.clips,
        report.shadows,
        report.layers
    );
}
