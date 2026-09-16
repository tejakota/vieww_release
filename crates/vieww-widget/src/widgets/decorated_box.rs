use vieww_foundation::{Border, BoxDecoration, Color, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Paints a [`BoxDecoration`] behind its child — a fill, rounded corners and a
/// border.
///
/// [`ColoredBox`](crate::ColoredBox) generalised. Prefer that one for a plain
/// rectangle of colour: it records a rectangle rather than a path, which is the
/// cheapest thing the paint layer has.
///
/// Like `ColoredBox`, it takes its size entirely from its child — a
/// `DecoratedBox` with no child is zero-sized and paints nothing. Give it a
/// [`SizedBox`](crate::SizedBox) to fill a region.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::foundation::BoxDecoration;
///
/// let pill = DecoratedBox::new(BoxDecoration::filled(Color::BLUE).stadium())
///     .child(Padding::all(8.0).child(Text::new("beta")));
/// ```
#[derive(Debug, Clone)]
pub struct DecoratedBox {
    decoration: BoxDecoration,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl DecoratedBox {
    #[must_use]
    pub const fn new(decoration: BoxDecoration) -> Self {
        Self {
            decoration,
            child: None,
            key: None,
        }
    }

    /// A solid fill with rounded corners.
    #[must_use]
    pub const fn rounded(color: Color, radius: f32) -> Self {
        Self::new(BoxDecoration::filled(color).radius(radius))
    }

    /// An outline with nothing inside it.
    #[must_use]
    pub const fn outlined(color: Color, width: f32, radius: f32) -> Self {
        Self::new(BoxDecoration::outlined(Border::new(color, width)).radius(radius))
    }

    /// Set the child.
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

    /// The decoration to paint — what the render layer builds from.
    #[must_use]
    pub const fn decoration(&self) -> BoxDecoration {
        self.decoration
    }
}

impl Widget for DecoratedBox {
    fn debug_name(&self) -> &'static str {
        "DecoratedBox"
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
        let mut props = vec![("color", self.decoration.color.to_string())];
        if self.decoration.radius > 0.0 {
            // `stadium()` is `f32::MAX`, which is not a number anybody wants to
            // read in a tree dump.
            props.push((
                "radius",
                if self.decoration.radius.is_finite() && self.decoration.radius < 1e6 {
                    self.decoration.radius.to_string()
                } else {
                    "stadium".to_owned()
                },
            ));
        }
        if let Some(border) = self.decoration.visible_border() {
            props.push(("border", format!("{} {}", border.width, border.color)));
        }
        props
    }
}

widget_node_from!(DecoratedBox);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stadium_radius_reads_as_a_word_rather_than_a_float() {
        let props = DecoratedBox::new(BoxDecoration::filled(Color::RED).stadium())
            .debug_properties()
            .into_iter()
            .collect::<Vec<_>>();
        assert!(
            props
                .iter()
                .any(|(name, shown)| *name == "radius" && shown == "stadium"),
            "{props:?}"
        );
    }

    #[test]
    fn a_childless_box_is_a_render_leaf() {
        let plain = DecoratedBox::new(BoxDecoration::default());
        assert!(matches!(plain.kind(), WidgetKind::RenderLeaf));
        assert!(matches!(
            plain.child(crate::SizedBox::square(4.0)).kind(),
            WidgetKind::RenderSingleChild(_)
        ));
    }
}
