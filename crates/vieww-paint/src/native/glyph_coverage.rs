//! Rasterised glyph coverage, exposed so a GPU backend can build an atlas
//! from *the same* rasterizer the CPU renderer draws with.
//!
//! # Why this is public, and why it is not a second implementation
//!
//! A GPU text pipeline needs one thing the rest of `vieww-gpu` can produce for
//! itself: an alpha bitmap per glyph, and where to put it. The obvious way to
//! get one is to write a glyph rasterizer in `vieww-gpu`. That would be the
//! wrong move here, for a reason this crate has already written down twice.
//!
//! `native/reference.rs` calls this renderer an **oracle**: the GPU path's
//! tests assert that a frame drawn on a real device matches
//! [`NativeRenderer`](super::NativeRenderer)'s output pixel for pixel. Two
//! independent glyph rasterizers cannot be compared that way — every
//! disagreement between them is a fight about which one is right, and the
//! parity suite stops being able to catch anything, because a real regression
//! and a difference of convention arrive looking identical. `vieww-hal`'s own
//! `interior_mismatches` history is the cautionary version of this: a parity
//! harness that cannot distinguish "wrong" from "differently rounded" reports
//! neither.
//!
//! So there is one rasterizer, and this module is the seam through which the
//! GPU reaches it. What comes out is exactly what the crate's internal
//! `glyph_raster::GlyphRasterCache` already had:
//! coverage rasterised at the glyph's fractional origin, with the whole-pixel
//! part of its position handed back separately so one bitmap serves every
//! occurrence of that glyph at that sub-pixel phase — which is precisely what
//! makes an atlas worth having.
//!
//! # The quantisation, stated rather than hidden
//!
//! Internally coverage is `f32` per pixel. [`InkedGlyph::alpha`] hands out
//! `u8`, because that is what an `R8_UNORM` atlas texture holds and because a
//! four-byte-per-texel atlas costs four times the upload and the sampler
//! bandwidth for precision no display can show.
//!
//! Rounding to `u8` moves the final composited pixel by **at most 1/255**, and
//! only where coverage is fractional — the antialiased rim of a glyph. That is
//! the same magnitude, in the same place, as the difference
//! `native/glyph_raster.rs` documents for its own phase-relative rasterisation
//! (57 pixels across the whole fixture gallery, every one by exactly 1/255),
//! and it is written here for the same reason: an oracle whose tolerance is
//! undocumented is an oracle nobody can use.
//!
//! `vieww-hal`'s text parity tests assert the interior of every glyph exactly
//! and allow one step of 1/255 on the rim, which is a bound this rounding can
//! reach and a mirrored, mispositioned or mis-scaled glyph cannot.

use vieww_foundation::{FontData, Offset, Transform};

use super::glyph::GlyphCache;
use super::glyph_raster::{Glyph, GlyphRasterCache};

/// A glyph rasterizer with the caches a repeated frame wants, exposed for
/// backends that build their own texture from the result.
///
/// Holds both caches the CPU renderer holds — outlines in font units, and
/// coverage in device pixels — for the same reason it does: neither depends on
/// anything that changes between frames, and a text-heavy window re-rasterising
/// its glyphs every frame is the single largest cost in it.
#[derive(Default)]
pub struct GlyphCoverage {
    outlines: GlyphCache,
    rasters: GlyphRasterCache,
}

/// Hand-written because `GlyphCache` has no `Debug` of its own — printing a
/// few thousand cached outlines is not a debugging aid — and the useful thing
/// to see here is the coverage cache's hit rate.
impl std::fmt::Debug for GlyphCoverage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphCoverage")
            .field("rasters", &self.rasters)
            .finish_non_exhaustive()
    }
}

/// What one glyph rasterised to.
///
/// Three cases rather than two, matching the crate-internal
/// `glyph_raster::Glyph` exactly: a caller that
/// collapses "the font has no outline for this id" into "this glyph drew
/// nothing" loses the distinction the renderer's own `SceneReport` counts, and
/// counting is how "a glyph quietly did not get drawn" is caught at all.
#[derive(Debug)]
pub enum GlyphAlpha<'a> {
    /// The font has no outline for this glyph id — a space, or a bitmap-only
    /// glyph. Nothing was drawn and nothing should be counted as drawn.
    NoOutline,
    /// An outline that covers no pixels at this size and phase. Drawn, in the
    /// sense that matters to a report; zero texels, in the sense that matters
    /// to an atlas.
    Blank,
    /// Coverage, and where it goes.
    Inked(InkedGlyph<'a>),
}

/// One glyph's alpha bitmap and its device position.
///
/// The bitmap is row-major, `width * height` values, one per pixel. Its
/// top-left texel belongs at device pixel ([`x`](Self::x), [`y`](Self::y)) —
/// already including the glyph's own ink offset, so a caller places the quad
/// there and does no further arithmetic. That is deliberately *not* how the
/// underlying cache stores it (it keeps the ink offset and the whole-pixel
/// translation apart, because only the second one varies between occurrences),
/// and resolving the two here is the difference between an atlas API and a
/// cache internal.
#[derive(Debug, Clone, Copy)]
pub struct InkedGlyph<'a> {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    coverage: &'a [f32],
}

impl InkedGlyph<'_> {
    /// Device x of the bitmap's left column.
    #[must_use]
    pub const fn x(&self) -> i32 {
        self.x
    }

    /// Device y of the bitmap's top row.
    #[must_use]
    pub const fn y(&self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Coverage in `0.0..=1.0`, row-major — the values the CPU compositor
    /// itself multiplies by.
    #[must_use]
    pub const fn coverage(&self) -> &[f32] {
        self.coverage
    }

    /// The same coverage as `u8`, for an `R8_UNORM` atlas.
    ///
    /// Rounds rather than truncates: truncation biases every fractional
    /// coverage downward, which thins every glyph on the screen by a fraction
    /// of a pixel and shows up as text that is systematically lighter on the
    /// GPU than on the CPU — a difference a parity test would report as a
    /// hundred failing pixels without saying why. See the module doc for the
    /// bound this leaves.
    #[must_use]
    pub fn alpha(&self) -> Vec<u8> {
        self.coverage
            .iter()
            .map(|c| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped to 0..=255 on the line above the cast"
                )]
                let byte = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                byte
            })
            .collect()
    }
}

impl GlyphCoverage {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cache hits and misses over this instance's life.
    ///
    /// An atlas builder's own hit rate is not this number — it holds texels,
    /// not coverage — but a frame that misses here also misses there, so this
    /// is what says whether a window's working set of glyphs fits.
    #[must_use]
    pub fn stats(&self) -> (u64, u64) {
        self.rasters.stats()
    }

    /// Forget everything. For a font change, or a test that wants a cold cache.
    pub fn clear(&mut self) {
        self.outlines.clear();
        self.rasters.clear();
    }

    /// Rasterise one glyph of one run.
    ///
    /// `run_origin` is the run's baseline start and `glyph_offset` the glyph's
    /// own offset within it — the two are kept apart rather than pre-summed
    /// because that is how [`GlyphRun`](vieww_foundation::GlyphRun) carries
    /// them and how the coverage cache keys on them.
    pub fn glyph(
        &mut self,
        font: &FontData,
        glyph_id: u16,
        size: f32,
        run_origin: Offset,
        glyph_offset: Offset,
        transform: Transform,
    ) -> GlyphAlpha<'_> {
        let Self { outlines, rasters } = self;
        // The atlas seam is a grayscale seam: one alpha per pixel is what a
        // glyph atlas is, so the `lcd` flag is false and an LCD rasterisation
        // is never requested from here. (The CPU renderer's LCD path keeps its
        // own cache entries; see `native/glyph_raster.rs`.)
        let answer = rasters.coverage(
            font,
            glyph_id,
            size,
            run_origin,
            glyph_offset,
            transform,
            false,
            || {
                let (path, upm) = outlines.outline(font, glyph_id);
                path.map(|p| (p, upm))
            },
        );
        match answer {
            Glyph::NoOutline => GlyphAlpha::NoOutline,
            Glyph::Blank => GlyphAlpha::Blank,
            Glyph::Inked { raster, x, y } => {
                let Some(mask) = raster.as_gray() else {
                    return GlyphAlpha::NoOutline;
                };
                GlyphAlpha::Inked(InkedGlyph {
                    // The mask's own ink offset plus the whole-pixel translation:
                    // see this type's doc for why they are summed here and not in
                    // the cache.
                    x: x + mask.x0,
                    y: y + mask.y0,
                    width: mask.width,
                    height: mask.height,
                    coverage: &mask.data,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rounding, at the two ends and the middle, because an off-by-one here is
    /// a systematically lighter or heavier screen rather than a visible bug.
    #[test]
    fn coverage_rounds_to_the_nearest_byte_rather_than_truncating() {
        let coverage = [0.0, 0.5, 1.0, 0.999, 0.001, 1.0 / 255.0];
        let glyph = InkedGlyph {
            x: 0,
            y: 0,
            width: 6,
            height: 1,
            coverage: &coverage,
        };
        assert_eq!(glyph.alpha(), vec![0, 128, 255, 255, 0, 1]);
    }

    /// The bound the module doc claims, asserted rather than described: no
    /// coverage value round-trips through `u8` more than half a step away.
    #[test]
    fn the_byte_bound_is_half_a_step_everywhere() {
        for i in 0..=10_000u32 {
            let c = f32::from(u16::try_from(i).unwrap()) / 10_000.0;
            let glyph = InkedGlyph {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                coverage: std::slice::from_ref(&c),
            };
            let back = f32::from(glyph.alpha()[0]) / 255.0;
            assert!(
                (back - c).abs() <= 0.5 / 255.0 + 1e-6,
                "coverage {c} became {back}"
            );
        }
    }
}
