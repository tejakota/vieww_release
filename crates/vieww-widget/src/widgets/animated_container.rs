use std::any::Any;
use std::time::Duration;

use vieww_animation::{AnimationController, Curve, Lerp};
use vieww_foundation::{Alignment, Border, Color, EdgeInsets, Gradient, Key, Offset, Shadow};

use crate::{
    widget_node_from, BuildContext, Container, ElementState, Widget, WidgetKind, WidgetNode,
};

/// How long an animated property takes to move when nothing says otherwise.
const DEFAULT_DURATION: Duration = Duration::from_millis(200);

/// The properties an [`AnimatedContainer`] interpolates.
///
/// `Option` per property, matching [`Container`]: a property that was never set
/// is not animated and not applied, so an `AnimatedContainer` costs exactly what
/// the equivalent `Container` costs plus the ones actually in motion.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AnimatedProps {
    pub color: Option<Color>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub padding: Option<EdgeInsets>,
    pub margin: Option<EdgeInsets>,
    pub alignment: Option<Alignment>,
    /// Corner radius. Interpolated as a number, so a square becomes a stadium
    /// through every shape in between.
    pub radius: Option<f32>,
    /// Outline. Colour and width move together.
    pub border: Option<Border>,
    /// One shadow: colour, offset, blur and spread all interpolate, which is
    /// what makes a card look like it is *rising* rather than switching to a
    /// different card.
    pub shadow: Option<Shadow>,
    /// A gradient. Two gradients interpolate stop by stop when they have the
    /// same geometry *kind* and the same number of stops; when they do not
    /// there is no sensible half-way and the new one is taken — the same rule
    /// `lerp_option` applies to a property that appears.
    pub gradient: Option<Gradient>,
}

/// Interpolate a property that may not be set on both sides.
///
/// A property appearing or disappearing has no "half way" — there is no colour
/// between *blue* and *no background at all* — so it snaps to whatever the new
/// description asks for and the rest of the properties still animate.
fn lerp_option<T: Lerp>(from: Option<T>, to: Option<T>, t: f32) -> Option<T> {
    match (from, to) {
        (Some(from), Some(to)) => Some(from.lerp(to, t)),
        (_, to) => to,
    }
}

/// A border half way between two borders.
fn lerp_border(from: Border, to: Border, t: f32) -> Border {
    Border {
        color: from.color.lerp(to.color, t),
        width: from.width.lerp(to.width, t),
    }
}

/// A shadow half way between two shadows.
///
/// `is_inset` is a kind rather than a quantity — there is nothing between a
/// shadow cast outward and one cast inward — so it snaps at the midpoint and
/// everything else moves.
fn lerp_shadow(from: Shadow, to: Shadow, t: f32) -> Shadow {
    Shadow {
        color: from.color.lerp(to.color, t),
        offset: Offset::new(
            from.offset.dx.lerp(to.offset.dx, t),
            from.offset.dy.lerp(to.offset.dy, t),
        ),
        blur: from.blur.lerp(to.blur, t),
        spread: from.spread.lerp(to.spread, t),
        is_inset: if t < 0.5 { from.is_inset } else { to.is_inset },
    }
}

/// A gradient half way between two gradients, when that means anything.
///
/// Two ramps only interpolate when they are the same *kind* of geometry with
/// the same number of stops. A linear fading into a radial has no half-way
/// state that is either of them, and a made-up one reads as a glitch — so the
/// new gradient is taken whole, exactly as an appearing property is.
fn lerp_gradient(from: Gradient, to: Gradient, t: f32) -> Gradient {
    use vieww_foundation::GradientGeometry;
    let geometry = match (from.geometry, to.geometry) {
        (
            GradientGeometry::Linear { start: a0, end: a1 },
            GradientGeometry::Linear { start: b0, end: b1 },
        ) => GradientGeometry::Linear {
            start: Offset::new(a0.dx.lerp(b0.dx, t), a0.dy.lerp(b0.dy, t)),
            end: Offset::new(a1.dx.lerp(b1.dx, t), a1.dy.lerp(b1.dy, t)),
        },
        (
            GradientGeometry::Radial {
                center: a,
                radius: ar,
            },
            GradientGeometry::Radial {
                center: b,
                radius: br,
            },
        ) => GradientGeometry::Radial {
            center: Offset::new(a.dx.lerp(b.dx, t), a.dy.lerp(b.dy, t)),
            radius: ar.lerp(br, t),
        },
        (
            GradientGeometry::Sweep {
                center: a,
                start_angle: a0,
                end_angle: a1,
            },
            GradientGeometry::Sweep {
                center: b,
                start_angle: b0,
                end_angle: b1,
            },
        ) => GradientGeometry::Sweep {
            center: Offset::new(a.dx.lerp(b.dx, t), a.dy.lerp(b.dy, t)),
            start_angle: a0.lerp(b0, t),
            end_angle: a1.lerp(b1, t),
        },
        _ => return to,
    };
    let (left, right) = (from.stops(), to.stops());
    if left.len() != right.len() {
        return to;
    }
    let stops: Vec<(f32, Color)> = left
        .iter()
        .zip(right)
        .map(|(a, b)| (a.offset.lerp(b.offset, t), a.color.lerp(b.color, t)))
        .collect();
    from.with_geometry(geometry).with_stops(&stops)
}

impl Lerp for AnimatedProps {
    fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            color: lerp_option(self.color, other.color, t),
            width: lerp_option(self.width, other.width, t),
            height: lerp_option(self.height, other.height, t),
            padding: lerp_option(self.padding, other.padding, t),
            margin: lerp_option(self.margin, other.margin, t),
            alignment: lerp_option(self.alignment, other.alignment, t),
            radius: lerp_option(self.radius, other.radius, t),
            border: match (self.border, other.border) {
                (Some(from), Some(to)) => Some(lerp_border(from, to, t)),
                (_, to) => to,
            },
            shadow: match (self.shadow, other.shadow) {
                (Some(from), Some(to)) => Some(lerp_shadow(from, to, t)),
                (_, to) => to,
            },
            gradient: match (self.gradient, other.gradient) {
                (Some(from), Some(to)) => Some(lerp_gradient(from, to, t)),
                (_, to) => to,
            },
        }
    }
}

impl AnimatedProps {
    /// The plain container these properties describe.
    fn container(self, child: Option<WidgetNode>) -> Container {
        let mut container = Container::new();
        if let Some(child) = child {
            container = container.child(child);
        }
        if let Some(color) = self.color {
            container = container.color(color);
        }
        if let Some(width) = self.width {
            container = container.width(width);
        }
        if let Some(height) = self.height {
            container = container.height(height);
        }
        if let Some(padding) = self.padding {
            container = container.padding(padding);
        }
        if let Some(margin) = self.margin {
            container = container.margin(margin);
        }
        if let Some(alignment) = self.alignment {
            container = container.alignment(alignment);
        }
        if let Some(radius) = self.radius {
            container = container.radius(radius);
        }
        if let Some(border) = self.border {
            container = container.border(border);
        }
        if let Some(shadow) = self.shadow {
            container = container.shadow(shadow);
        }
        if let Some(gradient) = self.gradient {
            container = container.gradient(gradient);
        }
        container
    }
}

/// A [`Container`] that animates to each new set of properties it is given.
///
/// This is the animated container, and the point of it is that there is no
/// controller in the calling code. Describe the container you want; when the
/// description changes, the difference is animated rather than applied.
///
/// ```
/// use std::time::Duration;
/// use vieww_widget::prelude::*;
/// use vieww_widget::AnimatedContainer;
///
/// # let selected = true;
/// let swatch = AnimatedContainer::new()
///     .duration(Duration::from_millis(250))
///     .color(if selected { Color::BLUE } else { Color::WHITE })
///     .width(if selected { 120.0 } else { 80.0 })
///     .child(SizedBox::square(40.0));
/// ```
///
/// # How it knows
///
/// The element holds an [`AnimatedContainerState`]. When reconciliation hands
/// the element a new `AnimatedContainer`, the state compares the new properties
/// to the ones it was already heading for, and retargets if they differ —
/// starting from wherever the previous animation had got to, so an interrupted
/// animation continues from the screen rather than jumping back.
///
/// # What it cannot animate
///
/// A property that is set in one description and absent in the next snaps
/// instead of animating; see [`AnimatedProps`]. Animating the child, or the
/// number of children, is not what this is for — that is a cross-fade, and it
/// needs a widget that can hold two subtrees at once.
#[derive(Debug, Clone)]
pub struct AnimatedContainer {
    props: AnimatedProps,
    duration: Duration,
    curve: Curve,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl AnimatedContainer {
    /// A container that takes its timing from the ambient
    /// [`Motion`](crate::Motion) tokens.
    ///
    /// Reads `duration_medium` and `curve_emphasized` — a container changing
    /// shape is a larger event than a switch thumb moving, and it both starts
    /// and stops on screen, which is what the emphasized curve is for.
    ///
    /// Prefer this over [`new`](Self::new) wherever a `BuildContext` is
    /// available: a hard-coded duration is a motion decision the application's
    /// theme cannot reach.
    #[must_use]
    pub fn themed(ctx: &BuildContext) -> Self {
        let motion = crate::ThemeData::of(ctx).motion;
        Self::new()
            .duration(motion.duration_medium)
            .curve(motion.curve_emphasized)
    }

    /// A container with a fixed default duration and curve.
    ///
    /// Prefer [`themed`](Self::themed) anywhere a `BuildContext` is available.
    #[must_use]
    pub fn new() -> Self {
        Self {
            props: AnimatedProps::default(),
            duration: DEFAULT_DURATION,
            curve: Curve::EASE_IN_OUT,
            child: None,
            key: None,
        }
    }

    /// How long a full-range change takes. A smaller change takes proportionally
    /// less; see [`AnimationController::animate_to`].
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// The easing. Defaults to [`Curve::EASE_IN_OUT`], since a container
    /// changing shape both starts and stops on screen.
    #[must_use]
    pub const fn curve(mut self, curve: Curve) -> Self {
        self.curve = curve;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.props.color = Some(color);
        self
    }

    /// Corner radius, interpolated as a number.
    #[must_use]
    pub const fn radius(mut self, radius: f32) -> Self {
        self.props.radius = Some(radius);
        self
    }

    /// An outline whose colour and width both move.
    #[must_use]
    pub const fn border(mut self, border: Border) -> Self {
        self.props.border = Some(border);
        self
    }

    /// One shadow. Animating it is how a surface *rises* under the pointer
    /// rather than swapping to a different surface.
    #[must_use]
    pub const fn shadow(mut self, shadow: Shadow) -> Self {
        self.props.shadow = Some(shadow);
        self
    }

    /// A gradient. Interpolates when the next one has the same geometry kind
    /// and stop count; otherwise it is taken whole.
    #[must_use]
    pub const fn gradient(mut self, gradient: Gradient) -> Self {
        self.props.gradient = Some(gradient);
        self
    }

    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.props.width = Some(width);
        self
    }

    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.props.height = Some(height);
        self
    }

    #[must_use]
    pub const fn size(self, width: f32, height: f32) -> Self {
        self.width(width).height(height)
    }

    #[must_use]
    pub const fn padding(mut self, padding: EdgeInsets) -> Self {
        self.props.padding = Some(padding);
        self
    }

    #[must_use]
    pub const fn margin(mut self, margin: EdgeInsets) -> Self {
        self.props.margin = Some(margin);
        self
    }

    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.props.alignment = Some(alignment);
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Set the reconciliation key.
    ///
    /// Worth setting deliberately here: the element is what holds the animation,
    /// so a key that changes replaces the element and starts the animation over
    /// from the new values rather than moving to them.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The properties this description is asking for.
    #[must_use]
    pub const fn props(&self) -> AnimatedProps {
        self.props
    }
}

impl Default for AnimatedContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for AnimatedContainer {
    fn debug_name(&self) -> &'static str {
        "AnimatedContainer"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(AnimatedContainerState::new(self)))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // Built from where the animation *is*, not from where it is going. With
        // no state — outside an element tree, as `debug_tree` does — the target
        // is the honest answer: there is nothing animating it.
        let props = ctx
            .state(|state: &AnimatedContainerState| state.current)
            .unwrap_or(self.props);
        props.container(self.child.clone()).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("duration", format!("{:?}", self.duration))];
        if let Some(color) = self.props.color {
            props.push(("color", color.to_string()));
        }
        if let Some(width) = self.props.width {
            props.push(("width", width.to_string()));
        }
        if let Some(height) = self.props.height {
            props.push(("height", height.to_string()));
        }
        props
    }
}

widget_node_from!(AnimatedContainer);

/// The animation an [`AnimatedContainer`]'s element carries across rebuilds.
///
/// Public because [`Widget::create_state`] returns it and a test may want to
/// look at it; there is nothing to configure here.
#[derive(Debug)]
pub struct AnimatedContainerState {
    /// Where the current animation started.
    from: AnimatedProps,
    /// Where it is going. Also what a new description is compared against, so
    /// that rebuilding with an unchanged target does not restart anything.
    to: AnimatedProps,
    /// Where it is now — what `build` reads.
    current: AnimatedProps,
    controller: AnimationController,
    /// A new target, noticed during the build phase and acted on by the next
    /// tick.
    ///
    /// Deferred rather than started immediately because starting needs `now`,
    /// and the only honest source of `now` is the frame — reading a clock in
    /// `widget_updated` would start the animation from a timestamp that is not
    /// the one the frame will be shown at.
    pending: Option<AnimatedProps>,
}

impl AnimatedContainerState {
    fn new(widget: &AnimatedContainer) -> Self {
        Self {
            from: widget.props,
            to: widget.props,
            current: widget.props,
            controller: AnimationController::new(widget.duration).curve(widget.curve),
            pending: None,
        }
    }

    /// The properties as they are on screen right now.
    #[must_use]
    pub const fn current(&self) -> AnimatedProps {
        self.current
    }

    /// The properties being animated towards.
    #[must_use]
    pub const fn target(&self) -> AnimatedProps {
        self.to
    }
}

impl ElementState for AnimatedContainerState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn widget_updated(&mut self, widget: &WidgetNode) {
        let Some(widget) = widget.downcast_ref::<AnimatedContainer>() else {
            return;
        };
        self.controller.set_duration(widget.duration);
        self.controller.set_curve(widget.curve);
        if widget.props != self.to {
            self.pending = Some(widget.props);
        }
    }

    fn tick(&mut self, now: Duration) -> bool {
        if let Some(target) = self.pending.take() {
            // From where it actually is, not from where the last animation
            // began: interrupting a half-finished move has to continue from the
            // screen, or the container jumps backwards before setting off again.
            self.from = self.current;
            self.to = target;
            self.controller.set_value(0.0);
            self.controller.forward(now);
        }
        if !self.controller.tick(now) {
            return false;
        }
        self.current = self.from.lerp(self.to, self.controller.value());
        true
    }

    fn is_animating(&self) -> bool {
        self.pending.is_some() || self.controller.is_animating()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// A container animating its colour and width over 200ms, linearly — a curve
    /// would make every assertion here a tolerance rather than a value.
    fn widget(color: Color, width: f32) -> AnimatedContainer {
        AnimatedContainer::new()
            .duration(ms(200))
            .curve(Curve::Linear)
            .color(color)
            .width(width)
    }

    fn state(widget: &AnimatedContainer) -> AnimatedContainerState {
        AnimatedContainerState::new(widget)
    }

    #[test]
    fn a_freshly_mounted_container_shows_its_properties_without_animating() {
        let state = state(&widget(Color::RED, 100.0));

        assert_eq!(state.current().color, Some(Color::RED));
        assert_eq!(state.current().width, Some(100.0));
        assert!(
            !state.is_animating(),
            "the first description is where it starts, not something to animate to"
        );
    }

    #[test]
    fn a_changed_description_animates_rather_than_applying() {
        let mut state = state(&widget(Color::RED, 100.0));
        state.widget_updated(&WidgetNode::from(widget(Color::BLUE, 200.0)));

        assert!(state.is_animating(), "the change has to book a frame");
        assert_eq!(
            state.current().width,
            Some(100.0),
            "and must not have applied yet — nothing has ticked"
        );

        state.tick(ms(0));
        state.tick(ms(100));
        assert_eq!(state.current().width, Some(150.0));
        let color = state.current().color.expect("colour is set");
        assert!(color.r > 0 && color.b > 0, "part way between: {color}");

        state.tick(ms(200));
        assert_eq!(state.current().width, Some(200.0));
        assert_eq!(state.current().color, Some(Color::BLUE));
        assert!(!state.is_animating(), "and then it stops costing frames");
    }

    #[test]
    fn rebuilding_with_the_same_target_does_not_restart_anything() {
        let mut state = state(&widget(Color::RED, 100.0));
        state.widget_updated(&WidgetNode::from(widget(Color::BLUE, 200.0)));
        state.tick(ms(0));
        state.tick(ms(100));

        // The tree rebuilt for some unrelated reason and handed over an equal
        // description. A retarget here would restart the animation from half way
        // every time anything else on the screen changed.
        state.widget_updated(&WidgetNode::from(widget(Color::BLUE, 200.0)));
        state.tick(ms(116));

        assert!(state.current().width.expect("width") > 150.0);
        state.tick(ms(200));
        assert_eq!(state.current().width, Some(200.0));
    }

    #[test]
    fn an_interrupted_animation_carries_on_from_where_it_is() {
        let mut state = state(&widget(Color::RED, 0.0));
        state.widget_updated(&WidgetNode::from(widget(Color::RED, 100.0)));
        state.tick(ms(0));
        state.tick(ms(100));
        assert_eq!(state.current().width, Some(50.0));

        // Half way there, the target changes again.
        state.widget_updated(&WidgetNode::from(widget(Color::RED, 0.0)));
        state.tick(ms(100));

        assert_eq!(
            state.current().width,
            Some(50.0),
            "the new animation starts from the screen, not from where the last \
             one began — otherwise the container jumps before setting off"
        );

        // And it takes a full duration from there, rather than inheriting what
        // was left of the interrupted one.
        state.tick(ms(200));
        assert_eq!(state.current().width, Some(25.0));
        state.tick(ms(300));
        assert_eq!(state.current().width, Some(0.0));
        assert!(!state.is_animating());
    }

    #[test]
    fn a_property_that_appears_snaps_while_the_rest_still_animate() {
        let mut state = state(&AnimatedContainer::new().duration(ms(200)).width(100.0));
        state.widget_updated(&WidgetNode::from(
            AnimatedContainer::new()
                .duration(ms(200))
                .width(200.0)
                .color(Color::GREEN),
        ));
        state.tick(ms(0));
        state.tick(ms(100));

        assert_eq!(
            state.current().color,
            Some(Color::GREEN),
            "there is no colour between 'green' and 'no background at all'"
        );
        assert!(
            state.current().width.expect("width") > 100.0,
            "but width moves"
        );
    }

    #[test]
    fn the_widget_builds_a_plain_container_from_the_current_values() {
        let widget = widget(Color::RED, 100.0);
        let built = widget.build(&BuildContext::root());

        assert_eq!(
            format!("{built:?}"),
            "Container",
            "an animated container owns no layout of its own — it retargets a \
             plain one"
        );
    }

    #[test]
    fn a_container_built_without_an_element_shows_its_target() {
        // `debug_tree` inflates widgets with no element tree under them, and a
        // widget that panicked or rendered nothing there would be unprintable.
        let dump = crate::debug_tree(widget(Color::RED, 100.0).child(crate::Text::new("x")));
        assert!(dump.contains("AnimatedContainer"), "{dump}");
        assert!(dump.contains("Container"), "{dump}");
    }
}

#[cfg(test)]
mod decoration_tests {
    use super::*;
    use vieww_foundation::Gradient;

    /// The whole point of animating a shadow: a card that rises reads as one
    /// card moving, and a card that swaps shadows reads as two cards.
    #[test]
    fn a_shadow_rises_rather_than_switching() {
        let resting = AnimatedProps {
            shadow: Some(Shadow::new(
                Color::rgba(0, 0, 0, 40),
                Offset::new(0.0, 1.0),
                3.0,
            )),
            ..AnimatedProps::default()
        };
        let raised = AnimatedProps {
            shadow: Some(Shadow::new(
                Color::rgba(0, 0, 0, 120),
                Offset::new(0.0, 6.0),
                18.0,
            )),
            ..AnimatedProps::default()
        };
        let half = resting.lerp(raised, 0.5).shadow.expect("still a shadow");
        assert!((half.offset.dy - 3.5).abs() < 0.01, "{:?}", half.offset);
        assert!((half.blur - 10.5).abs() < 0.01, "{}", half.blur);
        assert_eq!(half.color.a, 80);
    }

    #[test]
    fn a_radius_and_a_border_move_together() {
        let square = AnimatedProps {
            radius: Some(0.0),
            border: Some(Border::new(Color::rgb(0, 0, 0), 0.0)),
            ..AnimatedProps::default()
        };
        let round = AnimatedProps {
            radius: Some(20.0),
            border: Some(Border::new(Color::rgb(0, 0, 0), 2.0)),
            ..AnimatedProps::default()
        };
        let half = square.lerp(round, 0.5);
        assert!((half.radius.unwrap() - 10.0).abs() < 0.01);
        assert!((half.border.unwrap().width - 1.0).abs() < 0.01);
    }

    #[test]
    fn two_matching_ramps_interpolate_stop_by_stop() {
        let cool = Gradient::vertical().between(Color::rgb(0, 0, 0), Color::rgb(0, 0, 100));
        let warm = Gradient::vertical().between(Color::rgb(100, 0, 0), Color::rgb(200, 0, 0));
        let from = AnimatedProps {
            gradient: Some(cool),
            ..AnimatedProps::default()
        };
        let to = AnimatedProps {
            gradient: Some(warm),
            ..AnimatedProps::default()
        };
        let half = from.lerp(to, 0.5).gradient.expect("still a gradient");
        assert_eq!(half.stops()[0].color.r, 50);
        assert_eq!(half.stops()[1].color.r, 100);
        assert_eq!(half.stops()[1].color.b, 50);
    }

    /// A linear becoming a radial has no half-way that is either of them.
    #[test]
    fn ramps_of_different_kinds_snap_to_the_new_one() {
        let from = AnimatedProps {
            gradient: Some(Gradient::vertical().between(Color::RED, Color::BLUE)),
            ..AnimatedProps::default()
        };
        let to = AnimatedProps {
            gradient: Some(Gradient::radial_fill().between(Color::RED, Color::BLUE)),
            ..AnimatedProps::default()
        };
        let half = from.lerp(to, 0.5).gradient.expect("a gradient");
        assert!(matches!(
            half.geometry,
            vieww_foundation::GradientGeometry::Radial { .. }
        ));
    }
}
