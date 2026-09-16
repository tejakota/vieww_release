use std::fmt;

/// Which primaries a colour's channels are measured against.
///
/// # Why this is on the colour and not on the surface
///
/// A design system holds *values*, and a value that does not say which gamut it
/// was picked in is ambiguous the moment anything wide-gamut is in the room:
/// `#00FF00` means one green on an sRGB display and a **noticeably** more
/// saturated one on a P3 panel, and every modern phone is the second. CSS made
/// the same call with `color(display-p3 …)`, and for the same reason — the
/// alternative is a per-surface mode flag that every token has to be read
/// against, which is a fact stored in the wrong place.
///
/// # What this does and does not buy
///
/// It buys **correctness of intent**: a P3 colour survives being written down,
/// converts exactly into whatever the backend can show, and is never silently
/// reinterpreted. It does not buy extra *precision* — the channels are still
/// eight bits, which is what a token is and what a hex literal can carry, and
/// eight bits across the wider P3 volume does band on a gradient. Float
/// interpolation happens in [`Color::lerp_oklab`] and
/// [`Color::lerp_linear`]; only the endpoints are quantised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorSpace {
    /// The web's, and the default. Every `hex`, `rgb` and `rgba` literal.
    #[default]
    Srgb,
    /// Display P3: sRGB's transfer function over DCI-P3 primaries, which is
    /// what Apple's displays, most recent Android flagships and every iOS
    /// design tool work in.
    DisplayP3,
}

impl ColorSpace {
    /// The 3×3 matrix taking **linear** channels in this space to linear sRGB.
    ///
    /// Row-major. Identity for sRGB, so the conversion is free where nothing
    /// asked for a wider gamut.
    #[must_use]
    pub const fn to_linear_srgb(self) -> [[f32; 3]; 3] {
        match self {
            Self::Srgb => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            // P3 → XYZ(D65) → sRGB, folded into one matrix. Values from the
            // CSS Color 4 conversion appendix, which is the definition every
            // browser implements.
            Self::DisplayP3 => [
                [1.224_940_2, -0.224_940_18, 0.0],
                [-0.042_056_955, 1.042_056_9, 0.0],
                [-0.019_637_555, -0.078_636_04, 1.098_273_6],
            ],
        }
    }

    /// The inverse: linear sRGB into this space's linear channels.
    #[must_use]
    pub const fn from_linear_srgb(self) -> [[f32; 3]; 3] {
        match self {
            Self::Srgb => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            Self::DisplayP3 => [
                [0.822_461_96, 0.177_538_04, 0.0],
                [0.033_194_2, 0.966_805_8, 0.0],
                [0.017_082_632, 0.072_397_44, 0.910_519_9],
            ],
        }
    }
}

/// A straight (non-premultiplied) 8-bit color with alpha, in a named space.
///
/// Premultiplication is a paint-layer concern (Phase 4) and is deliberately not
/// baked in here — the widget layer should be able to describe a color without
/// knowing how the GPU will blend it.
///
/// # The space is part of the value
///
/// [`ColorSpace::Srgb`] unless a constructor says otherwise, so every existing
/// `hex`, `rgb` and `rgba` means exactly what it always did. [`Color::p3`] and
/// [`Color::p3a`] are the wide-gamut door; see [`ColorSpace`] for why the tag
/// lives here rather than on the surface being drawn to.
///
/// # Interpolation
///
/// Three, and the default is deliberate. [`lerp`](Self::lerp) mixes the encoded
/// sRGB channels, which is what CSS and every design tool do and therefore what a
/// designer's expectation is calibrated against. [`lerp_linear`](Self::lerp_linear)
/// mixes light, which is what a *physical* fade does. [`lerp_oklab`](Self::lerp_oklab)
/// mixes perceptually, which is the one that keeps a gradient's midpoint from
/// going muddy. None is "correct" in the abstract; the docs on each say which
/// question it answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    /// Which primaries [`r`](Self::r), [`g`](Self::g) and [`b`](Self::b) are
    /// measured against. See [`ColorSpace`].
    pub space: ColorSpace,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 255, 0);
    pub const BLUE: Self = Self::rgb(0, 0, 255);

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r,
            g,
            b,
            a,
            space: ColorSpace::Srgb,
        }
    }

    /// The same channels, read against **Display P3** primaries.
    ///
    /// `Color::p3(0, 255, 0)` is a green sRGB cannot reach. On a display that
    /// cannot show it the backend converts and it lands on sRGB's green; on one
    /// that can, it is the wider one — and either way the *value* said which it
    /// meant.
    #[must_use]
    pub const fn p3a(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r,
            g,
            b,
            a,
            space: ColorSpace::DisplayP3,
        }
    }

    /// [`p3a`](Self::p3a), fully opaque.
    #[must_use]
    pub const fn p3(r: u8, g: u8, b: u8) -> Self {
        Self::p3a(r, g, b, 255)
    }

    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    /// A color from a `0xAARRGGBB` literal.
    #[must_use]
    pub const fn argb(value: u32) -> Self {
        Self::rgba(
            ((value >> 16) & 0xFF) as u8,
            ((value >> 8) & 0xFF) as u8,
            (value & 0xFF) as u8,
            ((value >> 24) & 0xFF) as u8,
        )
    }

    /// A color from a `0xRRGGBB` literal, fully opaque.
    #[must_use]
    pub const fn hex(value: u32) -> Self {
        Self::argb(value | 0xFF00_0000)
    }

    /// `true` if this color contributes nothing when painted.
    ///
    /// Phase 4 uses this to drop no-op draw calls before they reach the GPU.
    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }

    /// This color with its alpha replaced.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// This color composited over `under`, as painting one on the other would.
    ///
    /// Ordinary source-over, done here rather than by drawing two rectangles:
    /// a control that wants to look pressed wants *one* fill of a shifted
    /// colour, not a second layer over the first with its own rounded corners
    /// to keep in step and its own entry in the command stream.
    ///
    /// Non-premultiplied throughout, matching the rest of this type.
    #[must_use]
    pub fn over(self, under: Self) -> Self {
        // Both operands are read in the destination's space, so a P3 wash over
        // an sRGB surface composites against the colour that is actually there
        // rather than against the same numbers meaning something else.
        let self_ = self.converted_to(under.space);
        let (src, dst) = (f32::from(self_.a) / 255.0, f32::from(under.a) / 255.0);
        let out = src + dst * (1.0 - src);
        if out <= 0.0 {
            // Both fully transparent: there is no colour to recover, and any
            // channel value is as good as another.
            return Self::TRANSPARENT;
        }
        let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
        let blend =
            |s: u8, d: u8| channel((f32::from(s) * src + f32::from(d) * dst * (1.0 - src)) / out);
        Self {
            r: blend(self_.r, under.r),
            g: blend(self_.g, under.g),
            b: blend(self_.b, under.b),
            a: channel(out * 255.0),
            space: under.space,
        }
    }

    // --------------------------------------------------------------- gamut

    /// The same colour, expressed against `space`'s primaries.
    ///
    /// A no-op when the spaces already match, which is the overwhelmingly
    /// common case and costs one comparison. Otherwise: decode the transfer
    /// function, apply the two matrices, re-encode, and **clamp** — a colour
    /// outside the destination gamut has no exact answer, and clipping is what
    /// CSS specifies and what every browser does. It is not gamut *mapping*,
    /// which preserves relative saturation across a whole palette and is a
    /// larger piece of work than one colour can decide alone.
    #[must_use]
    pub fn converted_to(self, space: ColorSpace) -> Self {
        if self.space == space {
            return self;
        }
        let [r, g, b, a] = self.to_linear();
        let to_srgb = self.space.to_linear_srgb();
        let from_srgb = space.from_linear_srgb();
        let apply = |m: [[f32; 3]; 3], v: [f32; 3]| {
            [
                m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
                m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
                m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
            ]
        };
        let linear = apply(from_srgb, apply(to_srgb, [r, g, b]));
        let mut out = Self::from_linear([linear[0], linear[1], linear[2], a]);
        out.space = space;
        out
    }

    /// The same colour in sRGB, whatever it was in.
    ///
    /// What a backend that cannot address a wide gamut asks for.
    #[must_use]
    pub fn to_srgb(self) -> Self {
        self.converted_to(ColorSpace::Srgb)
    }

    // ------------------------------------------------------- linear light

    /// The channels as **linear-light** floats, alpha last and left encoded.
    ///
    /// This is the space physical light adds in, and the one a renderer
    /// composites in. `to_f32_array` is the *encoded* channels, which is what a
    /// shader wants when it will do its own decode — the two are different and
    /// mixing them up produces a picture that is subtly too dark in the
    /// midtones and correct at both ends, which is exactly the bug nobody sees.
    #[must_use]
    pub fn to_linear(self) -> [f32; 4] {
        let decode = |c: u8| {
            let v = f32::from(c) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        [
            decode(self.r),
            decode(self.g),
            decode(self.b),
            f32::from(self.a) / 255.0,
        ]
    }

    /// The inverse of [`to_linear`](Self::to_linear), in this colour's own
    /// space (or sRGB, on the free function).
    #[must_use]
    pub fn from_linear(channels: [f32; 4]) -> Self {
        let encode = |v: f32| {
            let v = v.clamp(0.0, 1.0);
            let encoded = if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..=1 and scaled, so the product is 0..=255"
            )]
            {
                (encoded * 255.0).round().clamp(0.0, 255.0) as u8
            }
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "alpha is clamped to 0..=1"
        )]
        let alpha = (channels[3].clamp(0.0, 1.0) * 255.0).round() as u8;
        Self::rgba(
            encode(channels[0]),
            encode(channels[1]),
            encode(channels[2]),
            alpha,
        )
    }

    // ------------------------------------------------------ interpolation

    /// Mix towards `other`, in the **encoded** channels.
    ///
    /// The default, and deliberately: it is what CSS gradients, the classic
    /// colour lerp and every design tool's colour picker do, so it is the
    /// behaviour a designer's expectation was calibrated against. It is not
    /// physically correct — see [`lerp_linear`](Self::lerp_linear) — and for
    /// two colours of similar lightness the difference is invisible, which is
    /// most of the mixes an interface actually does.
    ///
    /// `t` is clamped. The result is in `self`'s space, with `other` converted
    /// into it first.
    #[must_use]
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let other = other.converted_to(self.space);
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a lerp between two u8s under a clamped t stays in range"
            )]
            {
                (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8
            }
        };
        let mut out = Self::rgba(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
            mix(self.a, other.a),
        );
        out.space = self.space;
        out
    }

    /// Mix towards `other` in **linear light**.
    ///
    /// What actually happens when two lights add, and therefore the right
    /// answer for a fade *to black*, a shadow, or anything modelling
    /// illumination. Against [`lerp`](Self::lerp) it holds the midtones up: a
    /// half-way mix of black and white is `#BCBCBC` here and `#808080` there,
    /// and the second is the one that looks like a hole in a gradient.
    #[must_use]
    pub fn lerp_linear(self, other: Self, t: f32) -> Self {
        let other = other.converted_to(self.space);
        let t = t.clamp(0.0, 1.0);
        let a = self.to_linear();
        let b = other.to_linear();
        let mut out = Self::from_linear([
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
            a[3] + (b[3] - a[3]) * t,
        ]);
        out.space = self.space;
        out
    }

    /// Mix towards `other` through **Oklab**.
    ///
    /// The perceptual one, and the one to reach for when the *midpoint* of a
    /// gradient matters. Both of the others are defined on the numbers;
    /// Oklab is defined on what the eye reports, so a blue-to-yellow ramp
    /// passes through a plausible green instead of the grey that linear light
    /// gives it and the muddy purple that encoded sRGB gives it.
    ///
    /// Ottosson's Oklab, via linear sRGB — so a P3 colour is converted, mixed
    /// and converted back, and the result stays in `self`'s space.
    #[must_use]
    pub fn lerp_oklab(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let a = Oklab::from_color(self);
        let b = Oklab::from_color(other);
        let mixed = Oklab {
            l: a.l + (b.l - a.l) * t,
            a: a.a + (b.a - a.a) * t,
            b: a.b + (b.b - a.b) * t,
            alpha: a.alpha + (b.alpha - a.alpha) * t,
        };
        mixed.to_color(self.space)
    }

    /// This colour's perceptual lightness, `0.0 ..= 1.0`.
    ///
    /// Oklab's `L`. Unlike the relative luminance a contrast ratio is built
    /// from, this is uniform — the difference between 0.4 and 0.5 looks like
    /// the difference between 0.8 and 0.9 — which is what makes it the right
    /// thing to sort a palette by or to derive a tint ramp from.
    #[must_use]
    pub fn lightness(self) -> f32 {
        Oklab::from_color(self).l
    }

    /// The four channels as `0.0 ..= 1.0` floats, ready for a GPU uniform.
    #[must_use]
    pub fn to_f32_array(self) -> [f32; 4] {
        [
            f32::from(self.r) / 255.0,
            f32::from(self.g) / 255.0,
            f32::from(self.b) / 255.0,
            f32::from(self.a) / 255.0,
        ]
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            write!(
                f,
                "#{:02X}{:02X}{:02X}{:02X}",
                self.r, self.g, self.b, self.a
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_opaque_and_argb_carries_alpha() {
        assert_eq!(Color::hex(0xFF8800), Color::rgb(255, 136, 0));
        assert_eq!(
            Color::argb(0x8012_3456),
            Color::rgba(0x12, 0x34, 0x56, 0x80)
        );
    }

    #[test]
    fn float_conversion_hits_the_endpoints_exactly() {
        assert_eq!(Color::WHITE.to_f32_array(), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(Color::TRANSPARENT.to_f32_array(), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn an_opaque_color_over_anything_is_itself() {
        assert_eq!(Color::RED.over(Color::BLUE), Color::RED);
        assert_eq!(Color::TRANSPARENT.over(Color::BLUE), Color::BLUE);
    }

    #[test]
    fn half_of_white_over_black_is_the_middle() {
        let mid = Color::WHITE.with_alpha(128).over(Color::BLACK);
        assert_eq!(mid.a, 255, "over an opaque colour the result is opaque");
        assert!(
            (126..=130).contains(&mid.r),
            "and halfway between them: {mid}"
        );
    }

    #[test]
    fn a_wash_over_nothing_keeps_its_own_transparency() {
        // The text-button case: there is no fill to tint, so the press is the
        // wash itself rather than a colour blended into something.
        let wash = Color::RED.with_alpha(31).over(Color::TRANSPARENT);
        assert_eq!(wash, Color::RED.with_alpha(31));
    }

    #[test]
    fn two_transparent_colors_composite_to_transparent() {
        // The division by the output alpha is undefined here, and the answer
        // that avoids a NaN reaching the GPU is the one that paints nothing.
        assert_eq!(
            Color::RED.with_alpha(0).over(Color::TRANSPARENT),
            Color::TRANSPARENT
        );
    }
}

/// Ottosson's Oklab: a perceptually uniform colour space.
///
/// Here rather than behind an interpolation helper because "how light is this
/// colour, to a human" is a question a theme, a contrast check and a tint ramp
/// all ask, and answering it from relative luminance gets the *ordering* right
/// and the *spacing* wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    /// Perceptual lightness, `0.0` black to `1.0` white.
    pub l: f32,
    /// Green (negative) to red (positive).
    pub a: f32,
    /// Blue (negative) to yellow (positive).
    pub b: f32,
    /// Carried through unchanged; Oklab says nothing about opacity.
    pub alpha: f32,
}

impl Oklab {
    /// Convert, through linear sRGB.
    #[must_use]
    pub fn from_color(color: Color) -> Self {
        let [r, g, b, alpha] = color.to_srgb().to_linear();
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_5 * b;
        let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
        Self {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
            alpha,
        }
    }

    /// Back to an 8-bit colour in `space`.
    ///
    /// Out-of-gamut results are clamped by
    /// [`Color::from_linear`](Color::from_linear); see
    /// [`Color::converted_to`] for why clipping rather than gamut mapping.
    #[must_use]
    pub fn to_color(self, space: ColorSpace) -> Color {
        let l_ = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m_ = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s_ = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;
        let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
        let srgb = Color::from_linear([
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
            self.alpha,
        ]);
        srgb.converted_to(space)
    }
}

#[cfg(test)]
mod space_tests {
    use super::*;

    /// The default is sRGB and every existing literal keeps meaning what it
    /// meant. This is the whole compatibility claim of adding the field.
    #[test]
    fn every_ordinary_constructor_is_srgb() {
        for color in [
            Color::hex(0x33_66FF),
            Color::rgb(1, 2, 3),
            Color::rgba(1, 2, 3, 4),
            Color::argb(0x80_112233),
            Color::WHITE,
            Color::TRANSPARENT,
        ] {
            assert_eq!(color.space, ColorSpace::Srgb);
        }
        assert_eq!(Color::p3(0, 255, 0).space, ColorSpace::DisplayP3);
    }

    /// **In float, a gamut round trip is exact.** This is the assertion that
    /// the two matrices are actually inverses — if they were not, it drifts in
    /// a direction no picture would make obvious.
    #[test]
    fn a_gamut_round_trip_is_exact_in_float() {
        let apply = |m: [[f32; 3]; 3], v: [f32; 3]| {
            [
                m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
                m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
                m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
            ]
        };
        let p3 = ColorSpace::DisplayP3;
        for v in [[0.5, 0.25, 0.75], [1.0, 0.0, 0.0], [0.02, 0.9, 0.4]] {
            let back = apply(p3.to_linear_srgb(), apply(p3.from_linear_srgb(), v));
            for (a, b) in v.iter().zip(back.iter()) {
                assert!((a - b).abs() < 1e-5, "{v:?} came back as {back:?}");
            }
        }
    }

    /// **In eight bits it is not**, and the gap is a property worth pinning
    /// rather than a tolerance to widen until it passes.
    ///
    /// A channel near black occupies a tiny slice of linear light, so encoding
    /// it into eight bits of a *different* gamut and decoding back cannot
    /// recover the value it started from. Bright channels survive; dark ones
    /// drift by a few counts. That is exactly what [`ColorSpace`]'s docs mean
    /// by "does not buy extra precision", and it is why interpolation happens
    /// in float and only the endpoints are quantised.
    #[test]
    fn an_eight_bit_gamut_round_trip_drifts_only_in_the_dark() {
        for color in [
            Color::hex(0x33_66FF),
            Color::hex(0xFF_8800),
            Color::hex(0x80_8080),
        ] {
            let back = color
                .converted_to(ColorSpace::DisplayP3)
                .converted_to(ColorSpace::Srgb);
            for (a, b, name) in [
                (color.r, back.r, "r"),
                (color.g, back.g, "g"),
                (color.b, back.b, "b"),
            ] {
                // Generous where the channel is dark, tight where it is not.
                let allowed = if a < 64 { 12 } else { 2 };
                assert!(
                    a.abs_diff(b) <= allowed,
                    "{color:?} {name} came back as {b} (from {a})"
                );
            }
            assert_eq!(color.a, back.a, "alpha must not be touched");
        }
    }

    /// **The two gamuts are genuinely different, and the tag is what carries
    /// that.** sRGB's green is inside P3, so expressing it there needs real
    /// red and blue — a straight reinterpretation of the same three numbers
    /// would leave both at zero and quietly show a more saturated colour than
    /// was asked for. That silent shift is what the space field exists to stop.
    #[test]
    fn srgb_green_needs_real_channels_to_be_said_in_p3() {
        let wide = Color::rgb(0, 255, 0).converted_to(ColorSpace::DisplayP3);
        assert_eq!(wide.space, ColorSpace::DisplayP3);
        assert!(
            wide.r > 100 && wide.b > 50,
            "sRGB green in P3 should be well inside the gamut: {wide:?}"
        );
    }

    /// And the other direction: P3's green is **outside** sRGB, so converting
    /// clips. Both out-of-range channels go negative, so the clip lands on
    /// sRGB's own green — the most saturated thing it has.
    #[test]
    fn a_p3_primary_clips_to_the_edge_of_srgb() {
        let narrow = Color::p3(0, 255, 0).to_srgb();
        assert_eq!(narrow.space, ColorSpace::Srgb);
        assert_eq!((narrow.r, narrow.g, narrow.b), (0, 255, 0));
    }

    /// Linear light holds the midtones up where encoded sRGB drops them. This
    /// is the difference the two methods exist to let a caller choose between,
    /// and 0x80 vs 0xBC is far too large to be a rounding argument.
    #[test]
    fn linear_and_encoded_midpoints_differ_as_documented() {
        let encoded = Color::BLACK.lerp(Color::WHITE, 0.5);
        let linear = Color::BLACK.lerp_linear(Color::WHITE, 0.5);
        assert_eq!(encoded.r, 128);
        assert!(
            linear.r > 180,
            "linear light should be much brighter, got {}",
            linear.r
        );
    }

    /// Oklab's midpoint of blue and yellow is a colour, not a grey. Encoded and
    /// linear mixes both pass through something desaturated; this is the whole
    /// reason to have a perceptual option.
    #[test]
    fn oklab_keeps_a_hue_through_the_middle() {
        let blue = Color::hex(0x00_00FF);
        let yellow = Color::hex(0xFF_FF00);
        let perceptual = blue.lerp_oklab(yellow, 0.5);
        let spread = |c: Color| {
            let max = c.r.max(c.g).max(c.b);
            let min = c.r.min(c.g).min(c.b);
            max - min
        };
        assert!(
            spread(perceptual) > 40,
            "the midpoint went grey: {perceptual:?}"
        );
    }

    /// Every interpolation is the identity at its ends, whichever space it
    /// works in. A midpoint that looks good and an endpoint that does not is
    /// the worst possible trade.
    #[test]
    fn every_mix_is_exact_at_both_ends() {
        let a = Color::hex(0x12_3456);
        let b = Color::hex(0xFE_DCBA);
        for mix in [Color::lerp, Color::lerp_linear, Color::lerp_oklab] {
            let start = mix(a, b, 0.0);
            let end = mix(a, b, 1.0);
            for (got, want) in [(start, a), (end, b)] {
                assert!(
                    got.r.abs_diff(want.r) <= 1
                        && got.g.abs_diff(want.g) <= 1
                        && got.b.abs_diff(want.b) <= 1,
                    "{got:?} should have been {want:?}"
                );
            }
        }
    }

    /// Perceptual lightness orders a ramp the way an eye does, and is
    /// **evenly spaced** where relative luminance is not — which is the
    /// property that makes it usable for deriving a tint scale.
    #[test]
    fn lightness_is_ordered_and_evenly_spaced() {
        let steps: Vec<f32> = (0..=4)
            .map(|n| {
                Color::BLACK
                    .lerp_oklab(Color::WHITE, n as f32 / 4.0)
                    .lightness()
            })
            .collect();
        for pair in steps.windows(2) {
            assert!(pair[1] > pair[0], "not monotonic: {steps:?}");
        }
        for pair in steps.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                (gap - 0.25).abs() < 0.03,
                "an Oklab ramp should be evenly spaced: {steps:?}"
            );
        }
    }

    /// Compositing reads both operands in the destination's space, so a P3
    /// wash over an sRGB surface does not silently reinterpret its numbers.
    #[test]
    fn compositing_converts_into_the_destination_space() {
        // Inside both gamuts, so the conversion is a real change rather than a
        // clip that happens to land on the same numbers.
        let wash = Color::p3a(255, 128, 0, 128);
        let under = Color::hex(0x00_0000);
        let out = wash.over(under);
        assert_eq!(out.space, ColorSpace::Srgb);
        let naive = Color::rgba(255, 128, 0, 128).over(under);
        assert_ne!(
            (out.r, out.g, out.b),
            (naive.r, naive.g, naive.b),
            "the P3 wash composited as if it were sRGB"
        );
    }
}
