//! Stroke expansion: turning a stroked path into filled outline geometry.
//!
//! Spec §5.1: "converts stroked paths to filled outlines honoring caps
//! (butt, round, square), joins (miter with limit, round, bevel), and dashes
//! ... the semantics are pinned by existing tests in the CPU backend, which
//! serve as the executable spec." This produces a set of small convex
//! polygons — one quad per segment, a join shape per vertex, a cap shape per
//! open end — handed to [`super::fill::rasterize`] together. They are
//! allowed to overlap: nonzero winding treats an overlap between
//! same-direction polygons as still "inside" (see
//! `fill::nonzero_winding_fills_overlap_of_same_direction_holes`), which is
//! what makes stamping independent join/cap shapes a sound way to build a
//! stroke outline rather than a hazard.

use vieww_foundation::{Dash, StrokeCap, StrokeJoin, StrokeStyle, Transform};

use super::flatten::{flatten_path, Polyline};

const ROUND_SEGMENTS: usize = 16;

/// Expand `path` (already stroked with `style`/`width`) into fill polygons,
/// in **device** space.
///
/// Flattening happens in local space first and the stroke offset is applied
/// there too, with the affine transform applied to the finished outline —
/// not the other way around. That is what keeps a stroke's width uniform
/// under a non-uniform (anisotropic) scale instead of the offset itself
/// being squashed.
pub(crate) fn stroke_to_polygons(
    path: &vieww_foundation::Path,
    width: f32,
    style: &StrokeStyle,
    transform: Transform,
) -> Vec<Polyline> {
    let half_width = (width.max(0.01)) * 0.5;
    let local = flatten_path(path, Transform::IDENTITY);
    let mut polygons = Vec::new();

    for line in &local {
        let segments = dashed_segments(&line.points, line.closed, &style.dash);
        for (points, closed) in segments {
            stroke_polyline(&points, closed, half_width, style, &mut polygons);
        }
    }

    // Lift every polygon from local space into device space as the very last
    // step.
    for polygon in &mut polygons {
        for point in &mut polygon.points {
            let p = transform.apply(vieww_foundation::Offset::new(point.0, point.1));
            *point = (p.dx, p.dy);
        }
    }
    polygons
}

/// Split a polyline into dash-on sub-polylines by arc length. `None`/solid
/// dash returns the line unchanged.
pub(crate) fn dashed_segments(
    points: &[(f32, f32)],
    closed: bool,
    dash: &Option<Dash>,
) -> Vec<(Vec<(f32, f32)>, bool)> {
    let Some(dash) = dash else {
        return vec![(points.to_vec(), closed)];
    };
    if dash.is_solid() || dash.pattern.is_empty() {
        return vec![(points.to_vec(), closed)];
    }

    let pattern = &dash.pattern;
    let period: f32 = pattern.iter().sum();
    if period <= 0.0 {
        return vec![(points.to_vec(), closed)];
    }

    let mut out = Vec::new();
    let mut current: Vec<(f32, f32)> = Vec::new();
    // Walk the pattern cyclically starting from `offset`.
    let mut pos_in_period = dash.offset.rem_euclid(period);
    let mut idx = 0usize;
    let mut remaining = pattern[0];
    while pos_in_period > 0.0 {
        if pos_in_period < remaining {
            remaining -= pos_in_period;
            break;
        }
        pos_in_period -= remaining;
        idx = (idx + 1) % pattern.len();
        remaining = pattern[idx];
    }
    let mut on = idx % 2 == 0;

    let all_points: Vec<(f32, f32)> = if closed && !points.is_empty() {
        let mut p = points.to_vec();
        p.push(points[0]);
        p
    } else {
        points.to_vec()
    };

    if on {
        if let Some(&first) = all_points.first() {
            current.push(first);
        }
    }

    for window in all_points.windows(2) {
        let (mut ax, mut ay) = window[0];
        let (bx, by) = window[1];
        let mut seg_len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        while seg_len > 0.0 {
            let step = remaining.min(seg_len);
            let t = step / seg_len.max(1e-6);
            let nx = ax + (bx - ax) * t;
            let ny = ay + (by - ay) * t;
            if on {
                current.push((nx, ny));
            }
            remaining -= step;
            seg_len -= step;
            ax = nx;
            ay = ny;
            if remaining <= 1e-6 {
                if on && current.len() >= 2 {
                    out.push((std::mem::take(&mut current), false));
                } else {
                    current.clear();
                }
                idx = (idx + 1) % pattern.len();
                remaining = pattern[idx].max(1e-6);
                on = !on;
                if on {
                    current.push((ax, ay));
                }
            }
        }
    }
    if on && current.len() >= 2 {
        out.push((current, false));
    }
    out
}

fn stroke_polyline(
    points: &[(f32, f32)],
    closed: bool,
    hw: f32,
    style: &StrokeStyle,
    out: &mut Vec<Polyline>,
) {
    if points.len() < 2 {
        return;
    }
    let n = points.len();
    let segment_count = if closed { n } else { n - 1 };

    let seg = |i: usize| -> ((f32, f32), (f32, f32)) {
        let a = points[i % n];
        let b = points[(i + 1) % n];
        (a, b)
    };

    let normal = |a: (f32, f32), b: (f32, f32)| -> (f32, f32) {
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        (-dy / len * hw, dx / len * hw)
    };

    // One quad per segment.
    for i in 0..segment_count {
        let (a, b) = seg(i);
        let (nx, ny) = normal(a, b);
        out.push(poly(&[
            (a.0 + nx, a.1 + ny),
            (b.0 + nx, b.1 + ny),
            (b.0 - nx, b.1 - ny),
            (a.0 - nx, a.1 - ny),
        ]));
    }

    // Joins at every interior vertex (and, for a closed path, the wrap join).
    let join_range: Vec<usize> = if closed {
        (0..n).collect()
    } else {
        (1..n - 1).collect()
    };
    for &i in &join_range {
        let (a_prev, _) = seg((i + n - 1) % n);
        let vertex = points[i];
        let (b_next_a, b_next_b) = seg(i % n);
        let n_in = normal(a_prev, vertex);
        let n_out = normal(b_next_a, b_next_b);
        emit_join(
            vertex,
            n_in,
            n_out,
            hw,
            style.join,
            style.effective_miter_limit(),
            out,
        );
    }

    // Caps at the two open ends.
    if !closed {
        let (a0, a1) = seg(0);
        emit_cap(a0, a1, hw, style.cap, out, true);
        let (b0, b1) = seg(segment_count - 1);
        emit_cap(b1, b0, hw, style.cap, out, false);
    }
}

fn emit_join(
    vertex: (f32, f32),
    n_in: (f32, f32),
    n_out: (f32, f32),
    hw: f32,
    join: StrokeJoin,
    miter_limit: f32,
    out: &mut Vec<Polyline>,
) {
    match join {
        StrokeJoin::Round => out.push(disc(vertex, hw)),
        StrokeJoin::Bevel => {
            out.push(poly(&[
                vertex,
                (vertex.0 + n_in.0, vertex.1 + n_in.1),
                (vertex.0 + n_out.0, vertex.1 + n_out.1),
            ]));
            out.push(poly(&[
                vertex,
                (vertex.0 - n_in.0, vertex.1 - n_in.1),
                (vertex.0 - n_out.0, vertex.1 - n_out.1),
            ]));
        }
        StrokeJoin::Miter => {
            for sign in [1.0f32, -1.0] {
                let p1 = (vertex.0 + sign * n_in.0, vertex.1 + sign * n_in.1);
                let p2 = (vertex.0 + sign * n_out.0, vertex.1 + sign * n_out.1);
                match line_intersection(p1, (n_in.1, -n_in.0), p2, (n_out.1, -n_out.0)) {
                    Some(miter) => {
                        let miter_len =
                            ((miter.0 - vertex.0).powi(2) + (miter.1 - vertex.1).powi(2)).sqrt();
                        if hw > 1e-6 && miter_len / hw <= miter_limit {
                            out.push(poly(&[vertex, p1, miter, p2]));
                        } else {
                            out.push(poly(&[vertex, p1, p2]));
                        }
                    }
                    None => out.push(poly(&[vertex, p1, p2])),
                }
            }
        }
    }
}

fn emit_cap(
    at: (f32, f32),
    towards: (f32, f32),
    hw: f32,
    cap: StrokeCap,
    out: &mut Vec<Polyline>,
    _is_start: bool,
) {
    match cap {
        StrokeCap::Butt => {}
        StrokeCap::Round => out.push(disc(at, hw)),
        StrokeCap::Square => {
            let dx = at.0 - towards.0;
            let dy = at.1 - towards.1;
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let (ex, ey) = (dx / len * hw, dy / len * hw);
            let (nx, ny) = (-ey, ex);
            out.push(poly(&[
                (at.0 + nx, at.1 + ny),
                (at.0 + nx + ex, at.1 + ny + ey),
                (at.0 - nx + ex, at.1 - ny + ey),
                (at.0 - nx, at.1 - ny),
            ]));
        }
    }
}

fn disc(center: (f32, f32), radius: f32) -> Polyline {
    let mut points = Vec::with_capacity(ROUND_SEGMENTS);
    for i in 0..ROUND_SEGMENTS {
        let theta = std::f32::consts::TAU * (i as f32) / (ROUND_SEGMENTS as f32);
        points.push((
            center.0 + radius * theta.cos(),
            center.1 + radius * theta.sin(),
        ));
    }
    Polyline {
        points,
        closed: true,
    }
}

fn poly(points: &[(f32, f32)]) -> Polyline {
    Polyline {
        points: points.to_vec(),
        closed: true,
    }
}

/// Intersection of line `p1 + t*d1` with line `p2 + s*d2`, `None` if parallel.
fn line_intersection(
    p1: (f32, f32),
    d1: (f32, f32),
    p2: (f32, f32),
    d2: (f32, f32),
) -> Option<(f32, f32)> {
    let denom = d1.0 * d2.1 - d1.1 * d2.0;
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = ((p2.0 - p1.0) * d2.1 - (p2.1 - p1.1) * d2.0) / denom;
    Some((p1.0 + d1.0 * t, p1.1 + d1.1 * t))
}
