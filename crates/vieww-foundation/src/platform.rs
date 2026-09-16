use std::fmt;

// `vieww` supports two shipped targets and three desktop harness targets. Any
// other target is a mistake worth catching at compile time rather than at the
// first `TargetPlatform::current()` call.
// `wasm32-unknown-unknown` reports `target_os = "unknown"`, so the browser is
// matched on the architecture instead — it is the one supported target whose
// name is not an operating system.
#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_arch = "wasm32",
)))]
compile_error!(
    "vieww supports android and ios (shipped), linux/macos/windows (development \
     harness only), and wasm32 (the browser). See docs/DESIGN.md §8."
);

/// Which platform the framework is running on.
///
/// This is the **only** way behavioural platform differences are allowed to
/// reach the core crates — scroll physics, default page transitions, and the
/// like. Structural differences (surface creation, IME, accessibility) go
/// through a trait implemented in a `vieww-platform-*` crate instead, never
/// through a branch on this enum.
///
/// The distinction matters: a `TargetPlatform` branch says "both platforms can
/// do this, they just prefer it differently", and every such branch must be
/// overridable per widget. If a branch would have one arm that cannot be
/// written at all, it is structural and belongs in a platform crate.
///
/// See `docs/DESIGN.md` §8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TargetPlatform {
    /// A shipped target.
    Android,
    /// A shipped target.
    IOS,
    /// Development harness only — not a shipped target.
    Linux,
    /// Development harness only — not a shipped target.
    MacOS,
    /// Development harness only — not a shipped target.
    Windows,
    /// The browser, through `vieww-platform-web`.
    ///
    /// Not a shipped target in the sense the two mobile ones are: it exists so
    /// a vieww tree can be *published* — a demo, a gallery, a page — rather than
    /// installed. Its conventions are the desktop ones, because a browser window
    /// is a desktop window with different chrome.
    Web,
}

impl TargetPlatform {
    /// The platform this build is compiled for.
    ///
    /// Resolved at compile time, so a branch on it costs nothing at runtime and
    /// the dead arm is discarded.
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "android")]
        return Self::Android;
        #[cfg(target_os = "ios")]
        return Self::IOS;
        #[cfg(target_os = "linux")]
        return Self::Linux;
        #[cfg(target_os = "macos")]
        return Self::MacOS;
        #[cfg(target_os = "windows")]
        return Self::Windows;
        // Last, and on the architecture rather than the OS: `target_os` is
        // `"unknown"` here, which is not a thing to branch on.
        #[cfg(target_arch = "wasm32")]
        return Self::Web;
    }

    /// `true` for the two platforms `vieww` actually ships to.
    ///
    /// The desktop variants exist so Phases 1–4 can be worked on without a
    /// device attached; anything that works only on desktop is not done.
    #[must_use]
    pub const fn is_shipped_target(self) -> bool {
        matches!(self, Self::Android | Self::IOS)
    }

    /// `true` on Android and iOS.
    #[must_use]
    pub const fn is_mobile(self) -> bool {
        self.is_shipped_target()
    }

    /// `true` on platforms whose conventions follow Apple's — iOS and macOS.
    ///
    /// This, rather than `== IOS`, is what a behavioural branch should test, so
    /// that running the harness on a Mac exercises the same code path an iPhone
    /// will. A branch that must distinguish iOS from macOS specifically is
    /// almost certainly structural and belongs in a platform crate.
    #[must_use]
    pub const fn is_apple(self) -> bool {
        matches!(self, Self::IOS | Self::MacOS)
    }

    /// The lowercase name, matching Rust's `target_os` values.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::IOS => "ios",
            Self::Linux => "linux",
            Self::MacOS => "macos",
            Self::Windows => "windows",
            // Not a `target_os` value — `wasm32-unknown-unknown` has none worth
            // printing, and "unknown" names nothing a reader can act on.
            Self::Web => "web",
        }
    }
}

impl Default for TargetPlatform {
    fn default() -> Self {
        Self::current()
    }
}

impl fmt::Display for TargetPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_matches_the_compiled_target_os() {
        assert_eq!(TargetPlatform::current().name(), std::env::consts::OS);
    }

    #[test]
    fn only_android_and_ios_are_shipped_targets() {
        assert!(TargetPlatform::Android.is_shipped_target());
        assert!(TargetPlatform::IOS.is_shipped_target());

        for harness in [
            TargetPlatform::Linux,
            TargetPlatform::MacOS,
            TargetPlatform::Windows,
        ] {
            assert!(!harness.is_shipped_target(), "{harness} is harness-only");
        }
    }

    #[test]
    fn apple_conventions_cover_the_mac_harness_as_well_as_ios() {
        assert!(TargetPlatform::IOS.is_apple());
        assert!(
            TargetPlatform::MacOS.is_apple(),
            "running the harness on a Mac must exercise the iOS code path"
        );
        assert!(!TargetPlatform::Android.is_apple());
        assert!(!TargetPlatform::Linux.is_apple());
    }

    #[test]
    fn default_is_the_current_platform() {
        assert_eq!(TargetPlatform::default(), TargetPlatform::current());
    }
}
