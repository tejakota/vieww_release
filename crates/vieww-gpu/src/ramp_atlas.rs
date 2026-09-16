//! Gradient colour ramps, one row of [`RampAtlas::WIDTH`] premultiplied
//! `RGBA32F` texels per distinct stop list.
//!
//! A gradient's *geometry* (where `t` comes from) is evaluated per fragment in
//! the scene shader; its *colours* (what `t` means) come from here. The row is
//! `vieww_paint::native::gradient_ramp` — the CPU rasterizer's own ramp — so
//! the two renderers disagree only inside a texel interval that contains a
//! stop, and there by less than the interpolation across one 1/255 step.
//!
//! Rows are never moved within a frame (vertices hold their row index); a full
//! texture doubles its height. [`RampAtlas::begin_frame`] drops rows the
//! previous frame did not use once more than half the rows are taken.

use std::collections::HashMap;

use vieww_foundation::Gradient;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RampKey(Vec<u32>);

impl RampKey {
    fn of(gradient: &Gradient) -> Self {
        let mut bits = Vec::with_capacity(gradient.stops().len() * 5);
        for stop in gradient.stops() {
            bits.push(stop.offset.to_bits());
            bits.push(u32::from_le_bytes([
                stop.color.r,
                stop.color.g,
                stop.color.b,
                stop.color.a,
            ]));
        }
        Self(bits)
    }
}

/// See the module doc.
#[derive(Debug)]
pub struct RampAtlas {
    rows: u32,
    texels: Vec<f32>,
    entries: HashMap<RampKey, (u32, u64)>,
    next_row: u32,
    frame: u64,
    version: u64,
}

impl Default for RampAtlas {
    fn default() -> Self {
        Self::new()
    }
}

impl RampAtlas {
    /// Samples per ramp.
    pub const WIDTH: u32 = 256;
    pub const INITIAL_ROWS: u32 = 16;
    pub const MAX_ROWS: u32 = 4096;

    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: Self::INITIAL_ROWS,
            texels: vec![0.0; (Self::WIDTH * Self::INITIAL_ROWS * 4) as usize],
            entries: HashMap::new(),
            next_row: 0,
            frame: 1,
            version: crate::generation::next(),
        }
    }

    /// Texture height in rows; the width is always [`Self::WIDTH`].
    #[must_use]
    pub const fn rows(&self) -> u32 {
        self.rows
    }
    /// `WIDTH * rows * 4` floats, premultiplied RGBA.
    #[must_use]
    pub fn texels(&self) -> &[f32] {
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

    pub fn begin_frame(&mut self) {
        let previous = self.frame;
        self.frame += 1;
        if self.next_row * 2 <= self.rows {
            return;
        }
        let live: Vec<(RampKey, u32, u64)> = self
            .entries
            .iter()
            .filter(|(_, (_, used))| *used >= previous)
            .map(|(k, (row, used))| (k.clone(), *row, *used))
            .collect();
        if live.len() == self.entries.len() {
            return;
        }
        let old = std::mem::take(&mut self.texels);
        self.texels = vec![0.0; old.len()];
        self.entries.clear();
        self.next_row = 0;
        let w = (Self::WIDTH * 4) as usize;
        for (key, row, used) in live {
            let to = self.next_row as usize * w;
            let from = row as usize * w;
            self.texels[to..to + w].copy_from_slice(&old[from..from + w]);
            self.entries.insert(key, (self.next_row, used));
            self.next_row += 1;
        }
        self.version = crate::generation::next();
    }

    /// The row holding `gradient`'s ramp, or `None` if the texture is full.
    pub fn row(&mut self, gradient: &Gradient) -> Option<u32> {
        let key = RampKey::of(gradient);
        let frame = self.frame;
        if let Some((row, used)) = self.entries.get_mut(&key) {
            *used = frame;
            return Some(*row);
        }
        if self.next_row >= self.rows {
            if self.rows >= Self::MAX_ROWS {
                return None;
            }
            self.rows *= 2;
            self.texels
                .resize((Self::WIDTH * self.rows * 4) as usize, 0.0);
        }
        let row = self.next_row;
        self.next_row += 1;
        let samples = vieww_paint::native::gradient_ramp(gradient, Self::WIDTH as usize);
        let start = (row * Self::WIDTH * 4) as usize;
        for (i, c) in samples.iter().enumerate() {
            self.texels[start + i * 4..start + i * 4 + 4].copy_from_slice(c);
        }
        self.entries.insert(key, (row, frame));
        self.version = crate::generation::next();
        Some(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Color;

    #[test]
    fn a_ramp_row_holds_the_stop_colours_premultiplied_at_its_ends() {
        let mut atlas = RampAtlas::new();
        let gradient = Gradient::horizontal().with_stops(&[
            (0.0, Color::rgba(255, 0, 0, 255)),
            (1.0, Color::rgba(0, 0, 255, 128)),
        ]);
        let row = atlas.row(&gradient).unwrap() as usize;
        let w = RampAtlas::WIDTH as usize;
        let first = &atlas.texels()[row * w * 4..row * w * 4 + 4];
        assert_eq!(first, &[1.0, 0.0, 0.0, 1.0]);
        let last = &atlas.texels()[(row * w + w - 1) * 4..(row * w + w) * 4];
        let a = 128.0 / 255.0;
        assert!((last[2] - a).abs() < 1e-6 && (last[3] - a).abs() < 1e-6);
    }

    #[test]
    fn the_same_stops_share_a_row_and_growing_keeps_rows() {
        let mut atlas = RampAtlas::new();
        let mut rows = Vec::new();
        for i in 0..40u8 {
            let g = Gradient::horizontal()
                .with_stops(&[(0.0, Color::rgba(i, 0, 0, 255)), (1.0, Color::WHITE)]);
            rows.push(atlas.row(&g).unwrap());
        }
        for (i, row) in rows.iter().enumerate() {
            let g = Gradient::horizontal()
                .with_stops(&[(0.0, Color::rgba(i as u8, 0, 0, 255)), (1.0, Color::WHITE)]);
            assert_eq!(atlas.row(&g), Some(*row));
        }
        assert_eq!(atlas.len(), 40);
    }
}
