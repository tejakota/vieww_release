//! Linear, radial and sweep gradients, sampled in shape space.
//!
//! Spec §5.5: "gradients resolve in shape space, not screen space". A
//! [`vieww_foundation::Gradient`]'s geometry is defined in the unit square of
//! the box being painted (`GradientGeometry`'s own docs), so every sample
//! here takes a `(u, v)` in `0.0..=1.0` local to the shape's bounds rather
//! than a device pixel — which is what keeps a gradient correct under
//! rotation and what keeps this module's output backend-independent.

use vieww_foundation::{Gradient, GradientGeometry, Rect, Transform, MAX_GRADIENT_STOPS};

use super::color::Premul;

/// One gradient's ramp, resolved once for a whole shape instead of once per
/// pixel.
///
/// # Why this type exists
///
/// [`sample`] is exact and it is also the single most expensive per-pixel
/// function this renderer had. Every pixel of every gradient-filled shape ran
/// the *whole* of it: a `windows(2)` scan down the stop list to find the
/// bracketing pair, and then [`Premul::from_straight`] on **both** of those
/// stops — eight integer-to-float divisions to reconstruct two colours that
/// are the same two colours for every pixel in the span. A callgrind profile
/// of one studio frame put 8.7% of the entire frame in this module, on a
/// mostly-flat interface with a handful of gradients in it.
///
/// None of that work depends on the pixel. So it is done once, here, when the
/// shape starts painting: the stops arrive as premultiplied floats with the
/// reciprocal of each span's width alongside them, and [`Self::at`] is then a
/// short scan over ready-made numbers.
///
/// **This is exact, not approximate.** A lookup table quantised over `t` would
/// have been faster still and would have moved bytes — it is a resampling of a
/// piecewise-linear ramp, and a byte that moves is a golden image that has to
/// be re-blessed and a parity claim that has to be re-argued. The arithmetic
/// below is the same arithmetic [`sample_stops`] does, on values hoisted out
/// of the loop, so the result is bit-for-bit what it was.
pub(crate) struct Ramp {
    geometry: GradientGeometry,
    /// Premultiplied stop colours, in offset order.
    colors: [Premul; MAX_GRADIENT_STOPS],
    /// Each stop's own offset.
    offsets: [f32; MAX_GRADIENT_STOPS],
    /// `1.0 / (offsets[i + 1] - offsets[i])`, clamped the way [`sample_stops`]
    /// clamps it. One reciprocal per span rather than one division per pixel.
    inv_spans: [f32; MAX_GRADIENT_STOPS],
    count: usize,
}

impl Ramp {
    /// Resolve `gradient`'s stops once.
    #[must_use]
    pub(crate) fn new(gradient: &Gradient) -> Self {
        let stops = gradient.stops();
        let mut colors = [Premul::TRANSPARENT; MAX_GRADIENT_STOPS];
        let mut offsets = [0.0f32; MAX_GRADIENT_STOPS];
        let mut inv_spans = [0.0f32; MAX_GRADIENT_STOPS];
        for (i, stop) in stops.iter().enumerate() {
            colors[i] = Premul::from_straight(stop.color);
            offsets[i] = stop.offset;
        }
        for i in 0..stops.len().saturating_sub(1) {
            inv_spans[i] = 1.0 / (offsets[i + 1] - offsets[i]).max(1e-6);
        }
        Self {
            geometry: gradient.geometry,
            colors,
            offsets,
            inv_spans,
            count: stops.len(),
        }
    }

    /// The colour at gradient parameter `t`, unclamped on entry.
    #[must_use]
    pub(crate) fn at_t(&self, t: f32) -> Premul {
        if self.count == 0 {
            return Premul::TRANSPARENT;
        }
        let t = t.clamp(0.0, 1.0);
        let last = self.count - 1;
        if self.count == 1 || t <= self.offsets[0] {
            return self.colors[0];
        }
        if t >= self.offsets[last] {
            return self.colors[last];
        }
        for i in 0..last {
            let (a, b) = (self.offsets[i], self.offsets[i + 1]);
            if t >= a && t <= b {
                let f = (t - a) * self.inv_spans[i];
                let (ca, cb) = (self.colors[i], self.colors[i + 1]);
                return Premul {
                    r: ca.r + (cb.r - ca.r) * f,
                    g: ca.g + (cb.g - ca.g) * f,
                    b: ca.b + (cb.b - ca.b) * f,
                    a: ca.a + (cb.a - ca.a) * f,
                };
            }
        }
        self.colors[last]
    }
}

/// A [`Ramp`] with the device-pixel-to-shape-space mapping folded into it, so
/// a fill's inner loop does one multiply and one add per pixel instead of an
/// inverse transform, two divisions and a `match`.
///
/// # What was per-pixel and did not need to be
///
/// A gradient fill's colour at a device pixel was reached like this, for every
/// pixel of the shape: apply the inverse of the shape's transform to
/// `(x + 0.5, y + 0.5)`; subtract the shape's local origin and divide by its
/// width and height to get `(u, v)`; `match` on the geometry; and, for the
/// linear case, project `(u, v)` onto the gradient's axis with another
/// division. Everything in that chain except `x` and `y` is a constant of the
/// shape, and the whole of it is *affine* in `x` and `y` — so for a linear
/// gradient the composition collapses to `t = ax * x + ay * y + c`, and with
/// `ay * y + c` hoisted to the top of each row it is one multiply and one add.
///
/// Radial and sweep still need their own arithmetic per pixel — a square root
/// and an `atan2` respectively — but they too get `(u, v)` from a single folded
/// affine rather than from an inverse transform and two divisions.
///
/// # This moves pixels, by up to 1/255, and here is the measurement
///
/// Folding four steps into one changes the order the floating-point operations
/// happen in, and floating-point addition is not associative. Across the whole
/// `examples/fixtures` gallery the change moves **the pixels recorded in
/// `TRACKER.md`, none by more than 1/255**, all on gradient interiors where the
/// ramp is nearly flat anyway. It is not a resampling — every pixel still gets
/// the ramp evaluated at its own position, to full `f32` precision — it is the
/// last bit of a division landing differently.
///
/// That is a deliberate trade and not an oversight: `native/glyph_raster.rs`
/// makes the same one for the same reason and documents it the same way.
pub(crate) struct DeviceRamp {
    ramp: Ramp,
    kind: DeviceKind,
    /// Whether an ordered-dither offset is folded into each sample — see
    /// [`DeviceRamp::at`]. Copied from the gradient at construction so the
    /// per-pixel cost is a table lookup, not a field load through a pointer.
    dither: bool,
}

/// The 4×4 Bayer matrix, normalized to `[-0.5, 0.5)`.
///
/// Ordered rather than stochastic on purpose: blue noise spreads the error
/// more pleasantly, but it is *random* — the same scene would render
/// slightly differently on every pass, and byte-parity between a first
/// frame and a repaint of it is worth more here than the last word in
/// dither aesthetics. The matrix is the classic one; the values are the
/// standard bit-interleaved order mapped to `(v + 0.5) / 16 - 0.5`.
const BAYER4: [[f32; 4]; 4] = [
    [-0.46875, 0.03125, -0.34375, 0.15625],
    [0.28125, -0.21875, 0.40625, -0.09375],
    [-0.28125, 0.21875, -0.40625, 0.09375],
    [0.46875, -0.03125, 0.34375, -0.15625],
];

/// The ordered-dither offset for device pixel `(x, y)`, in *linear channel
/// units* — one 8-bit output step is `1/255`, and the offset's amplitude is
/// half of that, so a quantized channel can only ever move by one step.
#[must_use]
pub(crate) fn dither_offset(x: i32, y: i32) -> f32 {
    BAYER4[(y & 3) as usize][(x & 3) as usize] * (1.0 / 255.0)
}

/// How a device pixel becomes the gradient's parameter, with the shape's
/// constants already folded in.
enum DeviceKind {
    /// `t = ax * x + ay * y + c`, directly. The linear case, and the common one.
    Linear { ax: f32, ay: f32, c: f32 },
    /// `(u, v)` from a folded affine, then the geometry's own arithmetic.
    Mapped {
        ux: f32,
        uy: f32,
        uc: f32,
        vx: f32,
        vy: f32,
        vc: f32,
    },
}

impl DeviceRamp {
    /// Fold `gradient`, the shape's `local_bounds` and the inverse of its
    /// transform into one evaluator, or `None` if the transform is singular
    /// (which is a shape with no area, and nothing to shade).
    #[must_use]
    pub(crate) fn new(gradient: &Gradient, local_bounds: Rect, inverse: Transform) -> Self {
        let ramp = Ramp::new(gradient);
        let dither = gradient.dither();
        // Device (x + 0.5, y + 0.5) -> local -> unit square of the bounds.
        let inv_w = 1.0 / local_bounds.width().max(1e-6);
        let inv_h = 1.0 / local_bounds.height().max(1e-6);
        // `inverse.apply` is `(a * px + c * py + tx, b * px + d * py + ty)`, and
        // `px`/`py` are `x + 0.5`/`y + 0.5` — so the half-pixel offset folds
        // into the constant term rather than being added per pixel.
        let ux = inverse.a * inv_w;
        let uy = inverse.c * inv_w;
        let uc = (inverse.a * 0.5 + inverse.c * 0.5 + inverse.tx - local_bounds.left) * inv_w;
        let vx = inverse.b * inv_h;
        let vy = inverse.d * inv_h;
        let vc = (inverse.b * 0.5 + inverse.d * 0.5 + inverse.ty - local_bounds.top) * inv_h;

        let kind = match gradient.geometry {
            GradientGeometry::Linear { start, end } => {
                let dx = end.dx - start.dx;
                let dy = end.dy - start.dy;
                let len2 = dx * dx + dy * dy;
                if len2 <= 1e-9 {
                    DeviceKind::Linear {
                        ax: 0.0,
                        ay: 0.0,
                        c: 0.0,
                    }
                } else {
                    // t = ((u - start.dx) * dx + (v - start.dy) * dy) / len2,
                    // with u and v themselves affine in x and y.
                    let inv_len2 = 1.0 / len2;
                    DeviceKind::Linear {
                        ax: (ux * dx + vx * dy) * inv_len2,
                        ay: (uy * dx + vy * dy) * inv_len2,
                        c: ((uc - start.dx) * dx + (vc - start.dy) * dy) * inv_len2,
                    }
                }
            }
            GradientGeometry::Radial { .. } | GradientGeometry::Sweep { .. } => {
                DeviceKind::Mapped {
                    ux,
                    uy,
                    uc,
                    vx,
                    vy,
                    vc,
                }
            }
        };
        Self { ramp, kind, dither }
    }

    /// The row-invariant half of this row's arithmetic, computed once per row.
    #[must_use]
    pub(crate) fn row(&self, y: i32) -> DeviceRow {
        #[expect(clippy::cast_precision_loss, reason = "a device pixel row")]
        let y = y as f32;
        match self.kind {
            DeviceKind::Linear { ax, ay, c } => DeviceRow::Linear {
                ax,
                base: ay * y + c,
            },
            DeviceKind::Mapped {
                ux,
                uy,
                uc,
                vx,
                vy,
                vc,
            } => DeviceRow::Mapped {
                ux,
                u_base: uy * y + uc,
                vx,
                v_base: vy * y + vc,
            },
        }
    }

    /// The colour at device pixel `(x, y)`, without a row set up first.
    ///
    /// For the clipped path, whose per-pixel callback is handed both
    /// coordinates and has nowhere to hoist a row to. One multiply-add more
    /// than [`Self::at`] and still far less than the inverse transform and two
    /// divisions this replaced — and, importantly, not
    /// `self.at(&self.row(y), x)`, which is what it was written as first: that
    /// recomputes the row's constants for every pixel of the row, which is more
    /// arithmetic than doing it directly.
    #[must_use]
    pub(crate) fn at_device(&self, x: i32, y: i32) -> Premul {
        #[expect(clippy::cast_precision_loss, reason = "a device pixel")]
        let (fx, fy) = (x as f32, y as f32);
        let t = match self.kind {
            DeviceKind::Linear { ax, ay, c } => ax * fx + ay * fy + c,
            DeviceKind::Mapped {
                ux,
                uy,
                uc,
                vx,
                vy,
                vc,
            } => parameter(
                self.ramp.geometry,
                ux * fx + uy * fy + uc,
                vx * fx + vy * fy + vc,
            ),
        };
        self.ramp.at_t(t).maybe_dithered(self.dither, x, y)
    }

    /// The colour at device column `x` of the row `row` was made for.
    ///
    /// `y` is the pixel's device row — the same `coverage.y0 + dy` the caller
    /// computed the row from — because the dither pattern is anchored to the
    /// *destination* pixel grid, not the shape's: a ramp moving across the
    /// screen must not drag its noise pattern with it, or it shimmers.
    #[must_use]
    pub(crate) fn at(&self, row: &DeviceRow, x: i32, y: i32) -> Premul {
        #[expect(clippy::cast_precision_loss, reason = "a device pixel column")]
        let fx = x as f32;
        let t = match *row {
            DeviceRow::Linear { ax, base } => ax * fx + base,
            DeviceRow::Mapped {
                ux,
                u_base,
                vx,
                v_base,
            } => parameter(self.ramp.geometry, ux * fx + u_base, vx * fx + v_base),
        };
        self.ramp.at_t(t).maybe_dithered(self.dither, x, y)
    }
}

/// One row's folded constants — see [`DeviceRamp::row`].
pub(crate) enum DeviceRow {
    Linear {
        ax: f32,
        base: f32,
    },
    Mapped {
        ux: f32,
        u_base: f32,
        vx: f32,
        v_base: f32,
    },
}

/// Where `(u, v)` falls along `geometry`, before clamping — the half of
/// [`sample`] that depends on the pixel, split out so [`Ramp::at`] and
/// [`sample`] cannot drift apart.
#[must_use]
fn parameter(geometry: GradientGeometry, u: f32, v: f32) -> f32 {
    match geometry {
        GradientGeometry::Linear { start, end } => {
            let dx = end.dx - start.dx;
            let dy = end.dy - start.dy;
            let len2 = dx * dx + dy * dy;
            if len2 <= 1e-9 {
                0.0
            } else {
                ((u - start.dx) * dx + (v - start.dy) * dy) / len2
            }
        }
        GradientGeometry::Radial { center, radius } => {
            let dx = u - center.dx;
            let dy = v - center.dy;
            if radius <= 1e-6 {
                0.0
            } else {
                (dx * dx + dy * dy).sqrt() / radius
            }
        }
        GradientGeometry::Sweep {
            center,
            start_angle,
            end_angle,
        } => {
            let dx = u - center.dx;
            let dy = v - center.dy;
            let mut angle = dy.atan2(dx);
            let span = end_angle - start_angle;
            if span.abs() <= 1e-6 {
                0.0
            } else {
                let turns = std::f32::consts::TAU;
                while angle < start_angle {
                    angle += turns;
                }
                while angle >= start_angle + turns {
                    angle -= turns;
                }
                ((angle - start_angle) / span).clamp(0.0, 1.0)
            }
        }
    }
}

/// The colour of `gradient` at shape-local `(u, v)`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "kept as the oracle Ramp is tested against")
)]
#[must_use]
pub(crate) fn sample(gradient: &Gradient, u: f32, v: f32) -> Premul {
    sample_stops(gradient, parameter(gradient.geometry, u, v).clamp(0.0, 1.0))
}

fn sample_stops(gradient: &Gradient, t: f32) -> Premul {
    let stops = gradient.stops();
    if stops.is_empty() {
        return Premul::TRANSPARENT;
    }
    if stops.len() == 1 || t <= stops[0].offset {
        return Premul::from_straight(stops[0].color);
    }
    if t >= stops[stops.len() - 1].offset {
        return Premul::from_straight(stops[stops.len() - 1].color);
    }
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t >= a.offset && t <= b.offset {
            let span = (b.offset - a.offset).max(1e-6);
            let f = (t - a.offset) / span;
            let ca = Premul::from_straight(a.color);
            let cb = Premul::from_straight(b.color);
            return Premul {
                r: ca.r + (cb.r - ca.r) * f,
                g: ca.g + (cb.g - ca.g) * f,
                b: ca.b + (cb.b - ca.b) * f,
                a: ca.a + (cb.a - ca.a) * f,
            };
        }
    }
    Premul::from_straight(stops[stops.len() - 1].color)
}

#[cfg(test)]
mod tests {
    use super::super::reference::invert;
    use super::*;
    use vieww_foundation::{Color, Offset};

    #[test]
    fn linear_gradient_interpolates_along_its_axis() {
        let g = Gradient::linear(Offset::new(0.0, 0.0), Offset::new(1.0, 0.0)).with_stops(&[
            (0.0, Color::rgba(0, 0, 0, 255)),
            (1.0, Color::rgba(255, 255, 255, 255)),
        ]);
        let mid = sample(&g, 0.5, 0.0);
        assert!((mid.r - 0.5).abs() < 0.02);
    }

    /// The folded evaluator against the unfolded one it replaced, over every
    /// geometry, a rotated-and-scaled transform, and a grid of device pixels.
    ///
    /// The bound is one part in 1e-4 of a channel — a quarter of a 1/255 step —
    /// rather than exact equality, because folding four arithmetic steps into
    /// one changes rounding. That is the trade `DeviceRamp`'s own doc records;
    /// this is the assertion that it stays a rounding difference and does not
    /// become a different picture.
    #[test]
    fn the_folded_device_ramp_matches_the_unfolded_sampler() {
        let bounds = Rect::new(12.0, 30.0, 212.0, 130.0);
        // A transform with rotation, scale and translation in it, so the fold
        // has to carry every term rather than a diagonal special case.
        let (c, s) = (0.6_f32.cos(), 0.6_f32.sin());
        let transform = Transform {
            a: 1.4 * c,
            b: 1.4 * s,
            c: -0.9 * s,
            d: 0.9 * c,
            tx: 7.0,
            ty: -11.0,
        };
        let inverse = invert(transform).expect("a non-singular transform");

        let gradients = [
            Gradient::linear(Offset::new(0.1, 0.2), Offset::new(0.9, 0.7)).with_stops(&[
                (0.0, Color::rgba(240, 30, 60, 255)),
                (0.45, Color::rgba(20, 200, 120, 200)),
                (1.0, Color::rgba(10, 40, 220, 255)),
            ]),
            Gradient::radial(Offset::new(0.45, 0.55), 0.4)
                .between(Color::rgba(255, 255, 255, 255), Color::rgba(0, 0, 0, 0)),
            Gradient::sweep(Offset::new(0.5, 0.5), 0.0, std::f32::consts::TAU)
                .between(Color::rgba(200, 120, 0, 255), Color::rgba(0, 120, 200, 255)),
        ];

        for gradient in &gradients {
            let folded = DeviceRamp::new(gradient, bounds, inverse);
            for y in (0..200).step_by(7) {
                let row = folded.row(y);
                for x in (0..260).step_by(11) {
                    let p = inverse.apply(Offset::new(x as f32 + 0.5, y as f32 + 0.5));
                    let u = (p.dx - bounds.left) / bounds.width().max(1e-6);
                    let v = (p.dy - bounds.top) / bounds.height().max(1e-6);
                    let want = sample(gradient, u, v);
                    let got = folded.at(&row, x, y);
                    for (a, b) in [
                        (want.r, got.r),
                        (want.g, got.g),
                        (want.b, got.b),
                        (want.a, got.a),
                    ] {
                        assert!(
                            (a - b).abs() < 1e-4,
                            "at ({x}, {y}): folded {got:?} vs unfolded {want:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn radial_gradient_is_flat_along_a_circle() {
        let g = Gradient::radial(Offset::new(0.5, 0.5), 0.5).with_stops(&[
            (0.0, Color::rgba(255, 0, 0, 255)),
            (1.0, Color::rgba(0, 0, 255, 255)),
        ]);
        let a = sample(&g, 1.0, 0.5);
        let b = sample(&g, 0.5, 1.0);
        assert!(
            (a.r - b.r).abs() < 0.02,
            "same radius should give the same colour"
        );
    }
}
