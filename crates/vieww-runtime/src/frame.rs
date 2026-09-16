//! Frame lifecycle orchestration.
//!
//! Ties the pipeline the target architecture names explicitly together:
//!
//! ```text
//! Input → Priority scheduler → State update → Incremental tree update →
//! Layout → Render-plan generation → GPU/CPU scheduling
//! ```
//!
//! Each arrow above is a [`Stage`] here. This crate does not implement any
//! stage's *content* — widget rebuild, layout, and scene-graph compilation
//! all live in `vieww-element`/`vieww-render`/`vieww-scene`/
//! `vieww-render-graph`/`vieww-render-planner`, each already real and
//! tested in its own crate. What was missing, and what this module adds, is
//! the thing that runs them *in order, once per frame, with per-stage
//! timing* — the seam a real `App` (`vieww-platform-winit`'s, today) plugs
//! its own concrete stages into.

use std::time::{Duration, Instant};

use vieww_render_planner::FrameBudget;

/// One named phase of a frame.
pub trait Stage {
    fn name(&self) -> &'static str;
    fn run(&mut self);
}

/// A stage built from a closure, for tests and for callers that don't want
/// a whole type per stage.
pub struct FnStage<F: FnMut()> {
    name: &'static str,
    f: F,
}

impl<F: FnMut()> std::fmt::Debug for FnStage<F> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FnStage")
            .field("name", &self.name)
            .finish()
    }
}

impl<F: FnMut()> FnStage<F> {
    pub fn new(name: &'static str, f: F) -> Self {
        Self { name, f }
    }
}

impl<F: FnMut()> Stage for FnStage<F> {
    fn name(&self) -> &'static str {
        self.name
    }
    fn run(&mut self) {
        (self.f)();
    }
}

/// How long each stage took in one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct StageTiming {
    pub name: &'static str,
    pub duration: Duration,
}

/// The result of running one frame through the pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameReport {
    pub stages: Vec<StageTiming>,
}

impl FrameReport {
    #[must_use]
    pub fn total(&self) -> Duration {
        self.stages.iter().map(|s| s.duration).sum()
    }

    /// The single slowest stage — the direct answer to
    /// `docs/RENDERER-V2-NOTES.md`'s devtools wishlist item, "why did this
    /// frame take 11.2ms," at the granularity this crate actually has
    /// (per-stage, not per-shader).
    #[must_use]
    pub fn slowest(&self) -> Option<&StageTiming> {
        self.stages.iter().max_by_key(|s| s.duration)
    }

    /// Charge this frame's total stage time against a
    /// [`vieww_render_planner::FrameBudget`] — the seam between "how long
    /// did the non-rendering pipeline stages take" and the planner's own
    /// per-pass budgeting, which needs to know how much of the frame
    /// deadline they already spent before a single render pass runs.
    #[must_use]
    pub fn charge_to(&self, mut budget: FrameBudget) -> FrameBudget {
        budget.spend(self.total().as_secs_f32() * 1000.0);
        budget
    }
}

/// Runs a fixed sequence of [`Stage`]s once per frame, timing each one.
pub struct FrameOrchestrator {
    stages: Vec<Box<dyn Stage>>,
}

impl std::fmt::Debug for FrameOrchestrator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameOrchestrator")
            .field("stage_count", &self.stages.len())
            .finish()
    }
}

impl Default for FrameOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameOrchestrator {
    #[must_use]
    pub fn new() -> Self {
        Self { stages: Vec::new() }
    }

    #[must_use]
    pub fn with_stage(mut self, stage: impl Stage + 'static) -> Self {
        self.stages.push(Box::new(stage));
        self
    }

    /// Run every stage in the order they were added, returning per-stage
    /// timing.
    pub fn run_frame(&mut self) -> FrameReport {
        let mut stages = Vec::with_capacity(self.stages.len());
        for stage in &mut self.stages {
            let start = Instant::now();
            stage.run();
            stages.push(StageTiming {
                name: stage.name(),
                duration: start.elapsed(),
            });
        }
        FrameReport { stages }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn stages_run_in_the_order_they_were_added() {
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let o1 = Arc::clone(&order);
        let o2 = Arc::clone(&order);

        let mut orchestrator = FrameOrchestrator::new()
            .with_stage(FnStage::new("input", move || {
                o1.lock().unwrap().push("input")
            }))
            .with_stage(FnStage::new("layout", move || {
                o2.lock().unwrap().push("layout")
            }));

        orchestrator.run_frame();
        assert_eq!(*order.lock().unwrap(), vec!["input", "layout"]);
    }

    #[test]
    fn charging_a_budget_reduces_its_remaining_time() {
        let mut orchestrator = FrameOrchestrator::new().with_stage(FnStage::new("work", || {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }));
        let report = orchestrator.run_frame();
        let budget = report.charge_to(FrameBudget::new(16.0));
        assert!(budget.remaining_ms() < 16.0);
    }

    #[test]
    fn the_report_names_the_slowest_stage() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        let mut orchestrator = FrameOrchestrator::new()
            .with_stage(FnStage::new("fast", || {}))
            .with_stage(FnStage::new("slow", move || {
                c.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }));

        let report = orchestrator.run_frame();
        assert_eq!(report.slowest().unwrap().name, "slow");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
