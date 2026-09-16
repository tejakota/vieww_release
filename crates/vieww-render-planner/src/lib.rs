//! Cost-aware CPU/GPU/hybrid rendering: the planner half of
//! `docs/RENDERER-V2-NOTES.md`'s "what is the cheapest possible execution
//! plan for this frame" question, and a direct implementation of the
//! external review's point that "CPU/GPU/hybrid should be a policy, not
//! just three backends."
//!
//! # Pipeline
//!
//! ```text
//! vieww_render_graph::ExecutionPlan  +  DeviceProfile  +  FrameBudget
//!         │  planner::plan_frame
//!         ▼
//!      FramePlan       (a Placement — Cpu / Gpu / Hybrid(split) — per pass)
//! ```
//!
//! Nothing here executes a pass. `FramePlan` is hand back to whatever
//! backend actually carries out each pass — the CPU path this workspace
//! ships today (`vieww_paint::native`) for every `Placement::Cpu`, and
//! `vieww-gpu`'s backends for `Placement::Gpu`/`Placement::Hybrid` once
//! wired (see that crate's own docs for exactly what is real there today
//! versus what needs a GPU to actually run).

pub mod capabilities;
pub mod cost;
pub mod frame_budget;
pub mod hybrid;
pub mod planner;
pub mod policy;
pub mod power;
pub mod quality;

pub use capabilities::{DeviceProfile, GpuKind};
pub use cost::{CostModel, ExecutorEstimate, HeuristicCostModel, MeasuredCostModel};
pub use frame_budget::FrameBudget;
pub use hybrid::Split;
pub use planner::{plan_frame, FramePlan};
pub use policy::Placement;
pub use quality::{
    QualityContract, QualityObservation, QualityReport, QualityViolation, RefreshRate,
};
