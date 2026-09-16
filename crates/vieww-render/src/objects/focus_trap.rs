use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Keyboard focus cannot leave this subtree while it is mounted.
///
/// What a modal dialog, alert, bottom sheet or drawer wraps itself in. Tab and
/// Shift+Tab wrap inside the subtree instead of walking out into the screen
/// behind it, and a press outside it moves no focus at all.
///
/// # Why the trap is a render object and not a stack the dialog pushes
///
/// A `push_scope` / `pop_scope` pair is a stack, and a stack can go out of
/// balance. The way it goes out of balance is not exotic: a dialog dismissed by
/// a route pop from a button *inside* it never reaches its own close path, so
/// the trap it pushed outlives it — anchored to a render id that is no longer in
/// the tree. Every subsequent Tab then finds nothing reachable, and focus is
/// stuck for the rest of the session with nothing on screen to explain why.
///
/// Declaring the trap *is* the subtree makes that unrepresentable. The trap
/// cannot outlive the thing it traps, because it is the same object.
/// [`FocusManager::sync_scopes`](crate::FocusManager::sync_scopes) rebuilds the
/// scope stack from the tree every frame, carrying each surviving scope's
/// restore target across, so closing a dialog still returns focus to whatever it
/// interrupted.
///
/// # Nesting
///
/// A dialog above a dialog is ordinary and works: the **innermost** trap is the
/// one in force. A non-modal popover — [`trapping`](Self::trapping) `false` —
/// opened on top of a modal does not widen the modal's trap, which is why the
/// manager looks for the innermost *trapping* scope rather than simply the
/// innermost one.
///
/// # It does not move focus itself
///
/// Which control a dialog opens on is the dialog's decision: a destructive
/// confirmation opens on Cancel, a form opens on its first field. A trap that
/// guessed would be overridden immediately by every caller that cared. What it
/// does do is drop a focus left standing *outside* it — a caret still blinking
/// in a field behind a modal is the visible half of this bug.
///
/// # Size
///
/// Its child's, exactly. It adds no geometry of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderFocusTrap {
    /// `true` while focus is confined to this subtree.
    ///
    /// A property rather than mounting and unmounting the object, for the reason
    /// [`RenderBlockSemantics`](crate::RenderBlockSemantics) gives: a dialog
    /// that becomes non-modal is a property change, and a reparent would unmount
    /// the state inside it — including, here, whatever had focus.
    pub trapping: bool,
}

impl RenderFocusTrap {
    #[must_use]
    pub const fn new(trapping: bool) -> Self {
        Self { trapping }
    }
}

impl RenderObject for RenderFocusTrap {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn traps_focus(&self) -> bool {
        self.trapping
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // By equality, as `RenderBlockSemantics` does: the flag provably moves
        // no geometry, and an unconditional `false` would opt a subtree out of
        // layout to save nothing worth the risk.
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderFocusTrap"
    }
}
