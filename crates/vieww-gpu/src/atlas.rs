//! A glyph atlas: many glyphs' coverage in one texture, so a whole screen of
//! text is one draw call.
//!
//! # Why an atlas at all
//!
//! A glyph is a small alpha bitmap. Uploading one texture per glyph would mean
//! a texture bind — and therefore a draw call — per glyph, and a line of code
//! in `viewwstudio`'s editor is a few hundred glyphs. The entire reason
//! [`ScenePlan`](crate::ScenePlan) batches is that nothing about the pipeline
//! changes between one shape and the next; a per-glyph texture would undo that
//! for the most common content on the screen.
//!
//! So every glyph goes into one texture, and the quad that draws it carries
//! the texture coordinates of its patch. Text then batches exactly like
//! everything else, and a screen of prose is one `draw_indexed`.
//!
//! # The white texel, and why solid fills sample it
//!
//! This atlas reserves texel `(0, 0)` and fills it with full coverage. Solid
//! geometry — every rectangle, path and stroke in the frame — is emitted with
//! all four of its vertices' UVs pointing at that texel.
//!
//! That is what lets **one** pipeline and **one** shader draw both. The
//! fragment shader is `color.a *= atlas(uv)` unconditionally: a glyph gets its
//! coverage, and a rectangle gets `1.0` and comes out exactly as it did before
//! this module existed. The alternative — two pipelines, or a branch on a
//! per-vertex "is this text" flag — costs a pipeline bind between a panel and
//! the label on it, which is to say between almost every pair of adjacent
//! draws in a real interface.
//!
//! A shelf allocator never reuses `(0, 0)` for a glyph because the first shelf
//! starts past it; see [`Atlas::new`].
//!
//! # Shelf packing, and why not something cleverer
//!
//! Glyphs are packed into horizontal shelves: a row of a fixed height, filled
//! left to right, and a new shelf opened above when the current one runs out.
//! It is the classic choice for glyph atlases (FreeType's own demo packer,
//! Skia's, and every text renderer that has published its packer) for a reason
//! that is specific to this content: glyphs from one run are nearly the same
//! height, so a shelf wastes very little, and the packer is O(1) per insert
//! with no bookkeeping to get wrong.
//!
//! Skyline or MaxRects packing wins on heterogeneous rectangles. Text is not
//! heterogeneous, and the cost of being wrong here is not wasted memory, it is
//! a subtle mispacking that puts one glyph's rim inside another's patch — a
//! bug that looks like a font rendering artefact and is not one.
//!
//! # Growth, and what happens when it stops
//!
//! The texture starts small and doubles when a glyph will not fit, up to
//! [`Atlas::MAX_SIDE`]. Doubling reflows every existing entry, which is why
//! [`Atlas::version`] exists: a backend re-uploads when the version changes
//! rather than trying to track individual texels.
//!
//! Past the maximum, [`Atlas::insert`] returns `None` and the planner records
//! the glyph as unsupported — the same honest gap every other unplannable
//! command gets. It does not evict, and that is deliberate for now: evicting
//! mid-frame would invalidate UVs already written into this frame's vertex
//! buffer, so a frame that genuinely needs more than 4096x4096 of distinct
//! glyph coverage belongs on the CPU rasterizer rather than on a GPU path
//! quietly drawing some of its text with stale coordinates.

use std::collections::HashMap;

/// Everything about a rasterised glyph that decides which texels it owns.
///
/// The same identity `vieww_paint`'s glyph raster cache keys on — font, id,
/// size, transform and sub-pixel phase — because two glyphs that share it are
/// the same bitmap and must share one patch, and two that do not are different
/// pictures and must not.
///
/// Carried as raw bits rather than floats so the key can be `Hash` and `Eq`,
/// and because bit equality is the equality actually wanted: two transforms
/// differing in the last bit can rasterise differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AtlasKey {
    pub font: u64,
    pub glyph: u16,
    pub size: u32,
    pub transform: [u32; 4],
    pub phase: (u32, u32),
}

/// Where one glyph lives in the atlas.
///
/// UVs are normalised `0..1` and name the patch's **edges**, not its texel
/// centres. The shader samples with nearest filtering and the quad is placed at
/// exactly the bitmap's device size, so each fragment lands on exactly one
/// texel and there is no filtering error to reason about — which is what makes
/// pixel-for-pixel parity with the CPU rasterizer achievable rather than
/// approximate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasSlot {
    /// Texel top-left inside the atlas. Stable for the atlas's lifetime.
    pub x: u32,
    pub y: u32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    /// Patch width in texels, which is also the quad's width in device pixels.
    pub width: u32,
    pub height: u32,
}

/// A growable single-channel coverage atlas.
pub struct Atlas {
    side: u32,
    texels: Vec<u8>,
    entries: HashMap<AtlasKey, Placement>,
    /// Where the next glyph goes on the current shelf.
    pen_x: u32,
    /// The current shelf's top row.
    shelf_y: u32,
    /// The tallest glyph on the current shelf — how far up the next shelf goes.
    shelf_height: u32,
    version: u64,
}

/// A packed glyph's texel rectangle, kept so a resize can reflow it.
#[derive(Debug, Clone, Copy)]
struct Placement {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl std::fmt::Debug for Atlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Atlas")
            .field("side", &self.side)
            .field("glyphs", &self.entries.len())
            .field("version", &self.version)
            .finish()
    }
}

impl Default for Atlas {
    fn default() -> Self {
        Self::new()
    }
}

impl Atlas {
    /// The side the atlas starts at.
    ///
    /// 256x256 is 64 KiB and holds a few hundred small glyphs — a whole
    /// interface's chrome, at one size, without a single resize. Starting
    /// larger wastes upload bandwidth on the common case of an application
    /// that draws one font at two sizes.
    pub const INITIAL_SIDE: u32 = 256;

    /// The largest atlas this will grow to, per axis.
    ///
    /// 4096 is the `maxImageDimension2D` floor every Vulkan implementation
    /// must meet and the same floor D3D11 feature level 11 and Metal family 1
    /// guarantee, so an atlas that fits here fits everywhere this framework
    /// targets. A backend with a larger limit gains nothing: past this, the
    /// working set is not glyph coverage, it is a leak.
    pub const MAX_SIDE: u32 = 4096;

    /// Padding, in texels, between packed glyphs.
    ///
    /// One texel. With nearest sampling and an exact 1:1 quad this is not
    /// strictly required — no fragment can reach a neighbour's texels — but it
    /// costs almost nothing and it is the difference between "a mispacking is
    /// invisible until someone enables filtering" and "a mispacking is
    /// impossible". The white texel at the origin is protected by the same
    /// gap.
    const PAD: u32 = 1;

    #[must_use]
    pub fn new() -> Self {
        let side = Self::INITIAL_SIDE;
        let mut atlas = Self {
            side,
            texels: vec![0; (side * side) as usize],
            entries: HashMap::new(),
            pen_x: 0,
            shelf_y: 0,
            shelf_height: 0,
            version: crate::generation::next(),
        };
        atlas.write_white_texel();
        // The first shelf starts below the white texel's row, so no glyph can
        // ever be packed over it.
        atlas.shelf_y = 1 + Self::PAD;
        atlas
    }

    /// Full coverage at `(0, 0)` — see the module doc.
    fn write_white_texel(&mut self) {
        self.texels[0] = u8::MAX;
    }

    /// The UV of the reserved full-coverage texel, for solid geometry.
    ///
    /// Its centre, not its corner: with nearest filtering a coordinate exactly
    /// on a texel boundary is a tie, and which side of it the hardware picks is
    /// not something this code should be relying on.
    #[must_use]
    pub fn white_uv(&self) -> [f32; 2] {
        let half = 0.5 / self.side as f32;
        [half, half]
    }

    /// The atlas texture's side, in texels. Always square.
    #[must_use]
    pub const fn side(&self) -> u32 {
        self.side
    }

    /// The coverage texels, row-major, one byte each — an `R8_UNORM` upload.
    #[must_use]
    pub fn texels(&self) -> &[u8] {
        &self.texels
    }

    /// Bumped whenever a texel or the size changed.
    ///
    /// A backend keeps the version it last uploaded and re-uploads when this
    /// differs. Cheaper schemes exist (dirty rectangles, a ring of staging
    /// buffers) and none of them is worth anything until a profile says the
    /// upload is a cost: a full 4096x4096 atlas is 16 MB, and a *steady* frame
    /// re-uploads nothing at all because no new glyph was rasterised.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// How many distinct glyphs are packed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Where `key` lives, if it is already packed.
    #[must_use]
    pub fn get(&self, key: &AtlasKey) -> Option<AtlasSlot> {
        self.entries.get(key).map(|p| self.slot(*p))
    }

    /// Pack `alpha` — `width * height` coverage bytes, row-major — under
    /// `key`, or return the slot it already occupies.
    ///
    /// `None` when the glyph cannot be packed even after growing to
    /// [`MAX_SIDE`](Self::MAX_SIDE); see the module doc for why that is
    /// reported rather than resolved by eviction.
    pub fn insert(
        &mut self,
        key: AtlasKey,
        width: u32,
        height: u32,
        alpha: &[u8],
    ) -> Option<AtlasSlot> {
        if let Some(existing) = self.entries.get(&key) {
            return Some(self.slot(*existing));
        }
        if width == 0 || height == 0 {
            return None;
        }
        debug_assert_eq!(
            alpha.len(),
            (width as usize) * (height as usize),
            "coverage buffer must be exactly width * height"
        );

        let placement = self.allocate(width, height)?;
        self.blit(placement, alpha);
        self.entries.insert(key, placement);
        self.version = crate::generation::next();
        Some(self.slot(placement))
    }

    /// Find room for `width` x `height`, growing if the current side cannot
    /// hold it.
    fn allocate(&mut self, width: u32, height: u32) -> Option<Placement> {
        loop {
            if let Some(placement) = self.try_allocate(width, height) {
                return Some(placement);
            }
            // A single glyph larger than the maximum atlas is not a packing
            // failure that growing can fix, and the loop must not spin on it.
            if width > Self::MAX_SIDE || height > Self::MAX_SIDE || self.side >= Self::MAX_SIDE {
                return None;
            }
            self.grow();
        }
    }

    /// One shelf-packing attempt at the current size.
    fn try_allocate(&mut self, width: u32, height: u32) -> Option<Placement> {
        if width > self.side {
            return None;
        }
        // Does it fit on the current shelf?
        if self.pen_x + width > self.side {
            // No — open a new one above.
            let next_y = self.shelf_y + self.shelf_height + Self::PAD;
            if next_y + height > self.side {
                return None;
            }
            self.shelf_y = next_y;
            self.pen_x = 0;
            self.shelf_height = 0;
        }
        if self.shelf_y + height > self.side {
            return None;
        }
        let placement = Placement {
            x: self.pen_x,
            y: self.shelf_y,
            width,
            height,
        };
        self.pen_x += width + Self::PAD;
        self.shelf_height = self.shelf_height.max(height);
        Some(placement)
    }

    /// Double the side, keeping every glyph at the texel it already had.
    ///
    /// # Why not repack
    ///
    /// This used to repack every glyph into the larger texture, which uses the
    /// new width better. It also moved glyphs that vertices earlier in the
    /// *same frame* already addressed: a frame whose text grew the atlas drew
    /// its first lines sampling whatever landed at their old positions.
    /// A grow happens at most a handful of times in a session; a stable
    /// address is worth the right half of a few old shelves. The shelf being
    /// filled simply continues into the wider row.
    fn grow(&mut self) {
        let old_side = self.side;
        let old_texels = std::mem::take(&mut self.texels);
        self.side = (old_side * 2).min(Self::MAX_SIDE);
        self.texels = vec![0; (self.side * self.side) as usize];
        for row in 0..old_side {
            let from = (row * old_side) as usize;
            let to = (row * self.side) as usize;
            self.texels[to..to + old_side as usize]
                .copy_from_slice(&old_texels[from..from + old_side as usize]);
        }
        self.version = crate::generation::next();
    }

    fn blit(&mut self, at: Placement, alpha: &[u8]) {
        for row in 0..at.height {
            let from = (row * at.width) as usize;
            let to = ((at.y + row) * self.side + at.x) as usize;
            let n = at.width as usize;
            self.texels[to..to + n].copy_from_slice(&alpha[from..from + n]);
        }
    }

    fn slot(&self, at: Placement) -> AtlasSlot {
        let side = self.side as f32;
        AtlasSlot {
            x: at.x,
            y: at.y,
            u0: at.x as f32 / side,
            v0: at.y as f32 / side,
            u1: (at.x + at.width) as f32 / side,
            v1: (at.y + at.height) as f32 / side,
            width: at.width,
            height: at.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: u16) -> AtlasKey {
        AtlasKey {
            font: 1,
            glyph,
            size: 13f32.to_bits(),
            transform: [0; 4],
            phase: (0, 0),
        }
    }

    #[test]
    fn the_white_texel_is_full_coverage_and_nothing_is_packed_over_it() {
        let mut atlas = Atlas::new();
        assert_eq!(atlas.texels()[0], 255);
        for g in 0..200u16 {
            atlas.insert(key(g), 6, 9, &[128; 54]);
        }
        assert_eq!(
            atlas.texels()[0],
            255,
            "the reserved texel survived {} glyphs",
            atlas.len()
        );
    }

    /// The property the whole design rests on: sampling the white texel returns
    /// full coverage, so a solid fill routed through the text shader is
    /// unchanged.
    #[test]
    fn the_white_uv_lands_inside_the_reserved_texel() {
        let atlas = Atlas::new();
        let [u, v] = atlas.white_uv();
        let side = atlas.side() as f32;
        assert!(
            u * side > 0.0 && u * side < 1.0,
            "u lands in texel column 0"
        );
        assert!(v * side > 0.0 && v * side < 1.0, "v lands in texel row 0");
    }

    #[test]
    fn the_same_key_is_packed_once() {
        let mut atlas = Atlas::new();
        let first = atlas.insert(key(7), 4, 4, &[9; 16]).unwrap();
        let version = atlas.version();
        let second = atlas.insert(key(7), 4, 4, &[9; 16]).unwrap();
        assert_eq!(first, second);
        assert_eq!(atlas.len(), 1);
        assert_eq!(
            atlas.version(),
            version,
            "re-inserting an existing key changes nothing, so it must not \
             force a re-upload"
        );
    }

    /// Packed glyphs must not overlap. Asserted by painting each glyph a
    /// distinct value and checking every texel is claimed by at most one — the
    /// failure this catches is one glyph's rim appearing inside another's
    /// patch, which reads as a font bug rather than a packer bug.
    #[test]
    fn packed_glyphs_never_share_a_texel() {
        let mut atlas = Atlas::new();
        let mut slots = Vec::new();
        for g in 1..60u16 {
            let w = 3 + u32::from(g % 7);
            let h = 4 + u32::from(g % 5);
            let value = u8::try_from(g).unwrap();
            let slot = atlas
                .insert(key(g), w, h, &vec![value; (w * h) as usize])
                .unwrap();
            slots.push((value, slot));
        }
        let side = atlas.side();
        for (value, slot) in slots {
            let x0 = (slot.u0 * side as f32).round() as u32;
            let y0 = (slot.v0 * side as f32).round() as u32;
            for row in 0..slot.height {
                for col in 0..slot.width {
                    let texel = atlas.texels()[((y0 + row) * side + x0 + col) as usize];
                    assert_eq!(
                        texel,
                        value,
                        "texel ({}, {}) belongs to glyph {value} and holds {texel}",
                        x0 + col,
                        y0 + row
                    );
                }
            }
        }
    }

    /// Growth must keep every glyph's bytes *and* keep its slot pointing at
    /// them. A resize that reflows without re-blitting is the bug this exists
    /// for, and it shows up as the whole screen's text turning into other
    /// letters.
    #[test]
    fn growing_preserves_every_glyphs_coverage() {
        let mut atlas = Atlas::new();
        let mut written = Vec::new();
        // Enough 30x30 glyphs to overflow a 256x256 atlas several times.
        for g in 1..400u16 {
            let value = u8::try_from(g % 251 + 1).unwrap();
            let Some(_) = atlas.insert(key(g), 30, 30, &vec![value; 900]) else {
                break;
            };
            written.push((key(g), value));
        }
        assert!(
            atlas.side() > Atlas::INITIAL_SIDE,
            "the test needs to have forced at least one growth"
        );
        let side = atlas.side();
        for (k, value) in written {
            let slot = atlas.get(&k).expect("still packed after growth");
            let x0 = (slot.u0 * side as f32).round() as u32;
            let y0 = (slot.v0 * side as f32).round() as u32;
            for row in 0..slot.height {
                for col in 0..slot.width {
                    assert_eq!(
                        atlas.texels()[((y0 + row) * side + x0 + col) as usize],
                        value,
                        "glyph {} lost its coverage across a resize",
                        k.glyph
                    );
                }
            }
        }
    }

    #[test]
    fn a_glyph_larger_than_the_biggest_atlas_is_refused_rather_than_looping() {
        let mut atlas = Atlas::new();
        let side = Atlas::MAX_SIDE + 1;
        assert!(atlas
            .insert(key(1), side, 4, &vec![0; (side * 4) as usize])
            .is_none());
    }

    #[test]
    fn a_zero_sized_glyph_is_not_packed() {
        let mut atlas = Atlas::new();
        assert!(atlas.insert(key(1), 0, 5, &[]).is_none());
        assert!(atlas.is_empty());
    }

    /// UVs must address the patch that was written, at every size the atlas
    /// passes through — an off-by-one in `slot` is text drawn one texel to the
    /// left, which is subtle enough to ship.
    #[test]
    fn a_slots_uv_rectangle_is_exactly_its_texels() {
        let mut atlas = Atlas::new();
        let slot = atlas.insert(key(3), 5, 7, &[200; 35]).unwrap();
        let side = atlas.side() as f32;
        assert!((slot.u1 - slot.u0 - 5.0 / side).abs() < 1e-6);
        assert!((slot.v1 - slot.v0 - 7.0 / side).abs() < 1e-6);
        assert_eq!(slot.width, 5);
        assert_eq!(slot.height, 7);
    }
}
