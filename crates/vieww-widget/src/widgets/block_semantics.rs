use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Hides everything painted *before* it from a screen reader.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::BlockSemantics;
///
/// let scrim = BlockSemantics::new(true).child(Text::new("the dialog's scrim"));
/// ```
///
/// Transparent in every other respect: the child is laid out at the same size,
/// painted in the same place, and takes taps exactly as it would without this in
/// the way. Only the semantics tree is affected, and only the part of it that
/// was already collected when this widget is reached.
///
/// What builds one is [`ModalBarrier`](crate::ModalBarrier), which is the only
/// thing in the framework that covers a screen it is not an ancestor of.
///
/// # Not the same as `ExcludeSemantics`
///
/// [`ExcludeSemantics`](crate::ExcludeSemantics) hides *its own subtree*. That
/// is the right tool when the thing to be silenced is underneath the widget
/// doing the silencing — a covered route, where `Navigator` wraps the screen it
/// is hiding.
///
/// A dialog cannot use it, because a dialog is not an ancestor of the screen it
/// covers. It is a sibling of it in the navigator's stack, painted afterwards,
/// and no node in the tree contains that screen without also containing the
/// dialog. Paint order is the only relationship the two have, so paint order is
/// what this widget names: everything before it goes, everything after it stays.
///
/// That "everything after it stays" is what keeps a dialog readable while it
/// silences the screen behind — the barrier is the first child of the dialog's
/// stack and the dialog's own surface is the second.
#[derive(Debug, Clone)]
pub struct BlockSemantics {
    blocking: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl BlockSemantics {
    /// `true` hides what was painted before it; `false` is the same as not being
    /// here at all.
    #[must_use]
    pub const fn new(blocking: bool) -> Self {
        Self {
            blocking,
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

    /// Whether it is currently blocking — read by the render layer.
    #[must_use]
    pub const fn is_blocking(&self) -> bool {
        self.blocking
    }
}

impl Widget for BlockSemantics {
    fn debug_name(&self) -> &'static str {
        "BlockSemantics"
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
        vec![("blocking", self.blocking.to_string())]
    }
}

widget_node_from!(BlockSemantics);
