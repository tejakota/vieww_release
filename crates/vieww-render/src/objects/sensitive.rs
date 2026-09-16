use vieww_foundation::{Color, Constraints, Offset, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject, Semantics};

/// A subtree that disappears while anything other than the screen is reading the
/// surface.
///
/// `docs/AIMS.md` §D, the *Correct* half: a framework that knows which subtree
/// is sensitive can suppress it **from screenshots, from the recents thumbnail
/// and from the semantics tree in one declaration**. This is the object that
/// makes those one thing rather than three.
///
/// # The whole implementation is one hook, and that is the point
///
/// [`skips_children`](RenderObject::skips_children) already means *do not paint,
/// do not hit test, do not give layers, do not announce* — see
/// [`RenderOffstage`](crate::RenderOffstage), which was built for a covered
/// route. Masking is exactly that set, so masking is exactly that hook. Building
/// three separate suppressions would have produced three chances to cover two of
/// them, which is the bug in every application that handles the screenshot and
/// forgets the thumbnail.
///
/// # Why the child is still laid out
///
/// Unlike `Offstage`, the geometry must not change. A masked subtree that
/// collapsed to nothing would reflow everything around it, so the mask would be
/// *visible in the screenshot as a different layout* — which leaks the shape of
/// what was hidden and, worse, is what the user sees for one frame when the
/// screenshot is taken. The child is measured and placed exactly as it would
/// have been; only the painting, the touches and the announcements stop.
///
/// # Why a cover is painted rather than nothing
///
/// A hole in a screenshot is a hole: whatever the previous layer left in those
/// pixels shows through, which on a reused surface is the previous frame's
/// contents. Filling the bounds is the only way to be sure the sensitive pixels
/// are gone, and it is also what the platforms do — `FLAG_SECURE` blacks the
/// window rather than making it transparent.
///
/// # Why it still declares semantics of its own
///
/// So a screen reader hears *that something is here and hidden* rather than
/// silence. A blind user recording their screen for support has the same right
/// to know the balance is masked as a sighted one, and a subtree that vanished
/// without a word would read as an application bug.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSensitive {
    /// `true` while something other than the screen is reading the surface.
    ///
    /// A property rather than a separate object, for `RenderOffstage`'s reason:
    /// masking must not reparent, or the state inside the sensitive subtree —
    /// a half-typed card number — is destroyed by the screenshot.
    pub masked: bool,
    /// What the mask is filled with. Opaque, or it is not a mask.
    pub cover: Color,
    /// What a screen reader is told is here. `None` says nothing beyond the
    /// role.
    pub label: Option<String>,
}

impl RenderSensitive {
    #[must_use]
    pub const fn new(masked: bool, cover: Color) -> Self {
        Self {
            masked,
            cover,
            label: None,
        }
    }

    #[must_use]
    pub fn with_label(mut self, label: Option<String>) -> Self {
        self.label = label;
        self
    }
}

impl RenderObject for RenderSensitive {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        // Laid out whether masked or not. See the type docs: a mask that changed
        // the geometry would be visible as a reflow in the very frame it is
        // meant to be invisible in.
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        if !self.masked {
            return;
        }
        let bounds = ctx.bounds();
        ctx.canvas().fill_rect(bounds, self.cover.into());
    }

    fn skips_children(&self) -> bool {
        self.masked
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The cover is opaque and absorbs the tap, rather than letting it fall
        // through to whatever is painted behind the masked subtree. A tap that
        // reached the screen underneath would be acting on something the user
        // cannot see.
        self.masked
    }

    fn semantics(&self) -> Option<Semantics> {
        if !self.masked {
            return None;
        }
        let mut declared = Semantics::new(crate::Role::Custom("note"));
        declared.label = self.label.clone();
        Some(declared)
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // Masking never changes this object's size — the child is laid out
        // either way — but `skips_children` gates *paint*, and the tree reads
        // that during the paint walk rather than the layout one. Comparing by
        // equality here would ask for a relayout that changes nothing.
        let _ = new;
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderSensitive"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderColoredBox, RenderId, RenderTree, SemanticsTree};

    fn tree_with(masked: bool) -> (RenderTree, RenderId, RenderId) {
        let mut tree = RenderTree::new();
        let root = tree.insert(
            None,
            Box::new(RenderSensitive::new(masked, Color::BLACK).with_label(Some("hidden".into()))),
        );
        let child = tree.insert(Some(root), Box::new(RenderColoredBox::new(Color::RED)));
        let _ = tree.layout_root(Constraints::tight(Size::new(100.0, 40.0)));
        (tree, root, child)
    }

    #[test]
    fn masking_does_not_move_anything() {
        let (masked, _, masked_child) = tree_with(true);
        let (plain, _, plain_child) = tree_with(false);
        assert_eq!(
            masked.size(masked_child),
            plain.size(plain_child),
            "a mask that reflowed would be visible in the frame it is meant to \
             be invisible in"
        );
        assert_eq!(masked.size(masked_child), Size::new(100.0, 40.0));
    }

    #[test]
    fn a_masked_subtree_is_hidden_from_a_screen_reader_by_the_same_declaration() {
        let (tree, root, child) = tree_with(true);
        let semantics = SemanticsTree::build(&tree, None);
        assert!(
            semantics.node(child).is_none(),
            "one declaration covers the screenshot and the screen reader, or \
             it is two declarations and an application will make only one"
        );
        let _ = root;
    }

    #[test]
    fn an_unmasked_subtree_reads_normally() {
        let (tree, _, _) = tree_with(false);
        let semantics = SemanticsTree::build(&tree, None);
        assert!(
            !semantics.is_empty(),
            "the mask is a state, not a permanent exclusion"
        );
    }

    #[test]
    fn a_masked_subtree_says_that_it_is_there() {
        let (tree, _, _) = tree_with(true);
        let semantics = SemanticsTree::build(&tree, None);
        assert!(
            semantics.describe().contains("hidden"),
            "silence would read as an application bug; the label is how a blind \
             user learns the balance is masked. got: {}",
            semantics.describe()
        );
    }

    #[test]
    fn a_masked_subtree_takes_no_taps_and_lets_none_through() {
        let (tree, root, child) = tree_with(true);
        let hit = tree.hit_test(Offset::new(10.0, 10.0));
        let target = hit.target().map(|entry| entry.id);
        assert_eq!(
            target,
            Some(root),
            "the cover absorbs the tap rather than letting it reach what is \
             painted behind the thing the user cannot see"
        );
        assert_ne!(target, Some(child));
    }
}
