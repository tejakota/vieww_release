//! An independent correctness oracle for `native/color.rs`'s blending —
//! "Renderer v2" pillar F (`docs/RENDERER-V2-NOTES.md`): "Reduce common-mode
//! risk in the parity oracle... Add independent mathematical tests... so the
//! CPU reference isn't the only thing certifying itself."
//!
//! Test-only by construction (this whole module is `#[cfg(test)]`, declared
//! that way in `native/mod.rs` — it never compiles into a shipped build).
//! Everything below is written directly from the W3C Compositing and
//! Blending Level 1 specification's normative equations, deliberately
//! **not** sharing a type, a helper function, or an internal factoring with
//! `native/color.rs`:
//!
//! - A distinct colour type, [`Straight`] — unpremultiplied `f64`, where
//!   `color.rs`'s `Premul` is premultiplied `f32`. Two different
//!   representations and two different float widths make it very unlikely
//!   that a rounding or premultiplication-order bug in one would reproduce
//!   itself identically in the other.
//! - Every Porter-Duff operator here goes through one literal `Fa`/`Fb`
//!   coefficient table straight out of the spec (see [`porter_duff_factors`]),
//!   rather than `color.rs`'s per-mode `match` arms that hand-simplify each
//!   operator's algebra (e.g. its `SrcIn` is `src.scaled(dst.a)`, already
//!   simplified from the general form — correct, but a different piece of
//!   code from evaluating the general `Fa = αb, Fb = 0` formula directly).
//! - The four non-separable (HSL-derived) functions are transcribed
//!   independently from W3C §3.7's `Lum`/`ClipColor`/`Sat`/`SetLum`/`SetSat`
//!   pseudocode, with the exact same numbered structure that section uses,
//!   rather than reusing `color.rs`'s `lum`/`clip_color`/`sat`/`set_lum`/
//!   `set_sat` functions.
//!
//! [`known_vectors`] checks this oracle against numeric results computed a
//! *third* way — by hand, worked with a Python script external to this
//! Rust codebase entirely, not derived from either Rust implementation —
//! for a representative spread of modes. [`agrees_with_the_shipped_renderer`]
//! is the actual pillar-F deliverable: it diffs `color.rs`'s `blend` against
//! this oracle across a grid of representative premultiplied colour pairs
//! (opaque/opaque, translucent/translucent, and the zero-alpha edges each
//! mode's own `if`/`else` guards exist for), with tolerance split by
//! category rather than one global epsilon — see that test's own comment
//! for why each category gets the bound it gets.

use vieww_foundation::{BlendMode, Color};

use super::color::{blend as shipped_blend, Premul};

/// Unpremultiplied, double-precision, deliberately distinct from
/// `color.rs`'s `Premul` — see the module docs.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Straight {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

/// The result of compositing: still unpremultiplied, so a caller can read
/// `r`/`g`/`b` directly without an unpremultiply step of its own — the
/// oracle's public contract is "here is the straight-alpha answer," not
/// "here is the premultiplied answer, go divide it yourself."
fn oracle_blend(mode: BlendMode, src: Straight, dst: Straight) -> Straight {
    if let Some(blend_fn) = separable_fn(mode) {
        return mix_and_composite(src, dst, blend_fn);
    }
    if let Some(hsl_fn) = non_separable_fn(mode) {
        return mix_and_composite_whole(src, dst, hsl_fn);
    }
    let (fa, fb) = porter_duff_factors(mode, src.a, dst.a);
    porter_duff_compose(src, dst, fa, fb)
}

/// W3C §3.2's simple alpha compositing, evaluated directly on straight-alpha
/// input by first computing each side's own contribution
/// (`component x alpha x factor`) rather than working through a
/// premultiplied intermediate the way `color.rs`'s `porter_duff` does.
fn porter_duff_compose(src: Straight, dst: Straight, fa: f64, fb: f64) -> Straight {
    let ao = src.a * fa + dst.a * fb;
    let composite = |cs: f64, cb: f64| -> f64 {
        let premultiplied = cs * src.a * fa + cb * dst.a * fb;
        if ao <= 0.0 {
            0.0
        } else {
            premultiplied / ao
        }
    };
    Straight {
        r: composite(src.r, dst.r),
        g: composite(src.g, dst.g),
        b: composite(src.b, dst.b),
        a: ao,
    }
}

/// The classic Porter-Duff operator table (Porter & Duff 1984, table 1;
/// reproduced by the CSS Compositing spec's own worked-out `Fa`/`Fb` per
/// operator) — `Plus` is not a standard Porter-Duff/CSS operator, so it is
/// handled on its own terms (simple saturating premultiplied addition, which
/// is what every implementation of it — including `color.rs`'s — actually
/// means by the name).
fn porter_duff_factors(mode: BlendMode, _asrc: f64, _adst: f64) -> (f64, f64) {
    use BlendMode::*;
    match mode {
        Clear => (0.0, 0.0),
        Src => (1.0, 0.0),
        Dst => (0.0, 1.0),
        Normal => (1.0, 1.0 - _asrc),
        DstOver => (1.0 - _adst, 1.0),
        SrcIn => (_adst, 0.0),
        DstIn => (0.0, _asrc),
        SrcOut => (1.0 - _adst, 0.0),
        DstOut => (0.0, 1.0 - _asrc),
        SrcAtop => (_adst, 1.0 - _asrc),
        DstAtop => (1.0 - _adst, _asrc),
        Xor => (1.0 - _adst, 1.0 - _asrc),
        // Not in the Porter-Duff table; `Fa = Fb = 1` (identity factors) —
        // the caller special-cases `Plus` before this table is consulted so
        // this arm never actually runs, kept only so the match is total.
        Plus => (1.0, 1.0),
        _ => unreachable!(
            "blend-function modes are routed to separable_fn/non_separable_fn before this table"
        ),
    }
}

fn separable_fn(mode: BlendMode) -> Option<fn(f64, f64) -> f64> {
    use BlendMode::*;
    Some(match mode {
        Multiply => multiply,
        Screen => screen,
        Overlay => overlay,
        Darken => darken,
        Lighten => lighten,
        ColorDodge => color_dodge,
        ColorBurn => color_burn,
        HardLight => hard_light,
        SoftLight => soft_light,
        Difference => difference,
        Exclusion => exclusion,
        _ => return None,
    })
}

fn multiply(cb: f64, cs: f64) -> f64 {
    cb * cs
}
fn screen(cb: f64, cs: f64) -> f64 {
    cb + cs - cb * cs
}
fn hard_light(cb: f64, cs: f64) -> f64 {
    if cs <= 0.5 {
        multiply(cb, 2.0 * cs)
    } else {
        screen(cb, 2.0 * cs - 1.0)
    }
}
/// W3C §3.6: "overlay(Cb, Cs) = hard-light(Cs, Cb)" — the two arguments to
/// `hard-light` swapped, transcribed exactly as the spec states it (not
/// re-derived, since there is nothing to derive: the spec defines overlay
/// *in terms of* hard-light with its arguments reversed).
fn overlay(cb: f64, cs: f64) -> f64 {
    hard_light(cs, cb)
}
fn darken(cb: f64, cs: f64) -> f64 {
    cb.min(cs)
}
fn lighten(cb: f64, cs: f64) -> f64 {
    cb.max(cs)
}
fn color_dodge(cb: f64, cs: f64) -> f64 {
    if cb == 0.0 {
        0.0
    } else if cs >= 1.0 {
        1.0
    } else {
        1.0f64.min(cb / (1.0 - cs))
    }
}
fn color_burn(cb: f64, cs: f64) -> f64 {
    if cb >= 1.0 {
        1.0
    } else if cs <= 0.0 {
        0.0
    } else {
        1.0 - 1.0f64.min((1.0 - cb) / cs)
    }
}
fn soft_light(cb: f64, cs: f64) -> f64 {
    fn d(x: f64) -> f64 {
        if x <= 0.25 {
            ((16.0 * x - 12.0) * x + 4.0) * x
        } else {
            x.sqrt()
        }
    }
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        cb + (2.0 * cs - 1.0) * (d(cb) - cb)
    }
}
fn difference(cb: f64, cs: f64) -> f64 {
    (cb - cs).abs()
}
fn exclusion(cb: f64, cs: f64) -> f64 {
    cb + cs - 2.0 * cb * cs
}

/// W3C §3.6's own compositing formula for a separable blend function,
/// mixing the blended colour with the plain source in proportion to
/// backdrop alpha, then compositing source-over — worked here directly on
/// straight-alpha input rather than through a premultiplied intermediate.
fn mix_and_composite(src: Straight, dst: Straight, b: impl Fn(f64, f64) -> f64) -> Straight {
    mix_and_composite_whole(src, dst, |cb, cs| {
        [b(cb[0], cs[0]), b(cb[1], cs[1]), b(cb[2], cs[2])]
    })
}

fn mix_and_composite_whole(
    src: Straight,
    dst: Straight,
    b: impl Fn([f64; 3], [f64; 3]) -> [f64; 3],
) -> Straight {
    let cs = [src.r, src.g, src.b];
    let cb = [dst.r, dst.g, dst.b];
    let blended = b(cb, cs);
    let mixed = [
        (1.0 - dst.a) * cs[0] + dst.a * blended[0],
        (1.0 - dst.a) * cs[1] + dst.a * blended[1],
        (1.0 - dst.a) * cs[2] + dst.a * blended[2],
    ];
    let ao = src.a + dst.a * (1.0 - src.a);
    let composite = |mixed_c: f64, cb_c: f64| -> f64 {
        let premultiplied = mixed_c * src.a + cb_c * dst.a * (1.0 - src.a);
        if ao <= 0.0 {
            0.0
        } else {
            premultiplied / ao
        }
    };
    Straight {
        r: composite(mixed[0], cb[0]),
        g: composite(mixed[1], cb[1]),
        b: composite(mixed[2], cb[2]),
        a: ao,
    }
}

/// One of the four non-separable blend modes, as a function of backdrop and
/// source in straight RGB.
///
/// Named because the bare `fn([f64; 3], [f64; 3]) -> [f64; 3]` reads as three
/// unrelated triples at the one call site that matters.
type NonSeparable = fn([f64; 3], [f64; 3]) -> [f64; 3];

fn non_separable_fn(mode: BlendMode) -> Option<NonSeparable> {
    use BlendMode::*;
    Some(match mode {
        Hue => |cb, cs| set_lum(set_sat(cs, sat(cb)), lum(cb)),
        Saturation => |cb, cs| set_lum(set_sat(cb, sat(cs)), lum(cb)),
        Color => |cb, cs| set_lum(cs, lum(cb)),
        Luminosity => |cb, cs| set_lum(cb, lum(cs)),
        _ => return None,
    })
}

/// W3C §3.7, `Lum`.
fn lum(c: [f64; 3]) -> f64 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

/// W3C §3.7, `ClipColor` — transcribed with its own three named
/// intermediates (`l`, `n`, `x`) rather than `color.rs`'s in-place mutation.
fn clip_color(c: [f64; 3]) -> [f64; 3] {
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut result = c;
    if n < 0.0 {
        for (i, v) in result.iter_mut().enumerate() {
            *v = l + (c[i] - l) * l / (l - n);
        }
    }
    let result_after_low_clip = result;
    if x > 1.0 {
        for (i, v) in result.iter_mut().enumerate() {
            *v = l + (result_after_low_clip[i] - l) * (1.0 - l) / (x - l);
        }
    }
    result
}

/// W3C §3.7, `SetLum`.
fn set_lum(c: [f64; 3], l: f64) -> [f64; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

/// W3C §3.7, `Sat`.
fn sat(c: [f64; 3]) -> f64 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// W3C §3.7, `SetSat` — its pseudocode names the channels by comparison
/// (`Cmax`/`Cmid`/`Cmin`) rather than by index; this keeps that naming by
/// sorting indices, then writes into a same-shaped array by that ordering.
fn set_sat(c: [f64; 3], s: f64) -> [f64; 3] {
    let mut order = [0usize, 1, 2];
    order.sort_by(|&i, &j| c[i].partial_cmp(&c[j]).expect("finite input"));
    let (min_i, mid_i, max_i) = (order[0], order[1], order[2]);
    let mut out = [0.0; 3];
    if c[max_i] > c[min_i] {
        out[mid_i] = (c[mid_i] - c[min_i]) * s / (c[max_i] - c[min_i]);
        out[max_i] = s;
    }
    out[min_i] = 0.0;
    out
}

fn to_straight(color: Premul) -> Straight {
    if color.a <= 0.0 {
        Straight {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }
    } else {
        Straight {
            r: f64::from(color.r / color.a),
            g: f64::from(color.g / color.a),
            b: f64::from(color.b / color.a),
            a: f64::from(color.a),
        }
    }
}

fn to_premul(color: Straight) -> Premul {
    Premul {
        r: (color.r * color.a) as f32,
        g: (color.g * color.a) as f32,
        b: (color.b * color.a) as f32,
        a: color.a as f32,
    }
}

/// `Plus` is not part of the W3C Porter-Duff/blend-function vocabulary at
/// all — it is a simple saturating premultiplied add, handled entirely on
/// its own (both here and in `color.rs`), so the general machinery above
/// never sees it.
fn oracle_blend_top_level(mode: BlendMode, src: Straight, dst: Straight) -> Straight {
    if mode == BlendMode::Plus {
        let premul_r = (src.r * src.a + dst.r * dst.a).min(1.0);
        let premul_g = (src.g * src.a + dst.g * dst.a).min(1.0);
        let premul_b = (src.b * src.a + dst.b * dst.a).min(1.0);
        let ao = (src.a + dst.a).min(1.0);
        return if ao <= 0.0 {
            Straight {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }
        } else {
            Straight {
                r: premul_r / ao,
                g: premul_g / ao,
                b: premul_b / ao,
                a: ao,
            }
        };
    }
    oracle_blend(mode, src, dst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
        (a - b).abs() <= eps
    }

    fn assert_straight_close(actual: Straight, expected: [f64; 4], eps: f64, mode: BlendMode) {
        let a = [actual.r, actual.g, actual.b, actual.a];
        for i in 0..4 {
            assert!(
                approx_eq(a[i], expected[i], eps),
                "{mode:?} channel {i}: got {}, expected {} (within {eps})",
                a[i],
                expected[i]
            );
        }
    }

    /// Numeric results for `Cs = (0.7, 0.3, 0.5)`, `αs = 0.6`,
    /// `Cb = (0.2, 0.6, 0.8)`, `αb = 0.7`, computed a *third* way: by a
    /// Python script written independently of both this file and
    /// `color.rs`, working the same W3C formulas by hand rather than
    /// importing either Rust implementation. This is the "known vectors"
    /// pillar F asks for — an external check on the oracle itself, not only
    /// the oracle checking the shipped renderer.
    #[test]
    fn known_vectors() {
        let src = Straight {
            r: 0.7,
            g: 0.3,
            b: 0.5,
            a: 0.6,
        };
        let dst = Straight {
            r: 0.2,
            g: 0.6,
            b: 0.8,
            a: 0.7,
        };
        let eps = 1e-6;

        let cases: &[(BlendMode, [f64; 4])] = &[
            (BlendMode::Normal, [0.476, 0.348, 0.524, 0.88]),
            (BlendMode::SrcIn, [0.294, 0.126, 0.21, 0.42]),
            (BlendMode::SrcAtop, [0.35, 0.294, 0.434, 0.7]),
            (BlendMode::Xor, [0.182, 0.222, 0.314, 0.46]),
            (BlendMode::Multiply, [0.2408, 0.2976, 0.482, 0.88]),
            (BlendMode::Screen, [0.5012, 0.5244, 0.692, 0.88]),
            (BlendMode::Difference, [0.392, 0.348, 0.44, 0.88]),
            (BlendMode::HardLight, [0.4004, 0.3732, 0.65, 0.88]),
            (BlendMode::ColorDodge, [0.462, 0.582, 0.734, 0.88]),
            (BlendMode::ColorBurn, [0.182, 0.222, 0.566, 0.88]),
            (BlendMode::SoftLight, [0.307664, 0.43368, 0.65, 0.88]),
            (BlendMode::Color, [0.5012, 0.3732, 0.5492, 0.88]),
            (BlendMode::Luminosity, [0.2408, 0.4488, 0.6248, 0.88]),
            (BlendMode::Hue, [0.55538, 0.34338, 0.56138, 0.88]),
            (BlendMode::Saturation, [0.30828, 0.46028, 0.60828, 0.88]),
        ];

        for &(mode, premultiplied_expected) in cases {
            // The Python vectors above are premultiplied `co`/`αo` (matching
            // how the spec itself states results); this oracle returns
            // straight alpha, so unpremultiply the expected vector the same
            // way `to_straight` does before comparing.
            let ao = premultiplied_expected[3];
            let expected = if ao <= 0.0 {
                [0.0, 0.0, 0.0, 0.0]
            } else {
                [
                    premultiplied_expected[0] / ao,
                    premultiplied_expected[1] / ao,
                    premultiplied_expected[2] / ao,
                    ao,
                ]
            };
            let actual = oracle_blend_top_level(mode, src, dst);
            assert_straight_close(actual, expected, eps, mode);
        }
    }

    /// Well-known blend-mode identities, independent of any specific
    /// numeric vector — a second, structurally different kind of check from
    /// `known_vectors`.
    #[test]
    fn known_identities() {
        let red = Straight {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let blue = Straight {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        };
        let white = Straight {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let black = Straight {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };

        assert_straight_close(
            oracle_blend_top_level(BlendMode::Normal, red, blue),
            [1.0, 0.0, 0.0, 1.0],
            1e-9,
            BlendMode::Normal,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Multiply, white, blue),
            [0.0, 0.0, 1.0, 1.0],
            1e-9,
            BlendMode::Multiply,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Screen, black, blue),
            [0.0, 0.0, 1.0, 1.0],
            1e-9,
            BlendMode::Screen,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Difference, blue, blue),
            [0.0, 0.0, 0.0, 1.0],
            1e-9,
            BlendMode::Difference,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Clear, red, blue),
            [0.0, 0.0, 0.0, 0.0],
            1e-9,
            BlendMode::Clear,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Src, red, blue),
            [1.0, 0.0, 0.0, 1.0],
            1e-9,
            BlendMode::Src,
        );
        assert_straight_close(
            oracle_blend_top_level(BlendMode::Dst, red, blue),
            [0.0, 0.0, 1.0, 1.0],
            1e-9,
            BlendMode::Dst,
        );
    }

    /// The actual pillar-F deliverable: `color.rs`'s shipped `blend`,
    /// diffed against this independently-coded oracle — not against
    /// itself — across every mode and a representative grid of
    /// premultiplied colour pairs, tolerance split by category rather than
    /// one global epsilon.
    #[test]
    fn agrees_with_the_shipped_renderer() {
        let colors = [
            Color::rgba(255, 0, 0, 255),
            Color::rgba(0, 200, 120, 255),
            Color::rgba(30, 60, 200, 255),
            Color::rgba(255, 255, 255, 255),
            Color::rgba(0, 0, 0, 255),
            Color::rgba(120, 40, 90, 180), // translucent
            Color::rgba(10, 220, 240, 60), // mostly transparent
            Color::rgba(255, 255, 255, 0), // fully transparent, non-zero colour
        ];

        for mode in BlendMode::ALL {
            // Coverage-only Porter-Duff operators are pure linear algebra on
            // premultiplied channels with no division and no per-channel
            // branch — an f32-vs-f64 rounding difference between the two
            // implementations should not exceed a couple of float ULPs at
            // this magnitude.
            let eps = if separable_fn(mode).is_some() || non_separable_fn(mode).is_some() {
                // Separable/non-separable modes go through an unpremultiply
                // (a division), a per-channel nonlinear function, and a
                // re-premultiply — three more f32-rounding opportunities
                // than a pure Porter-Duff operator, so a looser (but still
                // tight) bound.
                3e-3
            } else {
                5e-5
            };

            for &src_color in &colors {
                for &dst_color in &colors {
                    let src = Premul::from_straight(src_color);
                    let dst = Premul::from_straight(dst_color);

                    let shipped = shipped_blend(mode, src, dst);
                    let oracle = to_premul(oracle_blend_top_level(
                        mode,
                        to_straight(src),
                        to_straight(dst),
                    ));

                    for (channel, (s, o)) in [
                        (shipped.r, oracle.r),
                        (shipped.g, oracle.g),
                        (shipped.b, oracle.b),
                        (shipped.a, oracle.a),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        assert!(
                            approx_eq(f64::from(s), f64::from(o), eps),
                            "{mode:?} channel {channel}: shipped={s}, oracle={o} (src={src_color:?}, dst={dst_color:?}, eps={eps})"
                        );
                    }
                }
            }
        }
    }
}
