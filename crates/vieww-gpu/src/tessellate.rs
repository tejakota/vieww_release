//! CPU-side geometry preparation for GPU execution: turning a
//! [`vieww_foundation::Path`] into a triangle mesh a real GPU pipeline can
//! actually draw.
//!
//! `docs/RENDERER-V2-NOTES.md` names "GPU tessellation" and "GPU
//! batching/instancing" as absent. This module is the tessellation half —
//! genuinely CPU-computable geometry math, not something that needs a GPU
//! present to exist or be tested, which is exactly why it lives here rather
//! than behind a backend. [`crate::batch`] is the batching/instancing half.
//!
//! # Why `lyon_tessellation` rather than a hand-rolled triangulator
//!
//! Robust polygon triangulation (self-intersections, holes, non-convex
//! shapes, the exact winding-rule semantics `vieww_paint`'s scanline
//! rasterizer already implements for the CPU path) is a well-studied,
//! easy-to-get-subtly-wrong problem. This workspace's own dependency
//! choices already draw that line elsewhere — `cosmic-text` and
//! `ttf-parser` for text rather than a hand-rolled shaper — and the same
//! reasoning applies here: `lyon_tessellation` is exactly this crate's
//! target's own reference tessellator (used by Firefox's WebRender), so
//! reusing it gets correctness this delivery would not improve on by
//! rewriting it, and spends the effort instead on the part that is
//! actually new — the [`vieww_foundation::Path`] → GPU-mesh bridge, and how
//! the result feeds [`crate::batch`] and a real backend's vertex buffers.

use lyon_tessellation::{
    self as lyon,
    geometry_builder::{BuffersBuilder, Positions},
    math::Point as LyonPoint,
    FillOptions, FillRule, FillTessellator, StrokeOptions, StrokeTessellator, VertexBuffers,
};
use vieww_foundation::{Offset, Path, PathVerb};
use vieww_paint::Stroke;
use vieww_paint::{StrokeCap, StrokeJoin};

/// A GPU-ready triangle mesh: positions plus a triangle-list index buffer.
///
/// 32-bit indices. These were 16-bit, and a path past 65,536 vertices made
/// lyon return an error that this module turned into an **empty mesh** — a
/// shape silently missing from the frame, which is exactly the failure the
/// planner's coverage contract exists to rule out. The scene's index buffer is
/// 32-bit anyway, so the narrower type saved nothing downstream.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

fn to_lyon_path(path: &Path) -> lyon::path::Path {
    let mut builder = lyon::path::Path::builder();
    let mut open = false;
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(p) => {
                if open {
                    builder.end(false);
                }
                builder.begin(pt(p));
                open = true;
            }
            PathVerb::LineTo(p) => {
                builder.line_to(pt(p));
            }
            PathVerb::CubicTo(c1, c2, end) => {
                builder.cubic_bezier_to(pt(c1), pt(c2), pt(end));
            }
            PathVerb::Close => {
                builder.close();
                open = false;
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

fn pt(o: Offset) -> LyonPoint {
    LyonPoint::new(o.dx, o.dy)
}

/// Tessellate `path`'s fill into a triangle mesh, using the nonzero winding
/// rule — the same rule `vieww_paint`'s scanline rasterizer uses, so a
/// future GPU backend and today's CPU backend agree on which pixels are
/// "inside" a self-intersecting path.
///
/// Returns an empty mesh (never panics or errors) for a path with fewer
/// than two verbs, or one lyon's tessellator otherwise rejects (e.g. every
/// point coincident) — an empty mesh is a pass that draws nothing, which is
/// the correct degenerate answer for a degenerate path.
#[must_use]
pub fn tessellate_fill(path: &Path, tolerance: f32) -> Mesh {
    let lyon_path = to_lyon_path(path);
    let mut buffers: VertexBuffers<LyonPoint, u32> = VertexBuffers::new();
    let mut vertex_builder = BuffersBuilder::new(&mut buffers, Positions);
    // **Non-zero, not lyon's default even-odd.** The display list resolves
    // every fill to non-zero winding (`vieww-paint`'s `geometry/fill.rs`), and
    // an icon built from overlapping subpaths is filled under even-odd with
    // holes wherever two subpaths cross — the fixture census's `10-icon-grid`
    // showed them as two-pixel slivers of background inside solid glyphs.
    let options = FillOptions::tolerance(tolerance.max(0.001)).with_fill_rule(FillRule::NonZero);
    let mut tessellator = FillTessellator::new();

    if tessellator
        .tessellate_path(&lyon_path, &options, &mut vertex_builder)
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        positions: buffers.vertices.iter().map(|v| [v.x, v.y]).collect(),
        indices: buffers.indices,
    }
}

fn lyon_cap(cap: StrokeCap) -> lyon::LineCap {
    match cap {
        StrokeCap::Butt => lyon::LineCap::Butt,
        StrokeCap::Round => lyon::LineCap::Round,
        StrokeCap::Square => lyon::LineCap::Square,
    }
}

/// The CPU stroker's miter limit, in lyon's units.
///
/// `vieww-paint`'s stroker (and SVG) bevel a join when
/// `miter length / half width > limit`. lyon 1.0 bevels when the squared
/// length of its join normal exceeds `4 * limit^2` — the same test with the
/// limit doubled. Passed through unchanged, every sharp join on the GPU kept
/// its miter up to twice as far as the CPU renderer allows (the fixture
/// census's `21-dashboard` sparkline peak: a spike five pixels taller). lyon
/// requires a limit of at least 1, so a CPU limit below 2 clamps there and a
/// join between the two thresholds still mitres on the GPU.
fn lyon_miter_limit(limit: f32) -> f32 {
    (limit * 0.5).max(1.0)
}

fn lyon_join(join: StrokeJoin) -> lyon::LineJoin {
    match join {
        StrokeJoin::Miter => lyon::LineJoin::Miter,
        StrokeJoin::Round => lyon::LineJoin::Round,
        StrokeJoin::Bevel => lyon::LineJoin::Bevel,
    }
}

/// Tessellate `path`'s outline into a triangle mesh for `stroke`.
///
/// Dashing is not applied here — `vieww-foundation::Dash` describes *which
/// sub-segments* of a path get stroked at all, which belongs upstream of
/// tessellation (splitting the input path), not inside it. A caller with a
/// dashed stroke pre-splits the path into its dash-on segments and
/// tessellates each; that splitting is `vieww_foundation::path`'s existing
/// dash logic (already exercised by the CPU renderer), not new geometry
/// math this module should duplicate.
#[must_use]
pub fn tessellate_stroke(path: &Path, stroke: &Stroke, tolerance: f32) -> Mesh {
    let lyon_path = to_lyon_path(path);
    let mut buffers: VertexBuffers<LyonPoint, u32> = VertexBuffers::new();
    let mut vertex_builder = BuffersBuilder::new(&mut buffers, Positions);
    let options = StrokeOptions::tolerance(tolerance.max(0.001))
        .with_line_width(stroke.width)
        .with_start_cap(lyon_cap(stroke.style.cap))
        .with_end_cap(lyon_cap(stroke.style.cap))
        .with_line_join(lyon_join(stroke.style.join))
        .with_miter_limit(lyon_miter_limit(stroke.style.effective_miter_limit()));
    let mut tessellator = StrokeTessellator::new();

    if tessellator
        .tessellate_path(&lyon_path, &options, &mut vertex_builder)
        .is_err()
    {
        return Mesh::default();
    }

    Mesh {
        positions: buffers.vertices.iter().map(|v| [v.x, v.y]).collect(),
        indices: buffers.indices,
    }
}

/// Adaptively flatten a single cubic Bézier into line segments within
/// `tolerance` of the true curve — the primitive both tessellators above
/// use internally, exposed directly for callers that want flattened points
/// without a full fill/stroke (a hit-test outline, a debug overlay drawing
/// control polygons).
#[must_use]
pub fn flatten_cubic(
    p0: Offset,
    p1: Offset,
    p2: Offset,
    p3: Offset,
    tolerance: f32,
) -> Vec<Offset> {
    use lyon_tessellation::path::geom::euclid::default::Point2D;
    use lyon_tessellation::path::geom::CubicBezierSegment;

    let curve = CubicBezierSegment {
        from: Point2D::new(p0.dx, p0.dy),
        ctrl1: Point2D::new(p1.dx, p1.dy),
        ctrl2: Point2D::new(p2.dx, p2.dy),
        to: Point2D::new(p3.dx, p3.dy),
    };

    let mut points = vec![p0];
    curve.for_each_flattened(tolerance.max(0.001), &mut |line| {
        points.push(Offset::new(line.to.x, line.to.y));
    });
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_paint::StrokeStyle;

    fn square() -> Path {
        let mut path = Path::new();
        path.move_to(Offset::new(0.0, 0.0));
        path.line_to(Offset::new(10.0, 0.0));
        path.line_to(Offset::new(10.0, 10.0));
        path.line_to(Offset::new(0.0, 10.0));
        path.close();
        path
    }

    #[test]
    fn a_square_tessellates_into_at_least_two_triangles() {
        let mesh = tessellate_fill(&square(), 0.1);
        assert!(mesh.triangle_count() >= 2);
        assert!(!mesh.positions.is_empty());
    }

    #[test]
    fn an_empty_path_tessellates_to_an_empty_mesh() {
        let mesh = tessellate_fill(&Path::new(), 0.1);
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn a_stroked_square_produces_a_ring_of_triangles() {
        let mesh = tessellate_stroke(
            &square(),
            &Stroke {
                width: 2.0,
                style: StrokeStyle::default(),
            },
            0.1,
        );
        assert!(mesh.triangle_count() >= 8, "{}", mesh.triangle_count());
    }

    #[test]
    fn flattening_a_straight_line_shaped_cubic_keeps_endpoints() {
        let points = flatten_cubic(
            Offset::new(0.0, 0.0),
            Offset::new(3.0, 0.0),
            Offset::new(7.0, 0.0),
            Offset::new(10.0, 0.0),
            0.1,
        );
        assert_eq!(points.first().copied(), Some(Offset::new(0.0, 0.0)));
        assert_eq!(points.last().copied(), Some(Offset::new(10.0, 0.0)));
    }
}
