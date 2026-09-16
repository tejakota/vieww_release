//! Building a [`SceneGraph`] from a recorded [`vieww_paint::Scene`].
//!
//! This is the one and only bridge between the paint layer's flat,
//! push/pop-marked command list and the tree [`crate::graph`] and
//! `vieww-render-planner` actually want to reason over. It is a pure,
//! allocating walk — no rasterization, no I/O — so it is exactly as cheap
//! to test as any other tree-building code, which is what the tests below
//! lean on.

use vieww_paint::{Command, Scene};

use crate::cost::CostHint;
use crate::node::{DrawNode, LayerNode, Primitive, SceneNode};

impl SceneGraph {
    /// Build a scene graph from every command in `scene`, in order.
    ///
    /// # Panics
    ///
    /// Never, on any input `Scene` produces — `Scene`'s own invariant
    /// (`push_command`'s doc) is that `PushLayer`/`PopLayer` are always
    /// balanced, and this walk trusts that invariant the same way
    /// `vieww_paint::graph::PassGraph::build` does. An unbalanced stream
    /// (only reachable by hand-assembling commands, not by anything `Canvas`
    /// emits) closes any still-open layers at the top level rather than
    /// panicking — see `from_commands` for the precise rule, matched by
    /// `unbalanced_streams_close_open_layers_rather_than_panicking` below.
    #[must_use]
    pub fn from_scene(scene: &Scene) -> Self {
        Self::from_commands(scene.commands())
    }

    /// The same walk, taking a raw command slice — what
    /// `Scene::damage_cull`'s output and any other in-memory command list
    /// can feed directly.
    #[must_use]
    pub fn from_commands(commands: &[Command]) -> Self {
        // A stack of "currently open layer, with its children so far."
        // Closing one appends the finished `LayerNode` to whatever list is
        // now on top (the next open layer, or `roots` once the stack is
        // empty).
        let mut stack: Vec<(&Command, Vec<SceneNode>)> = Vec::new();
        let mut roots: Vec<SceneNode> = Vec::new();

        for command in commands {
            match command {
                Command::PushLayer { .. } => {
                    stack.push((command, Vec::new()));
                }
                Command::PopLayer => {
                    let Some((open, children)) = stack.pop() else {
                        // Unbalanced: a `PopLayer` with nothing open. Ignore
                        // it rather than panic — see this fn's doc.
                        continue;
                    };
                    let Command::PushLayer {
                        bounds,
                        alpha,
                        blend,
                        clip,
                        filter,
                    } = open
                    else {
                        unreachable!("stack only ever holds PushLayer commands")
                    };
                    let child_cost = children
                        .iter()
                        .fold(CostHint::CONSERVATIVE, |acc, c| acc.combine(c.cost()));
                    let node = SceneNode::Layer(LayerNode {
                        bounds: *bounds,
                        alpha: *alpha,
                        blend: *blend,
                        filter: *filter,
                        clip: clip.clone(),
                        cost: CostHint::classify_command(open).combine(child_cost),
                        children,
                    });
                    match stack.last_mut() {
                        Some((_, parent_children)) => parent_children.push(node),
                        None => roots.push(node),
                    }
                }
                draw => {
                    let node = SceneNode::Draw(DrawNode {
                        primitive: primitive_of(draw),
                        transform: draw.transform(),
                        clip: draw.clip().clone(),
                        cost: CostHint::classify_command(draw),
                    });
                    match stack.last_mut() {
                        Some((_, children)) => children.push(node),
                        None => roots.push(node),
                    }
                }
            }
        }

        // Unbalanced tail: any layers still open at the end are closed at
        // the top level, innermost first, rather than dropped — dropping
        // would silently lose every command already recorded inside them.
        while let Some((open, children)) = stack.pop() {
            let Command::PushLayer {
                bounds,
                alpha,
                blend,
                clip,
                filter,
            } = open
            else {
                unreachable!("stack only ever holds PushLayer commands")
            };
            let child_cost = children
                .iter()
                .fold(CostHint::CONSERVATIVE, |acc, c| acc.combine(c.cost()));
            roots.push(SceneNode::Layer(LayerNode {
                bounds: *bounds,
                alpha: *alpha,
                blend: *blend,
                filter: *filter,
                clip: clip.clone(),
                cost: CostHint::classify_command(open).combine(child_cost),
                children,
            }));
        }

        Self { roots }
    }
}

fn primitive_of(command: &Command) -> Primitive {
    match command {
        Command::FillRect { rect, paint, .. } => Primitive::Rect {
            rect: *rect,
            paint: *paint,
        },
        Command::FillPath { path, paint, .. } => Primitive::Path {
            path: path.clone(),
            paint: *paint,
        },
        Command::StrokePath {
            path,
            stroke,
            paint,
            ..
        } => Primitive::Stroke {
            path: path.clone(),
            stroke: stroke.clone(),
            paint: *paint,
        },
        Command::DrawShadow {
            rect,
            radius,
            shadow,
            ..
        } => Primitive::Shadow {
            rect: *rect,
            radius: *radius,
            shadow: *shadow,
        },
        Command::DrawGlyphs { run, .. } => Primitive::Glyphs { run: run.clone() },
        Command::DrawImage { rect, image, .. } => Primitive::Image {
            rect: *rect,
            image: image.clone(),
        },
        Command::PushLayer { .. } | Command::PopLayer => {
            unreachable!("primitive_of is only called on draw commands")
        }
    }
}

use crate::node::SceneGraph;

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{Color, Rect};
    use vieww_paint::{Canvas, Paint, Scene};

    #[test]
    fn a_flat_scene_becomes_a_flat_forest() {
        let mut scene = Scene::new();
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        scene.fill_rect(Rect::new(0.0, 0.0, 20.0, 20.0), Paint::solid(Color::BLUE));

        let graph = SceneGraph::from_scene(&scene);
        assert_eq!(graph.roots.len(), 2);
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn a_layer_nests_its_contents() {
        let mut scene = Scene::new();
        scene.push_layer(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            1.0,
            vieww_foundation::BlendMode::Normal,
        );
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        scene.pop_layer();

        let graph = SceneGraph::from_scene(&scene);
        assert_eq!(graph.roots.len(), 1);
        let SceneNode::Layer(layer) = &graph.roots[0] else {
            panic!("expected a layer");
        };
        assert_eq!(layer.children.len(), 1);
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn unbalanced_streams_close_open_layers_rather_than_panicking() {
        let mut scene = Scene::new();
        scene.push_layer(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            1.0,
            vieww_foundation::BlendMode::Normal,
        );
        scene.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::RED));
        // No matching `pop_layer` — build directly from the raw commands so
        // the "unbalanced" case can exist at all (`Scene`'s own public API
        // cannot produce one, per its module doc).
        let commands = scene.commands().to_vec();

        let graph = SceneGraph::from_commands(&commands);
        assert_eq!(graph.roots.len(), 1, "the open layer should still surface");
        let SceneNode::Layer(layer) = &graph.roots[0] else {
            panic!("expected a layer");
        };
        assert_eq!(layer.children.len(), 1);
    }

    #[test]
    fn total_cost_rolls_up_children() {
        let mut scene = Scene::new();
        scene.push_layer(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            1.0,
            vieww_foundation::BlendMode::Normal,
        );
        scene.draw_shadow(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            20.0,
            vieww_foundation::Shadow {
                color: Color::BLACK,
                offset: vieww_foundation::Offset::ZERO,
                blur: 20.0,
                spread: 0.0,
                is_inset: false,
            },
        );
        scene.pop_layer();

        let graph = SceneGraph::from_scene(&scene);
        assert_eq!(
            graph.total_cost().gpu_affinity,
            crate::cost::Affinity::GpuOnly
        );
    }
}
