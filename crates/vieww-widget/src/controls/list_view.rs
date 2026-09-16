use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Axis, Key, Size};

use crate::{
    widget_node_from, BuildContext, CrossAxisAlignment, Flex, MainAxisSize, ScrollMetrics,
    SizedBox, Widget, WidgetKind, WidgetNode,
};

/// How many rows either side of the window are built anyway.
///
/// One screen would be extravagant and none would show a blank row for the
/// frame between a scroll and the rebuild it causes. Two rows is enough to cover
/// a frame's worth of fling at any believable speed.
const OVERSCAN: usize = 2;

/// Builds one row.
pub type ItemBuilder = Rc<dyn Fn(usize) -> WidgetNode>;

/// How long row `index` is along the scroll axis.
///
/// Asked during **build**, which is before anything has been laid out — so it
/// has to be arithmetic the application already knows, not a measurement. A feed
/// knows an image's aspect ratio, a chat knows how many lines a message came to
/// last time it was shown, and a settings screen knows which of its rows are
/// two-line. See [`ListView::variable`] for what to do when nothing knows.
pub type ExtentBuilder = Rc<dyn Fn(usize) -> f32>;

/// Where the window falls, and what stands in for everything outside it.
///
/// The `before`/`after` lengths are carried rather than recomputed from the
/// indices, because for a variable list they are *not* a multiplication — they
/// are the walk that produced the indices in the first place, and asking twice
/// means walking twice.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    /// First row to build.
    first: usize,
    /// One past the last row to build.
    last: usize,
    /// The content length above `first`.
    before: f32,
    /// The content length below `last`.
    after: f32,
}

impl Span {
    /// Nothing to show, and nothing to stand in for it.
    const EMPTY: Self = Self {
        first: 0,
        last: 0,
        before: 0.0,
        after: 0.0,
    };
}

/// The half-open range of equal `pitch`-long slots a window covers.
///
/// Clamped to `count`, widened by [`OVERSCAN`] at both ends, and never empty for
/// a non-empty list — a window scrolled past the end still shows the last slot
/// rather than nothing.
///
/// Shared with [`GridView`](crate::GridView), whose slots are rows of cells
/// rather than rows of one, and whose pitch therefore carries the spacing
/// between rows as well as their height. This function is the whole of what the
/// two have in common: the grid does *not* build itself out of a `ListView`,
/// because a list's spacers are `count * item_extent` and a grid's content is a
/// gap shorter than `rows * pitch` — one row's spacing, which would show up as a
/// scroll overrun past the last row.
pub(crate) fn visible_slots(
    count: usize,
    pitch: f32,
    metrics: Option<ScrollMetrics>,
) -> (usize, usize) {
    if count == 0 || pitch <= 0.0 {
        return (0, 0);
    }

    let (top, bottom) = match metrics.and_then(ScrollMetrics::visible) {
        Some(window) => window,
        // Nothing has measured a window yet. A screenful of slots is the
        // conservative guess: too many is a wasted build, too few is a visibly
        // empty list.
        None => (0.0, pitch * 12.0),
    };

    let first = (top / pitch).floor().max(0.0) as usize;
    let last = (bottom / pitch).ceil().max(0.0) as usize;

    // Clamp `first` before `last` leans on it: a window scrolled far past the
    // end produces a first slot of thousands, and a `last` derived from the
    // unclamped value would name slots that do not exist.
    let first = first.saturating_sub(OVERSCAN).min(count.saturating_sub(1));
    let last = last.saturating_add(OVERSCAN).min(count).max(first + 1);
    (first, last)
}

/// A long list that only builds the rows you can see.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{ListView, Scrollable};
/// use std::rc::Rc;
///
/// # let offset = 0.0;
/// let list = Scrollable::vertical(offset).viewport(600.0).child(
///     ListView::new(10_000, 56.0, Rc::new(|index| {
///         Text::new(format!("Row {index}")).into()
///     })),
/// );
/// ```
///
/// Ten thousand rows cost about a dozen widgets: the ones on screen, plus a
/// spacer standing in for everything above and another for everything below. The
/// list is still exactly as long as it should be, so the scrollbar, the extents
/// reported out of layout and the scroll physics all see the true content —
/// only the *building* is skipped.
///
/// # Two ways to say how long a row is
///
/// [`new`](Self::new) gives every row the same length, and finding the window is
/// then one division. [`variable`](Self::variable) asks a closure per row, and
/// finding the window is a walk from the top — read that constructor before
/// reaching for it, because the two differ in cost as well as in shape.
///
/// Either way the *lengths* are declared rather than measured. Nothing here lays
/// a row out to find out how tall it is: layout happens after build, so a row's
/// real height is not knowable at the moment the decision has to be made.
///
/// # Give a stateful row a key
///
/// The rows are reconciled by *position*, and which row sits at a given position
/// changes as the list scrolls — row 100 is the first child at one offset and the
/// second at the next. For rows that are pure description that is invisible. For
/// a row that holds element state — an expanded section, a half-typed field — it
/// means the state follows the position rather than the row, so the builder
/// should key its widget by index. Doing it here instead would need a keyed
/// wrapper around every row, which is a widget per row to solve a problem most
/// lists do not have.
///
/// # It reads its window from the scrollable above it
///
/// Through [`ScrollMetrics`], published by [`Scrollable`](crate::Scrollable).
/// With no scrollable above — or before the first layout has measured one — it
/// builds a conservative first screenful, which is correct and merely
/// unvirtualised for one frame.
#[derive(Clone)]
pub struct ListView {
    count: usize,
    extents: Extents,
    builder: ItemBuilder,
    axis: Option<Axis>,
    key: Option<Key>,
}

/// How a list answers "how long is row `index`".
///
/// One widget rather than two, the same call as
/// [`Flexible`](crate::Flexible)'s: the difference is one field and a different
/// piece of arithmetic, and a second type would mean a second element, a second
/// entry in every re-export, and two lists to keep in step.
#[derive(Clone)]
enum Extents {
    /// Every row the same. Where the window falls is one division.
    Uniform(f32),
    /// Row by row. Where the window falls is a walk from the top.
    PerRow(ExtentBuilder),
}

impl ListView {
    /// A list of `count` rows, each `item_extent` long, built on demand.
    #[must_use]
    pub fn new(count: usize, item_extent: f32, builder: ItemBuilder) -> Self {
        Self {
            count,
            extents: Extents::Uniform(item_extent),
            builder,
            axis: None,
            key: None,
        }
    }

    /// A list of `count` rows of **differing** lengths, built on demand.
    ///
    /// ```
    /// use vieww_widget::prelude::*;
    /// use vieww_widget::{ListView, Scrollable};
    /// use std::rc::Rc;
    ///
    /// # let offset = 0.0;
    /// // A chat: their messages are one line, ours are two.
    /// let heights: vieww_widget::ExtentBuilder =
    ///     Rc::new(|index| if index % 2 == 0 { 44.0 } else { 76.0 });
    /// let thread = Scrollable::vertical(offset).viewport(600.0).child(
    ///     ListView::variable(2_000, heights, Rc::new(|index| {
    ///         Text::new(format!("Message {index}")).into()
    ///     })),
    /// );
    /// ```
    ///
    /// # The cost, plainly
    ///
    /// A uniform list finds its window with a division and does no work at all
    /// per row it does not build. This one **walks every row from the top**,
    /// because where row *n* starts is the sum of everything before it and there
    /// is no shortcut through a closure. Two walks, in fact: one to find the
    /// window and the total, one to add up what is above it.
    ///
    /// So `extents` must be cheap — a lookup or a multiplication, never a
    /// shaping pass. At a few nanoseconds a call, a hundred thousand rows is
    /// well inside a frame and a million is not. If your list is longer than
    /// that, it is uniform or it is paged.
    ///
    /// # It still does not measure anything
    ///
    /// `extents` is asked during build, and build happens before layout — so a
    /// row's *real* height is not available at the moment the window has to be
    /// decided, and a closure that tried to shape text to find out would be
    /// doing the work this exists to skip.
    ///
    /// When nothing knows the height in advance, the honest options are to give
    /// a reasonable constant per kind of row and accept that the scrollbar is an
    /// estimate, or to measure a row the first time it is built and feed the
    /// answer back in through a signal — which the framework does not yet do for
    /// you. Do not pass a closure that guesses differently on each build: the
    /// content length changes underneath the scroll offset and the list walks
    /// while nobody is touching it.
    ///
    /// # A negative length is nothing
    ///
    /// Extents are clamped at zero. A row of negative height would make the
    /// prefix sums non-monotonic, and every "which row is at this offset"
    /// answer after it would be wrong rather than merely odd.
    #[must_use]
    pub fn variable(count: usize, extents: ExtentBuilder, builder: ItemBuilder) -> Self {
        Self {
            count,
            extents: Extents::PerRow(extents),
            builder,
            axis: None,
            key: None,
        }
    }

    /// Lay out along this axis rather than the one the scrollable above uses.
    ///
    /// Rarely wanted: a list that runs across the direction it scrolls in shows
    /// one row.
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

    /// The half-open range of rows to build, given where the window is.
    ///
    /// Clamped to the list, widened by `OVERSCAN` at both ends, and never
    /// empty for a non-empty list — a window scrolled past the end still shows
    /// the last row rather than nothing.
    #[must_use]
    pub fn visible_range(&self, metrics: Option<ScrollMetrics>) -> (usize, usize) {
        let span = self.span(metrics);
        (span.first, span.last)
    }

    /// The list's whole length along the scroll axis.
    ///
    /// A multiplication for a uniform list and a walk for a variable one, which
    /// is the same difference as everywhere else here.
    #[must_use]
    pub fn content_extent(&self) -> f32 {
        match &self.extents {
            Extents::Uniform(extent) => self.count as f32 * extent.max(0.0),
            Extents::PerRow(of) => {
                let of = of.as_ref();
                (0..self.count).map(|index| of(index).max(0.0)).sum()
            }
        }
    }

    /// Where the window falls and what stands in for the rest of the list.
    fn span(&self, metrics: Option<ScrollMetrics>) -> Span {
        match &self.extents {
            Extents::Uniform(extent) => {
                let (first, last) = visible_slots(self.count, *extent, metrics);
                Span {
                    first,
                    last,
                    before: first as f32 * extent,
                    after: (self.count - last) as f32 * extent,
                }
            }
            Extents::PerRow(of) => self.variable_span(of, metrics),
        }
    }

    /// [`span`](Self::span) for a list whose rows differ, which is a walk.
    ///
    /// Deliberately one function rather than a `visible_slots` sibling plus a
    /// prefix-sum helper: the indices and the two spacer lengths come out of the
    /// same traversal, and splitting them would mean traversing twice to learn
    /// things that were both known the first time.
    fn variable_span(&self, of: &ExtentBuilder, metrics: Option<ScrollMetrics>) -> Span {
        if self.count == 0 {
            return Span::EMPTY;
        }
        // Through the `Rc` once, here: `Rc<dyn Fn>` is not itself `Fn`, and the
        // two loops below call it a hundred thousand times between them.
        let of = of.as_ref();

        let (top, bottom) = match metrics.and_then(ScrollMetrics::visible) {
            Some(window) => window,
            // Nothing has measured a window yet. There is no pitch to guess a
            // screenful from, so the first row stands in for one — the same
            // conservative dozen the uniform case takes, off a length that at
            // least came from this list.
            None => (0.0, of(0).max(1.0) * 12.0),
        };

        // First pass: which rows the window touches, and how long the list is.
        // `count` is the sentinel for "not found yet" in both, which is also the
        // right answer for a window that starts past the end.
        let mut cursor = 0.0_f32;
        let mut first = self.count;
        let mut last = self.count;
        for index in 0..self.count {
            let start = cursor;
            cursor += of(index).max(0.0);

            if first == self.count && cursor > top {
                first = index;
            }
            if last == self.count && start >= bottom {
                last = index;
            }
        }
        let total = cursor;

        // A window entirely past the end still shows the last row, exactly as
        // the uniform case does.
        let first = first.min(self.count - 1).saturating_sub(OVERSCAN);
        let last = last.saturating_add(OVERSCAN).min(self.count).max(first + 1);

        // Second pass, and only as far as `last`: the spacer lengths. It cannot
        // be folded into the first, because the overscan moves both ends after
        // the walk that would have had to capture them.
        let mut before = 0.0_f32;
        let mut through = 0.0_f32;
        for index in 0..last {
            let extent = of(index).max(0.0);
            if index < first {
                before += extent;
            }
            through += extent;
        }

        Span {
            first,
            last,
            before,
            after: (total - through).max(0.0),
        }
    }
}

impl Widget for ListView {
    fn debug_name(&self) -> &'static str {
        "ListView"
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
        let span = self.span(metrics);

        let spacer = |extent: f32| -> WidgetNode {
            SizedBox::from_size(match axis {
                Axis::Vertical => Size::new(0.0, extent),
                Axis::Horizontal => Size::new(extent, 0.0),
            })
            .into()
        };

        // The two spacers are what keep the list its true length while only the
        // middle is real. Without them a scrolled list would be as short as the
        // rows it happens to have built, and its own scroll offset would be
        // measuring against a content length that changes as it scrolls.
        //
        // They are driven by the span's *lengths* rather than by its indices:
        // for a variable list `first` says nothing about how much content is
        // above it, and multiplying by a row height there is none of is how the
        // spacers and the rows stop adding up.
        let mut children: Vec<WidgetNode> = Vec::with_capacity(span.last - span.first + 2);
        if span.before > 0.0 {
            children.push(spacer(span.before));
        }
        for index in span.first..span.last {
            children.push((self.builder)(index));
        }
        if span.after > 0.0 {
            children.push(spacer(span.after));
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
            ("itemExtent", self.extents.to_string()),
        ]
    }
}

impl fmt::Display for Extents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uniform(extent) => write!(f, "{extent}"),
            // Named rather than sampled: printing row 0's length would read as
            // the list's row height and be wrong for every other row.
            Self::PerRow(_) => f.write_str("variable"),
        }
    }
}

impl fmt::Debug for Extents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Debug for ListView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListView")
            .field("count", &self.count)
            .field("itemExtent", &self.extents)
            .finish_non_exhaustive()
    }
}

widget_node_from!(ListView);

#[cfg(test)]
mod tests {
    use crate::{inflate, DebugNode, Scrollable, Text};

    use super::*;

    const ROW: f32 = 50.0;
    const WINDOW: f32 = 500.0;

    fn rows(count: usize) -> ListView {
        ListView::new(
            count,
            ROW,
            Rc::new(|index| Text::new(format!("Row {index}")).into()),
        )
    }

    fn at(offset: f32) -> ScrollMetrics {
        ScrollMetrics {
            axis: Axis::Vertical,
            offset,
            viewport: Some(WINDOW),
        }
    }

    /// The list as built inside a scrollable at `offset`.
    fn built(count: usize, offset: f32) -> DebugNode {
        inflate(
            Scrollable::vertical(offset)
                .viewport(WINDOW)
                .child(rows(count)),
        )
    }

    #[test]
    fn a_huge_list_builds_a_screenful_and_not_a_row_more() {
        let tree = built(10_000, 0.0);
        let built_rows = tree.find_all("Text").len();

        // 500/50 = 10 on screen, plus overscan at the bottom.
        assert!(
            (10..=14).contains(&built_rows),
            "ten thousand rows should not cost ten thousand widgets: {built_rows}"
        );
    }

    #[test]
    fn scrolling_builds_the_rows_that_are_now_on_screen() {
        let list = rows(10_000);

        assert_eq!(list.visible_range(Some(at(0.0))), (0, 12));
        // 5000px down is row 100.
        assert_eq!(list.visible_range(Some(at(5_000.0))), (98, 112));
    }

    #[test]
    fn the_spacers_keep_the_list_its_true_length() {
        let tree = built(1_000, 5_000.0);
        let spacers = tree.find_all("SizedBox");

        assert_eq!(spacers.len(), 2, "one above the window and one below");
        let heights: Vec<f32> = spacers
            .iter()
            .filter_map(|node| node.property("height"))
            .filter_map(|value| value.parse().ok())
            .collect();
        let rows_stood_in_for: f32 = heights.iter().sum::<f32>() / ROW;
        let built_rows = tree.find_all("Text").len() as f32;

        assert!(
            (rows_stood_in_for + built_rows - 1000.0).abs() < 0.5,
            "spacers plus real rows must add up to the whole list: \
             {rows_stood_in_for} + {built_rows}"
        );
    }

    #[test]
    fn a_list_at_the_top_has_no_leading_spacer() {
        let tree = built(1_000, 0.0);
        assert_eq!(
            tree.find_all("SizedBox").len(),
            1,
            "nothing above the first row to stand in for"
        );
    }

    #[test]
    fn an_empty_list_builds_nothing_rather_than_a_spacer_of_nothing() {
        assert_eq!(rows(0).visible_range(Some(at(0.0))), (0, 0));
        assert!(built(0, 0.0).find("SizedBox").is_none());
    }

    #[test]
    fn scrolled_past_the_end_still_shows_the_last_row() {
        let list = rows(20);
        let (first, last) = list.visible_range(Some(at(100_000.0)));

        assert!(first < list.count(), "{first}..{last}");
        assert_eq!(last, 20);
    }

    #[test]
    fn with_no_scrollable_above_it_a_conservative_screenful_is_built() {
        let tree = inflate(rows(10_000));
        let built_rows = tree.find_all("Text").len();

        assert!(
            (12..=16).contains(&built_rows),
            "unmeasured means guess a screenful, not build everything: {built_rows}"
        );
    }

    // ------------------------------------------------------- variable extents

    /// Alternating rows: even ones 40 long, odd ones 80. Two rows to 120, so
    /// every offset below is checkable by hand and every sum is exact in `f32`.
    fn alternating(count: usize) -> ListView {
        ListView::variable(
            count,
            Rc::new(|index| if index % 2 == 0 { 40.0 } else { 80.0 }),
            Rc::new(|index| Text::new(format!("Row {index}")).into()),
        )
    }

    /// The heights of the spacers around the built rows, in order.
    fn spacer_extents(tree: &DebugNode) -> Vec<f32> {
        tree.find_all("SizedBox")
            .iter()
            .filter_map(|node| node.property("height"))
            .filter_map(|value| value.parse().ok())
            .collect()
    }

    #[test]
    fn a_variable_list_is_as_long_as_its_rows_add_up_to() {
        assert_eq!(alternating(10).content_extent(), 600.0);
        assert_eq!(rows(1_000).content_extent(), 50_000.0);
    }

    #[test]
    fn the_window_lands_on_the_rows_a_walk_says_it_does() {
        let list = alternating(1_000);

        // Row 500 starts at exactly 30_000; the overscan takes it back to 498.
        assert_eq!(list.visible_range(Some(at(30_000.0))), (498, 511));
        assert_eq!(list.visible_range(Some(at(0.0))), (0, 11));
    }

    #[test]
    fn a_uniform_multiplication_would_get_this_wrong() {
        // The point of the walk, stated as a test: 30_000 into a list whose
        // rows average 60 is row 500, and dividing by either row height gives
        // 375 or 750 — neither of them within a screen of the answer.
        let (first, _) = alternating(1_000).visible_range(Some(at(30_000.0)));
        assert_eq!(first, 498);
        assert_ne!(first, (30_000.0 / 40.0) as usize - OVERSCAN);
        assert_ne!(first, (30_000.0 / 80.0) as usize - OVERSCAN);
    }

    #[test]
    fn the_spacers_are_the_content_above_and_below_rather_than_a_row_count() {
        let list = alternating(1_000);
        let total = list.content_extent();
        let tree = inflate(Scrollable::vertical(30_000.0).viewport(WINDOW).child(list));

        let spacers = spacer_extents(&tree);
        assert_eq!(spacers, vec![29_880.0, 29_360.0]);

        // Rows 498..511: seven short ones and six tall.
        let built: f32 = 7.0 * 40.0 + 6.0 * 80.0;
        assert!(
            (spacers.iter().sum::<f32>() + built - total).abs() < 0.5,
            "spacers plus real rows must add up to the whole list"
        );
    }

    #[test]
    fn a_variable_list_at_the_top_has_no_leading_spacer() {
        let tree = inflate(
            Scrollable::vertical(0.0)
                .viewport(WINDOW)
                .child(alternating(1_000)),
        );
        assert_eq!(
            spacer_extents(&tree).len(),
            1,
            "nothing above the first row"
        );
    }

    #[test]
    fn an_empty_variable_list_builds_nothing() {
        let list = alternating(0);
        assert_eq!(list.visible_range(Some(at(0.0))), (0, 0));
        assert_eq!(list.content_extent(), 0.0);
    }

    #[test]
    fn scrolled_past_the_end_a_variable_list_still_shows_its_last_row() {
        let list = alternating(20);
        let (first, last) = list.visible_range(Some(at(100_000.0)));

        assert_eq!((first, last), (17, 20));
    }

    #[test]
    fn a_negative_row_is_nothing_rather_than_a_row_that_walks_backwards() {
        // Without the clamp the prefix sums stop increasing, and every "which
        // row is at this offset" answer after the negative one is wrong.
        let list = ListView::variable(
            10,
            Rc::new(|index| if index == 0 { -100.0 } else { 50.0 }),
            Rc::new(|index| Text::new(format!("Row {index}")).into()),
        );

        assert_eq!(
            list.content_extent(),
            450.0,
            "nine rows of fifty, not seven"
        );
    }

    #[test]
    fn an_unmeasured_variable_list_guesses_off_its_own_first_row() {
        // There is no pitch to guess a screenful from, so row 0 stands in for
        // one: 40 * 12 = 480, which reaches row 8 before the overscan.
        let tree = inflate(alternating(1_000));
        let built_rows = tree.find_all("Text").len();

        assert_eq!(built_rows, 10);
    }
}
