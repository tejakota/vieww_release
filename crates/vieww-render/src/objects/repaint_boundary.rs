use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Gives its subtree its own recording, so a repaint inside it stops here.
///
/// Layout-transparent: the child sees the constraints this object was given and
/// this object reports the size the child chose. It exists entirely for the
/// paint side, which is why it has no configuration at all — two of them are
/// interchangeable, and `layout_differs` says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderRepaintBoundary;

impl RenderRepaintBoundary {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl RenderObject for RenderRepaintBoundary {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn is_repaint_boundary(&self) -> bool {
        true
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderRepaintBoundary"
    }
}
