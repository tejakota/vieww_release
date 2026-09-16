use std::any::Any;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{AnimationController, Curve};
use vieww_foundation::Key;

use crate::{widget_node_from, BuildContext, ElementState, Widget, WidgetKind, WidgetNode};

/// Builds a subtree from where an animation currently is.
pub type AnimatedBuilder = Rc<dyn Fn(f32) -> WidgetNode>;

/// How long a control's own state change takes to arrive on screen.
///
/// Short. This is a switch's thumb sliding and a press fading in, not a screen
/// transition: long enough to be read as movement rather than a jump, short
/// enough that a fast tapper never waits for it.
pub const CONTROL_DURATION: Duration = Duration::from_millis(120);

/// Moves smoothly to whatever number it is last given.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Animated;
///
/// # let on = true;
/// let thumb = Animated::new(if on { 1.0 } else { 0.0 })
///     .build(|t| Opacity::new(t).child(Text::new("on")).into());
/// ```
///
/// # One number, and the caller decides what it means
///
/// [`AnimatedContainer`](crate::AnimatedContainer) animates a fixed set of
/// container properties; this animates a bare `0.0 ..= 1.0` and hands it to a
/// builder. That covers the cases a property set cannot — a thumb sliding along
/// a track, a wash fading in, a route moving in from the edge — without this
/// widget needing to know about any of them.
///
/// # It continues from where it is, not from where it started
///
/// A target that changes mid-animation is picked up from the value on screen.
/// The alternative — restarting from the last *start* — makes a switch flicked
/// twice quickly jump backwards before setting off again.
///
/// # A fresh element does not animate, unless it is told where to start
///
/// By default the first target is where it *is*: animating from zero on mount
/// would make every switch on a settings screen slide into place on the frame
/// the screen appeared, which reads as broken rather than alive.
///
/// [`from`](Self::from) is the exception, and it is what an *entrance* is — a
/// route arriving has to begin off the edge of the screen and travel in, and
/// that is precisely an animation that starts on mount.
#[derive(Clone)]
pub struct Animated {
    target: f32,
    /// Where a *freshly mounted* element starts, when that is not the target.
    from: Option<f32>,
    duration: Duration,
    curve: Curve,
    builder: AnimatedBuilder,
    key: Option<Key>,
}

impl Animated {
    /// An animation resting at `target`, taking its timing from the ambient
    /// [`Motion`](crate::Motion) tokens.
    ///
    /// **This is what a control should call.** [`new`](Self::new) exists for
    /// code with no `BuildContext` to hand and keeps a fixed default, which
    /// means an application that sets `Motion::expressive()` would not reach it
    /// — the whole point of a motion token is that it does.
    ///
    /// Reads `duration_short` and `curve_standard`: an `Animated` drives one
    /// control's own state change, which is the small end of the scale. A
    /// caller that wants a different token sets it explicitly afterwards, and
    /// `.duration(theme.motion.duration_long)` reads better than a number.
    ///
    /// ```
    /// use vieww_widget::prelude::*;
    /// use vieww_widget::Animated;
    ///
    /// # fn example(ctx: &BuildContext, on: bool) -> WidgetNode {
    /// Animated::themed(ctx, if on { 1.0 } else { 0.0 })
    ///     .build(|t| Opacity::new(t).child(Text::new("on")).into())
    ///     .into()
    /// # }
    /// ```
    #[must_use]
    pub fn themed(ctx: &BuildContext, target: f32) -> Self {
        let motion = crate::ThemeData::of(ctx).motion;
        Self::new(target)
            .duration(motion.duration_short)
            .curve(motion.curve_standard)
    }

    /// An animation resting at `target`, with nothing to build yet.
    ///
    /// Uses [`CONTROL_DURATION`] and a fixed curve. Prefer
    /// [`themed`](Self::themed) anywhere a `BuildContext` is available.
    #[must_use]
    pub fn new(target: f32) -> Self {
        Self {
            target,
            from: None,
            duration: CONTROL_DURATION,
            curve: Curve::EASE_IN_OUT,
            builder: Rc::new(|_| crate::SizedBox::shrink().into()),
            key: None,
        }
    }

    /// What to build from the current value.
    #[must_use]
    pub fn build(mut self, builder: impl Fn(f32) -> WidgetNode + 'static) -> Self {
        self.builder = Rc::new(builder);
        self
    }

    /// Where to start from when this element is first mounted.
    ///
    /// Ignored on every later rebuild: an element that already exists is
    /// somewhere, and being told where it should have started would teleport it.
    #[must_use]
    pub const fn from(mut self, start: f32) -> Self {
        self.from = Some(start);
        self
    }

    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    #[must_use]
    pub const fn curve(mut self, curve: Curve) -> Self {
        self.curve = curve;
        self
    }

    /// Set the reconciliation key.
    ///
    /// Worth setting deliberately: the element holds the animation, so a key
    /// that changes replaces the element and the value starts at the new target
    /// rather than travelling to it.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The value being animated towards.
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.target
    }
}

impl Widget for Animated {
    fn debug_name(&self) -> &'static str {
        "Animated"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(AnimatedState::new(self)))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // The target is the honest answer with no state behind it — a tree dump
        // is a description, and a description is not partway anywhere.
        let value = ctx
            .state(|state: &AnimatedState| state.current)
            .unwrap_or(self.target);
        (self.builder)(value)
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("target", self.target.to_string())]
    }
}

impl fmt::Debug for Animated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Animated")
            .field("target", &self.target)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Animated);

/// Where an [`Animated`] currently is, carried across rebuilds.
#[derive(Debug)]
pub struct AnimatedState {
    from: f32,
    to: f32,
    current: f32,
    controller: AnimationController,
    /// A target that no tick has picked up yet.
    ///
    /// Deferred rather than started immediately for the reason
    /// [`AnimatedContainer`](crate::AnimatedContainer) defers its own: starting
    /// needs `now`, and the only honest source of it is the frame.
    pending: bool,
}

impl AnimatedState {
    fn new(widget: &Animated) -> Self {
        let start = widget.from.unwrap_or(widget.target);
        Self {
            from: start,
            to: widget.target,
            current: start,
            controller: AnimationController::new(widget.duration).curve(widget.curve),
            // Only when told to start somewhere else. Otherwise a fresh element
            // is already where it belongs and has nothing to travel.
            pending: (start - widget.target).abs() > f32::EPSILON,
        }
    }

    /// Where it is on screen right now.
    #[must_use]
    pub const fn current(&self) -> f32 {
        self.current
    }

    /// Where it is going.
    #[must_use]
    pub const fn target(&self) -> f32 {
        self.to
    }
}

impl ElementState for AnimatedState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn widget_updated(&mut self, widget: &WidgetNode) {
        let Some(widget) = widget.downcast_ref::<Animated>() else {
            return;
        };
        self.controller.set_duration(widget.duration);
        self.controller.set_curve(widget.curve);
        if (widget.target - self.to).abs() > f32::EPSILON {
            self.to = widget.target;
            self.pending = true;
        }
    }

    fn tick(&mut self, now: Duration) -> bool {
        if std::mem::take(&mut self.pending) {
            // From where it actually is: interrupting a half-finished move has
            // to continue from the screen.
            self.from = self.current;
            self.controller.set_value(0.0);
            self.controller.forward(now);
        }
        if !self.controller.tick(now) {
            return false;
        }
        self.current = self.from + (self.to - self.from) * self.controller.value();
        true
    }

    fn is_animating(&self) -> bool {
        self.pending || self.controller.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// Linear, so every assertion below is a value rather than a tolerance.
    fn widget(target: f32) -> Animated {
        Animated::new(target).duration(ms(100)).curve(Curve::Linear)
    }

    #[test]
    fn a_fresh_element_rests_at_its_target() {
        let state = AnimatedState::new(&widget(1.0));
        assert!((state.current() - 1.0).abs() < f32::EPSILON);
        assert!(
            !state.is_animating(),
            "a switch that slid into place on the frame its screen appeared \
             would read as broken, not as alive"
        );
    }

    #[test]
    fn a_new_target_is_travelled_to_rather_than_jumped_to() {
        let mut state = AnimatedState::new(&widget(0.0));
        state.widget_updated(&WidgetNode::from(widget(1.0)));
        assert!(state.is_animating(), "the change has to book a frame");

        state.tick(ms(0));
        assert!(state.current() < 0.01, "still at the start");

        state.tick(ms(50));
        let half = state.current();
        assert!(
            (0.4..0.6).contains(&half),
            "halfway through the duration is halfway there: {half}"
        );

        state.tick(ms(100));
        assert!((state.current() - 1.0).abs() < 0.01);
        assert!(!state.is_animating(), "and then it stops");
    }

    #[test]
    fn an_interrupted_animation_carries_on_from_the_screen() {
        let mut state = AnimatedState::new(&widget(0.0));
        state.widget_updated(&WidgetNode::from(widget(1.0)));
        state.tick(ms(0));
        state.tick(ms(50));
        let interrupted_at = state.current();

        // Flicked back before it arrived.
        state.widget_updated(&WidgetNode::from(widget(0.0)));
        state.tick(ms(50));

        assert!(
            (state.current() - interrupted_at).abs() < 0.01,
            "it must set off from where it was, not jump to where the last \
             animation began: {} vs {interrupted_at}",
            state.current()
        );
    }

    #[test]
    fn an_element_told_where_to_start_travels_from_there_on_mount() {
        // What an entrance is: a route arriving begins off the edge and comes
        // in, which is exactly an animation that starts the moment it mounts.
        let mut state = AnimatedState::new(&widget(1.0).from(0.0));
        assert!(state.is_animating());
        assert!(state.current() < 0.01, "it starts where it was told");

        state.tick(ms(0));
        state.tick(ms(100));
        assert!((state.current() - 1.0).abs() < 0.01);
    }

    #[test]
    fn rebuilding_with_the_same_target_starts_nothing() {
        let mut state = AnimatedState::new(&widget(1.0));
        state.widget_updated(&WidgetNode::from(widget(1.0)));
        assert!(
            !state.is_animating(),
            "a rebuild that changed nothing must not restart the animation, or \
             a screen that rebuilds every frame never finishes one"
        );
    }
}
