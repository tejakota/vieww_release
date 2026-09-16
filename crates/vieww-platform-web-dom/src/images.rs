//! Pixels into something an `<img>` can point at.
//!
//! An `Image` widget carries decoded RGBA. The canvas backend hands that
//! straight to the rasteriser; an `<img>` needs a URL, so the pixels are
//! encoded as a PNG once and kept as a `data:` URL against the buffer's
//! address — a page that shows the same screenshot in two places encodes it
//! once, and a rebuild encodes it not at all.

use std::cell::RefCell;
use std::collections::HashMap;

use vieww_foundation::Image as ImageData;

thread_local! {
    static URLS: RefCell<HashMap<usize, Option<String>>> = RefCell::new(HashMap::new());
}

/// A `data:image/png;base64,…` URL for `image`, or `None` if it could not be
/// encoded — in which case the `<img>` is simply empty rather than broken.
#[must_use]
pub fn data_url(image: &ImageData) -> Option<String> {
    let key = image.pixels().as_ptr() as usize;
    URLS.with(|cache| {
        cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| encode(image))
            .clone()
    })
}

fn encode(image: &ImageData) -> Option<String> {
    let png = png_rgba(image.pixels(), image.width(), image.height())?;
    Some(format!("data:image/png;base64,{}", base64(&png)))
}

/// A minimal PNG writer: one `IHDR`, one uncompressed-`deflate` `IDAT`, one
/// `IEND`.
///
/// Deliberately not a dependency. The pixels are already in memory and already
/// decoded; all this has to do is put a container round them that a browser
/// will accept. Stored deflate blocks make the file about the size of the raw
/// pixels, which for a `data:` URL that gzip will compress on the wire is a
/// trade worth making against pulling in an encoder.
fn png_rgba(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if pixels.len() < (width as usize) * (height as usize) * 4 {
        return None;
    }
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter: none
        let start = y * width as usize * 4;
        raw.extend_from_slice(&pixels[start..start + width as usize * 4]);
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    Some(png)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32(kind);
    crc = crc32_continue(crc, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut rest = data;
    while !rest.is_empty() {
        let take = rest.len().min(0xFFFF);
        let last = u8::from(take == rest.len());
        out.push(last);
        out.extend_from_slice(&(take as u16).to_le_bytes());
        out.extend_from_slice(&(!(take as u16)).to_le_bytes());
        out.extend_from_slice(&rest[..take]);
        rest = &rest[take..];
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    // `crc32_continue` takes and returns a *finalised* CRC — it re-inverts on
    // the way in and back out, so chunks can be appended. Zero is the finalised
    // CRC of the empty message, which is the seed a fresh run wants.
    crc32_continue(0, data)
}

fn crc32_continue(crc: u32, data: &[u8]) -> u32 {
    let mut value = crc ^ 0xFFFF_FFFF;
    for byte in data {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = if value & 1 == 1 {
                (value >> 1) ^ 0xEDB8_8320
            } else {
                value >> 1
            };
        }
    }
    value ^ 0xFFFF_FFFF
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for group in data.chunks(3) {
        let b = [
            group[0],
            group.get(1).copied().unwrap_or(0),
            group.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if group.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if group.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_examples() {
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_png_starts_with_the_signature_and_ends_with_iend() {
        let pixels = vec![255u8; 4 * 4];
        let png = png_rgba(&pixels, 2, 2).expect("2x2 is encodable");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn too_few_pixels_is_none_rather_than_a_panic() {
        assert!(png_rgba(&[0, 0, 0, 0], 4, 4).is_none());
    }
}
