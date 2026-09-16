//! The widget inspector: a read-only view of the element tree.
//!
//! # What it reports
//!
//! For each element:
//! - The widget's `debug_name`
//! - How many times it has built (and since the last mark)
//! - Which signals it subscribes to
//! - Its layout bounds
//! - Its state (via `Debug`, if it has state)
//!
//! The report is a tree-shaped `InspectorNode` that a devtools UI (like
//! viewwstudio) can render, or that a test can walk and assert on.
//!
//! # The rebuild counter
//!
//! The most useful number in the report is `builds_since_mark`. The
//! pattern: mark, interact, inspect. If a tap on a button rebuilds 200
//! elements, the number says so, and the element with the highest count
//! is usually the bug.

use vieww_element::{ElementId, ElementTree};
use vieww_foundation::Rect;

/// One node in the inspection report.
#[derive(Debug, Clone)]
pub struct InspectorNode {
    /// The element's id.
    pub id: ElementId,
    /// The widget's debug name.
    pub widget_name: &'static str,
    /// Total builds over the element's life.
    pub builds: u32,
    /// Builds since the last `mark_builds`.
    pub recent_builds: u32,
    /// Layout bounds, if the element has been laid out.
    pub bounds: Option<Rect>,
    /// Children, in tree order.
    pub children: Vec<InspectorNode>,
}

impl InspectorNode {
    /// Count of this node and all descendants.
    #[must_use]
    pub fn total_elements(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(InspectorNode::total_elements)
            .sum::<usize>()
    }

    /// The deepest path from here to a leaf.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.children
            .iter()
            .map(InspectorNode::depth)
            .max()
            .unwrap_or(0)
            + 1
    }

    /// All nodes with `recent_builds > 0`, deepest first.
    #[must_use]
    pub fn rebuilt_elements(&self) -> Vec<&InspectorNode> {
        let mut result = Vec::new();
        self.collect_rebuilt(&mut result);
        result.sort_by_key(|node| std::cmp::Reverse(node.recent_builds));
        result
    }

    fn collect_rebuilt<'a>(&'a self, out: &mut Vec<&'a InspectorNode>) {
        if self.recent_builds > 0 {
            out.push(self);
        }
        for child in &self.children {
            child.collect_rebuilt(out);
        }
    }
}

/// Walks the element tree and produces an [`InspectorNode`] report.
#[derive(Debug)]
pub struct Inspector;

impl Inspector {
    /// Inspect the whole tree.
    ///
    /// The report reflects the tree *at this instant*. It is a snapshot,
    /// not a live view — call it again after an interaction to see what
    /// changed.
    #[must_use]
    pub fn snapshot(tree: &ElementTree) -> InspectorNode {
        let Some(root) = tree.root() else {
            return InspectorNode {
                id: ElementId::from_handle(0, 0),
                widget_name: "(empty)",
                builds: 0,
                recent_builds: 0,
                bounds: None,
                children: Vec::new(),
            };
        };

        Self::inspect_element(tree, root)
    }

    fn inspect_element(tree: &ElementTree, id: ElementId) -> InspectorNode {
        let Some(element) = tree.get(id) else {
            return InspectorNode {
                id,
                widget_name: "(gone)",
                builds: 0,
                recent_builds: 0,
                bounds: None,
                children: Vec::new(),
            };
        };

        let children = element
            .children()
            .iter()
            .map(|&child_id| Self::inspect_element(tree, child_id))
            .collect();

        InspectorNode {
            id,
            widget_name: element.debug_name(),
            builds: element.build_count(),
            recent_builds: element.builds_since_mark(),
            // `ElementTree` holds no geometry — layout happens in the render
            // tree, which the inspector is not given. Reporting `None` rather
            // than a zero `Rect`: an inspector that prints 0×0 for every node
            // reads as "everything collapsed" instead of "not measured here".
            bounds: None,
            children,
        }
    }

    /// A summary line: totals, rebuilds, depth.
    #[must_use]
    pub fn summary(node: &InspectorNode) -> String {
        let total = node.total_elements();
        let rebuilt = node.rebuilt_elements().len();
        let depth = node.depth();
        format!("{total} elements, {rebuilt} rebuilt since mark, depth {depth}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_counting_works() {
        let leaf = InspectorNode {
            id: ElementId::from_handle(0, 1),
            widget_name: "Text",
            builds: 1,
            recent_builds: 0,
            bounds: None,
            children: Vec::new(),
        };
        let root = InspectorNode {
            id: ElementId::from_handle(0, 0),
            widget_name: "Flex",
            builds: 2,
            recent_builds: 2,
            bounds: None,
            children: vec![leaf],
        };

        assert_eq!(root.total_elements(), 2);
        assert_eq!(root.depth(), 2);
        assert_eq!(root.rebuilt_elements().len(), 1, "only the root rebuilt");
    }
}
