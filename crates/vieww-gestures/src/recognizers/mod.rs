//! The built-in recognisers.
//!
//! Each owns one gesture and knows nothing about the others — disambiguation is
//! the [arena](crate::arena)'s job, not theirs. That separation is what lets a
//! new recogniser be added without revisiting the existing ones.

mod drag;
mod long_press;
mod scale;
mod tap;

pub use drag::DragRecognizer;
pub use long_press::LongPressRecognizer;
pub use scale::ScaleRecognizer;
pub use tap::TapRecognizer;
