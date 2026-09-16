//! `DrawGlyphs`: font-unit outlines, extracted once and rasterized per run.
//!
//! Spec §6.1: "Glyph outlines come from skrifa". This build extracts them
//! with `ttf-parser` instead — see the crate-level `Cargo.toml` comment for
//! why — but keeps the rest of the pipeline the spec describes: outlines are
//! cached per `(font id, glyph id)` in **font units** (spec's cache-key
//! space, "quarter-pixel x-phase" aside — see §6.2, not yet implemented
//! here), scaled to the run's `size` at draw time, and filled through the
//! exact same [`super::geometry::fill::rasterize`] every other shape uses —
//! which is what makes a glyph and a vector icon share one AA policy instead
//! of two.
//!
//! Hinting stays off (spec §6.1: "matching the current convention").
//!
//! # Variable fonts: the shaper already knows, and now this side does too
//!
//! Shaping has applied variation coordinates since `cosmic-text` 0.19: it
//! builds a HarfBuzz face per `wght` location, so advances, kerning and line
//! metrics were already correct for a variable font — while the outlines this
//! module extracted were still the *default instance's*, because the raw face
//! bytes were handed over with no coordinates attached. A variable font shaped
//! at 700 and drawn at 400 is a bug nobody can see in the metrics.
//!
//! [`vieww_foundation::FontData`] now carries [`FontVariation`]s, the store
//! attaches the shaper's weight to them, and [`extract_outline`] sets each
//! axis on the parsed face before walking contours — `ttf-parser` interpolates
//! `gvar` deltas and applies `avar` there, so the outline comes out at the
//! same instance the shaper measured. The cache stays keyed by `font.id()`,
//! and a `FontData` minted per variation set mints a fresh id (see
//! `FontData::with_variations`), so two weights of one variable font never
//! share an outline entry either.
//!
//! # Residency: identity versus backing storage
//!
//! Per-font data (`face_data`, `units_per_em`) stays a plain, wholesale-
//! cleared `HashMap` — clearing a font a page no longer uses is already
//! [`Trim`](vieww_foundation::Trim)'s job at the "everything" granularity,
//! and re-parsing a font from bytes it would have to keep around anyway
//! buys nothing. Per-*glyph* outlines are the part worth being selective
//! about: a page of text can reference hundreds of distinct glyphs, most of
//! them drawn once and never again, so each font's outlines live in a
//! [`ResidencyCache`] — logically keyed by glyph id, budget-bound, LRU-
//! evictable — rather than an unbounded map. See `native/residency.rs`'s
//! module docs for why that split is safe here: an outline is
//! deterministic, read-only data re-derived from `face_data`, so an
//! eviction is invisible to what gets drawn, only to how fast.

use std::collections::HashMap;

use vieww_foundation::{FontData, FontVariation, Offset, Path};

use super::geometry::flatten::Polyline;
use super::residency::{ResidencyCache, ResidencyStats};

/// How many bytes' worth of budget one font's glyph-outline cache is allowed
/// before it starts evicting its least-recently-used entries. Sized for "a
/// long document's worth of distinct glyphs" (a few thousand paths at a few
/// hundred bytes apiece), not "every glyph in every font a process ever
/// touches" — the latter is exactly what unbounded growth used to allow.
const GLYPH_OUTLINE_BUDGET_BYTES: usize = 1_000_000;

/// Outlines for one font, in font units, keyed by glyph id.
pub(crate) struct GlyphCache {
    fonts: HashMap<u64, FontOutlines>,
    /// Per-font glyph-outline residency budget, in bytes — see
    /// [`GLYPH_OUTLINE_BUDGET_BYTES`] for the default, and
    /// [`Self::with_glyph_outline_budget_bytes`] for overriding it.
    glyph_outline_budget_bytes: usize,
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self {
            fonts: HashMap::new(),
            glyph_outline_budget_bytes: GLYPH_OUTLINE_BUDGET_BYTES,
        }
    }
}

struct FontOutlines {
    face_data: Vec<u8>,
    units_per_em: f32,
    outlines: ResidencyCache<u16, Option<Path>>,
}

impl std::fmt::Debug for GlyphCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphCache")
            .field("cached_fonts", &self.fonts.len())
            .finish_non_exhaustive()
    }
}

/// A rough but real accounting of one outline's heap footprint: a `Path` is
/// a `Vec<PathVerb>`, and each verb carries up to three `Offset`s (two
/// `f32`s apiece) plus its discriminant — this doesn't need to be exact,
/// only proportional, since it drives an eviction *order*, not an
/// allocator.
fn outline_cost(outline: &Option<Path>) -> usize {
    const BYTES_PER_VERB: usize = std::mem::size_of::<vieww_foundation::PathVerb>();
    outline
        .as_ref()
        .map_or(std::mem::size_of::<Option<Path>>(), |path| {
            std::mem::size_of::<Path>() + path.verbs().len() * BYTES_PER_VERB
        })
}

impl GlyphCache {
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Like [`Self::new`], but with each font's glyph-outline
    /// [`ResidencyCache`] budgeted at `budget_bytes` instead of
    /// [`GLYPH_OUTLINE_BUDGET_BYTES`] — a smaller budget for a
    /// memory-constrained target, or to make eviction happen deterministically
    /// in a test rather than waiting on however many distinct glyphs real
    /// content happens to touch.
    #[must_use]
    pub(crate) fn with_glyph_outline_budget_bytes(budget_bytes: usize) -> Self {
        Self {
            fonts: HashMap::new(),
            glyph_outline_budget_bytes: budget_bytes,
        }
    }

    #[must_use]
    pub(crate) fn cached_fonts(&self) -> usize {
        self.fonts.len()
    }

    /// Hits, misses and evictions across every font's glyph-outline
    /// [`ResidencyCache`], summed. Each cache trims itself continuously, as
    /// it goes, under its own budget — this is read-only reporting, not
    /// something that has to be called to make eviction happen.
    pub(crate) fn cached_glyph_outline_stats(&self) -> ResidencyStats {
        self.fonts
            .values()
            .fold(ResidencyStats::default(), |acc, font| {
                let s = font.outlines.stats();
                ResidencyStats {
                    hits: acc.hits + s.hits,
                    misses: acc.misses + s.misses,
                    evictions: acc.evictions + s.evictions,
                }
            })
    }

    /// Drop every cached outline. Safe at any time: an outline is derived,
    /// read-only data re-extracted from the font's own bytes on next use, so
    /// clearing this changes nothing about what the next frame draws — only
    /// how long it takes.
    pub(crate) fn clear(&mut self) {
        self.fonts.clear();
    }

    /// The outline for `glyph_id` in `font`, in **font units** (not yet
    /// scaled to the run's size), or `None` if the font has no outline for
    /// it (missing glyph, or a bitmap/COLR-only glyph — see
    /// [`super::color_glyphs`], which handles the colour case this `None`
    /// reports).
    pub(crate) fn outline(&mut self, font: &FontData, glyph_id: u16) -> (Option<Path>, f32) {
        let budget = self.glyph_outline_budget_bytes;
        let entry = self.fonts.entry(font.id()).or_insert_with(|| {
            let face_data = font.bytes().to_vec();
            let units_per_em = ttf_parser::Face::parse(&face_data, font.index())
                .map(|f| f32::from(f.units_per_em()))
                .unwrap_or(1000.0);
            FontOutlines {
                face_data,
                units_per_em,
                outlines: ResidencyCache::new(budget),
            }
        });
        let upm = entry.units_per_em;
        let face_data = &entry.face_data;
        let font_index = font.index();
        let variations = font.variations().to_vec();
        let outline = entry
            .outlines
            .get_or_insert_with(
                glyph_id,
                || extract_outline(face_data, font_index, glyph_id, &variations),
                outline_cost,
            )
            .clone();
        (outline, upm)
    }
}

fn extract_outline(
    face_data: &[u8],
    face_index: u32,
    glyph_id: u16,
    variations: &[FontVariation],
) -> Option<Path> {
    let mut face = ttf_parser::Face::parse(face_data, face_index).ok()?;
    // **Every axis, before any contour is walked.** `set_variation` stores a
    // normalized coordinate on the face itself and applies `avar`, and
    // `outline_glyph` interpolates the `gvar` deltas against those
    // coordinates as it emits each point — so setting them after would leave
    // the builder with the default instance's geometry. Clamping is the
    // consumer's job (see `FontVariation`), and `set_variation` does it via
    // the axis's own min/max, so an out-of-range animation value lands on
    // the nearest extreme rather than producing a nonsense outline.
    for variation in variations {
        face.set_variation(ttf_parser::Tag::from_bytes(&variation.tag), variation.value);
    }
    let mut builder = PathBuilder::default();
    face.outline_glyph(ttf_parser::GlyphId(glyph_id), &mut builder)?;
    Some(builder.into_path())
}

#[derive(Default)]
struct PathBuilder {
    path: Path,
}

impl PathBuilder {
    fn into_path(self) -> Path {
        self.path
    }
}

/// Font-unit Y grows **up**; every other space in this crate has Y growing
/// down (screen convention), so every point is flipped here, once, at
/// extraction time.
impl ttf_parser::OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to(Offset::new(x, -y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to(Offset::new(x, -y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // Elevate the quadratic to a cubic (exact, not an approximation):
        // C1 = Q0 + 2/3*(Q1-Q0), C2 = Q2 + 2/3*(Q1-Q2).
        let last = self
            .path
            .verbs()
            .last()
            .map_or(Offset::new(0.0, 0.0), |v| match *v {
                vieww_foundation::PathVerb::MoveTo(p)
                | vieww_foundation::PathVerb::LineTo(p)
                | vieww_foundation::PathVerb::CubicTo(_, _, p) => p,
                vieww_foundation::PathVerb::Close => Offset::new(0.0, 0.0),
            });
        let q0 = last;
        let q1 = Offset::new(x1, -y1);
        let q2 = Offset::new(x, -y);
        let c1 = Offset::new(
            q0.dx + 2.0 / 3.0 * (q1.dx - q0.dx),
            q0.dy + 2.0 / 3.0 * (q1.dy - q0.dy),
        );
        let c2 = Offset::new(
            q2.dx + 2.0 / 3.0 * (q1.dx - q2.dx),
            q2.dy + 2.0 / 3.0 * (q1.dy - q2.dy),
        );
        self.path.cubic_to(c1, c2, q2);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.path.cubic_to(
            Offset::new(x1, -y1),
            Offset::new(x2, -y2),
            Offset::new(x, -y),
        );
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// A glyph's outline, positioned and scaled into device space, ready for
/// [`super::geometry::fill::rasterize`].
#[must_use]
pub(crate) fn place_glyph(
    outline: &Path,
    units_per_em: f32,
    run_origin: Offset,
    glyph_offset: Offset,
    size: f32,
    transform: vieww_foundation::Transform,
) -> Vec<Polyline> {
    let scale = size / units_per_em.max(1.0);
    let base = vieww_foundation::Transform {
        a: scale,
        b: 0.0,
        c: 0.0,
        d: scale,
        tx: run_origin.dx + glyph_offset.dx,
        ty: run_origin.dy + glyph_offset.dy,
    };
    let combined = base.then(transform);
    super::geometry::flatten::flatten_path(outline, combined)
}
