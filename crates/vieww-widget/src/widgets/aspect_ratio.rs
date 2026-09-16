use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Sizes its child to a width:height ratio, as large as the space allows.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// // A 16:9 thumbnail that keeps its shape at any width.
/// let video = AspectRatio::new(16.0 / 9.0).child(ColoredBox::new(Color::BLACK));
/// ```
///
/// The ratio is width divided by height, so `16.0 / 9.0` is widescreen, `1.0`
/// is square, and `0.5` is twice as tall as it is wide.
///
/// # What it does with the space
///
/// It takes all the width it is offered and derives the height from the ratio,
/// then walks that answer back through four checks if it broke a bound — too
/// wide, too tall, too narrow, too short, in that order, each pinning one axis
/// and re-deriving the other. Inside something with no width to offer, such as
/// a horizontal scroll view, it leads with the height instead. `RenderAspectRatio`
/// in `vieww-render` is where that lives.
///
/// **The child gets no say.** It is laid out tightly at whatever size the ratio
/// produced, because a child allowed to choose could return a size that is not
/// the requested shape, which would make the widget's one job untrue.
///
/// # Constraints win
///
/// A ratio is a preference. Given constraints that allow exactly one size — a
/// `SizedBox` parent, or a tight slot in a `Flex` — that size is what you get,
/// ratio or no ratio. Wrap it in an [`Align`](crate::Align) to loosen the
/// constraints first if the shape matters more than filling the slot.
///
/// # A ratio that is not a positive finite number
///
/// Zero, negative, infinite and `NaN` ratios describe no box at all. Rather
/// than letting one divide into a size and produce a `NaN` that propagates
/// silently through every ancestor, the render object substitutes a square.
///
/// This widget still reports the ratio it was **given**, so a tree dump shows
/// the value the application actually passed rather than the square that was
/// substituted for it — which is the difference between a diagnosable mistake
/// and a mysterious box.
#[derive(Debug, Clone)]
pub struct AspectRatio {
    ratio: f32,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl AspectRatio {
    /// A box `ratio` times as wide as it is tall.
    #[must_use]
    pub fn new(ratio: f32) -> Self {
        Self {
            ratio,
            child: None,
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The ratio this widget was given, before any substitution.
    #[must_use]
    pub const fn ratio(&self) -> f32 {
        self.ratio
    }
}

impl Widget for AspectRatio {
    fn debug_name(&self) -> &'static str {
        "AspectRatio"
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
        vec![("ratio", self.ratio.to_string())]
    }
}

widget_node_from!(AspectRatio);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{debug_tree, ColoredBox, SizedBox};
    use vieww_foundation::Color;

    #[test]
    fn it_is_a_render_widget_carrying_its_child() {
        let dump = debug_tree(AspectRatio::new(2.0).child(SizedBox::square(40.0)));
        assert!(dump.contains("AspectRatio"), "{dump}");
        assert!(dump.contains("Constrained"), "{dump}");
    }

    #[test]
    fn a_childless_aspect_ratio_is_a_leaf_rather_than_a_broken_parent() {
        // It still has a size — the ratio decides one without a child — so this
        // has to be expressible rather than a panic waiting to happen.
        let dump = debug_tree(AspectRatio::new(1.0));
        assert!(dump.contains("AspectRatio"), "{dump}");
    }

    #[test]
    fn the_dump_reports_the_ratio_that_was_asked_for() {
        // Including one that will be substituted: the whole point is that a
        // dump shows the mistake rather than the square it silently became.
        let dump = debug_tree(AspectRatio::new(f32::NAN).child(ColoredBox::new(Color::BLUE)));
        assert!(dump.contains("NaN"), "{dump}");
    }
}
