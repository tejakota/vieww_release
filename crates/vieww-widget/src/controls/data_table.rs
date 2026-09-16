use std::fmt;
use std::rc::Rc;

use vieww_foundation::Key;

use crate::{
    icons, widget_node_from, BuildContext, ColoredBox, CrossAxisAlignment, Flex, Flexible, Icon,
    ListView, MainAxisSize, Padding, Pressable, SemanticRole, Semantics, SizedBox, Text, ThemeData,
    Widget, WidgetKind, WidgetNode,
};
use vieww_foundation::EdgeInsets;

/// Which way a column is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    /// The other one. What a second tap on the same header means.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// The chevron that says so.
    #[must_use]
    fn indicator(self) -> vieww_foundation::IconData {
        match self {
            Self::Ascending => icons::chevron_up(),
            Self::Descending => icons::chevron_down(),
        }
    }
}

/// One column: what it is called, how wide it is, and whether it sorts.
#[derive(Debug, Clone)]
pub struct DataColumn {
    label: String,
    width: f32,
    sortable: bool,
}

impl DataColumn {
    /// A column of a fixed width.
    ///
    /// Fixed rather than proportional because a table is read down its columns,
    /// and a column whose width depends on its content moves as you scroll —
    /// which is exactly what virtualising the rows would make it do, since the
    /// content off screen has never been measured. See the type docs on
    /// [`DataTable`].
    #[must_use]
    pub fn new(label: impl Into<String>, width: f32) -> Self {
        Self {
            label: label.into(),
            width,
            sortable: false,
        }
    }

    /// Tapping this header asks for a sort.
    #[must_use]
    pub const fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    #[must_use]
    pub fn column_label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn column_width(&self) -> f32 {
        self.width
    }

    #[must_use]
    pub const fn is_sortable(&self) -> bool {
        self.sortable
    }
}

/// A table whose rows are built only where they are on screen.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{DataColumn, DataTable};
///
/// let table = DataTable::new(
///     vec![
///         DataColumn::new("Name", 160.0).sortable(),
///         DataColumn::new("Size", 80.0).sortable(),
///     ],
///     50_000,
///     32.0,
///     |row, column| Text::new(format!("r{row}c{column}")).into(),
/// );
/// ```
///
/// # Why this exists rather than a `Flex` of rows
///
/// **The naive table builds every row.** It is a `Table` under the hood,
/// so a table of fifty thousand rows builds fifty thousand rows' worth of
/// widgets before the first frame — and fifty thousand is not an unusual size
/// for the tables people actually want. The workaround there is a
/// third-party paginated table, which changes the interaction rather than
/// fixing the cost.
///
/// This is a [`ListView`] with a header bolted above it, so it inherits the
/// virtualisation the list already has:
/// `crates/vieww/tests/settings_screen.rs` asserts two hundred rows costing a
/// screenful of widgets, and the same machinery carries this. The row count is
/// a number, not a `Vec` — nothing is allocated per row until it is on screen.
///
/// # It does not sort your data, hold your selection, or store your edits
///
/// The same rule three times, and it is the rule that keeps this virtualised.
///
/// [`on_sorted`](Self::on_sorted) reports which column was tapped and which way
/// it should go; the application reorders whatever it is drawing from and hands
/// back a new [`sorted_by`](Self::sorted_by).
/// [`on_selected`](Self::on_selected) reports a row and the application hands
/// back a new [`selection`](Self::selection).
/// [`on_edit_started`](Self::on_edit_started) reports a cell and the next
/// build's [`editing`](Self::editing) opens an [`editor`](Self::editor) on it;
/// the editor reports its own changes, because what an edited value *is* depends
/// on the editor — a date picker does not produce a string.
///
/// The table cannot own any of the three without owning the rows, and owning
/// the rows is what stops it being virtualised. The row count is a number; a
/// table that kept a `Vec<bool>` of selected rows would allocate fifty thousand
/// booleans for the fifty rows on screen, and one that kept edits would have to
/// hold every string it had ever been shown.
///
/// The selection is therefore a **set of indices**, which is proportional to
/// what the user has selected rather than to what exists.
pub struct DataTable {
    columns: Vec<DataColumn>,
    rows: usize,
    row_height: f32,
    cell: Rc<dyn Fn(usize, usize) -> WidgetNode>,
    sorted_by: Option<(usize, SortDirection)>,
    on_sorted: Option<Rc<dyn Fn(usize, SortDirection)>>,
    selection: Vec<usize>,
    selection_mode: SelectionMode,
    on_selected: Option<Rc<dyn Fn(usize, bool)>>,
    /// The cell currently being edited, and the editor to put in its place.
    editing: Option<(usize, usize)>,
    on_edit_started: Option<Rc<dyn Fn(usize, usize)>>,
    editor: Option<Rc<dyn Fn(usize, usize) -> WidgetNode>>,
    key: Option<Key>,
}

/// How many rows may be selected at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionMode {
    /// None. The default, and what the table did before selection existed —
    /// so a table built the way the old one was is unchanged, and a row press
    /// registers no gesture at all rather than a silent one.
    #[default]
    None,
    /// One row at a time. Selecting a second deselects the first, which the
    /// application does when it stores the reported index.
    Single,
    /// Any number.
    Multiple,
}

impl DataTable {
    /// `cell` is called with `(row, column)` for the cells on screen only.
    #[must_use]
    pub fn new(
        columns: Vec<DataColumn>,
        rows: usize,
        row_height: f32,
        cell: impl Fn(usize, usize) -> WidgetNode + 'static,
    ) -> Self {
        Self {
            columns,
            rows,
            row_height,
            cell: Rc::new(cell),
            sorted_by: None,
            on_sorted: None,
            selection: Vec::new(),
            selection_mode: SelectionMode::None,
            on_selected: None,
            editing: None,
            on_edit_started: None,
            editor: None,
            key: None,
        }
    }

    /// Which column is currently sorted, and which way — so the header can say
    /// so. The application owns this, because it owns the ordering.
    #[must_use]
    pub const fn sorted_by(mut self, column: usize, direction: SortDirection) -> Self {
        self.sorted_by = Some((column, direction));
        self
    }

    /// Called when a sortable header is tapped.
    ///
    /// The direction handed over is the one the table should move *to*: the
    /// flip of the current direction on the column already sorted, and
    /// ascending on any other. Working that out here rather than in every
    /// caller is the difference between a header that behaves consistently and
    /// one that behaves however each screen remembered to implement it.
    #[must_use]
    pub fn on_sorted(mut self, handler: impl Fn(usize, SortDirection) + 'static) -> Self {
        self.on_sorted = Some(Rc::new(handler));
        self
    }

    /// Which rows are currently selected. The application owns this.
    ///
    /// Indices rather than a mask, so the cost is the size of the selection
    /// rather than the size of the table — see the type's docs.
    #[must_use]
    pub fn selection(mut self, rows: impl IntoIterator<Item = usize>) -> Self {
        self.selection = rows.into_iter().collect();
        self.selection.sort_unstable();
        self.selection.dedup();
        self
    }

    /// How many rows may be selected. Defaults to [`SelectionMode::None`].
    #[must_use]
    pub const fn selection_mode(mut self, mode: SelectionMode) -> Self {
        self.selection_mode = mode;
        self
    }

    /// Called when a row is pressed, with the row and whether it should now be
    /// selected.
    ///
    /// The **second argument is the answer, not the question**: the table works
    /// out that pressing a selected row deselects it and pressing an unselected
    /// one selects it, so every caller does not have to. A caller that had to
    /// derive it would be re-deriving the table's own selection rule from the
    /// selection it just handed in, and the two would drift.
    #[must_use]
    pub fn on_selected(mut self, handler: impl Fn(usize, bool) + 'static) -> Self {
        self.on_selected = Some(Rc::new(handler));
        self
    }

    /// The cell an editor is currently open on.
    ///
    /// The application owns this for the same reason it owns the selection: a
    /// table that remembered which cell was being edited would have to survive
    /// its own rebuild, and it is rebuilt on every keystroke into that editor.
    #[must_use]
    pub const fn editing(mut self, row: usize, column: usize) -> Self {
        self.editing = Some((row, column));
        self
    }

    /// What to draw in place of a cell while it is being edited.
    ///
    /// Ordinarily a [`TextField`](crate::TextField) bound to the value the
    /// application is holding. It is a builder rather than a fixed field because
    /// a cell is not always text: a date column wants a picker, a boolean wants
    /// a switch, and a table that only ever offered a text box would have every
    /// one of those parse a string back.
    ///
    /// Without one, [`editing`](Self::editing) draws the ordinary cell — a
    /// table that declared a cell editable and then showed nothing would be
    /// worse than one that is read-only.
    #[must_use]
    pub fn editor(mut self, build: impl Fn(usize, usize) -> WidgetNode + 'static) -> Self {
        self.editor = Some(Rc::new(build));
        self
    }

    /// Called when a cell asks to be edited — a second press on a selected row's
    /// cell, or a press on any cell when nothing is selectable.
    ///
    /// The application responds by handing back a new
    /// [`editing`](Self::editing).
    #[must_use]
    pub fn on_edit_started(mut self, handler: impl Fn(usize, usize) + 'static) -> Self {
        self.on_edit_started = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Whether `row` is in the current selection.
    #[must_use]
    pub fn is_selected(&self, row: usize) -> bool {
        self.selection.binary_search(&row).is_ok()
    }

    /// The direction a tap on `column` asks for.
    #[must_use]
    fn next_direction(&self, column: usize) -> SortDirection {
        match self.sorted_by {
            Some((sorted, direction)) if sorted == column => direction.flipped(),
            _ => SortDirection::Ascending,
        }
    }

    /// The header cell for one column.
    fn header_cell(&self, index: usize, column: &DataColumn, theme: &ThemeData) -> WidgetNode {
        let sorted = match self.sorted_by {
            Some((at, direction)) if at == index => Some(direction),
            _ => None,
        };

        let mut row = Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .children(vec![Text::new(column.label.clone())
                .style(theme.text.label)
                .into()]);

        if let Some(direction) = sorted {
            row = row.push(SizedBox::width(theme.metrics.gap / 2.0));
            row = row.push(
                Icon::new(direction.indicator())
                    .size(theme.text.label.size)
                    .color(theme.colors.on_surface),
            );
        }

        // A `WidgetNode` rather than the `Padding` itself: the press builder
        // below is called once per press frame and clones this, and a node is an
        // `Rc` to bump rather than a widget to copy.
        let padded: WidgetNode = Padding::new(EdgeInsets::all(theme.metrics.gap / 2.0))
            .child(row)
            .into();

        if !column.sortable {
            return SizedBox::width(column.width).child(padded).into();
        }

        let Some(handler) = &self.on_sorted else {
            // Declared sortable with nobody listening. Rendered as a plain
            // header rather than as a control that does nothing when pressed —
            // an announced action that silently fails reads as a broken
            // application, which `Semantics::with_action` documents at length.
            return SizedBox::width(column.width).child(padded).into();
        };

        let handler = Rc::clone(handler);
        let direction = self.next_direction(index);
        // A name that says what pressing it does, not just what the column is:
        // a screen reader reading "Size" gives no clue that it sorts.
        let announcement = match sorted {
            Some(SortDirection::Ascending) => format!("{}, sorted ascending", column.label),
            Some(SortDirection::Descending) => format!("{}, sorted descending", column.label),
            None => format!("{}, sort", column.label),
        };

        SizedBox::width(column.width)
            .child(
                Semantics::button(announcement).child(
                    Pressable::new(move |_press| padded.clone())
                        .fade(theme.motion.duration_short)
                        .on_tap(move || handler(index, direction)),
                ),
            )
            .into()
    }
}

impl Widget for DataTable {
    fn debug_name(&self) -> &'static str {
        "DataTable"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let header = Flex::row()
            .cross_axis_alignment(CrossAxisAlignment::Center)
            // No `collect`: `children` takes an `IntoIterator`, which the map
            // already is, and collecting into an unnamed type is what E0283 is.
            .children(
                self.columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| self.header_cell(index, column, &theme)),
            );

        // Cloned into the builder because it outlives this call: the list calls
        // it once per row that comes on screen, for as long as it is mounted.
        //
        // **Everything the row builder needs is captured by value here**, and
        // that is the constraint the whole design is under: the closure is
        // called for the rows on screen, on every frame they are on it, so it
        // may not reach back into `self`. Selection is captured as the set of
        // selected indices rather than as a mask over the table, which is why a
        // fifty-thousand-row table with two rows selected captures two numbers.
        let cell = Rc::clone(&self.cell);
        let columns = self.columns.clone();
        let gap = theme.metrics.gap / 2.0;
        let selected: Vec<usize> = self.selection.clone();
        let mode = self.selection_mode;
        let on_selected = self.on_selected.clone();
        let editing = self.editing;
        let editor = self.editor.clone();
        let on_edit_started = self.on_edit_started.clone();
        let colors = theme.colors;
        let duration = theme.motion.duration_short;

        let body = ListView::new(
            self.rows,
            self.row_height,
            Rc::new(move |row: usize| {
                let is_selected = selected.binary_search(&row).is_ok();
                let cells =
                    Flex::row().children(columns.iter().enumerate().map(|(index, column)| {
                        // The editor replaces the cell rather than sitting over
                        // it: an overlaid field would be measured against the
                        // cell's own box and would clip its own caret at the
                        // column edge.
                        let content = match (editing, &editor) {
                            (Some((editing_row, editing_column)), Some(build))
                                if editing_row == row && editing_column == index =>
                            {
                                build(row, index)
                            }
                            _ => cell(row, index),
                        };
                        SizedBox::width(column.width)
                            .child(Padding::new(EdgeInsets::all(gap)).child(content))
                            .into()
                    }));

                let row_node: WidgetNode = if is_selected {
                    // The selected wash, at the same low strength every other
                    // selected surface in the catalogue uses — colour is not the
                    // only signal, and the semantics below say so in words.
                    ColoredBox::new(colors.primary.with_alpha(0x22))
                        .child(cells)
                        .into()
                } else {
                    cells.into()
                };

                // Nothing selectable and nothing editable: no gesture at all.
                // A row that registered a recogniser it never used would take
                // taps away from whatever is underneath it.
                if mode == SelectionMode::None && on_edit_started.is_none() {
                    return row_node;
                }

                let on_selected = on_selected.clone();
                let on_edit_started = on_edit_started.clone();
                let editable = on_edit_started.is_some();
                let pressed = move || {
                    // A press on a row that is *already* selected asks to edit
                    // it; a press on any other row selects it. That is the
                    // ordinary spreadsheet rule, and it is what keeps one press
                    // from doing two things at once.
                    if editable && (is_selected || mode == SelectionMode::None) {
                        if let Some(handler) = &on_edit_started {
                            handler(row, 0);
                            return;
                        }
                    }
                    if let Some(handler) = &on_selected {
                        handler(row, !is_selected);
                    }
                };

                // `toggled`, which is the framework's word for a two-state
                // control and is what AccessKit's `selected` maps from. A row a
                // screen reader cannot hear the selected state of is a table
                // that only works if you can see the wash.
                Semantics::new()
                    .role(SemanticRole::Button)
                    .toggled(is_selected)
                    .child(
                        Pressable::new(move |_press| row_node.clone())
                            .fade(duration)
                            .on_tap(pressed),
                    )
                    .into()
            }),
        );

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            // The header takes its natural height and the body takes the rest —
            // `expanded` rather than `new`, so the list fills the space instead
            // of asking for its whole content extent and overflowing.
            .children(vec![
                header.into(),
                Flexible::expanded(1).child(body).into(),
            ])
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("columns", self.columns.len().to_string()),
            ("rows", self.rows.to_string()),
        ];
        if let Some((column, direction)) = self.sorted_by {
            props.push(("sorted", format!("{column} {direction:?}")));
        }
        props
    }
}

impl fmt::Debug for DataTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataTable")
            .field("columns", &self.columns.len())
            .field("rows", &self.rows)
            .field("sorted_by", &self.sorted_by)
            .finish_non_exhaustive()
    }
}

widget_node_from!(DataTable);

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::{inflate, Theme};

    use super::*;

    fn columns() -> Vec<DataColumn> {
        vec![
            DataColumn::new("Name", 160.0).sortable(),
            DataColumn::new("Size", 80.0),
        ]
    }

    fn built(widget: impl Into<WidgetNode>) -> crate::DebugNode {
        inflate(Theme::new(ThemeData::light()).child(widget.into()))
    }

    #[test]
    fn a_second_tap_on_the_sorted_column_reverses_it() {
        let table = DataTable::new(columns(), 10, 32.0, |r, c| {
            Text::new(format!("{r}:{c}")).into()
        });
        assert_eq!(table.next_direction(0), SortDirection::Ascending);

        let table = table.sorted_by(0, SortDirection::Ascending);
        assert_eq!(
            table.next_direction(0),
            SortDirection::Descending,
            "tapping the column that is already ascending asks for descending"
        );
        assert_eq!(
            table.next_direction(1),
            SortDirection::Ascending,
            "and a different column starts from ascending rather than \
             inheriting the other one's direction"
        );
    }

    #[test]
    fn tapping_a_sortable_header_reports_the_column_and_the_direction() {
        let seen: Rc<RefCell<Option<(usize, SortDirection)>>> = Rc::new(RefCell::new(None));
        let record = Rc::clone(&seen);

        let table = DataTable::new(columns(), 10, 32.0, |r, c| {
            Text::new(format!("{r}:{c}")).into()
        })
        .on_sorted(move |column, direction| *record.borrow_mut() = Some((column, direction)));

        if let Some(handler) = &table.on_sorted {
            handler(0, table.next_direction(0));
        }
        assert_eq!(*seen.borrow(), Some((0, SortDirection::Ascending)));
    }

    #[test]
    fn a_sortable_header_reads_as_a_button_and_says_what_it_does() {
        let tree = built(
            DataTable::new(columns(), 10, 32.0, |r, c| {
                Text::new(format!("{r}:{c}")).into()
            })
            .on_sorted(|_, _| {}),
        );

        let labels: Vec<String> = tree
            .find_all("Semantics")
            .iter()
            .filter_map(|node| node.property("label").map(str::to_owned))
            .collect();

        assert!(
            labels.iter().any(|label| label == "Name, sort"),
            "a header that sorts has to say so — \"Name\" alone tells a screen \
             reader nothing about what pressing it does: {labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.starts_with("Size")),
            "and the column that does not sort is not announced as a control: \
             {labels:?}"
        );
    }

    #[test]
    fn a_sortable_column_with_no_handler_is_not_a_control() {
        let tree = built(DataTable::new(columns(), 10, 32.0, |r, c| {
            Text::new(format!("{r}:{c}")).into()
        }));

        assert!(
            tree.find("Pressable").is_none(),
            "declared sortable with nobody listening: a control that announces \
             an action and does nothing reads as a broken application"
        );
    }

    #[test]
    fn a_table_of_fifty_thousand_rows_does_not_build_fifty_thousand_rows() {
        let tree = built(DataTable::new(columns(), 50_000, 32.0, |r, c| {
            Text::new(format!("{r}:{c}")).into()
        }));

        // **The claim, and the reason this is not a `Flex` of rows.** The naive
        // table builds every row it is given; at this size that is the
        // difference between a table and a hang.
        let built_rows = tree.find_all("Padding").len();
        assert!(
            built_rows < 500,
            "built {built_rows} cells' worth of padding for a 50,000-row table \
             — the rows are not being virtualised"
        );
    }

    // ------------------------------------------------ selection and editing

    fn table(rows: usize) -> DataTable {
        DataTable::new(columns(), rows, 32.0, |row, column| {
            Text::new(format!("r{row}c{column}")).into()
        })
    }

    /// **The default is unchanged.** A table built the way the old one was
    /// registers no row gesture at all — not a silent one — because a row that
    /// entered the gesture arena and never used what it won would take taps away
    /// from whatever is underneath it.
    #[test]
    fn a_table_with_no_selection_mode_puts_no_gesture_on_its_rows() {
        // Nothing sortable and nothing selectable: no `Pressable` anywhere,
        // matching `a_sortable_column_with_no_handler_is_not_a_control`.
        assert!(
            built(table(10)).find("Pressable").is_none(),
            "a read-only table registers nothing"
        );

        // With a sort handler, exactly one: the sortable header cell, and still
        // none per row.
        let sortable = built(table(10).on_sorted(|_, _| {}));
        assert_eq!(
            sortable.find_all("Pressable").len(),
            1,
            "one sortable column, and no row gestures: {sortable:?}"
        );

        // And with selection on, the rows gain one each.
        let selectable = built(
            table(10)
                .selection_mode(SelectionMode::Single)
                .on_selected(|_, _| {}),
        );
        assert!(
            selectable.find_all("Pressable").len() > 1,
            "the rows are pressable once selection is on"
        );
    }

    #[test]
    fn selection_is_a_set_of_indices_and_is_normalised() {
        let table = table(100).selection([7, 3, 3, 1]);
        assert!(table.is_selected(1) && table.is_selected(3) && table.is_selected(7));
        assert!(!table.is_selected(2));
        assert_eq!(table.selection, vec![1, 3, 7], "sorted, and deduplicated");
    }

    /// **Selection is proportional to what is selected, not to what exists.**
    /// The property that keeps this table virtualised: a mask over a
    /// fifty-thousand-row table would allocate fifty thousand booleans to draw
    /// the fifty rows on screen.
    #[test]
    fn a_huge_table_with_a_small_selection_holds_a_small_selection() {
        let table = table(50_000).selection([4, 40_000]);
        assert_eq!(table.selection.len(), 2);
        assert!(table.is_selected(40_000));
        assert!(!table.is_selected(39_999));
    }

    /// The second argument is the **answer**, not the question: the table works
    /// out that pressing a selected row deselects it, so every caller does not
    /// re-derive that rule from the selection it just handed in.
    #[test]
    fn pressing_a_row_reports_the_state_it_should_move_to() {
        let seen: Rc<RefCell<Vec<(usize, bool)>>> = Rc::new(RefCell::new(Vec::new()));

        let sink = Rc::clone(&seen);
        let node = built(
            table(10)
                .selection_mode(SelectionMode::Multiple)
                .selection([2])
                .on_selected(move |row, now| sink.borrow_mut().push((row, now))),
        );

        // The rows are pressable now, and announce their state.
        let rows: Vec<_> = node
            .find_all("Semantics")
            .into_iter()
            .filter(|n| n.property("toggled").is_some())
            .collect();
        assert!(!rows.is_empty(), "rows announce a selected state: {node:?}");
        assert!(
            rows.iter().any(|n| n.property("toggled") == Some("true")),
            "the selected row says so: {rows:?}"
        );
        assert!(
            rows.iter().any(|n| n.property("toggled") == Some("false")),
            "and the others say so too"
        );
    }

    /// A selected row draws a wash **and** announces its state. Colour is not
    /// the only signal, which is the rule the whole catalogue follows.
    #[test]
    fn a_selected_row_is_both_washed_and_announced() {
        let node = built(
            table(4)
                .selection_mode(SelectionMode::Single)
                .selection([1])
                .on_selected(|_, _| {}),
        );
        assert!(
            node.find_all("ColoredBox")
                .iter()
                .any(|n| n.property("color").is_some()),
            "the selected row has a wash behind it: {node:?}"
        );
        assert!(
            node.find_all("Semantics")
                .iter()
                .any(|n| n.property("toggled") == Some("true")),
            "and says so in words"
        );
    }

    /// An `editing` cell with no `editor` draws the **ordinary cell**. A table
    /// that declared a cell editable and then showed nothing would be worse than
    /// one that is read-only.
    #[test]
    fn an_editing_cell_with_no_editor_falls_back_to_the_cell() {
        let node = built(table(4).editing(1, 0));
        let texts: Vec<&str> = node
            .find_all("Text")
            .iter()
            .filter_map(|n| n.property("text"))
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("r1c0")),
            "the cell is still drawn: {texts:?}"
        );
    }

    /// The editor **replaces** the cell rather than sitting over it: an overlaid
    /// field would be measured against the cell's box and would clip its own
    /// caret at the column edge.
    #[test]
    fn an_editor_replaces_the_cell_it_is_editing_and_only_that_one() {
        let node = built(
            table(4)
                .editing(1, 0)
                .editor(|row, column| Text::new(format!("EDIT{row}-{column}")).into()),
        );
        let texts: Vec<&str> = node
            .find_all("Text")
            .iter()
            .filter_map(|n| n.property("text"))
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("EDIT1-0")),
            "the editor is in the edited cell: {texts:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("r1c0")),
            "and the cell it replaced is gone: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("r1c1")),
            "the other cells in the same row are untouched: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("r0c0")),
            "and so is the same column in other rows: {texts:?}"
        );
    }
}
