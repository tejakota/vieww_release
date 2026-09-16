use vieww_foundation::{Key, Offset, Transform};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Draws its child somewhere other than where it was laid out.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Transformed;
///
/// # let progress = 0.5_f32;
/// // Slide a route in from the right. Nothing relayouts as this changes.
/// let sliding = Transformed::translate(Offset::new(320.0 * (1.0 - progress), 0.0))
///     .child(ColoredBox::new(Color::WHITE));
/// ```
///
/// # Named `Transformed`, not `Transform`
///
/// [`Transform`](vieww_foundation::Transform) is the matrix, and it is in the
/// prelude because layout and hit testing both take one. A widget with the same
/// name would shadow it for every user of the prelude, and the error when it did
/// would be about a missing method rather than a name collision — there is no
/// such clash here to avoid.
///
/// # It changes drawing, not layout
///
/// The child is measured and placed exactly as it would be without this, and
/// this widget takes the child's size. So a translated child still occupies its
/// original space in a row, and a scaled one does not push its siblings around.
/// That is what makes a transform safe to animate: no frame of the animation
/// relayouts anything.
///
/// Hit testing follows the drawing — a finger lands where the child *appears*,
/// not where it was laid out.
#[derive(Debug, Clone)]
pub struct Transformed {
    transform: Transform,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Transformed {
    #[must_use]
    pub const fn new(transform: Transform) -> Self {
        Self {
            transform,
            child: None,
            key: None,
        }
    }

    /// Shifted by `offset`.
    #[must_use]
    pub const fn translate(offset: Offset) -> Self {
        Self::new(Transform::translate(offset))
    }

    /// Scaled about the child's top-left.
    #[must_use]
    pub const fn scale(x: f32, y: f32) -> Self {
        Self::new(Transform::scale(x, y))
    }

    /// Rotated clockwise about the child's top-left.
    #[must_use]
    pub fn rotate(radians: f32) -> Self {
        Self::new(Transform::rotate(radians))
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
    pub const fn matrix(&self) -> Transform {
        self.transform
    }
}

impl Widget for Transformed {
    fn debug_name(&self) -> &'static str {
        "Transformed"
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
        // A pure translation prints as the two numbers that describe it; anything
        // else prints the whole matrix, because there is no shorter honest
        // summary of a rotation composed with a scale.
        if self.transform.is_translation() {
            return vec![
                ("dx", self.transform.tx.to_string()),
                ("dy", self.transform.ty.to_string()),
            ];
        }
        vec![("matrix", format!("{:?}", self.transform.to_array()))]
    }
}

widget_node_from!(Transformed);
