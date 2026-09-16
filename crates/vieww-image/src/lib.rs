//! Image capabilities past decode-and-display.
//!
//! `vieww-asset` answers "how do I get this file's pixels" (path/bundle
//! resolution, format-sniffed decode, an identity-preserving cache with no
//! eviction). This crate answers what a renderer or a long-running app
//! needs *after* that, operating on and producing the same
//! [`vieww_foundation::Image`] pixels throughout:
//!
//! - [`mipmap`] — pure-CPU box-filter mip chain generation, straight-alpha
//!   aware (premultiplies before averaging so a transparent neighbour's
//!   meaningless colour cannot leak into a visible texel), correct at odd
//!   (non-power-of-two) dimensions.
//! - [`atlas`] — shelf-packing layout for a batch of images into one target
//!   size, reporting which requests (if any) did not fit, plus a real
//!   pixel-copying compositor that produces the packed atlas image.
//! - [`sequence`] — animated GIF decoding through `image`'s own
//!   `AnimationDecoder`, and looking up which frame is showing at a given
//!   elapsed time.
//! - [`profile`] — colour-space tagging (reusing
//!   [`vieww_foundation::ColorSpace`]) and bulk sRGB-transfer-function
//!   conversion for whole image buffers. Deliberately *not* general ICC
//!   profile parsing or gamut mapping — see the module's own docs for why
//!   that is out of scope rather than a hidden gap.
//! - [`residency`] — a byte-budgeted LRU cache for decoded images, for an
//!   application that wants bounded residency on purpose rather than
//!   `vieww-asset`'s unbounded-until-`clear()` cache.

pub mod atlas;
pub mod mipmap;
pub mod profile;
pub mod residency;
pub mod sequence;
