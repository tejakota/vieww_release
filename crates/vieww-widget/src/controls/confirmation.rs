use std::fmt;

use vieww_foundation::{IconData, Key, TextStyle};

use crate::{
    icons, widget_node_from, BuildContext, CrossAxisAlignment, Flex, Icon, MainAxisSize,
    SemanticRole, Semantics, SizedBox, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// A row saying something worked, in place.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Confirmation;
///
/// let saved = Confirmation::new("Changes saved");
/// ```
///
/// # The mirror of `InlineError`, deliberately
///
/// [`InlineError`](crate::InlineError) is a failure sitting where its content
/// would have been; this is the same shape for a success. They are built the
/// same way and announced the same way, and that symmetry is the point — an
/// application that shows failures in place and successes in a snackbar teaches
/// people that only bad news appears where they were looking.
///
/// [`Snackbar`](crate::Snackbar) remains the right answer for a success that
/// happened *somewhere else* and is passing through; this is for one that
/// happened right here.
///
/// # Colour is never the only signal
///
/// The tick is as load-bearing as the green, and the message says what
/// succeeded in words. Red-green is the most common form of colour blindness
/// and it is exactly the pair a success/failure convention reaches for first —
/// so a confirmation that differed from an error *only* in hue would be
/// indistinguishable from it for a substantial number of people. The icon
/// differs, the words differ, and the colour is the third signal rather than
/// the first. `ColorScheme::success` says the same thing in its own docs.
///
/// # Announced politely, where an error interrupts
///
/// [`SemanticRole::Custom("status")`](crate::SemanticRole::Custom), which
/// `Liveness::for_role` makes **polite** — so it is spoken at the next natural
/// pause rather than cutting across whatever is being read. That difference
/// from `InlineError`'s `alert` is the whole judgement: a failure has to
/// interrupt because the user is about to act on a wrong belief, and a success
/// confirms a belief they already hold. Interrupting somebody to agree with
/// them is the fastest way to have announcements turned off.
///
/// Nothing here opts in to that — the role carries it, which is `docs/AIMS.md`
/// §J's rule that announcement is part of what a transient widget *is*.
#[derive(Clone)]
pub struct Confirmation {
    icon: IconData,
    message: String,
    key: Option<Key>,
}

impl Confirmation {
    /// A confirmation with the standard tick.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            icon: icons::check(),
            message: message.into(),
            key: None,
        }
    }

    /// Use a different icon — but keep one. See the type's docs on colour.
    #[must_use]
    pub fn icon(mut self, icon: IconData) -> Self {
        self.icon = icon;
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// What it says.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Widget for Confirmation {
    fn debug_name(&self) -> &'static str {
        "Confirmation"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let success = theme.colors.success;

        Semantics::container(self.message.clone())
            .role(SemanticRole::Custom("status"))
            .child(
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(vec![
                        Icon::new(self.icon.clone())
                            .size(20.0)
                            .color(success)
                            .into(),
                        SizedBox::width(theme.metrics.gap).into(),
                        Text::new(self.message.clone())
                            .style(TextStyle {
                                color: success,
                                ..theme.text.body
                            })
                            .into(),
                    ]),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("message", self.message.clone())]
    }
}

impl fmt::Debug for Confirmation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Confirmation")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Confirmation);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::debug_tree;

    #[test]
    fn it_is_announced_politely_rather_than_interrupting() {
        // **The judgement this widget exists to encode.** A failure interrupts
        // because the user is about to act on a wrong belief; a success confirms
        // one they already hold. `status` is polite by `Liveness::for_role`,
        // where `InlineError`'s `alert` is assertive.
        let tree = debug_tree(Confirmation::new("Changes saved"));
        assert!(
            tree.contains("status"),
            "a confirmation must carry the status role: {tree}"
        );
        assert!(
            !tree.contains("alert"),
            "a success that interrupts is a success nobody keeps switched on: {tree}"
        );
    }

    #[test]
    fn the_message_is_the_announcement() {
        // Derived from the tree rather than posted, so it happens on every path
        // that mounts this widget — §J's mechanism, not an extra call.
        let tree = debug_tree(Confirmation::new("Changes saved"));
        assert!(tree.contains("Changes saved"), "{tree}");
    }

    #[test]
    fn it_carries_an_icon_so_colour_is_not_the_only_signal() {
        // Red-green is the most common form of colour blindness and is exactly
        // the pair this convention reaches for. The tick has to be there.
        let tree = debug_tree(Confirmation::new("Saved"));
        assert!(tree.contains("Icon"), "{tree}");
    }
}
