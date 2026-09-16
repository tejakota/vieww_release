//! The accessibility settings a person has already chosen, system-wide.
//!
//! # Why these are values in the tree and not feature flags
//!
//! Every one of these is a preference the user set once, in the OS, expecting
//! every application to honour it. A framework that exposes them as build-time
//! flags, or as something the application opts into, has quietly decided that
//! honouring them is a feature — and features get cut. Published above the tree
//! beside [`ViewMetrics`](crate::ViewMetrics) and [`Capture`](crate::Capture),
//! they are instead the ambient truth a widget has to read to draw at all, and
//! the default is the only thing anybody has to remember.
//!
//! # Why this is a third provider rather than fields on `ViewMetrics`
//!
//! The same reason `Capture` is separate, and the reasoning is recorded there:
//! these change on a wholly different clock. A rotation happens often, a
//! screenshot rarely, and a person turns "reduce motion" on roughly once in the
//! life of the device. A widget that reads one must not be rebuilt because
//! another changed.
//!
//! # What is actually wired, and what is only published
//!
//! Stated precisely, because "the framework supports reduced motion" is the
//! kind of claim that is easy to make and expensive to be wrong about.
//!
//! * **[`text_scale`](Accessibility::text_scale) is applied.** Every `Text` and
//!   `TextField` is scaled by it, in `vieww-render`'s factory — the one funnel
//!   they all pass through, so a widget added later cannot forget. The default
//!   is exactly `1.0`, so a tree that never sets it renders identically to one
//!   written before this existed.
//!
//! * **[`reduce_motion`](Accessibility::reduce_motion) is applied
//!   automatically, by two independent paths, deliberately kept both.**
//!   [`FrameDriver::set_accessibility`](https://docs.rs/vieww-render)
//!   forwards it to the animation registry every framework `AnimationController`
//!   is already ticked through once per frame
//!   (`vieww_animation::Tickers::set_reduce_motion`), which settles each one
//!   onto its destination instead of stepping toward it — see
//!   `AnimationController::settle`'s own doc for why a **repeating** animation
//!   (a spinner, a pulse) is the deliberate exception and keeps running rather
//!   than freezing on one frame, which is what every platform's own
//!   reduced-motion mode does with a progress indicator. This covers every
//!   spring, timeline and route transition the framework itself drives, with
//!   no change needed to `Widget::create_state` — the registry, not the
//!   widget, is what reads the ambient preference. It does **not** cover an
//!   application's own `AnimationController` if that application never hands
//!   it to the registry (rare — `Tickers` is where one is registered to
//!   receive frames at all) — for that case,
//!   [`motion`](Accessibility::motion) is still there to convert a duration by
//!   hand. `ThemeData::with_accessibility` is the second path, described next,
//!   and covers the case neither of the above does: application code that
//!   reads a *duration token* out of the theme and drives its own tween
//!   without ever creating an `AnimationController` at all.
//!
//! * **[`high_contrast`](Accessibility::high_contrast) is applied by
//!   `ThemeData`, automatically, at every `Theme::of(ctx)` call** —
//!   `vieww-widget::theme::ThemeData::of` reads the ambient `Accessibility`
//!   after resolving the inherited theme and, if either preference is set,
//!   returns `theme.with_accessibility(accessibility)` instead: a
//!   high-contrast-retinted `ColorScheme` (and its dependent `Typography`,
//!   which is coloured from the scheme) for `high_contrast`, and
//!   `Motion::reduced()` — every duration token collapsed to zero, springs
//!   left alone because those are already covered by the ticker path above —
//!   for `reduce_motion`. This is genuinely the palette-owning crate applying
//!   its own preference, not this crate reaching into it: `vieww-foundation`
//!   still has no palette and still could not apply either preference itself.
//!
//! # Nothing reads these off the OS yet
//!
//! `FrameDriver::set_accessibility` is the way in, and today the only caller is
//! the application. The platform bridge does not populate them, because **winit
//! exposes no portable API for any of the three** — checked against winit
//! 0.30, where the only thing of the kind is a private
//! `is_high_contrast()` inside the Windows dark-mode module. Reading them means
//! per-platform code: `UIAccessibility.isReduceMotionEnabled` and
//! `UIApplication.preferredContentSizeCategory` on iOS,
//! `Settings.Global.ANIMATOR_DURATION_SCALE` and `Configuration.fontScale` on
//! Android, `NSWorkspace.accessibilityDisplayShouldReduceMotion` on macOS,
//! `SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)` on Windows.
//!
//! Stated so that "vieww supports Dynamic Type" is not read into this file. The
//! *plumbing* is complete and tested end to end; the four platform reads are
//! not written.

use core::fmt;

/// System-wide accessibility preferences in force for this view.
///
/// [`Default`] is "nothing has been asked for", which is also what every
/// platform reports when it has no answer — so a backend that cannot read one
/// of these leaves it alone rather than guessing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accessibility {
    /// The person has asked for less movement.
    ///
    /// `prefers-reduced-motion` on the web, "Reduce Motion" on iOS and macOS,
    /// `ANIMATOR_DURATION_SCALE == 0` on Android, "Show animations in Windows"
    /// off on Windows.
    ///
    /// For vestibular disorders this is a medical setting, not a taste: large
    /// parallax and zoom transitions cause genuine nausea. Treat it as
    /// non-negotiable.
    pub reduce_motion: bool,
    /// The person has asked for stronger contrast between foreground and
    /// background.
    ///
    /// "Increase Contrast" on iOS/macOS, high-contrast themes on Windows,
    /// `prefers-contrast` on the web. Published for a theme to read; see the
    /// module docs for why this crate does not apply it itself.
    pub high_contrast: bool,
    /// Multiplier for every text size, from the OS's text-size setting.
    ///
    /// `1.0` is unscaled. Android's font scale and iOS's Dynamic Type both land
    /// here. Clamped by [`new`](Self::new) rather than trusted, because the
    /// platform range is wide — Android goes to 2.0 and iOS's accessibility
    /// sizes go further — and an unclamped multiplier arriving from an OS
    /// setting is a layout that cannot be designed for at all.
    pub text_scale: f32,
}

impl Default for Accessibility {
    fn default() -> Self {
        Self {
            reduce_motion: false,
            high_contrast: false,
            text_scale: 1.0,
        }
    }
}

/// The largest text multiplier the framework will apply.
///
/// Not a judgement about how large text should be — it is the point past which
/// a multiplier stops being a text-size setting and becomes a different layout.
/// Platforms do offer more, and an application that wants to honour the full
/// range should read [`Accessibility::text_scale`] and reflow, which is a
/// design decision this crate cannot make on its behalf.
pub const MAX_TEXT_SCALE: f32 = 3.0;

/// The smallest text multiplier. Below this, text is unreadable for everyone,
/// and a zero or negative scale from a misreported platform value would
/// collapse every line box in the tree.
pub const MIN_TEXT_SCALE: f32 = 0.5;

impl Accessibility {
    /// Preferences with `text_scale` clamped into the supported range.
    ///
    /// The only constructor that sanitises. A `NaN` scale — which is what a
    /// platform bridge produces from a missing setting more often than anyone
    /// expects — becomes `1.0` rather than propagating into every line box in
    /// the tree, because `NaN` compares false against both bounds and would
    /// survive a naive clamp.
    #[must_use]
    pub fn new(reduce_motion: bool, high_contrast: bool, text_scale: f32) -> Self {
        let text_scale = if text_scale.is_nan() {
            1.0
        } else {
            text_scale.clamp(MIN_TEXT_SCALE, MAX_TEXT_SCALE)
        };
        Self {
            reduce_motion,
            high_contrast,
            text_scale,
        }
    }

    /// `text_scale` applied to one size.
    ///
    /// The one place the multiplication happens, so that a rounding or clamping
    /// decision made later is made once.
    #[must_use]
    pub fn scale_text(self, size: f32) -> f32 {
        size * self.text_scale
    }

    /// `duration` as it should actually be run.
    ///
    /// [`Duration::ZERO`](core::time::Duration::ZERO) under
    /// [`reduce_motion`](Self::reduce_motion), and unchanged otherwise.
    ///
    /// **Zero rather than skipped**, for the reason the module docs give: an
    /// animation that is never started never completes, and completion is what
    /// most transitions actually depend on.
    #[must_use]
    pub fn motion(self, duration: core::time::Duration) -> core::time::Duration {
        if self.reduce_motion {
            core::time::Duration::ZERO
        } else {
            duration
        }
    }
}

impl fmt::Display for Accessibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "reduce_motion={} high_contrast={} text_scale={}",
            self.reduce_motion, self.high_contrast, self.text_scale
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn the_default_is_nothing_asked_for_and_changes_nothing() {
        let a = Accessibility::default();
        assert!(!a.reduce_motion);
        assert!(!a.high_contrast);
        assert_eq!(a.text_scale, 1.0);
        assert_eq!(a.scale_text(14.0), 14.0);
        assert_eq!(
            a.motion(Duration::from_millis(300)),
            Duration::from_millis(300)
        );
    }

    #[test]
    fn reduced_motion_zeroes_a_duration_rather_than_shortening_it() {
        let a = Accessibility::new(true, false, 1.0);
        assert_eq!(a.motion(Duration::from_millis(300)), Duration::ZERO);
        // And zero stays zero, so the call is idempotent.
        assert_eq!(a.motion(Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn text_scale_is_clamped_into_a_range_a_layout_can_be_designed_for() {
        assert_eq!(
            Accessibility::new(false, false, 10.0).text_scale,
            MAX_TEXT_SCALE
        );
        assert_eq!(
            Accessibility::new(false, false, 0.01).text_scale,
            MIN_TEXT_SCALE
        );
        assert_eq!(Accessibility::new(false, false, 1.5).text_scale, 1.5);
    }

    /// A zero or negative scale collapses every line box in the tree, and it is
    /// exactly what a platform bridge produces from a setting it failed to
    /// read.
    #[test]
    fn a_nonsense_scale_cannot_collapse_the_text() {
        assert!(Accessibility::new(false, false, 0.0).text_scale >= MIN_TEXT_SCALE);
        assert!(Accessibility::new(false, false, -4.0).text_scale >= MIN_TEXT_SCALE);
    }

    /// `NaN` survives a naive `clamp` — it compares false against both bounds —
    /// so it is handled before the clamp rather than by it. Pinned because the
    /// obvious refactor is to delete the branch.
    #[test]
    fn a_nan_scale_becomes_one_rather_than_poisoning_every_line_box() {
        let a = Accessibility::new(false, false, f32::NAN);
        assert!(!a.text_scale.is_nan(), "a NaN scale reached the tree");
        assert_eq!(a.text_scale, 1.0);
        assert_eq!(a.scale_text(14.0), 14.0);
    }

    #[test]
    fn scaling_is_applied_at_one_place_and_is_a_plain_multiply() {
        let a = Accessibility::new(false, false, 2.0);
        assert_eq!(a.scale_text(14.0), 28.0);
    }
}
