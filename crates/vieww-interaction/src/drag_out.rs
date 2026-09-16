//! Outbound drag-and-drop: dragging content *out of* the app to the OS or
//! another application.
//!
//! # Why this is not the same thing as `vieww-widget::Draggable`
//!
//! `crates/vieww-widget/src/controls/draggable.rs`'s `DragSession`/
//! `DragPayload` describe a drag entirely inside one running app — a card
//! moving between two lists in the same window, resolved by hit-testing the
//! render tree. Nothing there ever crosses a process boundary, and nothing
//! there needs the OS.
//!
//! This module is the other half: the content is leaving the process
//! entirely — a file icon dragged from the app into a Finder/Explorer
//! window, a snippet of text dragged into another application. There is no
//! render tree on the receiving end to hit test, no shared `WidgetNode`, and
//! the OS itself has to arbitrate rather than this framework's arena.
//!
//! # What is real here versus what is honestly deferred
//!
//! The payload types and the session state machine below are pure data and
//! pure logic — genuinely real, and fully tested with no window, no OS and no
//! GPU in sight, the same way `vieww-gestures`'s recognisers are. Actually
//! **starting** an OS-level drag is a platform API this framework's one
//! windowing backend does not yet expose: `winit` 0.30 (what
//! `vieww-platform-winit` is built on) has no `start_drag`/equivalent call on
//! any platform it supports — see
//! <https://github.com/rust-windowing/winit/issues/1550>, open across every
//! backend as of the version this workspace pins. So [`NullDragStarter`] is
//! not a placeholder standing in for real logic; it is the one honest answer
//! available today, returning [`DragError::NotSupportedByPlatform`] rather
//! than a plausible-looking success that never happened. The moment a
//! windowing backend gains a real `start_drag`, it implements
//! [`PlatformDragStarter`] and every caller above this module is unaffected.

use std::path::PathBuf;

/// What the drag is offering, and in what forms.
///
/// A `Vec` rather than one payload: the receiving application picks whichever
/// form it understands, the same negotiation the OS clipboard already does
/// (offer plain text *and* rich text; a file manager wants
/// [`DragContent::Files`], a text editor wants [`DragContent::Text`], and one
/// drag can honestly satisfy both without this framework guessing which the
/// drop target will turn out to be).
#[derive(Debug, Clone, PartialEq)]
pub struct OutgoingDrag {
    pub forms: Vec<DragContent>,
    /// Which of [`DropEffect`] the source is willing to have happen. A file
    /// manager offering a move (not a copy) sets only
    /// [`DropEffect::Move`] so a target that can only copy
    /// refuses cleanly instead of silently duplicating a file the user meant
    /// to relocate.
    pub allowed_effects: Vec<DropEffect>,
}

impl OutgoingDrag {
    /// A drag offering exactly one form, allowing exactly one effect — the
    /// common case (dragging one piece of text out to be copied elsewhere).
    #[must_use]
    pub fn single(content: DragContent, effect: DropEffect) -> Self {
        Self {
            forms: vec![content],
            allowed_effects: vec![effect],
        }
    }

    /// `true` if `effect` is one this drag permits.
    #[must_use]
    pub fn allows(&self, effect: DropEffect) -> bool {
        self.allowed_effects.contains(&effect)
    }
}

/// One form a dragged payload is offered in.
#[derive(Debug, Clone, PartialEq)]
pub enum DragContent {
    /// Plain text.
    Text(String),
    /// A single URI — a web link, or a `file://` reference to something not
    /// necessarily local. Kept distinct from [`Files`](Self::Files) because a
    /// browser drop target wants a URI even when nothing on disk exists yet.
    Uri(String),
    /// One or more local files or directories, by path — what a file manager
    /// drags.
    Files(Vec<PathBuf>),
    /// Anything else, tagged with a MIME type — the escape hatch for a
    /// custom drag between two applications that agree on a format this list
    /// does not name, the same open-ended registry pattern
    /// `vieww_foundation::capability::platform_view` uses for embedded native
    /// views.
    Custom { mime: String, bytes: Vec<u8> },
}

/// What happened to the dragged content at the drop target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DropEffect {
    /// The target kept a copy; the source's own data is unchanged.
    Copy,
    /// The target took ownership; the source should remove its own copy
    /// (moving a file out of the app's own file browser, say).
    Move,
    /// The target created a reference to the source rather than copying data
    /// (a desktop shortcut, a symlink).
    Link,
}

/// Where an outbound drag is in its lifetime.
///
/// A small explicit state machine rather than a bare `bool` "dragging" flag:
/// [`Self::Finished`] carries *which* effect actually happened, which is the
/// one piece of information the source needs afterward and a boolean cannot
/// hold (a completed move deletes the source's copy; a completed copy does
/// not; a cancelled drag does neither).
#[derive(Debug, Clone, PartialEq)]
pub enum DragSessionState {
    /// No drag in progress.
    Idle,
    /// [`PlatformDragStarter::start_drag`] has been asked to begin one.
    Requested(OutgoingDrag),
    /// The platform confirms the drag is live (the OS is now tracking the
    /// pointer on its own drag machinery, outside this framework's own
    /// pointer routing).
    InProgress(OutgoingDrag),
    /// The drag ended with `effect`, or `None` if it was cancelled (dropped
    /// nowhere that accepted it, or the user pressed Escape).
    Finished(Option<DropEffect>),
}

/// One outbound drag, tracked from request to completion.
#[derive(Debug, Clone, PartialEq)]
pub struct DragSession {
    state: DragSessionState,
}

impl Default for DragSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DragSession {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: DragSessionState::Idle,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &DragSessionState {
        &self.state
    }

    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self.state, DragSessionState::Idle)
    }

    /// Ask `starter` to begin dragging `drag` out of the app.
    ///
    /// # Errors
    /// [`DragError::AlreadyDragging`] if a drag is already in progress —
    /// starting a second native drag session on top of one the OS is already
    /// tracking is undefined on every platform this framework has looked at,
    /// so it is refused here rather than forwarded. Otherwise, whatever
    /// `starter` itself returns (see [`PlatformDragStarter::start_drag`]'s
    /// own doc for [`NullDragStarter`]'s honest answer).
    pub fn start(
        &mut self,
        drag: OutgoingDrag,
        starter: &impl PlatformDragStarter,
    ) -> Result<(), DragError> {
        if !self.is_idle() {
            return Err(DragError::AlreadyDragging);
        }
        self.state = DragSessionState::Requested(drag.clone());
        match starter.start_drag(&drag) {
            Ok(()) => {
                self.state = DragSessionState::InProgress(drag);
                Ok(())
            }
            Err(error) => {
                self.state = DragSessionState::Idle;
                Err(error)
            }
        }
    }

    /// The platform reports the drag ended, with `effect` if it was accepted
    /// somewhere or `None` if it was cancelled.
    ///
    /// A no-op (not a panic) when nothing was in progress — a duplicate or
    /// late-arriving platform "drag ended" notification racing an
    /// application-initiated reset is exactly the kind of platform-boundary
    /// event this framework treats as ignorable rather than fatal, the same
    /// stance `vieww-gestures`'s arena takes toward an event for a pointer it
    /// no longer recognises.
    pub fn finish(&mut self, effect: Option<DropEffect>) {
        if matches!(
            self.state,
            DragSessionState::Requested(_) | DragSessionState::InProgress(_)
        ) {
            self.state = DragSessionState::Finished(effect);
        }
    }

    /// Reset to [`DragSessionState::Idle`], ready for the next drag. Called
    /// once a caller has read and acted on a [`DragSessionState::Finished`]
    /// result (deleted the source file on a completed
    /// [`DropEffect::Move`], say).
    pub fn reset(&mut self) {
        self.state = DragSessionState::Idle;
    }
}

/// Why [`DragSession::start`] did not begin a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragError {
    /// A drag is already in progress on this session.
    AlreadyDragging,
    /// The platform bridge has no way to start a native drag — see this
    /// module's own doc for exactly why, and [`NullDragStarter`].
    NotSupportedByPlatform,
}

impl std::fmt::Display for DragError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyDragging => f.write_str("a drag is already in progress"),
            Self::NotSupportedByPlatform => {
                f.write_str("this platform bridge cannot start a native drag-out session")
            }
        }
    }
}

impl std::error::Error for DragError {}

/// The one platform-specific operation an outbound drag needs: actually
/// telling the OS to start tracking the pointer as a drag of `drag`'s
/// content.
///
/// Implemented by a windowing backend (`vieww-platform-winit`, or whatever
/// replaces/supplements it) once that backend's underlying toolkit exposes
/// the call. See this module's doc for why no implementation ships today.
pub trait PlatformDragStarter {
    /// # Errors
    /// Whatever prevents this platform from starting the drag right now.
    fn start_drag(&self, drag: &OutgoingDrag) -> Result<(), DragError>;
}

/// The only [`PlatformDragStarter`] this workspace ships today: honestly
/// refuses every request. See this module's own doc for why that is
/// correct rather than incomplete.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullDragStarter;

impl PlatformDragStarter for NullDragStarter {
    fn start_drag(&self, _drag: &OutgoingDrag) -> Result<(), DragError> {
        Err(DragError::NotSupportedByPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A starter that always succeeds, standing in for a future real
    /// platform backend so the session state machine can be tested end to
    /// end without one existing yet.
    struct AlwaysStarts;
    impl PlatformDragStarter for AlwaysStarts {
        fn start_drag(&self, _drag: &OutgoingDrag) -> Result<(), DragError> {
            Ok(())
        }
    }

    struct AlwaysFails;
    impl PlatformDragStarter for AlwaysFails {
        fn start_drag(&self, _drag: &OutgoingDrag) -> Result<(), DragError> {
            Err(DragError::NotSupportedByPlatform)
        }
    }

    fn text_drag() -> OutgoingDrag {
        OutgoingDrag::single(DragContent::Text("hello".into()), DropEffect::Copy)
    }

    #[test]
    fn a_fresh_session_is_idle() {
        assert!(DragSession::new().is_idle());
    }

    #[test]
    fn starting_with_a_working_platform_moves_to_in_progress() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysStarts).expect("starts");
        assert!(matches!(session.state(), DragSessionState::InProgress(_)));
    }

    #[test]
    fn starting_with_the_null_starter_is_an_honest_error_not_a_fake_success() {
        let mut session = DragSession::new();
        let result = session.start(text_drag(), &NullDragStarter);
        assert_eq!(result, Err(DragError::NotSupportedByPlatform));
        assert!(
            session.is_idle(),
            "a failed start must not leave the session mid-drag"
        );
    }

    #[test]
    fn a_failed_start_resets_to_idle_rather_than_sticking_in_requested() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysFails).unwrap_err();
        assert!(session.is_idle());
    }

    #[test]
    fn a_second_drag_is_refused_while_one_is_in_progress() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysStarts).unwrap();
        let result = session.start(text_drag(), &AlwaysStarts);
        assert_eq!(result, Err(DragError::AlreadyDragging));
    }

    #[test]
    fn finishing_records_the_effect() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysStarts).unwrap();
        session.finish(Some(DropEffect::Move));
        assert_eq!(
            session.state(),
            &DragSessionState::Finished(Some(DropEffect::Move))
        );
    }

    #[test]
    fn a_cancelled_drag_finishes_with_no_effect() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysStarts).unwrap();
        session.finish(None);
        assert_eq!(session.state(), &DragSessionState::Finished(None));
    }

    #[test]
    fn finishing_an_idle_session_is_a_harmless_no_op() {
        let mut session = DragSession::new();
        session.finish(Some(DropEffect::Copy));
        assert!(
            session.is_idle(),
            "a stray finish notification must not fabricate a drag"
        );
    }

    #[test]
    fn reset_returns_a_finished_session_to_idle() {
        let mut session = DragSession::new();
        session.start(text_drag(), &AlwaysStarts).unwrap();
        session.finish(Some(DropEffect::Copy));
        session.reset();
        assert!(session.is_idle());
    }

    #[test]
    fn allowed_effects_are_checked_not_assumed() {
        let drag = OutgoingDrag::single(DragContent::Text("x".into()), DropEffect::Copy);
        assert!(drag.allows(DropEffect::Copy));
        assert!(
            !drag.allows(DropEffect::Move),
            "a copy-only drag must not also claim to allow a move"
        );
    }

    #[test]
    fn a_drag_can_offer_multiple_forms() {
        let drag = OutgoingDrag {
            forms: vec![
                DragContent::Text("hello.txt".into()),
                DragContent::Uri("file:///tmp/hello.txt".into()),
            ],
            allowed_effects: vec![DropEffect::Copy],
        };
        assert_eq!(drag.forms.len(), 2);
    }
}
