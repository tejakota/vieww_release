//! The D3D12 backend (spec §4.3, §11.3) — Windows, landing M8.
//!
//! # What is real here
//!
//! [`D3d12Device::new`] does a real adapter/device bring-up, calling APIs
//! verified against the actual `windows` 0.58 crate source (fetched into
//! this delivery's registry cache with `cargo fetch --target
//! x86_64-pc-windows-msvc`, then grepped for exact signatures — not
//! recalled from memory, the same discipline used for `src/metal.rs`):
//!
//! 1. `CreateDXGIFactory1::<IDXGIFactory1>()` — a real DXGI factory.
//! 2. `IDXGIFactory1::EnumAdapters1(index)` in a loop, skipping any adapter
//!    whose `DXGI_ADAPTER_DESC1::Flags` has `DXGI_ADAPTER_FLAG_SOFTWARE`
//!    set (the WARP software rasterizer DXGI always reports — a real HAL
//!    probe wants the actual GPU, not WARP, the same way
//!    [`super::vulkan::VulkanDevice::new`] wants a real ICD, not a
//!    hypothetical CPU one).
//! 3. `D3D12CreateDevice::<_, ID3D12Device>(&adapter, D3D_FEATURE_LEVEL_11_0,
//!    &mut device)` on the first non-software adapter that accepts it (spec
//!    §4.3's minimum feature level for this backend) — the actual point at
//!    which "is there a working D3D12 adapter on this machine" gets
//!    answered, same role `VulkanDevice::new`'s `vkCreateDevice` call plays.
//! 4. `ID3D12Device::CreateCommandQueue::<ID3D12CommandQueue>(&desc)` with
//!    `D3D12_COMMAND_LIST_TYPE_DIRECT` — one command queue, the same
//!    "device + one queue" scope [`super::metal::MetalDevice::new`] stops
//!    at, for the same reason: it is the largest slice of this backend that
//!    can be verified call-by-call against real source without a compiler
//!    on this OS to check it.
//!
//! # What is still not here, and exactly why
//!
//! `render_clear_to_pixels` is **not** ported in this delivery, for the same
//! two honest reasons `src/metal.rs` gives, which apply here without change:
//!
//! 1. **This sandbox cannot compile-check Windows code at all** — no
//!    `x86_64-pc-windows-msvc`/`-gnu` Rust target is installed (`rustup
//!    target add` fails: this sandbox's network egress is allowlisted to
//!    package registries, not `static.rust-lang.org`), so unlike the Vulkan
//!    backend, there is no way to catch a wrong `D3D12_RESOURCE_DESC` field
//!    or a malformed root signature blob before it reaches a Windows
//!    machine.
//! 2. **The render pipeline is considerably more intricate than Metal's or
//!    Vulkan's** — D3D12 additionally requires a root signature (serialized
//!    and deserialized through its own COM objects), an explicit resource
//!    barrier transition before every render-target use, and a fence-based
//!    wait that has no single "wait until GPU idle" convenience call the
//!    other two backends have. Writing that much unfamiliar, unverified
//!    code and presenting it as done would be exactly the "wrong-looking
//!    pseudocode" this workspace treats as worse than an honest stub — the
//!    same principle `vieww-text::shaping`'s real `HarfBuzzShaper`
//!    implementation was built under.
//!
//! What *is* verified, so the next step is scoped precisely: the real
//! `windows` 0.58 source confirms every call the Vulkan/Metal ports'
//! structure needs a D3D12 equivalent for exists —
//! `ID3D12Device::CreateCommittedResource`, `CreateGraphicsPipelineState`,
//! `CreateRootSignature`, `GetGraphicsRootSignature`-consuming command-list
//! methods (`ID3D12GraphicsCommandList::{IASetVertexBuffers,
//! DrawIndexedInstanced, ResourceBarrier}`), and `ID3D12CommandQueue::{
//! ExecuteCommandLists, Signal}` paired with `ID3D12Fence::SetEventOnCompletion`
//! for the CPU-side wait — so step 3 below is "port the existing, proven
//! Vulkan structure onto these," not "discover whether D3D12 can do this at
//! all."
//!
//! # To finish this on Windows
//!
//! 1. `cargo check -p vieww-hal --features d3d12` on Windows first — it does
//!    not compile-check anywhere else, so that machine is where this
//!    module's remaining API usage gets its first real feedback.
//! 2. `d3dcompiler`/DXIL: translate [`super::vulkan::MESH_SHADER_WGSL`]
//!    (or its equivalent) with `naga::back::hlsl::Writer`, then compile the
//!    generated HLSL to DXIL/DXBC with the `windows`-crate-exposed
//!    `D3DCompile` (or the `hassle-rs`/`dxc` toolchain, if SM6+ features are
//!    needed) — the shader source itself needs no changes, only the backend
//!    that compiles it, same as Metal's MSL step.
//! 3. Port `render_clear_to_pixels`/`render_mesh_to_pixels` onto the calls
//!    listed above: committed resource for the render target, a minimal
//!    root signature, one graphics pipeline state, a command list recording
//!    a resource-barrier + clear/draw + resource-barrier-back, submit via
//!    the queue this module already creates, fence-wait, then map a staging
//!    buffer to read pixels back.
//! 4. Run `cargo test -p vieww-hal --features d3d12` on Windows; there is no
//!    WARP-for-testing-a-real-adapter equivalent to lavapipe worth using
//!    here (WARP is deliberately excluded from adapter selection above,
//!    since the point is testing the real backend), so this is the first
//!    point this backend can be exercised at all — same as Metal.

use super::{sealed, AdapterInfo, Device as HalDevice};
use vieww_foundation::Color;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
use windows::Win32::Graphics::Direct3D12::{
    D3D12CreateDevice, ID3D12CommandQueue, ID3D12Device, D3D12_COMMAND_LIST_TYPE_DIRECT,
    D3D12_COMMAND_QUEUE_DESC, D3D12_COMMAND_QUEUE_FLAG_NONE,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};

#[derive(Debug)]
pub enum D3d12Error {
    /// Every adapter `IDXGIFactory1::EnumAdapters1` reported was either the
    /// WARP software adapter or refused `D3D12CreateDevice` at feature level
    /// 11.0 — no usable hardware D3D12 adapter is reachable on this machine.
    NoAdapter,
    /// A DXGI/D3D12 call returned a failing `HRESULT`. Carries the
    /// `windows_core::Error`'s own `Display` text (it already includes the
    /// `HRESULT` code), so nothing is lost by not matching on it here.
    Api(String),
    /// The render pipeline is not implemented yet — see this module's own
    /// docs for exactly what remains and why.
    NotImplemented(&'static str),
}

impl std::fmt::Display for D3d12Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no usable (non-WARP) D3D12 adapter found"),
            Self::Api(message) => write!(f, "D3D12/DXGI call failed: {message}"),
            Self::NotImplemented(what) => write!(f, "not yet implemented: {what}"),
        }
    }
}

impl std::error::Error for D3d12Error {}

impl From<windows::core::Error> for D3d12Error {
    fn from(error: windows::core::Error) -> Self {
        Self::Api(error.to_string())
    }
}

pub struct D3d12Device {
    _factory: IDXGIFactory1,
    device: ID3D12Device,
    _queue: ID3D12CommandQueue,
    description: String,
}

impl std::fmt::Debug for D3d12Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3d12Device")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

impl sealed::Sealed for D3d12Device {}

/// Decode a DXGI adapter description's fixed `[u16; 128]` UTF-16 buffer
/// (null-terminated, like every other Win32 wide string) into a `String`.
fn decode_description(buffer: [u16; 128]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

impl D3d12Device {
    /// Enumerate real DXGI adapters, skip WARP/software ones, and bring up
    /// the first one that accepts `D3D12CreateDevice` at feature level 11.0
    /// plus one direct command queue.
    ///
    /// Real, and the smallest slice of this backend that can be — every
    /// call here is verified to exist with this exact signature in
    /// `windows` 0.58's actual source (see this module's top doc), so the
    /// only remaining uncertainty is compiling and running it on actual
    /// Windows, not whether the API shape is right.
    pub fn new() -> Result<Self, D3d12Error> {
        // SAFETY: `CreateDXGIFactory1` is a documented, argument-free COM
        // factory function; the `windows` crate's generic wrapper handles
        // the `IID`/`QueryInterface` plumbing for the requested interface.
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };

        let mut index = 0u32;
        loop {
            // SAFETY: `EnumAdapters1` takes an index and an out-pointer the
            // `windows` wrapper owns; a failing `HRESULT` here (past the
            // last adapter, `DXGI_ERROR_NOT_FOUND`) just ends the loop.
            let adapter = match unsafe { factory.EnumAdapters1(index) } {
                Ok(adapter) => adapter,
                Err(_) => return Err(D3d12Error::NoAdapter),
            };
            index += 1;

            // SAFETY: `GetDesc1` fills a plain `#[repr(C)]` struct from a
            // valid adapter handle; no pointers escape.
            let desc = match unsafe { adapter.GetDesc1() } {
                Ok(desc) => desc,
                Err(_) => continue,
            };
            if (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                // WARP or another software adapter — not what a GPU-backend
                // probe should report as "found a GPU".
                continue;
            }

            let mut device: Option<ID3D12Device> = None;
            // SAFETY: `adapter` is a live, valid `IDXGIAdapter1` (which
            // satisfies `D3D12CreateDevice`'s `Param<IUnknown>` bound via
            // its COM interface hierarchy); `device` is a valid out-pointer
            // for the requested `ID3D12Device` interface.
            let created =
                unsafe { D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device) };
            let Some(device) = created.ok().and(device) else {
                // This adapter doesn't support D3D12 at the required
                // feature level — try the next one, exactly like
                // `VulkanDevice::new` would move past a Vulkan-incapable
                // adapter if it enumerated more than one.
                continue;
            };

            let queue_desc = D3D12_COMMAND_QUEUE_DESC {
                Type: D3D12_COMMAND_LIST_TYPE_DIRECT,
                Priority: 0,
                Flags: D3D12_COMMAND_QUEUE_FLAG_NONE,
                NodeMask: 0,
            };
            // SAFETY: `queue_desc` is a fully-initialized, valid
            // `D3D12_COMMAND_QUEUE_DESC`; `device` is the just-created,
            // live device it's a method on.
            let queue: ID3D12CommandQueue = unsafe { device.CreateCommandQueue(&queue_desc)? };

            return Ok(Self {
                _factory: factory,
                device,
                _queue: queue,
                description: decode_description(desc.Description),
            });
        }
    }
}

impl HalDevice for D3d12Device {
    type Error = D3d12Error;

    fn info(&self) -> AdapterInfo {
        // `ID3D12Device` has no direct "is this integrated or discrete"
        // query of its own (that lives on the DXGI adapter description,
        // which this struct doesn't retain past construction beyond its
        // name) — reported honestly as "unknown" rather than guessed.
        AdapterInfo {
            name: self.description.clone(),
            backend: "d3d12",
            device_type: "unknown".to_string(),
        }
    }

    fn render_clear_to_pixels(
        &self,
        _width: u32,
        _height: u32,
        _clear_color: Color,
    ) -> Result<Vec<u8>, D3d12Error> {
        // Touch `self.device` so it's plainly "used, not dead code" even
        // though this stub doesn't record onto it yet — see this module's
        // top doc, "What is still not here, and exactly why".
        let _ = &self.device;
        Err(D3d12Error::NotImplemented(
            "render pipeline: port vulkan::VulkanDevice's render_clear_to_pixels/render_mesh_to_pixels using naga::back::hlsl and a D3D12 root signature + resource barriers",
        ))
    }
}
