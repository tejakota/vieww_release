//! vieww's own ground-up rasterizer — this crate's one and only renderer.
//! Requires the `native` feature.
//!
//! Built to `docs/RENDERER-SPEC.pdf`'s engineering specification: a
//! retained-tessellation rendering engine for this crate's
//! own [`Scene`], with a CPU reference oracle and — in the separate
//! `vieww-hal` crate — a Vulkan/Metal/D3D12 hardware abstraction layer,
//! "engineered to replace vello end to end without breaking the Scene seam"
//! (spec, cover). vello, `vello_cpu` and `vello_hybrid` — and the `gpu`,
//! `hybrid` and `cpu` modules that wrapped them — are gone from this crate
//! entirely; see `docs/RENDERER-MIGRATION.md` for the full migration
//! history, what shipped in this build versus what the spec schedules
//! across its 26-month solo timeline, and the honest accounting of the
//! difference — including why this module lives inside `vieww-paint`
//! rather than in its own crate (a real Cargo package-graph cycle: this
//! renderer's whole design point is consuming this crate's own
//! `Scene`/`Command` directly, which makes a separate crate's dependency
//! edge back to `vieww-paint` unavoidable, and Cargo forbids that cycle
//! even feature-gated — so the code moved in here instead of staying
//! beside a thin, permanently-glue-only sibling crate).
//!
//! # What's real in this build
//!
//! - `NativeRenderer` — the CPU reference rasterizer (spec §12.1's "oracle",
//!   and now the shipped renderer, not merely one): scanline nonzero-winding
//!   fill with 4x4 supersampling, adaptive cubic flattening, full stroke
//!   expansion (caps/joins/miter-limit/dashes), linear/radial/sweep
//!   gradients, glyph outlines via `ttf-parser`, image sampling, box-blurred
//!   rounded-rect shadows (inset and outer), the 5x4 colour matrix, and
//!   **all 28 blend modes** — including the six vello used to substitute
//!   with `Normal` (spec §7.2) — composited through a real isolated-group
//!   buffer per [`crate::Command::PushLayer`], so they are confined to
//!   their own layer instead of leaking across the whole target.
//!   `vieww-paint`'s `tests/native_parity.rs` is this renderer's own
//!   self-consistency suite now that there is no second CPU backend to
//!   diff against — see that file's own docs.
//! - `vieww-hal`'s Vulkan backend — a real, tested (headless via `lavapipe`,
//!   windowed via a real `Xvfb` X server) Vulkan 1.2 device: a WGSL shader
//!   translated through `naga` to SPIR-V, headless render-to-texture-then-
//!   readback, **and a real `VK_KHR_swapchain`** (`vulkan::swapchain`,
//!   requires that crate's `vulkan-swapchain` feature) that
//!   `vieww-platform-winit`'s own `native` module presents this renderer's
//!   output through. Spec §14.1's M0/M1 slice, swapchain included.
//!   `vieww-hal`'s device does the *presenting*, not the *drawing* — the
//!   scene is still rasterised entirely on the CPU by `NativeRenderer`;
//!   wiring GPU-accelerated tessellation underneath it is spec §14.1's M2
//!   onward.
//!
//! # What is scaffolded, not shipped
//!
//! The GPU tessellation/glyph/compositing pipeline (spec chapters 5-9 on the
//! GPU side), the Metal and D3D12 backends, and the mesh/atlas caches are
//! out of scope for a one-session build — see `docs/RENDERER-MIGRATION.md`
//! for the itemized list and the milestone each maps to. Nothing here claims
//! more than it does: `vieww_hal::Device` is frozen and proven on one
//! backend, ready to be ported rather than redesigned. Damage-region
//! rendering and multi-window device sharing are also not yet ported to
//! this renderer's windowed path — see `vieww-platform-winit`'s `native`
//! module docs.

/// An independent correctness oracle for `color.rs`'s blending — "Renderer
/// v2" pillar F. Test-only: see this module's own docs.
#[cfg(test)]
mod blend_oracle;
mod clip;
mod color;
/// COLRv0 layered glyphs and CBDT/sbix bitmap emoji — the colour answer the
/// monochrome path cannot give. See the module's own docs for the two
/// flavours and the cache-shape.
mod color_glyphs;
mod effects;
mod geometry;
mod glyph;
/// The seam a GPU backend builds its glyph atlas through — see the module doc
/// for why it reaches into *this* rasterizer rather than bringing its own.
mod glyph_coverage;
mod glyph_raster;
/// Clip, shadow, gradient, colour-glyph and mip facts for GPU backends — the
/// same seam `glyph_coverage` is for text. See the module doc.
mod gpu_seam;
mod gradient;
mod image;
/// Rasterize-once, translate-many-times shape reuse — "Renderer v2" pillar D.
mod instancing;
/// An opt-in linear-light compositing pipeline — "Renderer v2" pillar B.
mod linear;
mod pixels;
mod pool;
mod reference;
mod residency;
mod rounded_rect;
mod shadow;
mod target;

pub use glyph_coverage::{GlyphAlpha, GlyphCoverage, InkedGlyph};
pub use gpu_seam::{
    box_radius_for_sigma, dashed_path, gradient_ramp, minification_ratio, ColorGlyphParts,
    CoveragePatch, GpuSeam, ShadowPatch, BAYER4,
};
pub use linear::ColorPipeline;
pub use pixels::Pixels;
pub use pool::PoolStats;
pub use reference::{AaMode, GlyphRasterStats, NativeRenderer, RendererError, SceneReport};
pub use residency::ResidencyStats;
