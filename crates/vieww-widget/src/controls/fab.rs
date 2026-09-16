use std::fmt;
use std::rc::Rc;

use vieww_foundation::{BoxDecoration, Color, IconData, Key};

use crate::{
    widget_node_from, BuildContext, Center, DecoratedBox, Icon, Pressable, SemanticRole, Semantics,
    SizedBox, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The diameter of a standard [`FloatingActionButton`], in logical pixels.
///
/// Fixed rather than derived from [`Metrics::touch_target`](crate::Metrics),
/// on the same reasoning a real FAB has a size of its own: it is a single,
/// prominent, screen-anchored action rather than an ordinary control in a
/// row of them, and 56 is the number both the classic FAB and the size this control
/// is modelled on agree on regardless of platform.
pub const FAB_SIZE: f32 = 56.0;

/// A circular, prominent action, meant to be the one thing a screen is about.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{icons, FloatingActionButton};
///
/// let add = FloatingActionButton::new(icons::add())
///     .label("Add item")
///     .on_pressed(|| {});
/// ```
///
/// # It is not positioned by this widget
///
/// A FAB's screen-corner placement is a property of the *screen*, not the
/// button — this is a circular icon button and nothing more. Anchor it with
/// [`Positioned`](crate::Positioned) inside a [`Stack`](crate::Stack) that
/// covers the screen, the same way any other overlay is placed.
///
/// # `label` is spoken, not painted
///
/// The icon alone has no text a screen reader can read, so
/// [`label`](Self::label) is required for the button to be more than a
/// silent shape — mirroring [`Icon::label`](crate::Icon::label), which this
/// control sets internally rather than asking twice for the same string.
///
/// # With no handler, it is disabled
///
/// Same rule as [`Button`](crate::Button): drawn dimmed, no recogniser
/// registered at all, so it cannot steal a touch from whatever is behind it.
#[derive(Clone)]
pub struct FloatingActionButton {
    icon: IconData,
    label: String,
    on_pressed: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl FloatingActionButton {
    #[must_use]
    pub fn new(icon: IconData) -> Self {
        Self {
            icon,
            label: String::new(),
            on_pressed: None,
            key: None,
        }
    }

    /// What a screen reader says. Required in practice — an unlabelled FAB
    /// is a shape nobody using a screen reader can act on.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn on_pressed(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_pressed = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_pressed.is_some()
    }

    fn appearance(&self, theme: &ThemeData) -> (BoxDecoration, Color) {
        let colors = theme.colors;
        if self.is_enabled() {
            (
                BoxDecoration::filled(colors.primary).stadium(),
                colors.on_primary,
            )
        } else {
            (
                BoxDecoration::filled(colors.surface_variant).stadium(),
                crate::ColorScheme::dimmed(colors.on_surface),
            )
        }
    }

    fn body(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        let (decoration, ink) = self.appearance(theme);
        let decoration = crate::controls::pressed_fill(decoration, ink, press);

        DecoratedBox::new(decoration)
            .child(
                SizedBox::square(FAB_SIZE)
                    .child(Center::new().child(Icon::new(self.icon.clone()).color(ink))),
            )
            .into()
    }
}

impl Widget for FloatingActionButton {
    fn debug_name(&self) -> &'static str {
        "FloatingActionButton"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let interactive: WidgetNode = match &self.on_pressed {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let fab = self.clone();
                let theme = *theme;
                Pressable::themed(ctx, move |press| fab.body(&theme, press))
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
        let mut props = vec![("label", self.label.clone())];
        if !self.is_enabled() {
            props.push(("disabled", "true".to_owned()));
        }
        props
    }
}

impl fmt::Debug for FloatingActionButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloatingActionButton")
            .field("label", &self.label)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(FloatingActionButton);

#[cfg(test)]
mod tests {
    use crate::{icons, inflate, DebugNode, Theme};

    use super::*;

    fn built(fab: FloatingActionButton) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(fab))
    }

    #[test]
    fn with_no_handler_it_is_disabled_and_announced_that_way() {
        let node = built(FloatingActionButton::new(icons::add()).label("Add"));
        assert_eq!(
            node.find("Semantics").and_then(|n| n.property("enabled")),
            Some("false")
        );
    }

    #[test]
    fn with_a_handler_it_is_enabled_and_a_button() {
        let node = built(
            FloatingActionButton::new(icons::add())
                .label("Add")
                .on_pressed(|| {}),
        );
        let semantics = node.find("Semantics").expect("a Semantics node");
        assert_eq!(semantics.property("role"), Some("Button"));
        assert_eq!(
            semantics.property("enabled"),
            None,
            "an ordinary FAB says nothing, because enabled is the default"
        );
        assert_eq!(semantics.property("label"), Some("Add"));
    }

    #[test]
    fn disabled_falls_back_to_the_surface_variant() {
        let theme = ThemeData::light();
        let (decoration, _) = FloatingActionButton::new(icons::add()).appearance(&theme);
        assert_eq!(decoration.color, theme.colors.surface_variant);
    }
}
