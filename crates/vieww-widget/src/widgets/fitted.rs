use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Shrinks its child until it fits, instead of letting it run off the edge.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// // A card designed at 400 wide, on a screen that may be narrower.
/// let card = Fitted::new().child(SizedBox::from_size(Size::new(400.0, 200.0)));
/// ```
///
/// The child is measured **unconstrained** — its natural size — and then drawn
/// through a uniform scale small enough to bring it inside the space available.
///
/// # It only ever shrinks
///
/// A child that already fits is left at full size. Enlarging a small child to
/// fill its parent is a different operation, and doing it here would blow a
/// short label up to fill a screen the moment somebody reached for "make this
/// fit".
///
/// # When not to reach for this
///
/// **Scaling shrinks the text along with everything else**, so a screen that
/// does not fit is usually better served by making it scrollable, or by letting
/// a child [`Flexible`](crate::Flexible). Use this where the layout is genuinely
/// fixed and has to be seen whole — a diagram, a seating plan, a card designed
/// at one size.
///
/// Two consequences worth knowing before it surprises you:
///
/// - the child is measured with unbounded constraints, so a [`Text`](crate::Text)
///   inside **will not wrap** — it reports one long line and is then scaled down;
/// - because the child always fits afterwards, this also silences the overflow
///   report for that subtree. That is correct, and it is worth being deliberate
///   about: reaching for `Fitted` to quiet a message is how a screen ends up
///   with type nobody can read.
#[derive(Debug, Clone, Default)]
pub struct Fitted {
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Fitted {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Fitted {
    fn debug_name(&self) -> &'static str {
        "Fitted"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
}

widget_node_from!(Fitted);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{debug_tree, SizedBox};

    #[test]
    fn a_fitted_box_is_a_render_widget_carrying_its_child() {
        let dump = debug_tree(Fitted::new().child(SizedBox::square(40.0)));
        assert!(dump.contains("Fitted"), "{dump}");
        assert!(dump.contains("Constrained"), "{dump}");
    }

    #[test]
    fn a_childless_fitted_box_is_a_leaf_rather_than_a_broken_parent() {
        // `RenderFitted` returns the smallest allowed size with no child, so
        // this has to be expressible rather than a panic waiting to happen.
        let dump = debug_tree(Fitted::new());
        assert!(dump.contains("Fitted"), "{dump}");
    }
}
