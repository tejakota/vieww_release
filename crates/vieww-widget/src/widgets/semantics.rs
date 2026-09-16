use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// What a screen reader should say about a subtree.
///
/// Wrap anything whose meaning is not visible in its geometry — which is most
/// interactive things. A button is a coloured box with a label inside and a
/// gesture detector around it; nothing in that shape says "button", so somebody
/// has to.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Semantics, SemanticRole};
///
/// let submit = Semantics::new()
///     .role(SemanticRole::Button)
///     .label("Submit")
///     .child(Container::new().child(Text::new("Submit")));
/// ```
///
/// # By default it replaces its subtree's semantics rather than adding to them
///
/// A button labelled "Submit" containing text reading "Submit" should be *one*
/// thing a screen reader stops on, not two — otherwise every button in the
/// application is announced twice. So this drops what its descendants declared
/// and speaks for them, which is the merge-semantics behaviour folded
/// into the annotation rather than left as a second widget to remember.
///
/// That is exactly wrong for anything with interactive contents. A dialog
/// annotated this way announces its title and *swallows its own buttons*,
/// leaving a modal that a screen reader user cannot answer. Use
/// [`container`](Self::container) for those: it keeps the annotation and leaves
/// the subtree reachable.
#[derive(Debug, Clone)]
pub struct Semantics {
    role: SemanticRole,
    label: Option<String>,
    value: Option<String>,
    toggled: Option<bool>,
    enabled: bool,
    merge: bool,
    live: Option<SemanticLiveness>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

/// The widget-layer spelling of `vieww_render::Liveness`.
///
/// Duplicated rather than re-exported for [`SemanticRole`]'s reason:
/// `vieww-widget` must not depend on `vieww-render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SemanticLiveness {
    /// Read only when the user navigates to it. Almost everything.
    #[default]
    Off,
    /// Read when the user is not in the middle of something else.
    Polite,
    /// Read straight away, interrupting.
    Assertive,
}

/// The widget-layer spelling of a screen reader role.
///
/// Mirrors `vieww_render::Role`. Duplicated rather than re-exported because
/// `vieww-widget` must not depend on `vieww-render` — the dependency runs the
/// other way, and this is the same shape as every other vocabulary type that
/// two layers meet on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum SemanticRole {
    #[default]
    Group,
    Label,
    TextField,
    Button,
    ScrollView,
    CheckBox,
    /// One of a set, of which exactly one is chosen.
    ///
    /// Distinct from [`CheckBox`](Self::CheckBox) rather than borrowing it: a
    /// screen reader announces "radio button, 2 of 5" and offers different
    /// navigation for it, and a user hearing "checkbox" is told the wrong thing
    /// about whether the others stay selected.
    Radio,
    /// Work in progress, determinate or not.
    ///
    /// Carries no actions — there is nothing a user can do to a progress bar —
    /// so the role exists purely so a screen reader says "progress indicator"
    /// and reads the percentage, rather than announcing an unlabelled group.
    ProgressBar,
    /// One page of a set, of which exactly one is shown.
    ///
    /// Distinct from [`Radio`](Self::Radio) even though both are one-of-a-set:
    /// a screen reader announces a tab with its place in a tab list and offers
    /// its own navigation between them, and choosing a tab *replaces what is on
    /// the screen* rather than merely recording an answer. A user told "radio
    /// button" is not told that the content below is about to change.
    ///
    /// [`toggled`](Semantics::toggled) is what says which one is showing, and
    /// the platform layer sends it as *selected* rather than as *on* — a tab is
    /// chosen, not switched on.
    Tab,
    Switch,
    Slider,
    /// A role this framework does not enumerate, named for the platform layer
    /// to map — the widget-layer counterpart of `vieww_render::Role::Custom`,
    /// duplicated for the same reason every vocabulary type crossing that
    /// boundary is (`vieww-widget` must not depend on `vieww-render`).
    ///
    /// This closes a real gap rather than a documentation one: the render
    /// layer has carried an `alert` role (mapped to AccessKit's own
    /// `Role::Alert`, which every platform's screen reader treats as an
    /// implicit **live region** — announced the moment it appears, with no
    /// focus needed) since before this variant existed, and nothing in the
    /// widget layer's closed `SemanticRole` enum could reach it. A transient
    /// widget like [`Snackbar`](crate::Snackbar) announcing itself needed
    /// exactly this and had no route to it.
    ///
    /// See `vieww_render::Role::Custom`'s doc for the recognised names; an
    /// unrecognised one falls back to a plain group, same as there.
    Custom(&'static str),
}

impl Default for Semantics {
    fn default() -> Self {
        Self::new()
    }
}

impl Semantics {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            role: SemanticRole::Group,
            label: None,
            value: None,
            toggled: None,
            enabled: true,
            merge: true,
            live: None,
            child: None,
            key: None,
        }
    }

    /// A subtree announced as a button with this name.
    #[must_use]
    pub fn button(label: impl Into<String>) -> Self {
        Self::new().role(SemanticRole::Button).label(label)
    }

    /// A named region whose contents stay reachable.
    ///
    /// The annotation for anything *containing* controls rather than *being*
    /// one: a dialog, a sheet, a section of a screen. A screen reader is told
    /// what this region is and can still walk into it.
    #[must_use]
    pub fn container(label: impl Into<String>) -> Self {
        Self::new().label(label).merge(false)
    }

    /// Whether this replaces its descendants' semantics (the default) or merely
    /// names the region containing them.
    #[must_use]
    pub const fn merge(mut self, merge: bool) -> Self {
        self.merge = merge;
        self
    }

    #[must_use]
    pub const fn role(mut self, role: SemanticRole) -> Self {
        self.role = role;
        self
    }

    /// What the thing *is*: "Submit", "Search", "Delete message".
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// What it currently *says*, where that differs from the label.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Whether this thing is on, for a checkbox or a switch.
    ///
    /// Not a [`value`](Self::value): a screen reader announces it in its own
    /// words, matching the role and the user's language.
    #[must_use]
    pub const fn toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Whether this thing will respond at all.
    ///
    /// The one piece of a control's state that is otherwise carried by colour
    /// alone. A disabled button that announces itself as an ordinary button is
    /// one a blind user will keep trying to press, and then assume is broken.
    ///
    /// Every control here sets it from the same thing that greys it out — having
    /// no handler — so the two cannot drift apart.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Override how this subtree is announced, against what its role implies.
    ///
    /// Rarely right, and deliberately awkward to reach for. `docs/AIMS.md` §J
    /// asks that announcement be *part of what a transient widget is*, and the
    /// mechanism for that is [`SemanticRole::Custom`]'s implied liveness — see
    /// `vieww_render::Liveness::for_role`. This exists for the case where a role
    /// is right and its usual liveness is not, which happens, and for turning
    /// liveness *on* for a role that has none.
    #[must_use]
    pub const fn live(mut self, live: SemanticLiveness) -> Self {
        self.live = Some(live);
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

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn semantic_role(&self) -> SemanticRole {
        self.role
    }

    #[must_use]
    pub fn semantic_label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    #[must_use]
    pub fn semantic_value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    #[must_use]
    pub const fn semantic_toggled(&self) -> Option<bool> {
        self.toggled
    }

    #[must_use]
    pub const fn semantic_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn merges_descendants(&self) -> bool {
        self.merge
    }

    #[must_use]
    pub const fn semantic_live(&self) -> Option<SemanticLiveness> {
        self.live
    }
}

impl Widget for Semantics {
    fn debug_name(&self) -> &'static str {
        "Semantics"
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
        // What a screen reader would say, in a tree dump — which is the only
        // way to check it without a screen reader in the room.
        let mut props = vec![("role", format!("{:?}", self.role))];
        if let Some(label) = &self.label {
            props.push(("label", label.clone()));
        }
        if let Some(value) = &self.value {
            props.push(("value", value.clone()));
        }
        if let Some(toggled) = self.toggled {
            props.push(("toggled", toggled.to_string()));
        }
        if !self.enabled {
            props.push(("enabled", "false".to_owned()));
        }
        if !self.merge {
            props.push(("container", "true".to_owned()));
        }
        if let Some(live) = self.live {
            props.push(("live", format!("{live:?}")));
        }
        props
    }
}

widget_node_from!(Semantics);
