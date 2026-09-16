//! Power-aware adjustment.
//!
//! `docs/RENDERER-V2-NOTES.md`: "add power-per-frame to whatever benchmark
//! suite eventually exists — a renderer 5% faster but 20% more power-hungry
//! is not a win on a phone." This module is the planning-time half of that:
//! a rough energy estimate per placement choice, used to break ties (and
//! sometimes overrule a small speed win) when the device is
//! battery-powered.

use crate::capabilities::DeviceProfile;

/// Rough relative energy cost of running `duration_ms` of work on the GPU
/// versus the CPU, on `device`. Units are arbitrary and only meaningful
/// compared against each other, never against a real joule measurement —
/// this is a planning signal, not an energy audit.
#[must_use]
pub fn energy_cost(duration_ms: f32, on_gpu: bool, device: &DeviceProfile) -> f32 {
    let base = duration_ms.max(0.0);
    if on_gpu {
        base * device.gpu_relative_power_cost
    } else {
        base
    }
}

/// Whether a candidate placement should be rejected purely on power
/// grounds, given the current budget pressure.
///
/// The rule: on a battery-powered device with budget to spare (not
/// [`crate::frame_budget::FrameBudget::is_tight`]), prefer the
/// lower-energy option even if it is a little slower — up to
/// `speed_tradeoff`, the fraction of extra time the lower-energy option is
/// allowed to cost before it is rejected anyway (a "not a win" cutoff
/// mirroring the module doc's example: a renderer trading a little speed
/// for a lot of power is a win; trading a lot of speed for a little power
/// is not).
#[must_use]
pub fn prefer_lower_energy(
    device: &DeviceProfile,
    budget_is_tight: bool,
    faster_ms: f32,
    faster_energy: f32,
    slower_but_greener_ms: f32,
    slower_but_greener_energy: f32,
    speed_tradeoff: f32,
) -> bool {
    if !device.is_battery_powered || budget_is_tight {
        return false;
    }
    if slower_but_greener_energy >= faster_energy {
        return false;
    }
    let extra_time_fraction = (slower_but_greener_ms - faster_ms) / faster_ms.max(0.001);
    extra_time_fraction <= speed_tradeoff
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::DeviceProfile;

    #[test]
    fn a_desktop_gpu_costs_more_energy_per_ms_than_cpu() {
        let device = DeviceProfile::DESKTOP_DISCRETE;
        assert!(energy_cost(1.0, true, &device) > energy_cost(1.0, false, &device));
    }

    #[test]
    fn a_mobile_gpu_can_cost_less_energy_than_cpu() {
        let device = DeviceProfile::MOBILE_TILE_BASED;
        assert!(energy_cost(1.0, true, &device) < energy_cost(1.0, false, &device));
    }

    #[test]
    fn a_small_speed_cost_for_a_big_energy_win_is_preferred_when_budget_allows() {
        let device = DeviceProfile::MOBILE_TILE_BASED;
        assert!(prefer_lower_energy(
            &device, false, 1.0, 10.0, 1.05, 2.0, 0.5
        ));
    }

    #[test]
    fn a_tight_budget_never_trades_speed_for_power() {
        let device = DeviceProfile::MOBILE_TILE_BASED;
        assert!(!prefer_lower_energy(
            &device, true, 1.0, 10.0, 1.05, 2.0, 0.5
        ));
    }

    #[test]
    fn a_plugged_in_device_never_trades_speed_for_power() {
        let device = DeviceProfile::DESKTOP_DISCRETE;
        assert!(!prefer_lower_energy(
            &device, false, 1.0, 10.0, 1.05, 2.0, 0.5
        ));
    }
}
