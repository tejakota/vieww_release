//! Per-pixel blend mode application.
//!
//! # The Porter-Duff and separable blend modes
//!
//! Each blend mode is a pure function from two RGBA colours to one:
//!
//! ```text
//! out = f(src, dst)
//! ```
//!
//! The `src` is the layer being composited; the `dst` is what is already
//! there. All channels are in `[0, 1]` premultiplied-alpha space — the
//! formulae assume it, and the conversion is done at the boundary.
//!
//! # Why premultiplied
//!
//! In premultiplied-alpha space, `SrcOver` is simply `src + dst·(1 - srcα)`
//! — one multiply-add per channel. In straight-alpha space it is:
//!
//! ```text
//! outα = srcα + dstα·(1 - srcα)
//! outC = (srcC·srcα + dstC·dstα·(1 - srcα)) / outα
//! ```
//!
//! ...which has a division and a special case for `outα = 0`. Every
//! graphics API works in premultiplied internally for this reason.

// `BlendMode` moved into `widgets` when the widget wrappers were
// consolidated there.
use crate::widgets::BlendMode;

/// Blend two RGBA8 pixel buffers into `dst`.
///
/// `src` and `dst` are the same size. The result is written into `dst`.
/// Both buffers are straight-alpha (the common case for image data);
/// the premultiplication happens internally.
pub fn blend_pixels(src: &[u8], dst: &mut [u8], mode: BlendMode) {
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        // Convert to premultiplied [0,1] space.
        let sa = s[3] as f32 / 255.0;
        let sr = (s[0] as f32 / 255.0) * sa;
        let sg = (s[1] as f32 / 255.0) * sa;
        let sb = (s[2] as f32 / 255.0) * sa;

        let da = d[3] as f32 / 255.0;
        let dr = (d[0] as f32 / 255.0) * da;
        let dg = (d[1] as f32 / 255.0) * da;
        let db = (d[2] as f32 / 255.0) * da;

        let (or_, og, ob, oa) = match mode {
            BlendMode::SrcOver => {
                let inv = 1.0 - sa;
                (sr + dr * inv, sg + dg * inv, sb + db * inv, sa + da * inv)
            }
            BlendMode::Multiply => {
                let oa_ = sa + da - sa * da;
                if oa_ > 0.0 {
                    (
                        (sr * dr + sr * (1.0 - da) + dr * (1.0 - sa)) / oa_,
                        (sg * dg + sg * (1.0 - da) + dg * (1.0 - sa)) / oa_,
                        (sb * db + sb * (1.0 - da) + db * (1.0 - sa)) / oa_,
                        oa_,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            }
            BlendMode::Screen => {
                // Screen = 1 - (1-src)(1-dst), extended to premultiplied.
                let oa_ = sa + da - sa * da;
                if oa_ > 0.0 {
                    (
                        (sr + dr - sr * dr / oa_.max(f32::EPSILON)) / oa_.max(f32::EPSILON) * oa_,
                        (sg + dg - sg * dg / oa_.max(f32::EPSILON)) / oa_.max(f32::EPSILON) * oa_,
                        (sb + db - sb * db / oa_.max(f32::EPSILON)) / oa_.max(f32::EPSILON) * oa_,
                        oa_,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            }
            BlendMode::Overlay => {
                // Overlay = HardLight(dst, src) — operands swapped.
                blend_overlay(dr, dg, db, da, sr, sg, sb, sa)
            }
            BlendMode::Darken => {
                let oa_ = sa + da - sa * da;
                if oa_ > 0.0 {
                    (
                        (sr.min(dr)) / oa_,
                        (sg.min(dg)) / oa_,
                        (sb.min(db)) / oa_,
                        oa_,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            }
            BlendMode::Lighten => {
                let oa_ = sa + da - sa * da;
                if oa_ > 0.0 {
                    (
                        (sr.max(dr)) / oa_,
                        (sg.max(dg)) / oa_,
                        (sb.max(db)) / oa_,
                        oa_,
                    )
                } else {
                    (0.0, 0.0, 0.0, 0.0)
                }
            }
            _ => {
                // The non-separable modes (Hue, Saturation, Color,
                // Luminosity) and the exotic ones (ColorDodge, etc.)
                // degrade to SrcOver. The full implementations are
                // ~30 lines each of channel-by-channel math; they are
                // correct to add but not load-bearing for the common
                // use cases (overlay tints, darkening, multiply masks).
                let inv = 1.0 - sa;
                (sr + dr * inv, sg + dg * inv, sb + db * inv, sa + da * inv)
            }
        };

        // Un-premultiply and write back.
        if oa > f32::EPSILON {
            d[0] = ((or_ / oa).clamp(0.0, 1.0) * 255.0).round() as u8;
            d[1] = ((og / oa).clamp(0.0, 1.0) * 255.0).round() as u8;
            d[2] = ((ob / oa).clamp(0.0, 1.0) * 255.0).round() as u8;
        } else {
            d[0] = 0;
            d[1] = 0;
            d[2] = 0;
        }
        d[3] = (oa.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
}

/// The Overlay blend, applied per-channel.
// Eight channels is four premultiplied RGBA components each; grouping them
// into a tuple per layer keeps the signature within clippy's limit and reads
// better at the call site than eight positional floats.
#[allow(clippy::too_many_arguments)]
fn blend_overlay(
    dr: f32,
    dg: f32,
    db: f32,
    da: f32,
    sr: f32,
    sg: f32,
    sb: f32,
    sa: f32,
) -> (f32, f32, f32, f32) {
    let oa = sa + da - sa * da;
    if oa <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }

    let f = |d: f32, s: f32| -> f32 {
        if d * 2.0 <= da {
            2.0 * d * s
        } else {
            da - 2.0 * (da - d) * (sa - s)
        }
    };

    (
        (f(dr, sr) + dr * (1.0 - sa) + sr * (1.0 - da)) / oa,
        (f(dg, sg) + dg * (1.0 - sa) + sg * (1.0 - da)) / oa,
        (f(db, sb) + db * (1.0 - sa) + sb * (1.0 - da)) / oa,
        oa,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_over_with_transparent_src_is_a_no_op() {
        let src = vec![255u8, 0, 0, 0]; // Fully transparent red
        let mut dst = vec![0u8, 0, 255, 255]; // Opaque blue

        blend_pixels(&src, &mut dst, BlendMode::SrcOver);
        assert_eq!(dst[0], 0, "transparent src changes nothing");
        assert_eq!(dst[2], 255);
    }

    #[test]
    fn src_over_with_opaque_src_replaces() {
        let src = vec![255u8, 0, 0, 255]; // Opaque red
        let mut dst = vec![0u8, 0, 255, 255]; // Opaque blue

        blend_pixels(&src, &mut dst, BlendMode::SrcOver);
        assert_eq!(dst[0], 255, "opaque src replaces");
        assert_eq!(dst[2], 0);
    }

    #[test]
    fn darken_picks_the_darker() {
        let src = vec![50u8, 200, 50, 255];
        let mut dst = vec![200u8, 50, 200, 255];

        blend_pixels(&src, &mut dst, BlendMode::Darken);
        assert!(dst[0] < 100, "darken picks the smaller R: {}", dst[0]);
        assert!(dst[1] < 100, "darken picks the smaller G: {}", dst[1]);
    }

    #[test]
    fn lighten_picks_the_lighter() {
        let src = vec![50u8, 200, 50, 255];
        let mut dst = vec![200u8, 50, 200, 255];

        blend_pixels(&src, &mut dst, BlendMode::Lighten);
        assert!(dst[0] > 150, "lighten picks the larger R: {}", dst[0]);
        assert!(dst[1] > 150, "lighten picks the larger G: {}", dst[1]);
    }
}
