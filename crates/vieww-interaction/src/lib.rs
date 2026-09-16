//! Input the render tree does not already own.
//!
//! `vieww-render`'s own `pointer` and `focus` modules already answer "which
//! render object does this pointer or key event belong to" — hit-tested
//! routing, hover enter/leave, focus bubbling and scopes are real and live
//! there, because answering them needs the tree. This crate is deliberately
//! smaller and sits beside `vieww-gestures` rather than inside `vieww-render`:
//! everything here is pure logic over events that a caller already has, with
//! no tree, no frame and no window in sight — the same argument
//! `vieww-gestures`'s own module doc makes for gesture recognition.
//!
//! - [`shortcut`] — keyboard accelerator matching: does a [`KeyEvent`](vieww_foundation::KeyEvent) mean
//!   exactly this key combination.
//! - [`command`] — a named-action registry built on it: one enabled flag and
//!   one action per command, so a menu item, a toolbar button and an
//!   accelerator all point at the same thing instead of each holding its own
//!   copy.
//! - [`drag_out`] — the portable half of dragging content *out* of the app
//!   to the OS or another application; see its own doc for exactly what is
//!   real today versus honestly deferred behind a platform API this
//!   framework's one windowing backend does not yet expose.
//!
//! [`KeyEvent`]: vieww_foundation::KeyEvent

pub mod command;
pub mod drag_out;
pub mod shortcut;

pub use command::{CommandError, CommandId, CommandRegistry};
pub use drag_out::{
    DragContent, DragError, DragSession, DragSessionState, DropEffect, NullDragStarter,
    OutgoingDrag, PlatformDragStarter,
};
pub use shortcut::Shortcut;
