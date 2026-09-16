//! Turn an [`IconData`] into the RGBA pixels a system tray wants.
//!
//! # Why there is a second rasteriser in this workspace
//!
//! Because the first one cannot be asked this question. `vieww-paint` draws
//! through vello on the GPU, and everything it produces goes to a surface or to
//! a texture that a device owns. A tray icon is needed **before there is a
//! window**, is 32 pixels across, and is handed to the shell as a plain buffer
//! — so going through the GPU would mean standing up an adapter, a device and a
//! render target to fill an area smaller than one glyph, on a code path that
//! runs when an application starts and never again.
//!
//! So this is a scanline fill, on the CPU, in about a hundred lines. It is not a
//! general renderer and must not grow into one: no strokes, no gradients, no
//! clipping, no blend modes. One filled path, one colour, nonzero winding.
//!
//! # Nonzero winding, not even-odd
//!
//! Because that is what the rest of the framework fills with, and an icon that
//! rendered differently in the tray than it does in a button would be a defect
//! nobody could explain. It also matters for real icons: the built-in set draws
//! holes — the middle of an `O`, the gap in a bookmark — as a subpath wound the
//! other way, and even-odd would agree by accident on those and disagree on any
//! icon with two overlapping same-wound shapes.

use vieww_foundation::{Color, IconData, Offset, PathVerb, Rect};

/// How finely a cubic is chopped up, in output pixels.
///
/// A quarter of a pixel. Finer than the supersampling below can resolve, which
/// is the point: flattening error should be invisible against the sampling
/// error rather than adding to it.
const FLATNESS: f32 = 0.25;

/// Samples per pixel per axis, so 16 per pixel.
///
/// Antialiasing an icon matters more than it sounds like it should: a tray icon
/// is the smallest thing an application draws, and the aliased version of a
/// diagonal at 32 pixels does not read as the same shape.
const SAMPLES: u32 = 4;

/// One edge of the flattened outline.
///
/// Horizontal edges are dropped on the way in — they cannot cross a scanline, and
/// keeping them means every scanline that grazes one has to special-case a
/// division by zero.
struct Edge {
    top: f32,
    bottom: f32,
    /// x at `top`, and how much x moves per unit y.
    x: f32,
    slope: f32,
    /// `1` when the edge runs down the page, `-1` when it runs up. The sign is
    /// the whole of nonzero winding.
    winding: i32,
}

/// Chop `path` into straight edges, in output pixel space.
fn edges(icon: &IconData, size: f32) -> Vec<Edge> {
    // `fitted` is what keeps an icon's margins: it scales by the declared
    // viewbox rather than by the ink's own bounds, so a set of icons stays
    // visually consistent instead of each one growing to fill the box. See
    // `IconData`'s own reasoning.
    let path = icon.fitted(Rect::new(0.0, 0.0, size, size));

    let mut edges = Vec::new();
    let mut start = Offset::ZERO;
    let mut cursor = Offset::ZERO;

    // Not `mut`: it captures nothing and takes the buffer as a parameter, which
    // is the only shape that lets it be called from inside the loop that is
    // already borrowing `edges`.
    let edge = |from: Offset, to: Offset, edges: &mut Vec<Edge>| {
        if (to.dy - from.dy).abs() < f32::EPSILON {
            return;
        }
        let (top, bottom, winding) = if from.dy < to.dy {
            (from, to, 1)
        } else {
            (to, from, -1)
        };
        edges.push(Edge {
            top: top.dy,
            bottom: bottom.dy,
            x: top.dx,
            slope: (bottom.dx - top.dx) / (bottom.dy - top.dy),
            winding,
        });
    };

    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(to) => {
                // An open subpath is closed implicitly, which is what filling
                // means: there is no such thing as a filled open shape.
                if cursor != start {
                    edge(cursor, start, &mut edges);
                }
                start = to;
                cursor = to;
            }
            PathVerb::LineTo(to) => {
                edge(cursor, to, &mut edges);
                cursor = to;
            }
            PathVerb::CubicTo(c1, c2, to) => {
                // Segment count from the control polygon's length, which
                // over-estimates the curve's — the cheap direction to be wrong
                // in, and it never under-samples a tight corner.
                let hull = distance(cursor, c1) + distance(c1, c2) + distance(c2, to);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a clamped segment count is a small positive integer"
                )]
                let steps = ((hull / FLATNESS).ceil() as u32).clamp(1, 64);
                let mut previous = cursor;
                for step in 1..=steps {
                    #[expect(clippy::cast_precision_loss, reason = "steps is at most 64")]
                    let t = step as f32 / steps as f32;
                    let point = cubic(cursor, c1, c2, to, t);
                    edge(previous, point, &mut edges);
                    previous = point;
                }
                cursor = to;
            }
            PathVerb::Close => {
                edge(cursor, start, &mut edges);
                cursor = start;
            }
        }
    }
    if cursor != start {
        edge(cursor, start, &mut edges);
    }
    edges
}

fn distance(a: Offset, b: Offset) -> f32 {
    ((b.dx - a.dx).powi(2) + (b.dy - a.dy).powi(2)).sqrt()
}

/// A point on a cubic at `t`, by de Casteljau.
fn cubic(p0: Offset, p1: Offset, p2: Offset, p3: Offset, t: f32) -> Offset {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Offset::new(
        a * p0.dx + b * p1.dx + c * p2.dx + d * p3.dx,
        a * p0.dy + b * p1.dy + c * p2.dy + d * p3.dy,
    )
}

/// Fill `icon` into a `size`x`size` RGBA buffer, in `color`.
///
/// Straight (non-premultiplied) RGBA, four bytes per pixel, row-major from the
/// top — which is what every tray API in use here takes.
///
/// The colour is the caller's because `IconData` carries none: an icon is a
/// shape, and what it should look like in a tray is a per-platform convention
/// rather than a property of the shape. See `desktop::tray_foreground`.
pub(crate) fn rasterise(icon: &IconData, size: u32, color: Color) -> Vec<u8> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "an icon is tens of pixels across"
    )]
    let extent = size as f32;
    let edges = edges(icon, extent);
    let mut pixels = vec![0_u8; (size as usize) * (size as usize) * 4];

    // Reused across scanlines rather than allocated per row: 32 rows times 4
    // samples is 128 allocations of a handful of floats otherwise, which is
    // silly for a buffer whose size never changes.
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    // Coverage in samples, one per pixel of the current row.
    let mut coverage = vec![0_u32; size as usize];

    for y in 0..size {
        coverage.fill(0);
        for sub in 0..SAMPLES {
            #[expect(clippy::cast_precision_loss, reason = "both are small integers")]
            let sample_y = y as f32 + (sub as f32 + 0.5) / SAMPLES as f32;

            crossings.clear();
            for edge in &edges {
                // Half-open in y, which is what stops a vertex shared by two
                // edges being counted twice and punching a hole at every corner.
                if sample_y >= edge.top && sample_y < edge.bottom {
                    crossings.push((edge.x + (sample_y - edge.top) * edge.slope, edge.winding));
                }
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding == 0 {
                    continue;
                }
                // The span [pair[0].0, pair[1].0) is inside. Count whole samples
                // rather than measuring the partial ones: with 4 sub-samples per
                // axis the x error is already under a quarter pixel, and exact
                // area coverage here would be a different algorithm.
                let (from, to) = (pair[0].0, pair[1].0);
                for x in 0..size {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "an icon is tens of pixels across"
                    )]
                    let pixel = x as f32;
                    for sample in 0..SAMPLES {
                        #[expect(clippy::cast_precision_loss, reason = "both are small integers")]
                        let sample_x = pixel + (sample as f32 + 0.5) / SAMPLES as f32;
                        if sample_x >= from && sample_x < to {
                            coverage[x as usize] += 1;
                        }
                    }
                }
            }
        }

        let total = SAMPLES * SAMPLES;
        for x in 0..size {
            let hits = coverage[x as usize];
            if hits == 0 {
                continue;
            }
            let alpha = (hits * u32::from(color.a)) / total;
            let offset = ((y as usize) * (size as usize) + x as usize) * 4;
            pixels[offset] = color.r;
            pixels[offset + 1] = color.g;
            pixels[offset + 2] = color.b;
            // `min` rather than an `#[expect]` on the cast. The arithmetic
            // already cannot overflow a `u8` — `hits <= total`, so the product
            // over `total` is at most `color.a` — and clamping says so in a way
            // clippy can check, which is why there is no attribute here: with
            // the clamp, `cast_possible_truncation` does not fire at all, and an
            // expectation that is never fulfilled is itself an error under
            // `-D warnings`.
            let alpha = u8::try_from(alpha.min(255)).unwrap_or(u8::MAX);
            pixels[offset + 3] = alpha;
        }
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Path;

    /// Alpha of the pixel at `(x, y)`.
    fn alpha(pixels: &[u8], size: u32, x: u32, y: u32) -> u8 {
        pixels[((y as usize) * (size as usize) + x as usize) * 4 + 3]
    }

    /// Append a square subpath, wound in the direction asked for.
    ///
    /// Appends rather than returns, because the interesting tests need two
    /// subpaths in **one** path and `Path` has no way to concatenate.
    fn add_square(path: &mut Path, from: f32, to: f32, clockwise: bool) {
        path.move_to(Offset::new(from, from));
        if clockwise {
            path.line_to(Offset::new(to, from));
            path.line_to(Offset::new(to, to));
            path.line_to(Offset::new(from, to));
        } else {
            path.line_to(Offset::new(from, to));
            path.line_to(Offset::new(to, to));
            path.line_to(Offset::new(to, from));
        }
        path.close();
    }

    fn square(from: f32, to: f32, clockwise: bool) -> Path {
        let mut path = Path::new();
        add_square(&mut path, from, to, clockwise);
        path
    }

    #[test]
    fn a_full_square_covers_every_pixel() {
        let icon = IconData::square24(square(0.0, 24.0, true));
        let pixels = rasterise(&icon, 16, Color::WHITE);

        assert_eq!(alpha(&pixels, 16, 8, 8), 255, "the middle is solid");
        assert_eq!(alpha(&pixels, 16, 0, 0), 255, "and so is the corner");
        assert_eq!(pixels[0], 255, "in the colour asked for");
    }

    #[test]
    fn an_empty_path_draws_nothing() {
        // Not a crash and not a full square — the two failure modes a scanline
        // fill has when handed no edges at all.
        let icon = IconData::square24(Path::new());
        let pixels = rasterise(&icon, 8, Color::WHITE);
        assert!(
            pixels.iter().skip(3).step_by(4).all(|alpha| *alpha == 0),
            "every pixel is transparent"
        );
    }

    #[test]
    fn an_opposite_wound_subpath_punches_a_hole() {
        // The whole reason this fills nonzero rather than even-odd. The built-in
        // icon set draws holes this way, and a fill that ignored winding would
        // render a bookmark as a solid blob.
        let mut path = Path::new();
        add_square(&mut path, 0.0, 24.0, true);
        add_square(&mut path, 8.0, 16.0, false);
        let icon = IconData::square24(path);
        let pixels = rasterise(&icon, 24, Color::WHITE);

        assert_eq!(alpha(&pixels, 24, 2, 12), 255, "outside the hole is filled");
        assert_eq!(alpha(&pixels, 24, 12, 12), 0, "and the middle is not");
    }

    #[test]
    fn two_same_wound_shapes_do_not_cancel() {
        // The case even-odd gets wrong and nonzero gets right: overlapping
        // shapes wound the same way are one filled region, not a ring.
        let mut path = Path::new();
        add_square(&mut path, 0.0, 16.0, true);
        add_square(&mut path, 8.0, 24.0, true);
        let icon = IconData::square24(path);
        let pixels = rasterise(&icon, 24, Color::WHITE);

        assert_eq!(
            alpha(&pixels, 24, 12, 12),
            255,
            "the overlap stays filled — even-odd would clear it"
        );
    }

    #[test]
    fn the_viewbox_margin_survives_rasterising() {
        // An icon's meaning includes its margins, which is why `IconData` has a
        // viewbox at all. A shape occupying the middle half of its box must not
        // grow to fill the output.
        let icon = IconData::square24(square(6.0, 18.0, true));
        let pixels = rasterise(&icon, 24, Color::WHITE);

        assert_eq!(
            alpha(&pixels, 24, 12, 12),
            255,
            "the shape is in the middle"
        );
        assert_eq!(alpha(&pixels, 24, 1, 1), 0, "and the margin is still empty");
    }

    #[test]
    fn a_curve_is_flattened_rather_than_ignored() {
        // A path made only of cubics has no `LineTo` at all, so a flattener that
        // silently dropped them would produce an empty icon — which the
        // "empty path" test above would not catch.
        let mut path = Path::new();
        path.move_to(Offset::new(2.0, 12.0));
        path.cubic_to(
            Offset::new(2.0, 2.0),
            Offset::new(22.0, 2.0),
            Offset::new(22.0, 12.0),
        );
        path.cubic_to(
            Offset::new(22.0, 22.0),
            Offset::new(2.0, 22.0),
            Offset::new(2.0, 12.0),
        );
        path.close();
        let icon = IconData::square24(path);
        let pixels = rasterise(&icon, 24, Color::WHITE);

        assert_eq!(alpha(&pixels, 24, 12, 12), 255, "the disc is filled");
        assert_eq!(
            alpha(&pixels, 24, 0, 0),
            0,
            "and its corners are round, so the corner of the box is empty"
        );
    }

    #[test]
    fn the_alpha_of_a_translucent_colour_is_honoured() {
        let icon = IconData::square24(square(0.0, 24.0, true));
        let pixels = rasterise(&icon, 8, Color::rgba(255, 255, 255, 128));
        assert_eq!(
            alpha(&pixels, 8, 4, 4),
            128,
            "a half-transparent fill covers fully and shows through"
        );
    }
}
