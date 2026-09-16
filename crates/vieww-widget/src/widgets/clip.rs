use vieww_foundation::{Key, Path};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// The shape a [`Clip`] cuts its child down to.
///
/// Mirrors `vieww_render::ClipShape`, and is a separate type for the reason the
/// whole widget layer is separate: this crate must not depend on the render
/// layer. The factory translates between them.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ClipShape {
    /// The child's own bounds — a plain rectangle.
    #[default]
    Rect,
    /// Rounded corners.
    RRect { radius: f32 },
    /// The largest ellipse that fits — a circle in a square box.
    Oval,
    /// An arbitrary shape, in the child's coordinate space.
    Path(Path),
}

/// Cuts its child down to a shape.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Clip;
///
/// // A circular avatar.
/// let avatar = Clip::oval().child(
///     SizedBox::square(48.0),
/// );
///
/// // A card whose content cannot escape its rounded corners.
/// let card = Clip::rounded(12.0).child(Text::new("Contents"));
/// ```
///
/// # Why this is not a `Container` option
///
/// A decoration's radius rounds *what the box paints*. This rounds *what its
/// child paints*, which is a different job and a strictly more expensive one —
/// a clip is a compositing operation and a fill is not. Keeping them apart
/// means the common case, a rounded box with nothing overflowing it, costs a
/// rounded fill and no clip at all.
///
/// # Hit testing follows the box, not the shape
///
/// A tap in the corner of a circular avatar still reaches it. Every platform
/// behaves this way; the alternative makes targets smaller than they look.
#[derive(Debug, Clone)]
pub struct Clip {
    shape: ClipShape,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Clip {
    #[must_use]
    pub const fn new(shape: ClipShape) -> Self {
        Self {
            shape,
            child: None,
            key: None,
        }
    }

    /// Clip to the child's bounds.
    #[must_use]
    pub const fn rect() -> Self {
        Self::new(ClipShape::Rect)
    }

    /// Clip to rounded corners.
    #[must_use]
    pub const fn rounded(radius: f32) -> Self {
        Self::new(ClipShape::RRect { radius })
    }

    /// Clip to the inscribed ellipse.
    #[must_use]
    pub const fn oval() -> Self {
        Self::new(ClipShape::Oval)
    }

    /// Clip to an arbitrary path, in the child's coordinate space.
    #[must_use]
    pub fn path(path: Path) -> Self {
        Self::new(ClipShape::Path(path))
    }

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

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub fn shape(&self) -> &ClipShape {
        &self.shape
    }
}

impl Widget for Clip {
    fn debug_name(&self) -> &'static str {
        "Clip"
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
}

widget_node_from!(Clip);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_with_no_child_is_a_leaf_rather_than_a_broken_parent() {
        assert!(matches!(Clip::oval().kind(), WidgetKind::RenderLeaf));
    }

    #[test]
    fn the_shape_survives_to_the_factory() {
        assert_eq!(
            *Clip::rounded(8.0).shape(),
            ClipShape::RRect { radius: 8.0 }
        );
    }
}
