use vieww_foundation::{Alignment, AlignmentDirectional, Key, TextDirection};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// How a [`Stack`] sizes children that are not explicitly positioned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackFit {
    /// Children get the stack's constraints, loosened — they may be smaller.
    #[default]
    Loose,
    /// Children are forced to the stack's biggest allowed size.
    Expand,
    /// Children get the stack's incoming constraints unchanged.
    Passthrough,
}

/// Overlays its children, later children on top.
///
/// The stack sizes itself to its largest child, then places each child using
/// [`alignment`](Stack::alignment).
///
/// # Reading direction
///
/// **A stack does not mirror unless it is asked to**, and that is a deliberate
/// divergence from the classic default, where a stack's alignment defaults to
/// the directional top-start and therefore flips on its own.
///
/// [`Alignment::TOP_LEFT`] means the top left corner in every locale, because
/// that is what it says. Use
/// [`alignment_directional`](Self::alignment_directional) to place a child
/// against the edge reading *begins* at — a badge on an avatar, a close button
/// on a card — which is what usually wants to move.
///
/// The reason for the divergence is the rule the rest of this framework already
/// follows: [`EdgeInsets`](vieww_foundation::EdgeInsets) stays physical and
/// [`Padding`](crate::Padding) stays physical, and a caller opts *in* to
/// directional behaviour rather than out of it. [`Flex`](crate::Flex) is the one
/// exception, and it earns it — a row's *axis* runs in reading order, which is a
/// property of the sequence rather than of a named corner. Making the default
/// directional here would also silently move every `Stack` already written.
#[derive(Debug, Clone)]
pub struct Stack {
    alignment: Alignment,
    /// Set by [`alignment_directional`](Stack::alignment_directional), and it
    /// wins over [`alignment`](Stack::alignment) when present.
    ///
    /// Two fields for one property, with each setter clearing the other, rather
    /// than an enum: the enum would have to be public to cross into
    /// `vieww-render`, where the factory resolves it, and this keeps the new
    /// surface to one builder and one accessor.
    alignment_directional: Option<AlignmentDirectional>,
    fit: StackFit,
    children: Vec<WidgetNode>,
    key: Option<Key>,
}

impl Stack {
    #[must_use]
    pub fn new() -> Self {
        Self {
            alignment: Alignment::TOP_LEFT,
            alignment_directional: None,
            fit: StackFit::default(),
            children: Vec::new(),
            key: None,
        }
    }

    /// Replace the children, in paint order — last child paints on top.
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = WidgetNode>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// Append one child on top of the existing ones.
    #[must_use]
    pub fn push(mut self, child: impl Into<WidgetNode>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Set where children sit within the stack, by physical corner.
    ///
    /// Clears any [`alignment_directional`](Self::alignment_directional): the
    /// two are one property, and the last call wins.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self.alignment_directional = None;
        self
    }

    /// Set where children sit, against the reading direction.
    ///
    /// [`TOP_START`](AlignmentDirectional::TOP_START) is the top left corner in
    /// Latin and the top right in Arabic, so a badge pinned to it stays where
    /// reading begins. Resolved against the nearest
    /// [`Directionality`](crate::Directionality) when the render object is
    /// built.
    ///
    /// Clears any [`alignment`](Self::alignment); the last call wins.
    #[must_use]
    pub const fn alignment_directional(mut self, alignment: AlignmentDirectional) -> Self {
        self.alignment_directional = Some(alignment);
        self
    }

    /// Set how children are sized.
    #[must_use]
    pub const fn fit(mut self, fit: StackFit) -> Self {
        self.fit = fit;
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Where children sit within the stack, once the direction is known.
    ///
    /// Takes the direction rather than returning something the caller resolves,
    /// so that "which of the two fields is set" stays inside this type. The
    /// argument is ignored when the alignment is physical, which is the common
    /// case.
    ///
    /// Named differently from the `alignment` builder because a getter and a
    /// setter cannot share one name.
    #[must_use]
    pub const fn resolved_alignment(&self, direction: TextDirection) -> Alignment {
        match self.alignment_directional {
            Some(directional) => directional.resolve(direction),
            None => self.alignment,
        }
    }

    /// How children are sized.
    #[must_use]
    pub const fn stack_fit(&self) -> StackFit {
        self.fit
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Stack {
    fn debug_name(&self) -> &'static str {
        "Stack"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderMultiChild(&self.children)
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        // The unresolved value, as written. A dump that printed `topLeft` where
        // the source says `topStart` would disagree with the code on exactly
        // the screens the directional form exists for.
        let mut props = vec![match self.alignment_directional {
            Some(directional) => ("alignment", directional.to_string()),
            None => ("alignment", self.alignment.to_string()),
        }];
        if self.fit != StackFit::default() {
            props.push(("fit", format!("{:?}", self.fit)));
        }
        props
    }
}

widget_node_from!(Stack);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_physical_corner_is_that_corner_in_every_locale() {
        // The divergence, asserted rather than left to a doc
        // comment: the classic stack alignment defaults to directional top-start and mirrors on
        // its own. Here `TOP_LEFT` means top left, so no `Stack` already
        // written moves when a `Directionality` appears above it.
        let stack = Stack::new().alignment(Alignment::TOP_LEFT);
        assert_eq!(
            stack.resolved_alignment(TextDirection::Rtl),
            Alignment::TOP_LEFT
        );
        assert_eq!(
            stack.resolved_alignment(TextDirection::Ltr),
            Alignment::TOP_LEFT
        );
    }

    #[test]
    fn the_default_stack_does_not_mirror_either() {
        // `new()` sets TOP_LEFT, and the default is the case most likely to be
        // changed by accident later.
        assert_eq!(
            Stack::new().resolved_alignment(TextDirection::Rtl),
            Alignment::TOP_LEFT
        );
    }

    #[test]
    fn a_directional_corner_follows_the_reading_direction() {
        let stack = Stack::new().alignment_directional(AlignmentDirectional::TOP_START);
        assert_eq!(
            stack.resolved_alignment(TextDirection::Ltr),
            Alignment::TOP_LEFT
        );
        assert_eq!(
            stack.resolved_alignment(TextDirection::Rtl),
            Alignment::TOP_RIGHT,
            "a badge pinned where reading begins moves to the other corner"
        );
    }

    #[test]
    fn the_two_alignments_are_one_property_and_the_last_call_wins() {
        // Each setter clears the other, so a stack can never hold both and
        // leave "which applies" to the order the factory happens to read them.
        let directional_last = Stack::new()
            .alignment(Alignment::BOTTOM_RIGHT)
            .alignment_directional(AlignmentDirectional::TOP_START);
        assert_eq!(
            directional_last.resolved_alignment(TextDirection::Rtl),
            Alignment::TOP_RIGHT
        );

        let physical_last = Stack::new()
            .alignment_directional(AlignmentDirectional::TOP_START)
            .alignment(Alignment::BOTTOM_RIGHT);
        assert_eq!(
            physical_last.resolved_alignment(TextDirection::Rtl),
            Alignment::BOTTOM_RIGHT,
            "a physical corner set last is not re-read as directional"
        );
    }

    #[test]
    fn a_tree_dump_prints_the_alignment_as_it_was_written() {
        let directional =
            crate::debug_tree(Stack::new().alignment_directional(AlignmentDirectional::BOTTOM_END));
        assert!(directional.contains("bottomEnd"), "{directional}");

        let physical = crate::debug_tree(Stack::new().alignment(Alignment::BOTTOM_RIGHT));
        assert!(physical.contains("bottomRight"), "{physical}");
    }
}
