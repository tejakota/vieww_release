//! The element layer — the second of `vieww`'s three trees, and the one that
//! makes the other two affordable.
//!
//! A widget is a description that lives for one frame. A render object lays out
//! and paints. Between them sits the element: the thing that **persists**.
//! Rebuilding a widget tree is cheap precisely because the element tree
//! underneath it is reconciled rather than recreated, so identity, state and
//! signal subscriptions survive.
//!
//! Three pieces:
//!
//! - [`ElementTree`] — the arena, reconciliation, and the rebuild scheduler.
//! - [`Signal`] / [`Runtime`] — fine-grained reactivity. Reading a signal during
//!   a build subscribes that element; writing marks exactly those elements
//!   pending and nothing else.
//! - [`Element`] — one persistent node, with its widget, children, depth, state
//!   and pending flag.
//! - [`Animation`] — a value that moves over time and publishes itself into a
//!   signal, so a frame of animation rebuilds exactly what displays it.
//!
//! ```
//! use vieww_element::ElementTree;
//! use vieww_widget::prelude::*;
//!
//! let mut tree = ElementTree::new();
//! let name = tree.runtime().signal(String::from("Ada"));
//!
//! // A widget that reads the signal during its build subscribes to it.
//! # let shown = name.clone();
//! tree.mount(Text::new(name.peek()));
//!
//! // A write only *schedules*; nothing rebuilds until the tree is asked to.
//! shown.set(String::from("Grace"));
//! let rebuilt = tree.rebuild_pending();
//! # let _ = rebuilt;
//! ```
//!
//! See `docs/DESIGN.md` §1 for why this is signals-on-the-element rather than
//! positional hooks.

mod animation;
mod drag;
mod element;
mod error;
pub mod hero;
mod id;
mod inspect;
mod navigator;
mod scroll;
pub mod scroll_anim;
pub mod shared_element;
mod signal;
pub mod transition;
mod tree;

pub use animation::Animation;
pub use drag::{DragController, DragTargetId, Dropped};
pub use element::Element;
pub use error::{BuildError, ErrorPolicy};
pub use id::ElementId;
pub use inspect::Hotspot;
pub use navigator::NavigatorController;
pub use scroll::ScrollController;
pub use signal::{Memo, Runtime, Signal, SignalId};
pub use tree::ElementTree;

pub use vieww_animation as animation_layer;
pub use vieww_foundation as foundation;
pub use vieww_widget as widget;

// Patch 5 additions
pub mod physics_drag;
pub mod timeline;

pub use hero::{HeroController, HeroState};
pub use physics_drag::PhysicsDrag;
pub use scroll_anim::{ScrollTimeline, ScrollTrigger, ScrollTriggerEvent, ScrollTriggerManager};
pub use shared_element::{
    Flight, SharedElement, SharedFlight, SharedGeometry, SharedRegistry, SharedTag,
};
pub use timeline::{
    Action, Callback, Step, Timeline, TimelineBuilder, TimelineHandle, TimelinePlayer,
};
