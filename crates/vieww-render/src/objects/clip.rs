use vieww_foundation::{Constraints, Offset, Path, Rect, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// The shape a [`RenderClip`] cuts its child down to.
///
/// Resolved against the object's own bounds at paint time rather than being
/// given as absolute geometry, so one of these describes "a circle" without
/// knowing how large layout will make it.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipShape {
    /// The bounds themselves — a plain rectangle.
    Rect,
    /// Rounded corners, in logical pixels.
    RRect { radius: f32 },
    /// The largest circle that fits, centred.
    ///
    /// A separate variant rather than an `RRect` with a huge radius because a
    /// non-square box should give an *ellipse* here, and clamping a radius
    /// gives a stadium instead. Avatars are the whole reason this type exists.
    Oval,
    /// An arbitrary shape, in the child's coordinate space.
    Path(Path),
}

impl ClipShape {
    /// This shape as a path in `bounds`, or `None` for a plain rectangle.
    ///
    /// `None` rather than `Some(Path::rect(..))` so the common case stays on
    /// the canvas's rectangle fast path, where a clip costs a bounding box and
    /// no shape at all.
    #[must_use]
    pub fn to_path(&self, bounds: Rect) -> Option<Path> {
        match self {
            Self::Rect => None,
            Self::RRect { radius } if *radius <= 0.0 => None,
            Self::RRect { radius } => Some(Path::rounded_rect(bounds, *radius)),
            Self::Oval => Some(oval(bounds)),
            // Translated into `bounds`, not returned as-is.
            //
            // Every other arm here *builds* its shape from `bounds`, so it is
            // positioned where the widget is. A custom path is authored in the
            // widget's own space, with its origin at (0, 0) — returning it
            // unchanged pinned it to the top-left of the whole surface, so a
            // `Clip::path` anywhere below the first screenful clipped its
            // child away entirely. `ShapeMorph` rendered as an empty panel for
            // exactly this reason.
            Self::Path(path) => Some(path.transformed(vieww_foundation::Transform::translate(
                vieww_foundation::Offset::new(bounds.left, bounds.top),
            ))),
        }
    }
}

/// The ellipse inscribed in `bounds`, as four cubic arcs.
///
/// The magic constant is the standard circular approximation: a cubic Bézier
/// whose control points sit `k · r` along the tangent matches a quarter circle
/// to within about one part in a thousand, which is well inside a pixel at any
/// size a user interface uses.
fn oval(bounds: Rect) -> Path {
    const K: f32 = 0.552_284_8;

    let (cx, cy) = (
        (bounds.left + bounds.right) / 2.0,
        (bounds.top + bounds.bottom) / 2.0,
    );
    let (rx, ry) = (bounds.width() / 2.0, bounds.height() / 2.0);
    let (ox, oy) = (rx * K, ry * K);

    let mut path = Path::new();
    path.move_to(Offset::new(cx, bounds.top));
    path.cubic_to(
        Offset::new(cx + ox, bounds.top),
        Offset::new(bounds.right, cy - oy),
        Offset::new(bounds.right, cy),
    );
    path.cubic_to(
        Offset::new(bounds.right, cy + oy),
        Offset::new(cx + ox, bounds.bottom),
        Offset::new(cx, bounds.bottom),
    );
    path.cubic_to(
        Offset::new(cx - ox, bounds.bottom),
        Offset::new(bounds.left, cy + oy),
        Offset::new(bounds.left, cy),
    );
    path.cubic_to(
        Offset::new(bounds.left, cy - oy),
        Offset::new(cx - ox, bounds.top),
        Offset::new(cx, bounds.top),
    );
    path.close();
    path
}

/// Cuts its child down to a shape.
///
/// # What this replaces
///
/// Until the canvas could clip to a curve, a rounded card was drawn by painting
/// a rounded shape *over* its content — which works only against a known
/// background, breaks the moment anything is translucent, and cannot do an
/// avatar at all. Widgets in this tree were written that way because there was
/// no alternative; there is now.
///
/// # Layout and hit testing
///
/// Layout is untouched: the child gets this object's constraints and this object
/// takes the child's size. Clipping is *paint*, and a shape that changed the
/// geometry would make every rounded corner cost a relayout when its radius
/// animated.
///
/// Hit testing follows the **bounds**, not the shape. A tap in the corner of a
/// circular avatar still reaches it. That is deliberate and it is what every
/// platform does: the alternative makes targets smaller than they look and
/// costs a point-in-path test on every pointer event, and nobody has ever
/// complained that the corner of an avatar was tappable.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderClip {
    pub shape: ClipShape,
}

impl RenderClip {
    #[must_use]
    pub const fn new(shape: ClipShape) -> Self {
        Self { shape }
    }

    /// Rounded corners.
    #[must_use]
    pub const fn rounded(radius: f32) -> Self {
        Self::new(ClipShape::RRect { radius })
    }

    /// The inscribed ellipse — a circle in a square box.
    #[must_use]
    pub const fn oval() -> Self {
        Self::new(ClipShape::Oval)
    }
}

impl RenderObject for RenderClip {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        let path = self.shape.to_path(bounds);
        let canvas = ctx.canvas();
        canvas.save();
        match path {
            Some(path) => canvas.clip_path(&path),
            None => canvas.clip_rect(bounds),
        }
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        ctx.canvas().restore();
    }

    /// The clip's **bounding box**, for the children `paint` cannot reach — see
    /// [`RenderObject::layer_clip`].
    ///
    /// Deliberately looser than what `paint` records. A layer carries a
    /// rectangle, so a rounded or path shape can only report the box around it:
    /// a boundary child painting into a cut corner still shows there. That is
    /// the one place this fix is partial, and it is partial in the safe
    /// direction — content that should be visible never disappears.
    fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
        Some(bounds)
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Clipping is paint. An animating radius must not relayout the subtree
        // underneath it — the same reasoning as `RenderOpacity`.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderClip"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;
    use vieww_paint::{Canvas, Scene};

    use super::*;

    fn painted(object: &RenderClip, size: Size) -> Scene {
        let mut scene = Scene::new();
        // The object's own paint, then a child drawing well outside the shape.
        let bounds = Rect::from_origin_size(Offset::ZERO, size);
        let path = object.shape.to_path(bounds);
        scene.save();
        match path {
            Some(path) => scene.clip_path(&path),
            None => scene.clip_rect(bounds),
        }
        scene.fill_rect(Rect::new(-50.0, -50.0, 500.0, 500.0), Color::RED.into());
        scene.restore();
        scene
    }

    #[test]
    fn a_rounded_clip_reaches_the_backend_as_a_shape() {
        let scene = painted(&RenderClip::rounded(8.0), Size::new(100.0, 100.0));
        assert_eq!(scene.commands()[0].clip().shapes().len(), 1);
    }

    #[test]
    fn a_zero_radius_clip_stays_on_the_rectangle_fast_path() {
        let scene = painted(&RenderClip::rounded(0.0), Size::new(100.0, 100.0));
        assert!(scene.commands()[0].clip().shapes().is_empty());
        assert_eq!(
            scene.commands()[0].bounds(),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            "and it still clips"
        );
    }

    #[test]
    fn an_oval_in_an_oblong_box_is_an_ellipse_rather_than_a_stadium() {
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let path = oval(bounds);
        assert_eq!(
            path.bounds(),
            bounds,
            "an ellipse fills its box; a stadium would leave the ends short"
        );
    }

    #[test]
    fn an_oval_clip_bounds_the_child_to_the_box() {
        let scene = painted(&RenderClip::oval(), Size::new(100.0, 100.0));
        assert_eq!(
            scene.commands()[0].bounds(),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            "the shape cuts the corners; the box is what damage sees"
        );
    }

    #[test]
    fn clipping_never_relayouts() {
        let object = RenderClip::rounded(8.0);
        assert!(!object.layout_differs(&RenderClip::rounded(24.0)));
    }
}
