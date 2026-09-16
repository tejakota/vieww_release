use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Imposes additional constraints on its child.
///
/// The extra constraints are *enforced against* the incoming ones, so the
/// parent always wins: asking for 500px inside a 200px-max parent yields 200px
/// rather than an overflow.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderConstrainedBox {
    pub extra: Constraints,
}

impl RenderConstrainedBox {
    #[must_use]
    pub const fn new(extra: Constraints) -> Self {
        Self { extra }
    }
}

impl RenderObject for RenderConstrainedBox {
    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "width",
                describe_range(self.extra.min_width, self.extra.max_width),
            ),
            (
                "height",
                describe_range(self.extra.min_height, self.extra.max_height),
            ),
        ]
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // `finite_minimums`, not a bare `enforce`. `SizedBox::expand` asks to be
        // infinitely large, and on an axis the parent leaves unbounded —
        // any flex's main axis, anything inside a scrollable — `enforce` clamps
        // that infinite minimum against an infinite maximum and lets it through.
        // The resulting size is infinity, which survives every `constrain` on
        // the way up and turns into `NaN` at the first ancestor that subtracts
        // one extent from another. See `Constraints::finite_minimums` for why
        // that is silent and why shrink-wrapping is the right answer.
        //
        // Found by opening a `Dropdown` in a row: its `Menu` is a full-surface
        // overlay, its `ModalBarrier` is a `SizedBox::expand`, and the whole
        // screen laid out to `NaN` — a `debug_assert` in a test build and a
        // blank window in a release one.
        let effective = self.extra.enforce(constraints).finite_minimums(constraints);

        let Some(&child) = ctx.children().first() else {
            // Childless: take the smallest size the combined constraints allow,
            // which for a tight `SizedBox` is exactly the requested size.
            return constraints.constrain(effective.smallest());
        };

        let child_size = ctx.layout_child(child, effective);
        ctx.place_child(child, Offset::ZERO);
        constraints.constrain(child_size)
    }

    crate::baseline::pass_through_baseline!();

    /// The child's answer, clamped into this object's own extra constraints.
    ///
    /// A tight `SizedBox` therefore reports its exact size whether it has a
    /// child or not, which is what makes `SizedBox` the escape hatch for any
    /// subtree that cannot measure itself: wrapping something unmeasurable in
    /// a `SizedBox` makes the whole thing measurable again.
    ///
    /// Only this object's own constraints are applied. The *parent's*
    /// constraints are deliberately not — an intrinsic answers "what does this
    /// content want", and the caller is the one that decides what it gets.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        let (min, max) = match query.axis {
            Axis::Horizontal => (self.extra.min_width, self.extra.max_width),
            Axis::Vertical => (self.extra.min_height, self.extra.max_height),
        };

        // A tight extra constraint fixes the answer outright, so a childless
        // `SizedBox` is measurable and an unmeasurable child under a tight one
        // no longer poisons anything above it.
        if min.is_finite() && (min - max).abs() < f32::EPSILON {
            return Some(min);
        }

        let child = ctx.only_child_intrinsic(query).or_else(|| {
            // No child at all: the smallest this box can be, which mirrors
            // `layout`'s childless branch. An *unmeasurable* child is a
            // different thing and stays `None`.
            (ctx.child_count() == 0).then_some(if min.is_finite() { min } else { 0.0 })
        })?;

        let low = if min.is_finite() { min } else { 0.0 };
        let high = if max.is_finite() { max } else { f32::INFINITY };
        Some(child.clamp(low, high.max(low)))
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderConstrainedBox"
    }
}

/// Three forms rather than always printing both numbers, because "40.0 to
/// 40.0" is a tight constraint written the long way and a reader scanning a
/// list of them should not have to notice that the two are equal.
fn describe_range(min: f32, max: f32) -> String {
    if (min - max).abs() < f32::EPSILON {
        format!("{min:.1}")
    } else if max.is_finite() {
        format!("{min:.1}\u{2013}{max:.1}")
    } else {
        format!("\u{2265}{min:.1}")
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;
    use vieww_widget::{ColoredBox, Flex, SizedBox, WidgetNode};

    use super::*;
    use crate::FrameDriver;

    const SURFACE: Size = Size::new(400.0, 300.0);

    /// Every size in the tree after one frame, so a single `NaN` anywhere is
    /// caught rather than only the one an assertion happened to look at.
    fn sizes(root: impl Into<WidgetNode>) -> Vec<Size> {
        let mut driver = FrameDriver::new(SURFACE);
        driver.set_root(root);
        driver.draw_frame();
        let tree = driver.owner().tree();
        tree.ids().into_iter().map(|id| tree.size(id)).collect()
    }

    /// A full-surface overlay: what `ModalBarrier` builds, reduced to the two
    /// widgets that matter.
    fn overlay() -> WidgetNode {
        ColoredBox::new(Color::rgb(0, 0, 0))
            .child(SizedBox::expand())
            .into()
    }

    #[test]
    fn a_full_surface_overlay_in_a_column_stays_finite() {
        // A column leaves its main axis unbounded, so `SizedBox::expand` is
        // being asked to be infinitely tall. Before `finite_minimums` it agreed:
        // the column reported "overflowed by infpx", the first ancestor to
        // subtract produced `NaN`, and every sibling laid out to nothing.
        let sizes = sizes(Flex::column().push(overlay()));
        assert!(
            sizes
                .iter()
                .all(|s| s.width.is_finite() && s.height.is_finite()),
            "an unbounded axis made something infinite or NaN: {sizes:?}"
        );
    }

    #[test]
    fn the_overlay_shrink_wraps_rather_than_taking_the_whole_column() {
        // Shrink-wrap is the *defined* answer, not merely a finite one — so it
        // is pinned. An overlay with nothing to measure has no height, which
        // keeps the failure local to the widget that asked for the impossible
        // instead of pushing every sibling off the screen.
        let sizes = sizes(Flex::column().push(overlay()));
        assert!(
            sizes.contains(&Size::new(400.0, 0.0)),
            "the bounded axis still expands to the full 400 and the unbounded \
             one collapses to the parent's minimum: {sizes:?}"
        );
    }

    #[test]
    fn a_bounded_parent_still_gets_a_full_size_overlay() {
        // The half that must not regress: where the parent bounds both axes, an
        // expanding child fills them, which is what every scrim in the framework
        // relies on.
        let sizes = sizes(overlay());
        assert!(
            sizes.contains(&SURFACE),
            "a scrim under bounded constraints still covers the surface: {sizes:?}"
        );
    }
}
