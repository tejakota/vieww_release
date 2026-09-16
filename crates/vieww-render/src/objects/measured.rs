use std::cell::Cell;
use std::fmt;

use vieww_foundation::{Constraints, Offset, Rect, Size};
use vieww_widget::Handler;

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Lays its child out unchanged and reports where the child ended up.
///
/// The child's size, position and painting are untouched: this adds no geometry
/// of its own and is invisible in a layout diff. What it adds is an answer to a
/// question the widget layer cannot otherwise ask — **where am I on screen?**
///
/// # Why anything needs that
///
/// A menu opens *under a button*, and a tooltip *beside the thing it explains*.
/// Both are drawn in an overlay above the whole tree, because a popup clipped by
/// its parent's bounds is not a popup — so neither can be a child of the control
/// it belongs to, and neither can be positioned by ordinary layout.
///
/// The anchor's rectangle is the missing input, and a widget cannot know it:
/// `build` runs before layout, and layout hands a size *up* while position is
/// assigned by the parent afterwards. Only the frame that painted it knows both.
///
/// # Why paint rather than layout
///
/// Because layout does not know the answer yet. An object's global offset is
/// decided by its parent's `place_child`, after `layout` has returned — so a
/// report from inside `layout` would carry a stale position on the frame it
/// matters most, the one where something moved.
///
/// `PaintCtx::origin` is already global, so paint is the first phase where the
/// rectangle exists. It is also the safer phase: a signal written from layout is
/// marked pending after the build that would have read it, which is why
/// [`RenderViewport`](crate::RenderViewport) carries a warning about being the
/// only handler called from layout. This one is not; it lands one frame later,
/// which for a popup's anchor is invisible and for a layout loop is the
/// difference between converging and not.
///
/// # The change guard is not an optimisation
///
/// A handler here writes a signal, and a signal written on every paint marks its
/// readers pending on every paint — a rebuild that never settles, indistinguishable
/// from a runaway animation, pinning the display at the refresh rate for the life
/// of the application. Reporting only what changed is what makes calling out of
/// paint safe at all. Same obligation as `RenderViewport::report_extents`, which
/// documents the failure in detail.
pub struct RenderMeasured {
    on_measured: Option<Handler<Rect>>,
    /// The last rectangle handed upward, so the same one is never sent twice.
    reported: Cell<Option<Rect>>,
}

impl RenderMeasured {
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_measured: None,
            reported: Cell::new(None),
        }
    }

    #[must_use]
    pub fn on_measured(mut self, handler: Option<Handler<Rect>>) -> Self {
        self.on_measured = handler;
        self
    }

    /// Hand the rectangle upward, but only when it is news. See the type docs.
    fn report(&self, bounds: Rect) {
        if self.reported.get() == Some(bounds) {
            return;
        }
        self.reported.set(Some(bounds));
        if let Some(handler) = &self.on_measured {
            handler(bounds);
        }
    }
}

impl Default for RenderMeasured {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for RenderMeasured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The handler is an `Rc<dyn Fn>` with nothing printable about it; the
        // last reported rectangle is the state worth seeing in a tree dump.
        f.debug_struct("RenderMeasured")
            .field("reported", &self.reported.get())
            .finish_non_exhaustive()
    }
}

impl RenderObject for RenderMeasured {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        // Draws nothing, and does not paint its child either — the *tree* walks
        // children, for repaint boundaries and paint order. See
        // `RenderOffstage`, which documents that division. This is here purely
        // for `ctx.bounds()`, the first and only place the global rectangle
        // exists.
        self.report(ctx.bounds());
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // A new handler cannot move anything: this object's geometry is its
        // child's, and the child is laid out against the same constraints
        // either way. Relayout here would be work done for a closure swap,
        // which happens on every rebuild that captures state.
        let _ = new;
        false
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        // Required by returning `false` above. The cache is the *reported*
        // rectangle, and losing it across a rebuild would re-report an unchanged
        // position — waking every reader of the signal for no news, which is the
        // loop the guard exists to prevent.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.reported.set(old.reported.get());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderMeasured"
    }
}
