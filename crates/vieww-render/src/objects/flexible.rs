use vieww_foundation::{Constraints, Offset, Size};
use vieww_widget::FlexFactor;

use crate::{LayoutCtx, RenderObject};

/// Carries a flex factor for its parent to read.
///
/// Layout-transparent: constraints pass straight through and the size comes
/// straight back, so wrapping a child in one cannot move a pixel by itself.
/// The *parent* is what changes behaviour — [`RenderFlex`](crate::RenderFlex)
/// asks each child for [`RenderObject::flex`] and lays the ones that answer out
/// against a share of what is left rather than against their natural size.
///
/// Outside a flex it does nothing at all, which is the right answer: a factor
/// nobody is distributing is not an error, it is just unused.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFlexible {
    pub factor: FlexFactor,
}

impl RenderFlexible {
    #[must_use]
    pub const fn new(factor: FlexFactor) -> Self {
        Self { factor }
    }
}

impl RenderObject for RenderFlexible {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Whatever constraints arrive are handed on unchanged. When the parent
        // is a flex, those are already the tight or loose main-axis constraints
        // it computed from this object's own factor — so the child is sized by
        // the share without this object having to know how big the share was.
        match ctx.children().first().copied() {
            Some(child) => {
                let size = ctx.layout_child(child, constraints);
                ctx.place_child(child, Offset::ZERO);
                size
            }
            // A factor with nothing under it still has to satisfy its
            // constraints, and under a tight one that is not zero.
            None => constraints.constrain(Size::ZERO),
        }
    }

    fn flex(&self) -> Option<FlexFactor> {
        Some(self.factor)
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    /// The factor and the fit are the flex's to use, not this object's.
    ///
    /// Same reasoning as [`RenderPositioned`](crate::RenderPositioned): this
    /// object is layout-transparent, so marking only itself when the factor
    /// changes dirties nothing that would act on it. See
    /// [`RenderObject::parent_reads_configuration`].
    fn parent_reads_configuration(&self) -> bool {
        true
    }

    fn debug_name(&self) -> &'static str {
        "RenderFlexible"
    }
}
