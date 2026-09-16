use vieww_foundation::{Constraints, EdgeInsets, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Insets its child, and reports the child's size grown by the insets.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPadding {
    pub insets: EdgeInsets,
}

impl RenderPadding {
    #[must_use]
    pub const fn new(insets: EdgeInsets) -> Self {
        Self { insets }
    }
}

impl RenderObject for RenderPadding {
    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![("insets", format!("{:?}", self.insets))]
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let horizontal = self.insets.horizontal();
        let vertical = self.insets.vertical();

        let Some(&child) = ctx.children().first() else {
            // No child: the padding still occupies its own insets, clamped into
            // whatever the parent allows.
            return constraints.constrain(Size::new(horizontal, vertical));
        };

        // The child sees the space left after the insets are taken out.
        let child_size = ctx.layout_child(child, constraints.deflate(self.insets));
        ctx.place_child(child, Offset::new(self.insets.left, self.insets.top));

        constraints.constrain(Size::new(
            child_size.width + horizontal,
            child_size.height + vertical,
        ))
    }

    crate::baseline::pass_through_baseline!();

    /// The child's answer plus the insets, with the cross extent reduced
    /// before it is passed down.
    ///
    /// The cross reduction is the part that is easy to leave out and wrong to:
    /// asking "how tall are you in 300 points" of a padding with 20 either side
    /// has to ask the child about 260, or the answer is the height of a
    /// paragraph that was never going to get that much room.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        let (along, across) = match query.axis {
            Axis::Horizontal => (self.insets.horizontal(), self.insets.vertical()),
            Axis::Vertical => (self.insets.vertical(), self.insets.horizontal()),
        };

        let mut inner = query;
        inner.cross = query.cross.map(|extent| (extent - across).max(0.0));

        match ctx.only_child_intrinsic(inner) {
            Some(child) => Some(child + along),
            // Childless padding is still its own insets — the same answer
            // `layout` gives, and not `None`, because this one is known.
            None if ctx.child_count() == 0 => Some(along),
            None => None,
        }
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderPadding"
    }
}
