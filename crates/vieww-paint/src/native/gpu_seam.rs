//! The coverage, shadow, gradient and image facts a GPU backend needs, taken
//! from **this** rasterizer rather than re-derived.
//!
//! # Why this exists
//!
//! [`super::GlyphCoverage`] established the rule for text: the GPU path uploads
//! the CPU rasterizer's own coverage, so the parity suite compares two
//! *executors* of one definition rather than two definitions. The same rule
//! applies to every other primitive whose meaning lives in this crate:
//!
//! - **Shaped clips** — [`GpuSeam::clip_mask`] is the exact antialiased
//!   coverage `NativeRenderer` multiplies into a clipped draw.
//! - **Shadows** — [`GpuSeam::shadow_masks`] is the unblurred caster (and, for
//!   an inset shadow, the confinement box) plus the box radius. The *blur* is
//!   not done here: the GPU runs the same three separable box passes.
//! - **Blur radii** — [`box_radius_for_sigma`] is the one kernel formula.
//! - **Gradients** — [`gradient_ramp`] samples the premultiplied stop ramp and
//!   [`BAYER4`] is the ordered-dither matrix, so a fragment shader evaluates
//!   the same geometry and the same colours.
//! - **Colour glyphs** — [`GpuSeam::color_glyph`] resolves COLRv0 layers and
//!   CBDT/sbix bitmaps, which the monochrome coverage seam cannot express.
//! - **Image mips** — [`GpuSeam::image_mips`] and [`minification_ratio`] are
//!   the pyramid and the level selection `DrawImage` uses.
//!
//! Everything here is cached where the renderer caches the same thing, so a
//! steady frame pays nothing.

use std::sync::Arc;

use vieww_foundation::{FontData, Gradient, Image, Offset, Rect, Shadow, Transform};

use super::color_glyphs::{ColorGlyph, ColorGlyphCache};
use super::image::MipCache;
use crate::Clip;

/// One rasterised coverage patch in device pixels, as bytes.
///
/// `alpha` is row-major, `width * height`, rounded to the nearest byte — the
/// same quantisation [`super::InkedGlyph::alpha`] documents for glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePatch {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

/// A shadow's masks, before blur. See the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowPatch {
    /// The patch's device-space top-left and size. `caster` and `inner` both
    /// cover exactly this rectangle.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Unblurred coverage of the offset + spread caster.
    pub caster: Vec<u8>,
    /// For an inset shadow: coverage of the box the shadow is confined to.
    pub inner: Option<Vec<u8>>,
    /// Three-box blur radius; `None` for a blur too small to run.
    pub box_radius: Option<u32>,
}

/// A colour glyph, as a backend draws it.
#[derive(Debug, Clone)]
pub enum ColorGlyphParts {
    /// COLRv0: `(glyph id, straight colour)` pairs, bottom layer first. Each
    /// layer is an ordinary coverage glyph of the same font.
    Layers(Vec<(u16, vieww_foundation::Color)>),
    /// CBDT/sbix: a bitmap drawn like `DrawImage` at `rect` (run-local, before
    /// the command's transform).
    Bitmap { image: Image, rect: Rect },
}

/// The ordered-dither matrix gradients use, in 1/255 units of offset.
pub const BAYER4: [[f32; 4]; 4] = [
    [-0.46875, 0.03125, -0.34375, 0.15625],
    [0.28125, -0.21875, 0.40625, -0.09375],
    [-0.28125, 0.21875, -0.40625, 0.09375],
    [0.46875, -0.03125, 0.34375, -0.15625],
];

/// Caches for the facts above. Hold one per GPU planner.
#[derive(Default)]
pub struct GpuSeam {
    color_glyphs: ColorGlyphCache,
    mips: MipCache,
}

impl std::fmt::Debug for GpuSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuSeam").finish_non_exhaustive()
    }
}

fn to_byte(c: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=255 before the cast"
    )]
    let byte = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    byte
}

impl GpuSeam {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The antialiased coverage of `clip`'s shapes over `region` (device
    /// pixels, already integral), or `None` when the clip has no shapes —
    /// a rectangle-only clip is a scissor, not a mask.
    ///
    /// Pixels of `region` the shapes do not reach are present and zero.
    #[must_use]
    pub fn clip_mask(&self, clip: &Clip, region: Rect) -> Option<CoveragePatch> {
        if clip.shapes().is_empty() {
            return None;
        }
        #[expect(clippy::cast_possible_truncation, reason = "integral device pixels")]
        let (x0, y0, x1, y1) = (
            region.left.floor() as i32,
            region.top.floor() as i32,
            region.right.ceil() as i32,
            region.bottom.ceil() as i32,
        );
        let width = u32::try_from((x1 - x0).max(0)).unwrap_or(0);
        let height = u32::try_from((y1 - y0).max(0)).unwrap_or(0);
        let mut alpha = vec![0u8; width as usize * height as usize];
        if width > 0 && height > 0 {
            #[expect(clippy::cast_precision_loss, reason = "device pixels")]
            let bounds = Rect::new(x0 as f32, y0 as f32, x1 as f32, y1 as f32);
            let mask = super::clip::rasterize_clip_shapes(clip, Transform::IDENTITY, bounds);
            for y in 0..height {
                for x in 0..width {
                    let c = mask.coverage_at(x0 + x as i32, y0 + y as i32);
                    alpha[(y * width + x) as usize] = to_byte(c);
                }
            }
        }
        Some(CoveragePatch {
            x: x0,
            y: y0,
            width,
            height,
            alpha,
        })
    }

    /// The unblurred masks of a `DrawShadow`, confined to `surface` exactly as
    /// `NativeRenderer` confines them (the enclosing layer's buffer).
    #[must_use]
    pub fn shadow_masks(
        &self,
        rect: Rect,
        radius: f32,
        shadow: &Shadow,
        transform: Transform,
        surface: Rect,
    ) -> Option<ShadowPatch> {
        let masks = super::shadow::shadow_masks(rect, radius, shadow, transform, surface)?;
        Some(ShadowPatch {
            x: masks.x0,
            y: masks.y0,
            width: masks.width,
            height: masks.height,
            caster: masks.caster.iter().map(|c| to_byte(*c)).collect(),
            inner: masks
                .inner
                .map(|inner| inner.iter().map(|c| to_byte(*c)).collect()),
            box_radius: masks.box_radius.map(|r| u32::try_from(r).unwrap_or(1)),
        })
    }

    /// The colour answer for a glyph, if its font has one — see
    /// `NativeRenderer`'s `DrawGlyphs` handling, which asks the same question
    /// first for the same reason.
    pub fn color_glyph(
        &mut self,
        font: &FontData,
        glyph_id: u16,
        run_origin: Offset,
        glyph_offset: Offset,
        size: f32,
        transform: Transform,
    ) -> Option<ColorGlyphParts> {
        match self.color_glyphs.resolve(
            font,
            glyph_id,
            run_origin,
            glyph_offset,
            size,
            transform,
        )? {
            ColorGlyph::Layers(layers) => Some(ColorGlyphParts::Layers(layers)),
            ColorGlyph::Bitmap { image, rect } => Some(ColorGlyphParts::Bitmap { image, rect }),
        }
    }

    /// The half-resolution pyramid `DrawImage` samples when minifying.
    /// Cached by image identity; the returned images are stable between
    /// calls, so a backend can key a texture on them.
    pub fn image_mips(&mut self, image: &Image) -> Arc<Vec<Image>> {
        self.mips.chain(image)
    }

    /// Drop every cache — the GPU twin of `NativeRenderer`'s trim.
    pub fn clear(&mut self) {
        self.color_glyphs.clear();
        self.mips.clear();
    }
}

/// `path` split into its dash-on pieces exactly as the CPU stroker splits it,
/// in the path's own (local) space — or `None` for a solid stroke.
///
/// Curves are flattened first, with the CPU renderer's flattener, because that
/// is where its dash arc lengths are measured. Each dash-on piece is an open
/// subpath (so a stroker caps both ends, as the CPU renderer does); an undashed
/// closed contour stays closed.
#[must_use]
pub fn dashed_path(
    path: &vieww_foundation::Path,
    dash: &Option<vieww_foundation::Dash>,
) -> Option<vieww_foundation::Path> {
    let dash = dash.as_ref()?;
    if dash.is_solid() || dash.pattern.is_empty() || dash.pattern.iter().sum::<f32>() <= 0.0 {
        return None;
    }
    let mut out = vieww_foundation::Path::new();
    for line in super::geometry::flatten::flatten_path(path, Transform::IDENTITY) {
        for (points, closed) in
            super::geometry::stroke::dashed_segments(&line.points, line.closed, &Some(dash.clone()))
        {
            let mut iter = points.iter();
            let Some(&(x, y)) = iter.next() else { continue };
            out.move_to(Offset::new(x, y));
            for &(x, y) in iter {
                out.line_to(Offset::new(x, y));
            }
            if closed {
                out.close();
            }
        }
    }
    Some(out)
}

/// The CPU shadow blur's own formula (`native::shadow::box_radius_for_sigma`): the per-pass radius of the
/// three-box Gaussian approximation.
#[must_use]
pub fn box_radius_for_sigma(sigma: f32) -> u32 {
    u32::try_from(super::shadow::box_radius_for_sigma(sigma)).unwrap_or(1)
}

/// The texel rate `DrawImage` uses to decide whether to sample the pyramid.
#[must_use]
pub fn minification_ratio(image: &Image, rect: Rect, transform: Transform) -> Option<f32> {
    let inverse = super::reference::invert(transform)?;
    Some(super::image::minification_ratio(image, rect, &inverse))
}

/// `samples` evenly spaced premultiplied colours of `gradient`'s stop ramp,
/// `t = i / (samples - 1)`, each `[r, g, b, a]` in `0..=1`.
///
/// Linear interpolation between adjacent samples reproduces the ramp exactly
/// except inside a sample interval that contains a stop.
#[must_use]
pub fn gradient_ramp(gradient: &Gradient, samples: usize) -> Vec<[f32; 4]> {
    let ramp = super::gradient::Ramp::new(gradient);
    let last = samples.saturating_sub(1).max(1);
    (0..samples)
        .map(|i| {
            #[expect(clippy::cast_precision_loss, reason = "small sample counts")]
            let t = i as f32 / last as f32;
            ramp.at_t(t).channels()
        })
        .collect()
}
