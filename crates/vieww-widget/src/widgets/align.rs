use vieww_foundation::{Alignment, Key};

use crate::{widget_node_from, BuildContext, Widget, WidgetKind, WidgetNode};

/// Positions its child within itself according to an [`Alignment`].
///
/// By default it expands to fill whatever bounded space it is given, which is
/// what makes alignment meaningful — an `Align` that shrink-wrapped its child
/// would have nothing to align within. Where the incoming constraints are
/// unbounded it falls back to the child's size on that axis.
#[derive(Debug, Clone)]
pub struct Align {
    alignment: Alignment,
    width_factor: Option<f32>,
    height_factor: Option<f32>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Align {
    #[must_use]
    pub fn new(alignment: Alignment) -> Self {
        Self {
            alignment,
            width_factor: None,
            height_factor: None,
            child: None,
            key: None,
        }
    }

    /// Be `factor` times the child's width, instead of filling the space.
    ///
    /// The child is aligned inside the result, and anything hanging out of it
    /// is left for a [`Clip`](crate::Clip) above to cut — this widget sizes,
    /// it does not clip.
    #[must_use]
    pub const fn width_factor(mut self, factor: f32) -> Self {
        self.width_factor = Some(factor);
        self
    }

    /// Be `factor` times the child's height, instead of filling the space.
    ///
    /// Animating this from `0.0` to `1.0` under a `Clip` is a reveal to the
    /// child's own natural height, measured this frame — see
    /// [`Accordion`](crate::Accordion), which is exactly that and nothing else.
    #[must_use]
    pub const fn height_factor(mut self, factor: f32) -> Self {
        self.height_factor = Some(factor);
        self
    }

    /// The width factor, if one was set — read by the render layer.
    #[must_use]
    pub const fn width_factor_value(&self) -> Option<f32> {
        self.width_factor
    }

    /// The height factor, if one was set — read by the render layer.
    #[must_use]
    pub const fn height_factor_value(&self) -> Option<f32> {
        self.height_factor
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

    /// Where the child sits.
    #[must_use]
    pub const fn alignment(&self) -> Alignment {
        self.alignment
    }
}

impl Widget for Align {
    fn debug_name(&self) -> &'static str {
        "Align"
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
        vec![("alignment", self.alignment.to_string())]
    }
}

/// Centers its child. Composes to [`Align`] with [`Alignment::CENTER`].
///
/// It exists as its own type rather than a constructor on `Align` because it
/// reads better at call sites and costs one build, and because it demonstrates
/// the composed shape: no layout logic of its own.
#[derive(Debug, Clone)]
pub struct Center {
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Center {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            child: None,
            key: None,
        }
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
}

impl Default for Center {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Center {
    fn debug_name(&self) -> &'static str {
        "Center"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let align = Align::new(Alignment::CENTER);
        match &self.child {
            Some(child) => align.child(child.clone()),
            None => align,
        }
        .into()
    }
}

widget_node_from!(Align, Center);
