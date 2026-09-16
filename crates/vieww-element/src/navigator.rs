//! A navigation stack whose contents are a signal.
//!
//! The third join of the same shape as [`Animation`](crate::Animation) and
//! [`ScrollController`](crate::ScrollController): the widget layer describes a
//! stack of screens and cannot hold one, because a widget is a description that
//! lives for a frame. The stack itself is state, and state that a *tap* changes
//! needs something that both mutates it and marks the tree pending — a
//! [`Signal`], which lives here.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{Ticker, Tickers};
use vieww_widget::{Handler, Route, ROUTE_DURATION};

use crate::{Runtime, Signal};

/// A screen that has been popped and has not finished leaving.
///
/// Held here rather than dropped on the spot, because a route removed from the
/// stack is unmounted on the next rebuild — and a screen that is unmounted
/// cannot slide anywhere. Something has to outlive the pop by exactly the length
/// of the transition, and this is that thing.
struct Leaving {
    route: Route,
    /// When the pop happened, or `None` until a frame has told us. A handler has
    /// no honest clock; the frame does.
    since: Option<Duration>,
}

/// Drives the exit: publishes the route on its way out, and drops it when it has
/// gone.
struct Exit {
    leaving: Option<Leaving>,
    signal: Signal<Option<Route>>,
}

impl Ticker for Exit {
    fn tick(&mut self, now: Duration) -> bool {
        let Some(leaving) = &mut self.leaving else {
            return false;
        };
        let since = *leaving.since.get_or_insert(now);
        if now.saturating_sub(since) < ROUTE_DURATION {
            return false;
        }
        // Gone. Dropping it here is what finally unmounts the screen, and it is
        // deliberately the *only* place that happens: a route removed any
        // earlier would vanish mid-slide.
        self.leaving = None;
        self.signal.set(None);
        true
    }

    fn is_animating(&self) -> bool {
        self.leaving.is_some()
    }
}

impl fmt::Debug for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Exit")
            .field(
                "leaving",
                &self.leaving.as_ref().map(|it| it.route.name().to_owned()),
            )
            .finish()
    }
}

/// The stack of screens an application is showing, and the push and pop that
/// change it.
///
/// Cheap to clone; every clone is a handle to the same stack.
///
/// ```
/// use std::rc::Rc;
/// use vieww_element::{ElementTree, NavigatorController};
/// use vieww_widget::{Route, Text};
///
/// let mut tree = ElementTree::new();
/// let nav = NavigatorController::new(
///     tree.runtime(),
///     Route::new("home", Rc::new(|_| Text::new("Home").into())),
/// );
///
/// nav.push(Route::new("detail", Rc::new(|_| Text::new("Detail").into())));
/// assert_eq!(nav.depth(), 2);
/// assert_eq!(nav.current_name().as_deref(), Some("detail"));
///
/// assert!(nav.pop());
/// assert_eq!(nav.depth(), 1);
/// assert!(!nav.pop(), "the last screen cannot be popped");
/// ```
#[derive(Clone)]
pub struct NavigatorController {
    routes: Signal<Vec<Route>>,
    /// The screen on its way out, if any, and the clock that retires it.
    exit: Rc<RefCell<Exit>>,
    leaving: Signal<Option<Route>>,
}

impl NavigatorController {
    /// A stack showing `home`, which cannot be popped away.
    #[must_use]
    pub fn new(runtime: &Runtime, home: Route) -> Self {
        let leaving = runtime.signal(None);
        Self {
            routes: runtime.signal(vec![home]),
            exit: Rc::new(RefCell::new(Exit {
                leaving: None,
                signal: leaving.clone(),
            })),
            leaving,
        }
    }

    /// Register with a frame's tickers, so a popped screen is retired once it
    /// has finished leaving.
    ///
    /// A navigator that is never attached still works and never animates out: a
    /// pop takes effect immediately, which is the behaviour there was before
    /// transitions existed. Nothing is left half-gone.
    pub fn attach(&self, tickers: &mut Tickers) {
        tickers.add(&self.exit);
    }

    /// The screen that has been popped and is still sliding away, subscribing
    /// the element that is building.
    ///
    /// Handed to [`Navigator::leaving`](vieww_widget::Navigator::leaving).
    #[must_use]
    pub fn leaving(&self) -> Option<Route> {
        self.leaving.get()
    }

    /// The stack, subscribing the element that is building.
    ///
    /// This is what [`Navigator`](vieww_widget::Navigator) is handed, and the
    /// read that makes a push rebuild exactly the element displaying the stack.
    #[must_use]
    pub fn routes(&self) -> Vec<Route> {
        self.routes.get()
    }

    /// The stack, without subscribing. For handlers and tests.
    #[must_use]
    pub fn peek(&self) -> Vec<Route> {
        self.routes.peek()
    }

    /// How many screens deep.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.routes.peek().len()
    }

    /// The name of the screen on top.
    #[must_use]
    pub fn current_name(&self) -> Option<String> {
        self.routes
            .peek()
            .last()
            .map(|route| route.name().to_owned())
    }

    /// Show `route` on top of what is there.
    pub fn push(&self, route: Route) {
        let mut routes = self.routes.peek();
        routes.push(route);
        self.routes.set(routes);
    }

    /// Go back one screen.
    ///
    /// Returns `false` — and changes nothing — when there is only the home
    /// screen left. An application that popped its last screen would be showing
    /// a blank surface, and the platform's own back gesture routinely tries to:
    /// on Android, back at the root means *leave the app*, which is the
    /// platform's decision to make rather than this one's.
    pub fn pop(&self) -> bool {
        let mut routes = self.routes.peek();
        if routes.len() <= 1 {
            return false;
        }
        let gone = routes.pop().expect("checked non-empty");
        self.routes.set(routes);
        self.start_leaving(gone);
        true
    }

    /// Hand a popped route to the exit, replacing whatever was already on its
    /// way out.
    ///
    /// One at a time, deliberately. Popping twice quickly is a real thing — a
    /// back button held down, `pop_to_root` — and the second screen is the one
    /// the user is watching; keeping a queue of them would draw a pile of
    /// screens nobody asked to see.
    fn start_leaving(&self, route: Route) {
        if route.route_transition().is_instant() {
            // Nothing to wait for. Skipping the whole mechanism here is what
            // keeps an application that wants instant navigation — and every
            // test that has not asked for a transition — free of it.
            return;
        }
        self.exit.borrow_mut().leaving = Some(Leaving {
            route: route.clone(),
            since: None,
        });
        self.leaving.set(Some(route));
    }

    /// Go back to the home screen.
    pub fn pop_to_root(&self) {
        let mut routes = self.routes.peek();
        if routes.len() <= 1 {
            return;
        }
        let gone = routes.pop().expect("checked non-empty");
        routes.truncate(1);
        self.routes.set(routes);
        // Only the top screen was ever visible, so it is the only one with
        // anywhere to go.
        self.start_leaving(gone);
    }

    /// Replace the top screen rather than stacking on it.
    ///
    /// A login screen becoming the app it logged into: going *back* to it would
    /// be wrong, so it should not be on the stack.
    pub fn replace(&self, route: Route) {
        let mut routes = self.routes.peek();
        routes.pop();
        routes.push(route);
        self.routes.set(routes);
    }

    /// A handler that pops one screen, for handing to a back button.
    #[must_use]
    pub fn on_pop(&self) -> Rc<dyn Fn()> {
        let this = self.clone();
        Rc::new(move || {
            this.pop();
        })
    }

    /// A handler that pushes `route`, for handing to a row that leads somewhere.
    #[must_use]
    pub fn on_push(&self, route: Route) -> Rc<dyn Fn()> {
        let this = self.clone();
        Rc::new(move || this.push(route.clone()))
    }

    /// A handler that pops whatever it is given, for a dialog's dismiss.
    #[must_use]
    pub fn on_dismiss<T: 'static>(&self) -> Handler<T> {
        let this = self.clone();
        Rc::new(move |_| {
            this.pop();
        })
    }
}

impl fmt::Debug for NavigatorController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigatorController")
            .field("depth", &self.depth())
            .field("current", &self.current_name())
            .field("leaving", &self.leaving.peek().map(|r| r.name().to_owned()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use vieww_widget::{RouteTransition, Text};

    use crate::ElementTree;

    use super::*;

    fn route(name: &'static str) -> Route {
        Route::new(name, Rc::new(move |_| Text::new(name).into()))
    }

    fn nav(tree: &ElementTree) -> NavigatorController {
        NavigatorController::new(tree.runtime(), route("home"))
    }

    #[test]
    fn pushing_and_popping_walk_the_stack() {
        let tree = ElementTree::new();
        let nav = nav(&tree);

        nav.push(route("settings"));
        nav.push(route("wifi"));
        assert_eq!(nav.depth(), 3);
        assert_eq!(nav.current_name().as_deref(), Some("wifi"));

        assert!(nav.pop());
        assert_eq!(nav.current_name().as_deref(), Some("settings"));
    }

    #[test]
    fn the_home_screen_cannot_be_popped_away() {
        let tree = ElementTree::new();
        let nav = nav(&tree);

        assert!(!nav.pop());
        assert_eq!(nav.depth(), 1, "a blank surface is not a screen");
    }

    #[test]
    fn popping_to_the_root_leaves_exactly_the_home_screen() {
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.push(route("a"));
        nav.push(route("b"));

        nav.pop_to_root();
        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.current_name().as_deref(), Some("home"));
    }

    #[test]
    fn replacing_swaps_the_top_without_deepening_the_stack() {
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.replace(route("app"));

        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.current_name().as_deref(), Some("app"));
    }

    #[test]
    fn a_push_marks_the_element_that_reads_the_stack() {
        let mut tree = ElementTree::new();
        let nav = nav(&tree);
        let routes = nav.routes.clone();

        // Mount something that reads the stack during its build.
        tree.mount(Text::new(format!("{} deep", routes.peek().len())));
        tree.rebuild_pending();

        nav.push(route("settings"));
        // The push wrote the signal, so the next frame has something to do.
        assert_eq!(nav.depth(), 2);
    }

    #[test]
    fn the_handlers_do_what_a_back_button_and_a_row_need() {
        let tree = ElementTree::new();
        let nav = nav(&tree);

        (nav.on_push(route("detail")))();
        assert_eq!(nav.current_name().as_deref(), Some("detail"));

        (nav.on_pop())();
        assert_eq!(nav.current_name().as_deref(), Some("home"));
    }

    #[test]
    fn a_popped_route_with_no_transition_is_gone_at_once() {
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.push(route("detail"));

        assert!(nav.pop());
        assert!(
            nav.leaving().is_none(),
            "nothing to wait for, so nothing is held — which is what keeps an \
             application that wants instant navigation free of the machinery"
        );
    }

    #[test]
    fn a_popped_route_with_a_transition_outlives_the_pop() {
        let mut tickers = Tickers::new();
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.attach(&mut tickers);
        nav.push(route("detail").transition(RouteTransition::SlideFromEnd));

        assert!(nav.pop());
        assert_eq!(nav.depth(), 1, "the stack is already back to one");
        assert_eq!(
            nav.leaving().map(|r| r.name().to_owned()),
            Some("detail".to_owned()),
            "and the screen is still there to slide, because a screen that has \
             been unmounted cannot go anywhere"
        );
    }

    #[test]
    fn a_leaving_route_is_retired_once_it_has_finished_leaving() {
        let mut tickers = Tickers::new();
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.attach(&mut tickers);
        nav.push(route("detail").transition(RouteTransition::Fade));
        nav.pop();

        tickers.advance(Duration::ZERO);
        assert!(
            nav.leaving().is_some(),
            "the clock starts on the first frame"
        );

        tickers.advance(ROUTE_DURATION / 2);
        assert!(nav.leaving().is_some(), "still on its way out");

        tickers.advance(ROUTE_DURATION + Duration::from_millis(16));
        assert!(
            nav.leaving().is_none(),
            "and then it is dropped, which is what unmounts the screen"
        );
    }

    #[test]
    fn popping_twice_keeps_only_the_screen_being_watched() {
        let mut tickers = Tickers::new();
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.attach(&mut tickers);
        nav.push(route("one").transition(RouteTransition::Fade));
        nav.push(route("two").transition(RouteTransition::Fade));

        nav.pop();
        nav.pop();
        assert_eq!(
            nav.leaving().map(|r| r.name().to_owned()),
            Some("one".to_owned()),
            "a queue of exiting screens would draw a pile nobody asked to see"
        );
    }

    #[test]
    fn a_navigator_nobody_ticks_still_navigates() {
        // Attaching is optional, and forgetting to must not leave a screen
        // half-gone forever.
        let tree = ElementTree::new();
        let nav = nav(&tree);
        nav.push(route("detail").transition(RouteTransition::Fade));

        assert!(nav.pop());
        assert_eq!(nav.depth(), 1);
        assert_eq!(nav.current_name().as_deref(), Some("home"));
    }
}
