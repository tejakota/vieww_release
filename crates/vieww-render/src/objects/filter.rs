use vieww_foundation::{Constraints, ImageFilter, Offset, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Puts everything beneath it through an [`ImageFilter`] before compositing.
///
/// Layout is untouched and hit testing is unaffected — a blurred widget still
/// occupies its space and still takes taps. The same contract
/// [`RenderOpacity`](crate::RenderOpacity) has, and for the same reason: a
/// visual effect that moved things would be a layout bug wearing a filter's
/// name.
///
/// # Why this is a sibling of `RenderOpacity` and not a field on it
///
/// They do compose into one layer — `Command::PushLayer` carries `alpha`,
/// `blend` *and* `filter`, because a group is composited once and a second
/// marker would need a second pop. But they are separate *objects* because they
/// are separate widgets with separate costs: an `Opacity` is free enough to put
/// anywhere, and a filter allocates an offscreen buffer and runs a kernel over
/// it. Folding the two would hide that from anyone reading a tree dump.
///
/// # A filter, or a backdrop-filter — [`ImageFilter::backdrop`] decides
///
/// By default (`filter.backdrop == false`) this filters the subtree's
/// **own** pixels: the group is rasterised on nothing, filtered, then
/// composited, so a blurred card is a blurred card, not a window onto a
/// blurred background. When `filter.backdrop` is set, the group's starting
/// content is instead a real copy of the destination *behind* it — CSS's
/// `backdrop-filter` — with this object's own children then painting sharp
/// on top of the filtered result; the CPU backend
/// (`vieww-paint::native::reference`) implements exactly that ordering.
/// `vieww-widget::Filtered::with_backdrop` is the widget-level door into it.
///
/// # What each backend does
///
/// The CPU backend applies both the blur and the colour matrix for real, and is
/// the reference implementation. The GPU backend composites the layer
/// **unfiltered** and counts it in `SceneReport::skipped_filters`, because
/// vello 0.9 has no layer-filter API. Sharp rather than missing, and counted
/// rather than silent.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFilter {
    /// What the group's pixels go through.
    pub filter: ImageFilter,
}

impl RenderFilter {
    #[must_use]
    pub const fn new(filter: ImageFilter) -> Self {
        Self { filter }
    }

    /// `true` when filtering would change nothing, so no offscreen is worth
    /// allocating.
    fn is_a_no_op(&self) -> bool {
        self.filter.is_noop()
    }
}

impl RenderObject for RenderFilter {
    fn debug_name(&self) -> &'static str {
        "RenderFilter"
    }

    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![
            ("blur", format!("{:.1}", self.filter.blur_sigma)),
            (
                "color_matrix",
                if self.filter.color_matrix.is_some() {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
            ),
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
        // The object's own box. `Scene::push_filtered_layer` grows it by the
        // blur's reach, so a blurred panel is not cut off square at its edge.
        let bounds = ctx.bounds();
        let filter = self.filter;
        ctx.canvas()
            .push_filtered_layer(bounds, 1.0, vieww_foundation::BlendMode::Normal, filter);
    }

    fn paint_children_done(&self, ctx: &mut PaintCtx<'_>) {
        if self.is_a_no_op() {
            return;
        }
        ctx.canvas().pop_layer();
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Filtering cannot move anything, so animating a blur must not
        // relayout — the same reasoning as `RenderOpacity` and
        // `RenderTransform`.
        false
    }
}
