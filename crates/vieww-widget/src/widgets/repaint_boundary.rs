use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Gives its subtree its own recording, so that repainting it does not repaint
/// the screen.
///
/// A repaint in an ordinary subtree walks up to the root and re-records
/// everything on the way. One inside this widget stops here: only this subtree
/// is re-recorded, and the surrounding content keeps the commands it had.
///
/// # When it is worth one
///
/// A layer costs memory for its recording and a step in every composite, so this
/// is not free and not a default. It pays when a small subtree changes often
/// while a large one around it does not — an animating card over a static page,
/// a scrolling viewport, a counter in a toolbar. Wrapping a subtree that changes
/// only when its parent does makes frames slower, not faster; the standard guidance
/// for its own `RepaintBoundary` is the same.
///
/// Layout is unaffected: the child sees exactly the constraints this widget was
/// given, and this widget is exactly the size the child chose.
#[derive(Debug, Clone, Default)]
pub struct RepaintBoundary {
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl RepaintBoundary {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
}

impl Widget for RepaintBoundary {
    fn debug_name(&self) -> &'static str {
        "RepaintBoundary"
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

widget_node_from!(RepaintBoundary);
