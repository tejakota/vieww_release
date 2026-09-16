//! The decision an event loop makes, in one place, and a way to test it.
//!
//! # Why this module exists
//!
//! Everything in this crate is testable except the one thing that decides whether
//! any of it runs. The headless harness every other test uses calls
//! [`FrameDriver::draw_frame`] in a loop, so it supplies the single thing a real
//! event loop never does — **a decision to draw**. Every bug that has escaped this
//! project's test suite lived precisely in that difference:
//!
//! - a long press that could not fire, because a test draws frames in a row and a
//!   window draws the one frame the `Down` asked for and then sleeps;
//! - a tap that reached no handler;
//! - a mouse wheel whose sign nothing pinned;
//! - a `needs_frame` that answered `true` until a frame it was itself preventing,
//!   and pinned the loop into a spin — 2923 iterations, 2 frames;
//! - a `drive` that requested a frame while an element stayed pending, and looped
//!   frames forever producing no damage;
//! - a redraw the window never delivered, stranding a request for ever.
//!
//! Four fixes were written for the last two, three were reverted, and the whole
//! suite was green for every one of them. That is not a coverage shortfall. It is a
//! missing harness, and this is it.
//!
//! # The shape: one decision, two callers
//!
//! [`next_action`] is the `about_to_wait` body, extracted. The platform crate calls
//! it to drive a real window; [`LoopHarness`] calls it against a synthetic clock
//! with no window, no GPU and no compositor. **They must be the same code** — a
//! test that reimplements the decision tests the reimplementation, which is how a
//! duplicated rule drifts and how both of the reverted fixes looked correct.
//!
//! # What it deliberately does not model
//!
//! Nothing about *presenting*. There is no surface here, so a frame's damage is
//! whatever the layer tree says and no pixels are rasterised. That is the right
//! cut: the faults this exists to catch are all decisions about **whether** to
//! draw, and none of them need a pixel to reproduce.

use std::time::Duration;

use vieww_element::Runtime;
use vieww_foundation::Size;
use vieww_paint::FrameScheduler;

use crate::FrameDriver;

/// What an event loop should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    /// Draw a frame now, and ask again afterwards.
    Draw,
    /// Nothing is owed. Sleep until an event arrives.
    Sleep,
    /// Nothing is owed **until** this point on the frame timeline. Sleep until
    /// then, or until an event arrives, whichever comes first.
    ///
    /// # Why a third answer was needed
    ///
    /// With two, "something is animating" and "something is animating right
    /// now" were the same sentence. A text caret animates for as long as it
    /// exists and changes twice a second, so an idle editor window answered
    /// `Draw` on every iteration for ever: one core at 100%, indefinitely, with
    /// nothing on screen changing but a caret. That is measured, not supposed —
    /// `viewwstudio`'s `tests/idle_cost.rs` pinned it as a known defect.
    ///
    /// The duration is on the **frame timeline** (since the application's
    /// epoch), which is the clock every other time in this crate is on. The
    /// platform layer converts it to whatever its own loop wants — winit's
    /// `ControlFlow::WaitUntil` takes an `Instant`.
    SleepUntil(Duration),
}

/// The decision, and the re-request that goes with it.
///
/// This is the body of a winit `about_to_wait`, with the platform types removed:
/// `can_draw` is the lifecycle's answer, and the return value is what the caller
/// should set its control flow to.
///
/// # Why it takes `&mut` and is not pure
///
/// Because the real decision is not pure. When the *driver* wants another frame —
/// an animation mid-flight, a pointer deadline pending — the loop has to
/// [`request_frame`](FrameScheduler::request_frame) on its behalf, since
/// [`pulse`](FrameScheduler::pulse) consumes the request before running the phases
/// and would otherwise never see one. Hiding that side effect behind a pure
/// predicate and leaving the caller to remember it is exactly the split that let
/// `needs_frame` and `about_to_wait` disagree about who was responsible for the
/// next frame.
///
/// # The rule this encodes
///
/// A `Draw` must be *self-consuming*: `pulse` clears the request, so unless
/// something asks again the next call answers `Sleep`. Any condition that survives
/// the frame meant to clear it turns this into a livelock — which is a property
/// [`LoopHarness::settle`] checks and no other test in this project can.
pub fn next_action(
    scheduler: &mut FrameScheduler,
    driver: Option<&FrameDriver>,
    can_draw: bool,
    now: Duration,
) -> LoopAction {
    let pending = driver.is_some_and(FrameDriver::needs_frame);
    let wanted = scheduler.is_frame_requested() || pending;

    if !wanted || !can_draw {
        return LoopAction::Sleep;
    }

    // **A frame somebody asked for is never deferred.** The deadline below
    // answers "when would the *driver* next want one", and a request from
    // input, a signal write or the application is not that — deferring one
    // would show a keystroke a second after it was typed.
    if !scheduler.is_frame_requested() {
        if let Some(at) = driver.and_then(|driver| driver.frame_deadline(now)) {
            if at > now {
                return LoopAction::SleepUntil(at);
            }
        }
    }

    if pending {
        scheduler.request_frame();
    }
    LoopAction::Draw
}

/// Ask for a frame if another window's frame left this tree pending.
///
/// **The rule a second window needs and a first one never could.** Input marks
/// elements pending and then asks for a frame — for the window the input arrived
/// at. A signal written by that handler marks readers pending in *every* tree
/// sharing the [`Runtime`], and nobody asked on their behalf, so the other
/// window would show the previous state until something unrelated woke it.
///
/// # Why this is edge-triggered, and must stay so
///
/// It is called **once per frame that actually ran**, never once per loop
/// iteration. `pending_count() > 0` is a standing state, and the version of this
/// project that read a standing state from `about_to_wait` spun 2923 iterations
/// for 2 frames — see [`FrameDriver::needs_frame`]. One request, answered by one
/// frame, which clears the state that prompted it.
///
/// # Known, and bounded only by the application
///
/// Two trees that dirty *each other* on every frame will ping-pong, exactly as
/// one tree that dirties itself every frame does. The bound that catches the
/// single-tree case lives in [`FrameDriver::drive`] and counts consecutive
/// frames of one driver, so it does not span windows. Recorded rather than
/// guarded because the guard would have to be a cross-window streak counter, and
/// nothing has yet shown a tree that needs one.
///
/// Returns whether a frame was asked for, so a caller can log or count it.
pub fn wake_if_pending(driver: &FrameDriver, scheduler: &mut FrameScheduler) -> bool {
    let owed = driver.owes_rebuild();
    if owed {
        scheduler.request_frame();
    }
    owed
}

/// One window's share of the harness.
struct HarnessWindow {
    driver: FrameDriver,
    scheduler: FrameScheduler,
    frames: u64,
}

/// An event loop with no window: the real decision, a synthetic clock, and a
/// count of what it did.
///
/// The point of it is [`settle`](Self::settle), which runs the loop to idle under
/// a bound. A framework whose loop cannot reach idle is one whose apps peg a core
/// and look frozen, and that is invisible to every other kind of test here.
///
/// # Several windows
///
/// [`with_windows`](Self::with_windows) builds more than one, **sharing one
/// [`Runtime`]** — which is what a second window is, and what makes a document
/// open in two of them one document. Every method that names no window means the
/// first one, so a single-window harness reads exactly as it did before this
/// existed.
///
/// The loop's own multi-window rule is the interesting part and it is genuinely
/// different: it must draw *each* window that owes a frame and sleep only when
/// none does. A loop that asks only the first window is a second window that
/// never animates, and no single-window test can fail on it.
///
/// # Timekeeping
///
/// The clock advances by one [`budget`](FrameScheduler::budget) per drawn
/// **iteration** — not per window drawn in it, because windows drawn in the same
/// pass are drawn for the same vsync — and **not at all** while sleeping, because
/// a sleeping loop's duration is decided by when the next event arrives and
/// nothing here is waiting for one. Phase timings are reported as zero: this
/// measures decisions, not cost.
pub struct LoopHarness {
    /// At least one, and the first is what the unqualified methods mean.
    windows: Vec<HarnessWindow>,
    /// Shared by every window, and kept so a test can mint a signal that
    /// several trees read — the whole point of having more than one.
    runtime: Runtime,
    now: Duration,
    iterations: u64,
}

impl LoopHarness {
    /// A loop over one `surface`-sized driver at 60Hz.
    #[must_use]
    pub fn new(surface: Size) -> Self {
        Self::with_windows(&[surface])
    }

    /// A loop over several windows, **sharing one reactive graph**.
    ///
    /// Which is what a second window is: a second [`FrameDriver`] built with
    /// [`FrameDriver::with_runtime`], because a driver already owns exactly the
    /// per-surface things and a [`Runtime`] is an `Rc` to the one graph the
    /// application's state lives in.
    ///
    /// # Panics
    ///
    /// If `surfaces` is empty. A loop with no window has no decision to make,
    /// and every unqualified method here names the first one.
    #[must_use]
    pub fn with_windows(surfaces: &[Size]) -> Self {
        assert!(
            !surfaces.is_empty(),
            "a loop harness needs at least one window"
        );
        let runtime = Runtime::new();
        let windows = surfaces
            .iter()
            .map(|surface| HarnessWindow {
                driver: FrameDriver::with_runtime(*surface, runtime.clone()),
                scheduler: FrameScheduler::sixty_hz(),
                frames: 0,
            })
            .collect();
        Self {
            windows,
            runtime,
            now: Duration::ZERO,
            iterations: 0,
        }
    }

    /// The first window's driver, to mount a tree or send it input.
    pub fn driver(&mut self) -> &mut FrameDriver {
        &mut self.windows[0].driver
    }

    /// The first window's scheduler, to ask for a frame the way input does.
    pub fn scheduler(&mut self) -> &mut FrameScheduler {
        &mut self.windows[0].scheduler
    }

    /// One window's driver.
    ///
    /// # Panics
    ///
    /// If there is no such window.
    pub fn window(&mut self, index: usize) -> &mut FrameDriver {
        &mut self.windows[index].driver
    }

    /// The graph every window shares.
    ///
    /// Mint signals from here: one minted from a *different* runtime marks
    /// nothing pending in these trees, and a test written against it passes
    /// while proving nothing.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// How many windows this loop is driving.
    #[must_use]
    pub fn windows(&self) -> usize {
        self.windows.len()
    }

    /// Frames actually drawn, across every window.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.windows.iter().map(|window| window.frames).sum()
    }

    /// Frames actually drawn by one window.
    ///
    /// # Panics
    ///
    /// If there is no such window.
    #[must_use]
    pub fn frames_of(&self, index: usize) -> u64 {
        self.windows[index].frames
    }

    /// Loop iterations, drawn or not.
    ///
    /// The gap between this and [`frames`](Self::frames) is the whole subject: a
    /// window that iterates thousands of times per frame is the spin that shipped.
    #[must_use]
    pub const fn iterations(&self) -> u64 {
        self.iterations
    }

    /// The synthetic clock.
    #[must_use]
    pub const fn now(&self) -> Duration {
        self.now
    }

    /// Ask for a frame on the first window, as a tap or a keystroke would.
    pub fn request_frame(&mut self) {
        self.windows[0].scheduler.request_frame();
    }

    /// Ask for a frame on one window.
    ///
    /// Which is what input does: an event arrives at the window it was aimed at,
    /// and asks that window for a frame. Nothing asks on any other window's
    /// behalf — see [`wake_if_pending`], which is the rule that covers what that
    /// leaves out.
    ///
    /// # Panics
    ///
    /// If there is no such window.
    pub fn request_frame_for(&mut self, index: usize) {
        self.windows[index].scheduler.request_frame();
    }

    /// One iteration: decide **per window**, and draw the ones that owe a frame.
    ///
    /// Returns [`LoopAction::Draw`] if any window drew, so a caller can tell "it
    /// drew" from "it went to sleep" without inferring it from a frame count.
    /// Sleeping when *any* window still owes a frame is the multi-window
    /// livelock's opposite and just as bad: a window that never animates.
    pub fn step(&mut self) -> LoopAction {
        self.iterations += 1;
        let mut drew = false;
        // The soonest deadline any window offered, for the case where no window
        // wants a frame *now*. Reported so a test can see the difference
        // between "asleep for ever" and "asleep until the caret blinks", which
        // is the whole of the defect `LoopAction::SleepUntil` closes.
        let mut sleep_until: Option<Duration> = None;

        for index in 0..self.windows.len() {
            let window = &mut self.windows[index];
            let now = self.now;
            match next_action(&mut window.scheduler, Some(&window.driver), true, now) {
                LoopAction::Draw => {}
                LoopAction::SleepUntil(at) => {
                    sleep_until = Some(sleep_until.map_or(at, |held: Duration| held.min(at)));
                    continue;
                }
                LoopAction::Sleep => continue,
            }

            // Once per iteration rather than once per window: windows drawn in
            // the same pass are drawn for the same vsync. Before the frames, so
            // the timestamp is one interval after the last — which is what a
            // vsync callback hands over, and what animations sample against.
            if !drew {
                self.now += self.windows[index].scheduler.budget();
                drew = true;
            }
            let at = self.now;

            let window = &mut self.windows[index];
            let ran = window
                .driver
                .drive(&mut window.scheduler, at, || Duration::ZERO)
                .is_some();
            if ran {
                window.frames += 1;
                // The frame that just ran may have been the one whose handler
                // wrote a signal every other window reads.
                for (other, sibling) in self.windows.iter_mut().enumerate() {
                    if other != index {
                        wake_if_pending(&sibling.driver, &mut sibling.scheduler);
                    }
                }
            }
        }

        if drew {
            LoopAction::Draw
        } else if let Some(at) = sleep_until {
            LoopAction::SleepUntil(at)
        } else {
            LoopAction::Sleep
        }
    }

    /// Run until the loop chooses to sleep, and return how many iterations it
    /// took.
    ///
    /// # Panics
    ///
    /// If it has not slept within `limit` iterations. **That panic is the reason
    /// this type exists.** A loop that keeps answering [`LoopAction::Draw`] never
    /// yields to the compositor, so the window stops updating and input backs up
    /// behind a pegged core — indistinguishable, from outside, from a freeze. Both
    /// fixes reverted on 2026-08-11 had exactly this shape and the whole suite
    /// passed for both.
    pub fn settle(&mut self, limit: u64) -> u64 {
        let start = self.iterations;
        while self.step() == LoopAction::Draw {
            assert!(
                self.iterations - start < limit,
                "the loop has not slept after {} iterations ({} frames drawn). A \
                 condition that survives the frame meant to clear it is a livelock, \
                 not a slow settle",
                self.iterations - start,
                self.frames(),
            );
        }
        self.iterations - start
    }
}

impl std::fmt::Debug for LoopHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopHarness")
            .field("now", &self.now)
            .field("iterations", &self.iterations)
            .field("windows", &self.windows.len())
            .field("frames", &self.frames())
            .field(
                "requested",
                &self
                    .windows
                    .iter()
                    .filter(|window| window.scheduler.is_frame_requested())
                    .count(),
            )
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use vieww_animation::Tween;
    use vieww_element::Signal;
    use vieww_foundation::{Axis, Color};
    use vieww_widget::{
        children, BuildContext, ColoredBox, Flex, ScrollExtents, SizedBox, Viewport, Widget,
        WidgetKind, WidgetNode,
    };

    use super::*;

    fn harness() -> LoopHarness {
        LoopHarness::new(Size::new(200.0, 200.0))
    }

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A composed widget that reads a signal, so a write marks an element pending.
    #[derive(Debug)]
    struct Repaint {
        colour: Signal<Color>,
    }

    impl Widget for Repaint {
        fn debug_name(&self) -> &'static str {
            "Repaint"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            ColoredBox::new(self.colour.get())
                .child(SizedBox::square(20.0))
                .into()
        }
    }

    vieww_widget::widget_node_from!(Repaint);

    /// A composed widget whose height comes from a signal, so that a write out of
    /// *layout* leaves an element pending after the build phase is over.
    #[derive(Debug)]
    struct Spacer {
        height: Signal<f32>,
    }

    impl Widget for Spacer {
        fn debug_name(&self) -> &'static str {
            "Spacer"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            SizedBox::height(self.height.get()).into()
        }
    }

    vieww_widget::widget_node_from!(Spacer);

    /// A composed widget with **no fixed point**: its height is chosen by reading
    /// what layout measured, and it chooses the other one every time.
    ///
    /// Nothing in the framework does this. It is the shape a handler called from
    /// layout takes when it reports unconditionally instead of on change, and it
    /// is what the streak's budget exists to survive.
    #[derive(Debug)]
    struct Toggle {
        measured: Signal<f32>,
    }

    impl Widget for Toggle {
        fn debug_name(&self) -> &'static str {
            "Toggle"
        }

        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }

        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            SizedBox::height(if self.measured.get() > 0.0 {
                0.0
            } else {
                100.0
            })
            .into()
        }
    }

    vieww_widget::widget_node_from!(Toggle);

    #[test]
    fn an_untouched_loop_sleeps_without_drawing() {
        let mut app = harness();
        app.driver()
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        assert_eq!(app.step(), LoopAction::Sleep);
        assert_eq!(
            app.frames(),
            0,
            "nothing asked for a frame, so nothing may draw one — this is the \
             property that makes an idle application cost no CPU, and the one the \
             draw-in-a-loop harness cannot express at all"
        );
    }

    #[test]
    fn one_request_draws_exactly_one_frame_and_then_sleeps() {
        let mut app = harness();
        app.driver()
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        app.request_frame();
        let iterations = app.settle(8);

        assert_eq!(app.frames(), 1, "one request, one frame");
        assert_eq!(
            iterations, 2,
            "and two iterations: the one that drew, and the one that found nothing \
             left to do. Anything more means the frame did not clear what asked \
             for it"
        );
    }

    #[test]
    fn an_animation_runs_to_completion_and_the_loop_then_sleeps() {
        let mut app = harness();
        app.driver()
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        let animation = app.driver().animation(Tween::new(0.0_f32, 1.0), ms(100));
        animation.forward(ms(0));
        app.request_frame();

        // ~100ms at 60Hz is about six frames; the bound is generous because the
        // exact count is the animation system's business, not this harness's. What
        // is being asserted is that it **ends** — an animation is the one condition
        // that legitimately keeps `needs_frame` true, so it is also the one that
        // proves the loop can leave a self-sustaining Draw state.
        let iterations = app.settle(64);

        assert!(
            app.frames() >= 5,
            "an animation over 100ms at 60Hz is several frames, got {}",
            app.frames()
        );
        assert!(!app.driver().is_animating(), "and then it is over");
        assert_eq!(
            iterations,
            app.frames() + 1,
            "every iteration drew, save the last one that slept — no iteration \
             spun without producing a frame"
        );
    }

    #[test]
    fn a_pending_element_left_by_a_frame_does_not_livelock_the_loop() {
        // **The test neither reverted fix had.** Both put "is anything pending" into
        // the decision — one in `needs_frame`, one in `drive` — and both were
        // verified against a suite that draws frames unconditionally, where the
        // condition always clears because the next frame always runs. In a real
        // loop the frame is the thing being decided, so a condition that outlives
        // it re-decides `Draw` for ever.
        //
        // This mounts a tree, settles it, marks an element pending *without* asking for a
        // frame, and requires the loop to still reach sleep. It fails on any
        // implementation that treats a pending element as a standing reason to draw.
        let mut app = harness();
        let colour = app.driver().elements().runtime().signal(Color::RED);

        app.driver().elements().set_root(Repaint {
            colour: colour.clone(),
        });

        app.request_frame();
        app.settle(8);
        let settled = app.frames();

        // Pending, and deliberately unannounced.
        colour.set(Color::BLUE);
        assert!(
            app.driver().elements().pending_count() > 0,
            "the write landed and its reader is waiting to rebuild"
        );

        let iterations = app.settle(8);

        assert_eq!(
            iterations, 1,
            "one iteration, and it slept. A pending element is a real reason to want \
             a frame and still not a reason this decision may answer Draw to — the \
             version that did spun 2923 times for 2 frames"
        );
        assert_eq!(
            app.frames(),
            settled,
            "and drew nothing. A write with no request behind it is an event \
             handler that forgot to ask, and this decision is not the place to \
             cover for it — the one write that genuinely has nobody to ask on its \
             behalf comes out of layout, and `drive` requests that frame itself. \
             See `a_signal_written_from_layout_reaches_the_screen_and_stops`"
        );
    }

    #[test]
    fn a_signal_written_from_layout_reaches_the_screen_and_stops() {
        // **Both halves of the third `needs_frame` attempt, in the only place
        // that can hold them at once.** `RenderViewport` reports its extents out
        // of layout and the handler writes a signal, which lands after
        // `poll_states` and `rebuild_pending` have both run: nothing but the driver
        // is in a position to ask for the frame that shows the correction. So
        // `drive` asks — and the two reverted fixes asked in a way that never
        // stopped asking.
        //
        // A `FrameDriver` test can show the first half: it drives frames itself,
        // so a condition that survives the frame meant to clear it just draws
        // another frame and the loop reads as progress. The second half only
        // exists here, where the same condition re-decides `Draw` for ever and
        // `settle`'s bound is what says so — with no window, and in a suite that
        // passed for both reverted fixes.
        let mut app = harness();
        let content = app.driver().elements().runtime().signal(0.0_f32);
        let sink = content.clone();

        app.driver().set_root(Flex::column().children(children![
            Viewport::new(Axis::Vertical)
                .on_extents(Rc::new(move |extents: ScrollExtents| sink.set(extents.content)))
                .child(SizedBox::from_size(Size::new(50.0, 500.0))),
            Spacer {
                height: content.clone()
            },
        ]));

        app.request_frame();
        let iterations = app.settle(8);

        assert_eq!(
            app.frames(),
            1,
            "**one** frame, and it is not stale. This used to assert two: layout \
             measured the content, marked its reader pending, and the driver had \
             to ask for a second frame to show the correction — so the frame that \
             went to the screen was built against the previous answer. \
             `FrameSink::layout` now settles before painting, so the measuring \
             frame is also the correct one. Drew {} frames",
            app.frames()
        );
        assert_eq!(
            app.driver().elements().pending_count(),
            0,
            "and the frames it asked for are the frames that cleared the pending work"
        );
        assert!(
            !app.scheduler().is_frame_requested(),
            "with nothing owed at the end of it, so the next event draws one \
             frame rather than resuming a chain"
        );
        assert_eq!(
            content.peek(),
            500.0,
            "the value that travelled from layout to its reader is the content \
             extent the viewport measured. `peek`, not `get`, so asserting does \
             not subscribe this test and leave the tree pending behind itself"
        );

        // The bound is the assertion, and `settle` has already made it: this
        // states what the number actually was, because a settle that starts
        // needing four frames is a regression even though it terminates.
        assert_eq!(
            iterations, 2,
            "settling took {iterations} iterations: one that drew the frame, and \
             one that looked, found nothing owed, and slept. The shape used to \
             be *two* drawing iterations and a sleep, because the correction \
             cost a round trip through the event loop — the frame count above is \
             where that shows, and this is the loop overhead around it"
        );
    }

    #[test]
    fn a_chain_of_layout_writes_settles_rather_than_stopping_half_way() {
        // **The case a rising edge misses, and the reason the request is a
        // bounded streak instead.** `rebuild_pending` drains the whole pending set
        // every frame, so a non-zero count at the end of one is not pending work that
        // has been standing — it is pending work this frame's *layout* just made. A
        // chain therefore reads as a plateau rather than as an edge:
        //
        //   frame 1  V1 measures 500 and writes s1     pending = 2
        //   frame 2  the spacer inside V2 rebuilds to 500, so V2 now measures
        //            500 and writes s2                 pending = 1  ← no 0 -> N
        //   frame 3  s2's reader rebuilds, nothing moves, settled
        //
        // `pending > 0 && was == 0` asks after frame 1 and never again, so frame 3
        // does not happen and `s2`'s reader is left showing the old value. This
        // test fails on that implementation with `pending_count() == 1`.
        let mut app = harness();
        let inner = app.driver().elements().runtime().signal(0.0_f32);
        let outer = app.driver().elements().runtime().signal(0.0_f32);
        let (inner_sink, outer_sink) = (inner.clone(), outer.clone());

        app.driver().set_root(Flex::column().children(children![
            // Measures a fixed child, so it writes `inner` once and then agrees
            // with itself for ever.
            Viewport::new(Axis::Vertical)
                .on_extents(Rc::new(move |e: ScrollExtents| inner_sink.set(e.content)))
                .child(SizedBox::from_size(Size::new(50.0, 500.0))),
            // Measures a child whose height is what the first viewport reported,
            // so its own extents cannot be right until that rebuild has
            // happened. This is the link that makes it a chain.
            Viewport::new(Axis::Vertical)
                .on_extents(Rc::new(move |e: ScrollExtents| outer_sink.set(e.content)))
                .child(Spacer {
                    height: inner.clone()
                }),
            Spacer {
                height: outer.clone()
            },
        ]));

        app.request_frame();
        let iterations = app.settle(16);

        assert_eq!(
            app.frames(),
            1,
            "a two-link chain used to need three frames — measure, propagate, \
             settle — and each of the first two put a half-finished tree on the \
             screen. The settle loop walks the chain **inside** one frame, one \
             pass per link, so the only frame drawn is the finished one. Drew {}",
            app.frames()
        );
        assert_eq!(
            app.driver().elements().pending_count(),
            0,
            "and nothing is left waiting to rebuild at the end of it"
        );
        assert_eq!(
            outer.peek(),
            500.0,
            "the far end of the chain has the value that started at the near \
             end, which is the thing a screen would be showing"
        );
        assert_eq!(
            iterations, 2,
            "settled in {iterations} iterations — **the same number as the \
             single-link case** in \
             `a_signal_written_from_layout_reaches_the_screen_and_stops`, which \
             is the property worth pinning: one iteration to draw, one to sleep, \
             and the chain length no longer costs an event-loop round trip per \
             link. It used to cost one apiece.\n\nA chain longer than \
             `MAX_LAYOUT_PENDING_FRAMES` still gives up rather than spinning; \
             `layout_that_never_settles_gives_up_instead_of_spinning` is that \
             bound, and moving the loop inside the frame did not weaken it"
        );
    }

    #[test]
    fn layout_that_never_settles_gives_up_instead_of_spinning() {
        // **The reason the streak has a bound at all**, and the failure both
        // reverted fixes shipped. This tree cannot settle: the viewport measures
        // its child, the child's height is chosen by reading what was measured,
        // and it chooses the opposite one each time. Every frame's layout writes
        // a genuinely new value, so `report_extents`' change-only guard never
        // deduplicates and there is no fixed point to reach.
        //
        // An unbounded `pending > 0` request draws frames at 60Hz for ever here
        // and `settle` never returns. The budget makes the whole failure a fixed
        // cost — nine frames, one message on stderr naming `Toggle`, and a loop
        // that goes back to sleep with the content stale.
        //
        // Stale is the deliberate direction. The application stays responsive
        // and every other part of it stays correct, which is not true of a
        // pegged core, and the message says exactly which widget to look at.
        let mut app = harness();
        let measured = app.driver().elements().runtime().signal(0.0_f32);
        let sink = measured.clone();

        app.driver().set_root(
            Viewport::new(Axis::Vertical)
                .on_extents(Rc::new(move |e: ScrollExtents| sink.set(e.content)))
                .child(Toggle {
                    measured: measured.clone(),
                }),
        );

        app.request_frame();
        // Twice the budget, so the bound is what stops it rather than this.
        app.settle(2 * u64::from(FrameDriver::MAX_LAYOUT_PENDING_FRAMES));

        assert_eq!(
            app.frames(),
            u64::from(FrameDriver::MAX_LAYOUT_PENDING_FRAMES + 1),
            "the budget's worth of frames, plus the one that spends the last of \
             it and finds the tree still pending"
        );
        assert!(
            app.driver().elements().pending_count() > 0,
            "and it gave up holding pending work, which is the point — the alternative \
             to stale content here is not correct content, it is a loop that \
             never sleeps again"
        );
        assert!(
            !app.scheduler().is_frame_requested(),
            "nothing owed, so the loop is genuinely idle rather than owing a \
             frame it has decided not to draw"
        );

        // **And it recovers.** The streak counts *consecutive* pending frames, so
        // an application that oscillated once is not left in the degraded state
        // for the rest of its life. Settling the tree by hand and drawing one
        // clean frame is enough to arm the request again.
        app.driver()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));
        app.request_frame();
        app.settle(4);
        assert_eq!(
            app.driver().elements().pending_count(),
            0,
            "a tree with nothing writing from layout settles in one frame, and \
             the driver is no longer refusing to ask"
        );
    }

    #[test]
    fn a_request_is_always_consumed_by_the_frame_it_asked_for() {
        // The lost-redraw property. `about_to_wait` used to hand the request to the
        // window and draw only when a `RedrawRequested` came back — and
        // `request_redraw` is a no-op while one is already pending, so a single
        // dropped redraw stranded `requested` at true for ever, with the loop
        // polling at CPU speed against an event that could never arrive.
        //
        // Measured before the fix: a frame requested at 36.4s, `poll
        // requested=true` every iteration to 37.7s, no redraw, no frame.
        //
        // Drawing inside the decision makes this structural: the request cannot
        // outlive the iteration that saw it.
        let mut app = harness();
        app.driver()
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        for _ in 0..5 {
            app.request_frame();
            app.settle(8);
            assert!(
                !app.scheduler().is_frame_requested(),
                "a request that survives its own settle is a stranded frame, and a \
                 window that never draws again"
            );
        }

        assert_eq!(app.frames(), 5, "five requests, five frames");
    }

    #[test]
    fn a_lifecycle_that_forbids_drawing_sleeps_even_with_a_frame_owed() {
        // Backgrounded, occluded, or surface-less. The request stays owed so the
        // frame is not lost, and the loop must not busy-wait on a surface it cannot
        // draw into — which is the same failure as the spin, reached from the other
        // side.
        let mut scheduler = FrameScheduler::sixty_hz();
        scheduler.request_frame();

        assert_eq!(
            next_action(&mut scheduler, None, false, Duration::ZERO),
            LoopAction::Sleep,
            "cannot draw, so sleep"
        );
        assert!(
            scheduler.is_frame_requested(),
            "and the frame is still owed, so resuming draws it rather than losing it"
        );
    }

    #[test]
    fn a_driver_that_wants_a_frame_has_one_requested_on_its_behalf() {
        // `pulse` clears `requested` before running the phases, so a driver whose
        // *previous* frame implies another — an animation, a pointer deadline — has
        // no way to ask for it itself. This is where that is done, and doing it
        // anywhere else is how an animation runs for exactly one frame and stops.
        let mut app = harness();
        app.driver()
            .elements()
            .set_root(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        let animation = app.driver().animation(Tween::new(0.0_f32, 1.0), ms(100));
        animation.forward(ms(0));

        // Nothing has requested a frame; the driver merely wants one.
        assert!(!app.scheduler().is_frame_requested());
        assert_eq!(app.step(), LoopAction::Draw);
        assert_eq!(app.frames(), 1, "and it drew, unasked-for by any input");
    }

    // ------------------------------------------------------- several windows

    /// Two windows, the second narrower, sharing one graph.
    fn two_windows() -> LoopHarness {
        LoopHarness::with_windows(&[Size::new(200.0, 200.0), Size::new(120.0, 200.0)])
    }

    #[test]
    fn two_idle_windows_sleep() {
        let mut app = two_windows();
        app.driver().set_root(ColoredBox::new(Color::RED));
        app.window(1).set_root(ColoredBox::new(Color::BLUE));

        assert_eq!(app.step(), LoopAction::Sleep);
        assert_eq!(app.frames(), 0, "two windows cost no more CPU than one");
    }

    #[test]
    fn a_second_window_that_animates_keeps_the_loop_awake_while_the_first_is_idle() {
        // **The decision a single window cannot express.** A loop that asks only
        // the first window is a second window that never animates — and every
        // single-window test passes on it, because there is nothing to ask.
        let mut app = two_windows();
        app.driver().set_root(ColoredBox::new(Color::RED));
        app.window(1)
            .set_root(ColoredBox::new(Color::BLUE).child(SizedBox::square(20.0)));

        let animation = app.window(1).animation(Tween::new(0.0_f32, 1.0), ms(100));
        animation.forward(ms(0));
        app.request_frame_for(1);

        let iterations = app.settle(64);

        assert!(
            app.frames_of(1) >= 5,
            "the animating window ran to completion, got {} frames",
            app.frames_of(1)
        );
        assert_eq!(
            app.frames_of(0),
            0,
            "and the idle one drew nothing — a window repaints because *it* \
             changed, not because a sibling did"
        );
        assert_eq!(
            iterations,
            app.frames_of(1) + 1,
            "no iteration spun without producing a frame"
        );
    }

    #[test]
    fn a_write_in_one_window_reaches_the_other_and_the_loop_then_sleeps() {
        // **The defect a second window introduces, and the rule that closes
        // it.** Input marks elements pending and then asks for a frame — for the
        // window the input arrived at. The same write marks readers pending in
        // every tree sharing the runtime, and nobody asked on their behalf.
        //
        // Without `wake_if_pending` the second window draws nothing here and
        // shows the previous colour until something unrelated wakes it, which on
        // an idle screen is indefinitely.
        let mut app = two_windows();
        let colour = app.runtime().signal(Color::RED);

        app.driver().set_root(Repaint {
            colour: colour.clone(),
        });
        app.window(1).set_root(Repaint {
            colour: colour.clone(),
        });
        app.request_frame();
        app.request_frame_for(1);
        app.settle(8);
        let (first, second) = (app.frames_of(0), app.frames_of(1));

        // One write, and one request — the window whose handler ran.
        colour.set(Color::BLUE);
        app.request_frame();

        let iterations = app.settle(8);

        assert_eq!(
            app.frames_of(0) - first,
            1,
            "the window that was asked drew once"
        );
        assert_eq!(
            app.frames_of(1) - second,
            1,
            "**and so did the other one.** A document open in two windows is one \
             document, and the second window is not entitled to show the previous \
             state because the click landed elsewhere"
        );
        assert_eq!(
            iterations, 2,
            "and it settled: the iteration that drew both, and the one that found \
             nothing left. A standing `pending` read every iteration is the spin \
             that shipped — this asks once, per frame that actually ran"
        );
    }

    #[test]
    fn a_window_with_nothing_pending_is_not_woken_by_its_siblings_frame() {
        // The other direction, and the one that would make two windows cost
        // double: waking every sibling after every frame is a repaint of the
        // whole application whenever anything moves anywhere.
        let mut app = two_windows();
        let colour = app.runtime().signal(Color::RED);

        // Only the first window reads it.
        app.driver().set_root(Repaint {
            colour: colour.clone(),
        });
        app.window(1).set_root(ColoredBox::new(Color::BLUE));
        app.request_frame();
        app.request_frame_for(1);
        app.settle(8);
        let second = app.frames_of(1);

        colour.set(Color::BLUE);
        app.request_frame();
        app.settle(8);

        assert_eq!(
            app.frames_of(1),
            second,
            "the window that read nothing drew nothing"
        );
    }
}
