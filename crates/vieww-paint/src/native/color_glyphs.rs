//! Colour glyphs: COLRv0 layered outlines and CBDT/sbix bitmap emoji.
//!
//! A monochrome pipeline answers "what does glyph 42 of this font look like"
//! with one outline and one colour, and that answer covered every font the
//! framework could load — until a font answers it with *several* outlines in
//! *several* fixed colours (COLR) or with *pixels* (CBDT, sbix). Noto Color
//! Emoji is the font nearly every device has, and it has no outlines at all:
//! through the monochrome path its glyphs are `NoOutline`, take their advance,
//! and ink nothing. A whole class of modern type renders as
//! correctly-spaced blank.
//!
//! # The two flavours this module resolves
//!
//! **COLRv0** — the simplest colour table, and the one every "layered colour"
//! font uses when it does not need gradients or transforms. A base glyph maps
//! to a *list of layers*, each layer being an ordinary glyph id plus a palette
//! index; painting is painting those outlines bottom-up in their palette
//! colours. `v1` paint graphs (gradients, transforms, compositing modes) are
//! *not* handled here — `skrifa` can traverse them, but the rasterizer would
//! need a full paint interpreter to do it honestly, and this build would
//! rather render v0 exactly than render v1 approximately.
//!
//! **CBDT/sbix bitmaps** — struck pixel images with their own bearings and
//! advances. `skrifa::bitmap` walks the strike tables for both formats and
//! hands back the bytes; CBDT's 32-bit entries are PNG streams, which decode
//! here through the same `png` crate that encodes screenshots. The decoded
//! RGBA is cached per glyph: a strike image is the most expensive thing in
//! this module to produce and the cheapest to reuse.
//!
//! # Where the decision happens
//!
//! [`ColorGlyphCache::resolve`] is asked *before* the monochrome path, per
//! glyph, and answers `None` for every glyph of every font that has neither
//! table — a `HashMap` miss on the font id, which is the whole cost for the
//! entire non-colour world. Only when a font is *in* the map does the per-run
//! work begin.
//!
//! # Variations
//!
//! Variation coordinates apply to COLRv0 layer outlines exactly as they apply
//! to any other outline — the layers are ordinary glyphs of the same variable
//! face, drawn through the same [`super::glyph`] cache keyed by a `FontData`
//! id that already carries them. (COLRv1 carries its own variation machinery;
//! that is one more reason it stays out of scope.)

use std::collections::HashMap;

use vieww_foundation::{Color, FontData, Image, Offset, Rect, Transform};

// `colr()`/`cpal()` on a `FontRef` are `TableProvider` trait methods.
use skrifa::raw::TableProvider;

/// One resolved colour glyph, ready for the renderer's existing compositing
/// paths — no new rasterization primitive is introduced for either flavour.
pub(crate) enum ColorGlyph {
    /// COLRv0: `(glyph id, colour)` pairs, bottom layer first.
    ///
    /// Each layer's glyph goes through the *ordinary* outline → coverage →
    /// `composite_flat` pipeline with the layer's own colour; nothing about a
    /// layer is special except that it is not the run's colour.
    Layers(Vec<(u16, Color)>),
    /// CBDT/sbix: a decoded bitmap, plus where it lands in **run-local**
    /// coordinates.
    ///
    /// `rect` is in the same space a monochrome glyph's outline would be
    /// placed in — baseline at `origin + glyph.offset`, y down — already
    /// scaled from the strike's ppem to the run's size. The renderer draws it
    /// with the image sampler, so rotation, clips and the colour pipeline all
    /// apply exactly as they do to `DrawImage`.
    Bitmap { image: Image, rect: Rect },
}

/// Placement facts about one decoded strike image, in the strike's own pixel
/// units. Stored beside the decoded pixels so the rect arithmetic at draw
/// time has everything it needs without re-walking the strike tables.
#[derive(Debug, Clone, Copy)]
struct StrikeMetrics {
    /// Horizontal bearing from the glyph origin to the bitmap's left edge,
    /// in strike pixels.
    bearing_x: f32,
    /// Vertical bearing from the baseline **up** to the bitmap's top edge,
    /// in strike pixels.
    bearing_y: f32,
    /// The strike's pixels-per-em — what scales strike pixels into em units.
    ppem: f32,
}

/// Parsed colour tables for one font, plus the decoded bitmaps seen so far.
#[derive(Default)]
pub(crate) struct ColorGlyphCache {
    /// Per-font colour tables, inserted for **every** font asked about — an
    /// empty record for the non-colour ones, so an ordinary font is parsed
    /// once and never again, and a colour font's tables are walked once.
    fonts: HashMap<u64, ColorFace>,
    /// Decoded CBDT/sbix images and their placement, keyed by
    /// `(font id, glyph id)`.
    ///
    /// PNG decode is the expensive half of the bitmap path and byte-identical
    /// every time, so it happens once per glyph per renderer lifetime. The
    /// population is bounded by the number of distinct bitmap glyphs a scene
    /// draws — small in practice, and the same deliberate non-bound the
    /// shadow cache carries, for the same reason: evicting a strike image
    /// buys little and complicates everything.
    bitmaps: HashMap<(u64, u16), (Image, StrikeMetrics)>,
    /// How many bitmap images this cache has decoded — counted work, so a
    /// claim that emoji are cached is a number rather than a hope.
    decoded: usize,
}

/// The colour tables of one face, owned.
struct ColorFace {
    /// COLRv0 base-glyph records: `(base glyph id, layers)`, sorted by glyph
    /// id, layers resolved to `(glyph id, colour)` eagerly at parse.
    colr: Vec<(u16, Vec<(u16, Color)>)>,
    /// Whether this face carries CBDT/sbix strikes at all.
    has_strikes: bool,
    /// The face's bytes, kept for strike lookups and PNG decode.
    data: Vec<u8>,
    index: u32,
}

impl ColorGlyphCache {
    /// The colour glyph for `glyph_id`, if this font has one, resolved for a
    /// glyph placed at `run_origin + glyph_offset` at `size` under
    /// `transform`.
    ///
    /// `None` means the monochrome path owns this glyph — which is the answer
    /// for every glyph of every font without colour tables, and for the
    /// cases where the tables exist but have no record for this glyph.
    ///
    /// A glyph whose bitmap failed to decode also answers `None`, so the
    /// monochrome path draws whatever it can (usually a blank advance) rather
    /// than nothing at all.
    pub(crate) fn resolve(
        &mut self,
        font: &FontData,
        glyph_id: u16,
        run_origin: Offset,
        glyph_offset: Offset,
        size: f32,
        transform: Transform,
    ) -> Option<ColorGlyph> {
        let id = font.id();
        let face = self
            .fonts
            .entry(id)
            .or_insert_with(|| ColorFace::parse(font));

        // COLRv0 first: a layered record takes precedence over a bitmap of
        // the same glyph (the spec allows both; layered is the richer answer).
        if let Ok(index) = face.colr.binary_search_by_key(&glyph_id, |e| e.0) {
            let layers = face.colr[index].1.clone();
            return Some(ColorGlyph::Layers(layers));
        }

        if !face.has_strikes {
            return None;
        }

        // **Strike selection is by run size in device pixels, not em size.**
        // A transform can scale a 16px run onto a 3× display, and the strike
        // that is right for one is starved for the other. The transform's
        // scale factor — the geometric mean of its axis scales, the same
        // measure `native/shadow.rs` uses for σ under anisotropic maps — is
        // what reconciles them.
        let scale = (transform.a * transform.d - transform.b * transform.c)
            .abs()
            .sqrt()
            .max(0.0);
        let device_ppem = (size * scale).max(1.0);

        let key = (id, glyph_id);
        let entry = self.bitmaps.entry(key).or_insert_with(|| {
            let decoded = decode_strike(&face.data, face.index, glyph_id, device_ppem)
                .unwrap_or_else(|| {
                    (
                        Image::from_rgba8(vec![0, 0, 0, 0], 1, 1),
                        StrikeMetrics {
                            bearing_x: 0.0,
                            bearing_y: 0.0,
                            ppem: 1.0,
                        },
                    )
                });
            self.decoded += 1;
            decoded
        });
        if entry.0.width() < 2 {
            // The sentinel inserted when decode failed — treat it as "no
            // colour answer" rather than drawing a stray opaque pixel.
            return None;
        }

        let (image, metrics) = entry.clone();
        // Strike pixels → run-local units. `ppem` is the strike's own;
        // `size` is the run's em size in the same units.
        let pixel_scale = size / metrics.ppem.max(1.0);
        let left = run_origin.dx + glyph_offset.dx + metrics.bearing_x * pixel_scale;
        let top = run_origin.dy + glyph_offset.dy - metrics.bearing_y * pixel_scale;
        let width = image.width() as f32 * pixel_scale;
        let height = image.height() as f32 * pixel_scale;
        Some(ColorGlyph::Bitmap {
            image,
            rect: Rect::new(left, top, left + width, top + height),
        })
    }

    /// How many strike images this cache has decoded — the "emoji are
    /// cached" number.
    pub(crate) fn decoded_bitmaps(&self) -> usize {
        self.decoded
    }

    /// Drop every parsed table and decoded image.
    pub(crate) fn clear(&mut self) {
        self.fonts.clear();
        self.bitmaps.clear();
    }
}

impl ColorFace {
    fn parse(font: &FontData) -> Self {
        let data = font.bytes().to_vec();
        let index = font.index();
        let colr = parse_colr_v0(&data, index);
        let has_strikes = has_bitmap_strikes(&data, index);
        Self {
            colr,
            has_strikes,
            data,
            index,
        }
    }
}

/// Read COLRv0 base-glyph records, resolving palette indices against CPAL
/// palette 0.
///
/// Eager rather than lazy on purpose: the records are a few bytes per base
/// glyph, the palette a few dozen more, and resolving them once keeps the
/// per-glyph question a binary search with no table parsing on the hot path.
fn parse_colr_v0(data: &[u8], index: u32) -> Vec<(u16, Vec<(u16, Color)>)> {
    let Ok(font) = skrifa::FontRef::from_index(data, index) else {
        return Vec::new();
    };
    let Ok(colr) = font.colr() else {
        return Vec::new();
    };
    // `layer_records` is v0-only; a v1 font carrying a LayerList instead
    // answers `None` here and colour falls back to the bitmap path below.
    let (Some(Ok(bases)), Some(Ok(layers)), Ok(cpal)) =
        (colr.base_glyph_records(), colr.layer_records(), font.cpal())
    else {
        return Vec::new();
    };
    let palette = cpal_palette(&cpal, 0);

    let mut records = Vec::with_capacity(bases.len());
    for base in bases {
        let first = usize::from(base.first_layer_index());
        let count = usize::from(base.num_layers());
        let mut resolved = Vec::with_capacity(count);
        for layer in layers.iter().skip(first).take(count) {
            // A palette index past the palette's length is malformed font
            // data; magenta is the classic "you can see it is wrong" answer
            // rather than transparent, which would read as a missing layer.
            let color = palette
                .get(usize::from(layer.palette_index()))
                .copied()
                .unwrap_or(Color::rgb(255, 0, 255));
            resolved.push((layer.glyph_id().to_u16(), color));
        }
        if !resolved.is_empty() {
            records.push((base.glyph_id().to_u16(), resolved));
        }
    }
    records.sort_by_key(|e| e.0);
    records
}

/// CPAL palette `i` as straight RGBA colours.
fn cpal_palette(cpal: &skrifa::raw::tables::cpal::Cpal<'_>, i: usize) -> Vec<Color> {
    let entries = usize::from(cpal.num_palette_entries());
    let Some(Ok(records)) = cpal.color_records_array() else {
        return Vec::new();
    };
    let indices = cpal.color_record_indices();
    let start = indices.get(i).map_or(0, |v| usize::from(v.get()));
    let mut colors = Vec::with_capacity(entries);
    for entry in 0..entries {
        match records.get(start + entry) {
            Some(record) => colors.push(Color::rgba(
                record.red(),
                record.green(),
                record.blue(),
                record.alpha(),
            )),
            None => colors.push(Color::TRANSPARENT),
        }
    }
    colors
}

/// Whether this face carries bitmap strikes (CBDT or sbix) at all.
fn has_bitmap_strikes(data: &[u8], index: u32) -> bool {
    let Ok(font) = skrifa::FontRef::from_index(data, index) else {
        return false;
    };
    !skrifa::bitmap::BitmapStrikes::new(&font).is_empty()
}

/// Find the best strike for `ppem` and decode glyph `glyph_id` out of it,
/// with the placement facts needed to draw it.
fn decode_strike(
    data: &[u8],
    index: u32,
    glyph_id: u16,
    ppem: f32,
) -> Option<(Image, StrikeMetrics)> {
    let font = skrifa::FontRef::from_index(data, index).ok()?;
    let strikes = skrifa::bitmap::BitmapStrikes::new(&font);
    let glyph = strikes.glyph_for_size(
        skrifa::instance::Size::new(ppem),
        skrifa::raw::types::GlyphId::new(u32::from(glyph_id)),
    )?;
    let width = glyph.width;
    let height = glyph.height;
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = match &glyph.data {
        skrifa::bitmap::BitmapData::Png(bytes) => decode_png(bytes)?,
        skrifa::bitmap::BitmapData::Bgra(bytes) => bgra_to_rgba(bytes, width, height),
        skrifa::bitmap::BitmapData::Mask(_) => return None,
    };
    Some((
        Image::from_rgba8(pixels, width, height),
        StrikeMetrics {
            bearing_x: glyph.inner_bearing_x,
            bearing_y: glyph.inner_bearing_y,
            ppem: glyph.ppem_y.max(1.0),
        },
    ))
}

/// Decode a PNG stream from a CBDT strike into straight RGBA8.
fn decode_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Expand every less-than-8-bit and palette flavour to 8-bit colour so the
    // cache holds one pixel layout regardless of what the font shipped.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0_u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    match info.color_type {
        png::ColorType::Rgba => Some(buffer),
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(buffer.len() / 3 * 4);
            for pixel in buffer.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            Some(rgba)
        }
        // `normalize_to_color8` should have expanded grayscale and indexed
        // flavours to RGB(A) already; the arm exists so a surprise is drawn
        // honestly rather than skipped.
        png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha | png::ColorType::Indexed => {
            let has_alpha = matches!(info.color_type, png::ColorType::GrayscaleAlpha);
            let stride = if has_alpha { 2 } else { 1 };
            let mut rgba = Vec::with_capacity(buffer.len() / stride * 4);
            for pixel in buffer.chunks_exact(stride) {
                let v = pixel[0];
                let a = if has_alpha { pixel[1] } else { 255 };
                rgba.extend_from_slice(&[v, v, v, a]);
            }
            Some(rgba)
        }
    }
}

/// CBDT's 32-bit byte-aligned entries are BGRA **premultiplied** — straighten
/// both at once.
fn bgra_to_rgba(bytes: &[u8], width: u32, height: u32) -> Vec<u8> {
    let count = (width * height) as usize;
    let mut out = Vec::with_capacity(count * 4);
    for pixel in bytes.chunks_exact(4) {
        let (b, g, r, a) = (pixel[0], pixel[1], pixel[2], pixel[3]);
        if a == 0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            out.extend_from_slice(&[r, g, b, 255]);
        } else {
            let un = |c: u8, a: u8| {
                // `f32::from(u8)` is exact, and the clamp keeps the `as u8`
                // inside the type's range, so neither cast lint applies.
                let v = f32::from(c) * 255.0 / f32::from(a);
                v.clamp(0.0, 255.0) as u8
            };
            out.extend_from_slice(&[un(r, a), un(g, a), un(b, a), a]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_premultiplied_entry_is_straightened() {
        // 50% opaque pure red, premultiplied: B=0 G=0 R=128 A=128, followed
        // by a fully transparent entry.
        let out = bgra_to_rgba(&[0, 0, 128, 128, 0, 0, 0, 0], 1, 2);
        assert_eq!(out[0], 255, "red un-premultiplies to full");
        assert_eq!(out[3], 128, "alpha is carried, not scaled");
        assert_eq!(&out[4..8], &[0, 0, 0, 0], "transparent stays transparent");
    }

    #[test]
    fn a_static_font_answers_none() {
        // The embedded DejaVu subset has no colour tables: the first question
        // about it parses once, inserts an empty record, and every question —
        // including this first one — answers `None`, whatever the glyph id.
        let data = vieww_foundation::FontData::new(
            std::rc::Rc::new(vieww_text::EMBEDDED_FONT.to_vec()),
            0,
        );
        let mut cache = ColorGlyphCache::default();
        assert!(cache
            .resolve(
                &data,
                36,
                Offset::ZERO,
                Offset::ZERO,
                16.0,
                Transform::IDENTITY
            )
            .is_none());
    }
}
