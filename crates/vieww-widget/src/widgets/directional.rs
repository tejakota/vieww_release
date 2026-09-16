//! [`Padding`], [`Align`] and [`Positioned`], expressed against the reading
//! direction.
//!
//! All three are thin composed widgets: they read the ambient
//! [`Directionality`](crate::Directionality), resolve their directional value
//! into a physical one, and build the ordinary widget. Nothing below the widget
//! layer learns what direction the interface reads in, which is the whole
//! design — see [`EdgeInsetsDirectional`] for why resolution happens once, here,
//! rather than lazily during layout.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::{Directionality, PaddingDirectional};
//! use vieww_foundation::{EdgeInsetsDirectional, TextDirection};
//!
//! // 16 before the text and 8 after it, whichever way it reads.
//! let row = PaddingDirectional::new(EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0))
//!     .child(Text::new("مرحبا"));
//! let app = Directionality::new(TextDirection::Rtl).child(row);
//! ```
//!
//! # Why these are separate widgets rather than a second constructor
//!
//! [`Padding`], [`Align`] and [`Positioned`] are render widgets: the factory
//! reads their values straight into a render object, and they never run a
//! `build`, so they have no
//! [`BuildContext`] from which to ask what direction is in force. Resolving
//! needs one. So the directional forms are `Composed` and delegate.
//!
//! That costs **one extra element per directional padding**, which is the same
//! trade [`Flexible`](crate::Flexible) already makes and for the same reason:
//! a transparent wrapper is cheaper than pushing a new concept — here an
//! ambient direction, there parent data — through every node in the tree.

use vieww_foundation::{AlignmentDirectional, EdgeInsetsDirectional, Key};

use crate::{
    widget_node_from, Align, BuildContext, Directionality, Padding, Positioned,
    StackPositionDirectional, Widget, WidgetKind, WidgetNode,
};

/// Insets its child by an [`EdgeInsetsDirectional`].
#[derive(Debug, Clone)]
pub struct PaddingDirectional {
    insets: EdgeInsetsDirectional,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl PaddingDirectional {
    #[must_use]
    pub fn new(insets: EdgeInsetsDirectional) -> Self {
        Self {
            insets,
            child: None,
            key: None,
        }
    }

    /// Insets along the reading axis on start/end, and vertically.
    #[must_use]
    pub fn symmetric(along: f32, vertical: f32) -> Self {
        Self::new(EdgeInsetsDirectional::symmetric(along, vertical))
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

    /// The unresolved insets.
    #[must_use]
    pub const fn insets(&self) -> EdgeInsetsDirectional {
        self.insets
    }
}

impl Widget for PaddingDirectional {
    fn debug_name(&self) -> &'static str {
        "PaddingDirectional"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("insets", self.insets.to_string())]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let padding = Padding::new(self.insets.resolve(Directionality::of(ctx)));
        match &self.child {
            Some(child) => padding.child(child.clone()).into(),
            None => padding.into(),
        }
    }
}

widget_node_from!(PaddingDirectional);

/// Positions its child by an [`AlignmentDirectional`].
#[derive(Debug, Clone)]
pub struct AlignDirectional {
    alignment: AlignmentDirectional,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl AlignDirectional {
    #[must_use]
    pub fn new(alignment: AlignmentDirectional) -> Self {
        Self {
            alignment,
            child: None,
            key: None,
        }
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

    /// The unresolved alignment.
    #[must_use]
    pub const fn alignment(&self) -> AlignmentDirectional {
        self.alignment
    }
}

impl Widget for AlignDirectional {
    fn debug_name(&self) -> &'static str {
        "AlignDirectional"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("alignment", self.alignment.to_string())]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let align = Align::new(self.alignment.resolve(Directionality::of(ctx)));
        match &self.child {
            Some(child) => align.child(child.clone()).into(),
            None => align.into(),
        }
    }
}

widget_node_from!(AlignDirectional);

/// Pins a child to a [`Stack`](crate::Stack) edge, against the reading
/// direction.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Directionality, PositionedDirectional};
/// use vieww_foundation::TextDirection;
///
/// // A badge where reading finishes: bottom right in Latin, bottom left in Arabic.
/// let avatar = Stack::new().children(children![
///     SizedBox::square(48.0).child(ColoredBox::new(Color::rgb(70, 70, 88))),
///     PositionedDirectional::new()
///         .end(0.0)
///         .bottom(0.0)
///         .child(SizedBox::square(12.0).child(ColoredBox::new(Color::rgb(80, 200, 120)))),
/// ]);
/// let app = Directionality::new(TextDirection::Rtl).child(avatar);
/// ```
///
/// [`Positioned`] is physical and stays physical — `left` is the left edge in
/// every locale. This is how a caller opts *in*, exactly as
/// [`PaddingDirectional`] is to [`Padding`].
///
/// It **delegates** rather than reimplementing: `build` resolves start and end
/// into left and right for the direction in force and returns an ordinary
/// [`Positioned`], so every rule about how an axis resolves — both edges
/// beating an explicit width, an unpinned axis falling back to the stack's
/// alignment — is defined in exactly one place.
///
/// # It reaches the stack through a composed widget, and that still works
///
/// A [`Stack`](crate::Stack) reads positions off its **render** children, and a
/// `Composed` widget produces no render object. So what the stack sees is the
/// `Positioned` this builds, sitting exactly where this widget was. The cost is
/// one extra element, the same trade the two widgets above make.
#[derive(Debug, Clone, Default)]
pub struct PositionedDirectional {
    position: StackPositionDirectional,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl PositionedDirectional {
    /// Positioned on no axis yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pinned to all four edges, filling the stack.
    #[must_use]
    pub fn fill() -> Self {
        Self::new().start(0.0).top(0.0).end(0.0).bottom(0.0)
    }

    /// Distance from the edge reading begins at.
    #[must_use]
    pub const fn start(mut self, start: f32) -> Self {
        self.position.start = Some(start);
        self
    }

    /// Distance from the edge reading finishes at.
    #[must_use]
    pub const fn end(mut self, end: f32) -> Self {
        self.position.end = Some(end);
        self
    }

    #[must_use]
    pub const fn top(mut self, top: f32) -> Self {
        self.position.top = Some(top);
        self
    }

    #[must_use]
    pub const fn bottom(mut self, bottom: f32) -> Self {
        self.position.bottom = Some(bottom);
        self
    }

    /// Pin the child's width. Ignored when both `start` and `end` are set.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.position.width = Some(width);
        self
    }

    /// Pin the child's height. Ignored when both `top` and `bottom` are set.
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

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The unresolved position.
    #[must_use]
    pub const fn position(&self) -> StackPositionDirectional {
        self.position
    }
}

impl Widget for PositionedDirectional {
    fn debug_name(&self) -> &'static str {
        "PositionedDirectional"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        // The unresolved edges, as written — a dump reporting `left` where the
        // source says `start` would disagree with the code on exactly the
        // screens this widget exists for.
        let mut props = Vec::new();
        for (name, value) in [
            ("start", self.position.start),
            ("top", self.position.top),
            ("end", self.position.end),
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

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let positioned = Positioned::from_position(self.position.resolve(Directionality::of(ctx)));
        match &self.child {
            Some(child) => positioned.child(child.clone()).into(),
            None => positioned.into(),
        }
    }
}

widget_node_from!(PositionedDirectional);

#[cfg(test)]
mod tests {
    use vieww_foundation::{Alignment, EdgeInsets, TextDirection};

    use super::*;
    use crate::{debug_tree, Text};

    /// The physical `Padding` a directional one built under `direction`.
    fn resolved_insets(insets: EdgeInsetsDirectional, direction: TextDirection) -> EdgeInsets {
        let ctx = BuildContext::root().child_with(std::rc::Rc::new(direction));
        insets.resolve(Directionality::of(&ctx))
    }

    #[test]
    fn padding_resolves_against_the_ambient_direction() {
        let insets = EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0);
        assert_eq!(
            resolved_insets(insets, TextDirection::Ltr),
            EdgeInsets::only(16.0, 0.0, 8.0, 0.0)
        );
        assert_eq!(
            resolved_insets(insets, TextDirection::Rtl),
            EdgeInsets::only(8.0, 0.0, 16.0, 0.0),
            "start becomes the right edge, so the 16 moves across"
        );
    }

    #[test]
    fn padding_with_no_directionality_above_it_reads_left_to_right() {
        let insets = EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0);
        assert_eq!(
            insets.resolve(Directionality::of(&BuildContext::root())),
            EdgeInsets::only(16.0, 0.0, 8.0, 0.0)
        );
    }

    #[test]
    fn a_directional_padding_builds_an_ordinary_padding() {
        // The property that keeps the render layer direction-free: what lands in
        // the tree below is a plain `Padding`, indistinguishable from one an
        // application wrote by hand.
        let dump = debug_tree(
            PaddingDirectional::new(EdgeInsetsDirectional::all(4.0)).child(Text::new("hi")),
        );
        assert!(dump.contains("PaddingDirectional"), "{dump}");
        assert!(dump.contains("Padding"), "{dump}");
    }

    #[test]
    fn a_childless_directional_padding_still_builds() {
        // `Padding` is a render *leaf* with no child, and the delegation must
        // not turn that into a panic or a phantom child.
        let dump = debug_tree(PaddingDirectional::new(EdgeInsetsDirectional::all(4.0)));
        assert!(dump.contains("Padding"), "{dump}");
    }

    #[test]
    fn align_resolves_against_the_ambient_direction() {
        let ltr = BuildContext::root().child_with(std::rc::Rc::new(TextDirection::Ltr));
        let rtl = BuildContext::root().child_with(std::rc::Rc::new(TextDirection::Rtl));
        let start = AlignmentDirectional::CENTER_START;
        assert_eq!(
            start.resolve(Directionality::of(&ltr)),
            Alignment::CENTER_LEFT
        );
        assert_eq!(
            start.resolve(Directionality::of(&rtl)),
            Alignment::CENTER_RIGHT
        );
    }

    #[test]
    fn a_directional_align_builds_an_ordinary_align() {
        let dump = debug_tree(
            AlignDirectional::new(AlignmentDirectional::TOP_START).child(Text::new("hi")),
        );
        assert!(dump.contains("AlignDirectional"), "{dump}");
        assert!(dump.contains("Align"), "{dump}");
    }

    // --------------------------------------------------------- positioned

    #[test]
    fn start_and_end_swap_sides_with_the_reading_direction() {
        // The whole point of the type, on the axis that has handedness.
        let position = PositionedDirectional::new()
            .start(16.0)
            .end(4.0)
            .bottom(2.0)
            .position();

        let ltr = position.resolve(TextDirection::Ltr);
        assert_eq!((ltr.left, ltr.right), (Some(16.0), Some(4.0)));

        let rtl = position.resolve(TextDirection::Rtl);
        assert_eq!(
            (rtl.left, rtl.right),
            (Some(4.0), Some(16.0)),
            "start becomes the right edge, so the 16 moves across"
        );

        assert_eq!(
            (ltr.bottom, rtl.bottom),
            (Some(2.0), Some(2.0)),
            "vertical has no handedness and must not move"
        );
    }

    #[test]
    fn a_directional_position_builds_an_ordinary_positioned() {
        // The property that keeps the render layer direction-free, and the one
        // that makes this reach a `Stack` at all: `Composed` produces no render
        // object, so what the stack sees is the plain `Positioned` below.
        let dump = debug_tree(PositionedDirectional::new().end(4.0).child(Text::new("hi")));
        assert!(dump.contains("PositionedDirectional"), "{dump}");
        assert!(dump.contains("Positioned"), "{dump}");
    }

    #[test]
    fn a_childless_directional_position_still_builds() {
        let dump = debug_tree(PositionedDirectional::new().start(4.0));
        assert!(dump.contains("Positioned"), "{dump}");
    }

    #[test]
    fn the_dump_reports_the_edges_as_written_rather_than_resolved() {
        // `start: 4` and not `left: 4`, on the same reasoning as `Stack`
        // printing `topStart`: a dump that disagreed with the source would do so
        // on exactly the screens this widget exists for.
        let dump = debug_tree(PositionedDirectional::new().start(4.0).bottom(8.0));
        assert!(dump.contains("start: 4"), "{dump}");
        assert!(dump.contains("bottom: 8"), "{dump}");
        assert!(!dump.contains("end"), "{dump}");
    }

    #[test]
    fn resolving_carries_every_field_through_untouched() {
        // A field added to `StackPositionDirectional` and forgotten in `resolve`
        // would silently stop working; this is what notices.
        let position = PositionedDirectional::fill()
            .width(5.0)
            .height(6.0)
            .position();
        let resolved = position.resolve(TextDirection::Rtl);
        assert_eq!(
            (resolved.left, resolved.top, resolved.right, resolved.bottom),
            (Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        );
        assert_eq!((resolved.width, resolved.height), (Some(5.0), Some(6.0)));
    }

    #[test]
    fn an_unset_edge_stays_unset_through_the_swap() {
        // `end` alone must not invent a `left`, or a badge pinned to one side
        // would be stretched across the stack in the other direction.
        let position = PositionedDirectional::new().end(0.0).position();
        assert_eq!(
            position.resolve(TextDirection::Ltr).left,
            None,
            "nothing was said about the leading edge"
        );
        assert_eq!(position.resolve(TextDirection::Ltr).right, Some(0.0));
        assert_eq!(position.resolve(TextDirection::Rtl).left, Some(0.0));
        assert_eq!(position.resolve(TextDirection::Rtl).right, None);
    }

    #[test]
    fn the_unresolved_value_is_readable_for_a_tree_dump() {
        let padding = PaddingDirectional::symmetric(12.0, 4.0);
        assert_eq!(padding.insets().along(), 24.0);
        let align = AlignDirectional::new(AlignmentDirectional::BOTTOM_END);
        assert_eq!(align.alignment(), AlignmentDirectional::BOTTOM_END);
    }
}
