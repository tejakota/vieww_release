//! Static audits over a [`SemanticsTree`] — no live app, no screen reader,
//! no GPU: exactly the kind of thing this framework's own `debug_tree` makes
//! possible for widget shapes, applied here to what a screen reader would
//! actually be told.
//!
//! # What is checked, and — as importantly — what was considered and dropped
//!
//! Three checks are implemented, each chosen because the real
//! [`SemanticsNode`] data genuinely supports it:
//!
//! - [`missing_labels`] — an interactive role with no label (or an empty
//!   one), which a screen reader announces as its role and nothing else:
//!   "button" rather than "Submit, button".
//! - [`undersized_touch_targets`] — an actionable node smaller than
//!   [`MIN_TOUCH_TARGET_SIDE`].
//! - [`inconsistent_state`] — two internal-consistency checks the framework's
//!   own derivation already promises to uphold (see each variant's doc).
//!
//! **A fourth check — "an action without a focusable node" — was
//! deliberately not built.** Read `RenderSemantics::semantics` and the
//! collect walk in `vieww_render::semantics::SemanticsTree::build`
//! carefully: [`SemanticsNode::focusable`] is `object.is_focusable()` on
//! *whichever render object declared this exact node* — for every composed
//! control built the way this framework's own `Button` is (a `Semantics`
//! annotation wrapping a separately-focusable `GestureDetector` several
//! layers inside it), that is always `false`, because the annotation itself
//! never overrides `is_focusable`. That is not a bug this crate can catch —
//! it means "action declared, node not focusable" is true of essentially
//! every interactive control in the framework today, built or not, which
//! makes it a check with no diagnostic power: a check that always fires
//! tells a caller nothing a check never firing would. Building it anyway
//! would be exactly the "wrong-looking pseudocode" this codebase's own
//! conventions warn against — it would *look* like real keyboard-reachability
//! verification while actually just restating a structural fact about how
//! `Semantics` and `GestureDetector` compose. If a future version of
//! `vieww-render` changes `SemanticsNode` to carry the *subtree's* real
//! keyboard reachability (rather than one object's own flag), this check
//! becomes buildable and should be added then.

use vieww_render::{Role, SemanticsNode, SemanticsTree};

/// The smallest side, in logical pixels, an actionable node's bounds should
/// be.
///
/// `44.0` matches WCAG 2.2's Success Criterion 2.5.5 (Target Size, AAA) and
/// Apple's Human Interface Guidelines (44pt minimum tap target); Android's
/// Android guidance is slightly stricter at 48dp. `44.0` is used here
/// as the common floor every platform this framework targets agrees is *at
/// least* required — a target that fails this check fails on every platform,
/// not just the strictest one.
pub const MIN_TOUCH_TARGET_SIDE: f32 = 44.0;

/// Roles a screen reader announces as something to act on, and therefore
/// something that needs a name to act on it *by*. Matches the role set
/// `RenderSemantics::semantics` derives an action for, plus [`Role::TextField`]
/// (focusable and editable, per `RenderEditableText::is_focusable`, but not
/// itself represented in that action-derivation table since typing is not a
/// discrete "activate").
const INTERACTIVE_ROLES: &[Role] = &[
    Role::Button,
    Role::TextField,
    Role::CheckBox,
    Role::Radio,
    Role::Switch,
    Role::Slider,
    Role::Tab,
];

fn is_interactive(role: Role) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

/// How serious a [`Finding`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// A screen reader user cannot use this control at all, or is actively
    /// misled about its state.
    Error,
    /// Usable, but degraded — a smaller-than-recommended target, a role that
    /// is technically announced but reads poorly.
    Warning,
}

/// One thing [`audit`] found wrong with a node.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Which node this is about. Carries `Display`/`Debug` (`vieww_render`'s
    /// own `r{index}v{generation}` form) so a report can name it without
    /// this crate needing its own node-naming scheme.
    pub node_id: vieww_render::RenderId,
    pub role: Role,
    pub severity: Severity,
    pub message: String,
}

/// Every [`missing_labels`], [`undersized_touch_targets`] and
/// [`inconsistent_state`] finding for `tree`, in that order.
///
/// Read-only: `tree` is walked exactly once per check, over
/// [`SemanticsTree::nodes`], the same public accessor a devtools panel or a
/// custom AccessKit adapter would use — nothing here reaches into
/// `vieww-render`'s internals.
#[must_use]
pub fn audit(tree: &SemanticsTree) -> Vec<Finding> {
    let nodes = tree.nodes();
    let mut findings = missing_labels(nodes);
    findings.extend(undersized_touch_targets(nodes));
    findings.extend(inconsistent_state(nodes));
    findings
}

/// Interactive nodes with no label, or a label that is only whitespace —
/// which is what a screen reader announces as silence after the role: a
/// button that reads as "button" and nothing else.
#[must_use]
pub fn missing_labels(nodes: &[SemanticsNode]) -> Vec<Finding> {
    nodes
        .iter()
        .filter(|node| is_interactive(node.role))
        .filter(|node| {
            node.label
                .as_deref()
                .is_none_or(|label| label.trim().is_empty())
        })
        .map(|node| Finding {
            node_id: node.id,
            role: node.role,
            severity: Severity::Error,
            message: format!("{:?} has no label a screen reader can announce", node.role),
        })
        .collect()
}

/// Nodes that offer at least one [`vieww_render::SemanticAction`] but whose
/// laid-out [`SemanticsNode::bounds`] are smaller than
/// [`MIN_TOUCH_TARGET_SIDE`] on either side.
///
/// Actions, not role, decide what counts as "actionable" here: a disabled
/// control has its actions cleared by the same collect walk that builds this
/// tree (see `vieww_render::semantics`'s own doc on why), so a disabled
/// button — nothing left to reach — is correctly not flagged even though its
/// role is still [`Role::Button`].
#[must_use]
pub fn undersized_touch_targets(nodes: &[SemanticsNode]) -> Vec<Finding> {
    nodes
        .iter()
        .filter(|node| !node.actions.is_empty())
        .filter(|node| {
            node.bounds.width() < MIN_TOUCH_TARGET_SIDE
                || node.bounds.height() < MIN_TOUCH_TARGET_SIDE
        })
        .map(|node| Finding {
            node_id: node.id,
            role: node.role,
            severity: Severity::Warning,
            message: format!(
                "{:?} target is {:.1}x{:.1}, smaller than the {:.0}x{:.0} minimum",
                node.role,
                node.bounds.width(),
                node.bounds.height(),
                MIN_TOUCH_TARGET_SIDE,
                MIN_TOUCH_TARGET_SIDE
            ),
        })
        .collect()
}

/// Two internal-consistency checks against invariants the framework's own
/// derivation already promises — see this module's own doc for the one
/// check ("action without focusable") that looked like a third but is not
/// actually supported by the data.
///
/// - A [`Role::Group`] or [`Role::Label`] node with a non-empty
///   [`SemanticsNode::actions`] — `RenderSemantics::semantics`'s own test,
///   `nothing_is_offered_on_a_node_that_cannot_carry_it_out`, asserts these
///   roles get no derived actions; a node that has one anyway came from a
///   [`Role::Custom`] object that hand-rolled its own `Semantics` and got
///   the role/action pairing wrong.
/// - A node that is [`SemanticsNode::focused`] while not
///   [`SemanticsNode::focusable`] — focus cannot land somewhere unreachable
///   by definition, so this is a data bug in whatever produced the tree
///   (most plausibly a stale `focused` id surviving a reconciliation that
///   changed what that id now points at).
#[must_use]
pub fn inconsistent_state(nodes: &[SemanticsNode]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for node in nodes {
        if matches!(node.role, Role::Group | Role::Label) && !node.actions.is_empty() {
            findings.push(Finding {
                node_id: node.id,
                role: node.role,
                severity: Severity::Error,
                message: format!(
                    "{:?} declares {} action(s) but this role should offer none",
                    node.role,
                    node.actions.len()
                ),
            });
        }
        if node.focused && !node.focusable {
            findings.push(Finding {
                node_id: node.id,
                role: node.role,
                severity: Severity::Error,
                message: "node is reported as focused but is not focusable".to_owned(),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::Size;
    use vieww_test_harness::TestHarness;
    use vieww_widget::{Button, Flex, SemanticRole, Semantics, SizedBox};

    /// Mounts `content` under a real, semantics-silent [`Flex`] rather than
    /// directly as the tree's root.
    ///
    /// `SemanticsTree::build` gives the render tree's *root* a hard-coded
    /// `Role::Window` regardless of what it declares — see its own doc, "A
    /// declared root is still the window as far as a platform is concerned."
    /// A real application's interactive controls are never literally the
    /// window's own root render object (there is always at least a scaffold
    /// or a page layout above them), so mounting `content` directly would
    /// test an unrealistic tree shape and silently launder its role into
    /// `Window` before this crate's checks ever see it.
    ///
    /// [`Flex`] specifically, not a layout-transparent wrapper like
    /// `Container::new()` with nothing set on it: this framework elides a
    /// wrapper that contributes nothing of its own straight through to its
    /// child at the render-tree level, which would leave `content`'s render
    /// object as the tree's literal root again — confirmed by mounting
    /// through one and finding the same `Role::Window` overwrite reaching
    /// `content`. `Flex` always produces a real render object of its own, so
    /// it is genuinely between `content` and the tree's synthetic root.
    fn semantics_tree(content: impl Into<vieww_widget::WidgetNode>) -> SemanticsTree {
        let mut harness = TestHarness::new(Size::new(400.0, 400.0));
        harness.mount(Flex::column().children([content.into()]));
        harness.tick_and_settle(8);
        harness.driver().semantics()
    }

    /// The framework's own `Button` widget always sets a label, is enabled,
    /// and — being wrapped in `crate::controls::touch_target`, per its own
    /// doc — is never smaller than the theme's minimum touch target. A clean
    /// audit against it is the "no false positives on real, correct widgets"
    /// half of this module's test coverage.
    #[test]
    fn a_real_button_produces_no_findings() {
        let tree = semantics_tree(Button::new("Save").on_pressed(|| {}));
        let findings = audit(&tree);
        assert!(
            findings.is_empty(),
            "unexpected findings against a correct real button: {findings:?}"
        );
    }

    #[test]
    fn a_button_with_no_label_is_flagged() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Button)
                .label("")
                .child(SizedBox::square(60.0)),
        );
        let findings = missing_labels(tree.nodes());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].role, Role::Button);
    }

    #[test]
    fn a_button_with_only_whitespace_is_also_flagged() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Button)
                .label("   ")
                .child(SizedBox::square(60.0)),
        );
        assert_eq!(missing_labels(tree.nodes()).len(), 1);
    }

    #[test]
    fn a_labelled_button_is_not_flagged_for_its_label() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Button)
                .label("Delete")
                .child(SizedBox::square(60.0)),
        );
        assert!(missing_labels(tree.nodes()).is_empty());
    }

    /// A plain `Label` role is never interactive, so an absent label there
    /// is not this check's business — flagging it would be a false positive
    /// on ordinary static text that simply has no text this frame (a
    /// spacer, a placeholder).
    #[test]
    fn a_label_role_with_no_text_is_not_flagged() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Label)
                .child(SizedBox::square(10.0)),
        );
        assert!(missing_labels(tree.nodes()).is_empty());
    }

    #[test]
    fn an_undersized_actionable_target_is_flagged() {
        // A bare `Semantics::button` (which always derives an `Activate`
        // action, per `RenderSemantics::semantics`) around a genuinely tiny
        // 10x10 child — well under `MIN_TOUCH_TARGET_SIDE`.
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Button)
                .label("Tiny")
                .child(SizedBox::square(10.0)),
        );
        let findings = undersized_touch_targets(tree.nodes());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn a_large_enough_actionable_target_is_not_flagged() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Button)
                .label("Big enough")
                .child(SizedBox::square(MIN_TOUCH_TARGET_SIDE)),
        );
        assert!(undersized_touch_targets(tree.nodes()).is_empty());
    }

    /// A non-interactive label is never checked for target size at all, even
    /// if it happens to be tiny — nothing about it is meant to be tapped.
    #[test]
    fn a_tiny_non_actionable_label_is_not_flagged_for_target_size() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Label)
                .label("x")
                .child(SizedBox::square(2.0)),
        );
        assert!(undersized_touch_targets(tree.nodes()).is_empty());
    }

    #[test]
    fn a_group_with_no_actions_is_not_flagged() {
        let tree = semantics_tree(
            Semantics::new()
                .role(SemanticRole::Group)
                .child(SizedBox::square(60.0)),
        );
        assert!(inconsistent_state(tree.nodes()).is_empty());
    }

    /// This exercises `inconsistent_state`'s hand-constructed path directly
    /// against a synthetic `SemanticsNode` — `RenderId` cannot be built
    /// outside `vieww-render` (its fields are `pub(crate)`), but
    /// `SemanticsNode` is a plain public struct constructible via
    /// `SemanticsNode::new`, and no real widget in this framework can
    /// currently produce the actions-on-a-Group state this checks for (it
    /// would take a hand-rolled `RenderObject` outside this crate's reach) —
    /// so this is the honest way to exercise the check at all.
    #[test]
    fn a_group_role_with_an_action_is_flagged_as_inconsistent() {
        let tree = semantics_tree(Button::new("Save").on_pressed(|| {}));
        let real_id = tree.nodes()[0].id;
        let mut broken = SemanticsNode::new(real_id, Role::Group);
        broken.actions = vec![vieww_render::SemanticAction::Activate];
        let findings = inconsistent_state(std::slice::from_ref(&broken));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
    }

    #[test]
    fn a_focused_but_not_focusable_node_is_flagged() {
        let tree = semantics_tree(Button::new("Save").on_pressed(|| {}));
        let real_id = tree.nodes()[0].id;
        let mut broken = SemanticsNode::new(real_id, Role::Button);
        broken.focused = true;
        broken.focusable = false;
        let findings = inconsistent_state(std::slice::from_ref(&broken));
        assert!(findings.iter().any(|f| f.message.contains("not focusable")));
    }

    #[test]
    fn a_full_audit_on_a_clean_tree_reports_nothing() {
        let tree = semantics_tree(Button::new("Save").on_pressed(|| {}));
        assert!(audit(&tree).is_empty());
    }
}
