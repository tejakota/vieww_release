use vieww_foundation::{Constraints, Key, Size};

use crate::{widget_node_from, BuildContext, Widget, WidgetKind, WidgetNode};

/// Imposes additional [`Constraints`] on its child.
///
/// The constraints are *additional*: they are enforced against whatever the
/// parent passed down, and the parent always wins. Asking for 500px inside a
/// 200px-max parent yields 200px, not an overflow.
#[derive(Debug, Clone)]
pub struct Constrained {
    constraints: Constraints,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Constrained {
    #[must_use]
    pub fn new(constraints: Constraints) -> Self {
        Self {
            constraints,
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

    /// The extra constraints imposed on the child.
    #[must_use]
    pub const fn constraints(&self) -> Constraints {
        self.constraints
    }
}

impl Widget for Constrained {
    fn debug_name(&self) -> &'static str {
        "Constrained"
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
        vec![("constraints", self.constraints.to_string())]
    }
}

/// A box of a fixed width and/or height.
///
/// Composes to a [`Constrained`] with tight constraints on whichever axes were
/// given. With no child and no dimensions it is an empty gap of zero size;
/// [`SizedBox::square`] and the axis constructors cover the common spacer cases.
#[derive(Debug, Clone)]
pub struct SizedBox {
    width: Option<f32>,
    height: Option<f32>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl SizedBox {
    /// A box with no imposed size — it takes its child's size.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            width: None,
            height: None,
            child: None,
            key: None,
        }
    }

    /// A box of exactly `size`.
    #[must_use]
    pub const fn from_size(size: Size) -> Self {
        Self {
            width: Some(size.width),
            height: Some(size.height),
            child: None,
            key: None,
        }
    }

    /// A square box of the given side.
    #[must_use]
    pub const fn square(side: f32) -> Self {
        Self::from_size(Size::square(side))
    }

    /// A horizontal spacer.
    #[must_use]
    pub const fn width(width: f32) -> Self {
        Self {
            width: Some(width),
            height: None,
            child: None,
            key: None,
        }
    }

    /// A vertical spacer.
    #[must_use]
    pub const fn height(height: f32) -> Self {
        Self {
            width: None,
            height: Some(height),
            child: None,
            key: None,
        }
    }

    /// A box that is as small as its constraints allow.
    #[must_use]
    pub const fn shrink() -> Self {
        Self::from_size(Size::ZERO)
    }

    /// A box that is as large as its constraints allow.
    ///
    /// Infinity on both axes, which the parent's constraints then clamp — so
    /// this fills a bounded parent and, in an unbounded one, is exactly the
    /// mistake it looks like. A scrim, a full-screen background, and anything
    /// that has to be as big as the surface.
    #[must_use]
    pub const fn expand() -> Self {
        Self::from_size(Size::INFINITE)
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

impl Default for SizedBox {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for SizedBox {
    fn debug_name(&self) -> &'static str {
        "SizedBox"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // `tighten` only pins the axes that were given, leaving the others as
        // the parent set them.
        let constrained = Constrained::new(Constraints::UNBOUNDED.tighten(self.width, self.height));
        match &self.child {
            Some(child) => constrained.child(child.clone()),
            None => constrained,
        }
        .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = Vec::new();
        if let Some(width) = self.width {
            props.push(("width", width.to_string()));
        }
        if let Some(height) = self.height {
            props.push(("height", height.to_string()));
        }
        props
    }
}

widget_node_from!(Constrained, SizedBox);
