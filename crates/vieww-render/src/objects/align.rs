use vieww_foundation::{Alignment, Constraints, Size};

use crate::{LayoutCtx, RenderObject};

/// Positions its child within itself.
///
/// Expands to fill whatever bounded space it is given — an `Align` that
/// shrink-wrapped its child would have nothing to align *within*. On an
/// unbounded axis there is no space to fill, so it falls back to the child's
/// size there.
///
/// # The size factors, and why a reveal animation is one of these
///
/// A factor replaces "fill the space" with "be this multiple of the child's own
/// extent" on that axis. `height_factor: 0.5` makes this box half as tall as
/// its child, with the child aligned inside and the overflow left for a
/// [`RenderClip`](crate::RenderClip) above to cut.
///
/// Animating that factor from 0 to 1 is a reveal — and it is a reveal to the
/// child's **natural** height, measured this frame, in one layout pass. That is
/// what let `Accordion::content_height` stop being a required parameter: the
/// caller was being asked for a number the layout already had. The classic
/// size transition is the same mechanism for the same reason.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderAlign {
    pub alignment: Alignment,
    /// Width as a multiple of the child's own, instead of filling the space.
    pub width_factor: Option<f32>,
    /// Height as a multiple of the child's own, instead of filling the space.
    pub height_factor: Option<f32>,
}

impl RenderAlign {
    #[must_use]
    pub const fn new(alignment: Alignment) -> Self {
        Self {
            alignment,
            width_factor: None,
            height_factor: None,
        }
    }

    /// Take `factor` times the child's width rather than filling.
    #[must_use]
    pub const fn width_factor(mut self, factor: f32) -> Self {
        self.width_factor = Some(factor);
        self
    }

    /// Take `factor` times the child's height rather than filling.
    #[must_use]
    pub const fn height_factor(mut self, factor: f32) -> Self {
        self.height_factor = Some(factor);
        self
    }
}

impl RenderObject for RenderAlign {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.constrain(Size::ZERO);
        };

        // Loosened: the child may be any size up to the maximum, which is what
        // leaves slack for the alignment to work with.
        let child_size = ctx.layout_child(child, constraints.loosen());

        // A factor wins over filling, and over falling back to the child, on
        // whichever axis declares one. Clamped at zero because a negative
        // factor is an inverted box, which every later subtraction turns into
        // something worse.
        let size = constraints.constrain(Size::new(
            match self.width_factor {
                Some(factor) => child_size.width * factor.max(0.0),
                None if constraints.has_bounded_width() => constraints.max_width,
                None => child_size.width,
            },
            match self.height_factor {
                Some(factor) => child_size.height * factor.max(0.0),
                None if constraints.has_bounded_height() => constraints.max_height,
                None => child_size.height,
            },
        ));

        ctx.place_child(child, self.alignment.inscribe(child_size, size));
        size
    }

    crate::baseline::pass_through_baseline!();

    /// The child's answer, scaled by the factor on that axis.
    ///
    /// Without the scaling, a half-revealed accordion inside an
    /// `IntrinsicHeight` would reserve the room for the whole body.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        let factor = match query.axis {
            vieww_foundation::Axis::Horizontal => self.width_factor,
            vieww_foundation::Axis::Vertical => self.height_factor,
        };
        let child = ctx.only_child_intrinsic(query)?;
        Some(match factor {
            Some(factor) => child * factor.max(0.0),
            None => child,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderAlign"
    }
}
