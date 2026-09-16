//! Minimal PNG comparison for snapshot testing.
//!
//! # Why not a full PNG decoder
//!
//! The `NativeRenderer` produces a specific kind of PNG: 8-bit RGBA,
//! non-interlaced, with a predictable filter pattern. A full decoder
//! handles every PNG variation; we need to decode exactly two — the one
//! the renderer produces and the golden it produced on a previous run.
//!
//! # The decoder
//!
//! PNG structure:
//!
//! ```text
//! [8-byte signature]
//! [IHDR chunk: width, height, bit depth, colour type, ...]
//! [IDAT chunks: zlib-compressed scanlines with per-line filters]
//! [IEND chunk]
//! ```
//!
//! We parse the IHDR for dimensions, decompress the IDAT with
//! `miniz_oxide` (a pure-Rust inflate), and undo the per-line filters
//! (None, Sub, Up, Average, Paeth). That gives us raw RGBA bytes to
//! compare.

/// The result of comparing two PNGs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelComparison {
    /// Number of pixels that differ.
    pub differing_pixels: usize,
    /// Total pixels compared (or 0 if dimensions differ).
    pub total_pixels: usize,
    /// The maximum per-channel delta across all differing pixels.
    pub max_delta: u8,
}

/// Compare two PNG files pixel by pixel.
///
/// Returns `None` if either file cannot be decoded.
pub fn compare_pngs(a: &[u8], b: &[u8]) -> Option<PixelComparison> {
    let pixels_a = decode_rgba(a)?;
    let pixels_b = decode_rgba(b)?;

    if pixels_a.len() != pixels_b.len() {
        return Some(PixelComparison {
            differing_pixels: usize::MAX,
            total_pixels: 0,
            max_delta: 255,
        });
    }

    let total = pixels_a.len() / 4;
    let mut differing = 0;
    let mut max_delta = 0u8;

    for (pa, pb) in pixels_a.chunks_exact(4).zip(pixels_b.chunks_exact(4)) {
        let mut pixel_differs = false;
        for (&ca, &cb) in pa.iter().zip(pb.iter()) {
            let delta = ca.abs_diff(cb);
            if delta > 0 {
                pixel_differs = true;
                max_delta = max_delta.max(delta);
            }
        }
        if pixel_differs {
            differing += 1;
        }
    }

    Some(PixelComparison {
        differing_pixels: differing,
        total_pixels: total,
        max_delta,
    })
}

/// Decode a PNG to raw RGBA8 bytes.
///
/// Handles the subset that `NativeRenderer` produces: 8-bit RGBA,
/// non-interlaced. Returns `None` for anything else.
pub fn decode_rgba(png: &[u8]) -> Option<Vec<u8>> {
    // Check the PNG signature.
    if png.len() < 8 || png[0..8] != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }

    let mut width = 0usize;
    let mut height = 0usize;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut idat_data = Vec::new();

    // Walk the chunks.
    let mut pos = 8;
    while pos + 8 <= png.len() {
        let length =
            u32::from_be_bytes([png[pos], png[pos + 1], png[pos + 2], png[pos + 3]]) as usize;
        let chunk_type = &png[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start.checked_add(length)?;

        if data_end > png.len() {
            return None;
        }

        match chunk_type {
            b"IHDR" => {
                if length < 13 {
                    return None;
                }
                let data = &png[data_start..data_end];
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) as usize;
                bit_depth = data[8];
                color_type = data[9];
            }
            b"IDAT" => {
                idat_data.extend_from_slice(&png[data_start..data_end]);
            }
            b"IEND" => break,
            _ => {}
        }

        pos = data_end + 4; // Skip the CRC.
    }

    // We only handle 8-bit RGBA (colour type 6).
    if bit_depth != 8 || color_type != 6 {
        return None;
    }

    // Decompress the IDAT data.
    let decompressed = inflate(&idat_data)?;

    // Unfilter the scanlines.
    let bytes_per_pixel = 4;
    let stride = width * bytes_per_pixel;
    let mut output = vec![0u8; width * height * bytes_per_pixel];

    let mut src_pos = 0;
    for y in 0..height {
        if src_pos >= decompressed.len() {
            return None;
        }
        let filter = decompressed[src_pos];
        src_pos += 1;

        let row_start = y * stride;
        // Split at the row boundary so the previous row (`prev`) and the row
        // being written (`row`) are two non-overlapping slices. Indexing
        // `output` for `up`/`up_left` while `row` borrows it mutably is what
        // the borrow checker rejected, and splitting is the fix it suggests —
        // the two ranges genuinely never overlap.
        let (before, row) = output.split_at_mut(row_start);
        let row = &mut row[..stride];
        let prev: &[u8] = if y > 0 {
            &before[row_start - stride..]
        } else {
            &[]
        };

        for x in 0..stride {
            let raw = decompressed.get(src_pos + x).copied().unwrap_or(0);
            let left = if x >= bytes_per_pixel {
                row[x - bytes_per_pixel] as i16
            } else {
                0
            };
            let up = if y > 0 { prev[x] as i16 } else { 0 };
            let up_left = if y > 0 && x >= bytes_per_pixel {
                prev[x - bytes_per_pixel] as i16
            } else {
                0
            };

            let value = match filter {
                0 => raw as i16,                   // None
                1 => raw as i16 + left,            // Sub
                2 => raw as i16 + up,              // Up
                3 => raw as i16 + (left + up) / 2, // Average
                4 => {
                    // Paeth
                    let p = left + up - up_left;
                    let pa = (p - left).abs();
                    let pb = (p - up).abs();
                    let pc = (p - up_left).abs();
                    let predictor = if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        up_left
                    };
                    raw as i16 + predictor
                }
                _ => raw as i16,
            };

            row[x] = value.clamp(0, 255) as u8;
        }

        src_pos += stride;
    }

    Some(output)
}

/// Inflate a zlib stream (the IDAT payload).
///
/// Delegates to `miniz_oxide`, which is a real DEFLATE implementation.
///
/// What was here before was a stub: a hand-rolled reader that handled only
/// *stored* (uncompressed) DEFLATE blocks and returned `None` for anything
/// else, under a comment saying the real call "would" be
/// `decompress_to_vec(data).ok()`. Every PNG produced by a real encoder uses
/// dynamic Huffman blocks, so that path returned `None` every time and
/// `compare_pngs` silently reported "could not decode" for all genuine input.
/// Its stored-block branch was also wrong — it aligned to a 4-byte boundary
/// with `(pos + 3) & !3` and called that an approximation, where DEFLATE
/// aligns to the next *byte* — so even the case it claimed to cover would
/// have mis-parsed.
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    // `data` still carries the 2-byte zlib header; the `_zlib` variant reads it.
    miniz_oxide::inflate::decompress_to_vec_zlib(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOLID_RED_2X2: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 17, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 132, 25, 96, 12, 0, 71, 202, 7, 249, 103, 89, 110, 183, 0, 0, 0, 0, 73, 69, 78, 68,
        174, 66, 96, 130,
    ];
    const ONE_PIXEL_OFF_2X2: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6,
        0, 0, 0, 114, 182, 13, 36, 0, 0, 0, 20, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 132, 25, 64, 196, 47, 32, 6, 0, 71, 182, 7, 244, 233, 121, 39, 158, 0, 0, 0, 0, 73, 69,
        78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn identical_pngs_report_no_differences() {
        let result = compare_pngs(SOLID_RED_2X2, SOLID_RED_2X2).expect("both decode");
        assert_eq!(result.total_pixels, 4);
        assert_eq!(result.differing_pixels, 0);
        assert_eq!(result.max_delta, 0);
    }

    #[test]
    fn a_single_changed_pixel_is_found() {
        let result = compare_pngs(SOLID_RED_2X2, ONE_PIXEL_OFF_2X2).expect("both decode");
        assert_eq!(result.total_pixels, 4);
        assert_eq!(result.differing_pixels, 1, "exactly one pixel was changed");
        assert_eq!(result.max_delta, 5, "255 -> 250 on the red channel");
    }

    #[test]
    fn undecodable_input_is_none() {
        // The old test here built two identical `vec![0u8; 100]` buffers and
        // asserted `true` inside `if a == b`. It could not fail and it never
        // called into this module. Comparing garbage should report `None`.
        assert!(compare_pngs(&[0u8; 100], &[0u8; 100]).is_none());
    }

    #[test]
    fn decode_returns_rgba_bytes() {
        let pixels = decode_rgba(SOLID_RED_2X2).expect("decodes");
        assert_eq!(pixels.len(), 2 * 2 * 4, "2x2 RGBA");
        assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn png_signature_check() {
        let not_png = vec![0u8; 20];
        assert!(decode_rgba(&not_png).is_none());
    }
}
