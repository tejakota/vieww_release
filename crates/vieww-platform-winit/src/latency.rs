//! Input-to-photon latency — "Renderer v2" pillar C
//! (`docs/RENDERER-V2-NOTES.md`): "Input-to-photon latency as a first-class
//! metric (pointer-down → first visible pixel, pointer-move → visible
//! response, keyboard → glyph appearance, scroll → visual movement,
//! animation trigger → first frame), measured at median/p95/p99 — not just
//! frames per second."
//!
//! # What this measures, and how it is not `stats.rs`'s frame rate
//!
//! `FrameLog` (`stats.rs`) measures the gap *between presented frames* —
//! "how smooth does motion already in flight look." This module measures a
//! different interval entirely: the gap from *an input event arriving* to
//! *the frame that shows its effect being presented* — "how long did the
//! user wait to see what they just did." A perfectly smooth 60Hz session
//! (`FrameLog` reporting a flawless 16.7ms median interval) can still feel
//! laggy if every tap takes three frames to show up, which is exactly the
//! gap `stats.rs`'s own module docs name as the reason it measures interval
//! rather than input response: "vello rasterises, the queue is submitted,
//! the swapchain image is acquired" happens after the paint layer's
//! responsibility ends, and *when the input arrived relative to all of
//! that* is a third measurement neither `FrameStats` nor `FrameLog` takes.
//!
//! # Coalescing: many inputs, one frame
//!
//! An event-driven loop routinely batches several input events (two
//! pointer-move deltas, a key repeat) into the one frame that ends up
//! showing all of their combined effect — there is no "frame that resulted
//! from *this specific* input" to point to once several have queued up
//! before the next present. So [`InputLatencyLog::frame_presented`] treats
//! every input that arrived since the previous presented frame as
//! *incorporated by* this one, and measures each of their individual
//! latencies against this frame's presentation time — an input that
//! arrived early in the queue is correctly measured as having waited
//! longer than one that arrived just before present, even though both are
//! "closed" by the same frame.
//!
//! # Percentiles, reused rather than reimplemented
//!
//! [`InputLatencyLog::report`] computes p50/p95/p99 with `stats.rs`'s own
//! `percentile` function — the exact function `FrameReport`'s
//! `median_interval`/`median_work` already use — so this module's
//! percentiles are computed the same way the rest of this crate's are, not
//! a second implementation that could disagree about what "p95" means at
//! the boundary.
//!
//! # The mock clock the task asks for
//!
//! Nothing in this module ever calls `Instant::now()` — every timestamp
//! ([`InputLatencyLog::input_arrived`]'s `at`,
//! [`InputLatencyLog::frame_presented`]'s `presented_at`) is supplied by the
//! caller, exactly like `FrameLog::record`'s own `at: Instant` parameter
//! already is. That is what makes this module's own tests deterministic
//! without needing an actual mockable clock trait: a test picks one
//! `Instant` as a base and computes every other timestamp by adding a
//! `Duration` to it, so "one input arrives, 16ms later a frame presents"
//! is a fixed, repeatable sequence of values rather than a race against
//! real wall time. `tests::MockClock` below is that pattern wrapped in a
//! tiny helper so the tests read as "advance the clock" rather than
//! `Duration` arithmetic at every call site.
//!
//! # Integration status
//!
//! This module is real and fully tested standalone but is **not wired into
//! `app.rs`'s winit event loop** — that would mean finding and touching
//! every pointer/keyboard/scroll dispatch site across a 2,700-line event
//! loop to call [`InputLatencyLog::input_arrived`], and the one place
//! `FrameLog::record` is already called (`app.rs`, where a frame's present
//! completes) to call [`InputLatencyLog::frame_presented`] alongside it.
//! Both call sites are straightforward in principle — `frame_presented`
//! belongs immediately next to the existing `self.log.record(stats, gpu,
//! pixels, Instant::now())` call, using the same `Instant::now()` — but
//! wiring every input dispatch site correctly, without missing one and
//! silently under-counting, is real further work this delivery does not
//! claim to have done. What is done is the measurement mechanism itself:
//! proven correct, ready for an `App` to hold one and call into from both
//! sides.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::stats::percentile;

/// How many recent input-to-photon latencies the percentiles are computed
/// over — the same value, and the same reasoning, as `stats.rs`'s own
/// `WINDOW`.
const WINDOW: usize = 240;

/// A rolling record of input-to-photon latencies.
#[derive(Debug)]
pub struct InputLatencyLog {
    /// Inputs that have arrived but not yet been incorporated into a
    /// presented frame.
    pending: Vec<Instant>,
    recent: VecDeque<Duration>,
    total_events: u64,
    frames_with_input: u64,
}

impl Default for InputLatencyLog {
    fn default() -> Self {
        Self::new()
    }
}

impl InputLatencyLog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            recent: VecDeque::with_capacity(WINDOW),
            total_events: 0,
            frames_with_input: 0,
        }
    }

    /// Record that an input event — a pointer-down, a key press, a scroll
    /// delta, whatever the caller considers worth measuring the response to
    /// — arrived at `at`.
    pub fn input_arrived(&mut self, at: Instant) {
        self.pending.push(at);
        self.total_events += 1;
    }

    /// Record that a frame was presented at `presented_at`, incorporating
    /// every input that arrived since the previous presented frame.
    ///
    /// A frame with nothing pending closes nothing and is not counted —
    /// see the module docs' "Coalescing" section for why every pending
    /// input is measured against this one presentation time rather than
    /// only the earliest or the latest.
    pub fn frame_presented(&mut self, presented_at: Instant) {
        if self.pending.is_empty() {
            return;
        }
        self.frames_with_input += 1;
        for arrived in self.pending.drain(..) {
            let latency = presented_at.saturating_duration_since(arrived);
            if self.recent.len() == WINDOW {
                self.recent.pop_front();
            }
            self.recent.push_back(latency);
        }
    }

    /// Inputs that have arrived but whose frame has not presented yet.
    #[must_use]
    pub fn pending_events(&self) -> usize {
        self.pending.len()
    }

    /// What this log says so far.
    #[must_use]
    pub fn report(&self) -> InputLatencyReport {
        let mut sorted: Vec<Duration> = self.recent.iter().copied().collect();
        sorted.sort_unstable();
        InputLatencyReport {
            samples: sorted.len(),
            total_events: self.total_events,
            frames_with_input: self.frames_with_input,
            p50: percentile(&sorted, 0.50),
            p95: percentile(&sorted, 0.95),
            p99: percentile(&sorted, 0.99),
            worst: sorted.last().copied().unwrap_or_default(),
        }
    }
}

/// What a run of input-to-photon measurements came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputLatencyReport {
    /// Latencies in the current rolling window.
    pub samples: usize,
    /// Input events recorded over this log's whole lifetime, not just the
    /// current window.
    pub total_events: u64,
    /// Presented frames that incorporated at least one pending input, over
    /// this log's whole lifetime.
    pub frames_with_input: u64,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
    /// The worst (maximum) latency in the current rolling window.
    pub worst: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock that only ever moves forward by however much a test tells
    /// it to — see the module docs' "The mock clock the task asks for".
    struct MockClock {
        now: Instant,
    }

    impl MockClock {
        fn new() -> Self {
            Self {
                now: Instant::now(),
            }
        }

        fn now(&self) -> Instant {
            self.now
        }

        fn advance(&mut self, by: Duration) -> Instant {
            self.now += by;
            self.now
        }
    }

    #[test]
    fn a_report_with_no_frames_yet_is_all_zero() {
        let log = InputLatencyLog::new();
        let report = log.report();
        assert_eq!(report.samples, 0);
        assert_eq!(report.p50, Duration::ZERO);
        assert_eq!(report.p99, Duration::ZERO);
    }

    #[test]
    fn an_input_still_pending_does_not_appear_in_the_report_until_its_frame_presents() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();
        log.input_arrived(clock.now());
        assert_eq!(log.pending_events(), 1);
        assert_eq!(
            log.report().samples,
            0,
            "an unclosed input contributes no sample yet"
        );

        let presented = clock.advance(Duration::from_millis(16));
        log.frame_presented(presented);
        assert_eq!(log.pending_events(), 0);
        assert_eq!(log.report().samples, 1);
    }

    #[test]
    fn a_single_input_closed_by_the_next_frame_measures_exactly_that_gap() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();
        log.input_arrived(clock.now());
        let presented = clock.advance(Duration::from_millis(23));
        log.frame_presented(presented);

        let report = log.report();
        assert_eq!(report.samples, 1);
        assert_eq!(report.p50, Duration::from_millis(23));
        assert_eq!(report.worst, Duration::from_millis(23));
        assert_eq!(report.frames_with_input, 1);
    }

    #[test]
    fn several_inputs_coalesced_into_one_frame_each_measure_against_its_presentation() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();

        log.input_arrived(clock.now()); // t = 0
        log.input_arrived(clock.advance(Duration::from_millis(2))); // t = 2
        log.input_arrived(clock.advance(Duration::from_millis(3))); // t = 5

        let presented = clock.advance(Duration::from_millis(15)); // t = 20
        log.frame_presented(presented);

        let mut latencies: Vec<Duration> = log.recent.iter().copied().collect();
        latencies.sort_unstable();
        assert_eq!(
            latencies,
            vec![Duration::from_millis(15), Duration::from_millis(18), Duration::from_millis(20)],
            "the earliest input must show the longest wait, the latest the shortest, all against the same presentation"
        );
        assert_eq!(
            log.report().frames_with_input,
            1,
            "one frame closed three inputs, not three frames"
        );
    }

    #[test]
    fn a_frame_with_nothing_pending_is_not_counted_as_closing_anything() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();
        // A frame presents with no input having arrived at all — an
        // animation tick or a frame the scheduler ran for its own reasons.
        log.frame_presented(clock.now());
        assert_eq!(log.report().samples, 0);
        assert_eq!(log.report().frames_with_input, 0);

        log.input_arrived(clock.advance(Duration::from_millis(1)));
        log.frame_presented(clock.advance(Duration::from_millis(10)));
        assert_eq!(
            log.report().frames_with_input,
            1,
            "only the frame that actually had pending input counts"
        );
    }

    #[test]
    fn percentiles_order_correctly_across_a_spread_of_latencies() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();

        // 100 inputs, each closed by its own frame one-by-one, with
        // latencies climbing from 1ms to 100ms — a known, ordered
        // distribution to check percentile *placement* against, not just
        // percentile *arithmetic* (already covered by `stats.rs`'s own
        // `percentile` tests, reused here rather than re-tested).
        for ms in 1..=100u64 {
            let arrived = clock.now();
            log.input_arrived(arrived);
            let presented = clock.advance(Duration::from_millis(ms));
            log.frame_presented(presented);
        }

        let report = log.report();
        assert_eq!(report.samples, 100);
        assert!(report.p50 <= report.p95, "p50 must never exceed p95");
        assert!(report.p95 <= report.p99, "p95 must never exceed p99");
        assert!(
            report.p99 <= report.worst,
            "p99 must never exceed the worst sample"
        );
        // The worst single latency was the 100ms gap on the final input.
        assert_eq!(report.worst, Duration::from_millis(100));
    }

    #[test]
    fn the_rolling_window_drops_the_oldest_samples() {
        let mut clock = MockClock::new();
        let mut log = InputLatencyLog::new();
        for _ in 0..(WINDOW + 50) {
            log.input_arrived(clock.now());
            log.frame_presented(clock.advance(Duration::from_millis(1)));
        }
        assert_eq!(
            log.report().samples,
            WINDOW,
            "the window must cap at WINDOW even after more frames than that"
        );
        assert_eq!(
            log.report().total_events,
            (WINDOW + 50) as u64,
            "total_events is a lifetime count, not windowed"
        );
    }
}
