//! The tree as a screen reader understands it.
//!
//! # Why this cannot be derived from the render tree
//!
//! The render tree is a description of *pixels*: a row is a `RenderFlex`, a
//! button is a coloured box with a gesture detector and a label inside it, and a
//! checkbox is two rectangles. None of that is what a screen reader has to say
//! out loud. VoiceOver needs "Submit, button, enabled"; there is no function
//! from the first to the second, because the information was never in the render
//! tree to begin with — it is in the intent of whoever built the widget.
//!
//! So semantics are *declared*, by the render objects that know what they mean,
//! and collected into a parallel tree. Most objects declare nothing and are
//! skipped: a `Padding` is not a thing a blind user needs to hear about.
//!
//! # Why it is a flat list with parent links, not a nested structure
//!
//! Because that is what [`AccessKit`](https://docs.rs/accesskit) — and every
//! platform API under it — consumes: a set of nodes keyed by id, plus a tree
//! shape. Building nested nodes here and flattening them there would mean two
//! representations and a conversion nobody can see the point of.
//!
//! # Acting, as well as reading
//!
//! A screen reader does not only read: it activates buttons, increments
//! sliders, and scrolls lists, on behalf of somebody who cannot reach them any
//! other way. [`SemanticAction`] is that vocabulary.
//!
//! Which actions a node offers is *declared* rather than inferred, for the same
//! reason its role is: only the object knows what it will actually do. A node
//! that advertises an action it ignores is worse than one that advertises
//! nothing — a screen reader will offer it to the user, and the user will
//! conclude the application is broken rather than that the action was never
//! there.

use vieww_foundation::Rect;

use crate::{RenderId, RenderTree};
use vieww_foundation::{FastMap, FastSet};

/// What kind of thing a node is, to a screen reader.
///
/// Small on purpose. Every variant here is one this framework can actually
/// produce today; a role nothing declares is a role that has never been checked
/// against a real screen reader, and a wrong role is worse than a generic one —
/// VoiceOver announcing "button" for static text teaches the user to try
/// activating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Role {
    /// A grouping with no meaning of its own. The default, and what a container
    /// that merely has semantic children is.
    #[default]
    Group,
    /// Static text.
    Label,
    /// Editable text.
    TextField,
    /// Something that can be activated.
    Button,
    /// A scrollable region.
    ScrollView,
    /// A box that is ticked or not.
    CheckBox,
    /// One of a set. See `SemanticRole::Radio`.
    Radio,
    /// Work in progress. See `SemanticRole::ProgressBar`.
    ProgressBar,
    /// One page of a set. See `SemanticRole::Tab`.
    Tab,
    /// An on/off control. Distinct from a checkbox because screen readers say
    /// "on"/"off" for one and "ticked"/"unticked" for the other, and users of
    /// them expect the difference.
    Switch,
    /// A value chosen from a range.
    Slider,
    /// The window's contents as a whole.
    Window,
    /// A role this framework does not enumerate, named for the platform layer
    /// to map.
    ///
    /// The variants above are the ones vieww itself produces. A control written
    /// **outside** this repository — a tree, a menu, a toolbar — had no way to
    /// describe itself, and the failure was the quiet kind: the control still
    /// worked, still took taps, and was simply announced wrongly. Nobody without
    /// a screen reader running would ever notice.
    ///
    /// # Why a name and not AccessKit's own type
    ///
    /// Exposing `accesskit::Role` here would be smaller, and would tie the
    /// semantics tree to AccessKit's release cadence — a coupling
    /// `docs/STABILITY.md` already names as a pre-1.0 ceiling, and one that
    /// would reach every platform including those not using AccessKit at all.
    /// A name keeps this crate free of any accessibility backend.
    ///
    /// # What the names are
    ///
    /// Lowercase and `snake_case`. The winit bridge currently maps `tree`,
    /// `tree_item`, `menu`, `menu_item`, `toolbar`, `tooltip`, `status_bar`,
    /// `list`, `list_item`, `link`, `heading`, `dialog`, `alert`, `image`,
    /// `tab_list`, `navigation`, `search`, `form`, `article`, `banner`, `meter`,
    /// `timer`, `log` and `note`.
    ///
    /// **An unrecognised name falls back to a plain group**, deliberately: a
    /// screen reader saying less than it could is recoverable, and saying
    /// something untrue is not.
    Custom(&'static str),
}

/// Something a screen reader can ask a node to do.
///
/// Small, and every variant here is one this framework can carry out. The
/// vocabulary of AccessKit and of every platform API under it is far larger;
/// mapping more of it would mean advertising actions with nothing behind them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SemanticAction {
    /// "Click this." A button pressed, a switch flipped, a row opened. What a
    /// double-tap in VoiceOver and a double-tap in TalkBack both mean.
    Activate,
    /// Move a value one step up. A slider, and eventually a stepper.
    Increment,
    /// Move a value one step down.
    Decrement,
    /// Move a scrollable region on by about a screen.
    ScrollForward,
    /// Move it back by about a screen.
    ScrollBackward,
    /// An action this framework does not enumerate, named for the platform layer
    /// to map.
    ///
    /// The counterpart to [`Role::Custom`], and it travels **both ways**: a
    /// widget advertises one, and a screen reader asking for it arrives back as
    /// the same name. Before this, a request for "expand" was dropped on the
    /// floor, because approximating it as a click would do something the user
    /// never asked for.
    ///
    /// The winit bridge maps `expand`, `collapse`, `focus`, `blur`,
    /// `show_tooltip`, `hide_tooltip`, `scroll_into_view`, `scroll_left` and
    /// `scroll_right`.
    ///
    /// **An unrecognised name is not advertised at all**, which is the opposite
    /// of [`Role::Custom`]'s fallback and right for the same reason: a role is a
    /// description and degrades to a vaguer one, while an action is a promise
    /// and must not degrade into a different promise.
    Custom(&'static str),
}

/// Whether a screen reader should read this node out *without being asked*.
///
/// `docs/AIMS.md` §J: *"A snackbar that appears and disappears without being
/// announced is invisible to a screen reader… Announcement should be part of
/// what a transient widget **is**, not a property an application remembers to
/// set."*
///
/// # Why this is a field here rather than an imperative call
///
/// Every other framework spells announcement as a side effect —
/// `SemanticsService.announce(...)`, `UIAccessibility.post(...)`,
/// `aria-live` plus a mutation. All three share a failure: the call site and the
/// widget are different places, so the widget can be shown by a path that
/// forgets to make the call. Here the announcement is **derived from the tree**
/// by [`SemanticsTree::announcements`], so a widget that declares itself live is
/// announced by every path that mounts it, including ones written later by
/// somebody who has never read this file.
///
/// # Why it has a default rather than being required
///
/// Most nodes are not live, and a screen reader that read every button out as it
/// scrolled past would be unusable. [`Off`](Self::Off) is the right default and
/// [`for_role`](Self::for_role) is how the transient roles escape it without
/// each widget remembering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Liveness {
    /// Read only when the user navigates to it. Almost everything.
    #[default]
    Off,
    /// Read when the user is not in the middle of something else — a status
    /// line, a "saved" confirmation, a snackbar with no action.
    Polite,
    /// Read straight away, interrupting. An error, a warning, a snackbar
    /// carrying the only chance to undo something.
    Assertive,
}

impl Liveness {
    /// What a role is live by default, so a widget does not have to say.
    ///
    /// This is the line that makes §J's aim structural: `alert` is a live region
    /// on every platform's screen reader already, and a framework that knew that
    /// and did not act on it would be leaving each widget to rediscover it.
    /// A third party's `Custom("alert")` gets the same treatment as vieww's own,
    /// which is §A's rule reaching accessibility.
    #[must_use]
    pub fn for_role(role: Role) -> Self {
        // Not `const`: matching on `&str` in a const fn is not stable, and the
        // alternative — an integer tag per name — would put the mapping
        // somewhere other than beside the names it maps.
        match role {
            Role::Custom("alert") => Self::Assertive,
            Role::Custom("status" | "log" | "timer") => Self::Polite,
            _ => Self::Off,
        }
    }

    /// `true` when this node is read out without being navigated to.
    #[must_use]
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Something a screen reader should say now, because the tree changed.
///
/// Produced by [`SemanticsTree::announcements`] and consumed by the platform
/// layer. Deliberately carries the text rather than the node: by the time a
/// platform gets round to speaking, the node may be gone — a snackbar that
/// dismissed itself is the ordinary case — and an announcement that resolves a
/// node id at speaking time is one that goes silent exactly when it mattered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// What to say.
    pub text: String,
    /// Whether it interrupts.
    pub liveness: Liveness,
}

/// One node in the semantics tree.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticsNode {
    /// The render object that declared it. Doubles as the node's identity,
    /// which is what lets a screen reader's cursor survive a rebuild — the id is
    /// stable across reconciliation for the same reason focus is.
    pub id: RenderId,
    pub role: Role,
    /// What to read out. A field's label, a button's title, a paragraph's text.
    pub label: Option<String>,
    /// The current contents, where those differ from the label — a field's
    /// text, as opposed to what the field is *for*.
    pub value: Option<String>,
    /// For a checkbox or a switch, whether it is on.
    ///
    /// Separate from [`value`](Self::value) because a screen reader announces it
    /// in its own words, in the user's language, and with the phrasing that
    /// matches the role. A switch that reported `value: "on"` would be read as
    /// the literal string "on" in an English sentence on a French system.
    pub toggled: Option<bool>,
    /// Where it is on screen, in global logical pixels. A screen reader draws a
    /// focus rectangle here, and a touch exploration gesture hit tests against
    /// it.
    pub bounds: Rect,
    /// `false` if the thing is visibly there but will not respond.
    ///
    /// A screen reader announces this — "Submit, button, dimmed" — and it is the
    /// one piece of a control's state that is *only* carried by colour
    /// otherwise. A disabled button that reads as an ordinary one is a button a
    /// blind user will keep trying to press.
    pub enabled: bool,
    /// `true` if it can take keyboard focus.
    pub focusable: bool,
    /// `true` if it currently has it.
    pub focused: bool,
    /// What a screen reader may ask this node to do. See [`SemanticAction`].
    pub actions: Vec<SemanticAction>,
    /// Whether a screen reader reads this out unprompted. See [`Liveness`].
    pub live: Liveness,
    /// Children, in reading order.
    pub children: Vec<RenderId>,
}

impl SemanticsNode {
    /// A node with nothing but a role.
    #[must_use]
    pub fn new(id: RenderId, role: Role) -> Self {
        Self {
            id,
            role,
            label: None,
            value: None,
            toggled: None,
            enabled: true,
            bounds: Rect::ZERO,
            focusable: false,
            focused: false,
            actions: Vec::new(),
            live: Liveness::for_role(role),
            children: Vec::new(),
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
}

/// What a render object contributes to the semantics tree.
///
/// Returned by [`RenderObject::semantics`](crate::RenderObject::semantics).
/// Deliberately not a `SemanticsNode`: an object does not know its own position
/// on screen, its own id, or whether it has focus, and making it fill those in
/// would be asking it to lie about three of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Semantics {
    pub role: Role,
    pub label: Option<String>,
    pub value: Option<String>,
    /// For a checkbox or a switch, whether it is on. See
    /// [`SemanticsNode::toggled`].
    pub toggled: Option<bool>,
    /// Whether it will respond at all. See [`SemanticsNode::enabled`].
    ///
    /// `true` by default, because most things are: a role that has to opt *in*
    /// to being usable would make every new render object silently disabled.
    pub enabled: bool,
    /// What this thing can be asked to do. Empty by default: an object that has
    /// not said it handles an action does not handle it.
    pub actions: Vec<SemanticAction>,
    /// Whether a screen reader reads this out unprompted.
    ///
    /// `None` — the default — means *take it from the role*, which is what makes
    /// [`Liveness::for_role`] load-bearing rather than advisory: a widget has to
    /// go out of its way to make an alert silent. `Some` is an override, for the
    /// case where a role's usual liveness is wrong.
    pub live: Option<Liveness>,
}

impl Semantics {
    #[must_use]
    pub fn new(role: Role) -> Self {
        Self {
            role,
            label: None,
            value: None,
            toggled: None,
            enabled: true,
            actions: Vec::new(),
            live: None,
        }
    }

    /// Static text that reads as itself.
    #[must_use]
    pub fn label(text: impl Into<String>) -> Self {
        Self {
            role: Role::Label,
            label: Some(text.into()),
            value: None,
            toggled: None,
            enabled: true,
            actions: Vec::new(),
            live: None,
        }
    }

    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Declare this thing as on or off.
    #[must_use]
    pub const fn with_toggled(mut self, toggled: bool) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Declare whether it will respond.
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Override how this thing is announced, against what its role implies.
    ///
    /// Rarely right. The role already carries the answer for every transient
    /// role vieww knows about, and a widget that turns liveness *off* on an
    /// alert has built the thing §J exists to prevent.
    #[must_use]
    pub const fn with_live(mut self, live: Liveness) -> Self {
        self.live = Some(live);
        self
    }

    /// Declare an action this thing can be asked to carry out.
    ///
    /// Only declare what
    /// [`handle_semantic_action`](crate::RenderObject::handle_semantic_action)
    /// actually does. An advertised action that does nothing is offered to the
    /// user by their screen reader and then silently fails, which reads as the
    /// application being broken.
    #[must_use]
    pub fn with_action(mut self, action: SemanticAction) -> Self {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
        self
    }
}

impl Default for Semantics {
    /// A plain group, *enabled*.
    ///
    /// Written out rather than derived: a derived `Default` gives `enabled` the
    /// `bool` default, which is `false` — so every annotation built that way
    /// would silently announce itself as unavailable.
    fn default() -> Self {
        Self::new(Role::default())
    }
}

/// The whole tree, as a screen reader sees it.
#[derive(Debug, Clone, Default)]
pub struct SemanticsTree {
    nodes: Vec<SemanticsNode>,
    /// Where each id sits in [`nodes`](Self::nodes).
    ///
    /// The list is the truth and this is only a way in. It exists because
    /// [`node`](Self::node) is called from the ancestor walks — `action_target`
    /// climbs one node per level — so a linear scan made every screen-reader hit
    /// test and every AccessKit action lookup cost O(depth x nodes). Kept in
    /// step in the two places `nodes` changes, `push_node` and
    /// `remove_subtrees`, and nowhere else.
    ///
    /// Excluded from `PartialEq` because it is derived from `nodes`: two trees
    /// with the same nodes are the same tree, and comparing the map as well
    /// would only be a slower way of asking the same question.
    index: FastMap<RenderId, usize>,
    /// Who lists each id as a child, for the same reason and kept in step in the
    /// same two places.
    ///
    /// Without it the ancestor walk in [`action_target`](Self::action_target)
    /// stayed quadratic however fast [`node`](Self::node) became: it scanned
    /// every node's child list once per level.
    parents: FastMap<RenderId, RenderId>,
    root: Option<RenderId>,
}

impl PartialEq for SemanticsTree {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes && self.root == other.root
    }
}

/// What one level of the collect walk hands back to the level above it.
///
/// Two answers rather than one, because a subtree reports something about
/// itself *and* something about its siblings. Keeping them in a pair is what
/// lets the second travel: a barrier is nested inside the route that owns it,
/// and the screen it covers is a sibling of that route rather than of the
/// barrier, so the flag has to be passed up through every level in between.
struct Contribution {
    /// The semantic children this subtree contributes to the nearest ancestor
    /// that declared anything.
    children: Vec<RenderId>,
    /// `true` if something in here hides whatever was painted before it — a
    /// modal barrier, at whatever depth.
    blocks: bool,
}

impl Contribution {
    /// Nothing collected and nothing blocked.
    const fn nothing() -> Self {
        Self {
            children: Vec::new(),
            blocks: false,
        }
    }
}

impl SemanticsTree {
    /// Walk a render tree and collect what it declares.
    ///
    /// Objects declaring nothing are skipped, and their semantic children are
    /// re-parented onto the nearest ancestor that declared something — so the
    /// twelve layout boxes between a screen and its button do not become twelve
    /// levels a screen reader has to swipe through.
    #[must_use]
    pub fn build(tree: &RenderTree, focused: Option<RenderId>) -> Self {
        let mut built = Self::default();
        let Some(root) = tree.root() else {
            return built;
        };

        // The root always produces a node even if it declares nothing, because a
        // tree with no root is not something a platform adapter can attach to.
        //
        // A block reported all the way up to here has nowhere left to go: the
        // root has no siblings, and everything it could have cleared already
        // was, on the way up.
        let children = built.collect(tree, root, focused).children;
        let root_node = match built.index.get(&root).copied() {
            Some(index) => {
                built.root = Some(root);
                index
            }
            None => {
                let mut node = SemanticsNode::new(root, Role::Window);
                node.bounds = tree.global_bounds(root);
                node.children = children;
                built.push_node(node);
                built.root = Some(root);
                built.nodes.len() - 1
            }
        };
        // A declared root is still the window as far as a platform is concerned.
        built.nodes[root_node].role = Role::Window;
        built
    }

    /// Collect `id`'s subtree, returning the semantic children it contributes to
    /// whatever is above it.
    fn collect(
        &mut self,
        tree: &RenderTree,
        id: RenderId,
        focused: Option<RenderId>,
    ) -> Contribution {
        // Drawn, but not for a screen reader — a covered route that has to keep
        // painting because a transition reveals it. Returning before the object
        // is consulted drops its own node as well as its subtree's, so a
        // wrapper that declared semantics of its own could not leak one either.
        //
        // A barrier inside such a subtree is dropped with it, rather than
        // blocking on its way past. An excluded subtree contributes nothing,
        // and a dialog nobody can hear is not a dialog covering anything.
        if tree.hides_semantics(id) {
            return Contribution::nothing();
        }

        let mut descendants = Vec::new();
        // Set by a child that blocks, and reported upward at the end: the
        // barrier is nested inside the route that owns it, and the screen it
        // covers is a sibling of that route rather than of the barrier.
        let mut blocks = false;
        // A hidden subtree is hidden from a screen reader too. Reading out a
        // route that is behind another one is worse than not reading it: the
        // user hears a screen they cannot reach and has no way to tell why.
        if !tree.skips_children(id) {
            for &child in tree.children(id) {
                let contribution = self.collect(tree, child, focused);
                if contribution.blocks {
                    // Everything painted before this child is behind a modal
                    // barrier — already unreachable by touch, and now
                    // unreachable by a screen reader as well.
                    //
                    // The nodes have to be removed and not merely forgotten,
                    // for the reason the merging path gives: dropping them from
                    // this list alone would leave them in the flat one, where no
                    // walk from the root reaches them and an adapter reports
                    // them as orphans.
                    self.remove_subtrees(&descendants);
                    descendants.clear();
                    blocks = true;
                }
                descendants.extend(contribution.children);
            }
        }

        let Some(object) = tree.object(id) else {
            return Contribution {
                children: descendants,
                blocks,
            };
        };
        // Its own flag, in addition to any its children reported. This is the
        // barrier itself: it blocks what was painted before *it*, which is a
        // question for the level above, not for the children it just collected.
        let blocks = blocks || object.blocks_semantics();
        let Some(declared) = object.semantics() else {
            // Nothing of its own: its children belong to its parent instead.
            return Contribution {
                children: descendants,
                blocks,
            };
        };

        let children = if object.merges_descendant_semantics() {
            // This object speaks for them. Their nodes were collected on the way
            // down and have to be dropped, or they stay in the flat list
            // unreachable from the root — which an adapter would report as
            // orphans.
            self.remove_subtrees(&descendants);
            Vec::new()
        } else {
            descendants
        };

        let node = SemanticsNode {
            id,
            role: declared.role,
            label: declared.label,
            value: declared.value,
            toggled: declared.toggled,
            enabled: declared.enabled,
            bounds: tree.global_bounds(id),
            focusable: object.is_focusable(),
            focused: focused == Some(id),
            // A disabled control offers nothing, whatever it declared. Leaving
            // them on would have a screen reader offer "activate" for a button
            // it has just announced as unavailable.
            actions: if declared.enabled {
                declared.actions
            } else {
                Vec::new()
            },
            // The role's answer unless the object overrode it. A transient
            // widget therefore arrives announced rather than being made
            // announceable later, which is what `docs/AIMS.md` §J asks for.
            live: declared.live.unwrap_or(Liveness::for_role(declared.role)),
            children,
        };
        self.push_node(node);
        // The block keeps travelling past a node that declared something, and
        // this is the line that decides it. A stricter design stops at a semantic
        // boundary; stopping here would break the only case this exists for,
        // because `Dialog` wraps itself in a `Semantics` container so a screen
        // reader announces it as one group — the block would be trapped inside
        // the dialog, which is the one place it has nothing to clear.
        Contribution {
            children: vec![id],
            blocks,
        }
    }

    /// Drop these nodes and everything under them.
    ///
    /// Called when an ancestor merges: the descendants were already collected,
    /// and leaving them in the list would produce nodes no walk from the root
    /// can reach.
    /// Both the descent and the removal go through the id index rather than
    /// scanning: this used to be O(nodes x doomed), a linear `find` per level
    /// followed by a linear `contains` per surviving node, which is quadratic
    /// on a screen where several containers merge their descendants. It is now
    /// linear in the nodes, and the reindex after the removal is the only walk
    /// left.
    fn remove_subtrees(&mut self, roots: &[RenderId]) {
        let mut doomed: Vec<RenderId> = roots.to_vec();
        let mut seen: FastSet<RenderId> = roots.iter().copied().collect();
        let mut at = 0;
        while at < doomed.len() {
            let id = doomed[at];
            if let Some(node) = self.node(id) {
                let children: Vec<RenderId> = node.children.clone();
                for child in children {
                    if seen.insert(child) {
                        doomed.push(child);
                    }
                }
            }
            at += 1;
        }
        self.nodes.retain(|node| !seen.contains(&node.id));
        self.reindex();
    }

    /// Rebuild the id index from the list.
    ///
    /// Only after a removal, which is the one operation that moves nodes that
    /// are staying. A push appends and can patch the index in place.
    fn reindex(&mut self) {
        self.index.clear();
        self.parents.clear();
        for (position, node) in self.nodes.iter().enumerate() {
            self.index.insert(node.id, position);
            for &child in &node.children {
                self.parents.insert(child, node.id);
            }
        }
    }

    /// Append a node and record where it went.
    ///
    /// The only place the list grows, so that the index cannot fall out of step
    /// with it — an id missing from the index reads as a node that is not in the
    /// tree, and a screen reader would simply stop finding it.
    fn push_node(&mut self, node: SemanticsNode) {
        self.index.insert(node.id, self.nodes.len());
        for &child in &node.children {
            self.parents.insert(child, node.id);
        }
        self.nodes.push(node);
    }

    /// Every node, children before parents.
    #[must_use]
    pub fn nodes(&self) -> &[SemanticsNode] {
        &self.nodes
    }

    /// The root node's id, if the tree has anything in it.
    #[must_use]
    pub const fn root(&self) -> Option<RenderId> {
        self.root
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// A node by id.
    ///
    /// Through the index, so it is a hash lookup rather than a scan. The
    /// ancestor walks above call this once per level, and on a screen with a
    /// few hundred nodes the scan made every hit test and every AccessKit
    /// action lookup quadratic in the depth of the tree.
    #[must_use]
    pub fn node(&self, id: RenderId) -> Option<&SemanticsNode> {
        self.index.get(&id).map(|&position| &self.nodes[position])
    }

    /// The node that should carry out `action` on behalf of `id`.
    ///
    /// `id` itself when it offers the action, and otherwise the nearest ancestor
    /// that does — which is what makes "scroll down" work when a screen reader's
    /// cursor is on a *row* rather than on the list around it. A screen reader
    /// points at what it is reading, and what it is reading is rarely the thing
    /// that scrolls.
    #[must_use]
    pub fn action_target(&self, id: RenderId, action: SemanticAction) -> Option<RenderId> {
        let mut current = Some(id);
        while let Some(node) = current.and_then(|id| self.node(id)) {
            if node.actions.contains(&action) {
                return Some(node.id);
            }
            current = self.parent_of(node.id);
        }
        None
    }

    /// The node whose children include `id`.
    ///
    /// Read off the index rather than searched for. It used to be a scan, on the
    /// argument that the tree is small and a stored link would be a second thing
    /// to keep in step — but the link is stored in exactly the two places the
    /// node list is written, so there is no third place for it to drift in, and
    /// the scan made every ancestor walk cost a pass over the whole tree per
    /// level.
    fn parent_of(&self, id: RenderId) -> Option<RenderId> {
        self.parents.get(&id).copied()
    }

    /// The nodes in reading order, depth-first from the root.
    ///
    /// What a screen reader's "next item" gesture follows.
    #[must_use]
    pub fn reading_order(&self) -> Vec<RenderId> {
        let mut order = Vec::new();
        if let Some(root) = self.root {
            self.visit(root, &mut order);
        }
        order
    }

    fn visit(&self, id: RenderId, order: &mut Vec<RenderId>) {
        order.push(id);
        if let Some(node) = self.node(id) {
            // Cloned because `visit` borrows self again; a semantics tree is
            // rebuilt only when something structural changed, so this is not on
            // a per-frame path.
            for child in node.children.clone() {
                self.visit(child, order);
            }
        }
    }

    /// Every live node, in reading order.
    #[must_use]
    pub fn live_nodes(&self) -> Vec<&SemanticsNode> {
        self.reading_order()
            .into_iter()
            .filter_map(|id| self.node(id))
            .filter(|node| node.live.is_live())
            .collect()
    }

    /// What a screen reader should say now, given what it said last frame.
    ///
    /// The whole of `docs/AIMS.md` §J's mechanism, and it is a **diff rather
    /// than a call**. A live node that was not there before is announced; one
    /// whose text changed is announced again; one that merely stayed put is not.
    /// Nothing in a widget has to remember to do anything, which is the
    /// property that makes this structural — the announcement happens on every
    /// path that mounts the widget, including paths written later.
    ///
    /// # Why disappearing is not announced
    ///
    /// A snackbar that times out has nothing to say on the way out, and a screen
    /// reader that narrated every dismissal would be reading the user their own
    /// history. Appearing is news; going away is not.
    ///
    /// # Why the comparison is on text and not on id
    ///
    /// Because a snackbar replaced by a different snackbar is usually the *same*
    /// element reconciled with a new message — same id, new words — and that is
    /// exactly the case an id comparison misses and a user needs to hear.
    #[must_use]
    pub fn announcements(&self, previous: &Self) -> Vec<Announcement> {
        self.live_nodes()
            .into_iter()
            .filter_map(|node| {
                let text = Self::spoken(node)?;
                let said_before = previous
                    .node(node.id)
                    .and_then(Self::spoken)
                    .is_some_and(|before| before == text);
                if said_before {
                    return None;
                }
                Some(Announcement {
                    text,
                    liveness: node.live,
                })
            })
            .collect()
    }

    /// What a live node actually says: its label, then its value.
    ///
    /// `None` for a live node with neither, which is a widget that declared it
    /// wanted to interrupt the user and then had nothing to tell them. Silence
    /// is the right answer, and the node still exists to be navigated to.
    fn spoken(node: &SemanticsNode) -> Option<String> {
        match (node.label.as_deref(), node.value.as_deref()) {
            (Some(label), Some(value)) => Some(format!("{label}, {value}")),
            (Some(text), None) | (None, Some(text)) => Some(text.to_owned()),
            (None, None) => None,
        }
    }

    /// A readable dump, for tests and for debugging what a screen reader would
    /// say.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut out = String::new();
        if let Some(root) = self.root {
            self.describe_node(root, 0, &mut out);
        }
        out
    }

    fn describe_node(&self, id: RenderId, depth: usize, out: &mut String) {
        let Some(node) = self.node(id) else { return };
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(&format!("{:?}", node.role));
        if let Some(label) = &node.label {
            out.push_str(&format!(" {label:?}"));
        }
        if let Some(value) = &node.value {
            out.push_str(&format!(" = {value:?}"));
        }
        if node.focused {
            out.push_str(" [focused]");
        }
        out.push('\n');
        for child in node.children.clone() {
            self.describe_node(child, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Constraints, Size};

    use super::*;
    use crate::{RenderSemantics, RenderTree};

    /// A chain `depth` levels deep, annotated at every level.
    ///
    /// Deep rather than wide on purpose: the lookups these tests are about are
    /// the ones an ancestor walk makes, one per level.
    fn chain(depth: usize, merge_at: Option<usize>) -> (RenderTree, Vec<RenderId>) {
        let mut tree = RenderTree::new();
        let mut ids = Vec::new();
        let mut parent = None;
        for level in 0..depth {
            let role = if level == 0 {
                Role::ScrollView
            } else {
                Role::Group
            };
            let object = RenderSemantics::new(role)
                .label(format!("level {level}"))
                .merge(merge_at == Some(level));
            let id = tree.insert(parent, Box::new(object));
            ids.push(id);
            parent = Some(id);
        }
        let _ = tree.layout_root(Constraints::tight(Size::new(100.0, 40.0)));
        (tree, ids)
    }

    /// What the lookups answered before they were indexed: a scan of the list.
    ///
    /// The index has to agree with this for every node in the tree, or it is a
    /// second truth rather than a way into the first one.
    fn agrees_with_a_scan(semantics: &SemanticsTree) {
        for node in semantics.nodes() {
            assert_eq!(
                semantics.node(node.id),
                semantics.nodes().iter().find(|other| other.id == node.id),
                "looked up by id"
            );
            assert_eq!(
                semantics.parent_of(node.id),
                semantics
                    .nodes()
                    .iter()
                    .find(|other| other.children.contains(&node.id))
                    .map(|other| other.id),
                "looked up by who lists it as a child"
            );
        }
    }

    #[test]
    fn every_node_of_a_deep_tree_is_found_by_id_and_by_its_parent() {
        let (tree, ids) = chain(32, None);
        let semantics = SemanticsTree::build(&tree, None);

        for id in &ids {
            assert_eq!(
                semantics.node(*id).map(|node| node.id),
                Some(*id),
                "a node a screen reader can reach has to be findable"
            );
        }
        agrees_with_a_scan(&semantics);
    }

    #[test]
    fn a_node_that_is_not_in_the_tree_is_not_found() {
        let (tree, ids) = chain(4, None);
        // From a deeper tree, so its id names a slot the short one never filled
        // — ids are an index and a generation, and the *first* node of any tree
        // is the same id in all of them.
        let (_deeper, deeper_ids) = chain(12, None);
        let stranger = *deeper_ids.last().expect("a chain has a deepest node");
        let semantics = SemanticsTree::build(&tree, None);

        assert!(semantics.node(stranger).is_none());
        assert!(semantics.parent_of(ids[0]).is_none(), "the root has none");
    }

    #[test]
    fn merging_a_subtree_drops_exactly_the_nodes_underneath_it() {
        let merge_at = 3;
        let (tree, ids) = chain(9, Some(merge_at));
        let semantics = SemanticsTree::build(&tree, None);

        for (level, id) in ids.iter().enumerate() {
            assert_eq!(
                semantics.node(*id).is_some(),
                level <= merge_at,
                "level {level} against a merge at {merge_at}"
            );
        }
        assert!(
            semantics
                .node(ids[merge_at])
                .is_some_and(|node| node.children.is_empty()),
            "the node that merged speaks for its subtree, so it has no children left"
        );
        // The removal moves everything after it in the list, so this is the case
        // an index that was only appended to would get wrong.
        agrees_with_a_scan(&semantics);
    }

    #[test]
    fn an_action_is_carried_out_by_the_nearest_ancestor_offering_it() {
        let (tree, ids) = chain(16, None);
        let semantics = SemanticsTree::build(&tree, None);
        let deepest = *ids.last().expect("a chain has a deepest node");

        assert_eq!(
            semantics.action_target(deepest, SemanticAction::ScrollForward),
            Some(ids[0]),
            "only the scroll view offers it, however many levels up it is"
        );
        assert_eq!(
            semantics.action_target(deepest, SemanticAction::Activate),
            None,
            "and nothing invents one"
        );
    }
}
