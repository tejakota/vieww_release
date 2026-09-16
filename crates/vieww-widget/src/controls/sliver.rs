//! Scrolling screens made of slivers.
//!
//! [`Scrollable`](crate::Scrollable) and [`ListView`](crate::ListView) scroll
//! **one** thing. These scroll a *sequence* of differently-shaped things sharing
//! one scroll position, which is what a real screen is: a header that collapses,
//! then a grid, then a list, then a footer.
//!
//! The mechanism is the sliver protocol — see `vieww_render::sliver`. What
//! matters here is that a child of a [`CustomScrollView`] may be an ordinary
//! widget: anything that is not a sliver is adapted automatically, so
//! `Text::new("…")` between two lists needs no wrapper.

use vieww_foundation::{Axis, Key};

use crate::{widget_node_from, Widget, WidgetKind, WidgetNode};

/// A scrolling viewport whose children are slivers.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{CustomScrollView, SliverAppBar, SliverList};
///
/// let screen = CustomScrollView::vertical().children(vec![
///     SliverAppBar::new(200.0, 56.0).pinned().child(Text::new("Orders")).into(),
///     SliverList::new(10_000, 64.0).into(),
///     // Not a sliver, and it does not need to be.
///     Text::new("That is everything.").into(),
/// ]);
/// ```
#[derive(Debug, Clone)]
pub struct CustomScrollView {
    axis: Axis,
    offset: f32,
    children: Vec<WidgetNode>,
    key: Option<Key>,
}

impl CustomScrollView {
    #[must_use]
    pub const fn new(axis: Axis) -> Self {
        Self {
            axis,
            offset: 0.0,
            children: Vec::new(),
            key: None,
        }
    }

    #[must_use]
    pub const fn vertical() -> Self {
        Self::new(Axis::Vertical)
    }

    #[must_use]
    pub const fn horizontal() -> Self {
        Self::new(Axis::Horizontal)
    }

    /// Where the scroll currently is.
    ///
    /// Owned by the caller — usually a
    /// [`ScrollController`](crate::Scrollable) signal — exactly as
    /// [`Viewport`](crate::Viewport)'s is. The viewport is told where it is and
    /// does not decide, which is what keeps the physics in one place.
    #[must_use]
    pub const fn offset(mut self, offset: f32) -> Self {
        self.offset = offset;
        self
    }

    #[must_use]
    pub fn children(mut self, children: Vec<WidgetNode>) -> Self {
        self.children = children;
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.children.push(child.into());
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    #[must_use]
    pub const fn scroll_offset(&self) -> f32 {
        self.offset
    }
}

impl Widget for CustomScrollView {
    fn debug_name(&self) -> &'static str {
        "CustomScrollView"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderMultiChild(&self.children)
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("axis", format!("{:?}", self.axis)),
            ("offset", self.offset.to_string()),
        ]
    }
}

widget_node_from!(CustomScrollView);

/// A run of equal-height rows inside a [`CustomScrollView`].
///
/// Only the visible rows are built. `count` may be enormous; what it costs is
/// what is on screen.
///
/// # `builder` versus `children`
///
/// A list of ten thousand rows cannot take a `Vec` of ten thousand widgets — the
/// vector alone defeats the purpose. So rows come from a closure over an index,
/// and the widget builds the window the last layout asked for.
#[derive(Clone)]
pub struct SliverList {
    count: usize,
    item_extent: f32,
    /// The window the previous layout wanted, fed back in by the caller.
    first_index: usize,
    builder: Option<std::rc::Rc<dyn Fn(usize) -> WidgetNode>>,
    children: Vec<WidgetNode>,
    key: Option<Key>,
}

impl std::fmt::Debug for SliverList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliverList")
            .field("count", &self.count)
            .field("item_extent", &self.item_extent)
            .field("first_index", &self.first_index)
            .field("built", &self.children.len())
            .finish_non_exhaustive()
    }
}

impl SliverList {
    /// `count` rows, each `item_extent` long.
    #[must_use]
    pub fn new(count: usize, item_extent: f32) -> Self {
        Self {
            count,
            item_extent,
            first_index: 0,
            builder: None,
            children: Vec::new(),
            key: None,
        }
    }

    /// Build a row on demand.
    #[must_use]
    pub fn builder(mut self, builder: impl Fn(usize) -> WidgetNode + 'static) -> Self {
        self.builder = Some(std::rc::Rc::new(builder));
        self
    }

    /// Build the rows in `first..last` now.
    ///
    /// Called by the caller with the window the previous frame's layout asked
    /// for — `RenderSliverFixedList::visible_range`. Separate from
    /// [`builder`](Self::builder) because *which* rows to build is a layout
    /// answer and building them is a widget-layer job, and the frame boundary
    /// between the two is where a rebuild belongs.
    #[must_use]
    pub fn window(mut self, first: usize, last: usize) -> Self {
        let Some(builder) = self.builder.clone() else {
            return self;
        };
        let last = last.min(self.count);
        self.first_index = first.min(last);
        self.children = (self.first_index..last)
            .map(|index| builder(index))
            .collect();
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn item_extent(&self) -> f32 {
        self.item_extent
    }

    #[must_use]
    pub const fn first_index(&self) -> usize {
        self.first_index
    }
}

impl Widget for SliverList {
    fn debug_name(&self) -> &'static str {
        "SliverList"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderMultiChild(&self.children)
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("count", self.count.to_string()),
            ("built", self.children.len().to_string()),
        ]
    }
}

widget_node_from!(SliverList);

/// A header that collapses as the content scrolls under it.
///
/// The widget the whole sliver protocol was added for — see
/// `vieww_render::RenderSliverAppBar` for why a box layout cannot express it.
#[derive(Debug, Clone)]
pub struct SliverAppBar {
    max_extent: f32,
    min_extent: f32,
    pinned: bool,
    floating: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl SliverAppBar {
    /// A bar `max_extent` tall at rest, collapsing to `min_extent`.
    #[must_use]
    pub const fn new(max_extent: f32, min_extent: f32) -> Self {
        Self {
            max_extent,
            min_extent,
            pinned: false,
            floating: false,
            child: None,
            key: None,
        }
    }

    /// Stay on screen at the minimum extent once collapsed.
    #[must_use]
    pub const fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    /// Come back as soon as the user scrolls up, without waiting for the top.
    #[must_use]
    pub const fn floating(mut self) -> Self {
        self.floating = true;
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
    pub const fn extents(&self) -> (f32, f32) {
        (self.max_extent, self.min_extent)
    }

    #[must_use]
    pub const fn is_pinned(&self) -> bool {
        self.pinned
    }

    #[must_use]
    pub const fn is_floating(&self) -> bool {
        self.floating
    }
}

impl Widget for SliverAppBar {
    fn debug_name(&self) -> &'static str {
        "SliverAppBar"
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
}

widget_node_from!(SliverAppBar);

/// A pull-to-refresh control at the top of a [`CustomScrollView`].
///
/// Place it **first**. It lives in the gap that opens when the content is
/// dragged past its start, and that gap is above exactly one sliver.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{CustomScrollView, SliverList, SliverRefresh};
///
/// # let refreshing = false;
/// let screen = CustomScrollView::vertical().children(vec![
///     SliverRefresh::new(80.0).refreshing(refreshing).into(),
///     SliverList::new(200, 64.0).into(),
/// ]);
/// ```
///
/// # It does not decide when to refresh
///
/// `refreshing` is handed in, exactly as a scroll offset is. Deciding that a
/// drag ended past the trigger is a *gesture* question and belongs to whatever
/// owns the scroll position; this widget shows the state and nothing else. The
/// same division as every other control here — see the module docs on
/// controlled rather than self-managing.
#[derive(Debug, Clone)]
pub struct SliverRefresh {
    trigger_extent: f32,
    refreshing: bool,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl SliverRefresh {
    /// A control that arms once the content is pulled `trigger_extent` past its
    /// start.
    #[must_use]
    pub const fn new(trigger_extent: f32) -> Self {
        Self {
            trigger_extent,
            refreshing: false,
            child: None,
            key: None,
        }
    }

    /// Whether a refresh is currently running.
    #[must_use]
    pub const fn refreshing(mut self, refreshing: bool) -> Self {
        self.refreshing = refreshing;
        self
    }

    /// What to show — a spinner, usually.
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
    pub const fn trigger_extent(&self) -> f32 {
        self.trigger_extent
    }

    #[must_use]
    pub const fn is_refreshing(&self) -> bool {
        self.refreshing
    }
}

impl Widget for SliverRefresh {
    fn debug_name(&self) -> &'static str {
        "SliverRefresh"
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
}

widget_node_from!(SliverRefresh);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Text;

    #[test]
    fn a_scroll_view_takes_children_of_mixed_kinds() {
        // The claim in the module docs: an ordinary widget between two slivers
        // needs no adapter.
        let view = CustomScrollView::vertical()
            .child(SliverAppBar::new(200.0, 56.0).pinned())
            .child(SliverList::new(100, 50.0))
            .child(Text::new("footer"));

        assert!(
            matches!(view.kind(), WidgetKind::RenderMultiChild(children) if children.len() == 3)
        );
    }

    #[test]
    fn a_sliver_list_builds_only_the_window_it_is_given() {
        let list = SliverList::new(10_000, 50.0)
            .builder(|index| Text::new(format!("row {index}")).into())
            .window(100, 120);

        let WidgetKind::RenderMultiChild(children) = list.kind() else {
            panic!("expected children");
        };
        assert_eq!(children.len(), 20, "20 of 10,000");
        assert_eq!(list.first_index(), 100);
    }

    #[test]
    fn a_window_past_the_end_is_clamped_rather_than_building_nothing() {
        let list = SliverList::new(10, 50.0)
            .builder(|index| Text::new(index.to_string()).into())
            .window(5, 500);

        let WidgetKind::RenderMultiChild(children) = list.kind() else {
            panic!("expected children");
        };
        assert_eq!(children.len(), 5);
    }

    #[test]
    fn a_list_with_no_builder_builds_nothing_rather_than_panicking() {
        let list = SliverList::new(100, 50.0).window(0, 20);
        let WidgetKind::RenderMultiChild(children) = list.kind() else {
            panic!("expected children");
        };
        assert!(children.is_empty());
    }

    #[test]
    fn a_refresh_control_carries_its_state_to_the_factory() {
        let control = SliverRefresh::new(80.0).refreshing(true);
        assert_eq!(control.trigger_extent(), 80.0);
        assert!(control.is_refreshing());
    }

    #[test]
    fn a_bar_with_no_child_is_a_leaf() {
        assert!(matches!(
            SliverAppBar::new(200.0, 56.0).kind(),
            WidgetKind::RenderLeaf
        ));
    }
}
