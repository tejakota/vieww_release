//! What a tick produced.

use std::time::Duration;

/// Summary of what happened during one [`tick`](super::TestHarness::tick)
/// or [`settle`](super::TestHarness::settle) call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameReport {
    /// Frames drawn during this tick.
    pub frames_drawn: u64,
    /// Loop iterations (drawn + sleep-checks).
    pub iterations: u64,
    /// The synthetic clock after this tick.
    pub now: Duration,
}

impl FrameReport {
    /// `true` if at least one frame was drawn.
    #[must_use]
    pub const fn drew(&self) -> bool {
        self.frames_drawn > 0
    }

    /// No frames drawn, no iterations beyond the sleep check.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.frames_drawn == 0 && self.iterations <= 1
    }
}

impl std::fmt::Display for FrameReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FrameReport: {} frames, {} iterations, now={:?}",
            self.frames_drawn, self.iterations, self.now
        )
    }
}
