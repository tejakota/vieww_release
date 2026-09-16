//! Per-pass placement decisions.
//!
//! This is the policy the review this workspace's docs quote asks for
//! directly: "CPU/GPU/hybrid should be a policy, not just three
//! independent backends." [`decide`] is that policy, as one pure function
//! from (pass shape, cost estimate, device, budget) to a [`Placement`] —
//! nothing here executes anything, which is what makes every rule in it
//! independently testable.

use vieww_render_graph::PassDesc;
use vieww_scene::Affinity;

use crate::capabilities::DeviceProfile;
use crate::cost::{affinity_bias, ExecutorEstimate};
use crate::frame_budget::FrameBudget;
use crate::hybrid::{self, Split};
use crate::power;

/// Below this many touched pixels, splitting a pass across two executors
/// never pays for its own coordination cost — see
/// [`hybrid::worth_splitting`].
pub const HYBRID_MIN_PIXELS: f32 = 250_000.0;

/// Where a pass ends up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
    Cpu,
    Gpu,
    /// Split across both, in the given proportion.
    Hybrid(Split),
}

impl Placement {
    /// The fraction of this placement's work that runs on the GPU — `0.0`
    /// for [`Self::Cpu`], `1.0` for [`Self::Gpu`], the split's own fraction
    /// for [`Self::Hybrid`]. What [`crate::planner::FramePlan::gpu_share`]
    /// sums over every pass.
    #[must_use]
    pub fn gpu_fraction(self) -> f32 {
        match self {
            Self::Cpu => 0.0,
            Self::Gpu => 1.0,
            Self::Hybrid(s) => s.gpu_fraction(),
        }
    }
}

/// Decide where one pass should run.
///
/// `pixels` is the pass's approximate touched-pixel count (from its output
/// resource dimensions) — the same unit [`crate::cost::CostModel`] uses.
#[must_use]
pub fn decide(
    pass: &PassDesc,
    pixels: f32,
    estimate: ExecutorEstimate,
    device: &DeviceProfile,
    budget: &FrameBudget,
) -> Placement {
    let Some(gpu_ms) = estimate.gpu_ms else {
        return Placement::Cpu;
    };

    let bias = affinity_bias(pass.cost.gpu_affinity);

    // A pass whose shape is fundamentally CPU-shaped never moves, no
    // matter how idle the GPU is — the review's own example (a single
    // small fill, a short text run) where GPU dispatch overhead alone
    // would lose.
    if pass.cost.gpu_affinity == Affinity::Cpu {
        return Placement::Cpu;
    }

    // A pass that is only sensible on a GPU goes there whenever one
    // exists, regardless of the raw number — the CPU "fallback" this
    // models is a correctness fallback (device without the affinity's
    // preferred hardware), not a cost comparison.
    if pass.cost.gpu_affinity == Affinity::GpuOnly {
        return Placement::Gpu;
    }

    // GPU-leaning work gets a modest discount to its effective cost,
    // modelling benefits a raw pixel-throughput number doesn't capture on
    // its own (residency across frames, batching many instances) —
    // capped well short of ever inventing a win the raw numbers don't
    // support.
    let effective_gpu_ms = gpu_ms * (1.0 - 0.3 * bias.max(0.0));
    let cpu_ms = estimate.cpu_ms;

    let budget_tight = budget.is_tight(device.frame_deadline_ms() * 0.2);

    // Neither side clearly wins and the pass is big enough to bother:
    // split it.
    let ratio = effective_gpu_ms / cpu_ms.max(0.000_1);
    if hybrid::worth_splitting(pixels, HYBRID_MIN_PIXELS) && (0.3..=3.0).contains(&ratio) {
        let split = hybrid::split(
            pixels,
            device.cpu_fill_mpix_per_sec * 1_000_000.0,
            device.gpu_fill_mpix_per_sec * 1_000_000.0,
            device.gpu_dispatch_overhead_ms / 1000.0,
        );
        if split.cpu_fraction > 0.02 && split.gpu_fraction() > 0.02 {
            return Placement::Hybrid(split);
        }
    }

    let (faster_ms, faster_on_gpu) = if effective_gpu_ms <= cpu_ms {
        (effective_gpu_ms, true)
    } else {
        (cpu_ms, false)
    };
    let (slower_ms, slower_on_gpu) = if faster_on_gpu {
        (cpu_ms, false)
    } else {
        (effective_gpu_ms, true)
    };

    let faster_energy = power::energy_cost(faster_ms, faster_on_gpu, device);
    let slower_energy = power::energy_cost(slower_ms, slower_on_gpu, device);

    if power::prefer_lower_energy(
        device,
        budget_tight,
        faster_ms,
        faster_energy,
        slower_ms,
        slower_energy,
        0.5,
    ) {
        return if slower_on_gpu {
            Placement::Gpu
        } else {
            Placement::Cpu
        };
    }

    if faster_on_gpu {
        Placement::Gpu
    } else {
        Placement::Cpu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_render_graph::PassKind;
    use vieww_scene::{Cacheability, CostHint, Dynamicity};

    fn pass_with_affinity(affinity: Affinity) -> PassDesc {
        PassDesc::new(
            "p",
            PassKind::Raster,
            CostHint {
                dynamicity: Dynamicity::Occasional,
                cacheability: Cacheability::High,
                gpu_affinity: affinity,
                estimated_bytes: 0,
            },
        )
    }

    #[test]
    fn no_gpu_estimate_means_cpu_regardless_of_affinity() {
        let p = decide(
            &pass_with_affinity(Affinity::GpuOnly),
            1_000_000.0,
            ExecutorEstimate {
                cpu_ms: 1.0,
                gpu_ms: None,
            },
            &DeviceProfile::CPU_ONLY_FALLBACK,
            &FrameBudget::new(16.0),
        );
        assert_eq!(p, Placement::Cpu);
    }

    #[test]
    fn cpu_affinity_never_moves_even_when_gpu_looks_cheaper() {
        let p = decide(
            &pass_with_affinity(Affinity::Cpu),
            10.0,
            ExecutorEstimate {
                cpu_ms: 5.0,
                gpu_ms: Some(0.001),
            },
            &DeviceProfile::DESKTOP_DISCRETE,
            &FrameBudget::new(16.0),
        );
        assert_eq!(p, Placement::Cpu);
    }

    #[test]
    fn gpu_only_affinity_always_goes_to_the_gpu_when_present() {
        let p = decide(
            &pass_with_affinity(Affinity::GpuOnly),
            10.0,
            ExecutorEstimate {
                cpu_ms: 0.1,
                gpu_ms: Some(5.0),
            },
            &DeviceProfile::DESKTOP_DISCRETE,
            &FrameBudget::new(16.0),
        );
        assert_eq!(p, Placement::Gpu);
    }

    #[test]
    fn a_huge_balanced_pass_with_comparable_costs_gets_split() {
        let p = decide(
            &pass_with_affinity(Affinity::Balanced),
            2_000_000.0,
            ExecutorEstimate {
                cpu_ms: 10.0,
                gpu_ms: Some(11.0),
            },
            &DeviceProfile::DESKTOP_DISCRETE,
            &FrameBudget::new(16.0),
        );
        assert!(matches!(p, Placement::Hybrid(_)));
    }

    #[test]
    fn a_tiny_pass_is_never_split_even_if_costs_are_close() {
        let p = decide(
            &pass_with_affinity(Affinity::Balanced),
            100.0,
            ExecutorEstimate {
                cpu_ms: 0.01,
                gpu_ms: Some(0.011),
            },
            &DeviceProfile::DESKTOP_DISCRETE,
            &FrameBudget::new(16.0),
        );
        assert!(!matches!(p, Placement::Hybrid(_)));
    }
}
