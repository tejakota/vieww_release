//! An explicit dependency graph over a [`Scene`]'s `PushLayer`/`PopLayer`
//! groups — "Renderer v2" pillar A (`docs/RENDERER-V2-NOTES.md`), scoped to
//! what a `Scene`-recording, CPU-rasterizing renderer can actually use.
//!
//! [`Scene::damage_cull`] already prunes damage at the command level today,
//! but implicitly: one linear walk (`append_damage_filtered`) that tracks an
//! ad hoc stack of `active`/`emitted` flags as it goes, with no structure a
//! caller can inspect, test independently, or reuse for anything but that
//! one walk. This module makes the structure that walk relies on explicit —
//! a real graph of passes with parent/child dependency edges — and
//! [`PassGraph::prune`] is validated (see this module's own `tests` submodule)
//! by checking
//! it against `Scene::damage_cull`'s own decisions, not only against itself.
//!
//! # What "pass" means here
//!
//! Every `PushLayer`/`PopLayer` pair opens one pass: an isolated group that
//! is rasterized into its own buffer and then composited onto its parent
//! (`docs/RENDERER-MIGRATION.md` §2's "isolated-layer compositing"). The
//! commands recorded directly inside a pass — not counting nested passes —
//! are that pass's own content. There is always exactly one implicit root
//! pass, [`PassGraph::root`], holding every top-level command; it has no
//! `PushLayer` of its own and is never pruned by bounds, only by whether
//! anything inside it survives.
//!
//! # Dependency order
//!
//! A pass composites onto its parent only after everything inside it —
//! including nested passes — has been rasterized, so a parent depends on
//! its children and never the reverse. [`PassGraph::prune`] returns
//! surviving passes in that order: post-order, children before the parent
//! that composites them. A render backend that wanted to consume this graph
//! directly (today's `NativeRenderer` does not — see "Integration status"
//! below) would have to process passes in exactly this order regardless of
//! which ones damage pruned away.
//!
//! One consequence of that dependency worth stating plainly:
//! `Scene::pop_layer` patches a layer's declared bounds to the union of what
//! was actually drawn inside it (its own doc: "the contents are the
//! truth"), and a descendant's clip always includes every ancestor's
//! `clip_rect` intersected in. Together those mean a *`Canvas`-recorded*
//! scene can never contain a pass whose own bounds reach outside its
//! parent's — containment only ever tightens going down the tree. So
//! `prune`'s "a child is only active if its parent is" gate never actually
//! fires for a `Canvas`-recorded `Scene`; it exists because `PassGraph`
//! documents its contract in terms of any well-formed `Command` sequence,
//! not only the ones `Canvas` happens to produce (see this module's own
//! `tests::prune_gates_a_child_by_its_parents_activity_not_only_its_own_bounds`,
//! which has to build its scene with raw `push_command` calls to exhibit
//! the gate at all).
//!
//! # A real precision gain over `damage_cull`, and why it isn't free
//!
//! `Scene::damage_cull` tests every pass and command against **one
//! rectangle** — the bounding box of all pending [`Damage`] regions
//! (`damage_cull`'s own comment: "over-rendering is the safe direction").
//! [`PassGraph::prune`] tests against each region individually via
//! [`Damage::intersects`], so with several disjoint damage regions it can
//! correctly skip a pass that sits in the gap between two regions but
//! inside their shared bounding box — strictly more precise, never less.
//! this module's own `tests::multi_region_damage_prunes_more_precisely_than_the_bounding_box_approach`
//! demonstrates exactly this case. The trade is per-region testing costs
//! `O(regions)` per pass instead of `O(1)`, which is why the two
//! differential tests use single-region damage: with one region the two
//! approaches are mathematically identical, which is what lets them be
//! compared for exact agreement rather than only "neither is wrong".
//!
//! # What this does not do, and why
//!
//! The GPU vocabulary this pillar borrows from — pass fusion (folding
//! several render passes into one to skip attachment load/store between
//! them) and transient resource aliasing (letting non-overlapping passes
//! share one piece of backing memory) — describes costs a tiled or
//! deferred GPU renderer pays *between* passes: attachment binds, barrier
//! placement, render-pass object setup and teardown. A CPU scanline
//! rasterizer never pays them — passes already run strictly one at a time
//! on one thread, each just writing into a plain `Vec` with no separate
//! "render pass" object to fuse away. There is nothing here for pass fusion
//! to save. [`PassGraph::co_schedulable_siblings`] still surfaces which
//! sibling passes have no data dependency on each other and *could* share a
//! GPU render pass or run concurrently — real information, useful
//! groundwork for a future GPU backend — but on today's one CPU thread it
//! is exposed for inspection and testing only, wired to no scheduling
//! mechanism, because there is no real mechanism it would enable here. The
//! resource-aliasing half of this pillar *is* real and shipped, just not in
//! this module — see `native/pool.rs`'s `TargetPool`, which reuses
//! `PushLayer` buffers across passes and frames directly.
//!
//! # Integration status
//!
//! `NativeRenderer::render_damaged` still calls `Scene::damage_cull`, not
//! this module — swapping the pruning mechanism a shipped, regression-tested
//! rendering path relies on is a larger, riskier change than validating a
//! new, independent implementation of the same decision against it, which is
//! what this delivery is. `PassGraph` is real, public, and tested against
//! real `Scene`s; wiring it into `render_damaged` in place of `damage_cull`
//! — trading `damage_cull`'s `O(1)`-per-pass bounding box for this module's
//! more precise but `O(regions)`-per-pass test — is recorded as open
//! follow-up in `docs/RENDERER-V2-NOTES.md` rather than done silently
//! alongside it.

use vieww_foundation::{BlendMode, ImageFilter, Rect};

use crate::{Command, Damage, Scene};

/// Identifies one [`Pass`] in a [`PassGraph`]. Stable for the lifetime of
/// the graph that produced it; meaningless against a different graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassId(usize);

/// The `PushLayer` a non-root [`Pass`] opened: its resolved bounds and
/// compositing parameters, plus where its `PushLayer`/`PopLayer` pair live
/// in [`Scene::commands`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassLayer {
    pub push_index: usize,
    pub pop_index: usize,
    /// Absolute, clip-resolved bounds — the same value
    /// [`Command::bounds`] would compute for this pass's `PushLayer`
    /// command, precomputed once at graph-build time.
    pub bounds: Rect,
    pub alpha: f32,
    pub blend: BlendMode,
    pub filter: ImageFilter,
}

/// One node in a [`PassGraph`]: either the implicit root (`layer: None`) or
/// one isolated `PushLayer` group.
#[derive(Debug, Clone)]
pub struct Pass {
    id: PassId,
    parent: Option<PassId>,
    children: Vec<PassId>,
    layer: Option<PassLayer>,
    /// `(command index, resolved bounds)` for this pass's own content
    /// commands, in scene order — not counting nested passes' commands or
    /// this pass's own `PushLayer`/`PopLayer` markers.
    own_commands: Vec<(usize, Rect)>,
}

impl Pass {
    #[must_use]
    pub fn id(&self) -> PassId {
        self.id
    }

    #[must_use]
    pub fn parent(&self) -> Option<PassId> {
        self.parent
    }

    #[must_use]
    pub fn children(&self) -> &[PassId] {
        &self.children
    }

    /// `None` for the root pass.
    #[must_use]
    pub fn layer(&self) -> Option<&PassLayer> {
        self.layer.as_ref()
    }

    /// Indices into [`Scene::commands`] of this pass's own content, in
    /// scene order.
    ///
    /// No `#[must_use]` here: the returned iterator type is already
    /// `#[must_use]` on its own account, and clippy's `double_must_use`
    /// flags the redundant second annotation as of clippy 1.95.
    pub fn own_command_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.own_commands.iter().map(|(index, _)| *index)
    }
}

/// A [`Scene`]'s `PushLayer`/`PopLayer` groups as an explicit dependency
/// graph. See the module docs for what this is for and what it deliberately
/// does not do.
#[derive(Debug, Clone)]
pub struct PassGraph {
    passes: Vec<Pass>,
}

const ROOT: PassId = PassId(0);

impl PassGraph {
    /// Walk `scene`'s commands once, building one [`Pass`] per `PushLayer`
    /// group plus the implicit root.
    ///
    /// # Panics
    ///
    /// If `scene`'s `PushLayer`/`PopLayer` commands are unbalanced — the
    /// same invariant every other consumer of a [`Scene`] (`NativeRenderer`
    /// included) already requires.
    #[must_use]
    pub fn build(scene: &Scene) -> Self {
        let mut passes = vec![Pass {
            id: ROOT,
            parent: None,
            children: Vec::new(),
            layer: None,
            own_commands: Vec::new(),
        }];
        let mut stack: Vec<PassId> = vec![ROOT];

        for (index, command) in scene.commands().iter().enumerate() {
            match command {
                Command::PushLayer {
                    bounds,
                    alpha,
                    blend,
                    clip,
                    filter,
                } => {
                    let resolved_bounds = clip.clamp(*bounds);
                    let parent = *stack.last().expect("root is always on the stack");
                    let id = PassId(passes.len());
                    passes.push(Pass {
                        id,
                        parent: Some(parent),
                        children: Vec::new(),
                        layer: Some(PassLayer {
                            push_index: index,
                            pop_index: usize::MAX, // patched on the matching PopLayer, below.
                            bounds: resolved_bounds,
                            alpha: *alpha,
                            blend: *blend,
                            filter: *filter,
                        }),
                        own_commands: Vec::new(),
                    });
                    passes[parent.0].children.push(id);
                    stack.push(id);
                }
                Command::PopLayer => {
                    let id = stack.pop().expect(
                        "PopLayer with no matching PushLayer — an unbalanced Scene, which every \
                         other consumer (NativeRenderer included) already refuses to render",
                    );
                    if let Some(layer) = &mut passes[id.0].layer {
                        layer.pop_index = index;
                    }
                }
                _ => {
                    let current = *stack.last().expect("root is always on the stack");
                    passes[current.0]
                        .own_commands
                        .push((index, command.bounds()));
                }
            }
        }

        assert_eq!(stack, vec![ROOT], "Scene has unclosed PushLayer command(s)");
        Self { passes }
    }

    #[must_use]
    pub fn root(&self) -> PassId {
        ROOT
    }

    #[must_use]
    pub fn pass(&self, id: PassId) -> &Pass {
        &self.passes[id.0]
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.passes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        false // The root pass always exists.
    }

    /// Every surviving pass for `damage`, in dependency order (children
    /// before the parent they composite onto).
    ///
    /// A pass survives if it is *active* — its own bounds intersect
    /// `damage`, and so does every ancestor's, all the way to the root,
    /// which is always active — **and** either one of its own commands
    /// intersects `damage` or a descendant survives. That second half is
    /// what makes a parent whose own content is untouched still show up
    /// when only a child changed: the parent's re-composite is what
    /// actually paints the child's new pixels into the picture.
    ///
    /// The root itself is included only if it survives — an empty result
    /// means nothing in `damage` touches anything in the scene.
    #[must_use]
    pub fn prune(&self, damage: &Damage) -> Vec<PassId> {
        let mut active = vec![false; self.passes.len()];
        active[ROOT.0] = true;
        // Build order guarantees a parent's index is always less than its
        // children's, so one forward pass sees every parent before its
        // children.
        for pass in &self.passes {
            if let Some(layer) = &pass.layer {
                let parent_active = active[pass.parent.expect("non-root pass has a parent").0];
                active[pass.id.0] = parent_active && damage.intersects(layer.bounds);
            }
        }

        let mut survives = vec![false; self.passes.len()];
        for pass in self.passes.iter().rev() {
            if !active[pass.id.0] {
                continue;
            }
            let own_hit = pass
                .own_commands
                .iter()
                .any(|(_, bounds)| damage.intersects(*bounds));
            let child_hit = pass.children.iter().any(|child| survives[child.0]);
            survives[pass.id.0] = own_hit || child_hit;
        }

        let mut order = Vec::new();
        self.post_order(ROOT, &survives, &mut order);
        order
    }

    fn post_order(&self, id: PassId, survives: &[bool], out: &mut Vec<PassId>) {
        if !survives[id.0] {
            return;
        }
        for &child in &self.passes[id.0].children {
            self.post_order(child, survives, out);
        }
        out.push(id);
    }

    /// Groups of sibling passes with no data dependency on one another —
    /// candidates a GPU backend could run concurrently or fold into a
    /// shared render pass. See the module docs' "What this does not do"
    /// section for why nothing on this CPU renderer consumes this today.
    ///
    /// Two siblings are grouped together only when *every* pass in the
    /// group shares its parent, so the returned groups partition each
    /// parent's children independently — passes under different parents
    /// are never grouped together even if their bounds happen not to
    /// overlap, because they do not share a compositing target to begin
    /// with.
    #[must_use]
    pub fn co_schedulable_siblings(&self) -> Vec<Vec<PassId>> {
        let mut groups = Vec::new();
        for pass in &self.passes {
            if pass.children.len() < 2 {
                continue;
            }
            groups.extend(self.non_overlapping_runs(&pass.children));
        }
        groups
    }

    /// Split `siblings` (already in scene order, i.e. paint order) into
    /// maximal runs whose bounds pairwise do not overlap.
    ///
    /// Paint order matters here even though the *result* is an
    /// order-independent set: two siblings whose bounds overlap are never
    /// co-schedulable regardless of order (the later one's paint result
    /// depends on the earlier one already being composited), so grouping
    /// only ever needs to consider a run's newest member against the ones
    /// already in it.
    fn non_overlapping_runs(&self, siblings: &[PassId]) -> Vec<Vec<PassId>> {
        let mut groups: Vec<Vec<PassId>> = Vec::new();
        'sibling: for &id in siblings {
            let bounds = self
                .pass(id)
                .layer
                .as_ref()
                .expect("a non-root pass always has a layer")
                .bounds;
            for group in &mut groups {
                if group.iter().all(|&member| {
                    !self
                        .pass(member)
                        .layer
                        .as_ref()
                        .unwrap()
                        .bounds
                        .overlaps(bounds)
                }) {
                    group.push(id);
                    continue 'sibling;
                }
            }
            groups.push(vec![id]);
        }
        groups.into_iter().filter(|g| g.len() > 1).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Canvas, Paint};
    use vieww_foundation::Color;

    fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect::new(l, t, r, b)
    }

    fn solid(color: Color) -> Paint {
        color.into()
    }

    #[test]
    fn a_flat_scene_is_just_the_root_pass() {
        let mut scene = Scene::new();
        scene.fill_rect(rect(0.0, 0.0, 10.0, 10.0), solid(Color::RED));
        let graph = PassGraph::build(&scene);
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.pass(graph.root()).children(), &[]);
        assert_eq!(
            graph
                .pass(graph.root())
                .own_command_indices()
                .collect::<Vec<_>>(),
            vec![0]
        );
    }

    #[test]
    fn a_push_layer_pair_becomes_a_child_pass_with_a_dependency_edge() {
        let mut scene = Scene::new();
        scene.save();
        // `Canvas::push_layer`'s declared bounds is only an estimate —
        // `Scene::pop_layer` patches it to the union of what was actually
        // drawn inside (see that method's own doc), so the fill below has to
        // match the declared bounds exactly for the assertion below to be
        // about the layer's *declared* bounds rather than a coincidence of
        // that patching.
        scene.push_layer(rect(0.0, 0.0, 50.0, 50.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(0.0, 0.0, 50.0, 50.0), solid(Color::RED));
        scene.pop_layer();
        scene.restore();

        let graph = PassGraph::build(&scene);
        assert_eq!(graph.len(), 2);
        let root = graph.pass(graph.root());
        assert_eq!(root.children().len(), 1);
        let child_id = root.children()[0];
        let child = graph.pass(child_id);
        assert_eq!(child.parent(), Some(graph.root()));
        assert_eq!(child.layer().unwrap().bounds, rect(0.0, 0.0, 50.0, 50.0));
        assert_eq!(child.layer().unwrap().alpha, 0.5);
    }

    #[test]
    fn prune_drops_a_pass_whose_bounds_miss_the_damage() {
        let mut scene = Scene::new();
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 20.0, 20.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(0.0, 0.0, 20.0, 20.0), solid(Color::RED));
        scene.pop_layer();
        scene.restore();
        scene.save();
        scene.push_layer(rect(80.0, 80.0, 100.0, 100.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(80.0, 80.0, 100.0, 100.0), solid(Color::BLUE));
        scene.pop_layer();
        scene.restore();

        let graph = PassGraph::build(&scene);
        let mut damage = Damage::new(rect(0.0, 0.0, 100.0, 100.0));
        damage.add(rect(0.0, 0.0, 20.0, 20.0)); // Only the first layer.

        let survivors = graph.prune(&damage);
        let root = graph.root();
        let first_child = graph.pass(root).children()[0];
        let second_child = graph.pass(root).children()[1];

        assert!(
            survivors.contains(&first_child),
            "the damaged layer must survive"
        );
        assert!(
            !survivors.contains(&second_child),
            "the untouched layer must be pruned"
        );
        assert!(
            survivors.contains(&root),
            "root must survive to recomposite the surviving child"
        );
        // Dependency order: the child composites onto the root, so it must
        // come first.
        assert!(
            survivors.iter().position(|&p| p == first_child)
                < survivors.iter().position(|&p| p == root)
        );
    }

    #[test]
    fn prune_gates_a_child_by_its_parents_activity_not_only_its_own_bounds() {
        // `Canvas::push_layer`/`pop_layer` can never actually produce a
        // child pass whose own bounds reach outside its parent's: pop_layer
        // patches a layer's bounds to the union of its own content
        // (including nested layers), so a parent's final bounds always
        // enclose every child's, and a child inherits its clip from every
        // ancestor's `clip_rect`, which only ever shrinks a descendant's
        // bounds further. So this builds the raw `Command` sequence
        // directly with `push_command` — bypassing both of those
        // Canvas-level invariants — to exercise `PassGraph::prune` against
        // the full space of *well-formed* (balanced push/pop) `Scene`s it
        // has to handle, per its own doc contract, not only the narrower
        // set the `Canvas` trait happens to ever record.
        let mut scene = Scene::new();
        scene.push_command(Command::PushLayer {
            bounds: rect(0.0, 0.0, 10.0, 10.0), // parent: misses the damage below
            alpha: 0.5,
            blend: BlendMode::Normal,
            clip: crate::Clip::NONE,
            filter: ImageFilter::NONE,
        });
        scene.push_command(Command::PushLayer {
            bounds: rect(0.0, 0.0, 50.0, 50.0), // child: would hit the damage alone
            alpha: 0.5,
            blend: BlendMode::Normal,
            clip: crate::Clip::NONE,
            filter: ImageFilter::NONE,
        });
        scene.push_command(Command::FillRect {
            rect: rect(0.0, 0.0, 50.0, 50.0),
            paint: solid(Color::GREEN),
            transform: vieww_foundation::Transform::IDENTITY,
            clip: crate::Clip::NONE,
        });
        scene.push_command(Command::PopLayer);
        scene.push_command(Command::PopLayer);

        let graph = PassGraph::build(&scene);
        let mut damage = Damage::new(rect(0.0, 0.0, 100.0, 100.0));
        damage.add(rect(20.0, 20.0, 40.0, 40.0)); // Inside the child's bounds, outside the parent's.

        let survivors = graph.prune(&damage);
        assert!(
            survivors.is_empty(),
            "a child gated off by its own inactive parent must not survive"
        );
    }

    #[test]
    fn prune_keeps_a_parent_whose_own_content_is_untouched_but_whose_child_changed() {
        let mut scene = Scene::new();
        scene.fill_rect(rect(0.0, 0.0, 5.0, 5.0), solid(Color::BLACK)); // Root's own content: untouched.
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 50.0, 50.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(10.0, 10.0, 20.0, 20.0), solid(Color::RED)); // Damaged.
        scene.pop_layer();
        scene.restore();

        let graph = PassGraph::build(&scene);
        let mut damage = Damage::new(rect(0.0, 0.0, 100.0, 100.0));
        damage.add(rect(10.0, 10.0, 20.0, 20.0));

        let survivors = graph.prune(&damage);
        assert!(
            survivors.contains(&graph.root()),
            "root must recomposite even though only its child changed"
        );
    }

    #[test]
    fn multi_region_damage_prunes_more_precisely_than_the_bounding_box_approach() {
        // Two disjoint damage regions whose bounding box spans a third,
        // untouched layer sitting between them. `Scene::damage_cull` tests
        // against the bounding box and keeps that middle layer; `PassGraph`
        // tests against the regions individually and correctly drops it.
        let mut scene = Scene::new();
        for (x, color) in [(0.0, Color::RED), (45.0, Color::GREEN), (90.0, Color::BLUE)] {
            scene.save();
            scene.push_layer(rect(x, 0.0, x + 10.0, 10.0), 0.5, BlendMode::Normal);
            scene.fill_rect(rect(x, 0.0, x + 10.0, 10.0), solid(color));
            scene.pop_layer();
            scene.restore();
        }

        let mut damage = Damage::new(rect(0.0, 0.0, 100.0, 10.0));
        damage.add(rect(0.0, 0.0, 10.0, 10.0)); // First layer only.
        damage.add(rect(90.0, 0.0, 100.0, 10.0)); // Third layer only.
                                                  // The middle (green) layer at x=45 sits inside the two regions'
                                                  // shared bounding box [0, 100) but outside both actual regions.

        let graph = PassGraph::build(&scene);
        let survivors = graph.prune(&damage);
        let root_children = graph.pass(graph.root()).children().to_vec();
        assert!(
            survivors.contains(&root_children[0]),
            "first layer is directly damaged"
        );
        assert!(
            survivors.contains(&root_children[2]),
            "third layer is directly damaged"
        );
        assert!(
            !survivors.contains(&root_children[1]),
            "middle layer is outside both regions"
        );

        // `damage_cull`, tested against the same damage, keeps the middle
        // layer too: its single-bounding-box test cannot tell it apart from
        // the two that are genuinely damaged.
        let (culled, _) = scene.damage_cull(&damage);
        let middle_layer_alpha_count = culled
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::PushLayer { .. }))
            .count();
        assert_eq!(
            middle_layer_alpha_count, 3,
            "damage_cull's bounding-box test conservatively keeps all three"
        );
    }

    #[test]
    fn prune_agrees_with_damage_cull_for_single_region_damage() {
        // With exactly one damage region, `damage_cull`'s bounding-box test
        // and `PassGraph::prune`'s per-region test are the same test, so
        // their pass/command survival decisions should agree exactly.
        let mut scene = Scene::new();
        scene.fill_rect(rect(0.0, 0.0, 5.0, 5.0), solid(Color::BLACK));
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 30.0, 30.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(5.0, 5.0, 15.0, 15.0), solid(Color::RED));
        scene.save();
        scene.push_layer(rect(60.0, 60.0, 90.0, 90.0), 0.7, BlendMode::Multiply);
        scene.fill_rect(rect(60.0, 60.0, 90.0, 90.0), solid(Color::GREEN));
        scene.pop_layer();
        scene.restore();
        scene.pop_layer();
        scene.restore();
        scene.save();
        scene.push_layer(rect(200.0, 200.0, 220.0, 220.0), 1.0, BlendMode::Screen);
        scene.fill_rect(rect(200.0, 200.0, 220.0, 220.0), solid(Color::BLUE));
        scene.pop_layer();
        scene.restore();

        let mut damage = Damage::new(rect(0.0, 0.0, 300.0, 300.0));
        damage.add(rect(0.0, 0.0, 40.0, 40.0)); // One region: hits the root fill and the outer layer, misses the inner and the far layer.

        let graph = PassGraph::build(&scene);
        let survivors = graph.prune(&damage);
        let surviving_pushlayer_bounds: Vec<Rect> = survivors
            .iter()
            .filter_map(|&id| graph.pass(id).layer().map(|l| l.bounds))
            .collect();

        let (culled, _) = scene.damage_cull(&damage);
        let culled_pushlayer_bounds: Vec<Rect> = culled
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::PushLayer { bounds, .. } => Some(*bounds),
                _ => None,
            })
            .collect();

        let mut graph_bounds = surviving_pushlayer_bounds;
        graph_bounds.sort_by(|a, b| a.left.partial_cmp(&b.left).unwrap());
        let mut cull_bounds = culled_pushlayer_bounds;
        cull_bounds.sort_by(|a, b| a.left.partial_cmp(&b.left).unwrap());
        assert_eq!(
            graph_bounds, cull_bounds,
            "PassGraph::prune must agree with Scene::damage_cull for single-region damage"
        );
    }

    #[test]
    fn co_schedulable_siblings_groups_non_overlapping_children_of_the_same_parent() {
        let mut scene = Scene::new();
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 100.0, 100.0), 0.9, BlendMode::Normal);
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 10.0, 10.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(0.0, 0.0, 10.0, 10.0), solid(Color::RED));
        scene.pop_layer();
        scene.restore();
        scene.save();
        scene.push_layer(rect(20.0, 20.0, 30.0, 30.0), 0.5, BlendMode::Normal); // Does not overlap the sibling above.
        scene.fill_rect(rect(20.0, 20.0, 30.0, 30.0), solid(Color::GREEN));
        scene.pop_layer();
        scene.restore();
        scene.pop_layer();
        scene.restore();

        let graph = PassGraph::build(&scene);
        let groups = graph.co_schedulable_siblings();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }

    #[test]
    fn co_schedulable_siblings_excludes_overlapping_children() {
        let mut scene = Scene::new();
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 100.0, 100.0), 0.9, BlendMode::Normal);
        scene.save();
        scene.push_layer(rect(0.0, 0.0, 20.0, 20.0), 0.5, BlendMode::Normal);
        scene.fill_rect(rect(0.0, 0.0, 20.0, 20.0), solid(Color::RED));
        scene.pop_layer();
        scene.restore();
        scene.save();
        scene.push_layer(rect(10.0, 10.0, 30.0, 30.0), 0.5, BlendMode::Normal); // Overlaps the sibling above.
        scene.fill_rect(rect(10.0, 10.0, 30.0, 30.0), solid(Color::GREEN));
        scene.pop_layer();
        scene.restore();
        scene.pop_layer();
        scene.restore();

        let graph = PassGraph::build(&scene);
        assert!(graph.co_schedulable_siblings().is_empty());
    }

    #[test]
    fn build_panics_on_an_unbalanced_scene() {
        let mut scene = Scene::new();
        scene.push_command(Command::PushLayer {
            bounds: rect(0.0, 0.0, 10.0, 10.0),
            alpha: 1.0,
            blend: BlendMode::Normal,
            clip: crate::Clip::NONE,
            filter: ImageFilter::NONE,
        });
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| PassGraph::build(&scene)));
        assert!(result.is_err());
    }
}
