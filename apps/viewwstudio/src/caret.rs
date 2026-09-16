//! Which text field is being typed into, and whether its caret is showing.
//!
//! # The framework hands both of these to the caller, and the studio was
//! dropping them
//!
//! `RenderEditableText::show_cursor` is documented as carrying *two* things:
//! the blink's off phase, **and** the unfocused state. It is deliberately not
//! self-driving — the render object has no clock and no idea which of several
//! fields the user is in.
//!
//! The studio never set it. The consequence, once the find bar and the command
//! palette arrived, was four text fields on screen each painting a solid,
//! motionless caret at all times. Nothing blinked, so nothing looked live; and
//! nothing indicated which field a keystroke would reach. "The cursor does not
//! change when I click on the editor" is what that looks like from the outside
//! — the caret was already drawn, in four places, and clicking changed nothing
//! visible.
//!
//! # Why the active field is inferred rather than observed
//!
//! The honest way would be to ask the framework which render object has focus
//! and map it back to a field. `FrameDriver::focused` gives a `RenderId`, and
//! there is no route from that back to "the find bar's query box" — a widget
//! never learns its own render id, and there is no focus-changed callback to
//! subscribe to. Adding one is a change to `vieww-render`, which this
//! application does not make (plan §6).
//!
//! What *is* knowable is which surface is open, and that answers the question
//! completely for the states the studio can actually be in: the palette takes
//! the keyboard whenever it is up, the find bar takes it when it is opened,
//! and otherwise the editor has it. See [`Active::current`].
//!
//! This is an approximation with a stated boundary rather than a guess: if the
//! user clicks the editor while the find bar is open, the caret blinks in the
//! find box while typing goes to the editor. That case is listed in the plan's
//! known gaps rather than papered over.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::Ticker;

/// How long the caret spends on, and off.
///
/// 530ms is the interval Windows has used since forever and macOS is within a
/// few tens of milliseconds of; it is the rate people read as "a text cursor"
/// rather than as "something flashing".
pub const BLINK: Duration = Duration::from_millis(530);

/// Which of the studio's text fields the keyboard is going to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Active {
    Editor,
    Find,
    Replace,
    Palette,
}

impl Active {
    /// Which field is taking the keyboard, given what is open.
    ///
    /// Ordered innermost-first, and it is the same order [`Studio::escape`]
    /// closes them in — one rule, so the thing Escape dismisses is always the
    /// thing the caret was blinking in.
    ///
    /// [`Studio::escape`]: crate::state::Studio::escape
    #[must_use]
    pub fn current(palette_open: bool, find_open: bool, replacing: bool) -> Self {
        if palette_open {
            Self::Palette
        } else if find_open {
            if replacing {
                Self::Replace
            } else {
                Self::Find
            }
        } else {
            Self::Editor
        }
    }
}

/// The blink, as something a frame can advance.
///
/// A `Ticker` rather than a thread or a timer, for the reason the module docs
/// in `vieww-animation` give: the frame scheduler already runs at the right
/// moment, and a caret that blinks from its own clock blinks a frame out of
/// step with everything else on screen.
#[derive(Debug)]
pub struct Blink {
    /// Whether the caret is in its *on* phase.
    on: bool,
    /// When the current phase started. `None` until the first tick, so the
    /// first frame does not measure against a zero that is not a real time.
    since: Option<Duration>,
}

impl Blink {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            on: true,
            since: None,
        }
    }

    /// Whether the caret should be painted this frame.
    #[must_use]
    pub const fn is_on(&self) -> bool {
        self.on
    }

    /// Put the caret back in its *on* phase and restart the interval.
    ///
    /// Called on every edit and every caret move, because a caret that happens
    /// to be in its off phase when you type looks like the keystroke was
    /// dropped. Every editor does this; it is why typing never makes the cursor
    /// vanish.
    pub const fn wake(&mut self) {
        self.on = true;
        self.since = None;
    }
}

impl Default for Blink {
    fn default() -> Self {
        Self::new()
    }
}

impl Ticker for Blink {
    fn tick(&mut self, now: Duration) -> bool {
        let Some(since) = self.since else {
            self.since = Some(now);
            return false;
        };
        // Saturating, because a `now` that went backwards — a clock adjustment,
        // a test driving frames out of order — must not panic in the one place
        // that runs every single frame.
        if now.saturating_sub(since) < BLINK {
            return false;
        }
        self.since = Some(now);
        self.on = !self.on;
        true
    }

    /// Always. A caret blinks for as long as there is one, which is why this is
    /// the one ticker in the studio that never settles — and why
    /// [`Studio::blink_enabled`] exists to turn it off.
    ///
    /// [`Studio::blink_enabled`]: crate::state::Studio::blink_enabled
    fn is_animating(&self) -> bool {
        true
    }

    /// The next toggle — which is the whole reason a deadline exists.
    ///
    /// # The defect this closes
    ///
    /// `is_animating` above answers `true` for ever, and for a long time that
    /// was everything the event loop could learn. "Something is animating" and
    /// "something is animating *right now*" were one sentence, so a window with
    /// a caret in it and nothing else happening drew frames as fast as it
    /// could: **one core at 100%, indefinitely**, measured on an untouched
    /// window and pinned as a known defect by `tests/idle_cost.rs`.
    ///
    /// A blink is not continuous. It changes at `since + BLINK` and at no other
    /// moment, so saying so turns an idle editor from sixty frames a second
    /// into two — and `ControlFlow::WaitUntil` sleeps through the gap.
    ///
    /// `None` before the first tick, which means "as soon as possible": the
    /// phase has no start time yet, and the frame that establishes one is the
    /// frame being asked about.
    fn next_deadline(&self, _now: Duration) -> Option<Duration> {
        self.since.map(|since| since + BLINK)
    }
}

/// A [`Blink`] shared between the tickers and the widgets that read it.
pub type SharedBlink = Rc<RefCell<Blink>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_active_field_follows_what_is_open() {
        assert_eq!(Active::current(false, false, false), Active::Editor);
        assert_eq!(Active::current(false, true, false), Active::Find);
        assert_eq!(Active::current(false, true, true), Active::Replace);
        assert_eq!(
            Active::current(true, true, true),
            Active::Palette,
            "the palette is over everything, so it has the keyboard"
        );
    }

    #[test]
    fn the_blink_alternates_on_the_interval_and_not_before() {
        let mut blink = Blink::new();
        assert!(blink.is_on(), "a caret starts visible");

        // The first tick only establishes a baseline.
        assert!(!blink.tick(Duration::from_secs(10)));
        assert!(blink.is_on());

        // Short of the interval, nothing happens.
        assert!(!blink.tick(Duration::from_secs(10) + BLINK / 2));
        assert!(blink.is_on());

        assert!(blink.tick(Duration::from_secs(10) + BLINK));
        assert!(!blink.is_on(), "off phase");

        assert!(blink.tick(Duration::from_secs(10) + BLINK * 2));
        assert!(blink.is_on(), "and back");
    }

    #[test]
    fn typing_wakes_the_caret() {
        let mut blink = Blink::new();
        blink.tick(Duration::from_secs(1));
        blink.tick(Duration::from_secs(1) + BLINK);
        assert!(!blink.is_on(), "mid off-phase");

        blink.wake();
        assert!(
            blink.is_on(),
            "a keystroke that lands while the caret is invisible reads as a \
             dropped keystroke"
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_does_not_panic() {
        let mut blink = Blink::new();
        blink.tick(Duration::from_secs(100));
        // A clock adjustment between two frames. `Duration` subtraction would
        // panic here, and this runs every frame.
        assert!(!blink.tick(Duration::from_secs(1)));
    }
}
