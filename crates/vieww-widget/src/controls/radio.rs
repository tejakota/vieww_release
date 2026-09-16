use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, Key};

use crate::controls::touch_target;
use crate::{
    widget_node_from, BuildContext, Center, ColorScheme, DecoratedBox, Handler, Pressable,
    SemanticRole, Semantics, SizedBox, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The outside of the ring, inside its touch target. The same as a
/// [`Checkbox`](crate::Checkbox)'s box, so a form mixing the two lines up.
const RING: f32 = 20.0;
/// How thick the ring is. Two pixels for the reason a checkbox's outline is:
/// an unselected radio is nothing but its ring, and a hairline reads as absent.
const OUTLINE: f32 = 2.0;
/// The filled dot in the middle of a selected radio.
///
/// Ten of the ring's twenty, which leaves a clear gap between the dot and the
/// ring at every density. A dot that touches its ring reads as a filled circle,
/// which is a checkbox.
const DOT: f32 = 10.0;

/// One of a set, of which exactly one is chosen.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Radio;
/// use std::rc::Rc;
///
/// # #[derive(PartialEq, Clone, Copy)] enum Size { Small, Large }
/// # let chosen = Size::Small;
/// # let choose: Rc<dyn Fn(Size)> = Rc::new(|_| {});
/// let small = Radio::new(chosen == Size::Small)
///     .label("Small")
///     .on_selected({
///         let choose = Rc::clone(&choose);
///         Rc::new(move |()| choose(Size::Small))
///     });
/// ```
///
/// # Why this takes a `bool` rather than a value and a group
///
/// A group-value radio takes the option's value and the group's current value
/// and compares them for you. This takes the answer instead, for the reason
/// every control in this module takes its value rather than owning it: the
/// application already holds the selection in a signal, `chosen == Self::Small`
/// is the comparison it was going to write anyway, and a generic parameter
/// buys nothing but a turbofish at every call site.
///
/// It also means a radio can be driven by something that is not an enum at all
/// — a row index, a `String` from a server — without the group type having to
/// be nameable.
///
/// # Selecting is not toggling
///
/// [`on_selected`](Self::on_selected) takes no value, and a radio that is
/// already selected still calls it. There is no "deselect": the only thing a
/// user can express by tapping a radio is *this one*, and a set with nothing
/// chosen is a state the application decides to allow, not one a tap produces.
///
/// That is the whole difference from [`Checkbox`](crate::Checkbox), which hands
/// back `!value`.
#[derive(Clone)]
pub struct Radio {
    selected: bool,
    label: Option<String>,
    on_selected: Option<Handler<()>>,
    key: Option<Key>,
}

impl Radio {
    /// A radio that is selected, or not.
    #[must_use]
    pub const fn new(selected: bool) -> Self {
        Self {
            selected,
            label: None,
            on_selected: None,
            key: None,
        }
    }

    /// What a screen reader calls this option. Not drawn — see
    /// [`Switch::label`](crate::Switch::label) for why.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Called when this option is chosen. Giving a handler is what enables it.
    ///
    /// Called even when this radio is already selected — see the type docs.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<()>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn is_selected(&self) -> bool {
        self.selected
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_selected.is_some()
    }

    /// The colour of the ring and the dot.
    ///
    /// Also the ink a press washes the fill towards, which works here for the
    /// same reason it works for an outlined checkbox: an unselected radio has no
    /// fill of its own, so its ring is the only colour it has.
    fn ink(&self, theme: &ThemeData) -> Color {
        let resting = if self.selected {
            theme.colors.primary
        } else {
            theme.colors.outline
        };
        if self.is_enabled() {
            resting
        } else {
            ColorScheme::dimmed(resting)
        }
    }

    /// How the ring is painted.
    ///
    /// Always outlined, never filled — which is the visual difference from a
    /// checkbox, and it is load-bearing rather than decorative. A filled circle
    /// and a filled rounded square are one glance apart; a ring and a filled box
    /// are not, and that distinction is what tells a user whether choosing this
    /// one unchooses another.
    fn appearance(&self, theme: &ThemeData) -> BoxDecoration {
        // Half the side, so the rounding closes into a circle.
        BoxDecoration::outlined(Border::new(self.ink(theme), OUTLINE)).radius(RING / 2.0)
    }

    /// The ring, its dot, and the space a finger can hit.
    fn body(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        // Unlabelled: the ring around it is already announced as a selected
        // radio, and a screen reader stopping on the dot would say it twice.
        //
        // **The `Center` is load-bearing and its absence was a visible bug.**
        // `SizedBox::square(RING)` is a *tight* constraint and both a
        // `DecoratedBox` and a `SizedBox` pass tight constraints straight
        // through — so without something that loosens them, the dot was laid
        // out at the ring's full 20 pixels and painted as a rounded square
        // filling the ring, which is a checkbox. `Center` loosens what it hands
        // its child (see `RenderAlign::layout`) and fills the space itself,
        // which is the whole reason to reach for it here rather than padding.
        let contents: WidgetNode = if self.selected {
            Center::new()
                .child(
                    DecoratedBox::new(BoxDecoration::filled(self.ink(theme)).radius(DOT / 2.0))
                        .child(SizedBox::square(DOT)),
                )
                .into()
        } else {
            SizedBox::square(DOT).into()
        };

        let decoration =
            crate::controls::pressed_fill(self.appearance(theme), self.ink(theme), press);

        touch_target(
            theme.metrics.touch_target,
            // The ring is a fixed size whatever is inside it, so the dot
            // appearing does not move the row it sits in.
            SizedBox::square(RING).child(DecoratedBox::new(decoration).child(contents)),
        )
        .into()
    }
}

impl Widget for Radio {
    fn debug_name(&self) -> &'static str {
        "Radio"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let interactive: WidgetNode = match &self.on_selected {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let radio = self.clone();
                let theme = *theme;
                Pressable::themed(ctx, move |press| radio.body(&theme, press))
                    .on_tap(move || handler(()))
                    .into()
            }
            None => self.body(&theme, 0.0),
        };

        let mut semantics = Semantics::new()
            .role(SemanticRole::Radio)
            // `toggled` rather than a role-specific flag: AccessKit models a
            // radio's selection with the same property a checkbox's tick uses,
            // and the role is what tells a screen reader which word to say.
            .toggled(self.selected)
            .enabled(self.is_enabled());
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        semantics.child(interactive).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("selected", self.selected.to_string())];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        props.push(("enabled", self.is_enabled().to_string()));
        props
    }
}

/// Hand-written because `Handler` is a closure and closures are not `Debug`.
impl fmt::Debug for Radio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Radio")
            .field("selected", &self.selected)
            .field("label", &self.label)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Radio);

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_radio_with_no_handler_is_disabled() {
        let radio = Radio::new(false);
        assert!(!radio.is_enabled());
        assert!(Radio::new(false).on_selected(Rc::new(|()| {})).is_enabled());
    }

    #[test]
    fn selecting_an_already_selected_radio_still_reports_it() {
        let chosen = Rc::new(Cell::new(0_u32));
        let counted = Rc::clone(&chosen);
        let radio = Radio::new(true).on_selected(Rc::new(move |()| {
            counted.set(counted.get() + 1);
        }));

        // What a tap does. Twice, on a radio that is already on.
        if let Some(handler) = &radio.on_selected {
            handler(());
            handler(());
        }

        assert_eq!(
            chosen.get(),
            2,
            "there is no deselecting a radio, so an already-selected one must \
             still report the only thing a tap can mean"
        );
    }

    #[test]
    fn the_tree_says_radio_rather_than_checkbox() {
        let dump = crate::debug_tree(Radio::new(true).label("Small"));
        assert!(dump.contains("Radio"), "{dump}");
        assert!(
            !dump.contains("Checkbox"),
            "borrowing the checkbox role would have a screen reader say the \
             wrong word: {dump}"
        );
    }

    #[test]
    fn the_debug_properties_carry_what_a_tree_dump_needs() {
        let props = Radio::new(true).label("Large").debug_properties();
        assert!(props.contains(&("selected", "true".to_owned())));
        assert!(props.contains(&("label", "Large".to_owned())));
        assert!(props.contains(&("enabled", "false".to_owned())));
    }
}
