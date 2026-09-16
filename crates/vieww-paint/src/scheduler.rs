//! Frame scheduling: when the pipeline runs, and in what order.
//!
//! A UI does not redraw when something changes. It *notes* that something
//! changed, and redraws once, later, when the display is ready for a new frame.
//! Everything in this module exists to make that sentence true.
//!
//! # Why the trees are not mentioned here
//!
//! The paint layer sits below the element and render layers, so a scheduler
//! living here cannot name `ElementTree` or `RenderOwner` — the dependency points
//! the other way. It drives them through [`FrameSink`] instead, and that
//! inversion turns out to be the right shape regardless: frame timing, phase
//! discipline and budget accounting have nothing to do with what the phases
//! happen to do. A scheduler sits below its renderer for the same reason
//! and communicates the same way, through callbacks.
//!
//! # Why time is a parameter
//!
//! Nothing here reads a clock. The platform's vsync signal supplies `now` to
//! [`FrameScheduler::pulse`], which means the display drives the pipeline rather
//! than the pipeline guessing at the display — and it means the frame logic is
//! testable to the microsecond instead of by sleeping and hoping.

use std::fmt;
use std::time::Duration;

/// Where the pipeline is within a frame.
///
/// Worth exposing because several operations are only legal in some phases.
/// Marking a widget pending during [`Paint`](FramePhase::Paint), for instance, is a
/// bug: build and layout have already run, so the change cannot take effect this
/// frame, and if it silently schedules another one you get a repaint loop that
/// pegs the GPU at 100% and looks, from the outside, like the animation is simply
/// smooth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FramePhase {
    /// No frame in progress.
    Idle,
    /// Animations advance and may pending widgets.
    ///
    /// First, so that anything an animation changes is picked up by the build in
    /// the *same* frame rather than trailing it by one.
    Animate,
    /// Pending elements rebuild; the widget tree is reconciled.
    Build,
    /// Render objects measure and place.
    Layout,
    /// Pending layers re-record their scenes.
    Paint,
    /// Layers are assembled and handed to the backend.
    Composite,
}

impl FramePhase {
    /// The work phases, in the order a frame runs them.
    pub const PIPELINE: [Self; 5] = [
        Self::Animate,
        Self::Build,
        Self::Layout,
        Self::Paint,
        Self::Composite,
    ];

    /// `true` if the widget tree may be marked pending during this phase.
    ///
    /// Build is included: a widget's `build` legitimately mounts children that
    /// are themselves pending. Layout onward is not — by then the tree's shape is
    /// fixed for this frame.
    #[must_use]
    pub const fn allows_marking_pending(self) -> bool {
        matches!(self, Self::Idle | Self::Animate | Self::Build)
    }
}

impl fmt::Display for FramePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Idle => "idle",
            Self::Animate => "animate",
            Self::Build => "build",
            Self::Layout => "layout",
            Self::Paint => "paint",
            Self::Composite => "composite",
        };
        f.write_str(name)
    }
}

/// What a frame is told about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameInfo {
    /// Frames produced so far, this one included. Starts at 1.
    pub number: u64,
    /// The vsync time this frame is being drawn for.
    pub timestamp: Duration,
    /// Time since the previous frame's timestamp.
    ///
    /// Animations must integrate against this rather than against the frame
    /// budget: a dropped frame doubles the real interval, and an animation that
    /// assumes a fixed step will visibly stutter instead of skipping ahead.
    pub delta: Duration,
}

/// What one frame cost, phase by phase.
// `Eq` is gone rather than the `f32` being made comparable: `damage_area` is a
// measurement, and a measurement is the one kind of field for which exact
// equality is not a question anybody should be asking.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub number: u64,
    pub timestamp: Duration,
    pub animate: Duration,
    pub build: Duration,
    pub layout: Duration,
    pub paint: Duration,
    pub composite: Duration,
    /// Wall time from the start of `animate` to the end of `composite`.
    pub total: Duration,
    /// The budget this frame was measured against.
    pub budget: Duration,
    /// How much of the surface this frame actually had to redraw, in square
    /// logical pixels, and how many separate regions that was.
    ///
    /// # Why cost-in-pixels belongs beside cost-in-milliseconds
    ///
    /// A frame that took 3ms is not interpretable on its own. Three
    /// milliseconds to repaint the whole screen is excellent; three to repaint
    /// one checkbox is a bug worth finding. `Damage` has always known the
    /// answer — [`Damage::covered_area`](crate::Damage::covered_area) and
    /// [`Damage::regions`](crate::Damage::regions) — and nothing carried it out
    /// to where the timings are read, so every performance conversation about
    /// this framework was missing its denominator.
    ///
    /// **Filled in by the driver, not by the scheduler**, which is why it is
    /// zero on a `FrameStats` the scheduler produced alone: damage lives on the
    /// `FrameDriver` and the scheduler is deliberately ignorant of it. See
    /// `FrameDriver::drive`.
    pub damage_area: f32,
    /// How many separate damaged regions this frame had. See
    /// [`damage_area`](Self::damage_area).
    pub damage_regions: usize,
}

impl FrameStats {
    /// `true` if this frame took longer than its budget — a dropped frame.
    #[must_use]
    pub fn over_budget(&self) -> bool {
        self.total > self.budget
    }

    /// The damaged area as a fraction of `surface`, in `0.0..=1.0`.
    ///
    /// `None` for a zero-area surface, which is a window being torn down rather
    /// than a frame worth reporting a percentage for.
    #[must_use]
    pub fn damage_fraction(&self, surface: vieww_foundation::Size) -> Option<f32> {
        let total = surface.width * surface.height;
        (total > 0.0).then(|| (self.damage_area / total).clamp(0.0, 1.0))
    }

    /// How long a single phase took.
    #[must_use]
    pub fn phase_time(&self, phase: FramePhase) -> Duration {
        match phase {
            FramePhase::Idle => Duration::ZERO,
            FramePhase::Animate => self.animate,
            FramePhase::Build => self.build,
            FramePhase::Layout => self.layout,
            FramePhase::Paint => self.paint,
            FramePhase::Composite => self.composite,
        }
    }

    /// The phase that took longest — where to look first when a frame janks.
    #[must_use]
    pub fn slowest_phase(&self) -> FramePhase {
        FramePhase::PIPELINE
            .into_iter()
            .max_by_key(|&phase| self.phase_time(phase))
            .unwrap_or(FramePhase::Idle)
    }
}

impl fmt::Display for FrameStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frame {} {:.2}ms/{:.2}ms{}",
            self.number,
            self.total.as_secs_f64() * 1000.0,
            self.budget.as_secs_f64() * 1000.0,
            if self.over_budget() { " JANK" } else { "" }
        )?;
        for phase in FramePhase::PIPELINE {
            let time = self.phase_time(phase);
            if !time.is_zero() {
                write!(f, " {phase}={:.2}ms", time.as_secs_f64() * 1000.0)?;
            }
        }
        // Only when there was damage: a clean frame printing "damage=0" every
        // line is noise in exactly the log somebody is reading to find a busy
        // frame.
        if self.damage_regions > 0 {
            write!(
                f,
                " damage={:.0}px2/{}r",
                self.damage_area, self.damage_regions
            )?;
        }
        Ok(())
    }
}

/// The pipeline a frame drives, supplied by the layers above the paint layer.
///
/// Implemented once, by whatever owns the element and render trees. The methods
/// are called in [`FramePhase::PIPELINE`] order and nowhere else, which is the
/// scheduler's entire contribution: the ordering is a property of the framework
/// rather than of each application remembering it.
pub trait FrameSink {
    /// Advance animations. May mark widgets pending for this frame's build.
    fn animate(&mut self, frame: &FrameInfo) {
        let _ = frame;
    }

    /// Rebuild pending elements.
    fn build(&mut self, frame: &FrameInfo) {
        let _ = frame;
    }

    /// Lay out pending render objects.
    fn layout(&mut self, frame: &FrameInfo) {
        let _ = frame;
    }

    /// Re-record pending layers.
    fn paint(&mut self, frame: &FrameInfo) {
        let _ = frame;
    }

    /// Assemble the layers and submit them.
    fn composite(&mut self, frame: &FrameInfo) {
        let _ = frame;
    }
}

/// Turns "something changed" into "one frame, at the next vsync".
///
/// # The coalescing that matters
///
/// [`request_frame`](Self::request_frame) is idempotent within a frame interval.
/// A handler that changes twenty pieces of state calls it twenty times and gets
/// one frame — without that, an event handler's cost would scale with how many
/// things it touched rather than with how much of the screen changed.
///
/// A request made *during* a frame schedules the **next** one. Extending the
/// frame in progress would let a build that marks itself pending spin forever inside a
/// single vsync, with no frame ever reaching the screen.
#[derive(Debug, Clone)]
pub struct FrameScheduler {
    interval: Duration,
    phase: FramePhase,
    requested: bool,
    frame_count: u64,
    last_timestamp: Option<Duration>,
    last_stats: Option<FrameStats>,
    janked: u64,
}

impl FrameScheduler {
    /// A scheduler for a display refreshing at `hz`.
    ///
    /// # Panics
    ///
    /// If `hz` is not finite and positive.
    #[must_use]
    pub fn new(hz: f32) -> Self {
        assert!(
            hz.is_finite() && hz > 0.0,
            "refresh rate must be finite and positive, got {hz}"
        );
        Self {
            interval: Duration::from_secs_f32(1.0 / hz),
            phase: FramePhase::Idle,
            requested: false,
            frame_count: 0,
            last_timestamp: None,
            last_stats: None,
            janked: 0,
        }
    }

    /// A scheduler for a 60Hz display.
    #[must_use]
    pub fn sixty_hz() -> Self {
        Self::new(60.0)
    }

    /// Follow a display whose refresh rate is now known, or has changed.
    ///
    /// Only the budget moves: pacing comes from the swapchain, which blocks on
    /// vsync whatever this says. The frame in progress is left alone — its
    /// budget was decided when it started, and rejudging it against a number
    /// that arrived halfway through would make one frame's jank verdict depend
    /// on when a monitor was queried.
    ///
    /// A non-finite or non-positive `hz` is **ignored** rather than a panic:
    /// this is fed by a platform query, and a display that reports nonsense
    /// should not take the process with it. [`new`](Self::new) still panics,
    /// because that one is fed by an application.
    pub fn set_refresh_rate(&mut self, hz: f32) {
        if hz.is_finite() && hz > 0.0 {
            self.interval = Duration::from_secs_f32(1.0 / hz);
        }
    }

    /// How long a frame has before it is late.
    #[must_use]
    pub const fn budget(&self) -> Duration {
        self.interval
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> FramePhase {
        self.phase
    }

    /// `true` if a frame is currently being produced.
    #[must_use]
    pub fn is_in_frame(&self) -> bool {
        self.phase != FramePhase::Idle
    }

    /// Frames produced so far.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Frames that ran over budget.
    #[must_use]
    pub const fn janked_frames(&self) -> u64 {
        self.janked
    }

    /// The most recent frame's cost, if any frame has run.
    #[must_use]
    pub const fn last_stats(&self) -> Option<FrameStats> {
        self.last_stats
    }

    /// Record this frame's damage on the stats already produced.
    ///
    /// Called by `FrameDriver::drive` immediately after [`pulse`](Self::pulse).
    /// It exists because the scheduler measures *time* and the driver owns
    /// *damage*, and the alternative — handing the scheduler a `Damage` — would
    /// make the timing layer depend on the compositing layer to report a number
    /// it does not use.
    ///
    /// A no-op when no frame was produced, so a caller need not check.
    pub fn annotate_last_damage(&mut self, area: f32, regions: usize) {
        if let Some(stats) = &mut self.last_stats {
            stats.damage_area = area;
            stats.damage_regions = regions;
        }
    }

    /// Ask for a frame at the next vsync.
    ///
    /// Cheap and idempotent — call it from anywhere that changes something the
    /// screen depends on, without checking whether it has already been called.
    pub fn request_frame(&mut self) {
        self.requested = true;
    }

    /// `true` if a frame is pending.
    #[must_use]
    pub const fn is_frame_requested(&self) -> bool {
        self.requested
    }

    /// Cancel a pending request.
    ///
    /// For going idle — a backgrounded application should stop asking the display
    /// for frames rather than render into a surface nobody can see.
    pub fn cancel_frame(&mut self) {
        self.requested = false;
    }

    /// Drive one vsync.
    ///
    /// Returns `None` — having done nothing at all — when no frame was requested.
    /// That is the common case, and the reason an idle UI costs no CPU: vsync
    /// keeps firing, and the scheduler keeps declining.
    ///
    /// `now` is the vsync timestamp, and `elapsed` in the measurement closure is
    /// taken from the same source, so tests supply both and the timing is exact.
    ///
    /// # Panics
    ///
    /// If called while a frame is already in progress — a re-entrant frame means
    /// the pipeline is being driven from inside itself, which corrupts phase
    /// tracking and, left alone, recurses until the stack runs out.
    pub fn pulse<S, C>(&mut self, now: Duration, sink: &mut S, mut elapsed: C) -> Option<FrameStats>
    where
        S: FrameSink + ?Sized,
        C: FnMut() -> Duration,
    {
        assert!(
            !self.is_in_frame(),
            "pulse called during the {} phase; the pipeline is re-entrant",
            self.phase
        );
        if !self.requested {
            return None;
        }

        // Cleared *before* the phases run, so anything they pending schedules the
        // next frame instead of being swallowed by this one.
        self.requested = false;
        self.frame_count += 1;

        let delta = match self.last_timestamp {
            Some(previous) => now.saturating_sub(previous),
            // The first frame has no predecessor; one interval is the best
            // available guess and keeps animations from starting with a zero step.
            None => self.interval,
        };
        self.last_timestamp = Some(now);

        let frame = FrameInfo {
            number: self.frame_count,
            timestamp: now,
            delta,
        };

        let start = elapsed();
        let mut mark = start;
        let mut times = [Duration::ZERO; 5];
        for (slot, phase) in times.iter_mut().zip(FramePhase::PIPELINE) {
            self.phase = phase;
            match phase {
                FramePhase::Animate => sink.animate(&frame),
                FramePhase::Build => sink.build(&frame),
                FramePhase::Layout => sink.layout(&frame),
                FramePhase::Paint => sink.paint(&frame),
                FramePhase::Composite => sink.composite(&frame),
                FramePhase::Idle => unreachable!("PIPELINE holds no Idle"),
            }
            let next = elapsed();
            *slot = next.saturating_sub(mark);
            mark = next;
        }
        self.phase = FramePhase::Idle;

        let stats = FrameStats {
            number: frame.number,
            timestamp: now,
            animate: times[0],
            build: times[1],
            layout: times[2],
            paint: times[3],
            composite: times[4],
            total: mark.saturating_sub(start),
            budget: self.interval,
            // The driver fills these; see the field docs.
            damage_area: 0.0,
            damage_regions: 0,
        };
        if stats.over_budget() {
            self.janked += 1;
        }
        self.last_stats = Some(stats);
        Some(stats)
    }
}

impl Default for FrameScheduler {
    fn default() -> Self {
        Self::sixty_hz()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    /// Records the order phases ran in, and can cost time or pending the tree.
    #[derive(Default)]
    struct Recorder {
        calls: Vec<FramePhase>,
        frames: Vec<FrameInfo>,
        /// Time each phase should appear to take.
        cost: Duration,
        clock: Rc<Cell<Duration>>,
    }

    impl Recorder {
        fn new(clock: &Rc<Cell<Duration>>) -> Self {
            Self {
                clock: Rc::clone(clock),
                ..Self::default()
            }
        }

        fn enter(&mut self, phase: FramePhase, frame: &FrameInfo) {
            self.calls.push(phase);
            if phase == FramePhase::Animate {
                self.frames.push(*frame);
            }
            self.clock.set(self.clock.get() + self.cost);
        }
    }

    impl FrameSink for Recorder {
        fn animate(&mut self, frame: &FrameInfo) {
            self.enter(FramePhase::Animate, frame);
        }
        fn build(&mut self, frame: &FrameInfo) {
            self.enter(FramePhase::Build, frame);
        }
        fn layout(&mut self, frame: &FrameInfo) {
            self.enter(FramePhase::Layout, frame);
        }
        fn paint(&mut self, frame: &FrameInfo) {
            self.enter(FramePhase::Paint, frame);
        }
        fn composite(&mut self, frame: &FrameInfo) {
            self.enter(FramePhase::Composite, frame);
        }
    }

    fn harness() -> (FrameScheduler, Recorder, Rc<Cell<Duration>>) {
        let clock = Rc::new(Cell::new(Duration::ZERO));
        let recorder = Recorder::new(&clock);
        (FrameScheduler::sixty_hz(), recorder, clock)
    }

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn an_idle_scheduler_does_no_work_however_often_vsync_fires() {
        let (mut scheduler, mut sink, clock) = harness();

        for tick in 0..100 {
            assert!(scheduler
                .pulse(ms(tick * 16), &mut sink, || clock.get())
                .is_none());
        }
        assert!(sink.calls.is_empty(), "an idle UI must cost nothing");
        assert_eq!(scheduler.frame_count(), 0);
    }

    #[test]
    fn a_requested_frame_runs_every_phase_in_pipeline_order() {
        let (mut scheduler, mut sink, clock) = harness();
        scheduler.request_frame();

        let stats = scheduler
            .pulse(ms(16), &mut sink, || clock.get())
            .expect("a frame was requested");

        assert_eq!(sink.calls, FramePhase::PIPELINE.to_vec());
        assert_eq!(stats.number, 1);
        assert_eq!(
            scheduler.phase(),
            FramePhase::Idle,
            "the frame must end idle"
        );
    }

    #[test]
    fn many_requests_within_one_interval_coalesce_into_a_single_frame() {
        let (mut scheduler, mut sink, clock) = harness();
        for _ in 0..20 {
            scheduler.request_frame();
        }

        scheduler.pulse(ms(16), &mut sink, || clock.get());

        assert_eq!(scheduler.frame_count(), 1, "twenty changes, one frame");
        assert_eq!(sink.calls.len(), FramePhase::PIPELINE.len());
    }

    #[test]
    fn a_frame_is_not_produced_again_without_a_new_request() {
        let (mut scheduler, mut sink, clock) = harness();
        scheduler.request_frame();

        assert!(scheduler.pulse(ms(16), &mut sink, || clock.get()).is_some());
        assert!(
            scheduler.pulse(ms(32), &mut sink, || clock.get()).is_none(),
            "the request is consumed by the frame it produced"
        );
    }

    /// Marks the tree pending from inside `build`, as a self-invalidating widget would.
    struct SelfPending {
        builds: u32,
    }

    impl FrameSink for SelfPending {
        fn build(&mut self, _frame: &FrameInfo) {
            self.builds += 1;
        }
    }

    #[test]
    fn work_scheduled_during_a_frame_lands_on_the_next_one() {
        let mut scheduler = FrameScheduler::sixty_hz();
        let mut sink = SelfPending { builds: 0 };
        let clock = Duration::ZERO;

        scheduler.request_frame();
        scheduler.pulse(ms(16), &mut sink, || clock);
        // Standing in for a build that marked itself pending.
        scheduler.request_frame();

        assert_eq!(sink.builds, 1, "the frame in progress must not be extended");
        assert!(scheduler.is_frame_requested(), "the next frame is pending");

        scheduler.pulse(ms(32), &mut sink, || clock);
        assert_eq!(sink.builds, 2);
        assert_eq!(scheduler.frame_count(), 2);
    }

    #[test]
    fn cancelling_drops_a_pending_frame() {
        let (mut scheduler, mut sink, clock) = harness();
        scheduler.request_frame();
        scheduler.cancel_frame();

        assert!(scheduler.pulse(ms(16), &mut sink, || clock.get()).is_none());
        assert_eq!(scheduler.frame_count(), 0);
    }

    #[test]
    fn the_first_frame_gets_one_interval_as_its_delta_not_zero() {
        let (mut scheduler, mut sink, clock) = harness();
        scheduler.request_frame();
        scheduler.pulse(ms(5000), &mut sink, || clock.get());

        assert_eq!(
            sink.frames[0].delta,
            scheduler.budget(),
            "a zero first step would make every animation jump on frame two"
        );
    }

    #[test]
    fn delta_is_measured_between_vsyncs_so_a_dropped_frame_is_visible() {
        let (mut scheduler, mut sink, clock) = harness();

        scheduler.request_frame();
        scheduler.pulse(ms(16), &mut sink, || clock.get());
        scheduler.request_frame();
        // Vsync at 48ms: one frame was missed entirely.
        scheduler.pulse(ms(48), &mut sink, || clock.get());

        assert_eq!(
            sink.frames[1].delta,
            ms(32),
            "animations must integrate real time, not the nominal budget"
        );
    }

    #[test]
    fn each_phase_is_timed_separately() {
        let (mut scheduler, mut sink, clock) = harness();
        sink.cost = ms(2);
        scheduler.request_frame();

        let stats = scheduler
            .pulse(ms(16), &mut sink, || clock.get())
            .expect("frame");

        for phase in FramePhase::PIPELINE {
            assert_eq!(stats.phase_time(phase), ms(2), "{phase}");
        }
        assert_eq!(stats.total, ms(10));
    }

    #[test]
    fn a_slow_frame_is_counted_as_jank() {
        let (mut scheduler, mut sink, clock) = harness();
        sink.cost = ms(10); // 50ms total, well past a 16.67ms budget
        scheduler.request_frame();

        let stats = scheduler
            .pulse(ms(16), &mut sink, || clock.get())
            .expect("frame");

        assert!(stats.over_budget(), "{stats}");
        assert_eq!(scheduler.janked_frames(), 1);
    }

    #[test]
    fn a_frame_inside_budget_is_not_jank() {
        let (mut scheduler, mut sink, clock) = harness();
        sink.cost = Duration::from_micros(500); // 2.5ms total
        scheduler.request_frame();

        let stats = scheduler
            .pulse(ms(16), &mut sink, || clock.get())
            .expect("frame");

        assert!(!stats.over_budget(), "{stats}");
        assert_eq!(scheduler.janked_frames(), 0);
    }

    #[test]
    fn the_slowest_phase_is_reported_for_diagnosis() {
        let stats = FrameStats {
            number: 1,
            timestamp: ms(16),
            animate: ms(1),
            build: ms(2),
            layout: ms(9),
            paint: ms(3),
            composite: ms(1),
            total: ms(16),
            budget: ms(16),
            damage_area: 0.0,
            damage_regions: 0,
        };
        assert_eq!(stats.slowest_phase(), FramePhase::Layout);
    }

    #[test]
    fn marking_pending_is_legal_up_to_build_and_not_after() {
        assert!(FramePhase::Idle.allows_marking_pending());
        assert!(FramePhase::Animate.allows_marking_pending());
        assert!(
            FramePhase::Build.allows_marking_pending(),
            "a build may mount children that are themselves pending"
        );
        assert!(
            !FramePhase::Layout.allows_marking_pending(),
            "the tree's shape is fixed once layout starts"
        );
        assert!(!FramePhase::Paint.allows_marking_pending());
        assert!(!FramePhase::Composite.allows_marking_pending());
    }

    #[test]
    #[should_panic(expected = "re-entrant")]
    fn driving_the_pipeline_from_inside_itself_fails_loudly() {
        let mut scheduler = FrameScheduler::sixty_hz();
        scheduler.request_frame();
        // Standing in for a phase that reaches back into `pulse`; done directly
        // here because a sink holding the scheduler cannot also be passed to it.
        scheduler.phase = FramePhase::Build;
        let mut sink = Recorder::default();
        scheduler.pulse(ms(16), &mut sink, Duration::default);
    }

    #[test]
    fn the_budget_follows_the_refresh_rate() {
        assert_eq!(
            FrameScheduler::new(60.0).budget(),
            Duration::from_secs_f32(1.0 / 60.0)
        );
        assert_eq!(
            FrameScheduler::new(120.0).budget(),
            Duration::from_secs_f32(1.0 / 120.0)
        );
        assert!(
            FrameScheduler::new(120.0).budget() < FrameScheduler::new(60.0).budget(),
            "a faster display leaves less time, not more"
        );
    }

    #[test]
    #[should_panic(expected = "finite and positive")]
    fn a_nonsense_refresh_rate_is_rejected() {
        let _ = FrameScheduler::new(0.0);
    }

    #[test]
    fn stats_are_kept_for_the_last_frame() {
        let (mut scheduler, mut sink, clock) = harness();
        assert!(scheduler.last_stats().is_none());

        scheduler.request_frame();
        scheduler.pulse(ms(16), &mut sink, || clock.get());

        assert_eq!(scheduler.last_stats().expect("ran a frame").number, 1);
    }

    #[test]
    fn a_display_that_reports_its_rate_moves_the_budget() {
        let mut scheduler = FrameScheduler::sixty_hz();
        assert!((scheduler.budget().as_secs_f32() - 1.0 / 60.0).abs() < 1e-6);

        scheduler.set_refresh_rate(120.0);
        assert!(
            (scheduler.budget().as_secs_f32() - 1.0 / 120.0).abs() < 1e-6,
            "a 120Hz panel halves what counts as late, got {:?}",
            scheduler.budget()
        );
    }

    #[test]
    fn a_display_reporting_nonsense_is_ignored_rather_than_fatal() {
        // Fed by a platform query rather than by an application, so the answer
        // to a bad value is to keep the last good one. `new` still panics —
        // that one is fed by code somebody wrote.
        let mut scheduler = FrameScheduler::sixty_hz();
        let sixty = scheduler.budget();
        for bad in [0.0, -90.0, f32::NAN, f32::INFINITY] {
            scheduler.set_refresh_rate(bad);
            assert_eq!(scheduler.budget(), sixty, "{bad} should have been ignored");
        }
    }
}
