//! The HAL itself: the vocabulary `docs/RENDERER-V2-NOTES.md` names
//! directly — "keep the HAL brutally small (`Device`, `Queue`,
//! `CommandEncoder`, `Buffer`, `Texture`, `Sampler`, `Pipeline`, `Fence`,
//! `Surface` — not much past that) — the risk flagged is accidentally
//! rebuilding half of `wgpu`."
//!
//! Every type here is a trait, deliberately. A concrete backend
//! (`vieww-gpu-vulkan`, `vieww-gpu-metal`, `vieww-gpu-d3d12`, or the
//! in-process [`crate::null`] reference used for testing everything above
//! this crate without a GPU) implements the traits; nothing above this
//! crate ever names a backend type directly.

use vieww_foundation::{Rect, Size};

use crate::resources::{BufferUsage, TextureFormat, TextureUsage};

/// Opaque handle to a GPU buffer. Backends may wrap this in whatever real
/// handle type they need (a Vulkan `VkBuffer`, a Metal `MTLBuffer`, a D3D12
/// resource) — this crate never looks inside one.
pub trait Buffer: std::fmt::Debug {
    fn size(&self) -> u64;
    fn usage(&self) -> BufferUsage;
}

/// Opaque handle to a GPU texture.
pub trait Texture: std::fmt::Debug {
    fn size(&self) -> Size;
    fn format(&self) -> TextureFormat;
    fn usage(&self) -> TextureUsage;
}

/// A sampler: how a shader reads a [`Texture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    pub filter: FilterMode,
    pub address_mode: AddressMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterMode {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressMode {
    ClampToEdge,
    Repeat,
    MirrorRepeat,
}

/// A compiled graphics or compute pipeline. Backends attach whatever real
/// pipeline-state-object handle they have; the vocabulary a caller uses to
/// *ask* for one is `shader::PipelineDesc`.
pub trait Pipeline: std::fmt::Debug {}

/// A GPU-side wait/signal primitive: "has the work I submitted actually
/// finished." The one synchronization primitive this crate names — a real
/// backend's finer-grained semaphores/barriers live inside that backend,
/// driven by `vieww_render_graph`'s `Barrier` but expressed in whatever
/// vocabulary that API actually uses.
pub trait Fence: std::fmt::Debug {
    /// Block the calling thread until this fence signals, or `timeout_ms`
    /// elapses (`false` returned on timeout).
    fn wait(&self, timeout_ms: u64) -> bool;
    fn is_signaled(&self) -> bool;
}

/// Records GPU commands. A backend's real encoder wraps a command buffer;
/// this crate's vocabulary is the small set of operations every backend in
/// this workspace's target set (Vulkan, Metal, D3D12) can express
/// identically.
pub trait CommandEncoder {
    fn begin_pass(&mut self, target: &dyn Texture, clear: Option<[f32; 4]>);
    fn end_pass(&mut self);
    fn bind_pipeline(&mut self, pipeline: &dyn Pipeline);
    fn bind_vertex_buffer(&mut self, slot: u32, buffer: &dyn Buffer);
    fn bind_index_buffer(&mut self, buffer: &dyn Buffer);
    fn set_scissor(&mut self, rect: Rect);
    fn draw_indexed(&mut self, index_count: u32, instance_count: u32);
    /// A copy/blit — the CPU-scanline-to-GPU-presentation path this
    /// workspace ships today (`vieww_hal`'s Vulkan swapchain) is exactly
    /// one call to this with the whole surface as the region.
    fn blit(&mut self, src: &dyn Texture, dst: &dyn Texture);
}

/// A presentable surface: what a window's swapchain produces one
/// [`Texture`] from per frame.
pub trait Surface {
    fn size(&self) -> Size;
    /// Acquire the next presentable texture. `None` when the surface is
    /// temporarily unavailable (minimized window, device lost) — a
    /// backend's caller should skip the frame rather than treat this as
    /// fatal.
    fn acquire(&mut self) -> Option<Box<dyn Texture>>;
    fn present(&mut self);
}

/// A submission queue.
pub trait Queue {
    fn submit(&mut self, encoder: Box<dyn CommandEncoder>) -> Box<dyn Fence>;
}

/// The device itself: the factory for everything else in this module.
///
/// `probe()` on a concrete backend (not part of this trait, since probing
/// is backend-specific — see `vieww-gpu-vulkan::VulkanDevice::probe`) is
/// how a caller finds out whether a `Device` implementation is even
/// available on the running machine before calling any of these.
pub trait Device {
    fn create_buffer(&self, size: u64, usage: BufferUsage) -> Box<dyn Buffer>;
    fn create_texture(
        &self,
        size: Size,
        format: TextureFormat,
        usage: TextureUsage,
    ) -> Box<dyn Texture>;
    fn create_command_encoder(&self) -> Box<dyn CommandEncoder>;
    fn queue(&mut self) -> &mut dyn Queue;
    /// A human-readable description of the physical device — "NVIDIA
    /// RTX 4080", "Apple M3", "llvmpipe (LLVM 17.0.6, 256 bits)" — for
    /// devtools and diagnostics (`vieww doctor`, see `vieww-cli`).
    fn adapter_name(&self) -> String;
}
