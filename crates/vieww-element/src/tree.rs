use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Duration;
use vieww_foundation::FastSet;

use vieww_foundation::Key;
use vieww_widget::{BuildContext, ElementState, InheritedScope, WidgetKind, WidgetNode};

use crate::{BuildError, Element, ElementId, ErrorPolicy, Hotspot, Runtime};

/// Build one step of a [`ElementTree::snapshot_states`] path.
///
/// Free function rather than a `format!` at the call site so the shape of a key
/// is written once — it is a wire format between two compilations of the same
/// program, and two places that build it are two places that can disagree.
fn visit_path<R>(
    parent: &str,
    index: usize,
    siblings: usize,
    name: &str,
    then: impl FnOnce(String) -> R,
) -> R {
    then(format!("{parent}/{index}of{siblings}:{name}"))
}

/// Guards against a build that marks itself pending forever. Exceeding this is a bug
/// in a widget, not in the scheduler, so it fails loudly rather than hanging.
const MAX_REBUILD_PASSES: usize = 64;

// Numbers the trees, so that ids from different ones never collide.
//
// A `Cell` in a thread-local rather than an atomic, because the whole tree is
// single-threaded and `Rc`-based by DESIGN §3 — an atomic here would buy
// nothing and imply a sharing that does not exist.
//
// `//` and not `///`: rustdoc does not document a macro invocation, and a doc
// comment on one is an `unused_doc_comments` warning — which this workspace
// denies.
thread_local! {
    static NEXT_TREE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The next tree number.
fn next_tree() -> u32 {
    NEXT_TREE.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    })
}

struct Slot {
    generation: u32,
    element: Option<Element>,
}

/// `WidgetKind` is `#[non_exhaustive]`, so this crate must handle a variant it
/// has never heard of. Failing loudly is the only honest option: quietly
/// treating an unknown kind as a leaf would mount a widget that renders nothing
/// and give no clue why.
fn unhandled_kind(widget: &WidgetNode, kind: &WidgetKind<'_>) -> String {
    format!(
        "vieww-element cannot handle WidgetKind::{} (from {widget:?}). \
         A variant was added to WidgetKind without teaching the element tree \
         to mount it.",
        kind.tag()
    )
}

/// The persistent tree of elements, and the scheduler that rebuilds them.
///
/// This is the layer that makes rebuilds cheap. A widget tree is rebuilt and
/// thrown away constantly; the element tree underneath it is *reconciled*, so
/// state, identity and subscriptions survive.
///
/// ```
/// use vieww_element::ElementTree;
/// use vieww_widget::prelude::*;
///
/// let mut tree = ElementTree::new();
/// let label = tree.runtime().signal(String::from("before"));
///
/// let watched = label.clone();
/// tree.mount(Flex::column().children(children![Text::new("static")]));
///
/// // No signal was read during that build, so a write marks nothing pending.
/// watched.set(String::from("after"));
/// assert_eq!(tree.rebuild_pending(), 0);
/// ```
pub struct ElementTree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: Option<ElementId>,
    runtime: Runtime,
    /// Which tree this is, stamped into every id it mints.
    ///
    /// Load-bearing when a [`Runtime`] is shared: the runtime's pending set is
    /// keyed by [`ElementId`], and arena slots are per-tree, so without this two
    /// trees' first elements are the same key. See [`ElementId`].
    tree: u32,
    /// Every element that has durable state.
    ///
    /// Kept as a set rather than discovered by walking, because
    /// [`tick_states`](Self::tick_states) runs every frame an animation is
    /// running and a walk would make an idle screen's cost proportional to the
    /// size of the tree rather than to the number of things actually moving.
    /// States are rare — a widget only creates one if it has something to tear
    /// down — so this stays small.
    stateful: FastSet<ElementId>,
    /// Reused by [`tick_states`](Self::tick_states) and
    /// [`poll_states`](Self::poll_states) for the snapshot of `stateful` they
    /// walk, so a frame does not allocate one — the Vieww standard's
    /// steady-state allocation clause counts every frame's allocations.
    stateful_scratch: Vec<ElementId>,
    /// What to do about a `build` that panics.
    error_policy: ErrorPolicy,
    /// Panics caught out of builds, oldest first.
    errors: Vec<BuildError>,
    /// A monotonic clock for "something changed", ticked by [`Self::touch`].
    ///
    /// Not a frame counter and not a timestamp — it advances once per mutation,
    /// so two changes within one frame get different numbers and a consumer
    /// that syncs part-way through is still correct. See
    /// [`Element::subtree_revision`].
    revision: u64,
    /// How many elements this tree has ever unmounted.
    ///
    /// A consumer mirroring the tree cannot learn about a removal from
    /// [`Element::subtree_revision`] — the element it would have read the
    /// revision from is the one that is gone. So it has to sweep, and sweeping
    /// is the one part of a sync proportional to tree size rather than to what
    /// changed. This number lets it skip the sweep in O(1) whenever nothing has
    /// died, which is almost every frame.
    deaths: u64,
}

impl ElementTree {
    /// An empty tree with a fresh reactive runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::with_runtime(Runtime::new())
    }

    /// An empty tree sharing an existing runtime, so signals created before the
    /// tree existed still drive it.
    #[must_use]
    pub fn with_runtime(runtime: Runtime) -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            root: None,
            runtime,
            tree: next_tree(),
            stateful: FastSet::default(),
            stateful_scratch: Vec::new(),
            error_policy: ErrorPolicy::default(),
            errors: Vec::new(),
            revision: 0,
            deaths: 0,
        }
    }

    /// How many elements this tree has ever unmounted.
    ///
    /// Only the *change* in this number is meaningful. A consumer keeping its
    /// own mirror records it after sweeping, and can skip the next sweep
    /// entirely while it stays equal — see [`Self::revision`] for the other
    /// half of the same job.
    #[must_use]
    pub const fn deaths(&self) -> u64 {
        self.deaths
    }

    /// The tree's change clock.
    ///
    /// Record it after a sync; compare a later
    /// [`Element::subtree_revision`] against it to find what has moved since.
    /// A subtree whose revision is not greater has not changed.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Stamp an element and its ancestors as changed.
    ///
    /// Called from every path that replaces a widget or a child list.
    ///
    /// Costs one walk to the root per change — O(depth), against the O(tree)
    /// per *frame* it removes downstream. Trees are shallow and wide, and the
    /// early-out in [`Self::update`] means the number of changes per frame is
    /// already proportional to what moved.
    ///
    /// A `u64` counter cannot wrap in any run of a program: at one change per
    /// nanosecond it lasts five hundred years, so there is no rollover case to
    /// get wrong and none is written.
    fn touch(&mut self, id: ElementId) {
        self.revision += 1;
        let now = self.revision;
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            if !self.is_alive(current) {
                return;
            }
            let node = self.node_mut(current);
            node.subtree_revision = now;
            cursor = node.parent;
        }
    }

    // ------------------------------------------------------------ error policy

    /// What this tree does about a `build` that panics.
    #[must_use]
    pub const fn error_policy(&self) -> ErrorPolicy {
        self.error_policy
    }

    /// Choose what happens when a `build` panics.
    ///
    /// Defaults to [`ErrorPolicy::Placeholder`] in debug and
    /// [`ErrorPolicy::Propagate`] in release. Set it explicitly in a test that
    /// asserts on either behaviour, so the assertion does not silently change
    /// meaning under `cargo test --release`.
    pub fn set_error_policy(&mut self, policy: ErrorPolicy) {
        self.error_policy = policy;
    }

    /// Panics caught out of builds so far, oldest first.
    #[must_use]
    pub fn build_errors(&self) -> &[BuildError] {
        &self.errors
    }

    /// Take the recorded build errors, leaving the list empty.
    ///
    /// For a caller that reports each failure once — an overlay, or a test
    /// asserting a frame produced no new ones.
    pub fn take_build_errors(&mut self) -> Vec<BuildError> {
        std::mem::take(&mut self.errors)
    }

    /// The runtime owning this tree's signals. Create signals from it.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// The root element, once something has been mounted.
    #[must_use]
    pub const fn root(&self) -> Option<ElementId> {
        self.root
    }

    /// Number of live elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.element.is_some())
            .count()
    }

    /// `true` if nothing is mounted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrow an element, if the id is still live.
    #[must_use]
    pub fn get(&self, id: ElementId) -> Option<&Element> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.element.as_ref()
    }

    /// `true` if the id still addresses a live element.
    #[must_use]
    pub fn is_alive(&self, id: ElementId) -> bool {
        self.get(id).is_some()
    }

    fn node(&self, id: ElementId) -> &Element {
        self.get(id)
            .unwrap_or_else(|| panic!("element {id} is not live"))
    }

    fn node_mut(&mut self, id: ElementId) -> &mut Element {
        let slot = self
            .slots
            .get_mut(id.index as usize)
            .unwrap_or_else(|| panic!("element {id} is out of range"));
        assert!(
            slot.generation == id.generation,
            "element {id} has been unmounted"
        );
        slot.element
            .as_mut()
            .unwrap_or_else(|| panic!("element {id} is not live"))
    }

    /// Mount `widget` as the root, replacing anything already mounted.
    pub fn mount(&mut self, widget: impl Into<WidgetNode>) -> ElementId {
        if let Some(old) = self.root.take() {
            self.unmount(old);
        }
        let id = self.mount_child(None, widget.into(), InheritedScope::new(), 0);
        self.root = Some(id);
        id
    }

    /// Tear the whole tree down, disposing every element's state.
    ///
    /// The opposite of [`set_root`](Self::set_root), which *reconciles*. This
    /// one guarantees nothing is reused, which is exactly what a hot reload
    /// needs when the shape of the application's state has changed: adopting an
    /// old allocation into code that reads it with a new layout is undefined
    /// behaviour rather than a glitch, so the tree goes instead.
    ///
    /// Also the honest way to test that state *does* survive an ordinary
    /// reload — a test that never tears anything down cannot tell the
    /// difference between surviving and never having been at risk.
    pub fn clear(&mut self) {
        if let Some(root) = self.root.take() {
            self.unmount(root);
        }
        // `unmount` already disposed each state and cleared its dependencies;
        // what is left is the bookkeeping a caller would otherwise inherit.
        self.stateful.clear();
        self.errors.clear();
    }

    /// Re-supply the whole tree from a new root widget.
    ///
    /// Reconciles rather than remounting: an element whose widget still matches
    /// by type and key keeps its identity and state.
    pub fn set_root(&mut self, widget: impl Into<WidgetNode>) -> ElementId {
        let Some(root) = self.root else {
            return self.mount(widget);
        };
        let id = self.update_child(None, Some(root), widget.into(), &InheritedScope::new(), 0);
        self.root = Some(id);
        id
    }

    // ---------------------------------------------------------------- mounting

    fn allocate(&mut self, element: impl FnOnce(ElementId) -> Element) -> ElementId {
        let id = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                // The generation was already bumped on free, so any id handed
                // out before this reuse now compares unequal.
                ElementId::new(self.tree, index, slot.generation)
            }
            None => {
                let index = u32::try_from(self.slots.len()).expect("element arena overflowed u32");
                self.slots.push(Slot {
                    generation: 0,
                    element: None,
                });
                ElementId::new(self.tree, index, 0)
            }
        };
        self.slots[id.index as usize].element = Some(element(id));
        id
    }

    fn mount_child(
        &mut self,
        parent: Option<ElementId>,
        widget: WidgetNode,
        scope: InheritedScope,
        depth: usize,
    ) -> ElementId {
        // Boxed by the widget, shared by the tree: the animate phase mutates
        // this state and the element's own build reads it.
        let state: Option<Rc<RefCell<dyn ElementState>>> = widget
            .create_state()
            .map(|state| Rc::new(RefCell::new(state)) as Rc<RefCell<dyn ElementState>>);
        let id = self.allocate(|id| Element {
            id,
            widget: widget.clone(),
            parent,
            children: Vec::new(),
            depth,
            pending: false,
            scope: scope.clone(),
            state,
            build_count: 0,
            builds_at_mark: 0,
            // Zero, then stamped below once the children exist. A brand-new
            // element is always "changed" relative to any revision a consumer
            // recorded before it was mounted, and the stamp at the end of this
            // function is what makes that true for its ancestors too.
            subtree_revision: 0,
            provision: None,
            child_scope: None,
        });

        if let Some(state) = self.node(id).state.clone() {
            self.stateful.insert(id);
            state.borrow_mut().mounted();
        }

        let children = self.build_children(id, &widget, &scope, depth);
        self.node_mut(id).children = children;
        // After the children, not before: `touch` walks upward, so stamping
        // once at the end covers this element and every ancestor in one pass
        // regardless of how deep the subtree just mounted goes.
        self.touch(id);
        id
    }

    /// Produce the child elements for a freshly mounted element.
    fn build_children(
        &mut self,
        id: ElementId,
        widget: &WidgetNode,
        scope: &InheritedScope,
        depth: usize,
    ) -> Vec<ElementId> {
        match widget.kind() {
            WidgetKind::Composed => {
                let built = self.run_build(id, widget, scope, depth);
                vec![self.mount_child(Some(id), built, scope.clone(), depth + 1)]
            }
            WidgetKind::RenderLeaf => Vec::new(),
            WidgetKind::Inherited(child) => {
                let child_scope = self.child_scope(id, widget, scope, depth);
                vec![self.mount_child(Some(id), child.clone(), child_scope, depth + 1)]
            }
            WidgetKind::RenderSingleChild(child) => {
                vec![self.mount_child(Some(id), child.clone(), scope.clone(), depth + 1)]
            }
            WidgetKind::RenderMultiChild(children) => children
                .iter()
                .cloned()
                .map(|child| self.mount_child(Some(id), child, scope.clone(), depth + 1))
                .collect(),
            other => panic!("{}", unhandled_kind(widget, &other)),
        }
    }

    /// Run a composed widget's `build`, attributing every signal it reads to
    /// this element.
    ///
    /// Under a catching [`ErrorPolicy`] a panic out of the build is caught here
    /// and becomes whatever that policy substitutes — an
    /// [`ErrorPlaceholder`](vieww_widget::ErrorPlaceholder), or the
    /// application's own. This is the only place any `build` runs — mount and
    /// rebuild both come through it — so it is the only place that needs the
    /// guard.
    fn run_build(
        &mut self,
        id: ElementId,
        widget: &WidgetNode,
        scope: &InheritedScope,
        depth: usize,
    ) -> WidgetNode {
        // Drop last frame's subscriptions first: a branch this build no longer
        // takes must stop waking the element, or a stale dependency keeps it
        // rebuilding forever.
        self.runtime.clear_dependencies(id);

        let mut ctx = BuildContext::at(scope.clone(), depth).with_reader(id.to_handle());
        if let Some(state) = self.node(id).state.clone() {
            ctx = ctx.with_state(state);
        }

        let built = if self.catches_panics(widget) {
            // Recorded *before* the push so the restore below is against the
            // depth this build started at, whatever it left behind.
            let outer_depth = self.runtime.tracking_depth();
            self.runtime.push_tracking(id);

            // `AssertUnwindSafe` is the honest annotation rather than a
            // suppressed warning. Nothing here is `UnwindSafe` — the tree is
            // `Rc`/`RefCell` throughout by DESIGN §3 — and the guarantee that
            // matters is not the type-system one. It is that the two pieces of
            // shared state a half-finished build can corrupt are both repaired
            // on the way out: the tracking stack, immediately below, and the
            // element's subscriptions, which were cleared above and are simply
            // whatever the partial build managed to record.
            let result = panic::catch_unwind(AssertUnwindSafe(|| widget.build(&ctx)));

            // Both arms. The push and the pop sit either side of arbitrary user
            // code, so the pop is exactly the step a panic skips — and a
            // tracking stack left one frame deep attributes every signal read
            // in the rest of the frame to an element that is not building.
            self.runtime.truncate_tracking(outer_depth);

            match result {
                Ok(built) => built,
                Err(payload) => self.record_build_error(id, widget, &*payload),
            }
        } else {
            self.runtime.push_tracking(id);
            let built = widget.build(&ctx);
            self.runtime.pop_tracking();
            built
        };

        self.node_mut(id).build_count += 1;
        built
    }

    /// Whether a panic out of this widget's build should be caught.
    ///
    /// `false` for a widget standing in for a failure, which is what stops one
    /// from recurring: the substitute is mounted like any other widget, so if
    /// *it* could fail and be replaced by another substitute, a build that
    /// panics unconditionally would recurse until the stack ran out — a stack
    /// overflow reported in place of the original panic, which is strictly worse
    /// than the crash this feature exists to prevent.
    ///
    /// **Asked of the widget, not decided by its type.** This was
    /// `downcast_ref::<ErrorPlaceholder>()`, so the guard covered vieww's own
    /// placeholder and nothing else — an application supplying its own through
    /// [`ErrorPolicy::Custom`] would have had no protection at all.
    fn catches_panics(&self, widget: &WidgetNode) -> bool {
        self.error_policy.catches() && widget.catches_panics()
    }

    /// Record a caught build panic and produce what to mount instead.
    fn record_build_error(
        &mut self,
        id: ElementId,
        widget: &WidgetNode,
        payload: &(dyn Any + Send),
    ) -> WidgetNode {
        let error = BuildError {
            element: id,
            widget_name: widget.debug_name(),
            message: BuildError::message_from(payload),
        };
        // Built before the error is moved into the list, so a custom placeholder
        // sees the same record the application will later read off the tree.
        let placeholder = self.error_policy.placeholder_for(&error);
        self.errors.push(error);
        placeholder
    }

    /// The scope an [`Inherited`](vieww_widget::Inherited) widget publishes to
    /// its subtree.
    /// The scope an `Inherited` element hands its children.
    ///
    /// # Why this is not simply `scope.push(value)`
    ///
    /// Because that mints a new scope, and a new scope means every element
    /// below fails [`Self::update`]'s `scope.ptr_eq` early-out and rebuilds —
    /// the whole subtree, whatever it actually read. Harmless for a theme
    /// somebody toggles once, ruinous for `ScrollMetrics`, which is republished
    /// on every frame of a scroll.
    ///
    /// So the element keeps its [`Provision`](vieww_widget::Provision) and the *value inside it* moves.
    /// The child scope is then the same `Rc` frame after frame, the early-out
    /// holds, and only the elements that read the value are marked — which is
    /// what `Provision::republish` returns and what this marks pending.
    ///
    /// The cached scope is discarded when the scope *above* changes, because a
    /// provision's position in the chain is part of what it means.
    fn child_scope(
        &mut self,
        id: ElementId,
        widget: &WidgetNode,
        scope: &InheritedScope,
        depth: usize,
    ) -> InheritedScope {
        if !widget.publishes() {
            return scope.clone();
        }

        let cached = {
            let node = self.node(id);
            node.provision.as_ref().and_then(|provision| {
                node.child_scope
                    .as_ref()
                    .filter(|cached| cached.parent_is(scope))
                    .map(|cached| (Rc::clone(provision), cached.clone()))
            })
        };

        if let Some((provision, cached)) = cached {
            let tree = self.tree;
            for handle in widget.publish_into(&provision) {
                self.mark_pending(ElementId::from_handle(tree, handle));
            }
            return cached;
        }

        // First publication here, or the chain above moved. Build a fresh
        // provision through the widget, which is the only place `T` is still
        // known.
        let Some(child_ctx) = widget.publish_inherited(&BuildContext::at(scope.clone(), depth))
        else {
            return scope.clone();
        };
        let child_scope = child_ctx.scope().clone();
        let node = self.node_mut(id);
        node.provision = child_scope.head_provision();
        node.child_scope = Some(child_scope.clone());
        child_scope
    }

    // ----------------------------------------------------------- reconciliation

    /// Reconcile one child position.
    ///
    /// This is the single rule that decides whether state survives: an element
    /// is reused when the new widget matches by type **and** key, and replaced
    /// otherwise.
    fn update_child(
        &mut self,
        parent: Option<ElementId>,
        old: Option<ElementId>,
        widget: WidgetNode,
        scope: &InheritedScope,
        depth: usize,
    ) -> ElementId {
        match old {
            Some(old_id) if self.node(old_id).widget.can_update(&widget) => {
                self.update(old_id, widget, scope, depth);
                old_id
            }
            Some(old_id) => {
                // Type or key changed: this is a different thing in the same
                // position, so its state must not carry over.
                self.unmount(old_id);
                self.mount_child(parent, widget, scope.clone(), depth)
            }
            None => self.mount_child(parent, widget, scope.clone(), depth),
        }
    }

    /// Reconfigure a live element from a new widget of the same type and key.
    fn update(&mut self, id: ElementId, widget: WidgetNode, scope: &InheritedScope, depth: usize) {
        let unchanged = {
            let node = self.node(id);
            // Identity first — free, and the case a cloned subtree takes. Then
            // the widget's own claim, for a region its parent rebuilt and
            // reconstructed: see `Widget::same_configuration`, which is `false`
            // unless a widget opts in.
            (node.widget.ptr_eq(&widget) || node.widget.same_configuration(&widget))
                && node.scope.ptr_eq(scope)
                && !node.pending
        };
        if unchanged {
            // The same widget instance — or one that says it describes the same
            // thing — under the same scope cannot produce anything different,
            // so the whole subtree is skipped without being walked. This is what
            // makes a rebuild proportional to what changed rather than to tree
            // size.
            return;
        }

        let state = {
            let node = self.node_mut(id);
            node.widget = widget.clone();
            node.scope = scope.clone();
            node.depth = depth;
            node.state.clone()
        };
        // Past the early-out, so something about this element's description
        // changed. Stamped here rather than after the reconcile below, because
        // the widget swap alone is enough to change what the render tree needs
        // — a `Container` with a new colour reconciles to the same children.
        self.touch(id);

        // Before the rebuild, so that whatever the state decides — a new
        // animation target, most of all — is visible to the build that follows.
        // This is the element-level update hook (`widget_updated`), and it is where state changes
        // belong: `build` itself has to stay side-effect free.
        if let Some(state) = state {
            state.borrow_mut().widget_updated(&widget);
        }

        match widget.kind() {
            WidgetKind::Composed => self.rebuild(id),
            WidgetKind::RenderLeaf => {
                self.replace_children(id, Vec::new());
            }
            WidgetKind::Inherited(child) => {
                let child_scope = self.child_scope(id, &widget, scope, depth);
                self.reconcile_children(id, vec![child.clone()], &child_scope, depth + 1);
            }
            WidgetKind::RenderSingleChild(child) => {
                self.reconcile_children(id, vec![child.clone()], scope, depth + 1);
            }
            WidgetKind::RenderMultiChild(children) => {
                self.reconcile_children(id, children.to_vec(), scope, depth + 1);
            }
            other => panic!("{}", unhandled_kind(&widget, &other)),
        }
    }

    /// Re-run a composed element's build and reconcile the result.
    fn rebuild(&mut self, id: ElementId) {
        self.node_mut(id).pending = false;
        self.runtime.clear_pending(id);

        let (widget, scope, depth) = {
            let node = self.node(id);
            (node.widget.clone(), node.scope.clone(), node.depth)
        };

        let built = self.run_build(id, &widget, &scope, depth);
        self.reconcile_children(id, vec![built], &scope, depth + 1);
    }

    /// Reconcile an element's children against a new list of widgets.
    ///
    /// Follows one shape: sync the matching prefix, sync the matching
    /// suffix, then match what remains in the middle by key. Doing the ends
    /// first means the common edits — appending, prepending, changing one item
    /// — never build the key map at all.
    fn reconcile_children(
        &mut self,
        parent: ElementId,
        widgets: Vec<WidgetNode>,
        scope: &InheritedScope,
        depth: usize,
    ) {
        let old = self.node(parent).children.clone();
        let mut result: Vec<Option<ElementId>> = vec![None; widgets.len()];

        // --- sync from the top while positions still match.
        let mut head = 0;
        while head < old.len() && head < widgets.len() {
            if !self.node(old[head]).widget.can_update(&widgets[head]) {
                break;
            }
            result[head] = Some(self.update_child(
                Some(parent),
                Some(old[head]),
                widgets[head].clone(),
                scope,
                depth,
            ));
            head += 1;
        }

        // --- sync from the bottom while positions still match.
        let mut old_tail = old.len();
        let mut new_tail = widgets.len();
        while old_tail > head && new_tail > head {
            if !self
                .node(old[old_tail - 1])
                .widget
                .can_update(&widgets[new_tail - 1])
            {
                break;
            }
            old_tail -= 1;
            new_tail -= 1;
        }

        // --- the middle: match old keyed elements to new keyed widgets.
        let mut keyed: HashMap<Key, ElementId> = HashMap::new();
        let mut unkeyed: Vec<ElementId> = Vec::new();
        for &old_id in &old[head..old_tail] {
            match self.node(old_id).widget.key().cloned() {
                Some(key) => {
                    keyed.insert(key, old_id);
                }
                None => unkeyed.push(old_id),
            }
        }

        let mut unkeyed_cursor = 0;
        for index in head..new_tail {
            let widget = widgets[index].clone();
            let candidate = match widget.key() {
                // A keyed widget only ever matches its own key, wherever that
                // element moved to. This is what lets a reordered list keep its
                // state instead of shuffling it between rows.
                Some(key) => keyed.remove(key),
                None => loop {
                    let Some(&next) = unkeyed.get(unkeyed_cursor) else {
                        break None;
                    };
                    unkeyed_cursor += 1;
                    if self.node(next).widget.can_update(&widget) {
                        break Some(next);
                    }
                    // Not compatible and nothing else will match it here.
                    self.unmount(next);
                },
            };
            result[index] = Some(self.update_child(Some(parent), candidate, widget, scope, depth));
        }

        // --- anything left over in the middle is gone.
        for old_id in keyed.into_values() {
            self.unmount(old_id);
        }
        for &old_id in &unkeyed[unkeyed_cursor..] {
            self.unmount(old_id);
        }

        // --- the suffix, now that the middle has released its claims. The
        // bottom sync paired these off 1:1, so they zip.
        for (old_index, index) in (old_tail..).zip(new_tail..widgets.len()) {
            result[index] = Some(self.update_child(
                Some(parent),
                old.get(old_index).copied(),
                widgets[index].clone(),
                scope,
                depth,
            ));
        }

        let children: Vec<ElementId> = result
            .into_iter()
            .map(|id| id.expect("every child position was reconciled"))
            .collect();
        self.replace_children(parent, children);
    }

    /// The number of children above which membership is worth a hash set.
    ///
    /// Below it, `Vec::contains` on a short slice of `u64`-sized ids beats
    /// building a `HashSet` — the set costs an allocation and a hash per
    /// element, the scan costs a handful of compares that stay in cache. Above
    /// it the scan is quadratic: a list view with a thousand children was doing
    /// half a million comparisons on every reconcile, which is the shape this
    /// threshold exists to cut off.
    ///
    /// Sixteen is where the two cross for integer keys in the usual
    /// measurements; the exact number matters far less than that neither branch
    /// is the only branch.
    const HASH_STALE_CHECK_ABOVE: usize = 16;

    fn replace_children(&mut self, parent: ElementId, children: Vec<ElementId>) {
        let unchanged = self.node(parent).children == children;

        let stale: Vec<ElementId> = if children.len() > Self::HASH_STALE_CHECK_ABOVE {
            let fresh: FastSet<ElementId> = children.iter().copied().collect();
            self.node(parent)
                .children
                .iter()
                .copied()
                .filter(|old| !fresh.contains(old))
                .collect()
        } else {
            self.node(parent)
                .children
                .iter()
                .copied()
                .filter(|old| !children.contains(old))
                .collect()
        };

        for old in stale {
            if self.is_alive(old) {
                self.unmount(old);
            }
        }
        self.node_mut(parent).children = children;

        // Only when the list really moved. A composed element that rebuilds to
        // the same single child — the overwhelmingly common case — reaches here
        // every rebuild, and stamping it would make the revision useless as a
        // skip signal for exactly the subtree that did not change.
        //
        // Its own `update` has already stamped it if its *widget* changed, so
        // nothing is lost by being precise here.
        if !unchanged {
            self.touch(parent);
        }
    }

    // -------------------------------------------------------------- unmounting

    /// Tear down an element and everything beneath it.
    ///
    /// `dispose` runs before the children are torn down, matching the standard
    /// order, so a parent's cleanup can still reach its children.
    fn unmount(&mut self, id: ElementId) {
        if !self.is_alive(id) {
            return;
        }

        if let Some(state) = self.node(id).state.clone() {
            state.borrow_mut().dispose();
        }
        self.stateful.remove(&id);

        let children = self.node(id).children.clone();
        for child in children {
            self.unmount(child);
        }

        self.runtime.clear_dependencies(id);
        self.runtime.clear_pending(id);

        let slot = &mut self.slots[id.index as usize];
        slot.element = None;
        // Bump on free, not on reuse, so every id issued for this slot so far
        // is invalidated the moment the element goes away.
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        self.deaths += 1;
    }

    // --------------------------------------------------------------- scheduling

    /// Mark an element for rebuild on the next [`rebuild_pending`](Self::rebuild_pending).
    pub fn mark_pending(&mut self, id: ElementId) {
        if self.is_alive(id) {
            self.node_mut(id).pending = true;
            self.runtime.mark_pending(id);
        }
    }

    /// How many elements are waiting to rebuild.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        // This tree's share. The runtime's set is shared with any other tree on
        // the same runtime — a second window — and reporting its total here
        // would tell a caller that *this* window owes a frame when the work
        // belongs to another one.
        self.runtime
            .pending_ids()
            .into_iter()
            .filter(|id| id.tree() == self.tree)
            .count()
    }

    /// *Which* elements are waiting to rebuild, as `(element, widget name)`.
    ///
    /// # Why a count was not enough
    ///
    /// Two attempts at the `needs_frame` gap were written and reverted, and both
    /// diagnoses came from reading the source. What neither could establish is
    /// the thing the second revert then named as the precondition for a third
    /// attempt: *something in the scrolling path leaves an element pending every
    /// frame*, so a predicate of the form "is anything pending" is standing rather
    /// than transient. [`pending_count`](Self::pending_count) says one element is
    /// pending and cannot say which, which is exactly the missing half.
    ///
    /// Unsorted, because the pending set is a `HashSet` and imposing an order here
    /// would cost a sort on a path that is only ever read by an instrument.
    /// Names, not `debug_properties`: this is read once per frame while
    /// something is wrong, and formatting a widget's fields per element per
    /// frame is how an instrument becomes the fault it was built to find.
    #[must_use]
    pub fn pending_widgets(&self) -> Vec<(ElementId, &'static str)> {
        self.runtime
            .pending_ids()
            .into_iter()
            .filter(|&id| self.is_alive(id))
            .map(|id| (id, self.node(id).widget.debug_name()))
            .collect()
    }

    // ---------------------------------------------------------------- animation

    /// Advance every element's durable state to `now`, marking pending the ones
    /// whose value changed.
    ///
    /// Returns `true` if anything is still animating and the frame after this
    /// one is worth producing.
    ///
    /// # Why the tree does the marking
    ///
    /// An [`ElementState`] lives in `vieww-widget`, which sits *below* this
    /// crate and cannot name an [`ElementId`] or the [`Runtime`] — so it has no
    /// way to say "rebuild me". It answers `true` instead, and the tree, which
    /// knows whose state it just ticked, marks it. That keeps the widget layer
    /// free of the reactive graph, and it is why `tick` returns a `bool` rather
    /// than taking a callback.
    pub fn tick_states(&mut self, now: Duration) -> bool {
        let mut stateful = std::mem::take(&mut self.stateful_scratch);
        stateful.clear();
        stateful.extend(self.stateful.iter().copied());
        let mut animating = false;

        for &id in &stateful {
            let Some(state) = self.get(id).and_then(|node| node.state.clone()) else {
                // Unmounted since the set was built, or never had state.
                self.stateful.remove(&id);
                continue;
            };
            // The borrow is released before `mark_pending`: marking can run
            // arbitrary code in a later phase, and holding a state's borrow
            // across it would panic somewhere far from the cause.
            let (changed, still_going) = {
                let mut state = state.borrow_mut();
                (state.tick(now), state.is_animating())
            };
            if changed {
                self.mark_pending(id);
            }
            animating |= still_going;
        }
        self.stateful_scratch = stateful;
        animating
    }

    /// Mark pending every element whose state a *handler* wrote since the last
    /// frame. Returns how many there were.
    ///
    /// The counterpart to [`tick_states`](Self::tick_states) for the changes
    /// that time did not cause: a press highlight going on, a hover, a focus
    /// ring. A gesture handler runs during input dispatch, where it can reach
    /// its own [`ElementState`] but not the reactive graph — same asymmetry as
    /// `tick`, same answer, one hook later.
    ///
    /// Call it after gestures have been delivered and before the rebuild, so
    /// that a press that arrived this frame is shown by this frame rather than
    /// the next one. Costs one walk of the elements that *have* state, which is
    /// the same set `tick_states` walks and is small by construction.
    pub fn poll_states(&mut self) -> usize {
        let mut stateful = std::mem::take(&mut self.stateful_scratch);
        stateful.clear();
        stateful.extend(self.stateful.iter().copied());
        let mut marked = 0;

        for &id in &stateful {
            let Some(state) = self.get(id).and_then(|node| node.state.clone()) else {
                self.stateful.remove(&id);
                continue;
            };
            // Released before `mark_pending`, for the reason `tick_states` gives.
            let pending = state.borrow_mut().take_pending();
            if pending {
                self.mark_pending(id);
                marked += 1;
            }
        }
        self.stateful_scratch = stateful;
        marked
    }

    /// `true` if any element's state still needs frames.
    ///
    /// Asked *after* the build as well, because a build is where an implicit
    /// animation finds out it has a new target: the description changed this
    /// frame, and the motion starts on the next one.
    #[must_use]
    pub fn has_animating_states(&self) -> bool {
        self.stateful.iter().any(|&id| {
            self.get(id)
                .and_then(|node| node.state.as_ref())
                .is_some_and(|state| state.borrow().is_animating())
        })
    }

    /// How many elements carry durable state.
    #[must_use]
    pub fn stateful_count(&self) -> usize {
        self.stateful.len()
    }

    /// Rebuild every pending element, parents before children. Returns how many
    /// elements actually rebuilt.
    ///
    /// This is the scheduler. Writing a signal never rebuilds anything by
    /// itself — it only marks. Everything happens here, once, which is what
    /// lets a frame absorb any number of writes and what keeps a build from
    /// re-entering another build.
    ///
    /// Depth order matters: rebuilding a parent may replace or unmount a child
    /// that was itself pending, and doing parents first means that child is never
    /// rebuilt just to be thrown away.
    ///
    /// # Panics
    ///
    /// If rebuilding never settles. That means a `build` is writing to a signal
    /// it also reads, which the build contract forbids.
    pub fn rebuild_pending(&mut self) -> usize {
        // **Derivations first.** A memo whose inputs changed has to hold its
        // new value before anything builds against it, and re-deriving marks
        // the elements that read it — so this both feeds the pass below and
        // adds to its queue. Free when nothing is stale.
        self.runtime.settle_memos();

        let mut rebuilt = 0;
        let budget = MAX_REBUILD_PASSES * (self.len() + 1);

        while let Some(id) = self.next_pending() {
            assert!(
                rebuilt < budget,
                "rebuild did not settle after {rebuilt} rebuilds — \
                 a build is writing to a signal it reads; build must be side-effect free"
            );
            self.rebuild(id);
            rebuilt += 1;
        }

        rebuilt
    }

    /// The shallowest pending element still worth rebuilding.
    ///
    /// Re-read before every rebuild rather than snapshotting the queue once,
    /// because rebuilding a parent reconciles its children — which cleans any
    /// child that was independently pending, and can unmount it outright. A
    /// snapshot would then rebuild that child a second time against a widget
    /// its parent had already replaced.
    fn next_pending(&mut self) -> Option<ElementId> {
        // **Only this tree's ids.** The runtime's pending set is shared with
        // every other tree using the same runtime — a second window — and both
        // the search below and the sweep further down would otherwise reach
        // into a tree this one knows nothing about.
        let pending: Vec<ElementId> = self
            .runtime
            .pending_ids()
            .into_iter()
            .filter(|id| id.tree() == self.tree)
            .collect();
        if pending.is_empty() {
            return None;
        }

        let shallowest = pending
            .iter()
            .copied()
            .filter(|&id| self.is_alive(id))
            .min_by_key(|&id| (self.node(id).depth, id));

        if shallowest.is_none() {
            // Everything left was unmounted while pending. Drop it so the loop
            // terminates instead of spinning on ids that can never rebuild.
            for id in pending {
                self.runtime.clear_pending(id);
            }
        }

        shallowest
    }

    // -------------------------------------------------------------------- debug

    /// Every state under `root` that has something to say, keyed by where it
    /// sits in the tree.
    ///
    /// # What this is for
    ///
    /// Carrying a screen's state across a **remount** — the case a live-reload
    /// editor has and nothing else does. Reconciliation cannot help there: the
    /// new widgets come from a freshly compiled library, so their `TypeId`s are
    /// different and every element is a new element. See
    /// [`ElementState::snapshot`](vieww_widget::ElementState::snapshot).
    ///
    /// # The key, and what it can and cannot survive
    ///
    /// A path from `root` down, one step per level, each step naming the child's
    /// **index, how many siblings it had, and its `debug_name`** —
    /// `/0of2:Column/1of3:TabBar`. All three matter:
    ///
    /// * the **name** alone cannot tell two sibling tab bars apart;
    /// * the **index** alone would move a tab bar's state onto a switch;
    /// * and without the **sibling count**, inserting a row above a list of
    ///   controls shifts every one of them down by one and each then inherits
    ///   the state of the control that used to be above it — which is worse
    ///   than restoring nothing, because it is wrong and looks deliberate.
    ///
    /// The count is what makes an insertion invalidate the whole level rather
    /// than silently rotate it. That is the conservative direction, and it is
    /// the right one: a screen that comes back fresh is a mild annoyance, and a
    /// screen that comes back holding somebody else's values is a bug report
    /// about the editor.
    ///
    /// So it survives everything that does not change the *shape* of the tree —
    /// editing a colour, a string, a padding, a handler, a label — and nothing
    /// that does.
    #[must_use]
    pub fn snapshot_states(&self, root: ElementId) -> Vec<(String, String)> {
        let mut out = Vec::new();
        self.walk_states(root, String::new(), &mut |path, element| {
            if let Some(state) = element.state() {
                if let Some(saved) = state.borrow().snapshot() {
                    out.push((path.to_owned(), saved));
                }
            }
        });
        out
    }

    /// Put a [`snapshot_states`](Self::snapshot_states) back into the tree that
    /// replaced the one it was taken from.
    ///
    /// Returns how many states were restored. A path with no match is skipped
    /// in silence, which is the ordinary case after an edit that moved
    /// something: the count is what tells a caller whether to say "restored" or
    /// to say nothing.
    pub fn restore_states(&mut self, root: ElementId, snapshot: &[(String, String)]) -> usize {
        if snapshot.is_empty() {
            return 0;
        }
        // Collected first, because the walk borrows the tree and the restore
        // mutates a state through an `Rc` the tree also holds.
        let mut found: Vec<(String, Rc<RefCell<dyn ElementState>>)> = Vec::new();
        self.walk_states(root, String::new(), &mut |path, element| {
            if let Some(state) = element.state() {
                found.push((path.to_owned(), Rc::clone(state)));
            }
        });

        let mut restored = 0;
        for (path, state) in found {
            let Some((_, saved)) = snapshot.iter().find(|(key, _)| *key == path) else {
                continue;
            };
            if state.borrow_mut().restore(saved) {
                restored += 1;
            }
        }
        if restored > 0 {
            // The restored values have to reach the screen, and a state written
            // outside a build marks nothing on its own — that is what
            // `take_pending` is for, and it is polled once per frame. Marking
            // the root pending rebuilds the subtree that just changed under it.
            self.mark_pending(root);
        }
        restored
    }

    /// Depth-first, handing each element the path that identifies it.
    fn walk_states(&self, id: ElementId, path: String, visit: &mut impl FnMut(&str, &Element)) {
        let Some(element) = self.get(id) else { return };
        visit(&path, element);
        let siblings = element.children.len();
        for (index, &child) in element.children.iter().enumerate() {
            let name = self
                .get(child)
                .map_or("?", |child| child.widget.debug_name());
            visit_path(&path, index, siblings, name, |next| {
                self.walk_states(child, next, visit);
            });
        }
    }

    /// Render the element tree as an indented string, with ids and build counts.
    #[must_use]
    pub fn debug_tree(&self) -> String {
        let Some(root) = self.root else {
            return String::from("<empty>");
        };
        let mut out = String::new();
        self.write_node(&mut out, root, "", true, true);
        out.trim_end().to_owned()
    }

    fn write_node(&self, out: &mut String, id: ElementId, prefix: &str, last: bool, root: bool) {
        let Some(node) = self.get(id) else {
            return;
        };

        let connector = if root {
            ""
        } else if last {
            "└─ "
        } else {
            "├─ "
        };
        let pending = if node.pending { " PENDING" } else { "" };
        let _ = writeln!(
            out,
            "{prefix}{connector}{:?} {} builds={}{pending}",
            node.widget, node.id, node.build_count
        );

        let child_prefix = if root {
            String::new()
        } else {
            format!("{prefix}{}", if last { "   " } else { "│  " })
        };

        let last_index = node.children.len().saturating_sub(1);
        for (index, &child) in node.children.iter().enumerate() {
            self.write_node(out, child, &child_prefix, index == last_index, false);
        }
    }

    /// Every live element, in depth-first order from the root.
    #[must_use]
    pub fn iter(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.collect(root, &mut out);
        }
        out
    }

    fn collect<'a>(&'a self, id: ElementId, out: &mut Vec<&'a Element>) {
        let Some(node) = self.get(id) else {
            return;
        };
        out.push(node);
        for &child in &node.children {
            self.collect(child, out);
        }
    }

    /// The first element built from a widget with this debug name, depth-first.
    #[must_use]
    pub fn find(&self, debug_name: &str) -> Option<&Element> {
        self.iter()
            .into_iter()
            .find(|element| element.debug_name() == debug_name)
    }

    /// Every element built from a widget with this debug name.
    #[must_use]
    pub fn find_all(&self, debug_name: &str) -> Vec<&Element> {
        self.iter()
            .into_iter()
            .filter(|element| element.debug_name() == debug_name)
            .collect()
    }

    // ------------------------------------------------------------- inspection

    /// Start a new rebuild-measurement window.
    ///
    /// Every element's build count is remembered, and
    /// [`hotspots`](Self::hotspots) then reports against that baseline rather
    /// than against the whole session. Call it once a second, or on whatever
    /// interval the question "what is rebuilding *now*" is being asked over.
    ///
    /// Without this, lifetime totals answer a different and much less useful
    /// question: they rank by how long an element has existed, so the root of a
    /// screen that has been open for a minute outranks the row that is
    /// thrashing right now.
    pub fn mark_builds(&mut self) {
        for slot in &mut self.slots {
            if let Some(element) = slot.element.as_mut() {
                element.builds_at_mark = element.build_count;
            }
        }
    }

    /// The elements that have rebuilt most since the last
    /// [`mark_builds`](Self::mark_builds), busiest first, capped at `limit`.
    ///
    /// This is the actionable half of an inspector. A [`debug_tree`] dump of a
    /// real screen is hundreds of lines and the interesting part is three of
    /// them; ranking by rebuild count puts those three at the top.
    ///
    /// Elements that have not rebuilt in the window are left out entirely —
    /// including every render leaf, which never builds at all. So an empty
    /// result immediately after a mark means nothing rebuilt, which is the
    /// answer rather than a missing one.
    ///
    /// Ties break towards the deeper element, then by id, so the order is
    /// stable across calls: a list that reshuffled between frames would be
    /// unreadable in exactly the situation it is for.
    ///
    /// [`debug_tree`]: Self::debug_tree
    #[must_use]
    pub fn hotspots(&self, limit: usize) -> Vec<Hotspot> {
        let mut found: Vec<(usize, Hotspot)> = self
            .iter()
            .into_iter()
            .filter(|element| element.builds_since_mark() > 0)
            .map(|element| {
                (
                    element.depth,
                    Hotspot {
                        element: element.id,
                        widget_name: element.debug_name(),
                        builds: element.build_count,
                        recent: element.builds_since_mark(),
                    },
                )
            })
            .collect();

        found.sort_unstable_by(|(left_depth, left), (right_depth, right)| {
            right
                .recent
                .cmp(&left.recent)
                .then(right_depth.cmp(left_depth))
                .then(left.element.cmp(&right.element))
        });
        found.truncate(limit);
        found.into_iter().map(|(_, hotspot)| hotspot).collect()
    }

    /// Total builds across every live element since the last
    /// [`mark_builds`](Self::mark_builds).
    ///
    /// The one number to watch: a frame that rebuilds forty elements when you
    /// changed one thing is the symptom, and [`hotspots`](Self::hotspots) is
    /// where the cause is.
    #[must_use]
    pub fn builds_since_mark(&self) -> u32 {
        self.iter()
            .into_iter()
            .map(Element::builds_since_mark)
            .sum()
    }
}

impl Default for ElementTree {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ElementTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElementTree")
            .field("elements", &self.len())
            .field("pending", &self.pending_count())
            .finish()
    }
}
