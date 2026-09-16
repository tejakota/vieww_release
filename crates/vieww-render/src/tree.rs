use std::fmt::Write as _;

use vieww_foundation::{Constraints, FastMap, FastSet, Offset, Rect, Size, Transform};

use vieww_paint::{Canvas, LayerEffect, LayerId, LayerTree, Scene};
use vieww_text::FontStore;

use crate::intrinsics::{IntrinsicCache, IntrinsicQuery};
use crate::sliver::{SliverConstraints, SliverGeometry};
use crate::{ChildIds, HitTestResult, LayoutCtx, PaintCtx, RenderId, RenderObject};

struct Slot {
    generation: u32,
    node: Option<Node>,
}

/// A repaint boundary's layer, and which boundary encloses it.
///
/// The enclosing id is kept so a sync can tell "this layer moved within its
/// parent" from "this layer has a different parent now" — the first is a
/// transform, the second needs a new layer.
#[derive(Debug, Clone, Copy)]
struct LayerEntry {
    layer: LayerId,
    enclosing: Option<RenderId>,
}

/// A repaint boundary, and how it reaches its enclosing one.
///
/// `PartialEq`/`Debug` are not for any runtime decision — boundary collection
/// caches whole slices of these, and the cache's correctness oracle compares
/// the cached result against the from-scratch one. See `boundaries_uncached`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Boundary {
    id: RenderId,
    enclosing: Option<RenderId>,
    /// What was declared between this boundary and the enclosing one — the
    /// fade, the blend and the clip.
    ///
    /// Between, not from the root — see `collect_boundaries` for why carrying
    /// the running product would apply an ancestor's fade once per generation.
    /// The clip inside it is in the **enclosing boundary's** coordinate space,
    /// the same one `transform` maps out of.
    effect: LayerEffect,
    /// Where this boundary sits **in the enclosing boundary's space**, which is
    /// exactly the transform its layer needs.
    ///
    /// # Why it is recorded here rather than computed later
    ///
    /// `reconcile_layers` used to ask `global_offset(id)` for this, and again
    /// for the enclosing boundary, so every boundary cost two walks up the tree
    /// on every frame. The walk that finds the boundaries **already has this
    /// number**: `origin` accumulates down the recursion and resets to zero at
    /// each boundary, which is the definition of "relative to the enclosing
    /// one". Carrying it out is free.
    ///
    /// It also makes the boundary list *complete*: everything reconciliation
    /// needs is now in this struct, so two equal lists describe two identical
    /// layer trees — which is what lets an unchanged frame skip reconciliation
    /// altogether.
    origin: Offset,
}

/// The retained boundary list for one subtree, with the inputs it was built
/// under. See [`RenderTree::boundaries`] for the design.
#[derive(Clone)]
struct BoundaryCache {
    /// The [`Node::subtree_revision`] the slice was collected at. A matching
    /// stamp means the subtree's configuration has not changed.
    stamp: u64,
    /// The between-boundary effect reaching this subtree. A boundary's
    /// `effect` field is the product of this with what its own ancestors
    /// declare *between* boundaries, so a matching incoming effect is needed.
    effect: LayerEffect,
    /// The origin in the enclosing boundary's space. A clip's `bounds` is
    /// `Rect::from_origin_size(origin, size)`, so a matching origin is needed.
    origin: Offset,
    /// The boundaries collected under this subtree, in paint order.
    boundaries: Vec<Boundary>,
}

struct Node {
    object: Option<Box<dyn RenderObject>>,
    parent: Option<RenderId>,
    children: ChildIds,
    /// Position relative to the parent, assigned by the parent after layout.
    offset: Offset,
    /// Size chosen during the last layout.
    size: Size,
    /// The constraints that produced `size`. Layout is skipped when the same
    /// constraints come back down and nothing below has changed.
    last_constraints: Option<Constraints>,
    needs_layout: bool,
    /// How many times this node has actually run `layout`. Not used by the
    /// framework — it is what turns "only the necessary subtree relayouts"
    /// into something a test can measure.
    layout_count: u32,
    /// Intrinsic answers computed since the last time this node was marked.
    ///
    /// An intrinsic is a pure function of the subtree's configuration, so it
    /// is invalidated by exactly what invalidates layout — see
    /// [`RenderTree::intrinsic`].
    intrinsics: IntrinsicCache,
    /// Whether this node owns a layer, cached from
    /// [`RenderObject::is_repaint_boundary`].
    ///
    /// Cached rather than asked, because `layout` takes the object *out* of the
    /// node while it runs — and a `set_offset` during that layout walks up
    /// through this node looking for a boundary. Reading through the missing
    /// object would answer "not a boundary" and let a repaint escape past one.
    is_boundary: bool,
    /// The nearest repaint-boundary ancestor, and the structure generation it
    /// was worked out in.
    ///
    /// # Why this is cached
    ///
    /// [`mark_needs_paint`](RenderTree::mark_needs_paint) answers exactly one
    /// question — *which layer has to be re-recorded because this node changed*
    /// — by walking up until it finds a boundary. That walk is O(depth), and it
    /// runs once per marked node: a frame that marks a few hundred nodes in a
    /// tree twenty deep does thousands of parent hops to arrive at a handful of
    /// distinct answers. It was 5% of a keystroke frame in the studio.
    ///
    /// The answer only changes when the *shape* of the tree above the node
    /// changes, so it is cached against `structure_generation`, which every
    /// reparenting bumps. A stale entry cannot survive: the generation is
    /// checked, not the pointer.
    boundary_memo: Option<(u64, RenderId)>,
    /// A stamp that advances whenever this subtree's boundary-relevant
    /// configuration changes — structure, object, effect, clip or placement.
    /// Used to short-circuit [`RenderTree::boundaries`] when nothing under here
    /// moved. See `bump_subtree` for what advances it and why one stamp serves
    /// the whole path to the root.
    subtree_revision: u64,
}

/// The tree of render objects: layout, paint and hit testing.
///
/// Like the element tree, this is an arena rather than a tree of owned boxes,
/// so that layout can walk into a child while the parent is mid-`layout`
/// without fighting the borrow checker.
pub struct RenderTree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: Option<RenderId>,
    /// Fonts, for the render objects that shape text during layout.
    ///
    /// Defaults to the *embedded* faces rather than the system's. Loading system
    /// fonts is not free — measured at 1.1s release and 33s debug on a machine
    /// with 2000 faces installed — and a constructor is the wrong place for it.
    /// An application opts in once, at startup, with
    /// [`set_fonts`](Self::set_fonts). See docs/DESIGN.md §12.
    fonts: FontStore,
    /// Total number of actual layout executions. A monotonic counter lets the
    /// frame driver classify layout work without walking the whole tree.
    layout_runs: u64,
    /// Subtree roots awaiting layout.
    ///
    /// A relayout boundary stops the pending walk, which means the root never
    /// learns anything changed — so laying out from the root would skip the
    /// change entirely. The boundary has to be laid out *directly*, against the
    /// constraints it saw last time, which is what this records. Equivalent to
    /// a pending-nodes queue in other engines.
    pending_boundaries: FastSet<RenderId>,
    /// The largest amount by which any object in this tree wants to be
    /// reachable outside its own box. See [`RenderObject::hit_bounds`].
    ///
    /// # Why a tree-wide maximum rather than a rect per node
    ///
    /// Hit testing prunes by rejecting a point outside a node's box before
    /// descending, and that prune is what keeps it O(depth). An expanded child
    /// inside an exactly-sized parent — `Semantics` wrapping a
    /// `GestureDetector`, which is how every third-party control is built —
    /// would be pruned away at the parent and never reached.
    ///
    /// The alternative is a per-node union of the subtree's reach, computed
    /// bottom-up at layout. That is tighter and it goes stale: a child whose
    /// reach changes without its *size* changing does not relayout its parent,
    /// so the parent keeps a union that no longer covers it, and the failure is
    /// a control that silently stops taking taps near its edge.
    ///
    /// This is one number, it only ever grows, and growing only makes the prune
    /// more conservative — so it cannot be stale in the direction that loses a
    /// tap. The cost is descending into a few more subtrees near their edges,
    /// where the precise test below rejects them anyway.
    max_hit_slop: f32,
    /// Repaint boundaries whose recording is stale.
    ///
    /// The layout set above holds the node to *restart layout from*; this holds
    /// the node whose *layer* has to be re-recorded. Same idea, different kind of
    /// boundary: [`mark_needs_paint`](Self::mark_needs_paint) walks up to the
    /// nearest repaint boundary and stops, so a change deep inside one never
    /// reaches the root.
    pending_paints: FastSet<RenderId>,
    /// Which layer each repaint boundary records into.
    ///
    /// Kept here rather than on `Node` so that [`paint_layers`](Self::paint_layers)
    /// can notice a boundary that is *gone*: a dead node cannot tell anyone it
    /// owned a layer, and the layer has to be removed for the pixels it drew to
    /// become damage.
    layers: FastMap<RenderId, LayerEntry>,
    /// Bumped by every change to the parent/child shape of the tree.
    ///
    /// The invalidation key for [`Node::boundary_memo`]. Deliberately global
    /// rather than per-subtree: reparenting is rare, a `u64` bump is free, and
    /// a coarse key that cannot be wrong beats a precise one that can.
    structure_generation: u64,
    /// Monotonic source of fresh subtree revisions. [`bump_subtree`](Self::bump_subtree)
    /// assigns one fresh value along the whole path from a mutation to the
    /// root, so a subtree is unchanged iff its stamp matches the one recorded
    /// when its boundary cache was written.
    revision_counter: u64,
    /// Cached boundary list per subtree root, keyed by
    /// `(subtree_revision, incoming effect, incoming origin)`. See
    /// [`boundaries`](Self::boundaries) for why those three are the key.
    boundary_cache: FastMap<RenderId, BoundaryCache>,
    /// The previous frame's boundary list, kept for its capacity: every frame
    /// produces one, and allocating it afresh was a steady-state allocation.
    boundary_scratch: Vec<Boundary>,
    /// The layer → boundary map `paint_layers_with_ratio` rebuilds each frame,
    /// kept for its capacity for the same reason.
    owner_scratch: FastMap<LayerId, RenderId>,
    /// How many subtree walks [`boundaries`](Self::boundaries) has actually
    /// done since this tree was created — the counted form of "work
    /// proportional to what changed" for boundary collection, in the same
    /// spirit as [`layout_runs`](Self::layout_runs). An idle frame where every
    /// subtree is cached reports a value that does not grow.
    boundary_walks: usize,
    /// The boundary list the layer tree was last reconciled from.
    ///
    /// Compared against the new one to skip the phase entirely — see
    /// [`reconcile_layers`](Self::reconcile_layers).
    previous_boundaries: Vec<Boundary>,
    /// How many frames actually reconciled. Counted work, like
    /// [`layout_runs`](Self::layout_runs) and
    /// [`boundary_walks`](Self::boundary_walks): a claim about cost is only a
    /// claim if something counts.
    reconciles: usize,
    /// `LayerTree::len()` as of the last reconcile, so a different tree handed
    /// in on a later frame cannot be mistaken for the same one.
    previous_layer_count: usize,
}

impl Default for RenderTree {
    /// # Why this is written out rather than derived
    ///
    /// `#[derive(Default)]` gives `fonts` a `FontStore::default()`, which is
    /// `FontStore::new()`, which **scans the machine's fonts** — the exact
    /// opposite of what the field above documents and of what DESIGN §12
    /// decided. It was derived for a long time, and the cost was invisible on
    /// any single machine: a test measures text against whatever fonts that
    /// machine happens to have, and passes.
    ///
    /// It stops being invisible the moment a second machine runs the suite.
    /// Two `tap_to_caret` assertions about wrap points and caret affinity
    /// passed on Linux and failed on macOS, because the two runners shape the
    /// same string to different widths. The debug-build cost was hiding in
    /// plain sight too — 33s of font scanning per process, which is most of
    /// what a slow test run was.
    ///
    /// So: embedded faces only, deterministic on every machine, and an
    /// application opts into the system's with [`set_fonts`](RenderTree::set_fonts).
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            root: None,
            layout_runs: 0,
            fonts: FontStore::embedded_only(),
            pending_boundaries: FastSet::default(),
            max_hit_slop: 0.0,
            pending_paints: FastSet::default(),
            layers: FastMap::default(),
            revision_counter: 0,
            structure_generation: 0,
            boundary_cache: FastMap::default(),
            boundary_scratch: Vec::new(),
            owner_scratch: FastMap::default(),
            boundary_walks: 0,
            previous_boundaries: Vec::new(),
            reconciles: 0,
            previous_layer_count: 0,
        }
    }
}

impl RenderTree {
    /// An empty tree, with only the embedded fonts loaded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the font store — how an application opts into system fonts.
    ///
    /// Marks every text object for relayout, since new fonts mean new metrics and
    /// therefore new sizes for anything already measured. New metrics can move
    /// every clip and every boundary's placement, so the boundary cache is
    /// dropped wholesale rather than selectively invalidated.
    pub fn set_fonts(&mut self, fonts: FontStore) {
        self.fonts = fonts;
        self.boundary_cache.clear();
        let ids: Vec<RenderId> = self.ids();
        for id in ids {
            self.mark_needs_layout(id);
        }
    }

    /// The fonts text layout will shape against.
    pub fn fonts_mut(&mut self) -> &mut FontStore {
        &mut self.fonts
    }

    /// The root, once something has been inserted.
    #[must_use]
    pub const fn root(&self) -> Option<RenderId> {
        self.root
    }

    /// Number of live render objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|slot| slot.node.is_some()).count()
    }

    /// `true` if the tree holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many nodes have each number of children: `[0]` is the count of
    /// leaves, `[1]` of single-child wrappers, and so on, with everything at or
    /// past the last index folded into it.
    ///
    /// # Why this is public
    ///
    /// It is the measurement [`ChildIds`](crate::ChildIds) is sized from, and a
    /// constant chosen from a number nobody can reproduce is a constant nobody
    /// can re-argue. `viewwstudio`'s bench prints it under `--fanout`, so the
    /// table in that module's docs can be checked against a real tree — this
    /// one or a different application's — rather than taken on trust.
    ///
    /// Linear in the arena, allocating one small vector. It is a diagnostic and
    /// nothing in a frame calls it.
    #[must_use]
    pub fn child_fanout(&self, buckets: usize) -> Vec<usize> {
        let mut out = vec![0usize; buckets.max(1)];
        for slot in &self.slots {
            if let Some(node) = slot.node.as_ref() {
                let bucket = node.children.len().min(out.len() - 1);
                out[bucket] += 1;
            }
        }
        out
    }

    /// `true` if the id still addresses a live object.
    #[must_use]
    pub fn is_alive(&self, id: RenderId) -> bool {
        self.slots
            .get(id.index as usize)
            .is_some_and(|slot| slot.generation == id.generation && slot.node.is_some())
    }

    fn node(&self, id: RenderId) -> &Node {
        let slot = &self.slots[id.index as usize];
        assert!(slot.generation == id.generation, "render {id} is stale");
        slot.node
            .as_ref()
            .unwrap_or_else(|| panic!("render {id} is not live"))
    }

    fn node_mut(&mut self, id: RenderId) -> &mut Node {
        let slot = &mut self.slots[id.index as usize];
        assert!(slot.generation == id.generation, "render {id} is stale");
        slot.node
            .as_mut()
            .unwrap_or_else(|| panic!("render {id} is not live"))
    }

    /// Insert a render object as a child of `parent`, or as the root when
    /// `parent` is `None`.
    pub fn insert(&mut self, parent: Option<RenderId>, object: Box<dyn RenderObject>) -> RenderId {
        let id = self.insert_detached(object);
        match parent {
            Some(parent) => {
                self.node_mut(id).parent = Some(parent);
                self.node_mut(parent).children.push(id);
                self.mark_needs_layout(parent);
            }
            None => self.root = Some(id),
        }
        id
    }

    /// Insert a render object with no parent and without making it the root.
    ///
    /// Used while syncing from the element tree, where structure is assigned in
    /// a second step by [`set_children`](Self::set_children).
    pub fn insert_detached(&mut self, object: Box<dyn RenderObject>) -> RenderId {
        let id = match self.free.pop() {
            Some(index) => RenderId::new(index, self.slots[index as usize].generation),
            None => {
                let index = u32::try_from(self.slots.len()).expect("render arena overflowed u32");
                self.slots.push(Slot {
                    generation: 0,
                    node: None,
                });
                RenderId::new(index, 0)
            }
        };

        self.slots[id.index as usize].node = Some(Node {
            is_boundary: object.is_repaint_boundary(),
            object: Some(object),
            children: ChildIds::new(),
            parent: None,
            offset: Offset::ZERO,
            size: Size::ZERO,
            last_constraints: None,
            needs_layout: true,
            layout_count: 0,
            intrinsics: IntrinsicCache::default(),
            boundary_memo: None,
            subtree_revision: 0,
        });
        id
    }

    /// Replace an object's children wholesale, re-parenting them.
    ///
    /// Marks the object for relayout only when the list actually changed, so a
    /// sync that finds the same structure costs nothing.
    pub fn set_children(&mut self, id: RenderId, children: impl Into<ChildIds>) {
        // `impl Into` rather than `ChildIds`, so the hot caller —
        // `RenderOwner::sync_element`, which built one already — passes it
        // through untouched, while the tests and any application that has a
        // `vec![a, b]` in hand keep working unchanged.
        let children = children.into();
        if self.node(id).children == children {
            return;
        }
        for &child in &children {
            self.node_mut(child).parent = Some(id);
        }
        self.node_mut(id).children = children;
        // Every cached "nearest boundary above me" below this point may now be
        // wrong, because what is above them changed. One counter rather than a
        // walk: reparenting is rare and a stale memo that survived would be a
        // repaint sent to the wrong layer, which is invisible until it is not.
        self.structure_generation = self.structure_generation.wrapping_add(1);
        self.mark_needs_layout(id);
        // Different children draw different things even where the parent's own
        // size is unchanged, so the enclosing layer has to be re-recorded.
        self.mark_needs_paint(id);
        // The boundary *set* under `id` changed, which makes every ancestor's
        // cached boundary list stale — bump the revision up to the root.
        self.bump_subtree(id);
    }

    /// Make an object the root, or clear the root with `None`.
    ///
    /// A new root carries a different subtree's boundaries, so the cache —
    /// which is keyed by id — is dropped rather than trusted.
    ///
    /// # Setting the root it already has is a no-op
    ///
    /// `RenderOwner::sync` calls this on **every** frame with whatever root
    /// the element tree resolved to — almost always the same one. Clearing the
    /// boundary cache unconditionally here meant the cache was empty at the
    /// start of every real frame, so the incremental boundary walk
    /// `boundaries` documents never reused a single subtree outside of unit
    /// tests that call `boundaries` twice in a row. It surfaced as one
    /// allocation per steady frame under the `vieww-standard` counting
    /// allocator (the root's re-cached slice); the lost reuse was the larger
    /// cost.
    pub fn set_root(&mut self, id: Option<RenderId>) {
        if self.root == id {
            return;
        }
        if let Some(id) = id {
            self.node_mut(id).parent = None;
        }
        self.root = id;
        self.boundary_cache.clear();
        self.structure_generation = self.structure_generation.wrapping_add(1);
    }

    /// Remove an object and everything below it.
    pub fn remove(&mut self, id: RenderId) {
        if !self.is_alive(id) {
            return;
        }
        // A node leaving invalidates the cached boundary of anything that could
        // still be pointing through it. `mark_needs_paint` re-checks that the
        // memoised boundary is alive as well, so this is belt and braces — and
        // both are cheap next to the walk they replace.
        self.structure_generation = self.structure_generation.wrapping_add(1);
        for child in self.node(id).children.clone() {
            self.remove(child);
        }
        if let Some(parent) = self.node(id).parent {
            if self.is_alive(parent) {
                self.node_mut(parent).children.retain(|child| child != id);
                // The pixels this subtree drew are still on the surface, and the
                // parent's layer is what has to draw over them.
                self.mark_needs_paint(parent);
                // And a parent with one fewer child is a parent with a
                // different layout: a column loses that child's height and
                // everything below it moves up. Without this a list kept a gap
                // where a removed row had been, and the rows after it drew at
                // their old offsets, because nothing had asked the column to
                // lay out again.
                //
                // A removal also changes the boundary *set* under the parent, so
                // every ancestor's cached boundary list is stale — bump the
                // parent's subtree revision up to the root.
                self.bump_subtree(parent);
                //
                // It stayed hidden because `RenderOwner::sync` used to drop
                // stale objects *after* rebuilding every parent's child list,
                // so `set_children` had already dirtied the parent for its own
                // reasons. That was luck, not design — `remove` is public API,
                // and an application detaching a subtree itself got the stale
                // layout every time.
                self.mark_needs_layout(parent);
            }
        }
        if self.root == Some(id) {
            self.root = None;
        }
        self.pending_boundaries.remove(&id);
        self.pending_paints.remove(&id);

        let slot = &mut self.slots[id.index as usize];
        slot.node = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
    }

    /// Replace the object in a slot, keeping its identity and children.
    ///
    /// This is what an element update does when its widget changed: the
    /// configuration is new, but it is the same node in the same position.
    ///
    /// Always repaints, and relayouts only when the geometry could have moved.
    /// The asymmetry is deliberate: `layout_differs` exists because relayout is
    /// the expensive half and a rebuild hands down a new object either way,
    /// whereas *any* configuration change can change what the object draws —
    /// colour being the obvious one — and there is no cheap way to ask.
    pub fn replace_object(&mut self, id: RenderId, mut object: Box<dyn RenderObject>) {
        let differs = self
            .node(id)
            .object
            .as_deref()
            .is_none_or(|old| old.layout_differs(&*object));
        // **Two adoptions, and the difference between them is the whole point.**
        //
        // `adopt_reports` carries what layout does *not* recompute — a ledger of
        // something already sent outward — so it runs on every replacement.
        // `adopt_layout_cache` carries what layout *would* recompute but is
        // about to skip, so it runs only when the geometry agrees and nothing
        // will lay this replacement out.
        //
        // Collapsing the two was tried and is wrong in both directions. Gating
        // both loses `RenderViewport::reported` on every *scrolled* frame —
        // a scroll changes the offset, so `differs` is true — and the next
        // layout re-reports identical extents into a handler that writes a
        // signal, which `Signal::set` turns into a pending mark whether the value
        // changed or not. The tree then never settles: measured as `pending=1`
        // for the life of a desktop window.
        //
        // Ungating both is worse, and quieter. `RenderText::layout` *reuses*
        // `shaped` when it is present rather than recomputing it, so carrying a
        // paragraph across a genuine text change means the field never reshapes
        // and the caret is measured against the previous string. Three tests in
        // `tap_to_caret` caught that immediately, which is the only reason this
        // comment can be specific about it.
        if let Some(old) = self.node(id).object.as_deref() {
            object.adopt_reports(old);
        }
        if !differs {
            if let Some(old) = self.node(id).object.as_deref() {
                object.adopt_layout_cache(old);
            }
        }
        let was_boundary = self.node(id).is_boundary;
        let is_boundary = object.is_repaint_boundary();
        let node = self.node_mut(id);
        node.object = Some(object);
        node.is_boundary = is_boundary;
        if differs {
            self.mark_needs_layout(id);
            // Upward, past the boundary the walk above stops at. See
            // `RenderObject::parent_reads_configuration`: a positioned child's
            // offset is used by its *stack*, and the stack is not below the
            // boundary this node sits on, so nothing else reaches it.
            if self
                .node(id)
                .object
                .as_deref()
                .is_some_and(RenderObject::parent_reads_configuration)
            {
                if let Some(parent) = self.node(id).parent {
                    self.mark_needs_layout(parent);
                }
            }
        }
        if was_boundary != is_boundary {
            // A node gaining or losing its own layer changes the answer to
            // "which boundary is above me" for everything beneath it, which is
            // the other thing `boundary_memo` is keyed on.
            self.structure_generation = self.structure_generation.wrapping_add(1);
            // The node just gained or lost its own layer, so the *enclosing*
            // boundary's recording is wrong in both directions: it either still
            // holds content that moved into a new layer, or is missing content
            // whose layer is about to disappear. Marking the parent reaches it,
            // since the walk starts above the node itself.
            if let Some(parent) = self.node(id).parent {
                self.mark_needs_paint(parent);
            }
        }
        self.mark_needs_paint(id);
        // A configuration change can alter this object's `layer_effect` /
        // `layer_clip` (which feed the between-boundary effect) or its
        // boundary status (which feeds the boundary *set*). Either changes
        // the boundary list of every ancestor, so bump up to the root.
        self.bump_subtree(id);
    }

    /// Borrow the object in a slot.
    #[must_use]
    pub fn object(&self, id: RenderId) -> Option<&dyn RenderObject> {
        self.slots
            .get(id.index as usize)
            .filter(|slot| slot.generation == id.generation)
            .and_then(|slot| slot.node.as_ref())
            .and_then(|node| node.object.as_deref())
    }

    /// Mutably borrow the object in a slot, marking it for relayout.
    pub fn object_mut(&mut self, id: RenderId) -> Option<&mut Box<dyn RenderObject>> {
        if !self.is_alive(id) {
            return None;
        }
        self.mark_needs_layout(id);
        self.node_mut(id).object.as_mut()
    }

    /// This node's intrinsic cache.
    pub(crate) fn intrinsic_cache(&self, id: RenderId) -> &IntrinsicCache {
        &self.node(id).intrinsics
    }

    /// This node's intrinsic cache, mutably.
    pub(crate) fn intrinsic_cache_mut(&mut self, id: RenderId) -> &mut IntrinsicCache {
        &mut self.node_mut(id).intrinsics
    }

    /// Take the object out of a slot, as `layout` does, so a context can borrow
    /// the tree mutably while it runs.
    ///
    /// `None` when it is already out — which for intrinsics means a reentrant
    /// query rather than a bug, and is handled as "cannot answer".
    pub(crate) fn take_object(&mut self, id: RenderId) -> Option<Box<dyn RenderObject>> {
        self.node_mut(id).object.take()
    }

    /// Put an object back after [`take_object`](Self::take_object).
    pub(crate) fn put_object(&mut self, id: RenderId, object: Box<dyn RenderObject>) {
        self.node_mut(id).object = Some(object);
    }

    /// Drop every cached intrinsic answer in the tree.
    ///
    /// For a consumer that has changed something the tree cannot see — fonts,
    /// most of all, since a different face changes every text measurement in
    /// the tree at once.
    pub fn clear_intrinsics(&mut self) {
        for slot in &mut self.slots {
            if let Some(node) = slot.node.as_mut() {
                node.intrinsics.clear();
            }
        }
    }

    /// Ask the root an intrinsic question. `None` if there is no root.
    ///
    /// The entry point a consumer outside the render tree uses; render objects
    /// ask their own children through
    /// [`IntrinsicCtx`](crate::IntrinsicCtx) instead.
    pub fn root_intrinsic(&mut self, query: IntrinsicQuery) -> Option<f32> {
        self.root.and_then(|root| self.intrinsic(root, query))
    }

    /// Children in paint order.
    #[must_use]
    pub fn children(&self, id: RenderId) -> &[RenderId] {
        &self.node(id).children
    }

    /// The parent, or `None` at the root.
    #[must_use]
    pub fn parent(&self, id: RenderId) -> Option<RenderId> {
        self.node(id).parent
    }

    /// The size chosen by the last layout.
    #[must_use]
    pub fn size(&self, id: RenderId) -> Size {
        self.node(id).size
    }

    /// The offset relative to the parent.
    #[must_use]
    pub fn offset(&self, id: RenderId) -> Offset {
        self.node(id).offset
    }

    /// How many times this object has run layout.
    #[must_use]
    pub fn layout_count(&self, id: RenderId) -> u32 {
        self.node(id).layout_count
    }

    /// The offset relative to the root, following the parent chain.
    #[must_use]
    pub fn global_offset(&self, id: RenderId) -> Offset {
        let mut offset = Offset::ZERO;
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            let node = self.node(current);
            offset = offset + node.offset;
            cursor = node.parent;
        }
        offset
    }

    /// Bounds relative to the root.
    #[must_use]
    pub fn global_bounds(&self, id: RenderId) -> Rect {
        Rect::from_origin_size(self.global_offset(id), self.size(id))
    }

    pub(crate) fn set_offset(&mut self, id: RenderId, offset: Offset) {
        if self.node(id).offset == offset {
            // A parent places every child on every layout, usually where it
            // already was. Repainting for that would make the boundary useless.
            return;
        }
        self.node_mut(id).offset = offset;
        if self.is_repaint_boundary(id) {
            // A boundary records in its own space, so moving it does not
            // invalidate a single command — only where the layer lands. **No
            // repaint**, therefore, and that half is unchanged: marking it here
            // would re-record a subtree that draws exactly the same thing.
            //
            // # But the boundary list *is* stale, and this used to say it was not
            //
            // The old comment ended "`reconcile_layers` reads `global_offset`
            // every frame regardless", and that was true and load-bearing:
            // reconciliation recomputed every layer's position from the tree on
            // every frame, so a moved boundary was picked up whether or not the
            // cache knew about it.
            //
            // Reconciliation is now skipped when the boundary list is
            // unchanged, and `Boundary` carries `origin`. So the cached list is
            // exactly what decides where this layer is drawn, and leaving it
            // stale leaves the layer at its old position for ever.
            //
            // Caught by `a_boundary_that_only_moves_still_moves_its_layer`,
            // which is in the suite because this interaction is invisible from
            // either side alone.
            self.bump_subtree(id);
            return;
        }
        // A non-boundary move changes the relative `origin` of every boundary
        // below here, which changes their clip `bounds` and therefore their
        // between-boundary effect. The cached boundary list of every ancestor
        // is stale, so bump up to the root before the repaint.
        self.bump_subtree(id);
        self.mark_needs_paint(id);
    }

    // ------------------------------------------------------------------ layout

    /// Mark an object as needing layout, propagating up to the nearest
    /// relayout boundary.
    ///
    /// The walk stops at the first ancestor that was laid out under **tight**
    /// constraints. That ancestor's size cannot change no matter what happens
    /// below it, so its own parent has nothing to recompute — which is exactly
    /// what makes a deep local change cost a small subtree instead of a whole
    /// tree. It is the key layout optimisation and the reason a text edit
    /// in a fixed-size box does not relayout the screen.
    pub fn mark_needs_layout(&mut self, id: RenderId) {
        if !self.is_alive(id) {
            return;
        }
        // Intrinsics are invalidated on the way up rather than tree-wide,
        // because an answer only depends on the subtree *below* a node: a
        // sibling's text changing cannot change this node's answer, and
        // clearing the whole tree per mark would make a scroll frame quadratic.
        // The walk stops at the relayout boundary for layout's own reasons, and
        // that is not far enough here — a boundary's ancestors' intrinsics do
        // still depend on what changed inside it, even though their *sizes* do
        // not. So the intrinsic sweep continues to the root after the layout
        // walk has stopped. It is a pointer chase per level over a tree that is
        // tens deep, against an answer that is otherwise silently stale.
        let mut sweep = Some(id);
        while let Some(current) = sweep {
            if !self.is_alive(current) {
                break;
            }
            let cache = &mut self.node_mut(current).intrinsics;
            if cache.is_empty() {
                // Nothing cached here means nothing cached above it either:
                // a parent's answer is only ever produced by asking its
                // children, so an empty child cache bounds the sweep.
                break;
            }
            cache.clear();
            sweep = self.node(current).parent;
        }

        let mut current = id;
        loop {
            self.node_mut(current).needs_layout = true;
            if self.is_relayout_boundary(current) {
                break;
            }
            match self.node(current).parent {
                Some(parent) if self.is_alive(parent) => current = parent,
                _ => break,
            }
        }
        // Whatever the walk stopped at is where layout has to restart.
        self.pending_boundaries.insert(current);
    }

    /// Lay out every subtree marked since the last frame.
    ///
    /// Each boundary is re-laid-out against the constraints it saw last time —
    /// which are by definition still valid, since a boundary is a node whose
    /// own size cannot change from below.
    pub fn flush_layout(&mut self) {
        while let Some(&id) = self.pending_boundaries.iter().next() {
            self.pending_boundaries.remove(&id);
            if !self.is_alive(id) {
                continue;
            }
            let Some(constraints) = self.node(id).last_constraints else {
                // Never laid out, so it has no constraints of its own yet; its
                // parent will reach it.
                continue;
            };
            self.layout(id, constraints);
        }
    }

    /// Mark the layer holding this object as stale, walking up to the nearest
    /// repaint boundary.
    ///
    /// The walk starts at `id` itself, so a boundary marks its *own* layer. It
    /// stops at the first boundary found, and that single absence of further
    /// propagation is the whole mechanism: a repaint deep in the tree costs the
    /// subtree containing it rather than the screen. The root is always a
    /// boundary, so the walk always terminates somewhere that owns a layer.
    pub fn mark_needs_paint(&mut self, id: RenderId) {
        if !self.is_alive(id) {
            return;
        }
        let generation = self.structure_generation;
        if let Some((stamped, boundary)) = self.node(id).boundary_memo {
            if stamped == generation && self.is_alive(boundary) {
                self.pending_paints.insert(boundary);
                return;
            }
        }

        // Two walks rather than one walk and a `Vec` of the path.
        //
        // The first draft collected the path so every node on it could be
        // memoised in one pass. That `Vec` was then the single largest
        // allocation site in the whole frame — eighteen thousand allocations
        // over eight frames — which is a fine illustration of how a cache pays
        // for itself only if the bookkeeping is cheaper than the thing cached.
        // Climbing twice touches the same handful of parents already in cache
        // and allocates nothing.
        let mut current = id;
        let boundary = loop {
            if self.is_repaint_boundary(current) {
                break current;
            }
            match self.node(current).parent {
                Some(parent) if self.is_alive(parent) => current = parent,
                // A detached subtree, mid-sync. It has no layer to pending; the
                // `set_children` that attaches it marks the new parent. Nothing
                // is memoised: this node's answer is "none yet", and caching
                // that would have to be invalidated by the attach.
                _ => return,
            }
        };
        // Every node between the asker and the boundary has the same answer, so
        // all of them learn it. A parent marked right after its child — which
        // is what a rebuilt subtree does — then costs one lookup.
        let mut current = id;
        loop {
            self.node_mut(current).boundary_memo = Some((generation, boundary));
            if current == boundary {
                break;
            }
            match self.node(current).parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        self.pending_paints.insert(boundary);
    }

    /// Whether `id`'s children are mounted but out of the picture.
    ///
    /// See [`RenderObject::skips_children`]. A node with no object skips
    /// nothing: an empty slot hides no one.
    #[must_use]
    pub fn skips_children(&self, id: RenderId) -> bool {
        self.node(id)
            .object
            .as_deref()
            .is_some_and(RenderObject::skips_children)
    }

    /// Whether `id`'s subtree is hidden from a screen reader but still drawn.
    ///
    /// See [`RenderObject::hides_semantics`]. As above, a node with no object
    /// hides nothing.
    #[must_use]
    pub fn hides_semantics(&self, id: RenderId) -> bool {
        self.node(id)
            .object
            .as_deref()
            .is_some_and(RenderObject::hides_semantics)
    }

    /// Whether `id` hides everything painted before it from a screen reader.
    ///
    /// See [`RenderObject::blocks_semantics`]. As above, a node with no object
    /// blocks nothing.
    #[must_use]
    pub fn blocks_semantics(&self, id: RenderId) -> bool {
        self.node(id)
            .object
            .as_deref()
            .is_some_and(RenderObject::blocks_semantics)
    }

    /// `true` if this object's subtree keeps its own recording.
    ///
    /// The root always does — it owns the layer everything else composites onto,
    /// and there is nothing above it to fall back to.
    #[must_use]
    pub fn is_repaint_boundary(&self, id: RenderId) -> bool {
        let node = self.node(id);
        node.is_boundary || self.root == Some(id)
    }

    /// `true` if changes below this object cannot change its own size.
    fn is_relayout_boundary(&self, id: RenderId) -> bool {
        let node = self.node(id);
        node.parent.is_none() || node.last_constraints.is_some_and(Constraints::is_tight)
    }

    /// Lay out a subtree, returning the size it chose.
    ///
    /// Skips the whole subtree when the same constraints come back down and
    /// nothing below has been marked — the memoised half of the relayout
    /// boundary.
    ///
    /// # Panics
    ///
    /// In debug builds, if a render object returns a size that violates the
    /// constraints it was given. Catching it here names the culprit; letting it
    /// through surfaces as a mystery several levels away.
    pub fn layout(&mut self, id: RenderId, constraints: Constraints) -> Size {
        {
            let node = self.node(id);
            if !node.needs_layout && node.last_constraints == Some(constraints) {
                return node.size;
            }
        }

        // Take the object out so `LayoutCtx` can borrow the tree mutably to
        // reach the children. It goes back before this returns.
        let mut object = self
            .node_mut(id)
            .object
            .take()
            .expect("render object was already borrowed for layout — reentrant layout");

        let mut ctx = LayoutCtx { tree: self, id };
        // **Catch panics in the render object's `layout`.** A `CustomPainter`
        // or any render object whose `layout` divides by zero — or panics for
        // any other reason — used to abort the process. With both sides built
        // `-C prefer-dynamic`, `catch_unwind` catches guest panics as written;
        // the panic is reported through `report_render_panic` and a zero size
        // is returned, which leaves a hole in the layout rather than a dead
        // process.
        let size = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            object.layout(&mut ctx, constraints)
        })) {
            Ok(size) => size,
            Err(_) => {
                let name = object.debug_name();
                crate::owner::report_render_panic(name, "layout");
                Size::ZERO
            }
        };

        debug_assert!(
            size.width >= constraints.min_width - f32::EPSILON
                && size.width <= constraints.max_width + f32::EPSILON
                && size.height >= constraints.min_height - f32::EPSILON
                && size.height <= constraints.max_height + f32::EPSILON,
            "{} returned {size}, which violates {constraints}",
            object.debug_name()
        );
        self.layout_runs += 1;
        let node = self.node_mut(id);
        let resized = node.size != size;
        node.object = Some(object);
        node.size = size;
        node.last_constraints = Some(constraints);
        node.needs_layout = false;
        node.layout_count += 1;
        // Monotonic on purpose; see `max_hit_slop`.
        let slop = node
            .object
            .as_deref()
            .map_or(0.0, |object| hit_slop_of(object, size));
        if slop > self.max_hit_slop {
            self.max_hit_slop = slop;
        }
        self.pending_boundaries.remove(&id);
        if resized {
            // Only a *changed* size repaints. Layout runs on plenty of nodes that
            // settle on exactly what they had — a rebuild hands the subtree new
            // objects, and every one of them measures again — and repainting for
            // those would put the whole tree in the pending set every rebuild.
            // A size change also moves every descendant's clip `bounds` (which
            // are relative to this node's size via `origin`), so the cached
            // boundary list of every ancestor is stale.
            self.bump_subtree(id);
            self.mark_needs_paint(id);
        }
        size
    }

    /// Lay a node out under the **sliver** protocol.
    ///
    /// # The adaptation, which is the point
    ///
    /// A node whose object returns `None` from
    /// [`RenderObject::layout_sliver`] does not speak the protocol — which is
    /// every render object written before it existed. Rather than refusing, it
    /// is laid out with box constraints derived from the sliver ones and
    /// reported as a sliver of exactly that size. A `Text` therefore goes into
    /// a scrolling viewport unmodified, and the twenty-three render objects
    /// that predate this file needed no change at all.
    ///
    /// # Why this does not reuse the layout cache
    ///
    /// [`layout`](Self::layout) returns early when the constraints match what
    /// it was last given, and that is sound for box layout because a size is a
    /// pure function of its constraints. It is **not** sound here: a sliver's
    /// geometry depends on `scroll_offset`, which changes every frame of a
    /// scroll, so a cache keyed on box constraints would return last frame's
    /// answer for this frame's position. The box constraints a sliver derives
    /// are usually *identical* frame to frame while the scroll moves under
    /// them, which is exactly the case a naive cache gets wrong.
    pub fn layout_sliver(
        &mut self,
        id: RenderId,
        constraints: &SliverConstraints,
    ) -> SliverGeometry {
        let mut object = self
            .node_mut(id)
            .object
            .take()
            .expect("render object was already borrowed for layout — reentrant layout");

        let mut ctx = LayoutCtx { tree: self, id };
        // **Catch panics in `layout_sliver` and the box-layout fallback.**
        // Same reasoning as `layout`'s catch — a sliver render object that
        // panics no longer aborts the process.
        let geometry = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            object.layout_sliver(&mut ctx, constraints)
        })) {
            Ok(geometry) => geometry,
            Err(_) => {
                let name = object.debug_name();
                crate::owner::report_render_panic(name, "layout_sliver");
                None
            }
        };

        let geometry = match geometry {
            Some(geometry) => {
                debug_assert!(
                    geometry.is_consistent(constraints),
                    "{} returned {geometry:?}, which does not fit {constraints:?}",
                    object.debug_name()
                );
                geometry
            }
            None => {
                // A box child. Measure it once and report what it measured.
                let box_constraints = constraints.box_constraints();
                let size = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    object.layout(&mut ctx, box_constraints)
                })) {
                    Ok(size) => size,
                    Err(_) => {
                        let name = object.debug_name();
                        crate::owner::report_render_panic(name, "layout (sliver fallback)");
                        Size::ZERO
                    }
                };
                let extent = constraints.main_of(size);
                SliverGeometry::new(extent, crate::sliver::visible_extent(extent, constraints))
            }
        };

        let size = constraints.size(geometry.paint_extent, constraints.cross_axis_extent);
        self.layout_runs += 1;
        let node = self.node_mut(id);
        let resized = node.size != size;
        node.object = Some(object);
        node.size = size;
        // Deliberately cleared rather than recorded. The box cache must not
        // answer for a sliver — see this method's docs — and leaving a stale
        // entry behind would let a later box layout of the same node return a
        // size that was measured for a scroll position.
        node.last_constraints = None;
        node.needs_layout = false;
        node.layout_count += 1;
        // Monotonic on purpose; see `max_hit_slop`.
        let slop = node
            .object
            .as_deref()
            .map_or(0.0, |object| hit_slop_of(object, size));
        if slop > self.max_hit_slop {
            self.max_hit_slop = slop;
        }
        self.pending_boundaries.remove(&id);
        if resized {
            self.mark_needs_paint(id);
        }
        geometry
    }

    /// Lay out the whole tree against `constraints`.
    pub fn layout_root(&mut self, constraints: Constraints) -> Size {
        let Some(root) = self.root else {
            return Size::ZERO;
        };
        let size = self.layout(root, constraints);
        // Anything marked behind a relayout boundary is invisible to the root
        // pass and has to be caught here.
        self.flush_layout();
        size
    }

    // ------------------------------------------------------------------- paint

    /// Paint the tree onto `canvas`, in paint order.
    pub fn paint(&self, canvas: &mut dyn Canvas) {
        self.paint_with_ratio(canvas, 1.0);
    }

    /// The same, for a surface whose physical pixels are `dpr` to one logical
    /// one.
    ///
    /// Leaves that snap to the pixel grid read it off the [`PaintCtx`] this
    /// hands them; at 1:1 the two calls are identical.
    pub fn paint_with_ratio(&self, canvas: &mut dyn Canvas, dpr: f32) {
        if let Some(root) = self.root {
            self.paint_node(root, Offset::ZERO, canvas, dpr);
        }
    }

    fn paint_node(&self, id: RenderId, parent_origin: Offset, canvas: &mut dyn Canvas, dpr: f32) {
        let node = self.node(id);
        let origin = parent_origin + node.offset;

        if let Some(object) = node.object.as_deref() {
            let mut ctx = PaintCtx {
                canvas,
                origin,
                size: node.size,
                dpr,
            };
            // **Catch panics in `paint`.** Same reasoning as `layout`'s catch —
            // a `CustomPainter::paint` that panics no longer aborts the process.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                object.paint(&mut ctx);
            }))
            .is_err()
            {
                crate::owner::report_render_panic(object.debug_name(), "paint");
            }
        }

        // Children paint after their parent and in order, so later siblings
        // land on top. Hit testing walks this same order backwards.
        for &child in &node.children {
            self.paint_node(child, origin, canvas, dpr);
        }

        if let Some(object) = node.object.as_deref() {
            let mut ctx = PaintCtx {
                canvas,
                origin,
                size: node.size,
                dpr,
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                object.paint_children_done(&mut ctx);
            }))
            .is_err()
            {
                crate::owner::report_render_panic(object.debug_name(), "paint_children_done");
            }
        }
    }

    // ------------------------------------------------------------------ layers

    /// Paint into `layers`, re-recording only the boundaries that need it.
    ///
    /// This is the incremental counterpart to [`paint`](Self::paint), and the
    /// thing that makes a repaint boundary worth having: a layer whose subtree
    /// did not change keeps the commands it already had, and the frame is
    /// reassembled by compositing rather than by redrawing.
    ///
    /// Every call reconciles the layer tree against the render tree first —
    /// inserting layers for new boundaries, removing those whose boundary is
    /// gone, and updating where each one sits. Removal is not bookkeeping: a
    /// removed layer hands its last on-screen bounds to
    /// [`LayerTree::damage`](vieww_paint::LayerTree::damage), which is what makes
    /// a deleted subtree get painted over instead of lingering.
    pub fn paint_layers(&mut self, layers: &mut LayerTree) {
        self.paint_layers_with_ratio(layers, 1.0);
    }

    /// The same, for a surface whose physical pixels are `dpr` to one logical
    /// one. See [`paint_with_ratio`](Self::paint_with_ratio).
    pub fn paint_layers_with_ratio(&mut self, layers: &mut LayerTree, dpr: f32) {
        let boundaries = self.boundaries();
        self.reconcile_layers(layers, &boundaries);
        self.boundary_scratch = boundaries;

        // Hand our pending work to the layer tree and let it answer, rather than keeping
        // two pending sets that can disagree. It already has to track layers marked
        // by `begin_paint` and by insertion, which this side never sees.
        for id in std::mem::take(&mut self.pending_paints) {
            if let Some(entry) = self.layers.get(&id) {
                layers.mark_needs_paint(entry.layer);
            }
        }

        let mut owners = std::mem::take(&mut self.owner_scratch);
        owners.clear();
        owners.extend(
            self.layers
                .iter()
                .map(|(&boundary, entry)| (entry.layer, boundary)),
        );

        for layer in layers.needs_paint() {
            let Some(&boundary) = owners.get(&layer) else {
                continue;
            };
            let scene = layers.begin_paint(layer);
            let slots = self.paint_boundary(boundary, scene, dpr);
            // Straight back to the layer tree, against the recording that was
            // just made. Anything between here and there is a chance for the
            // parent to re-record and turn these into indices about a frame that
            // no longer exists.
            for (child, slot) in slots {
                if let Some(entry) = self.layers.get(&child) {
                    layers.set_slot(entry.layer, slot);
                }
            }
        }
        self.owner_scratch = owners;
    }

    /// Every repaint boundary in paint order, each with the boundary enclosing
    /// it and how it composites into that boundary.
    ///
    /// # Incremental: subtrees whose revision has not advanced are reused
    ///
    /// This used to walk the whole render tree every frame — the one phase
    /// behind the dirty-set architecture that did not have a dirty set of its
    /// own. It now retains, per subtree root, the boundary list it produced and
    /// the `(revision, incoming effect, incoming origin)` it produced it under;
    /// a subtree whose [`Node::subtree_revision`] has not advanced since the
    /// cache was written is lifted wholesale, with no recursion and no trait
    /// reads. See `docs/ARCHITECTURE-AUDIT.md` finding 3.
    ///
    /// The cache is correct because a boundary's *identity, effect, clip and
    /// relative placement* are a pure function of the subtree's configuration,
    /// and `subtree_revision` is bumped on exactly the mutations that can
    /// change any of them — see [`bump_subtree`](Self::bump_subtree). The
    /// absolute placement a layer lands at is recomputed by
    /// [`reconcile_layers`](Self::reconcile_layers) from
    /// [`global_offset`](Self::global_offset) regardless, so it is not part of
    /// the key.
    ///
    /// The invariant — that this returns exactly what a from-scratch walk would
    /// — is what [`boundaries_uncached`](Self::boundaries_uncached) exists to
    /// assert, and what the tests in this file pin after every mutation.
    fn boundaries(&mut self) -> Vec<Boundary> {
        let mut out = std::mem::take(&mut self.boundary_scratch);
        out.clear();
        let Some(root) = self.root else {
            return out;
        };

        // The walk reads `self.slots` (and the trait) immutably while it writes
        // the cache, so move the cache out for the duration of the walk and put
        // it back. Splitting `&self` / `&mut self.boundary_cache` directly is not
        // borrow-checkable because `collect_boundaries` is a method on `&self`.
        let mut cache = std::mem::take(&mut self.boundary_cache);
        // Drop entries whose subtree is gone. A dead id can never be reused for
        // a *different* subtree (the arena's wrapping generation counter forbids
        // it), but holding stale slices would grow the map without bound across
        // a long editing session.
        cache.retain(|id, _| self.is_alive(*id));
        let mut misses = 0usize;
        self.collect_boundaries(
            root,
            None,
            LayerEffect::default(),
            Offset::ZERO,
            &mut out,
            &mut cache,
            &mut misses,
        );
        self.boundary_cache = cache;
        self.boundary_walks = self.boundary_walks.saturating_add(misses);
        out
    }

    /// The boundary list with no caching — the oracle. Identical to
    /// [`boundaries`](Self::boundaries) on every frame, after every mutation,
    /// which is what the tests assert. Private because it returns a private
    /// type; the in-tree test module reaches it as a child of this one.
    #[allow(
        dead_code,
        reason = "the oracle used by the in-tree test module; \
                                 compiled out of tests but kept so the tests \
                                 can compare cached vs uncached"
    )]
    fn boundaries_uncached(&self) -> Vec<Boundary> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            // A throwaway, never-read cache so the oracle shares the walk body
            // without sharing its *result* — every call is a cold miss, which is
            // what "uncached" means here.
            let mut ctx = FastMap::default();
            let mut misses = 0usize;
            self.collect_boundaries_uncached(
                root,
                None,
                LayerEffect::default(),
                Offset::ZERO,
                &mut out,
                &mut ctx,
                &mut misses,
            );
        }
        out
    }

    /// How many subtree walks `boundaries` has actually done
    /// since this tree was created — the counted form of "work proportional to
    /// what changed" for boundary collection, in the same spirit as
    /// [`layout_runs`](Self::layout_runs). An idle frame where every subtree is
    /// cached reports a value that does not grow.
    #[must_use]
    pub const fn boundary_walks(&self) -> usize {
        self.boundary_walks
    }

    /// How many frames have reconciled the layer tree.
    ///
    /// A frame whose boundary list is unchanged does not, which is the point.
    #[must_use]
    pub const fn reconciles(&self) -> usize {
        self.reconciles
    }

    /// Bump the subtree revision of `id` and every ancestor, so every cached
    /// boundary list that *includes* this subtree is invalidated. A single
    /// fresh value is stamped along the whole path: a subtree is unchanged iff
    /// its stamp matches the one recorded when its cache was written, so one
    /// bump per mutation is enough and two mutations do not alias.
    ///
    /// Called from the mutation sites that can change a boundary's identity,
    /// effect, clip or relative placement — `set_children`, `replace_object`,
    /// `set_offset` (non-boundary), `layout` (resized) and `remove` — and
    /// **not** from `mark_needs_paint`. A pure repaint (a colour, a blinking
    /// caret) changes a layer's *recording*, never its boundary list, and
    /// bumping on it would defeat the cache on the one frame shape it is
    /// designed for.
    fn bump_subtree(&mut self, id: RenderId) {
        self.revision_counter = self.revision_counter.wrapping_add(1);
        let fresh = self.revision_counter;
        let mut cursor = Some(id);
        while let Some(current) = cursor {
            if !self.is_alive(current) {
                break;
            }
            self.node_mut(current).subtree_revision = fresh;
            cursor = self.node(current).parent;
        }
    }

    /// # The effect accumulates *between* boundaries, and resets at each one
    ///
    /// `effect` is what has been declared since the last boundary, not since the
    /// root. That is what makes the composite correct rather than
    /// double-applied: `LayerTree::composite` wraps a layer's own markers around
    /// its children too, so a child inherits its parent layer's effect by
    /// nesting. Carrying the running product down instead would apply an
    /// ancestor's fade once per generation.
    ///
    /// # `origin`, and why a clip needs one when a fade does not
    ///
    /// An alpha is a number and applies wherever the subtree happens to be. A
    /// clip is a *rectangle*, so it only means anything paired with a coordinate
    /// space, and the one it has to be in is the enclosing boundary's — that is
    /// the space `LayerTree::composite` applies it in, and the space
    /// `reconcile_layers` already expresses each layer's transform in.
    ///
    /// `origin` is therefore this node's position relative to the enclosing
    /// boundary, accumulated on the way down and reset to zero at each boundary,
    /// which mirrors `global_offset(id) - global_offset(enclosing)` without
    /// walking back up the tree once per node.
    ///
    /// # Caching
    ///
    /// Before recursing, the subtree under `id` is looked up in `ctx` by
    /// `(subtree_revision, effect, origin)`. A hit appends the retained slice
    /// and returns; a miss walks normally and then records the slice it
    /// produced under that key. The three key fields are exactly the inputs the
    /// walk reads, so a matching key guarantees a matching slice.
    #[allow(
        clippy::too_many_arguments,
        reason = "the seven params are the natural \
        inputs of a recursive walk — the cached and uncached variants share them, \
        and bundling into a struct would hide which inputs each branch reads"
    )]
    fn collect_boundaries(
        &self,
        id: RenderId,
        enclosing: Option<RenderId>,
        effect: LayerEffect,
        origin: Offset,
        out: &mut Vec<Boundary>,
        ctx: &mut FastMap<RenderId, BoundaryCache>,
        misses: &mut usize,
    ) {
        // Cache hit: the subtree's configuration and the between-boundary state
        // reaching it are both unchanged, so its boundary list is identical.
        let stamp = self.node(id).subtree_revision;
        if let Some(cached) = ctx.get(&id) {
            if cached.stamp == stamp && cached.effect == effect && cached.origin == origin {
                out.extend_from_slice(&cached.boundaries);
                return;
            }
        }

        // Cache miss: walk the subtree, recording where this entry's slice
        // starts so it can be cached at the end.
        *misses += 1;
        let start = out.len();
        self.collect_boundaries_uncached(id, enclosing, effect, origin, out, ctx, misses);

        // Record this subtree's slice for next time. A subtree that produced no
        // boundaries still caches (an empty slice), so an unchanged empty
        // subtree is a hit too.
        // Reuse the stale entry's allocation when there is one: a subtree that
        // misses on every frame (an animating one) would otherwise allocate its
        // slice every frame.
        match ctx.get_mut(&id) {
            Some(entry) => {
                entry.stamp = stamp;
                entry.effect = effect;
                entry.origin = origin;
                entry.boundaries.clear();
                entry.boundaries.extend_from_slice(&out[start..]);
            }
            None => {
                ctx.insert(
                    id,
                    BoundaryCache {
                        stamp,
                        effect,
                        origin,
                        boundaries: out[start..].to_vec(),
                    },
                );
            }
        }
    }

    /// The uncached walk — the oracle body, and the inner loop of the cached
    /// walk on a miss. Recurses through itself rather than through
    /// [`collect_boundaries`](Self::collect_boundaries), so the oracle is
    /// independent of the optimisation it checks. The one mutation from the old
    /// single-function form is that the cached walk passes `ctx`/`misses` down
    /// so each child's *own* subtree caches too.
    #[allow(
        clippy::too_many_arguments,
        reason = "shares the cached walk's signature so the \
        oracle and the miss path are one body; see `collect_boundaries`"
    )]
    fn collect_boundaries_uncached(
        &self,
        id: RenderId,
        enclosing: Option<RenderId>,
        effect: LayerEffect,
        origin: Offset,
        out: &mut Vec<Boundary>,
        ctx: &mut FastMap<RenderId, BoundaryCache>,
        misses: &mut usize,
    ) {
        let (enclosing, effect, origin) = if self.is_repaint_boundary(id) {
            out.push(Boundary {
                id,
                enclosing,
                effect,
                origin,
            });
            // Reset *before* this object's own declarations are read, which is
            // what makes a `RenderViewport` — a boundary that also clips — come
            // out right: it composites into its parent under whatever was
            // declared above it, and its own clip binds the boundaries below.
            (Some(id), LayerEffect::default(), Offset::ZERO)
        } else {
            (enclosing, effect, origin)
        };

        if self.skips_children(id) {
            // Without this a repaint boundary inside a hidden subtree still
            // gets a layer and is still recorded, straight past the skip above:
            // boundaries are painted from the root of the boundary, not from
            // the tree root.
            return;
        }

        // Read through the trait, never by asking what kind of object this is —
        // see `RenderObject::layer_effect`. An object written outside this
        // repository takes part on exactly the same terms as `RenderOpacity`.
        let node = self.node(id);
        let object = node.object.as_deref();

        let effect = match object.and_then(|o| o.layer_effect()) {
            Some(mine) => mine.then(effect),
            None => effect,
        };

        // The bounds are handed over already in the enclosing boundary's space,
        // so an object that clips to itself has no coordinate arithmetic to get
        // wrong — see `RenderObject::layer_clip`.
        let bounds = Rect::from_origin_size(origin, node.size);
        let effect = match object.and_then(|o| o.layer_clip(bounds)) {
            Some(mine) => effect.clipped_to(mine),
            None => effect,
        };

        for &child in &node.children {
            // Recurse through the cached entry point so each child's own subtree
            // is cached independently — the whole point. `boundaries_uncached`
            // (the oracle) calls this same function, so it shares the walk body
            // but not the cache.
            self.collect_boundaries(
                child,
                enclosing,
                effect,
                origin + self.node(child).offset,
                out,
                ctx,
                misses,
            );
        }
    }

    /// Bring the layer tree's shape and placement in line with the boundaries.
    ///
    /// # The frame where nothing changed costs one comparison
    ///
    /// `docs/ARCHITECTURE-AUDIT.md`'s finding 3 names two phases that were
    /// `O(tree)` behind a dirty-set architecture. Boundary collection was the
    /// first and is cached. This is the second, and it was worse per boundary:
    /// a `HashSet` of live ids, a `Vec` of stale ones, a `HashMap` of child
    /// order with a `Vec` per parent — four allocations — plus `global_offset`,
    /// an `O(depth)` walk, **twice for every boundary**, every frame, including
    /// the frame where one leaf changed colour.
    ///
    /// Two changes remove all of it:
    ///
    /// 1. [`Boundary::origin`] carries the transform, so no `global_offset`
    ///    walk happens here at all — not even on a frame that does reconcile.
    /// 2. With that field, the boundary list **completely describes** the layer
    ///    tree this function would build. So an identical list means an
    ///    identical result, and the whole phase is skipped.
    ///
    /// The second is only sound because of the first. A list without `origin`
    /// compares equal for a subtree that has merely *moved*, and skipping there
    /// would leave every layer in it drawn at its old position — the exact
    /// class of bug that makes an optimization worse than the cost it saved.
    ///
    /// # The one contract this adds
    ///
    /// `paint_layers` must be handed **the same `LayerTree` every frame**. It
    /// always was in practice — `FrameDriver` owns one for its lifetime — but
    /// retaining state across calls turns that from a habit into a requirement,
    /// so a mismatched count forces a full reconcile rather than trusting it.
    fn reconcile_layers(&mut self, layers: &mut LayerTree, boundaries: &[Boundary]) {
        // `layers.len()` counts the tree's own root as well as one layer per
        // boundary, so it is **not** comparable with `self.layers.len()`; what
        // is comparable is the count recorded the last time this ran. A caller
        // that hands over a different `LayerTree` fails this and gets a full
        // reconcile, which is the contract in the doc comment enforced rather
        // than trusted.
        if boundaries == self.previous_boundaries && layers.len() == self.previous_layer_count {
            return;
        }
        self.reconciles = self.reconciles.saturating_add(1);
        self.previous_boundaries.clear();
        self.previous_boundaries.extend_from_slice(boundaries);

        let live: FastSet<RenderId> = boundaries.iter().map(|b| b.id).collect();

        let stale: Vec<RenderId> = self
            .layers
            .keys()
            .copied()
            .filter(|id| !live.contains(id))
            .collect();
        for id in stale {
            if let Some(entry) = self.layers.remove(&id) {
                layers.remove(entry.layer);
            }
        }

        for &Boundary {
            id,
            enclosing,
            effect,
            origin,
        } in boundaries
        {
            // A layer records in its *parent layer's* space, not the screen's —
            // `LayerTree::composite` accumulates the chain — so the transform is
            // the offset from the enclosing boundary. `origin` is already
            // exactly that, measured by the walk that found this boundary; the
            // two `global_offset` calls this replaced were re-deriving it.
            let transform = Transform::translate(origin);

            let reparented = self
                .layers
                .get(&id)
                .is_some_and(|entry| entry.enclosing != enclosing);
            if reparented {
                // `LayerTree` has no reparent, and adding one would have to keep
                // the recording valid across a change of coordinate space. A
                // subtree that moves between boundaries is rare enough to pay for
                // a re-record.
                if let Some(entry) = self.layers.remove(&id) {
                    layers.remove(entry.layer);
                }
            }

            let layer = match self.layers.get(&id) {
                Some(entry) => {
                    layers.set_transform(entry.layer, transform);
                    entry.layer
                }
                None => {
                    let parent = enclosing.and_then(|id| self.layers.get(&id).map(|e| e.layer));
                    let layer = layers.insert(parent, transform);
                    self.layers.insert(id, LayerEntry { layer, enclosing });
                    layer
                }
            };
            // Unconditional, because `set_effect` already ignores a value that
            // has not changed — and because a fade that stops being declared has
            // to reach the layer just as surely as one that starts.
            layers.set_effect(layer, effect);
        }

        // Insertion appends, which is the right order only when every sibling is
        // new. Say it explicitly instead.
        let mut order: FastMap<RenderId, Vec<LayerId>> = FastMap::default();
        for boundary in boundaries {
            let (Some(enclosing), Some(entry)) =
                (boundary.enclosing, self.layers.get(&boundary.id))
            else {
                continue;
            };
            order.entry(enclosing).or_default().push(entry.layer);
        }
        for (boundary, children) in order {
            if let Some(entry) = self.layers.get(&boundary) {
                layers.set_child_order(entry.layer, children);
            }
        }

        self.previous_layer_count = layers.len();
    }

    /// Record one boundary's subtree, stopping at any boundary below it, and
    /// report where in the recording each of those stops happened.
    ///
    /// The second half is what makes compositing put a child boundary back where
    /// it belongs rather than on top of everything its parent drew. It is
    /// measured *here*, while recording, because the index is only meaningful
    /// for the command list being built right now — see
    /// [`LayerTree::set_slot`](vieww_paint::LayerTree::set_slot).
    fn paint_boundary(
        &self,
        boundary: RenderId,
        scene: &mut Scene,
        dpr: f32,
    ) -> Vec<(RenderId, usize)> {
        let mut slots = Vec::new();
        self.paint_into_layer(boundary, Offset::ZERO, scene, true, &mut slots, dpr);
        slots
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_into_layer(
        &self,
        id: RenderId,
        parent_origin: Offset,
        scene: &mut Scene,
        layer_root: bool,
        slots: &mut Vec<(RenderId, usize)>,
        dpr: f32,
    ) {
        if !layer_root && self.is_repaint_boundary(id) {
            // Below a nested boundary is somebody else's recording. Drawing it
            // here would both double it up and defeat the boundary, since this
            // layer would then have to re-record whenever that subtree changed.
            //
            // The hole that leaves is the child's slot, and this early return is
            // the only place it exists to be measured.
            //
            // Taken inside an open group too. Splicing there is safe — a child's
            // composited commands are balanced, so inserting them between two of
            // this recording's commands cannot unbalance the stack — and
            // `LayerTree::composite` suppresses the child's own markers when it
            // lands inside its parent's, so the fade is applied once. Leaving it
            // to the end instead would put an overlay recorded after the group
            // underneath a boundary recorded inside it.
            slots.push((id, scene.len()));
            return;
        }

        let node = self.node(id);
        // The layer's transform already carries the boundary's position, so the
        // boundary itself sits at the layer's origin.
        let origin = if layer_root {
            Offset::ZERO
        } else {
            parent_origin + node.offset
        };

        if let Some(object) = node.object.as_deref() {
            let mut ctx = PaintCtx {
                canvas: scene,
                origin,
                size: node.size,
                dpr,
            };
            // **Catch panics in the incremental `paint`.** Same reasoning as
            // `paint_node`'s catch.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                object.paint(&mut ctx);
            }))
            .is_err()
            {
                crate::owner::report_render_panic(object.debug_name(), "paint (layer)");
            }
            if object.skips_children() {
                // Mounted, and out of the picture. Painting a covered route is
                // the cost keeping every route on the stack would otherwise
                // charge on every frame.
                return;
            }
        }

        for &child in &node.children {
            self.paint_into_layer(child, origin, scene, false, slots, dpr);
        }

        if let Some(object) = node.object.as_deref() {
            let mut ctx = PaintCtx {
                canvas: scene,
                origin,
                size: node.size,
                dpr,
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                object.paint_children_done(&mut ctx);
            }))
            .is_err()
            {
                crate::owner::report_render_panic(
                    object.debug_name(),
                    "paint_children_done (layer)",
                );
            }
        }
    }

    /// The layer a repaint boundary records into, once a frame has assigned one.
    #[must_use]
    pub fn layer_of(&self, boundary: RenderId) -> Option<LayerId> {
        self.layers.get(&boundary).map(|entry| entry.layer)
    }

    // -------------------------------------------------------------- hit testing

    /// Find every render object under `point`, in root-to-topmost order.
    ///
    /// Walks children in **reverse** paint order, so the thing drawn last — the
    /// thing visually on top — is found first and ends up last in the result,
    /// where [`HitTestResult::target`] picks it up.
    /// The topmost object under `point`, if anything is there.
    ///
    /// The other half of the inspector seam, and the reason it is a method
    /// rather than `hit_test(...).target()` at every call site: what an
    /// inspector wants is *the thing you clicked*, and getting that out of a
    /// `HitTestResult` means knowing that the list is ordered with the topmost
    /// first. That is an invariant of `hit_test`'s reverse walk, documented on
    /// `HitTestResult::target` — and a caller that has to know it is a caller
    /// that can get it wrong.
    #[must_use]
    pub fn hit_test_identify(&self, point: Offset) -> Option<RenderId> {
        self.hit_test(point).target().map(|entry| entry.id)
    }

    /// What `id` is, for a reader.
    ///
    /// `None` for an id that is not in this tree, which is a stale id rather
    /// than a bug worth panicking over — an inspector holds one across frames
    /// and the tree is free to have dropped it.
    #[must_use]
    pub fn describe(&self, id: RenderId) -> Option<NodeDescription> {
        if !self.is_alive(id) {
            return None;
        }
        let node = self.node(id);
        Some(NodeDescription {
            id,
            name: node
                .object
                .as_deref()
                .map_or("(empty)", RenderObject::debug_name),
            size: node.size,
            offset: node.offset,
            children: node.children.len(),
            properties: node
                .object
                .as_deref()
                .map(RenderObject::describe)
                .unwrap_or_default(),
        })
    }

    /// `root` and everything under it, depth first, in paint order.
    ///
    /// Depth first and in child order, because that is both how the tree paints
    /// and how a reader expects an indented list to run. The depth is carried
    /// on each entry rather than left to be reconstructed: a flat list with a
    /// depth is what a virtualised list view can render, and a nested structure
    /// is not.
    ///
    /// Bounded by `limit` nodes. A render tree has no size limit and an
    /// inspector pane does; walking a hundred thousand nodes to show eleven of
    /// them is the kind of thing that only shows up on somebody else's machine.
    #[must_use]
    pub fn describe_subtree(&self, root: RenderId, limit: usize) -> Vec<(usize, NodeDescription)> {
        let mut out = Vec::new();
        let mut stack = vec![(root, 0_usize)];
        while let Some((id, depth)) = stack.pop() {
            if out.len() >= limit {
                break;
            }
            let Some(description) = self.describe(id) else {
                continue;
            };
            out.push((depth, description));
            // Reversed, because the stack pops last-in first and the children
            // have to come out in the order they paint.
            for &child in self.children(id).iter().rev() {
                stack.push((child, depth + 1));
            }
        }
        out
    }

    #[must_use]
    pub fn hit_test(&self, point: Offset) -> HitTestResult {
        let Some(root) = self.root else {
            return HitTestResult::new();
        };
        // **Two passes: where things are drawn, then where they can be
        // reached.**
        //
        // `RenderGestureDetector` widens its hit area to the theme's minimum
        // touch target, so a control smaller than a fingertip is still
        // reachable. Applied to controls that sit *next to each other* — two
        // 24-point icon buttons in a strip, or a six-point divider in a
        // six-point gutter — the widened areas overlap, and the one on top wins
        // the overlap outright. The one underneath then cannot be pressed at
        // its own centre.
        //
        // That is not theoretical. Reported from use, on this repository's own
        // studio: the tab strip's "+" was painted across x 948.9–957.1 and
        // could only be pressed across 929–944, because the invisible divider
        // beside it had claimed everything from 945 rightwards. The sidebar
        // header's two icons, the title bar's two, and the panel's two had the
        // same defect — in each pair, the left one was dead at its own centre.
        //
        // The rule that fixes all of them at once, and cannot reintroduce the
        // problem elsewhere: **a control is always reachable where it is
        // drawn.** Expansion is a fallback for a press that hit nothing, not a
        // claim over a neighbour. So the first pass tests every object against
        // its real layout box; only if that finds nothing does the second pass
        // allow the widened boxes in.
        //
        // The second pass costs a second walk, and only on a press that landed
        // on no control at all — which is the press where nothing else is
        // happening anyway.
        let mut result = HitTestResult::new();
        if self.hit_test_node(root, point, &mut result, false) {
            return result;
        }
        let mut result = HitTestResult::new();
        self.hit_test_node(root, point, &mut result, true);
        result
    }

    fn hit_test_node(
        &self,
        id: RenderId,
        point: Offset,
        result: &mut HitTestResult,
        expanded: bool,
    ) -> bool {
        let node = self.node(id);
        let local = point - node.offset;
        let transform = node.object.as_deref().and_then(RenderObject::transform);

        // The layout box is a rejection test only for an object that paints
        // inside it. A transformed subtree can be drawn anywhere — that is what
        // a transform is for — so its box says nothing about where a finger has
        // to land to reach it.
        //
        // Normally the layout box; wider for an object that must be reachable
        // where it does not draw. See `RenderObject::hit_bounds`.
        let tight = Rect::from_origin_size(Offset::ZERO, node.size);
        let bounds = if expanded {
            node.object
                .as_deref()
                .map_or(tight, |object| object.hit_bounds(node.size))
        } else {
            tight
        };
        // **Pruned against the inflated box and decided against the precise
        // one.** A node that is merely on the way to an expanded descendant has
        // to be descended into without itself becoming a target — a `Semantics`
        // wrapper sized exactly to the control inside it is how every
        // third-party control is built, and pruning it on its own box is what
        // made touch-target expansion stop at the first ancestor. See
        // `max_hit_slop` for why the inflation is one tree-wide number.
        // The prune still allows for the slop even on the tight pass: an
        // ancestor sized exactly to its child must be descended into so that a
        // *descendant's* expansion can be reached on the second pass, and on
        // the first pass the extra reach costs one containment test.
        if transform.is_none() && !bounds.inflate(self.max_hit_slop).contains(local) {
            return false;
        }

        // **A subtree that declines the pointer is not entered at all.** Not
        // "hit and then ignored": an object inside it must not be able to
        // report a hit either, or the node behind would still be missed. See
        // `RenderObject::takes_pointers` and `RenderIgnorePointer`.
        if node
            .object
            .as_deref()
            .is_some_and(|object| !object.takes_pointers())
        {
            return false;
        }

        // Paint applies the transform on the way down; hit testing undoes it.
        // A transform with no inverse has collapsed the subtree to a line or a
        // point, which draws nothing and so can be hit by nothing.
        let local = match transform {
            Some(transform) => match transform.invert() {
                Some(inverse) => inverse.apply(local),
                None => return false,
            },
            None => local,
        };

        let mut hit = false;

        // Topmost child first, unless they are out of the picture — something
        // nobody can see must not be pressable.
        if !self.skips_children(id) {
            for &child in node.children.iter().rev() {
                if self.hit_test_node(child, local, result, expanded) {
                    hit = true;
                    break;
                }
            }
        }

        if !hit {
            if let Some(object) = node.object.as_deref() {
                if bounds.contains(local) {
                    // **Catch panics in `hit_test_self`.** A render object whose
                    // hit-test panics returns `false` — the spot is dead until
                    // the panic is fixed — rather than aborting the process.
                    hit = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        object.hit_test_self(local, node.size)
                    })) {
                        Ok(result) => result,
                        Err(_) => {
                            crate::owner::report_render_panic(object.debug_name(), "hit_test");
                            false
                        }
                    };
                }
            }
        }

        if hit {
            // Recorded while unwinding, so the deepest hit lands first and the
            // list runs from the target outwards. See `HitTestResult`.
            result.push(id, local);
        }
        hit
    }

    // -------------------------------------------------------------------- debug

    /// Render the tree as an indented string with sizes, offsets and layout
    /// counts.
    #[must_use]
    pub fn debug_tree(&self) -> String {
        let Some(root) = self.root else {
            return String::from("<empty>");
        };
        let mut out = String::new();
        self.write_node(&mut out, root, "", true, true);
        out.trim_end().to_owned()
    }

    fn write_node(&self, out: &mut String, id: RenderId, prefix: &str, last: bool, root: bool) {
        let node = self.node(id);
        let connector = if root {
            ""
        } else if last {
            "└─ "
        } else {
            "├─ "
        };
        let name = node
            .object
            .as_deref()
            .map_or("<borrowed>", RenderObject::debug_name);

        let _ = writeln!(
            out,
            "{prefix}{connector}{name} {} at {} layouts={}",
            node.size, node.offset, node.layout_count
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

    /// Every live id, depth-first from the root.
    #[must_use]
    pub fn ids(&self) -> Vec<RenderId> {
        let mut out = Vec::new();
        if let Some(root) = self.root {
            self.collect(root, &mut out);
        }
        out
    }

    fn collect(&self, id: RenderId, out: &mut Vec<RenderId>) {
        out.push(id);
        for &child in &self.node(id).children {
            self.collect(child, out);
        }
    }

    /// Total layout calls across the whole tree — the number the exit test
    /// watches when it changes one leaf.
    #[must_use]
    pub fn total_layouts(&self) -> u32 {
        self.ids().into_iter().map(|id| self.layout_count(id)).sum()
    }

    /// Total number of actual layout executions since this tree was created.
    /// Unlike [`Self::total_layouts`], this is O(1) and is intended for frame
    /// instrumentation and invalidation classification.
    #[must_use]
    pub const fn layout_runs(&self) -> u64 {
        self.layout_runs
    }
}

impl std::fmt::Debug for RenderTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTree")
            .field("objects", &self.len())
            .field("root", &self.root)
            .finish()
    }
}

/// How far outside its own box an object asked to be reachable.
///
/// The largest overhang on any edge, and never negative — an object whose
/// `hit_bounds` is *smaller* than its layout box is asking for less reach, not
/// for a negative amount of it, and the prune must not shrink for that.
fn hit_slop_of(object: &dyn RenderObject, size: Size) -> f32 {
    let bounds = object.hit_bounds(size);
    let overhang = (-bounds.left)
        .max(-bounds.top)
        .max(bounds.right - size.width)
        .max(bounds.bottom - size.height);
    overhang.max(0.0)
}

#[cfg(test)]
mod boundary_cache_tests {
    //! Tests for the incremental boundary collection (ARCHITECTURE-AUDIT finding 3).
    //!
    //! Two properties, each pinned by a test that *fails* if it stops holding:
    //!
    //! 1. **Correctness** — `cached_boundaries_equal_uncached_through_mutations`
    //!    drives a tree through every mutation class and compares the cached
    //!    walk to the from-scratch oracle after each. The sample tree is built so
    //!    that each mutation genuinely *changes the boundary list* (a fade sits
    //!    above one boundary, a clip above another) — without that, cached and
    //!    uncached agree trivially and the test carries no signal. Reverting
    //!    `bump_subtree` at any one site makes the matching mutation stale-cache.
    //! 2. **Work** — `an_idle_frame_walks_no_subtrees` and
    //!    `a_single_mutation_walks_only_its_spine` pin that unchanged subtrees
    //!    are cache hits, counted by `boundary_walks`.

    use super::*;
    use vieww_foundation::{Constraints, Offset, Rect, Size};

    /// A render object whose boundary status, layer effect and clip are
    /// configurable, so a test can build a tree with fades and clips above
    /// boundaries without depending on any widget.
    #[derive(Debug)]
    struct Stub {
        boundary: bool,
        effect: Option<LayerEffect>,
        clip: bool,
    }

    impl Stub {
        fn boundary() -> Box<Self> {
            Box::new(Self {
                boundary: true,
                effect: None,
                clip: false,
            })
        }
        /// A non-boundary that declares a fade. Placed *above* a boundary, its
        /// effect becomes that boundary's `effect` field — so changing the alpha
        /// changes the boundary list.
        fn fade(alpha: f32) -> Box<Self> {
            Box::new(Self {
                boundary: false,
                effect: Some(LayerEffect::new(alpha)),
                clip: false,
            })
        }
        /// A non-boundary that clips to its own bounds. Placed above a boundary,
        /// its clip becomes that boundary's `effect.clip` — so moving or resizing
        /// it changes the boundary list.
        fn clipper() -> Box<Self> {
            Box::new(Self {
                boundary: false,
                effect: None,
                clip: true,
            })
        }
        fn leaf() -> Box<Self> {
            Box::new(Self {
                boundary: false,
                effect: None,
                clip: false,
            })
        }
    }

    impl RenderObject for Stub {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            constraints.biggest()
        }
        fn is_repaint_boundary(&self) -> bool {
            self.boundary
        }
        fn layer_effect(&self) -> Option<LayerEffect> {
            self.effect
        }
        fn layer_clip(&self, bounds: Rect) -> Option<Rect> {
            self.clip.then_some(bounds)
        }
        fn debug_name(&self) -> &'static str {
            "Stub"
        }
    }

    /// root (boundary)
    /// ├─ fade (non-boundary, alpha 0.5)        <- a's effect comes from here
    /// │  └─ a (boundary)
    /// │     └─ a1 (leaf)
    /// └─ clipper (non-boundary, clips to self) <- b's effect.clip comes from here
    ///    └─ b (boundary)
    ///       └─ b1 (leaf)
    ///
    /// Every mutation below changes the boundary list, which is what makes the
    /// oracle comparison carry signal rather than agree trivially.
    fn sample_tree() -> RenderTree {
        let mut t = RenderTree::new();
        let root = t.insert(None, Stub::boundary());
        let fade = t.insert(Some(root), Stub::fade(0.5));
        let a = t.insert(Some(fade), Stub::boundary());
        let a1 = t.insert(Some(a), Stub::leaf());
        let clipper = t.insert(Some(root), Stub::clipper());
        let b = t.insert(Some(clipper), Stub::boundary());
        let b1 = t.insert(Some(b), Stub::leaf());
        t.set_children(root, vec![fade, clipper]);
        t.set_children(fade, vec![a]);
        t.set_children(a, vec![a1]);
        t.set_children(clipper, vec![b]);
        t.set_children(b, vec![b1]);
        t.set_offset(clipper, Offset::new(10.0, 0.0));
        t.layout(root, Constraints::tight(Size::new(100.0, 100.0)));
        t
    }

    #[test]
    fn cached_boundaries_equal_uncached_through_mutations() {
        let mut t = sample_tree();
        let root = t.root().unwrap();

        // Cold: the cache is empty.
        assert_eq!(t.boundaries(), t.boundaries_uncached(), "cold");
        // Warm idle: every subtree is a hit.
        assert_eq!(t.boundaries(), t.boundaries_uncached(), "warm idle");

        // Mutation 1 — structure: detach `b` (a boundary) from `clipper`.
        // The boundary list loses `b`, so a missing bump here stale-caches it.
        let clipper = t.children(root)[1];
        let b = t.children(clipper)[0];
        t.set_children(clipper, vec![]);
        assert_eq!(
            t.boundaries(),
            t.boundaries_uncached(),
            "after detaching a boundary"
        );
        t.set_children(clipper, vec![b]); // restore
        assert_eq!(t.boundaries(), t.boundaries_uncached(), "after reattaching");

        // Mutation 2 — object/effect: change the fade's alpha. `a`'s effect
        // field changes, so the boundary list changes.
        let fade = t.children(root)[0];
        t.replace_object(fade, Stub::fade(0.8));
        assert_eq!(
            t.boundaries(),
            t.boundaries_uncached(),
            "after fade alpha change"
        );

        // Mutation 3 — offset: move the clipper. `b`'s effect.clip changes
        // (its bounds moved), so the boundary list changes.
        t.set_offset(clipper, Offset::new(40.0, 0.0));
        assert_eq!(
            t.boundaries(),
            t.boundaries_uncached(),
            "after clipper offset"
        );

        // Mutation 4 — resize: relayout the root bigger. The clipper fills, so
        // its bounds grow, so `b`'s effect.clip changes.
        t.layout(root, Constraints::tight(Size::new(200.0, 200.0)));
        assert_eq!(t.boundaries(), t.boundaries_uncached(), "after root resize");

        // Mutation 5 — remove: drop `b`'s subtree entirely.
        t.remove(b);
        assert_eq!(
            t.boundaries(),
            t.boundaries_uncached(),
            "after removing a boundary subtree"
        );
    }

    #[test]
    fn an_idle_frame_walks_no_subtrees() {
        let mut t = sample_tree();
        let _ = t.boundaries();
        let before = t.boundary_walks();
        let _ = t.boundaries();
        assert_eq!(
            t.boundary_walks(),
            before,
            "an idle frame rewalked subtrees"
        );
    }

    /// `RenderOwner::sync` re-asserts the root on every frame. That must not
    /// throw the cache away — it did, and no production frame ever reused a
    /// subtree. `an_idle_frame_walks_no_subtrees` could not see it, because it
    /// never set the root between its two walks.
    #[test]
    fn re_asserting_the_same_root_keeps_the_boundary_cache() {
        let mut t = sample_tree();
        let root = t.root();
        let _ = t.boundaries();
        let before = t.boundary_walks();
        t.set_root(root);
        let _ = t.boundaries();
        assert_eq!(
            t.boundary_walks(),
            before,
            "setting the root it already had discarded the boundary cache"
        );
        assert_eq!(t.boundaries(), t.boundaries_uncached());
    }

    #[test]
    fn a_single_clipper_offset_change_walks_only_its_spine() {
        let mut t = sample_tree();
        let root = t.root().unwrap();
        let _ = t.boundaries();
        let before = t.boundary_walks();

        // Move the clipper — only the clipper→root spine invalidates; `fade`'s
        // whole subtree (fade, a, a1) must stay a cache hit.
        let clipper = t.children(root)[1];
        t.set_offset(clipper, Offset::new(25.0, 25.0));

        let _ = t.boundaries();
        let walked = t.boundary_walks() - before;
        // The spine is clipper + root (2 nodes); the fade subtree (3 nodes) is a hit.
        assert!(
            walked <= 3,
            "walked {walked} subtrees for one clipper move; the fade subtree should have been a cache hit"
        );
    }

    // ---- reconciliation (ARCHITECTURE-AUDIT finding 3, second half) --------

    /// The shape of a layer tree, as a string: every layer's place, transform,
    /// effect and child order, reached from the root.
    ///
    /// The oracle for reconciliation. Comparing this before and after a forced
    /// full reconcile is what proves the skip changed nothing — an assertion
    /// about `reconciles()` alone would pass for an implementation that skipped
    /// work it needed to do.
    fn dump(layers: &LayerTree) -> String {
        fn walk(layers: &LayerTree, id: LayerId, depth: usize, out: &mut String) {
            let layer = layers.layer(id);
            out.push_str(&format!(
                "{:indent$}{:?} t={:?} screen={:?} effect={:?}\n",
                "",
                id.index(),
                layer.transform(),
                layers.screen_transform(id),
                layers.effect(id),
                indent = depth * 2,
            ));
            for &child in layer.children() {
                walk(layers, child, depth + 1, out);
            }
        }
        let mut out = String::new();
        if let Some(root) = layers.root() {
            walk(layers, root, 0, &mut out);
        }
        out
    }

    /// Reconcile, then reconcile again with the retained list thrown away, and
    /// return both dumps. Equal dumps mean the skip was sound.
    fn incremental_and_forced(t: &mut RenderTree, layers: &mut LayerTree) -> (String, String) {
        t.paint_layers(layers);
        let incremental = dump(layers);
        t.previous_boundaries.clear();
        t.paint_layers(layers);
        (incremental, dump(layers))
    }

    #[test]
    fn the_skipped_reconcile_leaves_the_same_layer_tree_as_a_forced_one() {
        let mut t = sample_tree();
        let mut layers = LayerTree::with_root();
        let root = t.root().unwrap();

        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "cold");
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "warm idle");

        // The same mutation classes the boundary-cache oracle uses, because the
        // same classes are what can invalidate a layer.
        let clipper = t.children(root)[1];
        let detached = t.children(clipper)[0];
        t.set_children(clipper, vec![]);
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after detaching a boundary");

        t.set_children(clipper, vec![detached]);
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after reattaching");

        let fade = t.children(root)[0];
        t.replace_object(fade, Stub::fade(0.8));
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after a fade alpha change");

        t.set_offset(clipper, Offset::new(40.0, 0.0));
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after moving the clipper");

        t.layout(root, Constraints::tight(Size::new(200.0, 200.0)));
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after a resize");

        t.remove(detached);
        let (a, b) = incremental_and_forced(&mut t, &mut layers);
        assert_eq!(a, b, "after removing a boundary subtree");
    }

    #[test]
    fn an_idle_frame_does_not_reconcile() {
        let mut t = sample_tree();
        let mut layers = LayerTree::with_root();
        t.paint_layers(&mut layers);
        let before = t.reconciles();

        t.paint_layers(&mut layers);
        t.paint_layers(&mut layers);
        assert_eq!(
            t.reconciles(),
            before,
            "two idle frames reconciled the layer tree"
        );
    }

    #[test]
    fn a_change_that_moves_a_boundary_does_reconcile() {
        let mut t = sample_tree();
        let mut layers = LayerTree::with_root();
        t.paint_layers(&mut layers);
        let before = t.reconciles();

        let root = t.root().unwrap();
        let clipper = t.children(root)[1];
        t.set_offset(clipper, Offset::new(60.0, 5.0));
        t.paint_layers(&mut layers);
        assert_eq!(t.reconciles(), before + 1, "a real change must reconcile");
    }

    /// The trap that makes `Boundary::origin` load-bearing rather than tidy.
    ///
    /// A subtree that merely **moves** produces a boundary list with the same
    /// ids, the same parents and the same effects. Without the origin in the
    /// comparison the two lists are equal, the phase is skipped, and every
    /// layer under the moved subtree keeps drawing at its old position — a
    /// silent, wrong picture, which is worse than the work that was saved.
    #[test]
    fn a_boundary_that_only_moves_still_moves_its_layer() {
        let mut t = sample_tree();
        let mut layers = LayerTree::with_root();
        let root = t.root().unwrap();
        t.paint_layers(&mut layers);
        let before = dump(&layers);

        // `b` is a boundary under `clipper`; moving `b` itself changes nothing
        // about the *shape* of the boundary list, only where one boundary sits.
        let clipper = t.children(root)[1];
        let b = t.children(clipper)[0];
        t.set_offset(b, Offset::new(37.0, 11.0));
        t.paint_layers(&mut layers);

        let after = dump(&layers);
        assert_ne!(before, after, "the moved boundary's layer did not move");

        // Exactly where it should be, not merely "different". `b`'s enclosing
        // boundary is the root — `clipper` is not one — so the layer transform
        // is the sum of the offsets between them: (10, 0) + (37, 11).
        let entry = t.layers[&b];
        assert_eq!(
            layers.layer(entry.layer).transform(),
            Transform::translate(Offset::new(47.0, 11.0)),
            "the layer is at the wrong place, not merely at a different one"
        );
    }

    #[test]
    fn an_empty_tree_collects_nothing_without_panicking() {
        let mut t = RenderTree::new();
        assert!(t.boundaries().is_empty());
        assert!(t.boundaries_uncached().is_empty());
        assert_eq!(t.boundary_walks(), 0);
    }
}

#[cfg(test)]
mod describe_tests {
    use super::*;
    use crate::objects::{RenderConstrainedBox, RenderFlex, RenderPadding};
    use vieww_foundation::{Axis, EdgeInsets};

    /// The whole point of the seam: an application holds a widget tree and
    /// never a render one, so without this there is no way to ask what is on
    /// screen — and the studio's Inspector was five lines of buffer statistics.
    #[test]
    fn a_subtree_reads_out_depth_first_in_paint_order() {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderFlex::new(Axis::Vertical)));
        tree.set_root(Some(root));
        let first = tree.insert(
            Some(root),
            Box::new(RenderPadding::new(EdgeInsets::all(8.0))),
        );
        tree.insert(
            Some(first),
            Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
                10.0, 10.0,
            )))),
        );
        tree.insert(
            Some(root),
            Box::new(RenderPadding::new(EdgeInsets::all(4.0))),
        );

        let described = tree.describe_subtree(root, 64);
        let shape: Vec<(usize, &str)> = described
            .iter()
            .map(|(depth, node)| (*depth, node.name))
            .collect();
        assert_eq!(
            shape,
            [
                // `debug_name` says which way it runs.
                (0, "RenderColumn"),
                (1, "RenderPadding"),
                (2, "RenderConstrainedBox"),
                (1, "RenderPadding"),
            ]
        );

        // And the properties are the object's own, not the tree's.
        let flex = &described[0].1;
        assert_eq!(flex.children, 2);
        assert!(flex
            .properties
            .iter()
            .any(|(name, value)| *name == "direction" && value == "Vertical"));
    }

    #[test]
    fn the_walk_is_bounded() {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderFlex::new(Axis::Vertical)));
        tree.set_root(Some(root));
        for _ in 0..50 {
            tree.insert(
                Some(root),
                Box::new(RenderPadding::new(EdgeInsets::all(1.0))),
            );
        }
        assert_eq!(tree.describe_subtree(root, 7).len(), 7);
    }

    /// An inspector holds an id across frames and the tree is free to have
    /// dropped it. That is a stale id, not a bug worth panicking over.
    #[test]
    fn a_stale_id_describes_as_nothing_rather_than_panicking() {
        let mut tree = RenderTree::new();
        let root = tree.insert(None, Box::new(RenderFlex::new(Axis::Vertical)));
        tree.set_root(Some(root));
        let child = tree.insert(
            Some(root),
            Box::new(RenderPadding::new(EdgeInsets::all(1.0))),
        );
        tree.remove(child);
        assert_eq!(tree.describe(child), None);
    }
}

/// One render object, as a reader sees it.
///
/// Produced by [`RenderTree::describe`]. Everything here is a fact the tree
/// already had — the object contributes only [`properties`](Self::properties),
/// through [`RenderObject::describe`].
#[derive(Debug, Clone, PartialEq)]
pub struct NodeDescription {
    pub id: RenderId,
    /// The object's `debug_name`, or `(empty)` for a node with nothing in it.
    pub name: &'static str,
    /// What layout gave it.
    pub size: Size,
    /// Where its parent put it, in the parent's coordinates.
    pub offset: Offset,
    pub children: usize,
    /// Whatever the object thought was worth saying about itself.
    pub properties: Vec<(&'static str, String)>,
}
