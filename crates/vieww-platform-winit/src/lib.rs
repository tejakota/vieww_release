//! The platform bridge — the one crate allowed to talk to the operating system.
//!
//! Everything below this crate is arithmetic. `vieww-render` can lay out a tree,
//! paint it, hit test it and hand you a scene, and none of that needs a window,
//! which is why 500-odd tests run without one. But a scene nobody sees is not a
//! frame, and the things that make it one — a window, a swapchain, a vsync, a
//! finger — belong to the OS. This is where they come in, and it is deliberately
//! the only place.
//!
//! ```no_run
//! use vieww_foundation::Color;
//! use vieww_platform_winit::App;
//! use vieww_widget::prelude::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let report = App::new()
//!     .title("hello")
//!     .run(|driver| {
//!         driver.elements().set_root(ColoredBox::new(Color::BLUE));
//!     })?;
//!
//! println!("{report}");
//! # Ok(()) }
//! ```
//!
//! # Why one crate for five platforms
//!
//! `docs/DESIGN.md` §8 says platform differences enter through a
//! `vieww-platform-*` crate rather than a `cfg` in a core crate, and it expects
//! more than one of them. This is the first, and it covers all five targets at
//! once, because `winit` already abstracts exactly the piece Phase 8's first
//! item asks for: a window, a surface, a lifecycle and raw input, on Android,
//! iOS, Linux, macOS and Windows. Writing that abstraction again per platform
//! would be the "own everything except this" rule applied to the one thing the
//! rule exempts.
//!
//! The later Phase 8 items are not like that. An IME bridge is
//! `InputConnection` on one platform and `UITextInput` on the other, with no
//! shared shape worth naming; accessibility is TalkBack and VoiceOver. Those get
//! their own crates, and this one does not grow to hold them.
//!
//! # Logical pixels stop here
//!
//! The framework measures in logical pixels; a window measures in physical ones.
//! [`Scale`] is where the two meet, and it is the only place — see its docs for
//! why that matters more than it sounds like it does.
//!
//! # What this does not do yet
//!
//! Stated so it is not mistaken for done. Everything this list used to say —
//! no keyboard events, no IME, no accessibility, no insets, no hover — is now
//! wired, and the list was wrong for long enough to be worth mentioning: a
//! "not done" note that outlives the work is more misleading than no note.
//!
//! What is genuinely still open:
//!
//! - **No frame has reached an iPhone.** The iOS paths compile, link and — for
//!   the portable half — run on a simulator in CI. `src/ios.rs` is written against
//!   UIKit and has never executed against it. Treat every iOS number as
//!   unverified until a device says otherwise.
//! - **Neither phone's screen reader has been heard.** Both are now *wired*,
//!   which is a correction: this list previously said `accesskit_winit` had no
//!   Android backend and did not wire the iOS one, and by 0.33.2 neither was
//!   true. iOS goes through `accesskit_winit`'s own unconditional iOS arm and
//!   has been in every build since; Android goes through `a11y_android`,
//!   which drives `accesskit_android` directly because that crate's `winit`
//!   shim only knows how to find a `GameActivity`'s view. **Untested on both**
//!   — nobody has switched VoiceOver or TalkBack on.
//! - **Back and forward mouse buttons are dropped** rather than translated;
//!   `PointerButton` has no shape for them.

// Crate-internal deliberately: every function in it names an `accesskit` type in
// its signature, and `app.rs` is the only caller. Published, it would make the
// AccessKit version part of this crate's contract — a caller could not handle a
// `TreeUpdate` without naming a matching `accesskit` themselves. Widening is not
// a breaking change, so this can be reopened the day something outside needs it.
pub(crate) mod a11y;
#[cfg(target_os = "android")]
mod a11y_android;
mod app;
pub mod clipboard;
pub mod crash;
pub mod deep_links;
// This crate's one and only renderer: CPU rasterisation via
// `vieww_paint::native::NativeRenderer`, presented through `vieww-hal`'s raw
// Vulkan swapchain — see the module's own docs. Public (unlike `raster`
// below) because an application has to be able to name
// `NativeRenderer`/`NativeSurface`.
mod input;
mod insets;
#[cfg(target_os = "ios")]
mod ios;
mod keys;
mod latency;
mod lifecycle;
pub mod locale;
pub mod native;
mod scale;
pub mod services;
// Real backends for `vieww_foundation::desktop`, and the CPU rasteriser the
// tray needs to turn an `IconData` into pixels. Both are behind the feature
// because the tray pulls GTK on Linux — see the manifest — and because a
// rasteriser with no tray to feed is dead code under `-D warnings`.
#[cfg(all(
    feature = "desktop-services",
    not(any(target_os = "android", target_os = "ios"))
))]
pub mod desktop;
#[cfg(all(
    feature = "desktop-services",
    not(any(target_os = "android", target_os = "ios"))
))]
mod raster;
mod stats;
pub mod storage;
mod windows;

pub use app::{App, CloseRequest, Closing, PlatformError, Waker};
pub use deep_links::PlatformDeepLinks;
pub use windows::PlatformWindows;

pub use input::{PointerTranslator, TouchPhase};
pub use keys::{ime, KeyTranslator};
pub use latency::{InputLatencyLog, InputLatencyReport};
pub use lifecycle::Lifecycle;
pub use scale::{Physical, Scale};
pub use stats::{Frame, FrameLog, FrameReport};
pub use storage::{data_dir, FileStorage};
/// The activity handle `android_main` is handed, re-exported so an application
/// needs no direct dependency on `android-activity`.
///
/// Taking it from `winit` rather than depending on the crate directly is not
/// tidiness: the two must agree on the *same* version, and a second copy in the
/// tree produces a type that looks identical and cannot be passed to
/// [`App::run_android`].
#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;
