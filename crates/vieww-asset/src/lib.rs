//! An application's own files: where they come from, and how bytes become
//! pixels.
//!
//! # The gap this closes
//!
//! [`Image`] had exactly one constructor — `from_rgba8` — so every application
//! brought its own decoder, its own cache, and its own answer to where a file
//! lives on each platform. That is three wheels reinvented before the first
//! picture appears on a screen, and the third one is the nasty one: an APK is a
//! zip and an iOS bundle is a directory, so "open a file" is not one thing.
//!
//! # The shape
//!
//! - [`AssetBundle`] is *where bytes come from*, and it is a
//!   [service](vieww_foundation::service): the platform registers one, and an
//!   application can register a different one — a downloaded pack, a test
//!   double, a directory on disk — without this crate knowing.
//! - [`decode`] is *bytes to pixels*, and is pure. It needs no platform, no
//!   files and no service, which is why it is testable everywhere.
//! - [`ImageCache`] keeps decoded pixels, because decoding a 2MB photograph on
//!   the frame that wants to draw it is a dropped frame every time.
//!
//! # Why decoding is not on the UI thread's critical path
//!
//! It is here, in a synchronous function, and that is deliberate: this crate
//! takes no position on threading. [`decode`] is pure and `Send`-friendly, so an
//! application hands it to [`Spawn`](vieww_foundation::task::Spawn) and gets a
//! [`Task`](vieww_foundation::task::Task) back. Baking a thread in here would
//! mean this crate choosing a runtime on the application's behalf, which
//! [`task`](vieww_foundation::task) already declined to do.

mod bundle;
mod cache;
mod codec;
#[cfg(feature = "svg")]
mod svg;

pub use bundle::{AssetBundle, AssetError, DirectoryBundle, EmbeddedBundle};
pub use cache::{ImageCache, SharedImageCache};
pub use codec::{decode, decode_sized, ImageFormat};
#[cfg(feature = "svg")]
pub use svg::parse_svg;

pub use vieww_foundation::Image;
