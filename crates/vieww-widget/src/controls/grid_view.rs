use std::fmt;

use vieww_foundation::{Axis, Key, Size};

use crate::{
    widget_node_from, BuildContext, CrossAxisAlignment, Flex, Flexible, ItemBuilder, MainAxisSize,
    ScrollMetrics, SizedBox, Widget, WidgetKind, WidgetNode,
};

use super::list_view::visible_slots;

/// A long grid that only builds the rows you can see.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{GridView, Scrollable};
/// use std::rc::Rc;
///
/// # let offset = 0.0;
/// let photos = Scrollable::vertical(offset).viewport(600.0).child(
///     GridView::new(10_000, 3, 120.0, Rc::new(|index| {
///         Text::new(format!("Photo {index}")).into()
///     }))
///     .spacing(8.0, 8.0),
/// );
/// ```
///
/// Ten thousand cells cost about three screenfuls of widgets, for the same
/// reason [`ListView`](crate::ListView) does and by the same means: the rows off
/// screen are two spacers, so the grid stays exactly as long as it should be and
/// only the *building* is skipped.
///
/// # It virtualises by row, not by cell
///
/// A row is all-or-nothing: partly-visible rows are built whole. That is what
/// keeps the arithmetic one division rather than a search, and a row is the
/// smallest unit that can be positioned without laying out the cells beside it.
///
/// # Fixed cell size, like the list
///
/// [`columns`](Self::new) cells across, each [`item_extent`](Self::new) long
/// down the scroll axis; the cross-axis size is whatever an equal share of the
/// width comes to. Same trade as `ListView`'s fixed row height, and the same
/// reason: a variable cell size means measuring every row above the window,
/// which is the cost this exists to avoid.
///
/// **The cross-axis size is not fixed and cannot be**, because nothing here
/// knows the viewport's width — [`ScrollMetrics`] carries the length along the
/// scroll axis only. So there is no `child_aspect_ratio` and asking for one
/// would mean guessing. Give the main-axis extent you want and let the columns
/// divide the width.
///
/// # A short last row does not stretch
///
/// `count` is rarely a multiple of `columns`. The missing cells are built as
/// *empty* shares of the width rather than left out, so seven photos in a
/// three-wide grid put one photo in the last row at one third of the width and
/// not at the whole of it.
///
/// # Give a stateful cell a key
///
/// Same rule as `ListView`, for the same reason: cells are reconciled by
/// position and which cell sits at a position changes as the grid scrolls. A
/// cell holding element state should be keyed by index in the builder.
#[derive(Clone)]
pub struct GridView {
    count: usize,
    columns: usize,
    item_extent: f32,
    main_spacing: f32,
    cross_spacing: f32,
    builder: ItemBuilder,
    axis: Option<Axis>,
    key: Option<Key>,
}

impl GridView {
    /// A grid of `count` cells, `columns` across, each `item_extent` long along
    /// the scroll axis.
    ///
    /// `columns` is clamped to at least one: a grid nought cells wide has no
    /// rows to divide the count into, and every arithmetic below it would be a
    /// division by zero.
    #[must_use]
    pub fn new(count: usize, columns: usize, item_extent: f32, builder: ItemBuilder) -> Self {
        Self {
            count,
            columns: columns.max(1),
            item_extent,
            main_spacing: 0.0,
            cross_spacing: 0.0,
            builder,
            axis: None,
            key: None,
        }
    }

    /// The gaps between rows and between columns.
    ///
    /// Both default to nothing, which is a grid of touching cells — right for a
    /// photo wall and wrong for almost everything else.
    #[must_use]
    pub const fn spacing(mut self, main: f32, cross: f32) -> Self {
        self.main_spacing = main;
        self.cross_spacing = cross;
        self
    }

    /// Lay out along this axis rather than the one the scrollable above uses.
    #[must_use]
    pub const fn axis(mut self, axis: Axis) -> Self {
        self.axis = Some(axis);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn columns(&self) -> usize {
        self.columns
    }

    /// How many rows the cells come to, the short last one included.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.count.div_ceil(self.columns)
    }

    /// How far apart two consecutive rows start — a cell plus the gap after it.
    #[must_use]
    pub fn pitch(&self) -> f32 {
        self.item_extent + self.main_spacing
    }

    /// The grid's whole length along the scroll axis.
    ///
    /// One [`pitch`](Self::pitch) per row **less one gap**: there are `rows`
    /// rows and only `rows - 1` gaps between them, and a grid that reported
    /// otherwise would scroll a spacing past its own last row.
    #[must_use]
    pub fn content_extent(&self) -> f32 {
        match self.rows() {
            0 => 0.0,
            rows => rows as f32 * self.pitch() - self.main_spacing,
        }
    }

    /// The half-open range of *rows* to build, given where the window is.
    ///
    /// Row-based rather than cell-based: see the type's docs for why the row is
    /// the unit.
    #[must_use]
    pub fn visible_rows(&self, metrics: Option<ScrollMetrics>) -> (usize, usize) {
        visible_slots(self.rows(), self.pitch(), metrics)
    }

    /// One row of cells, pinned to `item_extent` along `axis`.
    ///
    /// The inner flex runs *across* the scroll axis, and its
    /// [`CrossAxisAlignment::Stretch`] is what makes each cell as long as the
    /// row rather than as long as its own content.
    fn row(&self, axis: Axis, row: usize) -> WidgetNode {
        let start = row * self.columns;
        let mut cells: Vec<WidgetNode> = Vec::with_capacity(self.columns * 2);

        for column in 0..self.columns {
            if column > 0 && self.cross_spacing > 0.0 {
                cells.push(gap(axis.cross(), self.cross_spacing));
            }

            // Every column takes an equal share whether or not there is a cell
            // to put in it. An absent cell left out entirely would hand its
            // share to its neighbours, and the last row of a grid would be
            // wider cells rather than fewer.
            let flexible = Flexible::expanded(1);
            cells.push(if start + column < self.count {
                flexible.child((self.builder)(start + column)).into()
            } else {
                flexible.into()
            });
        }

        let line = Flex::new(axis.cross())
            .main_axis_size(MainAxisSize::Max)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(cells);

        // Only the main axis is pinned. The cross axis is left to the outer
        // flex's `Stretch`, which is the one that knows how wide the window is.
        match axis {
            Axis::Vertical => SizedBox::height(self.item_extent),
            Axis::Horizontal => SizedBox::width(self.item_extent),
        }
        .child(line)
        .into()
    }
}

/// A box `extent` long along `axis` and nothing across it.
///
/// The zero across is not a size: every use of this sits in a flex whose
/// `Stretch` overrides it with the real one.
fn gap(axis: Axis, extent: f32) -> WidgetNode {
    SizedBox::from_size(match axis {
        Axis::Vertical => Size::new(0.0, extent),
        Axis::Horizontal => Size::new(extent, 0.0),
    })
    .into()
}

impl Widget for GridView {
    fn debug_name(&self) -> &'static str {
        "GridView"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let metrics = ctx.inherit::<ScrollMetrics>().as_deref().copied();
        let axis = self
            .axis
            .or(metrics.map(|metrics| metrics.axis))
            .unwrap_or(Axis::Vertical);
        let rows = self.rows();
        let (first, last) = self.visible_rows(metrics);

        // The two spacers keep the grid its true length while only the middle is
        // real. Each stands in for whole rows *including the gap that precedes
        // them*, which is why they are a multiple of `pitch` and the gaps
        // between the built rows are emitted separately.
        let mut children: Vec<WidgetNode> = Vec::with_capacity((last - first) * 2 + 2);
        if first > 0 {
            children.push(gap(axis, first as f32 * self.pitch()));
        }
        for row in first..last {
            if row > first && self.main_spacing > 0.0 {
                children.push(gap(axis, self.main_spacing));
            }
            children.push(self.row(axis, row));
        }
        if last < rows {
            children.push(gap(axis, (rows - last) as f32 * self.pitch()));
        }

        Flex::new(axis)
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .children(children)
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("count", self.count.to_string()),
            ("columns", self.columns.to_string()),
            ("itemExtent", self.item_extent.to_string()),
        ]
    }
}

impl fmt::Debug for GridView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridView")
            .field("count", &self.count)
            .field("columns", &self.columns)
            .field("itemExtent", &self.item_extent)
            .finish_non_exhaustive()
    }
}

widget_node_from!(GridView);

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use crate::{inflate, DebugNode, Scrollable, Text};

    use super::*;

    const CELL: f32 = 100.0;
    const GAP: f32 = 10.0;
    const WINDOW: f32 = 500.0;

    fn cells(count: usize, columns: usize) -> GridView {
        GridView::new(
            count,
            columns,
            CELL,
            Rc::new(|index| Text::new(format!("Cell {index}")).into()),
        )
        .spacing(GAP, GAP)
    }

    fn at(offset: f32) -> ScrollMetrics {
        ScrollMetrics {
            axis: Axis::Vertical,
            offset,
            viewport: Some(WINDOW),
        }
    }

    /// The grid as built inside a scrollable at `offset`.
    fn built(grid: GridView, offset: f32) -> DebugNode {
        inflate(Scrollable::vertical(offset).viewport(WINDOW).child(grid))
    }

    #[test]
    fn a_huge_grid_builds_a_screenful_of_rows_and_not_a_row_more() {
        let tree = built(cells(10_000, 3), 0.0);
        let built_cells = tree.find_all("Text").len();

        // 500 / 110 is between four and five rows on screen, plus two rows of
        // overscan at the bottom — call it seven rows of three.
        assert!(
            (12..=24).contains(&built_cells),
            "ten thousand cells should not cost ten thousand widgets: {built_cells}"
        );
    }

    #[test]
    fn scrolling_builds_the_rows_that_are_now_on_screen() {
        let grid = cells(10_000, 3);

        assert_eq!(grid.visible_rows(Some(at(0.0))), (0, 7));
        // 5_500px down is exactly row 50, at a pitch of 110.
        assert_eq!(grid.visible_rows(Some(at(5_500.0))), (48, 57));
    }

    #[test]
    fn the_grid_is_a_gap_shorter_than_a_pitch_per_row() {
        // Nine cells three across is three rows: three cells and *two* gaps.
        let grid = cells(9, 3);
        assert_eq!(grid.rows(), 3);
        assert!((grid.content_extent() - (3.0 * CELL + 2.0 * GAP)).abs() < f32::EPSILON);
    }

    #[test]
    fn the_spacers_and_the_rows_add_up_to_the_whole_grid() {
        let grid = cells(3_000, 3);
        let expected = grid.content_extent();
        let tree = built(grid, 5_500.0);

        // The outer flex's direct children: two spacers, the built rows, and
        // the gaps between them. Anything nested inside a row is a cell.
        //
        // A `Flex` names itself for its *direction*, so the column that stacks
        // the rows of a vertical grid is `Column` and each row inside it is
        // `Row`. There is no node called `Flex` anywhere in the tree.
        let outer = tree.find("Column").expect("a column stacking the rows");
        let measured: f32 = outer
            .children
            .iter()
            .map(|child| {
                child
                    .property("height")
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or_default()
            })
            .sum();

        assert!(
            (measured - expected).abs() < 0.5,
            "spacers plus rows plus gaps must come to the content length: \
             {measured} against {expected}"
        );
    }

    #[test]
    fn a_short_last_row_keeps_its_cells_a_third_wide() {
        // Seven cells three across: the last row holds one, and it must not
        // stretch to fill the row.
        let tree = built(cells(7, 3), 0.0);
        let rows = tree.find_all("Row");
        let last = rows.last().expect("at least one row");

        assert_eq!(
            last.find_all("Flexible").len(),
            3,
            "an absent cell is an empty share of the width, not a missing child"
        );
        assert_eq!(
            tree.find_all("Text").len(),
            7,
            "and no builder is called for a cell that does not exist"
        );
    }

    #[test]
    fn a_grid_at_the_top_has_no_leading_spacer() {
        let tree = built(cells(3_000, 3), 0.0);
        let outer = tree.find("Column").expect("a column stacking the rows");
        let first = outer.children.first().expect("something in it");

        assert_eq!(
            first.name, "SizedBox",
            "the first child is the first row's box, not a spacer"
        );
        assert!(
            first.find("Text").is_some(),
            "nothing above the first row to stand in for"
        );
    }

    #[test]
    fn an_empty_grid_builds_nothing_rather_than_a_spacer_of_nothing() {
        let grid = cells(0, 3);
        assert_eq!(grid.rows(), 0);
        assert_eq!(grid.visible_rows(Some(at(0.0))), (0, 0));
        assert_eq!(grid.content_extent(), 0.0);
        assert!(built(cells(0, 3), 0.0).find("SizedBox").is_none());
    }

    #[test]
    fn scrolled_past_the_end_still_shows_the_last_row() {
        let grid = cells(20, 3);
        let (first, last) = grid.visible_rows(Some(at(100_000.0)));

        assert!(first < grid.rows(), "{first}..{last}");
        assert_eq!(last, 7, "20 cells three across is seven rows");
    }

    #[test]
    fn nought_columns_is_one_column_rather_than_a_division_by_zero() {
        let grid = GridView::new(5, 0, CELL, Rc::new(|_| Text::new("cell").into()));
        assert_eq!(grid.columns(), 1);
        assert_eq!(grid.rows(), 5);
    }

    #[test]
    fn no_spacing_emits_no_gaps_at_all() {
        let grid = GridView::new(9, 3, CELL, Rc::new(|_| Text::new("cell").into()));
        let outer = inflate(grid)
            .find("Column")
            .expect("a column stacking the rows")
            .children
            .len();

        assert_eq!(
            outer, 3,
            "three rows and nothing between them — a zero-size box per row is \
             a widget that draws nothing and costs a layout"
        );
    }

    #[test]
    fn a_horizontal_grid_is_the_transpose_of_a_vertical_one() {
        // A `Flex` names itself for its direction, so the shape reads straight
        // off the tree: a vertical grid is a `Column` of `Row`s, and a
        // horizontal one is exactly the other way round.
        let vertical = inflate(cells(9, 3));
        let sideways = inflate(cells(9, 3).axis(Axis::Horizontal));

        // `find` is pre-order, so naming the outer flex's direction reaches the
        // outer one and not a line of cells inside it.
        let pinned = |tree: &DebugNode, outer: &str| -> (Option<String>, Option<String>) {
            let line = tree
                .find(outer)
                .and_then(|flex| flex.children.first())
                .expect("a first line of cells");
            (
                line.property("width").map(str::to_owned),
                line.property("height").map(str::to_owned),
            )
        };

        // Only the axis the grid scrolls along is pinned. The other is left to
        // the outer flex's `Stretch`, which is the one that knows how wide the
        // window is.
        assert_eq!(pinned(&vertical, "Column"), (None, Some(CELL.to_string())));
        assert_eq!(pinned(&sideways, "Row"), (Some(CELL.to_string()), None));
    }

    #[test]
    fn with_no_scrollable_above_it_a_conservative_screenful_is_built() {
        let tree = inflate(cells(10_000, 3));
        let built_cells = tree.find_all("Text").len();

        assert!(
            (36..=48).contains(&built_cells),
            "unmeasured means guess a screenful, not build everything: {built_cells}"
        );
    }
}
