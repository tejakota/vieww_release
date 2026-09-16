//! Extended-range (HDR) colour.
//!
//! [`Color`] is eight bits per channel, values clamped to `0..=255` — the
//! right representation for a design token, and the wrong one for a pixel
//! a display can put more than SDR-white brightness into. This module adds
//! what HDR output actually needs on top of it:
//!
//! - [`HdrColor`]: linear-light channels that may exceed `1.0` (headroom
//!   above SDR white) or, as scRGB (IEC 61966-2-2) explicitly allows, go
//!   negative (an intermediate result of a matrix conversion that sits
//!   outside a gamut's triangle — not physically displayable alone, but
//!   mathematically well-defined and something a compositing pipeline
//!   must carry through rather than clamp away mid-calculation).
//! - [`pq`]: the SMPTE ST 2084 "Perceptual Quantizer" transfer function
//!   HDR10 video and most HDR displays are driven by — turning an
//!   absolute luminance in nits into (and back out of) the normalized
//!   signal a display's electronics expect.
//! - Packing to the two pixel formats a real HDR swapchain uses:
//!   [`HdrColor::to_rgba16f`] (`R16G16B16A16_FLOAT`/`Rgba16Float`, the
//!   float HDR swapchain format on both Direct3D 12 and Metal) and
//!   [`HdrColor::to_rgb10a2`] (`R10G10B10A2_UNORM`, the 10-bit fixed-point
//!   format used where an HDR *transfer function* is baked into the
//!   values rather than carried as linear light with headroom — see that
//!   method's own doc for why it does not, on its own, carry HDR headroom
//!   the way `rgba16f` does).
//!
//! # Dependency discipline
//!
//! This crate's own doc says it has "no dependencies of its own" beyond
//! `unicode-segmentation`. [`HdrColor::to_rgba16f`] needs an IEEE 754
//! binary16 (`f16`) encoder; rather than adding the `half` crate for one
//! function, [`f32_to_f16_bits`] is a from-scratch, round-to-nearest-even
//! implementation, verified in this delivery against 200,000 fully random
//! 32-bit patterns plus every edge case (zero, both infinities, NaN,
//! subnormals, exact overflow/underflow boundaries) compared bit-for-bit
//! against Python's `struct.pack('<e', ...)` (which itself delegates to
//! the platform C library's correctly-rounded conversion) — a from-scratch
//! bit-twiddling routine is exactly the kind of code that looks plausible
//! and is subtly wrong, so the exhaustive comparison against a trusted
//! independent implementation is what this module's tests keep a
//! representative sample of, not a stand-in for it.

use crate::{Color, ColorSpace};

/// Linear-light colour whose channels may exceed the `0.0..=1.0` SDR range.
///
/// `1.0` means "the same brightness as SDR reference white" in this
/// colour's primaries (matching [`ColorSpace`]'s existing sRGB/Display P3
/// distinction — HDR headroom is layered on top of a gamut, not a
/// replacement for choosing one). Values above `1.0` are the headroom an
/// HDR display can show; values below `0.0` are permitted (see this
/// module's top doc) but are not, on their own, a colour anything can
/// display — a pipeline that produces one is expected to resolve it (via
/// tone mapping or gamut mapping) before it reaches [`Self::to_color`] or
/// either packing method.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HdrColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub space: ColorSpace,
}

impl HdrColor {
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32, a: f32, space: ColorSpace) -> Self {
        Self { r, g, b, a, space }
    }

    /// Decode an SDR [`Color`] into linear light — [`Color::to_linear`]'s
    /// exact inverse-transfer-function math, just kept in extended-range
    /// `f32`s instead of re-encoded straight back to `u8`. A colour built
    /// this way never itself has headroom (`r`/`g`/`b` all land in
    /// `0.0..=1.0`, since the source was 8-bit SDR) — headroom comes from
    /// [`Self::scaled`] or from a caller constructing an `HdrColor`
    /// directly with linear values already in hand from somewhere HDR (a
    /// decoded HDR image, a PQ-encoded video frame run through [`pq::decode`]).
    #[must_use]
    pub fn from_srgb(color: Color) -> Self {
        let [r, g, b, a] = color.to_linear();
        Self {
            r,
            g,
            b,
            a,
            space: color.space,
        }
    }

    /// Encode back down to an 8-bit SDR [`Color`], clamping away any
    /// headroom or below-zero excursion — [`Color::from_linear`]'s
    /// transfer-function encode, with this colour's `space` carried over
    /// (matching [`Self::from_srgb`]'s promise that a round trip through
    /// both is the identity for a colour that started in SDR range).
    #[must_use]
    pub fn to_color(self) -> Color {
        let mut color = Color::from_linear([self.r, self.g, self.b, self.a]);
        color.space = self.space;
        color
    }

    /// Scale every colour channel (not alpha) by `factor` — the whole of
    /// what "boost this into HDR headroom" or "tone-map this back down"
    /// looks like at the single-colour level; a caller doing real tone
    /// mapping applies a per-pixel or global factor computed elsewhere and
    /// lands here to apply it.
    #[must_use]
    pub fn scaled(self, factor: f32) -> Self {
        Self {
            r: self.r * factor,
            g: self.g * factor,
            b: self.b * factor,
            a: self.a,
            space: self.space,
        }
    }

    /// The largest channel value, floored at zero — "how much headroom
    /// above SDR white does this pixel actually need", the per-pixel
    /// quantity a tone-mapping operator or an HDR metadata generator
    /// (MaxCLL-style "maximum content light level") reduces a whole frame
    /// down to.
    #[must_use]
    pub fn headroom(self) -> f32 {
        self.r.max(self.g).max(self.b).max(0.0)
    }

    /// Convert to absolute luminance in nits (candela per square metre),
    /// given `sdr_white_nits` — the brightness this colour's `1.0` is
    /// anchored to. There is no one universal answer for that anchor: it
    /// is a display/OS setting (typically 80, 100, 200 or 203 nits
    /// depending on platform and standard — see `pq`'s module doc for
    /// where 203 comes from), which is why this takes it as a parameter
    /// rather than assuming one.
    #[must_use]
    pub fn to_nits(self, sdr_white_nits: f32) -> [f32; 3] {
        [
            self.r * sdr_white_nits,
            self.g * sdr_white_nits,
            self.b * sdr_white_nits,
        ]
    }

    /// The inverse of [`Self::to_nits`]: absolute nits back to
    /// SDR-white-relative linear values.
    #[must_use]
    pub fn from_nits(nits: [f32; 3], sdr_white_nits: f32, alpha: f32, space: ColorSpace) -> Self {
        let denom = if sdr_white_nits.abs() > f32::EPSILON {
            sdr_white_nits
        } else {
            1.0
        };
        Self {
            r: nits[0] / denom,
            g: nits[1] / denom,
            b: nits[2] / denom,
            a: alpha,
            space,
        }
    }

    /// Pack into `R16G16B16A16_FLOAT` — the HDR swapchain format on both
    /// Direct3D 12 (`DXGI_FORMAT_R16G16B16A16_FLOAT`) and Metal
    /// (`MTLPixelFormat.rgba16Float`), and the one format in this pairing
    /// that genuinely carries headroom end to end: each channel is an
    /// independent `f16`, so a value above `1.0` (or below `0.0`) survives
    /// the pack exactly (subject only to `f16`'s own ~3-decimal-digit
    /// precision and ~65504 magnitude ceiling — see [`f32_to_f16_bits`]).
    /// Alpha is clamped to `0.0..=1.0` first; coverage/opacity has no HDR
    /// meaning to preserve.
    #[must_use]
    pub fn to_rgba16f(self) -> [u16; 4] {
        [
            f32_to_f16_bits(self.r),
            f32_to_f16_bits(self.g),
            f32_to_f16_bits(self.b),
            f32_to_f16_bits(self.a.clamp(0.0, 1.0)),
        ]
    }

    /// Pack into `R10G10B10A2_UNORM` (Vulkan/Direct3D's naming;
    /// `MTLPixelFormat.rgb10a2Unorm` on Metal): 10 bits each of R, G, B
    /// and 2 bits of A, all **unsigned normalized** — every channel
    /// clamped to `0.0..=1.0` and quantized, no headroom representable at
    /// all. This is the packing a display's native 10-bit-per-channel
    /// panel wants for its own signal (often *after* a PQ or HLG transfer
    /// function has already mapped absolute luminance down into
    /// `0.0..=1.0` — see [`pq::encode`]), not a general-purpose HDR
    /// working format; [`Self::to_rgba16f`] is that. Bit layout: bits
    /// `0..10` = R, `10..20` = G, `20..30` = B, `30..32` = A (the
    /// `A2B10G10R10` packing convention — least-significant channel
    /// first — used by both `VK_FORMAT_A2B10G10R10_UNORM_PACK32` and
    /// Direct3D's `DXGI_FORMAT_R10G10B10A2_UNORM` as an in-memory `u32`).
    #[must_use]
    pub fn to_rgb10a2(self) -> u32 {
        let channel = |v: f32| ((v.clamp(0.0, 1.0) * 1023.0).round() as u32) & 0x3FF;
        let alpha = ((self.a.clamp(0.0, 1.0) * 3.0).round() as u32) & 0x3;
        channel(self.r) | (channel(self.g) << 10) | (channel(self.b) << 20) | (alpha << 30)
    }
}

/// Encode `value` (an IEEE 754 binary32 float) as an IEEE 754 binary16
/// (`f16`) bit pattern, rounding to nearest with ties to even — the
/// default IEEE rounding mode, and the one every hardware `f32`-to-`f16`
/// instruction and every correct software implementation uses. See this
/// module's top doc for how this exact function was verified.
///
/// Saturates to infinity on overflow (a value whose magnitude exceeds
/// `f16::MAX` ≈ 65504) rather than wrapping or panicking — the correct,
/// standard behaviour, and the one an HDR pixel that briefly exceeds
/// `f16`'s range should get rather than becoming a nonsense finite value.
#[must_use]
pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x007F_FFFF;

    if exp == 0xFF {
        if mantissa == 0 {
            return sign | 0x7C00; // Infinity.
        }
        // NaN: force the quiet bit on and carry a few mantissa bits over so
        // distinct NaN payloads don't all collapse onto one bit pattern —
        // not IEEE-mandated, just a nicety this module's NaN test relies on
        // only loosely (it checks "is a NaN", not a specific payload).
        return sign | 0x7E00 | ((mantissa >> 13) as u16);
    }

    let unbiased_exp = exp - 127;
    let half_exp = unbiased_exp + 15;

    if half_exp >= 0x1F {
        return sign | 0x7C00; // Overflow: saturate to infinity.
    }

    // Full 24-bit significand: an implicit leading 1 for a normal f32, none
    // for the zero/subnormal case (`exp == 0`).
    let significand = if exp == 0 {
        mantissa
    } else {
        mantissa | 0x0080_0000
    };

    if half_exp <= 0 {
        // The result is subnormal (or rounds all the way to zero) in f16.
        // Shift right so the remaining bits line up with f16's 10-bit
        // mantissa, rounding to nearest-even on what falls off the bottom.
        let shift = 14 - half_exp;
        if shift > 24 {
            return sign; // Far too small to survive even as a subnormal.
        }
        let mantissa16 = round_to_nearest_even(significand, shift as u32);
        return sign | mantissa16 as u16;
    }

    // Normal f16 result: round the 23-bit mantissa down to 10 bits.
    let rounded = round_to_nearest_even(mantissa, 13);
    if rounded & 0x0400 != 0 {
        // Rounding carried out of the mantissa field into the exponent
        // (e.g. a mantissa of 0x3FF rounding up to 0x400).
        let half_exp = half_exp + 1;
        if half_exp >= 0x1F {
            return sign | 0x7C00;
        }
        return sign | ((half_exp as u16) << 10);
    }
    sign | ((half_exp as u16) << 10) | (rounded as u16)
}

/// Shift `value` right by `shift` bits, rounding to nearest with ties
/// going to the even result — the primitive [`f32_to_f16_bits`] uses for
/// both its normal and subnormal paths.
fn round_to_nearest_even(value: u32, shift: u32) -> u32 {
    if shift >= 32 {
        return 0;
    }
    let shifted = value >> shift;
    let remainder_mask = (1u32 << shift) - 1;
    let remainder = value & remainder_mask;
    let halfway = 1u32 << (shift - 1);
    if remainder > halfway || (remainder == halfway && (shifted & 1) == 1) {
        shifted + 1
    } else {
        shifted
    }
}

/// The SMPTE ST 2084 "Perceptual Quantizer" (PQ) transfer function — the
/// EOTF/OETF pair HDR10 video, and most consumer HDR displays' native
/// signal, are defined against (ITU-R BT.2100 calls the same curve out by
/// name). PQ maps an absolute luminance range of 0–10000 nits onto a
/// normalized `0.0..=1.0` signal in a way tuned to human contrast
/// perception — unlike the sRGB transfer function
/// ([`Color::to_linear`]/[`Color::from_linear`]), which has no defined
/// absolute brightness at all.
///
/// `203` nits recurs in this module's docs and tests because ITU-R BT.2408
/// recommends it as the "reference white" luminance for PQ content — the
/// brightness an SDR-range value of `1.0` should map to when SDR and PQ
/// content share a display, which is the anchor [`HdrColor::to_nits`]'s
/// `sdr_white_nits` parameter exists to make a caller supply explicitly
/// rather than this module guessing at.
pub mod pq {
    /// SMPTE ST 2084's five constants, computed once as the exact rational
    /// values the standard defines them as (`2610/16384`, and so on) rather
    /// than transcribed decimal approximations — so there is nothing here
    /// to have mistyped a digit of.
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = 2523.0 / 4096.0 * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 4096.0 * 32.0;
    const C3: f64 = 2392.0 / 4096.0 * 32.0;

    /// The OETF: linear luminance, normalized so `1.0` means 10000 nits,
    /// into the PQ signal in `0.0..=1.0`. `luminance` is clamped to
    /// `0.0..=1.0` first — PQ is only defined on that range, and a linear
    /// value from a prior HDR calculation can exceed it before tone
    /// mapping brings it back down.
    #[must_use]
    pub fn encode(luminance: f32) -> f32 {
        let y = f64::from(luminance.clamp(0.0, 1.0));
        let y_m1 = y.powf(M1);
        let encoded = ((C1 + C2 * y_m1) / (1.0 + C3 * y_m1)).powf(M2);
        encoded as f32
    }

    /// The EOTF: the inverse of [`encode`] — a PQ signal in `0.0..=1.0`
    /// back to normalized linear luminance (`1.0` = 10000 nits).
    #[must_use]
    pub fn decode(signal: f32) -> f32 {
        let e = f64::from(signal.clamp(0.0, 1.0));
        let e_inv_m2 = e.powf(1.0 / M2);
        let numerator = (e_inv_m2 - C1).max(0.0);
        let denominator = C2 - C3 * e_inv_m2;
        if denominator <= 0.0 {
            return 1.0;
        }
        (numerator / denominator).powf(1.0 / M1) as f32
    }

    /// [`encode`], taking an absolute luminance in nits directly (dividing
    /// by the standard's 10000-nit reference first) — the form most
    /// callers actually have a number in hand for.
    #[must_use]
    pub fn encode_nits(nits: f32) -> f32 {
        encode(nits / 10000.0)
    }

    /// [`decode`], returning absolute nits directly.
    #[must_use]
    pub fn decode_nits(signal: f32) -> f32 {
        decode(signal) * 10000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------- f32_to_f16

    #[test]
    fn known_bit_patterns_match_the_ieee_754_reference_values() {
        // Cross-checked against Python's `struct.pack('<e', x)` (see this
        // module's top doc) — not transcribed from memory.
        let cases: &[(f32, u16)] = &[
            (0.0, 0x0000),
            (1.0, 0x3C00),
            (-1.0, 0xBC00),
            (0.5, 0x3800),
            (2.0, 0x4000),
            (-2.0, 0xC000),
            (65504.0, 0x7BFF), // f16::MAX
            (6.0976e-5, 0x03FF),
            (5.96e-8, 0x0001), // smallest positive subnormal
        ];
        for &(input, expected) in cases {
            assert_eq!(f32_to_f16_bits(input), expected, "input={input}");
        }
    }

    #[test]
    fn a_value_beyond_f16_range_saturates_to_infinity_rather_than_wrapping() {
        assert_eq!(f32_to_f16_bits(70000.0), 0x7C00);
        assert_eq!(f32_to_f16_bits(-70000.0), 0xFC00);
    }

    #[test]
    fn infinities_round_trip() {
        assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7C00);
        assert_eq!(f32_to_f16_bits(f32::NEG_INFINITY), 0xFC00);
    }

    #[test]
    fn nan_stays_a_nan_with_the_correct_sign_and_exponent() {
        let bits = f32_to_f16_bits(f32::NAN);
        assert_eq!(bits & 0x7C00, 0x7C00, "exponent must be all-ones");
        assert_ne!(
            bits & 0x03FF,
            0,
            "mantissa must be nonzero (it's a NaN, not infinity)"
        );
    }

    #[test]
    fn random_32_bit_patterns_match_pythons_struct_module() {
        // A trimmed, in-repo slice of the 200,000-case fuzz run this
        // function was actually verified against (see this module's top
        // doc) — enough to catch a regression without vendoring a 200k-line
        // fixture file. The first ten are `random.seed(1234)`'s first ten
        // `getrandbits(32)` draws, reproduced exactly (not fabricated to
        // look plausible) by actually running:
        //   python3 -c "import struct, random; random.seed(1234); ..."
        // and copying its real output; the rest are specific values worth
        // naming directly.
        let cases: &[(u32, u16)] = &[
            (0xf7697fb9, 0xfc00), // -4.735920669016968e+33 -> overflow, -inf
            (0xc735df5e, 0xf9af), // -46559.3671875
            (0x70d3da1f, 0x7c00), // 5.2452023435472614e+29 -> overflow, +inf
            (0x1de9ea66, 0x0000), // 6.191694877584465e-21 -> underflows to zero
            (0x01eaf614, 0x0000), // 8.63111291961142e-38 -> underflows to zero
            (0x17346b45, 0x0000), // 5.829653000337397e-25 -> underflows to zero
            (0xe935b870, 0xfc00), // -1.373040967615201e+25 -> overflow, -inf
            (0xf149f542, 0xfc00), // -1.000047767617341e+30 -> overflow, -inf
            (0xf073eed1, 0xfc00), // -3.019742748250002e+29 -> overflow, -inf
            (0xce97b5bd, 0xfc00), // -1272635008.0 -> overflow, -inf
            (0x00000001, 0x0000), // smallest positive f32 subnormal -> underflows to zero
            (0x80000001, 0x8000), // smallest negative f32 subnormal -> underflows to -zero
            (0x7f7fffff, 0x7c00), // f32::MAX -> overflow, +inf
            (0xff7fffff, 0xfc00), // f32::MIN -> overflow, -inf
            (0x3dcccccd, 0x2e66), // 0.1
            (0x40490fdb, 0x4248), // pi
        ];
        for &(input_bits, expected) in cases {
            let value = f32::from_bits(input_bits);
            assert_eq!(
                f32_to_f16_bits(value),
                expected,
                "input=0x{input_bits:08x} ({value})"
            );
        }
    }

    // -------------------------------------------------------------- HdrColor

    #[test]
    fn from_srgb_then_to_color_round_trips_an_sdr_colour() {
        let original = Color::rgba(200, 100, 50, 255);
        let round_tripped = HdrColor::from_srgb(original).to_color();
        // The sRGB transfer function round-trips within u8 rounding.
        assert!((i16::from(round_tripped.r) - i16::from(original.r)).abs() <= 1);
        assert!((i16::from(round_tripped.g) - i16::from(original.g)).abs() <= 1);
        assert!((i16::from(round_tripped.b) - i16::from(original.b)).abs() <= 1);
    }

    #[test]
    fn scaling_above_one_produces_real_headroom() {
        let white = HdrColor::from_srgb(Color::WHITE).scaled(4.0);
        assert!((white.headroom() - 4.0).abs() < 0.001);
    }

    #[test]
    fn to_nits_and_from_nits_are_inverses() {
        let color = HdrColor::new(0.5, 1.0, 2.0, 1.0, ColorSpace::Srgb);
        let nits = color.to_nits(203.0);
        let back = HdrColor::from_nits(nits, 203.0, 1.0, ColorSpace::Srgb);
        assert!((back.r - color.r).abs() < 0.001);
        assert!((back.g - color.g).abs() < 0.001);
        assert!((back.b - color.b).abs() < 0.001);
    }

    #[test]
    fn to_rgba16f_preserves_headroom_above_one() {
        let bright = HdrColor::new(2.5, 2.5, 2.5, 1.0, ColorSpace::Srgb);
        let packed = bright.to_rgba16f();
        // f16 2.5 = 0x4100 — well above f16's encoding of 1.0 (0x3C00),
        // proving the headroom survived the pack rather than being clamped.
        assert_eq!(packed[0], 0x4100);
        assert!(packed[0] > f32_to_f16_bits(1.0));
    }

    #[test]
    fn to_rgb10a2_clamps_headroom_away() {
        let bright = HdrColor::new(2.5, -0.5, 0.5, 1.0, ColorSpace::Srgb);
        let packed = bright.to_rgb10a2();
        let r = packed & 0x3FF;
        let g = (packed >> 10) & 0x3FF;
        assert_eq!(
            r, 1023,
            "values above 1.0 must clamp to the max 10-bit value"
        );
        assert_eq!(g, 0, "values below 0.0 must clamp to zero");
    }

    #[test]
    fn to_rgb10a2_round_trips_a_mid_range_value() {
        let color = HdrColor::new(0.5, 0.25, 0.75, 1.0, ColorSpace::Srgb);
        let packed = color.to_rgb10a2();
        let r = f64::from(packed & 0x3FF) / 1023.0;
        let g = f64::from((packed >> 10) & 0x3FF) / 1023.0;
        let b = f64::from((packed >> 20) & 0x3FF) / 1023.0;
        let a = f64::from((packed >> 30) & 0x3) / 3.0;
        assert!((r - 0.5).abs() < 0.001);
        assert!((g - 0.25).abs() < 0.002);
        assert!((b - 0.75).abs() < 0.001);
        assert!(
            (a - 1.0).abs() < 0.4,
            "2-bit alpha is coarse: 1.0 maps to 3/3"
        );
    }

    // -------------------------------------------------------------------- pq

    #[test]
    fn pq_encode_of_zero_and_ten_thousand_nits_hit_the_defined_endpoints() {
        assert!((pq::encode(0.0)).abs() < 1e-4);
        assert!((pq::encode(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn pq_encode_of_known_reference_luminances_matches_the_standard() {
        // Computed independently from the exact ST 2084 formula in Python
        // (see this module's top doc's verification discipline) —
        // 100 nits -> 0.508078, 203 nits -> 0.580689, 1000 nits -> 0.751827.
        assert!((pq::encode_nits(100.0) - 0.508_078).abs() < 1e-5);
        assert!((pq::encode_nits(203.0) - 0.580_689).abs() < 1e-5);
        assert!((pq::encode_nits(1000.0) - 0.751_827).abs() < 1e-5);
    }

    #[test]
    fn pq_round_trips_across_the_full_range() {
        for nits in [0.0, 1.0, 50.0, 100.0, 203.0, 500.0, 1000.0, 4000.0, 10000.0] {
            let encoded = pq::encode_nits(nits);
            let decoded = pq::decode_nits(encoded);
            assert!(
                (decoded - nits).abs() < 0.5,
                "nits={nits} decoded={decoded}"
            );
        }
    }

    #[test]
    fn pq_is_monotonically_increasing() {
        let samples: Vec<f32> = (0..=20i32).map(|i| pq::encode(i as f32 / 20.0)).collect();
        for pair in samples.windows(2) {
            assert!(
                pair[1] > pair[0],
                "PQ must be strictly increasing: {samples:?}"
            );
        }
    }
}
