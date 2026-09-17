//! The Vulkan backend — spec §4.3's first-landing HAL implementation.
//!
//! Headless only in this build: instance, device, a WGSL shader translated
//! through `naga` to SPIR-V, one graphics pipeline, and a
//! render-to-texture-then-copy-to-buffer readback. No swapchain, no window —
//! spec §14.1's M0/M1 slice ("Vulkan device, swapchain, upload ring, first
//! quad + clear") minus the swapchain, which needs a live surface this
//! sandbox has neither the display nor the windowing to create. What is here
//! is real and runs: `tests/vulkan_smoke.rs` exercises it against
//! `lavapipe` (Mesa's software Vulkan implementation — installed for this
//! build specifically so the HAL could be *tested*, not just compiled) and
//! checks the read-back pixels against `clear_color`, exactly.

use std::ffi::{c_char, CStr, CString};

use ash::vk;
use vieww_foundation::Color;

use super::{AdapterInfo, Device as HalDevice};

/// [`scene::SceneRenderer`] — a persistent pipeline that executes a whole
/// `vieww_gpu::ScenePlan` per frame, rather than building and tearing down a
/// pipeline per shape the way [`VulkanDevice::render_mesh_to_pixels`] does.
/// See that module's own doc for the difference and why it matters at 16.7ms.
pub mod scene;
pub use scene::SceneRenderer;

/// [`VulkanDevice::for_window`] and [`swapchain::VulkanSwapchain`] — real
/// windowed presentation. See that module's own docs.
#[cfg(feature = "vulkan-swapchain")]
pub mod swapchain;
#[cfg(feature = "vulkan-swapchain")]
pub use swapchain::VulkanSwapchain;

#[derive(Debug)]
pub enum VulkanError {
    Loading(String),
    NoAdapter,
    Vulkan(String),
}

impl std::fmt::Display for VulkanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loading(m) => write!(f, "loading Vulkan: {m}"),
            Self::NoAdapter => write!(f, "no Vulkan-capable adapter"),
            Self::Vulkan(m) => write!(f, "Vulkan: {m}"),
        }
    }
}

impl std::error::Error for VulkanError {}

impl From<vk::Result> for VulkanError {
    fn from(result: vk::Result) -> Self {
        Self::Vulkan(result.to_string())
    }
}

pub struct VulkanDevice {
    entry: ash::Entry,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    info: AdapterInfo,
}

impl std::fmt::Debug for VulkanDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanDevice")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl super::sealed::Sealed for VulkanDevice {}

/// Load the Vulkan loader (or, on macOS, MoltenVK itself).
///
/// # Why not just `ash::Entry::load()`
///
/// On Linux and Windows the loader is a system library on the default search
/// path, and `Entry::load` is the whole story. On macOS it never is:
///
/// * Homebrew's `vulkan-loader` lives in `/opt/homebrew/lib` (Apple silicon) or
///   `/usr/local/lib` (Intel), and neither is on `dlopen`'s default path — the
///   first macOS CI run found a working MoltenVK device with `vulkaninfo` and
///   then failed every Vulkan stage with `dlopen(libvulkan.dylib): no such
///   file`, because `vulkaninfo` is linked against the loader by path and this
///   crate asked for it by name.
/// * The LunarG SDK sets `VULKAN_SDK` and installs into `$VULKAN_SDK/lib`.
/// * A shipped `.app` can only rely on what it carries in
///   `Contents/Frameworks` (blocker B1): either the loader, or MoltenVK
///   loaded directly — MoltenVK exports the Vulkan entry points itself.
///
/// So on macOS this tries the default name first (which honours
/// `DYLD_LIBRARY_PATH` / `DYLD_FALLBACK_LIBRARY_PATH`), then those places in
/// that order, and the error names every path it tried.
///
/// # Errors
///
/// [`VulkanError::Loading`] when no candidate loads.
pub fn load_entry() -> Result<ash::Entry, VulkanError> {
    match unsafe { ash::Entry::load() } {
        Ok(entry) => Ok(entry),
        Err(error) => load_entry_fallback(error.to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "same signature as the macOS fallback"
)]
fn load_entry_fallback(first: String) -> Result<ash::Entry, VulkanError> {
    Err(VulkanError::Loading(first))
}

#[cfg(target_os = "macos")]
fn load_entry_fallback(first: String) -> Result<ash::Entry, VulkanError> {
    use std::path::{Path, PathBuf};

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        let frameworks = dir.join("../Frameworks");
        candidates.push(frameworks.join("libvulkan.1.dylib"));
        candidates.push(frameworks.join("libMoltenVK.dylib"));
        candidates.push(dir.join("libvulkan.1.dylib"));
    }
    if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
        let sdk = PathBuf::from(sdk);
        candidates.push(sdk.join("lib/libvulkan.1.dylib"));
        candidates.push(sdk.join("lib/libvulkan.dylib"));
    }
    for prefix in ["/opt/homebrew", "/usr/local"] {
        candidates.push(Path::new(prefix).join("lib/libvulkan.1.dylib"));
        candidates.push(Path::new(prefix).join("lib/libMoltenVK.dylib"));
    }
    let mut tried = Vec::new();
    for path in candidates.into_iter().filter(|p| p.is_file()) {
        match unsafe { ash::Entry::load_from(&path) } {
            Ok(entry) => return Ok(entry),
            Err(error) => tried.push(format!("{}: {error}", path.display())),
        }
    }
    let detail = if tried.is_empty() {
        "no Vulkan loader or MoltenVK found in the app bundle, $VULKAN_SDK, /opt/homebrew/lib \
         or /usr/local/lib (install the Vulkan SDK, or `brew install vulkan-loader molten-vk`)"
            .to_owned()
    } else {
        format!("also tried {}", tried.join("; "))
    };
    Err(VulkanError::Loading(format!("{first}; {detail}")))
}

impl VulkanDevice {
    /// Bring up a headless Vulkan 1.1 device on the first adapter that
    /// offers a graphics queue — spec §4.3's "Vulkan 1.1 ... Timeline
    /// semaphores mandatory" (the instance is created at 1.2 so timeline
    /// semaphores, a core 1.2 feature, are always available; nothing here
    /// uses them yet — see the module docs on scope).
    pub fn new() -> Result<Self, VulkanError> {
        let entry = load_entry()?;
        let instance = Self::create_instance(&entry, &[])?;

        let (physical_device, queue_family) =
            Self::pick_physical_device(&instance, |pd| Self::find_graphics_family(&instance, pd))
                .ok_or_else(|| {
                unsafe { instance.destroy_instance(None) };
                VulkanError::NoAdapter
            })?;

        let info = Self::adapter_info(&instance, physical_device);
        let (device, queue) = Self::create_device(&instance, physical_device, queue_family, &[])?;

        Ok(Self {
            entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family,
            info,
        })
    }

    /// The instance every constructor shares: application/engine identity and
    /// API version are fixed, `extra_extensions` is the only thing that
    /// varies — empty for the headless path, `VK_KHR_surface` plus a
    /// platform surface extension (via `ash_window`) for
    /// [`swapchain`]'s `for_window`.
    fn create_instance(
        entry: &ash::Entry,
        extra_extensions: &[*const c_char],
    ) -> Result<ash::Instance, VulkanError> {
        let app_name = CString::new("vieww-hal").unwrap();
        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(0)
            .engine_name(&app_name)
            .engine_version(0)
            .api_version(vk::API_VERSION_1_2);

        // **Portability drivers (MoltenVK on macOS) are invisible without this.**
        //
        // Since Vulkan loader 1.3.216 a "portability" driver — one that is not
        // fully conformant, which is every Vulkan-on-Metal layer — is only
        // enumerated when the instance enables `VK_KHR_portability_enumeration`
        // and sets `ENUMERATE_PORTABILITY`. Without it a Mac with the Vulkan
        // SDK installed reports no adapter at all, and every window
        // (`vieww-platform-winit` presents through this module on every
        // desktop) fails to open. Enabled only when the loader offers it, so
        // conformant drivers elsewhere see no change.
        let mut extensions: Vec<*const c_char> = extra_extensions.to_vec();
        let portability = unsafe { entry.enumerate_instance_extension_properties(None) }
            .unwrap_or_default()
            .iter()
            .any(|ext| {
                ext.extension_name_as_c_str()
                    .is_ok_and(|name| name == ash::khr::portability_enumeration::NAME)
            });
        if portability {
            extensions.push(ash::khr::portability_enumeration::NAME.as_ptr());
        }

        let mut instance_info = vk::InstanceCreateInfo::default().application_info(&app_info);
        if !extensions.is_empty() {
            instance_info = instance_info.enabled_extension_names(&extensions);
        }
        if portability {
            instance_info = instance_info.flags(vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR);
        }
        unsafe { entry.create_instance(&instance_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))
    }

    /// Pick the best physical device, rather than whichever the loader listed
    /// first.
    ///
    /// # Why this is not "the first one with a graphics queue"
    ///
    /// Because a machine can have more than one, and the difference is not
    /// cosmetic. A laptop with switchable graphics enumerates its integrated
    /// part first; a machine with Mesa installed can enumerate `llvmpipe`
    /// first, and a frame drawn there is a CPU frame that every measurement
    /// will record as a GPU one. Neither case announces itself, and the second
    /// invalidates any performance claim made on that machine.
    ///
    /// So: discrete, then integrated, then virtual, then anything else, with
    /// CPU/software implementations last and only if nothing else will do.
    /// `VIEWW_VK_DEVICE` overrides the lot with a case-insensitive substring of
    /// the device name, because a developer with two real GPUs needs to be able
    /// to say which one without editing this function.
    fn pick_physical_device(
        instance: &ash::Instance,
        mut accept: impl FnMut(vk::PhysicalDevice) -> Option<u32>,
    ) -> Option<(vk::PhysicalDevice, u32)> {
        let physical_devices = unsafe { instance.enumerate_physical_devices() }.ok()?;
        let wanted = std::env::var("VIEWW_VK_DEVICE")
            .ok()
            .map(|s| s.to_lowercase());

        let mut best: Option<(u32, vk::PhysicalDevice, u32)> = None;
        for pd in physical_devices {
            let Some(family) = accept(pd) else { continue };
            let props = unsafe { instance.get_physical_device_properties(pd) };
            let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
                .to_string_lossy()
                .to_lowercase();
            if let Some(wanted) = wanted.as_deref() {
                if name.contains(wanted) {
                    return Some((pd, family));
                }
                continue;
            }
            // Higher is better. Software last, whatever it calls itself.
            let software = name.contains("llvmpipe")
                || name.contains("lavapipe")
                || name.contains("swiftshader");
            let rank = if software {
                0
            } else {
                match props.device_type {
                    vk::PhysicalDeviceType::DISCRETE_GPU => 4,
                    vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
                    vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                    _ => 1,
                }
            };
            if best.is_none_or(|(seen, _, _)| rank > seen) {
                best = Some((rank, pd, family));
            }
        }
        best.map(|(_, pd, family)| (pd, family))
    }

    fn find_graphics_family(instance: &ash::Instance, pd: vk::PhysicalDevice) -> Option<u32> {
        let families = unsafe { instance.get_physical_device_queue_family_properties(pd) };
        families
            .iter()
            .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .map(|idx| idx as u32)
    }

    fn adapter_info(instance: &ash::Instance, physical_device: vk::PhysicalDevice) -> AdapterInfo {
        let props = unsafe { instance.get_physical_device_properties(physical_device) };
        let name = unsafe { CStr::from_ptr(props.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let device_type = format!("{:?}", props.device_type);
        AdapterInfo {
            name,
            backend: "vulkan",
            device_type,
        }
    }

    /// One graphics queue, `extra_extensions` enabled (empty for headless,
    /// `VK_KHR_swapchain` for [`swapchain`]'s windowed path).
    fn create_device(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        queue_family: u32,
        extra_extensions: &[*const c_char],
    ) -> Result<(ash::Device, vk::Queue), VulkanError> {
        let priorities = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let queue_infos = [queue_info];
        // A portability device (MoltenVK) *must* have `VK_KHR_portability_subset`
        // enabled when it advertises it — the spec makes that a validity rule.
        let mut extensions: Vec<*const c_char> = extra_extensions.to_vec();
        let subset = unsafe { instance.enumerate_device_extension_properties(physical_device) }
            .unwrap_or_default()
            .iter()
            .any(|ext| {
                ext.extension_name_as_c_str()
                    .is_ok_and(|name| name == ash::khr::portability_subset::NAME)
            });
        if subset {
            extensions.push(ash::khr::portability_subset::NAME.as_ptr());
        }
        let mut device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queue_infos);
        if !extensions.is_empty() {
            device_info = device_info.enabled_extension_names(&extensions);
        }
        let device = unsafe { instance.create_device(physical_device, &device_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        Ok((device, queue))
    }

    pub(crate) fn find_memory_type(
        &self,
        filter: u32,
        flags: vk::MemoryPropertyFlags,
    ) -> Result<u32, VulkanError> {
        let mem_props = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        };
        (0..mem_props.memory_type_count)
            .find(|&i| {
                (filter & (1 << i)) != 0
                    && mem_props.memory_types[i as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or_else(|| VulkanError::Vulkan("no suitable memory type".into()))
    }

    /// The physical device's memory properties, so a renderer built on this one
    /// can choose a memory type per allocation instead of once at construction.
    ///
    /// Resolving a single index up front is what
    /// `VUID-vkBindImageMemory-memory-01047` fires on: an image accepts only the
    /// types listed in its own `memoryTypeBits`, and a discrete GPU's sets do
    /// not coincide the way a unified-memory part's do.
    pub(crate) fn memory_properties(&self) -> vk::PhysicalDeviceMemoryProperties {
        unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical_device)
        }
    }

    /// The loaded device handle, for a renderer built on top of this one.
    ///
    /// `pub(crate)`, not `pub`: an `ash::Device` in a caller's hands is a
    /// licence to destroy objects this type owns, and the sealing on
    /// [`Device`](crate::Device) exists precisely so that the set of things
    /// which can do that is a list inside this crate.
    /// [`scene::SceneRenderer`] is on that list; nothing outside it is.
    pub(crate) const fn raw_device(&self) -> &ash::Device {
        &self.device
    }

    /// The graphics queue this device submits on.
    pub(crate) const fn raw_queue(&self) -> vk::Queue {
        self.queue
    }

    /// The queue family [`raw_queue`](Self::raw_queue) belongs to, which a
    /// command pool has to be created against.
    pub(crate) const fn queue_family_index(&self) -> u32 {
        self.queue_family
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
        let _ = &self.entry;
    }
}

/// The one shader this backend ships in this build: a fullscreen triangle
/// (the standard 3-vertex, no-vertex-buffer trick) whose fragment stage
/// paints a uniform colour — spec §4.4: "All shaders are authored once in
/// WGSL ... naga validates and translates every WGSL module into SPIR-V ...
/// artifacts that are embedded in the binary". Translation happens at
/// [`VulkanDevice::render_clear_to_pixels`] call time in this build rather
/// than at compile time (spec's "runtime shader creation consumes the
/// pre-baked artifact"); baking it into `build.rs` is tracked in
/// `docs/RENDERER-MIGRATION.md` alongside the rest of the GPU tessellation
/// pipeline.
///
/// The WGSL source itself now lives in `vieww-shaders` (as
/// [`vieww_shaders::library::CLEAR_WGSL`]) rather than as a constant in
/// this module — see that crate's top doc for why one shared shader
/// library, rather than one copy per GPU backend, is the point of it
/// existing at all.
fn translate_wgsl_to_spirv(
    source: &str,
    entry_point: &str,
    stage: naga::ShaderStage,
) -> Result<Vec<u32>, VulkanError> {
    vieww_shaders::ParsedShader::parse(source)
        .and_then(|parsed| parsed.to_spirv(entry_point, stage))
        .map_err(|e| VulkanError::Vulkan(format!("vieww-shaders: {e}")))
}

impl HalDevice for VulkanDevice {
    type Error = VulkanError;

    fn info(&self) -> AdapterInfo {
        self.info.clone()
    }

    fn render_clear_to_pixels(
        &self,
        width: u32,
        height: u32,
        clear_color: Color,
    ) -> Result<Vec<u8>, VulkanError> {
        let device = &self.device;
        let format = vk::Format::R8G8B8A8_UNORM;

        // --- color attachment image -------------------------------------------------
        let image_info = vk::ImageCreateInfo::default()
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
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { device.create_image(&image_info, None) }?;
        let mem_req = unsafe { device.get_image_memory_requirements(image) };
        let mem_type = self.find_memory_type(
            mem_req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let image_mem_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_req.size)
            .memory_type_index(mem_type);
        let image_memory = unsafe { device.allocate_memory(&image_mem_info, None) }?;
        unsafe { device.bind_image_memory(image, image_memory, 0) }?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let image_view = unsafe { device.create_image_view(&view_info, None) }?;

        // --- render pass + framebuffer -----------------------------------------------
        let attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        let attachments = [attachment];
        let color_ref = [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_ref);
        let subpasses = [subpass];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses);
        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None) }?;

        let fb_attachments = [image_view];
        let fb_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&fb_attachments)
            .width(width)
            .height(height)
            .layers(1);
        let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }?;

        // --- uniform buffer -----------------------------------------------------------
        let straight = [
            f32::from(clear_color.r) / 255.0,
            f32::from(clear_color.g) / 255.0,
            f32::from(clear_color.b) / 255.0,
            f32::from(clear_color.a) / 255.0,
        ];
        let ubo_size = std::mem::size_of::<[f32; 4]>() as u64;
        let ubo_info = vk::BufferCreateInfo::default()
            .size(ubo_size)
            .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let ubo = unsafe { device.create_buffer(&ubo_info, None) }?;
        let ubo_req = unsafe { device.get_buffer_memory_requirements(ubo) };
        let ubo_mem_type = self.find_memory_type(
            ubo_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let ubo_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(ubo_req.size)
            .memory_type_index(ubo_mem_type);
        let ubo_memory = unsafe { device.allocate_memory(&ubo_alloc, None) }?;
        unsafe { device.bind_buffer_memory(ubo, ubo_memory, 0) }?;
        unsafe {
            let ptr = device.map_memory(ubo_memory, 0, ubo_size, vk::MemoryMapFlags::empty())?
                as *mut f32;
            ptr.copy_from_nonoverlapping(straight.as_ptr(), 4);
            device.unmap_memory(ubo_memory);
        }

        // --- descriptor set -------------------------------------------------------------
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let set_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None) }?;

        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 1,
        };
        let pool_sizes = [pool_size];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(1);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }?;
        let set_layouts = [set_layout];
        let set_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&set_layouts);
        let descriptor_set = unsafe { device.allocate_descriptor_sets(&set_alloc) }?[0];

        let buffer_info = vk::DescriptorBufferInfo {
            buffer: ubo,
            offset: 0,
            range: ubo_size,
        };
        let buffer_infos = [buffer_info];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(&buffer_infos);
        unsafe { device.update_descriptor_sets(&[write], &[]) };

        // --- shaders + pipeline -----------------------------------------------------------
        let vs_spirv = translate_wgsl_to_spirv(
            vieww_shaders::library::CLEAR_WGSL,
            "vs_main",
            naga::ShaderStage::Vertex,
        )?;
        let fs_spirv = translate_wgsl_to_spirv(
            vieww_shaders::library::CLEAR_WGSL,
            "fs_main",
            naga::ShaderStage::Fragment,
        )?;
        let vs_info = vk::ShaderModuleCreateInfo::default().code(&vs_spirv);
        let fs_info = vk::ShaderModuleCreateInfo::default().code(&fs_spirv);
        let vs_module = unsafe { device.create_shader_module(&vs_info, None) }?;
        let fs_module = unsafe { device.create_shader_module(&fs_info, None) }?;

        let entry_point = CString::new("vs_main").unwrap();
        let fs_entry_point = CString::new("fs_main").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs_module)
                .name(&entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs_module)
                .name(&fs_entry_point),
        ];

        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width, height },
        };
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend_attachments = [blend_attachment];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

        let pipeline_layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        let pipeline_layout =
            unsafe { device.create_pipeline_layout(&pipeline_layout_info, None) }?;

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        let pipelines = unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }
        .map_err(|(_, e)| VulkanError::Vulkan(e.to_string()))?;
        let pipeline = pipelines[0];

        // --- readback buffer -----------------------------------------------------------
        let readback_size = (width as u64) * (height as u64) * 4;
        let readback_info = vk::BufferCreateInfo::default()
            .size(readback_size)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let readback_buffer = unsafe { device.create_buffer(&readback_info, None) }?;
        let readback_req = unsafe { device.get_buffer_memory_requirements(readback_buffer) };
        let readback_mem_type = self.find_memory_type(
            readback_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let readback_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(readback_req.size)
            .memory_type_index(readback_mem_type);
        let readback_memory = unsafe { device.allocate_memory(&readback_alloc, None) }?;
        unsafe { device.bind_buffer_memory(readback_buffer, readback_memory, 0) }?;

        // --- command buffer -----------------------------------------------------------
        let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(self.queue_family);
        let command_pool = unsafe { device.create_command_pool(&pool_info, None) }?;
        let cmd_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = unsafe { device.allocate_command_buffers(&cmd_alloc) }?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe { device.begin_command_buffer(command_buffer, &begin_info) }?;

        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 0.0],
            },
        };
        let clear_values = [clear_value];
        let rp_begin = vk::RenderPassBeginInfo::default()
            .render_pass(render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            })
            .clear_values(&clear_values);
        unsafe {
            device.cmd_begin_render_pass(command_buffer, &rp_begin, vk::SubpassContents::INLINE);
            device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline_layout,
                0,
                &[descriptor_set],
                &[],
            );
            device.cmd_draw(command_buffer, 3, 1, 0, 0);
            device.cmd_end_render_pass(command_buffer);

            let copy_region = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };
            device.cmd_copy_image_to_buffer(
                command_buffer,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback_buffer,
                &[copy_region],
            );
        }
        unsafe { device.end_command_buffer(command_buffer) }?;

        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { device.create_fence(&fence_info, None) }?;
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        unsafe {
            device.queue_submit(self.queue, &[submit_info], fence)?;
            device.wait_for_fences(&[fence], true, u64::MAX)?;
        }

        let mut pixels = vec![0u8; readback_size as usize];
        unsafe {
            let ptr = device.map_memory(
                readback_memory,
                0,
                readback_size,
                vk::MemoryMapFlags::empty(),
            )? as *const u8;
            std::ptr::copy_nonoverlapping(ptr, pixels.as_mut_ptr(), readback_size as usize);
            device.unmap_memory(readback_memory);
        }

        unsafe {
            device.destroy_fence(fence, None);
            device.destroy_command_pool(command_pool, None);
            device.destroy_pipeline(pipeline, None);
            device.destroy_pipeline_layout(pipeline_layout, None);
            device.destroy_shader_module(vs_module, None);
            device.destroy_shader_module(fs_module, None);
            device.destroy_descriptor_pool(descriptor_pool, None);
            device.destroy_descriptor_set_layout(set_layout, None);
            device.free_memory(readback_memory, None);
            device.destroy_buffer(readback_buffer, None);
            device.free_memory(ubo_memory, None);
            device.destroy_buffer(ubo, None);
            device.destroy_framebuffer(framebuffer, None);
            device.destroy_render_pass(render_pass, None);
            device.destroy_image_view(image_view, None);
            device.free_memory(image_memory, None);
            device.destroy_image(image, None);
        }

        Ok(pixels)
    }
}

/// A list of platform-appropriate C-string extension names — kept as a
/// helper for the day a window surface is added (spec's Phase M1 swapchain);
/// unused by the headless path above, hence `#[allow(dead_code)]`.
#[allow(dead_code)]
fn required_extensions() -> Vec<*const c_char> {
    Vec::new()
}

/// [`VulkanDevice::render_mesh_to_pixels`]'s shader —
/// [`vieww_shaders::library::SOLID_WGSL`] — is vertex-buffer-driven: real
/// per-vertex positions in, a solid fragment colour out. This is the piece
/// `render_clear_to_pixels`'s fullscreen-triangle trick deliberately has no
/// use for (spec's M0/M1 clear-only slice) and that a real GPU compositing
/// path needs first — `docs/RENDERER-V2-NOTES.md`'s "GPU
/// tessellation/GPU-driven execution" gap starts exactly here: a pipeline
/// that actually reads a vertex buffer `vieww-gpu::tessellate` produced,
/// rather than three hardcoded clip-space corners.
impl VulkanDevice {
    /// Render a real tessellated mesh — `vieww_gpu::tessellate`'s
    /// output, though this function takes plain slices rather than
    /// depending on that crate, so `vieww-hal` gains no new dependency for
    /// it — as one solid-coloured draw call, and read the result back as
    /// straight-alpha RGBA8.
    ///
    /// This is the direct continuation of `render_clear_to_pixels`'s
    /// pipeline (same instance/device, same descriptor/shader/pipeline
    /// shape) with the one piece that function deliberately omits: a real
    /// vertex buffer, index buffer, and indexed draw call, so this is
    /// genuinely a GPU rasterizing caller-supplied geometry rather than a
    /// hardcoded clip-space triangle.
    ///
    /// `positions` are in the same pixel space `width`/`height` describe
    /// (origin top-left); `indices` index into `positions` as a triangle
    /// list (three indices per triangle, matching
    /// `vieww_gpu::tessellate::Mesh`'s own layout).
    ///
    /// # Errors
    ///
    /// Any Vulkan call failing — most commonly, in an environment with no
    /// ICD, at instance/device creation before this method is even
    /// reachable (see [`VulkanDevice::new`]).
    pub fn render_mesh_to_pixels(
        &self,
        width: u32,
        height: u32,
        positions: &[[f32; 2]],
        indices: &[u32],
        color: Color,
    ) -> Result<Vec<u8>, VulkanError> {
        if positions.is_empty() || indices.is_empty() {
            // A degenerate mesh draws nothing — return a fully transparent
            // frame rather than asking Vulkan to bind a zero-length buffer,
            // which some drivers reject outright.
            return Ok(vec![0u8; (width as usize) * (height as usize) * 4]);
        }

        let device = &self.device;
        let format = vk::Format::R8G8B8A8_UNORM;

        // --- color attachment image -------------------------------------------------
        let image_info = vk::ImageCreateInfo::default()
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
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = unsafe { device.create_image(&image_info, None) }?;
        let mem_req = unsafe { device.get_image_memory_requirements(image) };
        let mem_type = self.find_memory_type(
            mem_req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let image_mem_info = vk::MemoryAllocateInfo::default()
            .allocation_size(mem_req.size)
            .memory_type_index(mem_type);
        let image_memory = unsafe { device.allocate_memory(&image_mem_info, None) }?;
        unsafe { device.bind_image_memory(image, image_memory, 0) }?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let image_view = unsafe { device.create_image_view(&view_info, None) }?;

        // --- render pass + framebuffer -----------------------------------------------
        let attachment = vk::AttachmentDescription::default()
            .format(format)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        let attachments = [attachment];
        let color_ref = [vk::AttachmentReference {
            attachment: 0,
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        }];
        let subpass = vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&color_ref);
        let subpasses = [subpass];
        let render_pass_info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses);
        let render_pass = unsafe { device.create_render_pass(&render_pass_info, None) }?;

        let fb_attachments = [image_view];
        let fb_info = vk::FramebufferCreateInfo::default()
            .render_pass(render_pass)
            .attachments(&fb_attachments)
            .width(width)
            .height(height)
            .layers(1);
        let framebuffer = unsafe { device.create_framebuffer(&fb_info, None) }?;

        // --- vertex + index buffers ----------------------------------------------------
        let (vertex_buffer, vertex_memory) = self.create_host_visible_buffer(
            vk::BufferUsageFlags::VERTEX_BUFFER,
            bytemuck_cast_positions(positions),
        )?;
        let (index_buffer, index_memory) = self.create_host_visible_buffer(
            vk::BufferUsageFlags::INDEX_BUFFER,
            bytemuck_cast_u32(indices),
        )?;

        // --- uniform buffer (color + viewport) ------------------------------------------
        let straight = [
            f32::from(color.r) / 255.0,
            f32::from(color.g) / 255.0,
            f32::from(color.b) / 255.0,
            f32::from(color.a) / 255.0,
            width as f32,
            height as f32,
            0.0,
            0.0,
        ];
        let (ubo, ubo_memory) = self.create_host_visible_buffer(
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            bytemuck_cast_f32(&straight),
        )?;
        let ubo_size = std::mem::size_of_val(&straight) as u64;

        // --- descriptor set -------------------------------------------------------------
        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let set_layout = unsafe { device.create_descriptor_set_layout(&layout_info, None) }?;

        let pool_size = vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 1,
        };
        let pool_sizes = [pool_size];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(1);
        let descriptor_pool = unsafe { device.create_descriptor_pool(&pool_info, None) }?;
        let set_layouts = [set_layout];
        let set_alloc = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(descriptor_pool)
            .set_layouts(&set_layouts);
        let descriptor_set = unsafe { device.allocate_descriptor_sets(&set_alloc) }?[0];

        let buffer_info = vk::DescriptorBufferInfo {
            buffer: ubo,
            offset: 0,
            range: ubo_size,
        };
        let buffer_infos = [buffer_info];
        let write = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(&buffer_infos);
        unsafe { device.update_descriptor_sets(&[write], &[]) };

        // --- shaders + pipeline -----------------------------------------------------------
        let vs_spirv = translate_wgsl_to_spirv(
            vieww_shaders::library::SOLID_WGSL,
            "vs_main",
            naga::ShaderStage::Vertex,
        )?;
        let fs_spirv = translate_wgsl_to_spirv(
            vieww_shaders::library::SOLID_WGSL,
            "fs_main",
            naga::ShaderStage::Fragment,
        )?;
        let vs_info = vk::ShaderModuleCreateInfo::default().code(&vs_spirv);
        let fs_info = vk::ShaderModuleCreateInfo::default().code(&fs_spirv);
        let vs_module = unsafe { device.create_shader_module(&vs_info, None) }?;
        let fs_module = unsafe { device.create_shader_module(&fs_info, None) }?;

        let entry_point = CString::new("vs_main").unwrap();
        let fs_entry_point = CString::new("fs_main").unwrap();
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs_module)
                .name(&entry_point),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs_module)
                .name(&fs_entry_point),
        ];

        let binding_desc = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: std::mem::size_of::<[f32; 2]>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let attr_desc = [vk::VertexInputAttributeDescription {
            location: 0,
            binding: 0,
            format: vk::Format::R32G32_SFLOAT,
            offset: 0,
        }];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&binding_desc)
            .vertex_attribute_descriptions(&attr_desc);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width, height },
        };
        let viewports = [viewport];
        let scissors = [scissor];
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Straight (non-premultiplied) alpha blend — matches this crate's
        // own `render_clear_to_pixels`/readback contract ("straight-alpha
        // RGBA8"), so a caller does not have to remember two different
        // alpha conventions depending on which method it called.
        let blend_attachment = vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA);
        let blend_attachments = [blend_attachment];
        let color_blend =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments);

        let pipeline_layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        let pipeline_layout =
            unsafe { device.create_pipeline_layout(&pipeline_layout_info, None) }?;

        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&color_blend)
            .layout(pipeline_layout)
            .render_pass(render_pass)
            .subpass(0);
        let pipelines = unsafe {
            device.create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
        }
        .map_err(|(_, e)| VulkanError::Vulkan(e.to_string()))?;
        let pipeline = pipelines[0];

        // --- readback buffer -----------------------------------------------------------
        let readback_size = (width as u64) * (height as u64) * 4;
        let readback_info = vk::BufferCreateInfo::default()
            .size(readback_size)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let readback_buffer = unsafe { device.create_buffer(&readback_info, None) }?;
        let readback_req = unsafe { device.get_buffer_memory_requirements(readback_buffer) };
        let readback_mem_type = self.find_memory_type(
            readback_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let readback_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(readback_req.size)
            .memory_type_index(readback_mem_type);
        let readback_memory = unsafe { device.allocate_memory(&readback_alloc, None) }?;
        unsafe { device.bind_buffer_memory(readback_buffer, readback_memory, 0) }?;

        // --- command buffer -----------------------------------------------------------
        let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(self.queue_family);
        let command_pool = unsafe { device.create_command_pool(&pool_info, None) }?;
        let cmd_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = unsafe { device.allocate_command_buffers(&cmd_alloc) }?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe { device.begin_command_buffer(command_buffer, &begin_info) }?;

        let clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 0.0],
            },
        };
        let clear_values = [clear_value];
        let rp_begin = vk::RenderPassBeginInfo::default()
            .render_pass(render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D { width, height },
            })
            .clear_values(&clear_values);
        unsafe {
            device.cmd_begin_render_pass(command_buffer, &rp_begin, vk::SubpassContents::INLINE);
            device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline);
            device.cmd_bind_descriptor_sets(
                command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline_layout,
                0,
                &[descriptor_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(command_buffer, 0, &[vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(command_buffer, index_buffer, 0, vk::IndexType::UINT32);
            device.cmd_draw_indexed(command_buffer, indices.len() as u32, 1, 0, 0, 0);
            device.cmd_end_render_pass(command_buffer);

            let copy_region = vk::BufferImageCopy {
                buffer_offset: 0,
                buffer_row_length: 0,
                buffer_image_height: 0,
                image_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
                image_extent: vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                },
            };
            device.cmd_copy_image_to_buffer(
                command_buffer,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                readback_buffer,
                &[copy_region],
            );
        }
        unsafe { device.end_command_buffer(command_buffer) }?;

        let fence_info = vk::FenceCreateInfo::default();
        let fence = unsafe { device.create_fence(&fence_info, None) }?;
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default().command_buffers(&command_buffers);
        unsafe {
            device.queue_submit(self.queue, &[submit_info], fence)?;
            device.wait_for_fences(&[fence], true, u64::MAX)?;
        }

        let mut pixels = vec![0u8; readback_size as usize];
        unsafe {
            let ptr = device.map_memory(
                readback_memory,
                0,
                readback_size,
                vk::MemoryMapFlags::empty(),
            )? as *const u8;
            std::ptr::copy_nonoverlapping(ptr, pixels.as_mut_ptr(), readback_size as usize);
            device.unmap_memory(readback_memory);
        }

        unsafe {
            device.destroy_fence(fence, None);
            device.destroy_command_pool(command_pool, None);
            device.destroy_pipeline(pipeline, None);
            device.destroy_pipeline_layout(pipeline_layout, None);
            device.destroy_shader_module(vs_module, None);
            device.destroy_shader_module(fs_module, None);
            device.destroy_descriptor_pool(descriptor_pool, None);
            device.destroy_descriptor_set_layout(set_layout, None);
            device.free_memory(readback_memory, None);
            device.destroy_buffer(readback_buffer, None);
            device.free_memory(ubo_memory, None);
            device.destroy_buffer(ubo, None);
            device.free_memory(index_memory, None);
            device.destroy_buffer(index_buffer, None);
            device.free_memory(vertex_memory, None);
            device.destroy_buffer(vertex_buffer, None);
            device.destroy_framebuffer(framebuffer, None);
            device.destroy_render_pass(render_pass, None);
            device.destroy_image_view(image_view, None);
            device.free_memory(image_memory, None);
            device.destroy_image(image, None);
        }

        Ok(pixels)
    }

    /// Allocate a host-visible, host-coherent buffer of `usage`, upload
    /// `data` into it immediately, and return the buffer and its backing
    /// memory (owned by the caller — freed alongside everything else at the
    /// end of whichever `render_*` call created it, matching the existing
    /// uniform/readback buffers' lifetime pattern above).
    ///
    /// Host-visible rather than staged through a device-local buffer via a
    /// transfer queue: correct and simple, at the cost of being the wrong
    /// choice for a large, frequently-updated vertex buffer on a discrete
    /// GPU (a real per-frame path would stage through
    /// `vieww_gpu::resources::UploadQueue` instead) — exactly the kind of
    /// tradeoff `vieww-gpu`'s own resource-system docs call out as this
    /// crate's job to eventually replace, not this one-shot render
    /// function's.
    fn create_host_visible_buffer(
        &self,
        usage: vk::BufferUsageFlags,
        data: &[u8],
    ) -> Result<(vk::Buffer, vk::DeviceMemory), VulkanError> {
        let device = &self.device;
        let size = data.len() as u64;
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = unsafe { device.create_buffer(&info, None) }?;
        let req = unsafe { device.get_buffer_memory_requirements(buffer) };
        let mem_type = self.find_memory_type(
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(mem_type);
        let memory = unsafe { device.allocate_memory(&alloc, None) }?;
        unsafe { device.bind_buffer_memory(buffer, memory, 0) }?;
        unsafe {
            let ptr = device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty())? as *mut u8;
            ptr.copy_from_nonoverlapping(data.as_ptr(), data.len());
            device.unmap_memory(memory);
        }
        Ok((buffer, memory))
    }
}

fn bytemuck_cast_positions(positions: &[[f32; 2]]) -> &[u8] {
    // SAFETY: `[f32; 2]` has no padding and no alignment requirement beyond
    // 4 bytes, which `u8` satisfies trivially — reinterpreting a `&[[f32;
    // 2]]` as `&[u8]` of `positions.len() * 8` bytes is exactly what
    // `bytemuck::cast_slice` does, inlined here so this module gains no new
    // dependency for one cast used in only two call sites.
    unsafe {
        std::slice::from_raw_parts(
            positions.as_ptr().cast::<u8>(),
            std::mem::size_of_val(positions),
        )
    }
}

fn bytemuck_cast_u32(values: &[u32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn bytemuck_cast_f32(values: &[f32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}
