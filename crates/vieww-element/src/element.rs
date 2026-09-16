use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_widget::{ElementState, InheritedScope, WidgetNode};

use crate::ElementId;

/// One persistent node in the element tree.
///
/// The element is what survives a rebuild. Its widget is replaced every time
/// the description changes; its identity, its state, its children's identities
/// and its subscriptions do not.
pub struct Element {
    pub(crate) id: ElementId,
    pub(crate) widget: WidgetNode,
    pub(crate) parent: Option<ElementId>,
    pub(crate) children: Vec<ElementId>,
    /// Distance from the root. The rebuild queue is ordered by it, so a parent
    /// is always rebuilt before its children and never rebuilds a child twice.
    pub(crate) depth: usize,
    pub(crate) pending: bool,
    /// Values visible to this element from its ancestors.
    pub(crate) scope: InheritedScope,
    /// Durable state, shared rather than owned outright.
    ///
    /// The frame's animate phase mutates it, and the element's own `build` reads
    /// it through the [`BuildContext`](vieww_widget::BuildContext) — two
    /// borrowers at different times, which an `Rc<RefCell<_>>` expresses and a
    /// `Box` would not.
    pub(crate) state: Option<Rc<RefCell<dyn ElementState>>>,
    /// How many times this element has run `build`. Not needed by the
    /// framework — it is what makes "only the affected element rebuilt"
    /// an assertion rather than a claim.
    pub(crate) build_count: u32,
    /// What `build_count` was at the last
    /// [`mark_builds`](crate::ElementTree::mark_builds), so a rebuild
    /// measurement can be about a window of time rather than about the whole
    /// session.
    ///
    /// Lives here rather than in a map on the tree so that it resets with the
    /// element: a slot reused by a different widget starts a fresh count, and a
    /// map keyed by [`ElementId`] would have to be swept for the dead.
    pub(crate) builds_at_mark: u32,
    /// The tree revision at the last change to this element **or anything
    /// below it**.
    ///
    /// The element tree is already incremental — `update` returns without
    /// walking when a widget is the same `Rc` under the same scope. The render
    /// tree was not: `RenderOwner::sync` re-walked every element every frame,
    /// cloning a `WidgetNode` and rebuilding a child list per node, to discover
    /// that nothing had moved.
    ///
    /// This is the number that lets it stop. A consumer records the revision it
    /// last synced at; any subtree whose `subtree_revision` is not greater than
    /// that has not changed since, and can be skipped whole.
    ///
    /// **It is an upper bound, never a lower one.** Every mutation path calls
    /// [`ElementTree::touch`], which stamps this element and every ancestor.
    /// Stamping something that did not really change costs a wasted walk;
    /// failing to stamp something that did would leave a stale frame on screen,
    /// so the paths are deliberately conservative — `update` stamps whenever it
    /// gets past its own early-out, whether or not the new widget differs in a
    /// way the render tree would notice.
    ///
    /// `crates/vieww-render/tests/incremental_sync.rs` is the guard: it drives
    /// a randomised sequence of mutations and asserts an incrementally synced
    /// render tree is identical to one built from scratch.
    pub(crate) subtree_revision: u64,
    /// For an `Inherited` element, the value it publishes and who reads it.
    ///
    /// Held across rebuilds so that republishing a value does not mint a new
    /// scope — see [`Provision`](vieww_widget::Provision) for what that used to
    /// cost.
    pub(crate) provision: Option<Rc<vieww_widget::Provision>>,
    /// The scope this element hands its children, cached.
    ///
    /// Only meaningful alongside `provision`. Cached because it has to be the
    /// *same* `Rc` from frame to frame for the subtree-skipping early-out in
    /// `ElementTree::update` to hold; rebuilding it each time would defeat the
    /// provision it wraps.
    pub(crate) child_scope: Option<InheritedScope>,
}

impl Element {
    /// This element's stable handle.
    #[must_use]
    pub const fn id(&self) -> ElementId {
        self.id
    }

    /// The widget this element was last configured from.
    #[must_use]
    pub const fn widget(&self) -> &WidgetNode {
        &self.widget
    }

    /// The parent, or `None` at the root.
    #[must_use]
    pub const fn parent(&self) -> Option<ElementId> {
        self.parent
    }

    /// Children in paint order.
    #[must_use]
    pub fn children(&self) -> &[ElementId] {
        &self.children
    }

    /// Distance from the root.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// What ancestors have published to this position.
    ///
    /// The same scope a [`BuildContext`](vieww_widget::BuildContext) reads
    /// through `inherit`, exposed because **a render widget never runs a
    /// `build` and so never holds a context**. Without this, ambient state — a
    /// reading direction, a locale — could reach a composed widget and nothing
    /// else, and every render object needing one would have to be wrapped in a
    /// composed widget that exists only to read it.
    ///
    /// Used by `RenderOwner` to feed
    /// `RenderFactory::register_with_context`.
    #[must_use]
    pub const fn scope(&self) -> &InheritedScope {
        &self.scope
    }

    /// `true` if this element is waiting to rebuild.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.pending
    }

    /// How many times this element has built.
    #[must_use]
    pub const fn build_count(&self) -> u32 {
        self.build_count
    }

    /// How many times this element has built since the last
    /// [`mark_builds`](crate::ElementTree::mark_builds).
    ///
    /// Equal to [`build_count`](Self::build_count) when nothing has ever marked,
    /// so a caller that does not care about windows can ignore the distinction.
    #[must_use]
    pub const fn builds_since_mark(&self) -> u32 {
        self.build_count.saturating_sub(self.builds_at_mark)
    }

    /// The durable state created by [`Widget::create_state`](vieww_widget::Widget::create_state).
    #[must_use]
    pub fn state(&self) -> Option<&Rc<RefCell<dyn ElementState>>> {
        self.state.as_ref()
    }

    /// Read this element's state as a concrete type.
    ///
    /// `None` if it has no state, or state of another type.
    #[must_use]
    pub fn state_as<S: ElementState + 'static, R>(&self, read: impl FnOnce(&S) -> R) -> Option<R> {
        let state = self.state.as_ref()?;
        let borrowed = state.borrow();
        Some(read(borrowed.as_any().downcast_ref::<S>()?))
    }

    /// The name of the widget this element was built from.
    #[must_use]
    pub fn debug_name(&self) -> &'static str {
        self.widget.debug_name()
    }

    /// The tree revision at the last change here or anywhere below.
    ///
    /// Compare against a revision you recorded earlier: not greater means
    /// nothing in this subtree has changed since, so a consumer keeping its own
    /// mirror of the tree can skip it whole. See
    /// [`ElementTree::revision`](crate::ElementTree::revision).
    #[must_use]
    pub const fn subtree_revision(&self) -> u64 {
        self.subtree_revision
    }
}

impl fmt::Debug for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Element")
            .field("id", &self.id)
            .field("widget", &self.widget)
            .field("depth", &self.depth)
            .field("pending", &self.pending)
            .field("children", &self.children.len())
            .field("builds", &self.build_count)
            .finish()
    }
}
