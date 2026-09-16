use vieww_foundation::{Constraints, Size};

use crate::{LayoutCtx, Liveness, RenderObject, Role, SemanticAction, Semantics};

/// Announces its subtree as one thing, with a name.
///
/// Layout-transparent: it passes its constraints straight through and takes its
/// child's size, so wrapping something in it cannot move a pixel. Everything it
/// does happens in [`RenderObject::semantics`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderSemantics {
    pub role: Role,
    pub label: Option<String>,
    pub value: Option<String>,
    pub toggled: Option<bool>,
    /// Whether the thing will respond at all. See [`Semantics::enabled`].
    pub enabled: bool,
    /// Whether this speaks *for* its subtree or merely *about* it.
    pub merge: bool,
    /// Whether a screen reader reads this out unprompted, against what the role
    /// implies. `None` — the usual — takes the role's answer; see
    /// [`Liveness::for_role`].
    pub live: Option<Liveness>,
}

impl RenderSemantics {
    #[must_use]
    pub const fn new(role: Role) -> Self {
        Self {
            role,
            label: None,
            value: None,
            toggled: None,
            enabled: true,
            merge: true,
            live: None,
        }
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    #[must_use]
    pub const fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Whether the thing this annotates will respond.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether this node replaces its descendants' semantics or contains them.
    #[must_use]
    pub const fn merge(mut self, merge: bool) -> Self {
        self.merge = merge;
        self
    }

    /// Override the liveness the role implies. See [`Liveness`].
    #[must_use]
    pub const fn live(mut self, live: Liveness) -> Self {
        self.live = Some(live);
        self
    }
}

impl RenderObject for RenderSemantics {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Transparent in both directions: the child gets exactly what we got,
        // and we become exactly what it chose. An annotation that changed the
        // layout would make adding accessibility a visual regression, which is
        // the surest way to have it removed again.
        match ctx.children().first().copied() {
            Some(child) => {
                let size = ctx.layout_child(child, constraints);
                ctx.place_child(child, vieww_foundation::Offset::ZERO);
                size
            }
            None => constraints.smallest(),
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        // Derived from the role rather than declared per control, because the
        // role is already the answer: everything announced as a button can be
        // activated, and a control that had to say so twice would eventually say
        // it once. What actually carries the action out is found by walking into
        // the subtree — see `RenderObject::handle_semantic_action`.
        let actions: &[SemanticAction] = match self.role {
            Role::Button | Role::CheckBox | Role::Switch | Role::Radio | Role::Tab => {
                &[SemanticAction::Activate]
            }
            Role::Slider => &[SemanticAction::Increment, SemanticAction::Decrement],
            Role::ScrollView => &[
                SemanticAction::ScrollForward,
                SemanticAction::ScrollBackward,
            ],
            _ => &[],
        };
        let mut semantics = Semantics {
            role: self.role,
            label: self.label.clone(),
            value: self.value.clone(),
            toggled: self.toggled,
            enabled: self.enabled,
            actions: Vec::new(),
            live: self.live,
        };
        semantics.actions.extend_from_slice(actions);
        Some(semantics)
    }

    fn merges_descendant_semantics(&self) -> bool {
        // Merging gives "Submit, button" rather than "button" then "Submit". A
        // screen reader stopping twice on one control is the single most common
        // accessibility complaint about custom UI, which is why it is the
        // default.
        //
        // It is exactly wrong for a *container*, though — a dialog that merged
        // would announce its title and swallow its own buttons, leaving a modal
        // nobody using a screen reader can answer. Hence the flag.
        self.merge
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderSemantics"
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderSemantics, Role, SemanticAction};
    use crate::RenderObject;

    fn actions(role: Role) -> Vec<SemanticAction> {
        RenderSemantics::new(role)
            .semantics()
            .expect("an annotation always declares a node")
            .actions
    }

    #[test]
    fn everything_a_user_chooses_between_can_be_activated() {
        // The table above derives actions from the role, so a role added
        // without a thought for it falls into the `_ => &[]` default and
        // becomes a node a screen reader can read and cannot use. That is a
        // silent failure — the control looks correctly annotated in a tree
        // dump — so each of these is named rather than assumed.
        for role in [
            Role::Button,
            Role::CheckBox,
            Role::Switch,
            Role::Radio,
            Role::Tab,
        ] {
            assert!(
                actions(role).contains(&SemanticAction::Activate),
                "{role:?} is something a user picks, so a screen reader must be \
                 able to pick it"
            );
        }
    }

    #[test]
    fn nothing_is_offered_on_a_node_that_cannot_carry_it_out() {
        // Advertising an action that silently fails is worse than not offering
        // one: the user tries it, nothing happens, and they conclude the app is
        // broken rather than that the feature is missing.
        for role in [Role::Label, Role::Group, Role::ProgressBar] {
            assert!(actions(role).is_empty(), "{role:?} has nothing to do");
        }
    }
}
