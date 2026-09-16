//! Cost annotations: the vocabulary [`crate::graph`] and `vieww-render-planner`
//! use to answer "what is the cheapest way to produce these pixels," instead
//! of "how do I execute these commands."
//!
//! This is deliberately data, not policy. A [`CostHint`] describes what a
//! node *is* — how often it changes, how well it caches, which execution
//! style suits it — and says nothing about which device is running it or
//! what that device costs per unit of work. `vieww-render-planner` owns that
//! second half; keeping the two separate is what lets the planner change its
//! mind about a device without every node in the tree needing new opinions.

use vieww_paint::Command;

/// How often a node's content actually changes frame to frame.
///
/// This is about the *content*, not the transform — a static icon sliding
/// across the screen is `Static` even though its screen position changes
/// every frame, because a GPU can move a cached texture far more cheaply
/// than a CPU can re-rasterize one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dynamicity {
    /// Painted once, then reused verbatim until something invalidates it
    /// (a static label, an icon, a background).
    Static,
    /// Changes on user action, not every frame (a hover state, a toggled
    /// checkbox).
    Occasional,
    /// Changes most frames while active (a running animation, a scroll
    /// position).
    Frequent,
    /// Changes every single frame by construction (a live camera texture, a
    /// video frame, a continuously-updating chart).
    Continuous,
}

/// How well a node's rasterized output can be reused across frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cacheability {
    /// Reuse buys nothing — the output is different every time it would be
    /// looked up (a continuously updating texture).
    None,
    /// Reuse is possible but the hit rate is expected to be low (content
    /// that changes with user input but is not continuous).
    Low,
    /// Reuse is expected to pay off often (glyph runs, icons, decorative
    /// chrome).
    High,
    /// The content is immutable for the life of the node (a static image,
    /// a baked gradient).
    VeryHigh,
}

/// Which execution style a node's *shape of work* suits, independent of
/// what hardware happens to be available.
///
/// This is a suitability signal, not a placement decision — a device with no
/// GPU still executes a `GpuOnly`-affinity node, just on the CPU fallback
/// path, and `vieww-render-planner` is where that trade is actually made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Affinity {
    /// Cheaper on a CPU scanline rasterizer than it would be to set up a GPU
    /// pass for (a single small fill, a short text run).
    Cpu,
    /// No strong preference either way.
    Balanced,
    /// Benefits substantially from GPU execution (many repeated instances,
    /// a large filled area, a filter chain) but has a reasonable CPU
    /// fallback.
    Gpu,
    /// Only sensible on a GPU (a compute-shader effect, a large blur over a
    /// live backdrop) — the CPU fallback exists but is expected to be far
    /// more expensive per pixel.
    GpuOnly,
}

/// What a node costs, and what kind of work it is — the unit
/// `vieww-render-graph` and `vieww-render-planner` reason about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostHint {
    pub dynamicity: Dynamicity,
    pub cacheability: Cacheability,
    pub gpu_affinity: Affinity,
    /// A rough estimate of the memory a cached copy of this node's output
    /// would cost, in bytes. Used by `vieww-render-graph`'s transient
    /// allocator and by planner-side memory budgeting; deliberately coarse
    /// (rounded up, never exact) since it drives admission decisions, not
    /// billing.
    pub estimated_bytes: u64,
}

impl CostHint {
    /// The cheapest, most conservative hint: assume nothing caches, assume
    /// nothing is free, assume CPU. Used as a fallback when a command shape
    /// isn't recognized rather than guessing optimistically.
    pub const CONSERVATIVE: Self = Self {
        dynamicity: Dynamicity::Frequent,
        cacheability: Cacheability::Low,
        gpu_affinity: Affinity::Balanced,
        estimated_bytes: 0,
    };

    /// Classify a single draw [`Command`] by its static shape.
    ///
    /// This is a heuristic, not a measurement — `vieww-render-planner`'s
    /// `profiling` module is where a *measured* cost eventually overrides
    /// this default (see that crate's docs). The heuristic exists so a
    /// scene has *some* cost signal before any frame has ever been
    /// profiled, which matters for the very first frame an app ever draws.
    #[must_use]
    pub fn classify_command(command: &Command) -> Self {
        match command {
            Command::FillRect { rect, .. } => Self {
                dynamicity: Dynamicity::Occasional,
                cacheability: Cacheability::High,
                gpu_affinity: Affinity::Balanced,
                estimated_bytes: area_bytes(rect.width(), rect.height()),
            },
            Command::FillPath { path, .. } | Command::StrokePath { path, .. } => {
                let verbs = path.verbs().len() as u64;
                Self {
                    dynamicity: Dynamicity::Occasional,
                    // A path with many verbs re-tessellates expensively on
                    // every touch, so its cache value climbs with
                    // complexity rather than staying flat.
                    cacheability: if verbs > 32 {
                        Cacheability::VeryHigh
                    } else {
                        Cacheability::High
                    },
                    gpu_affinity: if verbs > 64 {
                        Affinity::Gpu
                    } else {
                        Affinity::Cpu
                    },
                    estimated_bytes: verbs * 32,
                }
            }
            Command::DrawShadow { rect, radius, .. } => Self {
                dynamicity: Dynamicity::Occasional,
                cacheability: Cacheability::High,
                // A blur kernel is exactly the workload a GPU compute /
                // fragment pass amortizes across pixels; a CPU box-blur
                // pays per-pixel-per-tap with no parallelism.
                gpu_affinity: if *radius > 4.0 {
                    Affinity::GpuOnly
                } else {
                    Affinity::Gpu
                },
                estimated_bytes: area_bytes(
                    rect.width() + radius * 4.0,
                    rect.height() + radius * 4.0,
                ),
            },
            Command::DrawGlyphs { run, .. } => Self {
                // Text content itself rarely changes frame to frame even
                // inside an animation (the string is static; only its
                // transform moves), so glyph runs default to a high cache
                // rating keyed on shaped content, not on screen position.
                dynamicity: Dynamicity::Static,
                cacheability: Cacheability::VeryHigh,
                gpu_affinity: Affinity::Gpu,
                estimated_bytes: run.glyphs.len() as u64 * 48,
            },
            Command::DrawImage { rect, .. } => Self {
                dynamicity: Dynamicity::Static,
                cacheability: Cacheability::VeryHigh,
                gpu_affinity: Affinity::Gpu,
                estimated_bytes: area_bytes(rect.width(), rect.height()) * 4,
            },
            Command::PushLayer { filter, .. } => Self {
                dynamicity: Dynamicity::Frequent,
                cacheability: Cacheability::Low,
                gpu_affinity: if filter.is_noop() {
                    Affinity::Balanced
                } else {
                    // Any non-trivial `ImageFilter` (blur, color matrix,
                    // backdrop sampling) is the textbook GPU-shader
                    // workload.
                    Affinity::GpuOnly
                },
                estimated_bytes: 0,
            },
            Command::PopLayer => Self::CONSERVATIVE,
        }
    }

    /// Combine a parent's hint with a child's, for rolling per-node hints up
    /// into a subtree summary.
    ///
    /// Dynamicity and affinity take the "more demanding" side (a subtree
    /// with one continuously-updating child is a continuously-updating
    /// subtree); cacheability takes the weaker side (one uncacheable child
    /// makes caching the whole group pointless); bytes add.
    #[must_use]
    pub fn combine(self, other: Self) -> Self {
        Self {
            dynamicity: self.dynamicity.max(other.dynamicity),
            cacheability: self.cacheability.min(other.cacheability),
            gpu_affinity: self.gpu_affinity.max(other.gpu_affinity),
            estimated_bytes: self.estimated_bytes.saturating_add(other.estimated_bytes),
        }
    }
}

fn area_bytes(w: f32, h: f32) -> u64 {
    (w.max(0.0) * h.max(0.0)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Rect;
    use vieww_foundation::Transform;
    use vieww_paint::Paint;

    #[test]
    fn a_big_shadow_is_gpu_only() {
        let hint = CostHint::classify_command(&Command::DrawShadow {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            radius: 20.0,
            shadow: vieww_foundation::Shadow {
                color: vieww_foundation::Color::BLACK,
                offset: vieww_foundation::Offset::ZERO,
                blur: 8.0,
                spread: 0.0,
                is_inset: false,
            },
            transform: Transform::IDENTITY,
            clip: vieww_paint::Clip::NONE,
        });
        assert_eq!(hint.gpu_affinity, Affinity::GpuOnly);
    }

    #[test]
    fn a_plain_fill_is_balanced_and_cacheable() {
        let hint = CostHint::classify_command(&Command::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            paint: Paint::solid(vieww_foundation::Color::BLACK),
            transform: Transform::IDENTITY,
            clip: vieww_paint::Clip::NONE,
        });
        assert_eq!(hint.gpu_affinity, Affinity::Balanced);
        assert_eq!(hint.cacheability, Cacheability::High);
    }

    #[test]
    fn combining_takes_the_worse_of_each_axis() {
        let cheap = CostHint {
            dynamicity: Dynamicity::Static,
            cacheability: Cacheability::VeryHigh,
            gpu_affinity: Affinity::Cpu,
            estimated_bytes: 10,
        };
        let expensive = CostHint {
            dynamicity: Dynamicity::Continuous,
            cacheability: Cacheability::None,
            gpu_affinity: Affinity::GpuOnly,
            estimated_bytes: 90,
        };
        let combined = cheap.combine(expensive);
        assert_eq!(combined.dynamicity, Dynamicity::Continuous);
        assert_eq!(combined.cacheability, Cacheability::None);
        assert_eq!(combined.gpu_affinity, Affinity::GpuOnly);
        assert_eq!(combined.estimated_bytes, 100);
    }
}
