//! The render layer — the third of `vieww`'s three trees, and the one that
//! decides where things actually go.
//!
//! Widgets describe, elements persist, render objects **measure and place**.
//!
//! **Not to be confused with `vieww_paint::native`**, one layer down. This
//! crate is the layout/paint *tree* (layout, hit testing,
//! `RenderObject::paint`) and sits *above* `vieww-paint`, which it calls
//! into to record a [`vieww_paint::Scene`]. `vieww_paint::native` is
//! the pixel-producing *backend* — `NativeRenderer`, this workspace's one
//! and only rasterizer — and sits *below* `vieww-paint`, rendering the
//! `Scene` this crate built. Neither this crate nor `vieww_paint::native`
//! depends on the other.
//!
//! # Constraints down, sizes up
//!
//! A parent hands each child a [`Constraints`](vieww_foundation::Constraints);
//! the child returns the [`Size`](vieww_foundation::Size) it chose within them;
//! the parent then decides where to put it. A child never learns its own
//! position from layout and never reads its parent. Those two rules make layout
//! a single pass rather than a fixed point, and they are why a child cannot be
//! "as wide as its parent" by asking — the parent has to say so with tight
//! constraints.
//!
//! # What makes it cheap
//!
//! [`RenderTree::layout`] skips a subtree outright when the same constraints
//! come back down and nothing below has been marked pending, and
//! [`RenderTree::mark_needs_layout`] stops propagating upward at the first
//! ancestor laid out under tight constraints — that ancestor's size cannot
//! change however its descendants shuffle, so its parent has nothing to
//! recompute. Together those are the relayout boundary, and they are the
//! difference between a text edit costing one box and costing a screen.
//!
//! ```
//! use vieww_element::ElementTree;
//! use vieww_foundation::{Constraints, Size};
//! use vieww_render::RenderOwner;
//! use vieww_widget::prelude::*;
//!
//! let mut elements = ElementTree::new();
//! let mut owner = RenderOwner::new();
//!
//! elements.mount(Padding::all(10.0).child(SizedBox::square(30.0)));
//!
//! let size = owner.draw_frame(&mut elements, Constraints::loose(Size::new(200.0, 200.0)));
//! assert_eq!(size, Size::new(50.0, 50.0)); // 30 + 10 + 10 on each axis
//! ```

pub mod baseline;
mod children;
mod event_loop;
mod factory;
mod focus;
mod frame;
mod id;
mod intrinsics;
mod object;
pub mod objects;
pub mod overflow;
mod owner;
mod pointer;
mod semantics;
pub mod sliver;
mod tree;
pub mod window;

pub use baseline::BaselineCtx;
pub use children::ChildIds;
pub use event_loop::{next_action, wake_if_pending, LoopAction, LoopHarness};
pub use factory::RenderFactory;
pub use focus::FocusManager;
pub use frame::{FrameDriver, FrameError, FramePhase, InvalidationKind};
pub use id::RenderId;
pub use intrinsics::{largest, sum, Extremum, IntrinsicCtx, IntrinsicQuery};
pub use object::{
    layout_differs_by_eq, HitTestEntry, HitTestResult, LayoutCtx, PaintCtx, RenderObject,
};
pub use objects::*;
pub use owner::{
    set_render_panic_sink, set_unregistered_render_object_sink, DiagnosticSink, RenderOwner,
};
pub use pointer::{Dispatched, PointerRouter};
pub use semantics::{
    Announcement, Liveness, Role, SemanticAction, Semantics, SemanticsNode, SemanticsTree,
};
pub use sliver::{visible_extent, ScrollDirection, SliverConstraints, SliverGeometry};
pub use tree::{NodeDescription, RenderTree};
pub use window::{
    RecordingWindows, WindowBuild, WindowKey, WindowRole, WindowSet, WindowSpec, Windows,
    WindowsExt,
};

// Re-exported so that painting a tree needs only this crate in scope. The
// canvas, the recorder and the layer model all belong to the paint layer, which
// sits below this one.
pub use vieww_paint::{
    Canvas, Command, DamageCullStats, Image, LayerEffect, LayerTree, Paint, Path, Scene,
};

pub use vieww_element as element;
pub use vieww_foundation as foundation;
pub use vieww_gestures as gestures;
pub use vieww_paint as paint;
pub use vieww_widget as widget;
