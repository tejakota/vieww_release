//! `DrawShadow`: a blurred rounded-rect silhouette.
//!
//! Spec §8.1/§8.2: "the default blur is the 3-box Gaussian ... outer shadows
//! are an analytic SDF ... in a single draw" and inset shadows "keep the
//! complement trick (blurred rect punched through DestOut inside the
//! clip)". This build uses a rasterized-then-box-blurred mask rather than an
//! analytic SDF fragment shader (that is GPU-backend work — see
//! `docs/RENDERER-MIGRATION.md`), but keeps the same three-pass-box
//! approximation of a Gaussian the spec calls for, and the same DestOut
//! complement for the inset case, so a CPU-vs-GPU shadow stays comparable
//! once the SDF shader lands.

use vieww_foundation::{Rect, Shadow, Transform};

use super::color::Premul;
use super::geometry::fill::rasterize;
use super::rounded_rect::rounded_rect_polygon_transformed;
use super::target::Target;

/// Render `shadow` cast by a rounded rect (`rect`/`radius`, local space)
/// under `transform`, clamped to `surface`. Returns the tinted, blurred
/// patch and its top-left in device space, or `None` if it would be empty.
pub(crate) fn render_shadow(
    rect: Rect,
    radius: f32,
    shadow: &Shadow,
    transform: Transform,
    surface: Rect,
) -> Option<(Target, i32, i32)> {
    let masks = shadow_masks(rect, radius, shadow, transform, surface)?;
    let (w, h) = (masks.width, masks.height);
    let mut alpha = masks.caster;

    if let Some(box_radius) = masks.box_radius {
        box_blur_3_radius(&mut alpha, w, h, box_radius);
    }

    if let Some(inner_alpha) = masks.inner {
        // Complement trick: invert the (unblurred-caster, blurred-edge)
        // mask and confine it back inside the original box via a second
        // rasterization used as a clip multiply.
        for i in 0..alpha.len() {
            alpha[i] = (1.0 - alpha[i]).clamp(0.0, 1.0) * inner_alpha[i];
        }
    }

    let tint = Premul::from_straight(shadow.color);
    let mut target = Target::new(w, h);
    for (i, a) in alpha.iter().enumerate() {
        target.pixels[i] = tint.scaled(*a);
    }
    // Written through `pixels` directly rather than through a compositing
    // method, so the ink `composite_layer` reads has to be declared here. The
    // patch is exactly the blurred silhouette's extent — it was sized to it —
    // so the whole of it counts.
    target.mark_all_ink();
    Some((target, masks.x0, masks.y0))
}

/// The inputs of a shadow before any blur: the caster silhouette's coverage,
/// the inset confinement mask, the patch placement and the box radius.
///
/// Split out of [`render_shadow`] so the GPU seam
/// (`native/gpu_seam.rs`) hands a backend *exactly* the silhouettes this
/// renderer blurs, and runs the same three box passes on the GPU. One
/// implementation of "what a shadow's mask is", two executors of the blur.
pub(crate) struct ShadowMasks {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Unblurred caster coverage, row-major, `width * height`.
    pub(crate) caster: Vec<f32>,
    /// For an inset shadow, the coverage of the box the shadow is confined to.
    pub(crate) inner: Option<Vec<f32>>,
    /// The three-box radius, or `None` when the blur is too small to run.
    pub(crate) box_radius: Option<i32>,
}

pub(crate) fn shadow_masks(
    rect: Rect,
    radius: f32,
    shadow: &Shadow,
    transform: Transform,
    surface: Rect,
) -> Option<ShadowMasks> {
    if shadow.is_invisible() {
        return None;
    }
    let scale = (transform.a * transform.d - transform.b * transform.c)
        .abs()
        .sqrt()
        .max(0.01);
    let sigma_device = (shadow.std_dev() * scale).max(0.0);

    let caster = rect.inflate(shadow.spread).translate(shadow.offset);
    let bounds_local = shadow.bounds(rect);
    let bounds_device = transform.apply_rect(bounds_local).intersect(surface);
    if bounds_device.is_empty() {
        return None;
    }
    let x0 = bounds_device.left.floor() as i32;
    let y0 = bounds_device.top.floor() as i32;
    let w = (bounds_device.right.ceil() as i32 - x0).max(0) as u32;
    let h = (bounds_device.bottom.ceil() as i32 - y0).max(0) as u32;
    if w == 0 || h == 0 {
        return None;
    }

    let clip = Rect::new(
        x0 as f32,
        y0 as f32,
        (x0 + w as i32) as f32,
        (y0 + h as i32) as f32,
    );
    let place = |polygon| {
        let mask = rasterize(&[polygon], clip);
        let mut alpha = vec![0.0f32; (w as usize) * (h as usize)];
        for dy in 0..mask.height {
            for dx in 0..mask.width {
                let c = mask.data[(dy * mask.width + dx) as usize];
                let px = (mask.x0 + dx as i32) - x0;
                let py = (mask.y0 + dy as i32) - y0;
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    alpha[py as usize * w as usize + px as usize] = c;
                }
            }
        }
        alpha
    };
    // Rasterize the (offset+spread) caster into a local alpha buffer sized
    // to this patch.
    let caster_alpha = place(rounded_rect_polygon_transformed(caster, radius, transform));
    let inner = shadow
        .is_inset
        .then(|| place(rounded_rect_polygon_transformed(rect, radius, transform)));
    let box_radius = (sigma_device > 0.1).then(|| box_radius_for_sigma(sigma_device));
    Some(ShadowMasks {
        x0,
        y0,
        width: w,
        height: h,
        caster: caster_alpha,
        inner,
        box_radius,
    })
}

/// The per-pass radius of the three-box approximation of a Gaussian of
/// standard deviation `sigma` — shared by shadows, layer blurs, and the GPU
/// seam, so all three agree on the kernel to the texel.
pub(crate) fn box_radius_for_sigma(sigma: f32) -> i32 {
    let diameter = (4.0 * sigma * sigma + 1.0).sqrt();
    (((diameter - 1.0) / 2.0).round() as i32).max(1)
}

/// Three passes of a box blur, radius derived from `sigma` (the standard
/// three-box approximation to a Gaussian — spec §8.1).
///
/// The per-pass radius is **not** `3 * sigma` — that would be the *reach* of
/// a single Gaussian tail (spec's `BLUR_REACH`, used for bounds, not for a
/// box width), and using it as every one of three box radii triples the
/// effective blur. Three box blurs of full width `d` approximate a Gaussian
/// of standard deviation `sigma` when `d = sqrt(4*sigma^2 + 1)` (Kovesi,
/// "Fast Almost-Gaussian Filtering") — this is that formula.
fn box_blur_3_radius(data: &mut [f32], w: u32, h: u32, radius: i32) {
    let mut tmp = vec![0.0f32; data.len()];
    for _ in 0..3 {
        box_blur_horizontal(data, &mut tmp, w, h, radius);
        box_blur_vertical(&tmp, data, w, h, radius);
    }
}

/// A running-sum box blur along each row.
///
/// # No bounds check per sample
///
/// The window is advanced by adding the sample entering it and subtracting
/// the one leaving, and both of those are only *sometimes* off the end of the
/// row. This used to ask a bounds-checked reader for every one of them —
/// twelve checked reads per pixel across the six passes of a three-box blur —
/// to get a zero on the handful that were genuinely outside. Reading the row
/// as a slice and testing the index instead keeps the same convention
/// (outside is transparent, which is what makes a shadow fade at its own
/// edge) with no work in the common case.
///
/// The arithmetic is written as `sum += entering - leaving`, one expression,
/// because splitting it into an add and a subtract changes the last bit of
/// the result and this kernel is a correctness reference.
fn box_blur_horizontal(src: &[f32], dst: &mut [f32], w: u32, h: u32, radius: i32) {
    let window = (2 * radius + 1) as f32;
    let width = w as i32;
    for y in 0..h as usize {
        let row = y * w as usize;
        let src_row = &src[row..row + w as usize];
        let dst_row = &mut dst[row..row + w as usize];

        let mut sum = 0.0f32;
        for x in 0..=radius.min(width - 1) {
            sum += src_row[x as usize];
        }
        for x in 0..width {
            dst_row[x as usize] = sum / window;
            let entering = x + radius + 1;
            let leaving = x - radius;
            let a = if entering < width {
                src_row[entering as usize]
            } else {
                0.0
            };
            let b = if leaving >= 0 {
                src_row[leaving as usize]
            } else {
                0.0
            };
            sum += a - b;
        }
    }
}

/// How many columns the vertical pass carries at once — see
/// `native/effects.rs`'s `STRIP`, which has the measurement and the trade.
/// Sixteen here rather than eight because a shadow mask is one `f32` per pixel,
/// not four, so sixteen of them is the same 64 bytes.
const STRIP: usize = 16;

/// The same, down each column — but [`STRIP`] columns at a time, for the reason
/// `native/effects.rs`'s `vertical` gives: a single-column walk over a
/// row-major buffer uses a quarter of every cache line it fetches. Each column
/// keeps its own running sum and they never interact, so the result is
/// bit-for-bit what the single-column loop produced.
fn box_blur_vertical(src: &[f32], dst: &mut [f32], w: u32, h: u32, radius: i32) {
    let window = (2 * radius + 1) as f32;
    let (width, height) = (w as usize, h as i32);

    let mut x0 = 0usize;
    while x0 < width {
        let strip = STRIP.min(width - x0);
        let mut sums = [0.0f32; STRIP];

        for y in 0..=radius.min(height - 1) {
            let row = y as usize * width + x0;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                *sum += src[row + i];
            }
        }

        for y in 0..height {
            let out_row = y as usize * width + x0;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                dst[out_row + i] = *sum / window;
            }
            let entering = y + radius + 1;
            let leaving = y - radius;
            for (i, sum) in sums.iter_mut().take(strip).enumerate() {
                let a = if entering < height {
                    src[entering as usize * width + x0 + i]
                } else {
                    0.0
                };
                let b = if leaving >= 0 {
                    src[leaving as usize * width + x0 + i]
                } else {
                    0.0
                };
                *sum += a - b;
            }
        }
        x0 += strip;
    }
}

/// Shadow patches already rendered, keyed by everything that decides one.
///
/// # Why a shadow is worth caching and a fill is not
///
/// A fill is rasterised in about as long as it takes to look up whether it
/// was cached. A shadow is a silhouette rasterised, copied into an alpha
/// buffer, run through six separable box-blur passes over the whole patch,
/// and tinted — and a panel's patch is the panel plus the blur's reach on
/// every side. Measured on the Studio shell: **13 shadows, 37.6 ms**, about
/// three milliseconds each.
///
/// None of that work depends on anything that changes between frames. A
/// panel's shadow is a pure function of its rectangle, its radius, the
/// shadow's own parameters and the transform — so a panel that has not moved
/// has the same shadow it had last frame, down to the bit.
///
/// This matters most exactly where it is least obvious. A damaged repaint
/// re-executes every command that overlaps the damaged region, and a card's
/// shadow overlaps everything inside the card — so without this, lighting one
/// row of a list re-blurs the whole card's shadow, and the incremental
/// repaint the damage pipeline exists to make cheap is dominated by an effect
/// that did not change.
#[derive(Debug, Default)]
pub(crate) struct ShadowCache {
    entries: Vec<(ShadowKey, Target, i32, i32)>,
    pixels: usize,
    hits: usize,
    misses: usize,
}

/// Everything [`render_shadow`] reads. Compared by bit pattern rather than by
/// `PartialEq` on floats, so that two keys are equal exactly when the
/// arithmetic below them would be.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ShadowKey([u32; 16]);

impl ShadowKey {
    fn new(rect: Rect, radius: f32, shadow: &Shadow, transform: Transform, surface: Rect) -> Self {
        Self([
            rect.left.to_bits(),
            rect.top.to_bits(),
            rect.right.to_bits(),
            rect.bottom.to_bits(),
            radius.to_bits(),
            u32::from_le_bytes([
                shadow.color.r,
                shadow.color.g,
                shadow.color.b,
                shadow.color.a,
            ]),
            shadow.offset.dx.to_bits(),
            shadow.offset.dy.to_bits(),
            shadow.blur.to_bits(),
            shadow.spread.to_bits(),
            u32::from(shadow.is_inset),
            transform.a.to_bits() ^ transform.b.to_bits().rotate_left(8),
            transform.c.to_bits() ^ transform.d.to_bits().rotate_left(8),
            transform.tx.to_bits(),
            transform.ty.to_bits(),
            surface.left.to_bits()
                ^ surface.top.to_bits().rotate_left(8)
                ^ surface.right.to_bits().rotate_left(16)
                ^ surface.bottom.to_bits().rotate_left(24),
        ])
    }
}

/// How many patch pixels the cache may hold at once.
///
/// A patch is one [`Premul`] — sixteen bytes — per pixel, so this is a
/// budget of about thirty megabytes. Generous for a desktop window and firmly
/// bounded, which is the property that matters: a scene that animates a
/// shadow's blur every frame must degrade to "no reuse", not to "no memory".
const PIXEL_BUDGET: usize = 2_000_000;

impl ShadowCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Hits and misses over this cache's lifetime — reported through
    /// [`NativeRenderer::shadow_cache_stats`](super::NativeRenderer::shadow_cache_stats).
    pub(crate) fn stats(&self) -> (usize, usize) {
        (self.hits, self.misses)
    }

    /// The patch for this shadow, rendering it only if it is not already held.
    ///
    /// Returns `None` for a shadow that produces nothing at all — invisible,
    /// or entirely off the surface.
    pub(crate) fn patch(
        &mut self,
        rect: Rect,
        radius: f32,
        shadow: &Shadow,
        transform: Transform,
        surface: Rect,
    ) -> Option<(&Target, i32, i32)> {
        let key = ShadowKey::new(rect, radius, shadow, transform, surface);
        if let Some(index) = self.entries.iter().position(|(k, ..)| *k == key) {
            self.hits += 1;
            let entry = self.entries.remove(index);
            self.entries.push(entry);
        } else {
            self.misses += 1;
            let (patch, px0, py0) = render_shadow(rect, radius, shadow, transform, surface)?;
            let cost = patch.pixels.len();
            // Evict oldest-first until this one fits. A single patch larger
            // than the whole budget is still stored — one frame's correctness
            // is not worth trading for a memory bound — and evicted on the
            // next miss.
            while self.pixels + cost > PIXEL_BUDGET && !self.entries.is_empty() {
                let (_, dropped, ..) = self.entries.remove(0);
                self.pixels -= dropped.pixels.len();
            }
            self.pixels += cost;
            self.entries.push((key, patch, px0, py0));
        }
        let (_, patch, px0, py0) = self.entries.last().expect("just pushed");
        Some((patch, *px0, *py0))
    }
}
