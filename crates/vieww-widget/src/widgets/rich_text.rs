use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Color, FontWeight, Key, TextAlign, TextDirection, TextStyle};

use crate::{widget_node_from, Handler, Widget, WidgetKind};

/// One run of text in one style, optionally a link.
///
/// The unit [`RichText`] is built from. A span is styled and, if it has a
/// handler, tappable — and because the whole paragraph is shaped as one, a span
/// that wraps across a line break is still one span and still one link.
#[derive(Clone)]
pub struct Span {
    text: String,
    style: Option<TextStyle>,
    /// Applied on top of whatever style is resolved, so a caller can say "bold"
    /// without restating the size and colour of the surrounding paragraph.
    weight: Option<FontWeight>,
    italic: Option<bool>,
    color: Option<Color>,
    size: Option<f32>,
    on_tap: Option<Handler<()>>,
}

impl Span {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            weight: None,
            italic: None,
            color: None,
            size: None,
            on_tap: None,
        }
    }

    /// Replace the whole style rather than adjusting the paragraph's.
    #[must_use]
    pub const fn style(mut self, style: TextStyle) -> Self {
        self.style = Some(style);
        self
    }

    #[must_use]
    pub const fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = Some(weight);
        self
    }

    /// Bold, which is the common case of [`weight`](Self::weight).
    #[must_use]
    pub const fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Make this span a link.
    ///
    /// The pointer becomes a hand over these words and only these words, and a
    /// tap anywhere in them — including on a second line, if the span wrapped —
    /// runs the handler.
    #[must_use]
    pub fn on_tap(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_tap = Some(Rc::new(move |()| handler()));
        self
    }

    /// This span's style, resolved against the paragraph's.
    #[must_use]
    pub fn resolved(&self, base: TextStyle) -> TextStyle {
        let mut style = self.style.unwrap_or(base);
        if let Some(weight) = self.weight {
            style.weight = weight;
        }
        if let Some(italic) = self.italic {
            style.italic = italic;
        }
        if let Some(color) = self.color {
            style.color = color;
        }
        if let Some(size) = self.size {
            style.size = size;
        }
        style
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn handler(&self) -> Option<Handler<()>> {
        self.on_tap.clone()
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Span")
            .field("text", &self.text)
            .field("link", &self.on_tap.is_some())
            .finish_non_exhaustive()
    }
}

impl From<&str> for Span {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for Span {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// A paragraph in more than one style, wrapped as a single run of prose.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{RichText, Span};
///
/// let line = RichText::new(vec![
///     Span::new("Deleting "),
///     Span::new("report.pdf").bold(),
///     Span::new(" cannot be undone. "),
///     Span::new("Learn more").on_tap(|| println!("open the docs")),
/// ]);
/// ```
///
/// # Why this is not several `Text` widgets in a row
///
/// Because prose wraps between **words**, and a [`Flex`](crate::Flex) wraps
/// between children. A bold word in the middle of a long sentence, built as its
/// own `Text` inside a row, either overflows or needs the shaper's line breaking
/// reimplemented outside the shaper. This is one paragraph, shaped once, with
/// the run boundaries falling where the style changes rather than where the
/// widget tree does.
///
/// # What it is built on
///
/// `vieww-text` has always taken a slice of styled spans;
/// [`Text`](crate::Text) passes it a slice of length one. The primitive was
/// there, unexposed, which is why [`Markdown`](crate::Markdown) stripped
/// `**bold**` and discarded link destinations for as long as it did.
#[derive(Debug, Clone)]
pub struct RichText {
    spans: Vec<Span>,
    /// The style spans inherit what they do not override. `None` takes the
    /// theme's body style at build time.
    base: Option<TextStyle>,
    align: TextAlign,
    direction: Option<TextDirection>,
    key: Option<Key>,
}

impl RichText {
    #[must_use]
    pub fn new(spans: impl IntoIterator<Item = Span>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
            base: None,
            align: TextAlign::default(),
            direction: None,
            key: None,
        }
    }

    /// The style every span starts from.
    #[must_use]
    pub const fn style(mut self, style: TextStyle) -> Self {
        self.base = Some(style);
        self
    }

    #[must_use]
    pub const fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub const fn direction(mut self, direction: TextDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The spans, for the render layer.
    #[must_use]
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    #[must_use]
    pub const fn base_style(&self) -> Option<TextStyle> {
        self.base
    }

    #[must_use]
    pub const fn alignment(&self) -> TextAlign {
        self.align
    }

    #[must_use]
    pub const fn base_direction(&self) -> Option<TextDirection> {
        self.direction
    }
}

impl Widget for RichText {
    fn debug_name(&self) -> &'static str {
        "RichText"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "text",
                self.spans.iter().map(Span::text).collect::<String>(),
            ),
            ("spans", self.spans.len().to_string()),
            (
                "links",
                self.spans
                    .iter()
                    .filter(|span| span.handler().is_some())
                    .count()
                    .to_string(),
            ),
        ]
    }
}

widget_node_from!(RichText);
