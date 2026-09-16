use std::cell::Cell;
use std::fmt;

use vieww_foundation::{Constraints, Offset, Size};
use vieww_widget::Handler;

use crate::{LayoutCtx, RenderObject};

/// Lays its child out unchanged and reports the constraints it was handed.
///
/// The child's size, position and painting are untouched: this adds no geometry
/// of its own. What it adds is the other half of the question
/// [`RenderMeasured`](crate::RenderMeasured) answers. That one reports **where
/// the child ended up**; this one reports **how much room it was offered**, and
/// they are not the same number — a child that shrink-wraps is smaller than the
/// space it was allowed.
///
/// The space offered is what a responsive layout needs. "Is this a phone or a
/// desktop" is a question about the box this subtree was given, not about what
/// it chose to fill.
///
/// # Why layout rather than paint
///
/// The opposite choice from `RenderMeasured`, and for the opposite reason.
/// A global position does not exist until the parent has placed the child, so a
/// rectangle reported from layout would be stale. Constraints are the input to
/// layout, so layout is the *only* phase that has them, and by paint they are
/// gone.
///
/// That puts this object in the same company as
/// [`RenderViewport`](crate::RenderViewport): a handler called from layout,
/// writing a signal that is marked after the build that would have read it. The
/// consequence is one frame of lag, and it is why the widget on top of this
/// seeds its first build from the surface size rather than waiting — see
/// [`LayoutBuilder`](vieww_widget::LayoutBuilder).
///
/// # The change guard is not an optimisation
///
/// A handler here writes a signal, and layout runs on every frame that anything
/// moved. A signal written on every layout marks its readers pending on every
/// layout — a rebuild that never settles, which pins the display at the refresh
/// rate for the life of the application and reads as a runaway animation.
///
/// Reporting only what changed is what makes calling out of layout safe at all.
/// `RenderViewport::report_extents` documents the same obligation, and
/// [`FrameDriver::MAX_LAYOUT_PENDING_FRAMES`](crate::FrameDriver::MAX_LAYOUT_PENDING_FRAMES)
/// is the backstop for when it is got wrong: eight frames, then the loop stops
/// and says which elements.
pub struct RenderMeasuredConstraints {
    on_constraints: Option<Handler<Constraints>>,
    /// The last constraints handed upward, so the same ones are never sent
    /// twice. See the type docs — this is load-bearing, not a saving.
    reported: Cell<Option<Constraints>>,
}

impl RenderMeasuredConstraints {
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_constraints: None,
            reported: Cell::new(None),
        }
    }

    #[must_use]
    pub fn on_constraints(mut self, handler: Option<Handler<Constraints>>) -> Self {
        self.on_constraints = handler;
        self
    }

    /// Hand the constraints upward, but only when they are news.
    fn report(&self, constraints: Constraints) {
        if self.reported.get() == Some(constraints) {
            return;
        }
        self.reported.set(Some(constraints));
        if let Some(handler) = &self.on_constraints {
            handler(constraints);
        }
    }
}

impl Default for RenderMeasuredConstraints {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RenderMeasuredConstraints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderMeasuredConstraints")
            .field("reported", &self.reported.get())
            .finish_non_exhaustive()
    }
}

impl RenderObject for RenderMeasuredConstraints {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Reported before the child is laid out rather than after, so that the
        // value is sent even by a frame where the child fails to size — the
        // question is what room there was, and that is known on entry.
        self.report(constraints);

        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // A new handler cannot move anything: the child is laid out against the
        // same constraints either way. Relayout here would be work done for a
        // closure swap, which happens on every rebuild that captures state —
        // and every rebuild of a `LayoutBuilder` captures state.
        let _ = new;
        false
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        // Required by returning `false` above, and this is the line that stops
        // the loop. The cache *is* the change guard: losing it across a rebuild
        // would re-report unchanged constraints, waking the reader that just
        // rebuilt, which rebuilds and loses it again. That is the rebuild loop
        // in its purest form, and it would look exactly like a working
        // application running hot.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.reported.set(old.reported.get());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderMeasuredConstraints"
    }
}
