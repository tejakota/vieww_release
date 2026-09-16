//! A human/machine-readable description of a compiled render graph.
//!
//! `vieww-render-graph`'s [`Graph`] and [`ExecutionPlan`] already carry every
//! fact a devtools panel would want: which passes exist, what each one reads
//! and writes, which passes a compile decided were dead, which ones can run
//! concurrently, and which physical memory slots got aliased onto which
//! declared resources. This module does not compute any of that a second
//! time — it walks the real [`Graph`]/[`ExecutionPlan`] pair a caller already
//! compiled and renders it as a report, the same relationship
//! [`crate::inspector::Inspector`] has to `ElementTree`.
//!
//! # Why it takes both the graph and the plan
//!
//! [`Graph`] alone has passes and resources but no notion of "did this
//! survive compilation" or "what order does this actually run in" —
//! [`ExecutionPlan`] is where culling, ordering, and aliasing live. A report
//! that only looked at the graph would be describing what was *declared*;
//! one that only looked at the plan would be missing names (an
//! [`ExecutionPlan`] talks in [`PassId`]/[`ResourceId`], not
//! `&'static str`). Both together produce a report a person can actually
//! read.

use std::collections::{HashMap, HashSet};

use vieww_render_graph::{ExecutionPlan, Graph, PassId, ResourceId};

/// One pass, described for a report rather than for a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassReport {
    pub id: PassId,
    pub name: &'static str,
    pub kind_name: &'static str,
    /// Names of the resources this pass reads.
    pub reads: Vec<&'static str>,
    /// Names of the resources this pass writes.
    pub writes: Vec<&'static str>,
    /// `true` if [`ExecutionPlan::culled`] contains this pass — declared but
    /// never actually needed to produce the presented output.
    pub culled: bool,
    /// Which concurrency batch this pass landed in (see
    /// [`ExecutionPlan::concurrency_batches`]), `None` if the pass was
    /// culled and so appears in no batch.
    pub concurrency_batch: Option<usize>,
}

/// A full report over one compiled graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderGraphReport {
    pub passes: Vec<PassReport>,
    /// How many distinct physical memory slots the plan needed, versus how
    /// many resources were declared — the aliasing win in one pair of
    /// numbers.
    pub declared_resource_count: usize,
    pub physical_slot_count: u32,
    /// How many passes were culled as dead work.
    pub culled_count: usize,
    /// How many concurrency batches the plan produced — 1 means everything
    /// is serialized, more than 1 means some passes can run in parallel.
    pub concurrency_batch_count: usize,
}

impl RenderGraphReport {
    /// Build a report from a graph and the plan compiled from it.
    ///
    /// `plan` is assumed to have been produced by `graph.compile(..)` — a
    /// plan compiled from a *different* graph would have [`PassId`]s that
    /// resolve to unrelated passes in this one, silently mislabeling the
    /// report. Nothing in either type carries a shared identity to check
    /// this against, so it is a caller contract rather than something this
    /// function can validate; keeping the two arguments separate (rather
    /// than, say, this module owning the compile step) is what lets a caller
    /// reuse an already-compiled plan without recompiling just to inspect it.
    #[must_use]
    pub fn build(graph: &Graph, plan: &ExecutionPlan) -> Self {
        let culled: HashSet<PassId> = plan.culled.iter().copied().collect();
        let mut batch_of: HashMap<PassId, usize> = HashMap::new();
        for (index, batch) in plan.concurrency_batches.iter().enumerate() {
            for &pass in batch {
                batch_of.insert(pass, index);
            }
        }

        let resource_name = |id: ResourceId| graph.resource(id).debug_name;

        // `PassId`'s inner index is private to `vieww-render-graph`, so the
        // only ids this module can ever hold are ones handed back by that
        // crate's own API — here, `plan.order` and `plan.culled`, which
        // together enumerate every pass in the graph exactly once (`order`
        // is the needed passes, `culled` is everything else `compile`
        // decided was dead work).
        let mut ids: Vec<PassId> = plan
            .order
            .iter()
            .chain(plan.culled.iter())
            .copied()
            .collect();
        ids.sort();

        let passes = ids
            .into_iter()
            .map(|id| {
                let desc = graph.pass(id);
                PassReport {
                    id,
                    name: desc.name,
                    kind_name: pass_kind_name(desc.kind),
                    reads: desc.reads.iter().copied().map(resource_name).collect(),
                    writes: desc.writes.iter().copied().map(resource_name).collect(),
                    culled: culled.contains(&id),
                    concurrency_batch: batch_of.get(&id).copied(),
                }
            })
            .collect();

        Self {
            passes,
            declared_resource_count: graph.resources().len(),
            physical_slot_count: plan.physical_slot_count,
            culled_count: plan.culled.len(),
            concurrency_batch_count: plan.concurrency_batches.len(),
        }
    }

    /// A readable multi-line dump, in `ExecutionPlan::order` for a caller
    /// that has it — this type does not keep the order itself, since a
    /// culled pass has none, so [`Self::describe`] just lists passes as
    /// they were declared. A devtools UI wanting execution order should read
    /// [`ExecutionPlan::order`] directly.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{} pass(es), {} culled, {} declared resource(s) aliased into {} physical slot(s), {} concurrency batch(es)\n",
            self.passes.len(),
            self.culled_count,
            self.declared_resource_count,
            self.physical_slot_count,
            self.concurrency_batch_count,
        ));
        for pass in &self.passes {
            out.push_str(&format!(
                "  [{}] {} ({}){}",
                pass.id.index(),
                pass.name,
                pass.kind_name,
                if pass.culled { " CULLED" } else { "" },
            ));
            if let Some(batch) = pass.concurrency_batch {
                out.push_str(&format!(" batch={batch}"));
            }
            out.push('\n');
            if !pass.reads.is_empty() {
                out.push_str(&format!("      reads:  {}\n", pass.reads.join(", ")));
            }
            if !pass.writes.is_empty() {
                out.push_str(&format!("      writes: {}\n", pass.writes.join(", ")));
            }
        }
        out
    }
}

/// [`vieww_render_graph::PassKind`] has no [`std::fmt::Display`] of its own
/// (it is a backend-classification enum, not a report-facing one), so this
/// module supplies the name it wants for a report rather than adding a
/// dependency's public API surface for one caller.
fn pass_kind_name(kind: vieww_render_graph::PassKind) -> &'static str {
    use vieww_render_graph::PassKind;
    match kind {
        PassKind::Raster => "raster",
        PassKind::Filter => "filter",
        PassKind::Composite => "composite",
        PassKind::Blit => "blit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_render_graph::{PassDesc, PassKind, ResourceDesc};
    use vieww_scene::CostHint;

    fn cost() -> CostHint {
        CostHint::CONSERVATIVE
    }

    /// A small real graph, built with `vieww-render-graph`'s own
    /// constructors: two independent raster passes feeding one composite,
    /// plus a dead pass nothing reads.
    fn sample_graph() -> (Graph, ExecutionPlan) {
        let mut g = Graph::new();
        let a = g.add_resource(ResourceDesc::color_target(64, 64, "layer_a"));
        let b = g.add_resource(ResourceDesc::color_target(64, 64, "layer_b"));
        let dead = g.add_resource(ResourceDesc::color_target(64, 64, "dead_target"));
        let screen = g.add_resource(ResourceDesc::presentable(64, 64, "screen"));

        g.add_pass(PassDesc::new("draw_a", PassKind::Raster, cost()).writing(a));
        g.add_pass(PassDesc::new("draw_b", PassKind::Raster, cost()).writing(b));
        g.add_pass(PassDesc::new("dead_pass", PassKind::Raster, cost()).writing(dead));
        g.add_pass(
            PassDesc::new("composite", PassKind::Composite, cost())
                .reading(a)
                .reading(b)
                .writing(screen),
        );

        let plan = g.compile(screen).expect("valid graph");
        (g, plan)
    }

    #[test]
    fn reports_every_declared_pass_with_its_resource_names() {
        let (graph, plan) = sample_graph();
        let report = RenderGraphReport::build(&graph, &plan);

        assert_eq!(
            report.passes.len(),
            4,
            "all declared passes are reported, culled or not"
        );

        let composite = report
            .passes
            .iter()
            .find(|p| p.name == "composite")
            .expect("composite pass present");
        assert_eq!(composite.reads, vec!["layer_a", "layer_b"]);
        assert_eq!(composite.writes, vec!["screen"]);
        assert!(!composite.culled);
    }

    #[test]
    fn a_dead_pass_is_reported_as_culled() {
        let (graph, plan) = sample_graph();
        let report = RenderGraphReport::build(&graph, &plan);

        let dead = report
            .passes
            .iter()
            .find(|p| p.name == "dead_pass")
            .expect("dead pass still reported, just marked");
        assert!(dead.culled);
        assert_eq!(
            dead.concurrency_batch, None,
            "a culled pass runs in no batch"
        );
        assert_eq!(report.culled_count, 1);
    }

    #[test]
    fn independent_passes_share_a_concurrency_batch() {
        let (graph, plan) = sample_graph();
        let report = RenderGraphReport::build(&graph, &plan);

        let batch_a = report
            .passes
            .iter()
            .find(|p| p.name == "draw_a")
            .unwrap()
            .concurrency_batch;
        let batch_b = report
            .passes
            .iter()
            .find(|p| p.name == "draw_b")
            .unwrap()
            .concurrency_batch;
        assert_eq!(batch_a, Some(0));
        assert_eq!(batch_b, Some(0));
        assert_eq!(
            report.concurrency_batch_count, 2,
            "draw batch, then composite batch"
        );
    }

    #[test]
    fn resource_and_slot_counts_match_the_plan() {
        let (graph, plan) = sample_graph();
        let report = RenderGraphReport::build(&graph, &plan);

        assert_eq!(report.declared_resource_count, 4);
        assert_eq!(report.physical_slot_count, plan.physical_slot_count);
    }

    #[test]
    fn describe_mentions_every_pass_name_and_the_culled_marker() {
        let (graph, plan) = sample_graph();
        let report = RenderGraphReport::build(&graph, &plan);
        let text = report.describe();

        for name in ["draw_a", "draw_b", "dead_pass", "composite"] {
            assert!(text.contains(name), "missing {name} in:\n{text}");
        }
        assert!(text.contains("CULLED"), "missing CULLED marker in:\n{text}");
    }
}
