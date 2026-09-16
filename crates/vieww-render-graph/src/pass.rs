//! Passes: one unit of work in a [`crate::graph::Graph`].
//!
//! A pass says what it reads and writes and nothing about how it runs — that
//! separation is what lets [`crate::graph::Graph::compile`] reorder,
//! parallelize, or drop a pass entirely without the pass itself changing.

use vieww_scene::CostHint;

use crate::resource::ResourceId;

/// Identifies one pass within a single [`crate::graph::Graph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PassId(pub(crate) u32);

impl PassId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What kind of work a pass performs — informs a backend's choice of
/// pipeline, and the planner's choice of executor
/// (`vieww-render-planner::cost`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    /// Rasterize geometry (fills, strokes, glyphs, images) into a target.
    Raster,
    /// A screen-space image filter reading one or more inputs and writing
    /// one output (blur, color matrix, backdrop composite).
    Filter,
    /// Composite one or more inputs onto a target with a blend mode and
    /// opacity — what closes a `PushLayer`/`PopLayer` pair.
    Composite,
    /// A copy/blit with no shading (e.g. resolving a target onto the
    /// presentable surface).
    Blit,
}

/// A pass declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct PassDesc {
    pub name: &'static str,
    pub kind: PassKind,
    pub reads: Vec<ResourceId>,
    pub writes: Vec<ResourceId>,
    pub cost: CostHint,
}

impl PassDesc {
    #[must_use]
    pub fn new(name: &'static str, kind: PassKind, cost: CostHint) -> Self {
        Self {
            name,
            kind,
            reads: Vec::new(),
            writes: Vec::new(),
            cost,
        }
    }

    #[must_use]
    pub fn reading(mut self, resource: ResourceId) -> Self {
        self.reads.push(resource);
        self
    }

    #[must_use]
    pub fn writing(mut self, resource: ResourceId) -> Self {
        self.writes.push(resource);
        self
    }
}
