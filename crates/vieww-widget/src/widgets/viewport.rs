use std::fmt;

use vieww_foundation::{Axis, Key};

use crate::{widget_node_from, Handler, Widget, WidgetKind, WidgetNode};

/// How long the content is, and how much of it is on screen.
///
/// Reported *out* of layout, because it is the one thing about scrolling that
/// nothing above the render layer can know: the content's length is whatever its
/// children turned out to be, and the viewport's length is whatever its parent
/// allowed. Physics, clamping and a scrollbar all need both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollExtents {
    /// The window's own length along the scroll axis.
    pub viewport: f32,
    /// The child's length along the same axis.
    pub content: f32,
}

impl ScrollExtents {
    #[must_use]
    pub const fn new(viewport: f32, content: f32) -> Self {
        Self { viewport, content }
    }

    /// The furthest the content can be scrolled before its end is in view.
    #[must_use]
    pub fn max_offset(self) -> f32 {
        (self.content - self.viewport).max(0.0)
    }
}

/// Shows part of a taller (or wider) child, clipped to its own bounds.
///
/// The mechanical half of scrolling: it gives the child an **unbounded** main
/// axis so the child can be as long as it likes, then draws the window into it at
/// [`offset`](Self::offset) and clips everything outside.
///
/// It does not scroll itself. Where the offset comes from — a drag, a fling, a
/// scrollbar, a keyboard — is somebody else's business, which is why this takes a
/// number rather than owning a gesture. [`Scrollable`](crate::Scrollable) is the
/// widget that wires a drag to it.
#[derive(Clone)]
pub struct Viewport {
    axis: Axis,
    offset: f32,
    on_extents: Option<Handler<ScrollExtents>>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Viewport {
    /// A viewport scrolling along `axis`, showing the start of its child.
    #[must_use]
    pub const fn new(axis: Axis) -> Self {
        Self {
            axis,
            offset: 0.0,
            on_extents: None,
            child: None,
            key: None,
        }
    }

    /// A vertical viewport — the usual one.
    #[must_use]
    pub const fn vertical() -> Self {
        Self::new(Axis::Vertical)
    }

    /// A horizontal viewport.
    #[must_use]
    pub const fn horizontal() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// How far into the child to start showing, in logical pixels.
    ///
    /// Positive scrolls *into* the content: the child moves up (or left) by this
    /// much.
    #[must_use]
    pub const fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    /// Called from layout when the viewport's or the content's length changes.
    ///
    /// Only on a *change*, so a handler that writes a signal does not mark
    /// something pending on every frame and rebuild forever.
    #[must_use]
    pub fn on_extents(mut self, handler: Handler<ScrollExtents>) -> Self {
        self.on_extents = Some(handler);
        self
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

    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    #[must_use]
    pub const fn scroll_offset(&self) -> f32 {
        self.offset
    }

    /// The extents handler, for the render layer to call.
    #[must_use]
    pub const fn extents_handler(&self) -> Option<&Handler<ScrollExtents>> {
        self.on_extents.as_ref()
    }
}

impl Widget for Viewport {
    fn debug_name(&self) -> &'static str {
        "Viewport"
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
        vec![
            ("axis", format!("{:?}", self.axis)),
            ("offset", self.offset.to_string()),
        ]
    }
}

impl fmt::Debug for Viewport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Viewport")
            .field("axis", &self.axis)
            .field("offset", &self.offset)
            .field("reports_extents", &self.on_extents.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Viewport);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_shorter_than_the_window_cannot_be_scrolled() {
        assert_eq!(ScrollExtents::new(300.0, 120.0).max_offset(), 0.0);
        assert_eq!(ScrollExtents::new(300.0, 900.0).max_offset(), 600.0);
    }
}
