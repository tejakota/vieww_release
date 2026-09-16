//! Turning a `vieww_paint::Scene` into something a GPU backend can execute.
//!
//! # Where this sits, and why it is here rather than in a backend
//!
//! `vieww-hal`'s Vulkan backend could walk a `Scene` itself. Then Metal would
//! walk it again, and D3D12 a third time, and the three would disagree about
//! what a `Clip` means long before any of them disagreed about a Vulkan call.
//! Everything on this page is backend-independent by construction — path
//! flattening, tessellation, batching, clip resolution, layer structure,
//! colour conversion — so it is computed exactly once, on the CPU, and handed
//! to whichever backend is executing the frame.
//!
//! ```text
//!   vieww_paint::Scene          (what to draw, in vieww's own vocabulary)
//!         │
//!         ▼   Planner::plan()   ← this module: one implementation, no GPU
//!   ScenePlan { vertices, indices, runs, steps, unsupported }
//!         │
//!         ▼
//!   Vulkan │ Metal │ D3D12      ← backends: targets, pipelines, submit
//! ```
//!
//! # The plan is a program, not a mesh
//!
//! A frame with no layers is still one vertex buffer and a handful of
//! `draw_indexed` calls. But a UI is not only fills: a sheet fades as a
//! *group*, a toolbar multiplies over a photo, a glass panel blurs what is
//! behind it, a card casts a shadow. None of those is expressible as geometry
//! drawn straight into the frame, so the plan carries an ordered list of
//! [`Step`]s a backend executes in order:
//!
//! ```text
//!   Draw        runs[a..b] into the current target
//!   PushLayer   open an offscreen target (optionally seeded with the backdrop,
//!               then filtered — CSS backdrop-filter)
//!   PopLayer    filter → shaped-clip multiply → blend (28 modes) × alpha into
//!               the parent
//!   Shadow      caster mask → 3 × (H, V) box blur → inset complement → tint
//!               → composite, in a scratch target
//! ```
//!
//! Every step mirrors, operation for operation, what
//! `vieww_paint::native::NativeRenderer` does for the same command, because
//! that renderer is the oracle `vieww-hal`'s parity suite compares against.
//! Where the CPU renderer's *definition* of something lives in its own code —
//! the antialiased coverage of a rounded clip, a shadow's silhouette, a
//! gradient's colour ramp, an image's mip pyramid — the planner takes it
//! through `vieww_paint::native::GpuSeam` rather than re-deriving it, so a
//! disagreement between the two renderers is always an *executor* bug.
//!
//! # Coverage contract
//!
//! [`ScenePlan::unsupported`] is the hard coverage boundary, and it is now a
//! list of **resource limits**, not missing features: an atlas that cannot
//! grow further, a layer stack deeper than [`MAX_TARGET_DEPTH`], an image
//! larger than the image atlas. A plan whose `unsupported` is empty is the
//! whole scene. A backend must not show an incomplete plan — see
//! [`ScenePlan::is_complete`].

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use vieww_foundation::{
    BlendMode, Color, GradientGeometry, Offset, Path, PathVerb, Rect, Transform,
};
use vieww_paint::native::{ColorGlyphParts, GlyphAlpha, GlyphCoverage, GpuSeam};
use vieww_paint::{Clip, Command, Paint, Scene};

use crate::atlas::{Atlas, AtlasKey};
use crate::image_atlas::{ImageAtlas, ImageSlot};
use crate::mask_atlas::{MaskAtlas, MaskKey, MaskSlot};
use crate::ramp_atlas::RampAtlas;
use crate::tessellate::{tessellate_fill, tessellate_stroke};

/// What a vertex's fragment computes. Carried in [`Vertex::texture_kind`].
pub mod material {
    /// A flat colour.
    pub const SOLID: f32 = 0.0;
    /// Colour × glyph-atlas coverage, fetched at an integer texel.
    pub const GLYPH: f32 = 1.0;
    /// A bilinear (optionally mip-blended) image sample.
    pub const IMAGE: f32 = 2.0;
    /// A per-fragment gradient: geometry in `params`, ramp row in `extra`.
    pub const GRADIENT: f32 = 3.0;
}

/// One vertex.
///
/// `#[repr(C)]` because it is uploaded verbatim and a backend's vertex
/// attribute descriptions are offsets into exactly this layout. Every field is
/// `f32` (23 of them, 92 bytes), so there is no padding to get wrong.
///
/// | field          | SOLID | GLYPH                    | IMAGE                        | GRADIENT                         |
/// |----------------|-------|--------------------------|------------------------------|----------------------------------|
/// | `color`        | paint | run colour               | unused (white)               | unused (white)                   |
/// | `uv`           | —     | glyph atlas fetch offset | —                            | —                                |
/// | `params`       | —     | —                        | lower mip patch `x, y, w, h` | geometry (see `gradient_params`) |
/// | `local`        | —     | —                        | unit rect coordinate         | unit paint-bounds coordinate     |
/// | `extra`        | —     | —                        | upper mip patch `x, y, w, h` | `row, dither, kind, 0`           |
/// | `mask`         | `ox, oy, enabled, _` for a shaped clip; `mask[3]` is the mip blend for IMAGE |
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub position: [f32; 2],
    /// Straight-alpha RGBA in `0..=1`. The shader premultiplies.
    pub color: [f32; 4],
    pub uv: [f32; 2],
    /// One of [`material`].
    pub texture_kind: f32,
    pub params: [f32; 4],
    pub local: [f32; 2],
    pub extra: [f32; 4],
    pub mask: [f32; 4],
}

impl Vertex {
    const fn solid(position: [f32; 2], color: [f32; 4], mask: [f32; 4]) -> Self {
        Self {
            position,
            color,
            uv: [0.0; 2],
            texture_kind: material::SOLID,
            params: [0.0; 4],
            local: [0.0; 2],
            extra: [0.0; 4],
            mask,
        }
    }
}

/// A contiguous run of indices sharing one scissor rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawRun {
    /// Offset of this run's first index into [`ScenePlan::indices`].
    pub first_index: u32,
    /// How many indices this run covers — always a multiple of three.
    pub index_count: u32,
    /// The scissor rectangle in device pixels (integral), or `None` for the
    /// whole target. Already intersected with the frame, every enclosing clip
    /// and every enclosing layer.
    pub scissor: Option<Rect>,
}

/// An integral device-pixel rectangle, `x0..x1` × `y0..y1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl PixelRect {
    pub const EMPTY: Self = Self {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    /// `rect` rounded outward — the same floor/ceil `NativeRenderer` sizes its
    /// buffers with.
    #[must_use]
    #[expect(clippy::cast_possible_truncation, reason = "device pixel coordinates")]
    pub fn round_out(rect: Rect) -> Self {
        Self {
            x0: rect.left.floor() as i32,
            y0: rect.top.floor() as i32,
            x1: rect.right.ceil() as i32,
            y1: rect.bottom.ceil() as i32,
        }
        .normalized()
    }

    fn normalized(self) -> Self {
        if self.x1 <= self.x0 || self.y1 <= self.y0 {
            Self {
                x1: self.x0,
                y1: self.y0,
                ..self
            }
        } else {
            self
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        if self.is_empty() {
            0
        } else {
            (self.x1 - self.x0) as u32
        }
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        if self.is_empty() {
            0
        } else {
            (self.y1 - self.y0) as u32
        }
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        Self {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        }
        .normalized()
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    #[must_use]
    pub fn inflate(self, by: i32) -> Self {
        if self.is_empty() {
            return self;
        }
        Self {
            x0: self.x0 - by,
            y0: self.y0 - by,
            x1: self.x1 + by,
            y1: self.y1 + by,
        }
    }

    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "device pixel coordinates")]
    pub fn to_rect(self) -> Rect {
        Rect::new(
            self.x0 as f32,
            self.y0 as f32,
            self.x1 as f32,
            self.y1 as f32,
        )
    }
}

/// An `ImageFilter`, resolved into what a backend executes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterOp {
    /// Radius of each of the three horizontal+vertical box passes; `None` for
    /// no blur. `vieww_paint::native::box_radius_for_sigma` of the sigma.
    pub box_radius: Option<u32>,
    /// A 4×5 colour matrix on unpremultiplied RGBA, applied after the blur.
    pub color_matrix: Option<[f32; 20]>,
}

/// A mask in [`MaskAtlas`], addressed by fetch offset. A mask always covers
/// the whole region its step (or its run's scissor) touches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskRef {
    /// `device pixel - offset = atlas texel`.
    pub offset: [f32; 2],
}

impl From<MaskSlot> for MaskRef {
    fn from(slot: MaskSlot) -> Self {
        Self {
            offset: slot.fetch_offset(),
        }
    }
}

/// One instruction in a frame's program. See the module doc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    /// Draw `runs[first_run .. first_run + run_count]` into the current target.
    Draw { first_run: usize, run_count: usize },
    /// Open an offscreen target covering `rect` (transparent outside `rect`).
    ///
    /// With `backdrop: Some(filter)` the target starts as a copy of the
    /// parent's pixels inside `rect`, and `filter` runs on that copy **before**
    /// any of the layer's own content draws — CSS `backdrop-filter`.
    PushLayer {
        rect: PixelRect,
        backdrop: Option<FilterOp>,
    },
    /// Close the innermost layer.
    ///
    /// In order: run `filter` over `rect`; multiply `mask` over `rect`; then
    /// composite `layer × alpha` into the parent through `blend`, touching only
    /// `composite` (the layer's ink inside `rect`, or nothing when `None`).
    PopLayer {
        rect: PixelRect,
        composite: Option<PixelRect>,
        filter: Option<FilterOp>,
        mask: Option<MaskRef>,
        blend: BlendMode,
        alpha: f32,
    },
    /// A blurred rounded-rect shadow, drawn source-over into the current
    /// target within `scissor`.
    ///
    /// `patch` is the silhouette's extent; outside it the blur reads
    /// transparent. `inner`, when present, makes it an inset shadow:
    /// `(1 - blurred) × inner`. `clip` multiplies the result.
    Shadow {
        patch: PixelRect,
        scissor: PixelRect,
        caster: MaskRef,
        inner: Option<MaskRef>,
        box_radius: Option<u32>,
        /// Premultiplied.
        color: [f32; 4],
        clip: Option<MaskRef>,
    },
}

/// A command this planner could not express, by kind.
///
/// Every remaining entry is a resource limit. See the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unsupported {
    /// A glyph that could not be given atlas texels (the atlas is at
    /// [`Atlas::MAX_SIDE`]), or any glyph in a plan built without a
    /// [`Planner`].
    Glyphs,
    /// An image larger than, or not fitting into, the image atlas.
    Image,
    /// A shadow whose masks do not fit the mask atlas, or whose work target
    /// would exceed [`MAX_TARGET_DEPTH`].
    Shadow,
    /// A gradient whose ramp does not fit the ramp atlas.
    Gradient,
    /// A shaped clip whose mask does not fit the mask atlas.
    ShapedClip,
    /// A layer nested deeper than [`MAX_TARGET_DEPTH`].
    Layer,
}

impl Unsupported {
    /// A short, stable name for reports and test failures.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Glyphs => "glyphs",
            Self::Image => "image",
            Self::Shadow => "shadow",
            Self::Gradient => "gradient",
            Self::ShapedClip => "shaped-clip",
            Self::Layer => "layer",
        }
    }
}

/// How many render targets a backend may be asked to hold at once: the frame
/// itself plus offscreen layers plus one shadow work target.
///
/// A full-frame target per level is what a backend allocates, so this is a
/// memory bound as much as a feature bound. Thirty-two is deeper than any
/// fixture in this workspace nests (the layout stress suite's deepest stack
/// is twenty-four).
pub const MAX_TARGET_DEPTH: usize = 32;

/// A whole frame, ready for a backend.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScenePlan {
    /// Every draw's vertices, concatenated.
    pub vertices: Vec<Vertex>,
    /// Every draw's indices, already offset into [`vertices`](Self::vertices).
    pub indices: Vec<u32>,
    /// The draw calls, in paint order. [`Step::Draw`] addresses them.
    pub runs: Vec<DrawRun>,
    /// The frame's program. See the module doc.
    pub steps: Vec<Step>,
    /// The deepest target index any step uses (0 = the frame). A backend
    /// allocates `target_depth + 1` targets.
    pub target_depth: usize,
    /// What this planner could not express, by kind and count. Empty means
    /// the plan is the whole scene.
    pub unsupported: BTreeMap<Unsupported, usize>,
}

impl ScenePlan {
    /// `true` when every command in the scene is in the plan.
    ///
    /// **The gate a caller must check before showing a GPU frame.** An
    /// incomplete plan is not a slightly worse frame; it is one with pieces
    /// missing.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.unsupported.is_empty()
    }

    /// Total triangles across every run.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// How many `draw_indexed` calls executing this plan's geometry costs.
    #[must_use]
    pub fn draw_call_count(&self) -> usize {
        self.runs.len()
    }

    /// How many offscreen compositing steps (layers and shadows) the plan has.
    #[must_use]
    pub fn offscreen_step_count(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s, Step::PushLayer { .. } | Step::Shadow { .. }))
            .count()
    }

    fn note(&mut self, what: Unsupported) {
        *self.unsupported.entry(what).or_insert(0) += 1;
    }
}

/// Assembles geometry into runs and runs into [`Step::Draw`]s.
struct Builder {
    plan: ScenePlan,
    /// Index of the first run not yet sealed into a `Draw` step.
    open_run: usize,
}

impl Builder {
    fn new() -> Self {
        Self {
            plan: ScenePlan::default(),
            open_run: 0,
        }
    }

    fn push_mesh(
        &mut self,
        vertices: impl Iterator<Item = Vertex>,
        indices: &[u32],
        scissor: Rect,
    ) {
        let base = u32::try_from(self.plan.vertices.len()).unwrap_or(u32::MAX);
        let before = self.plan.vertices.len();
        self.plan.vertices.extend(vertices);
        if self.plan.vertices.len() == before || indices.is_empty() {
            self.plan.vertices.truncate(before);
            return;
        }
        let first_index = u32::try_from(self.plan.indices.len()).unwrap_or(u32::MAX);
        self.plan.indices.extend(indices.iter().map(|i| base + i));
        let added = u32::try_from(indices.len()).unwrap_or(u32::MAX);
        let scissor = Some(scissor);
        let growable = self.plan.runs.len() > self.open_run;
        match self.plan.runs.last_mut() {
            // Only runs that are not sealed into an earlier step may grow.
            Some(run) if growable && run.scissor == scissor => {
                run.index_count += added;
            }
            _ => self.plan.runs.push(DrawRun {
                first_index,
                index_count: added,
                scissor,
            }),
        }
    }

    fn quad(&mut self, corners: [Vertex; 4], scissor: Rect) {
        self.push_mesh(corners.into_iter(), &[0, 1, 2, 0, 2, 3], scissor);
    }

    fn seal(&mut self) {
        let len = self.plan.runs.len();
        if len > self.open_run {
            self.plan.steps.push(Step::Draw {
                first_run: self.open_run,
                run_count: len - self.open_run,
            });
        }
        self.open_run = len;
    }

    fn step(&mut self, step: Step) {
        self.seal();
        self.plan.steps.push(step);
    }

    fn finish(mut self) -> ScenePlan {
        self.seal();
        self.plan
    }
}

/// A scene planner with the state a real frame needs.
///
/// Hold one per surface and keep it between frames: the atlases and the
/// rasterizer caches live here, and a steady frame adds to none of them.
#[derive(Debug, Default)]
pub struct Planner {
    atlas: Atlas,
    images: ImageAtlas,
    masks: MaskAtlas,
    ramps: RampAtlas,
    glyphs: GlyphCoverage,
    seam: GpuSeam,
}

impl Planner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The glyph coverage atlas (`R8`).
    #[must_use]
    pub const fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// The image atlas (straight-alpha `RGBA8`).
    #[must_use]
    pub const fn image_atlas(&self) -> &ImageAtlas {
        &self.images
    }

    /// The clip/shadow mask atlas (`R8`).
    #[must_use]
    pub const fn mask_atlas(&self) -> &MaskAtlas {
        &self.masks
    }

    /// The gradient ramp atlas (premultiplied `RGBA32F`).
    #[must_use]
    pub const fn ramp_atlas(&self) -> &RampAtlas {
        &self.ramps
    }

    /// Plan `scene` for execution at `width` x `height` device pixels.
    ///
    /// Never fails: what cannot be expressed is recorded in
    /// [`ScenePlan::unsupported`].
    pub fn plan(&mut self, scene: &Scene, width: f32, height: f32) -> ScenePlan {
        self.masks.begin_frame();
        self.ramps.begin_frame();
        self.images.begin_frame();
        plan_with(scene, width, height, Some(self))
    }
}

/// Plan `scene` without atlases.
///
/// Geometry, rectangular clips and layers without shaped clips plan normally;
/// anything that needs an atlas — glyphs, images, gradients, shaped clips,
/// shadows — is reported as unsupported. Kept because most of the planner is
/// much easier to test without a font. **Not the entry point for a real
/// frame**: use [`Planner`].
#[must_use]
pub fn plan(scene: &Scene, width: f32, height: f32) -> ScenePlan {
    plan_with(scene, width, height, None)
}

#[derive(Clone, Copy)]
enum FrameKind {
    Root,
    /// A layer that needs no target of its own: `Normal`, opaque, unfiltered,
    /// unshaped. Drawing its children straight into the parent is exactly
    /// equivalent, because source-over is associative — confined to the
    /// layer's rectangle by scissor, as `NativeRenderer` confines them to its
    /// buffer.
    PassThrough,
    Offscreen {
        backdrop: bool,
    },
    /// A layer that exceeded [`MAX_TARGET_DEPTH`]; everything inside is
    /// skipped (and was reported once, at the push).
    Skipped,
}

struct Frame {
    kind: FrameKind,
    rect: PixelRect,
    /// Where this frame's target has been written, as `NativeRenderer`'s
    /// `Target::ink` tracks it.
    ink: PixelRect,
    /// Target index this frame's content draws into.
    level: usize,
    // PopLayer inputs, for offscreen frames.
    alpha: f32,
    blend: BlendMode,
    filter: vieww_foundation::ImageFilter,
    clip: Clip,
}

fn plan_with(
    scene: &Scene,
    width: f32,
    height: f32,
    mut planner: Option<&mut Planner>,
) -> ScenePlan {
    let surface = Rect::new(0.0, 0.0, width, height);
    let mut out = Builder::new();
    let mut stack: Vec<Frame> = vec![Frame {
        kind: FrameKind::Root,
        rect: PixelRect::round_out(surface),
        ink: PixelRect::EMPTY,
        level: 0,
        alpha: 1.0,
        blend: BlendMode::Normal,
        filter: vieww_foundation::ImageFilter::NONE,
        clip: Clip::NONE,
    }];

    for command in scene.commands() {
        if stack.iter().any(|f| matches!(f.kind, FrameKind::Skipped)) {
            match command {
                Command::PushLayer { .. } => stack.push(skipped_frame()),
                Command::PopLayer if stack.len() > 1 => {
                    stack.pop();
                }
                _ => {}
            }
            continue;
        }

        match command {
            Command::PushLayer {
                bounds,
                alpha,
                blend,
                clip,
                filter,
            } => {
                let top = stack.last().expect("root frame");
                // `NativeRenderer`: bounds ∩ parent buffer ∩ the layer's clip
                // rectangle ∩ surface, rounded out.
                let dev = bounds
                    .intersect(top.rect.to_rect())
                    .intersect(clip.bounds().map_or(surface, |b| b.intersect(surface)))
                    .intersect(surface);
                let rect = PixelRect::round_out(dev);
                let needs_target = *alpha < 1.0
                    || *blend != BlendMode::Normal
                    || !filter.is_noop()
                    || filter.backdrop
                    || !clip.shapes().is_empty();
                if !needs_target {
                    let level = top.level;
                    stack.push(Frame {
                        kind: FrameKind::PassThrough,
                        rect,
                        ink: PixelRect::EMPTY,
                        level,
                        alpha: 1.0,
                        blend: BlendMode::Normal,
                        filter: *filter,
                        clip: Clip::NONE,
                    });
                    continue;
                }
                let level = top.level + 1;
                // One more level is reserved for a shadow's work target.
                if level + 1 >= MAX_TARGET_DEPTH {
                    out.plan.note(Unsupported::Layer);
                    stack.push(skipped_frame());
                    continue;
                }
                out.plan.target_depth = out.plan.target_depth.max(level);
                let backdrop = filter.backdrop.then(|| filter_op(filter));
                out.step(Step::PushLayer { rect, backdrop });
                stack.push(Frame {
                    kind: FrameKind::Offscreen {
                        backdrop: filter.backdrop,
                    },
                    rect,
                    // A backdrop copy writes every pixel of the buffer.
                    ink: if filter.backdrop {
                        rect
                    } else {
                        PixelRect::EMPTY
                    },
                    level,
                    alpha: *alpha,
                    blend: *blend,
                    filter: *filter,
                    clip: clip.clone(),
                });
            }
            Command::PopLayer => {
                if stack.len() <= 1 {
                    continue;
                }
                let frame = stack.pop().expect("checked above");
                match frame.kind {
                    FrameKind::Root | FrameKind::Skipped => {}
                    FrameKind::PassThrough => {
                        let parent = stack.last_mut().expect("root frame");
                        parent.ink = parent.ink.union(frame.ink);
                    }
                    FrameKind::Offscreen { backdrop } => {
                        let mut ink = frame.ink;
                        let filter = (!backdrop && !frame.filter.is_noop()).then(|| {
                            let op = filter_op(&frame.filter);
                            if !ink.is_empty() {
                                if let Some(r) = op.box_radius {
                                    ink = ink.inflate(3 * r as i32).intersect(frame.rect);
                                }
                                if op.color_matrix.is_some() {
                                    ink = frame.rect;
                                }
                            }
                            op
                        });
                        let mut mask = None;
                        if !frame.clip.shapes().is_empty() && !frame.rect.is_empty() {
                            match planner.as_mut() {
                                Some(p) => match p.clip_mask(&frame.clip, frame.rect) {
                                    Some(m) => mask = Some(m),
                                    None => out.plan.note(Unsupported::ShapedClip),
                                },
                                None => out.plan.note(Unsupported::ShapedClip),
                            }
                        }
                        let composite = ink.intersect(frame.rect);
                        let composite = (!composite.is_empty()).then_some(composite);
                        out.step(Step::PopLayer {
                            rect: frame.rect,
                            composite,
                            filter,
                            mask,
                            blend: frame.blend,
                            alpha: frame.alpha.clamp(0.0, 1.0),
                        });
                        if let Some(region) = composite {
                            let parent = stack.last_mut().expect("root frame");
                            parent.ink = parent.ink.union(region);
                        }
                    }
                }
            }
            Command::DrawShadow {
                rect,
                radius,
                shadow,
                transform,
                clip,
            } => {
                let Some(p) = planner.as_mut() else {
                    out.plan.note(Unsupported::Shadow);
                    continue;
                };
                let top = stack.last().expect("root frame");
                let level = top.level;
                if level + 1 >= MAX_TARGET_DEPTH {
                    out.plan.note(Unsupported::Shadow);
                    continue;
                }
                let local_surface = top.rect.to_rect().intersect(surface);
                match p.plan_shadow(*rect, *radius, shadow, *transform, clip, local_surface) {
                    ShadowPlan::Nothing => {}
                    ShadowPlan::Gap => out.plan.note(Unsupported::Shadow),
                    ShadowPlan::Step(step, touched) => {
                        out.plan.target_depth = out.plan.target_depth.max(level + 1);
                        out.step(step);
                        let top = stack.last_mut().expect("root frame");
                        top.ink = top.ink.union(touched);
                    }
                }
            }
            primitive => {
                let top = stack.last().expect("root frame");
                let Some(target) = clip_region(primitive.clip(), top.rect, surface) else {
                    continue;
                };
                let mask = if primitive.clip().shapes().is_empty() {
                    [0.0; 4]
                } else {
                    let Some(p) = planner.as_mut() else {
                        out.plan.note(Unsupported::ShapedClip);
                        continue;
                    };
                    match p.clip_mask(primitive.clip(), target) {
                        Some(m) => [m.offset[0], m.offset[1], 1.0, 0.0],
                        None => {
                            out.plan.note(Unsupported::ShapedClip);
                            continue;
                        }
                    }
                };
                let before = out.plan.vertices.len();
                plan_primitive(&mut out, planner.as_deref_mut(), primitive, target, mask);
                // Ink: the device bounds of what was emitted, confined to the
                // scissor — `NativeRenderer` marks the bounding box of the
                // coverage it composites.
                if out.plan.vertices.len() > before {
                    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                    for v in &out.plan.vertices[before..] {
                        x0 = x0.min(v.position[0]);
                        y0 = y0.min(v.position[1]);
                        x1 = x1.max(v.position[0]);
                        y1 = y1.max(v.position[1]);
                    }
                    let bounds = PixelRect::round_out(Rect::new(x0, y0, x1, y1));
                    let top = stack.last_mut().expect("root frame");
                    top.ink = top.ink.union(bounds.intersect(target));
                }
            }
        }
    }

    out.finish()
}

fn skipped_frame() -> Frame {
    Frame {
        kind: FrameKind::Skipped,
        rect: PixelRect::EMPTY,
        ink: PixelRect::EMPTY,
        level: 0,
        alpha: 1.0,
        blend: BlendMode::Normal,
        filter: vieww_foundation::ImageFilter::NONE,
        clip: Clip::NONE,
    }
}

fn filter_op(filter: &vieww_foundation::ImageFilter) -> FilterOp {
    FilterOp {
        box_radius: (filter.blur_sigma > 0.0)
            .then(|| vieww_paint::native::box_radius_for_sigma(filter.blur_sigma)),
        color_matrix: filter.color_matrix,
    }
}

/// The scissor a primitive draws within: its clip's rectangle ∩ the enclosing
/// target's rectangle. `None` when nothing can be drawn — which is a complete
/// rendering of the command, not a gap.
fn clip_region(clip: &Clip, layer: PixelRect, surface: Rect) -> Option<PixelRect> {
    let bounds = clip.bounds().map_or(surface, |b| b.intersect(surface));
    let region = PixelRect::round_out(bounds).intersect(layer);
    (!region.is_empty()).then_some(region)
}

enum ShadowPlan {
    Nothing,
    Gap,
    Step(Step, PixelRect),
}

fn hash_path(path: &Path, state: &mut impl Hasher) {
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(p) => (0u8, p.dx.to_bits(), p.dy.to_bits()).hash(state),
            PathVerb::LineTo(p) => (1u8, p.dx.to_bits(), p.dy.to_bits()).hash(state),
            PathVerb::CubicTo(a, b, c) => {
                (2u8, [a.dx, a.dy, b.dx, b.dy, c.dx, c.dy].map(f32::to_bits)).hash(state)
            }
            PathVerb::Close => 3u8.hash(state),
        }
    }
}

struct ClipMaskKey<'a>(&'a Clip, PixelRect);

impl Hash for ClipMaskKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "clip".hash(state);
        self.1.hash(state);
        for shape in self.0.shapes() {
            hash_path(shape, state);
            0xffu8.hash(state);
        }
    }
}

impl Planner {
    /// The mask for `clip`'s shapes over `region`. `None` only when the atlas
    /// is full; an empty region never reaches here.
    fn clip_mask(&mut self, clip: &Clip, region: PixelRect) -> Option<MaskRef> {
        let key = MaskKey::of(&ClipMaskKey(clip, region));
        let seam = &self.seam;
        self.masks
            .get_or_insert(key, || {
                let patch = seam.clip_mask(clip, region.to_rect())?;
                Some((patch.x, patch.y, patch.width, patch.height, patch.alpha))
            })
            .map(MaskRef::from)
    }

    fn plan_shadow(
        &mut self,
        rect: Rect,
        radius: f32,
        shadow: &vieww_foundation::Shadow,
        transform: Transform,
        clip: &Clip,
        local_surface: Rect,
    ) -> ShadowPlan {
        let Some(masks) = self
            .seam
            .shadow_masks(rect, radius, shadow, transform, local_surface)
        else {
            return ShadowPlan::Nothing;
        };
        let patch = PixelRect {
            x0: masks.x,
            y0: masks.y,
            x1: masks.x + masks.width as i32,
            y1: masks.y + masks.height as i32,
        };
        let key_base = {
            let mut h = std::hash::DefaultHasher::new();
            (
                "shadow",
                [rect.left, rect.top, rect.right, rect.bottom, radius].map(f32::to_bits),
                [
                    shadow.offset.dx,
                    shadow.offset.dy,
                    shadow.blur,
                    shadow.spread,
                ]
                .map(f32::to_bits),
                shadow.is_inset,
                [
                    transform.a,
                    transform.b,
                    transform.c,
                    transform.d,
                    transform.tx,
                    transform.ty,
                ]
                .map(f32::to_bits),
                patch,
            )
                .hash(&mut h);
            h.finish()
        };
        let Some(caster) = self
            .masks
            .get_or_insert(MaskKey(key_base), || {
                Some((
                    masks.x,
                    masks.y,
                    masks.width,
                    masks.height,
                    masks.caster.clone(),
                ))
            })
            .map(MaskRef::from)
        else {
            return ShadowPlan::Gap;
        };
        let inner = match &masks.inner {
            None => None,
            Some(inner) => match self
                .masks
                .get_or_insert(MaskKey(key_base ^ 0x9e37_79b9_7f4a_7c15), || {
                    Some((masks.x, masks.y, masks.width, masks.height, inner.clone()))
                }) {
                Some(slot) => Some(MaskRef::from(slot)),
                None => return ShadowPlan::Gap,
            },
        };
        // `NativeRenderer` composites the patch through the clip's *mask* only:
        // a rectangle-only clip does not cut a shadow (the patch is confined to
        // the enclosing buffer instead). Mirrored exactly — this planner's job
        // is parity with the oracle, and a change to that rule belongs in the
        // oracle first.
        let (scissor, clip_mask) = if clip.shapes().is_empty() {
            (patch, None)
        } else {
            let region = clip_region(clip, patch, local_surface);
            let Some(region) = region else {
                return ShadowPlan::Nothing;
            };
            match self.clip_mask(clip, region) {
                Some(mask) => (region, Some(mask)),
                None => return ShadowPlan::Gap,
            }
        };
        let tint = shadow.color;
        let a = f32::from(tint.a) / 255.0;
        let color = [
            f32::from(tint.r) / 255.0 * a,
            f32::from(tint.g) / 255.0 * a,
            f32::from(tint.b) / 255.0 * a,
            a,
        ];
        ShadowPlan::Step(
            Step::Shadow {
                patch,
                scissor,
                caster,
                inner,
                box_radius: masks.box_radius,
                color,
                clip: clip_mask,
            },
            scissor,
        )
    }
}

/// Emit one fill, stroke, glyph run or image.
fn plan_primitive(
    out: &mut Builder,
    planner: Option<&mut Planner>,
    command: &Command,
    target: PixelRect,
    mask: [f32; 4],
) {
    let scissor = target.to_rect();
    match command {
        Command::FillRect {
            rect,
            paint,
            transform,
            ..
        } => {
            if paint.is_invisible() {
                return;
            }
            let mesh = tessellate_fill(&rect_path(*rect, *transform), TOLERANCE);
            emit_painted(
                out,
                planner,
                &mesh.positions,
                &mesh.indices,
                *paint,
                *rect,
                *transform,
                scissor,
                mask,
            );
        }
        Command::FillPath {
            path,
            paint,
            transform,
            ..
        } => {
            if paint.is_invisible() {
                return;
            }
            let mesh = tessellate_fill(&transformed(path, *transform), TOLERANCE);
            emit_painted(
                out,
                planner,
                &mesh.positions,
                &mesh.indices,
                *paint,
                path.bounds(),
                *transform,
                scissor,
                mask,
            );
        }
        Command::StrokePath {
            path,
            stroke,
            paint,
            transform,
            ..
        } => {
            if paint.is_invisible() || stroke.is_invisible() {
                return;
            }
            // **In local space, then transformed — as the CPU stroker does.**
            // Tessellating the already-transformed path used the stroke width
            // in *device* units, so under any scale (a 2x display, a zoomed
            // canvas) the GPU stroke came out a different width from the
            // CPU's; and dashes were not applied at all. Both are silent
            // wrongness, not gaps. The dash split is the CPU renderer's own
            // (`GpuSeam`'s `dashed_path`), in the same space it measures in.
            let dashed = vieww_paint::native::dashed_path(path, &stroke.style.dash);
            let local = dashed.as_ref().unwrap_or(path);
            let scale = (transform.a * transform.d - transform.b * transform.c)
                .abs()
                .sqrt()
                .max(1e-3);
            let mut mesh = tessellate_stroke(local, stroke, TOLERANCE / scale);
            for p in &mut mesh.positions {
                let d = transform.apply(Offset::new(p[0], p[1]));
                *p = [d.dx, d.dy];
            }
            let bounds = path.bounds().inflate(stroke.reach());
            emit_painted(
                out,
                planner,
                &mesh.positions,
                &mesh.indices,
                *paint,
                bounds,
                *transform,
                scissor,
                mask,
            );
        }
        Command::DrawGlyphs { run, transform, .. } => match planner {
            Some(planner) => planner.plan_glyphs(out, run, *transform, scissor, mask),
            None => out.plan.note(Unsupported::Glyphs),
        },
        Command::DrawImage {
            rect,
            image,
            transform,
            ..
        } => match planner {
            Some(planner) => planner.plan_image(out, image, *rect, *transform, scissor, mask),
            None => out.plan.note(Unsupported::Image),
        },
        Command::PushLayer { .. } | Command::PopLayer | Command::DrawShadow { .. } => {
            unreachable!("handled by plan_with");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_painted(
    out: &mut Builder,
    planner: Option<&mut Planner>,
    positions: &[[f32; 2]],
    indices: &[u32],
    paint: Paint,
    local_bounds: Rect,
    transform: Transform,
    scissor: Rect,
    mask: [f32; 4],
) {
    if positions.is_empty() || indices.is_empty() {
        return;
    }
    let Some(gradient) = paint.gradient else {
        let color = straight_rgba(paint.color);
        out.push_mesh(
            positions.iter().map(|p| Vertex::solid(*p, color, mask)),
            indices,
            scissor,
        );
        return;
    };
    let Some(planner) = planner else {
        out.plan.note(Unsupported::Gradient);
        return;
    };
    let Some(inverse) = transform.invert() else {
        // A degenerate transform covers no pixels.
        return;
    };
    let Some(row) = planner.ramps.row(&gradient) else {
        out.plan.note(Unsupported::Gradient);
        return;
    };
    let (params, kind) = gradient_params(gradient.geometry);
    let inv_w = 1.0 / local_bounds.width().max(1e-6);
    let inv_h = 1.0 / local_bounds.height().max(1e-6);
    #[expect(clippy::cast_precision_loss, reason = "ramp rows < 2^24")]
    let extra = [
        row as f32,
        f32::from(u8::from(gradient.dither())),
        kind,
        0.0,
    ];
    out.push_mesh(
        positions.iter().map(|p| {
            let local = inverse.apply(Offset::new(p[0], p[1]));
            Vertex {
                position: *p,
                color: [1.0; 4],
                uv: [0.0; 2],
                texture_kind: material::GRADIENT,
                params,
                local: [
                    (local.dx - local_bounds.left) * inv_w,
                    (local.dy - local_bounds.top) * inv_h,
                ],
                extra,
                mask,
            }
        }),
        indices,
        scissor,
    );
}

/// A gradient's geometry for the shader, and its kind (0 linear, 1 radial,
/// 2 sweep). Units are the paint's unit square, as `NativeRenderer` uses.
fn gradient_params(geometry: GradientGeometry) -> ([f32; 4], f32) {
    match geometry {
        GradientGeometry::Linear { start, end } => ([start.dx, start.dy, end.dx, end.dy], 0.0),
        GradientGeometry::Radial { center, radius } => ([center.dx, center.dy, radius, 0.0], 1.0),
        GradientGeometry::Sweep {
            center,
            start_angle,
            end_angle,
        } => ([center.dx, center.dy, start_angle, end_angle], 2.0),
    }
}

impl Planner {
    /// One glyph run: colour glyphs first, then coverage glyphs.
    fn plan_glyphs(
        &mut self,
        out: &mut Builder,
        run: &vieww_foundation::GlyphRun,
        transform: Transform,
        scissor: Rect,
        mask: [f32; 4],
    ) {
        for glyph in run.glyphs.iter() {
            match self.seam.color_glyph(
                &run.font,
                glyph.id,
                run.origin,
                glyph.offset,
                run.size,
                transform,
            ) {
                Some(ColorGlyphParts::Layers(layers)) => {
                    for (layer_glyph, color) in layers {
                        self.plan_coverage_glyph(
                            out,
                            run,
                            layer_glyph,
                            glyph.offset,
                            color,
                            transform,
                            scissor,
                            mask,
                        );
                    }
                }
                Some(ColorGlyphParts::Bitmap { image, rect }) => {
                    self.plan_image(out, &image, rect, transform, scissor, mask);
                }
                None => {
                    self.plan_coverage_glyph(
                        out,
                        run,
                        glyph.id,
                        glyph.offset,
                        run.color,
                        transform,
                        scissor,
                        mask,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_coverage_glyph(
        &mut self,
        out: &mut Builder,
        run: &vieww_foundation::GlyphRun,
        glyph_id: u16,
        offset: Offset,
        color: Color,
        transform: Transform,
        scissor: Rect,
        mask: [f32; 4],
    ) {
        let inked = match self
            .glyphs
            .glyph(&run.font, glyph_id, run.size, run.origin, offset, transform)
        {
            // No outline, or no pixels at this size: both are complete
            // renderings (a space draws nothing).
            GlyphAlpha::NoOutline | GlyphAlpha::Blank => return,
            GlyphAlpha::Inked(inked) => inked,
        };
        let device = transform.apply(Offset::new(
            run.origin.dx + offset.dx,
            run.origin.dy + offset.dy,
        ));
        let key = AtlasKey {
            font: run.font.id(),
            glyph: glyph_id,
            size: run.size.to_bits(),
            transform: [transform.a, transform.b, transform.c, transform.d].map(f32::to_bits),
            phase: (
                (device.dx - device.dx.floor()).to_bits(),
                (device.dy - device.dy.floor()).to_bits(),
            ),
        };
        let slot = match self.atlas.get(&key) {
            Some(slot) => Some(slot),
            None => {
                let bytes = inked.alpha();
                self.atlas
                    .insert(key, inked.width(), inked.height(), &bytes)
            }
        };
        let Some(slot) = slot else {
            // A word with a letter missing looks like a font bug; report it.
            out.plan.note(Unsupported::Glyphs);
            return;
        };
        #[expect(clippy::cast_precision_loss, reason = "device pixels")]
        let (x0, y0) = (inked.x() as f32, inked.y() as f32);
        #[expect(clippy::cast_precision_loss, reason = "device pixels")]
        let (x1, y1) = (x0 + slot.width as f32, y0 + slot.height as f32);
        #[expect(clippy::cast_precision_loss, reason = "texel coordinates")]
        let fetch = [
            inked.x() as f32 - slot.x as f32,
            inked.y() as f32 - slot.y as f32,
        ];
        let rgba = straight_rgba(color);
        let corner = |x: f32, y: f32| Vertex {
            position: [x, y],
            color: rgba,
            uv: fetch,
            texture_kind: material::GLYPH,
            params: [0.0; 4],
            local: [0.0; 2],
            extra: [0.0; 4],
            mask,
        };
        out.quad(
            [
                corner(x0, y0),
                corner(x1, y0),
                corner(x1, y1),
                corner(x0, y1),
            ],
            scissor,
        );
    }

    /// An image quad under its transform, sampled per fragment exactly as
    /// `NativeRenderer::paint_image` samples: bilinear on premultiplied texels,
    /// clamped to the image, blended between two mip levels when minified.
    fn plan_image(
        &mut self,
        out: &mut Builder,
        image: &vieww_foundation::Image,
        rect: Rect,
        transform: Transform,
        scissor: Rect,
        mask: [f32; 4],
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 || image.width() == 0 || image.height() == 0
        {
            return;
        }
        let Some(ratio) = vieww_paint::native::minification_ratio(image, rect, transform) else {
            return;
        };
        let patch = |slot: ImageSlot| {
            #[expect(clippy::cast_precision_loss, reason = "texel coordinates")]
            let p = [
                slot.x as f32,
                slot.y as f32,
                slot.width as f32,
                slot.height as f32,
            ];
            p
        };
        let (lower, upper, frac) = if ratio >= 2.0 {
            let chain = self.seam.image_mips(image);
            if chain.is_empty() {
                (image.clone(), image.clone(), 0.0)
            } else {
                #[expect(clippy::cast_precision_loss, reason = "short chains")]
                let max_level = chain.len() as f32;
                let level = ratio.log2().clamp(0.0, max_level).min(max_level);
                let lower = level.floor();
                let frac = level - lower;
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "0..=len"
                )]
                let li = lower as usize;
                let lower_image = if li == 0 {
                    image.clone()
                } else {
                    chain[li - 1].clone()
                };
                let upper_image = chain[li.min(chain.len() - 1)].clone();
                (lower_image, upper_image, frac)
            }
        } else {
            (image.clone(), image.clone(), 0.0)
        };
        let Some(lower_slot) = self.images.insert(&lower) else {
            out.plan.note(Unsupported::Image);
            return;
        };
        let upper_slot = if frac > 0.0 {
            match self.images.insert(&upper) {
                Some(slot) => slot,
                None => {
                    out.plan.note(Unsupported::Image);
                    return;
                }
            }
        } else {
            lower_slot
        };
        // `insert` may have grown the atlas; slots keep their texel positions,
        // so re-reading is unnecessary — `x`/`y` are stable.
        let params = patch(lower_slot);
        let extra = patch(upper_slot);
        let mask = [mask[0], mask[1], mask[2], frac];
        let corner = |local: [f32; 2]| {
            let p = transform.apply(Offset::new(
                rect.left + local[0] * rect.width(),
                rect.top + local[1] * rect.height(),
            ));
            Vertex {
                position: [p.dx, p.dy],
                color: [1.0; 4],
                uv: [0.0; 2],
                texture_kind: material::IMAGE,
                params,
                local,
                extra,
                mask,
            }
        };
        out.quad(
            [
                corner([0.0, 0.0]),
                corner([1.0, 0.0]),
                corner([1.0, 1.0]),
                corner([0.0, 1.0]),
            ],
            scissor,
        );
    }
}

/// Curve flattening tolerance, in device pixels. See `tessellate`.
const TOLERANCE: f32 = 0.25;

/// Straight-alpha RGBA in 0..1.
fn straight_rgba(color: Color) -> [f32; 4] {
    [
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    ]
}

/// A rectangle as a closed path, with the command's transform applied.
fn rect_path(rect: Rect, transform: Transform) -> Path {
    let mut path = Path::new();
    path.move_to(transform.apply(Offset::new(rect.left, rect.top)));
    path.line_to(transform.apply(Offset::new(rect.right, rect.top)));
    path.line_to(transform.apply(Offset::new(rect.right, rect.bottom)));
    path.line_to(transform.apply(Offset::new(rect.left, rect.bottom)));
    path.close();
    path
}

/// `path` with `transform` applied to every point.
fn transformed(path: &Path, transform: Transform) -> Path {
    if transform.is_identity() {
        return path.clone();
    }
    let mut out = Path::new();
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(p) => {
                out.move_to(transform.apply(p));
            }
            PathVerb::LineTo(p) => {
                out.line_to(transform.apply(p));
            }
            PathVerb::CubicTo(a, b, c) => {
                out.cubic_to(transform.apply(a), transform.apply(b), transform.apply(c));
            }
            PathVerb::Close => {
                out.close();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{Gradient, Image, ImageFilter, Shadow};
    use vieww_paint::Stroke;

    fn solid(scene: &mut Scene, rect: Rect, color: Color, clip: Clip) {
        scene.push_command(Command::FillRect {
            rect,
            paint: Paint::solid(color),
            transform: Transform::IDENTITY,
            clip,
        });
    }

    fn opaque() -> Color {
        Color::rgba(10, 20, 30, 255)
    }

    fn layer(scene: &mut Scene, alpha: f32, blend: BlendMode, filter: ImageFilter, clip: Clip) {
        scene.push_command(Command::PushLayer {
            bounds: Rect::new(0.0, 0.0, 50.0, 50.0),
            alpha,
            blend,
            clip,
            filter,
        });
    }

    fn rounded_clip() -> Clip {
        let mut clip = Clip::NONE;
        clip.add_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut shape = Path::new();
        shape.move_to(Offset::new(0.0, 0.0));
        shape.cubic_to(
            Offset::new(10.0, 0.0),
            Offset::new(10.0, 10.0),
            Offset::new(0.0, 10.0),
        );
        shape.close();
        clip.add_path(shape);
        clip
    }

    #[test]
    fn an_empty_scene_plans_to_nothing_and_is_complete() {
        let plan = plan(&Scene::default(), 100.0, 100.0);
        assert!(plan.is_complete());
        assert_eq!(plan.draw_call_count(), 0);
        assert!(plan.steps.is_empty());
    }

    #[test]
    fn a_rectangle_becomes_two_triangles_in_one_draw_step() {
        let mut scene = Scene::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );
        let plan = plan(&scene, 100.0, 100.0);
        assert_eq!(plan.triangle_count(), 2);
        assert_eq!(plan.draw_call_count(), 1);
        assert_eq!(
            plan.steps,
            vec![Step::Draw {
                first_run: 0,
                run_count: 1
            }]
        );
    }

    #[test]
    fn shapes_sharing_a_scissor_collapse_into_one_run() {
        let mut scene = Scene::default();
        for i in 0..20i32 {
            let x = i as f32 * 4.0;
            solid(
                &mut scene,
                Rect::new(x, 0.0, x + 3.0, 3.0),
                opaque(),
                Clip::NONE,
            );
        }
        let plan = plan(&scene, 200.0, 100.0);
        assert_eq!(plan.draw_call_count(), 1, "20 unclipped fills, one draw");
        assert_eq!(plan.triangle_count(), 40);
    }

    #[test]
    fn a_change_of_clip_starts_a_new_run() {
        let mut clipped = Clip::NONE;
        clipped.add_rect(Rect::new(0.0, 0.0, 5.0, 5.0));
        let mut scene = Scene::default();
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        solid(&mut scene, r, opaque(), Clip::NONE);
        solid(&mut scene, r, opaque(), clipped.clone());
        solid(&mut scene, r, opaque(), clipped);
        solid(&mut scene, r, opaque(), Clip::NONE);
        let plan = plan(&scene, 100.0, 100.0);
        assert_eq!(plan.draw_call_count(), 3);
    }

    #[test]
    fn every_run_addresses_indices_that_exist_and_steps_cover_every_run_once() {
        let mut clipped = Clip::NONE;
        clipped.add_rect(Rect::new(2.0, 2.0, 8.0, 8.0));
        let mut scene = Scene::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );
        layer(
            &mut scene,
            0.5,
            BlendMode::Normal,
            ImageFilter::NONE,
            Clip::NONE,
        );
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            clipped,
        );
        scene.push_command(Command::PopLayer);
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );

        let plan = plan(&scene, 100.0, 100.0);
        let mut covered = 0;
        for run in &plan.runs {
            assert_eq!(run.index_count % 3, 0);
            assert!(run.first_index as usize + run.index_count as usize <= plan.indices.len());
            covered += run.index_count as usize;
        }
        assert_eq!(covered, plan.indices.len());
        let mut next = 0;
        for step in &plan.steps {
            if let Step::Draw {
                first_run,
                run_count,
            } = step
            {
                assert_eq!(*first_run, next, "draw steps address runs in order");
                next += run_count;
            }
        }
        assert_eq!(next, plan.runs.len());
    }

    #[test]
    fn a_group_opacity_layer_is_an_offscreen_pass_not_folded_alpha() {
        // Folding the group alpha into each child is wrong wherever children
        // overlap: two opaque squares at 50% group opacity are uniformly 50%,
        // not 75% where they cross. The CPU renderer composites the group.
        let mut scene = Scene::default();
        layer(
            &mut scene,
            0.5,
            BlendMode::Normal,
            ImageFilter::NONE,
            Clip::NONE,
        );
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );
        solid(
            &mut scene,
            Rect::new(5.0, 5.0, 15.0, 15.0),
            opaque(),
            Clip::NONE,
        );
        scene.push_command(Command::PopLayer);

        let plan = plan(&scene, 100.0, 100.0);
        assert!(plan.is_complete());
        assert_eq!(plan.target_depth, 1);
        assert!(plan
            .vertices
            .iter()
            .all(|v| (v.color[3] - 1.0).abs() < 1e-6));
        match plan.steps.as_slice() {
            [Step::PushLayer {
                rect,
                backdrop: None,
            }, Step::Draw { .. }, Step::PopLayer {
                alpha,
                composite,
                blend,
                ..
            }] => {
                assert_eq!(
                    *rect,
                    PixelRect {
                        x0: 0,
                        y0: 0,
                        x1: 50,
                        y1: 50
                    }
                );
                assert!((alpha - 0.5).abs() < 1e-6);
                assert_eq!(*blend, BlendMode::Normal);
                assert_eq!(
                    *composite,
                    Some(PixelRect {
                        x0: 0,
                        y0: 0,
                        x1: 15,
                        y1: 15
                    })
                );
            }
            other => panic!("unexpected steps: {other:?}"),
        }
    }

    #[test]
    fn an_opaque_normal_layer_needs_no_target_but_still_confines_its_children() {
        let mut scene = Scene::default();
        layer(
            &mut scene,
            1.0,
            BlendMode::Normal,
            ImageFilter::NONE,
            Clip::NONE,
        );
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 80.0, 80.0),
            opaque(),
            Clip::NONE,
        );
        scene.push_command(Command::PopLayer);
        let plan = plan(&scene, 100.0, 100.0);
        assert_eq!(plan.target_depth, 0);
        assert_eq!(plan.offscreen_step_count(), 0);
        assert_eq!(plan.runs[0].scissor, Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
    }

    #[test]
    fn every_blend_mode_and_filter_is_planned_not_reported() {
        for blend in [
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Xor,
            BlendMode::Luminosity,
        ] {
            for filter in [
                ImageFilter::NONE,
                ImageFilter::blur(4.0),
                ImageFilter::backdrop_blur(3.0),
            ] {
                let mut scene = Scene::default();
                layer(&mut scene, 0.8, blend, filter, Clip::NONE);
                solid(
                    &mut scene,
                    Rect::new(10.0, 10.0, 20.0, 20.0),
                    opaque(),
                    Clip::NONE,
                );
                scene.push_command(Command::PopLayer);
                let plan = plan(&scene, 100.0, 100.0);
                assert!(
                    plan.is_complete(),
                    "{blend:?} {filter:?}: {:?}",
                    plan.unsupported
                );
                let pop = plan
                    .steps
                    .iter()
                    .find(|s| matches!(s, Step::PopLayer { .. }))
                    .unwrap();
                let Step::PopLayer {
                    blend: b,
                    filter: f,
                    composite,
                    ..
                } = pop
                else {
                    unreachable!()
                };
                assert_eq!(*b, blend);
                if filter.backdrop {
                    assert!(
                        f.is_none(),
                        "a backdrop filter runs at push, never again at pop"
                    );
                    assert_eq!(
                        *composite,
                        Some(PixelRect {
                            x0: 0,
                            y0: 0,
                            x1: 50,
                            y1: 50
                        })
                    );
                } else if filter.blur_sigma > 0.0 {
                    let r = f.unwrap().box_radius.unwrap() as i32;
                    assert_eq!(
                        *composite,
                        Some(
                            PixelRect {
                                x0: 10 - 3 * r,
                                y0: 10 - 3 * r,
                                x1: 20 + 3 * r,
                                y1: 20 + 3 * r
                            }
                            .intersect(PixelRect {
                                x0: 0,
                                y0: 0,
                                x1: 50,
                                y1: 50
                            })
                        ),
                        "the blur grows the composited region by its reach"
                    );
                }
            }
        }
    }

    #[test]
    fn layers_deeper_than_the_target_limit_are_reported_once_each() {
        let mut scene = Scene::default();
        for _ in 0..(MAX_TARGET_DEPTH + 3) {
            layer(
                &mut scene,
                0.9,
                BlendMode::Normal,
                ImageFilter::NONE,
                Clip::NONE,
            );
        }
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );
        for _ in 0..(MAX_TARGET_DEPTH + 3) {
            scene.push_command(Command::PopLayer);
        }
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            Clip::NONE,
        );
        let plan = plan(&scene, 100.0, 100.0);
        assert_eq!(plan.unsupported.get(&Unsupported::Layer), Some(&1));
        assert!(plan.target_depth < MAX_TARGET_DEPTH);
        assert_eq!(
            plan.runs.len(),
            1,
            "planning resumes after the skipped stack"
        );
    }

    #[test]
    fn a_shaped_clip_becomes_a_mask_with_a_planner_and_a_gap_without_one() {
        let mut scene = Scene::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            opaque(),
            rounded_clip(),
        );

        let bare = plan(&scene, 100.0, 100.0);
        assert_eq!(bare.unsupported.get(&Unsupported::ShapedClip), Some(&1));
        assert_eq!(
            bare.triangle_count(),
            0,
            "never widened to its bounding box"
        );

        let mut planner = Planner::new();
        let full = planner.plan(&scene, 100.0, 100.0);
        assert!(full.is_complete(), "{:?}", full.unsupported);
        assert!(full.vertices.iter().all(|v| v.mask[2] == 1.0));
        assert_eq!(planner.mask_atlas().len(), 1);
        // A steady second frame reuses the mask and changes no texels.
        let version = planner.mask_atlas().version();
        let _ = planner.plan(&scene, 100.0, 100.0);
        assert_eq!(planner.mask_atlas().version(), version);
    }

    #[test]
    fn every_shadow_variant_is_a_complete_shadow_step() {
        let variants = [
            (
                Shadow::new(Color::BLACK, Offset::new(0.0, 4.0), 12.0),
                Transform::IDENTITY,
            ),
            (
                Shadow::inset(Color::BLACK, Offset::new(0.0, 4.0), 12.0),
                Transform::IDENTITY,
            ),
            (
                Shadow::new(Color::BLACK, Offset::new(2.0, 2.0), 8.0).spread(1.0),
                Transform::rotate(0.4),
            ),
            (
                Shadow::new(Color::BLACK, Offset::ZERO, 6.0),
                Transform::scale(1.5, 0.5),
            ),
        ];
        for (shadow, transform) in variants {
            let mut scene = Scene::default();
            scene.push_command(Command::DrawShadow {
                rect: Rect::new(30.0, 30.0, 60.0, 60.0),
                radius: 6.0,
                shadow,
                transform,
                clip: Clip::NONE,
            });
            let mut planner = Planner::new();
            let plan = planner.plan(&scene, 100.0, 100.0);
            assert!(plan.is_complete(), "{shadow:?}: {:?}", plan.unsupported);
            let Some(Step::Shadow {
                inner, box_radius, ..
            }) = plan.steps.first()
            else {
                panic!("{:?}", plan.steps)
            };
            assert_eq!(inner.is_some(), shadow.is_inset);
            assert!(box_radius.is_some());
            assert_eq!(plan.target_depth, 1, "a shadow uses one work target");
        }
    }

    #[test]
    fn gradients_are_per_fragment_materials_with_a_ramp_row() {
        for gradient in [
            Gradient::horizontal(),
            Gradient::radial(Offset::new(0.5, 0.5), 0.5),
            Gradient::sweep(Offset::new(0.5, 0.5), 0.0, std::f32::consts::TAU),
        ] {
            let gradient = gradient.with_stops(&[(0.0, Color::RED), (1.0, Color::BLUE)]);
            let mut scene = Scene::default();
            scene.push_command(Command::FillRect {
                rect: Rect::new(10.0, 10.0, 30.0, 20.0),
                paint: Paint::gradient(gradient),
                transform: Transform::IDENTITY,
                clip: Clip::NONE,
            });
            assert_eq!(
                plan(&scene, 100.0, 100.0)
                    .unsupported
                    .get(&Unsupported::Gradient),
                Some(&1)
            );
            let mut planner = Planner::new();
            let plan = planner.plan(&scene, 100.0, 100.0);
            assert!(plan.is_complete());
            assert!(plan
                .vertices
                .iter()
                .all(|v| v.texture_kind == material::GRADIENT));
            let corners: Vec<[f32; 2]> = plan.vertices.iter().map(|v| v.local).collect();
            assert!(
                corners.contains(&[0.0, 0.0]) && corners.contains(&[1.0, 1.0]),
                "{corners:?}"
            );
            assert_eq!(planner.ramp_atlas().len(), 1);
        }
    }

    #[test]
    fn a_rotated_image_is_a_rotated_quad_not_its_bounding_box() {
        let image = Image::from_rgba8(vec![255; 16], 2, 2);
        let mut scene = Scene::default();
        scene.push_command(Command::DrawImage {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            image,
            transform: Transform::rotate(std::f32::consts::FRAC_PI_4),
            clip: Clip::NONE,
        });
        let mut planner = Planner::new();
        let plan = planner.plan(&scene, 100.0, 100.0);
        assert!(plan.is_complete());
        assert_eq!(plan.vertices.len(), 4);
        let first = plan.vertices[1].position;
        let expected = Transform::rotate(std::f32::consts::FRAC_PI_4).apply(Offset::new(20.0, 0.0));
        assert!((first[0] - expected.dx).abs() < 1e-4 && (first[1] - expected.dy).abs() < 1e-4);
    }

    #[test]
    fn a_minified_image_blends_two_mip_levels() {
        let image = Image::from_rgba8(vec![200; 64 * 64 * 4], 64, 64);
        let mut scene = Scene::default();
        scene.push_command(Command::DrawImage {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            image,
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        let mut planner = Planner::new();
        let plan = planner.plan(&scene, 100.0, 100.0);
        assert!(plan.is_complete());
        let v = plan.vertices[0];
        assert!(
            v.params[2] < 64.0,
            "the lower level is a mip, not the base: {:?}",
            v.params
        );
        assert!(
            v.mask[3] > 0.0 && v.mask[3] < 1.0,
            "a fractional level blends: {}",
            v.mask[3]
        );
    }

    #[test]
    fn a_clip_that_lets_nothing_through_draws_nothing_and_is_not_a_gap() {
        let mut clip = Clip::NONE;
        clip.add_rect(Rect::new(5.0, 5.0, 5.0, 5.0));
        let mut scene = Scene::default();
        solid(&mut scene, Rect::new(0.0, 0.0, 10.0, 10.0), opaque(), clip);
        let plan = plan(&scene, 100.0, 100.0);
        assert!(plan.is_complete());
        assert_eq!(plan.triangle_count(), 0);
    }

    #[test]
    fn a_transform_moves_the_geometry_it_is_recorded_with() {
        let mut scene = Scene::default();
        scene.push_command(Command::FillRect {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            paint: Paint::solid(opaque()),
            transform: Transform::translate(Offset::new(30.0, 40.0)),
            clip: Clip::NONE,
        });
        let plan = plan(&scene, 100.0, 100.0);
        assert!(plan
            .vertices
            .iter()
            .all(|v| (30.0..=40.0).contains(&v.position[0])));
        assert!(plan
            .vertices
            .iter()
            .all(|v| (40.0..=50.0).contains(&v.position[1])));
    }

    #[test]
    fn a_stroke_produces_geometry_around_a_line_with_no_area() {
        let mut line = Path::new();
        line.move_to(Offset::new(10.0, 10.0));
        line.line_to(Offset::new(90.0, 10.0));
        let mut scene = Scene::default();
        scene.push_command(Command::StrokePath {
            path: line,
            stroke: Stroke::new(4.0),
            paint: Paint::solid(opaque()),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        let plan = plan(&scene, 100.0, 100.0);
        assert!(plan.is_complete());
        assert!(plan.triangle_count() >= 2);
    }

    #[test]
    fn the_colour_reaching_the_vertices_is_the_straight_paint_colour() {
        let mut scene = Scene::default();
        solid(
            &mut scene,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::rgba(255, 128, 0, 64),
            Clip::NONE,
        );
        let plan = plan(&scene, 100.0, 100.0);
        for vertex in &plan.vertices {
            assert!((vertex.color[1] - 128.0 / 255.0).abs() < 1e-6);
            assert!((vertex.color[3] - 64.0 / 255.0).abs() < 1e-6);
        }
    }

    #[test]
    fn the_vertex_layout_is_twenty_three_floats() {
        assert_eq!(std::mem::size_of::<Vertex>(), 23 * 4);
        assert_eq!(std::mem::align_of::<Vertex>(), 4);
    }
}
