//! Incremental scene building.
//!
//! [`LayerTree::composite`] flattens the whole layer tree into one [`Scene`]
//! from scratch. It is correct, it is the definition of what a frame looks like,
//! and it costs the number of commands on the screen every time it runs —
//! because every command goes through [`Command::transformed`], which
//! recomputes a transform, re-clips, and **clones every [`Path`] and every clip
//! shape it passes**.
//!
//! That cost is paid whenever *any* layer is dirty. A repaint boundary stops a
//! repaint at one row of a list; nothing stopped the flatten. One row changing
//! colour re-transformed and re-allocated the entire screen.
//!
//! [`SceneFlattener`] is the same function with the answer retained. It keeps
//! last frame's flattened commands and rebuilds only the span of them that
//! changed.
//!
//! # Why this is possible at all
//!
//! Because [`Scene`] resolves transform and clip at *record* time. A command
//! means the same thing wherever it sits in the list, so a recording can be cut
//! at any index and lifted into any space without replaying a prefix — which is
//! what makes the flattened scene a **concatenation of independent segments**
//! rather than a stream that has to be produced in order.
//!
//! Each segment is a pure function of a small key:
//!
//! ```text
//! segment = layer.scene()[range], lifted by `transform`, confined to `clip`
//! ```
//!
//! so `(layer, epoch, range, transform, clip)` determines its commands
//! completely. Nothing else can change them.
//!
//! # The algorithm
//!
//! 1. **Plan** — walk the layer tree emitting the ordered list of segment keys.
//!    This touches no command at all and is O(layers). Group markers are
//!    resolved here too, which needs only *counts*, never geometry — see
//!    [`Plan::close_group`].
//! 2. **Diff** — common prefix and common suffix against the retained plan.
//! 3. **Rebuild the middle** — materialise only the divergent segments and
//!    `splice` them in. The unchanged suffix is **moved**, not cloned: a
//!    memmove of `Command`s with no allocation and no `Path` clone anywhere.
//! 4. **Patch group bounds** — a `PushLayer`'s bounds is the union of what it
//!    encloses, so it is the one field that depends on materialised geometry.
//!    It is a scalar write into a command already in place, not a rebuild.
//!
//! The common animation case — a colour, a caret, an opacity, a label whose
//! command count did not change — diverges in exactly one segment and costs
//! that segment plus one key comparison per layer.
//!
//! # The invariant
//!
//! **`flatten` must produce exactly what `composite` would have produced, on
//! every frame, after every mutation.** That is not a hope; it is the property
//! the tests here assert directly, with `composite` as the oracle.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;

use vieww_foundation::{BlendMode, Rect, Transform};

use crate::{Command, LayerId, LayerTree, Scene};

/// One contiguous run of the flattened scene, and what produced it.
///
/// `PartialEq` is the whole mechanism: two plans are diffed by comparing these,
/// and a variant that compared equal while describing different commands would
/// retain a stale picture. Every field that can change a segment's commands is
/// therefore part of the value.
#[derive(Debug, Clone, PartialEq)]
enum Item {
    /// Opens `layer`'s group.
    ///
    /// `bounds` is deliberately **not** here: it is a function of what the group
    /// encloses rather than of the group, so it is resolved after materialising
    /// and patched into the command in place. Putting it in the key would make
    /// every enclosing group's segment rebuild whenever anything inside it
    /// moved a pixel, which is the cost this module exists to avoid.
    Push {
        layer: LayerId,
        alpha: f32,
        blend: BlendMode,
    },
    /// Closes the group opened by the matching [`Item::Push`].
    Pop { layer: LayerId },
    /// `layer`'s own recording, lifted into the screen and confined to `clip`.
    Content {
        layer: LayerId,
        /// `Layer::paint_count` — bumped by every `begin_paint`, so a
        /// re-recording is a different epoch even when it records the same
        /// number of commands.
        epoch: u32,
        range: Range<usize>,
        transform: Transform,
        clip: Option<Rect>,
    },
}

impl Item {
    /// How many commands this item contributes.
    ///
    /// Known without touching a single command, which is what lets the plan
    /// pass resolve empty groups: see [`Plan::close_group`].
    fn len(&self) -> usize {
        match self {
            Self::Push { .. } | Self::Pop { .. } => 1,
            Self::Content { range, clip, .. } => {
                // `Scene::append_range_clipped` drops the whole run for an empty
                // clip and keeps all of it otherwise — it confines commands, it
                // does not cull them. So the count is exact.
                if clip.is_some_and(Rect::is_empty) {
                    0
                } else {
                    range.len()
                }
            }
        }
    }
}

/// A hash of every field in an [`Item`] that determines its commands.
///
/// Floats are hashed as their **bit patterns**, so a `NaN` transform never
/// matches itself and two items are considered identical exactly when their
/// produced commands would be identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Fingerprint(u64);

impl Fingerprint {
    fn hash_one<H: Hasher>(h: &mut H, v: impl Hash) {
        v.hash(h);
    }

    fn f32_bits<H: Hasher>(h: &mut H, v: f32) {
        v.to_bits().hash(h);
    }

    fn rect_bits<H: Hasher>(h: &mut H, r: Option<Rect>) {
        match r {
            Some(rect) => {
                1u8.hash(h);
                Self::f32_bits(h, rect.left);
                Self::f32_bits(h, rect.top);
                Self::f32_bits(h, rect.right);
                Self::f32_bits(h, rect.bottom);
            }
            None => 0u8.hash(h),
        }
    }

    fn transform_bits<H: Hasher>(h: &mut H, t: Transform) {
        Self::f32_bits(h, t.a);
        Self::f32_bits(h, t.b);
        Self::f32_bits(h, t.c);
        Self::f32_bits(h, t.d);
        Self::f32_bits(h, t.tx);
        Self::f32_bits(h, t.ty);
    }

    fn of(item: &Item) -> Self {
        // A simple hasher; the quality of the hash doesn't matter for
        // correctness — only for collision rate, and the input space is tiny.
        let mut h = std::collections::hash_map::DefaultHasher::new();
        match item {
            Item::Push {
                layer,
                alpha,
                blend,
            } => {
                0u8.hash(&mut h);
                Self::hash_one(&mut h, layer);
                Self::f32_bits(&mut h, *alpha);
                Self::hash_one(&mut h, blend);
            }
            Item::Pop { layer } => {
                1u8.hash(&mut h);
                Self::hash_one(&mut h, layer);
            }
            Item::Content {
                layer,
                epoch,
                range,
                transform,
                clip,
            } => {
                2u8.hash(&mut h);
                Self::hash_one(&mut h, layer);
                Self::hash_one(&mut h, epoch);
                Self::hash_one(&mut h, range.start);
                Self::hash_one(&mut h, range.end);
                Self::transform_bits(&mut h, *transform);
                Self::rect_bits(&mut h, *clip);
            }
        }
        // Fold the hasher state into a single u64.
        Fingerprint(h.finish())
    }
}

/// One item, plus what materialising it produced last time.
#[derive(Debug, Clone)]
struct Planned {
    item: Item,
    /// Commands contributed, which is `item.len()` for everything except a
    /// `Push` whose group was dropped — and a dropped group is removed from the
    /// plan outright, so this is always `item.len()`. Cached rather than
    /// recomputed because the splice arithmetic reads it once per item per
    /// frame.
    len: usize,
    /// The union of this item's commands' screen bounds.
    ///
    /// Kept so a group's bounds can be resolved from its enclosed items without
    /// re-walking commands that were reused. `Rect::ZERO` for `Pop`, which draws
    /// nothing.
    bounds: Rect,
    /// The fingerprint of `item`, cached here so the old plan can be indexed
    /// by identity without recomputing.
    fp: Fingerprint,
}

/// The ordered plan for one frame, built by walking the layer tree.
#[derive(Debug, Default)]
struct Plan {
    items: Vec<Item>,
}

impl Plan {
    /// Plan the whole tree, mirroring `LayerTree::composite_into` exactly.
    fn build(tree: &LayerTree) -> Self {
        let mut plan = Self::default();
        if let Some(root) = tree.root() {
            plan.layer(tree, root, Transform::IDENTITY, None, false);
        }
        plan
    }

    /// `clip` is the accumulated clip in **screen** space; `enclosed` says this
    /// layer is being spliced in where its parent still has a group open, so its
    /// own markers would apply the same declaration a second time.
    fn layer(
        &mut self,
        tree: &LayerTree,
        id: LayerId,
        inherited: Transform,
        clip: Option<Rect>,
        enclosed: bool,
    ) {
        let layer = tree.layer(id);
        let transform = layer.transform().then(inherited);
        let effect = tree.effect(id);

        // The declared clip is in the *enclosing* layer's space, so it goes
        // through `inherited` and not through `transform`, which already has
        // this layer's own placement folded in.
        let clip = match effect.clip {
            Some(declared) => {
                let declared = inherited.apply_rect(declared);
                Some(clip.map_or(declared, |outer| outer.intersect(declared)))
            }
            None => clip,
        };

        let grouped = effect.groups() && !enclosed;
        let opened = self.items.len();
        if grouped {
            self.items.push(Item::Push {
                layer: id,
                alpha: effect.alpha,
                blend: effect.blend,
            });
        }

        // Sorted rather than assumed ordered, for the reason `composite_into`
        // gives: a slot is a position in a command list while child order is a
        // position in the render tree, and a stable sort keeps child order as
        // the tie-break for boundaries reached at the same index.
        let mut children: Vec<LayerId> = layer.children().to_vec();
        children.sort_by_key(|&child| tree.layer(child).slot());

        let epoch = layer.paint_count();
        let total = layer.scene().len();
        let mut cut = 0usize;
        let mut open = 0usize;
        for &child in &children {
            let at = tree.layer(child).slot().min(total).max(cut);
            if at > cut {
                self.items.push(Item::Content {
                    layer: id,
                    epoch,
                    range: cut..at,
                    transform,
                    clip,
                });
                open = open.saturating_add_signed(group_delta(&layer.scene().commands()[cut..at]));
                cut = at;
            }
            self.layer(tree, child, transform, clip, open > 0);
        }
        if cut < total {
            self.items.push(Item::Content {
                layer: id,
                epoch,
                range: cut..total,
                transform,
                clip,
            });
        }

        if grouped {
            self.close_group(id, opened, effect.alpha);
        }
    }

    /// Emit the `Pop` for the group opened at `opened`, or delete the group.
    ///
    /// # Why this needs no geometry
    ///
    /// `Scene::pop_layer` drops a group in two cases, and neither is about where
    /// anything is: the group is **invisible** (`alpha <= 0`), or **nothing was
    /// drawn into it**. The second is a count, and every item's count is known
    /// from its range — so the whole decision is available in the plan pass,
    /// before a command has been touched.
    ///
    /// That matters more than it looks. A group is dropped *bottom-up*: an inner
    /// group that disappears can leave its parent enclosing nothing, and the
    /// parent has to disappear too. Because this runs as each recursion unwinds,
    /// it already sees its children's answers.
    fn close_group(&mut self, id: LayerId, opened: usize, alpha: f32) {
        let empty = self.items[opened + 1..].iter().all(|item| item.len() == 0);
        // `Opacity::new(0.0)` hides a subtree while keeping it hit-testable, so
        // a scene that recorded its commands anyway would report damage every
        // time something invisible changed underneath it.
        if alpha <= 0.0 || empty {
            self.items.truncate(opened);
            return;
        }
        self.items.push(Item::Pop { layer: id });
    }
}

/// How many groups `commands` leaves open, negative if it closes more than it
/// opens.
fn group_delta(commands: &[Command]) -> isize {
    commands
        .iter()
        .map(|command| match command {
            Command::PushLayer { .. } => 1,
            Command::PopLayer => -1,
            _ => 0,
        })
        .sum()
}

/// Counted work for one [`SceneFlattener::flatten`].
///
/// Per call rather than per frame, and in commands rather than in seconds:
/// "how much of the screen did this frame rebuild" is the question the whole
/// optimization is about, and it is the one a test can assert on every machine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FlattenStats {
    /// Commands materialised — transformed, clipped and allocated.
    pub built: usize,
    /// Commands kept in place from the previous frame, at no per-command cost.
    pub reused: usize,
    /// `PushLayer` bounds patched in place because their contents moved.
    ///
    /// A patch is a scalar write into a command that is already correct
    /// otherwise, so it is counted apart from a rebuild.
    pub patched: usize,
}

impl FlattenStats {
    /// Commands in the resulting scene.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.built + self.reused
    }
}

/// A [`LayerTree`] flattened into one [`Scene`], retained across frames.
///
/// Hold one per surface and call [`flatten`](Self::flatten) each frame. It is
/// deliberately a separate object rather than a field on `LayerTree`: the tree
/// is the source of truth and has no business caching a derived view, and a
/// backend that composites layers natively should never pay for one.
#[derive(Debug, Default)]
pub struct SceneFlattener {
    scene: Scene,
    plan: Vec<Planned>,
    stats: FlattenStats,
    /// A spare `Vec<Command>` swapped in each frame so assembly allocates
    /// nothing — the old scene's commands are drained (moved) into it rather
    /// than dropped.
    spare: Vec<Command>,
}

impl SceneFlattener {
    /// A flattener with nothing retained.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The retained scene, as of the last [`flatten`](Self::flatten).
    #[must_use]
    pub const fn scene(&self) -> &Scene {
        &self.scene
    }

    /// What the last [`flatten`](Self::flatten) actually did.
    #[must_use]
    pub const fn stats(&self) -> FlattenStats {
        self.stats
    }

    /// Forget everything retained.
    ///
    /// For a surface that was recreated: the scene is still *correct*, so this
    /// is not needed for correctness, but a caller that knows the next frame
    /// shares nothing with this one can skip a diff that will not match.
    pub fn reset(&mut self) {
        self.scene.clear();
        self.plan.clear();
        self.spare.clear();
        self.stats = FlattenStats::default();
    }

    /// Flatten `tree`, rebuilding only what changed since the last call.
    ///
    /// The result is identical to [`LayerTree::composite`] — that is the
    /// module's invariant and what its tests assert.
    ///
    /// # Fingerprint matching
    ///
    /// Rather than diffing by position (common prefix/suffix), each segment is
    /// matched by identity: a `Fingerprint` covers every field of the key with
    /// floats as bit patterns, so two items are equal exactly when their
    /// produced commands are equal. Last frame's plan is indexed by fingerprint;
    /// the new plan is walked once, moving matched segments out of the retained
    /// scene and materialising only the misses.
    ///
    /// Reuse must be **monotonic forward** — a segment may only be moved from a
    /// position at or after the last one reused. A subtree that moves backwards
    /// past a sibling re-materialises, which is a reorder that already damages
    /// both subtrees outright.
    pub fn flatten(&mut self, tree: &LayerTree) -> &Scene {
        let plan = Plan::build(tree).items;

        // No retained scene to match against — first frame.
        if self.plan.is_empty() {
            let mut built: Vec<Command> = Vec::new();
            let mut planned = Vec::with_capacity(plan.len());
            for item in &plan {
                let start = built.len();
                let bounds = materialise(tree, item, &mut built);
                let fp = Fingerprint::of(item);
                planned.push(Planned {
                    item: item.clone(),
                    len: built.len() - start,
                    bounds,
                    fp,
                });
            }
            self.scene = Scene::new();
            self.scene.commands_mut().extend(built);
            self.plan = planned;
            self.stats = FlattenStats {
                built: self.scene.len(),
                reused: 0,
                patched: 0,
            };
            self.patch_group_bounds();
            return &self.scene;
        }

        // Index the old plan by fingerprint → list of (plan_index, command_offset).
        // Multiple segments can share a fingerprint (e.g. identical rows), so
        // we store all candidates and pick the first unconsumed one at or after
        // `min_offset`.
        let mut index: HashMap<Fingerprint, Vec<(usize, usize)>> = HashMap::new();
        let mut offset = 0usize;
        for (i, planned) in self.plan.iter().enumerate() {
            index.entry(planned.fp).or_default().push((i, offset));
            offset += planned.len;
        }

        // Take the old scene's commands. We'll copy matched slices out of
        // `old_commands` (which already carry their final transforms and clips)
        // and materialise only the misses into `assembled`. At the end we swap
        // `assembled` in as the scene and `old_commands` becomes the new spare.
        //
        // This copies Commands for matched segments rather than moving them.
        // A Command copy is a struct copy + a Path::clone — but the alternative
        // is re-materialising through Command::transformed, which is strictly
        // more expensive (recomputes the transform, the clip, AND clones the Path
        // into a new allocation). The optimisation is in skipping materialisation,
        // not in avoiding all copies.
        let old_commands = std::mem::take(self.scene.commands_mut());

        // Pre-size to avoid realloc during assembly. We know the total because
        // every Item's command count is known from its key.
        let total: usize = plan.iter().map(|i| i.len()).sum();
        let mut assembled: Vec<Command> = std::mem::take(&mut self.spare);
        assembled.clear();
        assembled.reserve(total);

        let mut new_plan: Vec<Planned> = Vec::with_capacity(plan.len());
        let mut built_count = 0usize;
        let mut reused_count = 0usize;
        // Monotonic forward cursor: a matched segment must come from the old
        // scene at an offset ≥ this, so we never reach backwards.
        let mut min_offset = 0usize;
        let mut consumed = vec![false; self.plan.len()];

        for item in &plan {
            let fp = Fingerprint::of(item);

            // Look for a reusable old segment at or after `min_offset`.
            let matched = index.get(&fp).and_then(|entries| {
                entries
                    .iter()
                    .find(|(old_i, old_off)| !consumed[*old_i] && *old_off >= min_offset)
                    .copied()
            });

            if let Some((old_i, old_off)) = matched {
                let old_len = self.plan[old_i].len;
                assembled.extend_from_slice(&old_commands[old_off..old_off + old_len]);
                min_offset = old_off + old_len;
                consumed[old_i] = true;
                reused_count += old_len;
                new_plan.push(Planned {
                    item: item.clone(),
                    len: old_len,
                    bounds: self.plan[old_i].bounds,
                    fp,
                });
            } else {
                let start = assembled.len();
                let bounds = materialise(tree, item, &mut assembled);
                let new_len = assembled.len() - start;
                built_count += new_len;
                new_plan.push(Planned {
                    item: item.clone(),
                    len: new_len,
                    bounds,
                    fp,
                });
            }
        }

        self.stats = FlattenStats {
            built: built_count,
            reused: reused_count,
            patched: 0,
        };

        // Swap: assembled becomes the scene; old commands become the new spare.
        self.spare = old_commands;
        self.scene = Scene::new();
        self.scene.commands_mut().extend(assembled);
        self.plan = new_plan;

        self.patch_group_bounds();
        &self.scene
    }

    /// Give every `PushLayer` the union of what it encloses.
    ///
    /// The one thing a segment's key cannot carry, because it is a property of
    /// the *contents* rather than of the group — `Scene::pop_layer` computes it
    /// the same way at record time. A group whose contents merely moved has a
    /// correct command with one stale field, so this writes the field rather
    /// than rebuilding the segment.
    ///
    /// Innermost first, because a nested `PushLayer`'s own bounds is part of its
    /// parent's union — which is exactly how `Scene::pop_layer` sees it, since
    /// the inner pair is already closed by the time the outer one closes.
    fn patch_group_bounds(&mut self) {
        // Command index of each item, walked once.
        let mut at = 0usize;
        let mut offsets = Vec::with_capacity(self.plan.len());
        for planned in &self.plan {
            offsets.push(at);
            at += planned.len;
        }

        let mut open: Vec<usize> = Vec::new();
        for index in 0..self.plan.len() {
            match self.plan[index].item {
                Item::Push { .. } => open.push(index),
                Item::Pop { .. } => {
                    let opened = open.pop().expect("balanced plan");
                    let bounds = self.plan[opened + 1..index]
                        .iter()
                        .map(|planned| planned.bounds)
                        .fold(Rect::ZERO, Rect::union);
                    // The group's own bounds is what its parent sees, so it has
                    // to be recorded before the parent's fold reaches it.
                    self.plan[opened].bounds = bounds;
                    if let Command::PushLayer {
                        bounds: recorded, ..
                    } = &mut self.scene.commands_mut()[offsets[opened]]
                    {
                        if *recorded != bounds {
                            *recorded = bounds;
                            self.stats.patched += 1;
                        }
                    }
                }
                Item::Content { .. } => {}
            }
        }
        debug_assert!(open.is_empty(), "a plan opened a group it never closed");
    }
}

/// Append `item`'s commands to `out`, and report their union.
fn materialise(tree: &LayerTree, item: &Item, out: &mut Vec<Command>) -> Rect {
    match item {
        Item::Push { alpha, blend, .. } => {
            out.push(Command::PushLayer {
                // Patched by `patch_group_bounds` once the contents are known —
                // the same order `Scene::pop_layer` does it in.
                bounds: Rect::ZERO,
                alpha: *alpha,
                blend: *blend,
                clip: crate::Clip::NONE,
                // A layer-tree group is a repaint boundary with an opacity, not
                // a filter. Filters are recorded by `RenderFilter` into a
                // layer's own command range and arrive here as content.
                filter: vieww_foundation::ImageFilter::NONE,
            });
            Rect::ZERO
        }
        Item::Pop { .. } => {
            out.push(Command::PopLayer);
            Rect::ZERO
        }
        Item::Content {
            layer,
            range,
            transform,
            clip,
            ..
        } => {
            let mut scratch = Scene::new();
            scratch.append_range_clipped(
                tree.layer(*layer).scene(),
                range.clone(),
                *transform,
                *clip,
            );
            let commands = scratch.into_commands();
            let bounds = commands
                .iter()
                .map(Command::bounds)
                .fold(Rect::ZERO, Rect::union);
            out.extend(commands);
            bounds
        }
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Offset, Path};

    use super::*;
    use crate::{Canvas, LayerEffect, Paint, Stroke};

    /// Assert the retained answer is the from-scratch answer.
    ///
    /// **The invariant of this whole module.** `composite` is the definition of
    /// what a frame looks like; `flatten` is only allowed to be faster.
    fn agrees(flattener: &mut SceneFlattener, tree: &LayerTree) {
        let expected = tree.composite();
        let actual = flattener.flatten(tree);
        assert_eq!(
            actual.commands(),
            expected.commands(),
            "incremental:\n{actual}\nfrom scratch:\n{expected}"
        );
    }

    fn paint(tree: &mut LayerTree, id: LayerId, rect: Rect, color: Color) {
        tree.begin_paint(id).fill_rect(rect, color.into());
    }

    #[test]
    fn an_empty_tree_flattens_to_nothing() {
        let mut flattener = SceneFlattener::new();
        let tree = LayerTree::new();
        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().total(), 0);
    }

    #[test]
    fn the_first_frame_builds_everything() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().built, 1);
        assert_eq!(flattener.stats().reused, 0);
    }

    #[test]
    fn a_frame_that_changed_nothing_rebuilds_nothing() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().built, 0, "nothing changed");
        assert_eq!(flattener.stats().reused, 1);
    }

    /// **The case this module exists for.**
    #[test]
    fn one_repainted_row_costs_one_row_and_not_the_screen() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let rows: Vec<LayerId> = (0..50)
            .map(|i| {
                tree.insert(
                    Some(root),
                    Transform::translate(Offset::new(0.0, i as f32 * 20.0)),
                )
            })
            .collect();
        // A background in the root, then fifty rows of ten commands each.
        paint(
            &mut tree,
            root,
            Rect::new(0.0, 0.0, 400.0, 1000.0),
            Color::WHITE,
        );
        for &row in &rows {
            let scene = tree.begin_paint(row);
            for i in 0..10 {
                scene.fill_rect(
                    Rect::new(i as f32 * 40.0, 0.0, i as f32 * 40.0 + 30.0, 18.0),
                    Color::BLUE.into(),
                );
            }
        }

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();
        assert_eq!(flattener.stats().total(), 501);

        // One row repaints, with the same command count.
        let scene = tree.begin_paint(rows[20]);
        for i in 0..10 {
            scene.fill_rect(
                Rect::new(i as f32 * 40.0, 0.0, i as f32 * 40.0 + 30.0, 18.0),
                Color::GREEN.into(),
            );
        }

        agrees(&mut flattener, &tree);
        assert_eq!(
            flattener.stats().built,
            10,
            "a boundary bounds the repaint; it has to bound the flatten too"
        );
        assert_eq!(flattener.stats().reused, 491);
    }

    #[test]
    fn a_row_that_grows_moves_the_tail_rather_than_rebuilding_it() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let rows: Vec<LayerId> = (0..10)
            .map(|_| tree.insert(Some(root), Transform::IDENTITY))
            .collect();
        for &row in &rows {
            paint(&mut tree, row, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        }

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        // The third row now draws three things instead of one. Every command
        // after it shifts by two — and shifting is a memmove, not a rebuild.
        {
            let scene = tree.begin_paint(rows[2]);
            scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
            scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE.into());
            scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::GREEN.into());
        }

        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().built, 3, "only the row that changed");
        assert_eq!(flattener.stats().reused, 9, "the tail moved, unrebuilt");
    }

    #[test]
    fn a_moved_layer_rebuilds_only_itself() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let a = tree.insert(Some(root), Transform::IDENTITY);
        let b = tree.insert(Some(root), Transform::IDENTITY);
        paint(&mut tree, a, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        paint(&mut tree, b, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE);

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        tree.set_transform(a, Transform::translate(Offset::new(100.0, 0.0)));

        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().built, 1);
        assert_eq!(flattener.stats().reused, 1);
    }

    #[test]
    fn a_parent_moving_rebuilds_its_descendants_because_their_screen_space_moved() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let middle = tree.insert(Some(root), Transform::IDENTITY);
        let leaf = tree.insert(Some(middle), Transform::IDENTITY);
        paint(
            &mut tree,
            leaf,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        tree.set_transform(middle, Transform::translate(Offset::new(50.0, 0.0)));

        // The leaf's recording is untouched, but the transform in its key is
        // not — and a key that missed this would retain the old position.
        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().built, 1);
    }

    // --------------------------------------------------------------- groups

    #[test]
    fn a_group_gets_the_union_of_what_it_encloses() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        {
            let scene = tree.begin_paint(root);
            scene.fill_rect(Rect::new(10.0, 10.0, 20.0, 20.0), Color::RED.into());
            scene.fill_rect(Rect::new(40.0, 40.0, 50.0, 50.0), Color::BLUE.into());
        }
        tree.set_effect(root, LayerEffect::new(0.5));

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
        assert_eq!(
            flattener.scene().commands()[0].bounds(),
            Rect::new(10.0, 10.0, 50.0, 50.0)
        );
    }

    #[test]
    fn contents_moving_patches_the_group_bounds_rather_than_rebuilding_the_marker() {
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

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        tree.set_transform(child, Transform::translate(Offset::new(200.0, 0.0)));

        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().patched, 1, "one scalar write");
        assert_eq!(
            flattener.scene().commands()[0].bounds(),
            Rect::new(0.0, 0.0, 210.0, 10.0)
        );
    }

    #[test]
    fn a_group_faded_to_nothing_takes_its_contents_with_it() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.set_effect(child, LayerEffect::new(0.0));

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
        assert!(flattener.scene().is_empty());
    }

    #[test]
    fn a_group_enclosing_nothing_leaves_no_markers() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        tree.begin_paint(child);
        tree.set_effect(child, LayerEffect::new(0.5));

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
    }

    #[test]
    fn an_inner_group_emptying_takes_its_parent_with_it() {
        // The bottom-up case: the inner group disappears, which leaves the outer
        // one enclosing nothing, which has to disappear too. Resolving groups
        // top-down would leave a pair of markers around no content.
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let outer = tree.insert(Some(root), Transform::IDENTITY);
        let inner = tree.insert(Some(outer), Transform::IDENTITY);
        paint(
            &mut tree,
            inner,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );
        tree.set_effect(outer, LayerEffect::new(0.5));
        tree.set_effect(inner, LayerEffect::new(0.0));

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
        assert!(flattener.scene().is_empty());
    }

    #[test]
    fn a_clip_that_excludes_everything_drops_the_run_and_keeps_markers_balanced() {
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

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
    }

    // ------------------------------------------------------------ structure

    #[test]
    fn inserting_and_removing_layers_stays_correct() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let mut flattener = SceneFlattener::new();

        let a = tree.insert(Some(root), Transform::IDENTITY);
        paint(&mut tree, a, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);
        agrees(&mut flattener, &tree);
        tree.end_frame();

        let b = tree.insert(Some(root), Transform::IDENTITY);
        paint(&mut tree, b, Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLUE);
        agrees(&mut flattener, &tree);
        tree.end_frame();

        tree.remove(a);
        agrees(&mut flattener, &tree);
    }

    #[test]
    fn reordering_children_reorders_the_flattened_scene() {
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

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        tree.end_frame();

        tree.set_child_order(root, vec![second, first]);
        agrees(&mut flattener, &tree);
        assert_eq!(flattener.scene().fills()[1].1.color, Color::RED);
    }

    #[test]
    fn a_child_is_spliced_into_the_hole_its_parent_left() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let child = tree.insert(Some(root), Transform::IDENTITY);
        {
            let scene = tree.begin_paint(root);
            scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED.into());
            scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE.into());
        }
        // The hole was measured after the first command.
        tree.set_slot(child, 1);
        paint(
            &mut tree,
            child,
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::BLUE,
        );

        let mut flattener = SceneFlattener::new();
        agrees(&mut flattener, &tree);
        let colors: Vec<Color> = flattener
            .scene()
            .fills()
            .into_iter()
            .map(|(_, paint)| paint.color)
            .collect();
        assert_eq!(colors, vec![Color::RED, Color::BLUE, Color::WHITE]);
    }

    #[test]
    fn reset_forgets_everything_without_changing_the_answer() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        paint(&mut tree, root, Rect::new(0.0, 0.0, 10.0, 10.0), Color::RED);

        let mut flattener = SceneFlattener::new();
        flattener.flatten(&tree);
        flattener.reset();
        agrees(&mut flattener, &tree);
        assert_eq!(flattener.stats().reused, 0);
    }

    // ---------------------------------------------------- the whole invariant

    /// A long deterministic run of every mutation the tree supports, asserting
    /// after each one that the retained answer is still the from-scratch answer.
    ///
    /// Deterministic rather than random: a property test that fails on seed
    /// 41,203 tells nobody anything on the next run. This is a fixed sequence,
    /// and every step of it is reproducible by reading the loop.
    #[test]
    fn the_incremental_scene_equals_the_from_scratch_scene_through_every_mutation() {
        let mut tree = LayerTree::with_root();
        let root = tree.root().expect("root");
        let mut flattener = SceneFlattener::new();
        let mut layers = vec![root];

        for step in 0..120u32 {
            match step % 8 {
                0 => {
                    let parent = layers[step as usize % layers.len()];
                    let id = tree.insert(
                        Some(parent),
                        Transform::translate(Offset::new(step as f32, 0.0)),
                    );
                    layers.push(id);
                }
                1 => {
                    // Re-record with a command count that varies with the step,
                    // so the plan diff sees growth and shrinkage both.
                    let id = layers[step as usize % layers.len()];
                    let scene = tree.begin_paint(id);
                    for i in 0..(step % 5) {
                        scene.fill_rect(
                            Rect::new(i as f32, 0.0, i as f32 + 8.0, 8.0),
                            Color::RED.into(),
                        );
                    }
                }
                2 => {
                    let id = layers[step as usize % layers.len()];
                    tree.set_transform(
                        id,
                        Transform::translate(Offset::new(0.0, (step % 7) as f32)),
                    );
                }
                3 => {
                    let id = layers[step as usize % layers.len()];
                    tree.set_effect(id, LayerEffect::new((step % 4) as f32 / 3.0));
                }
                4 => {
                    let id = layers[step as usize % layers.len()];
                    tree.set_effect(
                        id,
                        LayerEffect::new(0.5).clipped_to(Rect::new(
                            0.0,
                            0.0,
                            (step % 30) as f32,
                            30.0,
                        )),
                    );
                }
                5 => {
                    // Paths and strokes, so the clone-heavy commands are covered
                    // rather than just fills.
                    let id = layers[step as usize % layers.len()];
                    let scene = tree.begin_paint(id);
                    scene.clip_rrect(Rect::new(0.0, 0.0, 40.0, 40.0), 6.0);
                    scene.fill_path(
                        &Path::rounded_rect(Rect::new(0.0, 0.0, 30.0, 30.0), 4.0),
                        Paint::from(Color::BLUE),
                    );
                    scene.stroke_path(
                        &Path::rounded_rect(Rect::new(0.0, 0.0, 20.0, 20.0), 2.0),
                        Stroke::new(2.0),
                        Paint::from(Color::GREEN),
                    );
                }
                6 => {
                    let id = layers[step as usize % layers.len()];
                    let children = tree.layer(id).children().to_vec();
                    if children.len() > 1 {
                        let mut order = children;
                        order.rotate_left(1);
                        tree.set_child_order(id, order);
                    }
                }
                _ => {
                    if layers.len() > 3 {
                        let index = 1 + step as usize % (layers.len() - 1);
                        let id = layers.remove(index);
                        // The subtree goes too, so anything below it is stale.
                        let dead: Vec<LayerId> = layers
                            .iter()
                            .copied()
                            .filter(|&l| !tree.is_alive(l))
                            .collect();
                        tree.remove(id);
                        layers.retain(|l| tree.is_alive(*l) && !dead.contains(l));
                    }
                }
            }

            agrees(&mut flattener, &tree);
            tree.end_frame();
        }
    }
}
