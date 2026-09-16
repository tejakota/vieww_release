//! The hardware abstraction layer — spec §4 of `docs/RENDERER-SPEC.pdf`.
//!
//! A closed trait family implemented once per backend (Vulkan first, per the
//! landing order in spec §4.3), sealed so nothing outside this crate can
//! implement [`Device`] — spec §4.1: "The HAL is a private module; nothing
//! outside the crate can implement `Device`."
//!
//! # Why this is its own crate, separate from the rasterizer
//!
//! `vieww-paint::native` — vieww's ground-up CPU reference rasterizer, and
//! now the *only* rasterizer in this workspace — is the piece that needs
//! `vieww-paint`'s own `Scene`/`Command` as its input contract. This crate
//! needs none of that: `render_clear_to_pixels` takes a width, a height and
//! a [`Color`], nothing shaped like a display list. So it depends on
//! `vieww-foundation` alone, and stays reachable by anything that only wants
//! "is there a working Vulkan device on this machine" — which is exactly
//! what `vieww-hardware`'s `Capability::Gpu` probe uses it for, without
//! pulling in `vieww-paint`'s `Canvas`/`Scene`/`Damage`/`Layer`/`Scheduler`
//! machinery just to ask that question.
//!
//! # What actually ships in this build
//!
//! [`vulkan`] is real and tested — four suites, 20 tests, run against
//! `lavapipe` (Mesa's software Vulkan implementation, which is what stands in
//! for "a GPU" in a CI container with no display and no hardware adapter):
//!
//! * `vulkan_smoke` — instance/device bring-up, buffers, textures, a WGSL
//!   shader translated through `naga` to SPIR-V, one pipeline, and a headless
//!   render-to-texture-then-readback path.
//! * `vulkan_mesh_smoke` — caller-supplied geometry, drawn.
//! * `vulkan_scene` — a whole `vieww_gpu::ScenePlan` executed by
//!   [`vulkan::SceneRenderer`] and compared against `vieww_paint`'s CPU
//!   rasterizer.
//! * `vulkan_text` — the same, for **text**, through a glyph atlas, at a much
//!   tighter tolerance: every pixel within 1/255. See `vieww_gpu::atlas`.
//! * `vulkan_compositor` — every compositing feature against the CPU
//!   rasterizer over **every pixel**: group opacity, all 28 blend modes, layer
//!   blur and colour matrix, backdrop blur, shaped clips on draws and layers,
//!   outer/inset/rotated/scaled shadows, linear/radial/sweep gradients with
//!   dithering, magnified/rotated/minified images, nested mixed layers.
//!
//! All of them are `#[ignore]`d, because they need a loader and an ICD:
//! `cargo test -p vieww-hal --features vulkan -- --ignored --test-threads=1`.
//!
//! **`lavapipe` is software.** Nothing here is a performance claim.
//!
//! `vulkan::swapchain` is the piece that ships pixels to a screen today:
//! `vieww-platform-winit` opens one `VulkanDevice` per window and presents
//! `NativeRenderer`-rasterized, straight-alpha RGBA8 buffers onto it with
//! `vkCmdCopyBufferToImage`. The GPU *compositing* path —
//! [`vulkan::SceneRenderer`] executing a whole `vieww_gpu::ScenePlan`,
//! offscreen layers, blend modes, filters, masks and shadows included — is
//! complete as a **headless** renderer with readback. Promoting it to the live
//! window swapchain is a separate integration step and is not done: the
//! window path still presents the CPU surface.
//!
//! `metal` and `d3d12` are `cfg`-gated to their own operating systems and do
//! not compile — or appear in this doc build — as part of this Linux build at
//! all; see each module's own source comments for exactly what that means and
//! how to validate them on a machine that has the toolchain.

use vieww_foundation::Color;

/// Sealed: only this crate may implement [`Device`] (spec §4.1's "Sealed
/// traits"). A private supertrait is the standard pattern for that — nothing
/// outside `vieww-hal` can name `Sealed`, so nothing outside it can satisfy
/// `Device`'s bound.
mod sealed {
    pub trait Sealed {}
}

/// What this HAL requires of any GPU backend. See spec §4.2's sketch — this
/// is that trait, with the buffer/texture/write/present half spec §4.2 calls
/// "load-bearing" reduced, in this build, to what the smoke-test pipeline in
/// [`vulkan`] actually exercises: shader creation, one render pipeline, and a
/// present-free headless render.
pub trait Device: sealed::Sealed + Send + Sync {
    type Error: std::fmt::Debug + std::fmt::Display;

    /// Adapter/device identification — the same shape
    /// `vieww-hardware`'s capability probe already reads off `wgpu`'s
    /// `AdapterInfo` (spec §4.2), reproduced here so the probe's
    /// one-line-substitution path (spec §4.2's last paragraph) has
    /// something to substitute.
    fn info(&self) -> AdapterInfo;

    /// Render `width x height` of `clear_color`, with nothing else drawn —
    /// the M0/M1 smoke test (spec §14.1: "sustained 3-frame flight present;
    /// device-loss recovery test passes" starts from exactly this), and
    /// read the result back as straight-alpha RGBA8.
    fn render_clear_to_pixels(
        &self,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Result<Vec<u8>, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub name: String,
    pub backend: &'static str,
    pub device_type: String,
}

#[cfg(feature = "vulkan")]
pub mod vulkan;

/// The Metal backend (spec §4.3, landing M6) — macOS and iOS.
///
/// **Not compiled on Linux**, in two independent ways: this module is
/// `cfg`-gated to `target_os = "macos"` here, *and* the `metal` crate it
/// depends on is itself gated to `target_os = "macos"` in `Cargo.toml`
/// (behind the optional `metal` feature) — so a Linux build never resolves
/// the dependency, never sees this module, and is unaffected by either
/// existing. On macOS with `--features metal`: device/queue bring-up
/// (`MetalDevice::new`) is real, calling verified `metal` 0.31 APIs; the
/// full render pipeline is an honest, precisely-scoped stub — see
/// `src/metal.rs`'s own module doc for exactly what's real, what's deferred,
/// and why (this sandbox cannot compile-check macOS code at all, so writing
/// ~150 lines of unverifiable pipeline code would trade a real gap for a
/// fake-looking fill-in, which `docs/RENDERER-MIGRATION.md`'s own honesty
/// bar rules out).
#[cfg(all(target_os = "macos", feature = "metal"))]
pub mod metal;

/// The Direct3D 12 backend (spec §4.3, landing M8) — Windows.
///
/// **Not compiled on Linux or macOS**, for the same two-layer reason as
/// `metal`: gated here to `target_os = "windows"`, and the `windows` crate
/// it depends on is gated the same way in `Cargo.toml` behind the optional
/// `d3d12` feature. See `src/d3d12.rs`'s own module doc for what's real
/// (device/adapter enumeration, verified against the fetched `windows` 0.58
/// source) versus honestly deferred (the render pipeline, for the same
/// no-compile-check-here reason as Metal).
#[cfg(all(target_os = "windows", feature = "d3d12"))]
pub mod d3d12;
