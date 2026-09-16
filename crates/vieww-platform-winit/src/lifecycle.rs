//! Whether the application is on screen, and what that means for frames.
//!
//! # Why this is a state machine and not two booleans
//!
//! A window can be un-focused but visible, visible but occluded, and alive but
//! surface-less, and the four platforms report those in different orders and not
//! all of them at all. Tracked as separate flags, the question a frame actually
//! asks — *should I draw?* — becomes a conjunction that each call site gets
//! subtly wrong. Tracked as one state, it is a method.
//!
//! # Why it is defined here and not in a core crate
//!
//! `docs/DESIGN.md` §8 puts structural platform differences behind a trait in a
//! core crate. This is not one of those yet: `winit` is the *single* bridge for
//! all five targets, so a trait here would have exactly one implementor and no
//! second opinion to design against. When something above the platform layer
//! needs to observe lifecycle — a widget that pauses a video, say — this moves
//! down to `vieww-foundation` and this crate reports into it.

use std::fmt;

use vieww_foundation::Capture;

/// Where the application is in its life.
///
/// Ordered by how much of the machine it currently has: `Detached` has no
/// window at all, `Resumed` has everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lifecycle {
    /// No window and no surface — before the first resume, and after the last.
    ///
    /// On Android this is not only startup: a task can be detached and later
    /// re-created around the *same* process, which is why the element tree must
    /// outlive the surface.
    #[default]
    Detached,
    /// On screen, and the thing the user is interacting with.
    Resumed,
    /// On screen but not focused — another window is in front, or a system
    /// panel is up. Still visible, so still drawn.
    Inactive,
    /// Alive, but nothing of it can be seen: minimised, occluded, or behind
    /// another full-screen window.
    Hidden,
    /// The surface is gone. Nothing may be drawn until a resume brings one back.
    Paused,
}

impl Lifecycle {
    /// `true` when a frame would reach a human being.
    ///
    /// The single question the event loop asks. `Hidden` is drawable in the
    /// sense that the surface exists, but not worth drawing into — see
    /// [`should_draw`](Self::should_draw), which is the one to use.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        matches!(self, Self::Resumed | Self::Inactive)
    }

    /// `true` when there is a surface to draw into at all.
    ///
    /// Distinct from visibility: a hidden window still has a swapchain, and
    /// presenting to it is legal — merely pointless.
    #[must_use]
    pub const fn has_surface(self) -> bool {
        !matches!(self, Self::Detached | Self::Paused)
    }

    /// `true` when the event loop should keep producing frames.
    #[must_use]
    pub const fn should_draw(self) -> bool {
        self.is_visible()
    }

    /// The state after the platform reports a resume.
    #[must_use]
    pub const fn resumed(self) -> Self {
        Self::Resumed
    }

    /// The state after the platform takes the surface away.
    #[must_use]
    pub const fn suspended(self) -> Self {
        Self::Paused
    }

    /// The state after focus is gained or lost.
    ///
    /// Focus does not move a window that has no surface, and it does not
    /// un-hide one: a minimised window can be told it lost focus, and promoting
    /// that to `Inactive` would claim it is on screen.
    #[must_use]
    pub const fn focused(self, focused: bool) -> Self {
        match (self, focused) {
            (Self::Resumed | Self::Inactive, true) => Self::Resumed,
            (Self::Resumed | Self::Inactive, false) => Self::Inactive,
            (other, _) => other,
        }
    }

    /// The state after the compositor reports the window covered or uncovered.
    ///
    /// Un-occluding lands on `Inactive` rather than `Resumed`: being visible
    /// again says nothing about having the keyboard, and a `Focused` will
    /// follow if it does.
    #[must_use]
    pub const fn occluded(self, occluded: bool) -> Self {
        match (self, occluded) {
            (Self::Resumed | Self::Inactive, true) => Self::Hidden,
            (Self::Hidden, false) => Self::Inactive,
            (other, _) => other,
        }
    }

    /// The state after the event loop is told to exit.
    #[must_use]
    pub const fn detached(self) -> Self {
        Self::Detached
    }

    /// Who is reading the surface, as far as lifecycle alone can tell.
    ///
    /// The consuming half of [`Capture`] for the one capture mechanism that is
    /// a lifecycle event rather than a notification: the thumbnail the OS takes
    /// of an application on its way out of the foreground, to show in the task
    /// switcher. A real screenshot or screen-recording notification, where a
    /// platform offers one, is a separate call to
    /// [`FrameDriver::set_capture`](vieww_render::FrameDriver::set_capture) and
    /// does not come through here.
    ///
    /// # Why the line is drawn at visibility and not at focus
    ///
    /// `Inactive` — on screen, but another window has the keyboard — stays
    /// [`Capture::Screen`], and that is a deliberate refusal to be pessimistic
    /// in the one place where being pessimistic is expensive. Clicking another
    /// application is the single most common thing a desktop user does, and a
    /// framework that blanked every sensitive subtree each time would be one
    /// whose users turn the feature off. `docs/AIMS.md` §I makes the same
    /// argument about touch targets: an accessibility or privacy feature that
    /// reads as a visual regression is one that gets removed.
    ///
    /// Everything that is not visible is [`Capture::Recorded`], because
    /// `Hidden`, `Paused` and `Detached` are exactly the states an application
    /// is in while the OS is entitled to draw a picture of it.
    #[must_use]
    pub const fn capture(self) -> Capture {
        if self.is_visible() {
            Capture::Screen
        } else {
            Capture::Recorded
        }
    }

    /// The lowercase name, for logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Detached => "detached",
            Self::Resumed => "resumed",
            Self::Inactive => "inactive",
            Self::Hidden => "hidden",
            Self::Paused => "paused",
        }
    }
}

impl fmt::Display for Lifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_drawn_before_the_first_resume() {
        let state = Lifecycle::default();
        assert_eq!(state, Lifecycle::Detached);
        assert!(!state.should_draw());
        assert!(!state.has_surface());
    }

    #[test]
    fn a_resume_makes_it_drawable_from_anywhere() {
        for state in [
            Lifecycle::Detached,
            Lifecycle::Paused,
            Lifecycle::Hidden,
            Lifecycle::Inactive,
        ] {
            assert_eq!(state.resumed(), Lifecycle::Resumed, "from {state}");
        }
        assert!(Lifecycle::Resumed.should_draw());
    }

    #[test]
    fn a_suspend_stops_frames_and_takes_the_surface() {
        let paused = Lifecycle::Resumed.suspended();
        assert_eq!(paused, Lifecycle::Paused);
        assert!(!paused.should_draw());
        assert!(
            !paused.has_surface(),
            "drawing into a surface Android has reclaimed is a crash, not a wasted frame"
        );
    }

    #[test]
    fn losing_focus_keeps_drawing_because_the_window_is_still_on_screen() {
        let inactive = Lifecycle::Resumed.focused(false);
        assert_eq!(inactive, Lifecycle::Inactive);
        assert!(
            inactive.should_draw(),
            "an unfocused window still animates; freezing it is a visible bug"
        );
        assert_eq!(inactive.focused(true), Lifecycle::Resumed);
    }

    #[test]
    fn focus_cannot_wake_a_window_that_has_no_surface() {
        assert_eq!(Lifecycle::Paused.focused(true), Lifecycle::Paused);
        assert_eq!(Lifecycle::Detached.focused(true), Lifecycle::Detached);
        assert_eq!(
            Lifecycle::Hidden.focused(true),
            Lifecycle::Hidden,
            "a minimised window told it has focus is not on screen"
        );
    }

    #[test]
    fn being_covered_stops_frames_and_being_uncovered_starts_them() {
        let hidden = Lifecycle::Resumed.occluded(true);
        assert_eq!(hidden, Lifecycle::Hidden);
        assert!(!hidden.should_draw());
        assert!(
            hidden.has_surface(),
            "the swapchain is still there; it is only pointless to draw into it"
        );

        let back = hidden.occluded(false);
        assert_eq!(
            back,
            Lifecycle::Inactive,
            "visible again says nothing about focus; a Focused will follow if it has it"
        );
        assert!(back.should_draw());
    }

    #[test]
    fn occlusion_does_not_resurrect_a_paused_application() {
        assert_eq!(Lifecycle::Paused.occluded(false), Lifecycle::Paused);
        assert_eq!(Lifecycle::Detached.occluded(false), Lifecycle::Detached);
    }

    #[test]
    fn leaving_the_screen_is_a_capture_and_coming_back_is_not() {
        // The round trip, because the unmask direction is the one that fails
        // silently: a mask that is never lifted looks like a rendering bug and
        // gets blamed on the widget rather than on the lifecycle.
        assert_eq!(Lifecycle::Resumed.capture(), Capture::Screen);

        let hidden = Lifecycle::Resumed.occluded(true);
        assert_eq!(
            hidden.capture(),
            Capture::Recorded,
            "an occluded window is one the OS may be drawing a thumbnail of"
        );
        assert_eq!(hidden.occluded(false).capture(), Capture::Screen);

        assert_eq!(Lifecycle::Resumed.suspended().capture(), Capture::Recorded);
        assert_eq!(
            Lifecycle::Paused.resumed().capture(),
            Capture::Screen,
            "a resume must lift the mask, or the application comes back blank"
        );
    }

    #[test]
    fn losing_focus_is_not_a_capture() {
        // The deliberate non-pessimism documented on `capture`. Clicking another
        // application is not somebody photographing your screen, and treating it
        // as one blanks a sensitive subtree several times a minute on a desktop.
        let unfocused = Lifecycle::Resumed.focused(false);
        assert_eq!(unfocused, Lifecycle::Inactive);
        assert_eq!(unfocused.capture(), Capture::Screen);
    }

    #[test]
    fn nothing_is_ever_shown_to_a_surface_that_does_not_exist() {
        // Every state that cannot be drawn into masks, with no exceptions — the
        // property that matters is that this is a total function of the state
        // rather than a list of transitions somebody has to keep in step.
        for state in [
            Lifecycle::Detached,
            Lifecycle::Resumed,
            Lifecycle::Inactive,
            Lifecycle::Hidden,
            Lifecycle::Paused,
        ] {
            assert_eq!(
                state.capture() == Capture::Screen,
                state.is_visible(),
                "{state} disagrees with its own visibility"
            );
        }
    }
}
