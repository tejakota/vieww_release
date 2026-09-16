use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Hides everything painted *before* it from a screen reader.
///
/// A transparent wrapper in every other respect: the child is laid out at the
/// same size, painted in the same place, and takes taps exactly as it would
/// without this in the way. Only the semantics tree is affected — and only the
/// part of it that was already collected when this object is reached.
///
/// # Why the scope is preceding siblings and not a subtree
///
/// Because a modal barrier is not *above* the thing it hides. It is beside it.
///
/// [`RenderExcludeSemantics`](crate::RenderExcludeSemantics) answers "is this
/// subtree readable", which is the question a covered route asks: the route is
/// an ancestor of everything it wants to silence. A dialog is not. It is a
/// sibling of the screen behind it in the navigator's stack, painted after it,
/// and there is no node that contains the screen and not the dialog. Wrapping
/// the thing to be hidden is therefore not available here — the barrier has to
/// name what it covers by *paint order*, which is the one relationship it
/// actually has with the screen behind.
///
/// So the flag travels the other way: [`SemanticsTree`](crate::SemanticsTree)
/// collects children in paint order, and reaching an object that blocks drops
/// what the earlier siblings contributed. Everything painted after it is
/// untouched, which is what keeps the dialog's own content readable — the
/// barrier is the first child of the dialog's stack and the surface is the
/// second.
///
/// # It propagates upward
///
/// The barrier is nested several levels inside the route that owns it, and the
/// screen it hides is a sibling of that *route*. So the block does not stop at
/// the level that declares it: it is reported to each parent in turn, clearing
/// preceding siblings at every level on the way up until it reaches the stack
/// where the screen behind actually sits.
///
/// This deliberately does **not** stop at a semantic
/// boundary. It has to. `Dialog` wraps itself in a `Semantics` container so a
/// screen reader announces it as one group, and stopping there would leave the
/// block inside the dialog, which is the one place it has nothing to do.
///
/// # What it over-blocks, and why that is the direction to err in
///
/// Nothing bounds the climb, so the block is **wider than the scrim** whenever a
/// navigator is not the whole screen. In a `Column[TabBar, Navigator]`, a dialog
/// pushed inside the navigator silences the tab bar as well, even though the
/// scrim only ever covered the navigator's box — so a finger could still press
/// a tab that a screen reader can no longer find.
///
/// Two reasons to leave it there for now. It is the safe direction: the failure
/// is something unreachable that could have been reached, not something
/// reachable that should not have been, and the second is the defect this whole
/// object exists to fix. And it is what a full-screen modal should do anyway —
/// A root-overlay barrier sits above everything, so a dialog
/// there blocks the entire application, tab bar included.
///
/// The bound, if one is ever wanted, is geometric rather than structural: clear
/// a preceding sibling only where the barrier's global bounds contain its own.
/// That is a real design decision and no application here has needed it yet.
///
/// # Size
///
/// Its child's, exactly. It adds no geometry of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderBlockSemantics {
    /// `true` while everything painted before this object is hidden from a
    /// screen reader.
    ///
    /// A property rather than a separate object, for the reason
    /// [`RenderExcludeSemantics`](crate::RenderExcludeSemantics) gives: a
    /// barrier appearing and disappearing is a property change, and a reparent
    /// would unmount the state around it.
    pub blocking: bool,
}

impl RenderBlockSemantics {
    #[must_use]
    pub const fn new(blocking: bool) -> Self {
        Self { blocking }
    }
}

impl RenderObject for RenderBlockSemantics {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn blocks_semantics(&self) -> bool {
        self.blocking
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // By equality, for the reason `RenderExcludeSemantics` gives: the flag
        // provably moves no geometry, and returning `false` unconditionally
        // would opt a subtree out of layout to save nothing worth the risk.
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderBlockSemantics"
    }
}
