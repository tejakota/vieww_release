//! The CPU reference renderer: the oracle every backend is measured against.
//!
//! Spec §12.1: "a complete CPU implementation of the same Frame command set
//! ... deliberately slow and deliberately simple; its job is to be obviously
//! right so the GPU can be measurably right." [`NativeRenderer`] is that
//! implementation — and, since the `gpu`/`hybrid`/`cpu` vello-based backends
//! this crate used to also ship were removed entirely, it is now the
//! renderer itself rather than one oracle among several. Its public shape
//! (`new`, `render_to_pixels`, `render_to_png`, `render_damaged`) is the
//! same one those backends used to share, so an existing call site that
//! targeted one of them swaps by import rather than by rewrite. See
//! `docs/RENDERER-MIGRATION.md` for the full migration history.
//!
//! # What it does differently from vello, on purpose
//!
//! Every [`vieww_foundation::BlendMode`] composites correctly and none are
//! substituted (spec §7.2) — [`SceneReport::unsupported_blends`] is wired
//! for parity with vello's report shape but this renderer never increments
//! it, because [`PushLayer`](crate::Command::PushLayer) always
//! resolves into its own isolated buffer before compositing onto its parent
//! (see the walk below), which is what confines the six coverage-changing
//! modes to their own layer instead of letting them escape across the whole
//! target the way vello's un-isolated compositor does (audited in
//! `docs/RENDERER-MIGRATION.md`, reproducing the measurement spec §7.2
//! cites).

use std::fmt;

use crate::{Clip, Command, Damage, Scene};
use vieww_foundation::{Color, MemoryPressure, Rect, Transform, Trim};

use super::clip::{clip_bounds, resolve_clip_cached, ClipCache};
use super::color::Premul;
use super::color_glyphs::{ColorGlyph, ColorGlyphCache};
use super::geometry::{fill::rasterize, flatten::flatten_path, stroke::stroke_to_polygons};
use super::glyph::GlyphCache;
use super::glyph_raster::GlyphRasterCache;
use super::gradient;
use super::image::{
    invert as invert_transform, minification_ratio, sample_device_pixel_mipped,
    sample_device_pixel_with, MipCache,
};
use super::linear::{blend_with_pipeline, ColorPipeline};
use super::pool::{PoolStats, TargetPool};
use super::rounded_rect::rounded_rect_polygon_transformed;
use super::shadow::ShadowCache;
use super::target::Target;
use super::{effects, Pixels};

#[derive(Debug)]
pub enum RendererError {
    Render(String),
}

impl fmt::Display for RendererError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render(message) => write!(f, "vieww's native renderer: {message}"),
        }
    }
}

impl std::error::Error for RendererError {}

/// Hits and misses on the rasterised-glyph cache — see
/// [`NativeRenderer::glyph_raster_cache_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GlyphRasterStats {
    pub hits: u64,
    pub misses: u64,
}

/// The antialiasing policy for text — the one knob Phase 2's "AA modes"
/// asks a renderer to offer.
///
/// [`Gray`](AaMode::Gray) is the shared policy every shape already gets: four vertical
/// subsamples and exact analytic horizontal coverage, one alpha per pixel.
/// [`Lcd`](AaMode::Lcd) rasterises glyph coverage at three times the horizontal
/// resolution — one alpha per RGB sub-column — and blends each colour
/// channel with its own coverage, which is sharper on the horizontal stems
/// that dominate Latin text and is what a colour display's sub-pixel layout
/// makes possible.
///
/// # Where LCD applies, and where it deliberately does not
///
/// Per-channel blending is only defined over an **opaque** destination: a
/// translucent layer composited with per-channel fringes would show colour
/// halos wherever it lands later. So the renderer routes a glyph to the LCD
/// path only when all of these hold — and silently falls back to [`Gray`](AaMode::Gray)
/// coverage otherwise, as a separate cache entry rather than a conversion:
///
/// - the destination is the root surface, not a layer — layer glyphs are
///   gray;
/// - the frame's background is opaque — a transparent root is gray;
/// - the colour pipeline is [`GammaSpace`](ColorPipeline::GammaSpace) — LCD
///   filtering is a gamma-space idea; a linear-light pipeline gets gray;
/// - the glyph is monochrome — colour glyphs (COLRv0 layers, bitmap emoji)
///   are gray, because per-channel coverage would tint palette-resolved
///   layer colours.
///
/// Every other shape keeps [`Gray`](AaMode::Gray) regardless: LCD is a *text* mode, not a
/// scene mode, because a panel's edge is not a place where sub-pixel stripes
/// buy sharpness.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AaMode {
    /// Grayscale coverage — one alpha per pixel, the default and the policy
    /// every non-text shape always uses.
    #[default]
    Gray,
    /// RGB LCD subpixel coverage for text — three alphas per pixel, opaque
    /// root surfaces only, with automatic grayscale fallback everywhere else.
    Lcd,
}

/// Counted work for one translation — the same shape `vello_cpu`'s
/// `SceneReport` reports (spec §10.4, §2.4: "a visual feature that quietly
/// did not happen looks like a design decision"), so one [`Scene`] rendered
/// through both can have its counts diffed directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneReport {
    pub translated_commands: usize,
    pub skipped_commands: usize,
    pub filtered_layers: usize,
    pub shapes: usize,
    pub images: usize,
    pub glyph_runs: usize,
    pub glyphs: usize,
    /// Glyphs drawn from a colour table — COLRv0 layers or a CBDT/sbix
    /// bitmap — rather than the monochrome path. The number that says a
    /// font's emoji actually reached the screen instead of advancing blank.
    pub colour_glyphs: usize,
    pub clips: usize,
    pub shadows: usize,
    pub layers: usize,
    /// Always zero on this renderer — see the module docs.
    pub unsupported_blends: usize,
}

pub struct NativeRenderer {
    glyphs: GlyphCache,
    /// Clip masks already rasterised this frame — see `native/clip.rs`'s
    /// `ClipCache`, which is where the measurement that made this necessary
    /// is written down.
    clips: ClipCache,
    /// Blurred shadow patches, kept between frames — see `native/shadow.rs`'s
    /// `ShadowCache`.
    shadows: ShadowCache,
    /// Colour-glyph tables and decoded bitmap strikes — see
    /// `native/color_glyphs.rs`.
    color_glyphs: ColorGlyphCache,
    /// Mip pyramids for minified images — see `native/image.rs`.
    image_mips: MipCache,
    /// The root frame buffer, kept between frames.
    ///
    /// A window renders the same size over and over, so the buffer it renders
    /// into is reallocated and refilled for nothing on every frame but the
    /// first — 14.8 MB of it at 1366x679. See `Target::reset`, and the
    /// `floor_probe` example for the measurement.
    root: Target,
    /// The RGBA8 output, likewise kept — see `Target::write_rgba8`.
    output: Vec<u8>,
    /// Reused `PushLayer`/`PopLayer` offscreen buffers — see `native/pool.rs`'s
    /// module docs for why this is safe and what it does and does not save.
    targets: TargetPool,
    /// Rasterised glyph coverage, kept between frames — see
    /// `native/glyph_raster.rs`, which is where the reason a text-heavy window
    /// needs this at all is written down.
    glyph_rasters: GlyphRasterCache,
    /// Gamma-space (default, byte-identical to this renderer's historical
    /// behavior) or linear-light compositing — see `native/linear.rs`'s
    /// module docs.
    pipeline: ColorPipeline,
    /// The text antialiasing policy — see [`AaMode`].
    aa: AaMode,
    /// Whether *this frame* may route root-surface glyphs through the LCD
    /// path: the [`AaMode::Lcd`] choice, an opaque background, and the
    /// gamma-space pipeline, all at once. Recomputed at the top of every
    /// render entry point; `apply` reads it, so a glyph run inside a layer
    /// can fall back to gray by one further `stack.is_empty()` test.
    lcd_frame: bool,
}

impl fmt::Debug for NativeRenderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRenderer")
            .field("cached_fonts", &self.glyphs.cached_fonts())
            .finish_non_exhaustive()
    }
}

impl Default for NativeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

struct LayerFrame {
    target: Target,
    /// This frame's `(0, 0)` in device space.
    origin: (i32, i32),
    alpha: f32,
    blend: vieww_foundation::BlendMode,
    filter: vieww_foundation::ImageFilter,
    /// The clip in force when the layer was pushed.
    ///
    /// The rect half was already intersected into the layer's bounds at push
    /// (a smaller offscreen buffer for a rect-clipped layer); the *shape* half
    /// is applied at pop, as a coverage multiply over the resolved buffer —
    /// see the `PopLayer` handler for why it happens there rather than at
    /// push.
    clip: Clip,
}

impl NativeRenderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            glyphs: GlyphCache::new(),
            clips: ClipCache::new(),
            shadows: ShadowCache::new(),
            color_glyphs: ColorGlyphCache::default(),
            image_mips: MipCache::default(),
            root: Target::new(0, 0),
            output: Vec::new(),
            targets: TargetPool::new(),
            glyph_rasters: GlyphRasterCache::default(),
            pipeline: ColorPipeline::GammaSpace,
            aa: AaMode::Gray,
            lcd_frame: false,
        }
    }

    /// Like [`Self::new`], but with each font's glyph-outline residency
    /// cache budgeted at `budget_bytes` instead of the built-in default —
    /// see `native/glyph.rs`'s `GlyphCache::with_glyph_outline_budget_bytes`.
    /// A smaller budget evicts more eagerly, trading rasterization time for
    /// memory; this is what lets a memory-constrained target — or a test
    /// that wants eviction to happen deterministically rather than waiting
    /// on however many distinct glyphs real content happens to touch — ask
    /// for a different trade than the default.
    #[must_use]
    pub fn with_glyph_outline_budget_bytes(budget_bytes: usize) -> Self {
        Self {
            glyphs: GlyphCache::with_glyph_outline_budget_bytes(budget_bytes),
            clips: ClipCache::new(),
            shadows: ShadowCache::new(),
            color_glyphs: ColorGlyphCache::default(),
            image_mips: MipCache::default(),
            root: Target::new(0, 0),
            output: Vec::new(),
            targets: TargetPool::new(),
            glyph_rasters: GlyphRasterCache::default(),
            pipeline: ColorPipeline::GammaSpace,
            aa: AaMode::Gray,
            lcd_frame: false,
        }
    }

    /// Like [`Self::new`], but compositing in the color space `pipeline`
    /// selects — see `native/linear.rs`'s module docs. Defaults to
    /// [`ColorPipeline::GammaSpace`] (this renderer's historical behavior)
    /// everywhere else; this is the one entry point that can ask for
    /// [`ColorPipeline::LinearLight`] instead.
    #[must_use]
    pub fn with_color_pipeline(pipeline: ColorPipeline) -> Self {
        Self {
            glyphs: GlyphCache::new(),
            clips: ClipCache::new(),
            shadows: ShadowCache::new(),
            color_glyphs: ColorGlyphCache::default(),
            image_mips: MipCache::default(),
            root: Target::new(0, 0),
            output: Vec::new(),
            targets: TargetPool::new(),
            glyph_rasters: GlyphRasterCache::default(),
            pipeline,
            aa: AaMode::Gray,
            lcd_frame: false,
        }
    }

    /// Like [`Self::new`], but rasterising and compositing text under `mode`'s
    /// antialiasing policy — see [`AaMode`] for where [`AaMode::Lcd`] applies
    /// and where it falls back to grayscale. This is the one entry point that
    /// can ask for LCD; everything else defaults to [`AaMode::Gray`], which is
    /// byte-identical to this renderer's historical output.
    #[must_use]
    pub fn with_aa_mode(mode: AaMode) -> Self {
        Self {
            glyphs: GlyphCache::new(),
            clips: ClipCache::new(),
            shadows: ShadowCache::new(),
            color_glyphs: ColorGlyphCache::default(),
            image_mips: MipCache::default(),
            root: Target::new(0, 0),
            output: Vec::new(),
            targets: TargetPool::new(),
            glyph_rasters: GlyphRasterCache::default(),
            pipeline: ColorPipeline::GammaSpace,
            aa: mode,
            lcd_frame: false,
        }
    }

    /// Which color space this renderer composites in.
    #[must_use]
    pub fn color_pipeline(&self) -> ColorPipeline {
        self.pipeline
    }

    /// The text antialiasing policy this renderer was built with.
    #[must_use]
    pub fn aa_mode(&self) -> AaMode {
        self.aa
    }

    /// Whether `base` as a frame background admits the LCD path this frame —
    /// the mode choice, an opaque background, and the gamma-space pipeline.
    /// Written into [`Self::lcd_frame`] at the top of every render entry
    /// point, read per glyph run in `apply`.
    fn lcd_frame_valid(&self, base: Color) -> bool {
        matches!(self.aa, AaMode::Lcd)
            && matches!(self.pipeline, ColorPipeline::GammaSpace)
            && base.a == 255
    }

    #[must_use]
    pub fn cached_fonts(&self) -> usize {
        self.glyphs.cached_fonts()
    }

    /// How many `PushLayer` offscreen buffers this renderer has reused
    /// versus freshly allocated, over its whole lifetime — see
    /// `native/pool.rs`'s module docs.
    #[must_use]
    pub fn pool_stats(&self) -> PoolStats {
        self.targets.stats()
    }

    /// Hits, misses and evictions across every font's glyph-outline
    /// residency cache, over this renderer's whole lifetime — see
    /// `native/residency.rs`'s and `native/glyph.rs`'s module docs.
    #[must_use]
    pub fn glyph_outline_cache_stats(&self) -> super::ResidencyStats {
        self.glyphs.cached_glyph_outline_stats()
    }

    /// Hits and misses on the blurred-shadow-patch cache over this renderer's
    /// whole lifetime — see `native/shadow.rs`'s `ShadowCache`.
    ///
    /// Re-blurring a shadow that has not moved is the single most expensive
    /// thing a repaint can do for no change, so this is the number that says
    /// whether a scene is doing it.
    #[must_use]
    pub fn shadow_cache_stats(&self) -> (usize, usize) {
        self.shadows.stats()
    }

    /// Hits and misses on the rasterised-glyph cache over this renderer's
    /// whole lifetime — see `native/glyph_raster.rs`.
    ///
    /// This is the number that says whether a window is re-rasterising its
    /// text: a second frame of unchanged text is all hits, and a regression
    /// that reintroduces per-frame glyph rasterisation shows up here as misses
    /// long before it shows up as a dropped frame.
    #[must_use]
    pub fn glyph_raster_cache_stats(&self) -> GlyphRasterStats {
        let (hits, misses) = self.glyph_rasters.stats();
        GlyphRasterStats { hits, misses }
    }

    /// How many bitmap colour glyphs (CBDT/sbix strikes, e.g. emoji) this
    /// renderer has decoded — see `native/color_glyphs.rs`. The second frame
    /// of the same emoji adds nothing to it; a regression that re-decodes
    /// strikes per frame doubles it.
    #[must_use]
    pub fn decoded_color_glyphs(&self) -> usize {
        self.color_glyphs.decoded_bitmaps()
    }

    /// How many mip pyramids this renderer has generated — see
    /// `native/image.rs`. Like the glyph numbers, this is the "minification
    /// does not re-filter the pyramid every frame" number.
    #[must_use]
    pub fn generated_mip_chains(&self) -> usize {
        self.image_mips.generated_chains()
    }

    /// Rasterize `scene` over `base` into straight-alpha RGBA8 pixels.
    pub fn render_to_pixels(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<(Pixels, SceneReport), RendererError> {
        let report = self.render_in_place(scene, width, height, base)?;
        Ok((
            Pixels::from_rgba8(self.output.clone(), width, height),
            report,
        ))
    }

    /// [`render_to_pixels`](Self::render_to_pixels) without handing out a new
    /// buffer: the frame is left in [`last_frame`](Self::last_frame).
    ///
    /// # Why this exists
    ///
    /// `render_to_pixels` returns an owned [`Pixels`], which is a copy of the
    /// renderer's retained output — one full-frame allocation and memcpy per
    /// call, 5.6 MB at 1440x900, whether or not a single pixel changed. The
    /// Vieww standard's steady-state clause is *zero* allocations, and the
    /// `vieww-standard` runner counts them with a real allocator; a presenter
    /// that only needs to read the bytes should take them from here.
    ///
    /// # Errors
    ///
    /// As [`render_to_pixels`](Self::render_to_pixels).
    pub fn render_in_place(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<SceneReport, RendererError> {
        let surface = Rect::new(0.0, 0.0, width as f32, height as f32);
        // Whether this frame's root admits LCD text — the mode choice, an
        // opaque background and the gamma-space pipeline together. Read per
        // glyph run in `apply`; see `AaMode` for why each condition is there.
        self.lcd_frame = self.lcd_frame_valid(base);
        // Taken out of `self` so the command walk can borrow the renderer's
        // caches mutably at the same time, and put back at the end — the
        // buffer survives the frame either way, including the error paths
        // below, because it is restored before any of them return.
        let mut root = std::mem::replace(&mut self.root, Target::new(0, 0));
        root.reset(width, height, Premul::from_straight(base));
        let mut report = SceneReport::default();

        let mut stack: Vec<LayerFrame> = Vec::new();
        let mut failure = None;
        for command in scene.commands() {
            if let Err(error) = self.apply(command, &mut root, &mut stack, surface, &mut report) {
                failure = Some(error);
                break;
            }
        }
        let unclosed = stack.len();
        for frame in stack {
            self.targets.release(frame.target);
        }

        if failure.is_none() && unclosed == 0 {
            let wanted = (width as usize) * (height as usize) * 4;
            if self.output.len() != wanted {
                self.output.resize(wanted, 0);
            }
            root.write_rgba8(&mut self.output);
        }
        self.root = root;

        if let Some(error) = failure {
            return Err(error);
        }
        if unclosed > 0 {
            return Err(RendererError::Render(format!(
                "{unclosed} unclosed PushLayer command(s)"
            )));
        }
        Ok(report)
    }

    /// The straight-alpha RGBA8 bytes of the last frame this renderer
    /// completed, `width * height * 4` of the size it was rendered at.
    #[must_use]
    pub fn last_frame(&self) -> &[u8] {
        &self.output
    }

    /// Repaint only what `damage` says changed, **onto the pixels this
    /// renderer already has**, and hand back the whole surface.
    ///
    /// # The difference from `render_damaged`
    ///
    /// [`render_damaged`](Self::render_damaged) culls the *scene* to the
    /// damage and then renders that culled scene into a fresh full-size
    /// buffer, converting every pixel on the way out. It saves command work
    /// and nothing else, which is why a frame in which literally nothing
    /// changed still cost 11.4 ms on a 1440x900 Studio shell — the floor is
    /// the buffer, not the drawing.
    ///
    /// This keeps the buffer. Each damaged region is cleared to `base`,
    /// repainted with the scene restricted to that region, and converted to
    /// RGBA8 — and every pixel outside every region is last frame's, still
    /// correct precisely because nothing damaged it. A clean frame therefore
    /// costs approximately nothing, and a sidebar click costs the sidebar.
    ///
    /// # Correctness, and how it is checked
    ///
    /// The invariant is that this must produce the **same bytes** as
    /// [`render_to_pixels`](Self::render_to_pixels) on the same scene,
    /// whenever the damage honestly describes what changed. Two things make
    /// that true and both are easy to get wrong:
    ///
    /// - The region is cleared before it is repainted. Damage says what to
    ///   redraw, not what to skip clearing; a shape that moved leaves its old
    ///   pixels behind otherwise.
    /// - The region is passed down as the *surface*, so every clip, layer and
    ///   shadow is confined to it exactly as it would be confined to the
    ///   window. Restricting by culling commands alone would let a blur or a
    ///   shadow reach outside its region and paint over a neighbour's pixels
    ///   that nothing had cleared.
    ///
    /// `vieww-test-harness`'s `assert_partial_repaint_is_complete` is the
    /// oracle for the other direction — that the damage was large enough in
    /// the first place.
    ///
    /// # Errors
    ///
    /// As [`render_to_pixels`](Self::render_to_pixels).
    pub fn render_retained(
        &mut self,
        scene: &Scene,
        damage: &Damage,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<(Pixels, SceneReport), RendererError> {
        let report = self.render_retained_in_place(scene, damage, width, height, base)?;
        Ok((
            Pixels::from_rgba8(self.output.clone(), width, height),
            report,
        ))
    }

    /// [`render_retained`](Self::render_retained), leaving the frame in
    /// [`last_frame`](Self::last_frame) instead of copying it out — a clean
    /// frame through this path allocates nothing. See
    /// [`render_in_place`](Self::render_in_place).
    ///
    /// # Errors
    ///
    /// As [`render_to_pixels`](Self::render_to_pixels).
    pub fn render_retained_in_place(
        &mut self,
        scene: &Scene,
        damage: &Damage,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<SceneReport, RendererError> {
        let wanted = (width as usize) * (height as usize) * 4;
        // Same per-frame LCD admission as `render_to_pixels` — the retained
        // path repaints regions of an existing buffer, and that buffer's
        // background is this same `base` (a resize or first frame takes the
        // delegation below, which recomputes it anyway).
        self.lcd_frame = self.lcd_frame_valid(base);
        // A resize, a first frame, or damage that covers everything: there is
        // no retained content worth keeping, so this is a full render.
        if damage.is_everything()
            || self.root.width != width
            || self.root.height != height
            || self.output.len() != wanted
        {
            return self.render_in_place(scene, width, height, base);
        }

        let full = Rect::new(0.0, 0.0, width as f32, height as f32);
        let regions = damage.repaint_regions();
        let mut report = SceneReport::default();
        if regions.is_empty() {
            // Nothing changed. The buffer already holds the right picture and
            // the output already holds the right bytes — this is the case a
            // full-frame renderer spends its whole floor on.
            report.skipped_commands = scene.len();
            return Ok(report);
        }

        let mut root = std::mem::replace(&mut self.root, Target::new(0, 0));
        let mut output = std::mem::take(&mut self.output);
        let clear = Premul::from_straight(base);
        let mut failure = None;

        for region in regions {
            let region = region.intersect(full);
            if region.is_empty() {
                continue;
            }
            let bounds = (
                region.left.floor() as i32,
                region.top.floor() as i32,
                region.right.ceil() as i32,
                region.bottom.ceil() as i32,
            );
            let region_rect = Rect::new(
                bounds.0 as f32,
                bounds.1 as f32,
                bounds.2 as f32,
                bounds.3 as f32,
            );

            root.reset_region(bounds, clear);
            // **The surface stays the window; the mask is what narrows.**
            // Every effect — a shadow's blurred patch, a filtered layer's
            // kernel — is sized against the real surface so it computes the
            // same values it would in a full frame, and only the pixels
            // inside the region are allowed to land. Narrowing the surface
            // instead truncates the input to those effects and produces a
            // repaint that is wrong in a thin band along every region edge.
            // See `Target::write_mask`.
            root.set_write_mask(Some(bounds));

            // Commands that cannot reach this region at all are skipped —
            // this is the work a damaged frame actually saves.
            let mut stack: Vec<LayerFrame> = Vec::new();
            for command in scene.commands() {
                let is_layer = matches!(command, Command::PushLayer { .. } | Command::PopLayer);
                if !is_layer && command.bounds().intersect(region_rect).is_empty() {
                    report.skipped_commands += 1;
                    continue;
                }
                if let Err(error) = self.apply(command, &mut root, &mut stack, full, &mut report) {
                    failure = Some(error);
                    break;
                }
            }
            root.set_write_mask(None);
            let unclosed = stack.len();
            for frame in stack {
                self.targets.release(frame.target);
            }
            if failure.is_none() && unclosed > 0 {
                failure = Some(RendererError::Render(format!(
                    "{unclosed} unclosed PushLayer command(s)"
                )));
            }
            if failure.is_some() {
                break;
            }

            root.write_rgba8_region(&mut output, bounds);
        }

        self.root = root;
        self.output = output;

        if let Some(error) = failure {
            return Err(error);
        }
        Ok(report)
    }

    /// [`Self::render_to_pixels`], PNG-encoded — the "no display needed"
    /// entry point spec §12.1 calls the debugging loop that keeps a
    /// one-person project moving.
    pub fn render_to_png(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<(Vec<u8>, SceneReport), RendererError> {
        let (pixels, report) = self.render_to_pixels(scene, width, height, base)?;
        let png = pixels
            .encode_png()
            .map_err(|e| RendererError::Render(e.to_string()))?;
        Ok((png, report))
    }

    /// Rasterize only what `damage` says changed, background repainted in
    /// full: a damage region says what to redraw, not what to skip clearing
    /// first, so a shrunk or moved shape's old pixels are gone from the
    /// output — a partial-background repaint would leave them.
    pub fn render_damaged(
        &mut self,
        scene: &Scene,
        damage: &Damage,
        width: u32,
        height: u32,
        base: Color,
    ) -> Result<(Pixels, SceneReport), RendererError> {
        if damage.is_clean() {
            let (pixels, _) = self.render_to_pixels(&Scene::new(), width, height, base)?;
            return Ok((
                pixels,
                SceneReport {
                    skipped_commands: scene.len(),
                    ..SceneReport::default()
                },
            ));
        }
        if damage.is_everything() {
            return self.render_to_pixels(scene, width, height, base);
        }
        let (culled, _) = scene.damage_cull(damage);
        // A single render, not two: this used to call `render_to_pixels` on
        // `culled` twice and discard the first result, which cost every
        // damaged frame a second full rasterization pass for nothing.
        let (pixels, mut report) = self.render_to_pixels(&culled, width, height, base)?;
        report.skipped_commands += scene.len() - culled.len();
        Ok((pixels, report))
    }

    fn apply(
        &mut self,
        command: &Command,
        root: &mut Target,
        stack: &mut Vec<LayerFrame>,
        surface: Rect,
        report: &mut SceneReport,
    ) -> Result<(), RendererError> {
        macro_rules! current {
            () => {
                match stack.last_mut() {
                    Some(frame) => (&mut frame.target, frame.origin),
                    None => (&mut *root, (0, 0)),
                }
            };
        }

        match command {
            Command::FillRect {
                rect,
                paint,
                transform,
                clip,
            } => {
                let polygon = rect_polygon(*rect, *transform);
                let (target, origin) = current!();
                self.paint_shape(
                    target,
                    origin,
                    &[polygon],
                    *rect,
                    *transform,
                    paint,
                    clip,
                    surface,
                );
                report.shapes += 1;
                // Shaped clips are counted work on shapes exactly as they
                // are on glyphs — the `clips` field says "a clip mask was
                // resolved for this command", and most clipped commands are
                // shapes, so a counter that only ticked on glyph runs
                // understated the real clip cost by an order of magnitude.
                if !clip.shapes().is_empty() {
                    report.clips += 1;
                }
                report.translated_commands += 1;
            }
            Command::FillPath {
                path,
                paint,
                transform,
                clip,
            } => {
                let polylines = flatten_path(path, *transform);
                let (target, origin) = current!();
                self.paint_shape(
                    target,
                    origin,
                    &polylines,
                    path.bounds(),
                    *transform,
                    paint,
                    clip,
                    surface,
                );
                report.shapes += 1;
                if !clip.shapes().is_empty() {
                    report.clips += 1;
                }
                report.translated_commands += 1;
            }
            Command::StrokePath {
                path,
                stroke,
                paint,
                transform,
                clip,
            } => {
                let polygons = stroke_to_polygons(path, stroke.width, &stroke.style, *transform);
                let bounds = path.bounds().inflate(stroke.reach());
                let (target, origin) = current!();
                self.paint_shape(
                    target, origin, &polygons, bounds, *transform, paint, clip, surface,
                );
                report.shapes += 1;
                if !clip.shapes().is_empty() {
                    report.clips += 1;
                }
                report.translated_commands += 1;
            }
            Command::DrawShadow {
                rect,
                radius,
                shadow,
                transform,
                clip,
            } => {
                let (target, origin) = current!();
                let local_surface = Rect::new(
                    origin.0 as f32,
                    origin.1 as f32,
                    (origin.0 + target.width as i32) as f32,
                    (origin.1 + target.height as i32) as f32,
                )
                .intersect(surface);
                // The patch comes from the cache: a panel that has not moved
                // has the shadow it had last frame, and re-blurring it is the
                // single most expensive thing a repaint can do for no change.
                //
                // The two caches are borrowed disjointly rather than through
                // `self`, so the patch can be composited **by reference**. A
                // panel's patch is panel-sized plus the blur's reach on every
                // side; copying one out of the cache on every frame would give
                // back a good part of what caching it saved.
                let Self {
                    shadows,
                    clips,
                    pipeline,
                    ..
                } = self;
                let pipeline = *pipeline;
                if let Some((patch, px0, py0)) =
                    shadows.patch(*rect, *radius, shadow, *transform, local_surface)
                {
                    let need = Rect::new(
                        px0 as f32,
                        py0 as f32,
                        (px0 + patch.width as i32) as f32,
                        (py0 + patch.height as i32) as f32,
                    );
                    let resolved = resolve_clip_cached(clips, clip, *transform, surface, need);
                    let (target, origin) = match stack.last_mut() {
                        Some(frame) => (&mut frame.target, frame.origin),
                        None => (&mut *root, (0, 0)),
                    };
                    // A clip that carries no mask — no clip at all, or a plain
                    // rectangle, which the patch is already confined to —
                    // leaves every pixel at full coverage, so the cached patch
                    // composites straight through. Only a shape clip needs a
                    // modified copy, and only then is one made.
                    if resolved.has_mask() {
                        let mut clipped = patch.clone();
                        for y in 0..clipped.height as i32 {
                            let row = y as u32 * clipped.width;
                            for x in 0..clipped.width as i32 {
                                let c = resolved.coverage_at(px0 + x, py0 + y);
                                if c < 1.0 {
                                    let i = (row + x as u32) as usize;
                                    clipped.pixels[i] = clipped.pixels[i].scaled(c);
                                }
                            }
                        }
                        target.composite_layer(
                            &clipped,
                            px0 - origin.0,
                            py0 - origin.1,
                            vieww_foundation::BlendMode::Normal,
                            1.0,
                            pipeline,
                        );
                    } else {
                        target.composite_layer(
                            patch,
                            px0 - origin.0,
                            py0 - origin.1,
                            vieww_foundation::BlendMode::Normal,
                            1.0,
                            pipeline,
                        );
                    }
                }
                report.shadows += 1;
                report.translated_commands += 1;
            }
            Command::DrawImage {
                rect,
                image,
                transform,
                clip,
            } => {
                let (target, origin) = current!();
                self.paint_image(target, origin, image, *rect, *transform, clip, surface);
                report.images += 1;
                if !clip.shapes().is_empty() {
                    report.clips += 1;
                }
                report.translated_commands += 1;
            }
            Command::DrawGlyphs {
                run,
                transform,
                clip,
            } => {
                let bounds = clip_bounds(clip, surface);
                let color = Premul::from_straight(run.color);
                for glyph in run.glyphs.iter() {
                    // **Colour glyphs are asked about first**, because for
                    // the fonts that have them the monochrome answer is
                    // *wrong*, not merely incomplete: a bitmap-only emoji
                    // font has no outline to extract, so the path below
                    // skips the glyph as `NoOutline` and the run advances
                    // blank. For every font without colour tables the
                    // question costs one `HashMap` miss on the font id and
                    // falls straight through — see `native/color_glyphs.rs`.
                    let colour = self.color_glyphs.resolve(
                        &run.font,
                        glyph.id,
                        run.origin,
                        glyph.offset,
                        run.size,
                        *transform,
                    );
                    match colour {
                        Some(ColorGlyph::Layers(layers)) => {
                            for (layer_glyph, layer_color) in layers {
                                // Each layer is an ordinary glyph of the
                                // *same* font — same cache, same subpixel
                                // phase, same clip machinery — drawn in the
                                // layer's palette colour instead of the
                                // run's. Everything else about this loop is
                                // the monochrome one below, deliberately:
                                // two paths that differ in colour alone can
                                // be diffed by eye.
                                let Self {
                                    glyphs,
                                    glyph_rasters,
                                    clips,
                                    pipeline,
                                    ..
                                } = self;
                                let pipeline = *pipeline;
                                let font = &run.font;
                                // Colour layers are always gray coverage: LCD's
                                // per-channel blending would tint a
                                // palette-resolved layer colour, so the `lcd`
                                // flag below is unconditionally false here —
                                // see `AaMode`'s documentation.
                                let placed = glyph_rasters.coverage(
                                    font,
                                    layer_glyph,
                                    run.size,
                                    run.origin,
                                    glyph.offset,
                                    *transform,
                                    false,
                                    || {
                                        let (outline, upm) = glyphs.outline(font, layer_glyph);
                                        outline.map(|path| (path, upm))
                                    },
                                );
                                let (mask, gx, gy) = match placed {
                                    super::glyph_raster::Glyph::NoOutline => continue,
                                    super::glyph_raster::Glyph::Blank => continue,
                                    super::glyph_raster::Glyph::Inked { raster, x, y } => {
                                        let Some(mask) = raster.as_gray() else {
                                            continue;
                                        };
                                        (mask, x, y)
                                    }
                                };
                                let Some(view) = mask.view_translated(gx, gy, bounds) else {
                                    continue;
                                };
                                let need = Rect::new(
                                    view.x0 as f32,
                                    view.y0 as f32,
                                    (view.x0 + view.width as i32) as f32,
                                    (view.y0 + view.height as i32) as f32,
                                );
                                let resolved =
                                    resolve_clip_cached(clips, clip, *transform, surface, need);
                                let layer_color = Premul::from_straight(layer_color);
                                let (target, origin) = match stack.last_mut() {
                                    Some(frame) => (&mut frame.target, frame.origin),
                                    None => (&mut *root, (0, 0)),
                                };
                                if resolved.has_mask() {
                                    target.composite_coverage(
                                        view,
                                        origin,
                                        vieww_foundation::BlendMode::Normal,
                                        pipeline,
                                        |x, y| layer_color.scaled(resolved.coverage_at(x, y)),
                                    );
                                } else {
                                    target.composite_flat(
                                        view,
                                        origin,
                                        vieww_foundation::BlendMode::Normal,
                                        pipeline,
                                        layer_color,
                                    );
                                }
                            }
                            report.glyphs += 1;
                            report.colour_glyphs += 1;
                            continue;
                        }
                        Some(ColorGlyph::Bitmap { image, rect }) => {
                            // The bitmap is drawn with the image sampler, so
                            // rotation, clips and the colour pipeline apply
                            // to it exactly as they do to any `DrawImage` —
                            // including the mip path, which a downscaled
                            // strike can genuinely benefit from.
                            let (target, origin) = match stack.last_mut() {
                                Some(frame) => (&mut frame.target, frame.origin),
                                None => (&mut *root, (0, 0)),
                            };
                            self.paint_image(
                                target, origin, &image, rect, *transform, clip, surface,
                            );
                            report.glyphs += 1;
                            report.colour_glyphs += 1;
                            continue;
                        }
                        None => {}
                    }
                    // **The glyph's coverage comes from the raster cache, not
                    // from a fresh rasterisation.**
                    //
                    // The three allocations and the scanline pass this used to
                    // run for every glyph of every frame are `glyph_raster`'s
                    // miss path now, and a window whose text has not moved
                    // takes none of them. `native/glyph_raster.rs` has the
                    // measurement and the reason the key is exact rather than
                    // phase-quantised.
                    //
                    // The clip is *not* part of that key — the same glyph is
                    // drawn under different clips — so the mask is cached
                    // unclipped and confined to the clip's bounds here, on the
                    // way out, by `view_at`. The clip's *mask*, when it has
                    // one, is still resolved over the glyph's own ink rather
                    // than over the whole clip: a line of code is a few hundred
                    // glyphs sharing one panel-sized clip, and the difference
                    // between those two is the difference between a frame and a
                    // stall.
                    // All four fields borrowed disjointly in one place: the
                    // cached mask is a borrow of `glyph_rasters`, and the clip
                    // resolution that follows needs `clips` mutably at the same
                    // time. Reaching through `self` for the second would be a
                    // second borrow of the whole struct.
                    let Self {
                        glyphs,
                        glyph_rasters,
                        clips,
                        pipeline,
                        lcd_frame,
                        ..
                    } = self;
                    let pipeline = *pipeline;
                    // **The LCD gate, per glyph run.** `lcd_frame` already
                    // holds the frame-level answer (mode on, opaque
                    // background, gamma-space pipeline — see `AaMode`); the
                    // one test left is the destination, because per-channel
                    // blending is only defined over the opaque root and a
                    // layer's glyphs must stay gray. A run that falls back
                    // here is a *separate cache entry*, not a conversion —
                    // the same glyph can be live both ways at once.
                    let use_lcd = *lcd_frame && stack.is_empty();
                    let font = &run.font;
                    let id = glyph.id;
                    let placed = glyph_rasters.coverage(
                        font,
                        id,
                        run.size,
                        run.origin,
                        glyph.offset,
                        *transform,
                        use_lcd,
                        || {
                            let (outline, upm) = glyphs.outline(font, id);
                            outline.map(|path| (path, upm))
                        },
                    );
                    // The count is of glyphs that reached the backend, which
                    // is not the same as glyphs that inked pixels: a space has
                    // no outline and was never drawn, while an outline too
                    // small to cover a pixel was. See `glyph_raster::Glyph`.
                    match placed {
                        super::glyph_raster::Glyph::NoOutline => continue,
                        super::glyph_raster::Glyph::Blank => {
                            report.glyphs += 1;
                            continue;
                        }
                        super::glyph_raster::Glyph::Inked {
                            raster: super::glyph_raster::Rasterized::Lcd(mask),
                            x,
                            y,
                        } => {
                            // The LCD path: same clip machinery, same
                            // position arithmetic, a three-channel view and
                            // `composite_lcd` instead. The run's *straight*
                            // colour goes in — the per-channel blend wants to
                            // scale each channel by its own coverage, which a
                            // pre-scaled colour cannot do.
                            let Some(view) = mask.view_translated(x, y, bounds) else {
                                report.glyphs += 1;
                                continue;
                            };
                            let need = Rect::new(
                                view.x0 as f32,
                                view.y0 as f32,
                                (view.x0 + view.width as i32) as f32,
                                (view.y0 + view.height as i32) as f32,
                            );
                            let resolved =
                                resolve_clip_cached(clips, clip, *transform, surface, need);
                            let (target, origin) = match stack.last_mut() {
                                Some(frame) => (&mut frame.target, frame.origin),
                                None => (&mut *root, (0, 0)),
                            };
                            if resolved.has_mask() {
                                target.composite_lcd(view, origin, run.color, |x, y| {
                                    resolved.coverage_at(x, y)
                                });
                            } else {
                                target.composite_lcd(view, origin, run.color, |_, _| 1.0);
                            }
                            report.glyphs += 1;
                            continue;
                        }
                        super::glyph_raster::Glyph::Inked {
                            raster: super::glyph_raster::Rasterized::Gray(mask),
                            x,
                            y,
                        } => {
                            // The gray path, unchanged — which includes being
                            // what an LCD renderer draws inside layers, and
                            // what every renderer drew before `AaMode` existed.
                            let Some(view) = mask.view_translated(x, y, bounds) else {
                                report.glyphs += 1;
                                continue;
                            };
                            let need = Rect::new(
                                view.x0 as f32,
                                view.y0 as f32,
                                (view.x0 + view.width as i32) as f32,
                                (view.y0 + view.height as i32) as f32,
                            );
                            let resolved =
                                resolve_clip_cached(clips, clip, *transform, surface, need);
                            let (target, origin) = match stack.last_mut() {
                                Some(frame) => (&mut frame.target, frame.origin),
                                None => (&mut *root, (0, 0)),
                            };
                            if resolved.has_mask() {
                                target.composite_coverage(
                                    view,
                                    origin,
                                    vieww_foundation::BlendMode::Normal,
                                    pipeline,
                                    |x, y| color.scaled(resolved.coverage_at(x, y)),
                                );
                            } else {
                                target.composite_flat(
                                    view,
                                    origin,
                                    vieww_foundation::BlendMode::Normal,
                                    pipeline,
                                    color,
                                );
                            }
                            report.glyphs += 1;
                        }
                    }
                }
                report.glyph_runs += 1;
                if !clip.shapes().is_empty() {
                    report.clips += 1;
                }
                report.translated_commands += 1;
            }
            Command::PushLayer {
                bounds,
                alpha,
                blend,
                clip,
                filter,
            } => {
                let parent_bounds = match stack.last() {
                    Some(frame) => Rect::new(
                        frame.origin.0 as f32,
                        frame.origin.1 as f32,
                        (frame.origin.0 + frame.target.width as i32) as f32,
                        (frame.origin.1 + frame.target.height as i32) as f32,
                    ),
                    None => surface,
                };
                // No `.inflate(filter.bounds_expansion())` here: `bounds` has
                // already been grown by exactly that margin. Every `PushLayer`
                // with real content behind it reaches this as the second half
                // of a push/pop pair (`Canvas::pop_layer` — `scene.rs`), and
                // `pop_layer` *replaces* the declared bounds outright with
                // `contents.inflate(reach)` — see its own doc comment ("the
                // single most recognisable way a blur is implemented wrong")
                // for why the margin has to be there at all. Adding `reach`
                // again here doubled it for every filtered layer this
                // renderer has ever drawn: an offscreen buffer (and the blur
                // that runs on it) roughly twice as large as the filter
                // actually reaches, silently, because the two call sites each
                // assumed they were the only one growing the bounds. Found
                // from a visibly oversized glow behind a studio tab
                // indicator — see `blur_bounds_are_expanded_exactly_once`
                // below.
                let dev_bounds = bounds
                    .intersect(parent_bounds)
                    // **The layer's own clip cuts its region.** The `Clip`
                    // recorded with a `PushLayer` is the canvas clip in force
                    // at push time, and this renderer used to discard it
                    // outright (`let _ = clip;`) — a layer pushed inside a
                    // rounded clip escaped the clip entirely, drawing its
                    // whole resolved rectangle over corners the clip was
                    // supposed to remove. The rect half of the clip narrows
                    // the offscreen buffer here, for free; the *shape* half
                    // is applied as a coverage multiply at pop, because
                    // clipping the buffer is not expressible as a rectangle.
                    .intersect(clip_bounds(clip, surface))
                    .intersect(surface);
                let x0 = dev_bounds.left.floor() as i32;
                let y0 = dev_bounds.top.floor() as i32;
                let x1 = dev_bounds.right.ceil() as i32;
                let y1 = dev_bounds.bottom.ceil() as i32;
                let w = (x1 - x0).max(0) as u32;
                let h = (y1 - y0).max(0) as u32;
                #[cfg(test)]
                LAST_LAYER_TARGET_SIZE.with(|cell| cell.set(Some((w, h))));
                let mut target = self.targets.acquire(w, h);
                if filter.backdrop {
                    // Real backdrop sampling: seed this layer's buffer with a
                    // copy of whatever the parent already has painted at this
                    // exact position, then apply the filter to *that* copy
                    // right now — before any of this layer's own content
                    // (the child commands between this `PushLayer` and its
                    // matching `PopLayer`) paints into the same buffer. That
                    // ordering is the whole difference between a real
                    // backdrop filter and a plain one: content painted after
                    // this point must land on top of an already-blurred
                    // background, sharp, not get blurred along with it — see
                    // `ImageFilter::backdrop`'s doc. `PopLayer` below
                    // deliberately does not filter again for a backdrop
                    // layer, because it already happened here.
                    let (parent, parent_origin) = current!();
                    target.snapshot_from(parent, parent_origin, (x0, y0));
                    if filter.blur_sigma > 0.0 {
                        effects::blur(&mut target, filter.blur_sigma);
                    }
                    if let Some(matrix) = &filter.color_matrix {
                        effects::color_matrix(&mut target, matrix);
                    }
                    report.filtered_layers += 1;
                }
                stack.push(LayerFrame {
                    target,
                    origin: (x0, y0),
                    alpha: *alpha,
                    blend: *blend,
                    filter: *filter,
                    clip: clip.clone(),
                });
                report.layers += 1;
                report.translated_commands += 1;
            }
            Command::PopLayer => {
                let Some(mut frame) = stack.pop() else {
                    return Err(RendererError::Render(
                        "PopLayer with no matching PushLayer".into(),
                    ));
                };
                // A backdrop layer's filter already ran, in `PushLayer`
                // above, on the sampled backdrop alone — applying it again
                // here would additionally blur this layer's own content
                // (the tint, the children), which is exactly the ordering
                // bug a real backdrop filter must not have.
                if !frame.filter.is_noop() && !frame.filter.backdrop {
                    if frame.filter.blur_sigma > 0.0 {
                        effects::blur(&mut frame.target, frame.filter.blur_sigma);
                    }
                    if let Some(matrix) = &frame.filter.color_matrix {
                        effects::color_matrix(&mut frame.target, matrix);
                    }
                    report.filtered_layers += 1;
                }
                // **A shaped clip on the layer multiplies into the resolved
                // buffer, here, once.**
                //
                // Why at pop and not at push: a clip's *shape* cannot shrink
                // an offscreen buffer (that is a rectangle's job), and it
                // cannot be a per-draw clip inside the layer either — the
                // children already carry their own, tighter or equal, clip
                // states, so multiplying it in per child would be wasted
                // work. One coverage multiply over the finished layer, just
                // before it composites, is the whole cost — and it composites
                // through the ordinary path, so blends and group alpha still
                // apply to the clipped pixels exactly as they did before.
                if !frame.clip.is_none() && !frame.clip.shapes().is_empty() {
                    let region = Rect::new(
                        frame.origin.0 as f32,
                        frame.origin.1 as f32,
                        (frame.origin.0 + frame.target.width as i32) as f32,
                        (frame.origin.1 + frame.target.height as i32) as f32,
                    );
                    let resolved = resolve_clip_cached(
                        &mut self.clips,
                        &frame.clip,
                        Transform::IDENTITY,
                        surface,
                        region,
                    );
                    if resolved.has_mask() {
                        let width = frame.target.width as usize;
                        for y in 0..frame.target.height as i32 {
                            for x in 0..frame.target.width as i32 {
                                let coverage =
                                    resolved.coverage_at(frame.origin.0 + x, frame.origin.1 + y);
                                if coverage < 1.0 {
                                    let index = y as usize * width + x as usize;
                                    frame.target.pixels[index] =
                                        frame.target.pixels[index].scaled(coverage);
                                }
                            }
                        }
                    }
                }
                let (parent, parent_origin) = current!();
                parent.composite_layer(
                    &frame.target,
                    frame.origin.0 - parent_origin.0,
                    frame.origin.1 - parent_origin.1,
                    frame.blend,
                    frame.alpha,
                    self.pipeline,
                );
                self.targets.release(frame.target);
                report.translated_commands += 1;
            }
        }
        Ok(())
    }

    /// Draw `image` into `rect` through `transform`, confined to `clip` and
    /// `surface` — the shared body of `Command::DrawImage` and the bitmap
    /// colour-glyph path, extracted so the two cannot drift apart.
    ///
    /// # Minification goes to the mip pyramid
    ///
    /// Before the loop, the source-texels-per-destination-pixel rate is
    /// measured ([`minification_ratio`]); at `≥ 2` the sampler reads from the
    /// image's cached pyramid ([`MipCache`]) instead of the base image, which
    /// turns a moiré-producing undersample into a correct average. Below 2
    /// nothing changes: the plain bilinear path is byte-for-byte what it
    /// always was, which is what keeps every committed golden image valid.
    #[allow(clippy::too_many_arguments)]
    fn paint_image(
        &mut self,
        target: &mut Target,
        origin: (i32, i32),
        image: &vieww_foundation::Image,
        rect: Rect,
        transform: Transform,
        clip: &Clip,
        surface: Rect,
    ) {
        // Bounds first, then the mask over just the pixels this image
        // will be sampled into.
        let bounds = clip_bounds(clip, surface);
        let dev_bounds = transform
            .apply_rect(rect)
            .intersect(bounds)
            .intersect(surface);
        let resolved = resolve_clip_cached(&mut self.clips, clip, transform, surface, dev_bounds);
        let x0 = dev_bounds.left.floor().max(origin.0 as f32) as i32;
        let x1 = dev_bounds.right.ceil() as i32;
        let y0 = dev_bounds.top.floor().max(origin.1 as f32) as i32;
        let y1 = dev_bounds.bottom.ceil() as i32;
        // **Inverted once, not once per pixel.** The mapping from
        // device space back into the image is the same for every pixel
        // of this command; inverting it inside the loop made drawing
        // one 860x538 screenshot cost 47 ms — 460 000 matrix
        // inversions — and was the whole reason a page with a picture
        // on it could not scroll. A transform that cannot be inverted
        // is degenerate and covers no pixels, so there is nothing to
        // draw and the command is skipped rather than looped over.
        let Some(inverse) = invert_transform(transform) else {
            return;
        };
        let ratio = minification_ratio(image, rect, &inverse);
        let chain = if ratio >= 2.0 {
            Some(self.image_mips.chain(image))
        } else {
            None
        };
        for y in y0..y1 {
            for x in x0..x1 {
                let c = resolved.coverage_at(x, y);
                if c <= 0.0 {
                    continue;
                }
                let sample = match &chain {
                    Some(chain) => sample_device_pixel_mipped(
                        chain,
                        image,
                        rect,
                        inverse,
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        ratio,
                    ),
                    None => sample_device_pixel_with(
                        image,
                        rect,
                        inverse,
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                    ),
                };
                if let Some(sample) = sample {
                    let src = sample.scaled(c);
                    let (tx, ty) = (x - origin.0, y - origin.1);
                    let dst = target.get(tx, ty);
                    target.set(
                        tx,
                        ty,
                        blend_with_pipeline(
                            self.pipeline,
                            vieww_foundation::BlendMode::Normal,
                            src,
                            dst,
                        ),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_shape(
        &mut self,
        target: &mut Target,
        origin: (i32, i32),
        polylines: &[super::geometry::flatten::Polyline],
        local_bounds: Rect,
        transform: Transform,
        paint: &crate::Paint,
        clip: &Clip,
        surface: Rect,
    ) {
        if paint.is_invisible() {
            return;
        }
        // Rasterise the shape against the clip's bounding box first: that is
        // what decides which of its pixels can exist. Only then resolve the
        // clip's mask, and only over the region the shape actually inked.
        let mask = rasterize(polylines, clip_bounds(clip, surface));
        if mask.is_empty() {
            return;
        }
        let need = Rect::new(
            mask.x0 as f32,
            mask.y0 as f32,
            (mask.x0 + mask.width as i32) as f32,
            (mask.y0 + mask.height as i32) as f32,
        );
        let resolved = resolve_clip_cached(&mut self.clips, clip, transform, surface, need);
        let inverse = invert(transform);
        let flat = Premul::from_straight(paint.color);

        // **The flat, unclipped fill does not need a closure at all.**
        //
        // It is also the overwhelming majority of what an interface draws:
        // every panel, every divider, every button face, every icon path, the
        // interior of every glyph-free rectangle in the window. Routing it
        // through the general `color_at` callback meant, per pixel, an indirect
        // call, a `resolved.coverage_at` that re-checked four bounds to answer
        // "1.0", and a match on a `gradient: Option` that is `None`. None of
        // those depend on the pixel. `composite_flat` is the same compositing
        // arithmetic with the three of them hoisted out, so the inner loop is a
        // coverage load, a scale and a blend.
        if paint.gradient.is_none() && !resolved.has_mask() {
            target.composite_flat(
                mask.view(),
                origin,
                vieww_foundation::BlendMode::Normal,
                self.pipeline,
                flat,
            );
            return;
        }

        // A gradient's stops, the shape's bounds and the inverse of its
        // transform are all the same for every pixel of the shape. Folding them
        // together once — see `gradient::DeviceRamp` — is what took this branch
        // from the most expensive per-pixel function in the renderer down to a
        // multiply and an add.
        let device_ramp = match (&paint.gradient, inverse) {
            (Some(gradient), Some(inv)) => {
                Some(gradient::DeviceRamp::new(gradient, local_bounds, inv))
            }
            _ => None,
        };

        // An unclipped gradient fill, like an unclipped flat one, does not need
        // a closure per pixel.
        if let (Some(ramp), false) = (&device_ramp, resolved.has_mask()) {
            target.composite_gradient(
                mask.view(),
                origin,
                vieww_foundation::BlendMode::Normal,
                self.pipeline,
                ramp,
            );
            return;
        }
        // What is left is the clipped cases: a shape with a real clip *mask*
        // over it, which has to ask the mask per pixel and so keeps the
        // closure. The gradient still comes from the folded `DeviceRamp`; only
        // the row constants cannot be hoisted out through `composite_coverage`'s
        // callback, and a masked shape is a small minority of a frame's draws.
        target.composite_coverage(
            mask.view(),
            origin,
            vieww_foundation::BlendMode::Normal,
            self.pipeline,
            |x, y| {
                let clip_c = resolved.coverage_at(x, y);
                if clip_c <= 0.0 {
                    return Premul::TRANSPARENT;
                }
                let color = match &device_ramp {
                    Some(ramp) => ramp.at_device(x, y),
                    None => flat,
                };
                color.scaled(clip_c)
            },
        );
    }
}

/// This renderer's only cache is derived glyph outlines — it samples
/// [`vieww_foundation::Image`] data directly rather than holding a converted
/// copy of its own, so there is no image cache here to trim. Outlines are
/// cheap to re-extract and every visible run needs its font again on the very
/// next frame, so they are kept warm up to
/// [`MemoryPressure::trims_everything`] — matching the old vello-based
/// backends' "keep the small hot thing, drop the large idle one" trade, just
/// with nothing large and idle left to drop.
impl Trim for NativeRenderer {
    fn trim(&mut self, pressure: MemoryPressure) {
        if pressure.trims_everything() {
            self.glyphs.clear();
            self.glyph_rasters.clear();
            // Colour-glyph tables and mip pyramids are the same kind of
            // derived, re-buildable data the outline and raster caches are —
            // clear them under the same pressure, for the same reasons.
            self.color_glyphs.clear();
            self.image_mips.clear();
        }
    }
}

fn rect_polygon(rect: Rect, transform: Transform) -> super::geometry::flatten::Polyline {
    rounded_rect_polygon_transformed(rect, 0.0, transform)
}

pub(crate) fn invert(t: Transform) -> Option<Transform> {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let a = t.d * inv_det;
    let b = -t.b * inv_det;
    let c = -t.c * inv_det;
    let d = t.a * inv_det;
    let tx = -(a * t.tx + c * t.ty);
    let ty = -(b * t.tx + d * t.ty);
    Some(Transform { a, b, c, d, tx, ty })
}

// Test-only record of the last offscreen buffer size a `PushLayer` asked
// `TargetPool::acquire` for — the only way to observe, from outside this
// module, whether a filtered layer's margin was applied once or twice. A
// pixel diff cannot see it: a box blur's kernel has finite support, so
// pixels within the correct margin come out identical whether the
// offscreen buffer is padded further beyond that margin or not — see
// `blur_bounds_are_expanded_exactly_once` below, which found that out the
// hard way before landing on this instead.
#[cfg(test)]
thread_local! {
    static LAST_LAYER_TARGET_SIZE: std::cell::Cell<Option<(u32, u32)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Canvas;
    use vieww_foundation::{BlendMode, ImageFilter, Path};

    /// A clip costs a bounded amount of mask, not a mask per command.
    ///
    /// # The bug this exists to keep dead
    ///
    /// Every command carries a clone of the `Canvas` clip state in force when
    /// it was recorded, and resolving that clip means rasterising its shapes
    /// into a coverage mask. Resolving it per command therefore cost
    /// `commands × clip area` — and a clip's area is the *panel's*, not the
    /// glyph's, so a rounded panel charged every piece of text inside it a
    /// full-panel rounded-rectangle rasterisation.
    ///
    /// Measured on a 1366x679 surface before the cache: 1600 unclipped
    /// commands rendered in 24 ms, and the same 1600 under one window-sized
    /// rounded clip took **15.7 seconds**. Not a slow frame — a hung window,
    /// and every queued repaint behind it another one. Damage-region
    /// rendering would have divided that number rather than fixed it.
    ///
    /// The assertion is on how many masks were *built* rather than on elapsed
    /// time, because a timing threshold on a shared runner is a flaky test and
    /// the thing that actually went wrong is how many times the mask was
    /// derived. It is not "one": masks are built per tile-aligned region (see
    /// `clip.rs`'s `TILE`), so a clip spanning many tiles legitimately builds
    /// several small ones. What must never come back is the number scaling
    /// with the *command count*.
    #[test]
    fn a_clip_costs_a_bounded_number_of_masks_not_one_per_command() {
        const COMMANDS: usize = 400;

        let mut scene = Scene::new();
        scene.save();
        scene.clip_rrect(Rect::new(0.0, 0.0, 600.0, 400.0), 12.0);
        for i in 0..COMMANDS {
            let x = (i % 20) as f32 * 30.0;
            let y = (i / 20) as f32 * 20.0;
            scene.fill_rect(
                Rect::new(x, y, x + 28.0, y + 18.0),
                Color::rgb(20, 120, 220).into(),
            );
        }
        scene.restore();

        let mut renderer = NativeRenderer::new();
        renderer
            .render_to_pixels(&scene, 600, 400, Color::WHITE)
            .expect("render");

        let (hits, misses) = renderer.clips.stats();
        assert!(
            misses * 4 < COMMANDS,
            "a clip should cost a bounded number of tile masks, not one per \
             command: {misses} masks for {COMMANDS} commands"
        );
        assert!(
            hits > misses,
            "most commands should reuse a mask a neighbour already built: \
             {hits} hits against {misses} misses"
        );
    }

    /// Doubling the commands inside one clip must not double the masks.
    ///
    /// The sharper half of the invariant above: a bound that happens to hold
    /// at one command count could still be linear in it. Two scenes over the
    /// same clip, one twice as dense as the other, must build the same masks —
    /// the mask count follows the *area* the clip covers, never the number of
    /// things drawn under it.
    #[test]
    fn twice_the_commands_under_one_clip_build_the_same_masks() {
        fn masks_for(step: f32) -> usize {
            let mut scene = Scene::new();
            scene.save();
            scene.clip_rrect(Rect::new(0.0, 0.0, 600.0, 400.0), 12.0);
            let mut y = 0.0;
            while y < 400.0 {
                let mut x = 0.0;
                while x < 600.0 {
                    scene.fill_rect(
                        Rect::new(x, y, x + step - 2.0, y + step - 2.0),
                        Color::rgb(20, 120, 220).into(),
                    );
                    x += step;
                }
                y += step;
            }
            let mut renderer = NativeRenderer::new();
            renderer
                .render_to_pixels(&scene, 600, 400, Color::WHITE)
                .expect("render");
            renderer.clips.stats().1
        }

        let coarse = masks_for(40.0);
        let fine = masks_for(20.0);
        assert_eq!(
            coarse, fine,
            "four times as many commands over the same clip built {fine} masks \
             against {coarse} — the mask count is following the command count"
        );
    }

    /// The cached mask must produce the *same pixels* the uncached one did.
    ///
    /// A cache keyed on the wrong thing is worse than no cache: it is fast
    /// and wrong, and a clip that quietly resolves to a neighbour's mask is
    /// exactly the kind of fault that reads as a layout bug for a week. Two
    /// different rounded clips over identical content, rendered in one scene,
    /// must each cut their own shape.
    #[test]
    fn two_different_clips_in_one_scene_do_not_share_a_mask() {
        let mut scene = Scene::new();
        // Left: a rounded clip. Right: a rounded clip of a different radius,
        // so a cache keyed only on bounds — or on nothing — would show it.
        scene.save();
        scene.clip_rrect(Rect::new(0.0, 0.0, 100.0, 100.0), 40.0);
        scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED.into());
        scene.restore();
        scene.save();
        scene.clip_rrect(Rect::new(100.0, 0.0, 200.0, 100.0), 0.5);
        scene.fill_rect(Rect::new(100.0, 0.0, 200.0, 100.0), Color::RED.into());
        scene.restore();

        let mut renderer = NativeRenderer::new();
        let (pixels, _) = renderer
            .render_to_pixels(&scene, 200, 100, Color::WHITE)
            .expect("render");
        let data = pixels.data();
        let red_at =
            |x: usize, y: usize| data[(y * 200 + x) * 4] > 200 && data[(y * 200 + x) * 4 + 1] < 80;

        // The heavily rounded clip's corner is cut away...
        assert!(!red_at(2, 2), "the r=40 clip should not ink its own corner");
        // ...and the barely-rounded one's corner is not.
        assert!(red_at(102, 2), "the r=0.5 clip should ink its corner");
    }

    /// A filtered layer's offscreen target should be exactly the content
    /// bounds grown by the filter's reach once — not the content bounds
    /// grown by the reach, then grown by the reach again.
    ///
    /// `Scene::pop_layer` already replaces a filtered `PushLayer`'s declared
    /// bounds with `contents.inflate(reach)` (see its own doc comment), so a
    /// renderer that also inflates by `reach` when it *reads* that command
    /// back double-counts the margin. Regression test for exactly that: a
    /// 50x20 fill, blurred with sigma 5.0 (`reach` = `(5.0 * 3.0).ceil()` =
    /// 15.0), on a surface large enough that nothing clamps the layer —
    /// the acquired target must be exactly `50 + 2*15` by `20 + 2*15`.
    #[test]
    fn blur_bounds_are_expanded_exactly_once() {
        LAST_LAYER_TARGET_SIZE.with(|cell| cell.set(None));

        let content = Rect::new(100.0, 100.0, 150.0, 120.0); // 50 wide, 20 tall
        let sigma = 5.0f32;
        let mut scene = Scene::new();
        scene.push_filtered_layer(content, 1.0, BlendMode::Normal, ImageFilter::blur(sigma));
        scene.fill_path(&Path::rect(content), Color::WHITE.into());
        scene.pop_layer();

        let mut renderer = NativeRenderer::new();
        renderer
            .render_to_pixels(&scene, 400, 400, Color::TRANSPARENT)
            .expect("render");

        let reach = ImageFilter::blur(sigma).bounds_expansion();
        assert_eq!(reach, 15.0, "test assumes a round-number reach");
        let expected = (
            (content.width() + 2.0 * reach).round() as u32,
            (content.height() + 2.0 * reach).round() as u32,
        );
        let got = LAST_LAYER_TARGET_SIZE
            .with(std::cell::Cell::get)
            .expect("PushLayer ran");
        assert_eq!(
            got, expected,
            "offscreen target should be grown by the blur's reach exactly once, not twice"
        );
    }

    /// Identity colour matrix — `is_noop()` is still `false` (it checks
    /// `color_matrix.is_none()`, not "is the matrix a no-op"), so a real
    /// `PushLayer { filter, .. }` with `backdrop: true` is recorded and this
    /// renderer runs the whole backdrop path, but the pixels it produces are
    /// mathematically untouched — which turns "did this copy the real
    /// destination pixels" into a plain equality check instead of a fuzzy
    /// one.
    const IDENTITY_MATRIX: vieww_foundation::ColorMatrix = [
        1.0, 0.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 0.0, 1.0, 0.0,
    ];

    /// Proves `ImageFilter::backdrop` samples the *real, position-correct*
    /// destination pixels rather than degrading to a flat wash — the
    /// behaviour `vieww-effects::BackdropBlur` used to have before this
    /// feature existed (see that crate's `widgets.rs` module docs).
    ///
    /// Commands are built by hand via [`Scene::commands_mut`] rather than
    /// through the [`Canvas`] trait: that lets this test place a `PushLayer`
    /// with an exact, unexpanded `bounds` straddling a hard colour edge in
    /// the background, and know precisely what the sampled backdrop should
    /// look like at every pixel, without `Canvas::pop_layer`'s
    /// contents-replacement or empty-layer pruning (documented on
    /// `Scene::pop_layer`) rewriting the bounds this test depends on.
    #[test]
    fn a_backdrop_layer_samples_real_destination_pixels_not_a_flat_stand_in() {
        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);
        let green = Color::rgba(0, 255, 0, 255);

        let mut scene = Scene::new();
        let commands = scene.commands_mut();
        // Background: a hard vertical edge at x = 100 across the whole 200x100
        // surface.
        commands.push(Command::FillRect {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            paint: red.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        commands.push(Command::FillRect {
            rect: Rect::new(100.0, 0.0, 200.0, 100.0),
            paint: blue.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        // A backdrop layer straddling the edge, with no blur — an identity
        // colour matrix so the sampled backdrop should come out byte-for-byte
        // unchanged.
        commands.push(Command::PushLayer {
            bounds: Rect::new(80.0, 20.0, 120.0, 80.0),
            alpha: 1.0,
            blend: BlendMode::Normal,
            clip: Clip::NONE,
            filter: ImageFilter {
                blur_sigma: 0.0,
                color_matrix: Some(IDENTITY_MATRIX),
                backdrop: true,
            },
        });
        // Sharp content painted *after* the backdrop is sampled — this must
        // land on top untouched, not get folded into the (no-op, here, but
        // this is what the ordering guarantees in general) filter.
        commands.push(Command::FillRect {
            rect: Rect::new(110.0, 70.0, 120.0, 80.0),
            paint: green.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        commands.push(Command::PopLayer);

        let mut renderer = NativeRenderer::new();
        let (pixels, report) = renderer
            .render_to_pixels(&scene, 200, 100, Color::TRANSPARENT)
            .expect("render");

        assert_eq!(
            report.filtered_layers, 1,
            "the backdrop path should have run exactly once"
        );

        // Inside the layer, on the red side, away from the child: must be the
        // *real* background colour sampled from underneath, not transparent,
        // not black, not some placeholder tint.
        assert_eq!(pixels.pixel(85, 30), red, "backdrop sample on the red side");
        // Inside the layer, on the blue side: proves the sample is
        // position-correct across the edge, not e.g. reading everything from
        // one corner.
        assert_eq!(
            pixels.pixel(115, 30),
            blue,
            "backdrop sample on the blue side"
        );
        // The child painted after the backdrop was captured: sharp, on top.
        assert_eq!(
            pixels.pixel(115, 75),
            green,
            "foreground content stays on top of the sampled backdrop"
        );
        // Outside the layer entirely: the background must be exactly as
        // painted, untouched by the filter.
        assert_eq!(
            pixels.pixel(10, 10),
            red,
            "background outside the layer is untouched (red side)"
        );
        assert_eq!(
            pixels.pixel(190, 10),
            blue,
            "background outside the layer is untouched (blue side)"
        );
    }

    /// Proves the backdrop path runs a *real* blur over the sampled
    /// destination — not a flat tint standing in for one — and that the
    /// blur is confined to the backdrop, never reaching the foreground
    /// content this layer paints on top of it.
    #[test]
    fn a_backdrop_blur_genuinely_blurs_the_background_but_not_the_foreground() {
        let red = Color::rgba(255, 0, 0, 255);
        let blue = Color::rgba(0, 0, 255, 255);
        let green = Color::rgba(0, 255, 0, 255);
        let sigma = 5.0f32;
        let reach = ImageFilter::backdrop_blur(sigma).bounds_expansion();
        assert_eq!(reach, 15.0, "test assumes a round-number reach");

        let mut scene = Scene::new();
        let commands = scene.commands_mut();
        commands.push(Command::FillRect {
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            paint: red.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        commands.push(Command::FillRect {
            rect: Rect::new(100.0, 0.0, 200.0, 100.0),
            paint: blue.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        // The whole surface, so every sample point below has plenty of margin
        // (well over `reach`) from both the colour edge and the target's own
        // border.
        commands.push(Command::PushLayer {
            bounds: Rect::new(0.0, 0.0, 200.0, 100.0),
            alpha: 1.0,
            blend: BlendMode::Normal,
            clip: Clip::NONE,
            filter: ImageFilter::backdrop_blur(sigma),
        });
        commands.push(Command::FillRect {
            rect: Rect::new(180.0, 80.0, 195.0, 95.0),
            paint: green.into(),
            transform: Transform::IDENTITY,
            clip: Clip::NONE,
        });
        commands.push(Command::PopLayer);

        let mut renderer = NativeRenderer::new();
        let (pixels, report) = renderer
            .render_to_pixels(&scene, 200, 100, Color::TRANSPARENT)
            .expect("render");

        assert_eq!(report.filtered_layers, 1);

        // Deep inside a uniform region, far outside the blur's reach from
        // both the colour edge and the layer's own border: a box blur has
        // finite support, so this must come back exactly as painted.
        assert_eq!(
            pixels.pixel(30, 50),
            red,
            "well outside blur reach, still exactly the background red"
        );
        assert_eq!(
            pixels.pixel(170, 50),
            blue,
            "well outside blur reach, still exactly the background blue"
        );

        // Right on the colour edge: real sampled data, genuinely blurred,
        // must be a mix of both neighbours — neither pure colour, and
        // nothing close to the old flat-tint degradation (which would not
        // vary with position at all).
        let at_edge = pixels.pixel(100, 50);
        assert_ne!(at_edge, red, "the edge sample must not be untouched red");
        assert_ne!(at_edge, blue, "the edge sample must not be untouched blue");
        assert!(
            at_edge.r > 0,
            "a real blur of red-then-blue leaves red in the mix at the edge"
        );
        assert!(
            at_edge.b > 0,
            "a real blur of red-then-blue leaves blue in the mix at the edge"
        );

        // The child painted after the backdrop blur ran: sharp, not blurred
        // into the background it sits over.
        assert_eq!(
            pixels.pixel(187, 87),
            green,
            "foreground content is not blurred along with the backdrop"
        );
    }
}
