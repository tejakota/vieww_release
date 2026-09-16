//! What the frames actually cost, once they are on a screen.
//!
//! # Why `FrameScheduler`'s numbers are not enough
//!
//! [`FrameStats`] measures the five pipeline phases and stops at `composite`,
//! because that is where the paint layer's responsibility ends. Everything
//! expensive about putting a frame on a display happens *after* that: vello
//! rasterises, the queue is submitted, the swapchain image is acquired — and
//! acquiring it is where vsync actually blocks. A frame rate computed from
//! `FrameStats::total` alone would be a report on how fast the CPU can prepare
//! work, which is not a frame rate at all and is generally about ten times too
//! good.
//!
//! So this measures the two things Phase 4's exit test asks for and the
//! scheduler cannot see: the wall-clock interval between frames that reached the
//! screen, and how many pixels each of them cost.
//!
//! # Not every interval is a frame time
//!
//! An event-driven framework spends most of its life asleep, and the gap either
//! side of a nap is wall clock like any other. Counting it makes an application
//! that did nothing — correctly, at no cost — report the worst frame time in the
//! session, which is the opposite of what the number is for. So the loop tells
//! this module when it is about to sleep ([`FrameLog::about_to_sleep`]), and the
//! interval statistics are computed over the frames the application actually
//! owed. See [`Frame::continuous`].

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use vieww_paint::FrameStats;
use vieww_render::WindowKey;

/// How many recent frames the percentiles are computed over.
///
/// Four seconds at 60Hz: long enough that one slow frame does not dominate,
/// short enough that a stutter now is not hidden by a smooth minute ago.
const WINDOW: usize = 240;

/// One frame that reached the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Pipeline phases plus rasterisation and present, on the CPU.
    pub work: Duration,
    /// Wall clock since the previous presented frame. This is the number a
    /// frame rate is made of; `work` is why it is what it is.
    ///
    /// Zero on the first frame, which has no predecessor to be an interval
    /// from, and is excluded from the statistics for that reason.
    pub interval: Duration,
    /// `true` when a frame was owed for the whole of [`interval`](Self::interval)
    /// — the event loop never got to sleep in it.
    ///
    /// **This is what makes the interval a measurement of the application
    /// rather than of the user.** A frame drawn 48 seconds after the last one
    /// because nobody touched anything is not a stutter, and averaging it in
    /// says an idle app is the slowest thing on the machine. Only a gap the
    /// application was trying to fill is a gap it failed to fill.
    pub continuous: bool,
    /// Pixels vello actually rasterised, summed over the damaged regions.
    pub pixels: u64,
}

/// A rolling record of presented frames.
#[derive(Debug)]
pub struct FrameLog {
    budget: Duration,
    /// Physical pixels in the window, for stating rasterisation as a fraction.
    surface_pixels: u64,
    recent: VecDeque<Frame>,
    frames: u64,
    over_budget: u64,
    rasterised: u64,
    /// Frames that cost at least a whole surface to rasterise.
    ///
    /// Counted here rather than derived from [`recent`](Self::recent) because
    /// that window holds the last [`WINDOW`] frames and this is a claim about
    /// the whole run — a 600-frame session would otherwise report the share of
    /// its last four seconds and label it the total.
    full_repaint_frames: u64,
    /// Of those, the ones the event loop had slept before.
    ///
    /// **This is the number worth reading.** A frame that repaints everything
    /// while scrolling is the viewport moving every pixel, and is expected. A
    /// frame that repaints everything after the loop went to sleep is a screen
    /// nobody was animating, woken by one tap or one caret blink, rasterising
    /// the whole surface to show it — and that is the case damage tracking
    /// exists to avoid.
    idle_full_repaint_frames: u64,
    first: Option<Instant>,
    last: Option<Instant>,
    /// Whether the event loop has slept since the last presented frame.
    ///
    /// Set by [`about_to_sleep`](FrameLog::about_to_sleep) and cleared by the
    /// frame it applies to, so it describes exactly one interval.
    slept: bool,
    /// Which window these frames belong to.
    ///
    /// # Why the log carries it rather than the hook signature
    ///
    /// `App::on_frame` fires for **every** window, because a hook is the
    /// application's rather than any one surface's — an app with an inspector
    /// open still wants to know what its main window cost. That fan-out needs
    /// the hook to be able to tell them apart, and the two ways to do it are a
    /// second parameter on three public hook signatures, or one accessor on the
    /// thing already being passed.
    ///
    /// This is the second, and it is not merely the smaller change: the log
    /// *is* the per-window object, so a caller that has one and cannot say
    /// which window it describes is holding an unlabelled measurement. Every
    /// existing hook keeps compiling and keeps meaning what it meant, because
    /// a single-window application only ever sees `PRIMARY`.
    window: WindowKey,
}

impl FrameLog {
    /// A log for a display with the given per-frame budget.
    #[must_use]
    pub fn new(budget: Duration, surface_pixels: u64) -> Self {
        Self {
            budget,
            surface_pixels,
            recent: VecDeque::with_capacity(WINDOW),
            frames: 0,
            over_budget: 0,
            rasterised: 0,
            full_repaint_frames: 0,
            idle_full_repaint_frames: 0,
            first: None,
            last: None,
            slept: false,
            // The overwhelmingly common case, and the right default for a log
            // built by a test that has no windows at all.
            window: WindowKey::PRIMARY,
        }
    }

    /// Name the window these frames belong to.
    ///
    /// Called once, when the window is created. Separate from `new` because
    /// every test in this file builds a log without one and none of them should
    /// have to name a window to time a frame.
    pub(crate) fn set_window(&mut self, window: WindowKey) {
        self.window = window;
    }

    /// Which window these frames were drawn in.
    ///
    /// A per-frame hook fires for every open window; this is how one that only
    /// cares about the main window says so:
    ///
    /// ```
    /// # use vieww_platform_winit::FrameLog;
    /// # use vieww_render::WindowKey;
    /// # fn hook(log: &FrameLog) {
    /// if log.window() != WindowKey::PRIMARY {
    ///     return;
    /// }
    /// # }
    /// ```
    #[must_use]
    pub const fn window(&self) -> WindowKey {
        self.window
    }

    /// The event loop is about to sleep, so the next interval is not a stall.
    ///
    /// # Why the log cannot work this out for itself
    ///
    /// Because from in here a long interval and a stall are the same number.
    /// The only thing that can tell them apart is whether a frame was *owed*
    /// while the clock ran, and that is the event loop's decision —
    /// `vieww_render::next_action` answering `Sleep`. A log that guessed from
    /// the duration would need a threshold, and every threshold is wrong: 100ms
    /// of idle is not a stall and 100ms of animation is.
    ///
    /// Idempotent, and expected to be called far more often than frames are
    /// recorded — an idle loop passes through `about_to_wait` on every event it
    /// ignores.
    pub const fn about_to_sleep(&mut self) {
        self.slept = true;
    }

    /// Follow a window that changed size.
    pub const fn resize(&mut self, surface_pixels: u64) {
        self.surface_pixels = surface_pixels;
    }

    /// Follow a display whose refresh rate is now known, or has changed.
    ///
    /// The samples already recorded stay: a work duration is a measurement of
    /// this process and does not become wrong because the panel it was drawn on
    /// is faster than assumed. Only what counts as *late* changes, which is the
    /// whole point.
    ///
    /// The running `over_budget` tally is deliberately **not** recomputed. It
    /// counts frames judged at the time they happened, and a total that changed
    /// retrospectively would make two reports of the same run disagree.
    pub const fn set_budget(&mut self, budget: Duration) {
        self.budget = budget;
    }

    /// Record a frame that was presented at `at`, having cost `work`.
    ///
    /// `stats` is the pipeline's own measurement, which is folded into `work`;
    /// `None` when the scheduler declined to run a frame and only the present
    /// happened.
    pub fn record(&mut self, stats: Option<FrameStats>, gpu: Duration, pixels: u64, at: Instant) {
        let work = stats.map_or(gpu, |stats| stats.total + gpu);
        let interval = self.last.map_or(Duration::ZERO, |last| at - last);

        if self.first.is_none() {
            self.first = Some(at);
        }
        self.last = Some(at);
        self.frames += 1;
        self.rasterised += pixels;
        if work > self.budget {
            self.over_budget += 1;
        }

        // Consumed here rather than only read: the flag describes the interval
        // that just ended, and leaving it standing would mark every later frame
        // as slept-through no matter how hard the loop was working.
        let continuous = !std::mem::replace(&mut self.slept, false);

        // `>=` rather than `==`, and it is not defensive. Damage collapses to
        // the whole surface above `REPAINT_ALL_THRESHOLD`, which lands exactly
        // on `surface_pixels`; but regions that overlap rasterise the shared
        // pixels once each and land *above* it. Both cost at least a full
        // repaint, which is the claim being counted, so both belong here.
        //
        // Guarded on a known surface: `FrameLog` is constructed with zero
        // pixels and learns the real figure at the first resize, and every
        // frame comparing against zero would report a run of full repaints
        // before the window had a size.
        if self.surface_pixels > 0 && pixels >= self.surface_pixels {
            self.full_repaint_frames += 1;
            if !continuous {
                self.idle_full_repaint_frames += 1;
            }
        }

        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(Frame {
            work,
            interval,
            continuous,
            pixels,
        });
    }

    /// Frames presented.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// What each recent frame cost, oldest first.
    ///
    /// Feeds `vieww_widget::PerformanceOverlay`, which cannot be handed a
    /// [`FrameLog`] directly: it lives in `vieww-widget`, several crates below
    /// the one that owns the window, and this crate depends on it only for
    /// tests. Hence a plain `Vec<Duration>` as the interchange.
    ///
    /// `work` rather than `interval`, because `work` is the number a budget is
    /// a budget *for*. An interval is partly vsync waiting, so a perfectly
    /// healthy app pegged at 60Hz has 16.7ms intervals no matter how much room
    /// it had to spare, and a graph of that is a flat line that says nothing.
    #[must_use]
    pub fn work_samples(&self) -> Vec<Duration> {
        self.recent.iter().map(|frame| frame.work).collect()
    }

    /// The per-frame budget these samples are measured against.
    #[must_use]
    pub const fn budget(&self) -> Duration {
        self.budget
    }

    /// What the log says so far.
    #[must_use]
    pub fn report(&self) -> FrameReport {
        // Two exclusions, for two different reasons. The first frame has no
        // interval, and including its zero would drag every average towards a
        // frame rate nobody achieved. An interval the loop *slept* through is a
        // measurement of the user rather than of the application — see
        // [`Frame::continuous`], and the 216-second session that reported a
        // worst frame of 1979.50ms because nobody was touching the phone.
        let mut intervals: Vec<Duration> = self
            .recent
            .iter()
            .filter(|frame| frame.continuous && !frame.interval.is_zero())
            .map(|frame| frame.interval)
            .collect();
        intervals.sort_unstable();

        // Deliberately *not* filtered by `continuous`, unlike the intervals
        // above, and the asymmetry is the point rather than an oversight.
        //
        // An interval is a property of the *gap*, so a gap the loop slept through
        // measures the user. Work is a property of the *frame*, and a frame the
        // user woke up still cost what it cost. Filtering here would report an
        // event-driven application — a settings screen, a form — as having no
        // measured cost at all, which is strictly worse than the mix-dependence
        // it would remove. See
        // `a_session_that_slept_between_every_frame_says_so_rather_than_zero`.
        let mut work: Vec<Duration> = self.recent.iter().map(|frame| frame.work).collect();
        work.sort_unstable();

        let elapsed = match (self.first, self.last) {
            (Some(first), Some(last)) => last - first,
            _ => Duration::ZERO,
        };
        // `frames - 1` intervals span `frames` frames, and using the frame count
        // would report a rate one frame too high on a short run.
        let fps = if elapsed.is_zero() {
            0.0
        } else {
            (self.frames - 1) as f64 / elapsed.as_secs_f64()
        };

        FrameReport {
            frames: self.frames,
            elapsed,
            fps,
            over_budget: self.over_budget,
            budget: self.budget,
            median_interval: percentile(&intervals, 0.50),
            worst_interval: intervals.last().copied().unwrap_or_default(),
            continuous_intervals: intervals.len(),
            median_work: percentile(&work, 0.50),
            worst_work: work.last().copied().unwrap_or_default(),
            rasterised: self.rasterised,
            surface_pixels: self.surface_pixels,
            full_repaint_frames: self.full_repaint_frames,
            idle_full_repaint_frames: self.idle_full_repaint_frames,
        }
    }
}

/// The `p`th percentile of a sorted slice, or zero if it is empty.
///
/// `pub(crate)`, not private: `latency.rs`'s `InputLatencyLog::report`
/// reuses this exact function for its own p50/p95/p99 rather than
/// reimplementing percentile math a second time in this crate.
pub(crate) fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to a valid index below"
    )]
    let index = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// What a run of frames came to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameReport {
    /// Frames presented.
    pub frames: u64,
    /// Wall clock from the first presented frame to the last.
    pub elapsed: Duration,
    /// Presented frames per second, over `elapsed`.
    pub fps: f64,
    /// Frames whose CPU work exceeded the budget.
    pub over_budget: u64,
    /// The budget they were measured against.
    pub budget: Duration,
    /// The median gap between presented frames — the number that decides
    /// whether motion looks smooth.
    ///
    /// Over **continuous** intervals only; see
    /// [`worst_interval`](Self::worst_interval).
    pub median_interval: Duration,
    /// The worst gap the application was trying to fill. One of these is a
    /// visible stutter.
    ///
    /// Idle time is not in here. An interval the event loop slept through
    /// measures how long the user sat still, and counting it made an untouched
    /// 216-second session report a worst frame of 1979.50ms — a number that
    /// says the app is broken when what happened is that it was doing nothing,
    /// correctly and at no cost. [`Frame::continuous`] is the distinction, and
    /// the loop is what draws it.
    ///
    /// Zero when [`continuous_intervals`](Self::continuous_intervals) is zero.
    /// A purely event-driven session — a form nobody is scrolling — genuinely
    /// has nothing to say here, and saying nothing is the honest answer.
    ///
    /// # Soundness audit (2026-08-20)
    ///
    /// This field is a plain `Duration`, computed in `FrameLog::report` as
    /// `intervals.last().copied().unwrap_or_default()`. There is no interior
    /// mutability (`Cell`, `RefCell`, `UnsafeCell`) involved. The sorted
    /// `intervals` vec is built locally within `report(&self)` and dropped at
    /// the end of the call. This is sound under stacked borrows and safe for
    /// `#[deny(unsafe_code)]` adoption.
    pub worst_interval: Duration,
    /// How many intervals the two numbers above were computed over.
    ///
    /// Published because they are meaningless without it: "worst 18ms" over
    /// three samples of a two-hundred-frame session is not the same claim as
    /// "worst 18ms" over two hundred, and a reader cannot tell which they are
    /// holding.
    pub continuous_intervals: usize,
    /// Median CPU cost of a frame.
    ///
    /// Over **every** recorded frame, unlike the intervals above — a frame the
    /// user woke up still cost what it cost. Note that this mixes two
    /// populations: frames the scheduler ran a full pipeline for, and cheap
    /// present-only frames where it declined. The median therefore moves with
    /// that mix as well as with the code, which is worth remembering before
    /// reading a change in it as a regression.
    pub median_work: Duration,
    /// Worst CPU cost of a frame.
    pub worst_work: Duration,
    /// Pixels rasterised across the whole run.
    pub rasterised: u64,
    /// Pixels a single full repaint would cost.
    pub surface_pixels: u64,
    /// Frames that cost at least a whole surface to rasterise.
    ///
    /// Over the whole run, unlike the percentiles above, which are windowed.
    ///
    /// The companion to [`full_repaints`](Self::full_repaints), and the reason
    /// that number alone was not enough to act on: a ratio of 1.0 is equally
    /// consistent with *every* frame repainting in full and with a third of them
    /// repainting three times over. Those are different bugs, and one of them is
    /// not a bug at all.
    pub full_repaint_frames: u64,
    /// Of those, the ones the event loop had slept before — the suspicious ones.
    ///
    /// A scrolling frame repaints everything because the viewport moved
    /// everything; an idle one has no such excuse.
    pub idle_full_repaint_frames: u64,
}

impl FrameReport {
    /// Rasterised pixels as a multiple of full repaints.
    ///
    /// The Phase 4 claim, restated on a real screen: a frame that changes a
    /// corner should cost a fraction of one of these, not one each.
    #[must_use]
    pub fn full_repaints(&self) -> f64 {
        if self.surface_pixels == 0 {
            return 0.0;
        }
        self.rasterised as f64 / self.surface_pixels as f64
    }

    /// `true` if every frame stayed inside the budget.
    #[must_use]
    pub const fn is_smooth(&self) -> bool {
        self.over_budget == 0
    }

    /// The share of frames that cost at least a whole surface, 0.0 to 1.0.
    ///
    /// Zero for a run with no frames, which is the absence of a measurement
    /// rather than a measurement of zero — check [`frames`](Self::frames)
    /// before reading it as good news.
    #[must_use]
    pub fn full_repaint_share(&self) -> f64 {
        if self.frames == 0 {
            return 0.0;
        }
        self.full_repaint_frames as f64 / self.frames as f64
    }
}

impl fmt::Display for FrameReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Said in words rather than printed as `0.00ms`, because a zero here is
        // not a measurement of zero — it is the absence of one, and the two read
        // identically in a number. An app that slept between every frame is the
        // normal case for a form or a settings screen, not a fault.
        let gaps = if self.continuous_intervals == 0 {
            "no continuous run to measure".to_string()
        } else {
            format!(
                "median {:.2}ms, worst {:.2}ms over {} continuous",
                self.median_interval.as_secs_f64() * 1000.0,
                self.worst_interval.as_secs_f64() * 1000.0,
                self.continuous_intervals,
            )
        };

        write!(
            f,
            "{} frames in {:.2}s — {:.1}fps ({gaps}); \
             work median {:.2}ms, worst {:.2}ms; {} over a {:.2}ms budget; \
             rasterised {:.2} full repaints \
             ({} frame(s) cost a whole surface, {} of them idle)",
            self.frames,
            self.elapsed.as_secs_f64(),
            self.fps,
            self.median_work.as_secs_f64() * 1000.0,
            self.worst_work.as_secs_f64() * 1000.0,
            self.over_budget,
            self.budget.as_secs_f64() * 1000.0,
            self.full_repaints(),
            self.full_repaint_frames,
            self.idle_full_repaint_frames,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// The surface `log_of` and the full-repaint tests measure against.
    const SURFACE: u64 = 200 * 200;

    /// Frames presented every `interval`, each costing `work` on the CPU.
    fn log_of(count: u64, interval: Duration, work: Duration, pixels: u64) -> FrameLog {
        let mut log = FrameLog::new(ms(16), SURFACE);
        let start = Instant::now();
        for frame in 0..count {
            log.record(
                None,
                work,
                pixels,
                start + interval * u32::try_from(frame).unwrap(),
            );
        }
        log
    }

    #[test]
    fn an_empty_log_reports_nothing_rather_than_dividing_by_zero() {
        let report = FrameLog::new(ms(16), 0).report();
        assert_eq!(report.frames, 0);
        assert_eq!(report.fps, 0.0);
        assert_eq!(report.full_repaints(), 0.0);
    }

    #[test]
    fn sixty_frames_at_sixteen_milliseconds_is_sixty_fps() {
        let report = log_of(61, Duration::from_nanos(16_666_667), ms(2), 0).report();
        assert!(
            (report.fps - 60.0).abs() < 0.1,
            "expected ~60fps, got {:.3}",
            report.fps
        );
        assert!(report.is_smooth());
    }

    #[test]
    fn the_frame_rate_counts_intervals_not_frames() {
        // Two frames 100ms apart is one interval, so 10fps — not 20.
        let report = log_of(2, ms(100), ms(1), 0).report();
        assert!(
            (report.fps - 10.0).abs() < 0.01,
            "expected 10fps, got {:.3}",
            report.fps
        );
    }

    #[test]
    fn a_frame_over_budget_is_counted_and_the_run_is_not_smooth() {
        let mut log = FrameLog::new(ms(16), 100);
        let start = Instant::now();
        log.record(None, ms(2), 0, start);
        log.record(None, ms(40), 0, start + ms(40));
        log.record(None, ms(2), 0, start + ms(56));

        let report = log.report();
        assert_eq!(report.over_budget, 1);
        assert!(!report.is_smooth());
        assert_eq!(report.worst_work, ms(40));
        assert_eq!(
            report.median_work,
            ms(2),
            "one bad frame must not drag the median it is supposed to be robust to"
        );
    }

    #[test]
    fn the_pipelines_own_time_is_added_to_the_gpus() {
        let mut log = FrameLog::new(ms(16), 100);
        let stats = FrameStats {
            number: 1,
            timestamp: Duration::ZERO,
            animate: ms(1),
            build: ms(1),
            layout: ms(1),
            paint: ms(1),
            composite: ms(1),
            total: ms(5),
            budget: ms(16),
            // This log records timings; the damage figures are irrelevant to
            // what it asserts and zero is the honest value for a hand-made
            // `FrameStats` that no frame produced.
            damage_area: 0.0,
            damage_regions: 0,
        };
        log.record(Some(stats), ms(4), 0, Instant::now());

        assert_eq!(
            log.report().worst_work,
            ms(9),
            "a frame rate that ignored rasterisation would be a report on the CPU alone"
        );
    }

    #[test]
    fn damage_is_reported_against_what_a_full_repaint_would_have_cost() {
        // 100 frames each rasterising a tenth of the window.
        let report = log_of(100, ms(16), ms(1), 4_000).report();
        assert!(
            (report.full_repaints() - 10.0).abs() < 0.01,
            "100 frames at a tenth each is 10 full repaints, got {}",
            report.full_repaints()
        );
    }

    #[test]
    fn a_frame_that_rasterises_the_whole_surface_is_a_full_repaint() {
        let report = log_of(3, ms(16), ms(1), SURFACE).report();
        assert_eq!(report.full_repaint_frames, 3);
        assert!((report.full_repaint_share() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_frame_that_rasterises_a_corner_is_not() {
        let report = log_of(10, ms(16), ms(1), SURFACE / 10).report();
        assert_eq!(report.full_repaint_frames, 0);
        assert_eq!(report.full_repaint_share(), 0.0);
    }

    #[test]
    fn overlapping_regions_cost_more_than_a_surface_and_still_count_once() {
        // Damage collapses at 65% of the surface, so a frame landing *above*
        // 100% did not collapse — it rasterised shared pixels twice. That is
        // one frame that cost more than a full repaint, not two frames.
        let report = log_of(1, ms(16), ms(1), SURFACE * 3).report();
        assert_eq!(report.full_repaint_frames, 1);
        assert!(
            report.full_repaints() > 1.0,
            "the pixel ratio is what catches the overlap: {}",
            report.full_repaints()
        );
    }

    #[test]
    fn a_window_with_no_size_yet_is_not_a_run_of_full_repaints() {
        // `App` builds its log before the surface is configured, so
        // `surface_pixels` is zero until the first resize. Every frame is `>=`
        // zero, and counting them would report a startup that never happened.
        let mut log = FrameLog::new(ms(16), 0);
        log.record(None, ms(1), 0, Instant::now());

        assert_eq!(log.report().full_repaint_frames, 0);
    }

    #[test]
    fn an_idle_full_repaint_is_told_apart_from_a_scrolling_one() {
        // The distinction the whole counter exists for. Both frames repaint
        // everything; only the second one is a defect, because nothing was
        // moving and one wake-up rasterised the entire screen.
        let mut log = FrameLog::new(ms(16), SURFACE);
        let start = Instant::now();

        log.record(None, ms(2), SURFACE, start);
        log.about_to_sleep();
        log.record(None, ms(2), SURFACE, start + ms(500));

        let report = log.report();
        assert_eq!(report.full_repaint_frames, 2);
        assert_eq!(
            report.idle_full_repaint_frames, 1,
            "a frame the loop slept before had nothing moving to justify it"
        );
    }

    #[test]
    fn the_frame_count_separates_two_runs_the_pixel_ratio_cannot() {
        // Why `full_repaints()` alone could not be acted on. Both runs
        // rasterise 30 surfaces over 30 frames, so both report a ratio of
        // exactly 30.0 — and they are different bugs. The first is every frame
        // repainting in full; the second is a third of them repainting three
        // times over while the rest cost nothing.
        let every_frame = log_of(30, ms(16), ms(1), SURFACE).report();

        let mut log = FrameLog::new(ms(16), SURFACE);
        let start = Instant::now();
        for frame in 0..30u32 {
            let pixels = if frame % 3 == 0 { SURFACE * 3 } else { 0 };
            log.record(None, ms(1), pixels, start + ms(16) * frame);
        }
        let a_third_of_them = log.report();

        assert!(
            (every_frame.full_repaints() - a_third_of_them.full_repaints()).abs() < 0.01,
            "the premise: {} vs {}",
            every_frame.full_repaints(),
            a_third_of_them.full_repaints()
        );
        assert_eq!(every_frame.full_repaint_frames, 30);
        assert_eq!(a_third_of_them.full_repaint_frames, 10);
    }

    #[test]
    fn only_the_most_recent_frames_decide_the_percentiles() {
        let mut log = FrameLog::new(ms(16), 100);
        let start = Instant::now();
        // One catastrophic frame, then a long smooth run that pushes it out.
        log.record(None, ms(500), 0, start);
        for frame in 1..=u32::try_from(WINDOW).unwrap() {
            log.record(None, ms(2), 0, start + ms(16) * frame);
        }

        let report = log.report();
        assert_eq!(report.worst_work, ms(2), "the 500ms frame has aged out");
        assert_eq!(
            report.over_budget, 1,
            "but the running count still remembers it happened"
        );
    }

    #[test]
    fn a_new_budget_rejudges_nothing_that_already_happened() {
        // A work duration is a measurement of this process and does not become
        // wrong because the panel turned out to be faster than assumed. What
        // changes is only what counts as late from here on — and the running
        // tally stays, because two reports of the same run disagreeing about
        // how many frames were janky is worse than either answer.
        let mut log = FrameLog::new(ms(16), 1000);
        let start = Instant::now();
        // Twelve milliseconds: inside a 60Hz budget, outside a 120Hz one.
        log.record(None, ms(12), 0, start);
        let before = log.report();
        assert_eq!(before.over_budget, 0);

        log.set_budget(Duration::from_micros(8_333));
        let after = log.report();
        assert_eq!(
            after.over_budget, 0,
            "a frame already judged is not judged again"
        );
        assert_eq!(
            after.budget,
            Duration::from_micros(8_333),
            "but the budget reported from here on is the new one"
        );

        // And the next frame is measured against it.
        log.record(None, ms(12), 0, start + ms(16));
        assert_eq!(log.report().over_budget, 1, "12ms is late at 120Hz");
    }

    #[test]
    fn time_the_loop_slept_through_is_not_the_worst_frame() {
        // **The bug this pair of methods exists for.** A window that draws, goes
        // idle for five seconds because nobody touches it, and then draws again
        // has not stuttered. It has done exactly what an event-driven framework
        // is supposed to do, and the old report called it the worst frame of the
        // session — measured at 1979.50ms over an untouched 216 seconds, and at
        // 48302.83ms in the run that verified the layout-write fix.
        let mut log = FrameLog::new(ms(16), 1000);
        let start = Instant::now();

        log.record(None, ms(2), 0, start);
        log.record(None, ms(2), 0, start + ms(16));

        // Nobody is doing anything, so the loop sleeps. Several times over —
        // `about_to_wait` runs for every event the window ignores.
        log.about_to_sleep();
        log.about_to_sleep();
        log.record(None, ms(2), 0, start + ms(5016));

        // And then a frame that really was late, with the loop awake for it.
        log.record(None, ms(2), 0, start + ms(5116));

        let report = log.report();
        assert_eq!(
            report.worst_interval,
            ms(100),
            "the 100ms frame is the worst one the application owed; the five \
             second gap is the user, and reporting it as a stall is what made \
             this number useless on any app that is ever idle"
        );
        assert_eq!(
            report.continuous_intervals, 2,
            "two of the three intervals were the application's to fill"
        );
        assert_eq!(
            report.frames, 4,
            "and nothing is dropped from the frame count — the frame happened, \
             it is only the *gap before it* that measures the user"
        );
    }

    #[test]
    fn one_sleep_excuses_one_interval_and_not_the_next() {
        // The flag is consumed by the frame it describes. Leaving it standing
        // would be the same bug with the sign flipped: every frame after the
        // first idle moment excused for ever, so a genuine stall in a session
        // that had once been idle would never be reported again.
        let mut log = FrameLog::new(ms(16), 1000);
        let start = Instant::now();

        log.record(None, ms(2), 0, start);
        log.about_to_sleep();
        log.record(None, ms(2), 0, start + ms(2000));
        // No sleep this time: the loop was awake and took 250ms to produce it.
        log.record(None, ms(2), 0, start + ms(2250));

        let report = log.report();
        assert_eq!(report.continuous_intervals, 1);
        assert_eq!(
            report.worst_interval,
            ms(250),
            "the second gap is a stall and is reported as one"
        );
    }

    #[test]
    fn a_session_that_slept_between_every_frame_says_so_rather_than_zero() {
        // A settings screen nobody is scrolling. Every frame is a response to a
        // tap, every interval is the user thinking, and there is genuinely
        // nothing here to say about smoothness.
        //
        // The distinction that matters is between *zero* and *no measurement*.
        // Printing `worst 0.00ms` would read as a perfect score, which is the
        // most flattering possible way to be wrong.
        let mut log = FrameLog::new(ms(16), 1000);
        let start = Instant::now();

        for frame in 0..5_u32 {
            log.about_to_sleep();
            log.record(None, ms(2), 0, start + ms(400) * frame);
        }

        let report = log.report();
        assert_eq!(report.continuous_intervals, 0);
        assert_eq!(report.worst_interval, Duration::ZERO);
        assert!(
            format!("{report}").contains("no continuous run to measure"),
            "and it says so in words: {report}"
        );
        assert_eq!(
            report.frames, 5,
            "the frames still happened, and their work is still measured"
        );
        assert!(report.median_work > Duration::ZERO);
    }
}
