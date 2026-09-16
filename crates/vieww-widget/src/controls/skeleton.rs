use vieww_foundation::{BoxDecoration, Size};

use crate::{
    widget_node_from, BuildContext, ColoredBox, DecoratedBox, ExcludeSemantics, SizedBox,
    ThemeData, Widget, WidgetKind, WidgetNode,
};

/// A shaped placeholder shown while real content is loading.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Skeleton;
///
/// let line = Skeleton::text();
/// let thumbnail = Skeleton::new().width(64.0).height(64.0).corner(8.0);
/// let avatar_shape = Skeleton::circle(40.0);
/// ```
///
/// # A static fill, not a shimmer — stated rather than hidden
///
/// The animated sweep most skeleton implementations have is a real,
/// separate piece of work — a looping (rather than target-seeking) animation
/// primitive that does not exist in `vieww-animation` yet. Shipping a static
/// placeholder now and a shimmer once that primitive exists is the honest
/// order; shipping a *fake* shimmer built by fighting
/// [`Animated`](crate::Animated) into looping would be the wrong kind of
/// finished. See `docs/AIMS.md` §J.
///
/// # Invisible to a screen reader
///
/// A skeleton names no content — there is nothing behind it yet worth
/// describing, and a screen reader announcing an empty grey box would be
/// pure noise. Wrap the *real* content once it arrives; nothing here needs
/// undoing when that happens; the skeleton is simply replaced.
#[derive(Debug, Clone)]
pub struct Skeleton {
    width: Option<f32>,
    height: Option<f32>,
    corner: f32,
}

/// The height a single line of placeholder text takes, in logical pixels —
/// close to a body-text line's own height, so a block of `Skeleton::text()`
/// rows reads as text-shaped rather than as an arbitrary stack of bars.
pub const TEXT_LINE_HEIGHT: f32 = 14.0;

impl Skeleton {
    /// An unsized placeholder — give it a size with
    /// [`width`](Self::width)/[`height`](Self::height), or a fixed size from
    /// whatever it is placed inside.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            width: None,
            height: None,
            corner: 4.0,
        }
    }

    /// A placeholder shaped like one line of body text.
    #[must_use]
    pub const fn text() -> Self {
        Self::new()
            .height(TEXT_LINE_HEIGHT)
            .corner(TEXT_LINE_HEIGHT / 2.0)
    }

    /// A circular placeholder of the given diameter — an avatar's shape
    /// before the avatar has loaded.
    #[must_use]
    pub const fn circle(diameter: f32) -> Self {
        Self::new()
            .width(diameter)
            .height(diameter)
            .corner(f32::MAX)
    }

    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    #[must_use]
    pub const fn corner(mut self, corner: f32) -> Self {
        self.corner = corner;
        self
    }
}

impl Default for Skeleton {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Skeleton {
    fn debug_name(&self) -> &'static str {
        "Skeleton"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let sized: WidgetNode = match (self.width, self.height) {
            (Some(w), Some(h)) => SizedBox::from_size(Size::new(w, h)).into(),
            (Some(w), None) => SizedBox::width(w).into(),
            (None, Some(h)) => SizedBox::height(h).into(),
            (None, None) => ColoredBox::new(theme.colors.surface_variant).into(),
        };
        let box_node: WidgetNode = DecoratedBox::new(
            BoxDecoration::filled(theme.colors.surface_variant).radius(self.corner),
        )
        .child(sized)
        .into();

        ExcludeSemantics::new(true).child(box_node).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = Vec::new();
        if let Some(w) = self.width {
            props.push(("width", w.to_string()));
        }
        if let Some(h) = self.height {
            props.push(("height", h.to_string()));
        }
        props
    }
}

widget_node_from!(Skeleton);

#[cfg(test)]
mod tests {
    use crate::{inflate, DebugNode, Theme, ThemeData as TD};

    use super::*;

    fn built(skeleton: Skeleton) -> DebugNode {
        inflate(Theme::new(TD::light()).child(skeleton))
    }

    #[test]
    fn text_is_shaped_like_a_line_and_circle_is_a_stadium() {
        assert_eq!(Skeleton::text().height, Some(TEXT_LINE_HEIGHT));
        assert_eq!(Skeleton::circle(40.0).width, Some(40.0));
        assert_eq!(Skeleton::circle(40.0).height, Some(40.0));
        assert_eq!(Skeleton::circle(40.0).corner, f32::MAX);
    }

    #[test]
    fn it_is_excluded_from_the_semantics_tree() {
        let node = built(Skeleton::text());
        assert!(node.find("ExcludeSemantics").is_some());
    }
}
