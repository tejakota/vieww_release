//! Executing a [`vieww_gpu::ScenePlan`] on a real Vulkan device.
//!
//! # Shape
//!
//! [`SceneRenderer`] is built once per surface and keeps everything that does
//! not change between frames: two render passes, three graphics pipelines,
//! four texture resources and the descriptor sets that bind them. Per frame
//! it uploads whichever atlases changed, writes the vertex and index buffers,
//! records **one** command buffer for the plan's whole [`Step`](vieww_gpu::Step) program, and
//! submits it once.
//!
//! ```text
//!   targets  (RGBA16F, premultiplied, frame-sized)
//!     L[0]  the frame            S[0]  its scratch
//!     L[1]  first layer / work   S[1]  …
//!     …     up to plan.target_depth
//!
//!   pipelines
//!     scene         batched geometry, source-over     (Step::Draw)
//!     post_replace  full-screen, no blending          copy · blur · matrix · mask · blend
//!     post_over     full-screen, source-over          normal composite · shadow tint
//! ```
//!
//! # Why every target is RGBA16F and premultiplied
//!
//! `NativeRenderer` — the oracle this backend is tested against — composites
//! premultiplied `f32` pixels. A layer that is later scaled by group opacity,
//! blurred or pushed through `ColorBurn` must hold the same numbers the CPU
//! layer buffer would, and an 8-bit premultiplied buffer loses exactly the
//! low-alpha precision those operations amplify. RGBA16F is the smallest
//! format Vulkan *requires* to be renderable and blendable that does not.
//! The frame is quantised to 8 bits once, on readback, with the same rounding
//! `NativeRenderer` uses.
//!
//! # Refusal
//!
//! [`SceneRenderer::render`] refuses a plan that is not
//! [`ScenePlan::is_complete`](vieww_gpu::ScenePlan::is_complete). A plan's remaining gaps are resource limits
//! (see `vieww_gpu::scene`); a frame with a missing photograph is a wrong
//! frame, not a degraded one. [`SceneRenderer::render_incomplete`] exists for
//! tools that want the partial result on purpose.

use std::collections::HashMap;
use std::ffi::CString;

use ash::vk;
use vieww_foundation::{BlendMode, Color};
use vieww_gpu::{FilterOp, MaskRef, PixelRect, Planner, ScenePlan, Step, Vertex};

use super::{translate_wgsl_to_spirv, VulkanDevice, VulkanError};

/// Colour format of every render target.
const TARGET_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// A persistent Vulkan executor for [`ScenePlan`]s. See the module doc.
pub struct SceneRenderer {
    /// A clone of the device handle (function table + `VkDevice`). The
    /// [`VulkanDevice`] this was built from owns the device and must outlive
    /// the renderer.
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    host_visible_memory: u32,
    mem_props: vk::PhysicalDeviceMemoryProperties,

    rp_load: vk::RenderPass,
    rp_clear: vk::RenderPass,

    scene_pipeline: vk::Pipeline,
    scene_layout: vk::PipelineLayout,
    scene_set_layout: vk::DescriptorSetLayout,
    scene_pool: vk::DescriptorPool,
    scene_set: vk::DescriptorSet,

    post_replace: vk::Pipeline,
    post_over: vk::Pipeline,
    post_layout: vk::PipelineLayout,
    post_set_layout: vk::DescriptorSetLayout,
    post_pool: vk::DescriptorPool,
    post_sets: HashMap<(TargetId, TargetId), vk::DescriptorSet>,

    shaders: Vec<vk::ShaderModule>,
    uniform: Buffer,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,

    glyphs: Texture,
    images: Texture,
    masks: Texture,
    ramps: Texture,
    /// 1x1 transparent RGBA16F, bound wherever a post pass reads nothing.
    dummy: Target,

    size: (u32, u32),
    levels: Vec<Target>,
    scratch: Vec<Target>,
    readback: Buffer,

    vertices: Buffer,
    indices: Buffer,
}

/// Which image a post descriptor set binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TargetId {
    Level(usize),
    Scratch(usize),
    Dummy,
}

struct Buffer {
    handle: vk::Buffer,
    memory: vk::DeviceMemory,
    capacity: u64,
}

impl Buffer {
    const EMPTY: Self = Self {
        handle: vk::Buffer::null(),
        memory: vk::DeviceMemory::null(),
        capacity: 0,
    };
}

/// A render target: image, memory, view and a framebuffer for it.
struct Target {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    framebuffer: vk::Framebuffer,
}

impl Target {
    const NULL: Self = Self {
        image: vk::Image::null(),
        memory: vk::DeviceMemory::null(),
        view: vk::ImageView::null(),
        framebuffer: vk::Framebuffer::null(),
    };
}

/// A sampled texture fed from a CPU atlas.
struct Texture {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    format: vk::Format,
    width: u32,
    height: u32,
    /// The atlas version last uploaded; `None` for the initial placeholder.
    version: Option<u64>,
    staging: Buffer,
}

impl Texture {
    const NULL: Self = Self {
        image: vk::Image::null(),
        memory: vk::DeviceMemory::null(),
        view: vk::ImageView::null(),
        format: vk::Format::UNDEFINED,
        width: 0,
        height: 0,
        version: None,
        staging: Buffer::EMPTY,
    };
}

/// Which texture slot, for uploads and descriptor rebinding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Glyphs,
    Images,
    Masks,
    Ramps,
}

impl Slot {
    const ALL: [Self; 4] = [Self::Glyphs, Self::Images, Self::Masks, Self::Ramps];

    const fn binding(self) -> u32 {
        match self {
            Self::Glyphs => 1,
            Self::Images => 2,
            Self::Masks => 3,
            Self::Ramps => 4,
        }
    }
}

/// Push constants for `post.wgsl`: eight `vec4<f32>`, 128 bytes — the minimum
/// push-constant size every Vulkan implementation guarantees.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct PostPush {
    region: [f32; 4],
    args: [f32; 4],
    p: [[f32; 4]; 6],
}

/// `post.wgsl`'s mode table.
mod mode {
    pub(super) const COPY: f32 = 0.0;
    pub(super) const BOX_H: f32 = 1.0;
    pub(super) const BOX_V: f32 = 2.0;
    pub(super) const MATRIX: f32 = 3.0;
    pub(super) const MASK: f32 = 4.0;
    pub(super) const BLEND: f32 = 5.0;
    pub(super) const OVER: f32 = 6.0;
    pub(super) const SEED: f32 = 7.0;
    pub(super) const INSET: f32 = 8.0;
    pub(super) const TINT: f32 = 9.0;
}

/// `post.wgsl`'s blend-mode numbering: `BlendMode`'s declaration order.
const fn blend_index(mode: BlendMode) -> f32 {
    use BlendMode::*;
    match mode {
        Normal => 0.0,
        Clear => 1.0,
        Src => 2.0,
        Dst => 3.0,
        DstOver => 4.0,
        SrcIn => 5.0,
        DstIn => 6.0,
        SrcOut => 7.0,
        DstOut => 8.0,
        SrcAtop => 9.0,
        DstAtop => 10.0,
        Xor => 11.0,
        Plus => 12.0,
        Multiply => 13.0,
        Screen => 14.0,
        Overlay => 15.0,
        Darken => 16.0,
        Lighten => 17.0,
        ColorDodge => 18.0,
        ColorBurn => 19.0,
        HardLight => 20.0,
        SoftLight => 21.0,
        Difference => 22.0,
        Exclusion => 23.0,
        Hue => 24.0,
        Saturation => 25.0,
        Color => 26.0,
        Luminosity => 27.0,
    }
}

#[allow(clippy::cast_precision_loss)]
fn region(rect: PixelRect) -> [f32; 4] {
    [
        rect.x0 as f32,
        rect.y0 as f32,
        rect.x1 as f32,
        rect.y1 as f32,
    ]
}

/// One post pass, described.
struct Pass {
    into: TargetId,
    /// Where it writes.
    scissor: PixelRect,
    /// Where `src` is readable; transparent outside.
    read: PixelRect,
    src: TargetId,
    aux: TargetId,
    over: bool,
    push: PostPush,
}

impl Pass {
    fn new(into: TargetId, rect: PixelRect, src: TargetId, mode: f32) -> Self {
        let mut push = PostPush::default();
        push.args[0] = mode;
        Self {
            into,
            scissor: rect,
            read: rect,
            src,
            aux: TargetId::Dummy,
            over: false,
            push,
        }
    }
}

impl SceneRenderer {
    /// Build the render passes, pipelines and placeholder textures.
    ///
    /// # Errors
    ///
    /// Any Vulkan object failing to be created, or a shader failing to
    /// translate (a bug in this workspace, surfaced here).
    pub fn new(device: &VulkanDevice) -> Result<Self, VulkanError> {
        let handle = device.raw_device().clone();
        let host_visible = device.find_memory_type(
            u32::MAX,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let mem_props = device.memory_properties();

        let rp_load = create_render_pass(&handle, vk::AttachmentLoadOp::LOAD)?;
        let rp_clear = create_render_pass(&handle, vk::AttachmentLoadOp::CLEAR)?;

        // ── descriptor layouts ──────────────────────────────────────────────
        let scene_bindings: Vec<_> = (0..5u32)
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(if binding == 0 {
                        vk::DescriptorType::UNIFORM_BUFFER
                    } else {
                        vk::DescriptorType::SAMPLED_IMAGE
                    })
                    .descriptor_count(1)
                    .stage_flags(if binding == 0 {
                        vk::ShaderStageFlags::VERTEX
                    } else {
                        vk::ShaderStageFlags::FRAGMENT
                    })
            })
            .collect();
        let scene_set_layout = unsafe {
            handle.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&scene_bindings),
                None,
            )
        }?;
        let post_bindings: Vec<_> = (0..3u32)
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            })
            .collect();
        let post_set_layout = unsafe {
            handle.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&post_bindings),
                None,
            )
        }?;

        let scene_pool = unsafe {
            handle.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .pool_sizes(&[
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::UNIFORM_BUFFER,
                            descriptor_count: 1,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLED_IMAGE,
                            descriptor_count: 4,
                        },
                    ])
                    .max_sets(1),
                None,
            )
        }?;
        let scene_set = unsafe {
            handle.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(scene_pool)
                    .set_layouts(&[scene_set_layout]),
            )
        }?[0];
        let post_pool = create_post_pool(&handle)?;

        // ── shaders and pipelines ───────────────────────────────────────────
        let sources = [
            (
                vieww_shaders::library::SCENE_WGSL,
                naga::ShaderStage::Vertex,
                "vs_main",
            ),
            (
                vieww_shaders::library::SCENE_WGSL,
                naga::ShaderStage::Fragment,
                "fs_main",
            ),
            (
                vieww_shaders::library::POST_WGSL,
                naga::ShaderStage::Vertex,
                "vs_main",
            ),
            (
                vieww_shaders::library::POST_WGSL,
                naga::ShaderStage::Fragment,
                "fs_main",
            ),
        ];
        let mut shaders = Vec::new();
        for (source, stage, entry) in sources {
            let code = translate_wgsl_to_spirv(source, entry, stage)?;
            shaders.push(unsafe {
                handle
                    .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            }?);
        }

        let scene_layout = unsafe {
            handle.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&[scene_set_layout]),
                None,
            )
        }?;
        let push_ranges = [vk::PushConstantRange {
            stage_flags: vk::ShaderStageFlags::FRAGMENT,
            offset: 0,
            size: std::mem::size_of::<PostPush>() as u32,
        }];
        let post_layout = unsafe {
            handle.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&[post_set_layout])
                    .push_constant_ranges(&push_ranges),
                None,
            )
        }?;

        let f = std::mem::size_of::<f32>() as u32;
        let attribute =
            |location: u32, components: u32, offset: u32| vk::VertexInputAttributeDescription {
                location,
                binding: 0,
                format: match components {
                    1 => vk::Format::R32_SFLOAT,
                    2 => vk::Format::R32G32_SFLOAT,
                    _ => vk::Format::R32G32B32A32_SFLOAT,
                },
                offset: offset * f,
            };
        // `vieww_gpu::Vertex`, field by field: 2 + 4 + 2 + 1 + 4 + 2 + 4 + 4.
        let scene_attributes = [
            attribute(0, 2, 0),
            attribute(1, 4, 2),
            attribute(2, 2, 6),
            attribute(3, 1, 8),
            attribute(4, 4, 9),
            attribute(5, 2, 13),
            attribute(6, 4, 15),
            attribute(7, 4, 19),
        ];
        assert_eq!(
            std::mem::size_of::<Vertex>() as u32,
            23 * f,
            "vertex layout drifted"
        );
        let scene_binding = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: std::mem::size_of::<Vertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];

        let scene_pipeline = create_pipeline(
            &handle,
            rp_load,
            scene_layout,
            (shaders[0], shaders[1]),
            &scene_binding,
            &scene_attributes,
            true,
        )?;
        let post_replace = create_pipeline(
            &handle,
            rp_load,
            post_layout,
            (shaders[2], shaders[3]),
            &[],
            &[],
            false,
        )?;
        let post_over = create_pipeline(
            &handle,
            rp_load,
            post_layout,
            (shaders[2], shaders[3]),
            &[],
            &[],
            true,
        )?;

        let command_pool = unsafe {
            handle.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(device.queue_family_index())
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
        }?;
        let command_buffer = unsafe {
            handle.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }?[0];
        let fence = unsafe { handle.create_fence(&vk::FenceCreateInfo::default(), None) }?;

        let uniform = allocate_buffer(
            &handle,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            host_visible,
            std::mem::size_of::<[f32; 4]>() as u64,
        )?;
        let glyphs = create_texture(&handle, &mem_props, vk::Format::R8_UNORM, 1, 1)?;
        let images = create_texture(&handle, &mem_props, vk::Format::R8G8B8A8_UNORM, 1, 1)?;
        let masks = create_texture(&handle, &mem_props, vk::Format::R8_UNORM, 1, 1)?;
        let ramps = create_texture(&handle, &mem_props, vk::Format::R32G32B32A32_SFLOAT, 1, 1)?;
        let dummy = create_target(&handle, &mem_props, rp_load, 1, 1)?;

        let mut renderer = Self {
            device: handle,
            queue: device.raw_queue(),
            queue_family: device.queue_family_index(),
            host_visible_memory: host_visible,
            mem_props,
            rp_load,
            rp_clear,
            scene_pipeline,
            scene_layout,
            scene_set_layout,
            scene_pool,
            scene_set,
            post_replace,
            post_over,
            post_layout,
            post_set_layout,
            post_pool,
            post_sets: HashMap::new(),
            shaders,
            uniform,
            command_pool,
            command_buffer,
            fence,
            glyphs,
            images,
            masks,
            ramps,
            dummy,
            size: (0, 0),
            levels: Vec::new(),
            scratch: Vec::new(),
            readback: Buffer::EMPTY,
            vertices: Buffer::EMPTY,
            indices: Buffer::EMPTY,
        };

        let uniform_info = [vk::DescriptorBufferInfo {
            buffer: renderer.uniform.handle,
            offset: 0,
            range: renderer.uniform.capacity,
        }];
        unsafe {
            renderer.device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(renderer.scene_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&uniform_info)],
                &[],
            );
        }
        // A new image's contents are undefined; every placeholder is written
        // (zeros) and bound before anything can read it.
        for slot in Slot::ALL {
            let bytes = vec![0u8; bytes_per_texel(renderer.texture(slot).format)];
            renderer.upload(slot, &bytes, 1, 1, None, true)?;
        }
        renderer.initialise_targets(&[TargetId::Dummy])?;
        Ok(renderer)
    }

    /// The queue family this renderer records for.
    #[must_use]
    pub const fn queue_family(&self) -> u32 {
        self.queue_family
    }

    /// Upload the glyph atlas if it changed. For callers that manage a
    /// `vieww_gpu::Atlas` outside a [`Planner`].
    ///
    /// # Errors
    ///
    /// Any Vulkan call failing.
    pub fn upload_atlas(&mut self, atlas: &vieww_gpu::Atlas) -> Result<(), VulkanError> {
        self.upload(
            Slot::Glyphs,
            atlas.texels(),
            atlas.side(),
            atlas.side(),
            Some(atlas.version()),
            false,
        )
    }

    /// Upload every atlas `planner` holds that changed since the last upload.
    ///
    /// # Errors
    ///
    /// Any Vulkan call failing.
    pub fn upload_planner(&mut self, planner: &Planner) -> Result<(), VulkanError> {
        self.upload_atlas(planner.atlas())?;
        let images = planner.image_atlas();
        self.upload(
            Slot::Images,
            images.texels(),
            images.side(),
            images.side(),
            Some(images.version()),
            false,
        )?;
        let masks = planner.mask_atlas();
        self.upload(
            Slot::Masks,
            masks.texels(),
            masks.side(),
            masks.side(),
            Some(masks.version()),
            false,
        )?;
        let ramps = planner.ramp_atlas();
        self.upload(
            Slot::Ramps,
            as_bytes(ramps.texels()),
            vieww_gpu::RampAtlas::WIDTH,
            ramps.rows(),
            Some(ramps.version()),
            false,
        )
    }

    /// [`render`](Self::render), with `planner`'s atlases uploaded first — the
    /// pairing that keeps a plan's texel offsets and the textures they
    /// address in step.
    ///
    /// # Errors
    ///
    /// As [`render`](Self::render), plus any upload failing.
    pub fn render_planned(
        &mut self,
        planner: &Planner,
        plan: &ScenePlan,
        width: u32,
        height: u32,
        clear: Color,
    ) -> Result<Vec<u8>, VulkanError> {
        self.upload_planner(planner)?;
        self.render(plan, width, height, clear)
    }

    /// Execute `plan` at `width` x `height` over `clear` and read the frame
    /// back as straight-alpha RGBA8.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Vulkan`] carrying `"incomplete plan"` when the plan has
    /// gaps, and any Vulkan call failing.
    pub fn render(
        &mut self,
        plan: &ScenePlan,
        width: u32,
        height: u32,
        clear: Color,
    ) -> Result<Vec<u8>, VulkanError> {
        if !plan.is_complete() {
            let kinds: Vec<String> = plan
                .unsupported
                .iter()
                .map(|(kind, count)| format!("{}x{}", count, kind.name()))
                .collect();
            return Err(VulkanError::Vulkan(format!(
                "incomplete plan: this backend cannot draw {}. Render this frame on the CPU \
                 rasterizer, or call `render_incomplete` if a partial result is genuinely wanted",
                kinds.join(", ")
            )));
        }
        self.render_incomplete(plan, width, height, clear)
    }

    /// [`render`](Self::render) without the completeness check. **Not** for a
    /// screen — see the module doc.
    ///
    /// # Errors
    ///
    /// Any Vulkan call failing.
    pub fn render_incomplete(
        &mut self,
        plan: &ScenePlan,
        width: u32,
        height: u32,
        clear: Color,
    ) -> Result<Vec<u8>, VulkanError> {
        let width = width.max(1);
        let height = height.max(1);
        self.ensure_targets(width, height, plan.target_depth + 1)?;
        #[allow(clippy::cast_precision_loss)]
        let viewport = [width as f32, height as f32, 0.0, 0.0];
        write_bytes(&self.device, self.uniform.memory, as_bytes(&viewport))?;
        self.upload_geometry(plan)?;
        self.execute(plan, width, height, clear)?;
        self.read_back(width, height)
    }

    // ── targets ────────────────────────────────────────────────────────────

    fn ensure_targets(&mut self, width: u32, height: u32, depth: usize) -> Result<(), VulkanError> {
        if self.size != (width, height) {
            unsafe { self.device.device_wait_idle() }?;
            for target in self.levels.drain(..).chain(self.scratch.drain(..)) {
                unsafe { destroy_target(&self.device, target) };
            }
            let old = std::mem::replace(&mut self.readback, Buffer::EMPTY);
            unsafe { destroy_buffer(&self.device, old) };
            self.readback = allocate_buffer(
                &self.device,
                vk::BufferUsageFlags::TRANSFER_DST,
                self.host_visible_memory,
                u64::from(width) * u64::from(height) * 8,
            )?;
            self.reset_post_sets()?;
            self.size = (width, height);
        }
        let mut fresh = Vec::new();
        while self.levels.len() < depth {
            let level = self.levels.len();
            self.levels.push(create_target(
                &self.device,
                &self.mem_props,
                self.rp_load,
                width,
                height,
            )?);
            self.scratch.push(create_target(
                &self.device,
                &self.mem_props,
                self.rp_load,
                width,
                height,
            )?);
            fresh.push(TargetId::Level(level));
            fresh.push(TargetId::Scratch(level));
        }
        if !fresh.is_empty() {
            self.initialise_targets(&fresh)?;
        }
        Ok(())
    }

    /// Clear new targets to transparent and leave them in
    /// `SHADER_READ_ONLY_OPTIMAL`, the layout every target rests in.
    fn initialise_targets(&mut self, ids: &[TargetId]) -> Result<(), VulkanError> {
        self.begin_commands()?;
        let cmd = self.command_buffer;
        for id in ids {
            let image = self.target(*id).image;
            unsafe {
                transition(
                    &self.device,
                    cmd,
                    image,
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                );
                self.device.cmd_clear_color_image(
                    cmd,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &vk::ClearColorValue { float32: [0.0; 4] },
                    &[color_range()],
                );
                transition(
                    &self.device,
                    cmd,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                );
            }
        }
        self.submit_and_wait()
    }

    fn target(&self, id: TargetId) -> &Target {
        match id {
            TargetId::Level(i) => &self.levels[i],
            TargetId::Scratch(i) => &self.scratch[i],
            TargetId::Dummy => &self.dummy,
        }
    }

    fn reset_post_sets(&mut self) -> Result<(), VulkanError> {
        if !self.post_sets.is_empty() {
            unsafe {
                self.device
                    .reset_descriptor_pool(self.post_pool, vk::DescriptorPoolResetFlags::empty())
            }?;
            self.post_sets.clear();
        }
        Ok(())
    }

    fn post_set(&mut self, src: TargetId, aux: TargetId) -> Result<vk::DescriptorSet, VulkanError> {
        if let Some(set) = self.post_sets.get(&(src, aux)) {
            return Ok(*set);
        }
        let set = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.post_pool)
                    .set_layouts(&[self.post_set_layout]),
            )
        }?[0];
        let image_info = |view: vk::ImageView| {
            [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }]
        };
        let src_info = image_info(self.target(src).view);
        let aux_info = image_info(self.target(aux).view);
        let mask_info = image_info(self.masks.view);
        let write = |binding: u32, info| {
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(info)
        };
        let writes = [
            write(0, &src_info),
            write(1, &aux_info),
            write(2, &mask_info),
        ];
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        self.post_sets.insert((src, aux), set);
        Ok(set)
    }

    // ── textures ───────────────────────────────────────────────────────────

    fn texture(&self, slot: Slot) -> &Texture {
        match slot {
            Slot::Glyphs => &self.glyphs,
            Slot::Images => &self.images,
            Slot::Masks => &self.masks,
            Slot::Ramps => &self.ramps,
        }
    }

    fn texture_mut(&mut self, slot: Slot) -> &mut Texture {
        match slot {
            Slot::Glyphs => &mut self.glyphs,
            Slot::Images => &mut self.images,
            Slot::Masks => &mut self.masks,
            Slot::Ramps => &mut self.ramps,
        }
    }

    /// Replace a texture's contents, recreating it when the size changes. A
    /// no-op when `version` is what is already resident (unless `force`).
    fn upload(
        &mut self,
        slot: Slot,
        bytes: &[u8],
        width: u32,
        height: u32,
        version: Option<u64>,
        force: bool,
    ) -> Result<(), VulkanError> {
        let (format, resized, current) = {
            let t = self.texture(slot);
            (
                t.format,
                t.width != width || t.height != height,
                t.version == version,
            )
        };
        if !force && !resized && current && version.is_some() {
            return Ok(());
        }
        debug_assert_eq!(
            bytes.len(),
            width as usize * height as usize * bytes_per_texel(format)
        );
        if resized {
            unsafe { self.device.device_wait_idle() }?;
            let fresh = create_texture(&self.device, &self.mem_props, format, width, height)?;
            let old = std::mem::replace(self.texture_mut(slot), fresh);
            unsafe { destroy_texture(&self.device, old) };
        }

        let needed = bytes.len() as u64;
        if self.texture(slot).staging.capacity < needed {
            let old = std::mem::replace(&mut self.texture_mut(slot).staging, Buffer::EMPTY);
            unsafe { destroy_buffer(&self.device, old) };
            let staging = allocate_buffer(
                &self.device,
                vk::BufferUsageFlags::TRANSFER_SRC,
                self.host_visible_memory,
                needed,
            )?;
            self.texture_mut(slot).staging = staging;
        }
        write_bytes(&self.device, self.texture(slot).staging.memory, bytes)?;

        let (image, staging) = {
            let t = self.texture(slot);
            (t.image, t.staging.handle)
        };
        self.begin_commands()?;
        let cmd = self.command_buffer;
        unsafe {
            // UNDEFINED as the old layout on purpose: the whole image is being
            // replaced, so the driver may discard what was there.
            transition(
                &self.device,
                cmd,
                image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            );
            self.device.cmd_copy_buffer_to_image(
                cmd,
                staging,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: color_layers(),
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    },
                }],
            );
            transition(
                &self.device,
                cmd,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        }
        self.submit_and_wait()?;

        if resized || force {
            let info = [vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: self.texture(slot).view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            }];
            unsafe {
                self.device.update_descriptor_sets(
                    &[vk::WriteDescriptorSet::default()
                        .dst_set(self.scene_set)
                        .dst_binding(slot.binding())
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&info)],
                    &[],
                );
            }
            if slot == Slot::Masks {
                self.reset_post_sets()?;
            }
        }
        self.texture_mut(slot).version = version;
        Ok(())
    }

    fn upload_geometry(&mut self, plan: &ScenePlan) -> Result<(), VulkanError> {
        let vertex_bytes = as_bytes(&plan.vertices);
        let index_bytes = as_bytes(&plan.indices);
        self.vertices = grow(
            &self.device,
            std::mem::replace(&mut self.vertices, Buffer::EMPTY),
            vk::BufferUsageFlags::VERTEX_BUFFER,
            self.host_visible_memory,
            vertex_bytes.len() as u64,
        )?;
        self.indices = grow(
            &self.device,
            std::mem::replace(&mut self.indices, Buffer::EMPTY),
            vk::BufferUsageFlags::INDEX_BUFFER,
            self.host_visible_memory,
            index_bytes.len() as u64,
        )?;
        write_bytes(&self.device, self.vertices.memory, vertex_bytes)?;
        write_bytes(&self.device, self.indices.memory, index_bytes)?;
        Ok(())
    }

    // ── execution ──────────────────────────────────────────────────────────

    fn begin_commands(&self) -> Result<(), VulkanError> {
        let cmd = self.command_buffer;
        unsafe {
            self.device
                .reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn execute(
        &mut self,
        plan: &ScenePlan,
        width: u32,
        height: u32,
        clear: Color,
    ) -> Result<(), VulkanError> {
        self.begin_commands()?;
        let a = f32::from(clear.a) / 255.0;
        let premultiplied_clear = [
            f32::from(clear.r) / 255.0 * a,
            f32::from(clear.g) / 255.0 * a,
            f32::from(clear.b) / 255.0 * a,
            a,
        ];
        let mut rec = Recorder {
            open: None,
            width,
            height,
        };
        rec.begin(self, TargetId::Level(0), Some(premultiplied_clear));

        // The level the innermost open layer draws into.
        let mut depth = 0usize;
        for step in &plan.steps {
            match *step {
                Step::Draw {
                    first_run,
                    run_count,
                } => {
                    rec.ensure_open(self, TargetId::Level(depth));
                    self.draw_runs(plan, first_run, run_count, width, height);
                }
                Step::PushLayer { rect, backdrop } => {
                    let parent = depth;
                    depth += 1;
                    rec.begin(self, TargetId::Level(depth), Some([0.0; 4]));
                    if let (Some(filter), false) = (backdrop, rect.is_empty()) {
                        // Seed with the parent's pixels, then filter that copy
                        // before any of the layer's own content draws.
                        self.pass(
                            &mut rec,
                            Pass::new(
                                TargetId::Level(depth),
                                rect,
                                TargetId::Level(parent),
                                mode::COPY,
                            ),
                        )?;
                        self.filter(&mut rec, depth, rect, filter)?;
                    }
                }
                Step::PopLayer {
                    rect,
                    composite,
                    filter,
                    mask,
                    blend,
                    alpha,
                } => {
                    let child = depth;
                    depth = depth.saturating_sub(1);
                    let mut source = TargetId::Level(child);
                    if !rect.is_empty() {
                        if let Some(filter) = filter {
                            self.filter(&mut rec, child, rect, filter)?;
                        }
                        if let Some(MaskRef { offset }) = mask {
                            let mut pass =
                                Pass::new(TargetId::Scratch(child), rect, source, mode::MASK);
                            pass.push.p[0] = [offset[0], offset[1], 1.0, 0.0];
                            self.pass(&mut rec, pass)?;
                            source = TargetId::Scratch(child);
                        }
                    }
                    let Some(region) = composite else {
                        continue;
                    };
                    if blend == BlendMode::Normal {
                        let mut pass =
                            Pass::new(TargetId::Level(depth), region, source, mode::OVER);
                        pass.read = rect;
                        pass.over = true;
                        pass.push.args[3] = alpha;
                        self.pass(&mut rec, pass)?;
                    } else {
                        // Snapshot the destination, then write blend(src × α, dst).
                        self.pass(
                            &mut rec,
                            Pass::new(
                                TargetId::Scratch(depth),
                                region,
                                TargetId::Level(depth),
                                mode::COPY,
                            ),
                        )?;
                        let mut pass =
                            Pass::new(TargetId::Level(depth), region, source, mode::BLEND);
                        pass.read = rect;
                        pass.aux = TargetId::Scratch(depth);
                        pass.push.args[2] = blend_index(blend);
                        pass.push.args[3] = alpha;
                        self.pass(&mut rec, pass)?;
                    }
                }
                Step::Shadow {
                    patch,
                    scissor,
                    caster,
                    inner,
                    box_radius,
                    color,
                    clip,
                } => {
                    if patch.is_empty() || scissor.is_empty() {
                        continue;
                    }
                    let work = depth + 1;
                    rec.begin(self, TargetId::Level(work), Some([0.0; 4]));
                    let mut seed =
                        Pass::new(TargetId::Level(work), patch, TargetId::Dummy, mode::SEED);
                    seed.push.p[0] = [caster.offset[0], caster.offset[1], 1.0, 0.0];
                    self.pass(&mut rec, seed)?;
                    if let Some(radius) = box_radius {
                        self.blur(&mut rec, work, patch, radius)?;
                    }
                    let mut source = TargetId::Level(work);
                    if let Some(inner) = inner {
                        let mut pass =
                            Pass::new(TargetId::Scratch(work), patch, source, mode::INSET);
                        pass.push.p[0] = [inner.offset[0], inner.offset[1], 1.0, 0.0];
                        self.pass(&mut rec, pass)?;
                        source = TargetId::Scratch(work);
                    }
                    let mut tint = Pass::new(TargetId::Level(depth), scissor, source, mode::TINT);
                    tint.read = patch;
                    tint.over = true;
                    tint.push.p[2] = color;
                    if let Some(clip) = clip {
                        tint.push.p[1] = [clip.offset[0], clip.offset[1], 1.0, 0.0];
                    }
                    self.pass(&mut rec, tint)?;
                }
            }
        }
        rec.end(self);

        let cmd = self.command_buffer;
        let frame = self.levels[0].image;
        unsafe {
            transition(
                &self.device,
                cmd,
                frame,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            );
            self.device.cmd_copy_image_to_buffer(
                cmd,
                frame,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.readback.handle,
                &[vk::BufferImageCopy {
                    buffer_offset: 0,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: color_layers(),
                    image_offset: vk::Offset3D::default(),
                    image_extent: vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    },
                }],
            );
            transition(
                &self.device,
                cmd,
                frame,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            );
        }
        self.submit_and_wait()
    }

    fn draw_runs(
        &self,
        plan: &ScenePlan,
        first_run: usize,
        run_count: usize,
        width: u32,
        height: u32,
    ) {
        if plan.vertices.is_empty() || plan.indices.is_empty() {
            return;
        }
        let cmd = self.command_buffer;
        let full = full_rect(width, height);
        unsafe {
            self.device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.scene_pipeline,
            );
            self.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.scene_layout,
                0,
                &[self.scene_set],
                &[],
            );
            self.device
                .cmd_bind_vertex_buffers(cmd, 0, &[self.vertices.handle], &[0]);
            self.device
                .cmd_bind_index_buffer(cmd, self.indices.handle, 0, vk::IndexType::UINT32);
            for run in &plan.runs[first_run..first_run + run_count] {
                let scissor = run.scissor.map_or(full, |r| to_scissor(r, width, height));
                if scissor.extent.width == 0 || scissor.extent.height == 0 {
                    continue;
                }
                self.device.cmd_set_scissor(cmd, 0, &[scissor]);
                self.device
                    .cmd_draw_indexed(cmd, run.index_count, 1, run.first_index, 0, 0);
            }
        }
    }

    /// Three horizontal+vertical box passes over `rect` of `L[level]`,
    /// ping-ponging through `S[level]` and ending back in `L[level]`.
    fn blur(
        &mut self,
        rec: &mut Recorder,
        level: usize,
        rect: PixelRect,
        radius: u32,
    ) -> Result<(), VulkanError> {
        #[allow(clippy::cast_precision_loss)]
        let r = radius as f32;
        for _ in 0..3 {
            let mut h = Pass::new(
                TargetId::Scratch(level),
                rect,
                TargetId::Level(level),
                mode::BOX_H,
            );
            h.push.args[1] = r;
            self.pass(rec, h)?;
            let mut v = Pass::new(
                TargetId::Level(level),
                rect,
                TargetId::Scratch(level),
                mode::BOX_V,
            );
            v.push.args[1] = r;
            self.pass(rec, v)?;
        }
        Ok(())
    }

    /// A filter over `rect` of `L[level]`, result left in `L[level]`.
    fn filter(
        &mut self,
        rec: &mut Recorder,
        level: usize,
        rect: PixelRect,
        filter: FilterOp,
    ) -> Result<(), VulkanError> {
        if let Some(radius) = filter.box_radius {
            self.blur(rec, level, rect, radius)?;
        }
        if let Some(m) = filter.color_matrix {
            let mut pass = Pass::new(
                TargetId::Scratch(level),
                rect,
                TargetId::Level(level),
                mode::MATRIX,
            );
            pass.push.p[0] = [m[0], m[1], m[2], m[3]];
            pass.push.p[1] = [m[5], m[6], m[7], m[8]];
            pass.push.p[2] = [m[10], m[11], m[12], m[13]];
            pass.push.p[3] = [m[15], m[16], m[17], m[18]];
            pass.push.p[4] = [m[4], m[9], m[14], m[19]];
            self.pass(rec, pass)?;
            self.pass(
                rec,
                Pass::new(
                    TargetId::Level(level),
                    rect,
                    TargetId::Scratch(level),
                    mode::COPY,
                ),
            )?;
        }
        Ok(())
    }

    fn pass(&mut self, rec: &mut Recorder, mut pass: Pass) -> Result<(), VulkanError> {
        let scissor = to_scissor(pass.scissor.to_rect(), rec.width, rec.height);
        if scissor.extent.width == 0 || scissor.extent.height == 0 {
            return Ok(());
        }
        debug_assert!(
            pass.into != pass.src && pass.into != pass.aux,
            "a pass may not read its own target"
        );
        let set = self.post_set(pass.src, pass.aux)?;
        rec.ensure_open(self, pass.into);
        pass.push.region = region(pass.read);
        let cmd = self.command_buffer;
        unsafe {
            self.device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                if pass.over {
                    self.post_over
                } else {
                    self.post_replace
                },
            );
            self.device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.post_layout,
                0,
                &[set],
                &[],
            );
            self.device.cmd_push_constants(
                cmd,
                self.post_layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                as_bytes(std::slice::from_ref(&pass.push)),
            );
            self.device.cmd_set_scissor(cmd, 0, &[scissor]);
            self.device.cmd_draw(cmd, 3, 1, 0, 0);
        }
        Ok(())
    }

    fn submit_and_wait(&self) -> Result<(), VulkanError> {
        let cmd = self.command_buffer;
        unsafe {
            self.device.end_command_buffer(cmd)?;
            self.device.reset_fences(&[self.fence])?;
            self.device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&[cmd])],
                self.fence,
            )?;
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
        }
        Ok(())
    }

    /// Premultiplied RGBA16F → straight RGBA8, rounded exactly as
    /// `NativeRenderer`'s `Premul::to_straight_u8`.
    fn read_back(&self, width: u32, height: u32) -> Result<Vec<u8>, VulkanError> {
        let texels = width as usize * height as usize;
        let mut half = vec![0u16; texels * 4];
        unsafe {
            let ptr = self.device.map_memory(
                self.readback.memory,
                0,
                (texels * 8) as u64,
                vk::MemoryMapFlags::empty(),
            )? as *const u16;
            std::ptr::copy_nonoverlapping(ptr, half.as_mut_ptr(), texels * 4);
            self.device.unmap_memory(self.readback.memory);
        }
        let mut out = vec![0u8; texels * 4];
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = |v: f32| (v * 255.0 + 0.5) as u8;
        for (pixel, slot) in half.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
            let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]].map(half_to_f32);
            let inv = if a > 0.0 { 1.0 / a } else { 0.0 };
            slot.copy_from_slice(&[byte(r * inv), byte(g * inv), byte(b * inv), byte(a)]);
        }
        Ok(out)
    }
}

/// Tracks the open render pass, so consecutive operations on one target
/// share it.
struct Recorder {
    open: Option<TargetId>,
    width: u32,
    height: u32,
}

impl Recorder {
    fn begin(&mut self, renderer: &SceneRenderer, target: TargetId, clear: Option<[f32; 4]>) {
        self.end(renderer);
        let clear_values = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: clear.unwrap_or([0.0; 4]),
            },
        }];
        let info = vk::RenderPassBeginInfo::default()
            .render_pass(if clear.is_some() {
                renderer.rp_clear
            } else {
                renderer.rp_load
            })
            .framebuffer(renderer.target(target).framebuffer)
            .render_area(full_rect(self.width, self.height))
            .clear_values(&clear_values);
        #[allow(clippy::cast_precision_loss)]
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: self.width as f32,
            height: self.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        unsafe {
            renderer.device.cmd_begin_render_pass(
                renderer.command_buffer,
                &info,
                vk::SubpassContents::INLINE,
            );
            renderer
                .device
                .cmd_set_viewport(renderer.command_buffer, 0, &[viewport]);
        }
        self.open = Some(target);
    }

    fn ensure_open(&mut self, renderer: &SceneRenderer, target: TargetId) {
        if self.open != Some(target) {
            self.begin(renderer, target, None);
        }
    }

    fn end(&mut self, renderer: &SceneRenderer) {
        if self.open.take().is_some() {
            unsafe { renderer.device.cmd_end_render_pass(renderer.command_buffer) };
        }
    }
}

impl std::fmt::Debug for SceneRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneRenderer")
            .field("size", &self.size)
            .field("targets", &self.levels.len())
            .field("vertex_capacity", &self.vertices.capacity)
            .finish_non_exhaustive()
    }
}

impl Drop for SceneRenderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.device.device_wait_idle();
            for target in self.levels.drain(..).chain(self.scratch.drain(..)) {
                destroy_target(&self.device, target);
            }
            destroy_target(
                &self.device,
                std::mem::replace(&mut self.dummy, Target::NULL),
            );
            for buffer in [
                std::mem::replace(&mut self.readback, Buffer::EMPTY),
                std::mem::replace(&mut self.vertices, Buffer::EMPTY),
                std::mem::replace(&mut self.indices, Buffer::EMPTY),
                std::mem::replace(&mut self.uniform, Buffer::EMPTY),
            ] {
                destroy_buffer(&self.device, buffer);
            }
            for slot in Slot::ALL {
                let texture = std::mem::replace(self.texture_mut(slot), Texture::NULL);
                destroy_texture(&self.device, texture);
            }
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.command_pool, None);
            for pipeline in [self.scene_pipeline, self.post_replace, self.post_over] {
                self.device.destroy_pipeline(pipeline, None);
            }
            self.device.destroy_pipeline_layout(self.scene_layout, None);
            self.device.destroy_pipeline_layout(self.post_layout, None);
            for module in self.shaders.drain(..) {
                self.device.destroy_shader_module(module, None);
            }
            self.device.destroy_descriptor_pool(self.scene_pool, None);
            self.device.destroy_descriptor_pool(self.post_pool, None);
            self.device
                .destroy_descriptor_set_layout(self.scene_set_layout, None);
            self.device
                .destroy_descriptor_set_layout(self.post_set_layout, None);
            self.device.destroy_render_pass(self.rp_load, None);
            self.device.destroy_render_pass(self.rp_clear, None);
        }
    }
}

// ── construction helpers ───────────────────────────────────────────────────

fn create_render_pass(
    device: &ash::Device,
    load: vk::AttachmentLoadOp,
) -> Result<vk::RenderPass, VulkanError> {
    // Every target rests in SHADER_READ_ONLY_OPTIMAL between passes, so a
    // later pass finds it in the layout it needs. A clearing pass may discard
    // the old contents (UNDEFINED); a loading pass may not.
    //
    // **The layout is not the synchronisation, which is what the two
    // dependencies below are for.** A layer is drawn into its own target in
    // one pass and sampled from the next, and nothing in a render pass orders
    // those against each other on its own: a driver is free to start the
    // second pass's fragment work before the first pass's colour writes have
    // landed. A software rasteriser runs the passes one after another and
    // forgives the omission — every one of these suites passed on llvmpipe —
    // while real hardware overlaps them. On a GeForce 920MX that showed up as
    // four failures in `vulkan_compositor.rs`, all of them a pass reading what
    // an earlier pass had written: `shaped-clips` sampled a mask that was not
    // there yet and left 2413 pixels unclipped, and backdrop blur, shadow
    // spread and nested layers came back a few steps off.
    let initial = if load == vk::AttachmentLoadOp::CLEAR {
        vk::ImageLayout::UNDEFINED
    } else {
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
    };
    let attachments = [vk::AttachmentDescription::default()
        .format(TARGET_FORMAT)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(load)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(initial)
        .final_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
    let color_ref = [vk::AttachmentReference {
        attachment: 0,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
    }];
    let subpasses = [vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_ref)];
    let dependencies = [
        // Reads of this target by an earlier pass finish before this one
        // writes it (write-after-read).
        vk::SubpassDependency {
            src_subpass: vk::SUBPASS_EXTERNAL,
            dst_subpass: 0,
            src_stage_mask: vk::PipelineStageFlags::FRAGMENT_SHADER,
            dst_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            src_access_mask: vk::AccessFlags::SHADER_READ,
            dst_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                | vk::AccessFlags::COLOR_ATTACHMENT_READ,
            dependency_flags: vk::DependencyFlags::BY_REGION,
        },
        // This pass's writes are complete and visible before any later pass
        // samples the target (read-after-write). The one that was missing.
        vk::SubpassDependency {
            src_subpass: 0,
            dst_subpass: vk::SUBPASS_EXTERNAL,
            src_stage_mask: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            dst_stage_mask: vk::PipelineStageFlags::FRAGMENT_SHADER,
            src_access_mask: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            dst_access_mask: vk::AccessFlags::SHADER_READ,
            dependency_flags: vk::DependencyFlags::BY_REGION,
        },
    ];
    Ok(unsafe {
        device.create_render_pass(
            &vk::RenderPassCreateInfo::default()
                .attachments(&attachments)
                .subpasses(&subpasses)
                .dependencies(&dependencies),
            None,
        )
    }?)
}

fn create_post_pool(device: &ash::Device) -> Result<vk::DescriptorPool, VulkanError> {
    // Sets are keyed by (src, aux) target pairs — a handful per level, so
    // this is far beyond `MAX_TARGET_DEPTH`'s need.
    Ok(unsafe {
        device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .pool_sizes(&[vk::DescriptorPoolSize {
                    ty: vk::DescriptorType::SAMPLED_IMAGE,
                    descriptor_count: 3 * 1024,
                }])
                .max_sets(1024),
            None,
        )
    }?)
}

fn create_pipeline(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    layout: vk::PipelineLayout,
    (vs, fs): (vk::ShaderModule, vk::ShaderModule),
    bindings: &[vk::VertexInputBindingDescription],
    attributes: &[vk::VertexInputAttributeDescription],
    premultiplied_over: bool,
) -> Result<vk::Pipeline, VulkanError> {
    let vs_name = CString::new("vs_main").expect("no interior nul");
    let fs_name = CString::new("fs_main").expect("no interior nul");
    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vs)
            .name(&vs_name),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fs)
            .name(&fs_name),
    ];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(bindings)
        .vertex_attribute_descriptions(attributes);
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    // Premultiplied source-over, or no blending at all.
    let blend = [vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(premultiplied_over)
        .src_color_blend_factor(vk::BlendFactor::ONE)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA)];
    let color_blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .render_pass(render_pass)
        .subpass(0);
    let pipelines =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) }
            .map_err(|(_, e)| VulkanError::Vulkan(e.to_string()))?;
    Ok(pipelines[0])
}

fn allocate_image(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    format: vk::Format,
    (width, height): (u32, u32),
    usage: vk::ImageUsageFlags,
) -> Result<(vk::Image, vk::DeviceMemory, vk::ImageView), VulkanError> {
    let image = unsafe {
        device.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )
    }?;
    let requirements = unsafe { device.get_image_memory_requirements(image) };
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(pick_memory_type(
                    mem_props,
                    requirements.memory_type_bits,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )?),
            None,
        )
    }?;
    unsafe { device.bind_image_memory(image, memory, 0) }?;
    let view = unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
                .subresource_range(color_range()),
            None,
        )
    }?;
    Ok((image, memory, view))
}

fn create_target(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    render_pass: vk::RenderPass,
    width: u32,
    height: u32,
) -> Result<Target, VulkanError> {
    let (image, memory, view) = allocate_image(
        device,
        mem_props,
        TARGET_FORMAT,
        (width, height),
        vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC
            | vk::ImageUsageFlags::TRANSFER_DST,
    )?;
    let framebuffer = unsafe {
        device.create_framebuffer(
            &vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&[view])
                .width(width)
                .height(height)
                .layers(1),
            None,
        )
    }?;
    Ok(Target {
        image,
        memory,
        view,
        framebuffer,
    })
}

fn create_texture(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    format: vk::Format,
    width: u32,
    height: u32,
) -> Result<Texture, VulkanError> {
    let (image, memory, view) = allocate_image(
        device,
        mem_props,
        format,
        (width, height),
        vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
    )?;
    Ok(Texture {
        image,
        memory,
        view,
        format,
        width,
        height,
        version: None,
        staging: Buffer::EMPTY,
    })
}

const fn bytes_per_texel(format: vk::Format) -> usize {
    match format {
        vk::Format::R8_UNORM => 1,
        vk::Format::R8G8B8A8_UNORM => 4,
        vk::Format::R32G32B32A32_SFLOAT => 16,
        _ => 8,
    }
}

unsafe fn destroy_target(device: &ash::Device, target: Target) {
    if target.image == vk::Image::null() {
        return;
    }
    device.destroy_framebuffer(target.framebuffer, None);
    device.destroy_image_view(target.view, None);
    device.destroy_image(target.image, None);
    device.free_memory(target.memory, None);
}

unsafe fn destroy_texture(device: &ash::Device, texture: Texture) {
    destroy_buffer(device, texture.staging);
    if texture.image == vk::Image::null() {
        return;
    }
    device.destroy_image_view(texture.view, None);
    device.destroy_image(texture.image, None);
    device.free_memory(texture.memory, None);
}

const fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}

const fn color_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    }
}

unsafe fn transition(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    from: vk::ImageLayout,
    to: vk::ImageLayout,
) {
    let access = |layout: vk::ImageLayout| match layout {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => vk::AccessFlags::TRANSFER_WRITE,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => vk::AccessFlags::TRANSFER_READ,
        vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL => vk::AccessFlags::SHADER_READ,
        _ => vk::AccessFlags::empty(),
    };
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(from)
        .new_layout(to)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(color_range())
        .src_access_mask(access(from))
        .dst_access_mask(access(to));
    device.cmd_pipeline_barrier(
        cmd,
        vk::PipelineStageFlags::ALL_COMMANDS,
        vk::PipelineStageFlags::ALL_COMMANDS,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[barrier],
    );
}

const fn full_rect(width: u32, height: u32) -> vk::Rect2D {
    vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D { width, height },
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
fn to_scissor(rect: vieww_foundation::Rect, width: u32, height: u32) -> vk::Rect2D {
    let left = (rect.left.floor().max(0.0) as u32).min(width);
    let top = (rect.top.floor().max(0.0) as u32).min(height);
    let right = (rect.right.ceil().max(0.0) as u32).min(width);
    let bottom = (rect.bottom.ceil().max(0.0) as u32).min(height);
    vk::Rect2D {
        offset: vk::Offset2D {
            x: left as i32,
            y: top as i32,
        },
        extent: vk::Extent2D {
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        },
    }
}

fn allocate_buffer(
    device: &ash::Device,
    usage: vk::BufferUsageFlags,
    memory_type: u32,
    size: u64,
) -> Result<Buffer, VulkanError> {
    let size = size.max(4);
    let handle = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )
    }?;
    let requirements = unsafe { device.get_buffer_memory_requirements(handle) };
    let memory = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(memory_type),
            None,
        )
    }?;
    unsafe { device.bind_buffer_memory(handle, memory, 0) }?;
    Ok(Buffer {
        handle,
        memory,
        capacity: size,
    })
}

/// Keep `existing` if it is big enough; otherwise replace it with one of at
/// least double the capacity. Never shrinks — a list that scrolled once will
/// scroll again.
fn grow(
    device: &ash::Device,
    existing: Buffer,
    usage: vk::BufferUsageFlags,
    memory_type: u32,
    needed: u64,
) -> Result<Buffer, VulkanError> {
    if existing.handle != vk::Buffer::null() && existing.capacity >= needed.max(4) {
        return Ok(existing);
    }
    let mut capacity = existing.capacity.max(1024);
    while capacity < needed {
        capacity *= 2;
    }
    unsafe { destroy_buffer(device, existing) };
    allocate_buffer(device, usage, memory_type, capacity)
}

fn write_bytes(
    device: &ash::Device,
    memory: vk::DeviceMemory,
    bytes: &[u8],
) -> Result<(), VulkanError> {
    if bytes.is_empty() {
        return Ok(());
    }
    unsafe {
        let ptr = device
            .map_memory(memory, 0, bytes.len() as u64, vk::MemoryMapFlags::empty())?
            .cast::<u8>();
        ptr.copy_from_nonoverlapping(bytes.as_ptr(), bytes.len());
        device.unmap_memory(memory);
    }
    Ok(())
}

unsafe fn destroy_buffer(device: &ash::Device, buffer: Buffer) {
    if buffer.handle != vk::Buffer::null() {
        device.destroy_buffer(buffer.handle, None);
    }
    if buffer.memory != vk::DeviceMemory::null() {
        device.free_memory(buffer.memory, None);
    }
}

fn as_bytes<T: Copy>(values: &[T]) -> &[u8] {
    // SAFETY: only ever called with plain `repr(C)` float/integer data
    // (`Vertex`, `u32`, `f32`, `PostPush`), which has no padding and whose
    // every byte is initialised.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn pick_memory_type(
    props: &vk::PhysicalDeviceMemoryProperties,
    filter: u32,
    flags: vk::MemoryPropertyFlags,
) -> Result<u32, VulkanError> {
    (0..props.memory_type_count)
        .find(|&i| {
            (filter & (1 << i)) != 0
                && props.memory_types[i as usize]
                    .property_flags
                    .contains(flags)
        })
        .ok_or_else(|| VulkanError::Vulkan("no suitable memory type".into()))
}

/// IEEE 754 binary16 → `f32`.
fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((h >> 10) & 0x1f);
    let mantissa = f32::from(h & 0x3ff);
    match exponent {
        0 => sign * mantissa * 2f32.powi(-24),
        31 if mantissa == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        e => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(e - 15),
    }
}

#[cfg(test)]
mod tests {
    use super::half_to_f32;

    #[test]
    fn half_floats_decode() {
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0x7c00), f32::INFINITY);
        // 0x3555 ≈ 1/3, the value a three-box average of full coverage makes.
        assert!((half_to_f32(0x3555) - 1.0 / 3.0).abs() < 1e-4);
    }
}
