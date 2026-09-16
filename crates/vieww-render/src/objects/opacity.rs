use vieww_foundation::{BlendMode, Constraints, Offset, Size};
use vieww_paint::LayerEffect;

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Fades everything beneath it, and optionally blends it with what is behind.
///
/// Layout is untouched and hit testing is unaffected — a faded widget still
/// occupies its space and still takes taps, including at zero, which is what
/// distinguishes "invisible" from "gone". A widget that should do neither
/// should not be in the tree.
///
/// # Group opacity, as of the layer commands
///
/// The subtree is composited into a target of its own and *that* result is
/// faded, which is what "50% opacity" means to everyone who is not implementing
/// it. Overlapping children fade together: text over its own background at half
/// opacity no longer shows the background through the text.
///
/// This used to be per-primitive — the alpha multiplied into each colour as it
/// was recorded — because a `Scene` resolves state into each command at record
/// time and that is what lets damage treat commands independently. The
/// resolution was to add [`push_layer`](vieww_paint::Canvas::push_layer) as a
/// *pair* of commands rather than as canvas state: each command inside still
/// carries its own resolved transform and clip, and the push is patched on pop
/// with the union of what it encloses, so damage still measures a layer without
/// walking into it.
///
/// The cheap path has not gone anywhere.
/// [`push_alpha`](vieww_paint::Canvas::push_alpha) still fades per primitive at
/// no cost, and is the right tool for content that cannot overlap itself.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOpacity {
    /// 0.0 is fully transparent, 1.0 fully opaque. Clamped on construction.
    pub alpha: f32,
    /// How the composited group combines with what is already behind it.
    pub blend: BlendMode,
}

impl RenderOpacity {
    #[must_use]
    pub fn new(alpha: f32) -> Self {
        Self {
            // NaN would poison every colour under it, and a value outside the
            // range would brighten rather than fade.
            alpha: if alpha.is_nan() {
                1.0
            } else {
                alpha.clamp(0.0, 1.0)
            },
            blend: BlendMode::Normal,
        }
    }

    /// Blend the group with what is behind it.
    #[must_use]
    pub fn blended(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }

    /// `true` when this draws nothing at all.
    #[must_use]
    pub fn is_invisible(&self) -> bool {
        self.alpha <= 0.0
    }

    /// `true` when compositing would change nothing, so no layer is worth its
    /// target.
    ///
    /// Opaque *and* blending normally. An opaque `Multiply` still has work to
    /// do, which is why this is not just an alpha check.
    fn is_a_no_op(&self) -> bool {
        self.alpha >= 1.0 && self.blend.is_normal()
    }
}

impl RenderObject for RenderOpacity {
    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![
            ("alpha", format!("{:.2}", self.alpha)),
            ("blend", format!("{:?}", self.blend)),
        ]
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        match ctx.children().first().copied() {
            Some(child) => {
                let size = ctx.layout_child(child, constraints);
                ctx.place_child(child, Offset::ZERO);
                size
            }
            None => constraints.constrain(Size::ZERO),
        }
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if self.is_a_no_op() {
            return;
        }
        // The object's own box is the *estimate* a backend sizes a target from;
        // a child that paints outside it — a shadow — is not clipped by it. See
        // `Canvas::push_layer`.
        let bounds = ctx.bounds();
        let (alpha, blend) = (self.alpha, self.blend);
        ctx.canvas().push_layer(bounds, alpha, blend);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        if self.is_a_no_op() {
            return;
        }
        ctx.canvas().pop_layer();
    }

    /// The same fade, for the children `paint` cannot reach.
    ///
    /// The `push_layer` above covers everything recorded into *this* scene, and
    /// the paint walk stops at a nested repaint boundary — so a `Viewport` or an
    /// indeterminate spinner below this object records elsewhere and never sees
    /// those markers. Declaring the effect is how it reaches them: the layer
    /// tree carries it to the boundary's own layer.
    ///
    /// Both are needed, and neither double-applies, because they cover disjoint
    /// sets — `paint` covers what this scene records, and this covers what it
    /// deliberately does not.
    fn layer_effect(&self) -> Option<LayerEffect> {
        if self.is_a_no_op() {
            return None;
        }
        Some(LayerEffect::new(self.alpha).blend(self.blend))
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Fading cannot move anything, so an animated fade must not relayout —
        // the same reasoning as `RenderTransform`.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderOpacity"
    }
}
