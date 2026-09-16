use vieww_foundation::{Color, IconData, Key};

use crate::{widget_node_from, Widget, WidgetKind};

/// The size an icon takes when nobody says otherwise.
///
/// 24 logical pixels, matching the grid the built-in set is drawn on, so an
/// unsized icon is drawn at exactly the size it was designed at.
pub const DEFAULT_ICON_SIZE: f32 = 24.0;

/// A filled shape at a size and a colour.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{icons, Icon};
///
/// let tick = Icon::new(icons::check()).size(16.0).color(Color::WHITE);
/// ```
///
/// A render leaf, and square: it takes the size it is given up to its
/// constraints, and the shape is scaled uniformly into that box and centred —
/// so an icon in a box that is not square has room around it rather than being
/// stretched.
///
/// # It does not read the theme
///
/// Deliberately, and for the same reason [`Text`](crate::Text) does not: a leaf
/// has no `build` in which to look one up, and making it composed to gain one
/// would put a second widget in the tree for every icon on screen. The controls
/// in this crate pass a themed colour in explicitly, which is the same thing
/// they do for text.
#[derive(Debug, Clone)]
pub struct Icon {
    icon: IconData,
    size: f32,
    color: Color,
    stroke: Option<f32>,
    label: Option<String>,
    key: Option<Key>,
}

impl Icon {
    #[must_use]
    pub fn new(icon: IconData) -> Self {
        Self {
            icon,
            size: DEFAULT_ICON_SIZE,
            color: Color::BLACK,
            stroke: None,
            label: None,
            key: None,
        }
    }

    /// The side of the square the shape is fitted into.
    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Draw the path as a line of `width` rather than filling its interior.
    ///
    /// # Why an icon set would want this
    ///
    /// The built-in set, like the classic icon sets, is *outlines of shapes*: a tick is a
    /// six-sided polygon, not two strokes with a width. That is the right form
    /// for a filled set and it is the reason a filled set cannot be re-weighted
    /// — the weight is baked into the geometry, so the same icon at 13 points
    /// and at 21 points has proportionally different-looking strokes.
    ///
    /// A *centreline* set is the other tradition — VS Code's Codicons, Lucide,
    /// Feather — where the path is the line the pen travels and the weight is a
    /// number. It reads lighter, it stays even across sizes, and one number
    /// retunes a whole interface. Neither is better; they are different sets,
    /// and a renderer that could only fill could only have one of them.
    ///
    /// # The width is in the icon's own grid
    ///
    /// Not in points. The path is scaled from its viewbox into the box the icon
    /// was given, and the pen is scaled with it — so `stroke(1.6)` on a 24-unit
    /// grid is the same *drawing* at 13 points and at 21, which is the whole
    /// claim of a centreline set. A width in points would have to be restated
    /// at every call site that changes the size, and one of them would forget.
    ///
    /// The rasteriser floors the result at rather less than a point, so an icon
    /// small enough that its pen would fall below a pixel greys out no further.
    #[must_use]
    pub const fn stroke(mut self, width: f32) -> Self {
        self.stroke = Some(width);
        self
    }

    /// The stroke width, if this icon is drawn as a line.
    #[must_use]
    pub const fn stroke_width(&self) -> Option<f32> {
        self.stroke
    }

    /// What a screen reader should call this icon.
    ///
    /// Most icons want **no** label: a tick inside a checkbox, a chevron on a
    /// row that is already announced as a link. Labelling those makes a screen
    /// reader say the same thing twice. Set it only when the icon is the entire
    /// content of a control — a bare close button with no text.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn data(&self) -> &IconData {
        &self.icon
    }

    #[must_use]
    pub const fn icon_size(&self) -> f32 {
        self.size
    }

    #[must_use]
    pub const fn icon_color(&self) -> Color {
        self.color
    }

    #[must_use]
    pub fn icon_label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

impl Widget for Icon {
    fn debug_name(&self) -> &'static str {
        "Icon"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("size", self.size.to_string()),
            ("color", self.color.to_string()),
        ];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        props
    }
}

widget_node_from!(Icon);

#[cfg(test)]
mod tests {
    use crate::icons;

    use super::*;

    #[test]
    fn an_icon_is_drawn_at_the_size_it_was_designed_at_by_default() {
        assert_eq!(Icon::new(icons::check()).icon_size(), DEFAULT_ICON_SIZE);
        assert_eq!(
            Icon::new(icons::check()).data().viewbox().width(),
            DEFAULT_ICON_SIZE,
            "the default size and the built-in grid agree, so an unsized icon \
             is not resampled"
        );
    }

    #[test]
    fn an_unlabelled_icon_says_nothing_to_a_screen_reader() {
        assert_eq!(Icon::new(icons::close()).icon_label(), None);
        assert_eq!(
            Icon::new(icons::close()).label("Dismiss").icon_label(),
            Some("Dismiss")
        );
    }
}
