use vieww_foundation::{Color, Constraints, Offset, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Fills its bounds with a solid colour, then lets its child paint on top.
///
/// Takes its size entirely from its child. This is the one core render object
/// that actually draws, which makes it the one that is opaque to hit testing.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderColoredBox {
    pub color: Color,
}

impl RenderColoredBox {
    #[must_use]
    pub const fn new(color: Color) -> Self {
        Self { color }
    }
}

impl RenderObject for RenderColoredBox {
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
        ctx.canvas().fill_rect(bounds, self.color.into());
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Something was painted here, so a point inside the bounds landed on
        // it — unlike a transparent layout box, which lets input through to
        // whatever is behind.
        !self.color.is_transparent()
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Colour affects painting, never geometry: this box is always exactly
        // its child's size. Recolouring must not trigger a relayout.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderColoredBox"
    }
}
