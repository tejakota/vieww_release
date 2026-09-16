use vieww_foundation::{Axis, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Sizes its child to what the child *wants* on one axis, rather than to what
/// the parent offers.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::IntrinsicSize;
///
/// // A row whose two cells are both as tall as the taller one.
/// let row = IntrinsicSize::height(
///     Flex::row().children(children![
///         Text::new("one line"),
///         Text::new("a much longer run of text that will wrap onto several lines"),
///     ]),
/// );
/// ```
///
/// # What it is for
///
/// The constraints protocol runs one way: a parent decides, a child fits. That
/// is what makes layout a single pass, and it is the right default. This is the
/// escape hatch for the handful of layouts that genuinely need the other
/// direction — a card as wide as its longest label, a row of buttons all as
/// tall as the tallest, a divider between two columns that runs the full height
/// of the taller one.
///
/// # It costs a second walk
///
/// Measuring means querying the subtree before laying it out, so the subtree
/// below this widget is visited twice per layout. That is fine over a card and
/// wrong over a list of a thousand rows. The standard guidance holds unchanged:
/// reach for this when the layout cannot be expressed any other way, not as a
/// general convenience.
///
/// # When the subtree cannot be measured
///
/// Not every render object can answer — see
/// [`RenderObject::intrinsic`](https://docs.rs/vieww-render). One that cannot
/// makes this widget **transparent**: the child is laid out against the
/// original constraints, exactly as if the wrapper were not there. It never
/// collapses the child to zero, which is what an unimplemented intrinsic
/// produces in frameworks that default the answer to a number instead of to
/// "unknown".
///
/// The reliable escape hatch, when a subtree contains something unmeasurable
/// and a number is needed anyway, is a [`SizedBox`](crate::SizedBox): a tight
/// constraint reports itself, so wrapping the unmeasurable part makes the whole
/// subtree measurable again.
///
/// # The name
///
/// Not `Sized`. That is `std::marker::Sized`, which is in every Rust prelude,
/// and a widget of the same name compiles right up until a call site writes
/// `Sized::height(…)` and gets *"expected a type, found a trait"*. Found the
/// first time this was used from outside its own crate.
#[derive(Debug, Clone)]
pub struct IntrinsicSize {
    axis: Axis,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl IntrinsicSize {
    /// As wide as the child wants to be, then lay the child out at that width.
    #[must_use]
    pub fn width(child: impl Into<WidgetNode>) -> Self {
        Self {
            axis: Axis::Horizontal,
            child: Some(child.into()),
            key: None,
        }
    }

    /// As tall as the child wants to be, then lay the child out at that height.
    ///
    /// The common one, and the one a row of cells with a shared background
    /// needs: without it each cell is only as tall as its own content and the
    /// backgrounds end at different heights.
    #[must_use]
    pub fn height(child: impl Into<WidgetNode>) -> Self {
        Self {
            axis: Axis::Vertical,
            child: Some(child.into()),
            key: None,
        }
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Which axis is measured — read by the render layer.
    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }
}

impl Widget for IntrinsicSize {
    fn debug_name(&self) -> &'static str {
        "IntrinsicSize"
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
        vec![("axis", format!("{:?}", self.axis))]
    }
}

widget_node_from!(IntrinsicSize);
