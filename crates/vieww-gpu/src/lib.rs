//! The GPU execution layer.
//!
//! This crate owns the backend-neutral half of Vieww's real GPU pipeline:
//! tessellation, batching, glyph/image residency, and scene planning. It is
//! deliberately independent of Vulkan/Metal/D3D12 so the same scene contract
//! feeds every backend.
//!
//! # What is real here and what needs a GPU
//!
//! Everything in this crate compiles and is unit-tested without a GPU —
//! [`null::NullDevice`] is a complete, working software implementation of
//! every trait in [`device`], which is what makes that possible.
//!
//! What this crate does **not** contain is a real GPU backend. That is
//! **`vieww-hal`**, whose `vulkan`, `metal` and `d3d12` modules sit behind
//! their own features and need, respectively, a Vulkan device and driver,
//! macOS on Apple hardware, and Windows with a Direct3D 12 driver to *run*.
//!
//! (An earlier version of this paragraph named three crates —
//! `vieww-gpu-vulkan`, `vieww-gpu-metal`, `vieww-gpu-d3d12` — that have never
//! existed in this workspace. The backends were always modules of one crate.
//! `TRACKER.md`'s "Naming note, not a gap" records the same class of drift for
//! `vieww-window`/`vieww-native`, and the resolution here is the same: correct
//! the doc, do not rename the code to match a doc nobody checked.)
//!
//! The Vulkan path executes [`scene::Planner`]'s output directly. A plan
//! covers every `vieww_paint::Command`: fills, strokes, transforms,
//! per-fragment linear/radial/sweep gradients (with dithering), rectangular
//! and shaped clips (antialiased masks), monochrome and colour text, images
//! (bilinear and mip-blended), offscreen layers with group opacity, all 28
//! blend modes, layer and backdrop filters (blur + colour matrix), and outer,
//! inset and transformed shadows. [`ScenePlan::unsupported`] now reports only
//! resource limits — see [`scene`]'s coverage contract.

pub mod atlas;
pub mod batch;
pub mod device;
pub mod generation;
pub mod image_atlas;
pub mod mask_atlas;
pub mod null;
pub mod ramp_atlas;
pub mod resources;
pub mod scene;
pub mod tessellate;

pub use atlas::{Atlas, AtlasKey, AtlasSlot};
pub use batch::{group_by_mesh, worth_batching, Batch, Instance};
pub use device::{
    AddressMode, Buffer, CommandEncoder, Device, Fence, FilterMode, Pipeline, Queue, SamplerDesc,
    Surface, Texture,
};
pub use image_atlas::{ImageAtlas, ImageKey, ImageSlot};
pub use mask_atlas::{MaskAtlas, MaskKey, MaskSlot};
pub use ramp_atlas::RampAtlas;
pub use resources::{
    texture_bytes, AllocationId, BufferUsage, Lifetime, PipelineCache, PipelineCacheKey,
    ReadbackQueue, ResidencyPriority, ResourceHeap, TextureFormat, TextureUsage, UploadQueue,
};
pub use scene::{
    material, plan as plan_scene, DrawRun, FilterOp, MaskRef, PixelRect, Planner, ScenePlan, Step,
    Unsupported, Vertex, MAX_TARGET_DEPTH,
};
pub use tessellate::{flatten_cubic, tessellate_fill, tessellate_stroke, Mesh};
