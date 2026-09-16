//! `DrawImage`: straight-alpha RGBA8 source, sampled with a bilinear filter.
//!
//! Spec §11.1: "Decoded images arrive as straight-alpha RGBA8 ... the GPU
//! must premultiply at use, not at upload, or cached uploads get
//! double-converted." This module premultiplies at sample time for exactly
//! that reason: [`vieww_foundation::Image`] is never mutated or re-encoded,
//! so two draws of the same `Image` — one plain, one inside a faded layer —
//! can never disagree about what "the source" was.

use vieww_foundation::{Image, Offset, Rect, Transform};

use super::color::Premul;

/// The same sample, with the inverse mapping already in hand.
///
/// # Why this exists
///
/// Inverting the transform per call is what this replaced, and it is called
/// **once per destination pixel**. Drawing an 860x538 screenshot is 460 000 pixels, so it
/// was 460 000 matrix inversions per frame — measured at 47 ms to draw one
/// picture, or about 90 ns a pixel for what is otherwise a filtered copy. The
/// mapping is the same for every pixel of one `DrawImage`, so the caller
/// inverts once and passes it in; the inner loop keeps only the arithmetic
/// that actually varies with the pixel.
///
/// The result is identical for every pixel — this is a hoist, not an
/// approximation.
#[must_use]
pub(crate) fn sample_device_pixel_with(
    image: &Image,
    rect: Rect,
    inverse: Transform,
    x: f32,
    y: f32,
) -> Option<Premul> {
    let local = inverse.apply(Offset::new(x, y));
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let u = (local.dx - rect.left) / rect.width();
    let v = (local.dy - rect.top) / rect.height();
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    let sx = u * image.width() as f32 - 0.5;
    let sy = v * image.height() as f32 - 0.5;
    Some(bilinear(image, sx, sy))
}

fn bilinear(image: &Image, x: f32, y: f32) -> Premul {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;

    // **An image drawn at its own size lands on texel centres.** Then both
    // fractions are zero, `lerp(a, b, 0.0)` is `a` twice over, and the whole
    // filter reduces to `c00` — so the other three fetches, the four `Premul`
    // conversions they cost and the three lerps are all computing a number
    // that is thrown away. That is the common case on a page: a screenshot
    // shown at its natural width, which is 460 000 pixels of it.
    //
    // This is exact, not an approximation — it is the same value the general
    // path below returns, which is why it can be taken without a flag.
    if fx == 0.0 && fy == 0.0 {
        let ix = x0.clamp(0.0, image.width() as f32 - 1.0) as u32;
        let iy = y0.clamp(0.0, image.height() as f32 - 1.0) as u32;
        return texel(image, ix, iy);
    }
    let p = |ix: f32, iy: f32| -> Premul {
        let ix = ix.clamp(0.0, image.width() as f32 - 1.0) as u32;
        let iy = iy.clamp(0.0, image.height() as f32 - 1.0) as u32;
        texel(image, ix, iy)
    };
    let c00 = p(x0, y0);
    let c10 = p(x0 + 1.0, y0);
    let c01 = p(x0, y0 + 1.0);
    let c11 = p(x0 + 1.0, y0 + 1.0);
    let lerp = |a: Premul, b: Premul, t: f32| Premul {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    };
    let top = lerp(c00, c10, fx);
    let bottom = lerp(c01, c11, fx);
    lerp(top, bottom, fy)
}

fn texel(image: &Image, x: u32, y: u32) -> Premul {
    let data = image.pixels();
    let idx = ((y * image.width() + x) * 4) as usize;
    let a = f32::from(data[idx + 3]) / 255.0;
    Premul {
        r: f32::from(data[idx]) / 255.0 * a,
        g: f32::from(data[idx + 1]) / 255.0 * a,
        b: f32::from(data[idx + 2]) / 255.0 * a,
        a,
    }
}

pub(crate) fn invert(t: Transform) -> Option<Transform> {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-9 {
        return None;
    }
    let inv_det = 1.0 / det;
    let a = t.d * inv_det;
    let b = -t.b * inv_det;
    let c = -t.c * inv_det;
    let d = t.a * inv_det;
    let tx = -(a * t.tx + c * t.ty);
    let ty = -(b * t.tx + d * t.ty);
    Some(Transform { a, b, c, d, tx, ty })
}

// ─────────────────────────────────────────────────────────────────────────
// Mipmaps: minification without aliasing
// ─────────────────────────────────────────────────────────────────────────
//
// Bilinear sampling of a full-resolution image into a fraction of its size is
// the classic minification alias: each destination pixel touches a *footprint*
// of source texels — eight, thirty, a hundred of them — and the filter reads
// exactly four, chosen by where the footprint's centre happens to land. A
// high-contrast checkerboard shrunk 8× does not become grey; it becomes a
// moiré of random black and white, because the sample position drifts in and
// out of alignment with the pattern. Every serious sampler solves this with a
// **mip pyramid** — a chain of half-resolution copies, each pre-averaged over
// 2×2 texels, so a filter can read from the level whose texel size matches
// the destination footprint, and trilinearly blend the two bracketing levels
// while a scale animation crosses from one to the next.
//
// The chain is generated on demand, cached by image identity, and used only
// when the destination actually minifies — a 1:1 or magnified image takes
// exactly the pixels it always did, which is what keeps every committed
// golden image valid: the mipped path changes *only* the case that was
// producing aliasing.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

/// How many source texels one device pixel's sampling footprint covers,
/// conservatively (the smaller of the horizontal and vertical rates).
///
/// One device pixel, pushed backwards through the inverse transform, becomes
/// a footprint in the image's local space; scaled by the source-to-rect
/// ratio it becomes a texel rate. `≥ 2.0` means genuine minification — one
/// pixel is trying to average four or more texels and the filter cannot do
/// it, so the mip path takes over. `1.0` is a pixel-perfect 1:1, and `< 1.0`
/// is magnification, which bilinear handles as well as anything can.
///
/// The two axes are measured independently and the *smaller* rate decides,
/// because that is the conservative direction: an image squeezed to half
/// height but full width is minified vertically and must not be sampled from
/// a half-resolution level horizontally.
#[must_use]
pub(crate) fn minification_ratio(image: &Image, rect: Rect, inverse: &Transform) -> f32 {
    if rect.width() <= 0.0 || rect.height() <= 0.0 || image.width() == 0 || image.height() == 0 {
        return 1.0;
    }
    // One device pixel along +x maps to a local-space delta of
    // `(inverse.a, inverse.b)`; along +y, `(inverse.c, inverse.d)`. Their
    // lengths are the local-space footprint of one pixel on each axis.
    let local_per_px_x = (inverse.a * inverse.a + inverse.b * inverse.b).sqrt();
    let local_per_px_y = (inverse.c * inverse.c + inverse.d * inverse.d).sqrt();
    let texels_per_px_x = local_per_px_x * image.width() as f32 / rect.width();
    let texels_per_px_y = local_per_px_y * image.height() as f32 / rect.height();
    texels_per_px_x.min(texels_per_px_y).max(1.0)
}

/// The mipmap chain for one image, `0.5×` per level, down to 1×1.
///
/// Levels are stored as ordinary [`Image`]s — the same straight-alpha RGBA8
/// the source is — generated with a premultiplied box filter so that
/// transparent texels do not bleed their colour into opaque neighbours (the
/// classic "dark halo around a shrunken sprite" bug, which a naive straight
/// average of straight RGB produces whenever alpha varies).
/// A source image's allocation address and size.
type ChainKey = (*const u8, u32, u32);
/// The source it was built from (for the identity check) and the pyramid.
type ChainEntry = (Weak<Vec<u8>>, Arc<Vec<Image>>);

#[derive(Debug, Default)]
pub(crate) struct MipCache {
    /// Chains keyed by the source image's allocation identity, each with a
    /// `Weak` to that allocation.
    ///
    /// `Arc` address rather than content, for the same reason [`Image`]'s own
    /// `PartialEq` uses it: a chain regenerated for a *new* image with
    /// identical pixels is a wasted millisecond, not a wrong pixel.
    ///
    /// **The `Weak` is what makes the address an identity.** This comment used
    /// to say the cache "holds an `Arc` clone" of the image so the key "can
    /// never dangle"; it held only the mips. Once an image was dropped, the
    /// allocator could give its address to the next image of the same size,
    /// which was then drawn from the dead image's pyramid — at allocator whim,
    /// so it showed up as an intermittent wrong minified image (found through
    /// the GPU path, which shares this cache's design through `GpuSeam`). A
    /// live `Weak` pins the allocation, so the address cannot be reused while
    /// the entry exists, and a dead one marks the entry stale.
    chains: HashMap<ChainKey, ChainEntry>,
    /// How many chains have been generated — counted work, so the claim
    /// "minification does not re-filter the pyramid per frame" is a number.
    generated: usize,
}

impl MipCache {
    /// The chain for `image`, generating it on first ask.
    ///
    /// The returned `Arc` is shared between asks, so a scrolling list of
    /// thumbnails pays the pyramid once per image and then holds the same
    /// weights for every frame after that.
    pub(crate) fn chain(&mut self, image: &Image) -> Arc<Vec<Image>> {
        let key = (
            Arc::as_ptr(image.shared()) as *const u8,
            image.width(),
            image.height(),
        );
        if let Some((source, chain)) = self.chains.get(&key) {
            if source
                .upgrade()
                .is_some_and(|live| Arc::ptr_eq(&live, image.shared()))
            {
                return Arc::clone(chain);
            }
        }
        // A miss, or a stale entry for a dead image at a reused address. Dead
        // entries are dropped here too, so a scene that makes a new image
        // every frame does not grow this map without bound.
        self.chains
            .retain(|_, (source, _)| source.strong_count() > 0);
        self.generated += 1;
        let chain = Arc::new(generate_mips(image));
        self.chains
            .insert(key, (Arc::downgrade(image.shared()), Arc::clone(&chain)));
        chain
    }

    /// How many pyramids this cache has generated.
    pub(crate) fn generated_chains(&self) -> usize {
        self.generated
    }

    /// Drop every chain — `Trim` under pressure, or a resize that invalidates
    /// nothing but is cheap to honour.
    pub(crate) fn clear(&mut self) {
        self.chains.clear();
    }
}

/// Build the pyramid: `mips[0]` is a half-resolution copy of `image`,
/// `mips[1]` a half of that, and so on to 1×1.
///
/// Stops at 1×1 on the longer axis — a level narrower than the bilinear
/// filter's 2×2 footprint saves nothing but blur.
fn generate_mips(image: &Image) -> Vec<Image> {
    let mut mips = Vec::new();
    let mut width = image.width();
    let mut height = image.height();
    let mut source = None;
    while width > 1 && height > 1 {
        width /= 2;
        height /= 2;
        let halved = downsample_2x2(source.as_ref().unwrap_or(image), width, height);
        source = Some(halved.clone());
        mips.push(halved);
    }
    mips
}

/// One 2×2 box-filter step, in premultiplied space, back to straight RGBA8.
fn downsample_2x2(source: &Image, width: u32, height: u32) -> Image {
    let data = source.pixels();
    let (sw, sh) = (source.width(), source.height());
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        // The 2×2 rows this destination row averages, clamped at the bottom
        // edge for odd heights (a 5-row source halves to 3; the last row
        // re-uses the source's last row for its second half).
        let row_a = (y * 2) as usize;
        let row_b = ((y * 2 + 1).min(sh - 1)) as usize;
        for x in 0..width {
            let col_a = (x * 2) as usize;
            let col_b = ((x * 2 + 1).min(sw - 1)) as usize;
            let read = |row: usize, col: usize| -> [f32; 4] {
                let i = (row * sw as usize + col) * 4;
                let a = f32::from(data[i + 3]) / 255.0;
                [
                    f32::from(data[i]) / 255.0 * a,
                    f32::from(data[i + 1]) / 255.0 * a,
                    f32::from(data[i + 2]) / 255.0 * a,
                    a,
                ]
            };
            let corners = [
                read(row_a, col_a),
                read(row_a, col_b),
                read(row_b, col_a),
                read(row_b, col_b),
            ];
            let mut sum = [0.0f32; 4];
            for corner in &corners {
                for c in 0..4 {
                    sum[c] += corner[c];
                }
            }
            for v in &mut sum {
                *v *= 0.25;
            }
            if sum[3] <= f32::EPSILON {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let inv = 1.0 / sum[3];
                let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                out.extend_from_slice(&[
                    to_u8(sum[0] * inv),
                    to_u8(sum[1] * inv),
                    to_u8(sum[2] * inv),
                    to_u8(sum[3]),
                ]);
            }
        }
    }
    Image::from_rgba8(out, width, height)
}

/// The same sample as [`sample_device_pixel_with`], read trilinearly from a
/// mip pyramid when the destination minifies.
///
/// `ratio` is [`minification_ratio`]'s answer for this draw. Below 2 the
/// caller should not be here — the base image is the right level and the
/// sample is byte-for-byte what [`sample_device_pixel_with`] returns. At and
/// above 2, the filter reads the two bracketing levels — `floor` and
/// `floor + 1` — bilinearly and blends them by the fractional part, so a
/// scale animation crosses level boundaries without a visible pop.
pub(crate) fn sample_device_pixel_mipped(
    chain: &[Image],
    base: &Image,
    rect: Rect,
    inverse: Transform,
    x: f32,
    y: f32,
    ratio: f32,
) -> Option<Premul> {
    if chain.is_empty() {
        return sample_device_pixel_with(base, rect, inverse, x, y);
    }
    let local = inverse.apply(Offset::new(x, y));
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    let u = (local.dx - rect.left) / rect.width();
    let v = (local.dy - rect.top) / rect.height();
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    // Level 0 is the base image itself; level i is chain[i - 1]. The ideal
    // level is log2 of the texel rate, clamped to the pyramid's floor.
    let max_level = chain.len() as f32;
    let level = (ratio.log2().clamp(0.0, max_level)).min(max_level);
    let lower = level.floor();
    let frac = level - lower;
    let lower_image = if lower == 0.0 {
        base
    } else {
        &chain[(lower as usize) - 1]
    };
    let sample_at = |image: &Image| -> Premul {
        let sx = u * image.width() as f32 - 0.5;
        let sy = v * image.height() as f32 - 0.5;
        bilinear(image, sx, sy)
    };
    let low = sample_at(lower_image);
    if frac <= 0.0 {
        return Some(low);
    }
    let upper_image = &chain[(lower as usize).min(chain.len() - 1)];
    let high = sample_at(upper_image);
    Some(Premul {
        r: low.r + (high.r - low.r) * frac,
        g: low.g + (high.g - low.g) * frac,
        b: low.b + (high.b - low.b) * frac,
        a: low.a + (high.a - low.a) * frac,
    })
}

#[cfg(test)]
mod mip_tests {
    use super::*;

    fn image_of(w: u32, h: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> Image {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                pixels.extend_from_slice(&fill(x, y));
            }
        }
        Image::from_rgba8(pixels, w, h)
    }

    /// Images made and dropped one after another — the allocator hands the
    /// next one the previous one's address far more often than chance. Each
    /// must get its own pyramid, never the dead image's.
    #[test]
    fn a_new_image_at_a_dead_images_address_gets_its_own_chain() {
        let mut cache = MipCache::default();
        for i in 0..64u32 {
            let value = (i * 3 % 250) as u8;
            let img = image_of(8, 8, |_, _| [value, value, value, 255]);
            let chain = cache.chain(&img);
            assert_eq!(
                chain[0].pixels()[0],
                value,
                "image {i} was drawn from another image's pyramid"
            );
            drop(img);
        }
        assert!(
            cache.chains.len() <= 2,
            "dead chains are pruned, not accumulated"
        );
    }

    /// The pyramid must be a real halving chain whose final level is the
    /// flat average of the whole source: a half-black, half-white 8×8 ends
    /// at a grey 1×1, not at black-bleed or white-bleed.
    #[test]
    fn mip_chain_halves_and_averages() {
        let source = image_of(8, 8, |x, _| {
            if x < 4 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let mips = generate_mips(&source);
        assert_eq!(
            mips.iter().map(Image::size).collect::<Vec<_>>(),
            vec![
                vieww_foundation::Size::new(4.0, 4.0),
                vieww_foundation::Size::new(2.0, 2.0),
                vieww_foundation::Size::new(1.0, 1.0),
            ],
            "8×8 halves to 4×4, 2×2, 1×1"
        );
        // The 1×1 level is the average of the whole source — half white,
        // half black — so grey, and opaque.
        let data = mips[2].pixels();
        assert!((data[0] as i32 - 128).abs() <= 1, "grey, not bleed");
        assert_eq!(data[3], 255, "opaque stays opaque");
    }

    /// An 8× checkerboard minified 8× through the mip path must average
    /// towards grey — the whole point of the pyramid. Sampling the base
    /// image directly at the same positions produces black or white
    /// depending on alignment, which is the alias being fixed.
    #[test]
    fn a_minified_checkerboard_averages_instead_of_aliaging() {
        // 2-pixel cells: each destination pixel's 8×8 source footprint spans
        // several cells, so the correct sample is an average, and a cell
        // aligned to the footprint would still be an extreme — which is what
        // the un-mipped sampler produces.
        let source = image_of(64, 64, |x, y| {
            if (x / 2 + y / 2) % 2 == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let chain = generate_mips(&source);
        let rect = Rect::new(0.0, 0.0, 8.0, 8.0); // 8× minification
        let inverse = Transform::IDENTITY;

        // The mipped sample anywhere inside the rect is somewhere between
        // the two colours — never an extreme.
        for (x, y) in [(2.5, 2.5), (5.5, 6.5), (0.5, 0.5)] {
            let mipped = sample_device_pixel_mipped(&chain, &source, rect, inverse, x, y, 8.0)
                .expect("inside the rect");
            assert!(
                mipped.r > 0.1 && mipped.r < 0.9,
                "a minified checkerboard averages (r = {}), it does not alias \
                 to one of its extremes",
                mipped.r
            );
        }

        // The mipped sampler's answer must also be *stable* under a
        // sub-pixel shift of the sample position — the property whose
        // absence is the visible alias. (The un-mipped sampler can't be
        // asserted to hit an extreme at any *chosen* position, because at an
        // 8× minification its four-texel footprint straddles cells and often
        // averages by accident; the moiré appears in which four texels it
        // straddles, which is position drift, not a fixed extreme.)
        let a = sample_device_pixel_mipped(&chain, &source, rect, inverse, 4.32, 4.71, 8.0)
            .expect("inside the rect")
            .r;
        let b = sample_device_pixel_mipped(&chain, &source, rect, inverse, 4.68, 4.29, 8.0)
            .expect("inside the rect")
            .r;
        assert!(
            (a - b).abs() < 0.05,
            "the mipped answer is stable under sub-pixel drift ({a} vs {b})"
        );
    }

    /// 1:1 and magnified draws must not touch the pyramid at all — the
    /// mipped sampler is byte-identical to the plain one below ratio 2.
    #[test]
    fn a_one_to_one_draw_matches_the_plain_sampler_exactly() {
        let source = image_of(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 128, 255]);
        let chain = generate_mips(&source);
        let rect = Rect::new(0.0, 0.0, 16.0, 16.0);
        for y in 0..16 {
            for x in 0..16 {
                let px = sample_device_pixel_with(
                    &source,
                    rect,
                    Transform::IDENTITY,
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                )
                .expect("plain");
                let mipped = sample_device_pixel_mipped(
                    &chain,
                    &source,
                    rect,
                    Transform::IDENTITY,
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    1.0,
                )
                .expect("mipped");
                assert_eq!(
                    (px.r, px.g, px.b, px.a),
                    (mipped.r, mipped.g, mipped.b, mipped.a)
                );
            }
        }
    }
}
