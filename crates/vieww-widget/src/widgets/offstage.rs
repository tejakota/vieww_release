use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Keeps its child in the tree and takes it off the screen.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Offstage;
///
/// # let covered = true;
/// let page = Offstage::new(covered).child(Text::new("the screen underneath"));
/// ```
///
/// While offstage the child is not laid out, painted, hit tested or read out by
/// a screen reader — but it stays **mounted**, so its state, its scroll position
/// and its animations are exactly where they were when it comes back.
///
/// # Not the same as `Opacity(0.0)`
///
/// A fully faded child still occupies its space, still costs layout, and still
/// takes taps. That is deliberate there and wrong here: this is for something
/// genuinely out of the picture, like a route underneath an opaque one.
///
/// # Not the same as leaving it out of the tree
///
/// Removing the subtree unmounts it, and going back rebuilds it from nothing —
/// which is why so many applications lose your place in a list when you navigate
/// back to it. Keeping the state is the whole point; skipping the work is what
/// this adds.
#[derive(Debug, Clone)]
pub struct Offstage {
    offstage: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Offstage {
    /// `true` hides the child; `false` is the same as not being here at all.
    #[must_use]
    pub const fn new(offstage: bool) -> Self {
        Self {
            offstage,
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

    /// Whether the child is currently hidden — read by the render layer.
    #[must_use]
    pub const fn is_offstage(&self) -> bool {
        self.offstage
    }
}

impl Widget for Offstage {
    fn debug_name(&self) -> &'static str {
        "Offstage"
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
        vec![("offstage", self.offstage.to_string())]
    }
}

widget_node_from!(Offstage);
