use vieww_foundation::{
    brightness_matrix, compose_matrices, grayscale_matrix, saturation_matrix, sepia_matrix,
    tint_matrix, Color, ColorMatrix, ImageFilter, Key,
};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Blurs and recolours everything beneath it, as one group.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Filtered;
///
/// // A panel that reads as frosted glass.
/// let panel = Filtered::blur(8.0)
///     .tint(Color::WHITE, 0.35)
///     .child(Container::new().color(Color::rgba(255, 255, 255, 40)));
/// ```
///
/// # It filters the group, not each shape
///
/// The subtree is rasterised into a target of its own, filtered, and *then*
/// composited. That is what makes a blurred card look like a blurred card:
/// blurring each primitive separately gives soft edges on sharp overlaps, and a
/// card's text would blur away from its own background. The same argument
/// [`Opacity`](crate::Opacity) makes, which is why the two share one layer.
///
/// # `filter` versus `backdrop-filter`
///
/// By default this filters the subtree's own pixels — CSS's plain `filter`.
/// [`Self::with_backdrop`] switches it to CSS's `backdrop-filter`: what gets
/// filtered is a copy of whatever is already painted **behind** this widget
/// at this position (the real destination, read back before this group draws
/// anything of its own), not the subtree's own content. That is the real
/// "frosted glass" — content behind a `vieww-effects::BackdropBlur` (which
/// builds on top of this widget) shows through, blurred, with this widget's
/// own children painted sharp on top of it. See
/// [`vieww_foundation::ImageFilter::backdrop`] for exactly how the two
/// orderings differ and why getting them backwards is the classic bug this
/// distinction exists to name precisely.
///
/// # Cost, honestly
///
/// One offscreen buffer and one kernel pass per filtered layer per frame. That
/// is not free the way [`Opacity`](crate::Opacity) very nearly is — use it for
/// the two or three surfaces a screen actually wants frosted, not as a styling
/// convenience.
///
/// # Backend support
///
/// | backend | blur | colour matrix | how |
/// |---|---|---|---|
/// | CPU (`cpu`) | yes | yes | offscreen rasterise, Rust kernels |
/// | GPU (`gpu`) | yes | yes | segmented compositing, WGSL compute |
///
/// vello 0.9 has no layer-filter API, so the GPU path is this framework's own:
/// the scene is cut at each filtered group, each piece rendered with vello, and
/// the kernels run as compute passes in between. The two backends share no
/// filtering code and are held to each other by
/// `crates/vieww/tests/filter_to_pixels_gpu.rs`, which renders the same tree
/// through both and compares the pixels.
///
/// They agree to within about 2%, not exactly: the CPU approximates a Gaussian
/// with three box blurs and the shader samples a true one. Both are the same
/// Gaussian; a cross-backend snapshot wants a tolerance.
#[derive(Debug, Clone)]
pub struct Filtered {
    filter: ImageFilter,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Filtered {
    /// No filtering — the identity, for building one up conditionally.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            filter: ImageFilter::NONE,
            child: None,
            key: None,
        }
    }

    /// A Gaussian blur of standard deviation `sigma`, in logical pixels.
    ///
    /// σ, not the visible radius: the blur reaches about `3σ`, so CSS's
    /// `blur(24px)` is roughly `Filtered::blur(8.0)`.
    #[must_use]
    pub const fn blur(sigma: f32) -> Self {
        Self {
            filter: ImageFilter::blur(sigma),
            child: None,
            key: None,
        }
    }

    /// Desaturate. `0.0` is fully grey, `1.0` unchanged.
    #[must_use]
    pub fn saturation(self, amount: f32) -> Self {
        self.matrix(saturation_matrix(amount))
    }

    /// Fully desaturate. The standard "this control is unavailable" treatment
    /// that does not lie about the content by fading it.
    #[must_use]
    pub fn grayscale(self) -> Self {
        self.matrix(grayscale_matrix())
    }

    /// Scale every colour channel. `1.0` is unchanged, `0.0` is black.
    #[must_use]
    pub fn brightness(self, amount: f32) -> Self {
        self.matrix(brightness_matrix(amount))
    }

    /// The sepia tone.
    #[must_use]
    pub fn sepia(self) -> Self {
        self.matrix(sepia_matrix())
    }

    /// Wash the group towards `color` by `amount`, keeping its alpha.
    ///
    /// What goes on top of a blur to make frosted glass: a tint rather than an
    /// overlay, so the blurred detail underneath still reads.
    #[must_use]
    pub fn tint(self, color: Color, amount: f32) -> Self {
        self.matrix(tint_matrix(color, amount))
    }

    /// Add an arbitrary 5×4 colour matrix.
    ///
    /// Chained rather than replaced: `.grayscale().brightness(0.8)` composes
    /// into a single matrix, so a chain of five filters still costs one pass.
    #[must_use]
    pub fn matrix(mut self, matrix: ColorMatrix) -> Self {
        self.filter.color_matrix = Some(match self.filter.color_matrix {
            Some(existing) => compose_matrices(&existing, &matrix),
            None => matrix,
        });
        self
    }

    /// Blur, on a filter built some other way.
    #[must_use]
    pub const fn with_blur(mut self, sigma: f32) -> Self {
        self.filter.blur_sigma = sigma;
        self
    }

    /// Switch this filter from filtering the group's own content to
    /// filtering the real backdrop behind it — CSS's `backdrop-filter`. See
    /// this type's own "`filter` versus `backdrop-filter`" doc section.
    #[must_use]
    pub const fn with_backdrop(mut self) -> Self {
        self.filter = self.filter.with_backdrop();
        self
    }

    /// What gets filtered.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The filter this will apply, for tests and tree dumps.
    #[must_use]
    pub const fn filter(&self) -> ImageFilter {
        self.filter
    }
}

impl Default for Filtered {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Filtered {
    fn debug_name(&self) -> &'static str {
        "Filtered"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("blur", format!("{:.1}", self.filter.blur_sigma)),
            (
                "color_matrix",
                self.filter.color_matrix.is_some().to_string(),
            ),
        ]
    }
}

widget_node_from!(Filtered);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_filtered_does_nothing() {
        assert!(Filtered::new().filter().is_noop());
    }

    #[test]
    fn blur_and_colour_can_be_combined_on_one_layer() {
        let frosted = Filtered::blur(6.0).tint(Color::WHITE, 0.4);
        assert_eq!(frosted.filter().blur_sigma, 6.0);
        assert!(frosted.filter().color_matrix.is_some());
    }

    /// A chain has to compose into one matrix, or five filters means five
    /// passes over the pixels for something one pass can do.
    #[test]
    fn chained_colour_filters_compose_into_a_single_matrix() {
        let chained = Filtered::new().grayscale().brightness(0.5);
        let matrix = chained.filter().color_matrix.expect("a matrix");

        let expected = compose_matrices(&grayscale_matrix(), &brightness_matrix(0.5));
        for (a, b) in matrix.iter().zip(&expected) {
            assert!((a - b).abs() < 1e-6, "{matrix:?} vs {expected:?}");
        }
    }

    /// A blur has to grow the layer's bounds or the panel is cut off square at
    /// its own edge — the single most recognisable way a blur is implemented
    /// wrong.
    #[test]
    fn a_blur_reports_the_bounds_it_needs() {
        assert_eq!(Filtered::blur(8.0).filter().bounds_expansion(), 24.0);
        assert_eq!(Filtered::new().grayscale().filter().bounds_expansion(), 0.0);
    }
}
