use vieww_foundation::{Color, Key};

use crate::{widget_node_from, BuildContext, ColoredBox, SizedBox, Widget, WidgetKind, WidgetNode};

/// The side of the box an [`ErrorPlaceholder`] asks for.
///
/// Not a fill. See the type's own docs for why a placeholder that expanded
/// would turn one caught panic into a layout failure.
pub const ERROR_PLACEHOLDER_SIZE: f32 = 48.0;

/// The fill. Chosen to be impossible to mistake for a designed colour.
pub const ERROR_PLACEHOLDER_COLOR: Color = Color::hex(0xFF00FF);

/// What is mounted in place of a widget whose `build` panicked.
///
/// The element tree substitutes this when it catches a panic out of a build and
/// its `ErrorPolicy` says to carry on. It is deliberately the least capable
/// widget that is still visible: a coloured box of a fixed size, with no child,
/// no handlers and no text.
///
/// The types that decide when this appears live one crate *up*, in
/// `vieww-element`, which is why nothing here links to them: widgets cannot name
/// the element tree that mounts them.
///
/// # Why it does not expand
///
/// The obvious placeholder fills the space the broken widget would have taken.
/// It cannot: the failed widget might have been laid out under *unbounded*
/// constraints — inside a scrollable, or on a flex's cross axis — and
/// [`SizedBox::expand`] there is infinity, which is the one thing layout cannot
/// resolve. A caught panic that then blows up in layout has helped nobody.
///
/// A tight size is safe in both directions: an ancestor with tight constraints
/// overrides it (outer constraints win), and an unbounded one leaves it at
/// [`ERROR_PLACEHOLDER_SIZE`] instead of infinity.
///
/// # Why it does not draw the message
///
/// Drawing text needs a font, and a font store that has not been given faces
/// loads the system's — 33 seconds in a debug build, and the failure mode this
/// widget exists to make survivable is a debug-build failure. The message
/// travels through `ElementTree::build_errors` and through
/// [`Widget::debug_properties`], which is where a tree dump will show it, and
/// neither costs a glyph.
#[derive(Debug, Clone)]
pub struct ErrorPlaceholder {
    message: String,
    widget_name: &'static str,
    key: Option<Key>,
}

impl ErrorPlaceholder {
    /// A placeholder standing in for `widget_name`, whose build panicked with
    /// `message`.
    #[must_use]
    pub fn new(widget_name: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            widget_name,
            key: None,
        }
    }

    /// The panic message the failed build produced.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The `debug_name` of the widget this stands in for.
    #[must_use]
    pub const fn widget_name(&self) -> &'static str {
        self.widget_name
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for ErrorPlaceholder {
    fn debug_name(&self) -> &'static str {
        "ErrorPlaceholder"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        ColoredBox::new(ERROR_PLACEHOLDER_COLOR)
            .child(SizedBox::square(ERROR_PLACEHOLDER_SIZE))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("failed", self.widget_name.to_owned()),
            ("message", self.message.clone()),
        ]
    }

    /// This *is* the substitute, so catching a panic out of it would mount
    /// another one and recurse — see [`Widget::catches_panics`].
    fn catches_panics(&self) -> bool {
        false
    }
}

widget_node_from!(ErrorPlaceholder);
