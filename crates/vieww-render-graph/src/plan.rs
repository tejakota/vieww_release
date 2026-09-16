//! Compiling a [`crate::graph::Graph`] into an [`ExecutionPlan`].
//!
//! This is the "what is the cheapest possible execution plan for this
//! frame" question `docs/RENDERER-V2-NOTES.md` poses, answered in five
//! backend-agnostic steps: validate, order, cull, alias, batch. Every step
//! is pure data transformation over [`crate::pass::PassDesc`]/
//! [`crate::resource::ResourceDesc`] — nothing here touches a GPU, a
//! shader, or a pixel, which is exactly what lets it be unit-tested
//! without either.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::graph::Graph;
use crate::pass::PassId;
use crate::resource::ResourceId;
use crate::transient;

/// Why [`Graph::compile`](crate::graph::Graph::compile) refused to produce a
/// plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// Two or more passes declared a write to the same resource. See
    /// [`Graph::writer_of`](crate::graph::Graph::writer_of)'s docs for why
    /// this is rejected rather than resolved by declaration order.
    MultipleWriters(ResourceId),
    /// `present` names a resource nothing in the graph writes — there is
    /// nothing to compile a plan *toward*.
    PresentNeverWritten(ResourceId),
    /// The write-dependency graph has a cycle. Unreachable through
    /// [`Graph`]'s own builder (every edge is resource-mediated and
    /// [`MultipleWriters`](Self::MultipleWriters) is rejected first), kept
    /// as a named error rather than a panic for a `Graph` assembled by hand
    /// or by a future caller this crate does not control.
    Cycle,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MultipleWriters(r) => {
                write!(f, "resource {r:?} is written by more than one pass")
            }
            Self::PresentNeverWritten(r) => write!(f, "present resource {r:?} is never written"),
            Self::Cycle => write!(f, "the pass dependency graph has a cycle"),
        }
    }
}

impl std::error::Error for CompileError {}

/// A resource transition a backend must synchronize before a pass runs.
///
/// This is the CPU-computable half of "barrier generation"
/// (`docs/RENDERER-V2-NOTES.md`'s Architecture section) — *which* pass needs
/// to wait for *which* resource is a pure data-flow fact, independent of
/// which GPU API expresses the wait as `vkCmdPipelineBarrier`,
/// `MTLFence`, or a D3D12 resource-state transition. A backend maps one
/// `Barrier` to its own API's call; this crate does not know that API
/// exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Barrier {
    /// The pass that must wait.
    pub before_pass: PassId,
    /// The resource whose availability it is waiting on.
    pub resource: ResourceId,
    /// The pass whose write this barrier waits for.
    pub after_pass: PassId,
}

/// A compiled, backend-agnostic plan: what runs, in what order, what can
/// run concurrently, what needs to wait for what, and which resources share
/// physical memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    /// Surviving passes, dependency-ordered (a pass never precedes anything
    /// it depends on).
    pub order: Vec<PassId>,
    /// Passes present in the graph but not reachable from `present` —
    /// dead work, never scheduled. Kept (rather than dropped silently) so a
    /// devtool can show what culling actually removed.
    pub culled: Vec<PassId>,
    /// Passes grouped into levels: everything in one inner `Vec` has no
    /// dependency on anything else in that same `Vec`, so a backend with
    /// multiple queues (or a CPU thread pool) may run them concurrently.
    /// Levels themselves are still ordered — level *n+1* may depend on
    /// level *n*.
    pub concurrency_batches: Vec<Vec<PassId>>,
    /// Every synchronization point a backend must honor, in the order
    /// `order` reaches them.
    pub barriers: Vec<Barrier>,
    /// Which declared resources alias which physical "slot" — two
    /// resources with the same slot number never need to be live at the
    /// same time and may share one allocation. Slot numbers are dense from
    /// zero; see `transient::assign_slots` for the assignment algorithm.
    pub resource_slots: HashMap<ResourceId, u32>,
    /// How many distinct physical slots the plan actually needs — the
    /// number a backend allocates, versus `resources().len()` declarations.
    pub physical_slot_count: u32,
}

pub(crate) fn compile(graph: &Graph, present: ResourceId) -> Result<ExecutionPlan, CompileError> {
    let n = graph.passes().len();

    // 1. Validate the single-writer invariant for every resource actually
    //    written by more than zero passes.
    for (i, desc) in graph.resources().iter().enumerate() {
        let _ = desc;
        let id = ResourceId(i as u32);
        let mut writers = graph.writers_of(id);
        if let Some(_first) = writers.next() {
            if writers.next().is_some() {
                return Err(CompileError::MultipleWriters(id));
            }
        }
    }

    if graph.writer_of(present).is_none() {
        return Err(CompileError::PresentNeverWritten(present));
    }

    // 2. Build dependency edges: reader depends on the unique writer of
    //    each resource it reads (resources with no writer are already-
    //    resident and contribute no edge).
    let mut depends_on: Vec<HashSet<PassId>> = vec![HashSet::new(); n];
    let mut dependents: Vec<HashSet<PassId>> = vec![HashSet::new(); n];
    for (i, pass) in graph.passes().iter().enumerate() {
        let reader = PassId(i as u32);
        for &r in &pass.reads {
            if let Some(writer) = graph.writer_of(r) {
                if writer != reader {
                    depends_on[i].insert(writer);
                    dependents[writer.index()].insert(reader);
                }
            }
        }
    }

    // 3. Cull: keep only passes transitively needed to produce `present`.
    let root = graph.writer_of(present).expect("checked above");
    let mut needed: HashSet<PassId> = HashSet::new();
    let mut stack = vec![root];
    while let Some(p) = stack.pop() {
        if needed.insert(p) {
            for dep in &depends_on[p.index()] {
                stack.push(*dep);
            }
        }
    }
    let culled: Vec<PassId> = (0..n)
        .map(|i| PassId(i as u32))
        .filter(|p| !needed.contains(p))
        .collect();

    // 4. Topological order + concurrency levels via Kahn's algorithm,
    //    restricted to `needed` passes. This is a real toposort (not just
    //    "keep declaration order") so passes declared out of order still
    //    compile correctly — see `tests::out_of_order_declaration_still_
    //    compiles_correctly`.
    let mut in_degree: HashMap<PassId, usize> = needed
        .iter()
        .map(|p| {
            (
                *p,
                depends_on[p.index()]
                    .iter()
                    .filter(|d| needed.contains(d))
                    .count(),
            )
        })
        .collect();
    let mut frontier: VecDeque<PassId> = in_degree
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(p, _)| *p)
        .collect();
    // Deterministic order: process the current frontier lowest-index-first.
    let mut frontier_vec: Vec<PassId> = frontier.drain(..).collect();
    frontier_vec.sort();

    let mut order = Vec::with_capacity(needed.len());
    let mut concurrency_batches: Vec<Vec<PassId>> = Vec::new();
    let mut remaining = needed.len();
    let mut current = frontier_vec;

    while !current.is_empty() {
        remaining -= current.len();
        order.extend(current.iter().copied());
        concurrency_batches.push(current.clone());

        let mut next: Vec<PassId> = Vec::new();
        for p in &current {
            for dependent in dependents[p.index()].iter().filter(|d| needed.contains(d)) {
                let deg = in_degree.get_mut(dependent).expect("tracked");
                *deg -= 1;
                if *deg == 0 {
                    next.push(*dependent);
                }
            }
        }
        next.sort();
        next.dedup();
        current = next;
    }

    if remaining != 0 {
        return Err(CompileError::Cycle);
    }

    // 5. Barriers: one per dependency edge that survives culling.
    let mut barriers = Vec::new();
    for &reader in &order {
        for &r in &graph.pass(reader).reads {
            if let Some(writer) = graph.writer_of(r) {
                if needed.contains(&writer) && writer != reader {
                    barriers.push(Barrier {
                        before_pass: reader,
                        resource: r,
                        after_pass: writer,
                    });
                }
            }
        }
    }

    // 6. Transient resource aliasing over the surviving order.
    let (resource_slots, physical_slot_count) = transient::assign_slots(graph, &order, present);

    Ok(ExecutionPlan {
        order,
        culled,
        concurrency_batches,
        barriers,
        resource_slots,
        physical_slot_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::{PassDesc, PassKind};
    use crate::resource::ResourceDesc;
    use vieww_scene::CostHint;

    fn cost() -> CostHint {
        CostHint::CONSERVATIVE
    }

    #[test]
    fn a_dead_pass_is_culled() {
        let mut g = Graph::new();
        let live = g.add_resource(ResourceDesc::presentable(32, 32, "screen"));
        let dead = g.add_resource(ResourceDesc::color_target(32, 32, "unused"));

        let p_live = g.add_pass(PassDesc::new("live", PassKind::Raster, cost()).writing(live));
        let p_dead = g.add_pass(PassDesc::new("dead", PassKind::Raster, cost()).writing(dead));

        let plan = g.compile(live).unwrap();
        assert_eq!(plan.order, vec![p_live]);
        assert_eq!(plan.culled, vec![p_dead]);
    }

    #[test]
    fn out_of_order_declaration_still_compiles_correctly() {
        let mut g = Graph::new();
        let a = g.add_resource(ResourceDesc::color_target(32, 32, "a"));
        let screen = g.add_resource(ResourceDesc::presentable(32, 32, "screen"));

        // The composite pass is declared *before* the pass that produces
        // the resource it reads.
        let composite = g.add_pass(
            PassDesc::new("composite", PassKind::Composite, cost())
                .reading(a)
                .writing(screen),
        );
        let produce_a = g.add_pass(PassDesc::new("produce_a", PassKind::Raster, cost()).writing(a));

        let plan = g.compile(screen).unwrap();
        let pos_produce = plan.order.iter().position(|p| *p == produce_a).unwrap();
        let pos_composite = plan.order.iter().position(|p| *p == composite).unwrap();
        assert!(pos_produce < pos_composite);
    }

    #[test]
    fn multiple_writers_is_rejected() {
        let mut g = Graph::new();
        let r = g.add_resource(ResourceDesc::color_target(32, 32, "r"));
        g.add_pass(PassDesc::new("a", PassKind::Raster, cost()).writing(r));
        g.add_pass(PassDesc::new("b", PassKind::Raster, cost()).writing(r));
        assert_eq!(g.compile(r), Err(CompileError::MultipleWriters(r)));
    }

    #[test]
    fn present_never_written_is_rejected() {
        let mut g = Graph::new();
        let r = g.add_resource(ResourceDesc::presentable(32, 32, "screen"));
        assert_eq!(g.compile(r), Err(CompileError::PresentNeverWritten(r)));
    }

    #[test]
    fn independent_passes_land_in_the_same_concurrency_batch() {
        let mut g = Graph::new();
        let a = g.add_resource(ResourceDesc::color_target(32, 32, "a"));
        let b = g.add_resource(ResourceDesc::color_target(32, 32, "b"));
        let screen = g.add_resource(ResourceDesc::presentable(32, 32, "screen"));

        let pa = g.add_pass(PassDesc::new("a", PassKind::Raster, cost()).writing(a));
        let pb = g.add_pass(PassDesc::new("b", PassKind::Raster, cost()).writing(b));
        g.add_pass(
            PassDesc::new("composite", PassKind::Composite, cost())
                .reading(a)
                .reading(b)
                .writing(screen),
        );

        let plan = g.compile(screen).unwrap();
        assert_eq!(plan.concurrency_batches.len(), 2);
        let mut first_batch = plan.concurrency_batches[0].clone();
        first_batch.sort();
        let mut expected = vec![pa, pb];
        expected.sort();
        assert_eq!(first_batch, expected);
    }
}
