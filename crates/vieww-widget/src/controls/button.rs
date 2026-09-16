use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, EdgeInsets, Key, TextStyle};

use crate::controls::touch_target;
use crate::{
    widget_node_from, BuildContext, ColorScheme, DecoratedBox, Padding, Pressable, SemanticRole,
    Semantics, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How much a button asks to be noticed.
///
/// Three, not five: the distinction that matters is how loud a button is
/// relative to the ones beside it, and three levels is enough to say "this is
/// the action", "this is an action" and "this is a way out".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ButtonStyle {
    /// Filled with the accent colour. The one action a screen is about.
    #[default]
    Filled,
    /// An outline, no fill. An action that is not the main one.
    Outlined,
    /// A label and nothing else. Cancel, dismiss, "learn more".
    Text,
}

/// A labelled thing that does something when tapped.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Button, ButtonStyle};
///
/// let save = Button::new("Save").on_pressed(|| println!("saved"));
/// let cancel = Button::new("Cancel").style(ButtonStyle::Text);  // no handler: disabled
/// ```
///
/// A button with no [`on_pressed`](Self::on_pressed) is **disabled**: it is
/// drawn dimmed and registers no gesture recogniser at all, so it cannot take a
/// touch away from a scrollable it sits inside.
///
/// # Size
///
/// Never smaller than the theme's
/// [`touch_target`](crate::Metrics::touch_target) on either axis, and otherwise
/// as wide as its label plus padding. To make one fill its parent, give it tight
/// constraints from outside — a [`SizedBox`](crate::SizedBox) or a stretched
/// [`Flex`](crate::Flex) — rather than looking for a `full_width` flag.
#[derive(Clone)]
pub struct Button {
    label: String,
    style: ButtonStyle,
    on_pressed: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Button {
    /// A filled button reading `label`, disabled until it is given a handler.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: ButtonStyle::Filled,
            on_pressed: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// What to do when it is tapped. Giving a handler is what enables it.
    #[must_use]
    pub fn on_pressed(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_pressed = Some(Rc::new(handler));
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The text on the button, which is also what a screen reader announces.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// `false` when there is no handler to run.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_pressed.is_some()
    }

    /// The decoration and label colour with a finger on it, `press` far into the
    /// fade.
    ///
    /// A wash of the label's own colour over the fill — so a filled button
    /// darkens, an outlined one picks up a tint of its accent, and a text button
    /// grows a faint background where it had none. See
    /// [`pressed_fill`](crate::controls::pressed_fill).
    fn pressed_appearance(&self, theme: &ThemeData, press: f32) -> (BoxDecoration, Color) {
        let (decoration, label) = self.appearance(theme);
        (
            crate::controls::pressed_fill(decoration, label, press),
            label,
        )
    }

    /// The decoration and label colour for a style, in a theme, at an enablement.
    ///
    /// Disabling is not "the same colours, dimmed". A filled button dimmed that
    /// way keeps shouting — a washed-out accent is still an accent — so a
    /// disabled filled button drops to the surface variant instead, and only its
    /// *label* is dimmed. The outlined and text styles have nothing loud to drop,
    /// so for them dimming is the whole change.
    fn appearance(&self, theme: &ThemeData) -> (BoxDecoration, Color) {
        let colors = theme.colors;
        let enabled = self.is_enabled();

        // **A pill on Android, a rounded rectangle on Apple.** The modern Android
        // the fully-rounded button its default shape and it is the single most
        // recognisable thing about an Android screen; iOS has never used one —
        // its filled buttons are the continuous-corner rectangle the metrics'
        // `corner` approximates. Drawing one for the other is the difference
        // between a preview a developer trusts and a preview they check against
        // a simulator anyway.
        let shape = |decoration: BoxDecoration| {
            if theme.platform.is_apple() {
                decoration.radius(theme.metrics.corner)
            } else {
                decoration.stadium()
            }
        };

        match (self.style, enabled) {
            (ButtonStyle::Filled, true) => (
                shape(BoxDecoration::filled(colors.primary)),
                colors.on_primary,
            ),
            (ButtonStyle::Filled, false) => (
                shape(BoxDecoration::filled(colors.surface_variant)),
                ColorScheme::dimmed(colors.on_surface),
            ),
            (ButtonStyle::Outlined, enabled) => {
                let outline = if enabled {
                    colors.outline
                } else {
                    ColorScheme::dimmed(colors.outline)
                };
                (
                    shape(BoxDecoration::outlined(Border::thin(outline))),
                    if enabled {
                        colors.primary
                    } else {
                        ColorScheme::dimmed(colors.on_surface)
                    },
                )
            }
            (ButtonStyle::Text, enabled) => (
                BoxDecoration::default(),
                if enabled {
                    colors.primary
                } else {
                    ColorScheme::dimmed(colors.on_surface)
                },
            ),
        }
    }

    /// The button as it looks right now: fill, label, and the space around both.
    ///
    /// Taken as a whole rather than as a decoration handed to one builder,
    /// because the press changes the label's *background* and the label sits
    /// inside it — there is no seam between the two to pass a flag across.
    fn body(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        let (decoration, label_color) = self.pressed_appearance(theme, press);
        let gap = theme.metrics.gap;

        let label = Text::new(self.label.clone()).style(TextStyle {
            color: label_color,
            ..theme.text.label
        });

        DecoratedBox::new(decoration)
            .child(touch_target(
                theme.metrics.touch_target,
                Padding::new(EdgeInsets::symmetric(gap * 2.0, gap)).child(label),
            ))
            .into()
    }
}

impl Widget for Button {
    fn debug_name(&self) -> &'static str {
        "Button"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        // The pressable goes *inside* the semantics annotation and outside the
        // decoration, so the whole painted area is tappable rather than just the
        // label — and so a disabled button has no recogniser anywhere, and does
        // not react to a finger it is going to ignore.
        let interactive: WidgetNode = match &self.on_pressed {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let button = self.clone();
                let theme = *theme;
                Pressable::themed(ctx, move |press| button.body(&theme, press))
                    .on_tap(move || handler())
                    .into()
            }
            None => self.body(&theme, 0.0),
        };

        Semantics::new()
            .role(SemanticRole::Button)
            .label(self.label.clone())
            .enabled(self.is_enabled())
            .child(interactive)
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("label", self.label.clone()),
            ("style", format!("{:?}", self.style)),
        ];
        if !self.is_enabled() {
            props.push(("disabled", "true".to_owned()));
        }
        props
    }
}

impl fmt::Debug for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The handler is a closure: whether there *is* one is the informative
        // part, and it is the same thing as being enabled.
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("style", &self.style)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Button);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{debug_tree, inflate, Theme};

    use super::*;

    /// The inflated tree of a button built under `theme`.
    fn built(button: Button, theme: ThemeData) -> crate::DebugNode {
        inflate(Theme::new(theme).child(button))
    }

    #[test]
    fn a_tap_runs_the_handler() {
        let pressed = Rc::new(Cell::new(0));
        let sink = Rc::clone(&pressed);
        let button = Button::new("Save").on_pressed(move || sink.set(sink.get() + 1));

        let handler = button.on_pressed.clone().expect("enabled");
        handler();
        assert_eq!(pressed.get(), 1);
    }

    #[test]
    fn a_button_with_no_handler_registers_no_gesture_recogniser() {
        let tree = built(Button::new("Save"), ThemeData::light());
        assert!(
            tree.find("GestureDetector").is_none(),
            "a disabled button must not take a touch from a scrollable"
        );

        let live = built(Button::new("Save").on_pressed(|| {}), ThemeData::light());
        assert!(live.find("GestureDetector").is_some());
    }

    #[test]
    fn a_button_announces_itself_as_a_button_with_its_label() {
        let dump = debug_tree(Button::new("Send").on_pressed(|| {}));
        assert!(dump.contains("Semantics"), "{dump}");

        let tree = inflate(Button::new("Send").on_pressed(|| {}));
        assert_eq!(
            tree.find("Button").and_then(|b| b.property("label")),
            Some("Send")
        );
    }

    #[test]
    fn a_filled_button_takes_the_accent_from_whichever_theme_is_in_force() {
        let light = built(Button::new("Go").on_pressed(|| {}), ThemeData::light());
        let dark = built(Button::new("Go").on_pressed(|| {}), ThemeData::dark());

        let fill = |tree: &crate::DebugNode| {
            tree.find("DecoratedBox")
                .and_then(|node| node.property("color"))
                .map(ToOwned::to_owned)
        };

        assert_eq!(fill(&light), Some(ColorScheme::light().primary.to_string()));
        assert_eq!(fill(&dark), Some(ColorScheme::dark().primary.to_string()));
    }

    #[test]
    fn a_disabled_filled_button_drops_the_accent_rather_than_dimming_it() {
        let theme = ThemeData::light();
        let (decoration, label) = Button::new("Go").appearance(&theme);

        assert_eq!(decoration.color, theme.colors.surface_variant);
        assert_eq!(
            label.a,
            crate::DISABLED_ALPHA,
            "the label is what says unavailable"
        );
    }

    #[test]
    fn a_disabled_button_says_so_rather_than_only_looking_it() {
        // Being unavailable is otherwise carried by colour alone, which is no
        // information at all to a screen reader — and a button that announces
        // itself as ordinary is one a blind user keeps pressing.
        let disabled = built(Button::new("Save"), ThemeData::light());
        assert_eq!(
            disabled
                .find("Semantics")
                .and_then(|node| node.property("enabled")),
            Some("false")
        );

        let live = built(Button::new("Save").on_pressed(|| {}), ThemeData::light());
        assert_eq!(
            live.find("Semantics")
                .and_then(|node| node.property("enabled")),
            None,
            "and an ordinary button says nothing, because enabled is the default"
        );
    }

    #[test]
    fn a_press_shifts_the_fill_towards_the_label() {
        let theme = ThemeData::light();
        let button = Button::new("Go").on_pressed(|| {});

        let (resting, label) = button.appearance(&theme);
        let (pressed, pressed_label) = button.pressed_appearance(&theme, 1.0);

        assert_ne!(resting.color, pressed.color, "a press has to be visible");
        assert_eq!(
            pressed_label, label,
            "the label does not move — only what is behind it"
        );
        assert_eq!(
            pressed.radius, resting.radius,
            "and the shape is the same shape"
        );
    }

    #[test]
    fn a_pressed_text_button_grows_the_background_it_did_not_have() {
        let theme = ThemeData::light();
        let button = Button::new("Cancel")
            .style(ButtonStyle::Text)
            .on_pressed(|| {});

        assert!(button.appearance(&theme).0.is_invisible());
        let pressed = button.pressed_appearance(&theme, 1.0).0;
        assert!(
            !pressed.is_invisible(),
            "with nothing to tint, the wash is the whole of the feedback"
        );
    }

    #[test]
    fn a_disabled_button_has_nothing_to_press() {
        let tree = built(Button::new("Save"), ThemeData::light());
        assert!(
            tree.find("Pressable").is_none(),
            "a button that will ignore the tap must not react to the finger"
        );
        assert!(
            built(Button::new("Save").on_pressed(|| {}), ThemeData::light())
                .find("Pressable")
                .is_some()
        );
    }

    #[test]
    fn a_text_button_paints_nothing_behind_its_label() {
        let theme = ThemeData::light();
        let (decoration, _) = Button::new("Cancel")
            .style(ButtonStyle::Text)
            .on_pressed(|| {})
            .appearance(&theme);
        assert!(decoration.is_invisible());
    }

    #[test]
    fn a_button_is_never_smaller_than_the_themes_touch_target() {
        let theme = ThemeData::light();
        let tree = built(Button::new("Ok").on_pressed(|| {}), theme);
        let constrained = tree.find("Constrained").expect("a touch target");

        let min = theme.metrics.touch_target;
        assert_eq!(
            constrained.property("constraints"),
            Some(format!("w[{min}..inf] h[{min}..inf]").as_str())
        );
    }
}
