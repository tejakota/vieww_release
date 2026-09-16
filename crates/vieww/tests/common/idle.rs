//! "Does this widget stop asking for frames?", as one harness.
//!
//! `docs/AIMS.md` §F asks that **an idle tree cost nothing** — asserted, not
//! assumed — and says the pattern in `pressed_state.rs` and `drag_to_dismiss.rs`
//! "should cover every widget that animates or measures". This is that pattern,
//! in one place, so that covering a twenty-seventh widget is one line rather
//! than a copied loop.
//!
//! # "Settled" is two questions, and the hand-written version asked one
//!
//! Every hand-rolled copy of this checked that
//! [`needs_frame`](FrameDriver::needs_frame) had gone false. That is necessary
//! and not sufficient: a widget that re-arms every frame answers `false` for
//! exactly as long as it takes to ask again, so a loop that stops the moment it
//! sees `false` reports a spinning widget as settled.
//!
//! [`Idle::settle`] therefore runs frames until it stops, and **then runs
//! [`GRACE`] more and fails if it starts again**. That is the half that catches
//! the interesting bug, and it is the half nobody writes by hand.
//!
//! # Animating forever is a legitimate answer, and has to be declared
//!
//! An indeterminate progress indicator never settles, and must not. So the
//! harness has two entry points — [`Idle::settle`] and [`Idle::assert_animates`]
//! — and the second *fails if the widget stops*.
//!
//! What no widget is allowed to be is **undecided**. Before this, none of them
//! were checked either way.

// A test binary has no downstream crate, so `pub` here is unreachable by
// construction and `dead_code` fires for whichever half of the harness the
// including binary does not use. Both warnings are about the harness being a
// harness rather than about anything wrong.
#![allow(dead_code, unreachable_pub)]

use std::time::Duration;

use vieww::foundation::Size;
use vieww::prelude::WidgetNode;
use vieww::FrameDriver;

/// How long a frame is taken to be. 60Hz, because the number only has to be
/// plausible and consistent — nothing here measures wall-clock cost.
pub const FRAME: Duration = Duration::from_millis(16);

/// How many quiet frames after `needs_frame` goes false before a widget is
/// believed.
///
/// Four rather than one, because the failure this exists to catch is a widget
/// that re-arms *on the next frame* — which a single confirming frame sees and a
/// zero-frame check does not. Four is enough for a re-arm driven by a two- or
/// three-phase cycle, and cheap enough to run on every widget in the catalogue.
pub const GRACE: u32 = 4;

/// How many frames a widget is given to settle before it is called stuck.
///
/// Generous: the longest deliberate animation in the framework is well inside
/// this, so anything reaching it is spinning rather than slow.
pub const LIMIT: u32 = 600;

/// A driver showing one widget, driven by a clock this owns.
pub struct Idle {
    driver: FrameDriver,
    now: Duration,
    frames: u32,
}

impl Idle {
    /// A surface of `size` showing `widget`, with one frame already drawn.
    ///
    /// The first frame is drawn here because "does it ask for another" is not a
    /// meaningful question about a tree that has never been built.
    pub fn showing(size: Size, widget: impl Into<WidgetNode>) -> Self {
        let mut driver = FrameDriver::new(size);
        driver.set_root(widget);
        let mut idle = Self {
            driver,
            now: Duration::ZERO,
            frames: 0,
        };
        idle.step();
        idle
    }

    /// A 400×400 surface, which fits every widget in the catalogue.
    pub fn new(widget: impl Into<WidgetNode>) -> Self {
        Self::showing(Size::new(400.0, 400.0), widget)
    }

    /// The driver, for a test that needs to poke the tree between frames.
    pub fn driver(&mut self) -> &mut FrameDriver {
        &mut self.driver
    }

    /// How many frames have been drawn.
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// Draw one frame, advancing the clock.
    pub fn step(&mut self) {
        self.driver.draw_frame_at(self.now);
        self.now += FRAME;
        self.frames += 1;
    }

    /// Draw `count` frames.
    pub fn steps(&mut self, count: u32) {
        for _ in 0..count {
            self.step();
        }
    }

    /// `true` if the widget wants another frame right now.
    pub fn wants_frame(&self) -> bool {
        self.driver.needs_frame()
    }

    /// Run until the tree stops asking for frames, then check it stays stopped.
    ///
    /// Returns how many frames it took to settle, so a caller can also assert
    /// *how long* an animation ran where that is the point.
    ///
    /// # Panics
    ///
    /// If the tree is still asking after [`LIMIT`] frames, or if it goes quiet
    /// and then starts again within [`GRACE`] frames. The second message names
    /// the re-arm explicitly, because a test that only said "did not settle"
    /// sends the reader looking at the wrong thing.
    pub fn settle(&mut self) -> u32 {
        let start = self.frames;
        while self.wants_frame() {
            if self.frames - start > LIMIT {
                panic!(
                    "still asking for frames after {LIMIT} — this widget never \
                     settles. If that is deliberate, it must say so with \
                     `assert_animates` rather than being left undecided."
                );
            }
            self.step();
        }
        let settled_after = self.frames - start;

        for _ in 0..GRACE {
            self.step();
            assert!(
                !self.wants_frame(),
                "settled after {settled_after} frames and then asked for another \
                 {} frames later — it is re-arming, not idle. A widget that \
                 re-arms every frame answers `needs_frame() == false` for exactly \
                 as long as it takes to ask again, which is why this check exists \
                 and why a bare `assert!(!needs_frame())` passes on it.",
                self.frames - start - settled_after
            );
        }
        settled_after
    }

    /// The other half: fail if the widget ever *stops* asking.
    ///
    /// For the indeterminate indicators, whose whole contract is that they keep
    /// going. Without this they would be indistinguishable from a widget that
    /// settles, and "settles" is the answer this file exists to reward.
    ///
    /// # Panics
    ///
    /// If the tree goes quiet within `frames`.
    pub fn assert_animates(&mut self, frames: u32) {
        for drawn in 0..frames {
            assert!(
                self.wants_frame(),
                "stopped asking for frames after {drawn} — an indeterminate \
                 animation that stops is a spinner that freezes on screen."
            );
            self.step();
        }
    }
}
