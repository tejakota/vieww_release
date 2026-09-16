//! Enforceable quality contracts shared by platform certification and future CI.
//!
//! These contracts deliberately consume measurements instead of pretending that
//! an architectural intention is a performance result. The runtime can publish
//! counters from `FrameDriver`, a backend can publish GPU timing, and the
//! certification scripts can feed the same values into this policy layer.

use core::fmt;

/// Supported display refresh targets for frame-budget calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshRate {
    Hz60,
    Hz90,
    Hz120,
    Hz144,
    Hz240,
}

impl RefreshRate {
    /// Frame deadline in milliseconds for the refresh rate.
    #[must_use]
    pub const fn deadline_ms(self) -> f32 {
        match self {
            Self::Hz60 => 16.666_667,
            Self::Hz90 => 11.111_111,
            Self::Hz120 => 8.333_333,
            Self::Hz144 => 6.944_444,
            Self::Hz240 => 4.166_667,
        }
    }

    /// Whether this is the baseline 60 Hz target.
    #[must_use]
    pub const fn is_baseline(self) -> bool {
        matches!(self, Self::Hz60)
    }
}

/// Hard acceptance targets for a certified Vieww build.
///
/// The eight clauses of the "Vieww standard" — frame budget, steady-state
/// allocations, clean-scene rebuilds, GPU completeness, pixel parity, input
/// latency — plus the four this layer previously left to prose: startup,
/// animation latency, frame *pacing*, and the memory-trim guarantee. Every
/// clause has a measured counterpart in [`QualityObservation`] and a named
/// violation; a contract clause nothing measures is decoration, and a
/// measurement with no clause is a graph, not a guarantee.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityContract {
    pub refresh: RefreshRate,
    /// The worst single frame allowed, in milliseconds.
    pub max_frame_ms: f32,
    /// The 95th-percentile frame allowed, in milliseconds — the *pacing*
    /// clause. A contract that only capped the worst frame would accept a
    /// renderer that misses one frame in three as long as each miss is
    /// small; users do not experience that as smooth.
    pub max_p95_frame_ms: f32,
    /// Cold start to first presented frame, in milliseconds.
    pub max_startup_ms: f32,
    pub max_steady_allocations: u64,
    pub max_clean_scene_rebuilds: u64,
    pub max_unsupported_gpu_commands: u64,
    /// Maximum pixel drift expressed as a fraction in `0..=1`.
    pub max_pixel_drift: f32,
    /// Maximum end-to-end input latency in refresh intervals.
    pub max_input_latency_intervals: f32,
    /// Maximum animation-trigger-to-first-changed-frame latency, in refresh
    /// intervals — the clause that keeps a "60 fps" claim honest about the
    /// *start* of motion, not just its continuation.
    pub max_animation_latency_intervals: f32,
    /// Whether a memory-pressure trim may change even one byte of the next
    /// frame. The answer is zero: caches are derived data, and the
    /// memory contract is that pressure costs *speed*, never *pixels*.
    pub max_trim_pixel_changes: u64,
    /// Maximum accessibility-audit issues on a standard screen. The audit
    /// is part of the standard, not a courtesy: missing labels and
    /// under-sized touch targets are defects a certification catches.
    pub max_a11y_issues: u64,
    /// Maximum typographic drift between two shaping passes of the same
    /// text, in pixels. Zero: typography is deterministic or it is not
    /// typography, it is roulette.
    pub max_typography_drift_px: f32,
}

impl Default for QualityContract {
    fn default() -> Self {
        Self::for_refresh(RefreshRate::Hz60)
    }
}

impl QualityContract {
    #[must_use]
    pub const fn for_refresh(refresh: RefreshRate) -> Self {
        Self {
            refresh,
            max_frame_ms: refresh.deadline_ms(),
            // p95 sits at the same deadline: five percent of frames may be
            // late for scheduling reasons, but not by contract.
            max_p95_frame_ms: refresh.deadline_ms(),
            // Cold start: two frames' worth at the target rate — enough to
            // parse, shape and rasterise a first screen, tight enough that a
            // framework that lazily builds the world on the first frame
            // shows up here rather than in a marketing demo.
            max_startup_ms: refresh.deadline_ms() * 2.0,
            max_steady_allocations: 0,
            max_clean_scene_rebuilds: 0,
            max_unsupported_gpu_commands: 0,
            max_pixel_drift: 0.02,
            max_input_latency_intervals: 1.0,
            max_animation_latency_intervals: 1.0,
            max_trim_pixel_changes: 0,
            max_a11y_issues: 0,
            max_typography_drift_px: 0.0,
        }
    }

    /// A deliberately stricter contract for high-refresh devices.
    #[must_use]
    pub const fn strict_120() -> Self {
        Self {
            refresh: RefreshRate::Hz120,
            max_frame_ms: RefreshRate::Hz120.deadline_ms(),
            max_p95_frame_ms: RefreshRate::Hz120.deadline_ms(),
            max_startup_ms: RefreshRate::Hz120.deadline_ms() * 2.0,
            max_steady_allocations: 0,
            max_clean_scene_rebuilds: 0,
            max_unsupported_gpu_commands: 0,
            max_pixel_drift: 0.02,
            max_input_latency_intervals: 1.0,
            max_animation_latency_intervals: 1.0,
            max_trim_pixel_changes: 0,
            max_a11y_issues: 0,
            max_typography_drift_px: 0.0,
        }
    }
}

/// Measurements gathered from one certification run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityObservation {
    pub worst_frame_ms: f32,
    /// The 95th-percentile frame time — see the contract's pacing clause.
    pub p95_frame_ms: f32,
    /// Cold start to first presented frame.
    pub startup_ms: f32,
    pub steady_allocations: u64,
    pub clean_scene_rebuilds: u64,
    pub unsupported_gpu_commands: u64,
    pub pixel_drift: f32,
    pub input_latency_intervals: f32,
    /// Animation trigger → first frame that moved, in refresh intervals.
    pub animation_latency_intervals: f32,
    /// How many bytes of the post-trim frame differ from the pre-trim one.
    pub trim_pixel_changes: u64,
    /// Accessibility-audit issue count on the standard screen.
    pub a11y_issues: u64,
    /// Typographic drift between two shaping passes, in pixels.
    pub typography_drift_px: f32,
}

/// One violated contract clause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QualityViolation {
    FrameBudget { actual_ms: f32, limit_ms: f32 },
    FramePacing { p95_ms: f32, limit_ms: f32 },
    Startup { actual_ms: f32, limit_ms: f32 },
    SteadyAllocations { actual: u64, limit: u64 },
    CleanSceneRebuilds { actual: u64, limit: u64 },
    UnsupportedGpuCommands { actual: u64, limit: u64 },
    PixelDrift { actual: f32, limit: f32 },
    InputLatency { actual: f32, limit: f32 },
    AnimationLatency { actual: f32, limit: f32 },
    TrimChangedPixels { actual: u64, limit: u64 },
    Accessibility { actual: u64, limit: u64 },
    TypographyDrift { actual_px: f32, limit_px: f32 },
}

impl fmt::Display for QualityViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameBudget {
                actual_ms,
                limit_ms,
            } => {
                write!(f, "frame_budget: {actual_ms:.3}ms > {limit_ms:.3}ms")
            }
            Self::FramePacing { p95_ms, limit_ms } => {
                write!(f, "frame_pacing: p95 {p95_ms:.3}ms > {limit_ms:.3}ms")
            }
            Self::Startup {
                actual_ms,
                limit_ms,
            } => {
                write!(f, "startup: {actual_ms:.3}ms > {limit_ms:.3}ms")
            }
            Self::SteadyAllocations { actual, limit } => {
                write!(f, "steady_allocations: {actual} > {limit}")
            }
            Self::CleanSceneRebuilds { actual, limit } => {
                write!(f, "clean_scene_rebuilds: {actual} > {limit}")
            }
            Self::UnsupportedGpuCommands { actual, limit } => {
                write!(f, "unsupported_gpu_commands: {actual} > {limit}")
            }
            Self::PixelDrift { actual, limit } => {
                write!(f, "pixel_drift: {actual:.4} > {limit:.4}")
            }
            Self::InputLatency { actual, limit } => {
                write!(f, "input_latency_intervals: {actual:.3} > {limit:.3}")
            }
            Self::AnimationLatency { actual, limit } => {
                write!(f, "animation_latency_intervals: {actual:.3} > {limit:.3}")
            }
            Self::TrimChangedPixels { actual, limit } => {
                write!(f, "trim_changed_pixels: {actual} > {limit}")
            }
            Self::Accessibility { actual, limit } => {
                write!(f, "a11y_issues: {actual} > {limit}")
            }
            Self::TypographyDrift {
                actual_px,
                limit_px,
            } => {
                write!(f, "typography_drift: {actual_px:.3}px > {limit_px:.3}px")
            }
        }
    }
}

/// Result of checking one observation against one contract.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityReport {
    pub passed: bool,
    pub violations: Vec<QualityViolation>,
}

impl QualityReport {
    #[must_use]
    pub fn check(contract: QualityContract, observation: QualityObservation) -> Self {
        let mut violations = Vec::new();
        if observation.worst_frame_ms > contract.max_frame_ms {
            violations.push(QualityViolation::FrameBudget {
                actual_ms: observation.worst_frame_ms,
                limit_ms: contract.max_frame_ms,
            });
        }
        if observation.p95_frame_ms > contract.max_p95_frame_ms {
            violations.push(QualityViolation::FramePacing {
                p95_ms: observation.p95_frame_ms,
                limit_ms: contract.max_p95_frame_ms,
            });
        }
        if observation.startup_ms > contract.max_startup_ms {
            violations.push(QualityViolation::Startup {
                actual_ms: observation.startup_ms,
                limit_ms: contract.max_startup_ms,
            });
        }
        if observation.steady_allocations > contract.max_steady_allocations {
            violations.push(QualityViolation::SteadyAllocations {
                actual: observation.steady_allocations,
                limit: contract.max_steady_allocations,
            });
        }
        if observation.clean_scene_rebuilds > contract.max_clean_scene_rebuilds {
            violations.push(QualityViolation::CleanSceneRebuilds {
                actual: observation.clean_scene_rebuilds,
                limit: contract.max_clean_scene_rebuilds,
            });
        }
        if observation.unsupported_gpu_commands > contract.max_unsupported_gpu_commands {
            violations.push(QualityViolation::UnsupportedGpuCommands {
                actual: observation.unsupported_gpu_commands,
                limit: contract.max_unsupported_gpu_commands,
            });
        }
        if observation.pixel_drift > contract.max_pixel_drift {
            violations.push(QualityViolation::PixelDrift {
                actual: observation.pixel_drift,
                limit: contract.max_pixel_drift,
            });
        }
        if observation.input_latency_intervals > contract.max_input_latency_intervals {
            violations.push(QualityViolation::InputLatency {
                actual: observation.input_latency_intervals,
                limit: contract.max_input_latency_intervals,
            });
        }
        if observation.animation_latency_intervals > contract.max_animation_latency_intervals {
            violations.push(QualityViolation::AnimationLatency {
                actual: observation.animation_latency_intervals,
                limit: contract.max_animation_latency_intervals,
            });
        }
        if observation.trim_pixel_changes > contract.max_trim_pixel_changes {
            violations.push(QualityViolation::TrimChangedPixels {
                actual: observation.trim_pixel_changes,
                limit: contract.max_trim_pixel_changes,
            });
        }
        if observation.a11y_issues > contract.max_a11y_issues {
            violations.push(QualityViolation::Accessibility {
                actual: observation.a11y_issues,
                limit: contract.max_a11y_issues,
            });
        }
        if observation.typography_drift_px > contract.max_typography_drift_px {
            violations.push(QualityViolation::TypographyDrift {
                actual_px: observation.typography_drift_px,
                limit_px: contract.max_typography_drift_px,
            });
        }
        Self {
            passed: violations.is_empty(),
            violations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_hz_budget_is_1667ms() {
        assert!((RefreshRate::Hz60.deadline_ms() - 16.666_667).abs() < 0.001);
    }

    #[test]
    fn a_clean_frame_with_zero_work_passes() {
        let report = QualityReport::check(QualityContract::default(), clean_observation());
        assert!(report.passed);
        assert!(report.violations.is_empty());
    }

    /// The observation every clause of the default contract accepts — the
    /// baseline the violation tests below perturb one field at a time.
    fn clean_observation() -> QualityObservation {
        QualityObservation {
            worst_frame_ms: 10.0,
            p95_frame_ms: 9.0,
            startup_ms: 20.0,
            steady_allocations: 0,
            clean_scene_rebuilds: 0,
            unsupported_gpu_commands: 0,
            pixel_drift: 0.0,
            input_latency_intervals: 0.5,
            animation_latency_intervals: 0.5,
            trim_pixel_changes: 0,
            a11y_issues: 0,
            typography_drift_px: 0.0,
        }
    }

    #[test]
    fn unsupported_gpu_work_is_a_hard_failure() {
        let observation = QualityObservation {
            unsupported_gpu_commands: 1,
            ..clean_observation()
        };
        let report = QualityReport::check(QualityContract::default(), observation);
        assert!(!report.passed);
        assert!(matches!(
            report.violations[0],
            QualityViolation::UnsupportedGpuCommands { .. }
        ));
    }

    /// The pacing clause: a renderer whose p95 misses the budget fails even
    /// when its worst frame is inside it — five percent of frames being
    /// late is a *pacing* defect, not a spike.
    #[test]
    fn a_slow_p95_fails_even_with_a_fast_worst_frame() {
        let observation = QualityObservation {
            worst_frame_ms: 16.0,
            p95_frame_ms: 17.2,
            ..clean_observation()
        };
        let report = QualityReport::check(QualityContract::default(), observation);
        assert!(!report.passed);
        assert!(matches!(
            report.violations[0],
            QualityViolation::FramePacing { .. }
        ));
    }

    /// The startup clause: two frames' worth at 60 Hz, measured cold.
    #[test]
    fn a_slow_cold_start_fails() {
        let observation = QualityObservation {
            startup_ms: 40.0,
            ..clean_observation()
        };
        let report = QualityReport::check(QualityContract::default(), observation);
        assert!(!report.passed);
        assert!(matches!(
            report.violations[0],
            QualityViolation::Startup { .. }
        ));
    }

    /// The animation-latency clause: motion that starts a frame late is a
    /// contract violation even if it is smooth afterwards.
    #[test]
    fn a_late_animation_start_fails() {
        let observation = QualityObservation {
            animation_latency_intervals: 1.4,
            ..clean_observation()
        };
        let report = QualityReport::check(QualityContract::default(), observation);
        assert!(!report.passed);
        assert!(matches!(
            report.violations[0],
            QualityViolation::AnimationLatency { .. }
        ));
    }

    /// The memory clause: a trim that changes one byte of the next frame
    /// fails — pressure must cost speed, never pixels.
    #[test]
    fn a_trim_that_changes_pixels_fails() {
        let observation = QualityObservation {
            trim_pixel_changes: 1,
            ..clean_observation()
        };
        let report = QualityReport::check(QualityContract::default(), observation);
        assert!(!report.passed);
        assert!(matches!(
            report.violations[0],
            QualityViolation::TrimChangedPixels { .. }
        ));
    }
}
