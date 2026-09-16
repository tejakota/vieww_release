//! A rounded rectangle as a fill polygon — shared by ordinary fills and by
//! the shadow caster (spec §5.2's "Rounded rect" shape class; the shadow
//! path in §8.2 rasterizes the same caster shape before blurring it).

use vieww_foundation::{Offset, Rect};

use super::geometry::flatten::Polyline;

const CORNER_SEGMENTS: usize = 6;

/// `rect` with its four corners rounded to `radius` (clamped to half the
/// shorter side, same convention as every other rounded-rect consumer in the
/// framework), as a single closed polygon in the same space as `rect`.
#[must_use]
pub(crate) fn rounded_rect_polygon(rect: Rect, radius: f32) -> Polyline {
    let r = radius
        .max(0.0)
        .min(rect.width() / 2.0)
        .min(rect.height() / 2.0);
    if r <= 0.01 {
        return Polyline {
            points: vec![
                (rect.left, rect.top),
                (rect.right, rect.top),
                (rect.right, rect.bottom),
                (rect.left, rect.bottom),
            ],
            closed: true,
        };
    }
    let mut points = Vec::with_capacity(CORNER_SEGMENTS * 4 + 4);
    let corners = [
        (
            rect.right - r,
            rect.top + r,
            -std::f32::consts::FRAC_PI_2,
            0.0,
        ), // top-right
        (
            rect.right - r,
            rect.bottom - r,
            0.0,
            std::f32::consts::FRAC_PI_2,
        ), // bottom-right
        (
            rect.left + r,
            rect.bottom - r,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
        ), // bottom-left
        (
            rect.left + r,
            rect.top + r,
            std::f32::consts::PI,
            std::f32::consts::PI * 1.5,
        ), // top-left
    ];
    for (cx, cy, a0, a1) in corners {
        for i in 0..=CORNER_SEGMENTS {
            let t = a0 + (a1 - a0) * (i as f32) / (CORNER_SEGMENTS as f32);
            points.push((cx + r * t.cos(), cy + r * t.sin()));
        }
    }
    Polyline {
        points,
        closed: true,
    }
}

/// [`rounded_rect_polygon`], transformed into device space.
#[must_use]
pub(crate) fn rounded_rect_polygon_transformed(
    rect: Rect,
    radius: f32,
    transform: vieww_foundation::Transform,
) -> Polyline {
    let mut polygon = rounded_rect_polygon(rect, radius);
    for p in &mut polygon.points {
        let d = transform.apply(Offset::new(p.0, p.1));
        *p = (d.dx, d.dy);
    }
    polygon
}
