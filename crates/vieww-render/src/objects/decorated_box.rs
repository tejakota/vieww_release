use vieww_foundation::{BoxDecoration, Constraints, Offset, Size};
use vieww_paint::Path;

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Paints a [`BoxDecoration`] behind its child — a fill, rounded corners and a
/// border.
///
/// The general form of [`RenderColoredBox`](crate::RenderColoredBox), which
/// stays as it is: a plain rectangle of colour is what most of a tree is, and it
/// records one `FillRect` rather than a path the rasteriser has to flatten.
///
/// Sizes itself entirely from its child, like every other decorating box here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderDecoratedBox {
    pub decoration: BoxDecoration,
}

impl RenderDecoratedBox {
    #[must_use]
    pub const fn new(decoration: BoxDecoration) -> Self {
        Self { decoration }
    }
}

impl RenderObject for RenderDecoratedBox {
    fn describe(&self) -> Vec<(&'static str, String)> {
        let decoration = &self.decoration;
        let mut out = vec![("radius", format!("{:.1}", decoration.radius))];
        out.push(("color", format!("{:?}", decoration.color)));
        if decoration.border.is_some() {
            out.push(("border", "yes".to_owned()));
        }
        if decoration.shadow.is_some() {
            out.push(("shadow", "yes".to_owned()));
        }
        if decoration.gradient.is_some() {
            out.push(("gradient", "yes".to_owned()));
        }
        out
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        let decoration = self.decoration;

        // Behind everything, including the fill — a shadow the box then covers
        // most of is the whole idea, and drawing it after would put the blur on
        // top of the surface it is meant to lift off the page.
        if let Some(shadow) = decoration
            .visible_shadow()
            .filter(|shadow| !shadow.is_inset)
        {
            ctx.canvas().draw_shadow(bounds, decoration.radius, shadow);
        }

        if !decoration.color.is_transparent() {
            if decoration.radius > 0.0 {
                ctx.canvas()
                    .fill_rrect(bounds, decoration.radius, decoration.color.into());
            } else {
                // A square-cornered fill is a rectangle, and saying so keeps it
                // off the path rasteriser and inside damage tracking's cheapest
                // case.
                ctx.canvas().fill_rect(bounds, decoration.color.into());
            }
        }

        // Over the flat fill and under the border, so a translucent ramp reads
        // as a scrim over the surface colour rather than replacing it.
        if let Some(gradient) = decoration.visible_gradient() {
            let paint = vieww_paint::Paint::gradient(gradient);
            if decoration.radius > 0.0 {
                ctx.canvas().fill_rrect(bounds, decoration.radius, paint);
            } else {
                ctx.canvas().fill_rect(bounds, paint);
            }
        }

        // After the fill and the gradient, because an inset shadow darkens the
        // surface rather than sitting under it, and before the border, because
        // CSS puts it there and because a border drawn under the shadow would be
        // smudged by the blur it is meant to bound.
        if let Some(shadow) = decoration.visible_shadow().filter(|shadow| shadow.is_inset) {
            ctx.canvas().draw_shadow(bounds, decoration.radius, shadow);
        }

        if let Some(border) = decoration.visible_border() {
            let ring = Path::rounded_ring(bounds, decoration.radius, border.width);
            ctx.canvas().fill_path(&ring, border.color.into());
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Same rule as `RenderColoredBox`: something was painted here, so a
        // point inside the bounds landed on it. A decoration that draws nothing
        // lets input through to whatever is behind — which is what makes an
        // invisible decoration genuinely free rather than a transparent wall.
        !self.decoration.is_invisible()
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // A decoration is paint, never geometry: this box is always exactly its
        // child's size, border included, because the border is drawn inside the
        // bounds rather than around them.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderDecoratedBox"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Border, Color};
    use vieww_paint::{Command, Scene};

    use super::*;

    const BOX: Size = Size::new(40.0, 20.0);

    /// The commands a decoration records over a 40x20 box.
    fn painted(decoration: BoxDecoration) -> Scene {
        let mut scene = Scene::new();
        let object = RenderDecoratedBox::new(decoration);
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin: Offset::ZERO,
            size: BOX,
            dpr: 1.0,
        };
        object.paint(&mut ctx);
        scene
    }

    #[test]
    fn an_invisible_decoration_records_nothing_and_absorbs_nothing() {
        let object = RenderDecoratedBox::new(BoxDecoration::default());
        assert!(painted(BoxDecoration::default()).is_empty());
        assert!(!object.hit_test_self(Offset::ZERO, BOX));
    }

    #[test]
    fn a_square_fill_stays_a_rectangle_rather_than_becoming_a_path() {
        let scene = painted(BoxDecoration::filled(Color::RED));
        assert!(matches!(scene.commands(), [Command::FillRect { .. }]));
    }

    #[test]
    fn a_rounded_fill_becomes_a_path() {
        let scene = painted(BoxDecoration::filled(Color::RED).radius(4.0));
        assert!(matches!(scene.commands(), [Command::FillPath { .. }]));
    }

    #[test]
    fn a_border_is_drawn_over_the_fill_rather_than_instead_of_it() {
        let scene = painted(
            BoxDecoration::filled(Color::RED)
                .radius(4.0)
                .border(Border::thin(Color::BLACK)),
        );
        assert_eq!(scene.commands().len(), 2, "the fill, then the border");
    }

    #[test]
    fn an_outer_shadow_is_recorded_under_the_fill_and_an_inset_one_over_it() {
        use vieww_foundation::{Offset as Off, Shadow};

        let outer = painted(BoxDecoration::filled(Color::RED).shadow(Shadow::new(
            Color::BLACK,
            Off::ZERO,
            4.0,
        )));
        assert!(matches!(
            outer.commands(),
            [Command::DrawShadow { .. }, Command::FillRect { .. }]
        ));

        let inset = painted(BoxDecoration::filled(Color::RED).shadow(Shadow::inset(
            Color::BLACK,
            Off::ZERO,
            4.0,
        )));
        assert!(matches!(
            inset.commands(),
            [Command::FillRect { .. }, Command::DrawShadow { .. }]
        ));
    }

    #[test]
    fn an_inset_shadow_does_not_grow_the_area_the_box_repaints() {
        use vieww_foundation::{Offset as Off, Rect, Shadow};

        let bounds = Rect::new(0.0, 0.0, 40.0, 20.0);
        let decoration =
            BoxDecoration::filled(Color::RED).shadow(Shadow::inset(Color::BLACK, Off::ZERO, 30.0));
        assert_eq!(decoration.paint_bounds(bounds), bounds);
    }

    #[test]
    fn a_decoration_that_only_has_a_border_is_still_opaque_to_a_tap() {
        let object = RenderDecoratedBox::new(BoxDecoration::outlined(Border::thin(Color::BLACK)));
        assert!(object.hit_test_self(Offset::new(20.0, 10.0), BOX));
    }
}
