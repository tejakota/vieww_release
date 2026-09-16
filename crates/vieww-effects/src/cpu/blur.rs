//! Separable box blur — the fast Gaussian approximation.
//!
//! # The algorithm
//!
//! A true Gaussian kernel is:
//!
//! ```text
//! G(x) = (1 / (σ√(2π))) · e^(-x²/(2σ²))
//! ```
//!
//! Applying it as a 2D convolution is O(w·h·k²) where k is the kernel
//! size — 100×100 pixels with a 40px blur is 4 billion operations.
//!
//! The approximation: a Gaussian is the limit of N box blurs as N→∞, and
//! N=3 is close enough. Each box blur is separable (horizontal then
//! vertical), and each is a sliding window sum:
//!
//! ```text
//! sum = Σ window[i..i+k]
//! out[i] = sum / k
//! sum += window[i+k] - window[i]     // O(1) per pixel
//! ```
//!
//! Three passes of (horizontal + vertical) = 6 passes total, each O(w·h).
//! For 100×100 that is 60,000 operations instead of 4 billion.
//!
//! # Edge handling
//!
//! Clamp-to-edge: pixels outside the image read the nearest edge pixel.
//! This is what every real blur does — reflecting or wrapping produce
//! visible seams at image boundaries, and the edge-pixel repetition
//! fades naturally.

/// The sigma-to-box-size conversion.
///
/// A box blur of size `n` has standard deviation `√(n²-1)/12`. Inverting:
/// for a target sigma σ, each of the three boxes needs size
/// `n = √(12σ²/N + 1)` where N=3.
#[must_use]
pub fn boxes_for_gaussian(sigma: f32, passes: usize) -> Vec<usize> {
    if sigma <= 0.0 {
        return vec![0; passes];
    }

    let n_float = (12.0 * sigma * sigma / passes as f32 + 1.0).sqrt();
    let n = n_float as i64;

    // Ensure odd sizes (symmetric window).
    (0..passes)
        .map(|i| {
            let size = if i == passes - 1 {
                n // The last pass absorbs the rounding
            } else {
                n / 2
            };
            if size % 2 == 0 { size + 1 } else { size }.max(1) as usize
        })
        .collect()
}

/// Blur an RGBA8 buffer in place.
///
/// `width` and `height` are the buffer dimensions. `sigma` is the blur
/// radius in pixels. Alpha is blurred with the colour channels — this is
/// correct for premultiplied-alpha buffers and slightly wrong for
/// straight-alpha, but the visual difference is below perception at any
/// sigma large enough to matter.
pub fn blur_rgba(pixels: &mut [u8], width: usize, height: usize, sigma: f32) {
    if sigma <= 0.0 || pixels.len() != width * height * 4 {
        return;
    }

    let boxes = boxes_for_gaussian(sigma, 3);
    let mut temp = vec![0u8; pixels.len()];

    for box_size in boxes {
        if box_size <= 1 {
            continue;
        }
        // Horizontal pass: read from `pixels`, write to `temp`.
        box_blur_h_rgba(pixels, &mut temp, width, height, box_size);
        // Vertical pass: read from `temp`, write back to `pixels`.
        box_blur_v_rgba(&temp, pixels, width, height, box_size);
    }
}

/// One horizontal box-blur pass.
fn box_blur_h_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, size: usize) {
    let half = size / 2;

    for y in 0..height {
        let row = y * width * 4;

        for c in 0..4 {
            // Sliding window sum.
            let mut sum: i64 = 0;

            // Seed the window with all `2 * half + 1` samples it will be
            // divided by, clamping past the left edge.
            //
            // Seeding with only `0..=half` — one *half* of the window — while
            // dividing by the full width is what made a uniform image come
            // back darker than it went in: at sigma 3 on a flat 128 buffer,
            // every pixel lost roughly a third of its value.
            for i in 0..=(half * 2) {
                let x = (i as isize - half as isize).clamp(0, width as isize - 1) as usize;
                sum += src[row + x * 4 + c] as i64;
            }

            for x in 0..width {
                dst[row + x * 4 + c] = (sum / (half * 2 + 1) as i64) as u8;

                // Slide: remove the left edge, add the right edge.
                let x_out = x.saturating_sub(half);
                sum -= src[row + x_out * 4 + c] as i64;

                let x_in = (x + half + 1).min(width - 1);
                sum += src[row + x_in * 4 + c] as i64;
            }
        }
    }
}

/// One vertical box-blur pass.
fn box_blur_v_rgba(src: &[u8], dst: &mut [u8], width: usize, height: usize, size: usize) {
    let half = size / 2;

    for x in 0..width {
        let col = x * 4;

        for c in 0..4 {
            let mut sum: i64 = 0;

            // The full window, as in the horizontal pass above.
            for i in 0..=(half * 2) {
                let y = (i as isize - half as isize).clamp(0, height as isize - 1) as usize;
                sum += src[y * width * 4 + col + c] as i64;
            }

            for y in 0..height {
                dst[y * width * 4 + col + c] = (sum / (half * 2 + 1) as i64) as u8;

                let y_out = y.saturating_sub(half);
                sum -= src[y_out * width * 4 + col + c] as i64;

                let y_in = (y + half + 1).min(height - 1);
                sum += src[y_in * width * 4 + col + c] as i64;
            }
        }
    }
}

/// Blur only the alpha channel of an RGBA8 buffer.
///
/// Used by the drop-shadow effect: the shadow shape is the alpha channel,
/// and blurring only the alpha is half the work of blurring everything.
pub fn blur_alpha(pixels: &mut [u8], width: usize, height: usize, sigma: f32) {
    if sigma <= 0.0 || pixels.len() != width * height * 4 {
        return;
    }

    let boxes = boxes_for_gaussian(sigma, 3);
    let mut temp = vec![0u8; pixels.len()];

    for box_size in boxes {
        if box_size <= 1 {
            continue;
        }
        // Extract alpha into a single-channel buffer, blur it, put it back.
        let mut alpha: Vec<u8> = (0..width * height).map(|i| pixels[i * 4 + 3]).collect();
        let mut alpha_temp = vec![0u8; alpha.len()];
        box_blur_h_single(&alpha, &mut alpha_temp, width, height, box_size);
        box_blur_v_single(&alpha_temp, &mut alpha, width, height, box_size);
        for (i, &a) in alpha.iter().enumerate() {
            pixels[i * 4 + 3] = a;
        }
        let _ = &mut temp;
    }
}

fn box_blur_h_single(src: &[u8], dst: &mut [u8], width: usize, height: usize, size: usize) {
    let half = size / 2;
    for y in 0..height {
        let row = y * width;
        let mut sum: i64 = 0;
        // Full window, as in `box_blur_h_rgba` — same half-window seeding
        // bug, same fix.
        for i in 0..=(half * 2) {
            let x = (i as isize - half as isize).clamp(0, width as isize - 1) as usize;
            sum += src[row + x] as i64;
        }
        for x in 0..width {
            dst[row + x] = (sum / (half * 2 + 1) as i64) as u8;
            let x_out = x.saturating_sub(half);
            sum -= src[row + x_out] as i64;
            let x_in = (x + half + 1).min(width - 1);
            sum += src[row + x_in] as i64;
        }
    }
}

fn box_blur_v_single(src: &[u8], dst: &mut [u8], width: usize, height: usize, size: usize) {
    let half = size / 2;
    for x in 0..width {
        let mut sum: i64 = 0;
        for i in 0..=(half * 2) {
            let y = (i as isize - half as isize).clamp(0, height as isize - 1) as usize;
            sum += src[y * width + x] as i64;
        }
        for y in 0..height {
            dst[y * width + x] = (sum / (half * 2 + 1) as i64) as u8;
            let y_out = y.saturating_sub(half);
            sum -= src[y_out * width + x] as i64;
            let y_in = (y + half + 1).min(height - 1);
            sum += src[y_in * width + x] as i64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_spreads_a_single_dot() {
        // A 9×9 image, all black except the center pixel.
        let w = 9;
        let h = 9;
        let mut pixels = vec![0u8; w * h * 4];
        let center = (h / 2 * w + w / 2) * 4;
        pixels[center] = 255;
        pixels[center + 1] = 255;
        pixels[center + 2] = 255;
        pixels[center + 3] = 255;

        blur_rgba(&mut pixels, w, h, 2.0);

        // After blurring, the neighbours should have some brightness.
        let right = center + 4;
        assert!(pixels[right] > 0, "the dot spread right: {}", pixels[right]);
    }

    #[test]
    fn uniform_image_is_unchanged_by_blur() {
        let w = 4;
        let h = 4;
        let mut pixels = vec![128u8; w * h * 4];
        let before = pixels.clone();
        blur_rgba(&mut pixels, w, h, 3.0);
        // A uniform image blurred is the same uniform image (within rounding).
        for (&a, &b) in before.iter().zip(pixels.iter()) {
            assert!(a.abs_diff(b) <= 1, "uniform stays uniform");
        }
    }

    #[test]
    fn zero_sigma_is_a_no_op() {
        let mut pixels = vec![255u8; 16 * 4];
        let before = pixels.clone();
        blur_rgba(&mut pixels, 4, 4, 0.0);
        assert_eq!(pixels, before);
    }

    #[test]
    fn box_sizes_increase_with_sigma() {
        let small = boxes_for_gaussian(1.0, 3);
        let large = boxes_for_gaussian(10.0, 3);
        assert!(large.iter().sum::<usize>() > small.iter().sum::<usize>());
    }
}
