//! Per-pixel colour matrix application.
//!
//! # The math
//!
//! Each output channel is a linear combination of the input channels plus
//! a constant:
//!
//! ```text
//! R' = m[0]·R + m[1]·G + m[2]·B + m[3]·A + m[4]
//! G' = m[5]·R + m[6]·G + m[7]·B + m[8]·A + m[9]
//! B' = m[10]·R + m[11]·G + m[12]·B + m[13]·A + m[14]
//! A' = m[15]·R + m[16]·G + m[17]·B + m[18]·A + m[19]
//! ```
//!
//! With input in `[0, 1]` and output clamped back to `[0, 1]`.
//!
//! # Performance
//!
//! 20 multiplies + 16 adds + 4 clamps per pixel. A 512×512 image is
//! 262k pixels × 40 operations = 10.5M operations. On a 2020 CPU that is
//! under a millisecond. The autovectorizer handles this well — the inner
//! loop is four independent chains.

use vieww_foundation::Rect;

/// Apply a 5×4 colour matrix to an RGBA8 buffer, in place.
///
/// The matrix is row-major, 20 values: 4 rows × (4 coefficients + 1 constant).
pub fn apply_color_matrix(pixels: &mut [u8], matrix: &[f32; 20]) {
    // Pre-compute the row as f32 to avoid the bounds check in the hot loop.
    let m = [
        [matrix[0], matrix[1], matrix[2], matrix[3], matrix[4]],
        [matrix[5], matrix[6], matrix[7], matrix[8], matrix[9]],
        [matrix[10], matrix[11], matrix[12], matrix[13], matrix[14]],
        [matrix[15], matrix[16], matrix[17], matrix[18], matrix[19]],
    ];

    for chunk in pixels.chunks_exact_mut(4) {
        let r = chunk[0] as f32 / 255.0;
        let g = chunk[1] as f32 / 255.0;
        let b = chunk[2] as f32 / 255.0;
        let a = chunk[3] as f32 / 255.0;

        for c in 0..4 {
            let value = m[c][0] * r + m[c][1] * g + m[c][2] * b + m[c][3] * a + m[c][4];
            chunk[c] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
}

/// Apply a matrix to a rectangular subregion of a larger buffer.
///
/// For filters that affect only part of the image (a filtered widget
/// within a screen). `pixels` is the full buffer; `rect` is the region
/// in pixel coordinates.
pub fn apply_color_matrix_region(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    rect: Rect,
    matrix: &[f32; 20],
) {
    let x0 = rect.left.max(0.0) as usize;
    let y0 = rect.top.max(0.0) as usize;
    let x1 = (rect.right as usize).min(width);
    let y1 = (rect.bottom as usize).min(height);

    let m = [
        [matrix[0], matrix[1], matrix[2], matrix[3], matrix[4]],
        [matrix[5], matrix[6], matrix[7], matrix[8], matrix[9]],
        [matrix[10], matrix[11], matrix[12], matrix[13], matrix[14]],
        [matrix[15], matrix[16], matrix[17], matrix[18], matrix[19]],
    ];

    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * width + x) * 4;
            let r = pixels[i] as f32 / 255.0;
            let g = pixels[i + 1] as f32 / 255.0;
            let b = pixels[i + 2] as f32 / 255.0;
            let a = pixels[i + 3] as f32 / 255.0;

            for c in 0..4 {
                let value = m[c][0] * r + m[c][1] * g + m[c][2] * b + m[c][3] * a + m[c][4];
                pixels[i + c] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_leaves_pixels_unchanged() {
        let mut pixels = vec![
            255, 0, 0, 255, //
            0, 255, 0, 255, //
            0, 0, 255, 255, //
            128, 128, 128, 255,
        ];
        let before = pixels.clone();
        let identity: [f32; 20] = [
            1.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ];
        apply_color_matrix(&mut pixels, &identity);
        assert_eq!(pixels, before);
    }

    #[test]
    fn brightness_doubles() {
        let mut pixels = vec![100, 100, 100, 255];
        let double: [f32; 20] = [
            2.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 2.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 2.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ];
        apply_color_matrix(&mut pixels, &double);
        assert_eq!(pixels[0], 200, "100 × 2 = 200");
    }

    #[test]
    fn values_are_clamped() {
        let mut pixels = vec![200, 200, 200, 255];
        let triple: [f32; 20] = [
            3.0, 0.0, 0.0, 0.0, 0.0, //
            0.0, 3.0, 0.0, 0.0, 0.0, //
            0.0, 0.0, 3.0, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ];
        apply_color_matrix(&mut pixels, &triple);
        assert_eq!(pixels[0], 255, "200 × 3 clamps to 255");
    }

    #[test]
    fn grayscale_produces_equal_channels() {
        let mut pixels = vec![255, 0, 0, 255]; // Pure red
        let gray: [f32; 20] = [
            0.299, 0.587, 0.114, 0.0, 0.0, //
            0.299, 0.587, 0.114, 0.0, 0.0, //
            0.299, 0.587, 0.114, 0.0, 0.0, //
            0.0, 0.0, 0.0, 1.0, 0.0,
        ];
        apply_color_matrix(&mut pixels, &gray);
        // Luminance of pure red = 0.299 × 255 ≈ 76
        assert!(pixels[0].abs_diff(pixels[1]) <= 1);
        assert!(pixels[1].abs_diff(pixels[2]) <= 1);
    }
}
