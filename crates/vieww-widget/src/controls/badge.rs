use std::fmt;

use vieww_foundation::{Alignment, BoxDecoration, Color, EdgeInsets, Key};

use crate::{
    children, widget_node_from, BuildContext, DecoratedBox, Padding, Positioned, SemanticRole,
    Semantics, SizedBox, Stack, Text, TextStyle, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// What a [`Badge`] shows, kept small on purpose — a badge is a glance, not
/// a second control.
#[derive(Debug, Clone, PartialEq)]
enum BadgeContent {
    /// A plain dot: something happened, no count worth stating.
    Dot,
    /// A count above zero. `Badge::count(0)` clears this back to `None`
    /// rather than ever constructing `Count(0)` — see [`Badge::count`].
    Count(u32),
}

/// A small overlay indicator — a notification dot or a count — pinned past
/// the corner of whatever it wraps.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{icons, Badge};
///
/// let inbox = Badge::new(Icon::new(icons::close())).count(3);
/// let unread = Badge::new(Icon::new(icons::close())).dot();
/// ```
///
/// # Built on [`Positioned`], not a render object of its own
///
/// A badge is a [`Stack`] with the wrapped child as one un-positioned member
/// and the indicator as a [`Positioned`] one, pinned a few pixels past the
/// top-right corner. That is the whole implementation — the same composition
/// an application would write by hand, just named — which is why this lives
/// at this layer rather than needing a `vieww-render` change.
///
/// # `count(0)` shows nothing
///
/// A badge announcing zero of something is not information, it is noise.
/// Use [`dot`](Self::dot) instead of `count(1)` for "something happened, no
/// number attached" — a count whose number carries no meaning is a dot
/// wearing an unnecessary digit.
#[derive(Clone)]
pub struct Badge {
    child: WidgetNode,
    content: Option<BadgeContent>,
    color: Option<Color>,
    key: Option<Key>,
}

impl Badge {
    #[must_use]
    pub fn new(child: impl Into<WidgetNode>) -> Self {
        Self {
            child: child.into(),
            content: None,
            color: None,
            key: None,
        }
    }

    /// Show a count. `0` clears the badge back to nothing; anything above 99
    /// reads as `"99+"` rather than growing the badge to fit an arbitrary
    /// number of digits.
    #[must_use]
    pub fn count(mut self, count: u32) -> Self {
        self.content = (count > 0).then_some(BadgeContent::Count(count));
        self
    }

    /// Show a plain dot: something happened, with no number attached to it.
    #[must_use]
    pub fn dot(mut self) -> Self {
        self.content = Some(BadgeContent::Dot);
        self
    }

    /// Clear back to showing nothing at all.
    #[must_use]
    pub fn none(mut self) -> Self {
        self.content = None;
        self
    }

    /// Override the badge's fill. Defaults to
    /// [`ColorScheme::error`](crate::ColorScheme) — a badge is usually
    /// drawing attention to something, and that role is built for exactly
    /// that.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn is_shown(&self) -> bool {
        self.content.is_some()
    }
}

impl Widget for Badge {
    fn debug_name(&self) -> &'static str {
        "Badge"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let Some(content) = &self.content else {
            // Nothing to show: the wrapped child alone. No Stack, no cost —
            // a `Badge::new(x)` with nothing shown must cost exactly what
            // `x` alone would.
            return self.child.clone();
        };

        let theme = ThemeData::of(ctx);
        let color = self.color.unwrap_or(theme.colors.error);

        let indicator: WidgetNode = match content {
            // **Round, because it is called a dot.** A `ColoredBox` in a square
            // box is a square, and an 8pt square on the corner of an icon reads
            // as a rendering artefact rather than as an indicator — which is
            // what it looked like the first time this widget was put on a
            // screen. `stadium` on a square is a circle, which is the same
            // shape the count badge already uses.
            BadgeContent::Dot => SizedBox::square(8.0)
                .child(DecoratedBox::new(BoxDecoration::filled(color).stadium()))
                .into(),
            BadgeContent::Count(count) => {
                let text = if *count > 99 {
                    "99+".to_owned()
                } else {
                    count.to_string()
                };
                DecoratedBox::new(BoxDecoration::filled(color).stadium())
                    .child(Padding::new(EdgeInsets::symmetric(5.0, 2.0)).child(
                        Text::new(text).style(TextStyle {
                            color: theme.colors.on_error,
                            ..theme.text.label
                        }),
                    ))
                    .into()
            }
        };

        Semantics::new()
            .role(SemanticRole::Group)
            .child(
                Stack::new()
                    .alignment(Alignment::TOP_LEFT)
                    .children(children![
                        self.child.clone(),
                        Positioned::new().top(-4.0).right(-4.0).child(indicator),
                    ]),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        match &self.content {
            Some(BadgeContent::Dot) => vec![("content", "dot".to_owned())],
            Some(BadgeContent::Count(count)) => vec![("content", count.to_string())],
            None => Vec::new(),
        }
    }
}

impl fmt::Debug for Badge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Badge")
            .field("content", &self.content)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Badge);

#[cfg(test)]
mod tests {
    use crate::{icons, inflate, DebugNode, Icon, Theme};

    use super::*;

    fn built(badge: Badge) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(badge))
    }

    #[test]
    fn with_no_content_it_costs_nothing_extra() {
        let node = built(Badge::new(Icon::new(icons::close())));
        assert!(
            node.find("Stack").is_none(),
            "an unshown badge must not add a Stack around its child"
        );
    }

    #[test]
    fn count_zero_is_the_same_as_no_content() {
        assert!(!Badge::new(SizedBox::new()).count(0).is_shown());
        assert!(Badge::new(SizedBox::new()).count(1).is_shown());
    }

    #[test]
    fn a_count_over_99_reads_as_99_plus() {
        let node = built(Badge::new(Icon::new(icons::close())).count(150));
        let text = node.find("Text").expect("the count's Text node");
        assert_eq!(text.property("text"), Some("\"99+\""));
    }

    #[test]
    fn a_dot_has_no_text_at_all() {
        let node = built(Badge::new(Icon::new(icons::close())).dot());
        assert!(node.find("Text").is_none());
        assert!(node.find("Positioned").is_some());
    }

    #[test]
    fn a_dot_is_round() {
        // It was a `ColoredBox`, which is to say a square — and an 8pt square
        // pinned to the corner of an icon reads as a rendering artefact rather
        // than as an indicator. Found by looking at `examples/controls`, where
        // it had sat unremarked since the widget was written.
        let node = built(Badge::new(Icon::new(icons::close())).dot());
        let decorated = node
            .find("DecoratedBox")
            .expect("the dot is a decorated box so that it can carry a radius");
        assert_eq!(
            decorated.property("radius"),
            Some("stadium"),
            "a stadium in a square box is a circle"
        );
    }

    #[test]
    fn none_clears_a_previously_set_count() {
        assert!(!Badge::new(SizedBox::new()).count(5).none().is_shown());
    }
}
