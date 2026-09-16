//! Positioned glyphs: the handoff between shaping and drawing.
//!
//! These types are the *result* of text layout, not a request for it. A
//! [`GlyphRun`] says "draw glyph 42 of this font at exactly this position", with
//! every question of script, direction, kerning and line breaking already
//! answered.
//!
//! # Why this lives in foundation
//!
//! Shaping happens in `vieww-text` (which owns `cosmic-text`) and drawing happens
//! in `vieww-paint` (which owns the GPU backend). Neither should depend on the
//! other: the paint layer must not pull in a shaper, and the shaper must not pull
//! in a graphics stack. They meet here, on a type that is nothing but data.
//!
//! # Why a glyph id and not a character
//!
//! By the time text is shaped, characters have stopped being the unit of
//! anything. One character can produce several glyphs, several characters can
//! produce one glyph, and the mapping depends on the font. A backend that
//! received characters would have to shape them again to draw them — and would
//! get different answers than layout did, which is how text ends up overflowing
//! the box that was measured for it.

use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Color, Offset, Rect, Size};

/// Source of [`FontData::id`]. `u64` at one mint per construction does not
/// wrap in any process's lifetime, so "never reused" needs no wraparound
/// handling.
static NEXT_FONT_ID: AtomicU64 = AtomicU64::new(0);

/// One variation-axis setting on a variable font: a four-character axis tag
/// (`"wght"`, `"opsz"`, `"wdth"`, `"slnt"`…) and the **user-space** value to
/// pin it at.
///
/// User space, not normalized: `wght = 650` means the same thing it means in
/// CSS `font-variation-settings` and in every font editor, and the renderer
/// that consumes it normalizes against the axis's own min/default/max (and
/// applies `avar`) at the moment it parses the face. A caller never needs to
/// know a font's units-per-em or its normalization breakpoints.
///
/// Out-of-range values are clamped by the consumer rather than rejected here —
/// the same tolerance every platform's `font-variation-settings` has, and the
/// one that keeps an animated axis (a weight that eases between two stops)
/// expressible without a branch at every frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontVariation {
    /// The axis tag, four bytes, e.g. `b"wght"`.
    pub tag: [u8; 4],
    /// The user-space value to set the axis to.
    pub value: f32,
}

impl FontVariation {
    /// A setting for `tag` (a four-character axis tag, e.g. `"wght"`) at
    /// `value`.
    ///
    /// Panics if `tag` is not exactly four bytes — the OpenType axis-tag length
    /// is fixed, and a wrong-length tag is a caller bug worth failing loudly
    /// at the call site rather than silently at the rasterizer.
    #[must_use]
    pub const fn new(tag: &[u8; 4], value: f32) -> Self {
        Self { tag: *tag, value }
    }

    /// `"wght"` at `value` — the weight axis, and the one every framework's
    /// bold/medium/semibold vocabulary maps onto.
    #[must_use]
    pub const fn weight(value: f32) -> Self {
        Self {
            tag: *b"wght",
            value,
        }
    }

    /// The tag as the four-character string it was built from, when it is
    /// printable as one (it is, for every registered axis).
    #[must_use]
    pub fn tag_str(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.tag)
    }
}

/// The bytes of a font file, shared cheaply.
///
/// Shared rather than copied because the same font is referenced by every run
/// that uses it, and a font file is on the order of a megabyte.
#[derive(Clone)]
pub struct FontData {
    bytes: Rc<Vec<u8>>,
    index: u32,
    /// Variation coordinates this font is drawn at — empty for every static
    /// face, and for a variable face at its default instance.
    ///
    /// Part of the font's *identity* rather than a draw-time parameter on
    /// purpose: a backend's outline and raster caches are keyed by
    /// [`Self::id`], and the same variable face drawn at `wght 300` and
    /// `wght 700` produces two different sets of outlines that must never share
    /// a cache entry. Minting a distinct id per variation set (which
    /// [`Self::with_variations`] does) is what keeps that true without asking
    /// every backend to know what a variation is.
    variations: Rc<[FontVariation]>,
    id: u64,
}

/// `id` is deliberately left out.
///
/// It is process-allocation identity, not content — two independently loaded
/// `FontStore`s produce the *same font* with different ids, by design (see
/// [`FontData::id`]). Several tests compare a cached render's commands against
/// a from-scratch one *by their `Debug` string*, exactly to check that caching
/// changes nothing about what is drawn — `crates/vieww-render/tests/
/// incremental_sync.rs`'s `a_changed_label_reaches_the_screen` is one.
/// Printing `id` would fail that comparison for a reason that has nothing to
/// do with the claim it is checking, on every run, because two constructions
/// mint two different ids by definition.
impl fmt::Debug for FontData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontData")
            .field("bytes", &self.bytes)
            .field("index", &self.index)
            .field("variations", &self.variations)
            .finish()
    }
}

impl FontData {
    /// Wrap the bytes of a font file.
    ///
    /// `index` selects a face within a font *collection* (`.ttc`/`.otc`); it is
    /// 0 for the single-face files that most fonts are.
    ///
    /// Mints a fresh [`Self::id`]. Callers that want a *stable* id across
    /// frames — every render backend does, for its font cache — must hand out
    /// clones of one `FontData` rather than calling this twice for what is
    /// logically the same font; [`vieww_text::FontStore`](https://docs.rs/vieww-text)
    /// already does this.
    #[must_use]
    pub fn new(bytes: Rc<Vec<u8>>, index: u32) -> Self {
        Self {
            bytes,
            index,
            variations: Rc::from(Vec::new()),
            id: NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The same face, drawn at `variations` — see [`FontVariation`].
    ///
    /// For a static face this is a no-op with a new id: there are no axes to
    /// set, the bytes draw identically, and only the identity (cache keys,
    /// damage comparisons) changes. For a variable face this is what makes
    /// `wght 650` render as the interpolated instance rather than as whatever
    /// the font's default is.
    ///
    /// Mints a fresh [`Self::id`], for the reason the field's own doc gives:
    /// two variation settings of one font are two different sets of outlines
    /// and must not share a backend cache entry. Order in `variations` is
    /// insignificant — the consumer sets each axis by tag.
    #[must_use]
    pub fn with_variations(bytes: Rc<Vec<u8>>, index: u32, variations: &[FontVariation]) -> Self {
        Self {
            bytes,
            index,
            variations: Rc::from(variations.to_vec()),
            id: NEXT_FONT_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The shared handle, for a backend that wants to pin the allocation an
    /// [`Image`](https://docs.rs/vieww-paint)-style cache would key on it.
    #[must_use]
    pub fn shared(&self) -> &Rc<Vec<u8>> {
        &self.bytes
    }

    /// Which face within a font collection this refers to.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// A process-unique identity, minted once at construction and carried by
    /// every [`Clone`], for a backend to key a font cache on.
    ///
    /// **Not** the `Rc`'s address. An address identifies an allocation only
    /// while it is alive — drop the last `Rc` to one font, load another, let
    /// the allocator reuse the block, and an address-keyed cache serves the
    /// old font's glyphs for the new font's bytes, silently. This id cannot
    /// collide that way: it comes from a monotonic counter, not the
    /// allocator, so a font's id is never handed to a different font, dropped
    /// or not — only [`Clone`] hands out the same id twice, and a clone is by
    /// definition the same font.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// The variation coordinates this font is drawn at — empty for a static
    /// face or a variable face at its default instance.
    ///
    /// Values are user-space (see [`FontVariation`]); the consumer normalizes.
    #[must_use]
    pub fn variations(&self) -> &[FontVariation] {
        &self.variations
    }

    /// `true` when this font carries at least one variation coordinate.
    #[must_use]
    pub fn is_variable_instance(&self) -> bool {
        !self.variations.is_empty()
    }
}

/// Equality is by *identity*, for the same reason
/// [`Image`](https://docs.rs/vieww-paint)'s is: damage tracking compares last
/// frame's commands against this frame's, and comparing megabytes of font bytes
/// on every text run every frame would defeat the point of tracking damage.
///
/// Two separately loaded copies of the same font compare unequal and so repaint
/// needlessly. That is the safe direction to be wrong in.
///
/// Compared by [`Self::id`] rather than the `Rc`'s address, for the reason
/// documented there: an id is stable for the value's whole life and an
/// address is only stable while the allocation is.
impl PartialEq for FontData {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for FontData {}

/// One positioned glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Index into the font's glyph table. Meaningless without the font.
    pub id: u16,
    /// Where the glyph's origin sits, relative to the run's origin.
    ///
    /// The origin is on the **baseline**, not the top-left, because that is what
    /// every font's metrics are expressed relative to and what every rasteriser
    /// expects. Getting this wrong shifts text by an ascent and is the classic
    /// "why is my text above the box" bug.
    pub offset: Offset,
}

impl Glyph {
    /// The glyph id every font reserves for "I do not have this character".
    ///
    /// Zero, by the OpenType specification, in every font that exists.
    pub const NOTDEF: u16 = 0;

    #[must_use]
    pub const fn new(id: u16, offset: Offset) -> Self {
        Self { id, offset }
    }

    /// `true` when the shaper had no glyph for this character.
    ///
    /// # Why this needs a name
    ///
    /// A `.notdef` glyph is *drawn* like any other — the backend rasterises
    /// glyph 0 and reports a glyph drawn. In most fonts glyph 0 is a hollow
    /// box, and the reader sees the familiar "tofu". **In DejaVu, the font
    /// embedded here, glyph 0 is empty**: it takes its advance on the line and
    /// inks nothing at all.
    ///
    /// That is the worst possible failure. A missing box tells a reader "this
    /// text needs a font you do not have". Blank space tells them nothing, and
    /// tells a developer that layout is broken rather than that coverage is.
    /// A whole screen of Chinese renders as correctly-spaced emptiness.
    ///
    /// So the renderer detects this rather than trusting the font to draw
    /// something — see `RenderText::paint`, which strokes its own box.
    #[must_use]
    pub const fn is_missing(&self) -> bool {
        self.id == Self::NOTDEF
    }
}

/// A run of glyphs sharing one font, size and colour.
///
/// A run is the largest span a backend can draw in one call. A line of text
/// becomes several runs when it changes font, size, colour, or direction — which
/// is why a run carries [`is_rtl`](Self::is_rtl) rather than the paragraph
/// carrying a single direction.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphRun {
    /// The font these glyph ids index into.
    pub font: FontData,
    /// Em size in logical pixels.
    pub size: f32,
    pub color: Color,
    /// Where this run's baseline starts, in the coordinate space of whatever is
    /// drawing it. Individual [`Glyph`] offsets are relative to this.
    pub origin: Offset,
    /// The positioned glyphs, shared rather than owned.
    ///
    /// An `Rc<[Glyph]>` because a run is cloned twice on the way to the screen
    /// and never edited: the render object clones it to move its origin, and
    /// the scene clones it again to fade its colour. Both rewrite a single
    /// scalar field. As a `Vec` that was two heap allocations and two copies of
    /// every glyph in the run, per text node, **per frame** — the second
    /// largest per-frame allocation in the paint path after the image buffers.
    ///
    /// `Rc` and not `Arc` because [`FontData`] beside it is already `Rc`, so a
    /// run has never been `Send` and nothing gains the ability to cross a
    /// thread by this being atomic. Text is shaped where it is drawn.
    ///
    /// Build one with `glyphs.into()` from a `Vec<Glyph>`; every read —
    /// `len`, `iter`, indexing — works unchanged through `Deref`.
    pub glyphs: Rc<[Glyph]>,
    /// `true` if this run was laid out right-to-left.
    ///
    /// Carried through to drawing because a caret, a selection highlight and a
    /// hit test all need to know which end of the run is its logical start.
    /// Glyph positions are already final either way.
    pub is_rtl: bool,
    /// Distance from the baseline up to the top of the line box.
    pub ascent: f32,
    /// Distance from the baseline down to the bottom of the line box, positive.
    pub descent: f32,
}

impl GlyphRun {
    /// How many glyphs in this run the font could not supply.
    ///
    /// Counted work, in the spirit of `SceneReport::skipped_text`: a claim that
    /// text rendered is only a claim if something counts what did not.
    #[must_use]
    pub fn missing_count(&self) -> usize {
        self.glyphs
            .iter()
            .filter(|glyph| glyph.is_missing())
            .count()
    }

    /// The boxes a renderer should stroke in place of the glyphs it has no
    /// coverage for, in this run's own coordinate space.
    ///
    /// Each rect sits on the baseline the way a capital letter does: from
    /// `ascent * 0.72` above it down to the baseline, inset slightly inside the
    /// advance so consecutive boxes read as separate characters.
    ///
    /// # Where the width comes from
    ///
    /// A [`Glyph`] carries a position, not an advance — the next glyph's offset
    /// *is* the advance, which is all a rasteriser needs. So the width of the
    /// last glyph in a run has no source, and falls back to a proportion of the
    /// em size. That only matters for a run ending in an uncovered character,
    /// where a slightly-wrong box is still infinitely better than nothing.
    #[must_use]
    pub fn missing_boxes(&self) -> Vec<Rect> {
        // A capital's height, roughly, for a box that sits among letters
        // without towering over them.
        let top = self.ascent * 0.72;
        // Enough of the em that a box is legible at 11pt, small enough that a
        // run of them does not merge into a bar.
        let fallback_advance = self.size * 0.6;
        let inset = self.size * 0.06;

        self.glyphs
            .iter()
            .enumerate()
            .filter(|(_, glyph)| glyph.is_missing())
            .map(|(index, glyph)| {
                let advance = self
                    .glyphs
                    .get(index + 1)
                    .map(|next| next.offset.dx - glyph.offset.dx)
                    .filter(|width| *width > 0.0)
                    .unwrap_or(fallback_advance);
                let left = self.origin.dx + glyph.offset.dx + inset;
                let baseline = self.origin.dy + glyph.offset.dy;
                Rect::new(left, baseline - top, left + advance - inset * 2.0, baseline)
            })
            .collect()
    }

    /// `true` if this run would draw nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty() || self.color.is_transparent()
    }

    /// The horizontal extent from the leftmost glyph origin to the rightmost.
    ///
    /// Measured by minimum and maximum rather than first and last, because glyphs
    /// within a run are in **logical** order: in a right-to-left run the first
    /// glyph is the rightmost, so subtracting first from last would report a
    /// negative width.
    ///
    /// Not the inked width — it excludes the final glyph's own extent, which needs
    /// the font to know. For a *drawing* bound, use [`bounds`](Self::bounds).
    #[must_use]
    pub fn advance(&self) -> f32 {
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        for glyph in self.glyphs.iter() {
            left = left.min(glyph.offset.dx);
            right = right.max(glyph.offset.dx);
        }
        if left > right {
            0.0
        } else {
            right - left
        }
    }

    /// A conservative box around everything this run can ink.
    ///
    /// Padded by one em on the trailing edge, because the last glyph's width is
    /// not known without consulting the font, and by the ascent and descent
    /// vertically. Damage tracking consumes this, so it rounds *outward* — too
    /// large costs fill rate, too small leaves stale pixels.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        if self.glyphs.is_empty() {
            return Rect::ZERO;
        }
        let mut left = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        for glyph in self.glyphs.iter() {
            left = left.min(glyph.offset.dx);
            right = right.max(glyph.offset.dx);
        }
        Rect::new(
            self.origin.dx + left,
            self.origin.dy - self.ascent,
            self.origin.dx + right + self.size,
            self.origin.dy + self.descent,
        )
    }

    /// The line box this run occupies: its advance by its ascent plus descent.
    #[must_use]
    pub fn line_size(&self) -> Size {
        Size::new(self.advance(), self.ascent + self.descent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> FontData {
        FontData::new(Rc::new(vec![0_u8; 16]), 0)
    }

    fn run(glyphs: Vec<Glyph>) -> GlyphRun {
        GlyphRun {
            font: font(),
            size: 10.0,
            color: Color::BLACK,
            origin: Offset::new(100.0, 50.0),
            glyphs: glyphs.into(),
            is_rtl: false,
            ascent: 8.0,
            descent: 2.0,
        }
    }

    #[test]
    fn a_cloned_font_compares_equal_but_a_reloaded_one_does_not() {
        let font = font();
        assert_eq!(font, font.clone(), "a clone shares the bytes");
        assert_ne!(
            font,
            FontData::new(Rc::new(vec![0_u8; 16]), 0),
            "identity comparison keeps damage diffing off the font bytes"
        );
    }

    #[test]
    fn a_clone_keeps_the_same_id_a_reload_does_not() {
        let font = font();
        assert_eq!(
            font.id(),
            font.clone().id(),
            "a cache keyed on id must see a clone as one entry"
        );
        assert_ne!(
            font.id(),
            FontData::new(Rc::new(vec![0_u8; 16]), 0).id(),
            "two separately loaded fonts must never share an id"
        );
    }

    /// The bug `FontData::id` exists to close, reproduced without needing the
    /// allocator to actually reuse an address (which is not something a test
    /// can force): drop a font, load a different one, and confirm the two
    /// have different ids regardless of what the freed and the new allocation
    /// happen to share. A cache keyed on `Rc::as_ptr` cannot make this
    /// guarantee — this test is what a `usize`-keyed cache would fail if the
    /// allocator ever reused the block, which is exactly why the render
    /// backends no longer key on the pointer.
    #[test]
    fn a_dropped_fonts_id_is_never_reissued_to_a_different_font() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            // Each iteration's `Rc` is dropped at the end of its block, so the
            // allocator is free (not guaranteed, but free) to reuse the
            // address across iterations.
            let id = font().id();
            assert!(seen.insert(id), "id {id} was reissued to a later font");
        }
    }

    #[test]
    fn an_empty_run_draws_nothing_and_bounds_to_zero() {
        let run = run(Vec::new());
        assert!(run.is_empty());
        assert_eq!(run.bounds(), Rect::ZERO);
    }

    #[test]
    fn a_transparent_run_draws_nothing_even_with_glyphs() {
        let mut run = run(vec![Glyph::new(1, Offset::ZERO)]);
        run.color = Color::TRANSPARENT;
        assert!(run.is_empty());
    }

    #[test]
    fn bounds_are_measured_from_the_baseline_not_the_top() {
        let run = run(vec![
            Glyph::new(1, Offset::ZERO),
            Glyph::new(2, Offset::new(6.0, 0.0)),
        ]);
        let bounds = run.bounds();

        // origin.dy is the baseline at 50; the box runs from 50-ascent to
        // 50+descent. Treating the origin as the top would put it at 50..60.
        assert_eq!(bounds.top, 42.0, "{bounds}");
        assert_eq!(bounds.bottom, 52.0, "{bounds}");
    }

    #[test]
    fn bounds_pad_the_trailing_edge_because_the_last_glyphs_width_is_unknown() {
        let run = run(vec![
            Glyph::new(1, Offset::ZERO),
            Glyph::new(2, Offset::new(6.0, 0.0)),
        ]);
        let bounds = run.bounds();

        assert_eq!(bounds.left, 100.0);
        assert!(
            bounds.right >= 100.0 + 6.0,
            "the last glyph must not be clipped at its origin: {bounds}"
        );
    }

    #[test]
    fn advance_spans_the_leftmost_glyph_to_the_rightmost() {
        let run = run(vec![
            Glyph::new(1, Offset::ZERO),
            Glyph::new(2, Offset::new(6.0, 0.0)),
            Glyph::new(3, Offset::new(13.0, 0.0)),
        ]);
        assert_eq!(run.advance(), 13.0);
        assert_eq!(run.line_size(), Size::new(13.0, 10.0));
    }

    #[test]
    fn an_rtl_runs_advance_is_positive_despite_descending_glyph_order() {
        // Glyphs are in logical order, so a right-to-left run runs right to left.
        // Subtracting the first from the last would report -13.
        let mut rtl = run(vec![
            Glyph::new(1, Offset::new(13.0, 0.0)),
            Glyph::new(2, Offset::new(6.0, 0.0)),
            Glyph::new(3, Offset::ZERO),
        ]);
        rtl.is_rtl = true;

        assert_eq!(rtl.advance(), 13.0, "a width is never negative");
        assert_eq!(rtl.line_size(), Size::new(13.0, 10.0));
    }

    #[test]
    fn a_run_carries_its_own_direction_rather_than_the_paragraph_doing_so() {
        let mut rtl = run(vec![Glyph::new(1, Offset::ZERO)]);
        rtl.is_rtl = true;
        assert!(rtl.is_rtl, "a mixed-direction line needs this per run");
    }
}
