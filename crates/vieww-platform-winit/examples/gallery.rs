//! The sliver protocol and the paint layer, on one screen, to be *looked at*.
//!
//! ```console
//! # interactive: drag or use the wheel; drag *past the top* for the refresh gap
//! cargo run -p vieww-platform-winit --example gallery --release
//!
//! # the same screen with a floating header instead of a pinned one
//! cargo run -p vieww-platform-winit --example gallery --release -- --floating
//!
//! # start already refreshing, rather than having to pull for it
//! cargo run -p vieww-platform-winit --example gallery --release -- --refreshing
//!
//! # an instrument, not a mode: a flat block where the spinner goes, to find out
//! # whether the refresh control's clip confines its child at all
//! cargo run -p vieww-platform-winit --example gallery --release -- --probe
//!
//! # what the build asked for, and what the sliver was given, per change
//! VIEWW_TRACE_FRAMES=1 cargo run -p vieww-platform-winit --example gallery --release
//! ```
//!
//! # What this is for
//!
//! Slivers, gradients, shadows, shaped clipping and group opacity all arrived in
//! one pass with about a hundred and twenty tests and **no picture**. This
//! repository has learned twice what that is worth — a `Radio` whose dot was an
//! ellipse passed the whole suite, and a frame overlay reported every frame one
//! refresh period too long for as long as the platform crate had existed. Both
//! were obvious the moment anybody looked.
//!
//! Two of the things below have **never been seen moving by anyone**: the gap
//! that opens on overscroll, and a floating header coming back. They are the
//! reason this file exists rather than another test.
//!
//! # The claims, each checkable by eye
//!
//! Not numbered on purpose: this list has drifted from the screen twice by
//! carrying a count that nobody updated when a cell was added.
//!
//! Each is also a claim some test makes, which is the point — if the screen and
//! the suite disagree, one of them is lying and the screen has the better record.
//!
//! - **The header collapses, then pins.** Scroll down: the bar shrinks from 200
//!   to 56 and then stays. Under `--floating` it leaves entirely and comes back
//!   as soon as you scroll *up*, without waiting for the top — and that is a
//!   claim no scroll *position* can express, which is why the bar integrates
//!   deltas instead. At 600 pixels down it is out if you arrived scrolling up
//!   and gone if you arrived scrolling down.
//! - **The header's top edge leaves first.** Collapsing shrinks the box;
//!   *sliding* keeps it and moves it. A bar that squashed rather than slid would
//!   keep its title vertically centred the whole way, which is the wrong picture
//!   and an easy thing to get wrong.
//! - **The gradient ramps.** The header is a real vertical ramp. A flat band
//!   means `to_brush` in `gpu.rs` handed peniko one stop, and that function is
//!   the one place vieww touches peniko's colour types.
//! - **The shadow falls outside the card.** It must extend past the card's own
//!   box on all four sides. If it is cut off square at the edge, a layer's
//!   declared bounds is being treated as a ceiling instead of being replaced on
//!   pop with the union of its contents.
//! - **The avatar is a circle, not a stadium.** `Clip::oval()` in a square box.
//!   In an oblong box an oval is an *ellipse* — avatars are why the widget
//!   exists, so the shape is checked in the box the avatars use.
//! - **Group opacity: the overlap does not darken.** Two overlapping red squares
//!   inside one `Opacity`. Faded together they are one flat shape; faded
//!   *individually* the overlap is visibly darker. That single darker rectangle
//!   is the entire difference between group opacity and the per-primitive fade
//!   it replaced, and it is why three tests were rewritten rather than repaired.
//! - **The 16:9 box keeps its shape.** Drag the window: it stays widescreen
//!   while its neighbours merely narrow, and stops growing at 240 rather than
//!   pushing past the band it sits in.
//! - **The hearts are one shape at three sizes**, read from SVG path data. The
//!   parser's tests compare `PathVerb`s, which says the right instructions came
//!   out and nothing at all about whether they draw a heart. A mirrored curve, a
//!   dropped relative command or a misread control point would satisfy a
//!   surprising number of assertions and be obvious here.
//! - **Virtualisation is invisible.** The row numbers stay consecutive from 0 to
//!   399 however fast you scroll. About ten of the four hundred rows exist as
//!   widgets at any moment.
//! - **Pull down and the spinner comes out of the top; let go and it goes back
//!   the way it came.** Symmetric, and that symmetry is the claim: the control
//!   paints into the gap the overscroll opens (`layout_extent` 0, negative
//!   `paint_origin`), so it is revealed and hidden by the gap itself rather than
//!   by anything of its own.
//!
//!   Releasing always springs the offset back to zero —
//!   `ScrollPosition::fling` sees an overscroll and settles instead of throwing
//!   — so the retraction is the spring, and the spinner rides it.
//!
//!   It **fades as it goes**, from full strength at the trigger down to
//!   `REFRESH_MIN_ALPHA` as the gap closes. The alpha is derived from the same
//!   overscroll that drives the geometry, so a fade that disagreed with the
//!   slide would be impossible by construction rather than merely unlikely. It
//!   is also the only continuously-varying `Opacity` on this screen, which makes
//!   it the one place group opacity is exercised at values other than a half.
//!
//!   Use `--refreshing` to see the *other* geometry, which is the one no gesture
//!   here produces: with a refresh actually running the gap has sprung shut, so
//!   the control has to **make** room (`layout_extent` = its extent,
//!   `paint_origin` 0) or the spinner would vanish the instant the finger left.
//!   Reporting the same numbers for both states is that bug.
//!
//! # This screen found a framework bug: opacity did not cross a boundary
//!
//! Found here by an `Opacity` that visibly did nothing, and **fixed** rather
//! than worked around — see `RenderObject::layer_effect`.
//!
//! `RenderOpacity` records `PushLayer`/`PopLayer` into the scene it paints into,
//! and `RenderTree::paint_into_layer` returns as soon as it reaches a nested
//! repaint boundary, because that subtree is somebody else's recording. So the
//! boundary's commands were never enclosed by the layer and the alpha was never
//! applied to them. It failed by looking like nothing had happened.
//!
//! It bit two things, and the second is why it mattered more than a spinner:
//! `CircularProgress::indeterminate()` wraps itself in a `RepaintBoundary`, and
//! so does **`Viewport`** — meaning fading a scrollable did nothing, and
//! `RouteTransition` over any scrolling route was not doing what it said.
//!
//! The fix is that a boundary's layer carries the effect declared above it and
//! `LayerTree::composite` wraps the layer's own recording *and its children* in
//! it. Declared through a defaulted hook rather than by looking for a
//! `RenderOpacity`, so a blur or a colour filter written outside this repository
//! composites over a scrollable on the same terms.
//!
//! # A wheel has no release, so it now gives itself one — fixed
//!
//! Found by running this and reading a trace: every event was `MouseWheel` and
//! there was never a `MouseInput`, so `on_drag_end` never fired. The gap opened
//! and the spinner followed — `on_scroll` feeds the same `scroll.drag()` a
//! finger does — and then nothing brought it home, because **there is no such
//! thing as releasing a wheel.**
//!
//! `apply_drag` clears any animation and only `fling`/`settle` start one, so a
//! wheel that overscrolled left the content past the edge with nothing to
//! retract it. Invisible under `Overscroll::Clamp` — the platform default on a
//! desktop — because clamping leaves no overscroll to settle. **This example was
//! the first thing to put `ScrollPhysics::ios()` behind a wheel**, which is why
//! the combination had never been seen.
//!
//! This was written up as accepted — *"bounce is a touch idiom and the platform
//! default is right: a finger has a release and a wheel does not"* — and the
//! price recorded as needing the mouse button held. **That reading was wrong,
//! and it cost two sessions.** The three symptoms later reported against this
//! screen were all this one cause, re-investigated against the drag path, which
//! was never at fault.
//!
//! The mistake was in the framing. A wheel notch is not a gesture without a
//! release; it is a press, a movement *and* a release at a single instant, and
//! reporting only the middle third of it is a bug rather than a limitation. The
//! consequence was never confined to a demo either: any application using
//! [`Scrollable`] with `ScrollPhysics::ios()` could be left permanently
//! overscrolled with no gesture able to recover.
//!
//! So the notch now reports both halves, here and in `Scrollable`, on the
//! event's own clock — `ScrollEvent` always carried a `timestamp` and the
//! detector was discarding it. A spring started at `Duration::ZERO` is already
//! finished, and would snap rather than settle.
//!
//! # What building this found: the window is the application's problem
//!
//! [`SliverList`] does not know which rows to build. [`ListView`] does — it reads
//! an inherited `ScrollMetrics` that [`Scrollable`] publishes, and works out its
//! own window in `build`. `CustomScrollView` publishes nothing, so **the
//! arithmetic in [`window`] below is a re-derivation of
//! `RenderSliverFixedList::window`, in the application**, and every application
//! using a `SliverList` will write it again.
//!
//! Worse, it cannot be written *correctly* from outside: the offset at which the
//! list starts is the sum of the scroll extents of every sliver above it, which
//! is a layout answer. Here those are constants and so it can be done; on a
//! screen whose header measures itself, it could not be. That is a real gap and
//! it is not one the tests could have surfaced, because every test hands
//! `window` its numbers directly.
//!
//! The slack below is the honest consequence — see [`window`].
//!
//! # Run it in release
//!
//! A debug vello is ten to thirty times slower, and the shadow blur is the most
//! expensive thing on this screen by a wide margin.

use std::rc::Rc;

use vieww_element::ScrollController;
use vieww_foundation::{parse_path_data, DragDetails, Modifiers, ScrollEvent, Size};
use vieww_gestures::ScrollPhysics;
use vieww_platform_winit::App;
use vieww_widget::prelude::*;
use vieww_widget::widget_node_from;

const SURFACE: Size = Size {
    width: 640.0,
    height: 560.0,
};

/// The header, at rest and collapsed.
const BAR_MAX: f32 = 200.0;
const BAR_MIN: f32 = 56.0;

/// The band of ordinary widgets between the header and the list.
///
/// Deliberately **not** a sliver: the claim in the sliver module's docs is that
/// anything which is not one is adapted automatically, and a screen that only
/// ever held slivers would never test it.
const SHOWCASE: f32 = 260.0;

const ROWS: usize = 400;
const ROW: f32 = 64.0;

/// How far the content must be pulled past the top before a refresh would arm.
const REFRESH: f32 = 80.0;

/// The faintest the spinner gets, at the moment the gap closes.
///
/// **Deliberately not zero.** Two reasons, and the second is the interesting
/// one: a control that reaches zero looks like it was switched off a frame early
/// rather than withdrawn, because the last of the fade happens in the few pixels
/// where the shape is already almost gone. And `Scene::pop_layer` drops a layer
/// whose alpha is zero along with its contents — correct, and load-bearing for
/// damage — so the final frame of a fade to zero is a different code path from
/// every frame before it. Ending at a floor keeps the whole retraction on one.
const REFRESH_MIN_ALPHA: f32 = 0.35;

/// Everything the scroll view accounts for, in scroll-offset space.
///
/// The refresh control contributes **nothing** at rest — being pulled, it paints
/// into a gap that already exists and reports a `layout_extent` of zero. A
/// control that added its extent here would leave a permanent hole at the top.
///
/// While it *is* refreshing the content really is `REFRESH` longer than this
/// says, so the maximum scroll is 80pt short for as long as the spinner is up.
/// Left alone deliberately: the fix is for the scroll view to report its own
/// extents, not for the application to keep a second copy of the arithmetic in
/// sync with a control's state — see the module docs on what has to be
/// re-derived.
const CONTENT: f32 = BAR_MAX + SHOWCASE + ROWS as f32 * ROW;

// ------------------------------------------------------------------- palette

const INK: Color = Color::rgb(18, 18, 24);
const CARD: Color = Color::rgb(38, 38, 48);
const LABEL: Color = Color::rgb(196, 196, 208);
const HEART_INK: Color = Color::rgb(244, 114, 182);

/// The header's ramp. Two stops, far enough apart to be a ramp and not a band.
fn header_gradient() -> Gradient {
    Gradient::vertical().with_stops(&[
        (0.0, Color::rgb(58, 62, 148)),
        (1.0, Color::rgb(22, 24, 60)),
    ])
}

// -------------------------------------------------------------- the showcase

/// A card with a shadow that falls outside it.
///
/// The margin around it is not decoration — a shadow drawn at the very edge of
/// the band would be clipped by the viewport and the claim would be untestable.
fn shadowed_card() -> WidgetNode {
    Padding::all(18.0)
        .child(
            Container::new()
                .color(CARD)
                .radius(14.0)
                .border(Border::thin(Color::rgb(70, 70, 88)))
                // Offset down and blurred wide, so it is unmistakably *outside*
                // the box rather than a dark edge on it.
                .shadow(Shadow::new(
                    Color::rgba(0, 0, 0, 190),
                    Offset::new(0.0, 10.0),
                    18.0,
                ))
                .child(
                    Center::new()
                        .child(Text::new("shadow, radius, border").size(14.0).color(LABEL)),
                ),
        )
        .into()
}

/// A circular avatar with a presence dot pinned to its corner.
///
/// Square on purpose. `Clip::oval()` inscribes an ellipse in whatever box it is
/// given, so an oblong one would be an ellipse and correct — and would not
/// answer the question anybody actually has about an avatar.
///
/// **The dot is the visual check for `Positioned`**, and it is checking two
/// things at once. It has to sit on the avatar's lower-right *without making
/// the stack any bigger* — a positioned child takes no part in sizing — so if
/// the badge ever starts pushing the avatar around, or drifts to a corner of
/// the cell rather than of the avatar, this is where it shows.
fn avatar() -> WidgetNode {
    Center::new()
        .child(Stack::new().children(children![
            SizedBox::square(96.0).child(
                Clip::oval().child(
                    Container::new()
                        .gradient(Gradient::vertical().with_stops(&[
                            (0.0, Color::rgb(240, 148, 74)),
                            (1.0, Color::rgb(196, 54, 106)),
                        ]))
                        .child(
                            Center::new()
                                .child(Text::new("TV").size(30.0).bold().color(Color::WHITE)),
                        ),
                ),
            ),
            // Inset by 2 rather than flush, so a circle sits on a circle
            // without its corner poking outside the avatar's own edge.
            Positioned::new().right(2.0).bottom(2.0).child(
                SizedBox::square(22.0).child(
                    Clip::oval().child(Container::new().color(Color::rgb(80, 200, 120))),
                ),
            ),
        ]))
        .into()
}

/// Two overlapping squares under one `Opacity`.
///
/// **The whole claim is that the overlap is not darker than the rest.** Under
/// the per-primitive fade this replaced, each square was drawn at half alpha and
/// the region where they meet composited twice — a visibly darker rectangle in
/// the middle. Group opacity draws the pair into a layer and fades the layer, so
/// the two reds are indistinguishable from one.
fn group_opacity() -> WidgetNode {
    let square = |offset: Offset| -> WidgetNode {
        Transformed::translate(offset)
            .child(SizedBox::square(64.0).child(ColoredBox::new(Color::rgb(220, 60, 60))))
            .into()
    };

    Center::new()
        .child(
            Opacity::new(0.5).child(
                SizedBox::from_size(Size::new(120.0, 100.0)).child(
                    Stack::new()
                        .alignment(Alignment::TOP_LEFT)
                        .children(children![
                            square(Offset::ZERO),
                            square(Offset::new(40.0, 26.0))
                        ]),
                ),
            ),
        )
        .into()
}

/// A 16:9 box that keeps its shape whatever width it is given.
///
/// **The visual check for `AspectRatio`**, and it needs the window resized to
/// mean anything: drag the edge and this stays widescreen while its neighbours
/// just get narrower. Its label reports the ratio, so a box that has quietly
/// stopped honouring one is visible rather than merely suspicious.
///
/// The `Center` is load-bearing — it loosens the constraints. Handed the cell's
/// own tight width this would simply fill it, which is correct behaviour and a
/// useless demonstration.
///
/// The `Padding` is load-bearing too, and was added after looking at it: without
/// it the box fills its cell edge to edge, which on the last cell means flush
/// against the window frame with its right-hand corners cut off. It read as a
/// clipping bug in a screenshot when the layout was in fact correct — 18 to
/// match `shadowed_card`, so the four cells sit on one margin.
fn aspect_ratio_card() -> WidgetNode {
    Padding::all(18.0)
        .child(
            Center::new().child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(8.0)
                    .children(children![
                        // Capped at 240 wide, so the derived height cannot grow past
                        // the 260pt band on a wide window and overflow it.
                        // `enforce` means the cap yields to a narrower cell rather
                        // than overriding it, so the box still shrinks — and stays
                        // 16:9 — as the window closes in.
                        Constrained::new(Constraints::new(0.0, 240.0, 0.0, f32::INFINITY)).child(
                            AspectRatio::new(16.0 / 9.0).child(
                                Container::new().color(Color::rgb(56, 132, 255)).radius(6.0)
                            )
                        ),
                        Text::new("AspectRatio 16:9").size(14.0).color(LABEL),
                    ]),
            ),
        )
        .into()
}

/// An icon read from SVG path data, at three sizes.
///
/// **The visual half of the path parser.** Its tests compare `PathVerb`s, which
/// says the right instructions came out; nothing there says the instructions
/// draw a heart. A parser that mirrored every curve, dropped the relative
/// commands or misread a control point would satisfy a surprising number of
/// assertions and produce a shape obviously wrong at a glance.
///
/// The string is a classic SVG stress test, chosen because it *exercises* the
/// parser rather than merely using it: an implicit repeated `C`, relative `c`
/// and `l`, and `3.41.81` — two numbers sharing one decimal point, which is the
/// case a parser that splits on whitespace gets wrong.
///
/// Three sizes from one `IconData`, because scaling is `viewbox`'s job rather
/// than the path's: all three should be the same shape, not three drawings that
/// thicken or drift as they grow.
///
/// **Stacked rather than in a row, and that is a width budget.** Five cells on a
/// 640pt window are 128 each, or 92 inside the padding; three icons side by side
/// need 20 + 32 + 48 and two gaps, which is 120, and the largest heart was cut
/// off by the window edge. A column's width is its *widest* child rather than
/// the sum of them, so this fits wherever a 48pt icon does.
fn svg_icon_card() -> WidgetNode {
    const HEART: &str = "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 \
                         7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 \
                         19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";

    let heart = IconData::square24(parse_path_data(HEART).expect("the heart is valid path data"));

    Padding::all(18.0)
        .child(
            Center::new().child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .spacing(10.0)
                    .children(children![
                        Flex::column()
                            .main_axis_size(MainAxisSize::Min)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .spacing(6.0)
                            .children(children![
                                Icon::new(heart.clone()).size(20.0).color(HEART_INK),
                                Icon::new(heart.clone()).size(32.0).color(HEART_INK),
                                Icon::new(heart).size(48.0).color(HEART_INK),
                            ]),
                        Text::new("SVG path data").size(14.0).color(LABEL),
                    ]),
            ),
        )
        .into()
}

/// The band of ordinary widgets, five across.
fn showcase() -> WidgetNode {
    SizedBox::height(SHOWCASE)
        .child(Flex::row().children(children![
            Flexible::expanded(1).child(shadowed_card()),
            Flexible::expanded(1).child(avatar()),
            Flexible::expanded(1).child(group_opacity()),
            Flexible::expanded(1).child(aspect_ratio_card()),
            Flexible::expanded(1).child(svg_icon_card()),
        ]))
        .into()
}

// ------------------------------------------------------------------ the list

/// One numbered row.
///
/// The number is the instrument: a skipped or repeated row reads `6 7 8` then
/// `12 13`, which needs no interpretation. The alternating tint is only an aid
/// for scrolling fast enough that the labels blur.
fn row(index: usize) -> WidgetNode {
    let tint = if index % 2 == 0 { 30 } else { 24 };
    ColoredBox::new(Color::rgb(tint, tint, tint + 8))
        .child(Padding::all(20.0).child(Text::new(format!("row {index}")).size(15.0).color(LABEL)))
        .into()
}

/// Which rows to build for a given scroll offset.
///
/// # Why this is here at all, and why it has slack
///
/// This is `RenderSliverFixedList::window` re-derived in the application — see
/// the module docs. It is *approximate*, and the slack is the price of that:
///
/// - A **pinned** header keeps `BAR_MIN` on screen after it has stopped
///   accounting for scroll, so it covers rows the offset alone says are visible.
/// - **Overscroll** makes the offset negative, and a negative numerator into a
///   division by the item extent is a release-mode panic waiting for whoever
///   forgets the clamp.
///
/// Building three rows more than needed costs three widgets out of four hundred
/// and is invisible; building one too few is a blank strip at the bottom of the
/// screen on every frame of a drag. The asymmetry is why this errs the way it
/// does rather than being tightened.
fn window(offset: f32, viewport: f32) -> (usize, usize) {
    // Where the list itself starts, in scroll space: everything above it.
    let above = BAR_MAX + SHOWCASE;
    // `max(0.0)` is the overscroll clamp, and it is load-bearing rather than
    // defensive — dragging past the top is a supported gesture on this screen.
    let into_list = (offset - above).max(0.0);

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped non-negative above, and ROWS bounds the result"
    )]
    let first = (into_list / ROW) as usize;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a viewport is a handful of rows"
    )]
    let fits = (viewport / ROW).ceil() as usize;

    (first, (first + fits + 3).min(ROWS))
}

// ------------------------------------------------------------------ the tree

/// The scrolling screen.
///
/// Its own widget because the *read* of the offset has to happen in a build:
/// a signal read is only a subscription while an element is building, so reading
/// it in `main`'s closure would subscribe nothing and the screen would never
/// move. Same reason as `Swatches` in the grid example.
#[derive(Debug)]
struct Gallery {
    scroll: ScrollController,
    floating: bool,
    /// Held open by `--refreshing`, and otherwise never true — see the note on
    /// why the drag does not arm it.
    refreshing: bool,
    /// `--probe`: replace the spinner with a flat magenta block.
    ///
    /// **An instrument, not a feature.** One symptom on this screen is still
    /// unexplained — something of the refresh control appears to be visible
    /// before any drag — and the trace has already shown the geometry is right
    /// at rest (`overscroll=0.00 painted=0.00`, so the control's box is
    /// zero-height and its clip is a zero-height rectangle).
    ///
    /// That leaves two candidates and this separates them in one run. A block
    /// visible at rest means the **clip** is not confining the child, and
    /// `CircularProgress` is a bystander. Nothing visible at rest means the clip
    /// works and the ring is drawing outside the geometry it was given — most
    /// likely its track, which a determinate ring at `value == 0` still paints.
    probe: bool,
}

/// What the build decided, on `VIEWW_TRACE_FRAMES`, and only when it changes.
///
/// # Why this is here rather than reasoned out
///
/// Three symptoms are open on this screen — the spinner shows before any drag,
/// never retracts, and does not appear to fade — and *one* cause would explain
/// all three: a determinate ring at `pull == 0` still draws its track, so if the
/// refresh control's clip is not confining it there is a permanent faint ring,
/// and a constant `REFRESH_MIN_ALPHA` is indistinguishable by eye from an alpha
/// that was never applied.
///
/// That is a hypothesis. The last bug on this screen took three of them, each
/// refuted in one run by a trace, so this goes in before any fix does.
///
/// **Read it against `RenderSliverRefresh`'s own line.** This says what the
/// application asked for; that says what the sliver was given and what geometry
/// it reported back. If `offset` here never leaves zero the fault is above the
/// sliver and the sliver is innocent.
///
/// Change-only, for the reason `FrameScheduler::trace` is: `stderr` is
/// unbuffered, and a write syscall per build once took an example from working
/// scrolling to an unresponsive window.
fn trace_build(offset: f32, pull: f32, alpha: f32, refreshing: bool) {
    use std::cell::Cell;
    use std::sync::OnceLock;

    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("VIEWW_TRACE_FRAMES").is_some()) {
        return;
    }

    thread_local! {
        static LAST: Cell<Option<(f32, f32, bool)>> = const { Cell::new(None) };
    }

    // Compared with a tolerance rather than exactly, or a spring settling by
    // thousandths reprints every frame and the instrument becomes the fault
    // again. `alpha` is derived from `pull`, so it is not part of the state —
    // it is printed to be checked *against* it.
    let moved = |last: Option<(f32, f32, bool)>| match last {
        Some((was_offset, was_pull, was_refreshing)) => {
            (offset - was_offset).abs() > 0.1
                || (pull - was_pull).abs() > 0.01
                || refreshing != was_refreshing
        }
        None => true,
    };

    LAST.with(|last| {
        if moved(last.get()) {
            last.set(Some((offset, pull, refreshing)));
            eprintln!(
                "build   offset={offset:8.2} pull={pull:.2} alpha={alpha:.2} refreshing={refreshing}"
            );
        }
    });
}

impl Widget for Gallery {
    fn debug_name(&self) -> &'static str {
        "Gallery"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let offset = self.scroll.offset();
        let refreshing = self.refreshing;
        let (first, last) = window(offset, SURFACE.height);

        // How far through the pull we are, 0 at rest and 1 at the trigger.
        let pull = ((-offset).max(0.0) / REFRESH).clamp(0.0, 1.0);

        // The spinner fades with the gap it lives in: full strength once the
        // pull reaches the trigger, `REFRESH_MIN_ALPHA` as the gap closes. So
        // the retraction is a fade *and* a slide, and both run off the same
        // number — the overscroll — rather than off a separate animation that
        // could disagree with the geometry.
        //
        // A running refresh is always full strength: at that point the offset is
        // back at zero and the pull-derived alpha would be the floor, which
        // would show the faintest spinner exactly when it means the most.
        let alpha = if refreshing {
            1.0
        } else {
            REFRESH_MIN_ALPHA + (1.0 - REFRESH_MIN_ALPHA) * pull
        };

        // Determinate while pulling: a ring that fills says how much further to
        // go, which a sweeping spinner cannot.
        //
        // This started as a workaround — `CircularProgress::indeterminate()`
        // wraps itself in a `RepaintBoundary`, and an enclosing `Opacity` used
        // to have no effect across one. That is fixed in the framework now
        // (`RenderObject::layer_effect`), so either would fade. It stays because
        // it is the better affordance, not because it is the only one that works.
        //
        // Indeterminate once refreshing, where the work genuinely has no
        // measurable progress to report — and which is now also the case that
        // proves the fix on screen, since it is a boundary inside a fade.
        let spinner: WidgetNode = if self.probe {
            // Flat, opaque and unmissable, and *not* a ring: the question is
            // whether the clip confines the child at all, so the child must be
            // something with no geometry of its own to be wrong about.
            ColoredBox::new(Color::rgb(255, 0, 255))
                .child(SizedBox::from_size(Size::new(120.0, 40.0)))
                .into()
        } else if refreshing {
            CircularProgress::indeterminate().into()
        } else {
            CircularProgress::new(pull).into()
        };

        trace_build(offset, pull, alpha, refreshing);

        let bar = SliverAppBar::new(BAR_MAX, BAR_MIN).child(
            Container::new().gradient(header_gradient()).child(
                Center::new().child(
                    Text::new("vieww — slivers and paint")
                        .size(19.0)
                        .bold()
                        .color(Color::WHITE),
                ),
            ),
        );
        // Two behaviours, one flag. They are mutually exclusive in effect and
        // the interesting comparison is between them, not both at once.
        let bar = if self.floating {
            bar.floating()
        } else {
            bar.pinned()
        };

        CustomScrollView::vertical()
            .offset(offset)
            .children(children![
                // First, and it must be: it lives in the gap that opens above
                // exactly one sliver.
                SliverRefresh::new(REFRESH)
                    .refreshing(refreshing)
                    .child(Opacity::new(alpha).child(Center::new().child(spinner))),
                bar,
                showcase(),
                SliverList::new(ROWS, ROW).builder(row).window(first, last),
            ])
            .into()
    }
}

widget_node_from!(Gallery);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let floating = has_flag("--floating");
    let refreshing = has_flag("--refreshing");
    let probe = has_flag("--probe");

    let report = App::new()
        .title("vieww — gallery")
        .size(SURFACE)
        .background(INK)
        .run(|driver| {
            let runtime = driver.elements().runtime().clone();
            // **iOS physics, and not as a preference.** `Overscroll::Clamp` —
            // which is what `android()` and the platform default give on this
            // machine — pins the offset at zero, so no gap ever opens above the
            // first sliver and `SliverRefresh` is unreachable by dragging.
            // Bounce lets the offset go negative, which is exactly what
            // `RenderScrollView` accepts and what the refresh control lives in.
            let scroll = ScrollController::new(&runtime, ScrollPhysics::ios());
            // Without this a fling scrolls under the finger and stops dead the
            // moment it lifts: `Tickers` is what advances the simulation.
            scroll.attach(driver.tickers());

            // The scroll view reports no extents — `RenderScrollView` can answer
            // `max_scroll_extent()` but nothing carries it back out to a
            // controller the way `Viewport::on_extents` does. So the maximum is
            // stated here, and until it is, every drag clamps against a maximum
            // of zero and the screen does not move at all.
            scroll.resize(ScrollExtents::new(SURFACE.height, CONTENT));

            let gallery = Gallery {
                scroll: scroll.clone(),
                floating,
                refreshing,
                probe,
            };

            // `CustomScrollView` is a bare render widget carrying an offset;
            // unlike `Scrollable` it wires no gestures of its own. So this is
            // `Scrollable::build`'s gesture half, by hand — which is the second
            // thing an application has to re-derive to use one, after the window.
            //
            // The controller's own handlers rather than hand-written closures:
            // `on_drag_end` flings with `details.timestamp`, and a fling given
            // any other clock integrates against the wrong elapsed time. It is
            // the sort of detail that looks like a physics-tuning problem.
            let dragged = scroll.on_drag();
            let released = scroll.on_drag_end();
            let wheeled = scroll.on_drag();
            // A wheel gets a release too — see the note on the handler.
            let wheel_released = scroll.on_drag_end();

            let root = GestureDetector::new()
                .drag_axis(Axis::Vertical)
                // An `Rc<dyn Fn(_)>` derefs to something callable but does not
                // itself implement `Fn`, so each phase gets a wrapper. One
                // handler, two phases — the same cost `Scrollable` pays.
                .on_drag_start({
                    let dragged = Rc::clone(&dragged);
                    move |details: DragDetails| dragged(details)
                })
                .on_drag_update(move |details: DragDetails| dragged(details))
                .on_drag_end(move |details: DragDetails| released(details))
                // A wheel notch goes to the handler a drag does, with no
                // velocity. Treating it separately would mean two sets of
                // clamping and two sets of overscroll, and they would drift.
                //
                // **And to the release handler, because nothing else ever
                // will.** A notch is a press, a movement and a release at one
                // instant; reporting only the middle is what left this screen's
                // refresh gap hanging open with no gesture able to close it.
                // `Scrollable` now does the same, and this is its gesture half
                // by hand — so the two must agree or the example stops
                // demonstrating the widget.
                .on_scroll(move |event: ScrollEvent| {
                    let details = DragDetails {
                        position: event.position,
                        local: Offset::ZERO,
                        delta: Offset::new(0.0, event.delta.dy),
                        // A wheel has no fling. Reporting one launches the list
                        // into a simulation nobody asked for by flicking.
                        velocity: Offset::ZERO,
                        // The event's clock, not `Duration::ZERO`: a spring
                        // started at time zero is already finished, and snaps.
                        timestamp: event.timestamp,
                        // `ScrollEvent` does not carry modifiers, so there is
                        // nothing truthful to forward. `NONE` rather than a
                        // guess: a synthesised ⌥ would make this example claim
                        // a modifier the user never held.
                        modifiers: Modifiers::NONE,
                    };
                    wheeled(details);
                    wheel_released(details);
                })
                .child(gallery);

            // `FrameDriver::set_root`, not `driver.elements().set_root` — the
            // driver keeps the root so it can republish it under an
            // `Inherited<ViewMetrics>`, and going straight to the element tree
            // leaves it with none to republish.
            driver.set_root(root);
        })?;

    println!("{report}");
    Ok(())
}

fn has_flag(flag: &str) -> bool {
    std::env::args().skip(1).any(|arg| arg == flag)
}
