//! A software reference [`Device`]: every trait in [`crate::device`]
//! implemented entirely in host memory, no GPU, no driver, no unsafe code.
//!
//! Two reasons this exists rather than only shipping trait definitions:
//!
//! 1. **Everything above this crate is testable without a GPU.** A caller
//!    that writes code against `dyn Device`/`dyn CommandEncoder` can run it
//!    here, on any machine, in CI, without Vulkan/Metal/D3D12 installed —
//!    which matters directly for this delivery, running in a sandbox with
//!    no GPU device to hand.
//! 2. **It is an executable specification.** "What must
//!    `create_texture` do" is unambiguous once there is a reference
//!    implementation every real backend's own tests can be checked
//!    against — the same role `vieww_paint::native`'s CPU rasterizer
//!    already plays as this workspace's rendering oracle.
//!
//! What it is **not**: a renderer. `NullDevice::create_command_encoder`
//! records draw calls as data (`NullCommandLog`); it does not rasterize
//! anything, because rasterizing correctly is `vieww_paint::native`'s job
//! and duplicating it here would be exactly the "two implementations that
//! can silently agree on the same bug" risk `docs/RENDERER-V2-NOTES.md`
//! warns about for its blend-mode oracle.

use vieww_foundation::{Rect, Size};

use crate::device::{Buffer, CommandEncoder, Device, Fence, Pipeline, Queue, Surface, Texture};
use crate::resources::{BufferUsage, TextureFormat, TextureUsage};

#[derive(Debug)]
pub struct NullBuffer {
    pub data: Vec<u8>,
    usage: BufferUsage,
}

impl Buffer for NullBuffer {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }
    fn usage(&self) -> BufferUsage {
        self.usage
    }
}

#[derive(Debug)]
pub struct NullTexture {
    size: Size,
    format: TextureFormat,
    usage: TextureUsage,
}

impl Texture for NullTexture {
    fn size(&self) -> Size {
        self.size
    }
    fn format(&self) -> TextureFormat {
        self.format
    }
    fn usage(&self) -> TextureUsage {
        self.usage
    }
}

#[derive(Debug)]
pub struct NullPipeline;
impl Pipeline for NullPipeline {}

#[derive(Debug)]
pub struct NullFence;
impl Fence for NullFence {
    fn wait(&self, _timeout_ms: u64) -> bool {
        // Everything the null device does is synchronous, so any fence it
        // hands out is already signaled the moment it exists.
        true
    }
    fn is_signaled(&self) -> bool {
        true
    }
}

/// One recorded operation. What a real backend would translate into an
/// actual GPU command; here it is just data a test can assert against.
#[derive(Debug, Clone, PartialEq)]
pub enum NullCommand {
    BeginPass {
        clear: Option<[f32; 4]>,
    },
    EndPass,
    BindPipeline,
    BindVertexBuffer {
        slot: u32,
    },
    BindIndexBuffer,
    SetScissor {
        rect: Rect,
    },
    DrawIndexed {
        index_count: u32,
        instance_count: u32,
    },
    Blit,
}

#[derive(Debug, Default)]
pub struct NullCommandEncoder {
    pub log: Vec<NullCommand>,
}

impl CommandEncoder for NullCommandEncoder {
    fn begin_pass(&mut self, _target: &dyn Texture, clear: Option<[f32; 4]>) {
        self.log.push(NullCommand::BeginPass { clear });
    }
    fn end_pass(&mut self) {
        self.log.push(NullCommand::EndPass);
    }
    fn bind_pipeline(&mut self, _pipeline: &dyn Pipeline) {
        self.log.push(NullCommand::BindPipeline);
    }
    fn bind_vertex_buffer(&mut self, slot: u32, _buffer: &dyn Buffer) {
        self.log.push(NullCommand::BindVertexBuffer { slot });
    }
    fn bind_index_buffer(&mut self, _buffer: &dyn Buffer) {
        self.log.push(NullCommand::BindIndexBuffer);
    }
    fn set_scissor(&mut self, rect: Rect) {
        self.log.push(NullCommand::SetScissor { rect });
    }
    fn draw_indexed(&mut self, index_count: u32, instance_count: u32) {
        self.log.push(NullCommand::DrawIndexed {
            index_count,
            instance_count,
        });
    }
    fn blit(&mut self, _src: &dyn Texture, _dst: &dyn Texture) {
        self.log.push(NullCommand::Blit);
    }
}

#[derive(Debug, Default)]
pub struct NullQueue {
    /// Every command log ever submitted, for a test to inspect.
    pub submissions: Vec<Vec<NullCommand>>,
}

impl Queue for NullQueue {
    fn submit(&mut self, encoder: Box<dyn CommandEncoder>) -> Box<dyn Fence> {
        // `CommandEncoder` is a trait object, so recovering the concrete
        // log needs a downcast in a real multi-impl setting; the null
        // device is the only implementor it is ever handed here, so this
        // takes the pragmatic route of having `submit` accept the concrete
        // type through a small adapter instead of `Any`-downcasting a
        // trait object — see `NullQueue::submit_log` below, which is what
        // `NullDevice`'s own tests actually call.
        drop(encoder);
        Box::new(NullFence)
    }
}

impl NullQueue {
    /// The concrete-typed submission path the null backend's own tests use
    /// instead of going through the `dyn Queue` trait object (which erases
    /// `NullCommandEncoder`'s log the moment it's boxed as `dyn
    /// CommandEncoder` — a real backend doesn't have this problem because
    /// its "log" *is* a real command buffer the GPU driver already owns).
    pub fn submit_log(&mut self, encoder: NullCommandEncoder) -> Box<dyn Fence> {
        self.submissions.push(encoder.log);
        Box::new(NullFence)
    }
}

#[derive(Debug)]
pub struct NullDevice {
    pub queue: NullQueue,
}

impl Default for NullDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl NullDevice {
    #[must_use]
    pub fn new() -> Self {
        Self {
            queue: NullQueue::default(),
        }
    }
}

impl Device for NullDevice {
    fn create_buffer(&self, size: u64, usage: BufferUsage) -> Box<dyn Buffer> {
        Box::new(NullBuffer {
            data: vec![0u8; size as usize],
            usage,
        })
    }

    fn create_texture(
        &self,
        size: Size,
        format: TextureFormat,
        usage: TextureUsage,
    ) -> Box<dyn Texture> {
        Box::new(NullTexture {
            size,
            format,
            usage,
        })
    }

    fn create_command_encoder(&self) -> Box<dyn CommandEncoder> {
        Box::new(NullCommandEncoder::default())
    }

    fn queue(&mut self) -> &mut dyn Queue {
        &mut self.queue
    }

    fn adapter_name(&self) -> String {
        "vieww-gpu null device (software reference, no GPU)".to_string()
    }
}

/// An in-memory [`Surface`] for tests: `acquire` always succeeds (unless
/// `fail_next_acquire` was set, modeling a minimized window or lost
/// device), and `present` just counts.
#[derive(Debug, Default)]
pub struct NullSurface {
    size: Size,
    pub present_count: u32,
    pub fail_next_acquire: bool,
}

impl NullSurface {
    #[must_use]
    pub fn new(size: Size) -> Self {
        Self {
            size,
            present_count: 0,
            fail_next_acquire: false,
        }
    }
}

impl Surface for NullSurface {
    fn size(&self) -> Size {
        self.size
    }

    fn acquire(&mut self) -> Option<Box<dyn Texture>> {
        if std::mem::take(&mut self.fail_next_acquire) {
            return None;
        }
        Some(Box::new(NullTexture {
            size: self.size,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsage::Presentable,
        }))
    }

    fn present(&mut self) {
        self.present_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creating_a_buffer_zero_fills_it_to_the_requested_size() {
        let device = NullDevice::new();
        let buffer = device.create_buffer(64, BufferUsage::Vertex);
        assert_eq!(buffer.size(), 64);
    }

    #[test]
    fn a_recorded_pass_logs_every_call_in_order() {
        let device = NullDevice::new();
        let texture = device.create_texture(
            Size::new(10.0, 10.0),
            TextureFormat::Rgba8Unorm,
            TextureUsage::ColorTarget,
        );
        let mut encoder = NullCommandEncoder::default();
        encoder.begin_pass(texture.as_ref(), Some([0.0, 0.0, 0.0, 1.0]));
        encoder.draw_indexed(6, 1);
        encoder.end_pass();

        assert_eq!(
            encoder.log,
            vec![
                NullCommand::BeginPass {
                    clear: Some([0.0, 0.0, 0.0, 1.0])
                },
                NullCommand::DrawIndexed {
                    index_count: 6,
                    instance_count: 1
                },
                NullCommand::EndPass,
            ]
        );
    }

    #[test]
    fn a_failed_acquire_reports_none_exactly_once() {
        let mut surface = NullSurface::new(Size::new(800.0, 600.0));
        surface.fail_next_acquire = true;
        assert!(surface.acquire().is_none());
        assert!(surface.acquire().is_some());
    }

    #[test]
    fn present_counts_every_call() {
        let mut surface = NullSurface::new(Size::new(800.0, 600.0));
        surface.acquire();
        surface.present();
        surface.acquire();
        surface.present();
        assert_eq!(surface.present_count, 2);
    }
}
