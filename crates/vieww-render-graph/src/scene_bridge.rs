//! Building a [`Graph`] from a [`vieww_scene::SceneGraph`].
//!
//! This is where a scene's tree structure becomes an explicit data-flow
//! graph: every [`vieww_scene::LayerNode`] becomes a raster pass (its own
//! direct draw children), zero or more composite passes (one per nested
//! layer, chained — see the module-level note on why chaining rather than
//! multi-writing), and, if it declares a non-trivial filter, a filter pass —
//! matching `vieww_paint::Command::PushLayer`'s own doc: the filter runs on
//! "the group's rasterised pixels ... before compositing."
//!
//! # Why composites chain instead of one pass reading every child
//!
//! [`Graph`]'s single-writer invariant means a layer's accumulator resource
//! cannot be written by N separate composite passes. Chaining
//! (`raster → +child1 → +child2 → ... → filter?`) keeps every pass a
//! two-input operation, which is both what a real compositor primitive
//! actually is (a blend takes a source and a destination, not N sources)
//! and what lets `crate::transient` alias the N-1 intermediate
//! accumulators against each other — they are never live at the same time.

use vieww_foundation::Rect;
use vieww_scene::{LayerNode, SceneGraph, SceneNode};

use crate::graph::Graph;
use crate::pass::{PassDesc, PassKind};
use crate::resource::{ResourceDesc, ResourceId};
use vieww_scene::CostHint;

fn dimensions(bounds: Rect) -> (u32, u32) {
    (
        bounds.width().max(1.0).ceil() as u32,
        bounds.height().max(1.0).ceil() as u32,
    )
}

fn own_draw_cost(children: &[SceneNode]) -> CostHint {
    children
        .iter()
        .filter(|c| matches!(c, SceneNode::Draw(_)))
        .fold(CostHint::CONSERVATIVE, |acc, c| acc.combine(c.cost()))
}

fn build_layer(graph: &mut Graph, layer: &LayerNode) -> ResourceId {
    let (w, h) = dimensions(layer.bounds);
    let mut current = graph.add_resource(ResourceDesc::color_target(w, h, "layer_raster"));
    graph.add_pass(
        PassDesc::new(
            "layer_raster",
            PassKind::Raster,
            own_draw_cost(&layer.children),
        )
        .writing(current),
    );

    for child in &layer.children {
        if let SceneNode::Layer(child_layer) = child {
            let child_output = build_layer(graph, child_layer);
            let next = graph.add_resource(ResourceDesc::color_target(w, h, "layer_composite"));
            graph.add_pass(
                PassDesc::new("composite_child", PassKind::Composite, child_layer.cost)
                    .reading(current)
                    .reading(child_output)
                    .writing(next),
            );
            current = next;
        }
    }

    if !layer.filter.is_noop() {
        let filtered = graph.add_resource(ResourceDesc::color_target(w, h, "layer_filtered"));
        graph.add_pass(
            PassDesc::new("layer_filter", PassKind::Filter, layer.cost)
                .reading(current)
                .writing(filtered),
        );
        current = filtered;
    }

    current
}

/// Build a [`Graph`] for one frame of `scene`, targeting a presentable
/// surface `width`×`height`.
///
/// Returns the graph together with the presentable resource id — pass that
/// straight to [`Graph::compile`].
#[must_use]
pub fn build_graph(scene: &SceneGraph, width: u32, height: u32) -> (Graph, ResourceId) {
    let mut graph = Graph::new();
    let screen = graph.add_resource(ResourceDesc::presentable(width, height, "screen"));

    let mut current = graph.add_resource(ResourceDesc::color_target(width, height, "root_raster"));
    graph.add_pass(
        PassDesc::new("root_raster", PassKind::Raster, own_draw_cost(&scene.roots))
            .writing(current),
    );

    for root in &scene.roots {
        if let SceneNode::Layer(layer) = root {
            let child_output = build_layer(&mut graph, layer);
            let next =
                graph.add_resource(ResourceDesc::color_target(width, height, "root_composite"));
            graph.add_pass(
                PassDesc::new("composite_root_child", PassKind::Composite, layer.cost)
                    .reading(current)
                    .reading(child_output)
                    .writing(next),
            );
            current = next;
        }
    }

    graph.add_pass(
        PassDesc::new("present_blit", PassKind::Blit, CostHint::CONSERVATIVE)
            .reading(current)
            .writing(screen),
    );

    (graph, screen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{BlendMode, Color, ImageFilter, Rect};
    use vieww_paint::{Canvas, Paint, Scene};

    #[test]
    fn a_flat_scene_compiles_to_raster_then_blit() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        let scene_graph = SceneGraph::from_scene(&scene);

        let (graph, screen) = build_graph(&scene_graph, 100, 100);
        let plan = graph.compile(screen).unwrap();
        // root_raster, present_blit — nothing else, since there were no
        // nested layers to composite.
        assert_eq!(plan.order.len(), 2);
    }

    #[test]
    fn a_nested_layer_adds_a_composite_pass() {
        let mut scene = Scene::new();
        scene.push_layer(Rect::new(0.0, 0.0, 50.0, 50.0), 0.5, BlendMode::Normal);
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        scene.pop_layer();
        let scene_graph = SceneGraph::from_scene(&scene);

        let (graph, screen) = build_graph(&scene_graph, 100, 100);
        let plan = graph.compile(screen).unwrap();
        // root_raster, layer_raster, composite_root_child, present_blit.
        assert_eq!(plan.order.len(), 4);
    }

    #[test]
    fn a_non_trivial_filter_adds_a_filter_pass() {
        let mut scene = Scene::new();
        scene.push_filtered_layer(
            Rect::new(0.0, 0.0, 50.0, 50.0),
            1.0,
            BlendMode::Normal,
            ImageFilter::blur(4.0),
        );
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        scene.pop_layer();
        let scene_graph = SceneGraph::from_scene(&scene);

        let (graph, screen) = build_graph(&scene_graph, 100, 100);
        let plan = graph.compile(screen).unwrap();
        assert!(plan
            .order
            .iter()
            .any(|p| graph.pass(*p).kind == PassKind::Filter));
    }
}
