//! The public pixel output: tightly packed, top-row-first, straight-alpha
//! RGBA8 — the shape every reader in this workspace (PNG encoding, a GIF
//! frame, `vieww-hal`'s upload path) already expects.

#[derive(Debug, Clone, PartialEq)]
pub struct Pixels {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl Pixels {
    /// This `Target`'s pixels, converted.
    ///
    /// Test-only — see `Target::to_rgba8`, which it wraps. The renderer's own
    /// output path fills a buffer it already owns rather than allocating one.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn from_target(target: &super::target::Target) -> Self {
        Self {
            data: target.to_rgba8(),
            width: target.width,
            height: target.height,
        }
    }

    /// Wrap already-decoded straight-alpha RGBA8 bytes, tightly packed, top
    /// row first — the shape a GPU backend's readback produces (see
    /// `hal::vulkan`'s render-to-texture-then-readback path).
    ///
    /// # Panics
    ///
    /// If `data` is not exactly `width * height * 4` bytes.
    #[must_use]
    pub fn from_rgba8(data: Vec<u8>, width: u32, height: u32) -> Self {
        assert_eq!(data.len(), (width as usize) * (height as usize) * 4);
        Self {
            data,
            width,
            height,
        }
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// # Panics
    /// If the coordinates are outside the image.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> vieww_foundation::Color {
        assert!(x < self.width && y < self.height);
        let i = ((y * self.width + x) * 4) as usize;
        vieww_foundation::Color::rgba(
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        )
    }

    /// Encode as a PNG, straight alpha, 8 bits per channel.
    pub fn encode_png(&self) -> Result<Vec<u8>, png::EncodingError> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&self.data)?;
        }
        Ok(bytes)
    }
}
