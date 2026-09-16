use vieww_foundation::{BlendMode, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Fades its child without moving it.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Opacity;
///
/// # let enabled = false;
/// let label = Opacity::new(if enabled { 1.0 } else { 0.38 })
///     .child(Text::new("Sync over cellular"));
/// ```
///
/// # It still takes taps at zero
///
/// A fully faded child occupies its space and remains hit-testable, exactly as
/// everywhere. Invisible is not the same as absent, and a control that should be
/// neither seen nor pressed should be left out of the tree rather than faded to
/// nothing — otherwise the screen has a hole in it that swallows taps.
///
/// # Overlapping children fade together
///
/// The subtree is composited into a target of its own and *that* result is
/// faded, which is what "50% opacity" means to everyone who is not implementing
/// it: text over its own background at half opacity no longer shows the
/// background through the text. See
/// [`RenderOpacity`](https://docs.rs/vieww-render) for how that is done without
/// costing damage the ability to measure a layer.
///
/// # It is also where a blend mode goes
///
/// [`blend`](Self::blend) is how a subtree combines with what is already behind
/// it — the whole Porter-Duff family, the separable modes and the four
/// non-separable ones. It lives here rather than on a paint or a decoration
/// because blending is a property of a **composited group** against its
/// backdrop, and this widget is the thing in the widget layer that makes a
/// group. A blend on a single fill would be a different and much narrower
/// feature wearing the same name.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Opacity;
/// use vieww_foundation::BlendMode;
///
/// // A highlighter over text: multiply darkens without hiding what is under it.
/// let marker = Opacity::new(1.0)
///     .blend(BlendMode::Multiply)
///     .child(Text::new("highlighted"));
/// ```
///
/// Alpha and blend are independent, and `Opacity::new(1.0).blend(..)` is the
/// ordinary way to reach a blend mode with no fade at all.
#[derive(Debug, Clone)]
pub struct Opacity {
    alpha: f32,
    blend: BlendMode,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Opacity {
    /// `alpha` runs 0.0 (invisible) to 1.0 (unchanged), and is clamped.
    #[must_use]
    pub const fn new(alpha: f32) -> Self {
        Self {
            alpha,
            blend: BlendMode::Normal,
            child: None,
            key: None,
        }
    }

    /// How the composited subtree combines with what is already behind it.
    ///
    /// [`BlendMode::Normal`] by default, which composites the group over its
    /// backdrop and is what every other framework does without asking.
    #[must_use]
    pub const fn blend(mut self, blend: BlendMode) -> Self {
        self.blend = blend;
        self
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
    pub const fn alpha(&self) -> f32 {
        self.alpha
    }

    #[must_use]
    pub const fn blend_mode(&self) -> BlendMode {
        self.blend
    }
}

impl Widget for Opacity {
    fn debug_name(&self) -> &'static str {
        "Opacity"
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

widget_node_from!(Opacity);
