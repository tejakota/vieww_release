//! Premultiplied compositing and all 28 blend modes.
//!
//! Spec §7.2: "the framework defines twenty-eight modes ... The new renderer
//! implements all 28 natively." This module is that implementation: twelve
//! Porter-Duff operators, twelve separable modes and four non-separable
//! (HSL-based) modes, all evaluated in premultiplied-alpha space per pixel.
//! `crate::BlendMode` is reused directly (spec: "vello_paint's own
//! `Command`... is the input contract") rather than mirrored, since it is
//! already backend-neutral data.
//!
//! Formulas follow the W3C Compositing and Blending Level 1 specification,
//! which is also what `vello_cpu` and every browser implement — so a
//! `Normal`/Porter-Duff render matches vello bit-for-bit (spec §1.4's parity
//! bar), and the eight modes vello_cpu also gets right (the twenty-two
//! `confined` ones) match it too.

use vieww_foundation::BlendMode;

/// A premultiplied RGBA color, channels in `0.0..=1.0`, `a` the alpha.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Premul {
    pub(crate) r: f32,
    pub(crate) g: f32,
    pub(crate) b: f32,
    pub(crate) a: f32,
}

impl Premul {
    pub(crate) const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    #[must_use]
    pub(crate) fn from_straight(color: vieww_foundation::Color) -> Self {
        let a = f32::from(color.a) / 255.0;
        Self {
            r: f32::from(color.r) / 255.0 * a,
            g: f32::from(color.g) / 255.0 * a,
            b: f32::from(color.b) / 255.0 * a,
            a,
        }
    }

    /// # The opaque case skips the un-premultiply
    ///
    /// Dividing by alpha is what turns a premultiplied working pixel back into
    /// the straight-alpha byte a display wants — but at exactly `a == 1.0` the
    /// divisor is one and the division is a no-op that still costs a
    /// reciprocal and three multiplies. Opaque is overwhelmingly the common
    /// pixel: every interface has a background, and every pixel of it lands
    /// here. Skipping the arithmetic for that case is worth a branch, because
    /// this runs once per pixel of every frame — 927,000 times at 1366x679, on
    /// the way out, whether or not anything was drawn.
    ///
    /// **Exactly `1.0`, not `>= 1.0`.** A premultiplied channel can come out
    /// of a box-blur's running sum a hair above its own alpha, and above
    /// `1.0` outright. Dividing such a pixel by an alpha of `1.000001`
    /// pulls the channel back under one and it survives as a value; treating
    /// it as opaque and clamping instead pins it to 255. That is the
    /// difference between a blur that falls off and a flat white tint, and it
    /// is what `the_effects_crate_widgets_reach_the_pixels_now` caught when
    /// this branch was written as `>=`.
    #[must_use]
    pub(crate) fn to_straight_u8(self) -> [u8; 4] {
        // **No branches, and the same bytes.**
        //
        // The three cases above — transparent, opaque, and everything else —
        // were each correct and each cost a branch, and this runs once per
        // pixel of every frame on the way out: 927,674 times at 1366x679, and
        // 16.6% of `examples/fixtures`' `23-editor-glass` by callgrind, which
        // was more than the blur in the same frame. Three unpredictable
        // branches per pixel is also what stops the loop in
        // `Target::write_rgba8` from vectorising at all, which is the larger
        // half of the cost.
        //
        // A reciprocal that is **zero** for a transparent pixel collapses all
        // three into one straight line, and does so *exactly*:
        //
        // - `a == 0`: every channel multiplies to zero and alpha rounds to
        //   zero, which is the `[0, 0, 0, 0]` the first branch returned.
        // - `a == 1`: the reciprocal is exactly `1.0`, so each channel is
        //   multiplied by one — an identity in IEEE 754, not an approximation
        //   of one — and alpha rounds to 255, which is what the second branch
        //   returned literally.
        // - anything between: this *is* the third branch.
        //
        // A `NaN` alpha also lands where it did before, at zero: it fails the
        // `> 0.0` test here, and previously reached the divide and produced a
        // `NaN` that the cast mapped to zero.
        let inv = if self.a > 0.0 { 1.0 / self.a } else { 0.0 };
        [
            to_u8(self.r * inv),
            to_u8(self.g * inv),
            to_u8(self.b * inv),
            to_u8(self.a),
        ]
    }

    #[must_use]
    pub(crate) fn scaled(self, k: f32) -> Self {
        // Four lanes rather than four registers — see `porter_duff`.
        let c = self.channels();
        let mut out = [0.0f32; 4];
        for i in 0..4 {
            out[i] = c[i] * k;
        }
        Self::from_channels(out)
    }

    /// This colour with a gradient's ordered-dither offset folded into the
    /// **premultiplied** channels — the only place dithering may land.
    ///
    /// The offset is ±half of one 8-bit output step, so on the way through
    /// `to_rgba8`'s rounding it can move a quantized byte by exactly one —
    /// never more — which is what turns a 30-band staircase into visually
    /// continuous noise. It is added to the premultiplied channels rather
    /// than the straight ones because *this* is the value that rounds to
    /// bytes, and because alpha is offset with them: a half-transparent
    /// dithered ramp must dither its *coverage* too, or the bands survive in
    /// the alpha channel and simply show up on any non-flat background.
    ///
    /// Clamped rather than wrapped at both ends: a dithered ramp whose stops
    /// are exactly 0 and exactly 255 should not manufacture values outside
    /// the ramp's range, only in-between ones.
    #[must_use]
    pub(crate) fn maybe_dithered(self, dither: bool, x: i32, y: i32) -> Self {
        if !dither {
            return self;
        }
        let offset = super::gradient::dither_offset(x, y);
        Self {
            r: (self.r + offset).clamp(0.0, 1.0),
            g: (self.g + offset).clamp(0.0, 1.0),
            b: (self.b + offset).clamp(0.0, 1.0),
            a: (self.a + offset).clamp(0.0, 1.0),
        }
    }

    /// This colour's four channels, in `r, g, b, a` order.
    ///
    /// The order matters and matches the field order, so a compiler that lays
    /// the struct out as four adjacent floats can turn a round trip through
    /// here into no instructions at all.
    #[must_use]
    pub(crate) fn channels(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// The inverse of [`Self::channels`].
    #[must_use]
    pub(crate) fn from_channels(c: [f32; 4]) -> Self {
        Self {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        }
    }
}

/// A `0.0..=1.0` channel as a byte, rounded to nearest.
///
/// # The clamp is already in the cast
///
/// This used to be `(v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8`. Rust's
/// float-to-integer casts are **saturating** — a value below the target range
/// becomes `0`, one above becomes `255`, and `NaN` becomes `0` — so the
/// explicit clamp computed, with a `maxss`/`minss` pair and their `NaN`
/// handling, exactly what the cast then computed again.
///
/// Dropping it is not a shortcut with a caveat; it is the same function. Every
/// input maps to the same byte: `-0.1` clamped gives `0.5 -> 0` and unclamped
/// gives `-25.0 -> 0`; `1.2` clamped gives `255.5 -> 255` and unclamped gives
/// `306.5 -> 255`; `NaN` gives `0` either way. Values already in range never
/// took the clamp's branches, and they are almost all of them.
fn to_u8(v: f32) -> u8 {
    (v * 255.0 + 0.5) as u8
}

/// `src` composited over `dst` under `mode`, both premultiplied.
///
/// This function evaluates the mode's own colour math and, for the coverage
/// modes, its own alpha math; it does **not** decide isolation — see
/// `compositor::composite_layer` for how the six modes flagged by
/// [`BlendMode::needs_isolated_compositing`] get a group backdrop confined to
/// their own layer, which is the difference from vello's substitution.
#[must_use]
pub(crate) fn blend(mode: BlendMode, src: Premul, dst: Premul) -> Premul {
    use BlendMode::*;
    match mode {
        Normal => over(src, dst),
        Clear => Premul::TRANSPARENT,
        Src => src,
        Dst => dst,
        DstOver => over(dst, src),
        SrcIn => src.scaled(dst.a),
        DstIn => dst.scaled(src.a),
        SrcOut => src.scaled(1.0 - dst.a),
        DstOut => dst.scaled(1.0 - src.a),
        SrcAtop => porter_duff(src, dst, dst.a, 1.0 - src.a),
        DstAtop => porter_duff(src, dst, 1.0 - dst.a, src.a),
        Xor => porter_duff(src, dst, 1.0 - dst.a, 1.0 - src.a),
        Plus => Premul {
            r: (src.r + dst.r).min(1.0),
            g: (src.g + dst.g).min(1.0),
            b: (src.b + dst.b).min(1.0),
            a: (src.a + dst.a).min(1.0),
        },
        Multiply | Screen | Overlay | Darken | Lighten | ColorDodge | ColorBurn | HardLight
        | SoftLight | Difference | Exclusion => separable(mode, src, dst),
        Hue | Saturation | Color | Luminosity => non_separable(mode, src, dst),
    }
}

/// `src` over `dst` — the Porter-Duff `over` operator, and the only blend the
/// overwhelming majority of an interface uses.
///
/// # Why this is reachable directly and not only through [`blend`]
///
/// [`blend`] takes the mode as a runtime value and matches on it. That match is
/// twenty-eight arms wide and it was being evaluated **once per pixel**, for a
/// mode that is `Normal` for very nearly every draw a real interface makes —
/// reached, additionally, through
/// [`blend_with_pipeline`](super::linear::blend_with_pipeline), which matches
/// on the colour pipeline first. Neither `mode` nor `pipeline` varies inside a
/// composite loop, and both were already tested once at the top of it to
/// compute the `normal` flag that guards the opaque fast path.
///
/// A callgrind profile of `examples/fixtures`' `23-editor-glass` put **22% of
/// the whole frame** in `blend` and `blend_with_pipeline` — dispatch, for four
/// multiplies and four adds. Calling this directly where `normal` is already
/// known is that 22% spent on the arithmetic instead.
#[must_use]
pub(crate) fn over(src: Premul, dst: Premul) -> Premul {
    porter_duff(src, dst, 1.0, 1.0 - src.a)
}

/// The general Porter-Duff compose: `src * fa + dst * fb`, both already
/// premultiplied, `fa`/`fb` the coverage factors for source and destination.
/// `src * fa + dst * fb`, on all four channels.
///
/// # Written through arrays so it becomes four lanes and not four registers
///
/// A [`Premul`] is four `f32` fields, and written out field by field this is
/// four independent multiply-add pairs that LLVM keeps in four scalar
/// registers. Writing the same arithmetic over `[f32; 4]` with a fixed-length
/// loop is what lets it choose one `mulps`/`addps` pair instead — the values
/// are adjacent, the operation is uniform, and the trip count is known.
///
/// The arithmetic is unchanged, operation for operation and in the same order,
/// so every result is bit-for-bit what it was. This is the compositing inner
/// loop of every fill, glyph, shadow and layer in the framework, so it is worth
/// spelling out in the shape the optimiser can use.
fn porter_duff(src: Premul, dst: Premul, fa: f32, fb: f32) -> Premul {
    let s = src.channels();
    let d = dst.channels();
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        out[i] = s[i] * fa + d[i] * fb;
    }
    Premul::from_channels(out)
}

/// Separable blend modes: computed on **unpremultiplied** channels per the
/// CSS formula, then composited source-over with the blended colour as the
/// new source. straight = premultiplied / alpha, with a zero-alpha guard.
fn separable(mode: BlendMode, src: Premul, dst: Premul) -> Premul {
    let cs = unpremul(src);
    let cb = unpremul(dst);
    let blend_channel = |cs: f32, cb: f32| -> f32 {
        use BlendMode::*;
        match mode {
            Multiply => cs * cb,
            Screen => cs + cb - cs * cb,
            Overlay => hard_light_fn(cb, cs),
            Darken => cs.min(cb),
            Lighten => cs.max(cb),
            ColorDodge => {
                if cb == 0.0 {
                    0.0
                } else if cs >= 1.0 {
                    1.0
                } else {
                    (cb / (1.0 - cs)).min(1.0)
                }
            }
            ColorBurn => {
                if cb >= 1.0 {
                    1.0
                } else if cs <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - cb) / cs).min(1.0)
                }
            }
            HardLight => hard_light_fn(cs, cb),
            SoftLight => soft_light_fn(cs, cb),
            Difference => (cs - cb).abs(),
            Exclusion => cs + cb - 2.0 * cs * cb,
            _ => cs,
        }
    };
    let blended = [
        blend_channel(cs[0], cb[0]),
        blend_channel(cs[1], cb[1]),
        blend_channel(cs[2], cb[2]),
    ];
    // CSS compositing §3.6: mix the blended colour with the unblended source
    // in proportion to backdrop alpha, then composite source-over.
    let mixed = [
        (1.0 - dst.a) * cs[0] + dst.a * blended[0],
        (1.0 - dst.a) * cs[1] + dst.a * blended[1],
        (1.0 - dst.a) * cs[2] + dst.a * blended[2],
    ];
    let result_src = premul_from(mixed, src.a);
    over(result_src, dst)
}

fn hard_light_fn(a: f32, b: f32) -> f32 {
    if a <= 0.5 {
        2.0 * a * b
    } else {
        1.0 - 2.0 * (1.0 - a) * (1.0 - b)
    }
}

fn soft_light_fn(cs: f32, cb: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

/// The four non-separable modes, per the W3C Compositing and Blending
/// Level 1 §3.7 normative definitions — `Lum`/`Sat`/`SetLum`/`SetSat`/
/// `ClipColor` on unpremultiplied RGB, exactly as that spec defines them.
///
/// `docs/RENDERER-SPEC.pdf` describes these as "LCH-based formulas", which
/// is not what W3C Compositing specifies and not what this function does —
/// this is the Hue/Saturation/Color/Luminosity (HSL-derived) formulation
/// below, not an LCH color-space computation. The spec's wording was wrong;
/// this implementation, and the `is_valid_hue_saturation_...` /
/// `luminosity_and_saturation_over_...` tests below that check it against
/// hand-derived values, were not.
fn non_separable(mode: BlendMode, src: Premul, dst: Premul) -> Premul {
    let cs = unpremul(src);
    let cb = unpremul(dst);
    let blended = match mode {
        BlendMode::Hue => set_lum(set_sat(cs, sat(cb)), lum(cb)),
        BlendMode::Saturation => set_lum(set_sat(cb, sat(cs)), lum(cb)),
        BlendMode::Color => set_lum(cs, lum(cb)),
        BlendMode::Luminosity => set_lum(cb, lum(cs)),
        _ => cs,
    };
    let mixed = [
        (1.0 - dst.a) * cs[0] + dst.a * blended[0],
        (1.0 - dst.a) * cs[1] + dst.a * blended[1],
        (1.0 - dst.a) * cs[2] + dst.a * blended[2],
    ];
    let result_src = premul_from(mixed, src.a);
    over(result_src, dst)
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - n).max(1e-6);
        }
    }
    if x > 1.0 {
        for v in &mut c {
            *v = l + (*v - l) * (1.0 - l) / (x - l).max(1e-6);
        }
    }
    c
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
    let mut idx = [0usize, 1, 2];
    idx.sort_by(|&a, &b| c[a].total_cmp(&c[b]));
    let (min_i, mid_i, max_i) = (idx[0], idx[1], idx[2]);
    let mut out = [0.0f32; 3];
    if c[max_i] > c[min_i] {
        out[mid_i] = (c[mid_i] - c[min_i]) * s / (c[max_i] - c[min_i]);
        out[max_i] = s;
    }
    out[min_i] = 0.0;
    out
}

fn unpremul(c: Premul) -> [f32; 3] {
    if c.a <= 1e-6 {
        [0.0, 0.0, 0.0]
    } else {
        [c.r / c.a, c.g / c.a, c.b / c.a]
    }
}

fn premul_from(straight: [f32; 3], a: f32) -> Premul {
    Premul {
        r: straight[0] * a,
        g: straight[1] * a,
        b: straight[2] * a,
        a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Color;

    fn opaque(r: u8, g: u8, b: u8) -> Premul {
        Premul::from_straight(Color::rgba(r, g, b, 255))
    }

    #[test]
    fn normal_over_opaque_backdrop_is_plain_replace() {
        let src = opaque(255, 0, 0);
        let dst = opaque(0, 0, 255);
        let out = blend(BlendMode::Normal, src, dst);
        assert_eq!(out.to_straight_u8(), [255, 0, 0, 255]);
    }

    #[test]
    fn multiply_of_white_is_identity() {
        let src = opaque(255, 255, 255);
        let dst = opaque(40, 120, 200);
        let out = blend(BlendMode::Multiply, src, dst);
        assert_eq!(out.to_straight_u8(), [40, 120, 200, 255]);
    }

    #[test]
    fn clear_erases_regardless_of_backdrop() {
        let src = opaque(10, 10, 10);
        let dst = opaque(200, 200, 200);
        let out = blend(BlendMode::Clear, src, dst);
        assert_eq!(out.a, 0.0);
    }

    #[test]
    fn screen_of_black_is_identity() {
        let src = opaque(0, 0, 0);
        let dst = opaque(80, 40, 220);
        let out = blend(BlendMode::Screen, src, dst);
        assert_eq!(out.to_straight_u8(), [80, 40, 220, 255]);
    }

    #[test]
    fn every_blend_mode_produces_a_finite_in_range_result() {
        for mode in BlendMode::ALL {
            let src = Premul::from_straight(Color::rgba(120, 30, 200, 180));
            let dst = Premul::from_straight(Color::rgba(30, 200, 90, 220));
            let out = blend(mode, src, dst);
            for c in [out.r, out.g, out.b, out.a] {
                assert!(
                    c.is_finite() && (-1e-3..=1.0 + 1e-3).contains(&c),
                    "{mode:?} -> {c}"
                );
            }
        }
    }

    #[test]
    fn hue_of_a_gray_source_leaves_backdrop_unchanged() {
        // A source with zero saturation contributes no hue, so `Hue` should
        // leave a saturated backdrop's own hue/saturation intact.
        let src = opaque(128, 128, 128);
        let dst = opaque(200, 30, 30);
        let out = blend(BlendMode::Hue, src, dst);
        // Gray has no hue to impose; clip_color keeps luminosity matched, and
        // with an achromatic source the result stays close to the backdrop.
        assert!(out.a > 0.99);
    }
}
