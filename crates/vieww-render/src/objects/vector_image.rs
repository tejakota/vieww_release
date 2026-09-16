use vieww_foundation::{Constraints, Offset, Size, VectorImage};

use crate::{LayoutCtx, PaintCtx, RenderObject, Role, Semantics};

/// Fills every shape of a [`VectorImage`], scaled together into its box.
///
/// The layout half of this is [`RenderIcon`](crate::RenderIcon)'s: request the
/// natural size (the viewbox, unless overridden), take whatever the
/// constraints allow of it, and let the *shapes* be fitted uniformly and
/// centred rather than stretched. The difference is entirely in `paint` —
/// [`VectorImage::fitted`] applies one shared transform to every shape so a
/// multi-shape image's parts stay positioned relative to each other, where
/// `IconData::fitted` (right for one path) would re-centre each independently.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderVectorImage {
    pub image: VectorImage,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub label: Option<String>,
}

impl RenderVectorImage {
    #[must_use]
    pub fn new(image: VectorImage) -> Self {
        Self {
            image,
            width: None,
            height: None,
            label: None,
        }
    }

    fn requested(&self) -> Size {
        let natural = self.image.viewbox().size();
        Size::new(
            self.width.unwrap_or(natural.width),
            self.height.unwrap_or(natural.height),
        )
    }
}

impl RenderObject for RenderVectorImage {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(self.requested())
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }
        for shape in self.image.fitted(bounds) {
            if !shape.color.is_transparent() {
                ctx.canvas().fill_path(&shape.path, shape.color.into());
            }
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The box, the same reasoning `RenderImage` gives — a tap on a
        // transparent corner of an illustration is still a tap on the
        // illustration's bounding box, not a hole to fall through.
        true
    }

    fn semantics(&self) -> Option<Semantics> {
        self.label
            .as_ref()
            .map(|label| Semantics::new(Role::Label).with_label(label.clone()))
    }

    /// The viewbox, or whatever was asked for instead of it.
    ///
    /// The same shape as [`RenderImage`](crate::RenderImage)'s and for the same
    /// reason: `requested()` is a pure function of the fields, `layout` is
    /// nothing but that value put through the constraints, so answering with it
    /// keeps the intrinsic and a real loose layout in agreement by construction.
    ///
    /// # The shapes are not measured, the viewbox is
    ///
    /// It would be possible to union the bounds of every path and report that,
    /// and it would be wrong. A viewbox is an author's statement about how much
    /// room the drawing occupies — the padding around a glyph in an icon set is
    /// part of the drawing, and a logo measured to its ink would sit tighter
    /// than its siblings in a row of logos. `paint` fits the shapes into the
    /// viewbox for the same reason, so measuring the ink here would also mean
    /// the intrinsic and the paint disagreed about what the image is.
    ///
    /// Minimum and maximum coincide, and `cross` is ignored: vector shapes do
    /// not reflow, so there is no width at which the drawing would like to be a
    /// different height.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        let requested = self.requested();
        Some(match query.axis {
            vieww_foundation::Axis::Horizontal => requested.width,
            vieww_foundation::Axis::Vertical => requested.height,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>()
            .is_none_or(|other| self.requested() != other.requested())
    }

    fn debug_name(&self) -> &'static str {
        "RenderVectorImage"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Path, Rect, VectorShape};
    use vieww_paint::{Command, Scene};

    use super::*;
    use crate::RenderTree;

    fn image(viewbox: Rect, shapes: Vec<VectorShape>) -> VectorImage {
        VectorImage::new(shapes, viewbox)
    }

    fn shape(rect: Rect, color: Color) -> VectorShape {
        VectorShape {
            path: Path::rect(rect),
            color,
        }
    }

    fn painted(object: &RenderVectorImage, size: Size) -> Scene {
        let mut scene = Scene::new();
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin: Offset::ZERO,
            size,
            dpr: 1.0,
        };
        object.paint(&mut ctx);
        scene
    }

    fn laid_out(object: RenderVectorImage, constraints: Constraints) -> Size {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.layout(id, constraints)
    }

    #[test]
    fn it_asks_for_its_viewbox_size_by_default() {
        let img = image(Rect::new(0.0, 0.0, 24.0, 24.0), Vec::new());
        assert_eq!(
            laid_out(RenderVectorImage::new(img), Constraints::UNBOUNDED),
            Size::square(24.0)
        );
    }

    #[test]
    fn an_explicit_size_overrides_one_axis_and_not_the_other() {
        let img = image(Rect::new(0.0, 0.0, 24.0, 24.0), Vec::new());
        let mut object = RenderVectorImage::new(img);
        object.width = Some(100.0);
        assert_eq!(
            laid_out(object, Constraints::UNBOUNDED),
            Size::new(100.0, 24.0)
        );
    }

    #[test]
    fn every_shape_paints_and_transparent_ones_do_not() {
        let img = image(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![
                shape(Rect::new(0.0, 0.0, 5.0, 5.0), Color::rgb(200, 0, 0)),
                shape(Rect::new(5.0, 5.0, 10.0, 10.0), Color::TRANSPARENT),
            ],
        );
        let scene = painted(&RenderVectorImage::new(img), Size::square(10.0));
        let fills = scene
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::FillPath { .. }))
            .count();
        assert_eq!(fills, 1, "the transparent shape paints nothing");
    }

    #[test]
    fn a_zero_sized_box_draws_nothing() {
        let img = image(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            vec![shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK)],
        );
        assert!(painted(&RenderVectorImage::new(img), Size::ZERO).is_empty());
    }

    #[test]
    fn only_a_named_image_reaches_a_screen_reader() {
        let img = image(Rect::new(0.0, 0.0, 10.0, 10.0), Vec::new());
        let mut object = RenderVectorImage::new(img);
        assert!(object.semantics().is_none());
        object.label = Some("A logo".to_owned());
        assert_eq!(
            object.semantics().and_then(|s| s.label),
            Some("A logo".to_owned())
        );
    }

    #[test]
    fn changing_shapes_without_changing_size_does_not_relayout() {
        let viewbox = Rect::new(0.0, 0.0, 10.0, 10.0);
        let a = RenderVectorImage::new(image(
            viewbox,
            vec![shape(Rect::new(0.0, 0.0, 5.0, 5.0), Color::BLACK)],
        ));
        let b = RenderVectorImage::new(image(
            viewbox,
            vec![shape(Rect::new(0.0, 0.0, 9.0, 9.0), Color::WHITE)],
        ));
        assert!(!a.layout_differs(&b));
    }

    // -------------------------------------------------------------- intrinsics

    /// Ask a mounted image a question without laying it out — which is the state
    /// the callers that need intrinsics ask from.
    fn intrinsic(object: RenderVectorImage, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.intrinsic(id, query)
    }

    fn logo() -> RenderVectorImage {
        RenderVectorImage::new(image(
            Rect::new(0.0, 0.0, 32.0, 16.0),
            vec![shape(Rect::new(2.0, 2.0, 8.0, 8.0), Color::BLACK)],
        ))
    }

    #[test]
    fn a_vector_image_reports_its_viewbox_on_both_axes() {
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_width()),
            Some(32.0)
        );
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_height()),
            Some(16.0)
        );
    }

    #[test]
    fn shapes_smaller_than_the_viewbox_do_not_shrink_the_intrinsic() {
        // The single shape here occupies a 6x6 corner of a 32x16 viewbox. An
        // intrinsic that measured the ink would report six, and a row of logos
        // would then space them by how much of their box each happened to fill.
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_width()),
            Some(32.0)
        );
    }

    #[test]
    fn a_vector_image_has_no_slack_so_its_minimum_equals_its_maximum() {
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::min_width()),
            intrinsic(logo(), crate::IntrinsicQuery::max_width())
        );
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::min_height()),
            intrinsic(logo(), crate::IntrinsicQuery::max_height())
        );
    }

    #[test]
    fn an_explicit_size_is_what_the_intrinsic_reports() {
        let mut object = logo();
        object.height = Some(64.0);
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::max_height()),
            Some(64.0)
        );
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_width()),
            Some(32.0),
            "the axis that was not overridden still comes from the viewbox"
        );
    }

    #[test]
    fn a_cross_extent_changes_nothing_because_shapes_do_not_reflow() {
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_height().across(4.0)),
            Some(16.0)
        );
    }

    #[test]
    fn the_intrinsic_is_the_size_a_loose_layout_arrives_at() {
        let laid = laid_out(logo(), Constraints::UNBOUNDED);
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_width()),
            Some(laid.width)
        );
        assert_eq!(
            intrinsic(logo(), crate::IntrinsicQuery::max_height()),
            Some(laid.height)
        );
    }
}
