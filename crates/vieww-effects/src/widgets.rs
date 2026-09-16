//! Widget wrappers for the effects.
//!
//! These are composed widgets: they build into the existing widget
//! tree. The pixel operations in `cpu` are available for renderers
//! that want to apply them at raster time.

use vieww_foundation::Color;
use vieww_widget::prelude::*;

/// How to combine two layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    SrcOver,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

/// What to do to a captured backdrop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropFilter {
    /// The blur radius (sigma) in logical pixels. 0 = no blur.
    pub blur: f32,
    /// A colour drawn over the backdrop, with its alpha.
    pub tint: Color,
}

impl BackdropFilter {
    /// Frosted glass: a blur and a white tint.
    #[must_use]
    pub const fn frosted(blur: f32) -> Self {
        Self {
            blur,
            tint: Color::rgba(255, 255, 255, 26),
        }
    }

    /// Dark glass: a blur and a black tint.
    #[must_use]
    pub const fn dark(blur: f32) -> Self {
        Self {
            blur,
            tint: Color::rgba(0, 0, 0, 40),
        }
    }
}

/// A widget that blurs whatever is behind it.
///
/// A real backdrop filter: the CPU renderer samples the destination pixels
/// already painted beneath this widget's bounds, blurs and tints *that*
/// copy, and only then paints this widget's own children on top of the
/// result — see [`vieww_foundation::ImageFilter::backdrop`] for exactly how
/// that ordering is enforced. The layout is identical either way; the
/// visual is genuine frosted glass rather than a flat wash.
///
/// # Examples
///
/// ```ignore
/// BackdropBlur::new(BackdropFilter::frosted(24.0))
///     .child(nav_bar_content)
/// ```
#[derive(Debug)]
pub struct BackdropBlur {
    filter: BackdropFilter,
    child: WidgetNode,
}

impl BackdropBlur {
    /// Create a backdrop blur with `filter`.
    #[must_use]
    /// Not `const`: the placeholder child is a `WidgetNode`, which is an `Rc`
    /// allocation, and allocating is not something a const fn may do.
    pub fn new(filter: BackdropFilter) -> Self {
        Self {
            filter,
            child: SizedBox::shrink().into(),
        }
    }

    /// Set the content drawn on top of the blur.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = child.into();
        self
    }
}

impl Widget for BackdropBlur {
    fn debug_name(&self) -> &'static str {
        "BackdropBlur"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    /// # A real backdrop filter, not a degraded stand-in
    ///
    /// This builds a [`Filtered`] with
    /// [`Filtered::with_backdrop`](vieww_widget::Filtered::with_backdrop) set,
    /// which is the widget that owns the behaviour: `Command::PushLayer`
    /// carries an [`ImageFilter`](vieww_foundation::ImageFilter) whose
    /// `backdrop` flag tells the CPU renderer to sample the real destination
    /// pixels beneath this widget, blur and tint that sampled copy, and only
    /// then let this widget's children paint on top of it — genuine frosted
    /// glass, not a flat translucent wash standing in for one. Kept as its
    /// own type because "frosted glass" is a recognisable thing to ask for
    /// and `BackdropFilter::frosted(24.0)` says it in one line.
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let sigma = self.filter.blur;
        let mut filtered = vieww_widget::Filtered::blur(sigma).with_backdrop();
        if self.filter.tint.a > 0 {
            let amount = f32::from(self.filter.tint.a) / 255.0;
            filtered = filtered.tint(self.filter.tint, amount);
        }
        filtered.child(self.child.clone()).into()
    }
}

vieww_widget::widget_node_from!(BackdropBlur);

/// One colour filter, as a 5×4 matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    matrix: [f32; 20],
}

impl Filter {
    /// The identity — no change.
    #[must_use]
    pub const fn identity() -> Self {
        Self {
            matrix: [
                1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
            ],
        }
    }

    /// Adjust brightness. `1.0` = unchanged.
    #[must_use]
    pub fn brightness(amount: f32) -> Self {
        Self {
            matrix: [
                amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0, 0.0, 0.0, 0.0, amount, 0.0, 0.0,
                0.0, 0.0, 0.0, 1.0, 0.0,
            ],
        }
    }

    /// Adjust saturation. `1.0` = unchanged, `0.0` = grayscale.
    #[must_use]
    pub fn saturation(amount: f32) -> Self {
        const LR: f32 = 0.213;
        const LG: f32 = 0.715;
        const LB: f32 = 0.072;
        let sr = (1.0 - amount) * LR;
        let sg = (1.0 - amount) * LG;
        let sb = (1.0 - amount) * LB;

        Self {
            matrix: [
                sr + amount,
                sg,
                sb,
                0.0,
                0.0,
                sr,
                sg + amount,
                sb,
                0.0,
                0.0,
                sr,
                sg,
                sb + amount,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ],
        }
    }

    /// Convert to grayscale.
    #[must_use]
    pub fn grayscale() -> Self {
        Self::saturation(0.0)
    }

    /// The sepia tone.
    #[must_use]
    pub fn sepia() -> Self {
        Self {
            matrix: [
                0.393, 0.769, 0.189, 0.0, 0.0, 0.349, 0.686, 0.168, 0.0, 0.0, 0.272, 0.534, 0.131,
                0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
            ],
        }
    }

    /// The matrix, for the renderer.
    #[must_use]
    pub const fn matrix(&self) -> [f32; 20] {
        self.matrix
    }
}

/// A chain of filters applied to a subtree.
///
/// Without render-pipeline support, degrades to an opacity drop for
/// the "dimmed" case and a no-op otherwise.
///
/// # Examples
///
/// ```ignore
/// FilterChain::new()
///     .filter(Filter::grayscale())
///     .filter(Filter::brightness(0.6))
///     .child(panel_content)
/// ```
#[derive(Debug)]
pub struct FilterChain {
    filters: Vec<Filter>,
    child: WidgetNode,
}

impl FilterChain {
    /// An empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            child: SizedBox::shrink().into(),
        }
    }

    /// Add a filter to the chain.
    #[must_use]
    pub fn filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Set the filtered child.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = child.into();
        self
    }
}

impl Default for FilterChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for FilterChain {
    fn debug_name(&self) -> &'static str {
        "FilterChain"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    /// # No longer a degraded path
    ///
    /// This used to render `Opacity(0.7)` regardless of which filters were in
    /// the chain — a grayscale request came out faded and still colourful. The
    /// matrices were correct and simply never reached a renderer.
    ///
    /// They do now: the chain composes into one 5×4 matrix and rides on the
    /// group's layer, which the CPU backend applies to the rasterised pixels.
    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        if self.filters.is_empty() {
            return self.child.clone();
        }

        let mut filtered = vieww_widget::Filtered::new();
        for filter in &self.filters {
            filtered = filtered.matrix(filter.matrix());
        }
        filtered.child(self.child.clone()).into()
    }
}

vieww_widget::widget_node_from!(FilterChain);

/// Draws `foreground` over `background` using `mode`.
///
/// Without render-pipeline support, degrades to plain stacking.
#[derive(Debug)]
pub struct Blend {
    mode: BlendMode,
    foreground: WidgetNode,
    background: WidgetNode,
}

impl Blend {
    /// Create a blend with `mode`.
    #[must_use]
    /// Not `const`, for the same reason as `BackdropBlur::new`.
    pub fn new(mode: BlendMode) -> Self {
        Self {
            mode,
            foreground: SizedBox::shrink().into(),
            background: SizedBox::shrink().into(),
        }
    }

    /// The layer drawn on top.
    #[must_use]
    pub fn foreground(mut self, fg: impl Into<WidgetNode>) -> Self {
        self.foreground = fg.into();
        self
    }

    /// The layer beneath.
    #[must_use]
    pub fn background(mut self, bg: impl Into<WidgetNode>) -> Self {
        self.background = bg.into();
        self
    }
}

impl Widget for Blend {
    fn debug_name(&self) -> &'static str {
        "Blend"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Degraded: both layers in a Stack.
        let _ = self.mode;
        Stack::new()
            .push(self.background.clone())
            .push(self.foreground.clone())
            .into()
    }
}

vieww_widget::widget_node_from!(Blend);
