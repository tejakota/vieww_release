//! Path verbs to polylines: adaptive cubic flattening.
//!
//! Spec §5.1: "cubics into line segments with adaptive tolerance of 0.25
//! device pixels at the cached scale, using the standard recursive flatness
//! test with a depth cap of 16". This module is that one function, used by
//! both fill rasterization ([`super::fill`]) and stroke expansion
//! ([`super::stroke`]) so the two never disagree about where a curve actually
//! sits.

use vieww_foundation::{Offset, Path, PathVerb, Transform};

/// Device-pixel tolerance for cubic flattening (spec §5.1).
pub(crate) const FLATTEN_TOLERANCE: f32 = 0.25;

/// Recursion depth cap so a degenerate cubic (near-zero chord, cusp) cannot
/// spin the subdivider forever (spec §5.1, and the fuzz target in §12.3).
const MAX_DEPTH: u32 = 16;

/// One closed or open polyline in device space, produced by [`flatten_path`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Polyline {
    pub(crate) points: Vec<(f32, f32)>,
    pub(crate) closed: bool,
}

/// Flatten `path` under `transform` into device-space polylines.
///
/// A single [`Path`] may describe several subpaths (multiple `MoveTo`s); each
/// becomes its own [`Polyline`]. An unclosed subpath is returned open — the
/// fill rasterizer implicitly closes it (that is what "nonzero winding"
/// means for an open contour), while the stroker treats open and closed
/// differently (caps vs. joins), which is why the distinction is preserved
/// here rather than resolved at flatten time.
pub(crate) fn flatten_path(path: &Path, transform: Transform) -> Vec<Polyline> {
    let mut polylines = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    let mut closed = false;
    let mut start = Offset::new(0.0, 0.0);
    let mut last = Offset::new(0.0, 0.0);

    macro_rules! flush {
        () => {
            if current.len() >= 2 {
                polylines.push(Polyline {
                    points: std::mem::take(&mut current),
                    closed,
                });
            } else {
                current.clear();
            }
            #[allow(unused_assignments)]
            {
                closed = false;
            }
        };
    }

    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(point) => {
                flush!();
                let p = transform.apply(point);
                current.push((p.dx, p.dy));
                start = point;
                last = point;
            }
            PathVerb::LineTo(point) => {
                let p = transform.apply(point);
                current.push((p.dx, p.dy));
                last = point;
            }
            PathVerb::CubicTo(c1, c2, end) => {
                flatten_cubic(last, c1, c2, end, transform, 0, &mut current);
                last = end;
            }
            PathVerb::Close => {
                if (last.dx - start.dx).abs() > f32::EPSILON
                    || (last.dy - start.dy).abs() > f32::EPSILON
                {
                    let p = transform.apply(start);
                    current.push((p.dx, p.dy));
                }
                closed = true;
                last = start;
            }
        }
    }
    flush!();
    polylines
}

/// Recursive de Casteljau flattening of one cubic segment, appending device
/// points to `out`. `p0` is implicit (already the last point pushed).
fn flatten_cubic(
    p0: Offset,
    p1: Offset,
    p2: Offset,
    p3: Offset,
    transform: Transform,
    depth: u32,
    out: &mut Vec<(f32, f32)>,
) {
    if depth >= MAX_DEPTH || is_flat_enough(p0, p1, p2, p3, transform) {
        let d = transform.apply(p3);
        out.push((d.dx, d.dy));
        return;
    }
    // De Casteljau subdivision at t = 0.5.
    let mid = |a: Offset, b: Offset| Offset::new((a.dx + b.dx) * 0.5, (a.dy + b.dy) * 0.5);
    let p01 = mid(p0, p1);
    let p12 = mid(p1, p2);
    let p23 = mid(p2, p3);
    let p012 = mid(p01, p12);
    let p123 = mid(p12, p23);
    let p0123 = mid(p012, p123);

    flatten_cubic(p0, p01, p012, p0123, transform, depth + 1, out);
    flatten_cubic(p0123, p123, p23, p3, transform, depth + 1, out);
}

/// The recursive flatness test: how far the control points sit from the
/// chord `p0`-`p3`, measured in **device** pixels (control points are mapped
/// through `transform` first, so a zoomed-in curve gets more segments — spec
/// §5.1's "a zoomed canvas re-tessellates deliberately rather than visibly
/// aliasing", and why tolerance is a cache-key component in §5.3).
fn is_flat_enough(p0: Offset, p1: Offset, p2: Offset, p3: Offset, transform: Transform) -> bool {
    let d0 = transform.apply(p0);
    let d1 = transform.apply(p1);
    let d2 = transform.apply(p2);
    let d3 = transform.apply(p3);

    let ux = 3.0 * d1.dx - 2.0 * d0.dx - d3.dx;
    let uy = 3.0 * d1.dy - 2.0 * d0.dy - d3.dy;
    let vx = 3.0 * d2.dx - 2.0 * d3.dx - d0.dx;
    let vy = 3.0 * d2.dy - 2.0 * d3.dy - d0.dy;

    let ux = ux * ux;
    let uy = uy * uy;
    let vx = vx * vx;
    let vy = vy * vy;

    let m = if ux > vx { ux } else { vx } + if uy > vy { uy } else { vy };
    // Standard cubic flatness bound (Sederberg): deviation <= sqrt(m) / (16/3).
    m <= 16.0 * FLATTEN_TOLERANCE * FLATTEN_TOLERANCE / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_line_flattens_to_two_points() {
        let mut path = Path::new();
        path.move_to(Offset::new(0.0, 0.0));
        path.line_to(Offset::new(10.0, 0.0));
        let polylines = flatten_path(&path, Transform::IDENTITY);
        assert_eq!(polylines.len(), 1);
        assert_eq!(polylines[0].points, vec![(0.0, 0.0), (10.0, 0.0)]);
    }

    #[test]
    fn a_cubic_produces_more_than_two_points() {
        let mut path = Path::new();
        path.move_to(Offset::new(0.0, 0.0));
        path.cubic_to(
            Offset::new(0.0, 100.0),
            Offset::new(100.0, 100.0),
            Offset::new(100.0, 0.0),
        );
        let polylines = flatten_path(&path, Transform::IDENTITY);
        assert_eq!(polylines.len(), 1);
        assert!(polylines[0].points.len() > 4, "curve was not subdivided");
    }

    #[test]
    fn recursion_terminates_on_a_degenerate_cubic() {
        // All four control points coincident: a cusp that never satisfies the
        // flatness test through floating-point noise alone. The depth cap is
        // what has to save this, not luck.
        let mut path = Path::new();
        path.move_to(Offset::new(5.0, 5.0));
        path.cubic_to(
            Offset::new(5.0, 5.0),
            Offset::new(5.0, 5.0),
            Offset::new(5.0, 5.000_001),
        );
        let polylines = flatten_path(&path, Transform::IDENTITY);
        assert_eq!(polylines.len(), 1);
        assert!(polylines[0].points.len() < 100_000);
    }
}
