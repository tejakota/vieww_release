use std::fmt;

use crate::ElementId;

/// One element that has been rebuilding, and how much.
///
/// Produced by [`ElementTree::hotspots`](crate::ElementTree::hotspots).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotspot {
    /// Which element.
    pub element: ElementId,
    /// The `debug_name` of the widget it was last built from.
    pub widget_name: &'static str,
    /// Builds over this element's whole life.
    pub builds: u32,
    /// Builds since the last
    /// [`mark_builds`](crate::ElementTree::mark_builds), which is the number
    /// the list is ranked by. Equal to [`builds`](Self::builds) if nothing has
    /// ever marked.
    pub recent: u32,
}

impl fmt::Display for Hotspot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} builds={} recent={}",
            self.widget_name, self.element, self.builds, self.recent
        )
    }
}
