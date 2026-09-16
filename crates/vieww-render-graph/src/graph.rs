//! The graph builder.
//!
//! # Ordering contract
//!
//! A caller declares passes in an order where a pass that reads a resource
//! is added *after* the pass (or passes) that write it — the natural order
//! any tree walk (see [`crate::scene_bridge`]) already produces, since a
//! child layer's content exists before its parent composites it.
//! [`Graph::compile`] does not require this for correctness (its
//! topological sort in `crate::schedule` handles passes declared in any
//! order, and a unit test in that module proves it), but declaring
//! out of order is needless work for [`Graph::compile`] to undo, so builders
//! should still follow it.
//!
//! A resource read with no prior writer in the graph is not an error — it
//! means "whatever is already in this resource" (a cleared target, an
//! imported previous frame), and gets no dependency edge.

use crate::pass::{PassDesc, PassId};
use crate::plan::{CompileError, ExecutionPlan};
use crate::resource::{ResourceDesc, ResourceId};

/// A render graph under construction.
#[derive(Debug, Default)]
pub struct Graph {
    resources: Vec<ResourceDesc>,
    passes: Vec<PassDesc>,
}

impl Graph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_resource(&mut self, desc: ResourceDesc) -> ResourceId {
        let id = ResourceId(self.resources.len() as u32);
        self.resources.push(desc);
        id
    }

    pub fn add_pass(&mut self, desc: PassDesc) -> PassId {
        let id = PassId(self.passes.len() as u32);
        self.passes.push(desc);
        id
    }

    #[must_use]
    pub fn resource(&self, id: ResourceId) -> &ResourceDesc {
        &self.resources[id.index()]
    }

    #[must_use]
    pub fn pass(&self, id: PassId) -> &PassDesc {
        &self.passes[id.index()]
    }

    #[must_use]
    pub fn resources(&self) -> &[ResourceDesc] {
        &self.resources
    }

    #[must_use]
    pub fn passes(&self) -> &[PassDesc] {
        &self.passes
    }

    /// Find the pass that writes `resource`, if any pass does.
    ///
    /// # Single-writer resources
    ///
    /// A well-formed graph writes each resource from exactly one pass —
    /// the same discipline frame graphs in other engines use, and what
    /// makes "the pass this read depends on" unambiguous without also
    /// needing declaration order to disambiguate it. Content that
    /// genuinely accumulates over multiple passes (a target cleared, then
    /// drawn into by two separate raster passes) is modelled as two
    /// resources — the clear's output feeds the second pass as a read —
    /// not one resource written twice.
    ///
    /// [`Graph::compile`] checks this invariant explicitly
    /// ([`crate::plan::CompileError::MultipleWriters`]); this accessor
    /// itself just returns whichever writer was declared last, so it stays
    /// usable for inspection even on a graph that has not been validated
    /// yet.
    #[must_use]
    pub fn writer_of(&self, resource: ResourceId) -> Option<PassId> {
        self.passes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, p)| p.writes.contains(&resource))
            .map(|(i, _)| PassId(i as u32))
    }

    /// Every pass that writes `resource`, in declaration order. Used by
    /// [`Graph::compile`] to detect a violation of the single-writer
    /// invariant before it can produce a silently wrong dependency edge.
    pub(crate) fn writers_of(&self, resource: ResourceId) -> impl Iterator<Item = PassId> + '_ {
        self.passes
            .iter()
            .enumerate()
            .filter_map(move |(i, p)| p.writes.contains(&resource).then_some(PassId(i as u32)))
    }

    /// Compile this graph into a backend-agnostic [`ExecutionPlan`]:
    /// dependency-ordered, dead passes culled, transient resources aliased,
    /// barriers computed, concurrency batches identified.
    ///
    /// `present` is the resource the caller actually needs at the end —
    /// typically a [`crate::resource::ResourceKind::Presentable`] target.
    /// Nothing that cannot transitively affect `present` survives
    /// compilation.
    pub fn compile(&self, present: ResourceId) -> Result<ExecutionPlan, CompileError> {
        crate::plan::compile(self, present)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::PassKind;
    use crate::resource::ResourceDesc;
    use vieww_scene::CostHint;

    #[test]
    fn writer_of_finds_the_declaring_pass() {
        let mut g = Graph::new();
        let r = g.add_resource(ResourceDesc::color_target(64, 64, "a"));
        let p =
            g.add_pass(PassDesc::new("draw", PassKind::Raster, CostHint::CONSERVATIVE).writing(r));
        assert_eq!(g.writer_of(r), Some(p));
    }

    #[test]
    fn an_unwritten_resource_has_no_writer() {
        let mut g = Graph::new();
        let r = g.add_resource(ResourceDesc::color_target(64, 64, "a"));
        assert_eq!(g.writer_of(r), None);
    }
}
