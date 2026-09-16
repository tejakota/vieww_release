//! Several filled shapes sharing one coordinate box: what an icon is when it
//! is not one colour.
//!
//! [`IconData`](crate::IconData) is a single [`Path`] scaled by a viewbox —
//! right for a glyph meant to be recoloured to match its surroundings, wrong
//! for anything that carries its own colours (a logo, an illustration, most
//! of what "SVG" means in practice). [`VectorImage`] is the same viewbox
//! idea generalised to a list of `(Path, Color)` pairs, fitted **together**
//! rather than each shape scaled and centred on its own — which is what
//! keeps a multi-shape image's parts aligned with each other instead of each
//! one being independently centred in the box and drifting apart.

use crate::{Color, Path, Rect, Transform};

/// One filled shape and the colour it is filled with.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorShape {
    pub path: Path,
    pub color: Color,
}

/// Several [`VectorShape`]s, sharing one declared coordinate box.
///
/// Cheap to clone in the sense every type in this crate is — plain data, no
/// interior sharing — because a vector image is decoded once (see
/// `vieww-asset`'s `svg` feature) and then rebuilt into a widget tree on
/// every frame same as any other asset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VectorImage {
    shapes: Vec<VectorShape>,
    viewbox: Rect,
}

impl VectorImage {
    #[must_use]
    pub const fn new(shapes: Vec<VectorShape>, viewbox: Rect) -> Self {
        Self { shapes, viewbox }
    }

    #[must_use]
    pub fn shapes(&self) -> &[VectorShape] {
        &self.shapes
    }

    #[must_use]
    pub const fn viewbox(&self) -> Rect {
        self.viewbox
    }

    /// Every shape's outline, scaled into `bounds` by **one** transform
    /// shared across all of them.
    ///
    /// The alternative — calling [`Path::fitted`] on each shape separately —
    /// is [`IconData::fitted`](crate::IconData::fitted)'s answer, and it is
    /// wrong here: `fitted` centres *that one path's own bounds* inside the
    /// target, so two shapes that are meant to sit side by side in the
    /// source would each get re-centred independently and end up
    /// overlapping. A vector image's shapes are already positioned relative
    /// to each other in viewbox space; what changes is only where that
    /// shared space lands, which is exactly what applying the *viewbox's*
    /// fit transform — not each shape's own — to every shape preserves.
    #[must_use]
    pub fn fitted(&self, bounds: Rect) -> Vec<VectorShape> {
        let transform = fit_transform(self.viewbox, bounds);
        self.shapes
            .iter()
            .map(|shape| VectorShape {
                path: shape.path.transformed(transform),
                color: shape.color,
            })
            .collect()
    }
}

/// The uniform, centred transform [`Path::fitted`] applies internally,
/// pulled out so it can be shared across every shape in a [`VectorImage`]
/// instead of being rederived — and re-centred — per shape.
fn fit_transform(from: Rect, into: Rect) -> Transform {
    if from.width() <= 0.0 || from.height() <= 0.0 {
        return Transform::IDENTITY;
    }
    let scale = (into.width() / from.width()).min(into.height() / from.height());
    let width = from.width() * scale;
    let height = from.height() * scale;

    Transform::translate(crate::Offset::new(-from.left, -from.top))
        .then(Transform::scale(scale, scale))
        .then(Transform::translate(crate::Offset::new(
            into.left + (into.width() - width) / 2.0,
            into.top + (into.height() - height) / 2.0,
        )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Offset;

    fn shape(rect: Rect, color: Color) -> VectorShape {
        VectorShape {
            path: Path::rect(rect),
            color,
        }
    }

    #[test]
    fn two_shapes_keep_their_relative_position_after_fitting() {
        let viewbox = Rect::new(0.0, 0.0, 100.0, 100.0);
        let image = VectorImage::new(
            vec![
                shape(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK),
                shape(Rect::new(50.0, 50.0, 60.0, 60.0), Color::WHITE),
            ],
            viewbox,
        );

        let fitted = image.fitted(Rect::new(0.0, 0.0, 200.0, 200.0));
        let first = fitted[0].path.bounds();
        let second = fitted[1].path.bounds();

        // Scaled 2x, and the gap between them (40 viewbox units) is
        // preserved at 2x too — 80, not something each shape's own
        // independent centring would have produced.
        assert!(
            (second.left - first.right - 80.0).abs() < 1e-3,
            "{first:?} {second:?}"
        );
    }

    #[test]
    fn fitting_preserves_colour() {
        let image = VectorImage::new(
            vec![shape(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                Color::rgb(200, 30, 30),
            )],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let fitted = image.fitted(Rect::new(0.0, 0.0, 20.0, 20.0));
        assert_eq!(fitted[0].color, Color::rgb(200, 30, 30));
    }

    #[test]
    fn a_degenerate_viewbox_leaves_shapes_where_they_were() {
        let image = VectorImage::new(
            vec![shape(Rect::new(1.0, 1.0, 2.0, 2.0), Color::BLACK)],
            Rect::ZERO,
        );
        let fitted = image.fitted(Rect::new(0.0, 0.0, 50.0, 50.0));
        assert_eq!(fitted[0].path.bounds(), Rect::new(1.0, 1.0, 2.0, 2.0));
    }

    #[test]
    fn an_empty_image_fits_to_nothing() {
        let image = VectorImage::new(Vec::new(), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(image.fitted(Rect::new(0.0, 0.0, 10.0, 10.0)).is_empty());
        let _ = Offset::ZERO;
    }
}
