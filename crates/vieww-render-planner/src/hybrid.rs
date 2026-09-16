//! Splitting one workload across CPU and GPU.
//!
//! This is item 3 from the external review this codebase's own docs quote
//! elsewhere: CPU/GPU/hybrid should be a *policy*, computed per frame from
//! device and workload, not three hand-picked backends. [`split`] is the
//! actual arithmetic: given how fast each side is for this workload, find
//! the fraction that makes both sides finish at (about) the same time,
//! which is the throughput-optimal split for two workers processing
//! disjoint, independent slices of the same work.

/// A CPU/GPU work split, as the fraction of the total work assigned to the
/// CPU. `0.0` means "all GPU," `1.0` means "all CPU."
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub cpu_fraction: f32,
}

impl Split {
    pub const ALL_CPU: Self = Self { cpu_fraction: 1.0 };
    pub const ALL_GPU: Self = Self { cpu_fraction: 0.0 };

    #[must_use]
    pub fn gpu_fraction(self) -> f32 {
        1.0 - self.cpu_fraction
    }
}

/// Compute the throughput-balancing split for a workload of `total_units`,
/// given each side's steady-state throughput (units/second) and a fixed
/// dispatch overhead the GPU side always pays.
///
/// # The math
///
/// If the CPU takes `cpu_fraction * total / cpu_rate` seconds and the GPU
/// takes `gpu_overhead + (1 - cpu_fraction) * total / gpu_rate` seconds, the
/// split that makes them equal solves:
///
/// ```text
/// cpu_fraction / cpu_rate = gpu_overhead / total + (1 - cpu_fraction) / gpu_rate
/// cpu_fraction * (1/cpu_rate + 1/gpu_rate) = gpu_overhead / total + 1/gpu_rate
/// cpu_fraction = (gpu_overhead / total + 1/gpu_rate) / (1/cpu_rate + 1/gpu_rate)
/// ```
///
/// Clamped to `[0, 1]` because the overhead term can push the unclamped
/// solution past either end for a small enough `total_units` — the correct
/// answer there is "give it all to whichever side dispatch overhead
/// doesn't ruin," which is exactly what clamping produces.
#[must_use]
pub fn split(total_units: f32, cpu_rate: f32, gpu_rate: f32, gpu_overhead_seconds: f32) -> Split {
    if gpu_rate <= 0.0 {
        return Split::ALL_CPU;
    }
    if cpu_rate <= 0.0 {
        return Split::ALL_GPU;
    }
    let total = total_units.max(0.000_1);
    let numerator = gpu_overhead_seconds / total + 1.0 / gpu_rate;
    let denominator = 1.0 / cpu_rate + 1.0 / gpu_rate;
    Split {
        cpu_fraction: (numerator / denominator).clamp(0.0, 1.0),
    }
}

/// Whether splitting is even worth considering for this pass. Below a
/// minimum unit count, the coordination cost of a split (partitioning
/// geometry, two separate submits, a merge step) exceeds anything it could
/// save — matching `docs/RENDERER-V2-NOTES.md`'s caution that GPU-driven
/// paths "should stay selective, not a wholesale rewrite."
#[must_use]
pub fn worth_splitting(total_units: f32, minimum_units: f32) -> bool {
    total_units >= minimum_units
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_much_faster_gpu_takes_most_of_the_work() {
        let s = split(10_000_000.0, 250.0, 8000.0, 0.0005);
        assert!(s.cpu_fraction < 0.2, "cpu_fraction = {}", s.cpu_fraction);
    }

    #[test]
    fn a_tiny_workload_is_dominated_by_gpu_overhead_and_favors_cpu() {
        // Realistic units this time: rates in pixels/second, a genuinely
        // small (1000-pixel) workload. The GPU's fixed 0.5ms dispatch
        // overhead swamps its per-pixel advantage at this size, even though
        // it is 32x faster per pixel once running.
        let s = split(1_000.0, 250_000_000.0, 8_000_000_000.0, 0.0005);
        assert!(s.cpu_fraction > 0.5, "cpu_fraction = {}", s.cpu_fraction);
    }

    #[test]
    fn equal_rates_and_no_overhead_split_evenly() {
        let s = split(1_000_000.0, 100.0, 100.0, 0.0);
        assert!((s.cpu_fraction - 0.5).abs() < 1e-3);
    }

    #[test]
    fn no_gpu_forces_all_cpu() {
        assert_eq!(split(1000.0, 100.0, 0.0, 0.0), Split::ALL_CPU);
    }
}
