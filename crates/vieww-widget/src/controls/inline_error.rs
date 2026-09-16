use std::fmt;
use std::rc::Rc;

use vieww_foundation::{IconData, Key, TextStyle};

use crate::{
    widget_node_from, BuildContext, Button, ButtonStyle, CrossAxisAlignment, Flex, Icon,
    MainAxisSize, SemanticRole, Semantics, SizedBox, Text, ThemeData, Widget, WidgetKind,
    WidgetNode,
};

/// A row saying something did not work, in place, with a way to try again.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::InlineError;
///
/// let banner = InlineError::new(icons::close(), "Couldn't load messages")
///     .retry(|| {});
/// ```
///
/// # Not `FormField`'s error, and not `Snackbar`'s either
///
/// [`FormField`](crate::FormField) already renders a per-field validation
/// message under the one control it belongs to — that gap was never real, see
/// `forms.rs`'s own doc. This is the other shape: a whole section or screen
/// that failed to load — a network request, a query, a sync — with nothing
/// useful to show in its place. [`Snackbar`](crate::Snackbar) is for the third
/// shape, a passing notice about something that already happened elsewhere;
/// this is for a failure sitting exactly where its content would have been,
/// same as [`EmptyState`](crate::EmptyState) is for "nothing here" rather than
/// "this broke".
///
/// # Announced as an alert, the same reasoning as `Snackbar`
///
/// Wrapped in [`SemanticRole::Custom("alert")`](crate::SemanticRole::Custom),
/// which the platform layer maps to AccessKit's `Role::Alert` — an implicit
/// live region, announced the moment it appears with no focus required. A
/// failed request is exactly the kind of thing a screen reader user needs to
/// hear about without having had to be looking at the right part of the
/// screen when it happened, the same argument that motivated giving
/// `Snackbar` a route to this role in the first place.
///
/// [`Semantics::container`](crate::Semantics::container), not the default
/// merge, for the same reason `Snackbar` uses it: the retry
/// [`Button`](crate::Button) has to stay individually reachable rather than
/// being swallowed into the announcement.
#[derive(Clone)]
pub struct InlineError {
    icon: IconData,
    message: String,
    retry: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl InlineError {
    #[must_use]
    pub fn new(icon: IconData, message: impl Into<String>) -> Self {
        Self {
            icon,
            message: message.into(),
            retry: None,
            key: None,
        }
    }

    /// Giving a handler adds a "Retry" [`Button`](crate::Button) at the end of
    /// the row.
    #[must_use]
    pub fn retry(mut self, handler: impl Fn() + 'static) -> Self {
        self.retry = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for InlineError {
    fn debug_name(&self) -> &'static str {
        "InlineError"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let gap = theme.metrics.gap;
        let error = theme.colors.error;

        let message = Text::new(self.message.clone()).style(TextStyle {
            color: error,
            ..theme.text.body
        });

        let mut row: Vec<WidgetNode> = vec![
            Icon::new(self.icon.clone()).size(20.0).color(error).into(),
            SizedBox::width(gap).into(),
            message.into(),
        ];

        if let Some(handler) = &self.retry {
            let handler = Rc::clone(handler);
            row.push(SizedBox::width(gap).into());
            row.push(
                Button::new("Retry")
                    .style(ButtonStyle::Text)
                    .on_pressed(move || handler())
                    .into(),
            );
        }

        Semantics::container(self.message.clone())
            .role(SemanticRole::Custom("alert"))
            .child(
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .children(row),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("message", self.message.clone()),
            ("retry", self.retry.is_some().to_string()),
        ]
    }
}

impl fmt::Debug for InlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InlineError")
            .field("message", &self.message)
            .field("has_retry", &self.retry.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(InlineError);

#[cfg(test)]
mod tests {
    use crate::{icons, inflate, DebugNode, Theme};

    use super::*;

    fn built(error: InlineError) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(error))
    }

    #[test]
    fn the_message_is_on_screen() {
        let node = built(InlineError::new(icons::close(), "Couldn't load messages"));
        let text = node.find("Text").expect("the error message");
        assert_eq!(text.property("text"), Some("\"Couldn't load messages\""));
    }

    #[test]
    fn with_no_retry_there_is_no_button() {
        let node = built(InlineError::new(icons::close(), "Failed"));
        assert!(node.find("Button").is_none());
    }

    #[test]
    fn a_retry_handler_adds_a_real_button() {
        let node = built(InlineError::new(icons::close(), "Failed").retry(|| {}));
        assert!(node.find("Button").is_some());
    }

    #[test]
    fn it_announces_itself_as_an_alert_and_leaves_retry_reachable() {
        let node = built(InlineError::new(icons::close(), "Failed").retry(|| {}));
        let semantics = node.find("Semantics").expect("an announcement");
        assert_eq!(semantics.property("role"), Some("Custom(\"alert\")"));
        assert_eq!(semantics.property("label"), Some("Failed"));
        assert!(
            node.find("Button").is_some(),
            "the retry button must survive being wrapped in the alert's semantics"
        );
    }
}
