//! Whether something other than the user's own eyes is looking at the surface.
//!
//! `docs/AIMS.md` §D names masked sensitive views as one of the two items in
//! that section that genuinely touch a UI framework, and the reason it is a
//! framework's job rather than an application's is here: **the three places
//! sensitive content leaks are three different platform mechanisms with one
//! cause.** A screenshot, a screen recording and the thumbnail the OS takes when
//! an application is backgrounded are all "the surface was read by something
//! that is not the screen", and an application that handles one of them has
//! usually not handled the other two.
//!
//! The common answer is per-platform folklore: `FLAG_SECURE` on Android through a
//! plugin, an `NSWindow` overlay on macOS, and nothing at all for the recents
//! thumbnail unless you write the lifecycle code yourself. The framework does
//! not know which subtree is sensitive, so it cannot help.
//!
//! Here it does. [`Capture`] is published above the tree, a
//! `Sensitive` widget declares the subtree, and the two meet without either
//! half knowing about the other.
//!
//! # Why this is a state and not an event
//!
//! Because an event has to be responded to before the pixels are read, and on no
//! platform is that reliably possible — Android delivers `onPause` after the
//! thumbnail on some OEM builds. A *state* published above the tree means the
//! mask is already in the frame when the capture happens, which is the only
//! ordering that is actually safe.

use std::fmt;

/// Who is reading the surface.
///
/// Published above the widget tree by the frame driver and read by anything that
/// has something to hide. Two states rather than one per mechanism: a subtree
/// that must not appear in a screenshot must not appear in a recording or a
/// recents thumbnail either, and enumerating the mechanisms would invite a
/// declaration that covers two of the three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Capture {
    /// Only the screen, in front of the person holding the device. The ordinary
    /// state, and the default — a surface nobody has said anything about is one
    /// the user is looking at.
    #[default]
    Screen,
    /// Something is reading the pixels: a screenshot, a screen recording, a
    /// cast or mirror, or the OS building the thumbnail it shows in the task
    /// switcher.
    Recorded,
}

impl Capture {
    /// `true` while sensitive content must be hidden.
    #[must_use]
    pub const fn is_recorded(self) -> bool {
        matches!(self, Self::Recorded)
    }
}

impl fmt::Display for Capture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Screen => "screen",
            Self::Recorded => "recorded",
        })
    }
}
