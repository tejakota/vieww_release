use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, Key};

use crate::controls::touch_target;
use crate::{
    icons, widget_node_from, BuildContext, ColorScheme, DecoratedBox, Handler, Icon, Pressable,
    SemanticRole, Semantics, SizedBox, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The side of the box itself, inside its touch target.
const BOX: f32 = 20.0;
/// How thick the outline of an unticked box is. Two pixels rather than one:
/// an empty box is nothing but its outline, so a hairline reads as absent.
const OUTLINE: f32 = 2.0;

/// A box that is ticked or not.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Checkbox;
/// use std::rc::Rc;
///
/// # let agreed = false;
/// # let set: Rc<dyn Fn(bool)> = Rc::new(|_| {});
/// let terms = Checkbox::new(agreed)
///     .label("I agree")
///     .on_changed(Rc::new(move |next| set(next)));
/// ```
///
/// Controlled and disabled-without-a-handler, like every control here. Prefer a
/// [`Switch`](crate::Switch) for something that takes effect immediately and a
/// checkbox for something that is submitted — which is the distinction users
/// have learned, whatever the platform guidelines say this week.
///
/// # Ticked is filled *and* ticked
///
/// The fill and the outline carry the state on their own, without the tick and
/// without relying on colour — which is what makes it readable to someone who
/// cannot distinguish the two colours. The tick is drawn on top of that, not
/// instead of it.
#[derive(Clone)]
pub struct Checkbox {
    value: bool,
    label: Option<String>,
    on_changed: Option<Handler<bool>>,
    key: Option<Key>,
}

impl Checkbox {
    #[must_use]
    pub const fn new(value: bool) -> Self {
        Self {
            value,
            label: None,
            on_changed: None,
            key: None,
        }
    }

    /// What a screen reader calls this box. Not drawn — see
    /// [`Switch::label`](crate::Switch::label) for why.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called with the value the box would like to become. Giving a handler is
    /// what enables it.
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

    /// How the box is painted.
    fn appearance(&self, theme: &ThemeData) -> BoxDecoration {
        let colors = theme.colors;
        let enabled = self.is_enabled();
        let dim = |color| {
            if enabled {
                color
            } else {
                ColorScheme::dimmed(color)
            }
        };
        // **A circle on Apple, a rounded square elsewhere.** iOS has no square
        // checkbox: a settings list ticks with a bare glyph and a form uses a
        // filled circle. Drawing an Android rounded square in an iOS preview is
        // one of the two or three things that makes a screenshot obviously not
        // native. Otherwise: half the theme's corner radius, the same rounding
        // on a box a quarter the size of a button.
        let radius = if theme.platform.is_apple() {
            BOX / 2.0
        } else {
            theme.metrics.corner / 2.0
        };

        if self.value {
            BoxDecoration::filled(dim(colors.primary)).radius(radius)
        } else {
            BoxDecoration::outlined(Border::new(dim(colors.outline), OUTLINE)).radius(radius)
        }
    }

    /// The colour drawn on top of the box, which is what a press washes it with.
    ///
    /// The tick when there is one; the outline when there is not, since an empty
    /// box is nothing *but* its outline and has no other colour of its own.
    const fn ink(&self, theme: &ThemeData) -> Color {
        if self.value {
            theme.colors.on_primary
        } else {
            theme.colors.outline
        }
    }

    /// The box and its tick, at the size a finger can hit.
    fn body(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        // The tick is unlabelled: the box around it is already announced as a
        // checkbox that is on, and a screen reader stopping on the tick as well
        // would say it twice.
        let contents: WidgetNode = if self.value {
            Icon::new(icons::check())
                .size(BOX)
                .color(if self.is_enabled() {
                    theme.colors.on_primary
                } else {
                    ColorScheme::dimmed(theme.colors.on_primary)
                })
                .into()
        } else {
            SizedBox::square(BOX).into()
        };

        let decoration =
            crate::controls::pressed_fill(self.appearance(theme), self.ink(theme), press);

        touch_target(
            theme.metrics.touch_target,
            DecoratedBox::new(decoration).child(contents),
        )
        .into()
    }
}

impl Widget for Checkbox {
    fn debug_name(&self) -> &'static str {
        "Checkbox"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let interactive: WidgetNode = match &self.on_changed {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let next = !self.value;
                let checkbox = self.clone();
                let theme = *theme;
                Pressable::themed(ctx, move |press| checkbox.body(&theme, press))
                    .on_tap(move || handler(next))
                    .into()
            }
            None => self.body(&theme, 0.0),
        };

        let mut semantics = Semantics::new()
            .role(SemanticRole::CheckBox)
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

impl fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Checkbox")
            .field("value", &self.value)
            .field("label", &self.label)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Checkbox);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(checkbox: Checkbox) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(checkbox))
    }

    #[test]
    fn a_tap_reports_the_opposite_of_the_current_value() {
        let reported: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let sink = Rc::clone(&reported);
        let box_ = Checkbox::new(false).on_changed(Rc::new(move |next| sink.set(Some(next))));

        let handler = box_.on_changed.clone().expect("enabled");
        handler(!box_.value());
        assert_eq!(reported.get(), Some(true));
    }

    #[test]
    fn ticked_is_filled_and_unticked_is_an_outline() {
        let theme = ThemeData::light();
        // Enabled, both times: a disabled box is dimmed, which is a different
        // question from which of the two states it is in.
        let enabled = |value| Checkbox::new(value).on_changed(Rc::new(|_| {}));
        let ticked = enabled(true).appearance(&theme);
        let empty = enabled(false).appearance(&theme);

        assert_eq!(ticked.color, theme.colors.primary);
        assert!(ticked.visible_border().is_none());

        assert!(empty.color.is_transparent());
        assert_eq!(empty.visible_border().map(|b| b.width), Some(OUTLINE));
    }

    #[test]
    fn a_ticked_box_draws_a_tick_and_an_empty_one_draws_nothing() {
        let ticked = built(Checkbox::new(true).on_changed(Rc::new(|_| {})));
        let empty = built(Checkbox::new(false).on_changed(Rc::new(|_| {})));

        let tick = ticked.find("Icon").expect("a tick");
        assert_eq!(
            tick.property("color"),
            Some(ThemeData::light().colors.on_primary.to_string().as_str()),
            "the tick is drawn on the fill, so it takes the on-fill colour"
        );
        assert!(empty.find("Icon").is_none());
    }

    #[test]
    fn a_disabled_box_has_no_recogniser() {
        assert!(built(Checkbox::new(true)).find("GestureDetector").is_none());
        assert!(built(Checkbox::new(true).on_changed(Rc::new(|_| {})))
            .find("GestureDetector")
            .is_some());
    }

    #[test]
    fn a_screen_reader_hears_a_checkbox_rather_than_a_switch() {
        let tree = built(Checkbox::new(false).label("I agree"));
        let semantics = tree.find("Semantics").expect("annotated");

        assert_eq!(semantics.property("role"), Some("CheckBox"));
        assert_eq!(semantics.property("label"), Some("I agree"));
        assert_eq!(semantics.property("toggled"), Some("false"));
    }

    #[test]
    fn the_box_is_smaller_than_the_area_that_takes_the_tap() {
        let theme = ThemeData::light();
        let tree = built(Checkbox::new(true).on_changed(Rc::new(|_| {})));
        let target = tree.find("Constrained").expect("a touch target");

        let min = theme.metrics.touch_target;
        assert!(BOX < min, "the drawn box is the visual, not the target");
        assert_eq!(
            target.property("constraints"),
            Some(format!("w[{min}..inf] h[{min}..inf]").as_str())
        );
    }
}
