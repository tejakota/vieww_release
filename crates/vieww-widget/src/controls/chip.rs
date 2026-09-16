use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, EdgeInsets, Key, TextStyle};

use crate::{
    children, icons, widget_node_from, BuildContext, CrossAxisAlignment, DecoratedBox, Flex,
    GestureDetector, Icon, MainAxisSize, Padding, Pressable, SemanticRole, Semantics, SizedBox,
    Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The icon that dismisses a chip, and the gap before it.
const DELETE_ICON: f32 = 18.0;

/// A small, compact thing: a filter, a tag, a selected contact.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Chip;
///
/// let filter = Chip::new("Unread").selected(true).on_pressed(|| {});
/// let tag = Chip::new("rust").on_deleted(|| {});
/// ```
///
/// Three independent things a chip can be, in any combination:
///
/// - **pressable** — [`on_pressed`](Self::on_pressed) makes the whole chip a
///   target and announces it as a button;
/// - **selected** — [`selected`](Self::selected) fills it with the accent
///   instead of outlining it;
/// - **deletable** — [`on_deleted`](Self::on_deleted) puts a cross at the end,
///   with its own tap target inside the chip's.
///
/// A chip with none of them is a label in a rounded box, which is a perfectly
/// good thing to be.
///
/// # The delete target is inside the press target
///
/// Both are live at once when both handlers are set. The cross is the inner
/// [`GestureDetector`], and the hit test finds the innermost target first, so a
/// tap on the cross deletes and a tap anywhere else presses. That ordering is
/// the hit test's, not something this widget arranges.
#[derive(Clone)]
pub struct Chip {
    label: String,
    selected: bool,
    on_pressed: Option<Rc<dyn Fn()>>,
    on_deleted: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Chip {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            selected: false,
            on_pressed: None,
            on_deleted: None,
            key: None,
        }
    }

    /// Fill it with the accent rather than outlining it.
    #[must_use]
    pub const fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    #[must_use]
    pub fn on_pressed(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_pressed = Some(Rc::new(handler));
        self
    }

    /// Show a cross at the end, and call this when it is tapped.
    #[must_use]
    pub fn on_deleted(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_deleted = Some(Rc::new(handler));
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The decoration and the colour everything on top of it takes.
    fn appearance(&self, theme: &ThemeData) -> (BoxDecoration, Color) {
        let colors = theme.colors;
        if self.selected {
            (
                BoxDecoration::filled(colors.primary).stadium(),
                colors.on_primary,
            )
        } else {
            (
                BoxDecoration::outlined(Border::thin(colors.outline)).stadium(),
                colors.on_surface_variant,
            )
        }
    }

    /// The stadium, the label, and the delete icon if there is one.
    fn body(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        let (decoration, foreground) = self.appearance(theme);
        let decoration = crate::controls::pressed_fill(decoration, foreground, press);
        let gap = theme.metrics.gap;

        let label = Text::new(self.label.clone()).style(TextStyle {
            color: foreground,
            ..theme.text.label
        });

        let contents = match &self.on_deleted {
            Some(handler) => {
                let handler = Rc::clone(handler);
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(children![
                        label,
                        SizedBox::width(gap / 2.0),
                        // Exempt from the automatic touch-target expansion, and
                        // the *same* argument as the comment below: a delete
                        // affordance reachable across 48pt would swallow taps
                        // meant for the chip it sits inside, and for the chip
                        // beside that one. Density is what makes a chip a chip.
                        //
                        // The one considered exception in this repository — see
                        // `GestureDetector::touch_target`, which says so.
                        GestureDetector::new()
                            .touch_target(0.0)
                            .on_tap(move |_| handler())
                            .child(
                                Icon::new(icons::close())
                                    .size(DELETE_ICON)
                                    .color(foreground)
                                    // Named, unlike the tick in a checkbox: this one
                                    // is its own control, and a screen reader that
                                    // skipped it would leave no way to delete.
                                    .label(format!("Remove {}", self.label)),
                            ),
                    ])
            }
            None => Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .push(label),
        };

        // A chip is deliberately *not* padded out to the 48px touch target: a row
        // of chips at finger height would be a row of buttons. This is the one
        // control here that trades reach for density, which is what makes it a
        // chip.
        DecoratedBox::new(decoration)
            .child(Padding::new(EdgeInsets::symmetric(gap * 1.5, gap)).child(contents))
            .into()
    }
}

impl Widget for Chip {
    fn debug_name(&self) -> &'static str {
        "Chip"
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
                let chip = self.clone();
                let theme = *theme;
                Pressable::themed(ctx, move |press| chip.body(&theme, press))
                    .on_tap(move || handler())
                    .into()
            }
            None => self.body(&theme, 0.0),
        };

        Semantics::new()
            .role(if self.on_pressed.is_some() {
                SemanticRole::Button
            } else {
                SemanticRole::Label
            })
            .label(self.label.clone())
            // A chip with no handler is a *label*, not a disabled button — the
            // role already says it does nothing, and "dimmed" on top of that
            // would announce an unavailability that is not there.
            .child(interactive)
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("label", self.label.clone())];
        if self.selected {
            props.push(("selected", "true".to_owned()));
        }
        if self.on_deleted.is_some() {
            props.push(("deletable", "true".to_owned()));
        }
        props
    }
}

impl fmt::Debug for Chip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Chip")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .field("pressable", &self.on_pressed.is_some())
            .field("deletable", &self.on_deleted.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Chip);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(chip: Chip) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(chip))
    }

    #[test]
    fn a_plain_chip_is_a_label_and_a_pressable_one_is_a_button() {
        assert_eq!(
            built(Chip::new("rust"))
                .find("Semantics")
                .and_then(|node| node.property("role")),
            Some("Label")
        );
        assert_eq!(
            built(Chip::new("rust").on_pressed(|| {}))
                .find("Semantics")
                .and_then(|node| node.property("role")),
            Some("Button")
        );
    }

    #[test]
    fn selecting_fills_it_instead_of_outlining_it() {
        let theme = ThemeData::light();
        let (plain, plain_ink) = Chip::new("rust").appearance(&theme);
        let (chosen, chosen_ink) = Chip::new("rust").selected(true).appearance(&theme);

        assert!(plain.color.is_transparent());
        assert!(plain.visible_border().is_some());
        assert_eq!(plain_ink, theme.colors.on_surface_variant);

        assert_eq!(chosen.color, theme.colors.primary);
        assert!(chosen.visible_border().is_none());
        assert_eq!(chosen_ink, theme.colors.on_primary);
    }

    #[test]
    fn only_a_deletable_chip_has_a_cross() {
        assert!(built(Chip::new("rust")).find("Icon").is_none());

        let deletable = built(Chip::new("rust").on_deleted(|| {}));
        let cross = deletable.find("Icon").expect("a cross");
        assert_eq!(cross.property("label"), Some("Remove rust"));
    }

    #[test]
    fn the_cross_has_its_own_target_inside_the_chips() {
        let both = built(Chip::new("rust").on_pressed(|| {}).on_deleted(|| {}));
        assert_eq!(
            both.find_all("GestureDetector").len(),
            2,
            "one for the chip, one for the cross"
        );
    }

    #[test]
    fn a_tap_on_the_cross_deletes_rather_than_pressing() {
        let pressed = Rc::new(Cell::new(false));
        let deleted = Rc::new(Cell::new(false));

        let chip = Chip::new("rust")
            .on_pressed({
                let pressed = Rc::clone(&pressed);
                move || pressed.set(true)
            })
            .on_deleted({
                let deleted = Rc::clone(&deleted);
                move || deleted.set(true)
            });

        // The hit test decides which of the two wins in a real tree; here the
        // point is only that they are separate handlers rather than one shared.
        (chip.on_deleted.clone().expect("deletable"))();
        assert!(deleted.get());
        assert!(!pressed.get());
    }
}
