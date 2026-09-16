use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Draws its child and lets every pointer pass straight through it.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::IgnorePointer;
///
/// // A badge drawn over a button, which must not eat the button's taps.
/// let badge = IgnorePointer::new().child(Text::new("beta"));
/// ```
///
/// The child is laid out, painted and announced exactly as it would be
/// otherwise. Only input changes: a click, a drag or a hover over the subtree
/// reaches whatever is *behind* it.
///
/// # Not the same as `Opacity(0.0)` or `Offstage`
///
/// [`Opacity`](crate::Opacity) at zero is invisible and still takes taps, which
/// is the hole-in-the-screen this avoids. [`Offstage`](crate::Offstage) removes
/// the subtree from layout, paint and semantics as well, so it cannot be used
/// for something that has to stay *visible* while staying out of the way.
///
/// # What it is for
///
/// Anything drawn *over* an interface to describe it rather than to be operated:
/// a tooltip, a highlight, a picture of the window, a watermark. The studio's
/// activity-bar tooltip is the case that produced it — the tooltip covered the
/// icon it was explaining, took the hover away from it, and so put itself into a
/// loop of appearing and disappearing at frame rate. See
/// [`RenderIgnorePointer`](https://docs.rs/vieww-render).
#[derive(Debug, Clone)]
pub struct IgnorePointer {
    ignoring: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Default for IgnorePointer {
    fn default() -> Self {
        Self::new()
    }
}

impl IgnorePointer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ignoring: true,
            child: None,
            key: None,
        }
    }

    /// Turn the behaviour off, so the subtree takes input normally again.
    ///
    /// A flag rather than adding and removing the wrapper, so that switching it
    /// is a property change and the child keeps its state.
    #[must_use]
    pub const fn ignoring(mut self, ignoring: bool) -> Self {
        self.ignoring = ignoring;
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
    pub const fn is_ignoring(&self) -> bool {
        self.ignoring
    }
}

impl Widget for IgnorePointer {
    fn debug_name(&self) -> &'static str {
        "IgnorePointer"
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
        vec![("ignoring", self.ignoring.to_string())]
    }
}

widget_node_from!(IgnorePointer);
