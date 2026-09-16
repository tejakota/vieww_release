//! Top-level orchestration: turn a compiled [`ExecutionPlan`] into a
//! per-pass [`Placement`] for one specific device and frame budget.

use std::collections::HashMap;

use vieww_render_graph::{ExecutionPlan, Graph, PassId};

use crate::capabilities::DeviceProfile;
use crate::cost::CostModel;
use crate::frame_budget::FrameBudget;
use crate::policy::{self, Placement};

/// The planner's output for one frame: a placement for every surviving
/// pass, plus the summary numbers `docs/RENDERER-V2-NOTES.md`'s benchmark
/// section and this workspace's own README example ask for directly
/// ("Frame 100: GPU = 82%, CPU = 18%").
#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    pub placements: HashMap<PassId, Placement>,
    pub predicted_ms: f32,
    pub deadline_ms: f32,
}

impl FramePlan {
    /// Fraction of total predicted work-time running on the GPU, weighted
    /// by each pass's own predicted duration — a pass placed `Hybrid`
    /// contributes proportionally to its split, not as a whole unit either
    /// way.
    #[must_use]
    pub fn gpu_share(&self, per_pass_ms: &HashMap<PassId, f32>) -> f32 {
        let total: f32 = per_pass_ms.values().sum();
        if total <= 0.0 {
            return 0.0;
        }
        let gpu_weighted: f32 = self
            .placements
            .iter()
            .map(|(id, placement)| {
                per_pass_ms.get(id).copied().unwrap_or(0.0) * placement.gpu_fraction()
            })
            .sum();
        gpu_weighted / total
    }

    #[must_use]
    pub fn missed_deadline(&self) -> bool {
        self.predicted_ms > self.deadline_ms
    }
}

/// Plan one frame.
///
/// `graph` and `plan` must be the [`Graph`]/[`ExecutionPlan`] pair produced
/// together (`plan` came from `graph.compile(..)`) — this function trusts
/// that pairing rather than re-deriving it.
pub fn plan_frame(
    graph: &Graph,
    exec_plan: &ExecutionPlan,
    device: &DeviceProfile,
    model: &dyn CostModel,
    mut budget: FrameBudget,
) -> (FramePlan, HashMap<PassId, f32>) {
    let mut placements = HashMap::new();
    let mut per_pass_ms = HashMap::new();

    for &pass_id in &exec_plan.order {
        let pass = graph.pass(pass_id);
        let pixels: f32 = pass
            .writes
            .iter()
            .map(|r| {
                let desc = graph.resource(*r);
                (desc.width as f32) * (desc.height.max(1) as f32)
            })
            .sum();

        let estimate = model.estimate(pass, pixels, device);
        let placement = policy::decide(pass, pixels, estimate, device, &budget);

        let ms = match placement {
            Placement::Cpu => estimate.cpu_ms,
            Placement::Gpu => estimate.gpu_ms.unwrap_or(estimate.cpu_ms),
            Placement::Hybrid(split) => {
                let gpu_ms = estimate.gpu_ms.unwrap_or(estimate.cpu_ms);
                // The two sides run concurrently, so the pass's wall time is
                // the slower of the two shares, not their sum.
                (split.cpu_fraction * estimate.cpu_ms).max(split.gpu_fraction() * gpu_ms)
            }
        };

        budget.spend(ms);
        placements.insert(pass_id, placement);
        per_pass_ms.insert(pass_id, ms);
    }

    let predicted_ms = per_pass_ms.values().sum();
    (
        FramePlan {
            placements,
            predicted_ms,
            deadline_ms: budget.deadline_ms,
        },
        per_pass_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::HeuristicCostModel;
    use vieww_foundation::{Color, Rect};
    use vieww_paint::{Canvas, Paint, Scene};
    use vieww_scene::SceneGraph;

    #[test]
    fn a_cpu_only_device_places_every_pass_on_cpu() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        let scene_graph = SceneGraph::from_scene(&scene);
        let (graph, screen) = vieww_render_graph::build_graph(&scene_graph, 200, 200);
        let exec_plan = graph.compile(screen).unwrap();

        let (frame_plan, _) = plan_frame(
            &graph,
            &exec_plan,
            &DeviceProfile::CPU_ONLY_FALLBACK,
            &HeuristicCostModel,
            FrameBudget::new(16.0),
        );

        assert!(frame_plan
            .placements
            .values()
            .all(|p| matches!(p, Placement::Cpu)));
    }

    #[test]
    fn a_heavy_blur_on_a_capable_device_lands_on_the_gpu() {
        let mut scene = Scene::new();
        scene.push_filtered_layer(
            Rect::new(0.0, 0.0, 1000.0, 1000.0),
            1.0,
            vieww_foundation::BlendMode::Normal,
            vieww_foundation::ImageFilter::blur(20.0),
        );
        scene.fill_rect(
            Rect::new(0.0, 0.0, 1000.0, 1000.0),
            Paint::solid(Color::RED),
        );
        scene.pop_layer();
        let scene_graph = SceneGraph::from_scene(&scene);
        let (graph, screen) = vieww_render_graph::build_graph(&scene_graph, 1000, 1000);
        let exec_plan = graph.compile(screen).unwrap();

        let (frame_plan, per_pass_ms) = plan_frame(
            &graph,
            &exec_plan,
            &DeviceProfile::DESKTOP_DISCRETE,
            &HeuristicCostModel,
            FrameBudget::new(16.0),
        );

        assert!(frame_plan.gpu_share(&per_pass_ms) > 0.0);
    }
}
