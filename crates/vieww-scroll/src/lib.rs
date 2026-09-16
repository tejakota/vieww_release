//! Scrolling, gathered into one place.
//!
//! The physics ([`vieww_gestures::ScrollPosition`], `ScrollPhysics`), the
//! element-layer controller (`vieww_element::ScrollController`) and virtualised
//! lists (`vieww_widget::ListView`) already exist and are not duplicated here —
//! see each's own module docs. What this crate adds is what did not exist
//! anywhere in the workspace yet:
//!
//! - [`scrollbar`] — a real, draggable, proportionally-sized [`Scrollbar`]
//!   widget with auto-computed thumb geometry, built as a controlled widget
//!   in the same spirit as `vieww_widget::Scrollable`.
//! - [`nested`] — [`NestedScroll`], pure arithmetic that splits one drag
//!   delta between an inner and an outer [`vieww_gestures::ScrollPosition`],
//!   for a sub-list inside a page or a collapsing header above one.

pub mod nested;
pub mod scrollbar;

pub use nested::{NestedScroll, NestedScrollOrder};
pub use scrollbar::{Scrollbar, ThumbGeometry};
