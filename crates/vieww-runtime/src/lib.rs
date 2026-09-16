//! The application runtime: priority-ordered concurrency and frame
//! lifecycle orchestration.
//!
//! The architecture review this workspace's docs quote elsewhere puts it
//! directly: today's UI-thread-oriented, `Rc`/`RefCell` model "can be
//! perfectly valid," but a framework aiming higher needs a scheduler with
//! real priority classes and a named frame pipeline. This crate is both
//! halves: [`scheduler::Scheduler`] is the priority-ordered thread pool,
//! and [`frame::FrameOrchestrator`] is the named pipeline
//! (input → scheduler → state update → tree update → layout →
//! render-plan → GPU/CPU scheduling) that a real `App` runs once per frame.

pub mod frame;
pub mod priorities;
pub mod scheduler;

pub use frame::{FnStage, FrameOrchestrator, FrameReport, Stage, StageTiming};
pub use priorities::Priority;
pub use scheduler::Scheduler;
