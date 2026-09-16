//! A first-class render graph.
//!
//! `docs/RENDERER-V2-NOTES.md`'s Architecture section names this as the
//! central missing pillar: "render graph as a first-class component, not an
//! implementation detail... what is the cheapest possible execution plan
//! for this frame, not how do I execute these commands." This crate is
//! that component, built on top of [`vieww_scene::SceneGraph`] rather than
//! reaching back into `vieww_paint::Scene`'s flat command list.
//!
//! # Pipeline
//!
//! ```text
//! vieww_scene::SceneGraph
//!         │  scene_bridge::build_graph
//!         ▼
//!      Graph            (passes + resources, as declared)
//!         │  Graph::compile
//!         ▼
//!   ExecutionPlan        (ordered, culled, aliased, batched, barriered)
//! ```
//!
//! Nothing in this crate executes anything — it produces the *plan*.
//! `vieww-render-planner` decides, per pass, whether that plan's CPU/GPU
//! shape should actually run on the CPU, the GPU, or some hybrid split, and
//! a concrete backend (`vieww-gpu-vulkan` and friends, or
//! `vieww_paint::native` for the CPU path this workspace ships today)
//! carries the plan out.

pub mod graph;
pub mod pass;
pub mod plan;
pub mod resource;
pub mod scene_bridge;
mod transient;

pub use graph::Graph;
pub use pass::{PassDesc, PassId, PassKind};
pub use plan::{Barrier, CompileError, ExecutionPlan};
pub use resource::{ResourceDesc, ResourceId, ResourceKind};
pub use scene_bridge::build_graph;
