//! Keyboard focus: who has it, who can get it, and where Tab goes next.
//!
//! # Why the framework must own focus
//!
//! A screen reader user navigates by focus. A keyboard user navigates by
//! focus. A sighted mouse user never thinks about focus — which is exactly
//! why it is easy to build a UI where focus is broken and nobody notices
//! until the bug report titled "cannot use your app with a keyboard".
//!
//! Focus cannot be a widget-level concern because it is *global*: exactly
//! one thing has it at a time, and the order it moves in spans the whole
//! tree. It is framework state, exposed to widgets.
//!
//! # The traversal order
//!
//! Focus order follows the *reading order* of the tree, not the paint
//! order — a `Stack` where a later child is drawn on top still reads in
//! tree order, and Tab follows reading order. A widget can opt out
//! (`skip_in_traversal`) or redirect (`TraversalPolicy` for a dialog that
//! traps focus while it is open).
//!
//! # Focus scopes
//!
//! A dialog is a focus scope: when it opens, focus moves into it; when it
//! closes, focus returns to what had it before. Without scopes, closing a
//! dialog leaves focus nowhere, and a keyboard user is stranded at the top
//! of the page.

use std::cell::RefCell;
use std::rc::Rc;

/// A node in the focus graph.
///
/// Not every widget is a focus node — a `Container` is not focusable and
/// does not need to be. Widgets that *are* focusable (buttons, text
/// fields, links) create one, and the focus manager tracks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FocusNodeId(pub u64);

/// Whether and how a node participates in focus traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusProperties {
    /// Can this node receive focus at all?
    pub focusable: bool,
    /// Skip this node when Tab-pasting through, but allow programmatic focus.
    ///
    /// For decorative elements that should not interrupt the tab order.
    pub skip_in_traversal: bool,
    /// A positive number to focus earlier, negative to focus later.
    ///
    /// Rarely needed — the DOM order is almost always right — but the
    /// escape hatch for the case where the visual order must differ from
    /// the tree order.
    pub traversal_order: i32,
}

impl FocusProperties {
    /// A standard focusable node.
    ///
    /// Ergonomic constructor taken from patch 6's revision of this module —
    /// the rest of that revision dropped `traversal_order` and
    /// `FocusAction::Adjust`, so only this part was worth keeping.
    #[must_use]
    pub const fn focusable() -> Self {
        Self {
            focusable: true,
            skip_in_traversal: false,
            traversal_order: 0,
        }
    }

    /// A focusable node that Tab skips, but code can still focus.
    #[must_use]
    pub const fn skipped() -> Self {
        Self {
            focusable: true,
            skip_in_traversal: true,
            traversal_order: 0,
        }
    }
}

/// An action the focused node can respond to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusAction {
    Activate,
    /// Increase (true) or decrease (false).
    Adjust(bool),
    /// Dismiss (Escape).
    Dismiss,
    FocusNext,
    FocusPrevious,
}

/// The state of keyboard focus across the whole tree.
///
/// A single instance lives alongside the element tree; widgets read it to
/// draw focus rings and the platform layer feeds it keyboard events.
#[derive(Debug, Default)]
pub struct FocusManager {
    inner: Rc<RefCell<FocusInner>>,
}

#[derive(Default)]
struct FocusInner {
    /// The focus graph, in tree order.
    nodes: Vec<(FocusNodeId, FocusProperties)>,
    next_id: u64,
    /// Who currently has focus.
    focused: Option<FocusNodeId>,
    /// The focus scope stack. The top is the active scope.
    scopes: Vec<FocusScopeState>,
    /// A callback when focus changes — the platform bridge uses this to
    /// move the system focus highlight.
    on_change: Option<Box<dyn Fn(FocusNodeId)>>,
}

// `Debug` by hand rather than derived: `on_change` is a boxed closure, and no
// closure implements `Debug`. Printing whether one is installed is the useful
// half anyway — the address of a callback tells a reader nothing.
impl std::fmt::Debug for FocusInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusInner")
            .field("nodes", &self.nodes)
            .field("next_id", &self.next_id)
            .field("focused", &self.focused)
            .field("scopes", &self.scopes)
            .field("on_change", &self.on_change.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

#[derive(Debug, Default)]
struct FocusScopeState {
    /// The node that had focus before this scope opened.
    /// Restored when the scope closes.
    previous: Option<FocusNodeId>,
    /// Whether focus is trapped inside this scope.
    trapped: bool,
}

impl FocusManager {
    /// An empty focus graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a focusable node, returning its id.
    ///
    /// Called by widgets when they mount. The properties are read once at
    /// registration; a widget whose focusability changes must re-register,
    /// which the element tree's update path does.
    pub fn register(&self, properties: FocusProperties) -> FocusNodeId {
        let mut inner = self.inner.borrow_mut();
        let id = FocusNodeId(inner.next_id);
        inner.next_id += 1;
        inner.nodes.push((id, properties));
        id
    }

    /// Unregister a node (it unmounted).
    pub fn unregister(&self, id: FocusNodeId) {
        let mut inner = self.inner.borrow_mut();
        inner.nodes.retain(|(nid, _)| *nid != id);
        if inner.focused == Some(id) {
            inner.focused = None;
            // Focus was on the thing that went away. Move it to the first
            // focusable node rather than leaving it nowhere — a keyboard
            // user with focus nowhere is a keyboard user who cannot type.
            inner.focused = inner
                .nodes
                .iter()
                .find(|(_, p)| p.focusable && !p.skip_in_traversal)
                .map(|(nid, _)| *nid);
        }
    }

    /// Who has focus.
    #[must_use]
    pub fn focused(&self) -> Option<FocusNodeId> {
        self.inner.borrow().focused
    }

    /// Give focus to `id`.
    pub fn request_focus(&self, id: FocusNodeId) {
        let mut inner = self.inner.borrow_mut();
        // Check the node is registered and focusable.
        if inner.nodes.iter().any(|(nid, p)| *nid == id && p.focusable) {
            inner.focused = Some(id);
            if let Some(cb) = &inner.on_change {
                cb(id);
            }
        }
    }

    /// Clear focus (nothing has it).
    pub fn clear_focus(&self) {
        self.inner.borrow_mut().focused = None;
    }

    /// Move focus to the next focusable node in traversal order.
    ///
    /// Wraps around at the end — Tab from the last element goes to the
    /// first, which is what users expect.
    pub fn focus_next(&self) {
        let inner = self.inner.borrow();
        let focusable: Vec<_> = inner
            .nodes
            .iter()
            .filter(|(_, p)| p.focusable && !p.skip_in_traversal)
            .map(|(nid, _)| *nid)
            .collect();

        if focusable.is_empty() {
            return;
        }

        let next = match inner.focused {
            Some(current) => {
                let idx = focusable.iter().position(|n| *n == current);
                match idx {
                    Some(i) => focusable[(i + 1) % focusable.len()],
                    // Focused node not in the focusable list (it was
                    // unregistered): start from the beginning.
                    None => focusable[0],
                }
            }
            None => focusable[0],
        };
        drop(inner);
        self.request_focus(next);
    }

    /// Move focus to the previous focusable node.
    pub fn focus_previous(&self) {
        let inner = self.inner.borrow();
        let focusable: Vec<_> = inner
            .nodes
            .iter()
            .filter(|(_, p)| p.focusable && !p.skip_in_traversal)
            .map(|(nid, _)| *nid)
            .collect();

        if focusable.is_empty() {
            return;
        }

        let prev = match inner.focused {
            Some(current) => {
                let idx = focusable.iter().position(|n| *n == current);
                match idx {
                    Some(i) => focusable[(i + focusable.len() - 1) % focusable.len()],
                    None => focusable[focusable.len() - 1],
                }
            }
            None => focusable[focusable.len() - 1],
        };
        drop(inner);
        self.request_focus(prev);
    }

    /// Push a focus scope (a dialog opened).
    ///
    /// Remembers the current focus; when the scope pops, focus returns.
    pub fn push_scope(&self, trapped: bool) {
        let mut inner = self.inner.borrow_mut();
        // Read `focused` out before the `push` call: evaluating it inside the
        // argument holds an immutable borrow of `inner` across the mutable one
        // `push` needs.
        let previous = inner.focused;
        inner.scopes.push(FocusScopeState { previous, trapped });
    }

    /// Pop the innermost focus scope (a dialog closed).
    ///
    /// Focus returns to whatever had it before the scope opened.
    pub fn pop_scope(&self) {
        let mut inner = self.inner.borrow_mut();
        if let Some(scope) = inner.scopes.pop() {
            inner.focused = scope.previous;
        }
    }

    /// `true` if focus is currently trapped in a scope.
    #[must_use]
    pub fn is_trapped(&self) -> bool {
        self.inner.borrow().scopes.last().is_some_and(|s| s.trapped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focusable() -> FocusProperties {
        FocusProperties::focusable()
    }

    #[test]
    fn focus_starts_nowhere() {
        let mgr = FocusManager::new();
        assert_eq!(mgr.focused(), None);
    }

    #[test]
    fn focus_next_wraps_around() {
        let mgr = FocusManager::new();
        let a = mgr.register(focusable());
        let b = mgr.register(focusable());
        let c = mgr.register(focusable());

        mgr.focus_next();
        assert_eq!(mgr.focused(), Some(a));

        mgr.focus_next();
        assert_eq!(mgr.focused(), Some(b));

        mgr.focus_next();
        assert_eq!(mgr.focused(), Some(c));

        mgr.focus_next();
        assert_eq!(mgr.focused(), Some(a), "wrapped");
    }

    #[test]
    fn focus_previous_wraps_around() {
        let mgr = FocusManager::new();
        let a = mgr.register(focusable());
        let b = mgr.register(focusable());

        // Shift+Tab with nothing focused enters at the *end*, mirroring Tab
        // entering at the start. The generated test asserted `a` here and
        // called it "previous goes to first", which would mean Tab and
        // Shift+Tab both land on the same element from a cold start — and
        // Shift+Tab could then never reach the last one at all.
        mgr.focus_previous();
        assert_eq!(
            mgr.focused(),
            Some(b),
            "from nowhere, previous goes to last"
        );

        mgr.focus_previous();
        assert_eq!(mgr.focused(), Some(a));

        mgr.focus_previous();
        assert_eq!(mgr.focused(), Some(b), "wrapped");
    }

    #[test]
    fn unregistering_the_focused_node_moves_focus() {
        let mgr = FocusManager::new();
        let a = mgr.register(focusable());
        let b = mgr.register(focusable());

        mgr.request_focus(b);
        assert_eq!(mgr.focused(), Some(b));

        mgr.unregister(b);
        assert_eq!(
            mgr.focused(),
            Some(a),
            "focus fell to the first focusable node"
        );
    }

    #[test]
    fn a_scope_restores_focus_on_pop() {
        let mgr = FocusManager::new();
        let a = mgr.register(focusable());
        let dialog_button = mgr.register(focusable());

        mgr.request_focus(a);

        // Dialog opens.
        mgr.push_scope(true);
        mgr.request_focus(dialog_button);
        assert_eq!(mgr.focused(), Some(dialog_button));

        // Dialog closes.
        mgr.pop_scope();
        assert_eq!(mgr.focused(), Some(a), "focus returned to where it was");
    }

    #[test]
    fn trapped_reports_the_scope_state() {
        // Taken from patch 6's revision, which tested `is_trapped` and this
        // one did not.
        let mgr = FocusManager::new();
        assert!(!mgr.is_trapped());

        mgr.push_scope(true);
        assert!(mgr.is_trapped());

        mgr.pop_scope();
        assert!(!mgr.is_trapped());
    }

    #[test]
    fn skipped_nodes_are_not_traversed() {
        let mgr = FocusManager::new();
        let _a = mgr.register(focusable());
        let _skipped = mgr.register(FocusProperties {
            focusable: true,
            skip_in_traversal: true,
            ..Default::default()
        });
        let c = mgr.register(focusable());

        mgr.focus_next(); // a
        mgr.focus_next(); // should skip the middle, land on c
        assert_eq!(mgr.focused(), Some(c));
    }
}
