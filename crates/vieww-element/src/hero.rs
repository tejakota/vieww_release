//! The route integration for shared elements: capture, then fly.
//!
//! # What was missing, and what was not
//!
//! [`shared_element`](crate::shared_element) is complete and was reachable: a
//! registry that collects a tagged element's laid-out rectangle, a
//! [`SharedFlight`] that pairs the tags present on two screens, and an overlay
//! that draws the flying copies at a progress value. Every part of the hard half
//! — getting a *position* out of a tree that assigns positions one phase after
//! the widget is built — was already there.
//!
//! What was missing is the part that makes it usable: something that knows the
//! **sequence**. A flight cannot begin on the frame the tap happened, because
//! the destination screen has not been laid out and nobody knows where the
//! element is going. Every implementation of this spends one frame on that, and
//! every application that wired the pieces up by hand had to discover it, get
//! the ordering wrong once, and write the same three-state machine.
//!
//! [`HeroController`] is that state machine, written once.
//!
//! # The three states
//!
//! | state | what is on screen | what the registry holds |
//! |---|---|---|
//! | [`Resting`](HeroState::Resting) | one screen | that screen's geometry, refreshed every frame |
//! | [`Capturing`](HeroState::Capturing) | the new screen, laid out | the old screen's geometry, held |
//! | [`Flying`](HeroState::Flying) | both screens, originals hidden | both, paired into a flight |
//!
//! The transition from `Capturing` to `Flying` happens on the **frame after**
//! the new screen was first laid out, which is the one frame of latency named
//! above. It is not tunable, because it is not a delay — it is the earliest
//! moment the destination is knowable.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{SpringAnimation as Spring, SpringPreset, Ticker, Tickers};
use vieww_widget::WidgetNode;

use crate::shared_element::{SharedFlight, SharedGeometry, SharedRegistry, SharedTag};
use crate::{Runtime, Signal};

/// Where a hero transition is in its sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeroState {
    /// Nothing is moving. The registry is following whatever is on screen.
    Resting,
    /// The outgoing geometry is held and the incoming screen is being laid out.
    ///
    /// Exactly one frame long. See the module docs for why it cannot be zero.
    Capturing,
    /// Copies are flying between the two.
    Flying,
}

/// Runs the capture-then-fly sequence for a route change.
///
/// ```no_run
/// # use std::time::Duration;
/// # use vieww_animation::{SpringPreset, Tickers};
/// # use vieww_element::{HeroController, Runtime};
/// # fn example(runtime: &Runtime, tickers: &mut Tickers) {
/// let heroes = HeroController::new(runtime);
///
/// // Screens build their tagged elements against this registry.
/// let registry = heroes.registry().clone();
///
/// // On the frame the route changes:
/// heroes.begin();
///
/// // Once per frame, after paint:
/// heroes.advance(tickers, SpringPreset::Standard);
/// # }
/// ```
///
/// # Why the caller drives it rather than the navigator
///
/// Because a hero transition is not a property of navigation. The same movement
/// is wanted between two states of one screen — a card expanding into a detail
/// pane, a thumbnail growing into a viewer — where there is no route change at
/// all. Tying this to the navigator would have made the common case the only
/// case, and `RouteTransition` deliberately knows nothing about position for the
/// same reason.
pub struct HeroController {
    registry: SharedRegistry,
    inner: Rc<RefCell<Inner>>,
    /// `0.0 → 1.0` across the flight. Readable from a build, so a screen can
    /// fade its own content against the same curve the heroes fly on.
    progress: Signal<f32>,
}

struct Inner {
    state: HeroState,
    /// The outgoing screen's geometry, held from before the route changed.
    departed: SharedGeometry,
    /// What is currently flying.
    flight: SharedFlight,
    /// The spring, once a flight has started. Held strongly, because
    /// [`Tickers`] holds its entries weakly — the defect `play_transition` had.
    player: Option<Rc<RefCell<HeroPlayer>>>,
}

impl HeroController {
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            registry: SharedRegistry::new(),
            inner: Rc::new(RefCell::new(Inner {
                state: HeroState::Resting,
                departed: SharedGeometry::default(),
                flight: SharedFlight::default(),
                player: None,
            })),
            progress: runtime.signal(0.0),
        }
    }

    /// The registry every tagged element on every screen reports to.
    #[must_use]
    pub const fn registry(&self) -> &SharedRegistry {
        &self.registry
    }

    /// How far through the flight, `0.0 ..= 1.0`.
    ///
    /// Read it in a build to drive anything that should move with the heroes —
    /// a scrim, a title crossfade — so the whole transition is on one curve.
    #[must_use]
    pub const fn progress(&self) -> &Signal<f32> {
        &self.progress
    }

    #[must_use]
    pub fn state(&self) -> HeroState {
        self.inner.borrow().state
    }

    /// What is flying, for a screen deciding which of its own elements to hide.
    #[must_use]
    pub fn flight(&self) -> SharedFlight {
        self.inner.borrow().flight.clone()
    }

    /// `true` while this tag's copy is in flight, so the original should hide.
    ///
    /// The question a screen asks about each of its own tagged elements. See
    /// [`SharedElement::hidden`](crate::SharedElement::hidden): a hidden element
    /// still reports its rectangle, which is what lets the destination be
    /// captured from a screen that is at that moment invisible.
    #[must_use]
    pub fn is_flying(&self, tag: &SharedTag) -> bool {
        self.inner.borrow().flight.is_flying(tag)
    }

    /// The route is changing: hold what is on screen now as the departure.
    ///
    /// Called on the frame the route is pushed or popped, **before** the new
    /// screen is built. Whatever the registry has collected up to this point is
    /// the outgoing geometry.
    ///
    /// Beginning while a flight is already running restarts from where things
    /// are: the geometry currently on screen becomes the new departure, which is
    /// what makes a second tap during a transition move from where the eye last
    /// saw the element rather than snapping back to the original screen.
    pub fn begin(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.departed = self.registry.snapshot();
        inner.state = HeroState::Capturing;
        inner.flight = SharedFlight::default();
        inner.player = None;
        self.progress.set(0.0);
        // The incoming screen has to report into a clean registry, or a tag that
        // exists on the outgoing screen and not the incoming one would look like
        // it had arrived at the place it started.
        self.registry.clear();
    }

    /// Advance the sequence by one frame. Call after paint.
    ///
    /// Returns the state it moved to, which a caller can ignore.
    ///
    /// # The frame that matters
    ///
    /// In [`Capturing`](HeroState::Capturing), this is the first moment the
    /// incoming screen's rectangles exist, so it pairs them against the held
    /// departure and starts the spring. That is the whole of the one-frame
    /// latency, and it is why this is called *after* paint rather than before
    /// build.
    pub fn advance(&self, tickers: &mut Tickers, preset: SpringPreset) -> HeroState {
        let mut inner = self.inner.borrow_mut();
        match inner.state {
            HeroState::Resting => {}
            HeroState::Capturing => {
                let arrived = self.registry.snapshot();
                let flight = SharedFlight::between(&inner.departed, &arrived);
                if flight.is_empty() {
                    // Nothing is shared between the two screens, which is
                    // ordinary — most route changes have no heroes in them. The
                    // route transition handles the whole change on its own.
                    inner.state = HeroState::Resting;
                } else {
                    let mut spring = Spring::new(0.0, preset);
                    spring.retarget(1.0);
                    let player = Rc::new(RefCell::new(HeroPlayer {
                        spring,
                        progress: self.progress.clone(),
                    }));
                    // **Held**, not handed over. `Tickers` stores weak
                    // references, so an `Rc` that lived only for this statement
                    // would be gone by the next frame and the flight would never
                    // move — which is exactly the defect `play_transition` had.
                    tickers.add(&player);
                    inner.player = Some(player);
                    inner.flight = flight;
                    inner.state = HeroState::Flying;
                }
            }
            HeroState::Flying => {
                let settled = inner
                    .player
                    .as_ref()
                    .is_none_or(|player| !player.borrow().spring.is_animating());
                if settled {
                    // The copies come down and the originals come back, on the
                    // same frame: an order that showed neither for a frame is a
                    // flicker at the end of every transition.
                    self.progress.set(1.0);
                    inner.flight = SharedFlight::default();
                    inner.player = None;
                    inner.state = HeroState::Resting;
                }
            }
        }
        inner.state
    }

    /// The flying copies, to place at the root of the tree above both screens.
    ///
    /// `build` is called once per flying tag and returns the element as it looks
    /// **at its destination size** — see
    /// [`Flight::scale_at`](crate::Flight::scale_at).
    ///
    /// Returns an empty stack when nothing is flying, so a caller puts it in the
    /// tree unconditionally rather than branching.
    #[must_use]
    pub fn overlay(&self, build: impl Fn(&SharedTag) -> WidgetNode) -> WidgetNode {
        let inner = self.inner.borrow();
        inner.flight.overlay(self.progress.get(), build)
    }

    /// Stop any flight and return to rest, leaving the originals visible.
    ///
    /// For a screen torn down mid-transition. Without it the copies would keep
    /// flying towards a destination that is no longer mounted.
    pub fn cancel(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.flight = SharedFlight::default();
        inner.player = None;
        inner.state = HeroState::Resting;
        self.progress.set(1.0);
    }
}

impl Clone for HeroController {
    /// A second handle to the *same* transition, so a screen deep in the tree
    /// can ask what is flying without the controller being threaded down as a
    /// reference.
    fn clone(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            inner: Rc::clone(&self.inner),
            progress: self.progress.clone(),
        }
    }
}

impl std::fmt::Debug for HeroController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("HeroController")
            .field("state", &inner.state)
            .field("flying", &inner.flight.len())
            .finish_non_exhaustive()
    }
}

/// The spring behind a flight, writing into the controller's progress signal.
struct HeroPlayer {
    spring: Spring,
    progress: Signal<f32>,
}

impl Ticker for HeroPlayer {
    fn tick(&mut self, now: Duration) -> bool {
        let alive = self.spring.tick(now);
        self.progress.set(self.spring.value());
        alive
    }

    fn is_animating(&self) -> bool {
        self.spring.is_animating()
    }

    /// Reduced motion: arrive. See [`Ticker::settle`].
    ///
    /// A hero flight is a large object travelling across the whole viewport,
    /// which is precisely the motion a reduced-motion preference is asking not
    /// to see. The element is simply at its destination.
    fn settle(&mut self, now: Duration) -> bool {
        let moved = self.spring.settle(now);
        self.progress.set(self.spring.value());
        moved
    }
}

impl std::fmt::Debug for HeroPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeroPlayer").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Rect;

    fn rect(left: f32, top: f32) -> Rect {
        Rect::new(left, top, left + 40.0, top + 40.0)
    }

    fn controller() -> (Runtime, HeroController) {
        let runtime = Runtime::new();
        let heroes = HeroController::new(&runtime);
        (runtime, heroes)
    }

    /// **The sequence, which is the whole reason this type exists.** A flight
    /// cannot start on the frame the route changed, because the destination has
    /// not been laid out — so `begin` captures and the *next* `advance` flies.
    #[test]
    fn a_flight_starts_one_frame_after_the_route_changed() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();

        // Frame N: the outgoing screen has reported.
        heroes
            .registry()
            .record(SharedTag::new("avatar"), rect(10.0, 10.0));
        assert_eq!(heroes.state(), HeroState::Resting);

        // The route changes. The departure is held; nothing flies yet, because
        // nothing knows where it is going.
        heroes.begin();
        assert_eq!(heroes.state(), HeroState::Capturing);
        assert!(heroes.flight().is_empty());

        // Frame N+1: the incoming screen has been laid out and reported.
        heroes
            .registry()
            .record(SharedTag::new("avatar"), rect(200.0, 300.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        assert_eq!(heroes.state(), HeroState::Flying);
        assert_eq!(heroes.flight().len(), 1);
        assert!(heroes.is_flying(&SharedTag::new("avatar")));
    }

    /// A tag on only one screen has nothing to fly to. That is not an error —
    /// most route changes have no heroes at all — so the controller returns to
    /// rest and leaves the whole change to the route transition.
    #[test]
    fn an_unpaired_tag_does_not_fly_and_is_not_an_error() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();

        heroes
            .registry()
            .record(SharedTag::new("only-here"), rect(10.0, 10.0));
        heroes.begin();
        heroes
            .registry()
            .record(SharedTag::new("only-there"), rect(50.0, 50.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        assert_eq!(heroes.state(), HeroState::Resting);
        assert!(heroes.flight().is_empty());
    }

    /// **The registry is cleared on `begin`.** Without it, a tag present on the
    /// outgoing screen and absent from the incoming one would still be in the
    /// snapshot and would appear to have arrived exactly where it started —
    /// a hero that "flies" a distance of zero and hides its original for the
    /// duration, which reads as the element vanishing.
    #[test]
    fn a_tag_that_does_not_arrive_does_not_fly_to_where_it_started() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();

        heroes
            .registry()
            .record(SharedTag::new("gone"), rect(10.0, 10.0));
        heroes.begin();
        // The incoming screen reports nothing under that tag.
        heroes
            .registry()
            .record(SharedTag::new("other"), rect(80.0, 80.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        assert!(!heroes.is_flying(&SharedTag::new("gone")));
    }

    /// The player is **held**, not handed to a weak registry and dropped — the
    /// defect `play_transition` shipped with. A flight whose spring was
    /// collected would never move and would never log.
    #[test]
    fn the_flight_actually_moves_and_then_settles() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();

        heroes
            .registry()
            .record(SharedTag::new("card"), rect(0.0, 0.0));
        heroes.begin();
        heroes
            .registry()
            .record(SharedTag::new("card"), rect(200.0, 200.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        assert_eq!(
            tickers.len(),
            1,
            "the spring must still be registered, which a temporary Rc would not be"
        );
        assert_eq!(heroes.progress().peek(), 0.0);

        tickers.advance(Duration::from_millis(0));
        tickers.advance(Duration::from_millis(16));
        tickers.advance(Duration::from_millis(32));
        assert!(
            heroes.progress().peek() > 0.0,
            "the flight has to have moved: {}",
            heroes.progress().peek()
        );

        // Run it out. The controller returns to rest and takes the copies down.
        let mut now = Duration::from_millis(32);
        for _ in 0..200 {
            now += Duration::from_millis(16);
            tickers.advance(now);
            heroes.advance(&mut tickers, SpringPreset::Standard);
        }
        assert_eq!(heroes.state(), HeroState::Resting);
        assert!(heroes.flight().is_empty(), "the copies come down");
        assert_eq!(heroes.progress().peek(), 1.0);
    }

    /// Reduced motion lands the flight immediately: a large object crossing the
    /// viewport is exactly what the preference is about.
    #[test]
    fn reduced_motion_puts_the_hero_at_its_destination_at_once() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();
        tickers.set_reduce_motion(true);

        heroes
            .registry()
            .record(SharedTag::new("card"), rect(0.0, 0.0));
        heroes.begin();
        heroes
            .registry()
            .record(SharedTag::new("card"), rect(200.0, 200.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        tickers.advance(Duration::from_millis(0));
        assert_eq!(heroes.progress().peek(), 1.0);
    }

    /// A screen torn down mid-flight must not leave copies flying towards a
    /// destination that is no longer mounted.
    #[test]
    fn cancelling_takes_the_copies_down_and_shows_the_originals() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();

        heroes
            .registry()
            .record(SharedTag::new("card"), rect(0.0, 0.0));
        heroes.begin();
        heroes
            .registry()
            .record(SharedTag::new("card"), rect(200.0, 200.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);
        assert_eq!(heroes.state(), HeroState::Flying);

        heroes.cancel();
        assert_eq!(heroes.state(), HeroState::Resting);
        assert!(!heroes.is_flying(&SharedTag::new("card")));
        assert_eq!(
            heroes.progress().peek(),
            1.0,
            "the originals are shown at their destination, not mid-flight"
        );
    }

    /// A clone is a second handle to the same transition, so a screen deep in
    /// the tree can ask what is flying without the controller being threaded
    /// down by reference.
    #[test]
    fn a_clone_sees_the_same_flight() {
        let (_runtime, heroes) = controller();
        let mut tickers = Tickers::new();
        let elsewhere = heroes.clone();

        heroes
            .registry()
            .record(SharedTag::new("card"), rect(0.0, 0.0));
        heroes.begin();
        heroes
            .registry()
            .record(SharedTag::new("card"), rect(200.0, 200.0));
        heroes.advance(&mut tickers, SpringPreset::Standard);

        assert_eq!(elsewhere.state(), HeroState::Flying);
        assert!(elsewhere.is_flying(&SharedTag::new("card")));
    }
}
