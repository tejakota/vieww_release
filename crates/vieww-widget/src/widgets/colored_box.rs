use vieww_foundation::{Color, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Fills its own bounds with a solid color, then paints its child on top.
///
/// It takes its size entirely from its child — a `ColoredBox` with no child in
/// unbounded constraints is zero-sized and paints nothing. To fill a region,
/// give it a child that expands, or wrap it in a [`SizedBox`](crate::SizedBox).
#[derive(Debug, Clone)]
pub struct ColoredBox {
    color: Color,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl ColoredBox {
    #[must_use]
    pub const fn new(color: Color) -> Self {
        Self {
            color,
            child: None,
            key: None,
        }
    }

    /// Set the child.
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

    /// The fill color.
    #[must_use]
    pub const fn color(&self) -> Color {
        self.color
    }
}

impl Widget for ColoredBox {
    fn debug_name(&self) -> &'static str {
        "ColoredBox"
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

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("color", self.color.to_string())]
    }
}

widget_node_from!(ColoredBox);
