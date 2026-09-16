use vieww_foundation::{Key, TextDirection};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// Where a [`Stack`](crate::Stack) child sits, and how big it is.
///
/// Every field is optional and each axis is resolved independently, which is
/// what makes "pin to the bottom right, keep your natural width" expressible
/// without a second type.
///
/// # How an axis is resolved
///
/// Taking the horizontal as the example — the vertical is the same with
/// `top`/`bottom`/`height`:
///
/// | set | width | x |
/// |---|---|---|
/// | `left` and `right` | the gap between them | `left` |
/// | `left` only | the child's own | `left` |
/// | `right` only | the child's own | `stack − right − child` |
/// | `width` only | `width` | the stack's alignment |
/// | nothing | the child's own | the stack's alignment |
///
/// **An axis that pins neither edge falls back to the stack's own alignment**,
/// so a child positioned on one axis only still lands somewhere deliberate on
/// the other rather than at zero.
///
/// # Over-constraining is defined rather than rejected
///
/// `left`, `right` and `width` together are one instruction too many. Some
/// toolkits assert on it in debug and do something arbitrary in release; here **the
/// two edges win and `width` is ignored**, on the same reasoning as
/// [`AspectRatio`](crate::AspectRatio)'s substituted ratio — a rule that is
/// written down and tested beats a rule that only exists when assertions are
/// on. Pinning both edges is also the more specific request.
///
/// # Physical, not directional
///
/// `left` is the left edge in every locale, matching
/// [`Stack`](crate::Stack)'s own alignment and
/// [`EdgeInsets`](vieww_foundation::EdgeInsets). A caller opts *in* to
/// direction-aware behaviour here, as with
/// [`PaddingDirectional`](crate::PaddingDirectional) — a `PositionedDirectional`
/// would be the matching addition and is deliberately not built yet.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StackPosition {
    pub left: Option<f32>,
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl StackPosition {
    /// `true` if this says anything at all.
    ///
    /// A [`Positioned`] carrying nothing is **not** positioned: it sizes the
    /// stack and is aligned like any ordinary child. That is the useful answer
    /// for a position built up conditionally, where "no edges this time" should
    /// mean ordinary rather than pinned to a corner.
    #[must_use]
    pub const fn is_positioned(&self) -> bool {
        self.left.is_some()
            || self.top.is_some()
            || self.right.is_some()
            || self.bottom.is_some()
            || self.width.is_some()
            || self.height.is_some()
    }

    /// The width this pins the child to within a stack `extent` wide, if any.
    ///
    /// `None` leaves the axis free, and the child keeps its natural width.
    #[must_use]
    pub fn width_within(&self, extent: f32) -> Option<f32> {
        match (self.left, self.right) {
            // Both edges pinned: the child spans what is between them. Clamped
            // at zero because a stack narrower than its own insets would
            // otherwise ask for a negative width.
            (Some(left), Some(right)) => Some((extent - left - right).max(0.0)),
            _ => self.width,
        }
    }

    /// The height this pins the child to within a stack `extent` tall, if any.
    #[must_use]
    pub fn height_within(&self, extent: f32) -> Option<f32> {
        match (self.top, self.bottom) {
            (Some(top), Some(bottom)) => Some((extent - top - bottom).max(0.0)),
            _ => self.height,
        }
    }

    /// Where the child's left edge goes.
    ///
    /// `fallback` is what the stack's alignment would have chosen, used when
    /// neither horizontal edge is pinned.
    #[must_use]
    pub fn x_within(&self, extent: f32, child: f32, fallback: f32) -> f32 {
        match (self.left, self.right) {
            (Some(left), _) => left,
            (None, Some(right)) => extent - right - child,
            (None, None) => fallback,
        }
    }

    /// Where the child's top edge goes.
    #[must_use]
    pub fn y_within(&self, extent: f32, child: f32, fallback: f32) -> f32 {
        match (self.top, self.bottom) {
            (Some(top), _) => top,
            (None, Some(bottom)) => extent - bottom - child,
            (None, None) => fallback,
        }
    }
}

/// A [`StackPosition`] whose horizontal edges run from the start of the reading
/// direction to its end, rather than from left to right.
///
/// `start` is the left edge in Latin and the right edge in Arabic, so a badge
/// pinned to `end` stays where reading finishes in both. The vertical is
/// unchanged, because vertical has no handedness — the same split
/// [`EdgeInsetsDirectional`](vieww_foundation::EdgeInsetsDirectional) and
/// [`AlignmentDirectional`](vieww_foundation::AlignmentDirectional) make.
///
/// Resolved **once**, when
/// [`PositionedDirectional`](crate::PositionedDirectional) builds, into an
/// ordinary [`Positioned`]. Nothing below the widget layer ever learns what
/// direction the interface reads in.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StackPositionDirectional {
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub top: Option<f32>,
    pub bottom: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl StackPositionDirectional {
    /// The physical position this becomes when read in `direction`.
    #[must_use]
    pub const fn resolve(self, direction: TextDirection) -> StackPosition {
        let (left, right) = match direction {
            TextDirection::Ltr => (self.start, self.end),
            TextDirection::Rtl => (self.end, self.start),
        };
        StackPosition {
            left,
            top: self.top,
            right,
            bottom: self.bottom,
            width: self.width,
            height: self.height,
        }
    }
}

/// Pins a child to an edge of the [`Stack`](crate::Stack) around it.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// // A badge in the corner of an avatar.
/// let avatar = Stack::new().children(children![
///     SizedBox::square(48.0).child(ColoredBox::new(Color::rgb(70, 70, 88))),
///     Positioned::new()
///         .right(0.0)
///         .bottom(0.0)
///         .child(SizedBox::square(12.0).child(ColoredBox::new(Color::rgb(80, 200, 120)))),
/// ]);
/// ```
///
/// See [`StackPosition`] for how each axis is resolved, including what happens
/// when both edges are pinned and when neither is.
///
/// # It does nothing outside a stack
///
/// Layout-transparent by itself: constraints pass through and the size comes
/// back unchanged, so wrapping something in one cannot move a pixel on its own.
/// The **stack** is what reads the position. Outside one it is simply unused,
/// which is the same answer [`Flexible`](crate::Flexible) gives outside a
/// [`Flex`](crate::Flex) — an instruction nobody is following is not an error.
///
/// # A positioned child does not size the stack
///
/// The stack takes its size from its *unpositioned* children, then places the
/// positioned ones inside the result. That is what lets a badge hang off a
/// corner without making the stack bigger, and it means a stack whose children
/// are **all** positioned has no natural size — it takes the largest its
/// constraints allow.
#[derive(Debug, Clone, Default)]
pub struct Positioned {
    position: StackPosition,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Positioned {
    /// Positioned on no axis yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pinned to all four edges, filling the stack.
    ///
    /// The common case that would otherwise be four calls, and the one where
    /// forgetting an edge silently changes the meaning.
    #[must_use]
    pub fn fill() -> Self {
        Self::new().left(0.0).top(0.0).right(0.0).bottom(0.0)
    }

    /// A position decided somewhere else.
    ///
    /// What [`PositionedDirectional`](crate::PositionedDirectional) builds after
    /// resolving start and end against the reading direction — the same shape as
    /// [`Padding::new`](crate::Padding::new) taking already-resolved insets, so
    /// the directional form delegates rather than reimplementing.
    #[must_use]
    pub const fn from_position(position: StackPosition) -> Self {
        Self {
            position,
            child: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn left(mut self, left: f32) -> Self {
        self.position.left = Some(left);
        self
    }

    #[must_use]
    pub const fn top(mut self, top: f32) -> Self {
        self.position.top = Some(top);
        self
    }

    #[must_use]
    pub const fn right(mut self, right: f32) -> Self {
        self.position.right = Some(right);
        self
    }

    #[must_use]
    pub const fn bottom(mut self, bottom: f32) -> Self {
        self.position.bottom = Some(bottom);
        self
    }

    /// Pin the child's width.
    ///
    /// Ignored when both `left` and `right` are set — see [`StackPosition`].
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.position.width = Some(width);
        self
    }

    /// Pin the child's height.
    ///
    /// Ignored when both `top` and `bottom` are set.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.position.height = Some(height);
        self
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

    /// What this pins, for the stack to read.
    #[must_use]
    pub const fn position(&self) -> StackPosition {
        self.position
    }
}

impl Widget for Positioned {
    fn debug_name(&self) -> &'static str {
        "Positioned"
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
        // Only what was set. A dump listing four `None`s for a child pinned on
        // one edge buries the one fact worth reading.
        let mut props = Vec::new();
        for (name, value) in [
            ("left", self.position.left),
            ("top", self.position.top),
            ("right", self.position.right),
            ("bottom", self.position.bottom),
            ("width", self.position.width),
            ("height", self.position.height),
        ] {
            if let Some(value) = value {
                props.push((name, value.to_string()));
            }
        }
        props
    }
}

widget_node_from!(Positioned);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_with_nothing_set_is_not_positioned() {
        // So a conditionally-built position that pins nothing this time behaves
        // as an ordinary child rather than jumping to a corner.
        assert!(!StackPosition::default().is_positioned());
        assert!(!Positioned::new().position().is_positioned());
        assert!(Positioned::new().left(0.0).position().is_positioned());
    }

    #[test]
    fn both_edges_pin_the_extent_between_them() {
        let position = Positioned::new().left(10.0).right(30.0).position();
        assert_eq!(position.width_within(200.0), Some(160.0));
    }

    #[test]
    fn both_edges_beat_an_explicit_width() {
        // The over-constrained case, decided rather than asserted.
        let position = Positioned::new()
            .left(10.0)
            .right(30.0)
            .width(500.0)
            .position();
        assert_eq!(
            position.width_within(200.0),
            Some(160.0),
            "the two edges are the more specific request"
        );
    }

    #[test]
    fn one_edge_leaves_the_extent_to_the_child() {
        let position = Positioned::new().left(10.0).position();
        assert_eq!(position.width_within(200.0), None);
        assert_eq!(position.x_within(200.0, 40.0, 99.0), 10.0);
    }

    #[test]
    fn a_right_edge_measures_back_from_the_far_side() {
        let position = Positioned::new().right(10.0).position();
        assert_eq!(
            position.x_within(200.0, 40.0, 99.0),
            150.0,
            "200 − 10 − 40, so the child's right edge lands 10 from the stack's"
        );
    }

    #[test]
    fn an_unpinned_axis_defers_to_the_stacks_alignment() {
        // The fallback is the whole reason positioning is per-axis: a child
        // pinned to the bottom should still be centred horizontally if that is
        // what the stack says.
        let position = Positioned::new().bottom(0.0).position();
        assert_eq!(position.x_within(200.0, 40.0, 80.0), 80.0);
        assert_eq!(position.y_within(100.0, 20.0, 7.0), 80.0);
    }

    #[test]
    fn a_stack_smaller_than_its_own_insets_asks_for_zero_rather_than_less() {
        // `tighten` would clamp a negative into range anyway; being explicit
        // here means `width_within` is honest read on its own.
        let position = Positioned::new().left(80.0).right(80.0).position();
        assert_eq!(position.width_within(100.0), Some(0.0));
    }

    #[test]
    fn fill_pins_all_four_edges() {
        let position = Positioned::fill().position();
        assert_eq!(position.width_within(200.0), Some(200.0));
        assert_eq!(position.height_within(120.0), Some(120.0));
        assert_eq!(position.x_within(200.0, 200.0, 9.0), 0.0);
    }

    #[test]
    fn the_dump_lists_only_what_was_pinned() {
        let dump = crate::debug_tree(Positioned::new().left(4.0).bottom(8.0));
        assert!(dump.contains("left: 4"), "{dump}");
        assert!(dump.contains("bottom: 8"), "{dump}");
        assert!(!dump.contains("right"), "{dump}");
    }
}
