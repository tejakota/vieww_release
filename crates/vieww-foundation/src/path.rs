//! Filled outlines: the shapes a decoration, an icon or a border is made of.
//!
//! Lives in `foundation` rather than in the paint layer, because both sides of
//! the framework need to *name* a shape. The paint layer fills one; the widget
//! layer describes one — an icon is a path, and a widget that could not hold a
//! path could not describe an icon. `docs/DESIGN.md` §7 forbids the widget layer
//! from depending on the paint layer, so a shared vocabulary type belongs here,
//! beside `TextStyle` and `BoxDecoration`, for exactly the same reason.
//!
//! Nothing here knows how a path is *rasterised*. That is the paint layer's
//! business, and the reason this file has no notion of a paint, a blend mode or
//! a fill rule — except in the one place winding matters to the shape itself,
//! which is [`Path::reversed`].

use std::f32::consts::{FRAC_PI_2, PI};

use crate::{Offset, Rect, Transform};

/// One whole revolution, in radians.
///
/// Named because "a full turn" is what the arc code is actually reasoning about
/// at every use, and `2.0 * PI` at each of them reads as arithmetic rather than
/// as the idea.
const TURN: f32 = 2.0 * PI;

/// One segment of a [`Path`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathVerb {
    MoveTo(Offset),
    LineTo(Offset),
    /// A cubic Bézier: two control points then the end point.
    CubicTo(Offset, Offset, Offset),
    Close,
}

/// A filled outline.
///
/// Cubics only — quadratics are representable as cubics, and carrying one curve
/// type instead of two halves the match arms in every backend for no loss.
///
/// # Why the verbs are shared
///
/// Checklist item 7, and it is a measurement rather than a preference.
/// `vieww-paint`'s `Command::transformed` lifts a recorded command into another
/// coordinate space, which the compositor does for every command it moves
/// between layers. It **composes the transform and rewrites the clip, and never
/// touches the path's geometry** — the transform rides on the command instead
/// of being baked into the verbs. So a `Vec<PathVerb>` was heap-allocated,
/// memcpy'd and dropped, unchanged, once per filled or stroked command per
/// lift. `vieww-paint`'s `counting` module measures it: **84 allocations to
/// lift 64 filled paths**, of which 64 were these.
///
/// `docs/ARCHITECTURE-AUDIT.md`'s finding 2 said not to do this — *"do not
/// pre-emptively `Rc` the path; the transform has to produce new geometry
/// regardless"*. That is true of [`transformed`](Self::transformed), which does
/// rewrite every point, and it is **not** true of the call site that runs every
/// frame. The audit asked for a measurement before acting, and this is what the
/// measurement said.
///
/// `Arc` rather than `Rc` because a `Path` travels inside a `Scene` inside a
/// `GpuRenderer`, which the test harness and `vieww-hardware` park in a
/// `static Mutex` and which must therefore stay `Send` (see `NEXT.md` §6). A
/// compile-time test in `vieww-paint` pins that.
///
/// Mutation is copy-on-write through `Arc::make_mut`, so building a path is
/// unchanged for a caller: the first `move_to` on a shared path copies it once,
/// and everything after that writes in place.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    verbs: std::sync::Arc<Vec<PathVerb>>,
}

impl Path {
    #[must_use]
    pub fn new() -> Self {
        Self {
            verbs: std::sync::Arc::new(Vec::new()),
        }
    }

    /// The segments making up this path.
    #[must_use]
    pub fn verbs(&self) -> &[PathVerb] {
        &self.verbs
    }

    /// The shared segment buffer behind this path.
    ///
    /// A path is cloned by bumping the refcount on this buffer, so two `Path`
    /// values built from one recording share it — and a backend that converts
    /// a path into its own representation once and caches the result can key
    /// that cache on the buffer's address, *provided the entry holds the
    /// buffer too*, which is what keeps the address meaning this path for as
    /// long as the entry lives. `Path` is the only shape in the foundation
    /// whose sharing is this explicit, which is why the accessor is on it and
    /// not on some more general notion of shared geometry.
    #[must_use]
    pub fn shared_verbs(&self) -> &std::sync::Arc<Vec<PathVerb>> {
        &self.verbs
    }

    /// `true` if the path has no segments.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    pub fn move_to(&mut self, point: Offset) -> &mut Self {
        std::sync::Arc::make_mut(&mut self.verbs).push(PathVerb::MoveTo(point));
        self
    }

    pub fn line_to(&mut self, point: Offset) -> &mut Self {
        std::sync::Arc::make_mut(&mut self.verbs).push(PathVerb::LineTo(point));
        self
    }

    pub fn cubic_to(&mut self, c1: Offset, c2: Offset, end: Offset) -> &mut Self {
        std::sync::Arc::make_mut(&mut self.verbs).push(PathVerb::CubicTo(c1, c2, end));
        self
    }

    pub fn close(&mut self) -> &mut Self {
        std::sync::Arc::make_mut(&mut self.verbs).push(PathVerb::Close);
        self
    }

    /// A rectangle as a path.
    #[must_use]
    pub fn rect(rect: Rect) -> Self {
        let mut path = Self::new();
        path.move_to(Offset::new(rect.left, rect.top))
            .line_to(Offset::new(rect.right, rect.top))
            .line_to(Offset::new(rect.right, rect.bottom))
            .line_to(Offset::new(rect.left, rect.bottom))
            .close();
        path
    }

    /// A rounded rectangle, corners approximated with cubics.
    ///
    /// `0.552_284_8` is the standard circular-arc constant (4/3·(√2−1), rounded
    /// to the nearest `f32`): the control-point distance that makes a cubic match
    /// a quarter circle to within about 0.02%. Using the naive 2/3 would leave
    /// corners visibly fat.
    #[must_use]
    pub fn rounded_rect(rect: Rect, radius: f32) -> Self {
        const KAPPA: f32 = 0.552_284_8;

        let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
        if radius <= 0.0 {
            return Self::rect(rect);
        }
        let k = radius * KAPPA;
        let (l, t, r, b) = (rect.left, rect.top, rect.right, rect.bottom);

        let mut path = Self::new();
        path.move_to(Offset::new(l + radius, t))
            .line_to(Offset::new(r - radius, t))
            .cubic_to(
                Offset::new(r - radius + k, t),
                Offset::new(r, t + radius - k),
                Offset::new(r, t + radius),
            )
            .line_to(Offset::new(r, b - radius))
            .cubic_to(
                Offset::new(r, b - radius + k),
                Offset::new(r - radius + k, b),
                Offset::new(r - radius, b),
            )
            .line_to(Offset::new(l + radius, b))
            .cubic_to(
                Offset::new(l + radius - k, b),
                Offset::new(l, b - radius + k),
                Offset::new(l, b - radius),
            )
            .line_to(Offset::new(l, t + radius))
            .cubic_to(
                Offset::new(l, t + radius - k),
                Offset::new(l + radius - k, t),
                Offset::new(l + radius, t),
            )
            .close();
        path
    }

    /// A rounded rectangle with a rounded rectangle cut out of it: the border of
    /// a box, `width` logical pixels thick, drawn *inside* `outer`.
    ///
    /// The hole is a reversed subpath, so under non-zero winding the middle is
    /// genuinely unpainted. See [`reversed`](Self::reversed) for why that matters.
    ///
    /// The inner radius is the outer one less the thickness, which is what keeps
    /// a border concentric with the corner it follows instead of pinching at it.
    #[must_use]
    pub fn rounded_ring(outer: Rect, radius: f32, width: f32) -> Self {
        let inner = outer.inflate(-width);
        if width <= 0.0 || inner.is_empty() {
            // Thicker than the box is wide: the ring has swallowed its own hole.
            return Self::rounded_rect(outer, radius);
        }
        let mut path = Self::rounded_rect(outer, radius);
        path.extend(&Self::rounded_rect(inner, (radius - width).max(0.0)).reversed());
        path
    }

    /// A circular arc, as an open subpath.
    ///
    /// ```
    /// use vieww_foundation::{Offset, Path};
    /// use std::f32::consts::{FRAC_PI_2, PI};
    ///
    /// // The top half of a circle, starting at nine o'clock.
    /// let half = Path::arc(Offset::new(50.0, 50.0), 20.0, PI, PI);
    /// # let _ = (half, FRAC_PI_2);
    /// ```
    ///
    /// # Angles are clockwise, and zero is three o'clock
    ///
    /// The same convention as [`Transform::rotate`](crate::Transform::rotate),
    /// and clockwise for the same reason it is: y grows *downwards* here, so the
    /// rotation that looks clockwise on screen is the positive one. A spinner
    /// that should begin at twelve o'clock begins at `-FRAC_PI_2`.
    ///
    /// `sweep` is signed — a negative sweep runs anticlockwise — and is clamped
    /// to one full turn, because more than a turn draws over the same pixels and
    /// the segment count would otherwise grow without bound.
    ///
    /// # It is an outline, not a shape
    ///
    /// An arc has no area, and [`Canvas`](crate::Path) fills rather than strokes
    /// — so filling this alone paints the region between the arc and its own
    /// chord, which is a pie slice with a bite out of it and almost never what
    /// anybody wanted. [`arc_ring`](Self::arc_ring) is the fillable form.
    #[must_use]
    pub fn arc(center: Offset, radius: f32, start: f32, sweep: f32) -> Self {
        let mut path = Self::new();
        path.append_arc(center, radius, start, sweep, true);
        path
    }

    /// A band of a circle: the fillable form of [`arc`](Self::arc).
    ///
    /// The region between two concentric arcs — what a circular progress
    /// indicator is made of, and what a stroked arc would be if this canvas
    /// stroked anything. `width` is measured inwards from `radius`, matching
    /// [`rounded_ring`](Self::rounded_ring).
    ///
    /// ```
    /// use vieww_foundation::{Offset, Path};
    /// use std::f32::consts::{FRAC_PI_2, PI};
    ///
    /// // A quarter-turn band, four pixels thick, starting at twelve o'clock.
    /// let tick = Path::arc_ring(Offset::new(24.0, 24.0), 20.0, 4.0, -FRAC_PI_2, FRAC_PI_2);
    /// assert!(!tick.is_empty());
    /// # let _ = PI;
    /// ```
    ///
    /// # The ends are cut along the radius
    ///
    /// Flat caps, square to the circle rather than rounded. Round caps are two
    /// more half-circles and a different shape at the seam where a sweep meets
    /// itself, so they belong to whoever needs them rather than to everybody.
    ///
    /// # A full turn has no seam
    ///
    /// At a full sweep the two radial cuts land on top of each other, and a
    /// closed shape with a zero-width notch in it is a rendering artefact
    /// waiting to happen. So a full turn is built as a genuine ring instead —
    /// two closed circles, the inner one reversed, which is exactly what
    /// [`rounded_ring`](Self::rounded_ring) does with rectangles.
    ///
    /// A `width` at or beyond `radius` leaves no hole, and the result is a pie
    /// slice from the centre rather than a band. Same answer `rounded_ring`
    /// gives when the border swallows its own middle.
    #[must_use]
    pub fn arc_ring(center: Offset, radius: f32, width: f32, start: f32, sweep: f32) -> Self {
        let sweep = sweep.clamp(-TURN, TURN);
        let inner = radius - width;

        if radius <= 0.0 || width <= 0.0 || sweep == 0.0 {
            return Self::new();
        }

        let mut path = Self::new();

        // A full turn: two closed rings rather than a band whose two ends meet
        // at a seam of zero width.
        if sweep.abs() >= TURN {
            path.append_arc(center, radius, start, TURN, true);
            path.close();
            if inner > 0.0 {
                let mut hole = Self::new();
                hole.append_arc(center, inner, start, TURN, true);
                hole.close();
                path.extend(&hole.reversed());
            }
            return path;
        }

        // No hole left: a wedge from the centre, which is the honest answer
        // rather than an inside-out band.
        if inner <= 0.0 {
            path.move_to(center);
            path.append_arc(center, radius, start, sweep, false);
            path.close();
            return path;
        }

        // Out along the far edge, across, back along the near one, and shut.
        path.append_arc(center, radius, start, sweep, true);
        path.append_arc(center, inner, start + sweep, -sweep, false);
        path.close();
        path
    }

    /// Append an arc's cubics, optionally starting a new subpath at its origin.
    ///
    /// Split so that no segment spans more than a quarter turn: the cubic
    /// approximation is excellent to about 90 degrees and degrades quickly past
    /// it. `k = 4/3·tan(α/4)` is the general form of the constant
    /// [`rounded_rect`](Self::rounded_rect) hard-codes — at a quarter turn it
    /// *is* that constant, which is the arithmetic worth checking if this ever
    /// looks wrong.
    fn append_arc(&mut self, center: Offset, radius: f32, start: f32, sweep: f32, open: bool) {
        let sweep = sweep.clamp(-TURN, TURN);
        let at = |angle: f32| {
            let (sin, cos) = angle.sin_cos();
            Offset::new(center.dx + radius * cos, center.dy + radius * sin)
        };

        if open {
            self.move_to(at(start));
        }
        if sweep == 0.0 || radius <= 0.0 {
            return;
        }

        // Bounded by the clamp above: a full turn is four quarters. Written as a
        // ladder rather than a cast so the bound is visible rather than trusted.
        let quarters = sweep.abs() / FRAC_PI_2;
        let segments: u16 = if quarters <= 1.0 {
            1
        } else if quarters <= 2.0 {
            2
        } else if quarters <= 3.0 {
            3
        } else {
            4
        };

        let step = sweep / f32::from(segments);
        let k = 4.0 / 3.0 * (step / 4.0).tan() * radius;

        let mut angle = start;
        for _ in 0..segments {
            let next = angle + step;
            let (from, to) = (at(angle), at(next));
            // The tangent at an angle, which is the circle's derivative there.
            let out = Offset::new(-angle.sin(), angle.cos()).scale(k);
            let into = Offset::new(-next.sin(), next.cos()).scale(k);
            self.cubic_to(from + out, to - into, to);
            angle = next;
        }
    }

    /// Append every segment of `other` to this path.
    ///
    /// The subpaths stay separate — `other`'s leading `MoveTo` is what keeps
    /// them apart — so this composes shapes rather than joining them.
    pub fn extend(&mut self, other: &Self) -> &mut Self {
        std::sync::Arc::make_mut(&mut self.verbs).extend_from_slice(&other.verbs);
        self
    }

    /// The same outline traced in the opposite direction.
    ///
    /// Fills here are **non-zero winding**, which is what makes this useful: a
    /// shape plus a smaller reversed shape inside it is a ring with a real hole,
    /// rather than a smaller shape drawn on top of a larger one. That difference
    /// is invisible against an opaque background and obvious against anything
    /// else — a translucent border painted the naive way tints everything it
    /// surrounds.
    ///
    /// Each subpath is reversed in place, and the order of subpaths is kept.
    #[must_use]
    pub fn reversed(&self) -> Self {
        let mut out = Self::new();
        // A subpath runs from one `MoveTo` to just before the next.
        let mut start = 0;
        for index in 0..=self.verbs.len() {
            let ends_here = index == self.verbs.len()
                || (index > start && matches!(self.verbs[index], PathVerb::MoveTo(_)));
            if ends_here {
                out.extend(&reverse_subpath(&self.verbs[start..index]));
                start = index;
            }
        }
        out
    }

    /// Every point run through `transform`.
    ///
    /// Control points transform with the curve, which is what makes this exact
    /// for an affine transform rather than an approximation: an affine map takes
    /// a cubic Bézier to the cubic Bézier through the mapped control points.
    ///
    /// This is how an icon drawn in its own coordinates ends up the size it is
    /// asked to be. The alternative — pushing a transform onto the canvas around
    /// the fill — would work too, and would make the recorded command's bounds
    /// depend on canvas state that damage tracking has already resolved away.
    /// # The identity is a clone, not a copy
    ///
    /// A path transformed by [`Transform::IDENTITY`] is the same path, so it
    /// shares the same verb buffer instead of allocating an identical one.
    /// That is not only cheaper — it is what keeps the buffer's *address*
    /// meaningful, which is how a backend recognises "this is the clip I
    /// already rasterised" (see `shared_verbs`). The compositor moves
    /// commands between layers with an identity transform constantly; without
    /// this, every one of them got a fresh buffer holding the same numbers,
    /// and every clip cache downstream missed on every command.
    #[must_use]
    pub fn transformed(&self, transform: Transform) -> Self {
        if transform.is_identity() {
            return self.clone();
        }
        Self {
            verbs: self
                .verbs
                .iter()
                .map(|verb| match *verb {
                    PathVerb::MoveTo(p) => PathVerb::MoveTo(transform.apply(p)),
                    PathVerb::LineTo(p) => PathVerb::LineTo(transform.apply(p)),
                    PathVerb::CubicTo(c1, c2, end) => PathVerb::CubicTo(
                        transform.apply(c1),
                        transform.apply(c2),
                        transform.apply(end),
                    ),
                    PathVerb::Close => PathVerb::Close,
                })
                .collect::<Vec<_>>()
                .into(),
        }
    }

    /// This path scaled from `from` into `into`, preserving its aspect ratio and
    /// centred in whatever slack that leaves.
    ///
    /// What an icon does with its viewbox. Uniform, deliberately: an icon
    /// stretched to fill a non-square box is a broken icon, and the fix is
    /// always to give it a square box rather than to stretch it.
    #[must_use]
    pub fn fitted(&self, from: Rect, into: Rect) -> Self {
        if from.width() <= 0.0 || from.height() <= 0.0 {
            return self.clone();
        }
        let scale = (into.width() / from.width()).min(into.height() / from.height());
        let width = from.width() * scale;
        let height = from.height() * scale;

        // Scale about the source's origin, then move the scaled box to where it
        // sits centred in the destination.
        let transform = Transform::translate(Offset::new(-from.left, -from.top))
            .then(Transform::scale(scale, scale))
            .then(Transform::translate(Offset::new(
                into.left + (into.width() - width) / 2.0,
                into.top + (into.height() - height) / 2.0,
            )));
        self.transformed(transform)
    }

    /// The axis-aligned bounds of every point mentioned by the path.
    ///
    /// Control points are included, so this over-estimates for curves that bend
    /// inward. For damage tracking that is the safe direction to be wrong in —
    /// too large repaints correctly, too small leaves stale pixels.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let mut left = f32::INFINITY;
        let mut top = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        let mut include = |point: Offset| {
            left = left.min(point.dx);
            top = top.min(point.dy);
            right = right.max(point.dx);
            bottom = bottom.max(point.dy);
        };

        for verb in self.verbs.iter() {
            match *verb {
                PathVerb::MoveTo(p) | PathVerb::LineTo(p) => include(p),
                PathVerb::CubicTo(c1, c2, end) => {
                    include(c1);
                    include(c2);
                    include(end);
                }
                PathVerb::Close => {}
            }
        }

        if left > right {
            Rect::ZERO
        } else {
            Rect::new(left, top, right, bottom)
        }
    }

    /// The fillable form of stroking this path with a round pen of `width`.
    ///
    /// A stroke and a fill are different rasterisation problems, and not every
    /// backend solves the stroke one well: a thin pen whose edges straddle
    /// pixel boundaries is the *hard* case for a coverage antialiaser — the
    /// same thin line can render wide and soft on one rasteriser and crisp on
    /// another. Fills of ordinary shapes do not have that failure mode, which
    /// is why an icon set baked as outlines is stable where a set drawn as
    /// centreline strokes is not. This method moves a path across that line:
    /// it returns a path that, **filled with the nonzero rule**, paints the
    /// same ink as stroking the original with `width`.
    ///
    /// # How it is built, and why that is safe
    ///
    /// The outline is a **union**, not a contour: every segment becomes a
    /// quad between its two offset edges, and every sharp corner and every
    /// open end gets a disc of radius `width / 2` (round joins and round
    /// caps). Each piece is closed and wound the same way, so the nonzero
    /// rule takes their union no matter how they overlap — a self-intersecting
    /// centreline, the classic hard case of contour offsetting, needs no
    /// special handling because there is no contour to repair. Curves are
    /// flattened first, at a tolerance held well under a pixel, so a curve's
    /// joins are the same round joins a polyline's are.
    ///
    /// # What is *not* preserved
    ///
    /// Cap and join styles: this is round caps and round joins, always,
    /// because a union of quads and discs is what does not need the geometry
    /// repairs that miter and bevel joins do. Dash patterns are not expanded —
    /// a caller that wants a dashed *fillable* shape should dash the
    /// centreline first. Widths of zero or less return an empty path, which is
    /// also what a caller drawing an invisible line should paint.
    ///
    /// # Cross-checked against the ecosystem
    ///
    /// The defect this exists to fix is not hypothetical, and neither is the
    /// fix. Vello's own issue #592 ("anti-aliasing rendering seems not fine
    /// enough") diagnoses thin strokes rendered weak and ragged by its
    /// area-antialiasing pass, with the recommended workaround being *thicker
    /// geometry at partial alpha* — i.e. move the ink into a fill rather than
    /// argue with the stroker; other projects in that family adopted exactly
    /// that. The same lesson has been learned at the placement layer in other
    /// toolkits too: crispness came back only when rasterised
    /// pieces were pixel-grid aligned rather than blitted at fractional
    /// positions. The icon-and-text path of immediate-mode UIs is the same design end to end —
    /// rasterise once at an integer size, then blit at integer offsets — never
    /// handing a thin stroke to the GPU at all. The two halves of this
    /// workspace's fix (expand the pen here, snap the placement in the render
    /// objects) are those two findings, applied without depending on any of
    /// those projects' code.
    ///
    /// ```
    /// use vieww_foundation::{Offset, Path};
    ///
    /// // A short horizontal line, four wide: the outline's bounds are the
    /// // line grown by half the width on every side.
    /// let mut line = Path::new();
    /// line.move_to(Offset::new(10.0, 10.0));
    /// line.line_to(Offset::new(20.0, 10.0));
    /// let outline = line.stroke_outline(4.0);
    /// let bounds = outline.bounds();
    /// assert_eq!(bounds, vieww_foundation::Rect::new(8.0, 8.0, 22.0, 12.0));
    /// ```
    #[must_use]
    pub fn stroke_outline(&self, width: f32) -> Self {
        if width <= 0.0 || self.verbs.is_empty() {
            return Self::new();
        }
        let radius = width / 2.0;
        // Flattening error held to a small fraction of the pen and of a pixel,
        // so a curve reads as the curve at every size an icon is drawn at.
        let tolerance = (width * 0.1).clamp(0.01, 0.15);

        let mut out = Self::new();
        for polyline in self.polylines(tolerance) {
            append_stroke(&mut out, &polyline, radius);
        }
        out
    }

    /// Every subpath as a polyline, curves flattened to `tolerance`.
    ///
    /// Private: this is the sampling step of [`stroke_outline`](Self::stroke_outline)
    /// and the tolerance belongs to the pen that called it.
    fn polylines(&self, tolerance: f32) -> Vec<Polyline> {
        let mut out: Vec<Polyline> = Vec::new();
        let mut current = Polyline {
            points: Vec::new(),
            closed: false,
        };
        let mut cursor = Offset::ZERO;

        for verb in self.verbs.iter() {
            match *verb {
                PathVerb::MoveTo(point) => {
                    if current.points.len() > 1 {
                        out.push(std::mem::take(&mut current));
                    } else {
                        current.points.clear();
                    }
                    current.points.push(point);
                    cursor = point;
                }
                PathVerb::LineTo(point) => {
                    if current.points.is_empty() {
                        // Malformed rather than unreachable — every path this
                        // crate builds opens with a `MoveTo`, but a stroke
                        // outline of a malformed path should still be defined.
                        current.points.push(cursor);
                    }
                    current.points.push(point);
                    cursor = point;
                }
                PathVerb::CubicTo(c1, c2, end) => {
                    if current.points.is_empty() {
                        current.points.push(cursor);
                    }
                    flatten_cubic(&mut current.points, cursor, c1, c2, end, tolerance, 0);
                    cursor = end;
                }
                PathVerb::Close => {
                    // Not duplicating the start point: a closed polyline is
                    // treated cyclically by the outline, so the pen's corner
                    // at the seam is a corner like any other rather than a
                    // spurious zero-length segment.
                    current.closed = true;
                    if current.points.len() > 1 {
                        if let Some(&first) = current.points.first() {
                            cursor = first;
                        }
                        out.push(std::mem::take(&mut current));
                    } else {
                        current.points.clear();
                        current.closed = false;
                    }
                }
            }
        }
        if current.points.len() > 1 {
            out.push(current);
        }
        out
    }
}

/// One subpath sampled to points. `closed` when the original closed, in which
/// case the last point connects back to the first.
#[derive(Default)]
struct Polyline {
    points: Vec<Offset>,
    closed: bool,
}

/// Split `c` into line segments flat to `tolerance`, appended to `points`.
///
/// Recursive midpoint subdivision with a flatness test: a cubic is flat when
/// both control points sit within `tolerance` of the chord. Depth-capped so a
/// degenerate control polygon cannot recurse forever.
fn flatten_cubic(
    points: &mut Vec<Offset>,
    from: Offset,
    c1: Offset,
    c2: Offset,
    to: Offset,
    tolerance: f32,
    depth: u8,
) {
    // Distance from a point to the chord, which is the error the flattening
    // is holding down. No square roots: the comparison is squared both sides.
    let chord = to - from;
    let length = chord.distance_squared();
    let off_chord = |point: Offset| {
        let cross = chord.dx * (point.dy - from.dy) - chord.dy * (point.dx - from.dx);
        (cross * cross) / length.max(f32::EPSILON)
    };
    let flat = off_chord(c1) <= tolerance * tolerance && off_chord(c2) <= tolerance * tolerance;
    if flat || depth >= 16 {
        points.push(to);
        return;
    }

    // de Casteljau at the midpoint: two cubics where there was one.
    let e = |a: Offset, b: Offset| Offset::new((a.dx + b.dx) / 2.0, (a.dy + b.dy) / 2.0);
    let a = e(from, c1);
    let b = e(c1, c2);
    let c = e(c2, to);
    let ab = e(a, b);
    let bc = e(b, c);
    let mid = e(ab, bc);
    flatten_cubic(points, from, a, ab, mid, tolerance, depth + 1);
    flatten_cubic(points, mid, bc, c, to, tolerance, depth + 1);
}

/// Append the union-of-pieces outline of stroking `polyline` with a round pen.
///
/// Every piece is closed and wound the same way (negative shoelace area), so
/// the nonzero fill rule unions them however they overlap. A closed polyline
/// is traced cyclically: the segment from its last point to its first is
/// stroked like any other, and the seam corner is a corner like any other.
fn append_stroke(out: &mut Path, polyline: &Polyline, radius: f32) {
    let points = &polyline.points;
    debug_assert!(points.len() > 1, "a one-point polyline has no stroke");
    let closed = polyline.closed;
    let len = points.len();

    // A turn sharper than this leaves a bevel notch the eye can see, so the
    // corner gets a disc. Below it the notch is a fraction of the radius and
    // the two quads already cover the ink.
    const JOIN_DISC_COSINE: f32 = 0.75; // ~41 degrees of turn

    // One quad per segment, between the offset edges. A closed polyline has
    // `len` segments — the hop from its last point back to its first is a
    // segment like any other; an open one has `len - 1`.
    let segment_count = if closed { len } else { len - 1 };
    for index in 0..segment_count {
        let a = points[index];
        let b = points[(index + 1) % len];
        let span = b - a;
        let length = span.distance();
        if length <= f32::EPSILON {
            continue;
        }
        // The left-hand normal of the travel direction, scaled to the radius:
        // rotating (dx, dy) a quarter turn gives (-dy, dx).
        let normal = Offset::new(-span.dy, span.dx).scale(radius / length);
        push_wound_quad(out, a + normal, b + normal, b - normal, a - normal);
    }

    // A disc at every corner that turns sharply enough, and at each end of an
    // open subpath — the round cap. Closed polylines read their neighbours
    // cyclically, so the seam corner cannot be missed.
    let end = len - 1;
    for index in 0..len {
        let cap = !closed && (index == 0 || index == end);
        let before = if closed || index > 0 {
            points[(index + len - 1) % len]
        } else {
            points[0]
        };
        let at = points[index];
        let after = if closed || index < end {
            points[(index + 1) % len]
        } else {
            points[end]
        };
        let into = at - before;
        let out_of = after - at;
        let sharp = cap
            || (into.distance_squared() > f32::EPSILON
                && out_of.distance_squared() > f32::EPSILON
                && {
                    let into_unit = into.scale(1.0 / into.distance());
                    let out_unit = out_of.scale(1.0 / out_of.distance());
                    // The dot product of the unit directions: below the
                    // threshold the corner turns sharply enough to show.
                    into_unit.dx * out_unit.dx + into_unit.dy * out_unit.dy <= JOIN_DISC_COSINE
                });
        if sharp {
            push_disc(out, at, radius);
        }
    }
}

/// A quad, wound to match every other piece of the outline.
///
/// The construction in the caller is already consistent, but the guarantee
/// this function leaves behind — every closed piece negative under the
/// shoelace formula — is what makes the union sound, and a caller cannot
/// verify a construction it cannot see.
fn push_wound_quad(out: &mut Path, a: Offset, b: Offset, c: Offset, d: Offset) {
    // Shoelace sign of the quad as given; reversed if it disagrees.
    let area = (b.dx - a.dx) * (c.dy - a.dy) - (c.dx - a.dx) * (b.dy - a.dy);
    if area < 0.0 {
        out.move_to(a);
        out.line_to(b);
        out.line_to(c);
        out.line_to(d);
    } else {
        out.move_to(a);
        out.line_to(d);
        out.line_to(c);
        out.line_to(b);
    }
    out.close();
}

/// A disc of radius `r` around `centre`, wound to match the quads.
///
/// An octagon of sixteen points — the chord error at a one-pixel radius is
/// two hundredths of a pixel, which no antialiaser reports.
fn push_disc(out: &mut Path, centre: Offset, r: f32) {
    const SIDES: usize = 16;
    // Decreasing angle so the winding matches the quads: opposite windings
    // under the nonzero rule cut holes rather than unions.
    let step = -std::f32::consts::TAU / SIDES as f32;
    let mut angle = 0.0f32;
    out.move_to(centre + Offset::new(r, 0.0));
    for _ in 0..SIDES {
        angle += step;
        let (sin, cos) = angle.sin_cos();
        out.line_to(centre + Offset::new(r * cos, r * sin));
    }
    out.close();
}

/// One subpath, traced backwards.
///
/// The reversed subpath starts where the original ended and ends where it began,
/// with each cubic's control points swapped. A `Close` is carried over: in the
/// original it is the implicit segment from the last point back to the first, and
/// in the reversal it is that same segment traced the other way, so the loop
/// closes without an extra verb.
fn reverse_subpath(verbs: &[PathVerb]) -> Path {
    let Some(PathVerb::MoveTo(first)) = verbs.first().copied() else {
        // A subpath with no `MoveTo` has no start to reverse from. Callers build
        // paths through `move_to`, so this is a malformed path rather than a
        // case to handle; dropping it is quieter than panicking in a paint.
        return Path::new();
    };

    let closed = verbs.iter().any(|verb| matches!(verb, PathVerb::Close));
    // Each segment as (where it started, the verb), so the reversal knows the
    // point to travel *back* to.
    let mut segments = Vec::with_capacity(verbs.len());
    let mut cursor = first;
    for verb in &verbs[1..] {
        match *verb {
            PathVerb::LineTo(end) => {
                segments.push((cursor, *verb));
                cursor = end;
            }
            PathVerb::CubicTo(_, _, end) => {
                segments.push((cursor, *verb));
                cursor = end;
            }
            PathVerb::MoveTo(_) | PathVerb::Close => {}
        }
    }

    let mut out = Path::new();
    out.move_to(cursor);
    for (start, verb) in segments.into_iter().rev() {
        match verb {
            PathVerb::LineTo(_) => {
                out.line_to(start);
            }
            PathVerb::CubicTo(c1, c2, _) => {
                out.cubic_to(c2, c1, start);
            }
            PathVerb::MoveTo(_) | PathVerb::Close => {}
        }
    }
    if closed {
        out.close();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rect_path_bounds_itself() {
        let rect = Rect::new(1.0, 2.0, 11.0, 22.0);
        assert_eq!(Path::rect(rect).bounds(), rect);
    }

    #[test]
    fn a_zero_width_outline_is_empty() {
        let mut line = Path::new();
        line.move_to(Offset::new(0.0, 0.0));
        line.line_to(Offset::new(10.0, 0.0));
        assert!(line.stroke_outline(0.0).is_empty());
        assert!(line.stroke_outline(-1.0).is_empty());
    }

    #[test]
    fn an_outline_grows_by_half_the_pen_on_every_side() {
        let mut line = Path::new();
        line.move_to(Offset::new(10.0, 10.0));
        line.line_to(Offset::new(20.0, 10.0));
        // Round caps extend past both ends by the radius, and the offset
        // edges grow above and below by it.
        let bounds = line.stroke_outline(4.0).bounds();
        assert_eq!(bounds, Rect::new(8.0, 8.0, 22.0, 12.0));
    }

    #[test]
    fn an_outline_of_a_diagonal_covers_the_line() {
        let mut line = Path::new();
        line.move_to(Offset::new(0.0, 0.0));
        line.line_to(Offset::new(10.0, 10.0));
        let outline = line.stroke_outline(2.0);
        // The centre of the stroke — the original line — is inside the ink by
        // construction, and the bounds are the line grown by the radius along
        // both axes.
        let bounds = outline.bounds();
        assert_eq!(bounds, Rect::new(-1.0, -1.0, 11.0, 11.0));
        assert!(!outline.is_empty());
    }

    #[test]
    fn a_closed_outline_strokes_the_segment_home() {
        // A triangle whose last side exists only through `Close`: the outline
        // must cover that side too, which shows up as the bounds growing
        // below the open polyline's own extent.
        let mut triangle = Path::new();
        triangle.move_to(Offset::new(2.0, 2.0));
        triangle.line_to(Offset::new(12.0, 2.0));
        triangle.line_to(Offset::new(7.0, 12.0));
        triangle.close();
        let bounds = triangle.stroke_outline(2.0).bounds();
        // radius 1 all around the shape's own bounds (2,2)-(12,12).
        assert_eq!(bounds, Rect::new(1.0, 1.0, 13.0, 13.0));
    }

    #[test]
    fn every_piece_of_an_outline_is_wound_the_same_way() {
        // The union guarantee: each closed subpath must have the same shoelace
        // sign, or the nonzero rule cuts holes where pieces overlap.
        let mut line = Path::new();
        line.move_to(Offset::new(0.0, 0.0));
        line.line_to(Offset::new(10.0, 2.0));
        line.line_to(Offset::new(4.0, 8.0));
        let outline = line.stroke_outline(3.0);

        let mut subpaths = 0;
        let mut winding: Vec<f32> = Vec::new();
        let mut area = 0.0f32;
        let mut last = Offset::ZERO;
        let mut started = false;
        for verb in outline.verbs() {
            match *verb {
                PathVerb::MoveTo(point) => {
                    if started {
                        winding.push(area);
                        area = 0.0;
                    }
                    started = true;
                    subpaths += 1;
                    last = point;
                }
                PathVerb::LineTo(point) => {
                    area += last.dx * point.dy - point.dx * last.dy;
                    last = point;
                }
                PathVerb::Close => {
                    winding.push(area);
                    area = 0.0;
                    started = false;
                }
                PathVerb::CubicTo(..) => {}
            }
        }
        if started {
            winding.push(area);
        }

        assert!(subpaths >= 3, "quads and discs: {subpaths}");
        let negatives = winding.iter().filter(|w| **w < 0.0).count();
        assert_eq!(negatives, winding.len(), "windings: {winding:?}");
    }

    #[test]
    fn a_curved_centreline_outlines_to_a_band() {
        // A quarter arc of radius 10 from three o'clock sweeping a quarter
        // turn, stroked at 2: the outline's bounds are the arc's own quadrant
        // grown by the pen's radius. Angle zero is three o'clock and a
        // positive sweep runs with the screen's y axis, so the ink occupies
        // the bottom-right quadrant of the circle.
        let arc = Path::arc(
            Offset::new(10.0, 10.0),
            10.0,
            0.0,
            std::f32::consts::FRAC_PI_2,
        );
        let bounds = arc.stroke_outline(2.0).bounds();
        let radius = 1.0;
        assert!((bounds.left - (10.0 - radius)).abs() < 0.3, "{bounds:?}");
        assert!((bounds.right - (20.0 + radius)).abs() < 0.3, "{bounds:?}");
        assert!((bounds.top - (10.0 - radius)).abs() < 0.3, "{bounds:?}");
        assert!((bounds.bottom - (20.0 + radius)).abs() < 0.3, "{bounds:?}");
    }

    #[test]
    fn a_rounded_rect_stays_within_the_rect_it_rounds() {
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        let bounds = Path::rounded_rect(rect, 6.0).bounds();
        assert!(bounds.left >= rect.left - 1e-4);
        assert!(bounds.right <= rect.right + 1e-4);
        assert!(bounds.top >= rect.top - 1e-4);
        assert!(bounds.bottom <= rect.bottom + 1e-4);
    }

    #[test]
    fn an_over_large_radius_is_clamped_to_half_the_shorter_side() {
        let rect = Rect::new(0.0, 0.0, 40.0, 20.0);
        // Radius 500 must not invert the corners; it becomes a stadium.
        let bounds = Path::rounded_rect(rect, 500.0).bounds();
        assert!((bounds.width() - 40.0).abs() < 1e-3);
        assert!((bounds.height() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn a_zero_radius_degenerates_to_a_plain_rect() {
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(Path::rounded_rect(rect, 0.0), Path::rect(rect));
    }

    #[test]
    fn reversing_a_rect_visits_its_corners_in_the_opposite_order() {
        let rect = Rect::new(0.0, 0.0, 10.0, 20.0);
        let corners = |path: &Path| {
            path.verbs()
                .iter()
                .filter_map(|verb| match *verb {
                    PathVerb::MoveTo(p) | PathVerb::LineTo(p) => Some(p),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        let forward = corners(&Path::rect(rect));
        let mut backward = corners(&Path::rect(rect).reversed());
        backward.reverse();

        assert_eq!(forward, backward);
        assert_ne!(
            forward.first(),
            corners(&Path::rect(rect).reversed()).first(),
            "the reversed outline starts at the other end"
        );
    }

    #[test]
    fn a_reversed_cubic_swaps_its_control_points() {
        let mut path = Path::new();
        path.move_to(Offset::new(0.0, 0.0)).cubic_to(
            Offset::new(1.0, 0.0),
            Offset::new(2.0, 1.0),
            Offset::new(3.0, 3.0),
        );

        assert_eq!(
            path.reversed().verbs(),
            &[
                PathVerb::MoveTo(Offset::new(3.0, 3.0)),
                PathVerb::CubicTo(
                    Offset::new(2.0, 1.0),
                    Offset::new(1.0, 0.0),
                    Offset::new(0.0, 0.0)
                ),
            ]
        );
    }

    #[test]
    fn reversing_twice_returns_the_original_outline() {
        let path = Path::rounded_rect(Rect::new(0.0, 0.0, 40.0, 20.0), 6.0);
        assert_eq!(path.reversed().reversed(), path);
    }

    #[test]
    fn each_subpath_is_reversed_and_they_keep_their_order() {
        let mut path = Path::rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        path.extend(&Path::rect(Rect::new(20.0, 20.0, 30.0, 30.0)));

        let reversed = path.reversed();
        let starts: Vec<_> = reversed
            .verbs()
            .iter()
            .filter_map(|verb| match *verb {
                PathVerb::MoveTo(p) => Some(p),
                _ => None,
            })
            .collect();

        assert_eq!(starts.len(), 2, "two subpaths in, two out");
        assert!(
            starts[0].dx < starts[1].dx,
            "the first subpath is still first"
        );
    }

    #[test]
    fn a_ring_is_an_outline_around_a_hole() {
        let outer = Rect::new(0.0, 0.0, 40.0, 20.0);
        let ring = Path::rounded_ring(outer, 4.0, 2.0);

        assert_eq!(ring.bounds(), Path::rounded_rect(outer, 4.0).bounds());
        assert_eq!(
            ring.verbs()
                .iter()
                .filter(|v| matches!(v, PathVerb::MoveTo(_)))
                .count(),
            2,
            "an outer subpath and an inner one"
        );
    }

    #[test]
    fn a_ring_thicker_than_the_box_is_a_solid_shape() {
        let outer = Rect::new(0.0, 0.0, 10.0, 10.0);
        // 6px each side leaves no room for a hole in a 10px box.
        assert_eq!(
            Path::rounded_ring(outer, 2.0, 6.0),
            Path::rounded_rect(outer, 2.0)
        );
    }

    #[test]
    fn an_empty_path_has_zero_bounds_rather_than_infinite_ones() {
        assert_eq!(Path::new().bounds(), Rect::ZERO);
    }

    #[test]
    fn transforming_moves_control_points_with_their_curve() {
        let mut path = Path::new();
        path.move_to(Offset::new(1.0, 1.0)).cubic_to(
            Offset::new(2.0, 1.0),
            Offset::new(3.0, 2.0),
            Offset::new(3.0, 3.0),
        );

        let moved = path.transformed(Transform::translate(Offset::new(10.0, 0.0)));
        assert_eq!(
            moved.verbs(),
            &[
                PathVerb::MoveTo(Offset::new(11.0, 1.0)),
                PathVerb::CubicTo(
                    Offset::new(12.0, 1.0),
                    Offset::new(13.0, 2.0),
                    Offset::new(13.0, 3.0)
                ),
            ]
        );
    }

    #[test]
    fn fitting_scales_uniformly_and_centres_the_slack() {
        let viewbox = Rect::new(0.0, 0.0, 10.0, 10.0);
        let square = Path::rect(viewbox);

        // Into a wide box: the shape scales by the *height* and is centred
        // horizontally, rather than being stretched into an oblong.
        let fitted = square.fitted(viewbox, Rect::new(0.0, 0.0, 40.0, 20.0));
        let bounds = fitted.bounds();

        assert!((bounds.width() - 20.0).abs() < 1e-4, "{bounds:?}");
        assert!((bounds.height() - 20.0).abs() < 1e-4, "{bounds:?}");
        assert!((bounds.left - 10.0).abs() < 1e-4, "centred: {bounds:?}");
    }

    // ------------------------------------------------------------------- arcs

    /// Walk a path's verbs and hand back every point it actually reaches.
    ///
    /// Endpoints only — control points are off the curve and a shape is not
    /// obliged to pass anywhere near them, so including them would make every
    /// bounds assertion below wrong in a way that looked like a real failure.
    fn endpoints(path: &Path) -> Vec<Offset> {
        path.verbs()
            .iter()
            .filter_map(|verb| match *verb {
                PathVerb::MoveTo(p) | PathVerb::LineTo(p) | PathVerb::CubicTo(_, _, p) => Some(p),
                PathVerb::Close => None,
            })
            .collect()
    }

    /// How far each endpoint sits from `center`.
    fn radii(path: &Path, center: Offset) -> Vec<f32> {
        endpoints(path)
            .into_iter()
            .map(|p| ((p.dx - center.dx).powi(2) + (p.dy - center.dy).powi(2)).sqrt())
            .collect()
    }

    #[test]
    fn an_arcs_endpoints_all_sit_on_its_circle() {
        // The cheapest possible check on the cubic approximation, and the one
        // that catches a wrong tangent direction: whatever the curve does in
        // between, every point it is anchored at must be exactly `radius` away.
        let center = Offset::new(30.0, 40.0);
        let arc = Path::arc(center, 12.0, -FRAC_PI_2, TURN * 0.75);
        for r in radii(&arc, center) {
            assert!((r - 12.0).abs() < 1e-3, "off the circle: {r}");
        }
    }

    #[test]
    fn zero_is_three_oclock_and_a_positive_sweep_goes_clockwise() {
        // The convention `Transform::rotate` already states, pinned here because
        // it is the one thing about an arc API nobody can guess and everybody
        // has to know. Clockwise *on screen*, which is +y, because y is down.
        let center = Offset::ZERO;
        let quarter = Path::arc(center, 10.0, 0.0, FRAC_PI_2);
        let points = endpoints(&quarter);

        let first = points.first().copied().expect("an arc starts somewhere");
        let last = points.last().copied().expect("and ends somewhere");

        assert!(
            (first.dx - 10.0).abs() < 1e-3 && first.dy.abs() < 1e-3,
            "zero radians is three o'clock, got {first:?}"
        );
        assert!(
            last.dx.abs() < 1e-3 && (last.dy - 10.0).abs() < 1e-3,
            "a positive quarter turn ends at six o'clock, got {last:?}"
        );
    }

    #[test]
    fn a_negative_sweep_runs_the_other_way() {
        let center = Offset::ZERO;
        let back = Path::arc(center, 10.0, 0.0, -FRAC_PI_2);
        let last = *endpoints(&back).last().expect("an arc ends somewhere");
        assert!(
            last.dx.abs() < 1e-3 && (last.dy + 10.0).abs() < 1e-3,
            "an anticlockwise quarter ends at twelve o'clock, got {last:?}"
        );
    }

    #[test]
    fn a_long_sweep_is_split_rather_than_approximated_in_one_go() {
        // A single cubic cannot describe much more than a quarter turn without
        // visibly sagging, so a longer arc must be several. This asserts the
        // splitting happens at all — the accuracy itself is
        // `an_arcs_endpoints_all_sit_on_its_circle`, over three quarters.
        let cubics = |sweep: f32| {
            Path::arc(Offset::ZERO, 10.0, 0.0, sweep)
                .verbs()
                .iter()
                .filter(|v| matches!(v, PathVerb::CubicTo(..)))
                .count()
        };
        assert_eq!(cubics(FRAC_PI_2), 1, "a quarter is one cubic");
        assert_eq!(cubics(PI), 2, "a half is two");
        assert_eq!(cubics(TURN), 4, "a full turn is four");
        assert_eq!(cubics(TURN * 4.0), 4, "and more than a turn is still four");
    }

    #[test]
    fn a_band_stays_between_its_two_radii() {
        let center = Offset::new(50.0, 50.0);
        let band = Path::arc_ring(center, 20.0, 6.0, -FRAC_PI_2, PI);
        for r in radii(&band, center) {
            assert!(
                (14.0 - 1e-3..=20.0 + 1e-3).contains(&r),
                "a 6-thick band on a 20 circle lives between 14 and 20, got {r}"
            );
        }
    }

    #[test]
    fn a_full_band_is_a_ring_with_no_seam() {
        // Two closed subpaths, the inner one reversed, rather than a band whose
        // ends meet at a notch of zero width. The reversal is what makes the
        // hole a hole under non-zero winding.
        let center = Offset::new(10.0, 10.0);
        let ring = Path::arc_ring(center, 20.0, 5.0, 0.0, TURN);

        let closes = ring
            .verbs()
            .iter()
            .filter(|v| matches!(v, PathVerb::Close))
            .count();
        assert_eq!(closes, 2, "an outer circle and an inner one: {ring:?}");

        let moves = ring
            .verbs()
            .iter()
            .filter(|v| matches!(v, PathVerb::MoveTo(_)))
            .count();
        assert_eq!(moves, 2, "two subpaths, so two starts: {ring:?}");

        // And nothing wanders into the hole.
        for r in radii(&ring, center) {
            assert!(r >= 15.0 - 1e-3, "inside the hole: {r}");
        }
    }

    #[test]
    fn an_over_thick_band_becomes_a_wedge_from_the_centre() {
        // The same answer `rounded_ring` gives when a border swallows its own
        // middle: there is no hole left, so it is a pie slice. Drawing it
        // inside-out instead would be a shape nobody asked for.
        let center = Offset::new(5.0, 5.0);
        let wedge = Path::arc_ring(center, 10.0, 40.0, 0.0, FRAC_PI_2);
        assert!(
            endpoints(&wedge).contains(&center),
            "a wedge is anchored at the centre: {wedge:?}"
        );
    }

    #[test]
    fn a_band_with_nothing_to_draw_is_empty_rather_than_a_stray_point() {
        // Every one of these is reachable from an animation: a spinner at rest,
        // a zero-radius layout, a thickness that has not been set yet. A stray
        // `MoveTo` would be a subpath the filler still has to consider.
        assert!(Path::arc_ring(Offset::ZERO, 10.0, 2.0, 0.0, 0.0).is_empty());
        assert!(Path::arc_ring(Offset::ZERO, 0.0, 2.0, 0.0, PI).is_empty());
        assert!(Path::arc_ring(Offset::ZERO, 10.0, 0.0, 0.0, PI).is_empty());
    }

    #[test]
    fn a_quarter_arcs_constant_is_the_one_rounded_rect_hard_codes() {
        // `rounded_rect` uses 0.552_284_8 for its corners; the general form is
        // 4/3·tan(a/4). If these ever disagree, one of the two is wrong and the
        // corners of every button in the framework are the evidence.
        let general = 4.0 / 3.0 * (FRAC_PI_2 / 4.0_f32).tan();
        assert!(
            (general - 0.552_284_8).abs() < 1e-6,
            "the arc constant drifted from the corner constant: {general}"
        );
    }

    #[test]
    fn fitting_a_viewbox_with_no_area_leaves_the_path_alone() {
        let path = Path::rect(Rect::new(0.0, 0.0, 4.0, 4.0));
        assert_eq!(
            path.fitted(Rect::ZERO, Rect::new(0.0, 0.0, 20.0, 20.0)),
            path,
            "a zero viewbox has no scale factor, and dividing by it is worse"
        );
    }
}
