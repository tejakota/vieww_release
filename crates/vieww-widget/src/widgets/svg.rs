use vieww_foundation::{Key, VectorImage};

use crate::{widget_node_from, Widget, WidgetKind};

/// A decoded vector image on the screen — every shape it carries, filled with
/// its own colour.
///
/// ```
/// use vieww_foundation::{Path, Rect, VectorImage, VectorShape};
/// use vieww_widget::prelude::*;
/// use vieww_widget::Svg;
///
/// let logo = VectorImage::new(
///     vec![VectorShape { path: Path::rect(Rect::new(0.0, 0.0, 24.0, 24.0)), color: Color::BLACK }],
///     Rect::new(0.0, 0.0, 24.0, 24.0),
/// );
/// let mark = Svg::new(logo).label("Company logo");
/// ```
///
/// Same split as [`Image`](crate::Image)/`vieww_foundation::Image`: the type
/// an application writes is this widget, and the decoded data is
/// [`VectorImage`], from `vieww-foundation` so the widget layer can name it
/// without depending on the paint layer (`docs/DESIGN.md` §7). Decoding an
/// actual `.svg` file's *text* into one is `vieww-asset`'s `svg` feature —
/// this widget takes the result already parsed, the same reasoning `Image`
/// takes pixels rather than PNG bytes.
///
/// A render leaf. It asks for the viewbox's natural size and takes whatever
/// the constraints allow, the same as [`Icon`](crate::Icon) — a single shared
/// fit transform is what keeps a multi-shape image's parts positioned
/// relative to each other; see [`VectorImage::fitted`].
///
/// # Label it or do not, deliberately
///
/// The same rule `Image` and `Icon` both follow: unlabelled is right for
/// decoration, and wrong for anything carrying meaning of its own. There is
/// no default correct for both, so there is no default at all.
#[derive(Debug, Clone)]
pub struct Svg {
    image: VectorImage,
    label: Option<String>,
    width: Option<f32>,
    height: Option<f32>,
    key: Option<Key>,
}

impl Svg {
    #[must_use]
    pub fn new(image: VectorImage) -> Self {
        Self {
            image,
            label: None,
            width: None,
            height: None,
            key: None,
        }
    }

    /// What a screen reader should call this image. Leave it off for
    /// decoration; see the type's docs.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Ask for this width instead of the viewbox's own.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Ask for this height instead of the viewbox's own.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn data(&self) -> &VectorImage {
        &self.image
    }

    #[must_use]
    pub fn svg_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub const fn svg_width(&self) -> Option<f32> {
        self.width
    }

    #[must_use]
    pub const fn svg_height(&self) -> Option<f32> {
        self.height
    }
}

impl Widget for Svg {
    fn debug_name(&self) -> &'static str {
        "Svg"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("shapes", self.image.shapes().len().to_string())];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        props
    }
}

widget_node_from!(Svg);

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Path, Rect, VectorShape};

    use super::*;

    fn image() -> VectorImage {
        VectorImage::new(
            vec![VectorShape {
                path: Path::rect(Rect::new(0.0, 0.0, 10.0, 10.0)),
                color: Color::BLACK,
            }],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        )
    }

    #[test]
    fn an_svg_is_a_render_leaf() {
        assert!(matches!(Svg::new(image()).kind(), WidgetKind::RenderLeaf));
    }

    #[test]
    fn an_unlabelled_svg_says_nothing_to_a_screen_reader() {
        assert_eq!(Svg::new(image()).svg_label(), None);
        assert_eq!(
            Svg::new(image()).label("A logo").svg_label(),
            Some("A logo")
        );
    }

    #[test]
    fn a_size_is_only_overridden_when_it_is_given() {
        let natural = Svg::new(image());
        assert_eq!((natural.svg_width(), natural.svg_height()), (None, None));

        let fixed = Svg::new(image()).width(48.0);
        assert_eq!((fixed.svg_width(), fixed.svg_height()), (Some(48.0), None));
    }

    #[test]
    fn the_dump_shows_the_shape_count() {
        let props = Svg::new(image()).debug_properties();
        assert!(props.contains(&("shapes", "1".to_owned())));
    }
}
