//! Per-frame time budgeting.
//!
//! `docs/RENDERER-V2-NOTES.md`: "a per-frame work budget... target: minimum
//! work for identical pixels, not merely zero allocations," and the
//! milestone note that the deadline itself varies with refresh rate — 60Hz,
//! 90Hz, 120Hz, 144Hz, 240Hz are all real targets, not just 16.67ms.

/// Tracks how much of a frame's deadline is left as passes are placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameBudget {
    pub deadline_ms: f32,
    pub spent_ms: f32,
}

impl FrameBudget {
    #[must_use]
    pub const fn new(deadline_ms: f32) -> Self {
        Self {
            deadline_ms,
            spent_ms: 0.0,
        }
    }

    #[must_use]
    pub fn remaining_ms(&self) -> f32 {
        (self.deadline_ms - self.spent_ms).max(0.0)
    }

    /// Fraction of the deadline already spent, `0.0`..=`1.0` (saturating —
    /// a frame that overran does not report a fraction above 1).
    #[must_use]
    pub fn spent_fraction(&self) -> f32 {
        (self.spent_ms / self.deadline_ms.max(0.001)).clamp(0.0, 1.0)
    }

    /// `true` once less than `headroom_ms` of budget remains — the signal
    /// [`crate::policy`] uses to start biasing toward whichever executor is
    /// faster in absolute terms, even at higher power cost, rather than the
    /// power-optimal choice.
    #[must_use]
    pub fn is_tight(&self, headroom_ms: f32) -> bool {
        self.remaining_ms() < headroom_ms
    }

    pub fn spend(&mut self, ms: f32) {
        self.spent_ms += ms.max(0.0);
    }

    /// `true` if the frame missed its deadline — a real "jank" event
    /// (`docs/RENDERER-V2-NOTES.md`'s "jank/frame drop count" metric).
    #[must_use]
    pub fn missed_deadline(&self) -> bool {
        self.spent_ms > self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spending_reduces_remaining_budget() {
        let mut b = FrameBudget::new(16.0);
        b.spend(10.0);
        assert!((b.remaining_ms() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn overspending_is_reported_as_a_missed_deadline() {
        let mut b = FrameBudget::new(16.0);
        b.spend(20.0);
        assert!(b.missed_deadline());
        assert_eq!(b.remaining_ms(), 0.0);
    }

    #[test]
    fn a_120hz_deadline_is_tighter_than_60hz() {
        assert!(FrameBudget::new(8.33).deadline_ms < FrameBudget::new(16.67).deadline_ms);
    }
}
