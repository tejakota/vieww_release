//! Route transitions: how a screen arrives and how it leaves.
//!
//! # The approach
//!
//! The existing `NavigatorController` holds routes in a `Signal<Vec<Route>>`
//! and has an `Exit` ticker that retires popped routes. Transitions
//! compose with this by publishing *progress* signals alongside the
//! route change and wrapping route content in widgets that read them.
//!
//! No new framework mechanisms — the transition is a composed widget
//! pattern, not a navigator extension.
//!
//! # The two-progress model
//!
//! A transition is driven by *two* values:
//!
//! - **enter**: 0 → 1 as the new screen arrives
//! - **exit**: 1 → 0 as the old screen leaves
//!
//! They are not always complements. A push slide runs both; a modal
//! fade keeps the old screen still — only enter animates.
//!
//! # Playing a transition
//!
//! [`play_transition`] is a free function that:
//! 1. Creates the progress signals
//! 2. Builds both screens through the transition
//! 3. Starts the springs (from vieww-animation)
//! 4. Registers a ticker to advance them
//! 5. Returns the composed widget tree
//!
//! The application displays the returned tree; the navigator's own
//! stack management is unchanged.

use std::time::Duration;

use vieww_animation::{SpringAnimation as Spring, SpringPreset, Ticker, Tickers};
use vieww_widget::{Route, WidgetNode};

use crate::{Runtime, Signal};

/// A transition's progress values, exposed as signals.
///
/// # This value is also the transition's lifetime
///
/// [`Tickers`] holds its entries **weakly**, by design: dropping a screen has to
/// unregister that screen's animations, and a registry of strong references
/// would keep every route that had ever animated alive for the life of the
/// application. The consequence is that whoever registers a ticker must hold the
/// strong reference themselves, and [`Timeline::play`](crate::Timeline::play)
/// does exactly that by returning a `TimelineHandle`.
///
/// [`play_transition`] used to hand `tickers.add` a freshly-created `Rc` that
/// lived only for the duration of that one statement. The registry downgraded
/// it, the temporary was dropped at the semicolon, and every subsequent weak
/// upgrade failed — so the player was never ticked again. Nothing crashed and
/// nothing logged: the progress signals simply stayed at rest, and every route
/// transition that flowed through this path silently never moved.
///
/// So the player rides here, in the value the caller is already required to keep
/// in order to read `enter` and `exit`. Dropping the progress still unregisters
/// the animation, which is the behaviour the weak registry exists to provide.
#[derive(Clone, Debug)]
pub struct TransitionProgress {
    /// 0 → 1 as the new screen arrives.
    pub enter: Signal<f32>,
    /// 1 → 0 as the old screen leaves.
    pub exit: Signal<f32>,
    /// The strong reference that keeps the registered player alive.
    ///
    /// `None` until [`play_transition_with`] registers one — a progress built by
    /// [`new`](Self::new) has no player yet, which is what a custom driver that
    /// ticks the signals itself wants.
    player: Option<std::rc::Rc<std::cell::RefCell<TransitionPlayer>>>,
}

impl TransitionProgress {
    /// Create progress signals starting at rest.
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            enter: runtime.signal(0.0),
            exit: runtime.signal(1.0),
            player: None,
        }
    }

    /// `true` while the transition's springs are still moving.
    ///
    /// `false` for a progress that was never given a player, which is the same
    /// answer a settled one gives: neither is going to change on its own.
    #[must_use]
    pub fn is_animating(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|player| player.borrow().is_animating())
    }
}

/// How a route's screens animate during a push or a pop.
///
/// Implement this for custom transitions. The trait receives the route
/// and the progress signals; the implementation wraps the route's
/// content in widgets that read the signals.
pub trait RouteTransition {
    /// Build the incoming screen, reading `progress.enter`.
    fn animate_in(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode;

    /// Build the outgoing screen, reading `progress.exit`.
    fn animate_out(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode;

    /// The spring preset for this transition's feel.
    fn preset(&self) -> SpringPreset {
        SpringPreset::Standard
    }
}

/// The built-in transition library.
pub mod transitions {
    use super::*;

    /// The incoming screen slides in from the right; the old slides left
    /// with parallax.
    ///
    /// The canonical push. Both screens move, which sells the depth.
    #[derive(Debug)]
    pub struct SlideRight;

    impl RouteTransition for SlideRight {
        fn animate_in(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode {
            let enter = progress.enter.clone();
            let content = route.build(&vieww_widget::BuildContext::root());

            TranslateBySignal {
                progress: enter,
                dx: -1.0, // enter=0 → offset -1 (off-screen left)
                dy: 0.0,
                child: content,
            }
            .into()
        }

        fn animate_out(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode {
            let exit = progress.exit.clone();
            let content = route.build(&vieww_widget::BuildContext::root());

            // Parallax: the old screen moves at 30% speed.
            TranslateBySignal {
                progress: exit,
                dx: 0.3, // exit=1 → offset 0.3 (slightly left)
                dy: 0.0,
                child: content,
            }
            .into()
        }

        fn preset(&self) -> SpringPreset {
            SpringPreset::Standard
        }
    }

    /// Both screens fade — the new one in, the old one out.
    ///
    /// For peer navigation where neither screen is "above" the other.
    #[derive(Debug)]
    pub struct Fade;

    impl RouteTransition for Fade {
        fn animate_in(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode {
            let enter = progress.enter.clone();
            let content = route.build(&vieww_widget::BuildContext::root());

            OpacityBySignal {
                progress: enter,
                child: content,
            }
            .into()
        }

        fn animate_out(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode {
            let exit = progress.exit.clone();
            let content = route.build(&vieww_widget::BuildContext::root());

            OpacityBySignal {
                progress: exit,
                child: content,
            }
            .into()
        }
    }

    /// The new screen slides up from the bottom; the old stays still.
    ///
    /// For modals and sheets.
    #[derive(Debug)]
    pub struct SlideUp;

    impl RouteTransition for SlideUp {
        fn animate_in(&self, route: &Route, progress: &TransitionProgress) -> WidgetNode {
            let enter = progress.enter.clone();
            let content = route.build(&vieww_widget::BuildContext::root());

            TranslateBySignal {
                progress: enter,
                dx: 0.0,
                dy: 1.0, // enter=0 → offset +1 (below screen)
                child: content,
            }
            .into()
        }

        fn animate_out(&self, route: &Route, _progress: &TransitionProgress) -> WidgetNode {
            // The covered screen does not animate.
            route.build(&vieww_widget::BuildContext::root())
        }

        fn preset(&self) -> SpringPreset {
            SpringPreset::Expressive
        }
    }
}

// ── Internal widgets for the built-in transitions ─────────────────────────

/// A widget that translates its child based on a progress signal.
///
/// At progress=0, the child is offset by `(dx, dy)` multiplied by the
/// viewport's relevant dimension. At progress=1, no offset.
#[derive(Debug)]
struct TranslateBySignal {
    progress: Signal<f32>,
    /// Horizontal travel as a **fraction of the viewport**, not pixels.
    dx: f32,
    /// Vertical travel as a fraction of the viewport.
    dy: f32,
    child: WidgetNode,
}

impl vieww_widget::Widget for TranslateBySignal {
    fn debug_name(&self) -> &'static str {
        "TranslateBySignal"
    }

    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }

    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let progress = self.progress.clone();
        let (dx, dy) = (self.dx, self.dy);
        let child = self.child.clone();

        // Through `LayoutBuilder`, so `dx`/`dy` are scaled by the space the
        // transition actually occupies.
        //
        // `dx` is a fraction — `-1.0` means "one screen to the left" — and
        // `Transform::translate` takes logical pixels. Handing the raw
        // fraction straight to it moved an incoming screen by at most one
        // pixel, so the transition looked like nothing happened. The
        // fraction only means something once multiplied by a real width,
        // and that width is not known until constraints arrive.
        vieww_widget::LayoutBuilder::new(move |constraints: vieww_foundation::Constraints| {
            let t = progress.get();
            let width = if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                0.0
            };
            let height = if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                0.0
            };

            let offset_x = dx * (1.0 - t) * width;
            let offset_y = dy * (1.0 - t) * height;

            vieww_widget::Transformed::translate(vieww_foundation::Offset::new(offset_x, offset_y))
                .child(child.clone())
                .into()
        })
        .into()
    }
}

/// A widget that adjusts opacity based on a progress signal.
#[derive(Debug)]
struct OpacityBySignal {
    progress: Signal<f32>,
    child: WidgetNode,
}

impl vieww_widget::Widget for OpacityBySignal {
    fn debug_name(&self) -> &'static str {
        "OpacityBySignal"
    }

    fn kind(&self) -> vieww_widget::WidgetKind<'_> {
        vieww_widget::WidgetKind::Composed
    }

    fn build(&self, _ctx: &vieww_widget::BuildContext) -> WidgetNode {
        let t = self.progress.get().clamp(0.0, 1.0);
        vieww_widget::Opacity::new(t)
            .child(self.child.clone())
            .into()
    }
}

// ── The transition player ─────────────────────────────────────────────────

/// Drives a transition's springs, writing to the progress signals.
///
/// Implements `Ticker` so the existing frame pipeline advances it.
#[derive(Debug)]
struct TransitionPlayer {
    enter_spring: Spring,
    exit_spring: Spring,
    enter_signal: Signal<f32>,
    exit_signal: Signal<f32>,
}

impl Ticker for TransitionPlayer {
    /// Cut rather than slide: see
    /// [`Ticker::settle`](vieww_animation::Ticker::settle).
    ///
    /// A route transition is the largest piece of motion an application
    /// performs — a whole screen travelling across the viewport, often with
    /// parallax behind it — which makes it the single thing a reduced-motion
    /// preference is most asking not to see. Both progress signals land at their
    /// end, so the incoming screen is simply *there* and the outgoing one is
    /// simply gone, which is what the platform navigators do under the same
    /// setting.
    fn settle(&mut self, _now: Duration) -> bool {
        self.enter_spring.settle(_now);
        self.exit_spring.settle(_now);
        self.enter_signal.set(self.enter_spring.value());
        self.exit_signal.set(self.exit_spring.value());
        false
    }

    fn tick(&mut self, now: Duration) -> bool {
        let enter_alive = self.enter_spring.tick(now);
        let exit_alive = self.exit_spring.tick(now);

        self.enter_signal.set(self.enter_spring.value());
        self.exit_signal.set(self.exit_spring.value());

        enter_alive || exit_alive
    }

    fn is_animating(&self) -> bool {
        self.enter_spring.is_animating() || self.exit_spring.is_animating()
    }
}

/// Play a route transition.
///
/// Creates the progress signals, builds both screens through the
/// transition, starts the springs, registers a ticker, and returns
/// the widget tree to display (a Stack of both screens).
///
/// This is a free function rather than a method on `NavigatorController`
/// because the navigator does not need to know about transitions —
/// they compose with it.
///
/// # Examples
///
/// ```ignore
/// let (tree, progress) = play_transition(
///     &runtime,
///     driver.tickers(),
///     &transitions::SlideRight,
///     &incoming_route,
///     outgoing_route.as_ref(),
/// );
/// driver.set_root(tree);
/// ```
pub fn play_transition(
    runtime: &Runtime,
    tickers: &mut Tickers,
    transition: &dyn RouteTransition,
    incoming: &Route,
    outgoing: Option<&Route>,
) -> (WidgetNode, TransitionProgress) {
    play_transition_with(runtime, tickers, transition, incoming, outgoing, None)
}

/// [`play_transition`], with the spring taken from the application's
/// [`Motion`](vieww_widget::Motion) tokens.
///
/// A route change is the largest piece of motion an application performs, so it
/// is the last place that should be running a preset chosen inside the
/// framework. Pass `Some(theme.motion)` and a brand that has declared itself
/// calm or playful gets a route transition that agrees with its buttons.
///
/// `spatial_slow`, because a whole screen is the largest thing that moves.
/// `None` falls back to the transition's own [`RouteTransition::preset`], which
/// is what the built-ins were tuned with.
pub fn play_transition_with(
    runtime: &Runtime,
    tickers: &mut Tickers,
    transition: &dyn RouteTransition,
    incoming: &Route,
    outgoing: Option<&Route>,
    motion: Option<vieww_widget::Motion>,
) -> (WidgetNode, TransitionProgress) {
    let mut progress = TransitionProgress::new(runtime);

    // Build both screens through the transition.
    let incoming_node = transition.animate_in(incoming, &progress);
    let outgoing_node = outgoing.map(|r| transition.animate_out(r, &progress));

    // Create the springs: enter goes 0→1, exit goes 1→0.
    let preset = motion.map_or_else(|| transition.preset(), |m| m.spatial_slow);
    let mut enter_spring = Spring::new(0.0, preset);
    enter_spring.retarget(1.0);

    let mut exit_spring = Spring::new(1.0, preset);
    exit_spring.retarget(0.0);

    // Register the player, and **keep the strong reference**.
    //
    // `Tickers` stores weak references, so the `Rc` handed to `add` has to
    // outlive the statement or the registry's next upgrade fails and the
    // transition never moves. It rides in `progress`, which the caller already
    // has to hold on to; see [`TransitionProgress`] for the whole argument.
    let player = std::rc::Rc::new(std::cell::RefCell::new(TransitionPlayer {
        enter_spring,
        exit_spring,
        enter_signal: progress.enter.clone(),
        exit_signal: progress.exit.clone(),
    }));
    tickers.add(&player);
    progress.player = Some(player);

    // The widget tree: a Stack with the outgoing below the incoming.
    let mut stack = vieww_widget::Stack::new();
    if let Some(out) = outgoing_node {
        stack = stack.push(out);
    }
    stack = stack.push(incoming_node);

    (stack.into(), progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A route with nothing in it — these tests are about the springs, not the
    /// screens.
    fn route(name: &str) -> Route {
        Route::new(
            name,
            std::rc::Rc::new(|_: &vieww_widget::BuildContext| {
                vieww_widget::SizedBox::shrink().into()
            }),
        )
    }

    /// **The regression test for the bug that made every route transition
    /// silently stand still.**
    ///
    /// `play_transition` handed `Tickers::add` — a registry of *weak*
    /// references — an `Rc` created inline, which died at the semicolon. From
    /// the next frame on the upgrade failed, the player was dropped from the
    /// registry, and the progress signals stayed exactly where they started.
    /// Nothing panicked and nothing logged, so the only observable symptom was
    /// a screen that appeared without moving.
    ///
    /// So this asserts the one thing that distinguishes a live transition from
    /// a dead one: after a tick, `enter` has left zero. It is deliberately not
    /// an assertion about *where* it got to — that is the spring's business and
    /// retuning a preset should not fail this.
    #[test]
    fn a_transitions_progress_moves_after_a_tick() {
        let runtime = Runtime::new();
        let mut tickers = Tickers::new();
        let (_tree, progress) = play_transition(
            &runtime,
            &mut tickers,
            &transitions::SlideRight,
            &route("incoming"),
            Some(&route("outgoing")),
        );

        assert_eq!(progress.enter.peek(), 0.0, "starts at rest");
        assert_eq!(
            tickers.len(),
            1,
            "the player must still be registered — a weak entry whose strong \
             reference was a temporary is already gone by here"
        );

        tickers.advance(Duration::from_millis(0));
        tickers.advance(Duration::from_millis(16));
        tickers.advance(Duration::from_millis(32));

        assert!(
            progress.enter.peek() > 0.0,
            "the enter spring has to have moved; it read {} after three frames",
            progress.enter.peek()
        );
        assert!(progress.exit.peek() < 1.0, "and the exit spring with it");
    }

    /// Dropping the progress is what unregisters the animation — the reason the
    /// registry is weak in the first place. Keeping the player alive must not
    /// have turned that into a leak.
    #[test]
    fn dropping_the_progress_unregisters_the_player() {
        let runtime = Runtime::new();
        let mut tickers = Tickers::new();
        let (_tree, progress) = play_transition(
            &runtime,
            &mut tickers,
            &transitions::Fade,
            &route("incoming"),
            None,
        );
        assert_eq!(tickers.len(), 1);
        drop(progress);
        assert_eq!(tickers.len(), 0, "a dropped screen stops being ticked");
    }

    /// The distance an incoming screen travels at `progress`, on a viewport
    /// `width` wide. Mirrors `TranslateBySignal::build`.
    fn travel(dx_fraction: f32, progress: f32, width: f32) -> f32 {
        dx_fraction * (1.0 - progress) * width
    }

    #[test]
    fn a_slide_travels_a_fraction_of_the_viewport_not_a_fraction_of_a_pixel() {
        // A full-screen slide-in from the right on an 800px viewport starts
        // 800px off-screen, not 1px.
        assert_eq!(travel(-1.0, 0.0, 800.0), -800.0);
        assert_eq!(travel(-1.0, 0.5, 800.0), -400.0, "half way in");
        assert_eq!(travel(-1.0, 1.0, 800.0), 0.0, "arrived");

        // And it scales with the viewport: the same transition on a phone
        // moves less far in pixels than on a desktop.
        assert!(
            travel(-1.0, 0.0, 390.0).abs() < travel(-1.0, 0.0, 1440.0).abs(),
            "travel must depend on the viewport"
        );
    }

    #[test]
    fn transition_progress_starts_at_rest() {
        let runtime = Runtime::new();
        let progress = TransitionProgress::new(&runtime);

        assert_eq!(progress.enter.peek(), 0.0);
        assert_eq!(progress.exit.peek(), 1.0);
    }
}

vieww_widget::widget_node_from!(TranslateBySignal);
vieww_widget::widget_node_from!(OpacityBySignal);
