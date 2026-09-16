//! The animation layer — values that change because time passed.
//!
//! # The one idea
//!
//! Every animation in this crate is a **function of time**, sampled at the
//! frame's vsync timestamp. Nothing integrates a velocity by a frame delta and
//! nothing reads a clock. That single constraint is what makes an animation here
//! behave the same at 60Hz and 120Hz, survive a dropped frame by skipping ahead
//! rather than stretching, and be testable by passing it a `Duration` instead of
//! by sleeping.
//!
//! # The pieces
//!
//! | piece | job |
//! |---|---|
//! | [`Curve`] | the shape of a timed animation — ease-in, ease-out, a cubic bézier |
//! | [`Tween`] / [`Lerp`] | what "half way between these two values" means, per type |
//! | [`Spring`] / [`Fling`] | motion described by physics, for continuing a gesture |
//! | [`AnimationController`] | one `f32` moving over time, forwards, back, or under a simulation |
//! | [`Tickers`] | the registry a frame advances |
//!
//! # Layering
//!
//! This crate depends on [`vieww_foundation`] and nothing else, so it can be
//! used from the widget layer (implicit animation) and from the gesture layer
//! (scroll physics) without either depending on the other. `vieww-gestures`
//! re-exports [`Spring`] and [`Fling`], which lived there until Phase 7.
//!
//! ```
//! use std::time::Duration;
//! use vieww_animation::{AnimationController, Curve, Tween};
//! use vieww_foundation::Color;
//!
//! let ms = Duration::from_millis;
//! let tint = Tween::new(Color::RED, Color::BLUE);
//!
//! let mut controller = AnimationController::new(ms(300)).curve(Curve::EASE_IN_OUT);
//! controller.forward(ms(0));
//!
//! // The frame scheduler supplies the timestamp; the value is read off it.
//! controller.tick(ms(150));
//! let halfway = tint.at(controller.value());
//! assert!(halfway.r > 0 && halfway.b > 0, "part way between the two");
//!
//! controller.tick(ms(300));
//! assert_eq!(tint.at(controller.value()), Color::BLUE);
//! assert!(!controller.is_animating(), "and it stops costing frames");
//! ```

mod controller;
mod curve;
mod simulation;
mod ticker;
mod tween;

pub mod spring;

pub use controller::{AnimationController, AnimationStatus};
pub use curve::Curve;
pub use simulation::{Fling, Motion, Simulation, Spring, MIN_FLING_VELOCITY};
// `SpringAnimation` is the retargetable, Ticker-driven spring; `Spring` above
// is the closed-form simulation. See `spring.rs` for why both exist.
pub use spring::{Spring2D, SpringAnimation, SpringPreset, SpringSpec};
pub use ticker::{Ticker, Tickers};
pub use tween::{Lerp, Tween};

pub use vieww_foundation as foundation;
