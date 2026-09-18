//! Real windowed presentation for `VulkanDevice` — `VK_KHR_surface`
//! + `VK_KHR_swapchain`, raw Vulkan throughout. Requires the
//! `vulkan-swapchain` feature.
//!
//! This is the piece the crate's top-level docs used to call out as missing
//! ("It does **not** yet ... spec §14.1's M0/M1 slice, minus the swapchain").
//! It is now real: [`VulkanDevice::for_window`] opens a `VK_KHR_surface` on
//! whatever [`raw_window_handle`] handle it is given (a `winit::window::Window`
//! satisfies the trait bound directly — `winit` 0.30 implements
//! `raw-window-handle` 0.6, the same generation this module depends on),
//! builds a device with `VK_KHR_swapchain` enabled, and
//! [`VulkanDevice::present_pixels`] uploads a straight-alpha RGBA8 buffer
//! (exactly what `vieww_paint::native::NativeRenderer::render_to_pixels`
//! produces) onto the next swapchain image with `vkCmdCopyBufferToImage`, no
//! sampler, no shader, no `wgpu` — this backend has no compositing GPU work
//! of its own yet (that is spec §14.1's M2, tessellation-on-GPU), only a
//! correctly-synchronised way to get CPU-rasterised pixels onto a screen.
//!
//! # Simple over fast, deliberately
//!
//! Every [`VulkanDevice::present_pixels`] call allocates and destroys its own
//! staging buffer and command pool and ends with a full `vkQueueWaitIdle` —
//! the same "create everything fresh, wait, destroy everything" shape
//! `VulkanDevice::render_clear_to_pixels` already uses. That gives
//! up frame pipelining (double/triple buffering, present without a host
//! stall) in exchange for an implementation with no in-flight-resource
//! lifetime to get wrong on a first landing. Recorded here rather than left
//! implicit: pipelining `present_pixels` is real follow-up work, not a
//! silently-accepted permanent shape.
//!
//! # Format handling
//!
//! A swapchain's available formats are queried, not assumed: this module
//! accepts either `R8G8B8A8_UNORM` (a byte-for-byte copy of
//! `vieww_paint::native::Pixels`' own layout) or `B8G8R8A8_UNORM` (the
//! common case on Linux/X11 and Windows), swapping the R and B bytes into the
//! staging buffer when the surface only offers the latter. Both are non-sRGB
//! — an sRGB swapchain format would gamma-correct pixels the CPU rasterizer
//! already computed in the right space, double-applying the curve, so this
//! module never picks one.

use std::ffi::{c_char, CStr};

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use super::{VulkanDevice, VulkanError};

/// Which byte order the staging buffer must be written in to match the
/// swapchain's chosen surface format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelOrder {
    /// `vieww_paint::native::Pixels`' own layout — copy straight through.
    Rgba,
    /// Swap R and B while copying into the staging buffer.
    Bgra,
}

/// A window's swapchain: the surface, the swapchain itself, its images, and
/// what [`VulkanDevice::present_pixels`] needs to know to write into one.
pub struct VulkanSwapchain {
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    swapchain_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    format: vk::Format,
    order: ChannelOrder,
    width: u32,
    height: u32,
    /// Reused across frames — safe only because every
    /// [`VulkanDevice::present_pixels`] call `vkQueueWaitIdle`s before
    /// returning, so the semaphore is never re-signalled before its previous
    /// signal was consumed. See the module docs on "simple over fast".
    image_available: vk::Semaphore,
}

impl std::fmt::Debug for VulkanSwapchain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VulkanSwapchain")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("images", &self.images.len())
            .finish_non_exhaustive()
    }
}

impl VulkanDevice {
    /// Open a window and a device to present into it — the swapchain half of
    /// spec §14.1's M0/M1 slice.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Loading`] if no Vulkan loader is present,
    /// [`VulkanError::NoAdapter`] if no physical device offers both a
    /// graphics queue and presentation support on this surface, and
    /// [`VulkanError::Vulkan`] for any other Vulkan call that fails —
    /// including "no surface format we can write" and "surface offers no
    /// present mode", both surfaced as that variant since a broken surface
    /// enumeration is exactly as unrecoverable as any other Vulkan error
    /// here.
    ///
    /// # Panics
    ///
    /// If `width` or `height` is zero.
    pub fn for_window(
        window: &(impl HasWindowHandle + HasDisplayHandle),
        width: u32,
        height: u32,
    ) -> Result<(Self, VulkanSwapchain), VulkanError> {
        assert!(
            width > 0 && height > 0,
            "cannot present into a {width}x{height} window"
        );

        let display_handle = window
            .display_handle()
            .map_err(|e| VulkanError::Vulkan(format!("no display handle: {e}")))?
            .as_raw();
        let window_handle = window
            .window_handle()
            .map_err(|e| VulkanError::Vulkan(format!("no window handle: {e}")))?
            .as_raw();

        let entry = super::load_entry()?;

        let required = ash_window::enumerate_required_extensions(display_handle)
            .map_err(|e| VulkanError::Vulkan(format!("required surface extensions: {e}")))?;
        let instance = Self::create_instance(&entry, required)?;

        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = unsafe {
            ash_window::create_surface(&entry, &instance, display_handle, window_handle, None)
        }
        .map_err(|e| VulkanError::Vulkan(format!("creating surface: {e}")))?;

        let (physical_device, queue_family) = Self::pick_physical_device(&instance, |pd| {
            let graphics = Self::find_graphics_family(&instance, pd)?;
            let supports_present = unsafe {
                surface_loader.get_physical_device_surface_support(pd, graphics, surface)
            }
            .unwrap_or(false);
            supports_present.then_some(graphics)
        })
        .ok_or_else(|| {
            unsafe {
                surface_loader.destroy_surface(surface, None);
                instance.destroy_instance(None);
            }
            VulkanError::NoAdapter
        })?;

        let info = Self::adapter_info(&instance, physical_device);
        let swapchain_ext = [ash::khr::swapchain::NAME.as_ptr()];
        let (device, queue) =
            Self::create_device(&instance, physical_device, queue_family, &swapchain_ext)?;

        let device_bundle = Self {
            entry,
            instance,
            physical_device,
            device,
            queue,
            queue_family,
            info,
        };

        let swapchain =
            VulkanSwapchain::create(&device_bundle, surface_loader, surface, width, height)?;

        Ok((device_bundle, swapchain))
    }

    /// A second window's swapchain, on **this** device.
    ///
    /// # Why this exists, which is a crash rather than a tidiness argument
    ///
    /// [`for_window`](Self::for_window) opens a `VkInstance` and a `VkDevice`
    /// of its own, so an application with a dialog or a menu window had one
    /// complete driver per window. Closing one window then ran
    /// `vkDestroyDevice` and `vkDestroyInstance` while the other windows'
    /// instances were still live, and a driver's per-process state does not
    /// survive that: NVIDIA's jumped through a null pointer inside the *next*
    /// window's `vkDestroySwapchainKHR`, and Mesa's Intel driver reported a
    /// double free at the same moment. Measured on a GeForce 920MX and on
    /// Kaby Lake: the desktop suite died on signal 11 both times, and ran to
    /// 21 checks with 0 failed as soon as nothing was destroyed.
    ///
    /// So every window after the first asks the device that already exists
    /// for a surface. One instance, one device, one driver per process —
    /// which is also what every other toolkit does, and what makes opening a
    /// window cost a surface rather than a driver.
    ///
    /// # Errors
    ///
    /// [`VulkanError::NoAdapter`] if this device's adapter cannot present to
    /// this window — a caller that gets it should fall back to
    /// [`for_window`](Self::for_window) — and [`VulkanError::Vulkan`] for any
    /// other failing Vulkan call.
    ///
    /// # Panics
    ///
    /// If `width` or `height` is zero.
    pub fn swapchain_for_window(
        &self,
        window: &(impl HasWindowHandle + HasDisplayHandle),
        width: u32,
        height: u32,
    ) -> Result<VulkanSwapchain, VulkanError> {
        assert!(
            width > 0 && height > 0,
            "cannot present into a {width}x{height} window"
        );

        let display_handle = window
            .display_handle()
            .map_err(|e| VulkanError::Vulkan(format!("no display handle: {e}")))?
            .as_raw();
        let window_handle = window
            .window_handle()
            .map_err(|e| VulkanError::Vulkan(format!("no window handle: {e}")))?
            .as_raw();

        let surface_loader = ash::khr::surface::Instance::new(&self.entry, &self.instance);
        let surface = unsafe {
            ash_window::create_surface(
                &self.entry,
                &self.instance,
                display_handle,
                window_handle,
                None,
            )
        }
        .map_err(|e| VulkanError::Vulkan(format!("creating surface: {e}")))?;

        // This adapter was chosen for another window's surface. Usually the
        // same answer; not guaranteed, so it is asked rather than assumed, and
        // the surface is destroyed again rather than leaked on a no.
        let supported = unsafe {
            surface_loader.get_physical_device_surface_support(
                self.physical_device,
                self.queue_family,
                surface,
            )
        }
        .unwrap_or(false);
        if !supported {
            unsafe { surface_loader.destroy_surface(surface, None) };
            return Err(VulkanError::NoAdapter);
        }

        VulkanSwapchain::create(self, surface_loader, surface, width, height)
    }

    /// Upload `pixels` (tightly packed straight-alpha RGBA8, exactly
    /// `vieww_paint::native::Pixels::data`'s layout) and present it as the
    /// next frame of `swapchain`. Always a full-frame copy — see the module
    /// docs on why there is no partial/damage-aware path yet.
    ///
    /// # Errors
    ///
    /// [`VulkanError::Vulkan`] if the image could not be acquired even after
    /// one swapchain rebuild (ordinary during a resize race, not ordinary
    /// otherwise), or if `pixels.len() != width * height * 4` where `width`
    /// and `height` are `swapchain`'s current dimensions.
    pub fn present_pixels(
        &self,
        swapchain: &mut VulkanSwapchain,
        pixels: &[u8],
    ) -> Result<(), VulkanError> {
        let expected = (swapchain.width as usize) * (swapchain.height as usize) * 4;
        if pixels.len() != expected {
            return Err(VulkanError::Vulkan(format!(
                "present_pixels: expected {expected} bytes for a {}x{} frame, got {}",
                swapchain.width,
                swapchain.height,
                pixels.len()
            )));
        }

        let (image_index, suboptimal) = match unsafe {
            swapchain.swapchain_loader.acquire_next_image(
                swapchain.swapchain,
                u64::MAX,
                swapchain.image_available,
                vk::Fence::null(),
            )
        } {
            Ok(result) => result,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                swapchain.recreate(self, swapchain.width, swapchain.height)?;
                unsafe {
                    swapchain.swapchain_loader.acquire_next_image(
                        swapchain.swapchain,
                        u64::MAX,
                        swapchain.image_available,
                        vk::Fence::null(),
                    )
                }
                .map_err(|e| VulkanError::Vulkan(format!("acquire after rebuild: {e:?}")))?
            }
            Err(e) => {
                return Err(VulkanError::Vulkan(format!(
                    "acquiring swapchain image: {e:?}"
                )))
            }
        };
        let _ = suboptimal; // Reported as-is next present_pixels call via ERROR_OUT_OF_DATE_KHR/present's own bool; not tracked separately.

        let image = swapchain.images[image_index as usize];

        // --- staging buffer, byte-order-converted if the swapchain is BGRA ---
        let byte_len = pixels.len() as u64;
        let staging_info = vk::BufferCreateInfo::default()
            .size(byte_len)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging = unsafe { self.device.create_buffer(&staging_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        let req = unsafe { self.device.get_buffer_memory_requirements(staging) };
        let mem_type = self.find_memory_type(
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(mem_type);
        let memory = unsafe { self.device.allocate_memory(&alloc_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        unsafe { self.device.bind_buffer_memory(staging, memory, 0) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        unsafe {
            let ptr = self
                .device
                .map_memory(memory, 0, byte_len, vk::MemoryMapFlags::empty())?;
            match swapchain.order {
                ChannelOrder::Rgba => {
                    std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr.cast::<u8>(), pixels.len());
                }
                ChannelOrder::Bgra => {
                    let dst = std::slice::from_raw_parts_mut(ptr.cast::<u8>(), pixels.len());
                    for (src, dst) in pixels.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                        dst[0] = src[2];
                        dst[1] = src[1];
                        dst[2] = src[0];
                        dst[3] = src[3];
                    }
                }
            }
            self.device.unmap_memory(memory);
        }

        // --- command buffer: transition -> copy -> transition ---
        let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(self.queue_family);
        let command_pool = unsafe { self.device.create_command_pool(&pool_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        let cmd_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(command_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = unsafe { self.device.allocate_command_buffers(&cmd_alloc) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?[0];

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        unsafe {
            self.device
                .begin_command_buffer(command_buffer, &begin_info)
        }
        .map_err(|e| VulkanError::Vulkan(e.to_string()))?;

        let subresource = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let to_transfer_dst = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer_dst],
            );
        }

        let region = vk::BufferImageCopy {
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
                width: swapchain.width,
                height: swapchain.height,
                depth: 1,
            },
        };
        unsafe {
            self.device.cmd_copy_buffer_to_image(
                command_buffer,
                staging,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }

        let to_present = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(subresource)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::empty());
        unsafe {
            self.device.cmd_pipeline_barrier(
                command_buffer,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present],
            );
        }
        unsafe { self.device.end_command_buffer(command_buffer) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;

        // Wait on `image_available` before the copy executes on the device —
        // acquiring an image is not itself a guarantee the presentation
        // engine is done reading it. See the module docs for why a fence and
        // `vkQueueWaitIdle` (rather than a second, present-side semaphore)
        // are how this module knows it is safe to present afterwards.
        let wait_semaphores = [swapchain.image_available];
        let wait_stages = [vk::PipelineStageFlags::TRANSFER];
        let command_buffers = [command_buffer];
        let submit_info = vk::SubmitInfo::default()
            .wait_semaphores(&wait_semaphores)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&command_buffers);
        unsafe {
            self.device
                .queue_submit(self.queue, &[submit_info], vk::Fence::null())
                .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
            // Synchronous, on purpose — see the module docs' "simple over
            // fast" section. Once this returns, the copy (and its wait on
            // `image_available`) is fully retired, so presenting needs no
            // further wait semaphore.
            self.device
                .queue_wait_idle(self.queue)
                .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        }

        unsafe {
            self.device.destroy_command_pool(command_pool, None);
            self.device.destroy_buffer(staging, None);
            self.device.free_memory(memory, None);
        }

        let swapchains = [swapchain.swapchain];
        let image_indices = [image_index];
        let present_info = vk::PresentInfoKHR::default()
            .swapchains(&swapchains)
            .image_indices(&image_indices);
        match unsafe {
            swapchain
                .swapchain_loader
                .queue_present(self.queue, &present_info)
        } {
            Ok(_suboptimal) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                // The next present_pixels call's acquire will see this too
                // and rebuild; a stale frame on a resize race is not a
                // caller-visible error.
                Ok(())
            }
            Err(e) => Err(VulkanError::Vulkan(format!("presenting: {e:?}"))),
        }
    }
}

impl VulkanSwapchain {
    fn create(
        device: &VulkanDevice,
        surface_loader: ash::khr::surface::Instance,
        surface: vk::SurfaceKHR,
        width: u32,
        height: u32,
    ) -> Result<Self, VulkanError> {
        let capabilities = unsafe {
            surface_loader.get_physical_device_surface_capabilities(device.physical_device, surface)
        }
        .map_err(|e| VulkanError::Vulkan(format!("surface capabilities: {e}")))?;
        let formats = unsafe {
            surface_loader.get_physical_device_surface_formats(device.physical_device, surface)
        }
        .map_err(|e| VulkanError::Vulkan(format!("surface formats: {e}")))?;
        let present_modes = unsafe {
            surface_loader
                .get_physical_device_surface_present_modes(device.physical_device, surface)
        }
        .map_err(|e| VulkanError::Vulkan(format!("surface present modes: {e}")))?;

        let (format, order) = formats
            .iter()
            .find_map(|f| match f.format {
                vk::Format::R8G8B8A8_UNORM => Some((f.format, ChannelOrder::Rgba)),
                _ => None,
            })
            .or_else(|| {
                formats.iter().find_map(|f| match f.format {
                    vk::Format::B8G8R8A8_UNORM => Some((f.format, ChannelOrder::Bgra)),
                    _ => None,
                })
            })
            .ok_or_else(|| {
                VulkanError::Vulkan("surface offers no R8G8B8A8/B8G8R8A8 UNORM format".into())
            })?;

        // FIFO is the one present mode every conformant implementation must
        // support (vsync — a UI has no reason to present faster than the
        // display refreshes, same choice `vieww_paint::gpu`'s
        // `PresentMode::AutoVsync` makes); MAILBOX if it happens to be
        // offered, for lower latency without tearing.
        let present_mode = present_modes
            .iter()
            .copied()
            .find(|&m| m == vk::PresentModeKHR::MAILBOX)
            .unwrap_or(vk::PresentModeKHR::FIFO);

        let extent = if capabilities.current_extent.width != u32::MAX {
            capabilities.current_extent
        } else {
            vk::Extent2D {
                width: width.clamp(
                    capabilities.min_image_extent.width,
                    capabilities.max_image_extent.width,
                ),
                height: height.clamp(
                    capabilities.min_image_extent.height,
                    capabilities.max_image_extent.height,
                ),
            }
        };

        let mut image_count = capabilities.min_image_count + 1;
        if capabilities.max_image_count > 0 {
            image_count = image_count.min(capabilities.max_image_count);
        }

        let swapchain_loader = ash::khr::swapchain::Device::new(&device.instance, &device.device);
        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true);
        let swapchain = unsafe { swapchain_loader.create_swapchain(&create_info, None) }
            .map_err(|e| VulkanError::Vulkan(format!("creating swapchain: {e}")))?;
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain) }
            .map_err(|e| VulkanError::Vulkan(format!("swapchain images: {e}")))?;

        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let image_available = unsafe { device.device.create_semaphore(&semaphore_info, None) }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;

        Ok(Self {
            surface_loader,
            surface,
            swapchain_loader,
            swapchain,
            images,
            format,
            order,
            width: extent.width,
            height: extent.height,
            image_available,
        })
    }

    /// Rebuild the swapchain for a new size, or after
    /// `ERROR_OUT_OF_DATE_KHR`. The window's contents are gone after this —
    /// the next `present_pixels` repaints in full, which it always does
    /// anyway (see the module docs).
    ///
    /// # Errors
    ///
    /// [`VulkanError::Vulkan`] if the surface no longer offers a usable
    /// format or present mode (the window was likely closed or the surface
    /// was otherwise invalidated).
    pub fn recreate(
        &mut self,
        device: &VulkanDevice,
        width: u32,
        height: u32,
    ) -> Result<(), VulkanError> {
        // `device_wait_idle`, not `queue_wait_idle`: every queue has to be
        // done with the images, and a second queue is one refactor away.
        unsafe { device.device.device_wait_idle() }
            .map_err(|e| VulkanError::Vulkan(e.to_string()))?;
        let surface_loader = self.surface_loader.clone();
        let surface = self.surface;
        self.destroy_swapchain_only(device);
        let rebuilt = Self::create(device, surface_loader, surface, width, height)?;
        *self = rebuilt;
        Ok(())
    }

    /// Current swapchain dimensions — what `present_pixels` requires the
    /// uploaded buffer to match.
    #[must_use]
    pub fn extent(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// **The swapchain goes first, then the semaphore.** `image_available` is
    /// the semaphore `acquire_next_image` signals, so an acquire that was
    /// issued and never waited on — every error path between the acquire and
    /// the submit in `present_pixels` leaves one — still has a signal pending
    /// against it. Destroying a semaphore in that state is undefined, and
    /// destroying the swapchain first is what cancels the acquire that owns
    /// it. The old order crashed NVIDIA's driver inside
    /// `vkDestroySwapchainKHR` — a jump through a null pointer in
    /// `libnvidia-glcore` — when the desktop suite closed a window.
    fn destroy_swapchain_only(&mut self, device: &VulkanDevice) {
        unsafe {
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
            device.device.destroy_semaphore(self.image_available, None);
        }
    }
}

/// Dropping a [`VulkanSwapchain`] on its own (rather than through
/// `recreate`) cannot reach the owning [`VulkanDevice`] to wait idle or
/// destroy the surface — see [`VulkanSwapchain::destroy`] for the version
/// that does both, and drop that explicitly before the device goes away.
impl VulkanSwapchain {
    /// Destroy this swapchain and its surface. Takes `device` because
    /// neither the swapchain nor the surface can be destroyed safely without
    /// first waiting for the device to go idle, and a plain [`Drop`] impl has
    /// no way to reach it.
    ///
    /// # Panics
    ///
    /// If waiting for the device to go idle fails — meaning the device is
    /// already lost, in which case there is nothing left to destroy safely
    /// anyway.
    pub fn destroy(mut self, device: &VulkanDevice) {
        unsafe { device.device.device_wait_idle() }.expect("device idle before teardown");
        self.release(device);
    }

    /// [`destroy`](Self::destroy) through `&mut self`, for an owner's `Drop`
    /// impl — which cannot move the swapchain out to call `destroy`.
    ///
    /// Idempotent: the handles are nulled, so a second call (or a later
    /// `destroy`) does nothing. Waits for the queue to go idle but does not
    /// panic if it cannot, because it runs during teardown where a lost
    /// device is the one thing left to tolerate.
    ///
    /// # Why an owner must call this before the device and window go
    ///
    /// Nothing called `destroy`. A window's teardown dropped the winit window
    /// (destroying the `wl_surface`/`HWND`/`NSView`), then the `VulkanDevice`
    /// (`vkDestroyDevice` + `vkDestroyInstance` with the swapchain and
    /// `VkSurfaceKHR` still alive), and only then this struct, which has no
    /// `Drop`. That is undefined behaviour the spec's validation layer names,
    /// and drivers differ in how they punish it: lavapipe on Xvfb shrugs, while
    /// Mesa's Intel driver on Wayland segfaulted the desktop suite on a real
    /// laptop the moment a dialog window closed.
    pub fn release(&mut self, device: &VulkanDevice) {
        if self.swapchain == vk::SwapchainKHR::null() && self.surface == vk::SurfaceKHR::null() {
            return;
        }
        let _ = unsafe { device.device.device_wait_idle() };
        if self.swapchain != vk::SwapchainKHR::null() {
            self.destroy_swapchain_only(device);
            self.swapchain = vk::SwapchainKHR::null();
            self.image_available = vk::Semaphore::null();
            self.images.clear();
        }
        if self.surface != vk::SurfaceKHR::null() {
            unsafe {
                self.surface_loader.destroy_surface(self.surface, None);
            }
            self.surface = vk::SurfaceKHR::null();
        }
    }
}

#[allow(dead_code)]
fn required_extension_names_are_c_strings(names: &[*const c_char]) -> Vec<&CStr> {
    // Not called anywhere: exists only so a reviewer can see, in one place,
    // that `ash_window::enumerate_required_extensions`'s return type really
    // is the same `&[*const c_char]` shape `create_instance` already takes.
    names
        .iter()
        .map(|&p| unsafe { CStr::from_ptr(p) })
        .collect()
}
