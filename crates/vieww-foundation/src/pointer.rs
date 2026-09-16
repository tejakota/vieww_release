//! Raw pointer input, and what a recognised gesture reports.
//!
//! # Why these live in foundation
//!
//! The recognisers are in `vieww-gestures` and the widgets that react to them
//! are in `vieww-widget`, and neither may depend on the other — a
//! `GestureDetector` is a description, and descriptions do not know how a tap is
//! disambiguated from a drag. So the vocabulary they meet on is here, for the
//! same reason [`GlyphRun`](crate::GlyphRun) is: it is the handoff between two
//! layers that sit beside each other.
//!
//! # What a platform has to supply
//!
//! [`PointerEvent`] is deliberately close to what every touch API already
//! reports — an id, a phase, a position and a timestamp. A platform bridge
//! translates its own events into these and hands them to the framework; nothing
//! above this layer knows what an Android `MotionEvent` is.

use std::fmt;
use std::time::Duration;

use crate::{Modifiers, Offset};

/// Identifies one finger, stylus or mouse button across its whole lifetime.
///
/// A pointer is born at [`PointerPhase::Down`] and dies at `Up` or `Cancel`; the
/// same id may be reused for a later touch. Recognisers key everything on this,
/// because two fingers on the screen are two independent sequences that happen to
/// interleave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PointerId(pub u64);

impl fmt::Display for PointerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// What kind of device produced a pointer.
///
/// Recognisers use this for thresholds rather than for behaviour: a mouse is
/// precise and a finger is not, so the slop a tap tolerates differs by an order
/// of magnitude between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerDeviceKind {
    #[default]
    Touch,
    Mouse,
    Stylus,
}

impl PointerDeviceKind {
    /// How far a pointer may move and still count as stationary.
    ///
    /// The mainstream touch-slop value is 18 logical pixels, and the number is not
    /// arbitrary: a finger covers a large area and its reported centre wanders by
    /// several pixels while "holding still". A mouse does not, and giving it the
    /// same tolerance makes small drags feel dead.
    #[must_use]
    pub const fn touch_slop(self) -> f32 {
        match self {
            Self::Touch => 18.0,
            Self::Stylus => 8.0,
            Self::Mouse => 2.0,
        }
    }
}

/// Which button a pointer represents.
///
/// Only meaningful for a mouse. A finger and a stylus are always
/// [`Primary`](Self::Primary) — there is no second finger *of the same finger*,
/// and a touch API that reported otherwise would be describing a different
/// pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerButton {
    /// Left, or the whole of a touch.
    #[default]
    Primary,
    /// Right. What opens a context menu.
    Secondary,
    /// The wheel pressed as a button. Rare, and carried rather than dropped so
    /// that an application that wants it is not blocked on a framework change.
    Middle,
    /// The thumb button that means "go back" on the mice that have one.
    ///
    /// Carried, not interpreted. It is tempting to have the framework pop the
    /// navigator itself, and that is wrong for the same reason
    /// [`Middle`](Self::Middle) is merely carried: **the meaning belongs to the
    /// application**. A screen with unsaved edits, a modal, a wizard halfway
    /// through — each wants something different from "back", and a framework
    /// that decided would be overridden everywhere it mattered and obeyed
    /// exactly where it did harm.
    ///
    /// A widget hears it through `GestureDetector::on_back_tap`, which gets its
    /// own recogniser and its own arena entry — carrying the button on the
    /// event is not on its own enough to make it reachable, because a gesture
    /// nothing recognises is a gesture nothing is ever told about.
    Back,
    /// The thumb button that means "go forward". See [`Back`](Self::Back).
    Forward,
}

/// Where a pointer is in its lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PointerPhase {
    /// First contact. This is the only phase that hit tests — everything after
    /// it is routed to whatever the down found, even once it has moved outside.
    Down,
    /// Contact moved.
    Move,
    /// Contact lifted normally.
    Up,
    /// The platform took the pointer away: a call arrived, the app was
    /// backgrounded, a system gesture claimed it.
    ///
    /// Distinct from `Up` and never interchangeable with it. A cancel must not
    /// fire a tap, and a recogniser that treats the two alike will fire gestures
    /// when the user switches apps.
    Cancel,
}

impl PointerPhase {
    /// `true` if the pointer ceases to exist after this event.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Up | Self::Cancel)
    }
}

/// One raw input event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub pointer: PointerId,
    pub phase: PointerPhase,
    /// Position in global (surface) coordinates.
    pub position: Offset,
    /// Movement since this pointer's previous event. Zero on `Down`.
    pub delta: Offset,
    /// When the platform observed the event.
    ///
    /// Supplied rather than read from a clock here, because velocity computed
    /// from arrival time is wrong by however long the event queued — which is
    /// exactly when it matters, since a fling is measured from the last few
    /// events before release.
    pub timestamp: Duration,
    pub kind: PointerDeviceKind,
    /// Which button this pointer is.
    ///
    /// Carried on every event rather than only on the press, because a
    /// recogniser decides whether it [`wants`] a pointer at the moment it goes
    /// down and must be able to ask — and because a caller that has an event in
    /// hand should never have to go looking for the press that started it.
    ///
    /// [`wants`]: https://docs.rs/vieww-gestures
    pub button: PointerButton,
    /// The keyboard modifiers held when the platform observed this event.
    ///
    /// # Why a pointer event carries keys
    ///
    /// Because half of what a pointer means depends on them, and none of it can
    /// be recovered later. ⇧-click extends a selection, ⌘-click adds one, and
    /// ⌥-drag copies rather than moves — three behaviours that every list, tree
    /// and text field is expected to have and that no application on vieww
    /// could implement, because by the time a gesture reached a widget the keys
    /// were gone.
    ///
    /// Read from the platform's own modifier state at the moment of the event
    /// rather than from a keyboard poll at the point of use, for the reason
    /// [`timestamp`](Self::timestamp) gives: an event queued behind a slow
    /// frame must be interpreted with the keys that were down when the finger
    /// moved, not the ones held by the time the program got round to looking.
    ///
    /// `NONE` on a platform that does not report them, and on every touch
    /// device, which is why nothing may *require* a modifier to be reachable.
    pub modifiers: Modifiers,
    /// How hard a stylus or a force-sensitive touch surface is being pressed,
    /// `0.0..=1.0`.
    ///
    /// `None` rather than a default of `0.0` or `1.0`, and deliberately not
    /// merged into [`PointerDeviceKind`]: a plain mouse or an ordinary finger
    /// has no pressure at all, not a pressure of "fully down", and a recogniser
    /// that treated the two alike would make every pressure-sensitive brush
    /// stroke start at whatever "fully down" happened to map to instead of
    /// wherever the finger actually started. Carried for an application to use
    /// (a painting canvas widening its stroke, a 3D-Touch-style peek) — this
    /// framework's own recognisers do not read it, the same stance
    /// [`PointerButton::Middle`] documents for the same reason: **the meaning
    /// belongs to the application**.
    pub pressure: Option<f32>,
    /// How far a stylus is tilted from vertical, `(x, y)` in degrees, each
    /// `-90.0..=90.0`. `x` is tilt away from vertical along the surface's own
    /// horizontal axis, `y` along its vertical axis — the same two-angle shape
    /// every stylus API already reports (`altitude`/`azimuth` on iOS,
    /// `TILT_X`/`TILT_Y` on Android), so a platform bridge translates rather
    /// than derives it. `None` on every device that is not a stylus reporting
    /// tilt, which today is every device this framework's own platform bridges
    /// translate — see [`Self::tilt`]'s doc for exactly what is and is not
    /// wired up yet.
    pub tilt: Option<(f32, f32)>,
    /// Rotation of a stylus around its own long axis, in degrees, `0.0..360.0`.
    /// `None` on every device that does not report it — most styluses,
    /// including every one whose input reaches this framework's platform
    /// bridges today; see [`Self::tilt`].
    pub twist: Option<f32>,
}

impl PointerEvent {
    /// A `Down` at `position`.
    #[must_use]
    pub const fn down(pointer: PointerId, position: Offset, timestamp: Duration) -> Self {
        Self {
            pointer,
            phase: PointerPhase::Down,
            position,
            delta: Offset::ZERO,
            timestamp,
            kind: PointerDeviceKind::Touch,
            button: PointerButton::Primary,
            // The platform layer fills these in; a constructor here is for
            // tests and for synthesised events, where nothing is held.
            modifiers: Modifiers::NONE,
            pressure: None,
            tilt: None,
            twist: None,
        }
    }

    /// A `Move` to `position`, having come from `previous`.
    #[must_use]
    pub fn moved(
        pointer: PointerId,
        previous: Offset,
        position: Offset,
        timestamp: Duration,
    ) -> Self {
        Self {
            pointer,
            phase: PointerPhase::Move,
            position,
            delta: position - previous,
            timestamp,
            kind: PointerDeviceKind::Touch,
            button: PointerButton::Primary,
            // The platform layer fills these in; a constructor here is for
            // tests and for synthesised events, where nothing is held.
            modifiers: Modifiers::NONE,
            pressure: None,
            tilt: None,
            twist: None,
        }
    }

    /// An `Up` at `position`.
    #[must_use]
    pub const fn up(pointer: PointerId, position: Offset, timestamp: Duration) -> Self {
        Self {
            pointer,
            phase: PointerPhase::Up,
            position,
            delta: Offset::ZERO,
            timestamp,
            kind: PointerDeviceKind::Touch,
            button: PointerButton::Primary,
            // The platform layer fills these in; a constructor here is for
            // tests and for synthesised events, where nothing is held.
            modifiers: Modifiers::NONE,
            pressure: None,
            tilt: None,
            twist: None,
        }
    }

    /// A `Cancel` at `position`.
    #[must_use]
    pub const fn cancel(pointer: PointerId, position: Offset, timestamp: Duration) -> Self {
        Self {
            pointer,
            phase: PointerPhase::Cancel,
            position,
            delta: Offset::ZERO,
            timestamp,
            kind: PointerDeviceKind::Touch,
            button: PointerButton::Primary,
            // The platform layer fills these in; a constructor here is for
            // tests and for synthesised events, where nothing is held.
            modifiers: Modifiers::NONE,
            pressure: None,
            tilt: None,
            twist: None,
        }
    }

    /// The same event as a different device kind.
    #[must_use]
    pub const fn with_kind(mut self, kind: PointerDeviceKind) -> Self {
        self.kind = kind;
        self
    }

    /// The same event with the keyboard modifiers that were held.
    ///
    /// Stamped by the platform layer, in one place, on the way out — see
    /// [`modifiers`](Self::modifiers).
    #[must_use]
    pub const fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// The same event as a different button.
    #[must_use]
    pub const fn with_button(mut self, button: PointerButton) -> Self {
        self.button = button;
        self
    }

    /// `true` for the button every ordinary gesture is made with.
    #[must_use]
    pub const fn is_primary(&self) -> bool {
        matches!(self.button, PointerButton::Primary)
    }

    /// The same event with a stylus's pressure recorded. See
    /// [`Self::pressure`].
    #[must_use]
    pub const fn with_pressure(mut self, pressure: f32) -> Self {
        self.pressure = Some(pressure);
        self
    }

    /// The same event with a stylus's tilt recorded. See [`Self::tilt`].
    #[must_use]
    pub const fn with_tilt(mut self, tilt_x: f32, tilt_y: f32) -> Self {
        self.tilt = Some((tilt_x, tilt_y));
        self
    }

    /// The same event with a stylus's twist recorded. See [`Self::twist`].
    #[must_use]
    pub const fn with_twist(mut self, twist: f32) -> Self {
        self.twist = Some(twist);
        self
    }
}

/// A wheel or trackpad scroll.
///
/// Not a [`PointerEvent`]: a scroll has no lifetime, no id and no contact. It
/// happens *at* a position rather than travelling from one, so it is hit tested
/// where it lands rather than routed to whatever a press found — which is why it
/// cannot go through the gesture arena at all. Two wheel notches over two
/// different lists scroll two different lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollEvent {
    /// Where the cursor was, in global (surface) coordinates.
    pub position: Offset,
    /// How far to scroll the *content*, in logical pixels.
    ///
    /// Already resolved from whatever the platform reported: a wheel that
    /// reports lines rather than pixels is converted by the platform bridge,
    /// because only it knows how big a line is on that platform.
    ///
    /// **Finger-equivalent**: the movement a drag would have made to produce the
    /// same scroll, in the same screen-space convention as
    /// [`DragDetails::delta`]. So a negative `dy` — a finger travelling *up* the
    /// screen — moves further into the content, exactly as it does for a drag,
    /// and a wheel notch can be handed to a drag handler unchanged.
    ///
    /// This wording used to say "positive `dy` scrolls content up — the
    /// direction a finger would drag it", which is two opposite claims in one
    /// sentence: a finger dragging content up travels *up* the screen, and that
    /// is negative. The platform bridge negated to satisfy the first half, which
    /// put wheels in the reverse convention to drags and made every wheel scroll
    /// run backwards. Only the second half was ever the intent, because it is
    /// the one that lets the two share a handler.
    pub delta: Offset,
    pub timestamp: Duration,
}

impl ScrollEvent {
    #[must_use]
    pub const fn new(position: Offset, delta: Offset, timestamp: Duration) -> Self {
        Self {
            position,
            delta,
            timestamp,
        }
    }
}

/// A completed tap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TapDetails {
    /// Where the finger came down, in global coordinates.
    pub position: Offset,
    /// The same point, local to the widget that is being told about it.
    pub local: Offset,
    /// Which button made it.
    ///
    /// Carried on the gesture rather than left to the recogniser, because a
    /// widget can register a recogniser per button and they all report the same
    /// *kind* of gesture — without this, whatever routes them has no way to tell
    /// a right-click from a left one.
    pub button: PointerButton,
    /// When the press happened, from the platform's own clock.
    ///
    /// # Why a tap carries a time
    ///
    /// Because a *repeat* click is a tap plus a time. Double-click-to-select-a-
    /// word and triple-click-to-select-a-line are counted by asking whether the
    /// previous tap was recent and nearby, and a tap with no time leaves only
    /// "nearby" — which turns two deliberate clicks in the same spot, a minute
    /// apart, into a word selection.
    ///
    /// Read from the event rather than from a clock at the point of use, for
    /// the same reason [`PointerEvent::timestamp`] is: a gesture queued behind
    /// a slow frame is measured from when the finger moved, not from when the
    /// program got round to looking.
    pub timestamp: Duration,
    /// The modifiers held when the press happened.
    ///
    /// Carried from [`PointerEvent::modifiers`] by the recogniser, so a widget
    /// can tell a plain click from a ⇧-click without knowing that a keyboard
    /// exists. See that field for why the value is the one from the event
    /// rather than one read when the handler runs.
    pub modifiers: Modifiers,
}

impl TapDetails {
    /// A primary tap at `position`, not yet localised.
    #[must_use]
    pub const fn at(position: Offset) -> Self {
        Self {
            timestamp: Duration::ZERO,
            position,
            local: position,
            button: PointerButton::Primary,
            modifiers: Modifiers::NONE,
        }
    }
}

/// A drag beginning, continuing or ending.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragDetails {
    /// The pointer's current position, in global coordinates.
    pub position: Offset,
    /// The same point, local to the widget being told.
    pub local: Offset,
    /// Movement since the previous update. Zero at start and end.
    pub delta: Offset,
    /// Pixels per second at release. Only meaningful on end, where it is what a
    /// fling is simulated from.
    pub velocity: Offset,
    /// When the event that produced this was observed.
    ///
    /// Carried because a release velocity is only half of what a simulation
    /// needs: a spring launched from a drag has to start at the moment the
    /// finger left, not at whatever the next frame's timestamp happens to be.
    /// Reading a clock in the handler instead would date the animation from
    /// when the event was *processed*, which is exactly the interval that gets
    /// long when the system is busy.
    pub timestamp: Duration,
    /// The modifiers held. Taken from the event that produced this update, so a
    /// ⌥ pressed or released mid-drag is seen on the update it happened.
    pub modifiers: Modifiers,
}

/// A press held long enough to count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongPressDetails {
    pub position: Offset,
    pub local: Offset,
}

/// A two-finger pinch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleDetails {
    /// The midpoint between the pointers, in global coordinates — the point the
    /// gesture should be treated as zooming about.
    pub focal: Offset,
    /// Current spread over the spread when the gesture started. `1.0` at start.
    pub scale: f32,
    /// How much the focal point has moved since the last update.
    pub focal_delta: Offset,
}

/// How long a press must be held before it is a long press.
///
/// Android and most touch toolkits use 500ms. Shorter and a slow tap fires one;
/// longer and it stops feeling like a response to the finger.
pub const LONG_PRESS_TIMEOUT: Duration = Duration::from_millis(500);

/// How long after a `Down` a tap may still arrive.
///
/// A press held past this is no longer a tap, whatever happens next. This is what
/// stops a long press from also firing a tap when the finger finally lifts.
pub const TAP_TIMEOUT: Duration = Duration::from_millis(300);

/// How long a press is held before anything is allowed to look pressed.
///
/// The delay is the whole point, and it is not a debounce. A press that is about
/// to become a scroll produces a `Down` first and a `Move` a frame or two later,
/// so a control that highlighted on the `Down` flashes under every finger that
/// flicks past it — the artefact every list of buttons has and nobody wants.
/// Waiting this long means the finger has already declared itself.
///
/// Winning the arena reports the press immediately regardless, so a tap faster
/// than this is not silently swallowed. The familiar press timeout, same number.
pub const PRESS_TIMEOUT: Duration = Duration::from_millis(100);

/// How long after a tap a second one still counts as a repeat.
///
/// What separates a double-click from two clicks. 500ms is the interval Windows
/// exposes as its default and macOS and GTK sit within a few tens of
/// milliseconds of; it is also, conveniently, the number a person who has never
/// thought about it will produce when asked to "double-click".
pub const MULTI_TAP_TIMEOUT: Duration = Duration::from_millis(500);

/// How far a repeat tap may land from the one before it and still count.
///
/// Distance as well as time, because two quick clicks at opposite ends of a
/// paragraph are two clicks. Generous enough to tolerate a hand that moves a
/// couple of pixels between presses, which every hand does.
pub const MULTI_TAP_SLOP: f32 = 12.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_move_carries_the_delta_from_its_previous_position() {
        let event = PointerEvent::moved(
            PointerId(1),
            Offset::new(10.0, 10.0),
            Offset::new(14.0, 13.0),
            ms(16),
        );
        assert_eq!(event.delta, Offset::new(4.0, 3.0));
        assert_eq!(event.position, Offset::new(14.0, 13.0));
    }

    #[test]
    fn cancel_is_terminal_but_is_not_an_up() {
        assert!(PointerPhase::Cancel.is_terminal());
        assert!(PointerPhase::Up.is_terminal());
        assert!(!PointerPhase::Down.is_terminal());
        assert_ne!(
            PointerPhase::Cancel,
            PointerPhase::Up,
            "a cancel must never fire a tap; conflating them fires gestures when \
             the user switches apps"
        );
    }

    #[test]
    fn a_finger_is_allowed_to_wander_much_further_than_a_mouse() {
        assert!(
            PointerDeviceKind::Touch.touch_slop() > PointerDeviceKind::Mouse.touch_slop() * 5.0,
            "a finger's reported centre wanders by several pixels while holding \
             still; a mouse's does not"
        );
    }
}

/// The shape the pointing device draws while it is over something.
///
/// # Why this is a small, closed list
///
/// Every desktop platform has dozens of cursors and they do not agree on which
/// ones exist, what they look like, or what they mean. What a UI framework
/// needs is the handful whose *meaning* is the same everywhere — "you can type
/// here", "you can drag this edge", "this is a link", "wait" — because those
/// are the ones a widget can ask for and expect to be understood.
///
/// A widget that needs something outside this list is asking for a platform
/// affordance rather than a UI one, and the honest answer is to grow the list
/// deliberately rather than to expose an escape hatch that means something
/// different on each OS.
///
/// Touch platforms have no pointer to shape, so every value is a no-op there.
/// That is not a gap: a `Text` widget asking for [`Cursor::Text`] on a phone is
/// asking for nothing, correctly, without having to know what it is running on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Cursor {
    /// The ordinary arrow.
    #[default]
    Default,
    /// An I-beam: text that can be selected or edited.
    Text,
    /// A hand: something that will navigate if clicked.
    Pointer,
    /// A four-way move.
    Move,
    /// Horizontal resize, for a vertical split.
    ResizeColumn,
    /// Vertical resize, for a horizontal split.
    ResizeRow,
    /// Something is happening and input is not being taken.
    Wait,
    /// This target will not accept what is being dragged.
    NotAllowed,
    /// A crosshair, for precise selection.
    Crosshair,
    /// An open hand, over something grabbable.
    Grab,
    /// A closed hand, while it is being dragged.
    Grabbing,
}

impl Cursor {
    /// The CSS name for this shape.
    ///
    /// Every desktop cursor API in use — winit's `CursorIcon`, the web's
    /// `cursor` property, GTK's names — is either CSS's list or a superset of
    /// it, so this is the closest thing to a portable identifier there is, and
    /// it makes the mapping in a backend a lookup rather than a judgement.
    #[must_use]
    pub const fn css_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Text => "text",
            Self::Pointer => "pointer",
            Self::Move => "move",
            Self::ResizeColumn => "col-resize",
            Self::ResizeRow => "row-resize",
            Self::Wait => "wait",
            Self::NotAllowed => "not-allowed",
            Self::Crosshair => "crosshair",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
        }
    }
}
