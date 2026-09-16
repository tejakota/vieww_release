use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Draws its child and hides it from a screen reader.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::ExcludeSemantics;
///
/// # let covered = true;
/// let screen = ExcludeSemantics::new(covered).child(Text::new("behind the top route"));
/// ```
///
/// Transparent in every other respect: the child is laid out at the same size,
/// painted in the same place, and takes taps exactly as it would without this in
/// the way. Only the semantics tree is affected.
///
/// # Not the same as `Offstage`
///
/// [`Offstage`](crate::Offstage) hides a subtree from *everything* — layout,
/// paint, hit testing and screen readers alike. That is the right tool when
/// nobody can see the child and nobody should pay for it.
///
/// This is for the case where those two answers differ: the child **must** keep
/// drawing, and must not keep announcing. [`Navigator`](crate::Navigator) is
/// where that happens. It keeps the screen one below the top onstage on
/// purpose, because that screen is what a push slides over and a pop reveals,
/// and animating against a blank surface would be visibly wrong.
///
/// Left in the semantics tree, though, a covered screen is one a screen-reader
/// user can swipe to and activate without being able to see it — semantic
/// actions are dispatched by id, so the screen on top absorbing taps does not
/// help. Touch is fine; that path is not.
///
/// # Not the same as an empty `Semantics`
///
/// [`Semantics`](crate::Semantics) *adds* a node, or relabels the subtree under
/// one. It has no setting that removes what is below it, and giving it one would
/// mean the widget that describes a control and the widget that suppresses a
/// screen do the same job.
#[derive(Debug, Clone)]
pub struct ExcludeSemantics {
    excluded: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl ExcludeSemantics {
    /// `true` hides the child from a screen reader; `false` is the same as not
    /// being here at all.
    #[must_use]
    pub const fn new(excluded: bool) -> Self {
        Self {
            excluded,
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
    pub const fn is_excluded(&self) -> bool {
        self.excluded
    }
}

impl Widget for ExcludeSemantics {
    fn debug_name(&self) -> &'static str {
        "ExcludeSemantics"
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
        vec![("excluded", self.excluded.to_string())]
    }
}

widget_node_from!(ExcludeSemantics);
