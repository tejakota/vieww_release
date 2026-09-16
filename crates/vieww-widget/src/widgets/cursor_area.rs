use vieww_foundation::{Cursor, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Gives everything inside it a pointer shape.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::CursorArea;
/// use vieww_foundation::Cursor;
///
/// // The seam between two resizable panes.
/// let seam = CursorArea::new(Cursor::ResizeColumn).child(SizedBox::width(6.0));
/// ```
///
/// Layout, paint and input are unchanged — the child is drawn and behaves
/// exactly as it would on its own. What this adds is the answer the window asks
/// for when it decides what the mouse should look like.
///
/// # Innermost wins
///
/// The shape is read off the hit path from the target outwards, so a
/// [`TextField`](crate::TextField) inside a `CursorArea` still shows its I-beam.
/// Wrap a region, and let the controls inside it say otherwise.
///
/// # Touch platforms
///
/// A phone has no pointer to shape, so this is a no-op there rather than an
/// error. A widget can ask for a cursor without knowing what it is running on.
#[derive(Debug, Clone)]
pub struct CursorArea {
    cursor: Cursor,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl CursorArea {
    #[must_use]
    pub const fn new(cursor: Cursor) -> Self {
        Self {
            cursor,
            child: None,
            key: None,
        }
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
    pub const fn shape(&self) -> Cursor {
        self.cursor
    }
}

impl Widget for CursorArea {
    fn debug_name(&self) -> &'static str {
        "CursorArea"
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
        vec![("cursor", self.cursor.css_name().to_owned())]
    }
}

widget_node_from!(CursorArea);
