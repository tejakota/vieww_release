use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, EdgeInsets, Key, Offset, Size};

use crate::controls::touch_target;
use crate::{
    widget_node_from, Animated, BuildContext, ColorScheme, DecoratedBox, Flex, Handler,
    MainAxisAlignment, Padding, Pressable, SemanticRole, Semantics, SizedBox, ThemeData,
    Transformed, Widget, WidgetKind, WidgetNode,
};

/// The track, and the thumb that slides along it — per platform.
///
/// # Why these are numbers and not theme metrics
///
/// The three are in proportion to each other, and a switch whose thumb no
/// longer fits its track is not a themed switch, it is a broken one. The
/// *touch target* around them does come from the theme, which is the part that
/// has to adapt to a finger.
///
/// # Why there are two sets
///
/// **This is the control people point at when they say a preview does not look
/// native.** An iOS switch and an Android one are not the same object with
/// different colours: iOS is a 51×31 pill with a 27-point white knob that keeps
/// its colour in both states and casts a shadow, and the track is a solid grey
/// when off. Android is a 52×32 track with a *small* knob when off, an
/// outline around it, and a larger knob when on. Anyone who uses both phones
/// can tell them apart at a glance, and until now the studio's iOS preview drew
/// the Android one.
#[derive(Debug, Clone, Copy)]
struct Shape {
    track_width: f32,
    track_height: f32,
    /// The knob's diameter when the switch is on.
    thumb_on: f32,
    /// And when it is off. Equal on iOS; Android grows the knob as it travels.
    thumb_off: f32,
    /// Whether the off track is outlined rather than solid.
    outlined_when_off: bool,
}

impl Shape {
    const fn apple() -> Self {
        Self {
            track_width: 51.0,
            track_height: 31.0,
            thumb_on: 27.0,
            thumb_off: 27.0,
            // iOS has never outlined its off track. It is a solid grey fill,
            // and the shape cue is the knob's position rather than a border.
            outlined_when_off: false,
        }
    }

    const fn android() -> Self {
        Self {
            track_width: 52.0,
            track_height: 32.0,
            thumb_on: 24.0,
            thumb_off: 16.0,
            outlined_when_off: true,
        }
    }

    fn of(platform: vieww_foundation::TargetPlatform) -> Self {
        if platform.is_apple() {
            Self::apple()
        } else {
            Self::android()
        }
    }

    /// The knob's diameter `at` along the travel, `0.0` off and `1.0` on.
    fn thumb(self, at: f32) -> f32 {
        self.thumb_off + (self.thumb_on - self.thumb_off) * at.clamp(0.0, 1.0)
    }

    /// The gap between the largest thumb and the track, on all four sides.
    const fn inset(self) -> f32 {
        (self.track_height - self.thumb_on) / 2.0
    }
}

/// An on/off control.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Switch;
/// use std::rc::Rc;
///
/// # let wifi = true;
/// # let set: Rc<dyn Fn(bool)> = Rc::new(|_| {});
/// let switch = Switch::new(wifi)
///     .label("Wi-Fi")
///     .on_changed(Rc::new(move |next| set(next)));
/// ```
///
/// Controlled, like every control here: it draws the value it is given and
/// reports the one it would like to become. A `Switch` with no
/// [`on_changed`](Self::on_changed) is disabled — dimmed, and with no gesture
/// recogniser at all.
///
/// # The thumb slides
///
/// Between the ends over [`CONTROL_DURATION`](crate::CONTROL_DURATION), driven
/// by an [`Animated`](crate::Animated) whose state lives on the element — which
/// is what makes it animate *from where it was* rather than from wherever the
/// last description said. Flicking a switch twice quickly reverses from the
/// middle instead of jumping to an end first.
///
/// The travel is a paint-time [`Transformed`](crate::Transformed), so a thumb
/// halfway across costs no layout. That is the whole reason the track can be a
/// fixed size and the thumb still move smoothly through it.
#[derive(Clone)]
pub struct Switch {
    value: bool,
    label: Option<String>,
    on_changed: Option<Handler<bool>>,
    key: Option<Key>,
}

impl Switch {
    #[must_use]
    pub const fn new(value: bool) -> Self {
        Self {
            value,
            label: None,
            on_changed: None,
            key: None,
        }
    }

    /// What a screen reader calls this switch — "Wi-Fi", "Aeroplane mode".
    ///
    /// It is not drawn. A switch's visible label belongs to the row around it,
    /// which is free to lay it out however the screen needs; what a screen
    /// reader announces must travel with the control itself, or it is announced
    /// as an anonymous switch.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called with the value the switch would like to become. Giving a handler
    /// is what enables it.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<bool>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn value(&self) -> bool {
        self.value
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_changed.is_some()
    }

    /// The track's decoration and the thumb's colour.
    ///
    /// Off is an outline rather than a filled grey track, so that on and off
    /// differ in *shape* as well as in colour — which is what makes the state
    /// readable to someone who cannot tell the two colours apart.
    fn appearance(&self, theme: &ThemeData, shape: Shape) -> (BoxDecoration, Color) {
        let colors = theme.colors;
        let dim = |color: Color| {
            if self.is_enabled() {
                color
            } else {
                ColorScheme::dimmed(color)
            }
        };

        // **The knob is white on both ends on iOS**, which is the other half of
        // what makes the shape recognisable: the state is read from where the
        // knob is and what colour the *track* is, never from the knob.
        if !shape.outlined_when_off {
            // **The off track is `outline`, not `surface_variant`.**
            //
            // `surface_variant` is the *grouped background* grey — Apple's
            // `systemGroupedBackground`, `#F2F2F7` — and the thumb is
            // `surface`, which in the same scheme is pure white. An off switch
            // was therefore `#F2F2F7` behind `#FFFFFF`, sitting on a `#FFFFFF`
            // page: a contrast ratio of about 1.05:1, which is to say the
            // control was not visible at all. It was found by looking at a
            // screenshot of the studio's own "Switches and sliders" lesson and
            // noticing there were only two controls where the code said three.
            //
            // `outline` is the role that means "a neutral grey that reads
            // against the surface" — `#8E8E93` light, `#636366` dark, since the
            // Apple schemes moved to Apple's increased-contrast greys — which
            // is both what the switch needs and roughly where iOS puts its own
            // off track. Nothing else in the scheme has that job.
            let track = if self.value {
                colors.primary
            } else {
                colors.outline
            };
            return (
                BoxDecoration::filled(dim(track)).stadium(),
                dim(colors.surface),
            );
        }

        if self.value {
            (
                BoxDecoration::filled(dim(colors.primary)).stadium(),
                dim(colors.on_primary),
            )
        } else {
            (
                BoxDecoration::filled(dim(colors.surface_variant))
                    .stadium()
                    .border(Border::thin(dim(colors.outline))),
                dim(colors.outline),
            )
        }
    }

    /// The track and thumb, at the size a finger can hit.
    ///
    /// The touch target is *outside* the track, unlike a button's: a track
    /// stretched to 48 pixels is not a switch.
    ///
    /// `at` is where along the track the thumb currently is, `0.0` at the off
    /// end and `1.0` at the on end — which is not the same thing as `value`
    /// while it is travelling between them.
    fn body(&self, theme: &ThemeData, press: f32, at: f32) -> WidgetNode {
        let shape = Shape::of(theme.platform);
        let (track, thumb_color) = self.appearance(theme, shape);
        // The thumb keeps its colour: it is what the wash is drawn *with*, and a
        // thumb that dimmed along with its track would read as disabled.
        let track = crate::controls::pressed_fill(track, thumb_color, press);

        let diameter = shape.thumb(at);
        // **The knob casts a shadow**, which is the other half of how a white
        // thumb stays legible — on a blue track when the switch is on, and on a
        // grey one when it is off. iOS's own switch does this and relies on it;
        // without it the knob's only edge is the track's colour, and there is
        // no colour pair that reads well against white *and* against the
        // accent. A shadow reads against both.
        //
        // Small numbers on purpose: 1 point down and a 3-point blur is a
        // control that sits on its track, not a card floating over a page.
        let thumb = DecoratedBox::new(BoxDecoration::filled(thumb_color).stadium().shadow(
            vieww_foundation::Shadow::new(Color::rgba(0, 0, 0, 0x38), Offset::new(0.0, 1.0), 3.0),
        ))
        .child(SizedBox::square(diameter));

        // Painted at the off end and moved, rather than laid out somewhere: a
        // transform costs no relayout, so a thumb in flight is a repaint of one
        // small box per frame.
        //
        // The travel is measured against the *current* diameter, because a
        // The Android knob grows as it crosses and a fixed travel would leave it
        // hanging over the far edge at the end.
        let travel = shape.track_width - 2.0 * shape.inset() - diameter;
        // Centred vertically whatever size it is, so a small knob sits on the
        // track's midline rather than on its top edge.
        let lift = (shape.thumb_on - diameter) / 2.0;
        let slot = Flex::row()
            .main_axis_alignment(MainAxisAlignment::Start)
            .push(Transformed::translate(Offset::new(travel * at, lift)).child(thumb));

        let body = DecoratedBox::new(track).child(
            SizedBox::from_size(Size::new(shape.track_width, shape.track_height))
                .child(Padding::new(EdgeInsets::all(shape.inset())).child(slot)),
        );

        touch_target(theme.metrics.touch_target, body).into()
    }
}

impl Widget for Switch {
    fn debug_name(&self) -> &'static str {
        "Switch"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let target = if self.value { 1.0 } else { 0.0 };

        let switch = self.clone();
        let theme = *theme;

        // `Animated` *outside* `Pressable`, so the two states are independent
        // and neither restarts the other: a finger landing on a thumb that is
        // still travelling neither stops it nor is swallowed by it. Inside, the
        // press element would be replaced every frame of the travel and the
        // highlight would start over on each one.
        let interactive: WidgetNode = match &self.on_changed {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let next = !self.value;
                // Resolved here rather than inside the builder: the closure
                // outlives `ctx`, and a token read is a plain value once taken.
                let fade = crate::ThemeData::of(ctx).motion.duration_short;
                Animated::themed(ctx, target)
                    .build(move |at| {
                        let switch = switch.clone();
                        let handler = Rc::clone(&handler);
                        Pressable::new(move |press| switch.body(&theme, press, at))
                            .fade(fade)
                            .on_tap(move || handler(next))
                            .into()
                    })
                    .into()
            }
            None => Animated::themed(ctx, target)
                .build(move |at| switch.body(&theme, 0.0, at))
                .into(),
        };

        let mut semantics = Semantics::new()
            .role(SemanticRole::Switch)
            .toggled(self.value)
            .enabled(self.is_enabled());
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        semantics.child(interactive).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("value", self.value.to_string())];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        if !self.is_enabled() {
            props.push(("disabled", "true".to_owned()));
        }
        props
    }
}

impl fmt::Debug for Switch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Switch")
            .field("value", &self.value)
            .field("label", &self.label)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Switch);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{inflate, DebugNode, Theme};

    use super::*;
    use vieww_foundation::TargetPlatform;

    fn built(switch: Switch) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(switch))
    }

    #[test]
    fn a_tap_reports_the_opposite_of_the_current_value() {
        let reported: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = Rc::clone(&reported);
        let switch = Switch::new(true).on_changed(Rc::new(move |next| sink.set(Some(next))));

        let handler = switch.on_changed.clone().expect("enabled");
        handler(!switch.value());
        assert_eq!(
            reported.get(),
            Some(false),
            "a tap on an on switch turns it off"
        );
    }

    /// How far the thumb has been pushed along the track, in logical pixels.
    fn translation(body: WidgetNode) -> f32 {
        let dump = crate::inflate(body);
        let node = dump.find("Transformed").expect("the thumb is transformed");
        node.property("dx")
            .expect("a translation")
            .parse()
            .expect("a number")
    }

    #[test]
    fn the_thumb_travels_the_length_of_the_track_and_no_further() {
        // The thumb is always laid out at the off end and *moved*, so where it
        // appears is entirely the transform. A travel that did not account for
        // the inset on both sides would push the thumb out through the end of
        // the track — visible only at one value, which is how it survives.
        //
        // Asked of both platforms, because they have different tracks and
        // different knobs and the arithmetic has to hold for each.
        for platform in [TargetPlatform::IOS, TargetPlatform::Android] {
            let theme = ThemeData::adaptive(platform, false);
            let shape = Shape::of(platform);

            let off = translation(Switch::new(false).body(&theme, 0.0, 0.0));
            let on = translation(Switch::new(true).body(&theme, 0.0, 1.0));
            let travel = shape.track_width - 2.0 * shape.inset() - shape.thumb_on;

            assert_eq!(off, 0.0, "{platform:?}: off is where the layout put it");
            assert!((on - travel).abs() < 0.01, "{platform:?}: {on} vs {travel}");
            assert!(
                shape.inset() + travel + shape.thumb_on <= shape.track_width,
                "{platform:?}: the thumb has to still be inside the track"
            );
        }
    }

    #[test]
    fn a_thumb_in_flight_is_somewhere_between_the_two() {
        let theme = ThemeData::adaptive(TargetPlatform::IOS, false);
        let shape = Shape::of(TargetPlatform::IOS);
        let travel = shape.track_width - 2.0 * shape.inset() - shape.thumb_on;
        let halfway = translation(Switch::new(true).body(&theme, 0.0, 0.5));

        assert!(
            (halfway - travel / 2.0).abs() < 0.01,
            "the thumb follows the animation rather than the value: {halfway}"
        );
    }

    /// The reason [`Shape`] exists: an iOS switch and an Android one are
    /// different objects, and a preview that draws the same one for both is a
    /// preview a developer cannot use to check their iOS screen.
    #[test]
    fn ios_and_android_switches_are_not_the_same_shape() {
        let apple = Shape::of(TargetPlatform::IOS);
        let android = Shape::of(TargetPlatform::Android);

        assert_ne!(apple.track_height, android.track_height);
        assert_ne!(apple.thumb_on, android.thumb_on);
        assert_eq!(
            apple.thumb_off, apple.thumb_on,
            "iOS keeps one knob size across the travel"
        );
        assert!(
            android.thumb_off < android.thumb_on,
            "Android grows its knob as it crosses"
        );
        assert!(!apple.outlined_when_off && android.outlined_when_off);
    }

    /// And the knob's colour, which is the other half of the tell.
    #[test]
    fn the_ios_knob_keeps_its_colour_in_both_states() {
        let theme = ThemeData::adaptive(TargetPlatform::IOS, false);
        let shape = Shape::of(TargetPlatform::IOS);
        let (_, on) = Switch::new(true).appearance(&theme, shape);
        let (_, off) = Switch::new(false).appearance(&theme, shape);
        assert_eq!(on, off, "the state is read from the track, not the knob");
    }

    #[test]
    fn the_travel_is_a_transform_so_it_costs_no_layout() {
        // The whole reason a fixed-size track can hold a smoothly moving thumb.
        let tree = built(Switch::new(true));
        assert!(
            tree.find("Transformed").is_some(),
            "a thumb laid out at a fractional offset would relayout the track \
             on every frame of the slide"
        );
    }

    /// Off and on differ in more than colour — on **both** platforms, in the
    /// way each platform differs.
    ///
    /// # Why this names its platforms
    ///
    /// It used to ask `ThemeData::light()`, which now follows the host: the
    /// assertion below is the Android answer, and on a Mac the same line would
    /// have asked an iOS switch for a border it has never had. A test that
    /// checks a *shape* is checking a platform's shape, and saying which is the
    /// whole fix — see `ThemeData::with_platform`.
    ///
    /// The claim itself holds on both and is worth keeping on both, because it
    /// is an accessibility claim rather than a style one: somebody who cannot
    /// tell the two colours apart still has to be able to read the state.
    /// Android says it with an outline; iOS says it with where the knob is,
    /// which `the_thumb_travels_the_length_of_the_track_and_no_further` pins.
    #[test]
    fn off_differs_from_on_in_shape_as_well_as_colour() {
        let android = ThemeData::light().with_platform(TargetPlatform::Android);
        let (on, _) = Switch::new(true).appearance(&android, Shape::android());
        let (off, _) = Switch::new(false).appearance(&android, Shape::android());

        assert!(on.visible_border().is_none());
        assert!(
            off.visible_border().is_some(),
            "an outline is what makes off readable without colour"
        );

        // Apple's switch has no border in either state, and must not grow one:
        // its cue is the knob's position and the track's fill, and an outline
        // would make it an Android switch wearing Apple's colours.
        let apple = ThemeData::light().with_platform(TargetPlatform::IOS);
        for value in [true, false] {
            let (track, _) = Switch::new(value).appearance(&apple, Shape::apple());
            assert!(
                track.visible_border().is_none(),
                "an iOS track is a solid fill, on and off"
            );
        }
        let (on, _) = Switch::new(true).appearance(&apple, Shape::apple());
        let (off, _) = Switch::new(false).appearance(&apple, Shape::apple());
        assert_ne!(on.color, off.color, "so the track's fill is the colour cue");
    }

    #[test]
    fn a_disabled_switch_has_no_recogniser_and_is_dimmed() {
        let tree = built(Switch::new(true));
        assert!(tree.find("GestureDetector").is_none());

        let (track, thumb) = Switch::new(true).appearance(
            &ThemeData::light().with_platform(TargetPlatform::Android),
            Shape::android(),
        );
        assert_eq!(track.color.a, crate::DISABLED_ALPHA);
        assert_eq!(thumb.a, crate::DISABLED_ALPHA);
    }

    #[test]
    fn a_screen_reader_is_told_the_state_rather_than_a_word_for_it() {
        let tree = built(Switch::new(true).label("Wi-Fi"));
        let semantics = tree.find("Semantics").expect("annotated");

        assert_eq!(semantics.property("role"), Some("Switch"));
        assert_eq!(semantics.property("label"), Some("Wi-Fi"));
        assert_eq!(semantics.property("toggled"), Some("true"));
        assert_eq!(
            semantics.property("value"),
            None,
            "the state is a flag, not the literal word \"on\""
        );
    }
}
