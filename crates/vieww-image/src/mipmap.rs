//! Mipmap chain generation: a full-resolution [`Image`] in, a stack of
//! successively half-sized ones out.
//!
//! # Why this lives here and not in a GPU crate
//!
//! Generating the chain is pure arithmetic over bytes a CPU already has —
//! averaging four (or, at odd sizes, a slightly different count of) source
//! texels into one destination texel needs no rasteriser, no shader and no
//! device. The GPU-shaped part of mipmapping — deciding *which* level a
//! given draw samples, and whether a backend would rather have hardware
//! generate the chain itself (`vkCmdBlitImage` chains, or
//! `glGenerateMipmap`) — belongs to whatever owns the texture upload, not
//! here. What belongs here is the one piece that is honestly just CPU work:
//! producing the smaller levels from the top one, deterministically and
//! testably, for a backend that has no hardware generator (software
//! rasterisers, or platforms where regenerating on every upload is not
//! worth the device round-trip).
//!
//! # Box filtering, not something sharper
//!
//! Each destination texel is the average of the source texels that map onto
//! it — a 2x2 box filter at power-of-two sizes, widening slightly at odd
//! sizes (see "Odd dimensions" below). This is *not* the best filter that
//! exists: a Lanczos or Kaiser-windowed sinc filter produces a chain with
//! less ringing and better preserves high-frequency detail one level down.
//! It is, however, exactly what essentially every real-time engine and every
//! piece of mipmap-generation hardware actually does, because a box filter
//! is separable, needs no windowing decisions, can never ring or overshoot
//! (every output value is a true average of inputs already in range), and
//! costs one pass per level. For UI textures — icons, photos scaled for
//! thumbnails, atlas pages — the difference from a sharper filter is not
//! visible; for a game's compressed normal-map chain it might be, and that
//! is a problem for a texture-compression pipeline this crate does not
//! attempt to be.
//!
//! # Odd dimensions
//!
//! Halving an odd dimension the naive way (pairing texels and dropping the
//! leftover column or row) silently discards data — a 5-wide source loses
//! its last column at every level that stays odd, which is not what
//! "downsample" is supposed to mean. Instead each destination index `i` of
//! `dest_dim` claims the source range `[i * src_dim / dest_dim, (i+1) *
//! src_dim / dest_dim)`, widened by one if that range is empty. For a 4-wide
//! source halved to 2, that reproduces plain pairing (ranges `[0,2)` and
//! `[2,4)`); for a 5-wide source halved to 2, the two destination texels
//! average 2 and 3 source texels respectively (`[0,2)` and `[2,5)`) —
//! *every* source column contributes to exactly one destination texel, none
//! is dropped. The chain still terminates: `dest_dim = max(1, src_dim / 2)`
//! is strictly smaller than `src_dim` for every `src_dim > 1`, so each level
//! is strictly smaller in every dimension that is not already `1`, and
//! generation stops the moment a level is `1x1`.
//!
//! # Straight alpha, and why premultiplying matters here
//!
//! [`vieww_foundation::Image`] stores straight (non-premultiplied) alpha —
//! see its own docs, and `vieww-asset`'s decoder, which decodes to the same
//! convention. Averaging straight-alpha channels directly is a real bug, not
//! a cosmetic one: a fully opaque red texel averaged with a fully
//! transparent *black* one (a common edge case — anti-aliased sprite edges,
//! or padding around a packed icon) naively produces a muddy, half-red,
//! half-*black* result, because the transparent texel's meaningless RGB
//! value still pulls the average down. The fix is standard: convert each
//! source texel to premultiplied form (`rgb * alpha`) before averaging,
//! average premultiplied channels and alpha separately, then divide back out
//! by the averaged alpha. A transparent texel then contributes `(0, 0, 0,
//! 0)` to the sum — its meaningless colour cannot leak into a visible pixel
//! — while a texel with real alpha still counts fully. See the
//! `premultiplication_prevents_a_transparent_texels_black_from_leaking` test
//! for the exact numbers.

use vieww_foundation::Image;

/// The full mipmap chain for `image`: level 0 is `image` itself, and each
/// following level is half the width and half the height of the one before
/// (rounded down, never below `1`), ending at a `1x1` level.
///
/// A `1x1` source produces a chain of exactly one level (itself); nothing
/// asked for a level smaller than one texel.
#[must_use]
pub fn generate_mip_chain(image: &Image) -> Vec<Image> {
    let mut chain = vec![image.clone()];
    loop {
        let last = chain.last().expect("chain always has at least one level");
        if last.width() <= 1 && last.height() <= 1 {
            break;
        }
        chain.push(downsample_once(last));
    }
    chain
}

/// One level down: half width, half height (rounded down, floored at `1`).
#[must_use]
pub fn downsample_once(image: &Image) -> Image {
    let src_width = image.width();
    let src_height = image.height();
    let dest_width = (src_width / 2).max(1);
    let dest_height = (src_height / 2).max(1);

    let x_ranges = source_ranges(src_width, dest_width);
    let y_ranges = source_ranges(src_height, dest_height);
    let src = image.pixels();

    let mut out = vec![0u8; dest_width as usize * dest_height as usize * 4];
    for (dest_y, &(y0, y1)) in y_ranges.iter().enumerate() {
        for (dest_x, &(x0, x1)) in x_ranges.iter().enumerate() {
            let (r, g, b, a) = average_block(src, src_width, x0, x1, y0, y1);
            let index = (dest_y as u32 * dest_width + dest_x as u32) as usize * 4;
            out[index] = r;
            out[index + 1] = g;
            out[index + 2] = b;
            out[index + 3] = a;
        }
    }
    Image::from_rgba8(out, dest_width, dest_height)
}

/// For each of `dest_dim` destination indices, the half-open `[start, end)`
/// range of source indices that average into it. See the module docs' "Odd
/// dimensions" section.
fn source_ranges(src_dim: u32, dest_dim: u32) -> Vec<(u32, u32)> {
    (0..dest_dim)
        .map(|i| {
            let start = i * src_dim / dest_dim;
            let end = ((i + 1) * src_dim / dest_dim).max(start + 1).min(src_dim);
            (start, end)
        })
        .collect()
}

/// The premultiplied-then-unpremultiplied average of the straight-alpha RGBA8
/// block `[x0,x1) x [y0,y1)` of `src` (a `src_width`-wide RGBA8 buffer).
///
/// See the module docs' "Straight alpha" section for why this premultiplies
/// rather than averaging the four channels independently.
fn average_block(
    src: &[u8],
    src_width: u32,
    x0: u32,
    x1: u32,
    y0: u32,
    y1: u32,
) -> (u8, u8, u8, u8) {
    let count = f64::from((x1 - x0) * (y1 - y0));
    let mut premult_sum = [0.0f64; 3];
    let mut alpha_sum = 0.0f64;

    for y in y0..y1 {
        for x in x0..x1 {
            let index = ((y * src_width + x) * 4) as usize;
            let alpha = f64::from(src[index + 3]);
            let alpha_fraction = alpha / 255.0;
            for channel in 0..3 {
                premult_sum[channel] += f64::from(src[index + channel]) * alpha_fraction;
            }
            alpha_sum += alpha;
        }
    }

    let avg_alpha = alpha_sum / count;
    let to_byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    let rgb = if avg_alpha > 0.0 {
        let unpremultiply = |premult: f64| to_byte(premult / count * 255.0 / avg_alpha);
        [
            unpremultiply(premult_sum[0]),
            unpremultiply(premult_sum[1]),
            unpremultiply(premult_sum[2]),
        ]
    } else {
        // Every texel in the block is fully transparent: there is no colour
        // to recover, and any value is as good as another. Zero matches the
        // convention `vieww_foundation::Image::pixels` documents nowhere but
        // that `Color::TRANSPARENT` uses for the same reason.
        [0, 0, 0]
    };

    (rgb[0], rgb[1], rgb[2], to_byte(avg_alpha))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an RGBA8 image from a function of (x, y), alpha fixed at 255.
    fn opaque_image(width: u32, height: u32, color_at: impl Fn(u32, u32) -> (u8, u8, u8)) -> Image {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let (r, g, b) = color_at(x, y);
                let index = ((y * width + x) * 4) as usize;
                pixels[index] = r;
                pixels[index + 1] = g;
                pixels[index + 2] = b;
                pixels[index + 3] = 255;
            }
        }
        Image::from_rgba8(pixels, width, height)
    }

    /// A 4x4 image whose four 2x2 blocks each hold four known, distinct red
    /// values (green fixed at 100, blue fixed at 50, alpha opaque). Hand-summed
    /// block averages:
    ///
    /// - top-left block:     (0 + 4 + 8 + 12) / 4     = 6
    /// - top-right block:    (20 + 24 + 28 + 32) / 4  = 26
    /// - bottom-left block:  (40 + 44 + 48 + 52) / 4  = 46
    /// - bottom-right block: (60 + 64 + 68 + 72) / 4  = 66
    fn checkerboard_4x4() -> Image {
        opaque_image(4, 4, |x, y| {
            let block_index = match (x / 2, y / 2) {
                (0, 0) => 0,
                (1, 0) => 1,
                (0, 1) => 2,
                _ => 3,
            };
            let local = match (x % 2, y % 2) {
                (0, 0) => 0,
                (1, 0) => 1,
                (0, 1) => 2,
                _ => 3,
            };
            let bases = [0u8, 20, 40, 60];
            (bases[block_index] + local * 4, 100, 50)
        })
    }

    #[test]
    fn level_zero_is_the_original_image_unchanged() {
        let source = checkerboard_4x4();
        let chain = generate_mip_chain(&source);
        assert_eq!(chain[0], source);
    }

    #[test]
    fn a_4x4_image_downsamples_to_2x2_exact_block_averages() {
        let chain = generate_mip_chain(&checkerboard_4x4());
        assert_eq!(chain.len(), 3, "4x4 -> 2x2 -> 1x1");

        let level1 = &chain[1];
        assert_eq!((level1.width(), level1.height()), (2, 2));
        let px = level1.pixels();
        let pixel_at = |x: u32, y: u32| {
            let i = ((y * 2 + x) * 4) as usize;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };
        assert_eq!(pixel_at(0, 0), (6, 100, 50, 255), "top-left block average");
        assert_eq!(
            pixel_at(1, 0),
            (26, 100, 50, 255),
            "top-right block average"
        );
        assert_eq!(
            pixel_at(0, 1),
            (46, 100, 50, 255),
            "bottom-left block average"
        );
        assert_eq!(
            pixel_at(1, 1),
            (66, 100, 50, 255),
            "bottom-right block average"
        );
    }

    #[test]
    fn the_2x2_level_downsamples_to_the_exact_overall_average() {
        // (6 + 26 + 46 + 66) / 4 = 144 / 4 = 36, an exact division.
        let chain = generate_mip_chain(&checkerboard_4x4());
        let level2 = &chain[2];
        assert_eq!((level2.width(), level2.height()), (1, 1));
        assert_eq!(level2.pixels(), &[36, 100, 50, 255]);
    }

    #[test]
    fn premultiplication_prevents_a_transparent_texels_black_from_leaking() {
        // A 2x2 block: opaque red diagonally paired with fully transparent
        // (0,0,0,0) — a common case at a sprite's antialiased edge.
        let pixels = vec![
            255, 0, 0, 255, // opaque red
            0, 0, 0, 0, // transparent
            255, 0, 0, 255, // opaque red
            0, 0, 0, 0, // transparent
        ];
        let image = Image::from_rgba8(pixels, 2, 2);
        let down = downsample_once(&image);

        assert_eq!((down.width(), down.height()), (1, 1));
        // Premultiplied sum: r*alpha/255 is 255,0,255,0 -> avg 127.5;
        // alpha sum is 255,0,255,0 -> avg 127.5. Unpremultiply:
        // 127.5 * 255 / 127.5 = 255 exactly. Alpha rounds 127.5 -> 128.
        assert_eq!(
            down.pixels(),
            &[255, 0, 0, 128],
            "hue must survive averaging with a transparent neighbour"
        );

        // The bug this guards against: naively averaging the raw bytes
        // (including the transparent texel's meaningless black) gives a
        // darkened, muddy red instead.
        assert_ne!(
            down.pixels(),
            &[127, 0, 0, 128],
            "that would be the naive (wrong) per-channel average"
        );
    }

    #[test]
    fn odd_dimensions_round_down_and_terminate_at_1x1() {
        // Uniform colour, so every box-filtered level must reproduce it
        // exactly regardless of how the odd dimensions get carved up.
        let color = (200u8, 10u8, 90u8);
        let source = opaque_image(5, 3, |_, _| color);

        let chain = generate_mip_chain(&source);
        let sizes: Vec<(u32, u32)> = chain.iter().map(|i| (i.width(), i.height())).collect();
        assert_eq!(
            sizes,
            vec![(5, 3), (2, 1), (1, 1)],
            "5/2=2, 3/2=1, then 2/2=1 and 1/2 floors at 1 — three levels, no infinite loop"
        );

        for level in &chain {
            let px = level.pixels();
            for chunk in px.chunks_exact(4) {
                assert_eq!(
                    (chunk[0], chunk[1], chunk[2], chunk[3]),
                    (color.0, color.1, color.2, 255),
                    "a uniform image must downsample to the same uniform colour"
                );
            }
        }
    }

    #[test]
    fn a_1x1_image_produces_a_chain_of_exactly_itself() {
        let source = Image::from_rgba8(vec![9, 8, 7, 6], 1, 1);
        let chain = generate_mip_chain(&source);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0], source);
    }

    #[test]
    fn source_ranges_cover_every_source_index_exactly_once() {
        // The property odd-dimension handling depends on: no source column
        // is dropped and none is double-counted.
        for (src, dest) in [(5u32, 2u32), (7, 3), (9, 4), (4, 2), (1, 1)] {
            let ranges = source_ranges(src, dest);
            assert_eq!(ranges.len(), dest as usize);
            assert_eq!(
                ranges[0].0, 0,
                "coverage must start at the first source index"
            );
            assert_eq!(
                ranges.last().unwrap().1,
                src,
                "coverage must end at the last source index"
            );
            for window in ranges.windows(2) {
                assert_eq!(window[0].1, window[1].0, "ranges must be contiguous");
            }
            for &(start, end) in &ranges {
                assert!(
                    end > start,
                    "every destination texel must average at least one source texel"
                );
            }
        }
    }
}
