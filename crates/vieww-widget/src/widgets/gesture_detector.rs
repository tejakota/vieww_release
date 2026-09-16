use std::fmt;
use std::rc::Rc;

use vieww_foundation::{
    Axis, DragDetails, Key, LongPressDetails, ScaleDetails, ScrollEvent, TapDetails,
};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// A callback a gesture invokes.
///
/// `Rc` rather than `Box` because a widget is cloned as it is reconciled, and
/// `dyn Fn` rather than `FnMut` because a handler's job is to *set a signal* —
/// the state lives in the element tree, not captured in the closure. A handler
/// that needs to accumulate should read and write a signal, so that the element
/// tree knows something changed.
pub type Handler<T> = Rc<dyn Fn(T)>;

/// The handlers a gesture detector was given.
///
/// A struct rather than nine fields copied one at a time into the render object:
/// the render layer builds a `RenderGestureDetector` from this on every rebuild,
/// and a field added here that somebody forgets to copy there is a handler that
/// silently never fires.
#[derive(Clone, Default)]
pub struct GestureHandlers {
    pub on_tap: Option<Handler<TapDetails>>,
    /// A right-click. A separate gesture from `on_tap`, contested separately.
    pub on_secondary_tap: Option<Handler<TapDetails>>,
    /// The mouse's back thumb button, on the mice that have one.
    ///
    /// Its own handler rather than a framework-level "pop the navigator",
    /// because what "back" means belongs to the screen: a form with unsaved
    /// edits, a modal, a wizard halfway through and a list all want different
    /// things from it. See `PointerButton::Back`.
    pub on_back_tap: Option<Handler<TapDetails>>,
    /// The mouse's forward thumb button. See [`on_back_tap`](Self::on_back_tap).
    pub on_forward_tap: Option<Handler<TapDetails>>,
    /// The cursor entered (`true`) or left (`false`). Not a gesture: hover has
    /// no press to be routed from, so it is hit tested where the cursor is.
    pub on_hover: Option<Handler<bool>>,
    /// A wheel or trackpad scroll landed here. Return value ignored — a widget
    /// that took it says so by having a handler at all.
    ///
    /// **The whole event, not just its delta.** This used to hand over
    /// `Offset`, which threw away the `timestamp` — and a wheel is the one
    /// input that has to synthesise its own release, because there is no such
    /// thing as letting go of a wheel. A release needs a clock to start a spring
    /// on, and dropping the only clock on the path meant the release could not
    /// be written at all. See [`Scrollable`](crate::Scrollable).
    pub on_scroll: Option<Handler<ScrollEvent>>,
    /// The press became worth showing, and may still become a tap.
    pub on_tap_down: Option<Handler<TapDetails>>,
    /// The press ended without becoming a tap. Only ever after `on_tap_down`.
    pub on_tap_cancel: Option<Handler<()>>,
    pub on_long_press: Option<Handler<LongPressDetails>>,
    pub on_drag_start: Option<Handler<DragDetails>>,
    pub on_drag_update: Option<Handler<DragDetails>>,
    pub on_drag_end: Option<Handler<DragDetails>>,
    pub drag_axis: Option<Axis>,
    pub on_scale_start: Option<Handler<ScaleDetails>>,
    pub on_scale_update: Option<Handler<ScaleDetails>>,
    pub on_scale_end: Option<Handler<ScaleDetails>>,
}

impl GestureHandlers {
    /// `true` if any tap handler is set — which is what decides whether a tap
    /// recogniser is registered at all.
    ///
    /// A detector that only wants the press still needs the whole tap gesture:
    /// only the arena knows whether a press is still a candidate, and a widget
    /// watching raw pointers instead would highlight itself under a finger that
    /// was scrolling past.
    #[must_use]
    pub const fn wants_tap(&self) -> bool {
        self.on_tap.is_some() || self.on_tap_down.is_some() || self.on_tap_cancel.is_some()
    }

    /// `true` if a right-click is wanted.
    #[must_use]
    pub const fn wants_secondary_tap(&self) -> bool {
        self.on_secondary_tap.is_some()
    }

    /// `true` if the back thumb button is wanted.
    #[must_use]
    pub const fn wants_back_tap(&self) -> bool {
        self.on_back_tap.is_some()
    }

    /// `true` if the forward thumb button is wanted.
    #[must_use]
    pub const fn wants_forward_tap(&self) -> bool {
        self.on_forward_tap.is_some()
    }

    /// `true` if any drag handler is set.
    #[must_use]
    pub const fn wants_drag(&self) -> bool {
        self.on_drag_start.is_some() || self.on_drag_update.is_some() || self.on_drag_end.is_some()
    }

    /// `true` if any scale handler is set.
    #[must_use]
    pub const fn wants_scale(&self) -> bool {
        self.on_scale_start.is_some()
            || self.on_scale_update.is_some()
            || self.on_scale_end.is_some()
    }

    /// Which gestures are wired, for tree dumps.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.on_tap.is_some() {
            names.push("tap");
        }
        if self.on_secondary_tap.is_some() {
            names.push("secondary_tap");
        }
        if self.on_back_tap.is_some() {
            names.push("back_tap");
        }
        if self.on_forward_tap.is_some() {
            names.push("forward_tap");
        }
        if self.on_hover.is_some() {
            names.push("hover");
        }
        if self.on_scroll.is_some() {
            names.push("scroll");
        }
        if self.on_tap_down.is_some() {
            names.push("tap_down");
        }
        if self.on_tap_cancel.is_some() {
            names.push("tap_cancel");
        }
        if self.on_long_press.is_some() {
            names.push("long_press");
        }
        if self.wants_drag() {
            names.push("drag");
        }
        if self.wants_scale() {
            names.push("scale");
        }
        names
    }
}

impl fmt::Debug for GestureHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Closures are not Debug and there is nothing informative to print about
        // one anyway; which gestures are wired is the useful part.
        f.debug_struct("GestureHandlers")
            .field("gestures", &self.names())
            .finish()
    }
}

/// Reacts to gestures on its child.
///
/// # Layout-transparent
///
/// The child sees exactly the constraints this widget was given and this widget
/// is exactly the size the child chose. Only the hit test changes: this makes its
/// bounds opaque to input, so a gesture on a transparent area still reaches it.
///
/// # Which gestures compete
///
/// A recogniser is registered for each handler that is set, and *only* those. A
/// detector with only `on_tap` does not enter a drag into the arena, so it never
/// takes a touch away from a scrollable it sits inside. That is why the handlers
/// are individually optional rather than one enum of "gestures I care about".
#[derive(Debug, Clone, Default)]
pub struct GestureDetector {
    handlers: GestureHandlers,
    focusable: bool,
    /// An override for the theme's minimum touch target — see
    /// [`touch_target`](Self::touch_target). `None` takes the theme's.
    touch_target: Option<f32>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl GestureDetector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fires on a press and release with no significant movement.
    #[must_use]
    pub fn on_tap(mut self, handler: impl Fn(TapDetails) + 'static) -> Self {
        self.handlers.on_tap = Some(Rc::new(handler));
        self
    }

    /// Override the minimum square this must be reachable across.
    ///
    /// The theme's `Metrics::touch_target` applies automatically and this is the
    /// way out of it — `Some(0.0)` for a control that must take taps only where
    /// it draws.
    ///
    /// # Use this almost never
    ///
    /// `docs/AIMS.md` §I's aim is that reach is **not opt-in**, and every
    /// escape hatch is a way back to the world where it was. The one considered
    /// exception in this repository is [`Chip`](crate::Chip), whose own docs
    /// argue density over reach because chips come in rows where an expanded
    /// target would overlap its neighbours on both sides.
    #[must_use]
    pub const fn touch_target(mut self, minimum: f32) -> Self {
        self.touch_target = Some(minimum);
        self
    }

    /// The override set by [`touch_target`](Self::touch_target), if any.
    #[must_use]
    pub const fn touch_target_override(&self) -> Option<f32> {
        self.touch_target
    }

    /// Fires on a right-click.
    ///
    /// Its own recogniser, contesting its own arena entry — so a right-click
    /// over a scrollable neither scrolls it nor is swallowed by it, and a
    /// detector that wants only this never competes for an ordinary tap.
    #[must_use]
    pub fn on_secondary_tap(mut self, handler: impl Fn(TapDetails) + 'static) -> Self {
        self.handlers.on_secondary_tap = Some(Rc::new(handler));
        self
    }

    /// Fires on the mouse's back thumb button.
    ///
    /// Its own recogniser and its own arena entry, exactly like
    /// [`on_secondary_tap`](Self::on_secondary_tap).
    ///
    /// **The framework does not decide what this means.** It could pop the
    /// navigator for you, and that would be wrong: a half-finished form, a
    /// modal, and a list want different things from "back", and a default would
    /// be overridden everywhere it mattered while firing exactly where it did
    /// harm. Wire it to `NavigatorController::pop` at the one place that knows.
    #[must_use]
    pub fn on_back_tap(mut self, handler: impl Fn(TapDetails) + 'static) -> Self {
        self.handlers.on_back_tap = Some(Rc::new(handler));
        self
    }

    /// Fires on the mouse's forward thumb button. See
    /// [`on_back_tap`](Self::on_back_tap).
    #[must_use]
    pub fn on_forward_tap(mut self, handler: impl Fn(TapDetails) + 'static) -> Self {
        self.handlers.on_forward_tap = Some(Rc::new(handler));
        self
    }

    /// Fires when the cursor enters (`true`) or leaves (`false`) these bounds.
    ///
    /// Mouse and stylus only, obviously — a finger is either touching or not
    /// there. A control that changes appearance on hover should look the same
    /// after a touch as before it, which falls out of this never firing.
    #[must_use]
    pub fn on_hover(mut self, handler: impl Fn(bool) + 'static) -> Self {
        self.handlers.on_hover = Some(Rc::new(handler));
        self
    }

    /// Fires for a wheel or trackpad scroll landing here, carrying how far to
    /// move the content and **when**.
    ///
    /// Having a handler is what *consumes* the scroll. Without one it passes
    /// outward to whatever is around this, which is how a wheel over a list
    /// inside a page reaches the page once the list has nothing left to give.
    ///
    /// The timestamp is on the same clock [`DragDetails::timestamp`] is, so a
    /// handler can start a simulation on it. That is not a convenience: a wheel
    /// has no release, so anything that needs one has to synthesise it here.
    #[must_use]
    pub fn on_scroll(mut self, handler: impl Fn(ScrollEvent) + 'static) -> Self {
        self.handlers.on_scroll = Some(Rc::new(handler));
        self
    }

    /// Fires once the press is worth showing, and might still become a tap.
    ///
    /// This is what a control highlights on. It is **not** the `Down` event:
    /// it arrives [`PRESS_TIMEOUT`](vieww_foundation::PRESS_TIMEOUT) later, or
    /// at the moment the tap wins if that comes first, so that a finger which
    /// was actually starting a scroll never lights anything up on its way past.
    ///
    /// Exactly one of [`on_tap`](Self::on_tap) or
    /// [`on_tap_cancel`](Self::on_tap_cancel) follows every one of these.
    #[must_use]
    pub fn on_tap_down(mut self, handler: impl Fn(TapDetails) + 'static) -> Self {
        self.handlers.on_tap_down = Some(Rc::new(handler));
        self
    }

    /// Fires when a press reported by [`on_tap_down`](Self::on_tap_down) ends
    /// without becoming a tap — the finger moved away, lifted too late, or a
    /// drag underneath won the arena.
    ///
    /// Never fires without a press having been reported first, so a control can
    /// un-highlight unconditionally rather than tracking whether it was lit.
    #[must_use]
    pub fn on_tap_cancel(mut self, handler: impl Fn() + 'static) -> Self {
        self.handlers.on_tap_cancel = Some(Rc::new(move |()| handler()));
        self
    }

    /// Fires on a press held still for [`LONG_PRESS_TIMEOUT`](vieww_foundation::LONG_PRESS_TIMEOUT).
    #[must_use]
    pub fn on_long_press(mut self, handler: impl Fn(LongPressDetails) + 'static) -> Self {
        self.handlers.on_long_press = Some(Rc::new(handler));
        self
    }

    /// Fires once, when a drag is recognised.
    #[must_use]
    pub fn on_drag_start(mut self, handler: impl Fn(DragDetails) + 'static) -> Self {
        self.handlers.on_drag_start = Some(Rc::new(handler));
        self
    }

    /// Fires for every move of a recognised drag.
    #[must_use]
    pub fn on_drag_update(mut self, handler: impl Fn(DragDetails) + 'static) -> Self {
        self.handlers.on_drag_update = Some(Rc::new(handler));
        self
    }

    /// Fires when the finger lifts, carrying the release velocity.
    #[must_use]
    pub fn on_drag_end(mut self, handler: impl Fn(DragDetails) + 'static) -> Self {
        self.handlers.on_drag_end = Some(Rc::new(handler));
        self
    }

    /// Only recognise drags along `axis`.
    ///
    /// A vertical list inside a horizontal pager needs this, or whichever of the
    /// two is innermost takes every touch regardless of direction.
    #[must_use]
    pub const fn drag_axis(mut self, axis: Axis) -> Self {
        self.handlers.drag_axis = Some(axis);
        self
    }

    #[must_use]
    pub fn on_scale_start(mut self, handler: impl Fn(ScaleDetails) + 'static) -> Self {
        self.handlers.on_scale_start = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn on_scale_update(mut self, handler: impl Fn(ScaleDetails) + 'static) -> Self {
        self.handlers.on_scale_update = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn on_scale_end(mut self, handler: impl Fn(ScaleDetails) + 'static) -> Self {
        self.handlers.on_scale_end = Some(Rc::new(handler));
        self
    }

    /// Make this a tab stop that Space and Enter activate.
    ///
    /// Off by default, and that is not laziness: a detector wrapped around a
    /// scrollable, a modal barrier or a whole row is not something a keyboard
    /// user should have to tab through, and a tree where everything is focusable
    /// makes Tab useless. A *control* opts in — which in practice means
    /// [`Pressable`](crate::Pressable) does it once, for all of them.
    ///
    /// Has no effect without [`on_tap`](Self::on_tap): a tab stop that does
    /// nothing when activated is a trap, and no handler is how everything here
    /// spells disabled.
    #[must_use]
    pub const fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// Set the child.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The handlers this detector was given — what the render layer builds from.
    #[must_use]
    pub const fn handlers(&self) -> &GestureHandlers {
        &self.handlers
    }

    /// Whether this is a tab stop — read by the render layer.
    #[must_use]
    pub const fn is_focusable(&self) -> bool {
        self.focusable
    }
}

impl Widget for GestureDetector {
    fn debug_name(&self) -> &'static str {
        "GestureDetector"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("gestures", self.handlers.names().join(", "))];
        if self.focusable {
            props.push(("focusable", "true".to_owned()));
        }
        props
    }
}

widget_node_from!(GestureDetector);
