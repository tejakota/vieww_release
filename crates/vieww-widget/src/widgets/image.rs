use vieww_foundation::{Alignment, BoxFit, Image as ImageData, Key};

use crate::{widget_node_from, Widget, WidgetKind};

/// Decoded pixels on the screen.
///
/// ```
/// use vieww_foundation::Image as Pixels;
/// use vieww_widget::prelude::*;
///
/// // Four transparent pixels, standing in for something decoded from a file.
/// let avatar = Image::new(Pixels::from_rgba8(vec![0; 16], 2, 2))
///     .fit(BoxFit::Cover)
///     .label("Your photo");
/// ```
///
/// The widget is `Image` and the pixels are `vieww_foundation::Image`. Same
/// split as the classic `Image` and pixel-buffer pair, and for the same reason: the name
/// an application writes hundreds of times should be the widget. Only the
/// widget is in the prelude, so the two never collide by accident.
///
/// A render leaf. It asks for the image's natural size and takes what the
/// constraints allow; how the picture sits inside the box it ends up with is
/// [`BoxFit`], applied at paint time so that cropping never moves a parent.
///
/// # It does not decode anything
///
/// [`ImageData`] is tightly packed RGBA8 and this widget takes it already
/// decoded. Decoding belongs to the application: the formats worth supporting,
/// whether to cache, whether to decode off the main thread and what to show
/// while that happens are all questions with no framework-wide right answer,
/// and a `png` dependency in the widget layer would answer the first one for
/// everybody.
///
/// # Label it or do not, deliberately
///
/// An unlabelled image is invisible to a screen reader, which is right for
/// decoration next to text that already says the same thing, and wrong for an
/// image carrying meaning of its own. The same rule [`Icon`](crate::Icon)
/// follows, and there is no default that is correct for both.
#[derive(Debug, Clone)]
pub struct Image {
    image: ImageData,
    fit: BoxFit,
    alignment: Alignment,
    label: Option<String>,
    width: Option<f32>,
    height: Option<f32>,
    key: Option<Key>,
}

impl Image {
    #[must_use]
    pub fn new(image: ImageData) -> Self {
        Self {
            image,
            fit: BoxFit::default(),
            alignment: Alignment::CENTER,
            label: None,
            width: None,
            height: None,
            key: None,
        }
    }

    /// How the picture is fitted into the box. Defaults to [`BoxFit::Contain`].
    #[must_use]
    pub const fn fit(mut self, fit: BoxFit) -> Self {
        self.fit = fit;
        self
    }

    /// Where the picture sits when it does not fill the box.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// What a screen reader should call this image.
    ///
    /// Leave it off for decoration; see the type's docs.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Ask for this width instead of the image's own.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Ask for this height instead of the image's own.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
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
    pub const fn data(&self) -> &ImageData {
        &self.image
    }

    #[must_use]
    pub const fn box_fit(&self) -> BoxFit {
        self.fit
    }

    #[must_use]
    pub const fn image_alignment(&self) -> Alignment {
        self.alignment
    }

    #[must_use]
    pub fn image_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub const fn image_width(&self) -> Option<f32> {
        self.width
    }

    #[must_use]
    pub const fn image_height(&self) -> Option<f32> {
        self.height
    }
}

impl Widget for Image {
    fn debug_name(&self) -> &'static str {
        "Image"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            (
                "pixels",
                format!("{}x{}", self.image.width(), self.image.height()),
            ),
            ("fit", format!("{:?}", self.fit)),
        ];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        props
    }
}

widget_node_from!(Image);

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32) -> ImageData {
        ImageData::from_rgba8(
            vec![0; (width as usize) * (height as usize) * 4],
            width,
            height,
        )
    }

    #[test]
    fn an_image_is_a_leaf_and_contains_by_default() {
        let widget = Image::new(pixels(4, 4));
        assert!(matches!(widget.kind(), WidgetKind::RenderLeaf));
        assert_eq!(
            widget.box_fit(),
            BoxFit::Contain,
            "the fit that never crops and never distorts is the safe default"
        );
    }

    #[test]
    fn an_unlabelled_image_says_nothing_to_a_screen_reader() {
        assert_eq!(Image::new(pixels(4, 4)).image_label(), None);
        assert_eq!(
            Image::new(pixels(4, 4)).label("A cat").image_label(),
            Some("A cat")
        );
    }

    #[test]
    fn a_size_is_only_overridden_when_it_is_given() {
        let natural = Image::new(pixels(4, 4));
        assert_eq!(
            (natural.image_width(), natural.image_height()),
            (None, None)
        );

        let fixed = Image::new(pixels(4, 4)).width(64.0);
        assert_eq!(
            (fixed.image_width(), fixed.image_height()),
            (Some(64.0), None)
        );
    }

    #[test]
    fn the_dump_shows_the_pixels_and_the_fit_rather_than_the_bytes() {
        let props = Image::new(pixels(64, 32))
            .fit(BoxFit::Cover)
            .debug_properties();
        assert!(props.contains(&("pixels", String::from("64x32"))));
        assert!(props.contains(&("fit", String::from("Cover"))));
    }
}
