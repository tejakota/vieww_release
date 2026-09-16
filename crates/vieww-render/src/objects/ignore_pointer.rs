use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Lays out and paints its child, and is invisible to the pointer.
///
/// The subtree is drawn exactly as it would be otherwise; hit testing stops at
/// this object and never enters it, so whatever is *behind* the subtree is what
/// a click, a drag or a hover reaches.
///
/// # The defect that asked for it
///
/// The studio's activity-bar tooltip flickered at frame rate under a pointer
/// that was not moving. The tooltip is drawn near the icon it explains, and for
/// the icons near the top of the column its panel is clamped downwards until it
/// covers that icon. It has no gesture recogniser anywhere in it — its own docs
/// said so, and that was taken to mean it could not interfere — but the panel's
/// `DecoratedBox` and `Text` are opaque to hit testing because they *draw*, so
/// the hover the tooltip depends on landed on the tooltip instead of the icon.
/// The icon was told the pointer had left, the tooltip came down, the hover
/// returned to the icon, and the loop ran until the pointer moved away.
///
/// A widget that explains a control must not be able to take input away from
/// it. That is a property of the whole subtree rather than of any one node in
/// it, which is why this is a wrapper rather than a flag on the panel.
///
/// # Why the tree honours it rather than this object
///
/// Like [`RenderOffstage`](crate::RenderOffstage): the tree walks children
/// itself during a hit test, so declining to be hit is something only the tree
/// can enforce for a subtree. See [`RenderObject::takes_pointers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderIgnorePointer {
    /// `true` while the subtree is invisible to input. A field rather than a
    /// separate object so that turning it on and off is a property change,
    /// which keeps the child's state.
    pub ignoring: bool,
}

impl RenderIgnorePointer {
    #[must_use]
    pub const fn new(ignoring: bool) -> Self {
        Self { ignoring }
    }
}

impl RenderObject for RenderIgnorePointer {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn takes_pointers(&self) -> bool {
        !self.ignoring
    }

    crate::baseline::pass_through_baseline!();

    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        // Input has nothing to do with measurement: an ignored subtree still
        // occupies exactly the space it would have.
        ctx.only_child_intrinsic(query)
    }

    fn debug_name(&self) -> &'static str {
        "RenderIgnorePointer"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignoring_is_what_decides_whether_the_pointer_is_taken() {
        assert!(!RenderIgnorePointer::new(true).takes_pointers());
        assert!(RenderIgnorePointer::new(false).takes_pointers());
    }
}
