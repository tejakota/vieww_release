//! The same tree twice, read left-to-right and right-to-left.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example rtl --release
//!
//! # one half at a time, for a display shorter than the 880pt surface
//! cargo run -p vieww-platform-winit --example rtl --release -- --rtl
//!
//! # unattended, for a device or CI
//! cargo run -p vieww-platform-winit --example rtl --release -- --frames 120
//! ```
//!
//! # What this is for, and why a test could not do it
//!
//! Every claim on this screen already has an assertion behind it — the mirrored
//! flex has six tests in `vieww-render/tests/layout.rs`, the primitives have
//! twenty more. **An RTL layout is the canonical thing that passes every
//! assertion and is obviously wrong to look at**, because the bugs are in what
//! the numbers *mean*: an offset computed from the wrong edge is still a
//! perfectly good number, and a test that asserts it was written by the same
//! person who got the edge wrong.
//!
//! So this screen is built to need no reference. **The two halves are one tree,
//! built once and mounted twice**, under `Directionality::new(Ltr)` and
//! `Directionality::new(Rtl)`. There is exactly one thing to check:
//!
//! > **Every strip is a mirror image of the strip above it — except the second,
//! > which must be identical.**
//!
//! Anything that is *nearly* mirrored is a bug. Symmetry is the assertion, and
//! the eye checks it in about a second.
//!
//! # The seven strips, and the claim each one carries
//!
//! 1. **Ambient direction.** A `Flex` with **nothing set on it**. This is the
//!    seam that did not exist until `register_with_context`: a render widget
//!    never runs a `build`, so it has no `BuildContext`, and before that door
//!    existed an ambient direction could reach a composed widget and nothing
//!    else. If this strip does not mirror, the scope is not reaching the
//!    factory and everything else here is decoration.
//!
//!    The descending widths are the point — they say *which child is first*.
//!    Reversing the child list would also produce a mirrored picture, and it is
//!    the wrong fix, because it reorders paint, hit testing and the order a
//!    screen reader walks. Only placement is allowed to move.
//!
//! 2. **A pinned subtree, which must NOT mirror.** `.text_direction(Ltr)` on the
//!    flex. This is why `Flex::text_direction` is an `Option` rather than a
//!    defaulted field: `Ltr` is a real answer an application pins a subtree to —
//!    a phone number, a code snippet, a licence plate inside an Arabic screen —
//!    so it has to be distinguishable from never having said. If this strip
//!    mirrors along with the rest, the `Option` collapsed and "explicitly
//!    left-to-right" became the same value as "unspecified".
//!
//! 3. **`EdgeInsetsDirectional`.** A wide start inset and a narrow end one, so
//!    the bar hangs off the reading edge. The gap swaps sides; the bar keeps its
//!    width, because resolving must *move* space and never create or destroy it.
//!
//! 4. **`AlignmentDirectional`.** `CENTER_START`, so the marker sits against the
//!    edge reading begins at.
//!
//! 5. **`Stack::alignment_directional`** — a badge on a card. Worth its own
//!    strip rather than being taken as read from strip 4, because it resolves
//!    down a **different path**: a `Stack` is a render widget with no
//!    `BuildContext`, so its alignment is resolved in `vieww-render`'s factory
//!    through `register_with_context`, while strips 3 and 4 resolve in a
//!    composed widget's own `build`. The card spans the strip and cannot move;
//!    only the badge can.
//!
//!    Note what this strip does *not* claim: a `Stack` given a physical
//!    `Alignment` does not mirror at all, deliberately, unlike the auto-mirroring convention. Only
//!    the directional form moves.
//!
//! 6. **`PositionedDirectional`** — the same badge, pinned to an edge rather
//!    than aligned to one, inset by 8 so a swap with strip 5 is obvious. It
//!    lands where strip 5's does and crosses with it, and gets there down a
//!    **third** path: a `Composed` widget resolves start and end in its own
//!    `build` and hands an ordinary `Positioned` to a stack that never learns a
//!    direction was involved.
//!
//!    Only the horizontal is pinned, so the stack's alignment places the other
//!    axis. A badge that drifts to the top of this strip is the per-axis
//!    fallback broken.
//!
//! 7. **The scroll anchor.** A horizontal `Viewport` whose content is wider than
//!    it is, sitting at offset zero. The teal block is row 0 and it must be on
//!    screen in both halves — flush left in Latin, flush right in Arabic.
//!
//!    **This is two mechanisms agreeing, which is why it is worth looking at.**
//!    The inner `Flex` mirrors, putting row 0 at the *far end of the content*;
//!    the viewport anchors that end against the window. Get one without the
//!    other and the list opens on its **last** row — a screen that is obviously
//!    wrong at a glance and perfectly plausible to a test that only checks the
//!    flex, which is exactly the trap this file exists for.
//!
//! The gaps in strips 1 and 2 come from `Flex::spacing`, and they are here to be
//! looked at too: spacing walks the cursor forward while mirroring counts back
//! from the far edge, so getting the order wrong turns the gap into an overlap.
//! Overlapping boxes on strip 1 are that bug.

use vieww_foundation::{Color, Size};
use vieww_platform_winit::App;
use vieww_widget::prelude::*;

/// Deliberately short enough that a window manager grants it whole.
///
/// **This asked for 880 for a long time and never got it.** A 768pt display
/// hands back about 700 once its decorations are taken out, and the window is
/// silently clamped — so the *lower* pane was cut off, which is the mirrored one
/// and the only half this file exists for. The truncation looked like nothing
/// was wrong, because the strips it ate were the last two.
///
/// So the content is sized to fit here rather than the window being sized to fit
/// the content: 2 × (19 heading + 7 × 34 + 6 × 6 gap) plus the caption and the
/// column's spacing is about 656, inside 680. `--ltr` and `--rtl` remain for a
/// display shorter even than this.
const SURFACE: Size = Size {
    width: 760.0,
    height: 680.0,
};

/// Every strip is this wide, so a mirror is a reflection about one axis rather
/// than about four different ones.
const STRIP: f32 = 680.0;

/// The track each specimen row sits in.
///
/// **The specimen is mounted twice, so every strip costs this height twice** —
/// and the budget is the whole reason it is this small. See [`SURFACE`] for the
/// sum; adding a strip means checking it again.
const STRIP_HEIGHT: f32 = 34.0;

/// A plain block, in the strips that are about mirroring a row.
const BLOCK_HEIGHT: f32 = 24.0;

/// The card in strips 5 and 6, which spans the strip so it cannot move, and the
/// badge that sits on it — the only thing in those two strips that can.
const CARD_HEIGHT: f32 = 26.0;
const BADGE_WIDTH: f32 = 36.0;
const BADGE_HEIGHT: f32 = 16.0;

/// Wide start, narrow end — asymmetric on purpose, or the swap is invisible.
const PAD_START: f32 = 96.0;
const PAD_END: f32 = 12.0;

const BACKGROUND: Color = Color::rgb(18, 18, 22);
const TRACK: Color = Color::rgb(38, 38, 46);
/// The first child, and the only one that says which end the row starts at.
const FIRST: Color = Color::rgb(94, 234, 212);
const REST: Color = Color::rgb(71, 85, 105);
const BAR: Color = Color::rgb(250, 204, 21);
const MARK: Color = Color::rgb(244, 114, 182);
const LABEL: Color = Color::rgb(226, 232, 240);
const MUTED: Color = Color::rgb(148, 163, 184);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frames = frames_from_args();

    let mut app = App::new()
        .title("vieww — the same tree, both directions")
        .size(SURFACE)
        .background(BACKGROUND);
    if let Some(frames) = frames {
        app = app.exit_after_frames(frames);
    }

    // Both halves together need about 856pt inside an 880pt surface, and a
    // display that hands the window less than it asked for truncates the
    // **lower** pane — which is the mirrored one, and the only half anybody
    // opens this file to look at. One pane on its own is about 450.
    let ltr_only = has_flag("--ltr");
    let rtl_only = has_flag("--rtl");

    let report = app.run(|driver| {
        let mut rows: Vec<WidgetNode> = Vec::new();
        if !rtl_only {
            rows.push(heading("left to right — the reference"));
            rows.push(
                Directionality::new(TextDirection::Ltr)
                    .child(specimen())
                    .into(),
            );
        }
        if !ltr_only {
            rows.push(heading(
                "right to left — every strip mirrored, except the second",
            ));
            rows.push(
                Directionality::new(TextDirection::Rtl)
                    .child(specimen())
                    .into(),
            );
        }
        // Outside both halves on purpose. It is Latin text with no directional
        // widget in it, so it would render identically in the mirrored half —
        // and one thing that does not move inside a region whose whole claim is
        // "everything moves" is exactly the sort of detail that makes a reader
        // doubt a correct screen.
        rows.push(caption(
            "1 ambient   2 pinned Ltr   3 directional insets   \
             4 directional alignment   5 stack alignment   \
             6 directional position   7 scroll anchor",
        ));

        let root = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(10.0)
            .children(rows);

        driver.set_root(root);
    })?;

    println!("{report}");
    Ok(())
}

/// The four strips, identical in both halves.
///
/// Built by a function rather than cloned from a binding so that each half gets
/// its own widgets: the two live under different `Directionality` ancestors, and
/// sharing one node would invite the reconciler to treat them as the same
/// element and skip the second build — which is exactly the bug this screen
/// exists to make visible, and it would hide it.
fn specimen() -> WidgetNode {
    Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Start)
        .spacing(6.0)
        .children(children![
            // 1. Nothing set on this flex: it reads the ambient direction.
            strip(Flex::row().spacing(14.0).children(children![
                block(FIRST, 150.0),
                block(REST, 90.0),
                block(REST, 45.0),
            ])),
            // 2. Pinned, so the ambient direction must not reach it.
            strip(
                Flex::row()
                    .text_direction(TextDirection::Ltr)
                    .spacing(14.0)
                    .children(children![
                        block(FIRST, 150.0),
                        block(REST, 90.0),
                        block(REST, 45.0),
                    ])
            ),
            // 3. The inset swaps edges; the bar keeps its width.
            strip(
                PaddingDirectional::new(EdgeInsetsDirectional::only(
                    PAD_START, 12.0, PAD_END, 12.0,
                ))
                .child(block(BAR, STRIP - PAD_START - PAD_END))
            ),
            // 4. Against the edge reading begins at.
            strip(
                AlignDirectional::new(AlignmentDirectional::CENTER_START).child(block(MARK, 40.0))
            ),
            // 5. A badge on a card. The card spans the strip so it cannot move;
            //    only the badge can, and it is the `Stack`'s own alignment —
            //    resolved in the factory from the same ambient direction, which
            //    is a different code path from strips 3 and 4.
            strip(
                Stack::new()
                    .alignment_directional(AlignmentDirectional::CENTER_START)
                    .children(children![
                        box_of(REST, STRIP, CARD_HEIGHT),
                        box_of(MARK, BADGE_WIDTH, BADGE_HEIGHT),
                    ])
            ),
            // 6. The same badge, pinned by `PositionedDirectional` instead of by
            //    the stack's alignment. It should sit exactly where strip 5's
            //    does and cross with it — which is the point, because it gets
            //    there down a *third* path: a `Composed` widget resolving start
            //    and end in `build` and handing an ordinary `Positioned` to a
            //    stack that never learns a direction was involved.
            //
            //    Inset by 8 from the leading edge, so it is distinguishable from
            //    strip 5 sitting flush and a swapped pair is obvious.
            //
            //    **Only the horizontal is pinned**, and the alignment below is
            //    what places the other axis — the per-axis fallback, visible: a
            //    badge that says nothing about its vertical still lands
            //    deliberately rather than at the top. `Alignment::CENTER` on the
            //    same grounds the `strip` helper uses it: centre has no
            //    handedness, so no physical edge is being smuggled in here.
            strip(
                Stack::new()
                    .alignment(Alignment::CENTER)
                    .children(children![
                        box_of(REST, STRIP, CARD_HEIGHT),
                        PositionedDirectional::new().start(8.0).child(box_of(
                            MARK,
                            BADGE_WIDTH,
                            BADGE_HEIGHT
                        )),
                    ])
            ),
            // 7. A horizontal list at rest. The content is wider than the strip
            //    and gets clipped; what matters is *which end is showing*. The
            //    teal block is row 0, and it has to be on screen — an Arabic
            //    list that opened on its last row is the bug this closes, and
            //    it is one you see rather than assert.
            strip(
                Viewport::new(Axis::Horizontal).child(Flex::row().spacing(12.0).children(
                    children![
                        block(FIRST, 160.0),
                        block(REST, 160.0),
                        block(REST, 160.0),
                        block(REST, 160.0),
                        block(REST, 160.0),
                    ]
                ))
            ),
        ])
        .into()
}

/// One specimen row, on a track that makes its full extent visible.
///
/// The track matters: a mirrored child is only obviously mirrored against an
/// edge that did not move, and without it every strip would be its own
/// shrink-wrapped island with nothing to be flush against.
///
/// The inner `Align` is load-bearing rather than decorative. `SizedBox` hands
/// its child **tight** constraints, so without it every block would be stretched
/// to the full 56 of the track and the strips would be four solid bands. `Align`
/// loosens, which is what lets a 32-tall block be 32 tall. It is
/// [`Alignment::CENTER`] rather than `CENTER_LEFT` because a physical edge has
/// no business in this file — it happens to be inert, since all four contents
/// fill the track's width, and a reader should not have to work that out.
fn strip(content: impl Into<WidgetNode>) -> WidgetNode {
    ColoredBox::new(TRACK)
        .child(
            SizedBox::from_size(Size::new(STRIP, STRIP_HEIGHT))
                .child(Align::new(Alignment::CENTER).child(content.into())),
        )
        .into()
}

/// A solid block of a known width, which is what carries the whole reading.
fn block(color: Color, width: f32) -> WidgetNode {
    box_of(color, width, BLOCK_HEIGHT)
}

/// The same, at a height of its own — for the card and badge of strip 5, where
/// the badge has to be visibly smaller than what it sits on.
fn box_of(color: Color, width: f32, height: f32) -> WidgetNode {
    ColoredBox::new(color)
        .child(SizedBox::from_size(Size::new(width, height)))
        .into()
}

fn heading(text: &str) -> WidgetNode {
    Text::new(text).color(LABEL).size(16.0).bold().into()
}

fn caption(text: &str) -> WidgetNode {
    Text::new(text).color(MUTED).size(12.0).into()
}

/// `--frames N`, if it was passed.
/// Whether `flag` was passed.
///
/// `--ltr` and `--rtl` each drop one half of the screen. The default shows
/// both, which is the point of the file; these exist for a display shorter than
/// the surface, where the lower — mirrored — half is what gets cut off.
fn has_flag(flag: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == flag)
}

fn frames_from_args() -> Option<u64> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--frames" {
            return args.next().and_then(|count| count.parse().ok());
        }
    }
    None
}
