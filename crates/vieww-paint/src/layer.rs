//! Repaint boundaries.
//!
//! A [`Layer`] is a subtree that keeps its own recording. When something inside
//! it changes, only that recording is redone; the surrounding layers keep the
//! commands they already had and the frame is reassembled by
//! [`LayerTree::composite`].
//!
//! # Why not a layer per render object
//!
//! A layer costs memory for its recording and a step in every composite. Most
//! render objects change only when their parent changes, so a layer for each
//! would pay that cost for nothing. The default is therefore to paint into the
//! parent's layer, and a render object **opts in** when it expects to repaint
//! independently — a scrolling viewport, an animating transform, a video frame.
//! The same line is drawn by `RepaintBoundary`, and the guidance is the
//! same: too many layers is as slow as too few.
//!
//! # What a boundary actually buys
//!
//! [`mark_needs_paint`](LayerTree::mark_needs_paint) does **not** propagate to
//! the parent. That single absence is the whole mechanism: a repaint deep in the
//! tree stops at the nearest boundary instead of walking to the root, so the
//! cost of a change is bounded by the layer containing it rather than by the size
//! of the screen.

use std::fmt;

use vieww_foundation::{BlendMode, Rect, Transform};

use crate::{Canvas, Damage, Scene};
use vieww_foundation::FastSet;

/// A handle to a layer in a [`LayerTree`].
///
/// Generational for the same reason `RenderId` and `ElementId` are: slots are
/// reused when a subtree is torn down, so a stale handle must compare unequal
/// rather than silently address whatever landed there next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId {
    index: u32,
    generation: u32,
}

impl LayerId {
    const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    /// The arena slot this id addresses.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// How many times that slot had been reused when this id was issued.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "l{}v{}", self.index, self.generation)
    }
}

/// How a layer's whole subtree is composited into its parent.
///
/// # Why this is a value and not a list of known widgets
///
/// A layer carries one of these because *something above it* asked for its
/// subtree to be composited as a group — an `Opacity` today, and a blur, a
/// colour filter or a mask later. Neither this crate nor the render tree knows
/// which: the effect is read through a defaulted `RenderObject` hook, so a
/// render object written outside this repository declares one exactly as the
/// ones inside it do.
///
/// That is the whole reason it is a struct rather than an `alpha: f32`
/// parameter. Growing it is additive for implementors, who construct it through
/// [`new`](Self::new) and the builders rather than by literal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerEffect {
    /// Multiplied into the group as one, not into each primitive.
    pub alpha: f32,
    /// How the composited group mixes with what is beneath it.
    pub blend: BlendMode,
    /// What the group is confined to, in the **enclosing layer's** coordinate
    /// space — the same space [`Layer::transform`] maps out of.
    ///
    /// `None` is "not clipped", which is not the same as an empty rectangle:
    /// [`Rect::ZERO`] here means *nothing is visible*, and that distinction is
    /// what lets a clipped-away subtree cost nothing rather than everything.
    ///
    /// # Why a clip is part of how a subtree composites
    ///
    /// For exactly the reason `alpha` is. A clip recorded by an ancestor is
    /// folded into that ancestor's commands as they are recorded, so it cannot
    /// reach a descendant repaint boundary — the boundary records into a
    /// different scene, which the ancestor's `save`/`restore` never touches. A
    /// `Viewport` both clips *and* is a boundary, so without this every
    /// `RepaintBoundary` inside a scrollable painted outside it.
    pub clip: Option<Rect>,
}

impl LayerEffect {
    /// A group faded to `alpha`, mixed normally.
    #[must_use]
    pub fn new(alpha: f32) -> Self {
        Self {
            alpha: if alpha.is_nan() {
                // Same rule as `RenderOpacity`: a `NaN` from a division is an
                // invisible subtree, not a panic in a compositor.
                0.0
            } else {
                alpha.clamp(0.0, 1.0)
            },
            blend: BlendMode::Normal,
            clip: None,
        }
    }

    /// Mix the group with `blend` rather than normally.
    #[must_use]
    pub const fn blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
    }

    /// Confine the group to `rect`, intersected with whatever it is already
    /// confined to.
    ///
    /// Intersection rather than replacement, because two clips between the same
    /// pair of boundaries both apply — a list row inside a viewport is bounded
    /// by both, and taking the innermost would let content out of the outer one.
    #[must_use]
    pub fn clipped_to(mut self, rect: Rect) -> Self {
        self.clip = Some(match self.clip {
            Some(existing) => existing.intersect(rect),
            None => rect,
        });
        self
    }

    /// `true` if the group needs `PushLayer`/`PopLayer` around it.
    ///
    /// A clip deliberately does **not** count. It is a `save`/`clip`/`restore`,
    /// which costs no compositing target, so a clipped group that is otherwise
    /// untouched should not be given one.
    #[must_use]
    pub fn groups(self) -> bool {
        self.alpha < 1.0 || !self.blend.is_normal()
    }

    /// Nothing to do — full strength, mixed normally, unclipped.
    ///
    /// The case worth having a name for, because it is what lets `composite`
    /// skip every marker rather than emitting pairs that do nothing and that
    /// `Damage` would then have to reason about.
    #[must_use]
    pub fn is_a_no_op(self) -> bool {
        !self.groups() && self.clip.is_none()
    }

    /// Combine with an effect from further up, for a run of them between two
    /// boundaries.
    ///
    /// Alphas multiply, because two halves are a quarter. Blends do **not**
    /// compose — there is no meaningful product of two mix modes — so the
    /// innermost non-normal one wins and that is a documented limitation rather
    /// than an accident. Clips intersect, which is the one of the three that
    /// composes exactly.
    #[must_use]
    pub fn then(self, outer: Self) -> Self {
        Self {
            alpha: (self.alpha * outer.alpha).clamp(0.0, 1.0),
            blend: if self.blend.is_normal() {
                outer.blend
            } else {
                self.blend
            },
            clip: match (self.clip, outer.clip) {
                (Some(inner), Some(outer)) => Some(inner.intersect(outer)),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            },
        }
    }
}

impl Default for LayerEffect {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// One repaint boundary: a recording plus where it sits in its parent.
#[derive(Debug)]
pub struct Layer {
    /// Maps this layer's coordinate space into its parent's.
    transform: Transform,
    /// How this layer's subtree composites into its parent.
    ///
    /// Set from whatever encloses the boundary, which is how an `Opacity` above
    /// a boundary reaches content the boundary records separately. Without it
    /// the fade applies to everything except the one subtree that owns a layer,
    /// silently.
    effect: LayerEffect,
    scene: Scene,
    /// What this layer had recorded at the end of the last frame.
    ///
    /// Kept so damage can be the *difference* between two recordings rather than
    /// the layer's whole bounds. Without it, a boundary wrapping the screen —
    /// which the root always is — would report the screen as pending for a
    /// one-pixel change, and no amount of layering would help.
    previous: Scene,
    parent: Option<LayerId>,
    children: Vec<LayerId>,
    /// Where in the **parent's** recording this layer's content belongs.
    ///
    /// Recording a boundary's subtree leaves a hole in its parent's command
    /// list: the parent stops at the boundary and carries on afterwards. This is
    /// the index that hole was left at, so `composite` can put the child back
    /// where it was rather than on top of everything.
    ///
    /// Without it a boundary composites above its parent's *whole* recording,
    /// and anything the parent painted after reaching the boundary — a menu over
    /// a scrolled page, a snackbar over a list — goes underneath it.
    ///
    /// [`Self::AT_END`] is "nobody measured one", which is the old behaviour and
    /// the right answer for a layer whose hole was inside a group.
    slot: usize,
    needs_paint: bool,
    /// Where this layer's content sat on screen at the end of the last frame.
    ///
    /// Screen space, not local, so that a layer which *moved* still reports the
    /// pixels it vacated. Storing local bounds would lose exactly the case
    /// damage tracking most needs to get right.
    last_screen_bounds: Rect,
    /// The transform to the screen at the end of the last frame.
    ///
    /// A layer that moved is not diffable: its recording is unchanged, so a
    /// command diff reports nothing, while every pixel it covers moved. Comparing
    /// against this is how that case is told apart from a re-record in place. It
    /// is the transform to the *screen*, so an ancestor moving counts too.
    last_screen_transform: Transform,
    /// Set when something other than the recording changed the pixels — a
    /// reorder among siblings. The recording is identical, so the diff finds
    /// nothing, and the layer has to damage its bounds outright.
    bounds_damage: bool,
    /// How many times this layer has been re-recorded. Not used by the
    /// framework — it is what turns "only the necessary layers repaint" into
    /// something a test can measure.
    paint_count: u32,
}

impl Layer {
    /// A slot that no paint pass measured: composite after everything.
    ///
    /// The behaviour every layer had before slots existed, kept as the default
    /// rather than as a special case — see [`LayerTree::set_slot`] for the one
    /// situation that still wants it.
    pub const AT_END: usize = usize::MAX;

    /// Where in the parent's recording this layer's content belongs.
    #[must_use]
    pub const fn slot(&self) -> usize {
        self.slot
    }

    /// This layer's recording, in its own coordinate space.
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Where this layer sits in its parent's coordinate space.
    #[must_use]
    pub const fn transform(&self) -> Transform {
        self.transform
    }

    /// `true` if this layer's recording is stale.
    #[must_use]
    pub const fn needs_paint(&self) -> bool {
        self.needs_paint
    }

    /// How many times this layer has been re-recorded.
    #[must_use]
    pub const fn paint_count(&self) -> u32 {
        self.paint_count
    }

    /// This layer's children, painted on top of its own recording.
    #[must_use]
    pub fn children(&self) -> &[LayerId] {
        &self.children
    }
}

struct Slot {
    generation: u32,
    layer: Option<Layer>,
}

/// How many groups `commands` leaves open, negative if it closes more than it
/// opens.
///
/// Used by `composite` to know whether a child spliced in after this run lands
/// inside one of its parent's groups — which decides whether the child's own
/// effect is a second application of a fade already in force.
fn group_delta(commands: &[crate::Command]) -> isize {
    commands
        .iter()
        .map(|command| match command {
            crate::Command::PushLayer { .. } => 1,
            crate::Command::PopLayer => -1,
            _ => 0,
        })
        .sum()
}

/// The tree of repaint boundaries.
///
/// An arena rather than a tree of owned boxes, matching `RenderTree`, so that
/// compositing can walk into a child while holding the parent.
#[derive(Default)]
pub struct LayerTree {
    slots: Vec<Slot>,
    free: Vec<u32>,
    root: Option<LayerId>,
    /// Layers whose recordings are stale.
    ///
    /// A set rather than a flag walk, because a frame needs the pending layers
    /// without visiting the clean ones — which is the point of having
    /// boundaries at all.
    pending: FastSet<LayerId>,
    /// Screen-space bounds of layers removed since the last frame.
    ///
    /// A removed layer takes its `last_screen_bounds` with it, but the pixels it
    /// drew are still on the surface. Without this, deleting a subtree would
    /// leave it visible until something else happened to repaint over it.
    vacated: Vec<Rect>,
}

impl LayerTree {
    /// An empty tree.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A tree with a single root layer at the identity transform.
    #[must_use]
    pub fn with_root() -> Self {
        let mut tree = Self::new();
        tree.insert(None, Transform::IDENTITY);
        tree
    }

    /// The root, once something has been inserted.
    #[must_use]
    pub const fn root(&self) -> Option<LayerId> {
        self.root
    }

    /// Number of live layers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.layer.is_some())
            .count()
    }

    /// `true` if the tree holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` if the id still addresses a live layer.
    #[must_use]
    pub fn is_alive(&self, id: LayerId) -> bool {
        self.slots
            .get(id.index as usize)
            .is_some_and(|slot| slot.generation == id.generation && slot.layer.is_some())
    }

    /// The layer `id` addresses.
    ///
    /// # Panics
    ///
    /// If `id` is stale or was removed.
    #[must_use]
    pub fn layer(&self, id: LayerId) -> &Layer {
        let slot = &self.slots[id.index as usize];
        assert!(slot.generation == id.generation, "layer {id} is stale");
        slot.layer
            .as_ref()
            .unwrap_or_else(|| panic!("layer {id} is not live"))
    }

    fn layer_mut(&mut self, id: LayerId) -> &mut Layer {
        let slot = &mut self.slots[id.index as usize];
        assert!(slot.generation == id.generation, "layer {id} is stale");
        slot.layer
            .as_mut()
            .unwrap_or_else(|| panic!("layer {id} is not live"))
    }

    /// Add a layer under `parent`, or as the root when `parent` is `None`.
    ///
    /// `transform` maps the new layer's coordinate space into its parent's. The
    /// layer starts pending, since it has nothing recorded yet.
    pub fn insert(&mut self, parent: Option<LayerId>, transform: Transform) -> LayerId {
        let id = match self.free.pop() {
            Some(index) => LayerId::new(index, self.slots[index as usize].generation),
            None => {
                let index = u32::try_from(self.slots.len()).expect("layer arena overflowed u32");
                self.slots.push(Slot {
                    generation: 0,
                    layer: None,
                });
                LayerId::new(index, 0)
            }
        };

        self.slots[id.index as usize].layer = Some(Layer {
            transform,
            effect: LayerEffect::default(),
            scene: Scene::new(),
            previous: Scene::new(),
            parent,
            children: Vec::new(),
            slot: Layer::AT_END,
            needs_paint: true,
            last_screen_bounds: Rect::ZERO,
            last_screen_transform: Transform::IDENTITY,
            bounds_damage: false,
            paint_count: 0,
        });

        match parent {
            Some(parent) => self.layer_mut(parent).children.push(id),
            None => self.root = Some(id),
        }
        self.pending.insert(id);
        id
    }

    /// Remove a layer and everything below it.
    ///
    /// The pixels the removed subtree drew are recorded as damage, so the next
    /// frame paints over them.
    pub fn remove(&mut self, id: LayerId) {
        if !self.is_alive(id) {
            return;
        }
        for child in self.layer(id).children.clone() {
            self.remove(child);
        }
        if let Some(parent) = self.layer(id).parent {
            if self.is_alive(parent) {
                self.layer_mut(parent).children.retain(|&c| c != id);
            }
        }
        if self.root == Some(id) {
            self.root = None;
        }

        let vacated = self.layer(id).last_screen_bounds;
        if !vacated.is_empty() {
            self.vacated.push(vacated);
        }
        self.pending.remove(&id);

        let slot = &mut self.slots[id.index as usize];
        slot.layer = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
    }

    /// Move a layer within its parent.
    ///
    /// Marks the layer pending only when the transform actually changed, so a
    /// sync that reassigns the same position costs nothing.
    pub fn set_transform(&mut self, id: LayerId, transform: Transform) {
        if self.layer(id).transform == transform {
            return;
        }
        self.layer_mut(id).transform = transform;
        // The recording is still valid — only where it lands changed — but the
        // composite and the damage are not, and `pending` drives both.
        //
        // The whole subtree, not just this layer: a child composites through its
        // parent's transform, so moving a parent moves every descendant's pixels
        // while touching none of their recordings. Marking only this layer reports
        // the pixels *it* drew, which for a parent that draws nothing itself is
        // no pixels at all — damage would come back clean while the screen was
        // wrong, which is the one direction that is never safe.
        self.pending_subtree(id);
    }

    /// How this layer's subtree composites into its parent.
    #[must_use]
    pub fn effect(&self, id: LayerId) -> LayerEffect {
        self.layer(id).effect
    }

    /// Set how a layer's subtree composites into its parent.
    ///
    /// Marks it pending only when the effect actually changed, so a sync that
    /// reassigns the same value costs nothing — the same rule as
    /// [`set_transform`](Self::set_transform).
    ///
    /// `bounds_damage`, not a re-record: the recording is untouched, so a
    /// command diff between the two scenes finds *nothing* while every pixel the
    /// layer covers has changed strength. This is the same trap a reorder falls
    /// into, and it is the direction that is never safe to get wrong — damage
    /// would come back clean while the screen was stale.
    pub fn set_effect(&mut self, id: LayerId, effect: LayerEffect) {
        if self.layer(id).effect == effect {
            return;
        }
        self.layer_mut(id).effect = effect;
        self.layer_mut(id).bounds_damage = true;
        // The whole subtree, because a child composites *through* this layer's
        // effect: fading a parent changes every descendant's pixels while
        // touching none of their recordings.
        self.pending_subtree(id);
    }

    /// Add a layer and everything below it to the pending set.
    fn pending_subtree(&mut self, id: LayerId) {
        self.pending.insert(id);
        for child in self.layer(id).children.clone() {
            self.pending_subtree(child);
        }
    }

    /// Reorder a layer's children.
    ///
    /// Composite order is paint order, so a tree that reorders two overlapping
    /// subtrees changes pixels without either subtree's *content* changing. New
    /// layers are appended as they are inserted, which is the right order only
    /// when every sibling is new; a caller syncing an existing tree has to say
    /// what the order should be.
    ///
    /// Every child joins the pending set when the order changes, because the pixels
    /// that move are in the children's bounds and not in the parent's.
    ///
    /// # Panics
    ///
    /// If `order` is not a permutation of the layer's current children — a
    /// silently dropped child would vanish from the frame with nothing to explain
    /// it.
    pub fn set_child_order(&mut self, id: LayerId, order: Vec<LayerId>) {
        if self.layer(id).children == order {
            return;
        }
        assert!(
            order.len() == self.layer(id).children.len()
                && order
                    .iter()
                    .all(|child| self.layer(id).children.contains(child)),
            "layer {id}'s new child order is not a permutation of its children"
        );
        for &child in &order {
            self.pending.insert(child);
            self.layer_mut(child).bounds_damage = true;
        }
        self.pending.insert(id);
        self.layer_mut(id).children = order;
    }

    /// Record where in its parent's recording this layer's content belongs.
    ///
    /// # A slot is only meaningful for the recording it was measured in
    ///
    /// It is an index into the parent's command list, and that list is replaced
    /// wholesale every time the parent re-records. So this must be called by
    /// whatever *just* painted the parent, with the index it measured while
    /// painting — computing it anywhere else is a number about a previous frame,
    /// and the failure is an overlay that jumps behind its page one frame in
    /// four.
    ///
    /// # [`Layer::AT_END`] is a real answer, not a missing one
    ///
    /// A boundary reached while a group was open cannot be spliced into its
    /// parent's recording: the cut would fall between a `PushLayer` and its
    /// `PopLayer`. Such a layer keeps the placement it always had — after the
    /// parent's whole recording — which is also where the enclosing group's
    /// effect reaches it correctly.
    ///
    /// A layer's placement is not part of its recording, so a slot that changes
    /// damages the layer's bounds outright, the way a reorder does.
    pub fn set_slot(&mut self, id: LayerId, slot: usize) {
        let layer = self.layer_mut(id);
        if layer.slot == slot {
            return;
        }
        layer.slot = slot;
        layer.bounds_damage = true;
        self.pending.insert(id);
    }

    /// Clear a layer's recording and hand back the canvas to paint onto.
    ///
    /// Clearing is not optional: a paint pass records the layer's *whole*
    /// content, so keeping the previous commands would draw everything twice.
    ///
    /// Re-recording is itself a change, so the layer joins the pending set even if
    /// it was clean — otherwise a repaint that nobody asked for in advance would
    /// produce no damage, and the new content would never reach the screen.
    /// `needs_paint` clears because the recording is now current; `pending` stays
    /// until [`end_frame`](Self::end_frame).
    pub fn begin_paint(&mut self, id: LayerId) -> &mut Scene {
        self.pending.insert(id);
        let layer = self.layer_mut(id);
        // The recording about to be replaced becomes the one to diff against.
        // Swapping rather than cloning keeps both allocations alive across
        // frames, which matters when this runs sixty times a second.
        std::mem::swap(&mut layer.scene, &mut layer.previous);
        layer.scene.clear();
        layer.needs_paint = false;
        layer.paint_count += 1;
        &mut layer.scene
    }

    /// Mark a layer's recording stale.
    ///
    /// Deliberately does **not** propagate to the parent — see the module docs.
    /// An ancestor's commands do not change because a descendant's did, so
    /// walking upward here would throw away the entire benefit of the boundary.
    pub fn mark_needs_paint(&mut self, id: LayerId) {
        if !self.is_alive(id) {
            return;
        }
        self.layer_mut(id).needs_paint = true;
        self.pending.insert(id);
    }

    /// Every layer needing a repaint, in ascending id order.
    ///
    /// Sorted so a frame is deterministic; a `HashSet` iterates arbitrarily and
    /// that would make paint order — and test output — vary between runs.
    #[must_use]
    pub fn needs_paint(&self) -> Vec<LayerId> {
        let mut pending: Vec<LayerId> = self
            .pending
            .iter()
            .copied()
            .filter(|&id| self.is_alive(id) && self.layer(id).needs_paint)
            .collect();
        pending.sort_unstable();
        pending
    }

    /// `true` if nothing needs repainting or recompositing.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.pending.is_empty() && self.vacated.is_empty()
    }

    /// The transform mapping `id`'s space all the way to the screen.
    #[must_use]
    pub fn screen_transform(&self, id: LayerId) -> Transform {
        let mut transform = self.layer(id).transform;
        let mut current = self.layer(id).parent;
        while let Some(parent) = current {
            transform = transform.then(self.layer(parent).transform);
            current = self.layer(parent).parent;
        }
        transform
    }

    /// Where `id`'s own recording currently sits on screen.
    ///
    /// Excludes child layers; they are separate boundaries with their own
    /// bounds, and damage is accumulated per layer.
    #[must_use]
    pub fn screen_bounds(&self, id: LayerId) -> Rect {
        let bounds = self.layer(id).scene.bounds();
        if bounds.is_empty() {
            Rect::ZERO
        } else {
            self.screen_transform(id).apply_rect(bounds)
        }
    }

    /// Flatten the whole tree into one scene, in paint order.
    ///
    /// A child is spliced into its parent's recording at the index the paint
    /// pass left a hole for it ([`Layer::slot`]), so the result is the order the
    /// same drawing would have had in a single recording — including the case
    /// the parent painted something *after* reaching the boundary.
    ///
    /// Children with no measured slot go last, in child order, which is what
    /// every child did before slots existed.
    ///
    /// Backends that composite layers natively should walk the tree instead; this
    /// is for the ones that take a flat command list, and for tests.
    #[must_use]
    pub fn composite(&self) -> Scene {
        let mut scene = Scene::new();
        if let Some(root) = self.root {
            self.composite_into(&mut scene, root, Transform::IDENTITY, None, false);
        }
        scene
    }

    /// `clip` is the accumulated clip in **screen** space, or `None` for
    /// unclipped — see the body for why it is an argument rather than state.
    ///
    /// `enclosed` says this layer is being spliced into its parent's recording at
    /// a point where the parent still has a group open. That group's markers
    /// already fade what lands between them, and this layer's own effect is the
    /// *same declaration* reaching it a second way — see the note on
    /// [`Layer::effect`]. Emitting its markers too would fade it twice.
    fn composite_into(
        &self,
        scene: &mut Scene,
        id: LayerId,
        inherited: Transform,
        clip: Option<Rect>,
        enclosed: bool,
    ) {
        let layer = self.layer(id);
        let transform = layer.transform.then(inherited);

        // The markers enclose this layer's own recording *and* its children, so
        // the whole subtree fades as one group. Emitting them around only the
        // recording would fade a parent while leaving its nested boundaries at
        // full strength — which is the bug this exists to fix, one level down.
        //
        // No markers at all when there is nothing to do: a `PushLayer`/`PopLayer`
        // pair costs a compositing target in the backend and gives `Damage` a
        // layer to reason about, for an effect that changes no pixel.
        let effect = layer.effect;

        // The declared clip is in the *enclosing* layer's space — the space
        // `inherited` maps to the screen — so it goes through `inherited`, not
        // through `transform`, which has this layer's own placement folded in
        // already.
        //
        // It is carried down the recursion rather than set as scene state
        // because `Scene::append` deliberately ignores the state it appends
        // into: appended commands keep what they were recorded under. That is
        // the right contract — it is what makes a recording liftable into
        // another space without replaying it — so the clip travels as an
        // argument and is intersected into each command as it lands.
        let clip = match effect.clip {
            Some(declared) => {
                let declared = inherited.apply_rect(declared);
                Some(clip.map_or(declared, |outer| outer.intersect(declared)))
            }
            None => clip,
        };

        let grouped = effect.groups() && !enclosed;
        if grouped {
            // `Rect::ZERO` rather than a computed estimate, because
            // `Scene::pop_layer` replaces the declared bounds outright with the
            // union of what was recorded between the markers. Anything worked
            // out here would be discarded a few lines later — and a layer's own
            // bounds say nothing about where its *children* painted anyway.
            scene.push_layer(Rect::ZERO, effect.alpha, effect.blend);
        }

        // Clipping every primitive of a group is the same picture as clipping
        // the composited group, because a rectangular clip is a binary mask that
        // does not interact with the group's alpha. That equivalence is what
        // lets the clip go on the commands rather than around the markers.
        // Sorted rather than assumed ordered, because `set_child_order` orders
        // children by their position in the *render* tree while a slot is a
        // position in a command list, and the two agree only when every sibling
        // boundary was reached in the same order it is listed in. A stable sort
        // is what keeps child order as the tie-break for two boundaries reached
        // at the same index — which is every pair of adjacent boundaries, since
        // neither leaves a command behind.
        let mut children: Vec<LayerId> = layer.children.clone();
        children.sort_by_key(|&child| self.layer(child).slot);

        let total = layer.scene.len();
        let mut cut = 0usize;
        // How many of this layer's own groups are open at `cut`. Counted while
        // walking rather than asked of the scene, because the question is about
        // a *position* in a finished recording and `Scene` only knows about the
        // end of one.
        let mut open = 0usize;
        for &child in &children {
            // `min` rather than trusting the slot: it indexes a recording that
            // is replaced on every repaint of the parent, and a child re-slotted
            // against a longer previous recording would otherwise index past the
            // end on a frame where the parent got shorter. Clamping puts it at
            // the end, which is the same answer `AT_END` gives.
            let at = self.layer(child).slot.min(total).max(cut);
            if at > cut {
                scene.append_range_clipped(&layer.scene, cut..at, transform, clip);
                open = open.saturating_add_signed(group_delta(&layer.scene.commands()[cut..at]));
                cut = at;
            }
            self.composite_into(scene, child, transform, clip, open > 0);
        }
        if cut < total {
            scene.append_range_clipped(&layer.scene, cut..total, transform, clip);
        }

        if grouped {
            scene.pop_layer();
        }
    }

    /// The damage this frame's changes imply.
    ///
    /// Removed subtrees contribute the pixels they left behind. Every other pending
    /// layer contributes one of two things:
    ///
    /// - **It moved** — its transform to the screen changed, or an ancestor's
    ///   did. The recording is untouched, so a diff would report nothing while
    ///   every pixel under it changed. Both the old and the new bounds are
    ///   damaged: the first to erase, the second to draw.
    /// - **It was re-recorded in place** — the difference between the two
    ///   recordings is the damage, which is very much smaller than the layer for
    ///   the changes that actually dominate. Taking the whole bounds here instead
    ///   would make the root layer report the whole screen for a blinking caret,
    ///   and no amount of layering below would recover it.
    #[must_use]
    pub fn damage(&self, surface: Rect) -> Damage {
        let mut damage = Damage::new(surface);
        for &rect in &self.vacated {
            damage.add(rect);
        }
        for &id in &self.pending {
            if !self.is_alive(id) {
                continue;
            }
            let layer = self.layer(id);
            let transform = self.screen_transform(id);
            if layer.bounds_damage || transform != layer.last_screen_transform {
                damage.add(layer.last_screen_bounds);
                damage.add(self.screen_bounds(id));
            } else {
                damage.add_between(&layer.previous, &layer.scene, transform);
            }
        }
        damage
    }

    /// Settle the tree after a frame is presented.
    ///
    /// Records where every layer ended up, so the next frame can damage the
    /// pixels this one leaves behind, and drops the pending and vacated sets.
    pub fn end_frame(&mut self) {
        // Walked by index rather than collected first: this runs on every
        // presented frame, and the list it used to build was a steady-state
        // allocation the Vieww standard counts.
        for index in 0..self.slots.len() {
            let slot = &self.slots[index];
            if slot.layer.is_none() {
                continue;
            }
            let id = LayerId::new(
                u32::try_from(index).expect("layer arena overflowed u32"),
                slot.generation,
            );
            let bounds = self.screen_bounds(id);
            let transform = self.screen_transform(id);
            let layer = self.layer_mut(id);
            layer.last_screen_bounds = bounds;
            layer.last_screen_transform = transform;
            layer.bounds_damage = false;
        }
        self.pending.clear();
        self.vacated.clear();
    }
}

impl fmt::Debug for LayerTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LayerTree")
            .field("layers", &self.len())
            .field("root", &self.root)
            .field("pending", &self.pending.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Offset};

    use super::*;
    use crate::Canvas;

    const SURFACE: Rect = Rect::new(0.0, 0.0, 1000.0, 1000.0);

    /// Paints one rectangle into `id`, filling the layer's local space.
    fn paint(tree: &mut LayerTree, id: LayerId, rect: Rect, color: Color) {
        tree.begin_paint(id).fill_rect(rect, color.into());
    }

    /// Every `PushLayer` alpha in `scene`, in order.
    fn layer_alphas(scene: &Scene) -> Vec<f32> {
        scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                crate::Command::PushLayer { alpha, .. } => Some(*alpha),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_effect_on_a_layer_fades_its_own_recording() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        tree.set_effect(root, LayerEffect::new(0.5));

        let scene = tree.composite();
        assert_eq!(layer_alphas(&scene), vec![0.5]);
    }

    #[test]
    fn a_child_layers_pixels_are_inside_its_parents_effect() {
        // **The bug this whole mechanism exists for.** The paint walk stops at a
        // nested boundary, so a fade recorded above one never encloses its
        // commands. Here the fade lives on the layer instead, and `composite`
        // wraps the child's content in it along with the parent's own.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.set_effect(root, LayerEffect::new(0.5));

        let scene = tree.composite();
        let commands = scene.commands();
        let push = commands
            .iter()
            .position(|c| matches!(c, crate::Command::PushLayer { .. }))
            .expect("a layer was opened");
        let pop = commands
            .iter()
            .position(|c| matches!(c, crate::Command::PopLayer))
            .expect("and closed");
        let fills = commands
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, crate::Command::FillRect { .. }))
            .count();

        assert_eq!(fills, 2, "both layers recorded something");
        assert!(
            push < pop,
            "the child's fill is between the markers, not after them"
        );
        assert_eq!(pop, commands.len() - 1, "the group closes last");
    }

    #[test]
    fn a_layer_with_nothing_to_do_emits_no_markers() {
        // A pair of markers costs a compositing target in the backend and gives
        // `Damage` a layer to reason about, for an effect that changes no pixel.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        tree.set_effect(root, LayerEffect::new(1.0));

        assert!(layer_alphas(&tree.composite()).is_empty());
    }

    #[test]
    fn changing_an_effect_damages_the_layer_though_the_recording_is_identical() {
        // The trap: a command diff between the two scenes finds nothing, while
        // every pixel the layer covers changed strength. Clean damage over a
        // stale screen is the one direction that is never safe.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        tree.end_frame();
        assert!(tree.damage(SURFACE).regions().is_empty(), "settled");

        tree.set_effect(root, LayerEffect::new(0.5));
        assert!(
            !tree.damage(SURFACE).regions().is_empty(),
            "a fade that repaints nothing still changes pixels"
        );
    }

    #[test]
    fn re_declaring_the_same_effect_costs_nothing() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        tree.set_effect(root, LayerEffect::new(0.5));
        tree.end_frame();

        tree.set_effect(root, LayerEffect::new(0.5));
        assert!(
            tree.damage(SURFACE).regions().is_empty(),
            "a sync that reassigns the same value must not repaint"
        );
    }

    #[test]
    fn effects_multiply_rather_than_the_inner_one_winning() {
        let combined = LayerEffect::new(0.5).then(LayerEffect::new(0.5));
        assert!((combined.alpha - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn a_nan_alpha_is_an_invisible_group_rather_than_a_panic() {
        // Progress is usually a division, and `0.0 / 0.0` should not take down a
        // compositor. Same rule `RenderOpacity` follows.
        assert_eq!(LayerEffect::new(f32::NAN).alpha, 0.0);
    }

    /// Every fill's clip bounds, in order.
    fn fill_clips(scene: &Scene) -> Vec<Option<Rect>> {
        scene
            .commands()
            .iter()
            .filter(|command| matches!(command, crate::Command::FillRect { .. }))
            .map(|command| command.clip().bounds())
            .collect()
    }

    #[test]
    fn a_clip_reaches_a_child_layer_the_recording_could_not() {
        // **The second instance of the bug `LayerEffect` exists for.** The paint
        // walk stops at a nested boundary, so an ancestor's `save`/`clip_rect`
        // is folded into a scene the child's commands never enter. A `Viewport`
        // both clips and is a boundary, so this is every scrollable.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        // The child paints well outside what the parent allows.
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        tree.set_effect(
            child,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0)),
        );

        assert_eq!(
            fill_clips(&tree.composite()),
            vec![Some(Rect::new(0.0, 0.0, 10.0, 10.0))],
            "the child's own commands carry the clip declared above it"
        );
    }

    #[test]
    fn a_clip_keeps_applying_to_layers_further_down() {
        // The clip is declared once, two levels above. A grandchild that owns a
        // layer is exactly as far outside the window as a child that does not,
        // so stopping the clip at one generation would be the same bug moved.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        let grandchild = tree.insert(Some(child), Transform::IDENTITY);
        paint(
            &mut tree,
            grandchild,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        tree.set_effect(
            child,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0)),
        );

        assert_eq!(
            fill_clips(&tree.composite()),
            vec![Some(Rect::new(0.0, 0.0, 10.0, 10.0))]
        );
    }

    #[test]
    fn a_clip_does_not_leak_onto_a_sibling_subtree() {
        // The direction a fix is most likely to break, and the one that is
        // invisible in the test above: a clip left in force would silently trim
        // everything composited after it.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let clipped = tree.insert(Some(root), Transform::IDENTITY);
        let sibling = tree.insert(Some(root), Transform::IDENTITY);
        paint(
            &mut tree,
            clipped,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        paint(
            &mut tree,
            sibling,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::RED,
        );
        tree.set_effect(
            clipped,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0)),
        );

        assert_eq!(
            fill_clips(&tree.composite()),
            vec![Some(Rect::new(0.0, 0.0, 10.0, 10.0)), None],
            "the sibling is composited unclipped"
        );
    }

    #[test]
    fn the_clip_is_read_in_the_enclosing_layers_space() {
        // The clip is declared where it was *found* — between the enclosing
        // boundary and this one — so it goes through the inherited transform and
        // not through this layer's own. Getting that wrong offsets the window by
        // the child's placement, which looks right until the child moves.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(100.0, 0.0)));
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        tree.set_effect(
            child,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0)),
        );

        assert_eq!(
            fill_clips(&tree.composite()),
            vec![Some(Rect::new(0.0, 0.0, 10.0, 10.0))],
            "not shifted by the child's own 100pt offset"
        );
    }

    #[test]
    fn nested_clips_intersect() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        let grandchild = tree.insert(Some(child), Transform::IDENTITY);
        paint(
            &mut tree,
            grandchild,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        tree.set_effect(
            child,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 30.0, 30.0)),
        );
        tree.set_effect(
            grandchild,
            LayerEffect::default().clipped_to(Rect::new(20.0, 0.0, 100.0, 10.0)),
        );

        assert_eq!(
            fill_clips(&tree.composite()),
            vec![Some(Rect::new(20.0, 0.0, 30.0, 10.0))],
            "both apply, so the window is the overlap"
        );
    }

    #[test]
    fn a_clip_that_excludes_everything_records_nothing_at_all() {
        // Not "records commands that draw nothing": a `PushLayer` kept while its
        // `PopLayer` was dropped would unbalance the stack, so the whole
        // recording goes or none of it does.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 500.0, 500.0),
            Color::BLUE,
        );
        tree.set_effect(child, LayerEffect::new(0.5).clipped_to(Rect::ZERO));

        let scene = tree.composite();
        assert!(fill_clips(&scene).is_empty(), "nothing was recorded");
        assert_eq!(
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, crate::Command::PopLayer))
                .count(),
            scene
                .commands()
                .iter()
                .filter(|c| matches!(c, crate::Command::PushLayer { .. }))
                .count(),
            "and the markers are still balanced"
        );
    }

    #[test]
    fn changing_a_clip_damages_the_layer_though_the_recording_is_identical() {
        // The same trap the alpha falls into: the commands are untouched, so a
        // diff of the two recordings reports nothing while the window over them
        // moved. Clean damage over a stale screen is never safe.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 50.0, 50.0), Color::RED);
        tree.end_frame();
        assert!(tree.damage(SURFACE).regions().is_empty(), "settled");

        tree.set_effect(
            root,
            LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0)),
        );
        assert!(!tree.damage(SURFACE).regions().is_empty());
    }

    #[test]
    fn a_clip_alone_opens_no_compositing_target() {
        // A clip is a mask on commands, not a group: giving it `PushLayer` would
        // cost a texture in the backend and a layer for `Damage` to reason about
        // for something that needs neither.
        let effect = LayerEffect::default().clipped_to(Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(!effect.groups(), "no markers");
        assert!(!effect.is_a_no_op(), "but not nothing, either");
    }

    #[test]
    fn a_new_tree_has_no_root_until_something_is_inserted() {
        let tree = LayerTree::new();
        assert!(tree.root().is_none());
        assert!(tree.is_empty());
        assert!(tree.composite().is_empty());
    }

    #[test]
    fn the_first_insert_becomes_the_root() {
        let tree = LayerTree::with_root();
        assert_eq!(tree.len(), 1);
        assert!(tree.root().is_some());
    }

    #[test]
    fn a_new_layer_starts_pending_because_it_has_nothing_recorded() {
        let tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        assert_eq!(tree.needs_paint(), vec![root]);
    }

    #[test]
    fn painting_clears_the_previous_recording_rather_than_appending() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");

        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        paint(
            &mut tree,
            root,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );

        assert_eq!(tree.layer(root).scene().len(), 1, "content would double up");
        assert_eq!(tree.layer(root).scene().fills()[0].1.color, Color::BLUE);
    }

    #[test]
    fn painting_clears_the_pending_flag() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");

        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        assert!(tree.needs_paint().is_empty());
        assert_eq!(tree.layer(root).paint_count(), 1);
    }

    #[test]
    fn marking_a_child_pending_does_not_pending_its_parent() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            root,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::RED,
        );
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        tree.mark_needs_paint(child);

        assert_eq!(
            tree.needs_paint(),
            vec![child],
            "a repaint must stop at the boundary, not walk to the root"
        );
        assert_eq!(
            tree.layer(root).paint_count(),
            1,
            "the root must not repaint"
        );
    }

    #[test]
    fn compositing_puts_children_over_their_parents_content() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            root,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::RED,
        );
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );

        let fills = tree.composite().fills();
        assert_eq!(fills.len(), 2);
        assert_eq!(fills[0].1.color, Color::RED, "parent content is underneath");
        assert_eq!(fills[1].1.color, Color::BLUE);
    }

    #[test]
    fn a_layers_transform_is_applied_when_compositing() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(50.0, 20.0)));

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );

        assert_eq!(
            tree.composite().fills()[0].0,
            Rect::new(50.0, 20.0, 60.0, 30.0),
            "a layer records in its own space and is placed at composite time"
        );
    }

    #[test]
    fn nested_transforms_accumulate_down_the_layer_tree() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let middle = tree.insert(Some(root), Transform::translate(Offset::new(10.0, 10.0)));
        let leaf = tree.insert(Some(middle), Transform::translate(Offset::new(5.0, 5.0)));

        paint(&mut tree, leaf, Rect::new(0.0, 0.0, 1.0, 1.0), Color::RED);

        assert_eq!(
            tree.composite().fills()[0].0,
            Rect::new(15.0, 15.0, 16.0, 16.0)
        );
        assert_eq!(
            tree.screen_transform(leaf),
            Transform::translate(Offset::new(15.0, 15.0))
        );
    }

    #[test]
    fn a_repainted_layer_composites_its_new_content_without_the_parent_repainting() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            root,
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Color::RED,
        );
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::GREEN,
        );

        let fills = tree.composite().fills();
        assert_eq!(fills[0].1.color, Color::RED, "reused, not re-recorded");
        assert_eq!(fills[1].1.color, Color::GREEN);
        assert_eq!(tree.layer(root).paint_count(), 1);
    }

    #[test]
    fn moving_a_layer_damages_both_the_old_and_the_new_position() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        tree.set_transform(child, Transform::translate(Offset::new(500.0, 500.0)));

        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(0.0, 0.0, 10.0, 10.0)),
            "the vacated pixels still hold the old content: {damage}"
        );
        assert!(
            damage.intersects(Rect::new(500.0, 500.0, 510.0, 510.0)),
            "{damage}"
        );
    }

    #[test]
    fn setting_the_same_transform_marks_nothing_pending() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(5.0, 5.0)));

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();
        assert!(tree.is_clean());

        tree.set_transform(child, Transform::translate(Offset::new(5.0, 5.0)));
        assert!(tree.is_clean(), "a no-op sync must not schedule work");
    }

    #[test]
    fn removing_a_layer_damages_the_pixels_it_leaves_behind() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(300.0, 300.0)));

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        tree.remove(child);

        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(300.0, 300.0, 310.0, 310.0)),
            "a deleted subtree stays on screen until something repaints over it: {damage}"
        );
        assert!(tree.composite().is_empty());
    }

    #[test]
    fn removing_a_layer_removes_its_descendants() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let middle = tree.insert(Some(root), Transform::IDENTITY);
        let leaf = tree.insert(Some(middle), Transform::IDENTITY);

        tree.remove(middle);

        assert!(!tree.is_alive(middle));
        assert!(!tree.is_alive(leaf));
        assert_eq!(tree.len(), 1);
        assert!(tree.layer(root).children().is_empty());
    }

    #[test]
    fn a_reused_slot_does_not_answer_to_the_old_id() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let first = tree.insert(Some(root), Transform::IDENTITY);
        tree.remove(first);
        let second = tree.insert(Some(root), Transform::IDENTITY);

        assert_eq!(first.index(), second.index(), "the slot is reused");
        assert_ne!(first, second);
        assert!(!tree.is_alive(first));
        assert!(tree.is_alive(second));
    }

    #[test]
    fn mark_needs_paint_on_a_dead_layer_is_ignored() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        tree.remove(child);

        tree.mark_needs_paint(child);
        assert!(
            !tree.needs_paint().contains(&child),
            "a stale handle must not resurrect a layer"
        );
    }

    #[test]
    fn end_frame_settles_the_tree() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);

        tree.end_frame();
        assert!(tree.is_clean());
        assert!(
            tree.damage(SURFACE).is_clean(),
            "a settled tree needs no repaint"
        );
    }

    #[test]
    fn damage_covers_a_repainted_layers_old_and_new_content() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");

        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        tree.end_frame();

        // Shrinks: the uncovered part of the old bounds must still be repainted.
        paint(&mut tree, root, Rect::new(0.0, 0.0, 4.0, 4.0), Color::RED);

        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(8.0, 8.0, 10.0, 10.0)),
            "{damage}"
        );
    }

    #[test]
    fn a_never_painted_layer_contributes_no_spurious_damage() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");

        // The root has no previous bounds, so it reports `Rect::ZERO` for where it
        // used to be. That must contribute nothing — not a 1x1 region at the
        // origin, which is what inflating an empty rect by the AA bleed produces.
        paint(
            &mut tree,
            root,
            Rect::new(90.0, 90.0, 110.0, 110.0),
            Color::RED,
        );

        let damage = tree.damage(SURFACE);
        assert_eq!(damage.regions().len(), 1, "{damage}");
        assert!(
            !damage.intersects(Rect::new(0.0, 0.0, 2.0, 2.0)),
            "nothing was ever painted at the origin: {damage}"
        );
    }

    #[test]
    fn a_tree_of_empty_layers_is_clean() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(40.0, 40.0)));

        // Painted, but nothing drawn — both layers' bounds are empty.
        tree.begin_paint(root);
        tree.begin_paint(child);

        let damage = tree.damage(SURFACE);
        assert!(
            damage.is_clean(),
            "empty layers must not manufacture work: {damage}"
        );
    }

    #[test]
    fn damage_is_the_difference_between_two_recordings_not_the_whole_layer() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");

        // One layer covering the surface, holding two far-apart rectangles.
        {
            let scene = tree.begin_paint(root);
            scene.fill_rect(Rect::new(0.0, 0.0, 900.0, 900.0), Color::RED.into());
            scene.fill_rect(Rect::new(10.0, 10.0, 30.0, 30.0), Color::BLUE.into());
        }
        tree.end_frame();

        // Repaint with only the small rectangle changed.
        {
            let scene = tree.begin_paint(root);
            scene.fill_rect(Rect::new(0.0, 0.0, 900.0, 900.0), Color::RED.into());
            scene.fill_rect(Rect::new(10.0, 10.0, 30.0, 30.0), Color::GREEN.into());
        }

        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(10.0, 10.0, 30.0, 30.0)),
            "{damage}"
        );
        assert!(
            damage.covered_area() < SURFACE.area() * 0.05,
            "the root layer covers the surface, so taking its bounds would report \
             everything — a blinking caret would cost the screen: {damage}"
        );
    }

    #[test]
    fn a_layer_that_moved_damages_both_places_even_though_it_re_recorded_nothing() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        tree.set_transform(child, Transform::translate(Offset::new(500.0, 500.0)));

        // The recording is untouched, so the command diff finds nothing at all.
        // Only comparing the transform against last frame's catches this.
        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(0.0, 0.0, 10.0, 10.0)),
            "{damage}"
        );
        assert!(
            damage.intersects(Rect::new(500.0, 500.0, 510.0, 510.0)),
            "{damage}"
        );
        assert_eq!(tree.layer(child).paint_count(), 1, "and it did not repaint");
    }

    #[test]
    fn a_parent_moving_damages_its_children_old_and_new_positions() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let middle = tree.insert(Some(root), Transform::IDENTITY);
        let leaf = tree.insert(Some(middle), Transform::translate(Offset::new(5.0, 5.0)));

        paint(
            &mut tree,
            leaf,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.end_frame();

        tree.set_transform(middle, Transform::translate(Offset::new(300.0, 300.0)));

        // `middle` is what was marked; `leaf` is what has the pixels. Comparing
        // each layer's transform to the *screen* is what carries the change down.
        let damage = tree.damage(SURFACE);
        assert!(
            damage.intersects(Rect::new(5.0, 5.0, 15.0, 15.0)),
            "{damage}"
        );
        assert!(
            damage.intersects(Rect::new(305.0, 305.0, 315.0, 315.0)),
            "{damage}"
        );
    }

    #[test]
    fn reordering_children_changes_the_composite_and_damages_them() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let first = tree.insert(Some(root), Transform::IDENTITY);
        let second = tree.insert(Some(root), Transform::IDENTITY);

        paint(
            &mut tree,
            first,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Color::RED,
        );
        paint(
            &mut tree,
            second,
            Rect::new(0.0, 0.0, 40.0, 40.0),
            Color::BLUE,
        );
        tree.end_frame();
        assert_eq!(tree.composite().fills()[1].1.color, Color::BLUE, "on top");

        tree.set_child_order(root, vec![second, first]);

        assert_eq!(
            tree.composite().fills()[1].1.color,
            Color::RED,
            "the order is the composite order"
        );
        assert!(
            tree.damage(SURFACE)
                .intersects(Rect::new(0.0, 0.0, 40.0, 40.0)),
            "neither recording changed, so the diff finds nothing — a reorder has \
             to damage its bounds outright"
        );
    }

    #[test]
    fn setting_the_same_child_order_marks_nothing_pending() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let first = tree.insert(Some(root), Transform::IDENTITY);
        let second = tree.insert(Some(root), Transform::IDENTITY);

        paint(&mut tree, root, Rect::new(0.0, 0.0, 1.0, 1.0), Color::RED);
        paint(&mut tree, first, Rect::new(0.0, 0.0, 1.0, 1.0), Color::RED);
        paint(&mut tree, second, Rect::new(0.0, 0.0, 1.0, 1.0), Color::RED);
        tree.end_frame();

        tree.set_child_order(root, vec![first, second]);
        assert!(tree.is_clean(), "a no-op sync must not schedule work");
    }

    #[test]
    #[should_panic(expected = "not a permutation")]
    fn a_child_order_that_drops_a_child_panics() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let first = tree.insert(Some(root), Transform::IDENTITY);
        let _second = tree.insert(Some(root), Transform::IDENTITY);

        // Silently dropping a child would make it vanish from the frame with
        // nothing anywhere to explain it.
        tree.set_child_order(root, vec![first]);
    }

    #[test]
    fn screen_bounds_of_an_empty_layer_is_zero_not_a_transformed_point() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::translate(Offset::new(90.0, 90.0)));

        assert_eq!(
            tree.screen_bounds(child),
            Rect::ZERO,
            "an empty layer must not damage the point its origin maps to"
        );
    }
}
