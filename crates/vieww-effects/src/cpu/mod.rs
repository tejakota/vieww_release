//! CPU implementations of the render effects: blur, filters, and blend.
//!
//! # Why CPU first
//!
//! The GPU backend is the performance path; the CPU backend is the
//! *correctness* path. A CPU blur is slow (O(n²) in the kernel size) but
//! it is simple, it runs on every machine including CI, and it serves as
//! the reference implementation the GPU version is tested against.
//!
//! # The algorithm choices
//!
//! **Blur: separable box blur, three passes.** A Gaussian blur is the
//! sum of infinitely many box blurs; three box passes approximate a
//! Gaussian to within visual indistinguishability. The box blur itself is
//! O(n) in the radius (sliding window sum), which makes the whole thing
//! O(n·r) where r is the radius — the standard trick from Ivan Kutskir's
//! essay that every fast CSS blur uses.
//!
//! **Filters: direct matrix application.** Per-pixel, no tricks — the
//! 5×4 colour matrix is 20 multiplications and 16 additions per pixel,
//! and a modern CPU does this to a 256×256 image in under a millisecond.
//!
//! **Blend: per-pixel function dispatch.** Each blend mode is a pure
//! function `(src, dst) -> out`. The dispatch is a match in a hot loop;
//! a vtable per pixel would be slower.

pub mod blend;
pub mod blur;
pub mod matrix;

pub use blend::blend_pixels;
pub use blur::{blur_alpha, blur_rgba, boxes_for_gaussian};
pub use matrix::apply_color_matrix;
