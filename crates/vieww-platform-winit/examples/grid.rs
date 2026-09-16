//! Two hundred and forty-two numbered swatches in a four-wide grid, to be
//! *looked at*.
//!
//! ```console
//! # interactive: drag or use the wheel, and watch the numbers stay consecutive
//! cargo run -p vieww-platform-winit --example grid --release
//!
//! # open at the end, where the short last row is
//! cargo run -p vieww-platform-winit --example grid --release -- --bottom
//! ```
//!
//! `--bottom` exists because the most interesting claim here is the one you have
//! to scroll to, and "scroll to the bottom and look" is a step that gets skipped.
//!
//! # What this is for
//!
//! `GridView` and `ListView::variable` arrived with about twenty tests and no
//! screenshot, and this repository has already learned once what that is worth:
//! a `Radio` whose selected dot was stretched into an ellipse passed the entire
//! suite and was obvious in a phone screenshot within ten seconds. A grid is a
//! far more visual thing than a radio button.
//!
//! So the tree here is chosen to make each claim checkable **by eye**, and each
//! one is a claim a test also makes:
//!
//! - **Row-major order.** The top row reads `0 1 2 3`. A grid that filled
//!   column-major would read `0 61 122 183` and no assertion about counts would
//!   notice.
//! - **Equal shares.** Four cells across, all the same width, whatever the
//!   window is. Drag the window wider and they stay equal.
//! - **The spacing is between the cells and not around them.** Eight logical
//!   pixels each way, and no margin against the edges — the background shows
//!   through the gaps and not around the outside.
//! - **The short last row does not stretch.** 242 is not a multiple of 4, so the
//!   final row holds `240 241` at *one quarter* of the width each. This is the
//!   one that a wrong implementation gets wrong most naturally, because leaving
//!   the absent cells out entirely is the obvious way to write it and hands
//!   their width to the survivors.
//! - **Virtualisation is invisible.** Scroll from top to bottom: the numbers
//!   stay consecutive, the colour ramp stays continuous, and nothing blinks.
//!   Only about seven rows of the sixty-one exist as widgets at any moment, and
//!   the whole point is that you cannot tell.
//!
//! # The numbers are the instrument; the banding is an aid
//!
//! **The consecutive labels are what actually proves the range is right** — a
//! skipped row reads `0 1 2 3` then `8 9 10 11`, which is unambiguous and needs
//! no interpretation.
//!
//! The colour banding is for the case the numbers cannot serve: scrolling fast
//! enough that the labels blur, where a stutter or a jump shows up as a break in
//! the rhythm of the bands. It cycles every **four** rows, so it cannot
//! distinguish a fault that is an exact multiple of four rows — the numbers can,
//! and that division of labour is the point.
//!
//! Its first version cycled over twenty-eight rows and put four parts in 255
//! between adjacent rows, which looked like a gradient in the source and like a
//! flat colour on the screen. It was documented, in this file, as the thing that
//! would make a repeated row obvious. It could not have shown one. Worth
//! remembering the next time something diagnostic goes in unlooked-at: the
//! instrument was wrong in exactly the way the thing it was watching for would
//! have been.
//!
//! # Run it in release
//!
//! Debug builds of vello are between ten and thirty times slower, and a jank
//! count from one is a report on `rustc -O0`.

use vieww_element::ScrollController;
use vieww_foundation::{Color, Size};
use vieww_gestures::ScrollPhysics;
use vieww_platform_winit::App;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

use std::rc::Rc;

const SURFACE: Size = Size {
    width: 640.0,
    height: 480.0,
};

/// Deliberately **not** a multiple of [`COLUMNS`], so the last row is short.
const CELLS: usize = 242;
const COLUMNS: usize = 4;
const CELL: f32 = 72.0;
const GAP: f32 = 8.0;

/// The band across the top, and the window less that band.
const HEADER: f32 = 64.0;
const VIEWPORT: f32 = SURFACE.height - HEADER;

/// One numbered swatch, shaded by which row it is in.
///
/// The shade comes from the *row* rather than the index, so a row reads as a
/// band and the ramp down the grid is the thing that makes a skipped or repeated
/// row visible. Shading per cell would give a gradient across each row as well,
/// which is prettier and hides exactly the fault this is here to expose.
fn swatch(index: usize) -> WidgetNode {
    let row = index / COLUMNS;
    // **Four rows to a cycle, and the step is large on purpose.** The first
    // version of this ramped over twenty-eight rows, which put four parts in
    // 255 between one row and the next — five rows of it on screen were
    // indistinguishable, and a banding meant to make a repeated row obvious
    // could not have shown one. A short cycle with a visible step is worth more
    // than a long smooth one nobody can read.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "at most 3 * 56, which is inside a u8 by construction"
    )]
    let shade = ((row % 4) * 56) as u8;

    // Kept dark enough at both ends that the white label stays legible; the
    // whole range is a walk from blue towards slate rather than a full sweep.
    ColoredBox::new(Color::rgb(25 + shade / 3, 50 + shade / 4, 180 - shade / 4))
        .child(
            Center::new().child(
                Text::new(index.to_string())
                    .size(16.0)
                    .bold()
                    .color(Color::WHITE),
            ),
        )
        .into()
}

/// The grid itself, in one place.
///
/// [`Swatches`] builds it and `--bottom` asks it how long it is, and those two
/// must not be allowed to disagree about the cell size or the spacing — the
/// second would then open at an offset that is not the end.
fn grid() -> GridView {
    GridView::new(CELLS, COLUMNS, CELL, Rc::new(swatch)).spacing(GAP, GAP)
}

/// The scrolling grid.
///
/// Its own widget because the *read* of the offset has to happen here: a drag
/// then marks this element and nothing else, and the header above it does not
/// rebuild. Reading the offset in `main`'s closure instead would subscribe
/// nothing at all — a build is the only place a signal read is a subscription —
/// and the grid would never move.
#[derive(Debug)]
struct Swatches {
    scroll: ScrollController,
}

impl Widget for Swatches {
    fn debug_name(&self) -> &'static str {
        "Swatches"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // `.viewport(VIEWPORT)` is what lets the grid virtualise on the *first*
        // frame. Without it the grid gets no `ScrollMetrics` window until layout
        // has measured one, builds its conservative screenful, and is right but
        // unvirtualised for a frame. Here the number is known, so say it.
        Scrollable::vertical(self.scroll.offset())
            .viewport(VIEWPORT)
            .on_drag(self.scroll.on_drag())
            .on_drag_end(self.scroll.on_drag_end())
            // Reported out of *layout*, and the handler writes a signal. The
            // viewport only reports extents that changed, and it carries that
            // record across a rebuild through `RenderObject::adopt_reports` —
            // without which a scrolling application re-reports identical extents
            // every frame and can never go idle.
            .on_extents(self.scroll.on_extents())
            .child(grid())
            .into()
    }
}

widget_node_from!(Swatches);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let report = App::new()
        .title("vieww — 242 swatches, four across")
        .size(SURFACE)
        .background(Color::rgb(18, 18, 22))
        .run(|driver| {
            let runtime = driver.elements().runtime().clone();
            let scroll = ScrollController::new(&runtime, ScrollPhysics::android());
            // Without this the offset never advances after a fling: `Tickers` is
            // what drives the simulation, and a controller nobody attached
            // scrolls under the finger and then stops dead when it lifts.
            scroll.attach(driver.tickers());

            if wants_bottom() {
                // `resize` **before** the drag, and the order is the whole of
                // it: the offset is clamped against a maximum derived from the
                // extents, and until layout has reported any that maximum is
                // zero — so a drag applied first is clamped to nothing and the
                // window opens at the top, looking exactly like the flag being
                // ignored.
                //
                // `GridView` can answer how long it is without being laid out,
                // which is what `content_extent` is for.
                let length = grid().content_extent();
                scroll.resize(ScrollExtents::new(VIEWPORT, length));
                // A drag up is negative and scrolls into the content; the clamp
                // above turns "further than the end" into "the end".
                scroll.drag(-length);
            }

            let root = Flex::column().children(children![
                ColoredBox::new(Color::rgb(28, 28, 34)).child(
                    SizedBox::from_size(Size::new(SURFACE.width, HEADER)).child(
                        Center::new().child(
                            Text::new("row-major, equal shares, short last row")
                                .size(15.0)
                                .color(Color::rgb(190, 190, 200)),
                        ),
                    ),
                ),
                SizedBox::height(VIEWPORT).child(Swatches { scroll }),
            ]);

            // `FrameDriver::set_root`, not `driver.elements().set_root` — the
            // driver keeps the root so it can republish it under an
            // `Inherited<ViewMetrics>`, and going straight to the element tree
            // leaves it with none to republish.
            driver.set_root(root);
        })?;

    println!("{report}");
    Ok(())
}

/// `--bottom`, if it was passed.
fn wants_bottom() -> bool {
    std::env::args().skip(1).any(|arg| arg == "--bottom")
}
