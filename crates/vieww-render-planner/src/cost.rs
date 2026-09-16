//! Estimating what a pass costs on a given executor and device.
//!
//! Two layers, matching `docs/RENDERER-V2-NOTES.md`'s "feed measured cost
//! back into planning": [`HeuristicCostModel`] is a closed-form estimate
//! from [`vieww_scene::CostHint`] and [`DeviceProfile`] — always available,
//! even before a single frame has run — and [`MeasuredCostModel`] wraps it,
//! overriding the heuristic with an exponential moving average of real
//! measured durations once a pass (identified by name) has actually run at
//! least once.

use std::collections::HashMap;

use vieww_render_graph::{PassDesc, PassKind};
use vieww_scene::Affinity;

use crate::capabilities::DeviceProfile;

/// Which executor a pass could run on. The planner's actual placement
/// decision is [`crate::policy::Placement`]; this is just "how long would
/// each option take."
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExecutorEstimate {
    pub cpu_ms: f32,
    /// `None` when the device has no GPU at all — not "very expensive," a
    /// genuinely unavailable option so [`crate::policy`] never accidentally
    /// picks it.
    pub gpu_ms: Option<f32>,
}

/// Something that can estimate a pass's cost. A trait so
/// [`HeuristicCostModel`] and [`MeasuredCostModel`] are interchangeable
/// wherever `planner::Planner` needs an estimate.
pub trait CostModel {
    fn estimate(&self, pass: &PassDesc, pixels: f32, device: &DeviceProfile) -> ExecutorEstimate;
}

/// A pure, closed-form estimate: pixels touched divided by the device's
/// measured-elsewhere throughput number, plus GPU dispatch overhead.
///
/// This is deliberately simple — a real backend's actual cost has cache
/// effects, driver overhead variance, and thermal throttling this can never
/// capture — which is exactly why [`MeasuredCostModel`] exists to correct
/// it once real numbers are available. A heuristic that tried to be more
/// "realistic" without real measurements behind it would just be
/// confidently wrong in a different way.
#[derive(Debug, Default)]
pub struct HeuristicCostModel;

impl CostModel for HeuristicCostModel {
    fn estimate(&self, pass: &PassDesc, pixels: f32, device: &DeviceProfile) -> ExecutorEstimate {
        let mpix = (pixels / 1_000_000.0).max(0.000_1);
        let cpu_ms = 1000.0 * mpix / device.cpu_fill_mpix_per_sec.max(0.001);

        let gpu_ms = if device.has_gpu() {
            let raw = 1000.0 * mpix / device.gpu_fill_mpix_per_sec.max(0.001);
            // A filter/compute-shaped pass amortizes dispatch overhead
            // across more work than a trivial raster pass does, but every
            // GPU pass pays it at least once.
            let overhead = device.gpu_dispatch_overhead_ms
                * match pass.kind {
                    PassKind::Filter => 1.0,
                    _ => 1.0,
                };
            Some(raw + overhead)
        } else {
            None
        };

        ExecutorEstimate { cpu_ms, gpu_ms }
    }
}

/// A key identifying "the same pass" across frames, for measurement
/// history. Passes don't have stable ids across `Graph`s (a `PassId` is
/// only meaningful within the graph it came from), so history is keyed by
/// name — a real integration would want a more precise key (name plus a
/// content hash of what it reads/writes), left as an extension point via
/// `MeasuredCostModel::record`'s generic key type.
pub type MeasurementKey = &'static str;

/// Wraps a [`HeuristicCostModel`] with a measured-cost override, updated
/// via an exponential moving average so one outlier frame (a GC pause, a
/// thermal spike) does not swing the estimate wildly.
#[derive(Debug)]
pub struct MeasuredCostModel<M: CostModel = HeuristicCostModel> {
    fallback: M,
    /// EMA smoothing factor: how much weight the newest sample gets.
    alpha: f32,
    cpu_history: HashMap<MeasurementKey, f32>,
    gpu_history: HashMap<MeasurementKey, f32>,
}

impl MeasuredCostModel<HeuristicCostModel> {
    #[must_use]
    pub fn new() -> Self {
        Self::with_fallback(HeuristicCostModel, 0.2)
    }
}

impl Default for MeasuredCostModel<HeuristicCostModel> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: CostModel> MeasuredCostModel<M> {
    #[must_use]
    pub fn with_fallback(fallback: M, alpha: f32) -> Self {
        Self {
            fallback,
            alpha: alpha.clamp(0.0, 1.0),
            cpu_history: HashMap::new(),
            gpu_history: HashMap::new(),
        }
    }

    /// Record a real, measured duration for a pass that actually ran on
    /// `executor`. Subsequent [`CostModel::estimate`] calls for the same
    /// key use an EMA of every recorded sample instead of the heuristic.
    pub fn record_cpu(&mut self, key: MeasurementKey, measured_ms: f32) {
        record(&mut self.cpu_history, key, measured_ms, self.alpha);
    }

    pub fn record_gpu(&mut self, key: MeasurementKey, measured_ms: f32) {
        record(&mut self.gpu_history, key, measured_ms, self.alpha);
    }

    /// Estimate using measurement history keyed by `key`, falling back to
    /// the wrapped heuristic model per-executor where no history exists
    /// yet.
    #[must_use]
    pub fn estimate_keyed(
        &self,
        key: MeasurementKey,
        pass: &PassDesc,
        pixels: f32,
        device: &DeviceProfile,
    ) -> ExecutorEstimate {
        let heuristic = self.fallback.estimate(pass, pixels, device);
        ExecutorEstimate {
            cpu_ms: self
                .cpu_history
                .get(key)
                .copied()
                .unwrap_or(heuristic.cpu_ms),
            gpu_ms: heuristic
                .gpu_ms
                .map(|h| self.gpu_history.get(key).copied().unwrap_or(h)),
        }
    }
}

fn record(
    history: &mut HashMap<MeasurementKey, f32>,
    key: MeasurementKey,
    sample: f32,
    alpha: f32,
) {
    history
        .entry(key)
        .and_modify(|ema| *ema = alpha * sample + (1.0 - alpha) * *ema)
        .or_insert(sample);
}

impl<M: CostModel> CostModel for MeasuredCostModel<M> {
    fn estimate(&self, pass: &PassDesc, pixels: f32, device: &DeviceProfile) -> ExecutorEstimate {
        self.estimate_keyed(pass.name, pass, pixels, device)
    }
}

/// Whether a pass's [`vieww_scene::Affinity`] rules an executor out
/// entirely, independent of cost — a `GpuOnly` pass with no GPU present
/// still has to run *somewhere*, so this is advisory for CPU (never
/// actually forbidden) and load-bearing only in that it biases
/// [`crate::policy`] strongly toward the affinity's preferred side.
#[must_use]
pub fn affinity_bias(affinity: Affinity) -> f32 {
    match affinity {
        Affinity::Cpu => -1.0,
        Affinity::Balanced => 0.0,
        Affinity::Gpu => 0.6,
        Affinity::GpuOnly => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_render_graph::{PassDesc, PassKind};
    use vieww_scene::CostHint;

    fn pass() -> PassDesc {
        PassDesc::new("p", PassKind::Raster, CostHint::CONSERVATIVE)
    }

    #[test]
    fn a_device_with_no_gpu_reports_no_gpu_estimate() {
        let est =
            HeuristicCostModel.estimate(&pass(), 1_000_000.0, &DeviceProfile::CPU_ONLY_FALLBACK);
        assert!(est.gpu_ms.is_none());
    }

    #[test]
    fn a_discrete_gpu_is_faster_per_pixel_than_cpu_at_scale() {
        let est =
            HeuristicCostModel.estimate(&pass(), 100_000_000.0, &DeviceProfile::DESKTOP_DISCRETE);
        assert!(est.gpu_ms.unwrap() < est.cpu_ms);
    }

    #[test]
    fn measured_history_overrides_the_heuristic() {
        let mut model = MeasuredCostModel::new();
        let device = DeviceProfile::DESKTOP_DISCRETE;
        let before = model.estimate_keyed("my_pass", &pass(), 1_000_000.0, &device);
        model.record_cpu("my_pass", 0.001);
        let after = model.estimate_keyed("my_pass", &pass(), 1_000_000.0, &device);
        assert_ne!(before.cpu_ms, after.cpu_ms);
    }

    #[test]
    fn ema_smooths_toward_the_new_sample_without_jumping_all_the_way() {
        let mut model = MeasuredCostModel::with_fallback(HeuristicCostModel, 0.5);
        model.record_cpu("p", 10.0);
        model.record_cpu("p", 20.0);
        // 0.5*10 + 0.5*(prior 10) = 10 first time (inserted directly), then
        // 0.5*20 + 0.5*10 = 15 second time.
        let device = DeviceProfile::CPU_ONLY_FALLBACK;
        let est = model.estimate_keyed("p", &pass(), 1_000_000.0, &device);
        assert!((est.cpu_ms - 15.0).abs() < 1e-6);
    }
}
