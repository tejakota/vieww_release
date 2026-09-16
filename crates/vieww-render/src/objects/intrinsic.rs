use vieww_foundation::{Axis, Constraints, Offset, Size};

use crate::{IntrinsicQuery, LayoutCtx, RenderObject};

/// Tightens one axis to the child's own preferred extent before laying it out.
///
/// # The two-pass shape, stated plainly
///
/// 1. Ask the child how big it wants to be on `axis`, telling it what it will
///    get on the other axis. That query walks the subtree and lays nothing out.
/// 2. Clamp the answer into the incoming constraints, tighten that axis to it,
///    and lay the child out once.
///
/// So the subtree is *visited* twice and *laid out* once. The second visit is
/// the cost; nothing below is measured against constraints it will not get.
///
/// # Unmeasurable is transparent, never zero
///
/// A subtree containing a render object with no intrinsic implementation
/// answers `None`, and this object then hands the child the constraints it was
/// given and reports the child's size — the behaviour of not being in the tree
/// at all. That is the only safe failure: sizing to zero would silently delete
/// content, and sizing to the maximum would silently fill the screen.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderIntrinsicSize {
    /// The axis whose extent is taken from the child rather than the parent.
    pub axis: Axis,
}

impl RenderIntrinsicSize {
    #[must_use]
    pub const fn new(axis: Axis) -> Self {
        Self { axis }
    }
}

impl RenderObject for RenderIntrinsicSize {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };

        // The cross extent is passed down only when the parent actually pinned
        // it. Asking "how tall at unbounded width" and then laying out at a
        // bounded width would measure a paragraph on one line and then wrap it
        // — the classic way an intrinsic height comes out one line short.
        let cross_bound = match self.axis {
            Axis::Horizontal => constraints.max_height,
            Axis::Vertical => constraints.max_width,
        };
        let mut query = IntrinsicQuery {
            axis: self.axis,
            // `Max` — "how big would you like to be" — is the question this
            // widget asks. `Min` is what an overflow check wants, and is not
            // this.
            extremum: crate::Extremum::Max,
            cross: None,
        };
        if cross_bound.is_finite() {
            query = query.across(cross_bound);
        }

        let wanted = ctx.child_intrinsic(child, query);

        let child_constraints = match wanted {
            Some(extent) => match self.axis {
                Axis::Horizontal => {
                    let width = extent.clamp(constraints.min_width, constraints.max_width);
                    Constraints {
                        min_width: width,
                        max_width: width,
                        ..constraints
                    }
                }
                Axis::Vertical => {
                    let height = extent.clamp(constraints.min_height, constraints.max_height);
                    Constraints {
                        min_height: height,
                        max_height: height,
                        ..constraints
                    }
                }
            },
            // Unmeasurable: be transparent. See the type documentation.
            None => constraints,
        };

        let size = ctx.layout_child(child, child_constraints);
        ctx.place_child(child, Offset::ZERO);
        constraints.constrain(size)
    }

    crate::baseline::pass_through_baseline!();

    /// Forwarded unchanged.
    ///
    /// This object tightens one axis; it does not have an opinion of its own
    /// about what the content wants, so anything asking it should get the same
    /// answer it would get from the child. Returning the tightened extent
    /// instead would make two of these nested disagree.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        ctx.only_child_intrinsic(query)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderIntrinsicSize"
    }
}
