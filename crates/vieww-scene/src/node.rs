//! The scene tree itself: [`SceneNode`], [`DrawNode`], [`LayerNode`].
//!
//! Every geometric fact here (transform, clip, bounds) is already resolved
//! to absolute coordinates — inherited from [`vieww_paint::Scene`], whose own
//! docs explain why that is the right place to resolve it. What this module
//! adds is *structure* (a real tree instead of a flat command list with
//! matching push/pop markers) and *cost* (a [`crate::cost::CostHint`] on
//! every node), which a flat command list has no room to carry.

use vieww_foundation::{BlendMode, GlyphRun, ImageFilter, Rect, Shadow, Transform};
use vieww_paint::{Clip, Image, Paint, Path, Stroke};

use crate::cost::CostHint;

/// One drawable primitive, geometry already resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Rect {
        rect: Rect,
        paint: Paint,
    },
    Path {
        path: Path,
        paint: Paint,
    },
    Stroke {
        path: Path,
        stroke: Stroke,
        paint: Paint,
    },
    Shadow {
        rect: Rect,
        radius: f32,
        shadow: Shadow,
    },
    Glyphs {
        run: GlyphRun,
    },
    Image {
        rect: Rect,
        image: Image,
    },
}

impl Primitive {
    /// The primitive's own bounds, in the space `transform` maps out of.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Rect { rect, .. } | Self::Shadow { rect, .. } | Self::Image { rect, .. } => *rect,
            Self::Path { path, .. } | Self::Stroke { path, .. } => path.bounds(),
            Self::Glyphs { run } => {
                // A run's own bounds are the union of each glyph's advance
                // box, offset from the run's origin. Coarse (ignores glyph
                // ink bounds, which need face metrics this crate does not
                // have) but never an underestimate, which is what a damage
                // or culling consumer needs.
                let mut max_x = 0.0f32;
                for g in run.glyphs.iter() {
                    // `Glyph` carries no per-glyph advance (its origin is the
                    // authoritative position, already spaced by whatever
                    // shaped it), so a run's width is bounded by its last
                    // glyph's offset plus one em as a safe overestimate.
                    max_x = max_x.max(g.offset.dx + run.size);
                }
                Rect::new(
                    run.origin.dx,
                    run.origin.dy - run.size,
                    run.origin.dx + max_x,
                    run.origin.dy + run.size * 0.4,
                )
            }
        }
    }
}

/// A single draw call: one primitive under one transform and clip.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawNode {
    pub primitive: Primitive,
    pub transform: Transform,
    pub clip: Clip,
    pub cost: CostHint,
}

impl DrawNode {
    /// The node's bounds in the coordinate space its transform maps *into*
    /// (i.e. already transformed).
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.transform.apply_rect(self.primitive.bounds())
    }
}

/// An isolated group: rasterized separately, then composited onto its
/// parent — the scene-graph analogue of `vieww_paint::Command::PushLayer`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerNode {
    pub bounds: Rect,
    pub alpha: f32,
    pub blend: BlendMode,
    pub filter: ImageFilter,
    pub clip: Clip,
    pub children: Vec<SceneNode>,
    pub cost: CostHint,
}

impl LayerNode {
    /// Whether this layer is a pure grouping with no visual effect of its
    /// own — full opacity, normal blend, no filter. A backend can flatten a
    /// trivial layer into its parent without changing a single pixel.
    #[must_use]
    pub fn is_trivial(&self) -> bool {
        (self.alpha - 1.0).abs() < f32::EPSILON
            && self.blend == BlendMode::Normal
            && self.filter.is_noop()
    }
}

/// One node in the scene tree: either a draw call or a group of them.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneNode {
    Draw(DrawNode),
    Layer(LayerNode),
}

impl SceneNode {
    #[must_use]
    pub fn cost(&self) -> CostHint {
        match self {
            Self::Draw(d) => d.cost,
            Self::Layer(l) => l.cost,
        }
    }

    #[must_use]
    pub fn bounds(&self) -> Rect {
        match self {
            Self::Draw(d) => d.bounds(),
            Self::Layer(l) => l.bounds,
        }
    }

    /// Count every node in this subtree, including `self`.
    #[must_use]
    pub fn node_count(&self) -> usize {
        match self {
            Self::Draw(_) => 1,
            Self::Layer(l) => 1 + l.children.iter().map(Self::node_count).sum::<usize>(),
        }
    }
}

/// A whole scene: a forest of top-level nodes plus the bounds that enclose
/// all of them.
///
/// A forest rather than a single root because `vieww_paint::Scene`'s
/// top-level command list has no synthetic wrapping layer of its own — see
/// `PassGraph::root` for the same choice made for the same reason.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneGraph {
    pub roots: Vec<SceneNode>,
}

impl SceneGraph {
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.roots
            .iter()
            .map(SceneNode::bounds)
            .fold(Rect::ZERO, union)
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.roots.iter().map(SceneNode::node_count).sum()
    }

    /// Roll every node's [`CostHint`] up into one summary for the whole
    /// scene, via [`CostHint::combine`].
    #[must_use]
    pub fn total_cost(&self) -> CostHint {
        fn walk(node: &SceneNode, acc: CostHint) -> CostHint {
            let acc = acc.combine(node.cost());
            if let SceneNode::Layer(l) = node {
                l.children.iter().fold(acc, |acc, c| walk(c, acc))
            } else {
                acc
            }
        }
        self.roots
            .iter()
            .fold(CostHint::CONSERVATIVE, |acc, n| walk(n, acc))
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    if a == Rect::ZERO {
        return b;
    }
    if b == Rect::ZERO {
        return a;
    }
    Rect::new(
        a.left.min(b.left),
        a.top.min(b.top),
        a.right.max(b.right),
        a.bottom.max(b.bottom),
    )
}
