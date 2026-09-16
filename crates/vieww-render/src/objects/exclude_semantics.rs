use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Draws its child and hides it from a screen reader.
///
/// A transparent wrapper in every other respect: the child is laid out at the
/// same size, painted in the same place, and takes taps exactly as it would
/// without this in the way. Only the semantics tree is affected.
///
/// # Why this exists rather than reusing `RenderOffstage`
///
/// Because "visible" and "reachable" come apart, and
/// [`Navigator`](vieww_widget::Navigator) is where. It keeps the screen one
/// below the top **onstage** on purpose — that screen is what a push slides
/// over and a pop reveals, and animating against a blank surface instead would
/// be visibly wrong. Going offstage is therefore not available to it.
///
/// Leaving it in the semantics tree, though, means a screen reader can walk to
/// a control on a screen the user cannot see and activate it —
/// `handle_semantic_action` dispatches by id, so the screen on top absorbing
/// taps does not help. This is the object that closes that gap without giving
/// up the frame the transition needs.
///
/// # Size
///
/// Its child's, exactly. It adds no geometry of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderExcludeSemantics {
    /// `true` while the subtree is hidden from a screen reader.
    ///
    /// A property rather than a separate object for the same reason
    /// [`RenderOffstage`](crate::RenderOffstage)'s is: covering and uncovering a
    /// route is a property change, and a reparent would unmount the state the
    /// route stack exists to preserve.
    pub excluded: bool,
}

impl RenderExcludeSemantics {
    #[must_use]
    pub const fn new(excluded: bool) -> Self {
        Self { excluded }
    }
}

impl RenderObject for RenderExcludeSemantics {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn hides_semantics(&self) -> bool {
        self.excluded
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // By equality, like every sibling, even though the flag provably cannot
        // move any geometry — the child is laid out and placed identically
        // either way, and only the semantics walk reads it.
        //
        // Returning `false` unconditionally would be the tighter answer and is
        // not worth the risk: it opts a subtree out of layout, and the one thing
        // that reliably goes wrong there is a cache that needed the pass and
        // silently did not get it — see `adopt_layout_cache`. The flag flips
        // when a route is covered or uncovered, which is mid-transition, when a
        // relayout is happening anyway.
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderExcludeSemantics"
    }
}
