//! The gesture layer — turning pointer events into gestures, and deciding which
//! gesture won.
//!
//! # The problem this exists for
//!
//! A finger landing on the screen is ambiguous, and stays ambiguous. A touch on a
//! button inside a scrollable list might be a press or the start of a scroll, and
//! nothing about the moment of contact says which. Deciding immediately produces
//! one of two bugs, and which one you get depends only on which layer you let win:
//! a list that will not scroll because every drag begins on a button, or buttons
//! that will not press because the list took the touch.
//!
//! So the decision is deferred. Every recogniser under the pointer joins an
//! [`arena`] for it, and the arena holds the ambiguity until something resolves
//! it — a drag passing its slop threshold and declaring certainty, or the finger
//! lifting with nobody having claimed, in which case the innermost waiting
//! recogniser wins.
//!
//! # Layering
//!
//! This crate depends on [`vieww_foundation`] and nothing else. Recognition is
//! pure logic over pointer events and time, so every recogniser here can be
//! tested by handing it synthetic events, with no tree, no frame and no window in
//! sight. Routing gestures to actual widgets is the render layer's job, since
//! that is what owns the hit test.
//!
//! ```
//! use std::time::Duration;
//! use vieww_foundation::{Offset, PointerEvent, PointerId};
//! use vieww_gestures::{recognize, DragRecognizer, GestureDispatcher, Recognized, TapRecognizer};
//!
//! let mut dispatcher = GestureDispatcher::new();
//! dispatcher.add(TapRecognizer::new());   // innermost: wins a still finger
//! dispatcher.add(DragRecognizer::new());
//!
//! let pointer = PointerId(1);
//! let at = |x: f32| Offset::new(x, 0.0);
//! let gestures = recognize(
//!     &mut dispatcher,
//!     &[
//!         PointerEvent::down(pointer, at(0.0), Duration::from_millis(0)),
//!         PointerEvent::moved(pointer, at(0.0), at(2.0), Duration::from_millis(16)),
//!         PointerEvent::up(pointer, at(2.0), Duration::from_millis(32)),
//!     ],
//! );
//!
//! // Two pixels is inside the touch slop, so it is still a tap — and the press
//! // that preceded it is reported too, so a control has something to highlight
//! // on even when the whole gesture was over in 32ms.
//! assert!(matches!(
//!     gestures.as_slice(),
//!     [Recognized::TapDown(_), Recognized::Tap(_)]
//! ));
//! ```

pub mod arena;
mod dispatch;
mod physics;
mod recognizer;
mod recognizers;
mod velocity;

pub use arena::{ArenaOutcome, GestureArena, GestureDisposition, MemberId};
pub use dispatch::{contested, drive, drive_tick, recognize, GestureDispatcher, Member};
pub use physics::{Overscroll, ScrollPhysics, ScrollPosition};
pub use recognizer::{GestureRecognizer, Recognized, Sink};
pub use recognizers::{DragRecognizer, LongPressRecognizer, ScaleRecognizer, TapRecognizer};
pub use velocity::{VelocityTracker, HORIZON, MAX_VELOCITY};

/// The simulations a released gesture hands off to.
///
/// Written here, and moved to `vieww-animation` in Phase 7 so that an
/// `AnimationController` could use the same two. Re-exported because the
/// scroll physics are still their first caller, and because a scrollable
/// should not have to know which crate a spring lives in.
pub use vieww_animation::{Fling, Spring, MIN_FLING_VELOCITY};

pub use vieww_animation as animation;
pub use vieww_foundation as foundation;
