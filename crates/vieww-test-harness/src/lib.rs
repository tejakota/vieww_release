//! Headless event-loop test harness.
//!
//! A vieww application without a window: inject synthetic pointer, keyboard, and
//! scroll events, advance the loop by exact vsync intervals, and collect frame
//! reports after each tick.
//!
//! # Why this crate exists
//!
//! [`vieww_render::LoopHarness`] is the real thing: it runs the same
//! [`next_action`](vieww_render::next_action) decision that a winit event loop
//! makes, with a synthetic clock and no window. This crate wraps it with the
//! things a test writer reaches for every time:
//!
//! - **Event injection**: [`TestHarness::tap`], [`TestHarness::key`],
//!   [`TestHarness::scroll`] — one call, one frame request.
//! - **Time control**: [`TestHarness::tick`] advances by N vsync intervals and
//!   returns what happened.
//! - **Frame assertions**: [`FrameReport`] collects frame counts after each tick.
//!
//! # Quick start
//!
//! ```
//! use vieww_foundation::{Color, Offset, Size};
//! use vieww_test_harness::TestHarness;
//! use vieww_widget::{ColoredBox, SizedBox};
//!
//! let mut harness = TestHarness::new(Size::new(200.0, 200.0));
//! harness.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));
//!
//! // Draw the initial frame and settle.
//! let report = harness.tick_and_settle(32);
//! assert_eq!(report.frames_drawn, 1);
//! ```

mod report;
pub mod suite_report;
pub mod visual;

pub use report::FrameReport;
pub use suite_report::{TestEntry, TestReport, TestStatus};
pub use visual::{
    assert_matches_golden, assert_partial_repaint_is_complete, Difference, Frame, Persistent,
};

use std::time::Duration;

use vieww_element::Runtime;
use vieww_foundation::{
    KeyEvent, KeyState, LogicalKey, Modifiers, NamedKey, Offset, PointerEvent, PointerId,
    ScrollEvent, Size,
};
use vieww_paint::FrameScheduler;
use vieww_render::{FrameDriver, LoopAction, LoopHarness};
use vieww_widget::WidgetNode;

/// A headless vieww application for testing.
///
/// Wraps [`LoopHarness`] with event-injection helpers and frame-report
/// collection. No window, no GPU, no compositor — just the decision.
///
/// # Two modes
///
/// - **Single-window** (most tests): [`TestHarness::new`].
/// - **Multi-window**: [`TestHarness::with_windows`], sharing one reactive graph.
///
/// # Synthetic time
///
/// The clock starts at `Duration::ZERO` and advances by one vsync interval
/// (~16.667 ms) per drawn iteration — exactly as a real display does.
/// Sleeping iterations do not advance the clock, because a sleeping loop's
/// duration is decided by when the next event arrives and nothing here is
/// waiting for one.
pub struct TestHarness {
    loop_harness: LoopHarness,
}

impl TestHarness {
    /// A headless application with one `surface`-sized window at 60 Hz.
    #[must_use]
    pub fn new(surface: Size) -> Self {
        Self {
            loop_harness: LoopHarness::new(surface),
        }
    }

    /// A headless application with several windows sharing one reactive graph.
    ///
    /// # Panics
    ///
    /// If `surfaces` is empty.
    #[must_use]
    pub fn with_windows(surfaces: &[Size]) -> Self {
        Self {
            loop_harness: LoopHarness::with_windows(surfaces),
        }
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// The first window's driver — mount trees, poke state, read elements.
    pub fn driver(&mut self) -> &mut FrameDriver {
        self.loop_harness.driver()
    }

    /// The first window's scheduler — request frames.
    pub fn scheduler(&mut self) -> &mut FrameScheduler {
        self.loop_harness.scheduler()
    }

    /// One window's driver.
    ///
    /// # Panics
    ///
    /// If there is no such window.
    pub fn window(&mut self, index: usize) -> &mut FrameDriver {
        self.loop_harness.window(index)
    }

    /// The shared reactive graph.
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        self.loop_harness.runtime()
    }

    /// The synthetic clock.
    #[must_use]
    pub fn now(&self) -> Duration {
        self.loop_harness.now()
    }

    /// How many windows this harness is driving.
    #[must_use]
    pub fn window_count(&self) -> usize {
        self.loop_harness.windows()
    }

    /// Total frames drawn across all windows.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.loop_harness.frames()
    }

    /// Frames drawn by one window.
    #[must_use]
    pub fn frames_of(&self, index: usize) -> u64 {
        self.loop_harness.frames_of(index)
    }

    /// Total loop iterations, drawn or not.
    #[must_use]
    pub fn iterations(&self) -> u64 {
        self.loop_harness.iterations()
    }

    // ------------------------------------------------------------------
    // Mounting
    // ------------------------------------------------------------------

    /// Set the root widget of the first window.
    pub fn mount(&mut self, root: impl Into<WidgetNode>) {
        self.loop_harness.driver().set_root(root);
    }

    /// Set the root widget of one window.
    ///
    /// # Panics
    ///
    /// If there is no such window.
    pub fn mount_on(&mut self, index: usize, root: impl Into<WidgetNode>) {
        self.loop_harness.window(index).set_root(root);
    }

    // ------------------------------------------------------------------
    // Event injection
    // ------------------------------------------------------------------

    /// Inject a pointer event and request a frame.
    ///
    /// This is the synthetic equivalent of the platform delivering a touch
    /// or mouse event. The event is dispatched immediately; a frame is
    /// requested on the first window so the loop will draw it.
    pub fn pointer(&mut self, event: PointerEvent) {
        self.loop_harness.driver().handle_pointer(&event);
        self.loop_harness.request_frame();
    }

    /// Inject a pointer event on one window and request a frame there.
    pub fn pointer_on(&mut self, index: usize, event: PointerEvent) {
        self.loop_harness.window(index).handle_pointer(&event);
        self.loop_harness.request_frame_for(index);
    }

    /// Simulate a tap at `position` at the current time.
    ///
    /// Down then Up at the same instant. A frame is requested after each
    /// event, as the platform does.
    pub fn tap(&mut self, position: Offset) {
        let now = self.loop_harness.now();
        let id = PointerId(1);
        self.pointer(PointerEvent::down(id, position, now));
        self.pointer(PointerEvent::up(id, position, now));
    }

    /// Simulate a tap on one window.
    pub fn tap_on(&mut self, index: usize, position: Offset) {
        let now = self.loop_harness.now();
        let id = PointerId(1);
        self.pointer_on(index, PointerEvent::down(id, position, now));
        self.pointer_on(index, PointerEvent::up(id, position, now));
    }

    /// Simulate a drag from `start` to `end` over `steps` move events.
    ///
    /// Each intermediate step produces a Move event. A frame is requested
    /// after each event. The caller must [`settle`](Self::settle) or
    /// [`tick`](Self::tick) afterwards to run the loop and draw frames.
    ///
    /// Returns the number of pointer events injected (Down + moves + Up).
    pub fn drag(&mut self, start: Offset, end: Offset, steps: u32) -> u32 {
        let id = PointerId(1);
        self.pointer(PointerEvent::down(id, start, self.loop_harness.now()));

        let mut prev = start;
        for i in 1..=steps {
            let t = i as f32 / (steps + 1) as f32;
            let pos = Offset::new(
                start.dx + (end.dx - start.dx) * t,
                start.dy + (end.dy - start.dy) * t,
            );
            self.pointer(PointerEvent::moved(id, prev, pos, self.loop_harness.now()));
            prev = pos;
        }

        self.pointer(PointerEvent::up(id, end, self.loop_harness.now()));
        // Down + steps moves + Up
        1 + steps + 1
    }

    /// Inject a keyboard event and request a frame.
    pub fn key(&mut self, event: KeyEvent) {
        self.loop_harness.driver().handle_key(&event);
        self.loop_harness.request_frame();
    }

    /// Inject a typed character and request a frame.
    pub fn type_char(&mut self, ch: char) {
        self.key(KeyEvent::down(
            LogicalKey::Character(ch.into()),
            self.loop_harness.now(),
        ));
    }

    /// Inject a named key press (e.g. Enter, Escape, Backspace).
    pub fn press(&mut self, key: NamedKey) {
        self.key(KeyEvent::down(
            LogicalKey::Named(key),
            self.loop_harness.now(),
        ));
    }

    /// Inject a named key press with modifiers.
    pub fn press_with(&mut self, key: NamedKey, modifiers: Modifiers) {
        self.key(KeyEvent {
            key: LogicalKey::Named(key),
            state: KeyState::Down,
            repeat: false,
            modifiers,
            timestamp: self.loop_harness.now(),
        });
    }

    /// Inject a scroll event and request a frame.
    pub fn scroll(&mut self, position: Offset, delta: Offset) {
        let event = ScrollEvent {
            position,
            delta,
            timestamp: self.loop_harness.now(),
        };
        self.loop_harness.driver().handle_scroll(&event);
        self.loop_harness.request_frame();
    }

    /// Request a frame on the first window, as input does.
    pub fn request_frame(&mut self) {
        self.loop_harness.request_frame();
    }

    /// Request a frame on one window.
    pub fn request_frame_for(&mut self, index: usize) {
        self.loop_harness.request_frame_for(index);
    }

    // ------------------------------------------------------------------
    // Time and loop control
    // ------------------------------------------------------------------

    /// Advance the clock by `duration`, running the loop at vsync boundaries.
    ///
    /// Each vsync interval that passes, the loop's decision is evaluated:
    /// if it answers [`LoopAction::Draw`], a frame runs and the clock advances
    /// by one budget (~16.667 ms). Sleeping iterations do not advance the
    /// clock.
    ///
    /// Returns a [`FrameReport`] covering every frame drawn during this call.
    pub fn tick(&mut self, duration: Duration) -> FrameReport {
        let start_frames = self.frames();
        let start_iterations = self.iterations();
        let deadline = self.loop_harness.now() + duration;

        while self.loop_harness.now() < deadline {
            if self.loop_harness.step() == LoopAction::Sleep {
                break;
            }
        }

        FrameReport {
            frames_drawn: self.frames() - start_frames,
            iterations: self.iterations() - start_iterations,
            now: self.loop_harness.now(),
        }
    }

    /// Run the loop until it chooses to sleep.
    ///
    /// # Panics
    ///
    /// If the loop has not slept within `limit` iterations. This is the
    /// livelock detector — a loop that keeps answering `Draw` never yields
    /// to the compositor, pegging a core and freezing the window.
    pub fn settle(&mut self, limit: u64) -> FrameReport {
        let start_frames = self.frames();
        let start_iterations = self.iterations();
        self.loop_harness.settle(limit);
        FrameReport {
            frames_drawn: self.frames() - start_frames,
            iterations: self.iterations() - start_iterations,
            now: self.loop_harness.now(),
        }
    }

    /// Request a frame, settle the loop, and return the report.
    ///
    /// The most common pattern: "show me what this tap did".
    pub fn tick_and_settle(&mut self, limit: u64) -> FrameReport {
        self.request_frame();
        self.settle(limit)
    }

    /// Run exactly one loop iteration, returning whether it drew.
    pub fn step(&mut self) -> LoopAction {
        self.loop_harness.step()
    }
}

impl std::fmt::Debug for TestHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestHarness")
            .field("now", &self.loop_harness.now())
            .field("iterations", &self.loop_harness.iterations())
            .field("windows", &self.loop_harness.windows())
            .field("frames", &self.loop_harness.frames())
            .finish()
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;
    use vieww_widget::{ColoredBox, SizedBox};

    use super::*;

    fn harness() -> TestHarness {
        TestHarness::new(Size::new(200.0, 200.0))
    }

    #[test]
    fn an_idle_harness_draws_nothing() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        assert_eq!(app.step(), LoopAction::Sleep);
        assert_eq!(app.frames(), 0);
    }

    #[test]
    fn one_request_draws_one_frame_and_sleeps() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        let report = app.tick_and_settle(8);
        assert_eq!(report.frames_drawn, 1);
        assert_eq!(report.iterations, 2);
    }

    #[test]
    fn a_tap_requests_a_frame() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        app.tap(Offset::new(10.0, 10.0));
        let report = app.settle(8);
        assert!(report.frames_drawn >= 1);
    }

    #[test]
    fn tick_advances_the_clock() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        assert_eq!(app.now(), Duration::ZERO);
        app.request_frame();
        app.tick_and_settle(8);
        assert!(app.now() > Duration::ZERO);
    }

    #[test]
    fn key_injection_compiles_and_dispatches() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        app.press(NamedKey::Escape);
        let report = app.settle(8);
        assert_eq!(report.frames_drawn, 1);
    }

    #[test]
    fn type_char_injection_compiles_and_dispatches() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        app.type_char('a');
        let report = app.settle(8);
        assert_eq!(report.frames_drawn, 1);
    }

    #[test]
    fn scroll_injection_compiles_and_dispatches() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        app.scroll(Offset::new(100.0, 100.0), Offset::new(0.0, -50.0));
        let report = app.settle(8);
        assert_eq!(report.frames_drawn, 1);
    }

    #[test]
    fn drag_produces_multiple_events() {
        let mut app = harness();
        app.mount(ColoredBox::new(Color::RED).child(SizedBox::square(20.0)));

        let events = app.drag(Offset::new(10.0, 10.0), Offset::new(100.0, 100.0), 4);
        assert_eq!(events, 6, "Down + 4 moves + Up = 6 events");
        // Settling draws the frames for the injected events.
        let report = app.settle(32);
        assert!(report.frames_drawn >= 1, "drag produced at least one frame");
    }

    #[test]
    fn multi_window_harness_compiles() {
        let mut app =
            TestHarness::with_windows(&[Size::new(200.0, 200.0), Size::new(120.0, 200.0)]);
        app.mount(ColoredBox::new(Color::RED));
        app.mount_on(1, ColoredBox::new(Color::BLUE));

        assert_eq!(app.window_count(), 2);
        assert_eq!(app.step(), LoopAction::Sleep);
    }
}
