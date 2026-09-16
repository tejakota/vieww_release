//! Files dragged in from outside the application.
//!
//! # Why this is window-level and not a hit-tested widget
//!
//! Because the platform does not say where the pointer is. `winit` 0.30 reports
//! `HoveredFile`, `HoveredFileCancelled` and `DroppedFile` carrying **a path and
//! nothing else** — no position, no modifiers, no pointer id. During an OS drag
//! the drag is owned by the window manager, and the ordinary cursor stream stops.
//!
//! So a per-widget drop target cannot be built honestly on this backend. It
//! could be *written*: take the last cursor position seen before the drag began
//! and hit test with it. That answer is right whenever the user happens not to
//! have moved, which is to say it is right in the demo and wrong in use, and the
//! failure is a file landing in the wrong place with no way for the user to tell
//! it had. `docs/AIMS.md`'s first rule is that a design conceding a step down is
//! the wrong design rather than one to annotate.
//!
//! What is built instead is the truth the platform actually reports: **the
//! window is the drop target.** [`FileDrag`] is published above the tree the way
//! [`Capture`](crate::Capture) is, so a zone can light up while a file is over
//! the window, and the paths are delivered once to the application.
//!
//! A backend that *does* report a position — a future winit, or a first-party
//! platform layer — can route by it without changing anything here: the
//! published state stays what it is and the delivery grows a position.

use std::fmt;
use std::path::PathBuf;

/// Whether a file from outside is currently over the window.
///
/// Published above the tree so a drop zone can show that it would accept
/// something, which is the entire visual affordance of drag-and-drop: a user
/// dragging a file needs to know *before* letting go that letting go will work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileDrag {
    /// Nothing is being dragged over the window.
    #[default]
    Idle,
    /// At least one file is over the window and would be accepted.
    ///
    /// One state rather than a count, deliberately. The platform reports one
    /// event per file and gives no total, so a count assembled here would be
    /// "how many events have I seen since the last cancel" — which is not the
    /// number of files and drifts the moment one event is missed.
    Hovering,
}

impl FileDrag {
    /// `true` while something is over the window.
    #[must_use]
    pub const fn is_hovering(self) -> bool {
        matches!(self, Self::Hovering)
    }
}

impl fmt::Display for FileDrag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idle => "idle",
            Self::Hovering => "hovering",
        })
    }
}

/// Files the user dropped, gathered into one delivery.
///
/// # Why the platform's one-event-per-file is gathered here
///
/// Because "the user dropped these four files" is one action and an application
/// that receives it as four is one that uploads four times, or shows four
/// toasts. The platform reports per file because that is how the underlying
/// APIs are shaped; nothing above this layer should have to know that.
///
/// Gathered until the drag ends rather than on a timer, so the grouping is the
/// platform's own definition of one drop rather than a guess about how fast a
/// window manager delivers events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DroppedFiles {
    paths: Vec<PathBuf>,
}

impl DroppedFiles {
    /// A delivery of `paths`.
    #[must_use]
    pub const fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    /// The files, in the order the platform reported them.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether anything was dropped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// How many files.
    #[must_use]
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Add one file to this delivery.
    pub fn push(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    /// Take what has gathered, leaving this empty.
    ///
    /// A take rather than a read, for [`FrameDriver::take_announcements`]'s
    /// reason: a drop happens once, and two readers each handling it is worse
    /// than one of them missing it.
    ///
    /// [`FrameDriver::take_announcements`]: https://docs.rs/vieww-render
    #[must_use]
    pub fn take(&mut self) -> Self {
        Self {
            paths: std::mem::take(&mut self.paths),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_multi_file_drop_is_one_delivery() {
        // The platform reports one event per file; an application that saw four
        // would upload four times.
        let mut gathering = DroppedFiles::default();
        gathering.push(PathBuf::from("a.png"));
        gathering.push(PathBuf::from("b.png"));

        let dropped = gathering.take();
        assert_eq!(dropped.len(), 2);
        assert!(
            gathering.is_empty(),
            "taking must leave nothing behind, or the next drop carries these too"
        );
    }

    #[test]
    fn order_is_the_platforms_own() {
        let mut gathering = DroppedFiles::default();
        gathering.push(PathBuf::from("first"));
        gathering.push(PathBuf::from("second"));
        assert_eq!(
            gathering.paths(),
            [PathBuf::from("first"), PathBuf::from("second")]
        );
    }

    #[test]
    fn idle_is_the_default_because_nothing_is_being_dragged_yet() {
        assert_eq!(FileDrag::default(), FileDrag::Idle);
        assert!(!FileDrag::default().is_hovering());
    }
}
