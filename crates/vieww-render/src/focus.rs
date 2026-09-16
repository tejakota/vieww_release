//! Which render object the keyboard is talking to.
//!
//! # Why the render tree, and not the element tree
//!
//! Focus is answered by two questions that only the render tree can answer:
//! *what did the user just tap on* (a hit test) and *what is the next thing
//! along* (a traversal in paint order). The element tree knows neither — it has
//! no geometry, and its order is build order, which is not what Tab should
//! follow.
//!
//! It also has to survive a rebuild, and a `RenderId` does: reconciliation
//! swaps the object *inside* an id rather than replacing the id, so a field
//! keeps focus across every frame its contents change on, which is every frame
//! somebody is typing.
//!
//! # One focused object, plus a stack of scopes
//!
//! A focus *tree* lets an ancestor be "focused" in the sense
//! that focus is somewhere beneath it. This keeps one `Option<RenderId>` and
//! does the bubbling at dispatch time by walking parents, which answers every
//! question that distinction was needed for except one: **a dialog that traps
//! Tab**.
//!
//! That one is answered by [`FocusScope`] instead — a stack of subtree roots,
//! each remembering what had focus when it opened. It is a smaller mechanism
//! than a parallel tree, and it is the mechanism a modal actually needs:
//! traversal is restricted to the innermost trapping scope's subtree, a tap
//! outside it is refused, and popping restores the focus the dialog interrupted.
//!
//! # This replaced a second, unwired focus system
//!
//! `vieww-foundation` carried a complete scope-based `FocusManager` — register,
//! traverse, `push_scope`, `pop_scope`, `is_trapped` — that nothing in the
//! framework ever called; the live focus system was this one, and it had no
//! scopes, so **dialogs could not trap focus at all**. Two implementations of
//! one idea, one of them dead and the other one incomplete, is worse than
//! either alone: reading the foundation module gives an entirely accurate
//! picture of a feature that did not exist. The scopes are here now, where the
//! tree is, and the foundation copy is gone.

use vieww_foundation::{ImeEvent, KeyEvent, Rect};

use crate::{RenderId, RenderTree};

/// A subtree that focus is currently confined to, or merely anchored in.
///
/// Pushed when a modal opens and popped when it closes. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusScope {
    /// The render object whose subtree the scope covers.
    pub root: RenderId,
    /// What had focus when the scope opened, to be restored when it closes.
    ///
    /// `None` is a real answer — a dialog opened from a pointer click on a
    /// screen where nothing was focused — and it restores to nothing, which is
    /// correct: inventing a focus the person did not have is its own bug.
    pub previous: Option<RenderId>,
    /// Whether Tab and taps are confined to the subtree.
    ///
    /// `false` for a scope that only wants the restore-on-close behaviour — a
    /// non-modal popover, a snackbar with an action — where reaching the screen
    /// behind is legitimate.
    pub trapped: bool,
}

/// Who has the keyboard, and how it gets passed on.
#[derive(Debug, Default)]
pub struct FocusManager {
    focused: Option<RenderId>,
    /// Innermost last. A dialog above a dialog is ordinary.
    scopes: Vec<FocusScope>,
}

impl FocusManager {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            focused: None,
            scopes: Vec::new(),
        }
    }

    /// Confine focus to `root`'s subtree until the matching
    /// [`pop_scope`](Self::pop_scope).
    ///
    /// Returns the scope as recorded, so a caller can hold it and assert on the
    /// pop rather than trusting the stack to stay balanced.
    ///
    /// Focus is **not** moved into the scope here. Which control a dialog opens
    /// on is the dialog's decision — a destructive confirmation opens on
    /// Cancel, a form opens on its first field — and a manager that guessed
    /// would be overridden immediately by every caller that cared.
    /// Bring the scope stack into agreement with the tree.
    ///
    /// Called once per frame, from [`prune`](Self::prune). Every render object
    /// answering [`traps_focus`](crate::RenderObject::traps_focus) is a scope
    /// root; the stack is rebuilt from the tree in depth order, with the
    /// `previous` of each surviving scope carried across so that closing a
    /// dialog still restores what it interrupted.
    ///
    /// # Why the tree is the source of truth
    ///
    /// A push/pop pair is a stack that can go out of balance, and the way it
    /// goes out of balance is not exotic: a dialog dismissed by a route pop from
    /// a button inside it never runs its own close path. The trap then outlives
    /// the dialog, anchored to an id that is no longer in the tree, and every
    /// subsequent Tab finds nothing reachable — focus is stuck for the rest of
    /// the session with nothing on screen to explain why.
    ///
    /// Deriving the stack from the tree makes that unrepresentable: the trap is
    /// the subtree, so it cannot outlive it.
    ///
    /// Returns whether focus moved, which happens when a scope closed and
    /// restored, or when the object with focus fell outside a new trap.
    pub fn sync_scopes(&mut self, tree: &RenderTree) -> bool {
        let mut roots = Vec::new();
        if let Some(root) = tree.root() {
            collect_traps(tree, root, &mut roots);
        }
        if roots.iter().eq(self.scopes.iter().map(|scope| &scope.root)) {
            return false;
        }

        // What each surviving scope remembered, kept across the rebuild.
        let mut rebuilt: Vec<FocusScope> = Vec::with_capacity(roots.len());
        for root in roots {
            let previous = self
                .scopes
                .iter()
                .find(|scope| scope.root == root)
                .map(|scope| scope.previous)
                // A scope that has just opened remembers whatever had focus at
                // the moment it did, which is the focus standing right now.
                .unwrap_or(self.focused);
            rebuilt.push(FocusScope {
                root,
                previous: previous.filter(|&id| tree.is_alive(id)),
                trapped: true,
            });
        }

        // A scope that closed restores what it interrupted — the innermost
        // closed one, since an outer scope's restore target is still in force.
        let restore = self
            .scopes
            .iter()
            .rev()
            .find(|scope| !rebuilt.iter().any(|kept| kept.root == scope.root))
            .and_then(|closed| closed.previous)
            .filter(|&id| tree.is_alive(id));

        self.scopes = rebuilt;

        if let Some(id) = restore {
            if self.is_reachable(tree, id) {
                return self.focus(Some(id));
            }
        }
        // Focus left behind outside a newly-opened trap is dropped rather than
        // left pointing through it. The dialog decides where focus lands; a
        // caret still blinking in a field behind a modal is the visible bug.
        if let Some(id) = self.focused {
            if !self.is_reachable(tree, id) {
                return self.focus(None);
            }
        }
        false
    }

    pub fn push_scope(&mut self, root: RenderId, trapped: bool) -> FocusScope {
        let scope = FocusScope {
            root,
            previous: self.focused,
            trapped,
        };
        self.scopes.push(scope);
        scope
    }

    /// Close the innermost scope and restore the focus it interrupted.
    ///
    /// Returns whether focus moved, on the same contract as
    /// [`focus`](Self::focus): a caller uses it to decide whether a frame is
    /// owed.
    ///
    /// A restore target that has since left the tree resolves to nothing rather
    /// than to a dead id — the screen behind a long-lived dialog can legitimately
    /// have rebuilt out from under it.
    pub fn pop_scope(&mut self, tree: &RenderTree) -> bool {
        match self.scopes.pop() {
            Some(scope) => {
                let restore = scope.previous.filter(|&id| tree.is_alive(id));
                self.focus(restore)
            }
            None => false,
        }
    }

    /// The scope focus is currently confined to, if any is trapping.
    ///
    /// The **innermost trapping** one rather than simply the innermost: a
    /// non-modal popover opened on top of a modal dialog must not widen the
    /// trap the dialog established.
    #[must_use]
    pub fn trapping_scope(&self) -> Option<FocusScope> {
        self.scopes
            .iter()
            .rev()
            .find(|scope| scope.trapped)
            .copied()
    }

    /// `true` if focus cannot currently leave a subtree.
    #[must_use]
    pub fn is_trapped(&self) -> bool {
        self.trapping_scope().is_some()
    }

    /// The open scopes, innermost last.
    #[must_use]
    pub fn scopes(&self) -> &[FocusScope] {
        &self.scopes
    }

    /// Whether `id` is inside the current trap — or `true` when there is none.
    #[must_use]
    pub fn is_reachable(&self, tree: &RenderTree, id: RenderId) -> bool {
        self.trapping_scope()
            .is_none_or(|scope| is_within(tree, scope.root, id))
    }

    /// The object with the keyboard, if anything has it.
    #[must_use]
    pub const fn focused(&self) -> Option<RenderId> {
        self.focused
    }

    /// `true` if `id` currently has the keyboard.
    #[must_use]
    pub fn is_focused(&self, id: RenderId) -> bool {
        self.focused == Some(id)
    }

    /// Give the keyboard to `id`, or to nothing.
    ///
    /// Returns whether this changed anything, which is what tells a caller
    /// whether a frame is worth asking for: focus is drawn (a caret appears, a
    /// ring lights up), so a change needs a repaint and a no-op must not
    /// provoke one.
    pub fn focus(&mut self, id: Option<RenderId>) -> bool {
        if self.focused == id {
            return false;
        }
        self.focused = id;
        true
    }

    /// Take the keyboard away from whatever has it.
    pub fn unfocus(&mut self) -> bool {
        self.focus(None)
    }

    /// Drop focus if the focused object no longer exists.
    ///
    /// A rebuild can remove the thing being typed into — a field inside a list
    /// row that scrolled away, a dialog that closed. Without this the id stays
    /// and every subsequent keypress is dispatched into a hole; worse, ids are
    /// reused, so eventually the keys arrive at an unrelated widget that
    /// happens to have inherited the slot.
    pub fn prune(&mut self, tree: &RenderTree) {
        if let Some(id) = self.focused {
            if !tree.is_alive(id) {
                self.focused = None;
            }
        }
        // **The scope stack is rebuilt from the tree, every frame.** A trap is
        // a subtree, so it cannot outlive one; see `sync_scopes`.
        self.sync_scopes(tree);
    }

    /// Every focusable object, in the order Tab should visit them.
    ///
    /// Depth-first in paint order, which is reading order for every layout the
    /// framework has: a column's children top to bottom, a row's left to right.
    /// A widget that wants a different order needs an explicit traversal policy,
    /// and nothing does yet.
    #[must_use]
    pub fn traversal_order(tree: &RenderTree) -> Vec<RenderId> {
        let mut order = Vec::new();
        if let Some(root) = tree.root() {
            collect(tree, root, &mut order);
        }
        order
    }

    /// The traversal order Tab actually follows here and now: the whole tree,
    /// or the innermost trapping scope's subtree.
    ///
    /// Separate from [`traversal_order`](Self::traversal_order), which stays an
    /// associated function over the tree alone, because a semantics dump and an
    /// accessibility harness want the unconfined answer.
    #[must_use]
    pub fn reachable_order(&self, tree: &RenderTree) -> Vec<RenderId> {
        let root = match self.trapping_scope() {
            Some(scope) if tree.is_alive(scope.root) => scope.root,
            // No trap, or a trap whose root has died and `prune` has not run
            // yet — the whole tree either way, which fails open. A focus system
            // that fails closed is one the keyboard cannot escape.
            _ => match tree.root() {
                Some(root) => root,
                None => return Vec::new(),
            },
        };
        let mut order = Vec::new();
        collect(tree, root, &mut order);
        order
    }

    /// Move focus to the next focusable object, wrapping at the end.
    ///
    /// Returns whether anything moved — `false` when the tree has nothing
    /// focusable in it at all, which is the case a caller should not treat as an
    /// error.
    pub fn focus_next(&mut self, tree: &RenderTree, forward: bool) -> bool {
        let order = self.reachable_order(tree);
        if order.is_empty() {
            return self.unfocus();
        }

        let next = match self
            .focused
            .and_then(|id| order.iter().position(|&at| at == id))
        {
            Some(index) => {
                let count = order.len();
                // Wraps in both directions. `+ count` rather than a subtraction
                // that would underflow at index 0.
                let step = if forward { 1 } else { count - 1 };
                order[(index + step) % count]
            }
            // Nothing focused, or focus is on something no longer in the order.
            // Tab starts at the beginning, Shift+Tab at the end.
            None if forward => order[0],
            None => order[order.len() - 1],
        };
        self.focus(Some(next))
    }

    /// The innermost focusable object in a hit test, if any.
    ///
    /// What a tap uses. The hit test runs innermost-first, so the first
    /// focusable entry is the deepest one under the finger — a field inside a
    /// card takes focus rather than the card.
    #[must_use]
    pub fn focusable_in(tree: &RenderTree, hits: &crate::HitTestResult) -> Option<RenderId> {
        hits.entries()
            .iter()
            .map(|entry| entry.id)
            .find(|&id| is_focusable(tree, id))
    }

    /// The same, refusing anything outside the current trap.
    ///
    /// What a tap uses when a modal is open. A dialog that captures Tab and
    /// then hands focus to a field behind it on one click has not trapped
    /// anything — and the click landing on a scrim rather than a control is the
    /// common case, which is why this returns `None` rather than falling back
    /// to the dialog's first control: a press on the dim area outside a sheet
    /// should not move focus at all.
    #[must_use]
    pub fn reachable_focusable_in(
        &self,
        tree: &RenderTree,
        hits: &crate::HitTestResult,
    ) -> Option<RenderId> {
        Self::focusable_in(tree, hits).filter(|&id| self.is_reachable(tree, id))
    }

    /// Offer a key to the focused object, then to its ancestors.
    ///
    /// Returns `true` once something handles it. Bubbling outward is what lets a
    /// field consume ordinary typing while Escape still reaches the dialog
    /// around it — the field says "not mine" and the walk continues.
    ///
    /// Nothing focused means nothing handles it, and the caller decides what an
    /// unhandled key means. That is where Tab becomes traversal rather than a
    /// character.
    pub fn dispatch(&self, tree: &RenderTree, event: &KeyEvent) -> bool {
        self.bubble(tree, |object| object.handle_key(event))
    }

    /// Offer an input method event to the focused object, then to its ancestors.
    pub fn dispatch_ime(&self, tree: &RenderTree, event: &ImeEvent) -> bool {
        self.bubble(tree, |object| object.handle_ime(event))
    }

    /// Whether the focused object wants an input method open on it.
    ///
    /// Not bubbled: an ancestor cannot want a keyboard on a child's behalf, and
    /// treating it as if it could would raise the soft keyboard whenever
    /// anything inside a form was focused.
    #[must_use]
    pub fn accepts_text(&self, tree: &RenderTree) -> bool {
        self.focused
            .and_then(|id| tree.object(id))
            .is_some_and(|object| object.accepts_text())
    }

    /// Where the focused object's caret is, in global coordinates.
    ///
    /// Global rather than local because the only consumer is a platform placing
    /// a candidate window on the screen, and the render object has no idea where
    /// on the screen it is — a child never learns its own position (§3).
    #[must_use]
    pub fn ime_cursor_area(&self, tree: &RenderTree) -> Option<Rect> {
        let id = self.focused?;
        let local = tree.object(id)?.ime_cursor_area()?;
        Some(local.translate(tree.global_offset(id)))
    }

    fn bubble(
        &self,
        tree: &RenderTree,
        mut handle: impl FnMut(&dyn crate::RenderObject) -> bool,
    ) -> bool {
        let mut at = self.focused;
        while let Some(id) = at {
            if let Some(object) = tree.object(id) {
                if handle(object) {
                    return true;
                }
            }
            at = tree.parent(id);
        }
        false
    }
}

/// Depth-first pre-order, parents before their children.
///
/// A focusable object that also has focusable children — a card that can be
/// tabbed to and holds a field — is visited before them, which is the order a
/// reader would take.
/// Every trapping object in the tree, outermost first.
fn collect_traps(tree: &RenderTree, id: RenderId, roots: &mut Vec<RenderId>) {
    if tree.object(id).is_some_and(|object| object.traps_focus()) {
        roots.push(id);
    }
    for &child in tree.children(id) {
        collect_traps(tree, child, roots);
    }
}

fn collect(tree: &RenderTree, id: RenderId, order: &mut Vec<RenderId>) {
    if is_focusable(tree, id) {
        order.push(id);
    }
    for &child in tree.children(id) {
        collect(tree, child, order);
    }
}

fn is_focusable(tree: &RenderTree, id: RenderId) -> bool {
    tree.object(id).is_some_and(|object| object.is_focusable())
}

/// Whether `id` is `root` or a descendant of it.
///
/// Walks up rather than down: a subtree can be most of the screen, and the
/// chain from any node to the root is the depth of the tree.
fn is_within(tree: &RenderTree, root: RenderId, id: RenderId) -> bool {
    let mut at = Some(id);
    while let Some(current) = at {
        if current == root {
            return true;
        }
        at = tree.parent(current);
    }
    false
}
