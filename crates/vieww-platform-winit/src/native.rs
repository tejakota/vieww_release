//! Windowed presentation on `vieww`'s own stack, end to end: scenes are
//! rasterised on the CPU by `vieww_paint::native::NativeRenderer` (the
//! ground-up engine that replaced `vello_cpu`) and the resulting pixels are
//! presented through `vieww_hal::vulkan`'s real `VK_KHR_swapchain` — raw
//! Vulkan, no `wgpu`, no vello anywhere in this path — this is this crate's
//! one and only renderer.
//!
//! # What this is
//!
//! [`NativeRenderer::for_window`] opens a window's surface via
//! `vieww_hal::vulkan::VulkanDevice::for_window` and
//! [`NativeRenderer::present`] rasterises a [`vieww_paint::Scene`] to
//! straight-alpha RGBA8 pixels, then hands them to
//! `VulkanDevice::present_pixels`, which uploads them and copies them onto
//! the next swapchain image with `vkCmdCopyBufferToImage` — see
//! `vieww-hal`'s `vulkan::swapchain` module for the Vulkan half of this and
//! why it is correct rather than fast. **Nothing in this module, or anything
//! it calls, touches vello, `vello_cpu`, `vello_hybrid` or `wgpu`.**
//!
//! # What this is not, yet
//!
//! - **Damage-region rasterisation, whole-buffer upload.**
//!   [`present_damaged`](NativeRenderer::present_damaged) repaints only the
//!   regions a frame's `Damage` names, onto pixels the renderer retains
//!   between frames; [`present`](NativeRenderer::present) is the
//!   repaint-everything case a resize and a first frame want. What is *not*
//!   yet incremental is the upload: a swapchain hands out a different image
//!   each frame, so the whole buffer is still copied onto it. Narrowing that
//!   needs `VK_KHR_incremental_present` and a per-image record of what each
//!   already holds — see `present_damaged`'s own docs.
//! - **No multi-window device sharing.** Every
//!   [`NativeRenderer::for_window`] builds its own `VulkanDevice`; there is
//!   no shared-instance equivalent to what `vieww_paint::gpu`'s `GpuContext`
//!   used to give the vello path. One `vieww-hal` device per window is
//!   correct, just not the cheapest shape — a follow-up, not a defect.
//! - **No GPU-accelerated rasterisation.** `vieww-hal`'s Vulkan device does
//!   the *presentation* (upload + copy + present), not the drawing — the
//!   scene is still rasterised entirely on the CPU. Spec §14.1's M2 onward
//!   (tessellation and compositing on the GPU, underneath
//!   `vieww_paint::native::NativeRenderer`) is future work this module does
//!   not attempt.
//!
//! This module *is* `crate::app`'s one and only renderer — vello,
//! `vello_cpu`, `vello_hybrid` and `wgpu` are gone from this crate and from
//! `vieww-paint` entirely, not merely unused by default. See
//! `docs/RENDERER-MIGRATION.md` for the full account of the migration.
//!
//! # Verification
//!
//! This sandbox has a real (virtual, via `Xvfb`) X server, so this path is
//! proven the direct way: this crate's examples (`hello`, `gallery`, `grid`,
//! …) and `apps/viewwstudio` are run for real under it, through this exact
//! `App` → `crate::app` → [`NativeRenderer`] code path, and the live
//! window is screenshotted (ImageMagick's `import`) rather than checked
//! through an offscreen stand-in. `vieww-hal`'s own
//! `tests/vulkan_smoke.rs` separately proves the headless half (device,
//! pipeline, render-to-texture-then-readback) against `lavapipe`.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::Color;
use vieww_hal::vulkan::{VulkanDevice, VulkanError, VulkanSwapchain};
use vieww_paint::native::{NativeRenderer as CpuRenderer, RendererError, SceneReport};
use vieww_paint::{Damage, Scene};

/// Why a windowed present through [`NativeRenderer`] could not happen.
#[derive(Debug)]
pub enum NativeError {
    /// No Vulkan loader was found, or no physical device offers both a
    /// graphics queue and presentation support on this window's surface.
    /// The one variant worth matching on: `vieww-hardware`'s capability
    /// probe and `tests/wait_loop.rs` both recognise a headless runner this
    /// way, exactly as they used to recognise `vieww_paint::gpu::GpuError::NoAdapter`.
    NoAdapter,
    /// A Vulkan call other than adapter/queue selection failed while
    /// opening the device or surface.
    Device(String),
    /// The swapchain image could not be acquired or presented, even after
    /// one rebuild attempt. Ordinary during a resize race; not ordinary
    /// otherwise.
    Present(String),
    /// The CPU rasterizer itself failed — an unbalanced `PushLayer`/
    /// `PopLayer` pair in the scene, the one way
    /// [`vieww_paint::native::NativeRenderer`] can fail at all.
    Render(String),
}

impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => f.write_str("no Vulkan adapter available"),
            Self::Device(message) => write!(f, "opening a Vulkan device/surface: {message}"),
            Self::Present(message) => write!(f, "presenting a frame: {message}"),
            Self::Render(message) => write!(f, "rasterising a scene: {message}"),
        }
    }
}

impl std::error::Error for NativeError {}

impl From<RendererError> for NativeError {
    fn from(error: RendererError) -> Self {
        match error {
            RendererError::Render(message) => Self::Render(message),
        }
    }
}

impl From<VulkanError> for NativeError {
    fn from(error: VulkanError) -> Self {
        match error {
            VulkanError::NoAdapter => Self::NoAdapter,
            other => Self::Device(other.to_string()),
        }
    }
}

/// A window's swapchain. See [`vieww_hal::vulkan::VulkanSwapchain`] for the
/// Vulkan object this owns.
#[derive(Debug)]
pub struct NativeSurface {
    swapchain: VulkanSwapchain,
}

impl NativeSurface {
    /// Destroy the swapchain and its `VkSurfaceKHR` while `renderer`'s device
    /// and the window are both still alive. Must run before either is dropped;
    /// see `VulkanSwapchain::release` for the crash that ordering caused.
    /// Idempotent. The surface cannot present afterwards.
    pub fn release(&mut self, renderer: &NativeRenderer) {
        self.swapchain.release(&renderer.device);
    }
}

thread_local! {
    /// The one `VkInstance`/`VkDevice` this process opens, kept alive for as
    /// long as any window holds it.
    static SHARED_DEVICE: RefCell<Option<Rc<VulkanDevice>>> = const { RefCell::new(None) };
}

/// The shared device, and a swapchain for this window on it.
///
/// **One driver per process, not one per window.** Opening a `VkInstance` and
/// a `VkDevice` per window meant that closing any window destroyed a driver
/// out from under the windows that were still open, and the next window's
/// `vkDestroySwapchainKHR` crashed inside it — a null jump on NVIDIA, a double
/// free on Mesa's Intel driver, both reproduced by the desktop suite the
/// moment it closed a second window. See
/// [`VulkanDevice::swapchain_for_window`] for the measurements.
///
/// The event loop is single-threaded, so the cache is thread-local rather than
/// a lock, and it is only ever read on the thread that opens windows.
///
/// If the existing adapter cannot present to the new window — a second GPU
/// driving a second monitor is the case — this opens a device for that window
/// alone rather than failing, which is the old behaviour for the one situation
/// that needed it.
fn shared_device_for(
    window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
    width: u32,
    height: u32,
) -> Result<(Rc<VulkanDevice>, VulkanSwapchain), NativeError> {
    let existing = SHARED_DEVICE.with(|cell| cell.borrow().clone());
    if let Some(device) = existing {
        match device.swapchain_for_window(window, width, height) {
            Ok(swapchain) => return Ok((device, swapchain)),
            // Not this adapter's window. Fall through to a device of its own.
            Err(VulkanError::NoAdapter) => {}
            Err(error) => return Err(error.into()),
        }
        let (device, swapchain) = VulkanDevice::for_window(window, width, height)?;
        return Ok((Rc::new(device), swapchain));
    }

    let (device, swapchain) = VulkanDevice::for_window(window, width, height)?;
    let device = Rc::new(device);
    SHARED_DEVICE.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&device)));
    Ok((device, swapchain))
}

/// Presents [`vieww_paint::Scene`]s rasterised by `vieww-paint`'s `native`
/// backend, through `vieww-hal`'s Vulkan swapchain. See the module docs for
/// what this does and does not cover yet.
pub struct NativeRenderer {
    device: Rc<VulkanDevice>,
    cpu: CpuRenderer,
}

impl fmt::Debug for NativeRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRenderer")
            .field("cached_fonts", &self.cpu.cached_fonts())
            .finish_non_exhaustive()
    }
}

impl NativeRenderer {
    /// Open a window and a renderer to present into it.
    ///
    /// # Errors
    ///
    /// [`NativeError::Device`] if no Vulkan loader is present, or no adapter
    /// supports both a graphics queue and presenting to this window.
    ///
    /// # Panics
    ///
    /// If `width` or `height` is zero.
    pub fn for_window(
        window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
        width: u32,
        height: u32,
    ) -> Result<(Self, NativeSurface), NativeError> {
        Self::for_window_with(window, width, height, CpuRenderer::new())
    }

    /// The same, presenting through a rasterizer the caller configured.
    ///
    /// # Why this exists
    ///
    /// [`for_window`](Self::for_window) constructed `CpuRenderer::new()`, and
    /// that was the only rasterizer a windowed application could ever have. The
    /// consequence was not theoretical: `vieww_paint::native::NativeRenderer`
    /// offers `with_color_pipeline` — the linear-light compositing path, built,
    /// tested and documented at length in `native/linear.rs` as the
    /// colorimetrically correct way to blend a gradient or a translucent
    /// overlay — and `with_glyph_outline_budget_bytes`, for a
    /// memory-constrained target. **Neither could be reached from a real
    /// window.** They were configurable in a unit test and fixed in every
    /// application, which is a worse position than not having them.
    ///
    /// So the rasterizer is a parameter. `for_window` stays as the short form
    /// with the default, which is what almost every caller wants; this is the
    /// one that makes the crate's own options usable.
    ///
    /// # Errors
    ///
    /// As [`for_window`](Self::for_window).
    ///
    /// # Panics
    ///
    /// If `width` or `height` is zero.
    pub fn for_window_with(
        window: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
        width: u32,
        height: u32,
        cpu: CpuRenderer,
    ) -> Result<(Self, NativeSurface), NativeError> {
        let (device, swapchain) = shared_device_for(window, width, height)?;
        Ok((Self { device, cpu }, NativeSurface { swapchain }))
    }

    /// Cached glyph outlines — forwarded from the CPU rasterizer.
    #[must_use]
    pub fn cached_fonts(&self) -> usize {
        self.cpu.cached_fonts()
    }

    /// Rasterise `scene` on the CPU and present it to `surface`. Always a
    /// full repaint (see the module docs on why there is no damage path
    /// yet).
    ///
    /// `scene`'s commands are recorded in *logical* pixels, against
    /// `logical_size`; `surface` is measured in *physical* ones (see
    /// [`crate::Scale`]). At 1:1 — every display without HiDPI, and the case
    /// this sandbox's `Xvfb` runs in — the two match and this rasterises
    /// straight into the swapchain's own resolution. Off 1:1, this
    /// rasterises at `logical_size` and nearest-neighbour-upscales into the
    /// swapchain's physical resolution, rather than rasterising natively at
    /// physical resolution the way `vieww_paint::gpu::GpuRenderer::present_with`
    /// did by composing a `root` transform per damage region. That is a
    /// real, deliberate simplification — softer edges on a HiDPI display
    /// than native-resolution rasterisation would give — recorded here
    /// rather than silently accepted: composing a physical-space transform
    /// through `vieww_paint::native::NativeRenderer::apply` the way the
    /// vello backend did is the follow-up, tracked in
    /// `docs/RENDERER-MIGRATION.md`.
    ///
    /// # Errors
    ///
    /// [`NativeError::Render`] if rasterising failed, or
    /// [`NativeError::Present`] if the swapchain image could not be acquired
    /// or presented, even after one rebuild attempt.
    pub fn present(
        &mut self,
        surface: &mut NativeSurface,
        scene: &Scene,
        base: Color,
        logical_size: (u32, u32),
    ) -> Result<SceneReport, NativeError> {
        self.present_damaged(surface, scene, None, base, logical_size)
    }

    /// [`present`](Self::present), repainting only what `damage` says changed.
    ///
    /// # What this changes, and what it deliberately does not
    ///
    /// The **rasterisation** becomes incremental: the CPU renderer keeps the
    /// pixels it produced last frame and clears, redraws and converts only
    /// the damaged regions — see
    /// `vieww_paint::native::NativeRenderer::render_retained`, which also
    /// documents why the regions are a write mask rather than a smaller
    /// surface. Measured on the Studio shell at 1440x900: a full frame is
    /// 123 ms, the same frame through `render_damaged` (which culls commands
    /// but still rebuilds the whole framebuffer) is 13 ms, and through the
    /// retained path with nothing changed it is **0.5 ms**.
    ///
    /// The **upload** stays whole. A swapchain hands out a different image
    /// each frame, so the pixels this renderer kept are not the pixels
    /// already on the image being presented into; copying the full buffer is
    /// correct and costs a memcpy of a few megabytes. Narrowing that to the
    /// damaged rectangles needs `VK_KHR_incremental_present` and a per-image
    /// record of what each one already holds — a real follow-up, and a much
    /// smaller one than this, because the expensive half is now gone.
    ///
    /// Passing `None` repaints in full, which is what a resize and a first
    /// frame want.
    ///
    /// # Errors
    ///
    /// As [`present`](Self::present).
    pub fn present_damaged(
        &mut self,
        surface: &mut NativeSurface,
        scene: &Scene,
        damage: Option<&Damage>,
        base: Color,
        logical_size: (u32, u32),
    ) -> Result<SceneReport, NativeError> {
        let (physical_width, physical_height) = surface.swapchain.extent();
        let (logical_width, logical_height) = logical_size;
        // In place: the renderer's retained output is read directly, rather
        // than copied into a fresh `Pixels` every frame — that copy was a
        // full-frame allocation per present, which the Vieww standard's
        // steady-state clause counts.
        let report = match damage {
            Some(damage) => self.cpu.render_retained_in_place(
                scene,
                damage,
                logical_width,
                logical_height,
                base,
            )?,
            None => self
                .cpu
                .render_in_place(scene, logical_width, logical_height, base)?,
        };
        let frame = self.cpu.last_frame();

        let bytes = if (logical_width, logical_height) == (physical_width, physical_height) {
            std::borrow::Cow::Borrowed(frame)
        } else {
            std::borrow::Cow::Owned(nearest_upscale(
                frame,
                logical_width,
                logical_height,
                physical_width,
                physical_height,
            ))
        };

        self.device
            .present_pixels(&mut surface.swapchain, &bytes)
            .map_err(|error| NativeError::Present(error.to_string()))?;
        Ok(report)
    }

    /// Match the surface to a window that changed size. Contents are gone
    /// after this — the next [`present`](Self::present) repaints in full,
    /// which today it always does anyway (see the module docs).
    ///
    /// # Errors
    ///
    /// [`NativeError::Present`] if the swapchain could not be rebuilt at the
    /// new size (the surface was likely invalidated — e.g. the window
    /// closed mid-resize).
    pub fn resize(
        &self,
        surface: &mut NativeSurface,
        width: u32,
        height: u32,
    ) -> Result<(), NativeError> {
        if width == 0 || height == 0 {
            return Ok(());
        }
        surface
            .swapchain
            .recreate(&self.device, width, height)
            .map_err(|error| NativeError::Present(error.to_string()))
    }
}

/// Nearest-neighbour resize from a `src_width x src_height` straight-alpha
/// RGBA8 buffer to `dst_width x dst_height` — [`NativeRenderer::present`]'s
/// fallback for a HiDPI surface, where the
/// CPU rasteriser's output (logical resolution) does not already match the
/// swapchain's physical one. Nearest rather than bilinear: this only runs
/// off a scale factor the CPU rasteriser did not itself apply, and a cheap,
/// branch-free sampler is the honest choice until native-resolution
/// rasterisation (composing the physical transform through
/// `vieww_paint::native::NativeRenderer::apply`) replaces this path
/// entirely — see [`NativeRenderer::present`]'s own doc.
fn nearest_upscale(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    debug_assert_eq!(src.len(), (src_width as usize) * (src_height as usize) * 4);
    let mut out = vec![0u8; (dst_width as usize) * (dst_height as usize) * 4];
    for dy in 0..dst_height {
        let sy = (u64::from(dy) * u64::from(src_height) / u64::from(dst_height))
            .min(u64::from(src_height.saturating_sub(1))) as u32;
        for dx in 0..dst_width {
            let sx = (u64::from(dx) * u64::from(src_width) / u64::from(dst_width))
                .min(u64::from(src_width.saturating_sub(1))) as u32;
            let src_i = ((sy * src_width + sx) * 4) as usize;
            let dst_i = ((dy * dst_width + dx) * 4) as usize;
            out[dst_i..dst_i + 4].copy_from_slice(&src[src_i..src_i + 4]);
        }
    }
    out
}
