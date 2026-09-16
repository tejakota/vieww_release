use vieww_foundation::{Color, FontWeight, Key, TextAlign, TextDirection, TextStyle};
use vieww_text::TextOverflow;

use crate::{widget_node_from, Widget, WidgetKind};

/// A run of text.
///
/// A render leaf: it has no children. Its render object asks the text layer for
/// shaping, line breaking and bidirectional ordering.
#[derive(Debug, Clone)]
pub struct Text {
    data: String,
    style: TextStyle,
    align: TextAlign,
    direction: Option<TextDirection>,
    overflow: TextOverflow,
    max_lines: Option<usize>,
    key: Option<Key>,
}

impl Text {
    #[must_use]
    pub fn new(data: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            style: TextStyle::default(),
            align: TextAlign::default(),
            direction: None,
            overflow: TextOverflow::Visible,
            max_lines: None,
            key: None,
        }
    }

    /// Replace the whole style.
    #[must_use]
    pub const fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the text color.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.style.color = color;
        self
    }

    /// Set the font size in logical pixels.
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.style.size = size;
        self
    }

    /// Set the font weight.
    #[must_use]
    pub const fn weight(mut self, weight: FontWeight) -> Self {
        self.style.weight = weight;
        self
    }

    /// Shorthand for [`FontWeight::Bold`].
    #[must_use]
    pub const fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    /// Make the text italic.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.style.italic = true;
        self
    }

    /// How lines sit within the available width.
    ///
    /// Defaults to [`TextAlign::Start`], which follows the text's direction — so
    /// Arabic aligns right without the caller asking.
    #[must_use]
    pub const fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Force a base direction instead of inferring one from the text.
    ///
    /// Only needed when the text cannot decide — it is empty, or holds nothing but
    /// digits and punctuation.
    #[must_use]
    pub const fn direction(mut self, direction: TextDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// What to do with text that will not fit the width it is given.
    ///
    /// [`TextOverflow::Ellipsis`] cuts the end off, [`TextOverflow::EllipsisMiddle`]
    /// the middle — the second is for names, where both ends carry meaning. The
    /// cut is made **against the shaped width**, in the render layer, which is
    /// the only place that knows how wide the string actually is: a caller that
    /// keeps twenty-four characters instead is guessing, and guesses wrong in
    /// both directions depending on the glyphs.
    ///
    /// Give the run a bounded width to elide against — a `Constrained`, a
    /// `Container` with a width, or a `Flexible` slot. With unbounded width
    /// nothing is ever too wide and nothing is ever cut.
    #[must_use]
    pub const fn overflow(mut self, overflow: TextOverflow) -> Self {
        self.overflow = overflow;
        self
    }

    /// How many lines the run may occupy. Only `1` is honoured for elision
    /// today; see `RenderText::max_lines`.
    #[must_use]
    pub const fn max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines);
        self
    }

    /// The overflow behaviour asked for.
    #[must_use]
    pub const fn text_overflow(&self) -> TextOverflow {
        self.overflow
    }

    /// The line limit asked for, if any.
    #[must_use]
    pub const fn line_limit(&self) -> Option<usize> {
        self.max_lines
    }

    /// The alignment lines are laid out with.
    #[must_use]
    pub const fn text_align(&self) -> TextAlign {
        self.align
    }

    /// The forced base direction, if any.
    #[must_use]
    pub const fn text_direction(&self) -> Option<TextDirection> {
        self.direction
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The text content.
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// The style this run is drawn with.
    #[must_use]
    pub const fn text_style(&self) -> &TextStyle {
        &self.style
    }
}

impl Widget for Text {
    fn debug_name(&self) -> &'static str {
        "Text"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("text", format!("{:?}", self.data))];
        if self.style.size != TextStyle::default().size {
            props.push(("size", self.style.size.to_string()));
        }
        if self.style.weight != FontWeight::default() {
            props.push(("weight", format!("{:?}", self.style.weight)));
        }
        if self.style.italic {
            props.push(("italic", "true".to_owned()));
        }
        if self.style.color != Color::BLACK {
            props.push(("color", self.style.color.to_string()));
        }
        props
    }
}

widget_node_from!(Text);
