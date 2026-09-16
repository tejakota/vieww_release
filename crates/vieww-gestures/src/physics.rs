//! What a list does after the finger leaves it.
//!
//! # Two conventions, and why this is a branch rather than a platform crate
//!
//! Android decelerates to a stop and **clamps** at the edge; iOS decelerates
//! differently and **bounces** past it, springing back. Both platforms can do
//! either — this is a preference, not a capability — which is exactly the test
//! `docs/DESIGN.md` §8 sets for what may branch on [`TargetPlatform`]. And per
//! that section every such branch must be overridable per widget, so
//! [`ScrollPhysics`] is a value a caller can construct directly rather than
//! something read from the platform behind their back.
//!
//! # Where the simulations live
//!
//! [`Fling`] and [`Spring`] were written here and moved to `vieww-animation` in
//! Phase 7, because an `AnimationController` wants the same two and a dependency
//! from the animation layer onto the gesture layer would have been backwards.
//! They are re-exported, so a caller sees no difference. What stays here is the
//! part that is about *scrolling* specifically: the two platform conventions, the
//! overscroll mapping, and [`ScrollPosition`].
//!
//! Both are closed form — every simulation answers "where is it at time *t*"
//! directly. Stepping a velocity by a frame delta instead makes the result depend
//! on frame rate: a fling travels a different distance on a 60Hz and a 120Hz
//! screen, and a dropped frame shortens the throw. It also means a scroll
//! position can be sampled at whatever moment the frame actually lands, rather
//! than assuming it landed on time.

use std::time::Duration;

use vieww_animation::{Fling, Spring};
use vieww_foundation::TargetPlatform;

/// Deceleration applied to a fling, per second.
///
/// The value is the fraction of its speed a fling keeps after one second, so
/// smaller is stickier. Android's scroller is stickier than iOS's, which is most
/// of why the two feel different with identical input.
const ANDROID_DRAG: f32 = 0.000_015_5;
const IOS_DRAG: f32 = 0.000_135;

/// What happens at the end of the scrollable range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overscroll {
    /// Stop dead at the edge. Android's convention.
    Clamp,
    /// Go past it and spring back. Apple's.
    Bounce,
}

/// The feel of a scrollable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollPhysics {
    /// Fraction of its velocity a fling retains after one second.
    pub drag: f32,
    pub overscroll: Overscroll,
    /// How hard the spring pulls a bounced scroll back, per second.
    pub spring_stiffness: f32,
}

impl ScrollPhysics {
    /// Android's feel.
    #[must_use]
    pub const fn android() -> Self {
        Self {
            drag: ANDROID_DRAG,
            overscroll: Overscroll::Clamp,
            spring_stiffness: 0.0,
        }
    }

    /// Apple's feel.
    #[must_use]
    pub const fn ios() -> Self {
        Self {
            drag: IOS_DRAG,
            overscroll: Overscroll::Bounce,
            spring_stiffness: 12.0,
        }
    }

    /// The convention of the platform this build targets.
    ///
    /// Resolved at compile time. A widget that wants the other one says so
    /// explicitly — see the module docs for why that has to be possible.
    #[must_use]
    pub const fn platform_default() -> Self {
        if TargetPlatform::current().is_apple() {
            Self::ios()
        } else {
            Self::android()
        }
    }

    /// Simulate a fling released at `velocity` from `start`.
    #[must_use]
    pub fn fling(&self, start: f32, velocity: f32) -> Fling {
        Fling::new(start, velocity, self.drag)
    }

    /// `true` if dragging past an edge shows anything at all.
    ///
    /// The question [`ScrollPosition`] has to ask before deciding whether to
    /// keep accumulating past the end: under a physics that does not stretch,
    /// the accumulated part maps to no pixels and is pure dead travel.
    #[must_use]
    pub const fn stretches(&self) -> bool {
        matches!(self.overscroll, Overscroll::Bounce)
    }

    /// How far past the edge the content actually goes, for a finger that has
    /// dragged `raw` past it.
    ///
    /// Under [`Overscroll::Clamp`], nowhere: the list is at the end and stays
    /// there. Under `Bounce` it follows at first and then visibly refuses,
    /// approaching `extent` however hard it is pulled — which is what makes an
    /// iOS list feel elastic rather than broken.
    ///
    /// # Why a mapping and not a per-event factor
    ///
    /// The obvious version scales each incoming delta by the resistance at the
    /// moment it arrives. That makes the result depend on **how many move events
    /// arrived**, so the same physical drag overscrolls differently at 60Hz and
    /// at 120Hz, and differently again on a frame where events coalesced.
    /// Mapping the accumulated distance instead has no such dependence.
    #[must_use]
    pub fn stretch(&self, raw: f32, extent: f32) -> f32 {
        match self.overscroll {
            Overscroll::Clamp => 0.0,
            Overscroll::Bounce => {
                let extent = extent.max(1.0);
                let magnitude = raw.abs();
                raw.signum() * extent * magnitude / (magnitude + extent)
            }
        }
    }
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        Self::platform_default()
    }
}

/// Where a scrollable is, and what it does when dragged or flung.
///
/// The piece that makes the rest of this module load-bearing rather than a
/// library of curves nobody calls. A widget owns one of these, hands it drag
/// deltas and release velocities, and asks it for an offset each frame.
///
/// Offsets are positive **into** the content: zero is the top, and
/// [`max_offset`](Self::max_offset) is as far as it goes.
#[derive(Debug, Clone)]
pub struct ScrollPosition {
    /// Where the content actually sits, resistance included.
    offset: f32,
    /// Where it would sit if the edges were not there — what the finger has
    /// actually asked for, in total.
    ///
    /// Kept separately so that overscroll is a *mapping* of accumulated drag
    /// rather than a factor applied per event; see [`ScrollPhysics::stretch`].
    raw: f32,
    viewport: f32,
    content: f32,
    physics: ScrollPhysics,
    /// A fling or a spring in progress, with the time it started.
    animation: Option<(Animation, Duration)>,
}

#[derive(Debug, Clone, Copy)]
enum Animation {
    Fling(Fling),
    Settle(Spring),
}

impl ScrollPosition {
    /// A position at the top of `content` shown through a `viewport`-sized hole.
    #[must_use]
    pub fn new(viewport: f32, content: f32, physics: ScrollPhysics) -> Self {
        Self {
            offset: 0.0,
            raw: 0.0,
            viewport,
            content,
            physics,
            animation: None,
        }
    }

    /// How far it currently is into the content.
    ///
    /// May be negative or past [`max_offset`](Self::max_offset) under
    /// [`Overscroll::Bounce`], which is the whole point of that mode.
    #[must_use]
    pub const fn offset(&self) -> f32 {
        self.offset
    }

    /// The furthest it can scroll — zero when the content fits.
    #[must_use]
    pub fn max_offset(&self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }

    /// How far past an edge it currently is; zero when within range.
    #[must_use]
    pub fn overscroll(&self) -> f32 {
        if self.offset < 0.0 {
            self.offset
        } else {
            (self.offset - self.max_offset()).max(0.0)
        }
    }

    /// Change the size of the hole or the content, keeping the offset in range.
    pub fn resize(&mut self, viewport: f32, content: f32) {
        self.viewport = viewport;
        self.content = content;
        self.offset = self.offset.clamp(0.0, self.max_offset());
        self.raw = self.offset;
    }

    /// The size of the hole the content is seen through.
    #[must_use]
    pub const fn viewport(&self) -> f32 {
        self.viewport
    }

    /// Put the offset exactly here, in range, cancelling any animation.
    ///
    /// The programmatic counterpart to [`apply_drag`](Self::apply_drag), and
    /// the primitive [`reveal`](Self::reveal) is built on. Clamped rather than
    /// stretched: an overscroll is something a *finger* did, and a caller
    /// asking to be at offset 4000 in a 3000-pixel list means the end of it,
    /// not a bounce.
    pub fn jump_to(&mut self, offset: f32) {
        self.animation = None;
        self.offset = offset.clamp(0.0, self.max_offset());
        self.raw = self.offset;
    }

    /// Scroll the least distance that brings `start..start + extent` into view.
    ///
    /// `margin` is kept clear either side of the range when it has to move, so
    /// a revealed line does not land flush against the edge of the window with
    /// no context above or below it.
    ///
    /// # Already visible means do not move
    ///
    /// This is the rule that makes it usable from a jump-to-line, a
    /// find-next, or a focus change: those fire constantly, and a `reveal`
    /// that always centred would yank the view on every keystroke. Returns
    /// whether it moved, so a caller can tell the two apart.
    ///
    /// # A range taller than the window
    ///
    /// Aligned to its start. Centring it would leave both ends off screen and
    /// scrolling to its end would show the reader the bottom of something they
    /// have not seen the top of.
    pub fn reveal(&mut self, start: f32, extent: f32, margin: f32) -> bool {
        let before = self.offset;
        let top = self.offset;
        let bottom = self.offset + self.viewport;

        // Two reasons to align to the top, and they are deliberately one
        // branch: the range is taller than the window (centring would leave
        // both ends off screen), or it starts above the window.
        if extent + margin * 2.0 >= self.viewport || start - margin < top {
            self.jump_to(start - margin);
        } else if start + extent + margin > bottom {
            self.jump_to(start + extent + margin - self.viewport);
        } else {
            return false;
        }
        (self.offset - before).abs() > f32::EPSILON
    }

    /// Move by a finger's worth of drag.
    ///
    /// `delta` is the finger's movement, so dragging *down* scrolls *up* — the
    /// content follows the finger, and the offset goes the other way.
    pub fn apply_drag(&mut self, delta: f32) {
        self.animation = None;
        self.raw -= delta;
        self.sync_offset();
    }

    /// Map the accumulated raw position onto where the content may actually sit.
    ///
    /// # `raw` is only allowed to run past the edge when that means something
    ///
    /// Under [`Overscroll::Bounce`] the part past the edge *is* the stretch, and
    /// keeping it is the whole reason `raw` is a separate number.
    ///
    /// Under [`Overscroll::Clamp`] it means nothing on screen, and letting it
    /// accumulate turns it into **dead travel**: a wheel spun hard at the end of
    /// a list banks thousands of invisible pixels, and the list then refuses to
    /// move until every one of them has been scrolled back. A release cannot
    /// rescue it either — [`fling`](Self::fling) springs back only when there is
    /// an [`overscroll`](Self::overscroll) to spring from, and under `Clamp`
    /// there never is.
    fn sync_offset(&mut self) {
        let max = self.max_offset();
        if !self.physics.stretches() {
            self.raw = self.raw.clamp(0.0, max);
        }
        let edge = self.raw.clamp(0.0, max);
        // Only the part past the edge is resisted; the part still in range moves
        // freely, so a drag crossing the boundary does not stall a pixel early.
        let beyond = self.raw - edge;
        self.offset = if beyond == 0.0 {
            edge
        } else {
            edge + self.physics.stretch(beyond, self.viewport)
        };
    }

    /// Release the finger at `velocity` pixels per second.
    ///
    /// A velocity in the same sense as [`apply_drag`](Self::apply_drag): the
    /// finger's, not the content's.
    pub fn fling(&mut self, velocity: f32, now: Duration) {
        if self.overscroll() != 0.0 {
            // Already past the edge: spring back rather than throw further.
            self.settle(now, -velocity);
            return;
        }
        let fling = self.physics.fling(self.offset, -velocity);
        if fling.duration() == Duration::ZERO {
            self.animation = None;
            self.clamp_if_needed(now);
            return;
        }
        self.raw = self.offset;
        self.animation = Some((Animation::Fling(fling), now));
    }

    /// Start springing back to the nearest edge.
    pub fn settle(&mut self, now: Duration, velocity: f32) {
        let target = self.offset.clamp(0.0, self.max_offset());
        if (self.offset - target).abs() < f32::EPSILON {
            self.animation = None;
            return;
        }
        let stiffness = if self.physics.spring_stiffness > 0.0 {
            self.physics.spring_stiffness
        } else {
            Spring::DEFAULT_STIFFNESS
        };
        self.raw = self.offset;
        self.animation = Some((
            Animation::Settle(Spring::new(self.offset, target, velocity, stiffness)),
            now,
        ));
    }

    /// Advance any animation to `now`. Call once a frame.
    ///
    /// Returns `true` while something is still moving, which is what tells a
    /// widget to ask for another frame.
    pub fn advance(&mut self, now: Duration) -> bool {
        let Some((animation, started)) = self.animation else {
            return false;
        };
        let elapsed = now.saturating_sub(started);

        match animation {
            Animation::Fling(fling) => {
                self.offset = fling.position(elapsed);
                self.raw = self.offset;
                let done = fling.is_done(elapsed);
                if self.offset < 0.0 || self.offset > self.max_offset() {
                    // The fling ran into an edge. Under `Clamp` it stops there;
                    // under `Bounce` it carries its remaining speed into a spring,
                    // which is what makes an iOS list rebound rather than stick.
                    let velocity = fling.velocity_at(elapsed);
                    self.animation = None;
                    match self.physics.overscroll {
                        Overscroll::Clamp => {
                            self.offset = self.offset.clamp(0.0, self.max_offset());
                            self.raw = self.offset;
                            return false;
                        }
                        Overscroll::Bounce => {
                            self.settle(now, velocity);
                            return true;
                        }
                    }
                }
                if done {
                    self.animation = None;
                }
                !done
            }
            Animation::Settle(spring) => {
                self.offset = spring.position(elapsed);
                let done = elapsed >= spring.duration();
                if done {
                    self.offset = self.offset.clamp(0.0, self.max_offset());
                    self.animation = None;
                }
                self.raw = self.offset;
                !done
            }
        }
    }

    /// `true` while a fling or spring is still running.
    #[must_use]
    pub const fn is_animating(&self) -> bool {
        self.animation.is_some()
    }

    /// Stop any animation where it is. For a finger landing mid-fling.
    pub fn stop(&mut self) {
        self.animation = None;
    }

    fn clamp_if_needed(&mut self, now: Duration) {
        if self.overscroll() != 0.0 {
            self.settle(now, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn ios_flings_travel_further_than_android_ones_from_the_same_throw() {
        let android = ScrollPhysics::android().fling(0.0, 2000.0).destination();
        let ios = ScrollPhysics::ios().fling(0.0, 2000.0).destination();

        assert!(
            ios > android,
            "the two conventions are supposed to feel different with identical \
             input: android {android}, ios {ios}"
        );
    }

    #[test]
    fn clamping_refuses_to_move_past_the_edge_at_all() {
        let physics = ScrollPhysics::android();
        assert_eq!(physics.stretch(30.0, 600.0), 0.0);
        assert_eq!(physics.stretch(500.0, 600.0), 0.0);
    }

    #[test]
    fn bouncing_follows_the_finger_at_first_and_then_resists() {
        let physics = ScrollPhysics::ios();

        let small = physics.stretch(30.0, 600.0);
        let large = physics.stretch(600.0, 600.0);

        assert!(
            small > 25.0,
            "a small pull past the edge should mostly follow the finger: {small}"
        );
        assert!(small <= 30.0, "but never exceed it");
        assert!(
            large < 600.0 * 0.6,
            "a hard pull must visibly refuse, or the list feels broken rather \
             than elastic: {large}"
        );
        assert!(large > small, "while still moving at all");
    }

    #[test]
    fn overscroll_is_bounded_however_hard_it_is_pulled() {
        let physics = ScrollPhysics::ios();
        assert!(physics.stretch(100_000.0, 600.0) < 600.0);
        assert_eq!(physics.stretch(-30.0, 600.0), -physics.stretch(30.0, 600.0));
    }

    #[test]
    fn overscroll_does_not_depend_on_how_many_events_the_drag_arrived_in() {
        let one_go = {
            let mut scroll = position(ScrollPhysics::ios());
            scroll.apply_drag(200.0);
            scroll.offset()
        };
        let dribbled = {
            let mut scroll = position(ScrollPhysics::ios());
            for _ in 0..40 {
                scroll.apply_drag(5.0);
            }
            scroll.offset()
        };

        assert!(
            (one_go - dribbled).abs() < 0.01,
            "the same physical drag must overscroll the same at 60Hz and 120Hz: \
             {one_go} vs {dribbled}"
        );
    }

    // ------------------------------------------------------- scroll position

    fn position(physics: ScrollPhysics) -> ScrollPosition {
        // A 600-tall viewport onto 2000 of content: 1400 of travel.
        ScrollPosition::new(600.0, 2000.0, physics)
    }

    #[test]
    fn dragging_up_scrolls_down() {
        let mut scroll = position(ScrollPhysics::android());
        // The finger moved up 100px, so the content came up and the offset grew.
        scroll.apply_drag(-100.0);
        assert!(
            (scroll.offset() - 100.0).abs() < 0.01,
            "{}",
            scroll.offset()
        );
    }

    #[test]
    fn content_that_fits_does_not_scroll() {
        let mut scroll = ScrollPosition::new(600.0, 400.0, ScrollPhysics::android());
        assert_eq!(scroll.max_offset(), 0.0);
        scroll.apply_drag(-100.0);
        assert_eq!(scroll.offset(), 0.0, "there is nowhere to go");
    }

    #[test]
    fn clamping_stops_dead_at_the_top() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.apply_drag(200.0);
        assert_eq!(scroll.offset(), 0.0);
        assert_eq!(scroll.overscroll(), 0.0);
    }

    #[test]
    fn bouncing_goes_past_the_top_but_grudgingly() {
        let mut scroll = position(ScrollPhysics::ios());
        scroll.apply_drag(200.0);

        assert!(
            scroll.offset() < 0.0,
            "an elastic list follows the finger past the edge: {}",
            scroll.offset()
        );
        assert!(
            scroll.offset() > -200.0,
            "but not all the way — it resists: {}",
            scroll.offset()
        );
    }

    #[test]
    fn a_bounced_list_springs_back_to_the_edge() {
        let mut scroll = position(ScrollPhysics::ios());
        scroll.apply_drag(150.0);
        assert!(scroll.overscroll() < 0.0);

        scroll.fling(0.0, ms(0));
        assert!(
            scroll.is_animating(),
            "release has to start the spring back"
        );

        let mut now = ms(0);
        for _ in 0..300 {
            now += ms(8);
            if !scroll.advance(now) {
                break;
            }
        }
        assert!(!scroll.is_animating());
        assert!(scroll.offset().abs() < 0.5, "{}", scroll.offset());
    }

    #[test]
    fn a_fling_carries_the_list_onward_and_stops() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.apply_drag(-100.0);
        scroll.fling(-1500.0, ms(0));
        assert!(scroll.is_animating());

        let mut now = ms(0);
        while scroll.advance(now) && now < ms(6000) {
            now += ms(8);
        }
        assert!(
            scroll.offset() > 150.0,
            "the throw has to travel well past where the finger left it: {}",
            scroll.offset()
        );
        assert!(scroll.offset() <= scroll.max_offset() + 0.5);
        assert!(!scroll.is_animating());
    }

    #[test]
    fn a_clamping_fling_into_the_end_stops_there_rather_than_overshooting() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.apply_drag(-1300.0);
        scroll.fling(-6000.0, ms(0));

        let mut now = ms(0);
        while scroll.advance(now) && now < ms(6000) {
            now += ms(8);
        }
        assert_eq!(
            scroll.offset(),
            scroll.max_offset(),
            "android clamps at the end"
        );
    }

    #[test]
    fn a_bouncing_fling_into_the_end_rebounds_and_settles_back_at_it() {
        let mut scroll = position(ScrollPhysics::ios());
        scroll.apply_drag(-1300.0);
        scroll.fling(-6000.0, ms(0));

        let mut now = ms(0);
        let mut furthest: f32 = 0.0;
        while scroll.advance(now) && now < ms(8000) {
            now += ms(8);
            furthest = furthest.max(scroll.offset());
        }
        assert!(
            furthest > scroll.max_offset(),
            "a fling into the end has to carry its remaining speed past it: \
             reached {furthest}, end is {}",
            scroll.max_offset()
        );
        assert!(
            (scroll.offset() - scroll.max_offset()).abs() < 0.5,
            "and then come back to it: {}",
            scroll.offset()
        );
    }

    #[test]
    fn a_flick_too_slow_to_fling_leaves_the_list_where_it_is() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.apply_drag(-100.0);
        scroll.fling(-10.0, ms(0));

        assert!(!scroll.is_animating());
        assert!((scroll.offset() - 100.0).abs() < 0.01);
    }

    #[test]
    fn touching_a_flinging_list_stops_it() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.fling(-2000.0, ms(0));
        scroll.advance(ms(100));
        let caught = scroll.offset();

        scroll.apply_drag(0.0);
        assert!(!scroll.is_animating(), "a finger down must catch the list");
        assert!((scroll.offset() - caught).abs() < 0.01);
    }

    #[test]
    fn resizing_keeps_the_offset_in_range() {
        let mut scroll = position(ScrollPhysics::android());
        scroll.apply_drag(-1400.0);
        assert!((scroll.offset() - 1400.0).abs() < 0.01);

        // The content shrank under it — items were removed.
        scroll.resize(600.0, 800.0);
        assert_eq!(scroll.offset(), 200.0, "clamped to the new end");
    }

    #[test]
    fn the_platform_default_is_one_of_the_two_conventions() {
        let physics = ScrollPhysics::platform_default();
        assert!(physics == ScrollPhysics::ios() || physics == ScrollPhysics::android());
        assert_eq!(
            physics.overscroll == Overscroll::Bounce,
            TargetPlatform::current().is_apple()
        );
    }

    #[test]
    fn a_clamped_list_moves_the_moment_the_finger_reverses() {
        // The defect. `raw` used to keep accumulating past the edge while
        // `offset` sat clamped, so a wheel spun hard at the end banked invisible
        // travel and the list then refused to move until all of it was undone.
        let mut position = ScrollPosition::new(100.0, 300.0, ScrollPhysics::android());
        let max = position.max_offset();

        // Slam into the end and keep going, hard.
        position.apply_drag(-10_000.0);
        assert_eq!(position.offset(), max, "clamped physics stops at the edge");

        // One pixel back the other way has to move one pixel of content.
        position.apply_drag(1.0);
        assert!(
            (position.offset() - (max - 1.0)).abs() < f32::EPSILON,
            "reversing moved {} rather than 1px off the edge",
            max - position.offset()
        );
    }

    #[test]
    fn a_bouncing_list_keeps_what_the_finger_asked_for() {
        // The half that must not change with it: under `Bounce` the part past
        // the edge is the stretch, so it has to survive being accumulated.
        let mut position = ScrollPosition::new(100.0, 300.0, ScrollPhysics::ios());
        let max = position.max_offset();

        position.apply_drag(-10_000.0);
        assert!(
            position.offset() > max,
            "a bouncing list goes past the edge: {} vs {max}",
            position.offset()
        );

        // Giving one pixel back leaves it still stretched, because the finger is
        // still thousands of pixels past the end.
        let stretched = position.offset();
        position.apply_drag(1.0);
        assert!(
            position.offset() > max,
            "one pixel back does not undo the whole stretch"
        );
        assert!(
            position.offset() <= stretched,
            "and it does not stretch further"
        );
    }
    // ===================== programmatic scrolling =========================

    fn window(viewport: f32, content: f32) -> ScrollPosition {
        ScrollPosition::new(viewport, content, ScrollPhysics::ios())
    }

    #[test]
    fn a_jump_is_clamped_rather_than_stretched() {
        // An overscroll is something a finger did. A caller asking for offset
        // 4000 in a 3000-pixel list means the end of it, not a bounce.
        let mut position = window(300.0, 1000.0);
        position.jump_to(4000.0);
        assert_eq!(position.offset(), 700.0);
        assert_eq!(position.overscroll(), 0.0);

        position.jump_to(-500.0);
        assert_eq!(position.offset(), 0.0);
    }

    #[test]
    fn revealing_something_already_visible_does_not_move() {
        // The rule that makes `reveal` safe to call on every caret move. One
        // that always centred would yank the view on every keystroke.
        let mut position = window(300.0, 1000.0);
        position.jump_to(100.0);
        assert!(!position.reveal(150.0, 20.0, 8.0));
        assert_eq!(position.offset(), 100.0);
    }

    #[test]
    fn revealing_something_above_scrolls_up_to_it_with_its_margin() {
        let mut position = window(300.0, 1000.0);
        position.jump_to(400.0);
        assert!(position.reveal(380.0, 20.0, 8.0));
        assert_eq!(position.offset(), 372.0, "the range's top, less the margin");
    }

    #[test]
    fn revealing_something_below_scrolls_the_least_it_can() {
        // The least distance, not a centring: a jump-to-line one row below the
        // fold should move one row, not half a window.
        let mut position = window(300.0, 1000.0);
        assert!(position.reveal(310.0, 20.0, 8.0));
        assert_eq!(position.offset(), 38.0, "310 + 20 + 8 - 300");
    }

    #[test]
    fn a_range_taller_than_the_window_is_aligned_to_its_start() {
        // Centring it would leave both ends off screen, and scrolling to its
        // end would show the reader the bottom of something whose top they
        // have not seen.
        let mut position = window(100.0, 1000.0);
        assert!(position.reveal(400.0, 250.0, 8.0));
        assert_eq!(position.offset(), 392.0);
    }

    #[test]
    fn a_reveal_cancels_a_fling() {
        // Otherwise the view arrives where it was asked to go and then keeps
        // travelling, which reads as the jump having missed.
        let mut position = window(300.0, 5000.0);
        position.fling(-3000.0, Duration::ZERO);
        assert!(position.is_animating());
        position.reveal(2000.0, 20.0, 8.0);
        assert!(!position.is_animating());
    }

    #[test]
    fn a_reveal_before_the_first_layout_aligns_to_the_start() {
        // Viewport zero: every range looks taller than the window, which is
        // the branch that aligns to `start` — the right answer for a jump made
        // before the window has been measured.
        let mut position = ScrollPosition::new(0.0, 0.0, ScrollPhysics::ios());
        position.reveal(400.0, 20.0, 8.0);
        assert_eq!(position.offset(), 0.0, "and there is nowhere to scroll to");
    }
}
