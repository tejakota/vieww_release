//! Visual effects: blur, filters, and blend modes.
//!
//! # Architecture
//!
//! The pixel operations (blur, colour matrix, blend) are pure functions
//! over RGBA8 buffers — they live in the `cpu` module and are testable
//! without a renderer. The widget wrappers (`BackdropBlur`,
//! `FilterChain`, `Blend`) compose with the existing widget tree.
//!
//! # The CPU reference
//!
//! The CPU implementations are the correctness reference. They are
//! slow (O(n·r) for blur) but they run everywhere, including CI, and
//! they are what the GPU implementations are tested against.

pub mod cpu;
pub mod widgets;

pub use widgets::{BackdropBlur, BackdropFilter, Blend, BlendMode, Filter, FilterChain};
