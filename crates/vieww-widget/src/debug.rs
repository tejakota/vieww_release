//! Walking a widget tree and printing it.
//!
//! This is Phase 1's exit test made permanent: with no window, no GPU and no
//! element tree, you can build a tree in memory and confirm its structure is
//! what you meant.
//!
//! [`inflate`] is deliberately *not* the element tree. It builds composed
//! widgets and throws the results away, keeping no identity and no state. Phase
//! 2 replaces the recursion here with one that mounts elements and keeps them —
//! the shape of the walk is the same, which is the point of doing it now.

use std::fmt::{self, Write as _};

use vieww_foundation::Key;

use crate::{BuildContext, WidgetKind, WidgetNode};

/// A snapshot of one widget in an inflated tree.
#[derive(Debug, Clone)]
pub struct DebugNode {
    /// The widget's [`debug_name`](crate::Widget::debug_name).
    pub name: String,
    /// The widget's key, if it declared one.
    pub key: Option<Key>,
    /// Which [`WidgetKind`] variant the widget reported.
    pub kind: &'static str,
    /// Fields the widget chose to expose.
    pub properties: Vec<(&'static str, String)>,
    /// How deep this node sits below the root.
    pub depth: usize,
    /// For a composed widget, the subtree its `build` returned. For everything
    /// else, its declared children in paint order.
    pub children: Vec<DebugNode>,
}

impl DebugNode {
    /// Total node count including this one.
    #[must_use]
    pub fn len(&self) -> usize {
        1 + self.children.iter().map(Self::len).sum::<usize>()
    }

    /// Always `false` — a `DebugNode` is itself a node. Present because clippy
    /// asks for it alongside [`len`](Self::len).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Depth-first iterator over this node and all its descendants.
    pub fn iter(&self) -> impl Iterator<Item = &Self> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            // Pushed in reverse so siblings come out in declaration order.
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }

    /// The first descendant (or self) whose name matches, depth-first.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Self> {
        self.iter().find(|node| node.name == name)
    }

    /// Every descendant (or self) whose name matches.
    #[must_use]
    pub fn find_all(&self, name: &str) -> Vec<&Self> {
        self.iter().filter(|node| node.name == name).collect()
    }

    /// The value of an exposed property.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The chain of names from this node down to its first leaf, which is the
    /// useful assertion for a composed widget's expansion.
    #[must_use]
    pub fn spine(&self) -> Vec<&str> {
        let mut names = Vec::new();
        let mut cursor = self;
        loop {
            names.push(cursor.name.as_str());
            match cursor.children.first() {
                Some(child) => cursor = child,
                None => return names,
            }
        }
    }
}

impl fmt::Display for DebugNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write_node(&mut out, self, "", true, true)?;
        f.write_str(out.trim_end())
    }
}

/// Walk a widget tree, building every composed widget along the way.
///
/// Composed widgets appear in the result *and* so does what they built, so the
/// dump shows both what you wrote and what it expanded to.
///
/// # Panics
///
/// Propagates any panic from a widget's `build`. There is no error boundary
/// here by design — that is Phase 10, and swallowing build panics before then
/// would hide exactly the bugs this function exists to surface.
#[must_use]
pub fn inflate(root: impl Into<WidgetNode>) -> DebugNode {
    inflate_node(&root.into(), &BuildContext::root())
}

/// Walk a widget tree and render it as an indented string.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// let dump = vieww_widget::debug_tree(
///     Flex::column().children(children![Text::new("a"), Text::new("b")]),
/// );
/// assert!(dump.starts_with("Column"));
/// ```
#[must_use]
pub fn debug_tree(root: impl Into<WidgetNode>) -> String {
    inflate(root).to_string()
}

fn inflate_node(node: &WidgetNode, ctx: &BuildContext) -> DebugNode {
    let kind = node.kind();
    let mut debug = DebugNode {
        name: display_name(node),
        key: node.key().cloned(),
        kind: kind.tag(),
        properties: node.debug_properties(),
        depth: ctx.depth(),
        children: Vec::new(),
    };

    match kind {
        WidgetKind::Composed => {
            // The built subtree sits one level deeper, so a composed widget's
            // expansion is visible as nesting rather than replacing it.
            let built = node.build(ctx);
            debug.children.push(inflate_node(&built, &ctx.child()));
        }
        WidgetKind::Inherited(child) => {
            // The payload type is erased behind `dyn Widget`, so the widget
            // pushes its own value into the scope.
            let child_ctx = node.publish_inherited(ctx).unwrap_or_else(|| ctx.child());
            debug.children.push(inflate_node(child, &child_ctx));
        }
        WidgetKind::RenderLeaf => {}
        WidgetKind::RenderSingleChild(child) => {
            debug.children.push(inflate_node(child, &ctx.child()));
        }
        WidgetKind::RenderMultiChild(children) => {
            let child_ctx = ctx.child();
            debug.children = children
                .iter()
                .map(|child| inflate_node(child, &child_ctx))
                .collect();
        }
    }

    debug
}

/// Render one node and its subtree with box-drawing connectors.
fn write_node(
    out: &mut String,
    node: &DebugNode,
    prefix: &str,
    is_last: bool,
    is_root: bool,
) -> fmt::Result {
    if is_root {
        writeln!(out, "{}", node_line(node))?;
    } else {
        let connector = if is_last { "└─ " } else { "├─ " };
        writeln!(out, "{prefix}{connector}{}", node_line(node))?;
    }

    let child_prefix = if is_root {
        String::new()
    } else {
        format!("{prefix}{}", if is_last { "   " } else { "│  " })
    };

    let last_index = node.children.len().saturating_sub(1);
    for (index, child) in node.children.iter().enumerate() {
        write_node(out, child, &child_prefix, index == last_index, false)?;
    }
    Ok(())
}

fn node_line(node: &DebugNode) -> String {
    let mut line = node.name.clone();
    if let Some(key) = &node.key {
        let _ = write!(line, " {key}");
    }
    if !node.properties.is_empty() {
        let props: Vec<String> = node
            .properties
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        let _ = write!(line, " ({})", props.join(", "));
    }
    line
}

/// `Inherited<T>` reports `"Inherited"` from `debug_name` because that method
/// returns `&'static str`; splice the payload type back in for readability.
fn display_name(node: &WidgetNode) -> String {
    let name = node.debug_name();
    if name == "Inherited" {
        if let Some(provides) = node
            .debug_properties()
            .iter()
            .find(|(key, _)| *key == "provides")
        {
            return format!("Inherited<{}>", provides.1);
        }
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use vieww_foundation::EdgeInsets;

    use super::*;
    use crate::prelude::*;

    #[test]
    fn a_leaf_inflates_to_a_single_node() {
        let tree = inflate(Text::new("hello"));
        assert_eq!(tree.name, "Text");
        assert_eq!(tree.kind, "render:leaf");
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.property("text"), Some(r#""hello""#));
    }

    #[test]
    fn multi_child_nodes_keep_declaration_order() {
        let tree = inflate(Flex::row().children(children![
            Text::new("first"),
            Text::new("second"),
            Text::new("third"),
        ]));
        let texts: Vec<&str> = tree
            .find_all("Text")
            .iter()
            .filter_map(|node| node.property("text"))
            .collect();
        assert_eq!(texts, [r#""first""#, r#""second""#, r#""third""#]);
    }

    #[test]
    fn depth_increases_down_the_tree() {
        let tree =
            inflate(Flex::column().children(children![Padding::all(4.0).child(Text::new("deep"))]));
        assert_eq!(tree.depth, 0);
        assert_eq!(tree.find("Padding").unwrap().depth, 1);
        assert_eq!(tree.find("Text").unwrap().depth, 2);
    }

    #[test]
    fn keys_survive_inflation() {
        let tree = inflate(Flex::row().children(children![Text::new("x").key("greeting")]));
        assert_eq!(tree.find("Text").unwrap().key, Some(Key::str("greeting")));
    }

    #[test]
    fn rendering_uses_box_drawing_connectors() {
        let dump = debug_tree(Flex::column().children(children![Text::new("a"), Text::new("b"),]));
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("Column"));
        assert!(lines[1].starts_with("├─ Text"));
        assert!(lines[2].starts_with("└─ Text"));
    }

    #[test]
    fn a_padded_container_expands_in_the_documented_order() {
        let tree = inflate(
            Container::new()
                .padding(EdgeInsets::all(8.0))
                .color(Color::WHITE)
                .child(Text::new("hi")),
        );
        assert_eq!(
            tree.spine(),
            ["Container", "ColoredBox", "Padding", "Text"],
            "padding must sit inside the color so the background covers it"
        );
    }

    #[test]
    fn an_empty_container_still_builds_something() {
        let tree = inflate(Container::new());
        assert_eq!(tree.spine(), ["Container", "SizedBox", "Constrained"]);
    }
}
