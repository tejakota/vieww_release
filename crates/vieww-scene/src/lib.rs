//! Renderer-independent scene IR.
//!
//! `docs/RENDERER-V2-NOTES.md`'s "Architecture" section calls for treating
//! the render graph "as a first-class component, not an implementation
//! detail" and reframes the target question as "what is the cheapest
//! possible execution plan for this frame," not "how do I execute these
//! commands." [`vieww_paint::graph::PassGraph`] answers the first half of
//! that for the one renderer this workspace has today (a CPU scanline
//! rasterizer, scoped honestly in that module's own docs). This crate is
//! the next step: a scene representation that does not assume any one
//! renderer at all, so a render graph, a CPU/GPU/hybrid planner, and a real
//! GPU backend can all be built against the same tree instead of each
//! reaching back into `vieww_paint::Scene`'s command list on its own terms.
//!
//! # What is here and what is not
//!
//! [`SceneGraph`] is a tree over [`vieww_paint::Command`]'s same primitives
//! (rects, paths, strokes, shadows, glyph runs, images, layers) —
//! deliberately not a new set of drawing primitives, since duplicating
//! those would be the "isolated feature" failure mode `docs/AIMS.md`
//! warns about elsewhere in this workspace. What is new is *structure*: a
//! real tree instead of a flat list with matching push/pop markers
//! (`node`), and *cost* (`cost`) — a heuristic classification of how
//! dynamic, cacheable, and GPU-suited each node is, which a flat command
//! list has nowhere to put.
//!
//! [`SceneGraph::from_scene`] is the one bridge from today's paint layer
//! into this IR. Nothing here rasterizes anything; that is
//! `vieww-render-graph` and whatever backend it compiles a plan for.

mod build;
mod cost;
mod node;

pub use cost::{Affinity, Cacheability, CostHint, Dynamicity};
pub use node::{DrawNode, LayerNode, Primitive, SceneGraph, SceneNode};
