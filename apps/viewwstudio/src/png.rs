//! A minimal RGBA PNG writer, for the examples that produce pictures.
//!
//! `image` is already in the dependency graph through vello, but reaching it
//! from here would make this crate depend on a version vello chose. One
//! zlib-stored PNG is sixty lines and depends on nothing — and the pictures
//! are how this application is checked, so the writer belongs in the library
//! rather than being copied into each example that wants one.

use std::path::Path;

/// Encode `rgba` (`width * height * 4` bytes) as an 8-bit RGBA PNG.
///
/// Separate from [`write()`] so the bytes can be checked without a filesystem,
/// and so a caller that wants the picture in memory does not have to go
/// through a temporary file to get it.
///
/// # Panics
///
/// If a chunk is larger than `u32::MAX`, which needs an image no machine has
/// the memory to hold.
#[must_use]
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * height * 4 + height) as usize);
    for y in 0..height {
        raw.push(0); // filter: none
        let start = (y * width * 4) as usize;
        raw.extend_from_slice(&rgba[start..start + (width * 4) as usize]);
    }

    let mut png = Vec::new();
    png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);

    png
}

/// Write `rgba` to `path` as an 8-bit RGBA PNG.
///
/// # Errors
///
/// Whatever the write failed with — a directory that does not exist, a full
/// disk, a path the process cannot write. It used to `expect`, which turned a
/// full disk into a crash with no way for a caller to say anything better; a
/// picture is never important enough to take the process down for.
pub fn write(path: &Path, rgba: &[u8], width: u32, height: u32) -> std::io::Result<()> {
    std::fs::write(path, encode(rgba, width, height))
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).expect("chunk fits").to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// A zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (index, block) in data.chunks(65_535).enumerate() {
        let last = u8::from((index + 1) * 65_535 >= data.len());
        out.push(last);
        let len = u16::try_from(block.len()).expect("block fits");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four-pixel image every assertion below is about.
    fn swatch() -> (Vec<u8>, u32, u32) {
        #[rustfmt::skip]
        let rgba = vec![
            255, 0, 0, 255,   0, 255, 0, 255,
            0, 0, 255, 255,   255, 255, 255, 128,
        ];
        (rgba, 2, 2)
    }

    #[test]
    fn the_bytes_start_with_the_png_signature_and_an_ihdr() {
        let (rgba, w, h) = swatch();
        let png = encode(&rgba, w, h);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], &w.to_be_bytes());
        assert_eq!(&png[20..24], &h.to_be_bytes());
        // 8-bit, colour type 6 (RGBA), deflate, no filter, no interlace.
        assert_eq!(&png[24..29], &[8, 6, 0, 0, 0]);
    }

    #[test]
    fn every_chunk_carries_the_crc_a_decoder_will_check() {
        let (rgba, w, h) = swatch();
        let png = encode(&rgba, w, h);
        let mut at = 8;
        let mut kinds = Vec::new();
        while at + 12 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let body = &png[at + 8..at + 8 + len];
            let stated = u32::from_be_bytes(png[at + 8 + len..at + 12 + len].try_into().unwrap());
            let mut input = kind.to_vec();
            input.extend_from_slice(body);
            assert_eq!(
                crc32(&input),
                stated,
                "crc for {:?}",
                std::str::from_utf8(kind)
            );
            kinds.push(String::from_utf8_lossy(kind).into_owned());
            at += 12 + len;
        }
        assert_eq!(kinds, vec!["IHDR", "IDAT", "IEND"]);
        assert_eq!(at, png.len(), "no trailing bytes after IEND");
    }

    #[test]
    fn a_write_that_cannot_happen_is_reported_rather_than_fatal() {
        // The gap this closes: `write` used to `expect`, so a full disk or a
        // missing directory took the process down instead of returning.
        let (rgba, w, h) = swatch();
        let nowhere = Path::new("/this/directory/does/not/exist/shot.png");
        let error = write(nowhere, &rgba, w, h).expect_err("no such directory");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn a_write_that_can_happen_lands_the_same_bytes_encode_produced() {
        let (rgba, w, h) = swatch();
        let dir = std::env::temp_dir().join("viewwstudio-png-test");
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("swatch.png");
        write(&path, &rgba, w, h).expect("writing the PNG");
        assert_eq!(
            std::fs::read(&path).expect("reading it back"),
            encode(&rgba, w, h)
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
