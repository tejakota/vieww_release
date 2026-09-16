//! A rolling history of real per-frame timings, for spotting jank.
//!
//! # Why this wraps `vieww_paint::FrameStats` instead of measuring anything
//!
//! `vieww-paint`'s [`FrameScheduler`](vieww_paint::FrameScheduler) already
//! times every phase of every frame — animate, build, layout, paint,
//! composite — against a real clock supplied by the platform's vsync signal
//! (see that module's "why time is a parameter"), and
//! [`FrameDriver::drive`](vieww_render::FrameDriver::drive) is what actually
//! calls it and hands back a [`vieww_paint::FrameStats`] each frame. A second
//! timing mechanism here would either duplicate that clock-reading code badly
//! (a devtools crate has no business owning a stopwatch) or drift out of sync
//! with what the scheduler itself already decided a phase boundary is. So
//! [`FrameTimeline`] does neither: it is purely a *history* — a caller feeds
//! it the `FrameStats` the driver already produced, one per frame, and this
//! type answers the questions a jank hunt actually asks: what did the last N
//! frames cost on average, what is the worst case, and how are they
//! distributed.
//!
//! # Why a ring buffer, not an ever-growing `Vec`
//!
//! A performance overlay runs for the life of the application, sometimes
//! hours. Recording every frame's stats forever would be a memory leak with
//! extra steps — nobody needs frame 40,000 in a session displaying the last
//! two seconds of history. A fixed-capacity ring buffer bounds the cost of
//! "keep recording" to a constant, however long the app has been running.

use std::time::Duration;

use vieww_paint::FrameStats;

/// A bounded history of [`FrameStats`], newest overwriting oldest once full.
#[derive(Debug, Clone)]
pub struct FrameTimeline {
    /// Fixed-capacity backing storage. `Vec` rather than a fixed-size array
    /// because the capacity is chosen at construction time (a devtools panel
    /// showing one second of history at 60Hz needs a different capacity than
    /// one at 120Hz), not known at compile time.
    samples: Vec<FrameStats>,
    capacity: usize,
    /// Where the next `record` writes. Wraps at `capacity`.
    next: usize,
    /// Total samples ever recorded, including ones since overwritten — the
    /// only way to tell "buffer not yet full" from "buffer full and wrapped"
    /// when both look identical to `samples.len() == capacity`.
    total_recorded: u64,
}

impl FrameTimeline {
    /// A new, empty timeline holding at most `capacity` frames.
    ///
    /// `capacity` is clamped to at least 1: a timeline that could hold zero
    /// frames could not answer any of its own query methods honestly, and
    /// every one of them would need a special case for "no capacity" that is
    /// indistinguishable from "no data yet" to a caller.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            samples: Vec::with_capacity(capacity),
            capacity,
            next: 0,
            total_recorded: 0,
        }
    }

    /// Record one frame's stats, evicting the oldest sample if the timeline
    /// is already at capacity.
    pub fn record(&mut self, sample: FrameStats) {
        if self.samples.len() < self.capacity {
            self.samples.push(sample);
        } else {
            self.samples[self.next] = sample;
        }
        self.next = (self.next + 1) % self.capacity;
        self.total_recorded += 1;
    }

    /// How many frames are currently held (at most `capacity`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// The maximum number of frames this timeline holds at once.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// How many frames have ever been recorded, including ones since
    /// overwritten by wraparound.
    #[must_use]
    pub fn total_recorded(&self) -> u64 {
        self.total_recorded
    }

    /// Every held sample, oldest first.
    ///
    /// Reconstructs chronological order from the ring's internal layout: once
    /// the buffer has wrapped, `next` points at the oldest surviving sample
    /// (the one about to be overwritten), so the oldest-first order is the
    /// slice starting there, wrapped around.
    #[must_use]
    pub fn samples_oldest_first(&self) -> Vec<FrameStats> {
        if self.samples.len() < self.capacity {
            // Never wrapped: insertion order already is chronological order.
            return self.samples.clone();
        }
        let (before, after) = self.samples.split_at(self.next);
        after.iter().chain(before).copied().collect()
    }

    /// The most recent `n` frames, oldest first among themselves. Fewer than
    /// `n` if the timeline does not hold that many yet.
    #[must_use]
    pub fn last_n(&self, n: usize) -> Vec<FrameStats> {
        let all = self.samples_oldest_first();
        let start = all.len().saturating_sub(n);
        all[start..].to_vec()
    }

    /// Mean total frame time over the last `n` frames. `None` if there are
    /// none recorded.
    #[must_use]
    pub fn average(&self, n: usize) -> Option<Duration> {
        let window = self.last_n(n);
        if window.is_empty() {
            return None;
        }
        let sum: Duration = window.iter().map(|s| s.total).sum();
        Some(sum / window.len() as u32)
    }

    /// The maximum total frame time over the last `n` frames — the frame that
    /// hurt most. `None` if there are none recorded.
    #[must_use]
    pub fn max(&self, n: usize) -> Option<Duration> {
        self.last_n(n).into_iter().map(|s| s.total).max()
    }

    /// The 95th-percentile total frame time over the last `n` frames — the
    /// number a jank budget is usually written against, because the mean
    /// hides the rare bad frame and the max is one unlucky sample away from
    /// being noise.
    ///
    /// `None` if there are none recorded.
    ///
    /// # Which definition of "percentile"
    ///
    /// Nearest-rank on the sorted samples: index
    /// `ceil(0.95 * n) - 1`, clamped into range. This is the simplest
    /// percentile definition that needs no interpolation and therefore no
    /// argument about which interpolation scheme is right — appropriate for
    /// a frame-time budget check, where "which of these two adjacent
    /// microseconds counts as p95" is not a question anyone downstream cares
    /// about the answer to.
    #[must_use]
    pub fn p95(&self, n: usize) -> Option<Duration> {
        let mut window: Vec<Duration> = self.last_n(n).into_iter().map(|s| s.total).collect();
        if window.is_empty() {
            return None;
        }
        window.sort_unstable();
        let rank = ((window.len() as f64) * 0.95).ceil() as usize;
        let index = rank.saturating_sub(1).min(window.len() - 1);
        Some(window[index])
    }

    /// A histogram of total frame times over the last `n` frames, bucketed
    /// against `budget`.
    ///
    /// Buckets are multiples of `budget`: `[0, budget)`, `[budget, 2*budget)`,
    /// and so on, with a final overflow bucket for anything at or past
    /// `bucket_count * budget`. Bucketing against the budget rather than
    /// against fixed millisecond widths is what makes the histogram mean the
    /// same thing at 60Hz and at 120Hz: bucket 0 is always "hit the budget",
    /// bucket 1 is always "missed it by up to one frame's worth", and so on,
    /// regardless of what the budget itself is.
    ///
    /// Returns a `Vec` of length `bucket_count`, index `i` holding the count
    /// of frames whose total time fell in `[i*budget, (i+1)*budget)`, except
    /// the last index which also absorbs everything at or beyond it. A zero
    /// `budget` or zero `bucket_count` produces an all-overflow histogram
    /// (`bucket_count` clamped to at least 1) rather than dividing by zero.
    #[must_use]
    pub fn histogram(&self, n: usize, budget: Duration, bucket_count: usize) -> Vec<usize> {
        let bucket_count = bucket_count.max(1);
        let mut buckets = vec![0usize; bucket_count];
        for sample in self.last_n(n) {
            let index = if budget.is_zero() {
                bucket_count - 1
            } else {
                let ratio = sample.total.as_secs_f64() / budget.as_secs_f64();
                (ratio.floor() as usize).min(bucket_count - 1)
            };
            buckets[index] += 1;
        }
        buckets
    }

    /// How many of the last `n` frames missed their own recorded budget
    /// ([`FrameStats::over_budget`]).
    #[must_use]
    pub fn dropped_frame_count(&self, n: usize) -> usize {
        self.last_n(n).iter().filter(|s| s.over_budget()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic sample with `total` set to `millis` and every other field
    /// zeroed or defaulted — the query methods under test only look at
    /// `total` and `over_budget` (which is `total > budget`), so nothing else
    /// needs to be realistic here.
    fn sample(number: u64, millis: u64, budget_millis: u64) -> FrameStats {
        FrameStats {
            number,
            timestamp: Duration::from_millis(number * 16),
            animate: Duration::ZERO,
            build: Duration::ZERO,
            layout: Duration::ZERO,
            paint: Duration::ZERO,
            composite: Duration::ZERO,
            total: Duration::from_millis(millis),
            budget: Duration::from_millis(budget_millis),
            damage_area: 0.0,
            damage_regions: 0,
        }
    }

    #[test]
    fn an_empty_timeline_answers_every_query_with_none_or_zero() {
        let timeline = FrameTimeline::new(8);
        assert!(timeline.is_empty());
        assert_eq!(timeline.average(10), None);
        assert_eq!(timeline.max(10), None);
        assert_eq!(timeline.p95(10), None);
        assert_eq!(timeline.dropped_frame_count(10), 0);
    }

    #[test]
    fn recording_under_capacity_keeps_chronological_order() {
        let mut timeline = FrameTimeline::new(10);
        for i in 1..=5 {
            timeline.record(sample(i, i * 2, 16));
        }
        let order: Vec<u64> = timeline
            .samples_oldest_first()
            .iter()
            .map(|s| s.number)
            .collect();
        assert_eq!(order, vec![1, 2, 3, 4, 5]);
        assert_eq!(timeline.total_recorded(), 5);
    }

    #[test]
    fn wraparound_evicts_the_oldest_frame_and_keeps_order() {
        let mut timeline = FrameTimeline::new(3);
        for i in 1..=5u64 {
            timeline.record(sample(i, i, 16));
        }
        // Capacity 3, five recorded: frames 1 and 2 are gone, 3/4/5 remain.
        let order: Vec<u64> = timeline
            .samples_oldest_first()
            .iter()
            .map(|s| s.number)
            .collect();
        assert_eq!(order, vec![3, 4, 5]);
        assert_eq!(timeline.len(), 3);
        assert_eq!(
            timeline.total_recorded(),
            5,
            "history count survives eviction"
        );
    }

    #[test]
    fn average_is_the_hand_computed_mean_over_the_window() {
        let mut timeline = FrameTimeline::new(10);
        for millis in [10, 20, 30, 40] {
            timeline.record(sample(millis, millis, 16));
        }
        // (10 + 20 + 30 + 40) / 4 = 25
        assert_eq!(timeline.average(10), Some(Duration::from_millis(25)));
        // Last 2 only: (30 + 40) / 2 = 35
        assert_eq!(timeline.average(2), Some(Duration::from_millis(35)));
    }

    #[test]
    fn max_finds_the_worst_frame_in_the_window() {
        let mut timeline = FrameTimeline::new(10);
        for millis in [5, 50, 8, 12] {
            timeline.record(sample(millis, millis, 16));
        }
        assert_eq!(timeline.max(10), Some(Duration::from_millis(50)));
        // Excluding the spike (only the last 2 frames): max(8, 12) = 12.
        assert_eq!(timeline.max(2), Some(Duration::from_millis(12)));
    }

    #[test]
    fn p95_matches_a_hand_computed_nearest_rank_over_twenty_samples() {
        let mut timeline = FrameTimeline::new(20);
        // 1ms..=20ms, so the sorted list is exactly 1..=20.
        for millis in 1..=20u64 {
            timeline.record(sample(millis, millis, 16));
        }
        // rank = ceil(0.95 * 20) = 19, 1-indexed -> value at sorted index 18
        // (0-indexed) -> the 19th smallest value -> 19ms.
        assert_eq!(timeline.p95(20), Some(Duration::from_millis(19)));
    }

    #[test]
    fn p95_of_a_single_sample_is_that_sample() {
        let mut timeline = FrameTimeline::new(4);
        timeline.record(sample(1, 42, 16));
        assert_eq!(timeline.p95(10), Some(Duration::from_millis(42)));
    }

    #[test]
    fn histogram_buckets_by_multiples_of_budget() {
        let mut timeline = FrameTimeline::new(10);
        // Budget 16ms: bucket 0 = [0,16), bucket 1 = [16,32), bucket 2 = [32,48),
        // overflow (bucket_count=3, so index 2 is also the overflow bucket).
        for millis in [5, 10, 20, 25, 40, 100] {
            timeline.record(sample(millis, millis, 16));
        }
        let hist = timeline.histogram(10, Duration::from_millis(16), 3);
        // 5,10 -> bucket 0 (2 frames)
        // 20,25 -> bucket 1 (2 frames)
        // 40,100 -> bucket 2 / overflow (2 frames)
        assert_eq!(hist, vec![2, 2, 2]);
    }

    #[test]
    fn histogram_with_zero_budget_puts_everything_in_the_overflow_bucket() {
        let mut timeline = FrameTimeline::new(4);
        timeline.record(sample(1, 5, 0));
        timeline.record(sample(2, 500, 0));
        let hist = timeline.histogram(10, Duration::ZERO, 4);
        assert_eq!(hist, vec![0, 0, 0, 2]);
    }

    #[test]
    fn dropped_frame_count_matches_frames_over_their_own_budget() {
        let mut timeline = FrameTimeline::new(10);
        timeline.record(sample(1, 10, 16)); // under budget
        timeline.record(sample(2, 20, 16)); // over budget
        timeline.record(sample(3, 5, 16)); // under budget
        timeline.record(sample(4, 30, 16)); // over budget
        assert_eq!(timeline.dropped_frame_count(10), 2);
    }

    #[test]
    fn capacity_is_never_less_than_one() {
        let timeline = FrameTimeline::new(0);
        assert_eq!(timeline.capacity, 1);
    }
}
