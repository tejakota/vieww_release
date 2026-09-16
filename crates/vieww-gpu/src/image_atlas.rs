//! Persistent RGBA8 atlas for decoded [`vieww_foundation::Image`] values.
//!
//! Images are keyed by the identity of their shared pixel allocation, matching
//! `Image`'s own equality semantics. The atlas is deliberately simple and
//! frame-stable: it grows and repacks between frames, never evicts while a
//! `ScenePlan` is being built, and exposes a generation so a backend uploads
//! only when pixels or packing changed.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use vieww_foundation::Image;

use crate::atlas::AtlasSlot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageKey(u64);

impl ImageKey {
    #[must_use]
    pub fn of(image: &Image) -> Self {
        // `Image` equality is pointer identity for the shared pixel buffer.
        // The allocation is kept alive by `image` for the duration of planning,
        // so using its address as the cache identity is safe and collision-free
        // for live images in this process.
        Self(Arc::as_ptr(image.shared()) as usize as u64)
    }
}

#[derive(Debug, Clone, Copy)]
struct Placement {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageSlot {
    /// Texel top-left inside the atlas. Stable for the atlas's lifetime.
    pub x: u32,
    pub y: u32,
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub width: u32,
    pub height: u32,
}

impl From<ImageSlot> for AtlasSlot {
    fn from(slot: ImageSlot) -> Self {
        Self {
            x: slot.x,
            y: slot.y,
            u0: slot.u0,
            v0: slot.v0,
            u1: slot.u1,
            v1: slot.v1,
            width: slot.width,
            height: slot.height,
        }
    }
}

#[derive(Debug)]
pub struct ImageAtlas {
    side: u32,
    texels: Vec<u8>,
    entries: HashMap<ImageKey, Entry>,
    pen_x: u32,
    shelf_y: u32,
    shelf_height: u32,
    version: u64,
    frame: u64,
    used_texels: u64,
}

/// One resident image.
///
/// # Why a `Weak`, and why this used to be wrong
///
/// [`ImageKey`] is the image's `Arc` address. An address is only an identity
/// while the allocation is alive: once the image is dropped, the allocator is
/// free to hand the same address to the *next* image, and the atlas then
/// answered that image with the previous one's texels. A scene that builds a
/// fresh `Image` every frame (a video frame, a regenerated chart) hit it at
/// allocator whim — `test-gpu-work` rendered its minified photo from a stale
/// mip level on roughly one run in three.
///
/// Holding a `Weak` fixes both halves: while the entry exists the `ArcInner`
/// allocation cannot be freed, so its address cannot be reused; and a dead
/// `Weak` marks the entry stale, so it is replaced rather than trusted.
#[derive(Debug, Clone)]
struct Entry {
    placement: Placement,
    image: Weak<Vec<u8>>,
    last_used: u64,
}

impl Entry {
    fn is_alive(&self) -> bool {
        self.image.strong_count() > 0
    }
}

impl Default for ImageAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageAtlas {
    pub const INITIAL_SIDE: u32 = 512;
    pub const MAX_SIDE: u32 = 4096;
    const PAD: u32 = 1;

    #[must_use]
    pub fn new() -> Self {
        let side = Self::INITIAL_SIDE;
        Self {
            side,
            texels: vec![0; (side * side * 4) as usize],
            entries: HashMap::new(),
            pen_x: 0,
            shelf_y: 0,
            shelf_height: 0,
            version: crate::generation::next(),
            frame: 1,
            used_texels: 0,
        }
    }

    /// Start a frame: drop dead images, and when more than half the texture is
    /// in use, rebuild it with only the images the previous frame drew.
    ///
    /// Entries never move *within* a frame — vertices hold their texel
    /// positions — so this is the only place eviction happens.
    pub fn begin_frame(&mut self) {
        let previous = self.frame;
        self.frame += 1;
        // Dead images are forgotten immediately — which releases their `Weak`
        // and so their address — but their texels stay where they are until a
        // compaction reclaims the space. Repacking here would re-upload the
        // whole texture on every frame of a scene that makes a new image per
        // frame, which is exactly the scene that produces dead entries.
        self.entries.retain(|_, e| e.is_alive());
        let capacity = u64::from(self.side) * u64::from(self.side);
        if self.used_texels * 2 <= capacity {
            return;
        }
        let keep: Vec<(ImageKey, Entry, Vec<u8>)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.last_used >= previous)
            .map(|(k, e)| (*k, e.clone(), self.copy_out(e.placement)))
            .collect();
        self.texels.iter_mut().for_each(|t| *t = 0);
        self.entries.clear();
        self.pen_x = 0;
        self.shelf_y = 0;
        self.shelf_height = 0;
        self.used_texels = 0;
        for (key, entry, bytes) in keep {
            if let Some(placement) =
                self.try_allocate(entry.placement.width, entry.placement.height)
            {
                self.blit(placement, &bytes);
                self.replicate_padding(placement);
                self.used_texels += u64::from(placement.width) * u64::from(placement.height);
                self.entries.insert(key, Entry { placement, ..entry });
            }
        }
        self.version = crate::generation::next();
    }

    fn copy_out(&self, p: Placement) -> Vec<u8> {
        let mut out = Vec::with_capacity((p.width * p.height * 4) as usize);
        for row in 0..p.height {
            let from = ((p.y + row) * self.side * 4 + p.x * 4) as usize;
            out.extend_from_slice(&self.texels[from..from + (p.width * 4) as usize]);
        }
        out
    }

    #[must_use]
    pub const fn side(&self) -> u32 {
        self.side
    }
    #[must_use]
    pub fn texels(&self) -> &[u8] {
        &self.texels
    }
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The slot for a resident, still-alive image.
    #[must_use]
    pub fn get(&self, key: &ImageKey) -> Option<ImageSlot> {
        self.entries
            .get(key)
            .filter(|e| e.is_alive())
            .map(|e| self.slot(e.placement))
    }

    pub fn insert(&mut self, image: &Image) -> Option<ImageSlot> {
        let key = ImageKey::of(image);
        let frame = self.frame;
        if let Some(existing) = self.entries.get_mut(&key) {
            if existing
                .image
                .upgrade()
                .is_some_and(|live| Arc::ptr_eq(&live, image.shared()))
            {
                existing.last_used = frame;
                let placement = existing.placement;
                return Some(self.slot(placement));
            }
            // A dead image whose address the allocator reused. Its texels
            // are not this image's; drop the entry (its space is reclaimed at
            // the next compaction) and place the new image fresh.
            self.entries.remove(&key);
        }
        let w = image.width();
        let h = image.height();
        if w == 0
            || h == 0
            || w.saturating_add(Self::PAD * 2) > Self::MAX_SIDE
            || h.saturating_add(Self::PAD * 2) > Self::MAX_SIDE
        {
            return None;
        }
        let placement = self.allocate(w, h)?;
        self.blit(placement, image.pixels());
        self.used_texels += u64::from(w) * u64::from(h);
        self.entries.insert(
            key,
            Entry {
                placement,
                image: Arc::downgrade(image.shared()),
                last_used: frame,
            },
        );
        self.version = crate::generation::next();
        Some(self.slot(placement))
    }

    fn allocate(&mut self, w: u32, h: u32) -> Option<Placement> {
        loop {
            if let Some(p) = self.try_allocate(w, h) {
                return Some(p);
            }
            if self.side >= Self::MAX_SIDE {
                return None;
            }
            self.grow();
        }
    }
    fn try_allocate(&mut self, w: u32, h: u32) -> Option<Placement> {
        let total_w = w.saturating_add(Self::PAD * 2);
        let total_h = h.saturating_add(Self::PAD * 2);
        if self.pen_x.saturating_add(total_w) > self.side {
            let next = self
                .shelf_y
                .saturating_add(self.shelf_height)
                .saturating_add(Self::PAD);
            if next.saturating_add(total_h) > self.side {
                return None;
            }
            self.shelf_y = next;
            self.pen_x = 0;
            self.shelf_height = 0;
        }
        if self.shelf_y.saturating_add(total_h) > self.side {
            return None;
        }
        let p = Placement {
            x: self.pen_x + Self::PAD,
            y: self.shelf_y + Self::PAD,
            width: w,
            height: h,
        };
        self.pen_x = self.pen_x.saturating_add(total_w);
        self.shelf_height = self.shelf_height.max(total_h);
        Some(p)
    }
    /// Double the side, keeping every image at the texel it already had —
    /// see `Atlas::grow` for why a frame's earlier vertices need that.
    fn grow(&mut self) {
        let old_side = self.side;
        let old = std::mem::take(&mut self.texels);
        self.side = (old_side * 2).min(Self::MAX_SIDE);
        self.texels = vec![0; (self.side * self.side * 4) as usize];
        for row in 0..old_side {
            let from = (row * old_side * 4) as usize;
            let to = (row * self.side * 4) as usize;
            let n = (old_side * 4) as usize;
            self.texels[to..to + n].copy_from_slice(&old[from..from + n]);
        }
        self.version = crate::generation::next();
    }
    fn blit(&mut self, p: Placement, src: &[u8]) {
        debug_assert_eq!(src.len(), (p.width * p.height * 4) as usize);
        for row in 0..p.height {
            let from = (row * p.width * 4) as usize;
            let to = ((p.y + row) * self.side * 4 + p.x * 4) as usize;
            let n = (p.width * 4) as usize;
            self.texels[to..to + n].copy_from_slice(&src[from..from + n]);
        }
        self.replicate_padding(p);
    }

    fn replicate_padding(&mut self, p: Placement) {
        if p.width == 0 || p.height == 0 {
            return;
        }
        let left = p.x - Self::PAD;
        let top = p.y - Self::PAD;
        let right = p.x + p.width;
        let bottom = p.y + p.height;
        for row in p.y..bottom {
            let src = ((row * self.side + p.x) * 4) as usize;
            let dst = ((row * self.side + left) * 4) as usize;
            let pixel = [
                self.texels[src],
                self.texels[src + 1],
                self.texels[src + 2],
                self.texels[src + 3],
            ];
            self.texels[dst..dst + 4].copy_from_slice(&pixel);
            let src_r = ((row * self.side + right - 1) * 4) as usize;
            let dst_r = ((row * self.side + right) * 4) as usize;
            let pixel = [
                self.texels[src_r],
                self.texels[src_r + 1],
                self.texels[src_r + 2],
                self.texels[src_r + 3],
            ];
            self.texels[dst_r..dst_r + 4].copy_from_slice(&pixel);
        }
        for x in left..=right {
            let src = ((p.y * self.side + x) * 4) as usize;
            let dst = ((top * self.side + x) * 4) as usize;
            let pixel = [
                self.texels[src],
                self.texels[src + 1],
                self.texels[src + 2],
                self.texels[src + 3],
            ];
            self.texels[dst..dst + 4].copy_from_slice(&pixel);
            let src_b = (((bottom - 1) * self.side + x) * 4) as usize;
            let dst_b = ((bottom * self.side + x) * 4) as usize;
            let pixel = [
                self.texels[src_b],
                self.texels[src_b + 1],
                self.texels[src_b + 2],
                self.texels[src_b + 3],
            ];
            self.texels[dst_b..dst_b + 4].copy_from_slice(&pixel);
        }
    }
    fn slot(&self, p: Placement) -> ImageSlot {
        let s = self.side as f32;
        // Linear filtering must never see the neighbouring image. Sample at
        // half-texel inset points for multi-pixel images; a 1x1 image uses its
        // centre point, producing a non-zero span while staying inside itself.
        let inset = |size: u32| if size <= 1 { 0.25 } else { 0.5 };
        let left = p.x as f32 + inset(p.width);
        let top = p.y as f32 + inset(p.height);
        let right = p.x as f32 + p.width.max(1) as f32 - inset(p.width);
        let bottom = p.y as f32 + p.height.max(1) as f32 - inset(p.height);
        ImageSlot {
            x: p.x,
            y: p.y,
            u0: left / s,
            v0: top / s,
            u1: right / s,
            v1: bottom / s,
            width: p.width,
            height: p.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The allocator reuses a freed image's address for the next image of the
    /// same size far more often than chance. The atlas must answer the new
    /// image with the new image's texels.
    #[test]
    fn a_new_image_at_a_dead_images_address_is_not_served_its_texels() {
        let mut atlas = ImageAtlas::new();
        let mut reused = 0;
        for i in 0..200u32 {
            let value = (i % 250) as u8;
            let img = image(8, 8, [value, 0, 0, 255]);
            let slot = atlas.insert(&img).expect("fits");
            let at = ((slot.y * atlas.side() + slot.x) * 4) as usize;
            assert_eq!(
                atlas.texels()[at],
                value,
                "image {i} was served another image's texels"
            );
            if i > 0 && slot.x == 0 {
                reused += 1;
            }
            drop(img);
            atlas.begin_frame();
        }
        let _ = reused;
    }

    #[test]
    fn compaction_keeps_last_frames_images_and_reclaims_the_rest() {
        let mut atlas = ImageAtlas::new();
        // Four 400x400 images in a 1024x1024 texture: over half full, so the
        // next frame compacts.
        let big: Vec<Image> = (0..4).map(|i| image(400, 400, [i, 1, 2, 255])).collect();
        for img in &big {
            atlas.insert(img).expect("fits");
        }
        atlas.begin_frame();
        // Only the last image is drawn this frame; the rest stay alive.
        let kept = atlas.insert(&big[3]).expect("resident");
        let _ = kept;
        atlas.begin_frame();
        assert_eq!(
            atlas.len(),
            1,
            "only the image drawn last frame survives compaction"
        );
        let slot = atlas.get(&ImageKey::of(&big[3])).expect("still resident");
        let at = ((slot.y * atlas.side() + slot.x) * 4) as usize;
        assert_eq!(atlas.texels()[at], 3);
    }

    fn image(width: u32, height: u32, rgba: [u8; 4]) -> Image {
        Image::from_rgba8(rgba.repeat((width * height) as usize), width, height)
    }

    #[test]
    fn inserts_an_image_once_and_reuses_its_identity() {
        let img = image(2, 2, [10, 20, 30, 255]);
        let mut atlas = ImageAtlas::new();
        let a = atlas.insert(&img).expect("fits");
        let version = atlas.version();
        let b = atlas.insert(&img).expect("same image remains resident");
        assert_eq!(a, b);
        assert_eq!(atlas.len(), 1);
        assert_eq!(atlas.version(), version);
    }

    #[test]
    fn growth_preserves_existing_pixels() {
        let first = image(400, 400, [1, 2, 3, 255]);
        let second = image(400, 400, [7, 8, 9, 255]);
        let mut atlas = ImageAtlas::new();
        atlas.insert(&first).expect("first fits");
        atlas
            .insert(&second)
            .expect("second triggers shelf/growth as needed");
        let slot = atlas
            .get(&ImageKey::of(&first))
            .expect("first survived growth");
        assert_eq!(slot.width, 400);
        assert_eq!(slot.height, 400);
        let x = ((slot.u0 * atlas.side() as f32) - 0.5).round() as usize;
        let y = ((slot.v0 * atlas.side() as f32) - 0.5).round() as usize;
        let offset = (y * atlas.side() as usize + x) * 4;
        assert_eq!(&atlas.texels()[offset..offset + 4], &[1, 2, 3, 255]);
    }

    #[test]
    fn one_pixel_images_have_a_stable_nonzero_sampling_region() {
        let img = image(1, 1, [255, 128, 64, 255]);
        let mut atlas = ImageAtlas::new();
        let slot = atlas.insert(&img).expect("fits");
        assert!(slot.u1 > slot.u0);
        assert!(slot.v1 > slot.v0);
    }

    #[test]
    fn adjacent_images_get_edge_replicated_padding_for_linear_sampling() {
        let red = image(4, 4, [255, 0, 0, 255]);
        let blue = image(4, 4, [0, 0, 255, 255]);
        let mut atlas = ImageAtlas::new();
        let first = atlas.insert(&red).expect("red fits");
        let second = atlas.insert(&blue).expect("blue fits beside red");
        assert!(second.u0 > first.u1, "images are separated by padding");

        let sample_x = |slot: ImageSlot, edge: bool| {
            let px = if edge {
                slot.u1 * atlas.side() as f32
            } else {
                slot.u0 * atlas.side() as f32
            };
            px.floor() as usize
        };
        let y = (first.v0 * atlas.side() as f32).floor() as usize;
        let row = y * atlas.side() as usize * 4;
        let right = sample_x(first, true).saturating_sub(1) * 4;
        let left = sample_x(second, false).saturating_sub(1) * 4;
        assert_eq!(
            &atlas.texels()[row + right..row + right + 4],
            &[255, 0, 0, 255]
        );
        assert_eq!(
            &atlas.texels()[row + left..row + left + 4],
            &[0, 0, 255, 255]
        );
    }
}
