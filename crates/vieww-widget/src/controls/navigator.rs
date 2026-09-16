use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::{Alignment, Color, Key, Offset, Size};

use crate::{
    widget_node_from, Animated, BlockSemantics, BuildContext, ColoredBox, ExcludeSemantics,
    GestureDetector, Offstage, Opacity, SizedBox, Stack, StackFit, Transformed, Widget, WidgetKind,
    WidgetNode,
};

/// The surface a sliding route is assumed to travel across, until told
/// otherwise.
///
/// A guess, and a deliberately generous one — see [`Navigator::surface`].
const DEFAULT_SURFACE: Size = Size {
    width: 1024.0,
    height: 1024.0,
};

/// Builds a screen.
///
/// A closure rather than a [`WidgetNode`], and that is the whole design of this
/// module: a widget is a description that lives for one frame, so a route stored
/// as a node would be frozen at the moment it was pushed. Reconciliation would
/// see the identical `Rc` on every rebuild and skip the subtree — a screen that
/// never updates, which looks like a broken signal rather than a stale
/// description. Storing the *builder* means every rebuild produces a fresh
/// description from current state.
///
/// # A builder that captures its own navigator is a cycle
///
/// The usual shape — a route whose screen has a button that pushes another
/// route — has the builder holding the navigator, and the navigator holding the
/// builder. Both are `Rc`, so that is a reference cycle and the pair leaks when
/// the application drops them. It is bounded by the number of routes rather than
/// growing with use, and every UI toolkit with closures in a navigation stack
/// has it; breaking it needs a `Weak` handle the builder upgrades. Worth knowing
/// before wondering where the memory went.
pub type RouteBuilder = Rc<dyn Fn(&BuildContext) -> WidgetNode>;

/// How long a screen takes to arrive or leave.
///
/// Longer than a control's own state change, and for a reason: this is a
/// *change of place*, and the eye needs time to follow a whole screen where it
/// needs none to see a switch flip. Shorter than the 300ms most platforms use,
/// because a toolkit that feels slow is one people turn the animations off in.
pub const ROUTE_DURATION: Duration = Duration::from_millis(220);

/// How a screen arrives and leaves.
///
/// The fraction each variant is given runs `0.0` (off, or absent) to `1.0`
/// (settled), and the *same* fraction runs backwards on the way out — so a route
/// leaves the way it came rather than needing a second description.
#[derive(Clone, Default)]
#[non_exhaustive]
pub enum RouteTransition {
    /// Straight in. What a route with nothing to say about it gets, and what a
    /// test wants: an instant push is one frame rather than fourteen.
    #[default]
    None,
    /// Fades in place. For a dialog, and for anything whose position on screen
    /// is meaningful enough that moving it would be a lie.
    Fade,
    /// In from the trailing edge, out the same way. The push every phone does.
    SlideFromEnd,
    /// Up from the bottom. A sheet.
    SlideFromBottom,
    /// A transition of the application's own — see [`custom`](Self::custom).
    Custom(Rc<dyn Fn(WidgetNode, f32, Size) -> WidgetNode>),
}

impl RouteTransition {
    /// Any arrival at all, as a function of the screen and how far in it is.
    ///
    /// ```
    /// use vieww_widget::prelude::*;
    /// use vieww_widget::RouteTransition;
    ///
    /// // Grows into place from three-quarter size.
    /// let zoom = RouteTransition::custom(|screen, t, _surface| {
    ///     let size = 0.75 + 0.25 * t;
    ///     Opacity::new(t)
    ///         .child(Transformed::scale(size, size).child(screen))
    ///         .into()
    /// });
    /// # let _ = zoom;
    /// ```
    ///
    /// # Why this takes a closure where `Curve::custom` takes a `fn`
    ///
    /// Because the transitions people actually write capture something. A hero
    /// needs the rect it is flying from, a container transform needs the point
    /// it grew out of, and "slide from wherever the user came from" needs the
    /// direction. A `fn` pointer would have closed this seam on paper and left
    /// it useless for the cases it exists for.
    ///
    /// `Curve` makes the opposite trade for a stated reason — staying `Copy`,
    /// comparable and allocation-free, because a curve is sampled every frame of
    /// every animation. A transition is read once per frame of one route change
    /// and already lives inside a [`Route`], which holds an `Rc` builder and is
    /// not `Copy` either. So the cost lands where it was already paid.
    ///
    /// That is what this variant costs the enum: `Copy` and `Eq` are gone, and
    /// [`PartialEq`] compares two `Custom`s by pointer — the same closure shared
    /// is equal, two identical closures written twice are not.
    #[must_use]
    pub fn custom(wrap: impl Fn(WidgetNode, f32, Size) -> WidgetNode + 'static) -> Self {
        Self::Custom(Rc::new(wrap))
    }

    /// `true` for [`None`](Self::None) — nothing to animate, so nothing to wait
    /// for.
    ///
    /// The navigator skips its whole leaving mechanism on this, which is what
    /// keeps an application that wants instant navigation free of it.
    #[must_use]
    pub const fn is_instant(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Wrap `screen` as it looks `t` of the way in.
    ///
    /// Both the movement and the fade are paint-time — a
    /// [`Transformed`](crate::Transformed) and an [`Opacity`](crate::Opacity) —
    /// so a screen in flight costs no layout at all. That matters more here than
    /// anywhere: the thing being moved is an entire screen's worth of tree. A
    /// [`Custom`](Self::Custom) transition is handed the same job and should
    /// keep the same promise; nothing enforces it.
    #[must_use]
    pub fn wrap(&self, screen: WidgetNode, t: f32, surface: Size) -> WidgetNode {
        match self {
            Self::None => screen,
            Self::Fade => Opacity::new(t).child(screen).into(),
            Self::SlideFromEnd => {
                Transformed::translate(Offset::new(surface.width * (1.0 - t), 0.0))
                    .child(screen)
                    .into()
            }
            Self::SlideFromBottom => {
                Transformed::translate(Offset::new(0.0, surface.height * (1.0 - t)))
                    .child(screen)
                    .into()
            }
            Self::Custom(wrap) => wrap(screen, t, surface),
        }
    }
}

/// Hand-written: a closure is not `Debug`, and which *kind* of transition a
/// route has is the part a tree dump can use.
impl fmt::Debug for RouteTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::None => "None",
            Self::Fade => "Fade",
            Self::SlideFromEnd => "SlideFromEnd",
            Self::SlideFromBottom => "SlideFromBottom",
            Self::Custom(_) => "Custom(..)",
        };
        f.write_str(name)
    }
}

/// Hand-written, because two closures cannot be compared for behaviour.
///
/// `Custom` is equal to itself and to its own clones, by pointer. Two closures
/// written identically are two closures, which is the only answer available and
/// is why [`Eq`] is not implemented — the relation is about identity rather than
/// value, and claiming otherwise would invite `assert_eq!` on transitions that
/// look the same.
impl PartialEq for RouteTransition {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None)
            | (Self::Fade, Self::Fade)
            | (Self::SlideFromEnd, Self::SlideFromEnd)
            | (Self::SlideFromBottom, Self::SlideFromBottom) => true,
            (Self::Custom(a), Self::Custom(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// One screen on the navigator's stack.
#[derive(Clone)]
pub struct Route {
    name: String,
    builder: RouteBuilder,
    opaque: bool,
    transition: RouteTransition,
}

impl Route {
    /// A full-screen route that covers everything below it.
    #[must_use]
    pub fn new(name: impl Into<String>, builder: RouteBuilder) -> Self {
        Self {
            name: name.into(),
            builder,
            opaque: true,
            transition: RouteTransition::None,
        }
    }

    /// A route that lets what is beneath it show through — a dialog, a bottom
    /// sheet, a snackbar.
    ///
    /// What is beneath a modal route stays on stage: it is still laid out,
    /// painted, hit tested and read out, which is exactly what "shows through"
    /// means. An opaque route above it is what takes it off — see [`Navigator`].
    #[must_use]
    pub fn modal(name: impl Into<String>, builder: RouteBuilder) -> Self {
        Self {
            name: name.into(),
            builder,
            opaque: false,
            transition: RouteTransition::None,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How this screen arrives and leaves. [`None`](RouteTransition::None) by
    /// default — a transition is a choice about how an application feels, and
    /// defaulting to one would put it in every test's way.
    ///
    /// Not `const`: replacing the old transition drops it, and
    /// [`Custom`](RouteTransition::Custom) holds an `Rc` whose destructor cannot
    /// run at compile time. Nothing calls this in a const context — `Route`
    /// itself is not constructible in one, since it holds a boxed builder.
    #[must_use]
    pub fn transition(mut self, transition: RouteTransition) -> Self {
        self.transition = transition;
        self
    }

    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    #[must_use]
    /// By reference since [`RouteTransition::Custom`] holds an `Rc` and the enum
    /// is no longer `Copy`. Clone it to keep one past the borrow.
    pub const fn route_transition(&self) -> &RouteTransition {
        &self.transition
    }

    /// Build this route's screen.
    #[must_use]
    pub fn build(&self, ctx: &BuildContext) -> WidgetNode {
        (self.builder)(ctx)
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.name)
            .field("opaque", &self.opaque)
            .field("transition", &self.transition)
            .finish_non_exhaustive()
    }
}

/// A stack of screens, the topmost one on top.
///
/// ```
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Navigator, Route};
///
/// let settings = Route::new("settings", Rc::new(|_| Text::new("Settings").into()));
/// let detail = Route::new("wifi", Rc::new(|_| Text::new("Wi-Fi").into()));
///
/// let screen = Navigator::new(vec![settings, detail]);  // the detail is on top
/// ```
///
/// Controlled, like everything else here: it is handed a stack and draws it.
/// Pushing and popping is `vieww_element::NavigatorController`, which owns the
/// stack in a signal — the same split, and for the same reason, as
/// [`Scrollable`](crate::Scrollable) and its controller.
///
/// # Every route stays mounted
///
/// The whole stack is built into a [`Stack`], not just the top. That is what
/// makes going back cheap and what keeps a screen's state — a scroll position, a
/// half-typed field — alive underneath the screen that covers it. It is also
/// what makes a dialog possible at all, since a dialog is a route you can see
/// past.
///
/// The cost of that is bounded by [`Route::is_opaque`]: everything below the
/// topmost opaque route is wrapped in an [`Offstage`], so it keeps its state and
/// costs no layout, no paint, no hit test and no line in what a screen reader
/// reads out. A stack fifty screens deep draws one screen.
///
/// A [modal](Route::modal) route — a dialog, a sheet — does not hide what is
/// under it, so the search is for the *last opaque* route rather than for the
/// top one.
#[derive(Clone)]
pub struct Navigator {
    routes: Vec<Route>,
    leaving: Option<Route>,
    surface: Size,
    key: Option<Key>,
}

impl Navigator {
    #[must_use]
    pub fn new(routes: Vec<Route>) -> Self {
        Self {
            routes,
            leaving: None,
            surface: DEFAULT_SURFACE,
            key: None,
        }
    }

    /// A screen that has been popped and is still on its way out.
    ///
    /// It is drawn above the stack, because that is where it was, and its
    /// transition runs backwards. Handing it in rather than the navigator
    /// remembering it is the same split as everywhere else here:
    /// `vieww_element::NavigatorController` owns the timing, because holding a
    /// popped route for the length of its exit is state, and state needs a
    /// signal and a frame to tick it.
    #[must_use]
    pub fn leaving(mut self, route: Option<Route>) -> Self {
        self.leaving = route;
        self
    }

    /// How far a sliding route has to travel.
    ///
    /// Told rather than measured, because a transform is applied at paint time
    /// and layout is what knows the size — by the time this widget could ask,
    /// the answer would be a frame old. Overshooting is harmless: a screen
    /// pushed further off the edge than the edge actually is still arrives.
    #[must_use]
    pub const fn surface(mut self, surface: Size) -> Self {
        self.surface = surface;
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// How many screens are on the stack. A route on its way out is not one:
    /// it has already been popped, and the application has stopped counting it.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.routes.len()
    }

    /// The screen the user is looking at.
    #[must_use]
    pub fn current(&self) -> Option<&Route> {
        self.routes.last()
    }
}

impl Widget for Navigator {
    fn debug_name(&self) -> &'static str {
        "Navigator"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // Everything below the last opaque route is covered. `rposition` rather
        // than "the top one", because a dialog on top of a screen is a modal
        // route and the screen under it is still meant to be visible.
        //
        // One *less* than that is what actually goes offstage. The screen
        // immediately beneath the top one has to stay drawable, because it is
        // what a transition reveals — a push slides the new screen over it and a
        // pop slides the old screen off it, and both would otherwise happen over
        // a blank surface. The saving is unaffected in the case that matters: a
        // stack fifty deep still draws two.
        let hidden_below = self
            .routes
            .iter()
            .rposition(Route::is_opaque)
            .unwrap_or(0)
            .saturating_sub(1);

        // Everything below the top opaque route is **covered**, which is a wider
        // set than `hidden_below`: the screen immediately beneath keeps drawing
        // so a transition has something to reveal, and is covered all the same.
        //
        // Drawing and being reachable are different questions and this is where
        // they come apart. A screen reader walks the semantics tree rather than
        // the pixels, so a covered route left in it is one a user can swipe to
        // and activate without ever seeing it — and `handle_semantic_action`
        // dispatches by id, so the opaque screen on top absorbing taps is no
        // defence. Touch was always fine here; this was not.
        let covered = self.routes.iter().rposition(Route::is_opaque).unwrap_or(0);

        let mut screens: Vec<WidgetNode> = self
            .routes
            .iter()
            .enumerate()
            .map(|(index, route)| {
                self.screen(ctx, route, index < hidden_below, index < covered, true)
            })
            .collect();

        // On top of the live stack, because that is where it was when it was
        // popped, and running its transition backwards from anywhere else would
        // have it slide out from behind the screen it is revealing.
        if let Some(route) = &self.leaving {
            // Not covered — it is on top of everything while it animates out —
            // but hidden from a screen reader anyway. It is on its way off, and
            // announcing a screen the user is in the middle of leaving is how a
            // pop ends up read out as an arrival.
            screens.push(self.screen(ctx, route, false, true, false));
        }

        if screens.is_empty() {
            // An empty stack is a legitimate state — an app that has popped its
            // last screen — and a blank surface is the honest way to draw it.
            return SizedBox::shrink().into();
        }

        Stack::new().fit(StackFit::Expand).children(screens).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("depth", self.routes.len().to_string()),
            (
                "current",
                self.current()
                    .map_or_else(String::new, |route| route.name().to_owned()),
            ),
        ]
    }
}

impl fmt::Debug for Navigator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Navigator")
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}

impl Navigator {
    /// One screen, keyed, staged, and wrapped in whatever transition it asked
    /// for.
    ///
    /// `arriving` is what the transition animates *towards*: `1.0` for a route
    /// on the stack and `0.0` for one on its way out. The element is the same
    /// one either way — it is keyed by name and it was not replaced — so
    /// changing the target is what turns an entrance into an exit.
    fn screen(
        &self,
        ctx: &BuildContext,
        route: &Route,
        offstage: bool,
        unreadable: bool,
        arriving: bool,
    ) -> WidgetNode {
        // Cloned rather than borrowed: the builder below outlives this call, and
        // for a `Custom` transition this is one `Rc` bump.
        let transition = route.route_transition().clone();
        let surface = self.surface;
        // A `WidgetNode` rather than the `Stack` itself: the builder below is
        // called once per frame of the transition, and a node is an `Rc` to
        // clone rather than a description to rebuild.
        let screen: WidgetNode = Stack::new()
            .fit(StackFit::Expand)
            .push(route.build(ctx))
            .into();
        let target = if arriving { 1.0 } else { 0.0 };

        // Keyed by name, so pushing a screen leaves the ones below reconciled
        // against themselves rather than shifted by one — the difference between
        // going back to the settings page you left and going back to a fresh
        // one. The key belongs on the *outermost* node of the route; on anything
        // inside, a push would match by position again.
        // `ExcludeSemantics` **inside** `Offstage` rather than around it. An
        // offstage route is already unreadable — `skips_children` covers it —
        // so the outer position would be redundant there and would still have
        // to exist for the covered-but-drawn case. Inside, each wrapper answers
        // exactly one question, and the key stays on the outermost node where
        // reconciliation needs it.
        Offstage::new(offstage)
            .key(Key::from(route.name()))
            .child(
                ExcludeSemantics::new(unreadable).child(
                    Animated::themed(ctx, target)
                        // Off the edge on the way in, and only on the frame it
                        // is mounted: an element that already exists is
                        // somewhere.
                        .from(0.0)
                        .duration(ROUTE_DURATION)
                        .build(move |t| transition.wrap(screen.clone(), t, surface)),
                ),
            )
            .into()
    }
}

widget_node_from!(Navigator);

/// A full-surface scrim that swallows every tap that reaches it.
///
/// What sits under a dialog. Two jobs, both load-bearing: it stops taps reaching
/// the screen behind — which would let someone press a button they cannot see —
/// and it dims that screen so the dialog reads as being in front of it.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::ModalBarrier;
///
/// let scrim = ModalBarrier::new().on_dismiss(|| println!("tapped outside"));
/// ```
///
/// Tapping it dismisses only if [`on_dismiss`](Self::on_dismiss) is set. Without
/// it the barrier still absorbs the tap — a modal that ignores taps outside it
/// is a choice; one that lets them through to the screen behind is a bug.
#[derive(Clone)]
pub struct ModalBarrier {
    color: Color,
    on_dismiss: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Default for ModalBarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl ModalBarrier {
    /// A barrier at the usual scrim opacity.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            // Black at about a third. Dark enough to push the screen behind
            // back, light enough to still see what the dialog is about.
            color: Color::rgba(0, 0, 0, 0x59),
            on_dismiss: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Called when the barrier itself is tapped.
    #[must_use]
    pub fn on_dismiss(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for ModalBarrier {
    fn debug_name(&self) -> &'static str {
        "ModalBarrier"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // A gesture detector is opaque to hit testing whether or not it has
        // handlers, which is exactly the "absorbs the tap either way" behaviour
        // a barrier needs.
        let surface = ColoredBox::new(self.color).child(SizedBox::expand());
        let absorbing: WidgetNode = match &self.on_dismiss {
            Some(handler) => {
                let handler = Rc::clone(handler);
                GestureDetector::new()
                    .on_tap(move |_| handler())
                    .child(surface)
                    .into()
            }
            None => GestureDetector::new().child(surface).into(),
        };

        // The same job as the gesture detector, for the other input device.
        //
        // Absorbing taps only ever closed half the hole. A screen reader walks
        // the semantics tree rather than the pixels, and dispatches its actions
        // by id — so until this wrapper existed, a user could swipe onto a
        // button behind a dialog and activate it, with the barrier swallowing
        // nothing because no tap was ever involved.
        //
        // Unconditional, and not a builder the caller can turn off. Every
        // barrier in the framework is under something modal — `Dialog`,
        // `BottomSheet` and `Menu` each build one — and `Tooltip` deliberately
        // builds none precisely because it must not take the next interaction.
        // A barrier that let a screen reader through to the screen behind is the
        // same bug this type's own documentation already calls a bug for touch.
        BlockSemantics::new(true).child(absorbing).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("color", self.color.to_string()),
            ("dismissible", self.on_dismiss.is_some().to_string()),
        ]
    }
}

impl fmt::Debug for ModalBarrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModalBarrier")
            .field("color", &self.color)
            .field("dismissible", &self.on_dismiss.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(ModalBarrier);

/// Where an overlay sits on the surface.
///
/// Not an [`Alignment`] directly, so that the three things an overlay can be —
/// a dialog, a sheet, a snackbar — name themselves rather than being three
/// arbitrary pairs of numbers at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OverlayPosition {
    #[default]
    Center,
    Bottom,
    Top,
}

impl OverlayPosition {
    #[must_use]
    pub const fn alignment(self) -> Alignment {
        match self {
            Self::Center => Alignment::CENTER,
            Self::Bottom => Alignment::BOTTOM_CENTER,
            Self::Top => Alignment::TOP_CENTER,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{inflate, Text};

    use super::*;

    fn route(name: &'static str) -> Route {
        Route::new(name, Rc::new(move |_| Text::new(name).into()))
    }

    #[test]
    fn every_route_on_the_stack_is_built_not_just_the_top() {
        let tree = inflate(Navigator::new(vec![route("settings"), route("wifi")]));
        let texts: Vec<&str> = tree
            .find_all("Text")
            .iter()
            .filter_map(|node| node.property("text"))
            .collect();

        assert_eq!(
            texts,
            vec!["\"settings\"", "\"wifi\""],
            "a covered screen keeps its state only if it stays mounted"
        );
    }

    /// The `offstage` flag of each screen, bottom of the stack first.
    fn stages(navigator: Navigator) -> Vec<String> {
        inflate(navigator)
            .find_all("Offstage")
            .iter()
            .filter_map(|node| node.property("offstage").map(ToOwned::to_owned))
            .collect()
    }

    #[test]
    fn a_covered_screen_is_mounted_but_taken_off_the_stage() {
        // Both halves matter. Mounted, so going back returns you to the screen
        // you left rather than a fresh one; offstage, so a stack fifty deep does
        // not lay out and paint fifty screens on every frame.
        //
        // The screen *directly* beneath the top one stays on show: it is what a
        // push slides over and a pop reveals, and both would otherwise happen
        // against a blank surface.
        assert_eq!(
            stages(Navigator::new(vec![
                route("a"),
                route("b"),
                route("c"),
                route("settings"),
                route("wifi"),
            ])),
            vec!["true", "true", "true", "false", "false"]
        );
    }

    #[test]
    fn the_screen_a_transition_reveals_is_never_offstage() {
        assert_eq!(
            stages(Navigator::new(vec![route("settings"), route("wifi")])),
            vec!["false", "false"],
            "a two-deep stack has nothing to hide and a push to draw"
        );
    }

    #[test]
    fn a_transparent_route_does_not_hide_what_it_sits_on() {
        // A dialog over a screen. Hiding the screen underneath would leave the
        // dialog floating over a blank surface, which is the bug that comes from
        // taking "the top route" rather than "the last opaque one".
        let dialog = Route::modal("confirm", Rc::new(|_| Text::new("confirm").into()));
        assert_eq!(
            stages(Navigator::new(vec![
                route("a"),
                route("b"),
                route("settings"),
                dialog
            ])),
            vec!["true", "false", "false", "false"],
            "the dialog hides nothing, so the screen under it is the top opaque \
             one and the screen under *that* is what a transition reveals"
        );
    }

    #[test]
    fn the_top_of_the_stack_is_the_current_screen() {
        let navigator = Navigator::new(vec![route("settings"), route("wifi")]);
        assert_eq!(navigator.current().map(Route::name), Some("wifi"));
        assert_eq!(navigator.depth(), 2);
    }

    #[test]
    fn a_route_is_rebuilt_from_its_builder_rather_than_replayed() {
        let counter = Rc::new(std::cell::Cell::new(0));
        let counted = Rc::clone(&counter);
        let route = Route::new(
            "counter",
            Rc::new(move |_| {
                counted.set(counted.get() + 1);
                Text::new(format!("{}", counted.get())).into()
            }),
        );

        let navigator = Navigator::new(vec![route]);
        let _ = inflate(navigator.clone());
        let second = inflate(navigator);

        assert_eq!(
            second.find("Text").and_then(|node| node.property("text")),
            Some("\"2\""),
            "a stored node would show 1 forever"
        );
    }

    #[test]
    fn an_empty_stack_draws_nothing_rather_than_panicking() {
        let tree = inflate(Navigator::new(Vec::new()));
        assert!(tree.find("Text").is_none());
    }

    #[test]
    fn a_barrier_absorbs_taps_even_with_nothing_to_do_with_them() {
        let tree = inflate(ModalBarrier::new());
        assert!(
            tree.find("GestureDetector").is_some(),
            "a modal that lets taps through to the screen behind is a bug"
        );
    }

    #[test]
    fn the_three_overlay_positions_are_the_three_useful_alignments() {
        assert_eq!(OverlayPosition::Center.alignment(), Alignment::CENTER);
        assert_eq!(
            OverlayPosition::Bottom.alignment(),
            Alignment::BOTTOM_CENTER
        );
        assert_eq!(OverlayPosition::Top.alignment(), Alignment::TOP_CENTER);
    }
}
