//! A devtools-facing summary of a [`vieww_render::SemanticsTree`].
//!
//! The semantics tree is what a screen reader actually sees — see that
//! type's own module docs on why it cannot be derived from the render tree.
//! This module answers the questions an accessibility audit asks of it: how
//! many nodes are there, what roles do they use, which interactive nodes
//! forgot a label (the single most common accessibility bug — a button a
//! screen reader can reach but can only announce as "button", with nothing
//! to say what it does), how many nodes take focus, and how many are live
//! regions that will interrupt or announce themselves. Nothing here
//! re-derives the tree; it only counts what [`SemanticsTree::nodes`] already
//! reports.

use vieww_foundation::FastMap;
use vieww_render::{Role, SemanticsTree};

/// Whether a role is one a sighted user would expect to be able to interact
/// with — the set this module checks unlabeled nodes against.
///
/// Deliberately narrower than "every non-`Group`/non-`Label` role": a
/// [`Role::ScrollView`] with no label is completely ordinary (nobody expects
/// a screen reader to announce a name for the page they are scrolling), so
/// including it here would flood a real audit with nodes that are not bugs.
/// The roles below are exactly the ones a screen reader announces as
/// *actionable*, where landing on one with nothing to say is the failure
/// `docs/AIMS.md`'s accessibility aims exist to catch.
#[must_use]
pub fn is_interactive(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::TextField
            | Role::CheckBox
            | Role::Radio
            | Role::Switch
            | Role::Slider
            | Role::Tab
    )
}

/// A summary over one [`SemanticsTree`] snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticsReport {
    pub total_nodes: usize,
    /// How many nodes declared each [`Role`]. A [`FastMap`] rather than a
    /// fixed per-variant struct because [`Role`] is `#[non_exhaustive]` and
    /// carries a [`Role::Custom`] payload — a report has to be able to count
    /// a role it has never heard of by name, not just the ones this crate
    /// happens to enumerate.
    pub role_histogram: FastMap<Role, usize>,
    /// Interactive nodes ([`is_interactive`]) with no
    /// [`label`](vieww_render::SemanticsNode::label) — a button a screen
    /// reader can reach and activate but can only announce as "button",
    /// with nothing to say what it does.
    pub unlabeled_interactive_count: usize,
    /// How many nodes can take keyboard focus
    /// ([`focusable`](vieww_render::SemanticsNode::focusable)).
    pub focusable_count: usize,
    /// How many nodes are live regions — read out unprompted, per
    /// [`Liveness::is_live`](vieww_render::Liveness::is_live).
    pub live_region_count: usize,
}

impl SemanticsReport {
    /// Summarize `tree` as it stands right now.
    #[must_use]
    pub fn build(tree: &SemanticsTree) -> Self {
        let mut role_histogram: FastMap<Role, usize> = FastMap::default();
        let mut unlabeled_interactive_count = 0;
        let mut focusable_count = 0;
        let mut live_region_count = 0;

        for node in tree.nodes() {
            *role_histogram.entry(node.role).or_insert(0) += 1;
            if is_interactive(node.role) && node.label.is_none() {
                unlabeled_interactive_count += 1;
            }
            if node.focusable {
                focusable_count += 1;
            }
            if node.live.is_live() {
                live_region_count += 1;
            }
        }

        Self {
            total_nodes: tree.nodes().len(),
            role_histogram,
            unlabeled_interactive_count,
            focusable_count,
            live_region_count,
        }
    }

    /// How many nodes declared `role`. Zero for a role nothing declared,
    /// which is the common case for most of the enum on any given screen.
    #[must_use]
    pub fn count_of(&self, role: Role) -> usize {
        self.role_histogram.get(&role).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{Constraints, Size};
    use vieww_render::{LayoutCtx, RenderObject, RenderSemantics, RenderTree, Semantics};

    /// Wraps a [`RenderSemantics`] to make it focusable.
    ///
    /// `RenderSemantics` itself never answers `true` from `is_focusable` —
    /// that is a property of the interactive object it annotates (a real
    /// button's `GestureDetector`, say), not of the annotation. This is the
    /// same shape relationship in miniature, real enough to exercise
    /// `SemanticsTree::build`'s `object.is_focusable()` call honestly rather
    /// than faking a `focusable: true` field directly on a `SemanticsNode`.
    #[derive(Debug)]
    struct Focusable(RenderSemantics);

    impl RenderObject for Focusable {
        fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            self.0.layout(ctx, constraints)
        }

        fn semantics(&self) -> Option<Semantics> {
            self.0.semantics()
        }

        fn merges_descendant_semantics(&self) -> bool {
            self.0.merges_descendant_semantics()
        }

        fn is_focusable(&self) -> bool {
            true
        }

        fn debug_name(&self) -> &'static str {
            "Focusable"
        }
    }

    /// Builds a small real tree — a single-child chain, the same shape
    /// `vieww-render`'s own semantics tests use (see `semantics.rs`'s
    /// `chain` helper), because `RenderSemantics::layout` only lays out its
    /// *first* child: siblings under one parent would simply never be
    /// placed. Each level explicitly turns off merging
    /// (`RenderSemantics::new` defaults to `merge: true`), or every node
    /// below the first would be swallowed into it rather than reported
    /// separately — exactly what
    /// `crate::semantics::tests::merging_a_subtree_drops_exactly_the_nodes_underneath_it`
    /// exists to prove happens when merge is left on.
    ///
    /// The outermost node is a plain, undeclared-label `Group` rather than
    /// one of the buttons below — `SemanticsTree::build` always reports the
    /// *root* node's role as [`Role::Window`], overwriting whatever it
    /// declared ("a declared root is still the window as far as a platform
    /// is concerned", per that function's own comment). Making a `Button`
    /// the root would silently turn it into a `Window` in the histogram,
    /// which is correct platform behaviour but would make this fixture's
    /// own role counts wrong on their face.
    ///
    /// Mixes labeled/unlabeled, interactive/non-interactive, focusable/not,
    /// and live/not so every counter in the report has something real to
    /// count.
    fn sample_tree() -> SemanticsTree {
        let mut tree = RenderTree::new();

        // The root: forced to `Role::Window` by `SemanticsTree::build`
        // regardless of what is declared here — see this function's own
        // docs.
        let root = RenderSemantics::new(Role::Group).merge(false);
        let root_id = tree.insert(None, Box::new(root));

        // A labeled button: interactive, focusable, correctly labeled.
        let button = Focusable(
            RenderSemantics::new(Role::Button)
                .label("Submit")
                .merge(false),
        );
        let button_id = tree.insert(Some(root_id), Box::new(button));

        // An unlabeled button: interactive, the bug this report exists to
        // surface.
        let unlabeled_button = Focusable(RenderSemantics::new(Role::Button).merge(false));
        let unlabeled_button_id = tree.insert(Some(button_id), Box::new(unlabeled_button));

        // A checkbox with no label: also interactive and unlabeled.
        let checkbox = Focusable(RenderSemantics::new(Role::CheckBox).merge(false));
        let checkbox_id = tree.insert(Some(unlabeled_button_id), Box::new(checkbox));

        // A plain label: not interactive, so an absent label (it has one
        // here anyway) would not count even if missing.
        let label = RenderSemantics::new(Role::Label)
            .label("Status: ready")
            .merge(false);
        let label_id = tree.insert(Some(checkbox_id), Box::new(label));

        // A live status region, deepest so it needs no `merge(false)` of its
        // own — it has no descendants to swallow.
        let status = RenderSemantics::new(Role::Custom("status")).label("Saved");
        tree.insert(Some(label_id), Box::new(status));

        let _ = tree.layout_root(Constraints::tight(Size::new(200.0, 200.0)));
        SemanticsTree::build(&tree, None)
    }

    #[test]
    fn total_node_count_matches_every_node_that_declared_semantics() {
        let semantics = sample_tree();
        let report = SemanticsReport::build(&semantics);
        // root(Window) + button + unlabeled_button + checkbox + label +
        // status = 6
        assert_eq!(report.total_nodes, 6);
        assert_eq!(report.total_nodes, semantics.nodes().len());
    }

    #[test]
    fn role_histogram_counts_each_role_exactly() {
        let semantics = sample_tree();
        let report = SemanticsReport::build(&semantics);

        assert_eq!(
            report.count_of(Role::Window),
            1,
            "the root, whatever it declared"
        );
        assert_eq!(
            report.count_of(Role::Group),
            0,
            "overwritten to Window by SemanticsTree::build"
        );
        assert_eq!(report.count_of(Role::Button), 2);
        assert_eq!(report.count_of(Role::CheckBox), 1);
        assert_eq!(report.count_of(Role::Label), 1);
        assert_eq!(report.count_of(Role::Custom("status")), 1);
        assert_eq!(
            report.count_of(Role::Switch),
            0,
            "a role nothing declared is zero"
        );
    }

    #[test]
    fn unlabeled_interactive_nodes_are_counted_exactly() {
        let semantics = sample_tree();
        let report = SemanticsReport::build(&semantics);
        // The unlabeled button and the unlabeled checkbox; the labeled
        // button and the (non-interactive) label do not count.
        assert_eq!(report.unlabeled_interactive_count, 2);
    }

    #[test]
    fn focusable_count_matches_the_declared_focusable_nodes() {
        let semantics = sample_tree();
        let report = SemanticsReport::build(&semantics);
        // button, unlabeled_button, checkbox are focusable; label and status
        // are not.
        assert_eq!(report.focusable_count, 3);
    }

    #[test]
    fn live_region_count_matches_liveness_for_role() {
        let semantics = sample_tree();
        let report = SemanticsReport::build(&semantics);
        // Only the `Custom("status")` node is live by
        // `Liveness::for_role`; nothing else declared an override.
        assert_eq!(report.live_region_count, 1);
    }

    #[test]
    fn an_empty_tree_reports_all_zeros() {
        let tree = RenderTree::new();
        let semantics = SemanticsTree::build(&tree, None);
        let report = SemanticsReport::build(&semantics);

        assert_eq!(report.total_nodes, 0);
        assert_eq!(report.unlabeled_interactive_count, 0);
        assert_eq!(report.focusable_count, 0);
        assert_eq!(report.live_region_count, 0);
    }

    #[test]
    fn is_interactive_matches_the_documented_role_set() {
        assert!(is_interactive(Role::Button));
        assert!(is_interactive(Role::TextField));
        assert!(is_interactive(Role::CheckBox));
        assert!(is_interactive(Role::Radio));
        assert!(is_interactive(Role::Switch));
        assert!(is_interactive(Role::Slider));
        assert!(is_interactive(Role::Tab));
        assert!(!is_interactive(Role::Group));
        assert!(!is_interactive(Role::Label));
        assert!(!is_interactive(Role::ScrollView));
        assert!(!is_interactive(Role::Window));
        assert!(!is_interactive(Role::Custom("status")));
    }
}
