use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Keyboard focus cannot leave this subtree while it is mounted.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::FocusTrap;
///
/// let modal = FocusTrap::new(true).child(Text::new("the dialog's surface"));
/// ```
///
/// [`Dialog`](crate::Dialog), [`BottomSheet`](crate::BottomSheet) and
/// [`Drawer`](crate::Drawer) wrap themselves in one, so an application that uses
/// those gets the behaviour without asking. Wrap something in this directly when
/// building a modal the catalogue does not have.
///
/// Transparent in every other respect: the child is laid out at the same size,
/// painted in the same place, and takes taps exactly as it would without this in
/// the way.
///
/// # What "trapped" means, precisely
///
/// * Tab and Shift+Tab wrap **inside** the subtree rather than walking out into
///   the screen behind.
/// * A press outside the subtree moves no focus — neither into the thing pressed
///   nor to nothing. A press on a scrim should not empty the dialog's keyboard.
/// * Whatever had focus when the trap appeared is restored when it goes.
/// * A focus standing outside the subtree when the trap appears is dropped. A
///   caret still blinking in a field behind a modal is the visible half of the
///   defect this exists to fix.
///
/// It does **not** move focus into the subtree. Which control a dialog opens on
/// is the dialog's decision — a destructive confirmation opens on Cancel, a form
/// opens on its first field — and a guess here would be overridden by every
/// caller that cared.
///
/// # `trapping: false`
///
/// The same widget, doing nothing: for a modal that is conditionally modal, and
/// for a non-modal popover that wants none of this. It is a property rather than
/// a widget that comes and goes, because a reparent would unmount the state
/// inside it — including whatever had focus.
#[derive(Debug, Clone)]
pub struct FocusTrap {
    trapping: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl FocusTrap {
    /// `true` confines focus to the subtree; `false` is the same as not being
    /// here at all.
    #[must_use]
    pub const fn new(trapping: bool) -> Self {
        Self {
            trapping,
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

    /// Whether it is currently trapping — read by the render layer.
    #[must_use]
    pub const fn is_trapping(&self) -> bool {
        self.trapping
    }
}

impl Widget for FocusTrap {
    fn debug_name(&self) -> &'static str {
        "FocusTrap"
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
        vec![("trapping", self.trapping.to_string())]
    }
}

widget_node_from!(FocusTrap);
