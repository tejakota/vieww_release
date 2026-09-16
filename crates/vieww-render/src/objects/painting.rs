//! Replays a [`Painting`](vieww_widget::Painting)'s recording onto the canvas.

use std::rc::Rc;

use vieww_foundation::{Brush, Constraints, Offset, Size, Sketch, Sketchbook};
use vieww_paint::{Canvas, Paint, Stroke};
use vieww_widget::Painter;

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// Runs a painter and translates what it recorded into canvas calls.
///
/// # The painter draws at the origin
///
/// A [`Sketchbook`] is recorded in the painter's own space, top-left at
/// `(0, 0)`, and this object translates the canvas to its bounds before
/// replaying. That is the whole reason the recording is a value and not a
/// sequence of canvas calls: a painter that had to add its own origin to every
/// coordinate would get it wrong once and be wrong only when the widget moved.
#[derive(Debug, Clone)]
pub struct RenderPainting {
    pub painter: Rc<dyn Painter>,
    pub size: Option<Size>,
}

impl RenderPainting {
    #[must_use]
    pub const fn new(painter: Rc<dyn Painter>, size: Option<Size>) -> Self {
        Self { painter, size }
    }
}

/// One recorded item onto the canvas, with `origin` already applied by the
/// caller's translate.
fn replay(canvas: &mut dyn Canvas, item: &Sketch) {
    if item.is_invisible() {
        return;
    }
    match item {
        Sketch::Fill { path, brush } => canvas.fill_path(path, paint_of(brush)),
        Sketch::Stroke {
            path,
            brush,
            width,
            style,
        } => canvas.stroke_path(
            path,
            Stroke::new(*width).styled(style.clone()),
            paint_of(brush),
        ),
        Sketch::Shadow {
            rect,
            radius,
            shadow,
        } => canvas.draw_shadow(*rect, *radius, *shadow),
        Sketch::Layer {
            alpha,
            blur,
            clip,
            children,
        } => {
            // The children's own bounds, un-grown: `Canvas::push_filtered_layer`
            // (`scene.rs`) already grows a filtered layer's bounds by the
            // blur's reach, and `Canvas::pop_layer` replaces the declared
            // bounds with the real content bounds grown by that same reach
            // once the layer closes — this used to grow them a second time
            // here first, which `native/reference.rs`'s `PushLayer` handling
            // then read as the bounds and grew a *third* time on top of its
            // own now-removed duplicate. See that file's `PushLayer` arm for
            // the full account; `objects/filter.rs`'s `paint` already passes
            // its object's own uninflated box for the same reason.
            let bounds = bounds_of(children);
            canvas.save();
            if let Some(path) = clip {
                canvas.clip_path(path);
            }
            if *blur > 0.0 {
                canvas.push_filtered_layer(
                    bounds,
                    *alpha,
                    vieww_foundation::BlendMode::Normal,
                    vieww_foundation::filter::ImageFilter::blur(*blur),
                );
            } else {
                canvas.push_layer(bounds, *alpha, vieww_foundation::BlendMode::Normal);
            }
            for child in children {
                replay(canvas, child);
            }
            canvas.pop_layer();
            canvas.restore();
        }
        Sketch::Transformed {
            transform,
            children,
        } => {
            canvas.save();
            canvas.transform(*transform);
            for child in children {
                replay(canvas, child);
            }
            canvas.restore();
        }
    }
}

fn paint_of(brush: &Brush) -> Paint {
    match brush {
        Brush::Solid(color) => (*color).into(),
        Brush::Gradient(gradient) => Paint::gradient(*gradient),
    }
}

/// What a recording covers, for a layer's bounds.
fn bounds_of(items: &[Sketch]) -> vieww_foundation::Rect {
    let mut bounds: Option<vieww_foundation::Rect> = None;
    let mut grow = |rect: vieww_foundation::Rect| {
        bounds = Some(bounds.map_or(rect, |current| current.union(rect)));
    };
    for item in items {
        match item {
            Sketch::Fill { path, .. } => grow(path.bounds()),
            // **`reach_factor`, not a bare half-width.** A square cap reaches
            // past the end of a line and a miter past a sharp corner, so a
            // stroke's ink can land further out than its geometry — and paint
            // bounds that were a fraction short leave the tip of an arrowhead
            // outside the damaged region, drawn once and never repainted.
            Sketch::Stroke {
                path, width, style, ..
            } => grow(path.bounds().inflate(*width / 2.0 * style.reach_factor())),
            Sketch::Shadow { rect, shadow, .. } => grow(shadow.bounds(*rect)),
            Sketch::Layer { children, blur, .. } => {
                grow(bounds_of(children).inflate(*blur * 3.0));
            }
            Sketch::Transformed {
                transform,
                children,
            } => grow(transform.apply_rect(bounds_of(children))),
        }
    }
    bounds.unwrap_or(vieww_foundation::Rect::ZERO)
}

impl RenderObject for RenderPainting {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // No requested size means "as large as I am allowed", which is what a
        // background wants. `biggest` can be infinite under an unbounded
        // constraint, so fall back to the smallest finite answer rather than
        // laying out an infinite box.
        self.size.map_or_else(
            || {
                let biggest = constraints.biggest();
                Size::new(
                    if biggest.width.is_finite() {
                        biggest.width
                    } else {
                        constraints.min_width
                    },
                    if biggest.height.is_finite() {
                        biggest.height
                    } else {
                        constraints.min_height
                    },
                )
            },
            |size| constraints.constrain(size),
        )
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        let mut book = Sketchbook::new();
        self.painter.paint(&mut book, bounds.size());
        if book.is_empty() {
            return;
        }
        let canvas = ctx.canvas();
        canvas.save();
        canvas.translate(Offset::new(bounds.left, bounds.top));
        for item in book.items() {
            replay(canvas, item);
        }
        canvas.restore();
    }

    fn debug_name(&self) -> &'static str {
        "RenderPainting"
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The box, not the drawing. A painting is decoration; the control
        // around it is what should absorb a tap, and hit testing an arbitrary
        // path would make every hole in a glyph a place a press falls through.
        false
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // Only the requested size decides geometry — a painter that draws
        // something different in the same box is a repaint, not a relayout,
        // and the sync repaints a replaced object regardless.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>()
            .is_none_or(|new| new.size != self.size)
    }
}
