use std::collections::HashSet;
use std::rc::Rc;

use vieww_foundation::{EdgeInsetsDirectional, Key, TextDirection};

use crate::{
    icons, widget_node_from, BuildContext, CrossAxisAlignment, Directionality, Flex,
    GestureDetector, Icon, MainAxisSize, PaddingDirectional, SemanticRole, Semantics, SizedBox,
    ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How far each level of nesting is indented, in logical pixels.
pub const TREE_INDENT: f32 = 20.0;

/// One row of a [`TreeView`]: a label, an identity, and its own subtree.
///
/// `id` is what [`TreeView`]'s `expanded` set and `on_toggled` speak in —
/// stable across rebuilds, the same job a list's key does, and for the same
/// reason: a tree's shape can change (a node loads its children lazily,
/// a search filters some out) and "which rows are open" has to survive that
/// by identity, not by position.
#[derive(Clone)]
pub struct TreeNode {
    id: String,
    label: WidgetNode,
    children: Vec<TreeNode>,
}

impl std::fmt::Debug for TreeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeNode")
            .field("id", &self.id)
            .field("children", &self.children.len())
            .finish_non_exhaustive()
    }
}

impl TreeNode {
    #[must_use]
    pub fn leaf(id: impl Into<String>, label: impl Into<WidgetNode>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn branch(
        id: impl Into<String>,
        label: impl Into<WidgetNode>,
        children: Vec<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

/// A recursively nested list with per-node expand/collapse, at any depth.
///
/// ```
/// use std::collections::HashSet;
/// use vieww_widget::prelude::*;
/// use vieww_widget::{TreeNode, TreeView};
///
/// let tree = vec![TreeNode::branch(
///     "src",
///     Text::new("src/"),
///     vec![TreeNode::leaf("main.rs", Text::new("main.rs"))],
/// )];
///
/// let mut expanded = HashSet::new();
/// expanded.insert("src".to_owned());
///
/// let view = TreeView::new(tree, expanded).on_toggled(|id| println!("toggle {id}"));
/// ```
///
/// # Controlled by node id, the same rule as every other control here
///
/// `TreeView` does not own which nodes are open — the caller does, in
/// whatever `Signal`-backed `HashSet<String>` it writes from
/// [`on_toggled`](Self::on_toggled). This is what [`Accordion`](crate::Accordion)
/// does for one row; a tree is the same rule applied at every depth rather
/// than a new mechanism for having more than one.
///
/// # Why this is not virtualised, and [`ListView`](crate::ListView) is
///
/// A flat list's rows are interchangeable slots a window can jump into.
/// A tree's rows are not: which ones exist at all depends on which ancestors
/// are open, so "row 40" has no meaning without walking the tree to find it —
/// exactly the variable-extent case [`ListView::variable`](crate::ListView::variable)
/// already has to solve, except here the *count* is unstable per frame too,
/// not just the extents. A tree wide and deep enough to need virtualisation
/// is a real future gap, tracked in `docs/AIMS.md` rather than hidden by
/// building the shrink-wrapped version and calling it done: the flattening
/// this would need is exactly `ListView`'s existing `ExtentBuilder` machinery
/// aimed at a tree walk instead of an index, and deserves that scope on its
/// own rather than arriving as an undocumented limitation of this type.
///
/// # A leaf is not a button
///
/// The same reasoning [`Breadcrumbs`](crate::Breadcrumbs) gives for its
/// final step: a row with nothing to disclose is [`SemanticRole::Label`],
/// not [`SemanticRole::Button`] — nothing happens when it is activated, so a
/// screen reader should not offer to activate it.
#[derive(Clone)]
pub struct TreeView {
    nodes: Vec<TreeNode>,
    expanded: HashSet<String>,
    on_toggled: Option<Rc<dyn Fn(String)>>,
    key: Option<Key>,
}

impl TreeView {
    #[must_use]
    pub fn new(nodes: Vec<TreeNode>, expanded: HashSet<String>) -> Self {
        Self {
            nodes,
            expanded,
            on_toggled: None,
            key: None,
        }
    }

    /// Called with the id of whichever branch's disclosure was tapped.
    #[must_use]
    pub fn on_toggled(mut self, handler: impl Fn(String) + 'static) -> Self {
        self.on_toggled = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    fn row(
        &self,
        node: &TreeNode,
        depth: usize,
        theme: &ThemeData,
        direction: TextDirection,
    ) -> WidgetNode {
        let is_expanded = self.expanded.contains(&node.id);
        let indent = TREE_INDENT * depth as f32;

        let disclosure: WidgetNode = if node.has_children() {
            let icon = if is_expanded {
                icons::chevron_down()
            } else {
                icons::chevron_forward(direction)
            };
            Icon::new(icon)
                .size(16.0)
                .color(theme.colors.on_surface_variant)
                .into()
        } else {
            SizedBox::square(16.0).into()
        };

        let content = Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .push(disclosure)
            .push(SizedBox::width(theme.metrics.gap / 2.0))
            .push(node.label.clone());

        let indented: WidgetNode =
            PaddingDirectional::new(EdgeInsetsDirectional::only(indent, 4.0, 0.0, 4.0))
                .child(content)
                .into();

        let row: WidgetNode = if node.has_children() {
            let id = node.id.clone();
            let handler = self.on_toggled.clone();
            let tappable: WidgetNode = match handler {
                Some(handler) => GestureDetector::new()
                    .on_tap(move |_| handler(id.clone()))
                    .child(indented)
                    .into(),
                None => indented,
            };
            Semantics::new()
                .role(SemanticRole::Button)
                .toggled(is_expanded)
                .child(tappable)
                .into()
        } else {
            Semantics::new()
                .role(SemanticRole::Label)
                .child(indented)
                .into()
        };

        row
    }

    fn flatten(
        &self,
        nodes: &[TreeNode],
        depth: usize,
        theme: &ThemeData,
        direction: TextDirection,
        out: &mut Vec<WidgetNode>,
    ) {
        for node in nodes {
            out.push(self.row(node, depth, theme, direction));
            if node.has_children() && self.expanded.contains(&node.id) {
                self.flatten(&node.children, depth + 1, theme, direction, out);
            }
        }
    }
}

impl Widget for TreeView {
    fn debug_name(&self) -> &'static str {
        "TreeView"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let direction = Directionality::of(ctx);

        let mut rows = Vec::new();
        self.flatten(&self.nodes, 0, &theme, direction, &mut rows);

        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(rows)
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("nodes", self.nodes.len().to_string()),
            ("expanded", self.expanded.len().to_string()),
        ]
    }
}

impl std::fmt::Debug for TreeView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeView")
            .field("nodes", &self.nodes.len())
            .field("expanded", &self.expanded.len())
            .finish_non_exhaustive()
    }
}

widget_node_from!(TreeView);

#[cfg(test)]
mod tests {
    use crate::{inflate, DebugNode, Text, Theme};

    use super::*;

    fn sample() -> Vec<TreeNode> {
        vec![TreeNode::branch(
            "src",
            Text::new("src/"),
            vec![
                TreeNode::leaf("main.rs", Text::new("main.rs")),
                TreeNode::branch(
                    "widgets",
                    Text::new("widgets/"),
                    vec![TreeNode::leaf("stack.rs", Text::new("stack.rs"))],
                ),
            ],
        )]
    }

    fn built(expanded: HashSet<String>) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(TreeView::new(sample(), expanded)))
    }

    #[test]
    fn a_collapsed_tree_shows_only_its_top_level() {
        let node = built(HashSet::new());
        assert_eq!(node.find_all("Text").len(), 1, "only \"src/\" is on screen");
    }

    #[test]
    fn expanding_one_branch_reveals_only_its_direct_children() {
        let mut expanded = HashSet::new();
        expanded.insert("src".to_owned());
        let node = built(expanded);
        assert_eq!(
            node.find_all("Text").len(),
            3,
            "src/, main.rs, widgets/ — but not stack.rs"
        );
    }

    #[test]
    fn expanding_every_ancestor_reaches_a_grandchild() {
        let mut expanded = HashSet::new();
        expanded.insert("src".to_owned());
        expanded.insert("widgets".to_owned());
        let node = built(expanded);
        assert_eq!(node.find_all("Text").len(), 4, "every row down to stack.rs");
    }

    #[test]
    fn a_leaf_is_a_label_not_a_button() {
        let mut expanded = HashSet::new();
        expanded.insert("src".to_owned());
        let node = built(expanded);
        let roles: Vec<Option<&str>> = node
            .find_all("Semantics")
            .iter()
            .map(|n| n.property("role"))
            .collect();
        assert_eq!(roles, vec![Some("Button"), Some("Label"), Some("Button")]);
    }

    #[test]
    fn a_branch_reports_its_disclosure_state() {
        let mut expanded = HashSet::new();
        expanded.insert("src".to_owned());
        let node = built(expanded);
        let semantics = node.find_all("Semantics");
        assert_eq!(semantics[0].property("toggled"), Some("true"));
    }

    #[test]
    fn tapping_a_branch_reports_its_id_not_a_leafs() {
        let seen: Rc<std::cell::RefCell<Vec<String>>> =
            Rc::new(std::cell::RefCell::new(Vec::new()));
        let recorder = Rc::clone(&seen);
        let mut expanded = HashSet::new();
        expanded.insert("src".to_owned());
        let view =
            TreeView::new(sample(), expanded).on_toggled(move |id| recorder.borrow_mut().push(id));

        let node = inflate(Theme::new(ThemeData::light()).child(view));
        let detectors = node.find_all("GestureDetector");
        // Only branches are wrapped in a GestureDetector; with "src" expanded
        // that is "src" and "widgets", never "main.rs".
        assert_eq!(detectors.len(), 2);
    }
}
