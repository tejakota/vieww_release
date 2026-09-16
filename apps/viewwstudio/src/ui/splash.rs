//! The launch splash: the application's own mark, drawn and animated, over the
//! chrome while the studio finishes waking up.
//!
//! # What was wrong with the first version, because all three are worth naming
//!
//! **It never appeared.** `build` read a `Signal<Option<Instant>>` for the
//! start time and `Instant::now()` for the present, and a signal that never
//! changes never marks its reader pending — so the element built exactly once,
//! at zero elapsed, where every reveal is still at `0.0`, and stayed there.
//! Every launch drew a fully transparent overlay for as long as the window was
//! open. The hook in `main.rs` that was supposed to keep frames coming was
//! commented out, and would not have helped if it had not been: a frame is not
//! a rebuild, and nothing was going to rebuild an element whose signals had not
//! moved. (The commented-out body called `request_close`, which would have quit
//! the application every frame instead.)
//!
//! **The panels were not panels.** Each was a `Container` carrying both the
//! colour and a padding — and a container paints its background across its
//! padding, so what was described as a 40%-wide panel at a centre was in fact a
//! rounded rectangle reaching from the mark's top-left out to that centre. Two
//! of them, one over the other, over the ground.
//!
//! **It did not fit.** The mark was a fixed 360 points at a fixed 180 from the
//! top, the wordmark 72 point below that, and a version line pushed down
//! another 160 — 920 points of composition, taller than the 900-point window it
//! was measured against and 240 past the 679-point one the studio is reviewed
//! at. It printed `RenderColumn overflowed by 117px` on every launch.
//!
//! # How it works now
//!
//! One [`Animated`] from `0.0` to `1.0` over [`SPLASH_DURATION`], started on
//! mount by `from(0.0)`. That is the framework's own entrance idiom and it
//! fixes the first bug at the root: the animation lives in the element's state,
//! so it advances itself, rebuilds this subtree on every frame it moves, and
//! keeps `FrameDriver::is_animating` true for exactly as long as it is running
//! — no signal writes during build, no per-frame hook in `main.rs`, no wall
//! clock, and a deterministic picture at whatever timestamp a headless harness
//! asks for.
//!
//! Everything else is read off that one number: each element has a window in
//! milliseconds, `reveal` turns the master clock into that element's own eased
//! `0..=1`, and the last 350 ms fade the whole overlay off the chrome that has
//! been sitting behind it, fully drawn, the entire time.
//!
//! # The mark
//!
//! The same two overlapping rounded panels `examples/icon.rs` draws, in the
//! same fractions of the same square, in the same three colours — so the splash
//! *is* the icon, animated, and a change to one is visibly a change to the
//! other. Sized from the window rather than fixed, so it is a mark on a laptop
//! and a mark on a 5K display.
//!
//! `examples/icon.rs`: ../../../../examples/icon.rs

use std::time::Duration;

use vieww_foundation::{Alignment, Color, Constraints, Offset, Size};
use vieww_widget::prelude::*;
use vieww_widget::{
    widget_node_from, Animated, Container, GestureDetector, LayoutBuilder, Positioned, SizedBox,
    Stack, StackFit, Transformed,
};

use crate::state::Studio;
use crate::ui::chrome::{label, label_bold};

/// How long the splash plays before it takes itself off the screen.
///
/// Long enough for the reveal to read, short enough that somebody who opened
/// the studio to fix one line is not waiting on an animation. A click ends it
/// early — see `overlay` — which is what makes the length a matter of taste
/// rather than a tax.
pub const SPLASH_DURATION: Duration = Duration::from_millis(1900);

/// The same length in milliseconds, which is the unit the reveal windows are in.
const DURATION_MS: f32 = 1900.0;

/// The page behind the mark.
///
/// A shade under the icon's own ground rather than equal to it, because the
/// mark is a rounded square and a rounded square drawn in the colour of the
/// page it is on is not a square at all — it is two panels floating in the
/// dark. One step is enough to give it an edge without turning the backdrop
/// into a second surface.
const BACKDROP: Color = Color::rgba(0x0E, 0x10, 0x13, 0xFF);

/// The wordmark, the tagline and the version line, in that order of loudness.
const INK: Color = Color::rgba(0xE8, 0xEA, 0xF0, 0xFF);
const INK_MUTED: Color = Color::rgba(0x9A, 0xA2, 0xB1, 0xFF);
const INK_FAINT: Color = Color::rgba(0x6B, 0x73, 0x83, 0xFF);

/// Reveal windows, in milliseconds from the start.
///
/// Overlapping on purpose: the editor and the preview land together, the
/// preview a little behind, so the relationship the two panes have on screen —
/// preview drawn over editor — reads in the animation too.
const T_MARK: (f32, f32) = (0.0, 420.0);
const T_EDITOR: (f32, f32) = (180.0, 620.0);
const T_PREVIEW: (f32, f32) = (320.0, 780.0);
const T_WORDMARK: (f32, f32) = (520.0, 900.0);
const T_TAGLINE: (f32, f32) = (660.0, 1040.0);
const T_VERSION: (f32, f32) = (820.0, 1180.0);
/// The exit. The overlay fades away rather than cutting, so the studio behind
/// it is uncovered instead of appearing.
///
/// # The hold between the last reveal and this used to be dead air
///
/// `T_VERSION` finished at 1180 ms and the exit began at 2050 ms, over a total
/// of 2400 ms. Rendered at 25 fps and diffed frame against frame, twenty-two
/// consecutive frames — **870 ms, a third of the splash** — were byte-identical
/// to each other: a completely static picture, held, while somebody waited to
/// get to their editor.
///
/// A still frame that long does not read as a pause. It reads as a hang, and it
/// reads as one *especially* on a launch screen, because a launch screen is
/// exactly where a person is already asking themselves whether the application
/// has finished starting. The fix is not to animate the hold — something moving
/// with nothing to say is worse — but to not have one: the exit now begins
/// 300 ms after the last line lands, which is long enough for the composition
/// to be read as a whole and short enough that nothing is being waited on.
const T_EXIT: (f32, f32) = (1480.0, DURATION_MS);

/// Where `now_ms` sits inside `window`, eased, clamped to `0..=1` outside it.
///
/// Decelerating: everything here is arriving at a rest position, and a reveal
/// that eases in as well begins by looking like a dropped frame.
fn reveal(now_ms: f32, window: (f32, f32)) -> f32 {
    let (start, end) = window;
    if now_ms <= start {
        return 0.0;
    }
    if now_ms >= end {
        return 1.0;
    }
    let t = (now_ms - start) / (end - start);
    1.0 - (1.0 - t).powi(3)
}

#[derive(Debug)]
pub struct Splash {
    pub studio: Studio,
}

impl Widget for Splash {
    fn debug_name(&self) -> &'static str {
        "Splash"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Not launching, or already dismissed: nothing in the tree at all. The
        // shell mounts this unconditionally, so this branch is what keeps a
        // studio that has been open for an hour from carrying a splash.
        if !self.studio.splash.get() {
            return SizedBox::shrink().into();
        }

        let studio = self.studio.clone();
        // **The whole clock, in one widget's state.** `from(0.0)` is what makes
        // this an entrance rather than a control settling into place: a freshly
        // mounted `Animated` starts where it is told and travels to its target,
        // ticking itself and rebuilding what it built on every frame it moves.
        Animated::new(1.0)
            .from(0.0)
            .duration(SPLASH_DURATION)
            // Linear, because the easing belongs to each reveal rather than to
            // the master clock: a curve here would bend six windows that have
            // already been placed against real milliseconds.
            .curve(vieww_animation::Curve::Linear)
            .build(move |t| overlay(t, &studio))
            .into()
    }
}

widget_node_from!(Splash);

/// One frame of the splash, at `t` (`0..=1`) through [`SPLASH_DURATION`].
fn overlay(t: f32, studio: &Studio) -> WidgetNode {
    let now = t * DURATION_MS;
    // Done. Nothing drawn, nothing hit-tested, and — because `Animated` has
    // stopped moving — no further frames requested on the splash's account.
    if now >= DURATION_MS {
        return SizedBox::shrink().into();
    }

    let exit = reveal(now, T_EXIT);
    let veil = (1.0 - exit).clamp(0.0, 1.0);
    let dismiss = studio.clone();

    // A `Stack` with one full-bleed `Positioned` in it, which is how
    // `welcome.rs` covers the window and the one shape that does not depend on
    // what constraints the shell's own stack happens to pass down.
    Stack::new()
        .fit(StackFit::Expand)
        .alignment(Alignment::CENTER)
        .children(children![Positioned::new()
            .left(0.0)
            .right(0.0)
            .top(0.0)
            .bottom(0.0)
            .child(
                Opacity::new(veil).child(
                    // **It swallows the click, and the click ends it.** A
                    // splash that leaks a press to the chrome underneath opens
                    // whatever the pointer happened to be over; one that cannot
                    // be skipped is a delay with a picture on it. One gesture
                    // detector answers both.
                    GestureDetector::new()
                        .on_tap_down(move |_| dismiss.splash.set(false))
                        .child(Clip::rect().child(LayoutBuilder::new(
                            move |constraints: Constraints| composition(now, exit, constraints)
                        )))
                )
            )])
        .into()
}

/// The ground, the mark and the three lines, sized to the window they are in.
///
/// Every number here is a fraction of `side`, and `side` is a fraction of the
/// window — so this is the same picture on a 1366×679 laptop and a 5120×2880
/// display, and there is no size at which it is taller than the window it is in.
fn composition(now: f32, exit: f32, constraints: Constraints) -> WidgetNode {
    // The fallbacks keep the arithmetic finite if this is ever mounted
    // somewhere unbounded; inside a full-bleed `Positioned` it is handed the
    // window.
    let width = if constraints.max_width.is_finite() {
        constraints.max_width
    } else {
        crate::WINDOW.width
    };
    let height = if constraints.max_height.is_finite() {
        constraints.max_height
    } else {
        crate::WINDOW.height
    };

    // Under a third of the shorter edge, bounded at both ends: large enough to
    // be the thing you are looking at, never so large that the lines under it
    // leave the window.
    let side = (width.min(height) * 0.30).clamp(96.0, 300.0);
    let gap = side * 0.16;

    let column = Flex::column()
        .main_axis_size(MainAxisSize::Min)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(children![
            grown(
                reveal(now, T_MARK),
                0.78,
                Size::new(side, side),
                crate::ui::brand::revealed(side, reveal(now, T_EDITOR), reveal(now, T_PREVIEW))
            ),
            SizedBox::from_size(Size::new(0.0, gap)),
            rising(
                reveal(now, T_WORDMARK),
                label_bold("vieww Studio", (side * 0.20).clamp(20.0, 58.0), INK),
            ),
            SizedBox::from_size(Size::new(0.0, side * 0.055)),
            rising(
                reveal(now, T_TAGLINE),
                label(
                    "a code editor and device-framed preview for vieww screens",
                    (side * 0.062).clamp(11.0, 17.0),
                    INK_MUTED,
                ),
            ),
            SizedBox::from_size(Size::new(0.0, side * 0.10)),
            rising(
                reveal(now, T_VERSION),
                label(
                    &format!("version {}", env!("CARGO_PKG_VERSION")),
                    (side * 0.045).clamp(9.5, 13.0),
                    INK_FAINT,
                ),
            ),
        ]);

    // **The exit is a departure, not a dissolve.**
    //
    // The overlay's own opacity is already falling (`veil`, in `overlay`); on
    // its own that is a cross-fade, and a cross-fade between two full screens
    // reads as a smear rather than as one thing giving way to another. Lifting
    // and enlarging the composition very slightly as it goes gives the fade a
    // direction — the mark moves *off*, toward the viewer, and the studio is
    // uncovered behind it rather than emerging through it.
    //
    // The numbers are small on purpose: 1.04 and eighteen points over 420 ms.
    // Large enough to be felt as movement, small enough that nobody watching
    // could tell you what moved, which is the difference between a transition
    // and an effect.
    let lift = -18.0 * exit;
    let swell = 1.0 + 0.04 * exit;
    let composed = Transformed::translate(Offset::new(0.0, lift))
        .child(Transformed::scale(swell, swell).child(column));

    Container::new()
        .color(BACKDROP)
        .alignment(Alignment::CENTER)
        .child(composed)
        .into()
}

/// `child` faded and scaled up into place from `from`, about its own centre.
fn grown(t: f32, from: f32, size: Size, child: WidgetNode) -> WidgetNode {
    let scale = from + (1.0 - from) * t;
    let inset = Offset::new(
        (1.0 - scale) * size.width / 2.0,
        (1.0 - scale) * size.height / 2.0,
    );

    // Sized explicitly, because a transform changes drawing and not layout: the
    // column has to reserve the mark's whole square whatever the scale is
    // doing, or the lines beneath it would slide up as it grew.
    SizedBox::from_size(size)
        .child(Opacity::new(t.clamp(0.0, 1.0)).child(
            Transformed::translate(inset).child(Transformed::scale(scale, scale).child(child)),
        ))
        .into()
}

/// `child` faded up and arriving from a few points below.
///
/// The rise is small on purpose. A line that travels far enough to notice reads
/// as a slide, and three of them in sequence read as a carousel.
fn rising(t: f32, child: impl Into<WidgetNode>) -> WidgetNode {
    Transformed::translate(Offset::new(0.0, (1.0 - t) * 10.0))
        .child(Opacity::new(t.clamp(0.0, 1.0)).child(child))
        .into()
}
