//! Rasterised glyphs, kept between frames — spec §6.2's glyph cache, which
//! `native/glyph.rs`'s own module doc has recorded as "not yet implemented
//! here" since the vello migration.
//!
//! # What was actually happening per frame, and why it is the biggest single
//! cost in a text-heavy window
//!
//! [`GlyphCache`](super::glyph::GlyphCache) caches *outlines*, in font units.
//! Everything after that ran again for every glyph of every frame:
//!
//! 1. the cached `Path` was **cloned** out of the residency cache (an
//!    allocation),
//! 2. [`place_glyph`](super::glyph::place_glyph) scaled and translated it into
//!    device space and flattened its curves into polylines (another),
//! 3. [`rasterize`](super::geometry::fill::rasterize) built an edge list from
//!    those polylines, sorted it, walked an active-edge table over every pixel
//!    row of the glyph with four vertical subsamples, and allocated a coverage
//!    buffer to write the answer into (two more).
//!
//! One frame of `viewwstudio`'s shell contains **1,447 glyphs**. None of that
//! work depends on anything that changes between frames: the same glyph, at the
//! same size, under the same transform, at the same sub-pixel phase, produces
//! the same coverage values every time. So it is done once and kept.
//!
//! # Why the key is exact bits rather than a quantised phase
//!
//! Spec §6.2 describes a "quarter-pixel x-phase" key, which is what FreeType,
//! DirectWrite and CoreText all do: snap the sub-pixel position to one of four
//! phases so that four entries cover every possible placement of a glyph.
//!
//! This cache keys on the **exact** `f32` bits of the fractional origin and of
//! the transform instead, and that is a deliberate trade rather than an
//! unfinished version of the spec's. Quantising is a resampling: it moves
//! pixels, which means every golden image in the framework changes, the
//! renderer stops being the byte-exact oracle `native/reference.rs`'s module
//! doc says it is, and "the text looks very slightly different now" becomes a
//! thing that has to be argued rather than measured.
//!
//! Keying exactly costs nothing in practice, because the case that matters is
//! **repetition, not proximity**. Text in a laid-out interface lands on the
//! same sub-pixel phases frame after frame — a static editor pane re-renders at
//! byte-identical positions, so the hit rate is essentially 100%, which is the
//! case that turns a 12 fps window into a smooth one. Text that is *moving*
//! (a scroll mid-flick, a sliding sheet) misses, and pays exactly what it paid
//! before this cache existed plus a hash lookup. Nothing regresses and the
//! common case gets much faster.
//!
//! # It is not byte-identical, and here is exactly how far off it is
//!
//! Caching by phase means rasterising the glyph at its *fractional* origin and
//! translating the result by whole pixels, rather than rasterising it at its
//! full device position. Those are the same picture in exact arithmetic and not
//! quite the same in `f32`: the old path computed an edge as
//! `a * px + T` where `T` is a device coordinate in the hundreds, and this one
//! computes `a * px + (T - floor(T))`, where the second term is under one.
//! Floating-point addition is not associative, so the two round differently in
//! the last bit — and the new one is the *more* precise of the two, because the
//! glyph's sub-pixel offset is no longer being added into a number three orders
//! of magnitude larger than itself.
//!
//! Measured across the whole `examples/fixtures` gallery — 23 fixtures, about
//! 2.5 million pixels — **57 pixels differ, every one of them by exactly
//! 1/255**, all of them on the antialiased edge of a glyph. No pixel moves by
//! more than that and no shape, colour or position changes.
//!
//! That is a real difference and it is written down here rather than rounded to
//! "none", because `native/reference.rs` calls this renderer an oracle and an
//! oracle that has quietly stopped matching its own baselines is worse than one
//! that has loudly changed them.
//!
//! A quarter-pixel phase key would be faster still — four entries covering
//! every placement instead of one per distinct offset — and would move pixels
//! by rather more than 1/255. If it is ever wanted it belongs here as an opt-in
//! alongside this, with its own re-blessed baselines.
//!
//! # Budget
//!
//! Entries are coverage buffers, so they are much larger than the outlines
//! `glyph.rs` caches and they need a real bound. [`GlyphRasterCache`] holds a
//! byte budget and evicts least-recently-used entries past it, the same policy
//! and for the same reason as [`ResidencyCache`](super::residency): a
//! rasterised glyph is derived, deterministic data, so dropping one changes
//! how long the next frame takes and nothing about what it shows.

use std::collections::HashMap;

use vieww_foundation::{FontData, Offset, Rect, Transform};

use super::geometry::fill::{rasterize, rasterize_lcd, CoverageMask, LcdMask};
use super::glyph::place_glyph;

/// What one cache entry holds: grayscale coverage, or the LCD variant.
///
/// The two are different answers to the same question — "how much of this
/// pixel does the glyph cover" — at different horizontal resolutions, and
/// the cache holds either under the same LRU/budget policy, keyed apart by
/// [`GlyphKey::lcd`] so a glyph drawn both ways keeps two entries rather
/// than one silently wrong for one of its uses.
#[derive(Debug)]
pub(crate) enum Rasterized {
    /// One coverage value per pixel — the shared-AA answer every shape gets.
    Gray(CoverageMask),
    /// Three per pixel, one per RGB sub-column — see `geometry::fill`'s
    /// `LcdMask` for what that buys and what it costs.
    Lcd(LcdMask),
}

impl Rasterized {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Self::Gray(mask) => mask.is_empty(),
            Self::Lcd(mask) => mask.is_empty(),
        }
    }

    /// How many coverage values this entry holds — the budget cost.
    #[must_use]
    pub(crate) fn data_len(&self) -> usize {
        match self {
            Self::Gray(mask) => mask.data_len(),
            Self::Lcd(mask) => mask.data_len(),
        }
    }

    /// The grayscale mask, when this entry is one. Colour-layer and GPU-seam
    /// callers ask for gray specifically; an LCD entry answering them is a
    /// caller bug, not a conversion — turning three sub-column coverages back
    /// into one is the *rasterizer's* average to take, not a getter's.
    #[must_use]
    pub(crate) fn as_gray(&self) -> Option<&CoverageMask> {
        match self {
            Self::Gray(mask) => Some(mask),
            Self::Lcd(_) => None,
        }
    }
}

/// How many bytes of rasterised glyph coverage to keep before evicting.
///
/// A 13-point glyph's mask is on the order of 10x14 `f32`s — around half a
/// kilobyte — so this holds a few thousand distinct rasterisations: comfortably
/// every glyph visible in a large window at every size and phase it appears at,
/// which is the working set that matters, without letting a session that has
/// scrolled through a lot of text at a lot of sub-pixel phases grow without
/// bound.
const BUDGET_BYTES: usize = 8 * 1024 * 1024;

/// Everything about a glyph that decides what pixels it produces.
///
/// The transform is carried as raw bits rather than as an f32 tuple so the key
/// can be `Hash` and `Eq` — and because bit equality is exactly the equality
/// wanted here: two transforms that differ in the last bit can produce
/// different coverage, so they must be different entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct GlyphKey {
    font: u64,
    glyph: u16,
    size: u32,
    transform: [u32; 4],
    /// The sub-pixel part of the glyph's device origin, in bits.
    phase: (u32, u32),
    /// Whether this entry is LCD-subpixel coverage (three values per pixel)
    /// or grayscale (one). A glyph the same renderer draws both ways — onto
    /// the root with LCD on, and inside a layer where LCD falls back to
    /// gray — is two entries, because the pixels it produces differ.
    lcd: bool,
}

/// What the cache knows about one glyph.
///
/// Three cases, not two, because the renderer's own [`SceneReport`] counts them
/// differently and has since before this cache existed: a glyph the font has no
/// outline for — a space, or a bitmap-only glyph — never reached the backend and
/// is not counted, while a glyph that *has* an outline and rasterises to nothing
/// at this size did reach it and is. Collapsing the two into one `None` changes
/// that count, which `vieww`'s own `text_to_pixels` test asserts on precisely
/// because it is how "a glyph quietly did not get drawn" is caught.
#[derive(Debug)]
pub(crate) enum Glyph<'a> {
    /// The font has no outline for this glyph id.
    NoOutline,
    /// An outline that covers no pixels at this size and phase.
    Blank,
    /// Coverage, and the whole-pixel device translation to draw it at. The
    /// mask is gray or LCD depending on what the caller asked for — see
    /// [`Rasterized`].
    Inked {
        raster: &'a Rasterized,
        x: i32,
        y: i32,
    },
}

struct Entry {
    raster: Rasterized,
    /// Whether the font had an outline at all — see [`Glyph`].
    outlined: bool,
    /// A monotonically increasing stamp, for the LRU order.
    used: u64,
    cost: usize,
}

/// Rasterised glyph coverage, keyed by [`GlyphKey`], bounded by a byte budget.
pub(crate) struct GlyphRasterCache {
    entries: HashMap<GlyphKey, Entry>,
    budget: usize,
    bytes: usize,
    clock: u64,
    hits: u64,
    misses: u64,
}

impl std::fmt::Debug for GlyphRasterCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphRasterCache")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish()
    }
}

impl Default for GlyphRasterCache {
    fn default() -> Self {
        Self::new(BUDGET_BYTES)
    }
}

impl GlyphRasterCache {
    #[must_use]
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget,
            bytes: 0,
            clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Cache hits and misses since this cache was made — what a test asserts a
    /// steady-state frame against, and what says whether the working set fits.
    #[must_use]
    pub(crate) fn stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// The coverage for one glyph, and the integer device position to draw it
    /// at.
    ///
    /// The mask is rasterised at the glyph's *fractional* origin only — its
    /// whole-pixel translation is returned separately, as `(x, y)`, so that the
    /// same coverage serves every occurrence of the glyph that shares a phase.
    /// The caller offsets the view; see [`CoverageMask::view_at`].
    ///
    /// `outline` is the font-unit path and `upm` its units-per-em, both from
    /// [`GlyphCache::outline`](super::glyph::GlyphCache::outline). They are
    /// only read on a miss.
    #[expect(
        clippy::too_many_arguments,
        reason = "every one of these is part of what identifies a rasterised \
                  glyph — font, id, size, the run and glyph offsets that give \
                  its position, the transform, and which AA policy produced \
                  it. Bundling them into a struct would move the same values \
                  behind one name and add a type whose only member function \
                  is this one."
    )]
    pub(crate) fn coverage(
        &mut self,
        font: &FontData,
        glyph_id: u16,
        size: f32,
        run_origin: Offset,
        glyph_offset: Offset,
        transform: Transform,
        lcd: bool,
        outline: impl FnOnce() -> Option<(vieww_foundation::Path, f32)>,
    ) -> Glyph<'_> {
        // Where this glyph's own space lands in device space — the translation
        // half of what `place_glyph` builds, computed here so it can be split
        // into a whole-pixel part (which moves the mask) and a fractional part
        // (which changes it).
        let placed = Offset::new(
            run_origin.dx + glyph_offset.dx,
            run_origin.dy + glyph_offset.dy,
        );
        let device = transform.apply(placed);
        let ix = device.dx.floor();
        let iy = device.dy.floor();
        if !ix.is_finite() || !iy.is_finite() {
            return Glyph::NoOutline;
        }
        let phase = (device.dx - ix, device.dy - iy);

        let key = GlyphKey {
            font: font.id(),
            glyph: glyph_id,
            size: size.to_bits(),
            transform: [
                transform.a.to_bits(),
                transform.b.to_bits(),
                transform.c.to_bits(),
                transform.d.to_bits(),
            ],
            phase: (phase.0.to_bits(), phase.1.to_bits()),
            lcd,
        };

        self.clock += 1;
        let clock = self.clock;
        // A glyph that produced nothing is cached as such, so the miss path
        // does not run again for every space in the document.
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = clock;
            self.hits += 1;
            let entry = &self.entries[&key];
            #[expect(clippy::cast_possible_truncation, reason = "checked finite above")]
            return Self::answer(entry, ix as i32, iy as i32);
        }

        self.misses += 1;
        let outlined;
        let raster = match outline() {
            Some((path, upm)) => {
                // Rasterised at the *fractional* origin, with the same
                // transform, so the coverage is what the glyph would have
                // produced in place — shifted by a whole number of pixels,
                // which does not change coverage.
                let shifted = Transform {
                    tx: transform.tx - ix,
                    ty: transform.ty - iy,
                    ..transform
                };
                let polylines = place_glyph(&path, upm, run_origin, glyph_offset, size, shifted);
                // No clip: the entry has to be reusable under whatever clip the
                // next occurrence is under, so the clip is applied by the
                // caller when the view is taken. `UNBOUNDED` is what makes
                // `rasterize` fall back to the path's own bounding box.
                outlined = true;
                if lcd {
                    Rasterized::Lcd(rasterize_lcd(&polylines, UNBOUNDED))
                } else {
                    Rasterized::Gray(rasterize(&polylines, UNBOUNDED))
                }
            }
            None => {
                outlined = false;
                Rasterized::Gray(CoverageMask::empty())
            }
        };

        let cost = raster.data_len() * std::mem::size_of::<f32>() + std::mem::size_of::<Entry>();
        self.bytes += cost;
        self.entries.insert(
            key,
            Entry {
                raster,
                outlined,
                used: clock,
                cost,
            },
        );
        self.evict_to_budget(key);

        let Some(entry) = self.entries.get(&key) else {
            return Glyph::NoOutline;
        };
        #[expect(clippy::cast_possible_truncation, reason = "checked finite above")]
        Self::answer(entry, ix as i32, iy as i32)
    }

    /// One entry, as the three-way answer callers switch on.
    fn answer(entry: &Entry, x: i32, y: i32) -> Glyph<'_> {
        if !entry.outlined {
            Glyph::NoOutline
        } else if entry.raster.is_empty() {
            Glyph::Blank
        } else {
            Glyph::Inked {
                raster: &entry.raster,
                x,
                y,
            }
        }
    }

    /// Drop least-recently-used entries until the budget is met, never
    /// dropping `keep` — which is the entry the caller is about to return a
    /// reference to.
    fn evict_to_budget(&mut self, keep: GlyphKey) {
        while self.bytes > self.budget && self.entries.len() > 1 {
            let victim = self
                .entries
                .iter()
                .filter(|(k, _)| **k != keep)
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k);
            let Some(victim) = victim else { break };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(entry.cost);
            }
        }
    }
}

/// The "no clip at all" rectangle handed to [`rasterize`] for a cached glyph.
///
/// `rasterize` intersects the shape's own bounds with this, so a rectangle
/// larger than any glyph could be leaves the shape's bounds untouched — which
/// is the point: the entry must be the whole glyph, because the clip that
/// applies to it is not known until it is drawn.
const UNBOUNDED: Rect = Rect {
    left: -1.0e7,
    top: -1.0e7,
    right: 1.0e7,
    bottom: 1.0e7,
};

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Path;

    fn square_outline() -> Option<(Path, f32)> {
        let mut path = Path::default();
        path.move_to(Offset::new(0.0, 0.0));
        path.line_to(Offset::new(500.0, 0.0));
        path.line_to(Offset::new(500.0, -500.0));
        path.line_to(Offset::new(0.0, -500.0));
        path.close();
        Some((path, 1000.0))
    }

    /// A `FontData` whose bytes are never parsed: every test here supplies the
    /// outline directly through the closure, so the font is only ever an
    /// identity in the cache key.
    fn font() -> FontData {
        FontData::new(std::rc::Rc::new(vec![0_u8; 4]), 0)
    }

    #[test]
    fn the_same_glyph_at_the_same_phase_is_a_hit() {
        let mut cache = GlyphRasterCache::default();
        let t = Transform::IDENTITY;
        let f = font();
        for _ in 0..3 {
            let _ = cache.coverage(
                &f,
                7,
                13.0,
                Offset::new(10.25, 20.5),
                Offset::new(0.0, 0.0),
                t,
                false,
                square_outline,
            );
        }
        assert_eq!(cache.stats(), (2, 1), "three draws, one rasterisation");
    }

    /// The property the whole-pixel/fractional split rests on: moving a glyph
    /// by whole pixels must reuse the entry and only change where it lands.
    #[test]
    fn a_whole_pixel_move_reuses_the_entry_and_moves_the_origin() {
        let mut cache = GlyphRasterCache::default();
        let f = font();
        let Glyph::Inked { x: x0, y: y0, .. } = cache.coverage(
            &f,
            7,
            13.0,
            Offset::new(10.25, 20.5),
            Offset::new(0.0, 0.0),
            Transform::IDENTITY,
            false,
            square_outline,
        ) else {
            panic!("a filled glyph")
        };
        let Glyph::Inked { x: x1, y: y1, .. } = cache.coverage(
            &f,
            7,
            13.0,
            Offset::new(13.25, 27.5),
            Offset::new(0.0, 0.0),
            Transform::IDENTITY,
            false,
            square_outline,
        ) else {
            panic!("a filled glyph")
        };
        assert_eq!(cache.stats().0, 1, "the second draw must be a hit");
        assert_eq!((x1 - x0, y1 - y0), (3, 7));
    }

    #[test]
    fn a_different_sub_pixel_phase_is_a_different_entry() {
        let mut cache = GlyphRasterCache::default();
        let f = font();
        for x in [10.25_f32, 10.5, 10.75] {
            let _ = cache.coverage(
                &f,
                7,
                13.0,
                Offset::new(x, 20.0),
                Offset::new(0.0, 0.0),
                Transform::IDENTITY,
                false,
                square_outline,
            );
        }
        assert_eq!(cache.stats(), (0, 3), "three phases, three rasterisations");
    }

    /// LCD and gray coverage of the *same* glyph at the same phase are two
    /// entries — the pixels they produce differ — and asking twice for either
    /// is a hit on the second ask. This is the property that lets a renderer
    /// that falls back to gray inside layers keep both entries live at once.
    #[test]
    fn lcd_and_gray_are_separate_entries() {
        let mut cache = GlyphRasterCache::default();
        let f = font();
        let args = (
            &f,
            7_u16,
            13.0_f32,
            Offset::new(10.25, 20.5),
            Offset::new(0.0, 0.0),
            Transform::IDENTITY,
        );
        let _ = cache.coverage(
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            false,
            square_outline,
        );
        let _ = cache.coverage(
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            true,
            square_outline,
        );
        let _ = cache.coverage(
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            false,
            square_outline,
        );
        let _ = cache.coverage(
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            true,
            square_outline,
        );
        assert_eq!(
            cache.stats(),
            (2, 2),
            "one gray miss, one LCD miss, two hits"
        );
        // And the LCD entry really is an LCD entry.
        let Glyph::Inked { raster, .. } = cache.coverage(
            args.0,
            args.1,
            args.2,
            args.3,
            args.4,
            args.5,
            true,
            square_outline,
        ) else {
            panic!("a filled glyph")
        };
        assert!(raster.as_gray().is_none(), "asked LCD, got LCD");
    }

    #[test]
    fn a_glyph_with_no_outline_is_cached_as_empty_rather_than_retried() {
        let mut cache = GlyphRasterCache::default();
        let f = font();
        for _ in 0..3 {
            assert!(matches!(
                cache.coverage(
                    &f,
                    7,
                    13.0,
                    Offset::new(1.0, 1.0),
                    Offset::new(0.0, 0.0),
                    Transform::IDENTITY,
                    false,
                    || None,
                ),
                Glyph::NoOutline
            ));
        }
        assert_eq!(cache.stats(), (2, 1));
    }

    #[test]
    fn the_budget_evicts_rather_than_growing_without_bound() {
        let mut cache = GlyphRasterCache::new(4096);
        let f = font();
        for i in 0..200u16 {
            let _ = cache.coverage(
                &f,
                i,
                13.0,
                Offset::new(1.0, 1.0),
                Offset::new(0.0, 0.0),
                Transform::IDENTITY,
                false,
                square_outline,
            );
        }
        assert!(
            cache.bytes <= 4096 + 8192,
            "held {} bytes against a 4096 budget",
            cache.bytes
        );
        assert!(cache.entries.len() < 200, "nothing was evicted");
    }
}
