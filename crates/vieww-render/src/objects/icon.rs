use std::cell::RefCell;

use vieww_foundation::{Color, Constraints, IconData, Offset, Path, PathVerb, Rect, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject, Role, Semantics};

/// The thinnest a stroked icon's pen is allowed to get, in logical points.
///
/// One physical pixel at 1x. Below this a line does not read as thinner, it
/// reads as a paler colour, which turns a small icon into a lighter grey than
/// the text it sits beside.
const MIN_STROKE: f32 = 1.0;

/// Added before truncating, so a pen rounds *up* once it is a quarter of a
/// pixel over.
///
/// `round` — which this replaced — is the wrong rule for a pen, and the
/// asymmetry is the whole point. Rounding a 1.4-pixel pen down to 1.0 throws
/// away 29% of the ink in an icon; rounding it up to 2.0 adds 43% to a shape
/// whose *silhouette* is unchanged, because a stroke grows about its
/// centreline. Ink that is not there cannot be read at all, and ink that is a
/// little heavy still reads as the shape it is — so the two errors are not
/// worth the same and the rule should not treat them as if they were.
///
/// This is what "the icons are thin and not visible" was: the studio's 21-point
/// activity bar wanted a 1.4-pixel pen and got a 1.0-pixel one, at which width
/// every antialiased corner and diagonal falls below half coverage and drops
/// out of the picture entirely.
///
/// 0.75 puts the switch at a fractional part of 0.25 — heavier than "nearest",
/// but not so eager that a pen genuinely close to `n` is pushed to `n + 1`.
const PEN_ROUND_UP: f32 = 0.75;

/// How far apart two coordinates may be and still count as the same line.
///
/// In logical points, against geometry that has already been fitted, so this is
/// float noise from one multiply and nothing else — an icon set's real
/// coordinates are a tenth of a design unit apart at the closest.
const AXIS_TOLERANCE: f32 = 1e-3;

/// Fills an icon's shape, scaled into its box.
///
/// # Square by request, not by force
///
/// It asks for a square of `size` and takes whatever the constraints allow of
/// that, so an icon in a tight box that is not square ends up the shape of the
/// box — and the *shape* inside it is still fitted uniformly and centred, so it
/// is never stretched. Squashing an icon is the one thing a viewbox exists to
/// prevent.
#[derive(Debug, Clone)]
pub struct RenderIcon {
    pub icon: IconData,
    pub size: f32,
    pub color: Color,
    /// `Some(width)` draws the path as a line of that width instead of filling
    /// it. In the icon's **viewbox units**, scaled into the box like the path
    /// — see [`Icon::stroke`](vieww_widget::Icon::stroke).
    pub stroke: Option<f32>,
    pub label: Option<String>,
    /// The expanded pen from the last stroked paint, with the key it answered.
    ///
    /// Expanding a pen is the expensive half of painting a stroked icon — the
    /// outline carries twenty times the verbs of the centreline it replaced —
    /// and a damaged frame repaints an icon that did not change, so the walk
    /// runs again for the same answer. The cache holds that answer. `paint`
    /// takes `&self`, so it lives in a `RefCell` — the same interior
    /// mutability `RenderSlider::width` and `RenderViewport::reported`
    /// already use — and the key is everything the outline is a *function*
    /// of: the pen, the box, and the grid. A paint whose key matches reuses;
    /// one whose does not recomputes and replaces. Correctness rests on the
    /// key alone, never on when the cache was written or adopted.
    outline: RefCell<Option<(OutlineKey, Path)>>,
}

/// Everything the expanded pen of a stroked icon is a function of.
///
/// The pen in logical points is already the rounded one — the phase and the
/// inset are derived from it and the ratio, so they need no separate slot in
/// the key. The bounds are the box the paint was fitted into, before the
/// inset and snap, because those are derived from the pen too. Fewer fields,
/// each an input rather than an intermediate, is a key that cannot disagree
/// with the computation it guards.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OutlineKey {
    pen: f32,
    bounds: Rect,
    dpr: f32,
}

/// Equality ignores the cached outline.
///
/// The same rule `RenderText` states for its shaped paragraph: the cache is
/// *derived* from the fields, so including it would make a freshly-built icon
/// unequal to the identical one that has already painted, and every rebuild
/// would relayout an icon that never moved. `layout_differs` already ignores
/// the colour and the label on purpose; this is the other half of that
/// decision for the one field that cannot affect geometry.
impl PartialEq for RenderIcon {
    fn eq(&self, other: &Self) -> bool {
        self.icon == other.icon
            && self.size == other.size
            && self.color == other.color
            && self.stroke == other.stroke
            && self.label == other.label
    }
}

impl RenderIcon {
    #[must_use]
    pub fn new(icon: IconData, size: f32, color: Color) -> Self {
        Self {
            icon,
            size,
            color,
            stroke: None,
            label: None,
            outline: RefCell::new(None),
        }
    }

    /// Draw the path as a line of `width` rather than filling it.
    #[must_use]
    pub const fn stroke(mut self, width: f32) -> Self {
        self.stroke = Some(width);
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The outline this paint should fill: the cached one when the key it was
    /// built for matches this paint, a fresh expansion when it does not.
    ///
    /// `shape` is the hinted, fitted geometry — consumed only on the miss,
    /// which is the only branch that needs it.
    fn cached_outline(&self, shape: Path, pen: f32, bounds: Rect, dpr: f32) -> Path {
        let key = OutlineKey { pen, bounds, dpr };
        if let Some((held, path)) = self.outline.borrow().as_ref() {
            if *held == key {
                return path.clone();
            }
        }
        let outline = shape.stroke_outline(pen);
        *self.outline.borrow_mut() = Some((key, outline.clone()));
        outline
    }
}

impl RenderObject for RenderIcon {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(Size::square(self.size))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.color.is_transparent() {
            return;
        }
        let bounds = ctx.bounds();
        let Some(grid_width) = self.stroke else {
            // **A filled icon is snapped to the physical grid too.** The stroked
            // path below has always aligned its stems; a filled one landed
            // wherever `fitted` centred it, which at a fractional display scale
            // is a half-pixel off the grid on every edge — the same softness
            // the stroked set was fixed for, arriving by a different route.
            // The snap moves the *box*, not the shape: the fit inside it is
            // unchanged, and the move is under half a logical pixel.
            let dpr = ctx.device_pixel_ratio().max(1.0);
            let snap = |value: f32| (value * dpr).round() / dpr;
            let snapped = bounds.translate(Offset::new(
                snap(bounds.left) - bounds.left,
                snap(bounds.top) - bounds.top,
            ));
            let shape = self.icon.fitted(snapped);
            // **The stems get hinted too, same as the stroked branch.** The
            // box snap above only moves the shape's *origin*; every
            // coordinate inside it still lands wherever the icon's own
            // viewbox units and `fitted`'s scale put it, which is off-grid
            // more often than not — a filled icon's straight edges were the
            // one shape in the tree softened by exactly the "smeared" failure
            // mode `hint_stems` exists to fix for a stroked one. Phase `0.0`:
            // a fill's edge wants to sit on a pixel *boundary*, not the
            // pixel-centre phase an odd stroked pen wants. Diagonals and
            // curves are left untouched, as `hint_stems` always leaves them.
            let shape = hint_stems(&shape, dpr, 0.0);
            ctx.canvas().fill_path(&shape, self.color.into());
            return;
        };

        // The pen is given in viewbox units and travels with the path, so it
        // is scaled by whatever `fitted` scales the geometry by — the smaller
        // of the two ratios, since the fit is uniform.
        let viewbox = self.icon.viewbox();
        let scale = (bounds.width() / viewbox.width()).min(bounds.height() / viewbox.height());
        // **Rounded to a whole *physical* pixel, and then placed on the grid
        // that width wants.** This is the difference between an icon set that
        // looks drawn and one that looks photocopied.
        //
        // A 24-unit icon at 21 points scales by 0.875, so a 1.6-unit pen comes
        // out 1.4 logical pixels wide. There is no way to put 1.4 logical
        // pixels of ink on a pixel grid: the rasteriser spreads it over two rows
        // at roughly 70% each, and *every* edge in the icon gets the same
        // treatment, because 0.875 puts every coordinate off the grid too. The
        // result is a shape with no crisp edge anywhere — which is exactly what
        // "smudgy" means, and why it sharpened under the pointer, where the
        // icon is painted in a brighter colour that hides the same softness.
        //
        // So: round the pen to a whole number of pixels, then shift the box by
        // the half-pixel that puts the pen's *centreline* where a whole-pixel
        // pen needs it — on a pixel centre when the pen is odd, on a boundary
        // when it is even. Every axis-aligned run in the icon then lands on
        // exactly the pixels it covers.
        //
        // **The grid is the surface's, not the tree's.** This used to snap in
        // logical points, which is exact at 1x and merely *wrong* at 2x and 3x —
        // a whole logical pixel is two or three physical pixels there, so the
        // snap was coarser than it had to be — and on a **fractional** display
        // scale it was not even wrong in a consistent direction: a coordinate
        // rounded to the logical grid lands on the fractional-physical one
        // off-grid almost always, so every stroke edge straddled two physical
        // pixels and the whole set came out smeared. The ratio is what makes
        // "round to the nearest pixel" a statement about pixels that exist; see
        // [`PaintCtx::device_pixel_ratio`].
        let dpr = ctx.device_pixel_ratio().max(1.0);
        // Pen in physical pixels, then back into the logical points the canvas
        // speaks. `MIN_STROKE` is a logical floor, so it scales with the ratio:
        // one physical pixel at 1x is its definition.
        let pen_physical = (grid_width * scale * dpr + PEN_ROUND_UP)
            .floor()
            .max(MIN_STROKE * dpr);
        let pen = pen_physical / dpr;
        // Inset by half the pen, so a stroke that straddles its path still ends
        // up inside the box: without it two icons a hair apart in a strip touch.
        let box_for_shape = bounds.inflate(-pen / 2.0);
        // The phase the pen's centreline wants, in logical points: an odd pen
        // sits on a physical pixel's centre (half a physical pixel, divided by
        // the ratio), an even one on a boundary.
        let odd = (pen_physical as i64) % 2 == 1;
        let phase = if odd { 0.5 / dpr } else { 0.0 };
        // Snap to the phase in *physical* space and report the delta to apply.
        let snap = |value: f32| {
            let physical = value * dpr - phase * dpr;
            (physical.round() + phase * dpr) / dpr - value
        };
        let shape = self.icon.fitted(box_for_shape.translate(Offset::new(
            snap(box_for_shape.left),
            snap(box_for_shape.top),
        )));
        // **The stems are snapped too, and this is the second half of the
        // smear.** Rounding the pen and the box puts the *outline* of the shape
        // on the grid, but every coordinate inside the path still lands
        // wherever `scale` put it — and `scale` is a ratio like 0.875, so a
        // path coordinate at `3.4` in a 24-unit grid lands at `2.975`, a
        // fraction of the way between two physical pixels. A horizontal run
        // there is antialiased along its whole length at partial coverage on
        // two rows instead of full coverage on one, which is what "smeared"
        // means and why it sharpened under the pointer, where a brighter
        // colour lifts both half-covered rows far enough to read as a line.
        //
        // So the fitted geometry is hinted — but **only its stems**. See
        // [`hint_stems`] for why snapping everything, which is what this used
        // to do, traded the smear for a worse artefact.
        let shape = hint_stems(&shape, dpr, phase);
        // **The pen is expanded into a fill, not handed to the rasteriser as a
        // stroke.** A stroke is a rasterisation problem the backends do not
        // agree on: a thin pen whose two edges straddle pixel boundaries is the
        // hard case for a coverage antialiaser, and the same 2-point pen drew
        // crisp on one backend and a wide, soft, double-weighted band on
        // another — the artefact this comment's neighbours were hunting all
        // along. A fill of an ordinary shape has no such failure mode, so the
        // hinted, snapped geometry is expanded into its own outline
        // (round caps and joins, a union of segment quads and corner discs
        // under the nonzero rule) and *filled*. The pen maths above — the
        // rounding, the phase, the grid — still governs where that outline's
        // edges land; only the rasteriser's least reliable primitive is
        // retired. `Path::stroke_outline` documents the construction.
        //
        // The expansion goes through [`Self::cached_outline`]: a damaged frame
        // repaints an icon that did not change, and the outline carries twenty
        // times the verbs of the centreline it replaced — re-expanding it per
        // paint would spend the fix's geometry savings on re-allocation
        // instead. The key is (pen, bounds, dpr), everything the outline is a
        // function of besides the icon itself, which is this object's own
        // identity and cannot change under a paint.
        let outline = self.cached_outline(shape, pen, bounds, dpr);
        ctx.canvas().fill_path(&outline, self.color.into());
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The *box*, not the shape. Hit testing the outline would make the gap
        // between a tick's two strokes a hole that a tap falls through, and a
        // control's icon is never the thing that should be absorbing taps
        // anyway — the control around it is.
        !self.color.is_transparent()
    }

    fn semantics(&self) -> Option<Semantics> {
        // Only when it was given a name. An unlabelled icon is decoration on
        // something that already announces itself, and a screen reader that
        // stops on it says the same thing twice.
        self.label
            .as_ref()
            .map(|label| Semantics::new(Role::Label).with_label(label.clone()))
    }

    /// Square, at the declared size. A leaf that knows exactly what it wants.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        _query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        Some(self.size)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // The colour and the label change nothing geometric; the size and the
        // shape do. `layout_differs_by_eq` would relayout on a recolour.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>().is_none_or(|other| {
            (self.size - other.size).abs() > f32::EPSILON || self.icon != other.icon
        })
    }

    fn debug_name(&self) -> &'static str {
        "RenderIcon"
    }
}

/// Move a fitted path's **stems** onto the pen's phase grid, and nothing else.
///
/// The companion of the pen-width rounding in [`RenderIcon::paint`]. A
/// whole-pixel pen whose centreline runs between physical pixels is still
/// antialiased on both edges, so the horizontal and vertical runs the pen
/// travels along have to sit on the grid the pen was rounded to — pixel centres
/// for an odd pen, boundaries for an even one. `phase` is that offset in
/// logical points and `dpr` the surface's ratio; together they are the same
/// grid the box origin was snapped to, so anchors and outline cannot disagree.
///
/// # Why only the stems, and what snapping everything cost
///
/// This used to snap every `MoveTo`/`LineTo` endpoint and every curve anchor to
/// the nearest grid position in **both** axes, which is a strictly worse
/// picture and was the second bug hiding inside "the icons look smeared":
///
/// * **A diagonal gains nothing.** A line at 45° is antialiased along its whole
///   length wherever its endpoints are — there is no grid position that makes
///   it crisp. Moving both endpoints up to half a pixel each therefore bought
///   no sharpness at all and changed the *angle*, which is why a set of
///   chevrons came out visibly ragged and a warning triangle's two sides came
///   out at different slopes.
/// * **A curve is actively damaged.** A circle at 21 points is a ring seven
///   pixels across drawn as four cubics. Snapping each anchor independently
///   moves four points on that ring by up to half a pixel in each axis, which
///   is a percent or two of the radius applied unevenly — the ring stops being
///   round, and the thin places in it fall below half coverage and break the
///   line up. A magnifier that renders as a dotted lozenge is that, exactly.
///
/// So a point moves only in the axis some **axis-aligned line segment through
/// it** asks for: the `y` of a horizontal run, the `x` of a vertical one, both
/// at a corner where the two meet, and neither for a point that only diagonals
/// and curves pass through. That is what a font hinter does to a stem, and it
/// is the whole of the sharpness that is available to be won: the long straight
/// runs — a folder's lid, a panel's divider, a slider's track, the bar of a
/// warning's exclamation mark — snap onto single pixel rows and columns, and
/// every curve and diagonal keeps the shape it was drawn with.
///
/// A `Close` counts as a segment back to where the subpath started, so a
/// rectangle written as three lines and a close gets its fourth side hinted
/// like the other three.
///
/// Control points ride along with whatever delta their endpoint took, which
/// keeps a curve's shape exactly and only moves its anchor: snapping a control
/// point in its own right would put kinks in arcs, because it is a
/// direction-and-distance handle, not a place the ink goes.
///
/// # One scratch buffer
///
/// This runs on every paint of every stroked icon — a shell with a chrome set
/// has a few dozen — so it walks the verbs twice against a single
/// `Vec<Anchor>` rather than keeping an anchor list, a segment list and two
/// flag vectors. The flags are decided *during* the first walk, because the
/// only segment that can reach backwards is a `Close` and its target index is
/// known the moment the subpath opens.
fn hint_stems(path: &Path, dpr: f32, phase: f32) -> Path {
    /// One point the pen passes through, with the axes some straight run
    /// through it has claimed.
    #[derive(Clone, Copy)]
    struct Anchor {
        point: Offset,
        snap_x: bool,
        snap_y: bool,
    }

    let verbs = path.verbs();
    let mut anchors: Vec<Anchor> = Vec::with_capacity(verbs.len());
    let mut current: Option<usize> = None;
    let mut subpath_start: Option<usize> = None;

    // Claim both ends of a straight run in whichever axis it is constant in. A
    // run that is neither horizontal nor vertical claims nothing, which is the
    // whole point — see the note above.
    fn claim(anchors: &mut [Anchor], from: usize, to: usize) {
        let (a, b) = (anchors[from].point, anchors[to].point);
        if (a.dy - b.dy).abs() <= AXIS_TOLERANCE {
            anchors[from].snap_y = true;
            anchors[to].snap_y = true;
        }
        if (a.dx - b.dx).abs() <= AXIS_TOLERANCE {
            anchors[from].snap_x = true;
            anchors[to].snap_x = true;
        }
    }

    let push = |anchors: &mut Vec<Anchor>, point: Offset| {
        anchors.push(Anchor {
            point,
            snap_x: false,
            snap_y: false,
        });
        anchors.len() - 1
    };

    for verb in verbs {
        match *verb {
            PathVerb::MoveTo(point) => {
                let index = push(&mut anchors, point);
                current = Some(index);
                subpath_start = Some(index);
            }
            PathVerb::LineTo(point) => {
                let index = push(&mut anchors, point);
                if let Some(from) = current {
                    claim(&mut anchors, from, index);
                }
                current = Some(index);
            }
            PathVerb::CubicTo(_, _, end) => {
                current = Some(push(&mut anchors, end));
            }
            PathVerb::Close => {
                // The implicit segment home. A path that closes without having
                // moved is malformed; matching `None` records no segment rather
                // than guessing at one.
                if let (Some(from), Some(start)) = (current, subpath_start) {
                    if from != start {
                        claim(&mut anchors, from, start);
                    }
                    current = Some(start);
                }
            }
        }
    }

    let snap = |value: f32| ((value * dpr - phase * dpr).round() + phase * dpr) / dpr;
    let hinted_anchor = |anchor: Anchor| {
        Offset::new(
            if anchor.snap_x {
                snap(anchor.point.dx)
            } else {
                anchor.point.dx
            },
            if anchor.snap_y {
                snap(anchor.point.dy)
            } else {
                anchor.point.dy
            },
        )
    };

    let mut hinted = Path::new();
    let mut next = 0usize;
    for verb in verbs {
        match *verb {
            PathVerb::MoveTo(_) => {
                hinted.move_to(hinted_anchor(anchors[next]));
                next += 1;
            }
            PathVerb::LineTo(_) => {
                hinted.line_to(hinted_anchor(anchors[next]));
                next += 1;
            }
            PathVerb::CubicTo(c1, c2, end) => {
                let moved = hinted_anchor(anchors[next]);
                let delta = moved - end;
                hinted.cubic_to(c1 + delta, c2 + delta, moved);
                next += 1;
            }
            PathVerb::Close => {
                hinted.close();
            }
        }
    }
    hinted
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Path, Rect};
    use vieww_paint::{Command, Scene};

    use super::*;

    fn icon() -> IconData {
        IconData::square24(Path::rect(Rect::new(6.0, 6.0, 18.0, 18.0)))
    }

    /// One vertical stem. The pen a paint computed reads straight off the
    /// ink: the outline of a stem is a quad exactly one pen wide, where a
    /// square's outline hides the pen behind the box the fit inset and the
    /// outline grew back.
    fn line() -> IconData {
        let mut path = Path::new();
        path.move_to(Offset::new(12.0, 2.0));
        path.line_to(Offset::new(12.0, 22.0));
        IconData::square24(path)
    }

    fn painted(object: &RenderIcon, origin: Offset, size: Size) -> Scene {
        let mut scene = Scene::new();
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin,
            size,
            dpr: 1.0,
        };
        object.paint(&mut ctx);
        scene
    }

    fn painted_at_ratio(object: &RenderIcon, origin: Offset, size: Size, dpr: f32) -> Scene {
        let mut scene = Scene::new();
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin,
            size,
            dpr,
        };
        object.paint(&mut ctx);
        scene
    }

    #[test]
    fn the_shape_is_placed_where_the_object_is() {
        let object = RenderIcon::new(icon(), 24.0, Color::BLACK);
        let scene = painted(&object, Offset::new(100.0, 50.0), Size::square(24.0));

        let [Command::FillPath { path, .. }] = scene.commands() else {
            panic!("one filled path, got {:?}", scene.commands());
        };
        // The inner square starts a quarter of the way into the viewbox.
        assert_eq!(path.bounds(), Rect::new(106.0, 56.0, 118.0, 68.0));
    }

    #[test]
    fn a_transparent_icon_records_nothing() {
        let object = RenderIcon::new(icon(), 24.0, Color::TRANSPARENT);
        assert!(painted(&object, Offset::ZERO, Size::square(24.0)).is_empty());
    }

    #[test]
    fn recolouring_does_not_ask_for_a_relayout() {
        let object = RenderIcon::new(icon(), 24.0, Color::BLACK);
        let recoloured = RenderIcon::new(icon(), 24.0, Color::RED);
        let resized = RenderIcon::new(icon(), 16.0, Color::BLACK);

        assert!(!object.layout_differs(&recoloured));
        assert!(object.layout_differs(&resized));
    }

    #[test]
    fn only_a_named_icon_reaches_a_screen_reader() {
        assert!(RenderIcon::new(icon(), 24.0, Color::BLACK)
            .semantics()
            .is_none());
        assert_eq!(
            RenderIcon::new(icon(), 24.0, Color::BLACK)
                .label("Dismiss")
                .semantics()
                .and_then(|declared| declared.label),
            Some("Dismiss".to_owned())
        );
    }

    /// The regression behind "the icons are smeared until I hover them": at a
    /// fractional display scale a pen rounded in *logical* points is a
    /// fractional number of physical pixels, and every stroke edge then
    /// straddles two physical pixels. The pen has to be a whole number of
    /// **physical** pixels, and the box's origin has to sit where that pen
    /// wants its centreline — pixel boundary for an even pen, pixel centre for
    /// an odd one.
    ///
    /// The pen is no longer emitted as a stroke: it is baked into the outline
    /// that is filled (see `RenderIcon::paint`). What the test can still see
    /// is the geometry the pen produced — the outline of a full-bleed square
    /// is the square grown by half the pen, so the width reads straight off
    /// the bounds, and the grid the centreline was snapped to shows up as the
    /// outline's edges sitting a whole half-pen from it.
    #[test]
    fn a_fractional_scale_snaps_the_pen_to_whole_physical_pixels() {
        // One stem, so the ink is a quad exactly one pen wide and the snap
        // puts its edges on the display's own grid.
        let object = RenderIcon::new(line(), 21.0, Color::BLACK).stroke(1.6);
        let scene = painted_at_ratio(&object, Offset::new(13.5, 40.0), Size::square(21.0), 1.25);

        let [Command::FillPath { path, .. }] = scene.commands() else {
            panic!("one filled outline, got {:?}", scene.commands());
        };
        let bounds = path.bounds();
        // The pen: 2 physical pixels at 1.25x is 1.6 logical, and a stem's
        // outline is exactly the pen wide.
        assert!(
            (bounds.width() - 1.6).abs() < 0.05,
            "stem ink {} wide, not the 1.6 a 2-physical-pixel pen is at 1.25x",
            bounds.width()
        );

        // Even pen, so the stem's *edges* land on physical pixel boundaries:
        // every pixel under the stem is fully inked or untouched, which is
        // the crispness the snap exists to buy.
        for edge in [bounds.left, bounds.right] {
            let physical = edge * 1.25;
            assert!(
                (physical - physical.round()).abs() < 1e-3,
                "stem edge {edge} is {physical} physical pixels from the origin"
            );
        }
    }

    /// A pen is rounded to a whole physical pixel, but **upwards** once it is a
    /// quarter of a pixel over. This is the fix for "the icons are thin and not
    /// visible": nearest-rounding took the studio's 1.4-pixel activity-bar pen
    /// down to 1.0, and a one-pixel pen has no antialiasing headroom left — every
    /// corner and diagonal on it sits below half coverage and disappears.
    #[test]
    fn a_pen_is_rounded_up_rather_than_thinned_away() {
        let object = RenderIcon::new(line(), 21.0, Color::BLACK).stroke(1.6);
        let scene = painted_at_ratio(&object, Offset::new(13.5, 40.0), Size::square(21.0), 1.0);

        let [Command::FillPath { path, .. }] = scene.commands() else {
            panic!("one filled outline, got {:?}", scene.commands());
        };
        // 1.6 units in a 21/24 fit is 1.4 physical pixels. Nearest gives 1.0 and
        // throws away 29% of the ink; this rounds up to 2. A stem's outline is
        // exactly the pen wide, so the width is the pen.
        assert!((path.bounds().width() - 2.0).abs() < 0.05, "stem ink width");
    }

    /// A pen genuinely close to a whole pixel is *not* pushed up a size — the
    /// bias is a quarter of a pixel, not a blanket `ceil`, which would double
    /// every hairline in the tree.
    #[test]
    fn a_pen_already_near_a_whole_pixel_stays_there() {
        // 1.15 units in a 24/24 fit: 1.15 physical pixels, a fractional part
        // under the quarter-pixel switch.
        let object = RenderIcon::new(line(), 24.0, Color::BLACK).stroke(1.15);
        let scene = painted_at_ratio(&object, Offset::ZERO, Size::square(24.0), 1.0);

        let [Command::FillPath { path, .. }] = scene.commands() else {
            panic!("one filled outline, got {:?}", scene.commands());
        };
        // The stem's outline is the pen: 1, not 2.
        assert!((path.bounds().width() - 1.0).abs() < 0.05, "stem ink width");
    }

    /// A pen is never thinner than [`MIN_STROKE`], which is a **logical** floor
    /// and so is the same apparent weight on every display. A physical-pixel
    /// floor would let an icon get visually thinner the denser the screen got,
    /// which is the opposite of what a floor is for.
    #[test]
    fn a_tiny_icon_still_gets_the_minimum_pen() {
        let object = RenderIcon::new(line(), 4.0, Color::BLACK).stroke(0.2);
        for dpr in [1.0, 2.0, 3.0] {
            let scene = painted_at_ratio(&object, Offset::ZERO, Size::square(4.0), dpr);
            let [Command::FillPath { path, .. }] = scene.commands() else {
                panic!("one filled outline, got {:?}", scene.commands());
            };
            // A stem's outline is exactly the pen wide, and the floor holds
            // the pen at one logical point on every display.
            assert!(
                (path.bounds().width() - MIN_STROKE).abs() < 0.05,
                "at {dpr}x: stem ink {} wide, not the {} floor",
                path.bounds().width(),
                MIN_STROKE
            );
        }
    }

    /// A vertical run lands on the pen's column and keeps its own `y`s; a
    /// horizontal run lands on the pen's row and keeps its own `x`s. This is
    /// the half of the fix that removes the smear proper — a stem whose
    /// centreline runs between two pixel columns is drawn at partial coverage
    /// on both of them, which is a pale wide line where a crisp narrow one
    /// belongs.
    ///
    /// With the pen baked into a filled outline, the stem's ink is the quad
    /// between its two offset edges — so the property to assert is that those
    /// edges are whole: an even pen centred on a boundary puts both edges on
    /// boundaries too, and every pixel the stem touches is then either fully
    /// inked or untouched.
    #[test]
    fn a_stems_own_axis_snaps_and_the_other_is_left_alone() {
        // A right angle: down, then across. Both runs are stems; the corner
        // they share is the one point that snaps in both axes.
        let mut path = Path::new();
        path.move_to(Offset::new(3.4, 5.4));
        path.line_to(Offset::new(3.4, 19.6));
        path.line_to(Offset::new(17.3, 19.6));
        let object = RenderIcon::new(IconData::square24(path), 24.0, Color::BLACK).stroke(1.6);
        // An even pen at 1:1 wants its centreline on pixel boundaries, so the
        // grid here is the whole numbers.
        let scene = painted_at_ratio(&object, Offset::ZERO, Size::square(24.0), 1.0);

        let [Command::FillPath { path, .. }] = scene.commands() else {
            panic!("one filled outline, got {:?}", scene.commands());
        };
        let bounds = path.bounds();
        let whole = |value: f32| (value - value.round()).abs() < 1e-3;

        // The fitted, hinted geometry: the fit scaled 24 into the 22-point
        // pen-inset box, so the runs sit at (4.0, 5.95)\u{2192}(4.0, 19.0)\u{2192}(16.86, 19.0)
        // — the stems' own axes snapped to whole points, the free ends left
        // alone. The outline is that grown by the pen's half and capped with
        // discs, so:
        //
        // * the vertical run's left edge is centreline 4.0 minus 1 — whole,
        //   which is the whole point of the snap: both edges of the stem's
        //   ink are on pixel boundaries;
        // * the horizontal run's bottom edge is centreline 19.0 plus 1 —
        //   whole for the same reason;
        // * the free ends (top, right) keep their fractional positions, which
        //   is the other half of the design: caps are round and antialiased
        //   wherever they sit.
        assert!(whole(bounds.left), "the vertical's left edge: {bounds:?}");
        assert!(
            whole(bounds.bottom),
            "the horizontal's bottom edge: {bounds:?}"
        );
        assert!(
            !whole(bounds.top) || !whole(bounds.right),
            "free ends moved: {bounds:?}"
        );
        // The outline is the pen wider than the runs it covers, and no wider:
        // 12.86 of run plus 2 of pen across, 13.05 plus 2 down.
        assert!((bounds.width() - 14.86).abs() < 0.05, "{bounds:?}");
        assert!((bounds.height() - 15.05).abs() < 0.05, "{bounds:?}");
    }

    /// **A diagonal is left exactly where it was.** Snapping its endpoints wins
    /// no sharpness — a 45° line is antialiased along its whole length wherever
    /// it sits — and it changes the angle, which is what made a set of chevrons
    /// come out ragged and a triangle come out with two different slopes.
    #[test]
    fn a_diagonal_is_not_hinted_at_all() {
        let (from, to) = (Offset::new(9.2, 7.4), Offset::new(3.9, 12.0));
        let mut path = Path::new();
        path.move_to(from);
        path.line_to(to);
        let hinted = hint_stems(&path, 1.0, 0.0);

        assert_eq!(
            hinted.verbs(),
            &[PathVerb::MoveTo(from), PathVerb::LineTo(to)],
            "a diagonal has no stem to hint"
        );
    }

    /// **A curve is left exactly where it was**, control points and anchor
    /// alike. A ring seven pixels across is four cubics; moving each anchor
    /// half a pixel in each axis is a percent or two of the radius applied
    /// unevenly, and the ring stops being round and starts breaking up — which
    /// is what turned the magnifier into a dotted lozenge.
    #[test]
    fn a_curve_is_not_hinted_at_all() {
        let (c1, c2, end) = (
            Offset::new(9.0, 7.2),
            Offset::new(12.4, 9.9),
            Offset::new(14.8, 7.4),
        );
        let start = Offset::new(3.4, 5.4);
        let mut path = Path::new();
        path.move_to(start);
        path.cubic_to(c1, c2, end);
        let hinted = hint_stems(&path, 1.0, 0.5);

        assert_eq!(
            hinted.verbs(),
            &[PathVerb::MoveTo(start), PathVerb::CubicTo(c1, c2, end)],
        );
    }

    /// A curve whose anchor is *shared with a stem* moves with it, and its
    /// control points ride the same delta so the curve keeps its shape and only
    /// its anchor travels. Snapping a control point in its own right would
    /// flatten the arc: it says "head this way, this hard", it is not a place
    /// the ink reaches.
    #[test]
    fn a_curves_controls_ride_a_shared_anchors_delta() {
        // A vertical stem into a curve. The stem's `x` snaps, dragging the
        // curve's starting anchor with it, and nothing else moves.
        let corner = Offset::new(3.4, 12.0);
        let (c1, c2, end) = (
            Offset::new(6.0, 12.0),
            Offset::new(9.0, 14.0),
            Offset::new(11.3, 16.7),
        );
        let mut path = Path::new();
        path.move_to(Offset::new(3.4, 4.0));
        path.line_to(corner);
        path.cubic_to(c1, c2, end);
        let hinted = hint_stems(&path, 1.0, 0.0);

        let verbs = hinted.verbs().to_vec();
        let [PathVerb::MoveTo(top), PathVerb::LineTo(joint), PathVerb::CubicTo(h1, h2, tail)] =
            verbs.as_slice()
        else {
            panic!("three verbs, got {verbs:?}");
        };
        // The stem snapped in `x` only, at both ends.
        assert_eq!(*top, Offset::new(3.0, 4.0));
        assert_eq!(*joint, Offset::new(3.0, 12.0));
        // The curve's own anchor is not on a stem, so it did not move — and
        // with no delta, neither did its controls.
        assert_eq!(*tail, end);
        assert_eq!(*h1, c1);
        assert_eq!(*h2, c2);
    }

    /// `Close` is a segment too. A rectangle written as three lines and a close
    /// has four sides, and the fourth is hinted like the other three — without
    /// this its two endpoints would each be hinted in one axis only and the
    /// closing side would come out crooked.
    #[test]
    fn a_closing_segment_counts_as_a_stem() {
        let mut path = Path::new();
        path.move_to(Offset::new(3.4, 4.6));
        path.line_to(Offset::new(20.2, 4.6));
        path.line_to(Offset::new(20.2, 19.4));
        path.line_to(Offset::new(3.4, 19.4));
        path.close();
        let hinted = hint_stems(&path, 1.0, 0.0);

        let points: Vec<Offset> = hinted
            .verbs()
            .iter()
            .filter_map(|verb| match *verb {
                PathVerb::MoveTo(point) | PathVerb::LineTo(point) => Some(point),
                _ => None,
            })
            .collect();
        // The closing side runs from the last point back to the first, so the
        // first point's `x` had to snap for that side to be vertical.
        assert_eq!(
            points,
            vec![
                Offset::new(3.0, 5.0),
                Offset::new(20.0, 5.0),
                Offset::new(20.0, 19.0),
                Offset::new(3.0, 19.0),
            ]
        );
    }
}
