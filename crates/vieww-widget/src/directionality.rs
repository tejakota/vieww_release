//! Which way the interface reads, published to a subtree.
//!
//! The direction a *paragraph* runs is already settled by the text layer, which
//! infers it from the characters. This is the other half: which way the
//! *interface* is laid out — which edge a back arrow sits on, which side a
//! switch's thumb travels to, which end of a row a badge hangs off. That is not
//! inferable from any one string, so somebody has to say it, and this is where.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::Directionality;
//! use vieww_foundation::TextDirection;
//!
//! let app = Directionality::new(TextDirection::Rtl).child(Text::new("مرحبا"));
//! ```
//!
//! The mechanism is [`Inherited<T>`](crate::Inherited), the same one [`Theme`]
//! uses, for the same reason: a direction is scoped, an inner one shadows an
//! outer one, and a subtree that pins itself to `Ltr` — a code block, a phone
//! number, a licence plate — is a supported thing to write rather than a
//! special case.
//!
//! # Why the default is LTR rather than "unknown"
//!
//! [`TextDirection`] has no third state, and adding one would push an
//! `Option` into every widget that resolves an alignment. The cost of the
//! default being wrong is a mirrored layout in an app that forgot to say; the
//! cost of `Option` is every call site handling a case that a real application
//! never produces, since an app that supports Arabic sets this at the root.
//!
//! [`Theme`]: crate::Theme

use std::rc::Rc;

use vieww_foundation::{Key, TextDirection};

use crate::{widget_node_from, BuildContext, Inherited, Widget, WidgetKind, WidgetNode};

/// Publishes a [`TextDirection`] to its subtree.
#[derive(Debug, Clone)]
pub struct Directionality {
    direction: Rc<TextDirection>,
    child: WidgetNode,
    key: Option<Key>,
}

impl Directionality {
    #[must_use]
    pub fn new(direction: TextDirection) -> Self {
        Self {
            direction: Rc::new(direction),
            child: crate::SizedBox::shrink().into(),
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = child.into();
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The direction in force at this position, or [`TextDirection::Ltr`].
    ///
    /// Never fails, for the same reason [`ThemeData::of`](crate::ThemeData::of)
    /// never fails: a widget built with no `Directionality` above it — in a
    /// test, in a tree dump, in the first five minutes of an application —
    /// should lay out left to right rather than panic.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> TextDirection {
        // Copied out rather than handed back as an `Rc`, unlike `ThemeData`: a
        // `TextDirection` is two words and every caller immediately passes it to
        // a `resolve`, so sharing it would be an allocation and a deref to avoid
        // copying one byte.
        ctx.inherit::<TextDirection>()
            .map_or(TextDirection::Ltr, |direction| *direction)
    }
}

impl Widget for Directionality {
    fn debug_name(&self) -> &'static str {
        "Directionality"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Inherited::shared(Rc::clone(&self.direction), self.child.clone()).into()
    }
}

widget_node_from!(Directionality);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_with_nothing_above_it_reads_left_to_right() {
        assert_eq!(
            Directionality::of(&BuildContext::root()),
            TextDirection::Ltr
        );
    }

    #[test]
    fn a_direction_reaches_a_descendant() {
        let root = BuildContext::root().child_with(Rc::new(TextDirection::Rtl));
        assert_eq!(Directionality::of(&root), TextDirection::Rtl);
    }

    #[test]
    fn an_inner_direction_shadows_an_outer_one() {
        // The case this exists for: a phone number or a code block inside an
        // Arabic screen reads left to right, and pinning it must not require
        // unwinding the ancestor.
        let arabic = BuildContext::root().child_with(Rc::new(TextDirection::Rtl));
        let pinned = arabic.child_with(Rc::new(TextDirection::Ltr));
        assert_eq!(Directionality::of(&arabic), TextDirection::Rtl);
        assert_eq!(Directionality::of(&pinned), TextDirection::Ltr);
    }

    #[test]
    fn it_publishes_through_the_ordinary_inherited_mechanism() {
        // Asserted so that a future refactor cannot quietly make direction a
        // special case in the element tree: it must stay something the tree
        // dump can see, like a theme.
        let dump = crate::debug_tree(
            Directionality::new(TextDirection::Rtl).child(crate::Text::new("hi")),
        );
        assert!(dump.contains("Directionality"), "{dump}");
        assert!(dump.contains("Inherited<TextDirection>"), "{dump}");
    }
}

/// Reading the system accessibility preferences from the tree.
///
/// A namespace rather than a widget: unlike [`Directionality`], nothing in an
/// application declares these — they come from the OS, are published once by
/// `FrameDriver::set_accessibility`, and are only ever *read* here. A widget
/// that wanted to override them for a subtree can publish an
/// [`Accessibility`](vieww_foundation::Accessibility) with `Inherited`
/// directly, which is what a settings screen previewing a text size does.
#[derive(Debug)]
pub struct AccessibilityOf;

impl AccessibilityOf {
    /// The preferences in force at this position, or the defaults.
    ///
    /// Never fails, for the reason [`Directionality::of`] never fails: a widget
    /// built with nothing above it should render with no preferences applied
    /// rather than panic — and the default applies none.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> vieww_foundation::Accessibility {
        ctx.inherit::<vieww_foundation::Accessibility>()
            .map_or_else(Default::default, |a| *a)
    }
}
