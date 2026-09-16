//! Looking at the pixels: golden images, and the partial-repaint oracle.
//!
//! # The hole this fills
//!
//! `docs/PRODUCTION-GAPS.md` is blunt about it: *"the pixel-test suites assert
//! inline probes rather than golden baselines, so a cross-cutting rendering
//! regression that keeps correct probe pixels could pass."* A probe is a
//! statement about one pixel that somebody thought to name. A golden image is a
//! statement about all of them.
//!
//! And there was a second, sharper hole beside it. The damage tests assert that
//! **pixels outside the damage survive** — that a partial repaint does not
//! disturb what it should not touch. Nothing asserted the other direction: that
//! the damage is *large enough*. A damage region that is too small passes every
//! one of those tests, because every pixel it failed to repaint is by definition
//! outside it, and produces exactly the failure that is hardest to find by
//! eye — ink from an earlier frame left standing on a persistent target,
//! accumulating over the frames of an animation.
//!
//! That failure is also nearly invisible in a light theme and obvious in a dark
//! one, for a reason that is about perception rather than about rendering: a
//! five-percent residue of light ink on a near-black ground is a large relative
//! change in luminance, and the same residue of dark ink on near-white is not.
//! A bug present in both themes therefore gets reported as "the icons look
//! smeared in dark mode", and every investigation that starts from the theme
//! finds nothing wrong with the theme.
//!
//! [`assert_partial_repaint_is_complete`] is the missing invariant, stated
//! directly: **compositing the damaged regions onto the previous frame must
//! give the same picture as repainting everything.**

use std::path::{Path, PathBuf};

use vieww_foundation::{Color, Rect, Size};
use vieww_paint::native::NativeRenderer;
use vieww_paint::Damage;
use vieww_render::FrameDriver;

/// One rasterised frame, as straight RGBA8.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Frame {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The pixel at `(x, y)` as `(r, g, b, a)`.
    ///
    /// # Panics
    ///
    /// If the coordinates are outside the frame.
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> (u8, u8, u8, u8) {
        assert!(x < self.width && y < self.height, "({x}, {y}) is off-frame");
        let i = ((y * self.width + x) * 4) as usize;
        (
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        )
    }

    /// The raw bytes, four per pixel, top row first.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Every pixel where `self` and `other` differ by more than `tolerance` in
    /// any channel, in scan order.
    ///
    /// A tolerance because two rasterisations of identical geometry can differ
    /// by a unit on an antialiased edge, and a harness that failed on that
    /// would be a harness people turn off.
    #[must_use]
    pub fn differences(&self, other: &Self, tolerance: u8) -> Vec<Difference> {
        if self.width != other.width || self.height != other.height {
            return vec![Difference {
                x: 0,
                y: 0,
                left: (0, 0, 0, 0),
                right: (0, 0, 0, 0),
            }];
        }
        let mut out = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let (left, right) = (self.at(x, y), other.at(x, y));
                let apart = |a: u8, b: u8| a.abs_diff(b) > tolerance;
                if apart(left.0, right.0)
                    || apart(left.1, right.1)
                    || apart(left.2, right.2)
                    || apart(left.3, right.3)
                {
                    out.push(Difference { x, y, left, right });
                }
            }
        }
        out
    }

    /// Write this frame as a PNG.
    ///
    /// # Errors
    ///
    /// Whatever the filesystem or the encoder reports.
    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_png())
    }

    /// This frame encoded as a PNG.
    #[must_use]
    pub fn to_png(&self) -> Vec<u8> {
        encode_png(&self.data, self.width, self.height)
    }

    /// Read a frame back from a PNG written by [`write_png`](Self::write_png).
    ///
    /// # Errors
    ///
    /// If the file is missing, or is not a PNG this module wrote.
    pub fn read_png(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        decode_png(&bytes).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} is not a readable PNG", path.display()),
            )
        })
    }
}

/// One differing pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Difference {
    pub x: u32,
    pub y: u32,
    pub left: (u8, u8, u8, u8),
    pub right: (u8, u8, u8, u8),
}

/// Rasterise what `driver` last painted, whole.
#[must_use]
pub fn render(driver: &FrameDriver, size: Size, base: Color) -> Frame {
    let (width, height) = dimensions(size);
    let (pixels, _) = NativeRenderer::new()
        .render_to_pixels(driver.scene(), width, height, base)
        .expect("vieww's own rasterizer needs no display");
    Frame {
        data: pixels.data().to_vec(),
        width,
        height,
    }
}

/// Rasterise only the commands that reach `damage`, over `base`.
#[must_use]
fn render_damaged(driver: &FrameDriver, damage: &Damage, size: Size, base: Color) -> Frame {
    let (width, height) = dimensions(size);
    let (pixels, _) = NativeRenderer::new()
        .render_damaged(driver.scene(), damage, width, height, base)
        .expect("vieww's own rasterizer needs no display");
    Frame {
        data: pixels.data().to_vec(),
        width,
        height,
    }
}

/// A persistent target, driven by damage — the model a GPU backend uses.
///
/// Constructed from a first, complete frame; [`advance`](Self::advance) then
/// composites each subsequent frame's damaged regions onto it, exactly as
/// `NativeRenderer::render_damaged`'s caller re-composites tiles over the
/// previous frame when driving a persistent surface.
#[derive(Debug, Clone)]
pub struct Persistent {
    frame: Frame,
    base: Color,
    size: Size,
}

impl Persistent {
    /// Start from a complete render of the current frame.
    #[must_use]
    pub fn new(driver: &FrameDriver, size: Size, base: Color) -> Self {
        Self {
            frame: render(driver, size, base),
            base,
            size,
        }
    }

    /// The picture currently on the target.
    #[must_use]
    pub const fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Composite this frame's damage onto the target.
    ///
    /// Region by region, each rasterised on its own and blitted back — not one
    /// union of them all. The distinction is the point: a union is more generous
    /// than what a GPU backend actually does, so an oracle built on one would
    /// pass against damage that the real backend under-repaints.
    pub fn advance(&mut self, driver: &FrameDriver) {
        for region in driver.damage().repaint_regions() {
            let mut one = Damage::new(driver.surface());
            one.add(region);
            let tile = render_damaged(driver, &one, self.size, self.base);
            self.blit(&tile, region);
        }
    }

    fn blit(&mut self, tile: &Frame, region: Rect) {
        let region = region.round_out();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "rounded outward and clamped to the surface below"
        )]
        let (x0, y0, x1, y1) = (
            region.left.max(0.0) as u32,
            region.top.max(0.0) as u32,
            (region.right.max(0.0) as u32).min(self.frame.width),
            (region.bottom.max(0.0) as u32).min(self.frame.height),
        );
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * self.frame.width + x) * 4) as usize;
                self.frame.data[i..i + 4].copy_from_slice(&tile.data[i..i + 4]);
            }
        }
    }
}

/// **The invariant the damage tests were missing.**
///
/// Drives `driver` through `frames` frames — each one produced by `step`, which
/// is where a test makes whatever change it is testing — and asserts after every
/// one that the damage-driven target and a full repaint agree.
///
/// # What a failure means
///
/// Not that the frame is drawn wrongly. A full repaint of the same frame is
/// correct by construction, and that is what the target is compared against. A
/// failure means the frame **under-reported what changed**, so a real backend
/// with a persistent surface would leave the previous frame's ink standing in
/// the pixels the damage did not name.
///
/// Over the frames of an animation that residue accumulates, and what a user
/// reports is not "the damage region is a pixel short" — it is "the icons look
/// smeared".
///
/// # Panics
///
/// With the first differing pixel and both colours, which is usually enough to
/// name the object that moved without being reported.
pub fn assert_partial_repaint_is_complete(
    driver: &mut FrameDriver,
    size: Size,
    base: Color,
    frames: usize,
    mut step: impl FnMut(&mut FrameDriver, usize),
) {
    let mut target = Persistent::new(driver, size, base);

    for frame in 0..frames {
        step(driver, frame);
        target.advance(driver);

        let truth = render(driver, size, base);
        // A tolerance of one: the damaged tile and the full render are two
        // separate rasterisations of identical geometry, and an antialiased edge
        // can land a unit apart between them. Anything the eye could see is
        // orders of magnitude larger than that.
        let differences = target.frame().differences(&truth, 1);
        assert!(
            differences.is_empty(),
            "frame {frame}: the damaged repaint and a full repaint disagree at \
             {} pixel(s). First: {:?}.\n\nThis means the frame reported less \
             damage than it changed. A backend with a persistent target would \
             keep the previous frame's ink at those pixels, and over an \
             animation that residue accumulates into what gets reported as \
             smearing.",
            differences.len(),
            differences[0]
        );
    }
}

/// Compare a frame against a stored baseline, writing the baseline if it is
/// missing or if `VIEWW_UPDATE_GOLDEN` is set.
///
/// # Why an environment variable rather than a flag
///
/// Because the update has to be a *deliberate, separate act*. A harness where
/// updating the baseline is as easy as running the test is a harness that
/// records every regression as the new truth — which is the failure mode golden
/// images are famous for, and the reason to say so here rather than let it be
/// discovered.
///
/// A missing baseline is written and the test passes, once. A baseline that
/// exists and disagrees fails, and both the expected and the actual image are
/// left on disk beside each other so a person can look at them — which is the
/// whole point of a golden image over a probe.
///
/// # Panics
///
/// If the frame differs from the stored baseline.
pub fn assert_matches_golden(frame: &Frame, baseline: &Path) {
    let updating = std::env::var_os("VIEWW_UPDATE_GOLDEN").is_some();

    if updating || !baseline.exists() {
        frame
            .write_png(baseline)
            .expect("writing the golden baseline");
        if !updating {
            eprintln!(
                "wrote a new baseline at {} — check that it looks right, and \
                 commit it",
                baseline.display()
            );
        }
        return;
    }

    let stored = Frame::read_png(baseline).expect("reading the golden baseline");
    let differences = frame.differences(&stored, 1);
    if differences.is_empty() {
        return;
    }

    let actual: PathBuf = baseline.with_extension("actual.png");
    let _ = frame.write_png(&actual);
    panic!(
        "{} differs from its baseline at {} pixel(s). First: {:?}.\n\nThe \
         rendered frame was written to {} — look at the two side by side. If \
         the change is intended, re-run with VIEWW_UPDATE_GOLDEN=1.",
        baseline.display(),
        differences.len(),
        differences[0],
        actual.display()
    );
}

// ------------------------------------------------------------------ plumbing

fn dimensions(size: Size) -> (u32, u32) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a surface size"
    )]
    (size.width as u32, size.height as u32)
}

/// A minimal PNG encoder: one `IDAT`, stored (uncompressed) deflate blocks.
///
/// # Why not pull in a PNG crate
///
/// This crate is a *test* harness, and its dependency list is the thing every
/// test in the workspace inherits. A baseline image needs to be written, read
/// back byte-identically, and opened by a human in an image viewer; none of that
/// needs a compressor. Stored deflate blocks are part of the format, every
/// decoder handles them, and the whole encoder is short enough to read.
///
/// The cost is file size, which for a few hundred baseline frames of flat UI
/// chrome is not the constraint.
fn encode_png(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * height * 4 + height) as usize);
    for y in 0..height {
        // Filter type 0 (none), once per scanline: part of the format, and the
        // reason a decoder can be as simple as the one below.
        raw.push(0u8);
        let start = (y * width * 4) as usize;
        raw.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    // 8 bits per channel, colour type 6 (RGBA), deflate, no filter, no interlace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(
        &u32::try_from(data.len())
            .expect("a chunk shorter than 4GiB")
            .to_be_bytes(),
    );
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A zlib stream of stored deflate blocks: no compression, valid everywhere.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    // 0x78 0x01: deflate, 32KiB window, no preset dictionary, fastest level.
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(0xFFFF).peekable();
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    while let Some(block) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        out.push(last);
        #[expect(clippy::cast_possible_truncation, reason = "chunked to 0xFFFF above")]
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// The matching decoder: only the files [`encode_png`] writes.
///
/// Deliberately narrow. A baseline is written by this module and read by this
/// module, so accepting the whole format would be accepting inputs that cannot
/// occur — and every one of those branches would be untested.
fn decode_png(bytes: &[u8]) -> Option<Frame> {
    if bytes.get(..8)? != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let (mut at, mut width, mut height) = (8usize, 0u32, 0u32);
    let mut deflate = Vec::new();

    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let data = bytes.get(at + 8..at + 8 + len)?;
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes(data.get(..4)?.try_into().ok()?);
                height = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?);
                // Only what this module writes: 8-bit RGBA, uninterlaced.
                if data.get(8..) != Some(&[8, 6, 0, 0, 0][..]) {
                    return None;
                }
            }
            b"IDAT" => deflate.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }

    let raw = inflate_stored(&deflate)?;
    let stride = (width * 4 + 1) as usize;
    if raw.len() != stride * height as usize {
        return None;
    }
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        // Filter 0 only, which is all this module writes.
        if raw[y * stride] != 0 {
            return None;
        }
        data.extend_from_slice(&raw[y * stride + 1..(y + 1) * stride]);
    }
    Some(Frame {
        data,
        width,
        height,
    })
}

fn inflate_stored(stream: &[u8]) -> Option<Vec<u8>> {
    // Skip the two-byte zlib header; the trailing Adler-32 is not re-checked,
    // because a corrupt baseline shows up as a pixel difference and a
    // checksum failure would be a less useful message than a picture.
    let mut at = 2usize;
    let mut out = Vec::new();
    loop {
        let header = *stream.get(at)?;
        // Only stored blocks: bits 1-2 must be zero.
        if header & 0b110 != 0 {
            return None;
        }
        let len = u16::from_le_bytes(stream.get(at + 1..at + 3)?.try_into().ok()?) as usize;
        out.extend_from_slice(stream.get(at + 5..at + 5 + len)?);
        at += 5 + len;
        if header & 1 == 1 {
            return Some(out);
        }
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, fill: (u8, u8, u8, u8)) -> Frame {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            data.extend_from_slice(&[fill.0, fill.1, fill.2, fill.3]);
        }
        Frame {
            data,
            width,
            height,
        }
    }

    /// The whole harness rests on this: a baseline written and read back must be
    /// the same pixels, or a golden test fails on its own encoder.
    #[test]
    fn a_frame_survives_a_round_trip_through_png() {
        let mut original = frame(37, 19, (10, 200, 30, 255));
        // Something that is not a flat fill, so a decoder that ignored the rows
        // could not pass.
        for y in 0..19 {
            for x in 0..37 {
                let i = ((y * 37 + x) * 4) as usize;
                original.data[i] = u8::try_from(x).unwrap();
                original.data[i + 1] = u8::try_from(y).unwrap();
            }
        }

        let png = original.to_png();
        let read = decode_png(&png).expect("the decoder reads what the encoder writes");
        assert_eq!(read.width(), 37);
        assert_eq!(read.height(), 19);
        assert_eq!(read.data(), original.data(), "byte-identical");
    }

    /// A large image crosses the 0xFFFF stored-block boundary, which is the one
    /// place the encoder can go wrong in a way a small fixture never reaches.
    #[test]
    fn an_image_larger_than_one_deflate_block_round_trips() {
        let original = frame(200, 200, (7, 8, 9, 255));
        assert!(original.data.len() > 0xFFFF, "spans several blocks");
        let read = decode_png(&original.to_png()).expect("decoded");
        assert_eq!(read.data(), original.data());
    }

    #[test]
    fn differences_are_reported_with_a_tolerance() {
        let a = frame(4, 4, (100, 100, 100, 255));
        let mut b = a.clone();
        b.data[0] = 101;
        assert!(a.differences(&b, 1).is_empty(), "one unit is antialiasing");
        assert_eq!(a.differences(&b, 0).len(), 1, "and zero tolerance sees it");

        b.data[4] = 200;
        let seen = a.differences(&b, 1);
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].x, seen[0].y), (1, 0));
    }

    #[test]
    fn frames_of_different_sizes_are_reported_as_different() {
        assert!(!frame(4, 4, (0, 0, 0, 255))
            .differences(&frame(5, 4, (0, 0, 0, 255)), 255)
            .is_empty());
    }
}
