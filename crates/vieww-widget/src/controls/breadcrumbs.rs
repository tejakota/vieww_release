use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Key, TextStyle};

use crate::{
    icons, widget_node_from, BuildContext, CrossAxisAlignment, Directionality, Flex,
    GestureDetector, Icon, MainAxisSize, SemanticRole, Semantics, SizedBox, Text, ThemeData,
    Widget, WidgetKind, WidgetNode,
};

/// A trail of steps that led to the current screen, each one a way back.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Breadcrumbs;
///
/// let trail = Breadcrumbs::new(vec![
///     "Home".to_owned(),
///     "Settings".to_owned(),
///     "Notifications".to_owned(),
/// ])
/// .on_selected(|index| println!("go back to {index}"));
/// ```
///
/// # The last item is the current screen, not a link
///
/// It is drawn in the ordinary text colour rather than the accent, carries
/// no tap target, and — being *where the user already is* — reads that way
/// to a screen reader too: [`SemanticRole::Label`], not
/// [`SemanticRole::Button`]. Every item before it is a step you can return
/// to, and is both.
///
/// # With no handler, nothing is tappable
///
/// Same rule as every control in this module: a `Breadcrumbs` with no
/// [`on_selected`](Self::on_selected) is a trail you can read but not act
/// on, not a broken one — matching what a `Button` with no `on_pressed` does.
#[derive(Clone)]
pub struct Breadcrumbs {
    items: Vec<String>,
    on_selected: Option<Rc<dyn Fn(usize)>>,
    key: Option<Key>,
}

impl Breadcrumbs {
    #[must_use]
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            on_selected: None,
            key: None,
        }
    }

    /// Called with the index of whichever step, other than the last, was
    /// tapped.
    #[must_use]
    pub fn on_selected(mut self, handler: impl Fn(usize) + 'static) -> Self {
        self.on_selected = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Breadcrumbs {
    fn debug_name(&self) -> &'static str {
        "Breadcrumbs"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let direction = Directionality::of(ctx);
        let last = self.items.len().saturating_sub(1);
        let gap = theme.metrics.gap;

        let mut children_nodes: Vec<WidgetNode> = Vec::with_capacity(self.items.len() * 2);
        for (index, item) in self.items.iter().enumerate() {
            let is_current = index == last;
            let style = TextStyle {
                color: if is_current {
                    theme.colors.on_surface
                } else {
                    theme.colors.primary
                },
                ..theme.text.body
            };
            let label = Text::new(item.clone()).style(style);

            let step: WidgetNode = if is_current {
                Semantics::new()
                    .role(SemanticRole::Label)
                    .label(item.clone())
                    .child(label)
                    .into()
            } else {
                match &self.on_selected {
                    Some(handler) => {
                        let handler = Rc::clone(handler);
                        // A word or two of text is not a finger's worth of
                        // hit area; without this a trail of short step names
                        // is a row of tap targets thinner than the platform
                        // allows.
                        let tappable = crate::controls::touch_target(
                            theme.metrics.touch_target,
                            GestureDetector::new()
                                .on_tap(move |_| handler(index))
                                .child(label),
                        );
                        Semantics::new()
                            .role(SemanticRole::Button)
                            .label(item.clone())
                            .child(tappable)
                            .into()
                    }
                    None => label.into(),
                }
            };
            children_nodes.push(step);

            if !is_current {
                // A chevron pointing towards where reading continues, which
                // is `forward` regardless of physical left/right — the same
                // reasoning `Flex`'s reading-order axis follows.
                children_nodes.push(
                    Icon::new(icons::chevron_forward(direction))
                        .size(16.0)
                        .color(theme.colors.on_surface_variant)
                        .into(),
                );
                children_nodes.push(SizedBox::width(gap / 2.0).into());
            }
        }

        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .text_direction(direction)
            .children(children_nodes)
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("steps", self.items.len().to_string())]
    }
}

impl fmt::Debug for Breadcrumbs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Breadcrumbs")
            .field("items", &self.items)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Breadcrumbs);

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(trail: Breadcrumbs) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(trail))
    }

    fn trail() -> Breadcrumbs {
        Breadcrumbs::new(vec![
            "Home".to_owned(),
            "Settings".to_owned(),
            "Wifi".to_owned(),
        ])
    }

    #[test]
    fn the_last_step_is_a_label_and_the_rest_are_buttons_when_selectable() {
        let node = built(trail().on_selected(|_| {}));
        let semantics = node.find_all("Semantics");
        let roles: Vec<Option<&str>> = semantics.iter().map(|n| n.property("role")).collect();
        assert_eq!(roles, vec![Some("Button"), Some("Button"), Some("Label")]);
    }

    #[test]
    fn with_no_handler_nothing_is_announced_as_tappable() {
        // The current screen still gets its `Label` semantics with no
        // handler at all — that announcement is about *where the user is*,
        // not about whether the trail is interactive — but no step should
        // ever be announced as a `Button` when there is nothing to tap.
        let node = built(trail());
        let semantics = node.find_all("Semantics");
        assert_eq!(
            semantics
                .iter()
                .map(|n| n.property("role"))
                .collect::<Vec<_>>(),
            vec![Some("Label")],
            "only the current step gets Semantics, and only as a Label"
        );
        assert_eq!(node.find_all("Text").len(), 3);
    }

    #[test]
    fn there_is_one_fewer_chevron_than_step() {
        let node = built(trail().on_selected(|_| {}));
        assert_eq!(node.find_all("Icon").len(), 2, "3 steps, 2 separators");
    }

    #[test]
    fn the_stored_handler_is_called_with_the_tapped_index() {
        // Accessing the private field directly rather than through a tree —
        // this module is a child of the one that defines it, so the field
        // is visible here the same way `Chip`'s tests reach `on_deleted`.
        // The hit test decides which step wins in a real tree; the point
        // here is only that the closure `Breadcrumbs` actually stores does
        // the right thing when called.
        let selected = Rc::new(Cell::new(None));
        let breadcrumbs = trail().on_selected({
            let selected = Rc::clone(&selected);
            move |index| selected.set(Some(index))
        });

        let handler = breadcrumbs.on_selected.expect("selectable");
        handler(1);
        assert_eq!(selected.get(), Some(1));
    }
}
