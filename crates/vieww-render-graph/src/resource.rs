//! Graph resources: the buffers and textures passes read and write.
//!
//! A resource here is a *declaration*, not an allocation — `vieww-render-graph`
//! decides which declarations can share physical memory (`crate::transient`)
//! and in what order barriers are needed (`crate::schedule`); it never
//! allocates anything itself, since that is a backend's job (`vieww-gpu` and
//! its Vulkan/Metal/D3D12 implementations).

/// Identifies one resource within a single [`crate::graph::Graph`].
///
/// Not `Ord`/`Hash`-derived from a pointer or a name — it is the index the
/// resource was declared at, so two graphs never confuse each other's ids and
/// a resource can be looked up in O(1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceId(pub(crate) u32);

impl ResourceId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// What kind of resource this is, for aliasing compatibility and future
/// backend allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// An offscreen color target — what a `PushLayer` rasterizes into.
    ColorTarget,
    /// A depth/stencil target.
    DepthTarget,
    /// A linear buffer (vertex/index/uniform data, an instance buffer).
    Buffer,
    /// The final presentable surface. Exactly one per graph, never aliased,
    /// never culled if reachable.
    Presentable,
}

/// A resource declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceDesc {
    pub kind: ResourceKind,
    /// Pixel dimensions for a target; byte length for a buffer. Zero for a
    /// dimension that does not apply (a `Buffer`'s `height`).
    pub width: u32,
    pub height: u32,
    pub debug_name: &'static str,
}

impl ResourceDesc {
    #[must_use]
    pub const fn color_target(width: u32, height: u32, debug_name: &'static str) -> Self {
        Self {
            kind: ResourceKind::ColorTarget,
            width,
            height,
            debug_name,
        }
    }

    #[must_use]
    pub const fn presentable(width: u32, height: u32, debug_name: &'static str) -> Self {
        Self {
            kind: ResourceKind::Presentable,
            width,
            height,
            debug_name,
        }
    }

    /// Whether two resources are interchangeable for the purpose of sharing
    /// one physical allocation (`crate::transient`).
    ///
    /// Exact match on kind and dimensions only. A backend that wants
    /// "close enough" aliasing (a pool of power-of-two-sized targets, say)
    /// builds that on top of this, rather than this crate silently
    /// resizing something a caller asked for at an exact size.
    #[must_use]
    pub fn aliasable_with(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.kind != ResourceKind::Presentable
            && self.width == other.width
            && self.height == other.height
    }

    /// A rough byte cost, for budget-aware transient allocation.
    #[must_use]
    pub const fn estimated_bytes(&self) -> u64 {
        match self.kind {
            ResourceKind::Buffer => self.width as u64,
            _ => (self.width as u64) * (self.height as u64) * 4,
        }
    }
}
