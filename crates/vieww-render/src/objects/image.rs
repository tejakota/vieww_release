use vieww_foundation::{Alignment, BoxFit, Constraints, Image, Offset, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject, Role, Semantics};

/// Draws decoded pixels, fitted into whatever box layout gave it.
///
/// # Sizing
///
/// It asks for the image's natural size and takes whatever the constraints
/// allow of that — so an image in a tight box is the size of the box, and one in
/// a loose box is its own size. What happens to the *picture* inside that box is
/// [`BoxFit`]'s business, not layout's, and the two are deliberately separate:
/// an image that changed the layout depending on how it was cropped would make
/// every parent's size depend on a paint-time decision.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderImage {
    pub image: Image,
    pub fit: BoxFit,
    pub alignment: Alignment,
    /// What a screen reader should call it. `None` for decoration.
    pub label: Option<String>,
    /// Overrides the natural width when laying out.
    pub width: Option<f32>,
    /// Overrides the natural height when laying out.
    pub height: Option<f32>,
}

impl RenderImage {
    #[must_use]
    pub fn new(image: Image) -> Self {
        Self {
            image,
            fit: BoxFit::default(),
            alignment: Alignment::CENTER,
            label: None,
            width: None,
            height: None,
        }
    }

    /// The size this asks for before constraints are applied.
    fn requested(&self) -> Size {
        let natural = self.image.size();
        Size::new(
            self.width.unwrap_or(natural.width),
            self.height.unwrap_or(natural.height),
        )
    }
}

impl RenderObject for RenderImage {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(self.requested())
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return;
        }

        let source = self.image.size();
        let drawn = self.fit.apply(source, bounds, self.alignment);
        if drawn.width() <= 0.0 || drawn.height() <= 0.0 {
            return;
        }

        // Clip only when the fit actually escapes the box. `BoxFit::overflows`
        // measures rather than matching on the variant, so a `Cover` that
        // happens to match the box's aspect ratio costs no save/restore — and
        // those are two commands on every frame, per image.
        let clipped = self.fit.overflows(source, bounds, self.alignment);
        if clipped {
            ctx.canvas().save();
            ctx.canvas().clip_rect(bounds);
        }
        ctx.canvas().draw_image(drawn, &self.image);
        if clipped {
            ctx.canvas().restore();
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The box, not the opaque pixels. A transparent corner of a PNG is still
        // part of the thing the user is pointing at, and testing alpha would
        // make a tap depend on image contents the layout never looked at.
        true
    }

    fn semantics(&self) -> Option<Semantics> {
        // Only when named. An unlabelled image is decoration next to something
        // that already announces itself, and a screen reader stopping on it
        // reads "image" and nothing useful — the same rule `RenderIcon` follows.
        self.label
            .as_ref()
            .map(|label| Semantics::new(Role::Label).with_label(label.clone()))
    }

    /// Exactly what `layout` asks for, before the constraints have their say.
    ///
    /// A decoded image is the easiest thing in the tree to measure: it knows its
    /// own pixel dimensions, `width` and `height` override them when they are
    /// given, and nothing about that depends on being laid out. So this is the
    /// same `requested()` that `layout` constrains — one function, so an
    /// intrinsic and the size that comes back from a real layout in a loose box
    /// cannot drift apart.
    ///
    /// # Min and max are the same number, and the fit is not part of either
    ///
    /// An image has no slack: it does not wrap, it does not reflow, and there is
    /// no width at which it would like more room. Asking for the minimum and the
    /// maximum of the same axis is therefore the same question, and `cross` has
    /// nothing to change — a paragraph's height depends on the width it is given
    /// and a bitmap's does not.
    ///
    /// [`BoxFit`] and [`Alignment`] are deliberately ignored. They decide what
    /// happens to the *picture* inside the box, at paint time, and an intrinsic
    /// that consulted them would make a parent's size depend on how the image is
    /// cropped — the same separation the type documentation argues for, and it
    /// has to hold here or `IntrinsicWidth` would report one width while layout
    /// produced another.
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
        // The fit, the alignment and the label are all paint-time or
        // announcement-time. Only what this asks *for* is geometry — and note
        // that swapping the image for one of a different natural size is a
        // relayout, while swapping it for the same size is not.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>()
            .is_none_or(|other| self.requested() != other.requested())
    }

    fn debug_name(&self) -> &'static str {
        "RenderImage"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::Rect;
    use vieww_paint::{Command, Scene};

    use super::*;
    use crate::RenderTree;

    fn image(width: u32, height: u32) -> Image {
        Image::from_rgba8(
            vec![0; (width as usize) * (height as usize) * 4],
            width,
            height,
        )
    }

    fn painted(object: &RenderImage, size: Size) -> Scene {
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

    /// The rect the one `DrawImage` command was given.
    fn drawn_at(scene: &Scene) -> Rect {
        scene
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::DrawImage { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("an image was drawn")
    }

    /// Lay an image out in a tree, the way every other object's tests do it —
    /// `LayoutCtx` has no public constructor, and should not.
    fn laid_out(object: RenderImage, constraints: Constraints) -> Size {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.layout(id, constraints)
    }

    #[test]
    fn an_image_asks_for_its_natural_size() {
        assert_eq!(
            laid_out(RenderImage::new(image(40, 20)), Constraints::UNBOUNDED),
            Size::new(40.0, 20.0)
        );
    }

    #[test]
    fn a_tight_box_wins_over_the_natural_size() {
        let size = laid_out(
            RenderImage::new(image(40, 20)),
            Constraints::tight(Size::square(100.0)),
        );
        assert_eq!(size, Size::square(100.0));
    }

    #[test]
    fn an_explicit_size_replaces_the_natural_one() {
        let mut object = RenderImage::new(image(40, 20));
        object.width = Some(200.0);
        assert_eq!(
            laid_out(object, Constraints::UNBOUNDED),
            Size::new(200.0, 20.0),
            "the axis that was given is overridden and the other is not"
        );
    }

    #[test]
    fn contain_letterboxes_inside_the_box() {
        let mut object = RenderImage::new(image(10, 10));
        object.fit = BoxFit::Contain;
        let scene = painted(&object, Size::new(200.0, 100.0));
        assert_eq!(drawn_at(&scene), Rect::new(50.0, 0.0, 150.0, 100.0));
    }

    #[test]
    fn cover_is_clipped_to_the_box_it_overflows() {
        let mut object = RenderImage::new(image(10, 10));
        object.fit = BoxFit::Cover;
        let scene = painted(&object, Size::new(200.0, 100.0));

        let clips = scene
            .commands()
            .iter()
            .filter(|command| command.clip_bounds().is_some())
            .count();
        assert!(clips > 0, "an overflowing fit is clipped, got {scene:?}");
    }

    #[test]
    fn a_fit_that_stays_inside_costs_no_clip() {
        // The reason the clip is decided by measuring rather than by variant.
        let mut object = RenderImage::new(image(10, 10));
        object.fit = BoxFit::Contain;
        let scene = painted(&object, Size::new(200.0, 100.0));

        assert!(scene
            .commands()
            .iter()
            .all(|command| command.clip().is_none()));
    }

    #[test]
    fn a_zero_sized_box_draws_nothing() {
        let object = RenderImage::new(image(10, 10));
        assert!(painted(&object, Size::ZERO).is_empty());
    }

    #[test]
    fn changing_only_the_fit_does_not_relayout() {
        let object = RenderImage::new(image(10, 10));
        let mut refitted = RenderImage::new(object.image.clone());
        refitted.fit = BoxFit::Cover;
        assert!(!object.layout_differs(&refitted));
    }

    #[test]
    fn a_differently_sized_image_does_relayout() {
        let object = RenderImage::new(image(10, 10));
        assert!(object.layout_differs(&RenderImage::new(image(20, 20))));
    }

    #[test]
    fn only_a_named_image_reaches_a_screen_reader() {
        let mut object = RenderImage::new(image(10, 10));
        assert!(object.semantics().is_none());

        object.label = Some(String::from("A cat"));
        assert_eq!(
            object.semantics().and_then(|declared| declared.label),
            Some(String::from("A cat"))
        );
    }

    #[test]
    fn an_image_absorbs_a_tap_even_where_it_is_transparent() {
        let object = RenderImage::new(image(10, 10));
        assert!(object.hit_test_self(Offset::new(1.0, 1.0), Size::square(10.0)));
    }

    // -------------------------------------------------------------- intrinsics

    /// Ask a freshly mounted image a question, without ever laying it out.
    ///
    /// Never laid out on purpose: an intrinsic that only worked after a layout
    /// would be useless to the callers that need it — `Accordion` asks about
    /// content that is not on screen at all.
    fn intrinsic(object: RenderImage, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.intrinsic(id, query)
    }

    #[test]
    fn an_image_reports_its_natural_size_on_both_axes() {
        let object = RenderImage::new(image(40, 20));
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::max_width()),
            Some(40.0)
        );
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_height()),
            Some(20.0)
        );
    }

    #[test]
    fn an_image_has_no_slack_so_its_minimum_equals_its_maximum() {
        // Nothing about a bitmap reflows, so "the narrowest it can be without
        // overflowing" and "the widest it would like to be" are one number. A
        // minimum that came back smaller would let an `IntrinsicWidth` squeeze
        // it and clip pixels.
        let object = RenderImage::new(image(40, 20));
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::min_width()),
            intrinsic(object.clone(), crate::IntrinsicQuery::max_width())
        );
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::min_height()),
            intrinsic(object, crate::IntrinsicQuery::max_height())
        );
    }

    #[test]
    fn an_explicit_size_is_what_the_intrinsic_reports() {
        let mut object = RenderImage::new(image(40, 20));
        object.width = Some(200.0);
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::max_width()),
            Some(200.0)
        );
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_height()),
            Some(20.0),
            "the axis that was not given still measures the pixels"
        );
    }

    #[test]
    fn a_cross_extent_changes_nothing_because_an_image_does_not_wrap() {
        let object = RenderImage::new(image(40, 20));
        assert_eq!(
            intrinsic(
                object.clone(),
                crate::IntrinsicQuery::max_height().across(5.0)
            ),
            Some(20.0),
            "a bitmap given five pixels of width is still twenty tall"
        );
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_width().across(5.0)),
            Some(40.0)
        );
    }

    #[test]
    fn the_fit_and_the_alignment_do_not_move_the_intrinsic() {
        // They are paint-time decisions. An intrinsic that consulted them would
        // make every parent's size depend on how the picture is cropped.
        let mut object = RenderImage::new(image(40, 20));
        object.fit = BoxFit::Cover;
        object.alignment = Alignment::TOP_LEFT;
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_width()),
            Some(40.0)
        );
    }

    #[test]
    fn the_intrinsic_is_the_size_a_loose_layout_arrives_at() {
        // The property that matters more than any single number: one
        // `requested()` behind both, so an `IntrinsicWidth` above an image
        // cannot report a width the image then refuses to take.
        let object = RenderImage::new(image(40, 20));
        let laid = laid_out(object.clone(), Constraints::UNBOUNDED);
        assert_eq!(
            intrinsic(object.clone(), crate::IntrinsicQuery::max_width()),
            Some(laid.width)
        );
        assert_eq!(
            intrinsic(object, crate::IntrinsicQuery::max_height()),
            Some(laid.height)
        );
    }
}
