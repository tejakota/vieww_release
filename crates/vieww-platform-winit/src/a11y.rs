//! The semantics tree as [`AccessKit`](https://docs.rs/accesskit) sees it.
//!
//! `vieww-render` produces a [`SemanticsTree`]; AccessKit consumes a
//! `TreeUpdate`; the platform adapters under it turn that into UI Automation,
//! AT-SPI, NSAccessibility or the Android accessibility API. This module is the
//! first of those arrows and the only one this project writes — the rest is
//! exactly the part `docs/DESIGN.md` §9 says not to attempt from scratch.
//!
//! # Why the conversion is a free function
//!
//! Everything interesting about a11y here is *what a node becomes*, and that is
//! a pure function of the semantics tree. Keeping it separate from the adapter
//! means it can be tested by reading the result, on a machine with no screen
//! reader and no window — which is the only way this would ever be checked at
//! all.

use accesskit::{Node, NodeId, Rect as AccessRect, Role as AccessRole, Tree, TreeUpdate};
use vieww_render::{Liveness, Role, SemanticAction, SemanticsNode, SemanticsTree};

use crate::scale::Scale;

/// A render object's id as AccessKit's.
///
/// Both halves are packed in rather than the index alone: a slot that is reused
/// produces a new generation, and a screen reader that saw the old node must not
/// be handed the new one under the same id. Accessibility clients cache
/// aggressively, and a reused id is how a screen reader ends up reading a
/// deleted row.
#[must_use]
pub(crate) fn node_id(id: vieww_render::RenderId) -> NodeId {
    NodeId((u64::from(id.generation()) << 32) | u64::from(id.index()))
}

/// A [`Role::Custom`] name as AccessKit's role.
///
/// The whole point of the name being a name: this table is the only place that
/// knows AccessKit exists, so `vieww-render` stays free of an accessibility
/// backend and a platform without AccessKit can write its own table.
///
/// `None` for a name this bridge does not know, which the caller turns into a
/// plain group. **Saying less than we could is recoverable; saying something
/// untrue is not** — a screen reader announcing "button" for a toolbar teaches
/// the user to try activating it.
fn custom_role(name: &str) -> Option<AccessRole> {
    Some(match name {
        "tree" => AccessRole::Tree,
        "tree_item" => AccessRole::TreeItem,
        "menu" => AccessRole::Menu,
        "menu_item" => AccessRole::MenuItem,
        "toolbar" => AccessRole::Toolbar,
        "tooltip" => AccessRole::Tooltip,
        // AccessKit has no `StatusBar`. `ContentInfo` is the ARIA role a status
        // bar carries, and is what every platform maps that to.
        "status_bar" => AccessRole::ContentInfo,
        "list" => AccessRole::List,
        "list_item" => AccessRole::ListItem,
        "link" => AccessRole::Link,
        "heading" => AccessRole::Heading,
        "dialog" => AccessRole::Dialog,
        "alert" => AccessRole::Alert,
        "image" => AccessRole::Image,
        "tab_list" => AccessRole::TabList,
        "navigation" => AccessRole::Navigation,
        "search" => AccessRole::Search,
        "form" => AccessRole::Form,
        "article" => AccessRole::Article,
        "banner" => AccessRole::Banner,
        "meter" => AccessRole::Meter,
        "timer" => AccessRole::Timer,
        "log" => AccessRole::Log,
        "note" => AccessRole::Note,
        // `ComboBox` and not `EditableComboBox`: `Dropdown` picks from a fixed
        // list and has nowhere to type. The editable variant tells a screen
        // reader there is a text field here, and the user then hunts for one.
        "combobox" => AccessRole::ComboBox,
        _ => return None,
    })
}

/// Our role as AccessKit's.
fn role(role: Role) -> AccessRole {
    match role {
        Role::Custom(name) => custom_role(name).unwrap_or(AccessRole::Group),
        Role::Group => AccessRole::Group,
        Role::Label => AccessRole::Label,
        Role::TextField => AccessRole::TextInput,
        Role::Button => AccessRole::Button,
        Role::ScrollView => AccessRole::ScrollView,
        Role::CheckBox => AccessRole::CheckBox,
        Role::Radio => AccessRole::RadioButton,
        Role::ProgressBar => AccessRole::ProgressIndicator,
        Role::Tab => AccessRole::Tab,
        Role::Switch => AccessRole::Switch,
        Role::Slider => AccessRole::Slider,
        Role::Window => AccessRole::Window,
        // `Role` is `#[non_exhaustive]`, so adding one there must not stop this
        // compiling. A generic group is the safe direction: a screen reader says
        // less than it could rather than something untrue.
        _ => AccessRole::Group,
    }
}

/// Our liveness as AccessKit's, or `None` for a node that is not live.
///
/// `None` rather than `Live::Off` for [`Liveness::Off`], and the difference is
/// not cosmetic: AccessKit's `live` is an optional property, so leaving it
/// absent is how a node says the concept does not apply to it. Writing
/// `Live::Off` on every ordinary button would put a property on every node in
/// the tree to say nothing.
///
/// This is the far end of the wire `docs/AIMS.md` §J describes, **and it is the
/// whole wire** — which is worth stating because the obvious next step is wrong.
///
/// # Why [`FrameDriver::take_announcements`] is not called from this crate
///
/// It looks like the missing half. It is not: calling it here would make every
/// announcement happen **twice**.
///
/// AccessKit is tree-driven. Its adapters compute the same diff
/// [`SemanticsTree::announcements`] computes, from the `TreeUpdate` alone —
/// `accesskit_atspi_common` emits `ObjectEvent::Announcement` when a node with
/// a non-`Off` live property is added to the tree (`adapter.rs`, `add_node`)
/// and again when such a node's name changes (`node.rs`,
/// `notify_property_changes`). Read against `accesskit_atspi_common` 0.19.1 on
/// 2026-08-16. Setting `live` on the node, which [`to_node`] does, is therefore
/// the complete delivery mechanism on every platform AccessKit backs, and a
/// posted announcement on top of it is a second utterance of the same words.
///
/// So the two are **alternatives, not layers**, and a bridge picks one:
///
/// - AccessKit backends — this crate, on all five targets — publish `live` on
///   the node and let the adapter diff. Nothing calls `take_announcements`.
/// - A backend with no live-region concept of its own, speaking to a text-to-
///   speech engine directly, calls `take_announcements` once per frame and
///   speaks the result, and must then *not* also publish liveness to something
///   that would diff it again.
///
/// `take_announcements` is therefore live API with no caller in this
/// repository rather than dead code, and `crates/vieww/tests/announcements.rs`
/// is the test of the mechanism a second backend would use.
fn live(liveness: Liveness) -> Option<accesskit::Live> {
    match liveness {
        Liveness::Off => None,
        Liveness::Polite => Some(accesskit::Live::Polite),
        Liveness::Assertive => Some(accesskit::Live::Assertive),
        // `Liveness` is `#[non_exhaustive]`. Silence is the safe direction for
        // the reason the `role` fallback gives, sharpened: a screen reader that
        // interrupts for a reason the user cannot predict is one they turn off.
        _ => None,
    }
}

/// A [`SemanticAction::Custom`] name as AccessKit's action, both ways.
///
/// One table rather than two, so the halves cannot drift: a name advertised and
/// then not recognised on the way back is an action a screen reader can ask for
/// and never get.
const CUSTOM_ACTIONS: &[(&str, accesskit::Action)] = &[
    ("expand", accesskit::Action::Expand),
    ("collapse", accesskit::Action::Collapse),
    ("focus", accesskit::Action::Focus),
    ("blur", accesskit::Action::Blur),
    ("show_tooltip", accesskit::Action::ShowTooltip),
    ("hide_tooltip", accesskit::Action::HideTooltip),
    ("scroll_into_view", accesskit::Action::ScrollIntoView),
    ("scroll_left", accesskit::Action::ScrollLeft),
    ("scroll_right", accesskit::Action::ScrollRight),
];

/// One of our actions as AccessKit's, or `None` for one it cannot express.
///
/// **`None` means "do not advertise it"**, which is the opposite of what an
/// unknown [`Role`] does and right for the opposite reason: a role is a
/// description, and a vaguer one is still true; an action is a promise, and
/// approximating it as a click does something the user did not ask for.
fn to_action(action: SemanticAction) -> Option<accesskit::Action> {
    Some(match action {
        // "Do the default thing", which is what a double-tap in VoiceOver and a
        // double-tap in TalkBack both come through as.
        SemanticAction::Activate => accesskit::Action::Click,
        SemanticAction::Increment => accesskit::Action::Increment,
        SemanticAction::Decrement => accesskit::Action::Decrement,
        SemanticAction::ScrollForward => accesskit::Action::ScrollDown,
        SemanticAction::ScrollBackward => accesskit::Action::ScrollUp,
        SemanticAction::Custom(name) => {
            let found = CUSTOM_ACTIONS
                .iter()
                .find(|(known, _)| *known == name)
                .map(|&(_, action)| action);
            return found;
        }
        // `SemanticAction` is `#[non_exhaustive]`; a variant added there must not
        // stop this compiling, and silence is safer than a guessed action.
        _ => return None,
    })
}

/// One of AccessKit's actions as ours, or `None` for one we cannot carry out.
///
/// The unmapped ones are dropped rather than approximated. A screen reader that
/// asks for "expand" and gets a click has done something the user did not ask
/// for, which is worse than nothing happening.
#[must_use]
pub(crate) fn from_action(action: accesskit::Action) -> Option<SemanticAction> {
    match action {
        accesskit::Action::Click => Some(SemanticAction::Activate),
        accesskit::Action::Increment => Some(SemanticAction::Increment),
        accesskit::Action::Decrement => Some(SemanticAction::Decrement),
        accesskit::Action::ScrollDown => Some(SemanticAction::ScrollForward),
        accesskit::Action::ScrollUp => Some(SemanticAction::ScrollBackward),
        // The other half of `CUSTOM_ACTIONS`, read backwards. Without this a
        // widget could advertise "expand" and never be told when one was asked
        // for — which is worse than not advertising it, because the screen
        // reader offers the user something that then does nothing.
        other => CUSTOM_ACTIONS
            .iter()
            .find(|(_, known)| *known == other)
            .map(|&(name, _)| SemanticAction::Custom(name)),
    }
}

/// The render object an AccessKit node id refers to.
///
/// The inverse of [`node_id`], and it has to be exact: both halves are packed
/// in, so a stale id from a screen reader's cache resolves to a generation the
/// tree no longer has and finds nothing rather than finding the wrong node.
#[must_use]
pub(crate) fn render_id(id: NodeId, semantics: &SemanticsTree) -> Option<vieww_render::RenderId> {
    semantics
        .nodes()
        .iter()
        .map(|node| node.id)
        .find(|&candidate| node_id(candidate) == id)
}

/// The whole tree, as an update a platform adapter can apply.
///
/// `scale` converts our logical pixels into the physical ones every
/// accessibility API reports rectangles in — a screen reader draws its focus
/// rectangle in screen coordinates, and one that is a third of the way to the
/// right of the control is worse than none.
///
/// Returns `None` for a tree with no root, which is what a driver that has not
/// laid anything out yet produces. An adapter should skip the update rather than
/// publish an empty tree, because an empty tree tells a screen reader the window
/// is genuinely empty.
#[must_use]
pub(crate) fn tree_update(semantics: &SemanticsTree, scale: Scale) -> Option<TreeUpdate> {
    let root = semantics.root()?;
    let root_id = node_id(root);

    let nodes = semantics
        .nodes()
        .iter()
        .map(|node| (node_id(node.id), to_node(node, scale)))
        .collect();

    // Focus must be reported on every update, and must be a node that exists —
    // AccessKit panics otherwise. Focus that is not *in* the semantics tree
    // (a focusable object that declares no semantics) resolves to the root,
    // which is AccessKit's own convention for "nothing in particular".
    let focus = semantics
        .nodes()
        .iter()
        .find(|node| node.focused)
        .map_or(root_id, |node| node_id(node.id));

    Some(TreeUpdate {
        nodes,
        tree: Some(Tree::new(root_id)),
        tree_id: accesskit::TreeId::ROOT,
        focus,
    })
}

/// A tree with one empty window in it.
///
/// What to publish when there is nothing to publish. AccessKit requires *some*
/// root on every update, so a driver that has not laid anything out yet still
/// needs an answer, and "a window containing nothing" is the true one.
#[must_use]
pub(crate) fn empty_update() -> TreeUpdate {
    let root = NodeId(0);
    TreeUpdate {
        nodes: vec![(root, Node::new(AccessRole::Window))],
        tree: Some(Tree::new(root)),
        tree_id: accesskit::TreeId::ROOT,
        focus: root,
    }
}

fn to_node(node: &SemanticsNode, scale: Scale) -> Node {
    let mut out = Node::new(role(node.role));

    if let Some(label) = &node.label {
        // AccessKit is explicit that a node whose role *is* Label carries its
        // text in `value`, not in `label` — setting the wrong one makes some
        // screen readers announce nothing at all.
        if node.role == Role::Label {
            out.set_value(label.clone());
        } else {
            out.set_label(label.clone());
        }
    }
    if let Some(value) = &node.value {
        out.set_value(value.clone());
    }
    // A live node with a value and no label is announced by nobody, and this is
    // the line that fixes it.
    //
    // AccessKit's adapters announce a live node by its *name*, and a name is a
    // node's `label` — except for `Role::Label`, where it is the value, which
    // the branch above already handles. So a live node carrying only a value —
    // a `status` reporting "50%", a third party's `Custom("alert")` assembled
    // out of a value — reaches AT-SPI with `name: None`, and
    // `ObjectEvent::Announcement` is never emitted for it. Silence, from the one
    // kind of node whose entire reason for existing is to interrupt the user.
    //
    // `SemanticsTree::spoken` already decided what a live node says, and it
    // reads the value when there is no label. This makes the bridge agree with
    // it rather than quietly saying less than the tree does.
    //
    // Deliberately narrow — only when the node is live — because promoting a
    // value into the name of an *ordinary* node makes a screen reader read it
    // twice on the way past, once as the name and once as the value.
    if node.live.is_live() && node.label.is_none() && node.role != Role::Label {
        if let Some(value) = &node.value {
            out.set_label(value.clone());
        }
    }
    if let Some(toggled) = node.toggled {
        // Not a string: a screen reader phrases this itself, in the user's
        // language and in the words that go with the role — "on" for a switch,
        // "ticked" for a checkbox.
        //
        // A tab is the exception, and it is AccessKit's distinction rather than
        // ours: `toggled` is for things that are *on or off*, `selected` for one
        // chosen out of a set. Sending a tab as toggled makes VoiceOver announce
        // "on", which says nothing about which page is showing.
        if node.role == Role::Tab {
            out.set_selected(toggled);
        } else {
            out.set_toggled(accesskit::Toggled::from(toggled));
        }
    }

    if !node.enabled {
        // AccessKit's own spelling, and the reason this is a flag rather than a
        // word in the label: a screen reader says "dimmed" or "unavailable" in
        // the user's language, and knows not to offer an activate action.
        out.set_disabled();
    }

    let bounds = scale.to_physical_rect(node.bounds);
    out.set_bounds(AccessRect {
        x0: f64::from(bounds.left),
        y0: f64::from(bounds.top),
        x1: f64::from(bounds.right),
        y1: f64::from(bounds.bottom),
    });

    if let Some(live) = live(node.live) {
        out.set_live(live);
    }

    if node.focusable {
        out.add_action(accesskit::Action::Focus);
    }
    for &action in &node.actions {
        // Skipped rather than approximated when this bridge cannot express it.
        // An action advertised and not honoured is one a screen reader offers
        // the user and that then does nothing.
        if let Some(action) = to_action(action) {
            out.add_action(action);
        }
    }
    out.set_children(
        node.children
            .iter()
            .map(|&child| node_id(child))
            .collect::<Vec<_>>(),
    );

    out
}

#[cfg(test)]
mod tests {
    use vieww_render::RenderId;

    use super::*;

    /// A node with nothing but the fields a test cares about.
    fn node(role: Role) -> SemanticsNode {
        SemanticsNode::new(ids_differing_only_by_generation().0, role)
    }

    #[test]
    fn a_reused_slot_does_not_reuse_its_accessibility_id() {
        // Two ids for the same arena slot, one generation apart. A screen
        // reader caching the first must not be handed the second under it.
        let tree = vieww_render::RenderTree::new();
        let _ = &tree;
        let first = ids_differing_only_by_generation();
        assert_ne!(node_id(first.0), node_id(first.1));
    }

    /// Two `RenderId`s with the same index and different generations.
    ///
    /// Built through a real tree, because the fields are crate-private and
    /// faking them here would test the wrong thing.
    fn ids_differing_only_by_generation() -> (RenderId, RenderId) {
        use vieww_render::{RenderTree, Semantics};

        #[derive(Debug)]
        struct Leaf;
        impl vieww_render::RenderObject for Leaf {
            fn layout(
                &mut self,
                _ctx: &mut vieww_render::LayoutCtx<'_>,
                constraints: vieww_foundation::Constraints,
            ) -> vieww_foundation::Size {
                constraints.smallest()
            }
            fn semantics(&self) -> Option<Semantics> {
                Some(Semantics::label("leaf"))
            }
            fn debug_name(&self) -> &'static str {
                "Leaf"
            }
        }

        let mut tree = RenderTree::new();
        let first = tree.insert(None, Box::new(Leaf));
        tree.remove(first);
        let second = tree.insert(None, Box::new(Leaf));
        (first, second)
    }

    #[test]
    fn bounds_arrive_in_the_physical_pixels_a_screen_reader_draws_in() {
        let node = SemanticsNode {
            id: ids_differing_only_by_generation().0,
            role: Role::Button,
            label: Some("Submit".to_owned()),
            value: None,
            toggled: None,
            enabled: true,
            actions: Vec::new(),
            bounds: vieww_foundation::Rect::new(10.0, 20.0, 30.0, 40.0),
            focusable: true,
            focused: false,
            live: Liveness::Off,
            children: Vec::new(),
        };

        let converted = to_node(&node, Scale::new(2.0));
        let bounds = converted.bounds().expect("bounds are always set");

        assert!(
            (bounds.x0 - 20.0).abs() < 0.01 && (bounds.x1 - 60.0).abs() < 0.01,
            "a focus rectangle in the wrong coordinate space is worse than none: {bounds:?}"
        );
        assert_eq!(converted.label(), Some("Submit"));
    }

    #[test]
    fn a_label_carries_its_text_in_value_because_accesskit_says_so() {
        let node = SemanticsNode {
            id: ids_differing_only_by_generation().0,
            role: Role::Label,
            label: Some("hello".to_owned()),
            value: None,
            toggled: None,
            enabled: true,
            actions: Vec::new(),
            bounds: vieww_foundation::Rect::ZERO,
            focusable: false,
            focused: false,
            live: Liveness::Off,
            children: Vec::new(),
        };

        let converted = to_node(&node, Scale::ONE);
        assert_eq!(
            converted.value(),
            Some("hello"),
            "a Label with its text in `label` is announced as nothing by some \
             screen readers"
        );
    }

    #[test]
    fn a_switch_reports_its_state_as_a_flag_rather_than_a_word() {
        let node = SemanticsNode {
            id: ids_differing_only_by_generation().0,
            role: Role::Switch,
            label: Some("Wi-Fi".to_owned()),
            value: None,
            toggled: Some(true),
            enabled: true,
            actions: Vec::new(),
            bounds: vieww_foundation::Rect::ZERO,
            focusable: true,
            focused: false,
            live: Liveness::Off,
            children: Vec::new(),
        };

        let converted = to_node(&node, Scale::ONE);
        assert_eq!(converted.role(), AccessRole::Switch);
        assert!(
            !converted.is_disabled(),
            "an ordinary control must not read as unavailable"
        );
        assert_eq!(
            converted.toggled(),
            Some(accesskit::Toggled::True),
            "a screen reader says \"on\" in the user's language, not ours"
        );
    }

    #[test]
    fn a_live_region_reaches_accesskit_rather_than_stopping_at_the_tree() {
        // **The gap this closes.** `Liveness` on the semantics tree is what
        // decides an announcement, but a desktop screen reader learns it from
        // AccessKit. A tree that knows a snackbar is live and a bridge that
        // does not forward it is silence with a passing test suite behind it.
        let mut polite = node(Role::Group);
        polite.live = Liveness::Polite;
        assert_eq!(
            to_node(&polite, Scale::ONE).live(),
            Some(accesskit::Live::Polite)
        );

        let mut assertive = node(Role::Group);
        assertive.live = Liveness::Assertive;
        assert_eq!(
            to_node(&assertive, Scale::ONE).live(),
            Some(accesskit::Live::Assertive),
            "an error that does not interrupt is read after the user has acted \
             on the state it was warning about"
        );
    }

    #[test]
    fn a_live_node_with_only_a_value_still_has_something_to_announce() {
        // **The gap this closes.** AccessKit announces a live node by its
        // *name*, and a name is the `label`. A live node carrying only a value
        // — a status reporting a percentage, an alert assembled out of a value
        // — therefore reached AT-SPI with no name at all, and
        // `ObjectEvent::Announcement` is guarded by `if let Some(name)`. It said
        // nothing, from the one kind of node that exists to interrupt.
        let mut status = node(Role::Custom("status"));
        status.value = Some("50%".to_owned());
        assert!(status.label.is_none());

        let out = to_node(&status, Scale::ONE);
        assert_eq!(out.live(), Some(accesskit::Live::Polite));
        assert_eq!(
            out.label(),
            Some("50%"),
            "a live node with no name is a live node nobody hears"
        );
        assert_eq!(
            out.value(),
            Some("50%"),
            "and it is still a value, so navigating to it reads the same thing"
        );
    }

    #[test]
    fn a_label_is_never_invented_for_a_node_that_is_not_live() {
        // The narrowness is the point: promoting a value into the name of an
        // ordinary node makes a screen reader read it twice on the way past.
        let mut ordinary = node(Role::Button);
        ordinary.value = Some("50%".to_owned());
        assert_eq!(to_node(&ordinary, Scale::ONE).label(), None);
    }

    #[test]
    fn a_live_label_keeps_carrying_its_text_in_the_value() {
        // `Role::Label` is the one role where AccessKit takes the name *from*
        // the value, so the fallback above must not fire and put the text in
        // both — `label_comes_from_value` in `accesskit_consumer` is the rule.
        let mut label = node(Role::Label);
        label.live = Liveness::Polite;
        label.value = Some("saved".to_owned());
        let out = to_node(&label, Scale::ONE);
        assert_eq!(out.value(), Some("saved"));
        assert_eq!(out.label(), None);
    }

    #[test]
    fn a_live_node_with_a_label_is_left_exactly_as_it_was() {
        let mut alert = node(Role::Custom("alert"));
        alert.label = Some("Saved".to_owned());
        alert.value = Some("just now".to_owned());
        let out = to_node(&alert, Scale::ONE);
        assert_eq!(out.label(), Some("Saved"));
        assert_eq!(out.value(), Some("just now"));
    }

    #[test]
    fn an_ordinary_node_carries_no_liveness_property_at_all() {
        assert_eq!(
            to_node(&node(Role::Button), Scale::ONE).live(),
            None,
            "`Live::Off` on every button is a property per node to say nothing"
        );
    }

    #[test]
    fn a_role_that_is_live_by_default_arrives_live_without_anyone_setting_it() {
        // The §J claim end to end through this bridge: `SemanticsNode::new`
        // takes the liveness from the role, so a third party's alert is
        // announced without vieww or the application having said so.
        assert_eq!(
            to_node(&node(Role::Custom("alert")), Scale::ONE).live(),
            Some(accesskit::Live::Assertive)
        );
        assert_eq!(
            to_node(&node(Role::Custom("log")), Scale::ONE).live(),
            Some(accesskit::Live::Polite)
        );
    }

    #[test]
    fn a_role_this_framework_never_produces_still_reaches_the_platform() {
        // **The gap this closes.** A tree, a menu, a toolbar — controls written
        // outside vieww had no way to describe themselves, and the failure was
        // silent: the control worked, took taps, and was announced wrongly.
        assert_eq!(
            to_node(&node(Role::Custom("tree_item")), Scale::ONE).role(),
            AccessRole::TreeItem
        );
        assert_eq!(
            to_node(&node(Role::Custom("toolbar")), Scale::ONE).role(),
            AccessRole::Toolbar
        );
        assert_eq!(
            to_node(&node(Role::Custom("status_bar")), Scale::ONE).role(),
            AccessRole::ContentInfo,
            "AccessKit has no StatusBar; ContentInfo is the ARIA role one carries"
        );
    }

    #[test]
    fn an_unknown_role_name_degrades_to_a_group_rather_than_guessing() {
        // Saying less than we could is recoverable. Saying something untrue
        // teaches the user to try activating a thing that does nothing.
        assert_eq!(
            to_node(&node(Role::Custom("no_such_role")), Scale::ONE).role(),
            AccessRole::Group
        );
    }

    #[test]
    fn a_custom_action_survives_the_round_trip() {
        // Both directions, because one without the other is worse than neither:
        // advertising "expand" and not recognising the request for it offers the
        // user something that then does nothing.
        for &(name, expected) in CUSTOM_ACTIONS {
            let ours = SemanticAction::Custom(name);
            assert_eq!(to_action(ours), Some(expected), "{name} out");
            assert_eq!(from_action(expected), Some(ours), "{name} back");
        }
    }

    #[test]
    fn an_unknown_custom_action_is_not_advertised_at_all() {
        // The opposite of an unknown role, and right for the opposite reason: a
        // role is a description and degrades; an action is a promise and must
        // not turn into a different promise.
        assert_eq!(to_action(SemanticAction::Custom("teleport")), None);

        let mut node = node(Role::Button);
        node.actions = vec![SemanticAction::Custom("teleport")];
        assert!(
            !to_node(&node, Scale::ONE).supports_action(accesskit::Action::Click),
            "an unmappable action must not arrive as a click"
        );
    }

    #[test]
    fn a_tab_reports_which_page_is_showing_as_selected_rather_than_as_on() {
        // AccessKit's own distinction: `toggled` is for a thing that is on or
        // off, `selected` for one chosen out of a set. Both `TabBar` and
        // `BottomNavigation` carry it in `toggled` because that is the one field
        // the semantics tree has for "this one is the current one" — the
        // translation to AccessKit's vocabulary happens here.
        let mut showing = node(Role::Tab);
        showing.label = Some("Inbox".to_owned());
        showing.toggled = Some(true);

        let converted = to_node(&showing, Scale::ONE);
        assert_eq!(converted.role(), AccessRole::Tab);
        assert_eq!(
            converted.is_selected(),
            Some(true),
            "a tab is chosen, not switched on"
        );
        assert_eq!(
            converted.toggled(),
            None,
            "VoiceOver announcing \"on\" says nothing about which page is showing"
        );

        let mut hidden = node(Role::Tab);
        hidden.toggled = Some(false);
        assert_eq!(to_node(&hidden, Scale::ONE).is_selected(), Some(false));
    }

    #[test]
    fn an_empty_tree_produces_no_update_rather_than_an_empty_one() {
        let semantics = SemanticsTree::default();
        assert!(
            tree_update(&semantics, Scale::ONE).is_none(),
            "publishing an empty tree tells a screen reader the window is empty"
        );
    }

    #[test]
    fn a_disabled_control_is_announced_as_unavailable() {
        // The one part of a control's state that is otherwise carried by colour
        // alone. Without this a screen reader offers an activate action for a
        // button that will ignore it, and the user concludes the app is broken.
        let mut disabled = node(Role::Button);
        disabled.label = Some("Save".to_owned());
        disabled.enabled = false;

        assert!(to_node(&disabled, Scale::ONE).is_disabled());
        assert!(!to_node(&node(Role::Button), Scale::ONE).is_disabled());
    }
}
