use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Color, IconData, Key, TextAlign, TextStyle};

use crate::{
    widget_node_from, BuildContext, Button, Center, ColorScheme, CrossAxisAlignment, Flex, Icon,
    MainAxisAlignment, MainAxisSize, SemanticRole, Semantics, SizedBox, Text, ThemeData, Widget,
    WidgetKind, WidgetNode,
};

/// A screen or a section with nothing in it, and what to do about that.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::EmptyState;
///
/// let inbox = EmptyState::new(icons::add(), "No messages yet")
///     .description("New messages will show up here")
///     .action("Compose", || {});
/// ```
///
/// # Why this is a widget and not a convention every screen repeats
///
/// Every list screen in an application eventually needs one — a centred icon,
/// a short title, an optional line of explanation, an optional way out — and
/// without a shared widget it gets rebuilt slightly differently each time:
/// different spacing, different type scale, an accessibility label somebody
/// forgot on the third one. Naming it once means every empty list in an
/// application reads the same way, the way [`Skeleton`](crate::Skeleton) does
/// for loading and [`InlineError`](crate::InlineError) does for failure —
/// the three states a list actually has, other than the data itself.
///
/// # What it is not
///
/// Not a route, not a modal, not something with its own scrim — it is meant to
/// sit exactly where the list's rows would have been, typically inside a
/// [`Center`](crate::Center) or filling a [`Scrollable`](crate::Scrollable)'s
/// viewport. And not an error: a search with no results and an inbox with
/// nothing in it are both this; a request that *failed* is
/// [`InlineError`](crate::InlineError) instead, because "empty" and "broken"
/// read differently to a screen reader and should look different on screen
/// too.
///
/// # Announced as one region, not three separate stops
///
/// [`Semantics::container`](crate::Semantics::container) around the whole
/// thing, labelled with the title — the same shape
/// [`Dialog`](crate::Dialog) uses. A screen reader hears the title once, on
/// arrival, and the description and action stay individually reachable
/// underneath it rather than being swallowed by a default merge, which
/// matters because the action is a real [`Button`](crate::Button) a user has
/// to be able to find and activate.
#[derive(Clone)]
pub struct EmptyState {
    icon: IconData,
    title: String,
    description: Option<String>,
    action: Option<(String, Rc<dyn Fn()>)>,
    key: Option<Key>,
}

impl EmptyState {
    #[must_use]
    pub fn new(icon: IconData, title: impl Into<String>) -> Self {
        Self {
            icon,
            title: title.into(),
            description: None,
            action: None,
            key: None,
        }
    }

    /// A second line under the title, explaining or suggesting what to do.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// A way out — "Compose", "Try again", "Add your first item".
    ///
    /// Giving one adds a real [`Button`](crate::Button) below the description;
    /// leaving it unset is correct for an empty state with nothing useful for
    /// the user to do right now, which is a real and common case (a search
    /// with no results, for one).
    #[must_use]
    pub fn action(mut self, label: impl Into<String>, handler: impl Fn() + 'static) -> Self {
        self.action = Some((label.into(), Rc::new(handler)));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for EmptyState {
    fn debug_name(&self) -> &'static str {
        "EmptyState"
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
        let muted: Color = ColorScheme::dimmed(theme.colors.on_surface);

        let mut column: Vec<WidgetNode> = vec![
            Icon::new(self.icon.clone()).size(48.0).color(muted).into(),
            SizedBox::height(gap).into(),
            Text::new(self.title.clone())
                .style(TextStyle {
                    color: theme.colors.on_surface,
                    ..theme.text.title
                })
                .align(TextAlign::Center)
                .into(),
        ];

        if let Some(description) = &self.description {
            column.push(SizedBox::height(gap / 2.0).into());
            column.push(
                Text::new(description.clone())
                    .style(TextStyle {
                        color: muted,
                        ..theme.text.body
                    })
                    .align(TextAlign::Center)
                    .into(),
            );
        }

        if let Some((label, handler)) = &self.action {
            let handler = Rc::clone(handler);
            column.push(SizedBox::height(gap * 1.5).into());
            column.push(
                Button::new(label.clone())
                    .on_pressed(move || handler())
                    .into(),
            );
        }

        Semantics::container(self.title.clone())
            .role(SemanticRole::Group)
            .child(
                Center::new().child(
                    Flex::column()
                        .main_axis_size(MainAxisSize::Min)
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .children(column),
                ),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("title", self.title.clone()),
            ("action", self.action.is_some().to_string()),
        ]
    }
}

impl fmt::Debug for EmptyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmptyState")
            .field("title", &self.title)
            .field("description", &self.description)
            .field("has_action", &self.action.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(EmptyState);

#[cfg(test)]
mod tests {
    use crate::{icons, inflate, DebugNode, Theme};

    use super::*;

    fn built(state: EmptyState) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(state))
    }

    #[test]
    fn the_title_and_description_are_both_on_screen() {
        let node =
            built(EmptyState::new(icons::add(), "No messages").description("Check back later"));
        let text = node.find_all("Text");
        assert_eq!(text.len(), 2);
        assert_eq!(text[0].property("text"), Some("\"No messages\""));
        assert_eq!(text[1].property("text"), Some("\"Check back later\""));
    }

    #[test]
    fn with_no_description_there_is_only_the_title() {
        let node = built(EmptyState::new(icons::add(), "No messages"));
        assert_eq!(node.find_all("Text").len(), 1);
    }

    #[test]
    fn with_no_action_there_is_no_button() {
        let node = built(EmptyState::new(icons::add(), "No messages"));
        assert!(node.find("Button").is_none());
    }

    #[test]
    fn an_action_adds_a_real_button() {
        let node = built(EmptyState::new(icons::add(), "No messages").action("Compose", || {}));
        assert!(node.find("Button").is_some());
    }

    #[test]
    fn the_whole_thing_is_one_announced_region_and_the_action_stays_reachable() {
        let node = built(EmptyState::new(icons::add(), "No messages").action("Compose", || {}));
        let semantics = node.find("Semantics").expect("a container announcement");
        assert_eq!(semantics.property("label"), Some("No messages"));
        assert_eq!(semantics.property("container"), Some("true"));
        assert!(
            node.find("Button").is_some(),
            "the action must survive being wrapped in the container's semantics"
        );
    }
}
