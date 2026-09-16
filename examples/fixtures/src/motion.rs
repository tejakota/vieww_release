//! Tier 3 — motion, as GIFs you can actually watch.
//!
//! # Why this is not a still fixture with a `t` parameter
//!
//! Because the bug class it exists to catch does not show up in one frame.
//! A splash screen that "never appears" renders perfectly at every timestamp
//! you ask it for — the fault is that nothing ever *asks*, or that the frames
//! arrive so far apart that the animation is over before the second one
//! lands. Both look like a correct still and a blank window.
//!
//! So each motion fixture renders a whole sequence, writes it as a GIF, and
//! reports two numbers per frame: how long the frame took to rasterise, and
//! whether the picture actually changed from the one before it. A sequence of
//! identical frames is a dead animation however pretty frame one is, and
//! frames that each cost 200 ms are a dead animation too — at that rate a
//! 400 ms transition has three frames in it.
//!
//! The GIFs are the deliverable. Numbers say a frame was cheap; only the
//! picture says the motion reads.

use std::path::Path as FsPath;
use std::time::{Duration, Instant};

use vieww_foundation::{Color, EdgeInsets, Gradient, Offset, Rect, Shadow, Size};
use vieww_paint::native::NativeRenderer;
use vieww_render::FrameDriver;
use vieww_widget::prelude::*;
use vieww_widget::{Clip, Filtered, Offstage, PaintWith, Painting};

use crate::primitives::{ACCENT, INK, MINT, MUTED, PAPER, VIOLET};

/// Frames per sequence, and the interval between them. 24 frames at 40 ms is
/// just under a second — long enough to read, short enough that five of these
/// do not dominate the gallery's runtime.
const FRAMES: usize = 24;
const INTERVAL_MS: u16 = 4; // GIF delay is in hundredths of a second.

const SIZE: Size = Size::new(560.0, 360.0);

/// One animated fixture: a function from progress to a tree.
struct Motion {
    name: &'static str,
    about: &'static str,
    background: Color,
    frame: fn(f32) -> WidgetNode,
}

/// Render every motion fixture to a GIF and report what it cost.
pub(crate) fn run_all(out: &FsPath) {
    let motions = [
        Motion {
            name: "30-fade-in",
            about: "opacity 0 -> 1: the simplest reveal",
            background: PAPER,
            frame: fade_in,
        },
        Motion {
            name: "31-stagger",
            about: "five rows revealing in sequence",
            background: PAPER,
            frame: stagger,
        },
        Motion {
            name: "32-splash",
            about: "a launch splash: scale, rise, fade out",
            background: Color::rgb(14, 16, 19),
            frame: splash,
        },
        Motion {
            name: "33-sheet",
            about: "a modal sheet over a blurred backdrop",
            background: PAPER,
            frame: sheet,
        },
        Motion {
            name: "34-spinner",
            about: "a continuous sweep — transforms every frame",
            background: PAPER,
            frame: spinner,
        },
        Motion {
            name: "35-shared-element",
            about: "a card expanding into a detail view — the hard transition",
            background: PAPER,
            frame: shared_element,
        },
        Motion {
            name: "36-morph",
            about: "one vector path morphing into another, stroked and filled",
            background: Color::rgb(16, 18, 24),
            frame: morph,
        },
    ];

    println!(
        "\n{:<20} {:>7} {:>10} {:>10} {:>8} {:>7}  what it covers",
        "motion", "frames", "mean", "worst", "moving", "stall"
    );
    println!("{}", "-".repeat(118));

    for motion in &motions {
        let measured = render_sequence(motion, out);
        // Three different faults, reported as three different things.
        //
        // A sequence whose motion finishes before its last frame legitimately
        // ends on a run of identical pictures — the splash holds its finished
        // mark before it fades, the sheet settles — so `moving` alone cannot be
        // the gate, and setting it at "fewer than half the frames differ" was
        // the previous attempt to work around exactly that. `stall` is the
        // number that can be a gate, because an interior run of identical
        // frames has no legitimate version. See `longest_interior_stall`.
        let stall_ms = measured.stall * INTERVAL_MS as usize * 10;
        let flag = if measured.overflows > 0 {
            format!(
                "  <-- LAYOUT OVERFLOWS in {} of its {FRAMES} frames",
                measured.overflows
            )
        } else if measured.moving * 2 < FRAMES - 1 {
            "  <-- BARELY MOVES: the animation may not be running".to_owned()
        } else if measured.stall >= 3 {
            format!("  <-- STALLS mid-transition for {stall_ms}ms")
        } else if measured.worst.as_secs_f64() > 0.016_667 {
            "  <-- OVER BUDGET".to_owned()
        } else {
            String::new()
        };
        println!(
            "{:<20} {:>7} {:>8.2}ms {:>8.2}ms {:>6}/{:<2} {:>7}  {}{}",
            motion.name,
            FRAMES,
            measured.mean.as_secs_f64() * 1000.0,
            measured.worst.as_secs_f64() * 1000.0,
            measured.moving,
            FRAMES - 1,
            measured.stall,
            motion.about,
            flag
        );
    }
}

struct MotionResult {
    mean: Duration,
    worst: Duration,
    /// How many frames differ from the one before them. The animation is dead
    /// if this is near zero however good the stills look.
    moving: usize,
    /// The longest run of identical frames *between* two moving ones — see
    /// [`longest_interior_stall`]. This is the number that finds a hitch.
    stall: usize,
    /// Layout overflows across every frame of the sequence.
    ///
    /// Animated fixtures are where overflow hides best: a sheet that slides in
    /// is the right size in its first frame and its last, and wrong for the
    /// twenty in between. Nobody looks at frame 11 of a GIF, and the still
    /// gallery's own gate cannot see these frames at all — so they are counted
    /// here, per sequence, for the same reason and by the same mechanism.
    overflows: usize,
}

fn render_sequence(motion: &Motion, out: &FsPath) -> MotionResult {
    let (width, height) = (SIZE.width as u32, SIZE.height as u32);
    let mut renderer = NativeRenderer::new();
    let mut frames: Vec<Vec<u8>> = Vec::with_capacity(FRAMES);
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;

    vieww_render::overflow::forget_reported();
    for i in 0..FRAMES {
        let t = i as f32 / (FRAMES - 1) as f32;
        let mut driver = FrameDriver::new(SIZE);
        driver.elements().set_root((motion.frame)(t));
        driver.draw_frame();

        let start = Instant::now();
        let (pixels, _) = renderer
            .render_to_pixels(driver.scene(), width, height, motion.background)
            .expect("rendering a motion frame");
        let elapsed = start.elapsed();
        total += elapsed;
        worst = worst.max(elapsed);
        frames.push(pixels.data().to_vec());
    }

    let overflows = vieww_render::overflow::reported();
    let moving = frames.windows(2).filter(|w| w[0] != w[1]).count();
    let stall = longest_interior_stall(&frames);
    write_gif(
        &frames,
        width,
        height,
        &out.join(format!("{}.gif", motion.name)),
    );

    MotionResult {
        mean: total / FRAMES as u32,
        worst,
        moving,
        stall,
        overflows,
    }
}

/// The longest run of identical consecutive frames that has motion on **both**
/// sides of it, in frames.
///
/// # Why the trailing run does not count and the interior one does
///
/// `moving` — how many frames differ from the one before — cannot tell the two
/// apart, and they are opposite things. A sequence that finishes its motion and
/// then holds is behaving correctly: the sheet settles, the splash rests on its
/// finished mark before it fades. Counting those against it makes a
/// well-behaved animation look broken, which is what `33-sheet`'s entirely
/// honest `14/23` looked like for as long as `moving` was the only number here.
///
/// A run of identical frames with motion resuming *after* it is the opposite,
/// and there is no benign version of it: it is a hold in the middle of a
/// transition, felt as a hitch or as the application having stopped. It is
/// exactly what the studio's own launch splash turned out to contain — 870 ms
/// of held picture between its last reveal and its exit — and nothing in this
/// harness could see it, because the number it reported was the one that
/// legitimately drops on every settling animation.
///
/// So: leading and trailing runs are ignored, interior runs are the
/// measurement.
fn longest_interior_stall(frames: &[Vec<u8>]) -> usize {
    let changes: Vec<bool> = frames.windows(2).map(|w| w[0] != w[1]).collect();
    let Some(first) = changes.iter().position(|&c| c) else {
        // Nothing moved at any point. That is not a stall, it is a dead
        // animation, and `moving == 0` already says so.
        return 0;
    };
    let last = changes.iter().rposition(|&c| c).unwrap_or(first);
    let mut longest = 0;
    let mut run = 0;
    for &changed in &changes[first..=last] {
        if changed {
            longest = longest.max(run);
            run = 0;
        } else {
            run += 1;
        }
    }
    longest.max(run)
}

/// Encode a sequence of straight-alpha RGBA8 frames as an animated GIF.
///
/// GIF is one palette entry per colour and one transparent index, so every
/// frame is quantised. That is fine here — this is a picture of *motion*, and
/// the still fixtures are the PNGs that carry colour fidelity.
fn write_gif(frames: &[Vec<u8>], width: u32, height: u32, path: &FsPath) {
    let file = std::fs::File::create(path).expect("creating the GIF");
    let mut encoder =
        gif::Encoder::new(file, width as u16, height as u16, &[]).expect("starting the GIF");
    encoder
        .set_repeat(gif::Repeat::Infinite)
        .expect("looping the GIF");
    for data in frames {
        let mut rgba = data.clone();
        let mut frame = gif::Frame::from_rgba_speed(width as u16, height as u16, &mut rgba, 10);
        frame.delay = INTERVAL_MS;
        encoder.write_frame(&frame).expect("writing a GIF frame");
    }
}

// ─────────────────────────────────────────────────────────── the sequences

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// `t` mapped into a window `[start, end]` of the sequence, eased.
fn window(t: f32, start: f32, end: f32) -> f32 {
    if t <= start {
        return 0.0;
    }
    if t >= end {
        return 1.0;
    }
    ease_out((t - start) / (end - start))
}

fn fade_in(t: f32) -> WidgetNode {
    let a = ease_out(t);
    Container::new()
        .color(PAPER)
        .alignment(Alignment::CENTER)
        .child(
            Opacity::new(a).child(
                Transformed::scale(0.9 + 0.1 * a, 0.9 + 0.1 * a).child(
                    Container::new()
                        .gradient(Gradient::vertical().with_stops(&[(0.0, ACCENT), (1.0, VIOLET)]))
                        .radius(20.0)
                        .size(240.0, 150.0)
                        .alignment(Alignment::CENTER)
                        .child(Text::new("Ready").color(Color::WHITE).size(26.0).bold()),
                ),
            ),
        )
        .into()
}

fn stagger(t: f32) -> WidgetNode {
    // **Five rows, not eight, and the count is a fix rather than a taste.**
    //
    // Eight rows are 453px of column in the 320px this fixture's padding
    // leaves, so every one of its twenty-four frames reported a 133px
    // overflow and the GIF — the reference for what a staggered reveal looks
    // like here — showed three rows painting past the bottom of the page.
    //
    // Five leaves 33px of headroom. That margin is deliberate: a gate that
    // trips whenever a font's line metrics move by a pixel gets disabled, and
    // a disabled gate is how this got to eight in the first place.
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0);
    for i in 0..5 {
        let start = i as f32 * 0.07;
        let a = window(t, start, (start + 0.35).min(1.0));
        column = column.push(
            Transformed::translate(Offset::new(0.0, (1.0 - a) * 14.0)).child(
                Opacity::new(a).child(
                    Container::new()
                        .color(Color::WHITE)
                        .radius(10.0)
                        .shadow(Shadow::new(
                            Color::rgba(20, 30, 60, 30),
                            Offset::new(0.0, 2.0),
                            10.0,
                        ))
                        .padding(EdgeInsets::all(12.0))
                        .width(420.0)
                        .child(
                            Flex::row()
                                .cross_axis_alignment(CrossAxisAlignment::Center)
                                .spacing(10.0)
                                .push(Container::new().color(ACCENT).radius(6.0).size(24.0, 24.0))
                                .push(
                                    Flex::column()
                                        .cross_axis_alignment(CrossAxisAlignment::Start)
                                        .spacing(4.0)
                                        .push(
                                            Text::new(format!("Row {}", i + 1))
                                                .color(INK)
                                                .size(13.0),
                                        )
                                        .push(
                                            Container::new()
                                                .color(Color::rgb(226, 231, 240))
                                                .radius(3.0)
                                                .size(220.0 - i as f32 * 12.0, 6.0),
                                        ),
                                ),
                        ),
                ),
            ),
        );
    }
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .alignment(Alignment::CENTER)
        .child(column)
        .into()
}

/// The launch splash, as its own fixture rather than as an application.
///
/// Deliberately the same *shape* as a real one — mark scales up, wordmark and
/// tagline rise in behind it, the whole overlay fades off at the end — because
/// this is the sequence that was reported as "a blank screen". A GIF of it
/// answers the question a still cannot: is the animation wrong, or is nothing
/// driving it?
fn splash(t: f32) -> WidgetNode {
    // **The exit starts 0.14 after the last reveal lands, not 0.28.**
    //
    // It used to begin at 0.82 while `tag` — the last thing to arrive —
    // finished at 0.54, leaving 28% of the sequence, five of its twenty-three
    // transitions, as byte-identical frames in the *middle* of a transition.
    // `longest_interior_stall` is the number that reports that, and this
    // fixture is the reason it exists: the studio's own launch splash had the
    // same shape of hole and neither of them could be seen through `moving`.
    //
    // A beat after the composition lands is right; a fifth of a second of held
    // picture is a beat, and a third of the animation is a hang.
    let exit = window(t, 0.68, 1.0);
    let veil = 1.0 - exit;
    let mark = window(t, 0.0, 0.22);
    let panel_a = window(t, 0.08, 0.32);
    let panel_b = window(t, 0.14, 0.40);
    let word = window(t, 0.22, 0.46);
    let tag = window(t, 0.30, 0.54);

    let side = 150.0f32;
    let scale = 0.78 + 0.22 * mark;

    Container::new()
        .color(Color::rgb(14, 16, 19))
        .alignment(Alignment::CENTER)
        .child(
            // The departure: the composition lifts and swells very slightly as
            // it fades, so the fade has a direction and reads as the mark
            // moving off rather than as two pictures cross-dissolving. Same
            // numbers as `viewwstudio`'s own splash, deliberately — this
            // fixture is what that one is reviewed against.
            Transformed::translate(Offset::new(0.0, -18.0 * exit)).child(
                Transformed::scale(1.0 + 0.04 * exit, 1.0 + 0.04 * exit).child(
                    Opacity::new(veil).child(
                        Flex::column()
                            .main_axis_size(MainAxisSize::Min)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .spacing(18.0)
                            .push(SizedBox::from_size(Size::new(side, side)).child(
                                Opacity::new(mark).child(Transformed::scale(scale, scale).child(
                                    Painting::sized(
                                        Size::new(side, side),
                                        PaintWith::new(move |book, size| {
                                            // The mark: two overlapping rounded
                                            // panels, each revealing on its own
                                            // clock, over a rounded ground.
                                            let s = size.width;
                                            book.rrect(
                                                Rect::new(0.0, 0.0, s, s),
                                                s * 0.22,
                                                Color::rgb(20, 24, 30),
                                            );
                                            let a = panel_a;
                                            book.rrect(
                                                Rect::new(
                                                    s * 0.14,
                                                    s * 0.20,
                                                    s * 0.14 + s * 0.46 * a,
                                                    s * 0.80,
                                                ),
                                                s * 0.06,
                                                ACCENT,
                                            );
                                            let b = panel_b;
                                            book.rrect(
                                                Rect::new(
                                                    s * 0.44,
                                                    s * 0.30,
                                                    s * 0.44 + s * 0.42 * b,
                                                    s * 0.90,
                                                ),
                                                s * 0.06,
                                                Color::rgba(168, 85, 247, 235),
                                            );
                                        }),
                                    ),
                                )),
                            ))
                            .push(
                                Transformed::translate(Offset::new(0.0, (1.0 - word) * 10.0))
                                    .child(
                                        Opacity::new(word).child(
                                            Text::new("vieww Studio")
                                                .color(Color::rgb(232, 234, 240))
                                                .size(30.0)
                                                .bold(),
                                        ),
                                    ),
                            )
                            .push(
                                Transformed::translate(Offset::new(0.0, (1.0 - tag) * 10.0)).child(
                                    Opacity::new(tag).child(
                                        Text::new("a code editor and device-framed preview")
                                            .color(Color::rgb(154, 162, 177))
                                            .size(13.0),
                                    ),
                                ),
                            ),
                    ),
                ),
            ),
        )
        .into()
}

/// A modal sheet rising over a blurred, dimmed page — the transition that
/// costs the most in a real application, because every frame of it re-blurs
/// a full-surface backdrop.
fn sheet(t: f32) -> WidgetNode {
    let rise = window(t, 0.0, 0.6);
    let dim = window(t, 0.0, 0.4);
    let blur = 12.0 * dim;

    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            Filtered::blur(blur).child(page_behind()),
            Positioned::new()
                .left(0.0)
                .right(0.0)
                .top(0.0)
                .bottom(0.0)
                .child(Container::new().color(Color::rgba(10, 14, 22, (150.0 * dim) as u8))),
            Positioned::new()
                .left(60.0)
                .right(60.0)
                .top(SIZE.height - 40.0 - 210.0 * rise)
                .child(
                    Container::new()
                        .color(Color::WHITE)
                        .radius(18.0)
                        .shadow(Shadow::new(
                            Color::rgba(0, 0, 0, 90),
                            Offset::new(0.0, -6.0),
                            40.0
                        ))
                        .padding(EdgeInsets::all(18.0))
                        .child(
                            Flex::column()
                                .cross_axis_alignment(CrossAxisAlignment::Start)
                                .spacing(10.0)
                                .push(
                                    Container::new()
                                        .color(Color::rgb(220, 226, 236))
                                        .radius(3.0)
                                        .size(48.0, 5.0),
                                )
                                .push(Text::new("Share this render").color(INK).size(17.0).bold())
                                .push(
                                    Text::new("A PNG, a GIF of the sequence, or the scene itself.")
                                        .color(MUTED)
                                        .size(12.0),
                                )
                                .push(
                                    Container::new()
                                        .color(ACCENT)
                                        .radius(9.0)
                                        .padding(EdgeInsets::all(10.0))
                                        .child(Text::new("Export").color(Color::WHITE).size(13.0)),
                                ),
                        ),
                ),
        ])
        .into()
}

fn page_behind() -> WidgetNode {
    // 8, not 10. At 10 this column came to 322px in 320px of space — a two
    // pixel overflow, reported on every frame of both fixtures that use this
    // page, and the least visible and most persistent kind: nothing looks
    // wrong in the picture, and the warning is real.
    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(8.0)
        .push(Text::new("Recent renders").color(INK).size(18.0).bold());
    for i in 0..5 {
        column = column.push(
            Container::new()
                .color(Color::WHITE)
                .radius(10.0)
                .padding(EdgeInsets::all(10.0))
                .width(440.0)
                .child(
                    Flex::row()
                        .spacing(10.0)
                        .push(
                            Container::new()
                                .gradient(Gradient::vertical().with_stops(&[
                                    (0.0, if i % 2 == 0 { ACCENT } else { MINT }),
                                    (1.0, VIOLET),
                                ]))
                                .radius(6.0)
                                .size(40.0, 30.0),
                        )
                        .push(
                            Text::new(format!("frame-{:03}.png", i * 7 + 3))
                                .color(MUTED)
                                .size(12.0),
                        ),
                ),
        );
    }
    Container::new()
        .color(PAPER)
        .padding(EdgeInsets::all(20.0))
        .child(column)
        .into()
}

/// A continuous sweep: a rotating arc plus counter-rotating dots. Nothing
/// fades, so every frame differs by transform alone — the case where a
/// renderer that caches by *shape* has to notice the transform changed.
fn spinner(t: f32) -> WidgetNode {
    let angle = t * std::f32::consts::TAU;
    Container::new()
        .color(PAPER)
        .alignment(Alignment::CENTER)
        .child(Painting::sized(
            Size::new(260.0, 260.0),
            PaintWith::new(move |book, size| {
                let c = Offset::new(size.width / 2.0, size.height / 2.0);
                book.ring(c, 96.0, 10.0, Color::rgb(226, 231, 240));
                let arc = vieww_foundation::Path::arc_ring(c, 96.0, 10.0, angle, 2.2);
                book.fill(arc, ACCENT);
                for i in 0..12 {
                    let a = -angle + i as f32 / 12.0 * std::f32::consts::TAU;
                    book.circle(
                        Offset::new(c.dx + 62.0 * a.cos(), c.dy + 62.0 * a.sin()),
                        5.0,
                        Color::rgba(168, 85, 247, (60 + i * 16).min(255) as u8),
                    );
                }
            }),
        ))
        .into()
}

/// The clipped variant, kept for chasing clip costs under animation.
#[allow(dead_code)]
fn spinner_clipped(t: f32) -> WidgetNode {
    Clip::rounded(24.0).child(spinner(t)).into()
}

// ───────────────────────────────────────────── the two that were added last

/// A card growing into a full-bleed detail view, with its own content
/// cross-fading — the "shared element" transition.
///
/// # Why this one is here
///
/// Everything above it moves *one* property: an opacity, an offset, a rotation.
/// This moves a rectangle's four edges, its corner radius, its elevation and
/// two different subtrees' opacities at once, all off one clock, and it is the
/// transition a real application is judged on — it is what opening a message,
/// a photo or a settings pane looks like on every platform that does it well.
///
/// It is also the one that finds interpolation bugs. A shared-element move is
/// where a renderer that snaps a rounded corner to integers, or that recomputes
/// a shadow from scratch at every radius, stops being smooth — and neither of
/// those shows up in a fixture that only fades something.
fn shared_element(t: f32) -> WidgetNode {
    // Two clocks, overlapping. The geometry leads and the content follows, so
    // the destination's text arrives into a box that has nearly finished
    // moving rather than sliding around with it.
    let grow = window(t, 0.0, 0.78);
    let old_out = 1.0 - window(t, 0.0, 0.30);
    let new_in = window(t, 0.42, 1.0);

    // The card's rest position, and the full-bleed one it grows into.
    // The collapsed card is 124px tall, not 100. `card_face` is 87px of
    // content inside 14px of padding, which needs 115 — so at 100 the card
    // this transition *starts* from was overflowing its own face by 15px, on
    // every frame before the grow began, in the fixture whose whole subject is
    // one object changing size correctly.
    let (x0, y0, x1, y1) = (
        lerp(150.0, 24.0, grow),
        lerp(198.0, 24.0, grow),
        lerp(410.0, SIZE.width - 24.0, grow),
        lerp(322.0, SIZE.height - 24.0, grow),
    );
    // Corner radius and elevation travel with the size, which is what makes a
    // grow read as one object rather than as a box being replaced by a bigger
    // box: a small card is rounder and sits closer to the page.
    let radius = lerp(14.0, 26.0, grow);
    let elevation = lerp(10.0, 44.0, grow);

    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            // The page it lifts off, dimming as it goes.
            Opacity::new(1.0 - 0.55 * grow).child(page_behind()),
            Positioned::new()
                .left(x0)
                .top(y0)
                .right(SIZE.width - x1)
                .bottom(SIZE.height - y1)
                .child(
                    Container::new()
                        .color(Color::WHITE)
                        .radius(radius)
                        .shadow(Shadow::new(
                            Color::rgba(12, 18, 32, 70),
                            Offset::new(0.0, elevation * 0.25),
                            elevation,
                        ))
                        .child(
                            // Both states are in the tree the whole way across,
                            // cross-fading. Swapping one for the other at a
                            // threshold is the cheap version and it is visible
                            // as a blink at exactly the moment somebody is
                            // looking at it.
                            // **`Offstage` around each face while it is at zero
                            // opacity, and it is not an optimisation.**
                            //
                            // `detail_face` is 192px tall at every moment of
                            // its life. The card it lives in starts 100px tall
                            // and grows, so for the first two fifths of this
                            // transition the detail view was being laid out
                            // into a box less than half its height —
                            // 192 into 60, then 94, then 123 — and painting
                            // outside the card that is supposed to contain it,
                            // while invisible at `Opacity(0.0)`.
                            //
                            // `Opacity(0.0)` deliberately still lays out and
                            // still occupies space; see that widget's own doc
                            // for why. This is the case it names as the other
                            // one. The comment above still holds — both faces
                            // stay *mounted* the whole way across, so there is
                            // no blink and no lost state — but a face nobody
                            // can see does not need to be measured, and a face
                            // that does not fit must not be.
                            Stack::new().fit(StackFit::Expand).children(children![
                                Offstage::new(old_out <= 0.0)
                                    .child(Opacity::new(old_out).child(card_face())),
                                Offstage::new(new_in <= 0.0)
                                    .child(Opacity::new(new_in).child(detail_face())),
                            ]),
                        ),
                ),
        ])
        .into()
}

/// The collapsed card's own face: a thumbnail block and two lines.
fn card_face() -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::all(14.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(8.0)
                .push(
                    Container::new()
                        .radius(8.0)
                        .gradient(
                            Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0))
                                .between(VIOLET, ACCENT),
                        )
                        .size(96.0, 40.0),
                )
                .push(Text::new("Aurora").color(INK).size(15.0).bold())
                .push(Text::new("3 layers").color(MUTED).size(11.0)),
        )
        .into()
}

/// The expanded detail view: a hero band, a title and a paragraph.
fn detail_face() -> WidgetNode {
    Container::new()
        .padding(EdgeInsets::all(20.0))
        .child(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(12.0)
                .push(
                    Container::new()
                        .radius(12.0)
                        .gradient(
                            Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0))
                                .between(VIOLET, MINT),
                        )
                        .size(SIZE.width - 88.0, 96.0),
                )
                .push(Text::new("Aurora").color(INK).size(24.0).bold())
                .push(
                    Text::new(
                        "Three stacked gradients, a blurred backdrop and a stroke \
                         that follows the outline. Rendered on the CPU.",
                    )
                    .color(MUTED)
                    .size(12.0),
                ),
        )
        .into()
}

/// One closed vector path interpolating into another, drawn filled and
/// stroked, over a rotating sweep gradient.
///
/// # Why a *path* fixture and not another box fixture
///
/// Every other motion fixture here animates a widget's properties, which
/// exercises layout and compositing. None of them change the geometry a path is
/// made of, so none of them re-flatten a curve — and curve flattening is the
/// one per-frame cost in this renderer that scales with how much the shape
/// moved rather than with how big it is. A morph is the worst case for it and
/// the best case for finding it: sixty-four cubic segments rebuilt, re-flattened
/// and re-rasterised every frame, filled once and stroked once.
fn morph(t: f32) -> WidgetNode {
    let phase = window(t, 0.0, 1.0);
    // A blob and a star, sampled around the same circle so the interpolation is
    // between corresponding points rather than between two point *lists* of
    // different lengths — which is what makes a morph read as one shape
    // changing instead of as vertices being reassigned.
    let spin = t * std::f32::consts::TAU * 0.5;

    Container::new()
        .color(Color::rgb(16, 18, 24))
        .alignment(Alignment::CENTER)
        .child(Painting::sized(
            Size::new(320.0, 320.0),
            PaintWith::new(move |book, size| {
                let c = Offset::new(size.width / 2.0, size.height / 2.0);
                let base = size.width * 0.34;

                // **Cubics, not a polyline.** Sampling the radius densely and
                // joining the samples with `line_to` draws the same silhouette
                // and exercises none of the flattener — which is the whole
                // stated reason this fixture exists. It also *looks* like what
                // it is: a blob with visible flat facets.
                //
                // So the outline is 14 control points joined by cubic segments,
                // with each point's tangent taken from its two neighbours (a
                // Catmull-Rom to Bézier conversion, the standard one: the
                // control handles sit a third of the way along the chord
                // through the neighbouring points). The curve passes through
                // every sample, so the morph is still a straight interpolation
                // of radii, and the segments between them are real curves the
                // renderer has to flatten at whatever tolerance the current
                // transform asks for.
                // **The samples are locked to the shape and the whole path is
                // spun, not the other way round.**
                //
                // Adding `spin` to the angle the radius is evaluated at slides
                // the sample points around the shape's own lobes, so the star's
                // peaks land between samples about as often as on them and the
                // curve through them averages the star away — at full phase it
                // came out as a circle. Evaluating the radius at a fixed angle
                // and rotating the resulting *point* keeps every sample on its
                // own feature for the whole sequence.
                //
                // 28 points for a 7-lobed star: four samples per lobe, which is
                // what a cubic through them needs to keep a point sharp and a
                // valley round.
                const POINTS: usize = 28;
                let (sin_spin, cos_spin) = spin.sin_cos();
                let at = |i: usize| -> Offset {
                    let a = i as f32 / POINTS as f32 * std::f32::consts::TAU;
                    // Radius A: a soft three-lobed blob. Radius B: a
                    // seven-pointed star. The morph is between the radii, so
                    // every sample stays on its own ray — which is what keeps
                    // it one shape changing rather than vertices being
                    // reassigned to each other.
                    let blob = base * (1.0 + 0.16 * (3.0 * a).sin());
                    let star = base * (0.62 + 0.38 * (7.0 * a).cos().abs());
                    let r = blob + (star - blob) * phase;
                    let (x, y) = (r * a.cos(), r * a.sin());
                    Offset::new(
                        c.dx + x * cos_spin - y * sin_spin,
                        c.dy + x * sin_spin + y * cos_spin,
                    )
                };

                let mut path = vieww_foundation::Path::default();
                path.move_to(at(0));
                for i in 0..POINTS {
                    let p0 = at((i + POINTS - 1) % POINTS);
                    let p1 = at(i);
                    let p2 = at((i + 1) % POINTS);
                    let p3 = at((i + 2) % POINTS);
                    let c1 =
                        Offset::new(p1.dx + (p2.dx - p0.dx) / 6.0, p1.dy + (p2.dy - p0.dy) / 6.0);
                    let c2 =
                        Offset::new(p2.dx - (p3.dx - p1.dx) / 6.0, p2.dy - (p3.dy - p1.dy) / 6.0);
                    path.cubic_to(c1, c2, p2);
                }
                path.close();

                book.fill(
                    path.clone(),
                    Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 1.0))
                        .between(VIOLET, MINT),
                );
                book.stroke(path, Color::rgba(232, 236, 245, 190), 2.0);
            }),
        ))
        .into()
}

/// Linear interpolation, spelled out because it appears four times above and
/// reads better than the arithmetic does.
fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
