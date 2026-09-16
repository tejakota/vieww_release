//! Device capability and profile description.
//!
//! `docs/RENDERER-V2-NOTES.md`: "capability-driven rendering, richer than a
//! single 'GPU has feature X' flag ... so the renderer can ask 'what's the
//! cheapest implementation of X on this device,' not just 'can I do X.'"
//! [`DeviceProfile`] is that richer question's input.

/// The shape of GPU a device has, if any — different shapes have
/// different cost curves for the same workload (a tile-based mobile GPU
/// pays differently for overdraw than a discrete desktop GPU does).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuKind {
    /// No GPU available, or none this renderer can use.
    None,
    /// Shares memory bandwidth with the CPU (most mobile SoCs, integrated
    /// desktop graphics).
    Integrated,
    /// Dedicated VRAM and memory bus.
    Discrete,
    /// Tile-based deferred rendering — most mobile GPUs (Mali, Adreno,
    /// Apple GPUs). Cheap for on-chip blending, expensive for anything
    /// that forces a tile flush (a full-screen readback, most notably).
    TileBased,
}

impl GpuKind {
    #[must_use]
    pub const fn is_present(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What a device can do and roughly how fast — enough for
/// [`crate::cost`]'s formulas to produce a real number, not enough to
/// pretend this is a GPU driver's own capability query (a real backend
/// still asks the real API; this is what the *planner* reasons with before
/// any backend is chosen).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceProfile {
    pub gpu: GpuKind,
    /// CPU raster throughput, in megapixels of simple fill work per second.
    /// A rough, single-number stand-in for what a real profiler would
    /// eventually measure per device (`vieww-devtools`'s benchmark
    /// harness) — see [`crate::cost::CostModel`] for where a measured
    /// number supersedes this.
    pub cpu_fill_mpix_per_sec: f32,
    /// GPU raster throughput, same unit. Zero when `gpu` is
    /// [`GpuKind::None`].
    pub gpu_fill_mpix_per_sec: f32,
    /// Fixed overhead of dispatching *any* GPU work for one pass, in
    /// milliseconds — command buffer submission, pipeline bind, whatever a
    /// backend's driver charges no matter how small the pass is. This is
    /// what keeps a one-pixel fill on the CPU even on a device with a fast
    /// GPU.
    pub gpu_dispatch_overhead_ms: f32,
    /// How many CPU cores raster work can be spread across.
    pub cpu_cores: u32,
    pub supports_hdr: bool,
    pub supports_wide_gamut: bool,
    pub refresh_hz: f32,
    pub is_battery_powered: bool,
    /// Relative power draw of one second of GPU work versus one second of
    /// CPU work, on this device (>1 means the GPU costs more energy per
    /// second of work — true for a discrete desktop GPU, often false for
    /// a mobile SoC's fixed-function GPU blocks doing what they're
    /// good at). Used by [`crate::power`].
    pub gpu_relative_power_cost: f32,
}

impl DeviceProfile {
    /// A conservative CPU-only profile: no GPU, modest single-core
    /// throughput. What a planner should assume before any real probing
    /// has happened, and what every device with a probe failure falls
    /// back to.
    pub const CPU_ONLY_FALLBACK: Self = Self {
        gpu: GpuKind::None,
        cpu_fill_mpix_per_sec: 40.0,
        gpu_fill_mpix_per_sec: 0.0,
        gpu_dispatch_overhead_ms: 0.0,
        cpu_cores: 1,
        supports_hdr: false,
        supports_wide_gamut: false,
        refresh_hz: 60.0,
        is_battery_powered: true,
        gpu_relative_power_cost: 1.0,
    };

    /// A representative high-end discrete-GPU desktop.
    pub const DESKTOP_DISCRETE: Self = Self {
        gpu: GpuKind::Discrete,
        cpu_fill_mpix_per_sec: 250.0,
        gpu_fill_mpix_per_sec: 8000.0,
        gpu_dispatch_overhead_ms: 0.05,
        cpu_cores: 8,
        supports_hdr: true,
        supports_wide_gamut: true,
        refresh_hz: 144.0,
        is_battery_powered: false,
        gpu_relative_power_cost: 3.0,
    };

    /// A representative mid-range phone with a tile-based mobile GPU.
    pub const MOBILE_TILE_BASED: Self = Self {
        gpu: GpuKind::TileBased,
        cpu_fill_mpix_per_sec: 60.0,
        gpu_fill_mpix_per_sec: 1200.0,
        gpu_dispatch_overhead_ms: 0.3,
        cpu_cores: 4,
        supports_hdr: true,
        supports_wide_gamut: true,
        refresh_hz: 120.0,
        is_battery_powered: true,
        gpu_relative_power_cost: 0.6,
    };

    #[must_use]
    pub const fn has_gpu(&self) -> bool {
        self.gpu.is_present()
    }

    /// Frame deadline in milliseconds implied by `refresh_hz`.
    #[must_use]
    pub fn frame_deadline_ms(&self) -> f32 {
        1000.0 / self.refresh_hz.max(1.0)
    }
}
