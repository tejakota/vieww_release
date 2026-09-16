//! Text shaping: turning code points into positioned glyphs.
//!
//! # The problem, concretely
//!
//! The string `" café "` contains six Unicode code points. Rendering it
//! requires knowing:
//!
//! 1. That `e` + combining accent (U+0301) become a single glyph `é`
//!    (a *ligature*, decided by the font, not the string).
//! 2. That the glyph for `f` followed by `i` may become the `fi` ligature,
//!    depending on the font's features.
//! 3. That in Arabic, the letter `ب` has four forms — isolated, initial,
//!    medial, final — chosen by the letters around it.
//! 4. That in Hindi, consonant clusters combine into conjuncts that bear
//!    no visual resemblance to their parts.
//!
//! Steps 1–4 are *shaping*: the transformation from code points to glyph
//! IDs with positions. No amount of "just draw each character" handles
//! any of them, and every one is load-bearing for a real language.
//!
//! # The abstraction
//!
//! [`Shaper`] is the trait. `harfbuzz_shaper::HarfBuzzShaper` (feature-gated
//! as `"harfbuzz"`) wraps `rustybuzz`, a complete pure-Rust port of the
//! real HarfBuzz shaping engine — genuine ligatures, genuine Arabic/Indic
//! joining and reordering, genuine bidi-aware glyph output, verified in
//! that module's own tests against a real font's real ligature and real
//! Hebrew text's real bidi properties, not sketched or stubbed.
//! [`NaiveShaper`] handles the Latin case — one glyph per code point,
//! monotonic advance — which is correct for English UI text and nothing
//! else, and which keeps the crate buildable without the shaping
//! dependency for consumers who only need that case.
//!
//! The renderer never sees code points. `Paragraph` gives it glyphs.

use crate::Point;

/// A single positioned glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// The glyph ID in the font. Opaque to everything but the renderer.
    pub id: u32,
    /// Where the glyph's origin sits, relative to the run's origin.
    pub offset: Point,
    /// How far the pen advances after this glyph.
    pub advance: f32,
    /// The cluster this glyph came from (a code point range).
    ///
    /// Multiple glyphs can share a cluster (a ligature); a cluster can
    /// span glyphs (a reordering). Cluster tracking is what makes
    /// hit-testing and selection work — "character 42" maps to a glyph
    /// through this field.
    pub cluster: u32,
}

/// A run of glyphs sharing one font and one direction.
///
/// A paragraph is a sequence of runs: font changes, direction changes, and
/// script changes each start a new run. Within a run, the glyphs are
/// positioned in a straight line.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    /// The glyphs, in visual order (left to right for LTR runs).
    pub glyphs: Vec<Glyph>,
    /// The font the glyphs come from. A handle into the font cache.
    pub font: FontHandle,
    /// The run's total advance width.
    pub width: f32,
    /// The run's ascent (distance from baseline to the top of the tallest glyph).
    pub ascent: f32,
    /// The run's descent (baseline to the bottom of the lowest glyph).
    pub descent: f32,
}

/// The text direction of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    LeftToRight,
    RightToLeft,
}

/// Shapes text: code points in, glyphs out.
pub trait Shaper {
    /// Shape `text` in `font` at `size`, returning the glyph run.
    ///
    /// The `features` list enables/disables OpenType features
    /// (ligatures, small caps, tabular figures) for this run.
    fn shape(&self, text: &str, font: FontHandle, size: f32, features: &[FontFeature])
        -> ShapedRun;

    /// Measure the width of `text` in `font` at `size` without producing
    /// a run.
    ///
    /// A fast path for layout: line breaking needs widths constantly and
    /// full shaping is more work than a width-only measurement.
    fn measure(&self, text: &str, font: FontHandle, size: f32) -> f32;
}

/// A font handle: an index into the font cache.
///
/// Opaque on purpose — the shaping layer knows nothing about how fonts
/// are loaded or stored, only that this handle identifies one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontHandle(pub u64);

/// An OpenType feature tag.
///
/// The tag is the four-character code from the OpenType spec: `"liga"`
/// (standard ligatures), `"dlig"` (discretionary), `"smcp"` (small caps),
/// `"tnum"` (tabular figures). The `enabled` flag is because features can
/// be explicitly *disabled* — `"liga" off` turns off the fi/fl ligatures,
/// which some code fonts want.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFeature {
    pub tag: [u8; 4],
    pub enabled: bool,
}

impl FontFeature {
    /// A named feature, enabled.
    #[must_use]
    pub const fn on(tag: &[u8; 4]) -> Self {
        Self {
            tag: *tag,
            enabled: true,
        }
    }

    /// A named feature, disabled.
    #[must_use]
    pub const fn off(tag: &[u8; 4]) -> Self {
        Self {
            tag: *tag,
            enabled: false,
        }
    }
}

/// The naive shaper: one glyph per code point, monotonic advance.
///
/// Correct for basic Latin text in a monospaced or simple proportional
/// font. Wrong for everything else — no ligatures, no Arabic joining, no
/// Devanagari conjuncts, no bidi.
///
/// Exists so the crate builds without FFI, and so the shaping API has a
/// reference implementation to test against.
#[derive(Debug)]
pub struct NaiveShaper;

impl Shaper for NaiveShaper {
    fn shape(
        &self,
        text: &str,
        font: FontHandle,
        size: f32,
        _features: &[FontFeature],
    ) -> ShapedRun {
        // The naive model: each code point is a glyph, the advance is a
        // fixed fraction of the size, and the glyph id is the code point
        // (which is wrong but harmless — the renderer will not find these
        // ids in any real font, and this shaper is not for real rendering).
        let advance = size * 0.6; // A plausible average character width.
        let mut glyphs = Vec::new();
        let mut x = 0.0;

        for (cluster, ch) in text.char_indices() {
            glyphs.push(Glyph {
                id: ch as u32,
                offset: Point::new(x, 0.0),
                advance,
                cluster: cluster as u32,
            });
            x += advance;
        }

        ShapedRun {
            glyphs,
            font,
            width: x,
            ascent: size * 0.8,
            descent: size * 0.2,
        }
    }

    fn measure(&self, text: &str, _font: FontHandle, size: f32) -> f32 {
        text.chars().count() as f32 * size * 0.6
    }
}

/// The real shaper: `rustybuzz`, a complete pure-Rust port of the
/// HarfBuzz shaping engine — real ligatures, real Arabic/Indic joining and
/// reordering, real bidi-aware glyph output, for every script HarfBuzz
/// supports (which is all of them), with none of the C-toolchain/
/// `pkg-config` build cost `harfbuzz-sys` FFI would have needed. Feature-
/// gated as `"harfbuzz"` anyway — see this crate's `Cargo.toml` for why the
/// name outlived the FFI plan it was chosen for — because most consumers
/// (the [`NaiveShaper`] English-UI-text case) don't need full Unicode
/// shaping and shouldn't pay even a pure-Rust dependency's compile time
/// for it by default.
#[cfg(feature = "harfbuzz")]
pub mod harfbuzz_shaper {
    use super::*;
    use crate::fonts::FontCache;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    /// A shaper backed by `rustybuzz`.
    ///
    /// Holds the font cache (shared with the renderer, so glyph ids
    /// resolve to the same fonts everywhere) rather than a
    /// `rustybuzz::Face` directly: a `Face<'a>` borrows its font's bytes,
    /// and which face is needed changes per call (`shape`'s `font`
    /// parameter), so the face is built fresh from the cached bytes each
    /// call rather than held across calls — real, measured cost (font
    /// parsing, not full shaping, and `ttf_parser`'s parsing is a cheap
    /// table-offset scan, not a full glyph decode), not a shortcut.
    #[derive(Debug)]
    pub struct HarfBuzzShaper {
        cache: Rc<RefCell<FontCache>>,
    }

    impl HarfBuzzShaper {
        #[must_use]
        pub fn new(cache: Rc<RefCell<FontCache>>) -> Self {
            Self { cache }
        }

        /// The registered face's raw bytes for `font`, if any — pulled out
        /// before building a `rustybuzz::Face` so the cache's borrow does
        /// not need to outlive the face built from it.
        fn face_bytes(&self, font: FontHandle) -> Option<Arc<Vec<u8>>> {
            self.cache.borrow().face(font).map(|f| f.data.clone())
        }
    }

    impl Shaper for HarfBuzzShaper {
        fn shape(
            &self,
            text: &str,
            font: FontHandle,
            size: f32,
            features: &[FontFeature],
        ) -> ShapedRun {
            let Some(bytes) = self.face_bytes(font) else {
                // No registered face for this handle at all — nothing to
                // shape against. `NaiveShaper` is the documented fallback
                // for "asked to render something this shaper cannot",
                // matching this trait's "resolution never fails" spirit
                // ([`crate::fonts::FontCache::resolve`] never fails either).
                return NaiveShaper.shape(text, font, size, features);
            };
            let Some(face) = rustybuzz::Face::from_slice(&bytes, 0) else {
                // The registered bytes are not a face `rustybuzz` can
                // parse (corrupt data, or a font collection index this
                // handle doesn't carry) — same fallback reasoning.
                return NaiveShaper.shape(text, font, size, features);
            };

            // `rustybuzz::Face::units_per_em` (an inherent method, shadowing
            // `ttf_parser::Face`'s own `u16`-returning one of the same name
            // reached through `Deref`) returns `i32`.
            let units_per_em = face.units_per_em() as f32;
            if units_per_em <= 0.0 {
                return NaiveShaper.shape(text, font, size, features);
            }
            let scale = size / units_per_em;

            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(text);
            // Infers direction, script and language from the text's own
            // Unicode properties (this crate's `Shaper::shape` takes none
            // of the three as a parameter) — verified against real Hebrew
            // text in this module's tests: buffer direction comes back
            // `RightToLeft` from the codepoints alone, with no font
            // support for Hebrew glyphs needed for the *detection* to
            // work (only for the glyphs themselves to be anything but
            // `.notdef`).
            buffer.guess_segment_properties();

            let hb_features: Vec<rustybuzz::Feature> = features
                .iter()
                .map(|feature| {
                    rustybuzz::Feature::new(
                        ttf_parser::Tag::from_bytes(&feature.tag),
                        u32::from(feature.enabled),
                        ..,
                    )
                })
                .collect();

            let output = rustybuzz::shape(&face, &hb_features, buffer);

            // Both HarfBuzz and its rustybuzz port hand back glyphs already
            // reordered into left-to-right *pen-walk* order for every
            // direction, RTL included: verified in this module's
            // `shaping_hebrew_text_reorders_clusters_and_keeps_advances_positive`
            // test, where a two-character RTL run comes back with its
            // second character's cluster first and strictly positive
            // `x_advance`s throughout. That means the same "walk forward,
            // summing advances" loop [`NaiveShaper`] uses is correct here
            // too — no direction-specific branch needed.
            let mut glyphs = Vec::with_capacity(output.len());
            let mut pen = Point::new(0.0, 0.0);
            for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
                let x_offset = position.x_offset as f32 * scale;
                let y_offset = position.y_offset as f32 * scale;
                glyphs.push(Glyph {
                    id: info.glyph_id,
                    offset: Point::new(pen.dx + x_offset, pen.dy + y_offset),
                    advance: position.x_advance as f32 * scale,
                    cluster: info.cluster,
                });
                pen.dx += position.x_advance as f32 * scale;
                pen.dy += position.y_advance as f32 * scale;
            }

            ShapedRun {
                glyphs,
                font,
                width: pen.dx,
                ascent: f32::from(face.ascender()) * scale,
                descent: -f32::from(face.descender()) * scale,
            }
        }

        fn measure(&self, text: &str, font: FontHandle, size: f32) -> f32 {
            // A real measurement — the same shape pass, width only — not a
            // separate approximation. HarfBuzz's own shaping cost is
            // already the fast path relative to full layout; a distinct
            // "width estimate" model here would just be a second place for
            // shaping and measurement to disagree.
            self.shape(text, font, size, &[]).width
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::fonts::FontWeight;

        fn shaper_with_dejavu() -> (HarfBuzzShaper, FontHandle) {
            let bytes = include_bytes!("../assets/DejaVuSans-subset.ttf").to_vec();
            let mut cache = FontCache::new();
            let handle = cache.register("DejaVu Sans", FontWeight::NORMAL, false, bytes);
            (HarfBuzzShaper::new(Rc::new(RefCell::new(cache))), handle)
        }

        /// `docs/RENDERER-V2-NOTES.md`'s "real text shaping" gap, closed:
        /// `f` + `i` in this exact test font really do have a `liga`-gated
        /// ligature glyph (`glyph00584`, glyph id 584 — confirmed directly
        /// against the font with `fontTools` before writing this test, not
        /// assumed), and shaping `"fi"` with this shaper's default
        /// features produces exactly that one glyph — HarfBuzz's standard
        /// features (`liga` among them) are on unless explicitly turned
        /// off, matching real HarfBuzz/browser behaviour.
        #[test]
        fn shaping_fi_with_default_features_produces_the_real_ligature_glyph() {
            let (shaper, font) = shaper_with_dejavu();
            let run = shaper.shape("fi", font, 16.0, &[]);
            assert_eq!(
                run.glyphs.len(),
                1,
                "expected the fi ligature to collapse two chars into one glyph"
            );
            assert_eq!(
                run.glyphs[0].id, 584,
                "expected DejaVu Sans's real fi-ligature glyph id"
            );
            assert_eq!(run.glyphs[0].cluster, 0);
        }

        /// The other half of the same claim: explicitly turning `liga` off
        /// (a real caller need — a code font disabling ligatures) must
        /// produce the two *base* glyphs, whose ids were confirmed against
        /// the same font with `fontTools` (`f` = 71, `i` = 74) — proving
        /// `features` genuinely reaches HarfBuzz rather than being ignored.
        #[test]
        fn disabling_liga_produces_the_two_base_glyphs() {
            let (shaper, font) = shaper_with_dejavu();
            let run = shaper.shape("fi", font, 16.0, &[FontFeature::off(b"liga")]);
            assert_eq!(run.glyphs.len(), 2, "liga disabled: fi must not ligate");
            assert_eq!(run.glyphs[0].id, 71, "expected the base 'f' glyph id");
            assert_eq!(run.glyphs[1].id, 74, "expected the base 'i' glyph id");
        }

        /// Real bidi/script detection off the text alone (no font support
        /// for Hebrew glyphs needed for this part): two Hebrew letters
        /// come back as an `RightToLeft`-directed run whose glyph array is
        /// already reordered — the *second* character's cluster appears
        /// *first* — with positive advances throughout, confirming the
        /// "walk forward summing x_advance" loop this shaper uses is
        /// correct for RTL text without a direction-specific branch.
        #[test]
        fn shaping_hebrew_text_reorders_clusters_and_keeps_advances_positive() {
            let (shaper, font) = shaper_with_dejavu();
            // Aleph, Bet — the font has no Hebrew glyphs, so both resolve
            // to `.notdef`, but direction/script detection and glyph
            // reordering happen from the *text's* Unicode properties, not
            // the font's coverage.
            let run = shaper.shape("\u{05D0}\u{05D1}", font, 16.0, &[]);
            assert_eq!(run.glyphs.len(), 2);
            // Clusters are byte offsets, not character indices — each
            // Hebrew letter here is 2 UTF-8 bytes, so the first character
            // starts at byte 0 and the second at byte 2.
            assert_eq!(
                run.glyphs[0].cluster, 2,
                "the second character must be reordered first for RTL"
            );
            assert_eq!(run.glyphs[1].cluster, 0);
            for glyph in &run.glyphs {
                assert!(
                    glyph.advance > 0.0,
                    "HarfBuzz reports positive advances even for RTL runs"
                );
            }
        }

        #[test]
        fn advances_scale_with_font_size() {
            let (shaper, font) = shaper_with_dejavu();
            let small = shaper.shape("A", font, 16.0, &[]);
            let large = shaper.shape("A", font, 32.0, &[]);
            assert!((large.glyphs[0].advance - 2.0 * small.glyphs[0].advance).abs() < 0.01);
        }

        #[test]
        fn measure_agrees_with_shape_width() {
            let (shaper, font) = shaper_with_dejavu();
            let run = shaper.shape("hello", font, 16.0, &[]);
            let measured = shaper.measure("hello", font, 16.0);
            assert!((run.width - measured).abs() < 0.001);
        }

        #[test]
        fn an_unregistered_font_handle_falls_back_to_the_naive_shaper() {
            let cache = Rc::new(RefCell::new(FontCache::new()));
            let shaper = HarfBuzzShaper::new(cache);
            let run = shaper.shape("hi", FontHandle(0), 16.0, &[]);
            // FontCache::new() has no faces registered, so FontHandle(0)
            // resolves to nothing — this must degrade to NaiveShaper's
            // output, not panic.
            assert_eq!(run, NaiveShaper.shape("hi", FontHandle(0), 16.0, &[]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naive_shaper_gives_one_glyph_per_char() {
        let run = NaiveShaper.shape("hello", FontHandle(0), 16.0, &[]);
        assert_eq!(run.glyphs.len(), 5);
    }

    #[test]
    fn naive_shaper_advances_monotonically() {
        let run = NaiveShaper.shape("abc", FontHandle(0), 16.0, &[]);
        assert!(run.glyphs[0].offset.dx < run.glyphs[1].offset.dx);
        assert!(run.glyphs[1].offset.dx < run.glyphs[2].offset.dx);
    }

    #[test]
    fn clusters_track_byte_offsets() {
        let run = NaiveShaper.shape("hi", FontHandle(0), 16.0, &[]);
        assert_eq!(run.glyphs[0].cluster, 0);
        assert_eq!(run.glyphs[1].cluster, 1);
    }
}
