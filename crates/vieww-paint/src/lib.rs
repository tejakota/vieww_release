//! The paint layer — how a laid-out tree becomes pixels.
//!
//! Layout answers *where*; this crate answers *what colour*. It sits **below**
//! the render layer, not above it: `RenderObject::paint` draws onto a
//! [`Canvas`], so `vieww-render` depends on `vieww-paint`, the same
//! shape any layered toolkit has: rendering below paint.
//!
//! # The pieces
//!
//! - [`Canvas`] is the drawing interface render objects see. It is deliberately
//!   Skia-shaped, because that model maps onto every backend worth having.
//! - [`Scene`] is a `Canvas` that records instead of drawing, resolving the
//!   transform and clip into absolute coordinates as each command arrives. That
//!   makes every command independently inspectable and self-bounding.
//! - [`LayerTree`] holds the repaint boundaries: subtrees that keep their own
//!   recording, so a repaint stops at a boundary instead of running to the root.
//! - [`Damage`] accumulates what changed, so a frame can redraw a corner of the
//!   screen rather than all of it.
//! - [`FrameScheduler`] decides *when* a frame runs and in what order its phases
//!   go, driving the layers above through the [`FrameSink`] trait — it cannot
//!   name them, since they are above it.
//!
//! Layers, damage and the scheduler together are what make a change cost
//! something proportional to the change rather than to the size of the screen.
//!
//! # No graphics stack by default
//!
//! Everything above is pure computation over `f32` and needs no graphics
//! stack at all. The one renderer this crate ships, [`native`], lives behind
//! the optional `native` feature, which is off by default, so the
//! workspace — and the Android and iOS cross-builds in particular —
//! compiles in seconds. Applications turn it on.
//!
//! # vello is gone
//!
//! Earlier builds of this crate shipped three vello-based backends —
//! `gpu` (`vello` + `wgpu`), `hybrid` (`vello_hybrid`) and `cpu`
//! (`vello_cpu`) — none of which exist any more. [`native`] replaced all
//! three: it is the CPU reference rasterizer that used to sit beside them
//! as an oracle, now the only renderer, presented through `vieww-hal`'s raw
//! Vulkan swapchain (see the `vieww-platform-winit` crate's `native`
//! module) rather than through `wgpu`. See `docs/RENDERER-MIGRATION.md`
//! for the full account.
//!
//! ```
//! use vieww_foundation::{Color, Offset, Rect};
//! use vieww_paint::{Canvas, Scene};
//!
//! let mut scene = Scene::new();
//! scene.save();
//! scene.translate(Offset::new(20.0, 10.0));
//! scene.fill_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Color::RED.into());
//! scene.restore();
//!
//! // The command carries absolute bounds, so a backend needs no state stack.
//! assert_eq!(scene.fills()[0].0, Rect::new(20.0, 10.0, 120.0, 60.0));
//! ```

mod canvas;
mod damage;
mod flatten;
/// An explicit dependency graph over a [`Scene`]'s `PushLayer`/`PopLayer`
/// groups — "Renderer v2" pillar A. See the module's own docs for what it
/// is, how it is validated against [`Scene::damage_cull`], and what it
/// deliberately does not do.
pub mod graph;
mod layer;
mod scene;
mod scheduler;

/// Counting allocations, for checklist item 7. Test builds only — see the
/// module's own docs for what a number from it does and does not mean.
#[cfg(test)]
mod counting;

/// The allocator this crate's **test binary** uses.
///
/// Not a shipped choice: `#[cfg(test)]`, so a build of `vieww-paint` for an
/// application has the system allocator with no wrapper, no counter and no
/// atomics. It is here because checklist item 7 is entirely about allocation
/// and nothing in the suite could see one.
#[cfg(test)]
#[global_allocator]
static ALLOCATOR: counting::Counting = counting::Counting;

/// vieww's own ground-up rasterizer, presented through `vieww-hal`'s raw
/// Vulkan swapchain. Requires the `native` feature.
///
/// A retained-tessellation-free, CPU reference rasteriser that implements
/// all 28 [`BlendMode`] variants natively via isolated-layer compositing —
/// see this module's own docs for the full account of what ships and what
/// is still ahead, and `docs/RENDERER-MIGRATION.md` for how it replaced
/// vello.
///
/// Lives inside this crate rather than in its own, because it needs this
/// crate's own [`Scene`]/[`Command`] as a regular (non-optional) dependency
/// to do its job at all, which made a separate crate impossible to also
/// depend on optionally from here — Cargo rejects that cycle outright,
/// feature-gating or not. See this module's own docs for the full account.
#[cfg(feature = "native")]
pub mod native;

pub use canvas::{Canvas, Image, Paint, Stroke};
pub use damage::{Damage, AA_BLEED, MAX_REGIONS, REPAINT_ALL_THRESHOLD};
pub use flatten::{FlattenStats, SceneFlattener};
pub use graph::{Pass, PassGraph, PassId, PassLayer};
pub use layer::{Layer, LayerEffect, LayerId, LayerTree};
pub use scene::{Clip, Command, DamageCullStats, Scene};
pub use scheduler::{FrameInfo, FramePhase, FrameScheduler, FrameSink, FrameStats};
pub use vieww_foundation::{Dash, StrokeCap, StrokeJoin, StrokeStyle};

// A path is a *shape*, and both sides of the framework need to name one: the
// widget layer to describe an icon, this layer to fill it. So it lives in
// foundation, and is re-exported here because a backend that fills paths should
// not have to know which crate the geometry came from.
//
// Gradients, shadows and blend modes are here for exactly the same reason: a
// widget describes one, this layer paints it, and neither may depend on the
// other.
pub use vieww_foundation::{BlendMode, Gradient, GradientStop, Path, PathVerb, Shadow};

pub use vieww_foundation as foundation;
