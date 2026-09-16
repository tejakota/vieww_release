//! Colour-space tagging for whole images, and the one colour-management
//! operation this crate implements for real: bulk sRGB transfer-function
//! conversion.
//!
//! # What this is not: ICC profile parsing
//!
//! A decoded photo can carry an *arbitrary* embedded ICC profile — camera
//! vendors ship their own, wide-gamut printers use gamut-mapped ones, and
//! the format is a large binary structure of tone curves, chromatic
//! adaptation tables and rendering-intent metadata with no single obvious
//! "the" transform to apply. Parsing that generally, and gamut-mapping
//! (rather than just clamping) a wide profile down to a display's own gamut,
//! is a substantial project in its own right — one with its own dedicated
//! crates (`lcms2` bindings, `qcms`) built by people who do nothing else.
//! Reimplementing a slice of that badly here would be exactly the trap this
//! codebase's own conventions warn against: code that *looks* like colour
//! management but silently mishandles the one embedded profile in ten
//! thousand that is not sRGB-like, which is a worse outcome than not
//! offering profile conversion at all. There is also no test oracle for it
//! here — asserting an ICC transform is correct needs a reference CMM to
//! check against, which is the dependency this module is deliberately not
//! taking on.
//!
//! What *is* real and useful, and what this module actually provides:
//!
//! - [`ColorSpace`] tagging (reusing [`vieww_foundation::ColorSpace`] — the
//!   same sRGB/Display P3 vocabulary [`vieww_foundation::Color`] already
//!   uses, rather than inventing a second one) via [`tag_color_space`], so an
//!   image can at least *say* which of the two colour spaces the overwhelming
//!   majority of real content is actually authored in.
//! - [`convert_to_linear`] and [`convert_to_srgb`]: the sRGB piecewise
//!   transfer function (the curve every one of those two spaces uses —
//!   Display P3 is "sRGB's curve over wider primaries") applied to every
//!   pixel of a whole image buffer. This is the one piece of "colour
//!   management" that has an exact, checkable, universally-agreed
//!   definition (the sRGB spec's own formula) and a real use even without
//!   full ICC support: [`crate::mipmap`]'s box filter averages in encoded
//!   (gamma) space for speed, same as hardware mipmap generation — a caller
//!   that wants a colorimetrically correct chain instead can linearize with
//!   this module first, average, and re-encode after.
//!
//! # Why this duplicates `Color`'s transfer-function constants instead of
//! calling it
//!
//! [`vieww_foundation::Color::to_linear`] and
//! [`vieww_foundation::Color::from_linear`] already implement this exact
//! curve — but as private closures inside those methods, operating one
//! [`Color`](vieww_foundation::Color) at a time. There is no `pub` scalar
//! `srgb_to_linear(u8) -> f32` to call into, and adding one to
//! `vieww-foundation` for a single caller in a hot per-pixel loop with a
//! different shape (a raw byte buffer, not a `Color`) would be a public API
//! added for one use site. The formula is copied here verbatim — same
//! magic constants (`0.04045`, `12.92`, `0.055`, `2.4`) — and cross-checked
//! against `Color::to_linear`/`from_linear` directly in this module's own
//! tests, so a future edit to one curve that forgets the other is caught
//! rather than silently drifting.

use vieww_foundation::{ColorSpace, Image};

/// An image, together with the colour space its channels were authored in.
///
/// A thin pairing, not a transform — see the module docs for why turning
/// this into an *arbitrary* colour-managed pipeline is out of scope. What
/// this buys a caller: an image loaded from, say, a JPEG with an embedded
/// Display P3 marker can be tagged as such and handed to
/// [`vieww_foundation::Color::converted_to`]-style reasoning downstream,
/// instead of every image being silently assumed sRGB.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedImage {
    pub image: Image,
    pub space: ColorSpace,
}

/// Tag `image` as authored in `space`. Free — no pixels are touched.
#[must_use]
pub fn tag_color_space(image: Image, space: ColorSpace) -> TaggedImage {
    TaggedImage { image, space }
}

/// Decode one sRGB-encoded channel (`0.0..=1.0`) to linear light.
///
/// Verbatim the sRGB piece-wise transfer function — see the module docs for
/// why this is not a call into `vieww_foundation::Color`.
fn srgb_channel_to_linear(encoded: f32) -> f32 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of [`srgb_channel_to_linear`]: linear light back to encoded.
fn linear_channel_to_srgb(linear: f32) -> f32 {
    let linear = linear.clamp(0.0, 1.0);
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// Apply `transfer` to the RGB channels of every pixel in `image`, leaving
/// alpha untouched (alpha is a coverage fraction, not a light quantity — the
/// sRGB transfer function has nothing to do with it, and `vieww_foundation`'s
/// straight-alpha convention means it is never premultiplied here either).
fn map_channels(image: &Image, transfer: impl Fn(f32) -> f32) -> Image {
    let src = image.pixels();
    let mut out = vec![0u8; src.len()];
    for (input, output) in src.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        for channel in 0..3 {
            let encoded = f32::from(input[channel]) / 255.0;
            let mapped = transfer(encoded);
            output[channel] = (mapped * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        output[3] = input[3];
    }
    Image::from_rgba8(out, image.width(), image.height())
}

/// Every pixel's RGB channels, decoded from sRGB encoding to linear light.
/// Alpha is passed through unchanged.
///
/// # This is lossy
///
/// The result is still an 8-bit-per-channel [`Image`] — `vieww_foundation`
/// has no 16-bit or float pixel buffer to put a genuinely linear-light image
/// in. Linear light needs far more precision near black than sRGB's
/// perceptually-shaped curve provides (that curve exists specifically to
/// spend more of its 256 codes where the eye is more sensitive), so
/// quantising a linearized image back into `u8` reintroduces banding in
/// dark regions that the original sRGB-encoded bytes did not have. This is
/// still the right operation for the one caller that exists today —
/// linearizing before a mipmap box filter and re-encoding after, where the
/// intermediate loss is well below what the downsampling itself already
/// discards — and the wrong one for anything meant to be stored or
/// inspected as a "linear master" image.
#[must_use]
pub fn convert_to_linear(image: &Image) -> Image {
    map_channels(image, srgb_channel_to_linear)
}

/// The inverse of [`convert_to_linear`]: linear-light channels re-encoded to
/// sRGB. Alpha is passed through unchanged.
#[must_use]
pub fn convert_to_srgb(image: &Image) -> Image {
    map_channels(image, linear_channel_to_srgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Color;

    fn image_from_bytes(bytes: &[u8]) -> Image {
        // One pixel per byte given, replicated across RGB with full alpha,
        // so each test byte is easy to pick back out of the result.
        let mut pixels = Vec::with_capacity(bytes.len() * 4);
        for &byte in bytes {
            pixels.extend_from_slice(&[byte, byte, byte, 200]);
        }
        let width = bytes.len() as u32;
        Image::from_rgba8(pixels, width, 1)
    }

    #[test]
    fn tagging_pairs_the_image_and_space_without_touching_pixels() {
        let image = Image::from_rgba8(vec![1, 2, 3, 4], 1, 1);
        let tagged = tag_color_space(image.clone(), ColorSpace::DisplayP3);
        assert_eq!(tagged.image, image);
        assert_eq!(tagged.space, ColorSpace::DisplayP3);
    }

    /// Exact expected values, computed by hand from the sRGB spec's own
    /// formula (`v <= 0.04045 ? v/12.92 : ((v+0.055)/1.055)^2.4`) at
    /// double precision and rounded to the nearest byte:
    ///
    /// | sRGB byte | linear byte |
    /// |-----------|-------------|
    /// | 0         | 0           |
    /// | 1         | 0           |
    /// | 10        | 1           |
    /// | 12        | 1           |
    /// | 128       | 55          |
    /// | 200       | 147         |
    /// | 255       | 255         |
    #[test]
    fn linear_conversion_matches_the_srgb_formula_at_hand_computed_points() {
        let input = image_from_bytes(&[0, 1, 10, 12, 128, 200, 255]);
        let linear = convert_to_linear(&input);

        let expect_bytes = [0u8, 0, 1, 1, 55, 147, 255];
        for (index, &expected) in expect_bytes.iter().enumerate() {
            let px = &linear.pixels()[index * 4..index * 4 + 4];
            assert_eq!(
                (px[0], px[1], px[2]),
                (expected, expected, expected),
                "byte at index {index}"
            );
        }
    }

    #[test]
    fn alpha_passes_through_conversion_untouched() {
        let input = image_from_bytes(&[128]);
        let linear = convert_to_linear(&input);
        assert_eq!(linear.pixels()[3], 200, "alpha is not a light value");
        let back = convert_to_srgb(&linear);
        assert_eq!(back.pixels()[3], 200);
    }

    #[test]
    fn these_sample_bytes_round_trip_exactly() {
        // Not a general guarantee — see the next test for a byte where it
        // fails — but true for these particular values, which is worth
        // pinning down as a regression check.
        let input = image_from_bytes(&[0, 128, 200, 255]);
        let round_tripped = convert_to_srgb(&convert_to_linear(&input));
        assert_eq!(round_tripped.pixels(), input.pixels());
    }

    /// The banding this module's docs warn about, made concrete: a byte in
    /// the shadows does *not* survive linearize-then-re-encode, because the
    /// linear intermediate needed more precision near black than an 8-bit
    /// buffer has to spend. `10` decodes to linear byte `1`, and `1/255` of
    /// linear-light re-encodes to sRGB byte `13`, not back to `10`.
    #[test]
    fn small_bytes_do_not_round_trip_losslessly() {
        let input = image_from_bytes(&[10]);
        let round_tripped = convert_to_srgb(&convert_to_linear(&input));
        assert_eq!(
            round_tripped.pixels()[0],
            13,
            "the exact banding this module's docs describe"
        );
    }

    /// Cross-checked directly against `vieww_foundation::Color`'s own
    /// (private, per-`Color`) transfer function — see the module docs' "Why
    /// this duplicates" section for why this test exists.
    #[test]
    fn matches_colors_own_transfer_function_for_every_byte_value() {
        for byte in 0..=255u8 {
            let via_color = Color::rgb(byte, byte, byte).to_linear()[0];
            let via_this_module = srgb_channel_to_linear(f32::from(byte) / 255.0);
            assert!(
                (via_color - via_this_module).abs() < 1e-6,
                "byte {byte}: Color gives {via_color}, this module gives {via_this_module}"
            );

            let restored_via_color = Color::from_linear([via_color, via_color, via_color, 1.0]).r;
            let restored_via_this_module = (linear_channel_to_srgb(via_this_module) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            assert_eq!(
                restored_via_color, restored_via_this_module,
                "byte {byte}: re-encoding disagrees with Color::from_linear"
            );
        }
    }
}
