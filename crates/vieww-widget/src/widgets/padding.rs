use vieww_foundation::{EdgeInsets, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Insets its child by [`EdgeInsets`].
///
/// In Phase 3 this deflates the incoming constraints, lays the child out
/// against them, and reports the child's size grown back by the insets.
#[derive(Debug, Clone)]
pub struct Padding {
    insets: EdgeInsets,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Padding {
    #[must_use]
    pub fn new(insets: EdgeInsets) -> Self {
        Self {
            insets,
            child: None,
            key: None,
        }
    }

    /// The same inset on all four sides.
    #[must_use]
    pub fn all(value: f32) -> Self {
        Self::new(EdgeInsets::all(value))
    }

    /// Horizontal insets on left/right, vertical on top/bottom.
    #[must_use]
    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self::new(EdgeInsets::symmetric(horizontal, vertical))
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

    /// The insets applied to the child.
    #[must_use]
    pub const fn insets(&self) -> EdgeInsets {
        self.insets
    }
}

impl Widget for Padding {
    fn debug_name(&self) -> &'static str {
        "Padding"
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
        vec![("insets", self.insets.to_string())]
    }
}

widget_node_from!(Padding);
