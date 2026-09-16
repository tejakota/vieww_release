//! A scroll position whose offset is a signal.
//!
//! # The join between three layers
//!
//! `vieww-gestures` knows the physics — where a fling lands and how an
//! overscroll stretches — and nothing about trees. `vieww-widget`'s
//! [`Scrollable`](vieww_widget::Scrollable) knows how to draw a window onto
//! content and cannot hold state at all. This is the piece that needs both, plus
//! the element layer's [`Signal`]: it applies drags to a
//! [`ScrollPosition`](vieww_gestures::ScrollPosition), advances a fling each
//! frame, and publishes the offset into a signal — so the elements that display
//! it, and only those, are rebuilt.
//!
//! It is [`Animation`](crate::Animation)'s shape exactly, for the same reason,
//! and the two are the only types in this crate that exist to join layers rather
//! than to be one.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{Ticker, Tickers};
use vieww_foundation::{Axis, DragDetails, Offset};
use vieww_gestures::{ScrollPhysics, ScrollPosition};
use vieww_widget::{Handler, ScrollExtents};

use crate::{Runtime, Signal};

/// The position, and the signal its offset is published to.
struct Driven {
    position: ScrollPosition,
    signal: Signal<f32>,
    /// How close to the end counts as "near it", or `None` for no interest.
    near_end: Option<f32>,
    /// Told once each time the offset crosses into that zone.
    on_near_end: Option<Rc<dyn Fn()>>,
    /// Whether the offset was inside the zone last time this was evaluated.
    ///
    /// **The whole of the edge-triggering, and the reason this is a field
    /// rather than a comparison at the call site.** The offset is republished
    /// on every drag delta and every animation tick, so a level-triggered
    /// version — "if offset > threshold, fetch" — fires tens of times per
    /// second for as long as somebody rests at the bottom of a list, and the
    /// application sees a burst of identical requests rather than one.
    ///
    /// Rearming on the way *out* is what makes it usable more than once: fetch,
    /// the content grows, the end moves away, and the next approach fires
    /// again.
    was_near_end: bool,
}

impl Driven {
    fn publish(&mut self) {
        self.signal.set(self.position.offset());
        self.notify_near_end();
    }

    /// Fire `on_near_end` if this publish is the one that crossed into the zone.
    fn notify_near_end(&mut self) {
        let Some(distance) = self.near_end else {
            return;
        };
        let remaining = self.position.max_offset() - self.position.offset();
        // Content shorter than the window has `max_offset == 0`, so `remaining`
        // is zero and every such list would count as "at the end" the moment it
        // was measured. That is a list with nothing to page, and firing there
        // means an application that starts empty asks for page two before page
        // one has arrived.
        let has_somewhere_to_go = self.position.max_offset() > 0.0;
        let near = has_somewhere_to_go && remaining <= distance;

        if near && !self.was_near_end {
            if let Some(handler) = &self.on_near_end {
                handler();
            }
        }
        self.was_near_end = near;
    }
}

impl Ticker for Driven {
    /// # Publishing is decided by the offset, not by the return value
    ///
    /// **`ScrollPosition::advance` answers "call me again", and the step that
    /// answers *no* is the same step that changes the offset one last time.**
    /// Publishing on that answer — `if moved { publish() }`, which this was —
    /// therefore drops precisely the value that matters and leaves the screen
    /// one step short of home, for ever, because nothing is pending and no further
    /// frame is ever asked for.
    ///
    /// Three paths through `advance` change the offset and return `false`:
    ///
    /// - a **settled spring**, whose last step clamps the offset to exactly the
    ///   edge — found on a real window, where every pull-to-refresh left 0.4–0.8
    ///   logical pixels of overscroll on screen and the control drawn as a
    ///   hairline that nothing repainted away;
    /// - a **finished fling**, whose last step lands on its final position;
    /// - a fling that **ran into an edge** under `Overscroll::Clamp`, which is
    ///   the desktop platform default — so an ordinary scroll to the end of a
    ///   list settles on the last interpolated position rather than the edge.
    ///
    /// The sibling ticker in `animation.rs` looks identical and is correct,
    /// which is most of why this survived: `AnimationController::tick` returns
    /// `true` on its final step and `false` only when asked again, so *there*
    /// the return value really does mean "the value changed". Two conventions,
    /// one shape. Do not copy either one without reading which it is.
    ///
    /// # Why compared and not simply published every tick
    ///
    /// [`Signal::set`] notifies subscribers whether or not the value differs,
    /// and `Tickers::advance` ticks **every** registered ticker every frame, not
    /// only the animating ones. An unconditional publish would therefore mark
    /// the tree pending on every frame for the whole life of every scrollable on
    /// screen, and the loop would never idle.
    ///
    /// # The return value was the same mistake
    ///
    /// [`Ticker::tick`] is documented as "`true` if the value may have changed",
    /// and [`Ticker::is_animating`] as "`true` while this still needs frames" —
    /// two questions, deliberately separate. Returning `advance`'s answer here
    /// answered the second one to the first one's question. `is_animating` below
    /// already reports the position's own state, so nothing needed it.
    fn tick(&mut self, now: Duration) -> bool {
        // Discarded, and that is the whole fix: "call me again" is a question
        // this method was never asked, and `is_animating` answers it below.
        let _ = self.position.advance(now);

        let changed = self.signal.peek() != self.position.offset();
        if changed {
            self.publish();
        }
        changed
    }

    fn is_animating(&self) -> bool {
        self.position.is_animating()
    }
}

impl fmt::Debug for Driven {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollController")
            .field("offset", &self.position.offset())
            .field("max", &self.position.max_offset())
            .field("animating", &self.position.is_animating())
            .finish()
    }
}

/// Where a scrollable is, what moves it, and what to rebuild when it does.
///
/// Cheap to clone — like a [`Signal`], every clone is a handle to the same
/// position. Hold one for as long as the screen it scrolls exists; drop the last
/// handle and the fling stops with nothing to unregister, because
/// [`attach`](Self::attach) holds it weakly.
///
/// ```
/// use std::time::Duration;
/// use vieww_animation::Tickers;
/// use vieww_element::{ElementTree, ScrollController};
/// use vieww_gestures::ScrollPhysics;
/// use vieww_widget::ScrollExtents;
///
/// let ms = Duration::from_millis;
/// let mut tree = ElementTree::new();
/// let mut tickers = Tickers::new();
///
/// let scroll = ScrollController::new(tree.runtime(), ScrollPhysics::android());
/// scroll.attach(&mut tickers);
/// scroll.resize(ScrollExtents::new(300.0, 1200.0));
///
/// scroll.drag(-120.0);            // a finger moving up scrolls down
/// assert_eq!(scroll.peek(), 120.0);
///
/// scroll.fling(-2000.0, ms(0));   // released, still moving
/// tickers.advance(ms(16));
/// assert!(scroll.peek() > 120.0);
/// ```
#[derive(Clone)]
pub struct ScrollController {
    inner: Rc<RefCell<Driven>>,
    signal: Signal<f32>,
    /// Which component of a drag counts. Immutable, so every clone agrees
    /// without needing to share it.
    axis: Axis,
}

impl ScrollController {
    /// A controller for vertical content of unknown length, at the top.
    ///
    /// The signal is created on `runtime`, so it must be the runtime of the tree
    /// that will display it — otherwise the write marks nothing.
    #[must_use]
    pub fn new(runtime: &Runtime, physics: ScrollPhysics) -> Self {
        Self::along(Axis::Vertical, runtime, physics)
    }

    /// The same, for content that scrolls sideways.
    #[must_use]
    pub fn horizontal(runtime: &Runtime, physics: ScrollPhysics) -> Self {
        Self::along(Axis::Horizontal, runtime, physics)
    }

    fn along(axis: Axis, runtime: &Runtime, physics: ScrollPhysics) -> Self {
        let signal = runtime.signal(0.0_f32);
        Self {
            inner: Rc::new(RefCell::new(Driven {
                // Zero extents until layout says otherwise, which pins the
                // offset at zero — the honest state for content nobody has
                // measured, and one `resize` away from correct.
                position: ScrollPosition::new(0.0, 0.0, physics),
                signal: signal.clone(),
                near_end: None,
                on_near_end: None,
                was_near_end: false,
            })),
            signal,
            axis,
        }
    }

    /// Have `tickers` advance a fling once a frame.
    ///
    /// Held weakly; see the type's documentation.
    pub fn attach(&self, tickers: &mut Tickers) {
        tickers.add(&self.inner);
    }

    /// The current offset, subscribing the element that is building.
    #[must_use]
    pub fn offset(&self) -> f32 {
        self.signal.get()
    }

    /// The current offset, without subscribing. For handlers and tests.
    #[must_use]
    pub fn peek(&self) -> f32 {
        self.signal.peek()
    }

    /// The signal the offset is published to.
    #[must_use]
    pub fn signal(&self) -> Signal<f32> {
        self.signal.clone()
    }

    /// The furthest this content can be scrolled.
    #[must_use]
    pub fn max_offset(&self) -> f32 {
        self.inner.borrow().position.max_offset()
    }

    /// `true` while a fling or a settle is still running.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.inner.borrow().position.is_animating()
    }

    /// Be told once, each time the offset comes within `distance` of the end.
    ///
    /// The infinite-scroll trigger, and the whole of what
    /// `feature-checklist.md` asks for under "infinite scroll automated trigger
    /// thresholds". `ListView` already virtualises, so the missing piece was
    /// never the rendering — it was that nothing told the application when to
    /// fetch the next page.
    ///
    /// # Edge-triggered, and that is the entire design
    ///
    /// The handler fires on the publish that *crosses into* the zone, not on
    /// every publish while inside it. Level-triggering is the obvious version
    /// and it is unusable: the offset is republished on every drag delta and
    /// every animation tick, so resting at the bottom of a list would fire this
    /// tens of times a second and the application would send a burst of
    /// identical requests. Deduplicating those is then the application's
    /// problem, forever, in every application.
    ///
    /// It rearms when the offset leaves the zone — which is what the fetch
    /// itself causes, since new content moves the end away. So the sequence is
    /// fetch, grow, rearm, fetch, and it works for as many pages as there are.
    ///
    /// A list whose content is shorter than its window never fires: its end is
    /// always in view, and an application that starts empty must not ask for
    /// page two before page one has arrived.
    ///
    /// ```
    /// # use vieww_element::{Runtime, ScrollController};
    /// # use vieww_gestures::ScrollPhysics;
    /// # let runtime = Runtime::new();
    /// let scroll = ScrollController::new(&runtime, ScrollPhysics::android());
    /// scroll.on_near_end(600.0, || { /* fetch the next page */ });
    /// ```
    pub fn on_near_end(&self, distance: f32, handler: impl Fn() + 'static) {
        let mut inner = self.inner.borrow_mut();
        inner.near_end = Some(distance.max(0.0));
        inner.on_near_end = Some(Rc::new(handler));
        // Evaluated immediately, so a list that is *already* short enough to be
        // near its end when the handler is attached fires once now rather than
        // waiting for a scroll that may never come.
        inner.notify_near_end();
    }

    /// Stop reporting the end of the content.
    pub fn clear_near_end(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.near_end = None;
        inner.on_near_end = None;
        inner.was_near_end = false;
    }

    /// Tell it how long the window and the content are.
    ///
    /// What [`Scrollable::on_extents`](vieww_widget::Scrollable::on_extents)
    /// reports out of layout. Republishes the offset, because content that got
    /// shorter can leave the window past its end — a list that loses rows while
    /// scrolled to the bottom would otherwise show blank space below them.
    pub fn resize(&self, extents: ScrollExtents) {
        let mut inner = self.inner.borrow_mut();
        inner.position.resize(extents.viewport, extents.content);
        inner.publish();
    }

    /// The size of the window the content is seen through.
    ///
    /// Published because a caller that wants to *reveal* something has to know
    /// it, and until now nothing outside the physics did. Zero before the first
    /// layout has reported extents.
    #[must_use]
    pub fn viewport(&self) -> f32 {
        self.inner.borrow().position.viewport()
    }

    /// Scroll to exactly this offset, cancelling any fling.
    pub fn jump_to(&self, offset: f32) {
        let mut inner = self.inner.borrow_mut();
        inner.position.jump_to(offset);
        inner.publish();
    }

    /// Scroll the least distance that brings `start..start + extent` into view.
    ///
    /// The primitive behind "reveal this line", "scroll the next match into
    /// view" and "follow the focus". Does nothing when the range is already
    /// visible, which is what makes it safe to call on every caret move;
    /// returns whether it moved.
    ///
    /// Before the first layout the viewport is zero and every range looks
    /// taller than the window, so this aligns to `start` — which is the right
    /// answer for a jump made before the window has been measured.
    pub fn reveal(&self, start: f32, extent: f32, margin: f32) -> bool {
        let mut inner = self.inner.borrow_mut();
        let moved = inner.position.reveal(start, extent, margin);
        if moved {
            inner.publish();
        }
        moved
    }

    /// Move by a finger's movement, in the finger's direction.
    ///
    /// A drag *up* is a negative delta and scrolls *into* the content, which is
    /// why this takes the gesture's delta rather than an offset change.
    pub fn drag(&self, delta: f32) {
        let mut inner = self.inner.borrow_mut();
        inner.position.apply_drag(delta);
        // Published immediately rather than at the next tick: a finger that has
        // already moved must not wait a frame to be followed.
        inner.publish();
    }

    /// Release, carrying `velocity` in pixels per second.
    pub fn fling(&self, velocity: f32, now: Duration) {
        let mut inner = self.inner.borrow_mut();
        inner.position.fling(velocity, now);
        inner.publish();
    }

    /// Stop wherever it is.
    pub fn stop(&self) {
        self.inner.borrow_mut().position.stop();
    }

    // ------------------------------------------------- handlers for the widget

    /// A handler for [`Scrollable::on_drag`](vieww_widget::Scrollable::on_drag),
    /// reading the movement along this controller's axis.
    #[must_use]
    pub fn on_drag(&self) -> Handler<DragDetails> {
        let this = self.clone();
        Rc::new(move |details: DragDetails| this.drag(this.component(details.delta)))
    }

    /// A handler for
    /// [`Scrollable::on_drag_end`](vieww_widget::Scrollable::on_drag_end),
    /// flinging with the release velocity.
    #[must_use]
    pub fn on_drag_end(&self) -> Handler<DragDetails> {
        let this = self.clone();
        Rc::new(move |details: DragDetails| {
            this.fling(this.component(details.velocity), details.timestamp);
        })
    }

    /// A handler for
    /// [`Scrollable::on_extents`](vieww_widget::Scrollable::on_extents).
    #[must_use]
    pub fn on_extents(&self) -> Handler<ScrollExtents> {
        let this = self.clone();
        Rc::new(move |extents| this.resize(extents))
    }

    /// The component of a movement along the axis this controller scrolls.
    ///
    /// A `Scrollable` only reports drags along its own axis, so the *other*
    /// component is not merely unwanted — it is noise from a gesture that was
    /// already filtered. Taking the larger of the two would work by accident
    /// until a horizontal list appeared.
    const fn component(&self, movement: Offset) -> f32 {
        match self.axis {
            Axis::Vertical => movement.dy,
            Axis::Horizontal => movement.dx,
        }
    }
}

impl fmt::Debug for ScrollController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.borrow().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use vieww_gestures::ScrollPhysics;

    use crate::ElementTree;

    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn controller(tree: &ElementTree) -> ScrollController {
        let scroll = ScrollController::new(tree.runtime(), ScrollPhysics::android());
        scroll.resize(ScrollExtents::new(300.0, 1200.0));
        scroll
    }

    #[test]
    fn a_drag_up_scrolls_into_the_content() {
        let tree = ElementTree::new();
        let scroll = controller(&tree);

        scroll.drag(-100.0);
        assert_eq!(scroll.peek(), 100.0);
    }

    #[test]
    fn the_offset_reaches_the_signal_without_waiting_for_a_frame() {
        let tree = ElementTree::new();
        let scroll = controller(&tree);
        let offset = scroll.signal();

        scroll.drag(-40.0);
        assert_eq!(
            offset.peek(),
            40.0,
            "a finger that has moved must not lag a frame behind itself"
        );
    }

    #[test]
    fn content_that_shrinks_pulls_the_window_back_over_it() {
        let tree = ElementTree::new();
        let scroll = controller(&tree);
        scroll.drag(-900.0);
        assert_eq!(scroll.peek(), 900.0, "the end of 1200 in a 300 window");

        scroll.resize(ScrollExtents::new(300.0, 600.0));
        assert_eq!(
            scroll.peek(),
            300.0,
            "a list that loses rows while scrolled to the bottom must not leave \
             the window past the end"
        );
    }

    #[test]
    fn nothing_scrolls_before_anything_has_been_measured() {
        let tree = ElementTree::new();
        let scroll = ScrollController::new(tree.runtime(), ScrollPhysics::android());

        scroll.drag(-500.0);
        assert_eq!(scroll.max_offset(), 0.0);
        assert_eq!(scroll.peek(), 0.0, "content of unknown length cannot move");
    }

    #[test]
    fn a_fling_keeps_moving_after_the_finger_has_gone() {
        let tree = ElementTree::new();
        let scroll = controller(&tree);
        let mut tickers = Tickers::new();
        scroll.attach(&mut tickers);

        scroll.fling(-2000.0, ms(0));
        assert!(scroll.is_animating());

        tickers.advance(ms(16));
        let after_one_frame = scroll.peek();
        assert!(after_one_frame > 0.0, "{after_one_frame}");

        tickers.advance(ms(200));
        assert!(scroll.peek() > after_one_frame);
    }

    /// Drive `tickers` at 60Hz until nothing is animating.
    fn run_to_rest(tickers: &mut Tickers) {
        let mut now = ms(0);
        while tickers.is_animating() {
            now += ms(16);
            tickers.advance(now);
            assert!(now < ms(30_000), "never came to rest");
        }
    }

    #[test]
    fn the_last_step_of_a_spring_reaches_the_signal() {
        // **The bug this is the regression test for.** `advance` returns "call
        // me again", and the step that says *no* is the same step that clamps
        // the offset to the edge. Publishing on that answer dropped exactly that
        // value, so the position was home and the screen was not — permanently,
        // because nothing was pending and no further frame was ever requested.
        //
        // Seen on a real window as 0.4–0.8px of overscroll left at the top after
        // every pull-to-refresh, with the control drawn as a hairline.
        let tree = ElementTree::new();
        let scroll = ScrollController::new(tree.runtime(), ScrollPhysics::ios());
        scroll.resize(ScrollExtents::new(300.0, 1200.0));
        let mut tickers = Tickers::new();
        scroll.attach(&mut tickers);

        // Past the start, which only bounce physics allows, then let go.
        scroll.drag(150.0);
        assert!(scroll.peek() < 0.0, "overscrolled: {}", scroll.peek());
        scroll.fling(0.0, ms(0));
        run_to_rest(&mut tickers);

        assert_eq!(
            scroll.signal().peek(),
            0.0,
            "the settled offset must be the one on screen, not the step before it"
        );
    }

    #[test]
    fn the_last_step_of_a_fling_into_an_edge_reaches_the_signal() {
        // The same fault on the path a desktop actually takes: under
        // `Overscroll::Clamp` a fling that reaches the end clamps to the edge
        // and returns `false` in one step, so an ordinary scroll to the bottom
        // of a list used to settle on the last interpolated position instead.
        let tree = ElementTree::new();
        let scroll = ScrollController::new(tree.runtime(), ScrollPhysics::android());
        scroll.resize(ScrollExtents::new(300.0, 1200.0));
        let mut tickers = Tickers::new();
        scroll.attach(&mut tickers);

        // Near the bottom *and then* flicked, which is what it takes to reach
        // the edge at all: a fling from rest at this velocity stops around 718,
        // short of the 900 edge, and never enters the branch being tested.
        scroll.drag(-850.0);
        scroll.fling(-8000.0, ms(0));
        run_to_rest(&mut tickers);

        assert_eq!(
            scroll.signal().peek(),
            900.0,
            "the end of 1200 in a 300 window, exactly"
        );
    }

    #[test]
    fn a_tick_that_changes_nothing_does_not_mark_the_tree_pending() {
        // The direction the fix could have broken: publishing unconditionally
        // would be simpler and would keep the tree permanently pending, because
        // `Signal::set` notifies whether or not the value differs. That is the
        // spinning event loop this repository has fought before.
        let tree = ElementTree::new();
        let scroll = controller(&tree);
        let mut tickers = Tickers::new();
        scroll.attach(&mut tickers);

        scroll.fling(-2000.0, ms(0));
        run_to_rest(&mut tickers);

        let settled = tree.runtime().pending_count();
        tickers.advance(ms(20_000));
        assert_eq!(
            tree.runtime().pending_count(),
            settled,
            "a ticker with nothing to do must not schedule a rebuild"
        );
    }

    #[test]
    fn dropping_the_last_handle_stops_the_fling_being_ticked() {
        let tree = ElementTree::new();
        let mut tickers = Tickers::new();
        {
            let scroll = controller(&tree);
            scroll.attach(&mut tickers);
            scroll.fling(-2000.0, ms(0));
        }
        // The screen holding it went away mid-fling. Advancing must not panic
        // and must find nothing to advance.
        tickers.advance(ms(16));
        assert!(!tickers.is_animating());
    }
}

#[cfg(test)]
mod near_end_tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use vieww_gestures::ScrollPhysics;
    use vieww_widget::ScrollExtents;

    use super::ScrollController;
    use crate::Runtime;

    /// A controller over 1000pt of content in a 200pt window.
    fn scroller(runtime: &Runtime) -> ScrollController {
        let scroll = ScrollController::new(runtime, ScrollPhysics::android());
        scroll.resize(ScrollExtents::new(200.0, 1000.0));
        scroll
    }

    /// A counter a handler can bump, and a handle to read it.
    fn counter() -> (Rc<Cell<u32>>, impl Fn() + Clone) {
        let count = Rc::new(Cell::new(0_u32));
        let bump = {
            let count = Rc::clone(&count);
            move || count.set(count.get() + 1)
        };
        (count, bump)
    }

    #[test]
    fn it_fires_when_the_end_comes_into_range() {
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);
        assert_eq!(count.get(), 0, "the top of a long list is not near its end");

        // max_offset is 800; within 100 of it means past 700.
        scroll.drag(-650.0);
        assert_eq!(count.get(), 0);
        scroll.drag(-100.0);
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn it_does_not_fire_again_while_it_stays_there() {
        // **The reason this is edge-triggered.** The offset is republished on
        // every drag delta, so a level-triggered version fires on each of these
        // and an application sees a burst of identical page requests.
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);

        scroll.drag(-750.0);
        assert_eq!(count.get(), 1);
        for _ in 0..20 {
            scroll.drag(-1.0);
        }
        assert_eq!(
            count.get(),
            1,
            "twenty more drag deltas inside the zone asked for twenty more pages"
        );
    }

    #[test]
    fn growing_the_content_rearms_it_for_the_next_page() {
        // The sequence that makes it usable more than once: fetch, grow, rearm.
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);

        scroll.drag(-800.0);
        assert_eq!(count.get(), 1);

        // The page arrived, so the end moved away.
        scroll.resize(ScrollExtents::new(200.0, 2000.0));
        assert_eq!(
            count.get(),
            1,
            "growing the content is not itself an approach"
        );

        scroll.drag(-1000.0);
        assert_eq!(count.get(), 2, "the second approach must fire");
    }

    #[test]
    fn scrolling_back_up_and_down_again_fires_twice() {
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);

        scroll.drag(-800.0);
        assert_eq!(count.get(), 1);
        scroll.drag(500.0);
        scroll.drag(-500.0);
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn a_list_shorter_than_its_window_never_fires() {
        // Its end is always in view. An application that starts empty must not
        // ask for page two before page one has arrived.
        let runtime = Runtime::new();
        let scroll = ScrollController::new(&runtime, ScrollPhysics::android());
        scroll.resize(ScrollExtents::new(200.0, 50.0));
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);
        assert_eq!(count.get(), 0);

        scroll.drag(-10.0);
        assert_eq!(count.get(), 0);
    }

    #[test]
    fn attaching_to_an_already_scrolled_list_fires_immediately() {
        // Evaluated on attach rather than waiting for a scroll that may never
        // come — a list restored at the bottom would otherwise never page.
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        scroll.drag(-800.0);

        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);
        assert_eq!(count.get(), 1);
    }

    #[test]
    fn clearing_it_stops_the_reports() {
        let runtime = Runtime::new();
        let scroll = scroller(&runtime);
        let (count, bump) = counter();
        scroll.on_near_end(100.0, bump);
        scroll.clear_near_end();
        scroll.drag(-800.0);
        assert_eq!(count.get(), 0);
    }
}
