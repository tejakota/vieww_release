use vieww_foundation::Key;

use crate::{widget_node_from, FlexFactor, FlexFit, Widget, WidgetKind, WidgetNode};

/// Gives its child a share of the space a [`Flex`](crate::Flex) has left over.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Flexible;
///
/// // The label takes whatever is left after the switch has had its say, so it
/// // can wrap instead of pushing the switch off the edge.
/// let row = Flex::row().children(children![
///     Flexible::expanded(1).child(Text::new("A setting with a long name")),
///     SizedBox::square(48.0),
/// ]);
/// ```
///
/// # Expanded and Flexible are one widget here
///
/// Classic toolkits have two: `Expanded` is `Flexible` with `FlexFit.tight`. They are
/// [`expanded`](Self::expanded) and [`new`](Self::new) instead, because the
/// difference is one field and two types would mean two elements, two render
/// objects and two factory entries to say it.
///
/// # Why this is a render object and not parent data
///
/// The classic design stores a flex factor in the *parent's* slot for the child — a
/// `parentData` field that `Expanded`, a `ParentDataWidget`, writes into. That
/// buys one thing: no extra node in the tree.
///
/// Here the factor lives on a layout-transparent render object of the child's
/// own, and `RenderFlex` reads it back through
/// [`RenderObject::flex`](https://docs.rs/vieww-render). The cost is one node
/// per flexible child. What it avoids is a `dyn Any` slot on every node in the
/// tree, a `setup_parent_data` step in attachment, and a class of bug where the
/// data is stale because the child moved to a different parent. The same
/// trade-off as [`RenderSemantics`](https://docs.rs/vieww-render), which is
/// also a transparent wrapper carrying something its parent reads.
///
/// # It must be a *direct* child of the flex
///
/// `Flex > Flexible > Text` works; `Flex > Padding > Flexible > Text` does not,
/// because the flex only inspects the children it lays out. The classic has the
/// same rule and asserts on it. Here it silently lays the child out
/// inflexibly — wrap the `Padding` instead of the `Text`.
#[derive(Debug, Clone)]
pub struct Flexible {
    factor: FlexFactor,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Flexible {
    /// Take *up to* `flex` shares of what is left over.
    ///
    /// A child that wants less keeps its own size, and the remainder stays free
    /// for the flex's [`MainAxisAlignment`](crate::MainAxisAlignment).
    #[must_use]
    pub const fn new(flex: u16) -> Self {
        Self {
            factor: FlexFactor::loose(flex),
            child: None,
            key: None,
        }
    }

    /// Fill `flex` shares of what is left over, exactly.
    ///
    /// The classic `Expanded` semantics. This is what a label that should absorb the slack
    /// in a row wants, and what a divider spanning the remaining width needs.
    #[must_use]
    pub const fn expanded(flex: u16) -> Self {
        Self {
            factor: FlexFactor::tight(flex),
            child: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn fit(mut self, fit: FlexFit) -> Self {
        self.factor.fit = fit;
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn factor(&self) -> FlexFactor {
        self.factor
    }
}

impl Widget for Flexible {
    fn debug_name(&self) -> &'static str {
        "Flexible"
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

widget_node_from!(Flexible);
