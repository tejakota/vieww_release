use std::any::Any;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_animation::{AnimationController, Curve};
use vieww_foundation::Key;

use crate::{
    widget_node_from, BuildContext, ElementState, GestureDetector, Widget, WidgetKind, WidgetNode,
};

/// Builds a subtree that knows how pressed it is.
///
/// `0.0` is untouched and `1.0` is fully pressed; in between is the fade. A
/// fraction rather than a flag because the press *moves* — see [`Pressable`].
pub type PressBuilder = Rc<dyn Fn(Sense) -> WidgetNode>;

/// How a pointer is currently treating a [`Pressable`].
///
/// Two fractions rather than two flags, for the same reason a press is a
/// fraction: both fade, and a control that snapped between states would read as
/// a glitch at the speed a pointer actually moves.
///
/// # Why hover is here and not a widget of its own
///
/// A pointer that is *over* a control and a finger that is *on* one are the same
/// question asked by two input devices, and every control that answers one wants
/// to answer the other. Splitting them would mean every button in the library
/// wrapping a `GestureDetector` a second time, and — the part that actually
/// bites — two element states whose fades run on separate controllers and drift
/// apart mid-transition.
///
/// **Hover is `0.0` on a touch device and stays there.** Nothing synthesises it
/// from a tap: a phone has no pointer to be over anything, and a control that
/// lit up under a finger and stayed lit after it left is the bug this avoids.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sense {
    /// How pressed it looks right now, `0.0 ..= 1.0`.
    pub press: f32,
    /// How hovered it looks right now, `0.0 ..= 1.0`. Always `0.0` where there
    /// is no pointer.
    pub hover: f32,
}

impl Sense {
    /// Untouched and un-hovered — what a subtree built outside an element tree
    /// gets, and what `debug_tree` describes.
    pub const IDLE: Self = Self {
        press: 0.0,
        hover: 0.0,
    };

    /// The strongest of the two, which is what a single-wash control wants: a
    /// press should never look *less* lit than the hover it grew out of.
    #[must_use]
    pub fn emphasis(self) -> f32 {
        self.press.max(self.hover)
    }
}

/// A tappable region that keeps track of being pressed, and rebuilds when it
/// changes.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Pressable, foundation::Color};
///
/// let swatch = Pressable::new(|press| {
///     ColoredBox::new(if press > 0.5 { Color::BLUE } else { Color::WHITE }).into()
/// })
/// .on_tap(|| println!("tapped"));
/// ```
///
/// # The one widget here that owns its state
///
/// Every control in this library is *controlled* — handed a value, reporting
/// the value it would like to become — and `docs/DESIGN.md` §7 says why: durable
/// state a gesture changes has to be somewhere the application can read it,
/// which means a signal in the element layer.
///
/// Being pressed is not that. It lasts as long as a finger does, no application
/// wants to own it, and making every `Button` call site wire a signal for a
/// highlight is how a feature ends up written and never used. So it lives here,
/// in an [`ElementState`], written by the gesture handlers this widget builds
/// and turned into a rebuild by [`ElementState::take_pending`].
///
/// The rule that separates the two: **if the application would ever want to read
/// it, it does not go here.**
///
/// # Why it takes a builder rather than a child
///
/// A child would have to be built before this widget knew whether it was
/// pressed, which is the one thing the child needs. The closure is called on
/// every rebuild with the current answer.
///
/// # A tab stop, when there is something to activate
///
/// A pressable with an [`on_tap`](Self::on_tap) is focusable, and Space and
/// Enter run it — see
/// [`GestureDetector::focusable`](crate::GestureDetector::focusable). One
/// without stays out of the tab order rather than becoming a stop that does
/// nothing.
///
/// # The press fades rather than appearing
///
/// The builder is handed a fraction, and it travels between 0 and 1 over
/// [`CONTROL_DURATION`](crate::CONTROL_DURATION). A highlight that snaps on and
/// off reads as a glitch at the speed a finger actually moves; the same
/// highlight faded over a tenth of a second reads as a response.
///
/// The cost is an [`AnimationController`](crate::AnimationController) per
/// pressable, and it is worth being clear about what that does and does not
/// mean: the controller is inert unless the fraction is moving, so a list of two
/// hundred rows costs two hundred small structs and no frames.
#[derive(Clone)]
pub struct Pressable {
    builder: PressBuilder,
    on_tap: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
    /// How long the highlight takes to fade in and out.
    ///
    /// `None` means [`CONTROL_DURATION`](crate::CONTROL_DURATION), which is what
    /// a `Pressable` built with no `BuildContext` to hand gets.
    /// [`themed`](Self::themed) fills it from the ambient
    /// [`Motion`](crate::Motion) tokens instead.
    fade: Option<Duration>,
}

impl Pressable {
    /// A pressable region whose highlight fades at the ambient
    /// [`Motion`](crate::Motion) tokens' `duration_short`.
    ///
    /// The press wash is the most-seen micro-interaction in any application —
    /// every button, chip, row and tile goes through one — so it is the single
    /// place where a theme's motion either reaches the user or does not.
    #[must_use]
    pub fn themed(ctx: &BuildContext, builder: impl Fn(f32) -> WidgetNode + 'static) -> Self {
        let fade = crate::ThemeData::of(ctx).motion.duration_short;
        Self::new(builder).fade(fade)
    }

    /// Override how long the highlight takes to fade.
    #[must_use]
    pub const fn fade(mut self, fade: Duration) -> Self {
        self.fade = Some(fade);
        self
    }

    /// A pressable region whose subtree is built from how pressed it is.
    #[must_use]
    pub fn new(builder: impl Fn(f32) -> WidgetNode + 'static) -> Self {
        Self::sensed(move |sense: Sense| builder(sense.press))
    }

    /// A pressable whose subtree is built from the press *and* the hover.
    ///
    /// [`new`](Self::new) is this with the hover dropped, which is the right
    /// default for a control that looks the same to a finger and a mouse. Reach
    /// for this one when the two should differ — a row that tints under the
    /// pointer, a tab that lifts before it is clicked.
    #[must_use]
    pub fn sensed(builder: impl Fn(Sense) -> WidgetNode + 'static) -> Self {
        Self {
            builder: Rc::new(builder),
            fade: None,
            on_tap: None,
            key: None,
        }
    }

    /// What to do when the press becomes a tap.
    ///
    /// Optional: a region that only wants the highlight — a row that reacts to a
    /// long press, say — still enters the arena and still tracks the press.
    #[must_use]
    pub fn on_tap(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_tap = Some(Rc::new(handler));
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Pressable {
    fn debug_name(&self) -> &'static str {
        "Pressable"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        let mut state = PressState::default();
        state.apply_fade(self.fade);
        Some(Box::new(state))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // Un-pressed with no state, which is the honest answer outside an
        // element tree — `debug_tree` builds a description, and a description
        // has no finger on it.
        let sense = ctx
            .state(|state: &PressState| Sense {
                press: state.current,
                hover: state.hover_current,
            })
            .unwrap_or(Sense::IDLE);

        let hold = setter(ctx, true);
        let release = setter(ctx, false);
        let hovered = hover_setter(ctx);
        // The tap has to clear the press too. It is a *separate* gesture from
        // the cancel — one of the two follows every press, never both — so a
        // control whose tap did not release it would stay lit until the next
        // rebuild happened to come along for some other reason.
        let on_tap = self.on_tap.clone();
        let tapped = setter(ctx, false);

        GestureDetector::new()
            // Every control in the library goes through here, so this is the one
            // place that has to say they are keyboard-reachable — and only when
            // there is something for the keyboard to do. The detector's `on_tap`
            // below is set either way, because a tap always has to clear the
            // highlight, so this cannot be inferred from that.
            .focusable(self.on_tap.is_some())
            // Set unconditionally, like `on_tap` below: a control that only
            // tracked the pointer when its builder happened to read the hover
            // would report a stale fraction the first time one did.
            .on_hover(hovered)
            .on_tap_down(move |_| hold())
            .on_tap_cancel(release)
            .on_tap(move |_| {
                // Un-highlight first: the handler may replace this whole
                // subtree, and a state written afterwards would be written to
                // an element that is on its way out.
                tapped();
                if let Some(handler) = &on_tap {
                    handler();
                }
            })
            .child((self.builder)(sense))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("tappable", self.on_tap.is_some().to_string())]
    }
}

impl fmt::Debug for Pressable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pressable")
            .field("tappable", &self.on_tap.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Pressable);

/// A callback that writes `pressed` into the building element's own state.
///
/// Captures a handle rather than reaching for one when it runs, because by then
/// there is no build context: a gesture arrives during input dispatch, and the
/// only thing that ever knew where this element's state lived was the build.
fn setter(ctx: &BuildContext, pressed: bool) -> impl Fn() + 'static {
    let handle = ctx.state_handle();
    move || {
        let Some(handle) = &handle else { return };
        // Nothing else holds this borrow: a build takes it and gives it straight
        // back, and handlers run outside builds.
        let mut state = handle.borrow_mut();
        if let Some(state) = state.as_any_mut().downcast_mut::<PressState>() {
            state.set(pressed);
        }
    }
}

/// A callback that writes whether the pointer is over this element into its own
/// state. The mirror of [`setter`], for the input device that has no gesture.
fn hover_setter(ctx: &BuildContext) -> impl Fn(bool) + 'static {
    let handle = ctx.state_handle();
    move |over| {
        let Some(handle) = &handle else { return };
        let mut state = handle.borrow_mut();
        if let Some(state) = state.as_any_mut().downcast_mut::<PressState>() {
            state.set_hovered(over);
        }
    }
}

/// Whether a finger is on a [`Pressable`], and how far the fade has got.
#[derive(Debug)]
pub struct PressState {
    /// What a gesture last said. The *target*, not what is on screen.
    pressed: bool,
    /// Where the fade started, so an interrupted one continues from the screen.
    from: f32,
    /// Where it is now — what `build` reads.
    current: f32,
    controller: AnimationController,
    /// A gesture changed the target and no tick has picked it up yet.
    ///
    /// Deferred rather than started in the handler, for the reason every other
    /// state here defers: starting needs `now`, and a gesture handler has no
    /// honest source of it — the frame does.
    pending: bool,
    /// The same four fields again for the pointer.
    ///
    /// A second controller rather than a second value on the first, because the
    /// two overlap constantly — a pointer presses a control it is already over,
    /// and releases without leaving it — and one controller cannot run two fades
    /// with different targets at once.
    hovered: bool,
    hover_from: f32,
    hover_current: f32,
    hover_controller: AnimationController,
    hover_pending: bool,
}

impl Default for PressState {
    fn default() -> Self {
        Self {
            pressed: false,
            from: 0.0,
            current: 0.0,
            // Out rather than in-out: a press should appear promptly and let go
            // gently, and the same curve read backwards does the opposite.
            controller: AnimationController::new(crate::CONTROL_DURATION).curve(Curve::EASE_OUT),
            pending: false,
            hovered: false,
            hover_from: 0.0,
            hover_current: 0.0,
            hover_controller: AnimationController::new(crate::CONTROL_DURATION)
                .curve(Curve::EASE_OUT),
            hover_pending: false,
        }
    }
}

impl PressState {
    /// Whether a finger is on it. The target, which the fade may not have
    /// reached yet.
    #[must_use]
    pub const fn pressed(&self) -> bool {
        self.pressed
    }

    /// How pressed it looks right now, `0.0 ..= 1.0`.
    #[must_use]
    pub const fn press(&self) -> f32 {
        self.current
    }

    fn set(&mut self, pressed: bool) {
        if self.pressed == pressed {
            return;
        }
        self.pressed = pressed;
        // Only if there is somewhere to travel to. A tap quicker than a frame
        // reports its press and its release in one batch: the target ends where
        // it began, nothing on screen ever moved, and booking a frame for it
        // would be an animation per tap of a fade nobody could have seen.
        self.pending = (self.current - self.target()).abs() > f32::EPSILON;
    }

    /// Whether the pointer is over it. Always false where there is no pointer.
    #[must_use]
    pub const fn hovered(&self) -> bool {
        self.hovered
    }

    /// How hovered it looks right now, `0.0 ..= 1.0`.
    #[must_use]
    pub const fn hover(&self) -> f32 {
        self.hover_current
    }

    fn set_hovered(&mut self, hovered: bool) {
        if self.hovered == hovered {
            return;
        }
        self.hovered = hovered;
        self.hover_pending = (self.hover_current - self.hover_target()).abs() > f32::EPSILON;
    }

    const fn target(&self) -> f32 {
        if self.pressed {
            1.0
        } else {
            0.0
        }
    }

    const fn hover_target(&self) -> f32 {
        if self.hovered {
            1.0
        } else {
            0.0
        }
    }
}

impl PressState {
    /// Apply a widget's fade duration. Separate from
    /// [`ElementState::widget_updated`] so the mount path can call it too.
    fn apply_fade(&mut self, fade: Option<Duration>) {
        let fade = fade.unwrap_or(crate::CONTROL_DURATION);
        self.controller.set_duration(fade);
        self.hover_controller.set_duration(fade);
    }

    /// One fade, advanced by one tick. Shared by the press and the hover, which
    /// differ only in which four fields they own.
    fn advance(
        now: Duration,
        pending: &mut bool,
        from: &mut f32,
        current: &mut f32,
        controller: &mut AnimationController,
        target: f32,
    ) -> bool {
        if std::mem::take(pending) {
            *from = *current;
            controller.set_value(0.0);
            controller.forward(now);
        }
        if !controller.tick(now) {
            return false;
        }
        *current = *from + (target - *from) * controller.value();
        true
    }
}

impl ElementState for PressState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    /// Pick up a new fade duration when the widget is reconfigured, so that
    /// swapping the application's theme retimes presses that already exist
    /// rather than only the ones built afterwards.
    fn widget_updated(&mut self, widget: &WidgetNode) {
        if let Some(pressable) = widget.downcast_ref::<Pressable>() {
            self.apply_fade(pressable.fade);
        }
    }

    fn tick(&mut self, now: Duration) -> bool {
        // Both fades every tick, and `|` rather than `||`: short-circuiting
        // would leave the hover fade un-ticked whenever a press was running,
        // which is exactly when both are moving at once.
        let (press_target, hover_target) = (self.target(), self.hover_target());
        let pressed = Self::advance(
            now,
            &mut self.pending,
            &mut self.from,
            &mut self.current,
            &mut self.controller,
            press_target,
        );
        let hovered = Self::advance(
            now,
            &mut self.hover_pending,
            &mut self.hover_from,
            &mut self.hover_current,
            &mut self.hover_controller,
            hover_target,
        );
        pressed | hovered
    }

    fn is_animating(&self) -> bool {
        // Both halves: `pending` books the frame the fade will start on, and the
        // controller books the ones it runs over. Without the first, a press
        // arriving between frames would wait for something else to ask for one.
        self.pending
            || self.controller.is_animating()
            || self.hover_pending
            || self.hover_controller.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Color;

    use super::*;
    use crate::{debug_tree, ColoredBox, CONTROL_DURATION};

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn swatch() -> Pressable {
        Pressable::new(|press| {
            ColoredBox::new(if press > 0.5 {
                Color::BLUE
            } else {
                Color::WHITE
            })
            .into()
        })
    }

    #[test]
    fn a_pressable_with_no_state_builds_un_pressed() {
        // What `debug_tree` and any other tree-less build sees. The alternative
        // is a panic in exactly the tool used to find out why something panicked.
        let dump = debug_tree(swatch());
        assert!(dump.contains("Pressable"), "{dump}");
        assert!(
            dump.contains(&Color::WHITE.to_string()),
            "un-pressed is the honest answer with no element behind it: {dump}"
        );
    }

    #[test]
    fn a_pressable_registers_a_tap_recogniser_even_with_no_tap_handler() {
        // The highlight is the whole gesture: only the arena knows whether a
        // press is still a candidate, so a region that wants to look pressed has
        // to contest for the pointer like anything else.
        let dump = debug_tree(swatch());
        assert!(dump.contains("GestureDetector"), "{dump}");
        assert!(dump.contains("tap_down"), "{dump}");
    }

    #[test]
    fn a_pressable_with_nothing_to_activate_is_not_a_tab_stop() {
        assert!(!debug_tree(swatch()).contains("focusable"));
        assert!(debug_tree(swatch().on_tap(|| {})).contains("focusable"));
    }

    #[test]
    fn the_press_fades_in_rather_than_appearing() {
        let mut state = PressState::default();
        assert!(!state.is_animating(), "nothing has touched it");

        state.set(true);
        assert!(state.is_animating(), "a press has to book a frame");
        assert!(state.press() < 0.01, "and has not moved yet");

        state.tick(ms(0));
        state.tick(CONTROL_DURATION / 2);
        let half = state.press();
        assert!(
            (0.01..0.999).contains(&half),
            "partway through the fade is partway lit: {half}"
        );

        state.tick(CONTROL_DURATION);
        assert!((state.press() - 1.0).abs() < 0.01);
        assert!(!state.is_animating(), "and then it stops");
    }

    #[test]
    fn a_release_fades_back_from_wherever_the_press_had_reached() {
        let mut state = PressState::default();
        state.set(true);
        state.tick(ms(0));
        state.tick(CONTROL_DURATION / 3);
        let reached = state.press();

        state.set(false);
        state.tick(CONTROL_DURATION / 3);
        assert!(
            state.press() <= reached + 0.01,
            "a press let go early must not brighten first: {} vs {reached}",
            state.press()
        );
    }

    #[test]
    fn a_press_and_release_between_two_frames_leaves_nothing_moving() {
        // A tap quicker than a frame. Both gestures land in one batch, the
        // target ends where it began, and there is nothing on screen to animate.
        let mut state = PressState::default();
        state.set(true);
        state.set(false);

        assert!(!state.pressed());
        assert!(state.press() < f32::EPSILON);
        assert!(
            !state.is_animating(),
            "and no frame is booked for a fade with nowhere to go"
        );
    }

    #[test]
    fn repeating_a_press_starts_nothing_new() {
        let mut state = PressState::default();
        state.set(true);
        state.tick(ms(0));
        state.tick(CONTROL_DURATION);
        assert!(!state.is_animating());

        state.set(true);
        assert!(
            !state.is_animating(),
            "a rebuild that changed nothing must not restart the fade"
        );
    }
}
