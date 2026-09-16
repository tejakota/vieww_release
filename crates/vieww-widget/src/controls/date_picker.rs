//! A month grid, and the arrows that move it.
//!
//! # Controlled, like everything else here
//!
//! [`DatePicker`] owns no state. The caller passes the month being shown *and*
//! the selection, and is told when either should change. That is the same shape
//! [`Menu`](crate::Menu), [`Navigator`](crate::Navigator) and every other
//! control in this crate has, and it is what makes a picker driven by a form,
//! restored from storage, or animated between months possible without the
//! widget knowing about any of them.
//!
//! It also means **the month is not derived from the selection**. A user who
//! opens a picker on a date in March and pages to May without choosing anything
//! is looking at May, and a widget that recomputed the month from the selection
//! would snap back to March on the next rebuild.
//!
//! # Where the words come from
//!
//! [`CalendarNames`], through [`Localizations`](crate::Localizations) — a
//! `Locale` in this framework carries a language, a direction and a plural rule,
//! and no month names at all. English by default. See that type for why nothing
//! here bundles CLDR.

use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Date, EdgeInsets, Key, Size};

use crate::{
    widget_node_from, BuildContext, CalendarNames, Center, DecoratedBox, Flex, Flexible,
    GestureDetector, Handler, Localizations, Padding, Semantics, SizedBox, Text, ThemeData, Widget,
    WidgetKind, WidgetNode,
};

/// How many week rows a month grid draws.
///
/// **Always six, never "as many as this month needs".** A 28-day February
/// starting on the first column of the week needs four; a 31-day month starting
/// on the last needs six. A grid that changed height as the user paged would
/// move the arrows under their finger, and on a phone that means paging twice by
/// accident. The cost is a blank row in about a fifth of months.
const WEEK_ROWS: usize = 6;

const DAYS_IN_WEEK: usize = 7;

/// A calendar for one month.
///
/// ```
/// # use std::rc::Rc;
/// # use vieww_widget::prelude::*;
/// # use vieww_widget::DatePicker;
/// # use vieww_foundation::Date;
/// # fn demo(shown: Date, chosen: Option<Date>) {
/// let picker = DatePicker::new(shown)
///     .selected(chosen)
///     .today(Date::new(2026, 8, 14).unwrap())
///     .on_selected(Rc::new(|date| println!("chose {date}")))
///     .on_month_changed(Rc::new(|month| println!("showing {month}")));
/// # }
/// ```
pub struct DatePicker {
    month: Date,
    selected: Option<Date>,
    today: Option<Date>,
    on_selected: Option<Handler<Date>>,
    on_month_changed: Option<Handler<Date>>,
    key: Option<Key>,
}

impl DatePicker {
    /// A picker showing the month `month` falls in.
    ///
    /// The day component is kept rather than normalised to the 1st, because it
    /// is what [`Date::next_month`] carries forward when the arrows are used —
    /// so paging March→April→March returns to the day the caller started on
    /// instead of drifting to the first.
    #[must_use]
    pub const fn new(month: Date) -> Self {
        Self {
            month,
            selected: None,
            today: None,
            on_selected: None,
            on_month_changed: None,
            key: None,
        }
    }

    /// The chosen day, if there is one.
    #[must_use]
    pub const fn selected(mut self, selected: Option<Date>) -> Self {
        self.selected = selected;
        self
    }

    /// Which day to mark as today.
    ///
    /// **Passed in, never read from a clock.** There is no clock service in this
    /// framework, and a widget that called one could not be tested — every
    /// assertion about "today" would depend on the day the suite ran. Omitting
    /// it simply means no day is marked.
    #[must_use]
    pub const fn today(mut self, today: Date) -> Self {
        self.today = Some(today);
        self
    }

    /// Called with the day the user chose.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<Date>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Called with the new anchor when an arrow is pressed.
    ///
    /// Without this the arrows do nothing, because the month is the caller's
    /// state. That is deliberate and it is the same bargain every controlled
    /// widget here makes.
    #[must_use]
    pub fn on_month_changed(mut self, handler: Handler<Date>) -> Self {
        self.on_month_changed = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The day drawn in a given cell, or `None` where the grid is padding.
    ///
    /// Pulled out of `build` because it is the only real arithmetic here and it
    /// is worth testing without a tree: `lead` is how many columns the 1st sits
    /// from the start of the week, which is the whole of "why does the month
    /// begin on a Wednesday".
    fn day_at(&self, names: &CalendarNames, cell: usize) -> Option<u8> {
        let first = Date::first_of_month(self.month.year(), self.month.month())?;
        let lead = first.weekday().columns_from(names.first_day) as usize;
        let days = first.days_in_its_month() as usize;

        // `checked_sub` rather than a comparison: the leading blanks are exactly
        // the cells whose index is below `lead`, and the subtraction says so.
        let day = cell.checked_sub(lead)?;
        if day < days {
            #[allow(clippy::cast_possible_truncation)]
            Some(day as u8 + 1)
        } else {
            None
        }
    }
}

impl Widget for DatePicker {
    fn debug_name(&self) -> &'static str {
        "DatePicker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("month", self.month.to_string()),
            (
                "selected",
                self.selected
                    .map_or_else(|| String::from("none"), |d| d.to_string()),
            ),
        ]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let names = Localizations::calendar_of(ctx);
        let cell = theme.metrics.touch_target;

        let mut rows: Vec<WidgetNode> = Vec::with_capacity(WEEK_ROWS + 2);
        rows.push(self.header(&names, &theme, cell));
        rows.push(self.weekday_headings(&names, &theme, cell));

        for week in 0..WEEK_ROWS {
            let mut cells: Vec<WidgetNode> = Vec::with_capacity(DAYS_IN_WEEK);
            for column in 0..DAYS_IN_WEEK {
                cells.push(self.cell(&names, &theme, cell, week * DAYS_IN_WEEK + column));
            }
            rows.push(Flex::row().children(cells).into());
        }

        Semantics::container("Calendar")
            .child(Flex::column().children(rows))
            .into()
    }
}

impl DatePicker {
    /// The month name, the year, and the two arrows.
    fn header(&self, names: &CalendarNames, theme: &ThemeData, cell: f32) -> WidgetNode {
        let label = format!("{} {}", names.month(self.month.month()), self.month.year());

        Flex::row()
            .children(vec![
                self.arrow(theme, cell, Direction::Previous),
                // The label takes the space the arrows do not, so the arrows sit
                // at the ends of the grid rather than beside the text — which is
                // where a thumb reaches on a phone.
                Flexible::expanded(1)
                    .child(
                        Center::new().child(
                            Text::new(label)
                                .style(theme.text.title)
                                .color(theme.colors.on_surface),
                        ),
                    )
                    .into(),
                self.arrow(theme, cell, Direction::Next),
            ])
            .into()
    }

    fn arrow(&self, theme: &ThemeData, cell: f32, direction: Direction) -> WidgetNode {
        let (glyph, label) = match direction {
            Direction::Previous => ("‹", "Previous month"),
            Direction::Next => ("›", "Next month"),
        };

        let target = SizedBox::from_size(Size::new(cell, cell)).child(
            Center::new().child(
                Text::new(glyph)
                    .style(theme.text.title)
                    .color(theme.colors.on_surface_variant),
            ),
        );

        let node: WidgetNode = match &self.on_month_changed {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let month = match direction {
                    Direction::Previous => self.month.previous_month(),
                    Direction::Next => self.month.next_month(),
                };
                GestureDetector::new()
                    .on_tap(move |_| handler(month))
                    .child(target)
                    .into()
            }
            // Inert but still drawn and still announced, matching `Menu`'s
            // disabled rows: an arrow that vanished from the hit test while
            // looking identical is the worse failure.
            None => target.into(),
        };

        Semantics::button(label)
            .enabled(self.on_month_changed.is_some())
            .child(node)
            .into()
    }

    /// One row of column headings, in display order.
    fn weekday_headings(&self, names: &CalendarNames, theme: &ThemeData, cell: f32) -> WidgetNode {
        let headings = (0..DAYS_IN_WEEK)
            .map(|column| {
                #[allow(clippy::cast_possible_truncation)]
                let heading = names.weekday_column(column as u8);
                SizedBox::from_size(Size::new(cell, cell * 0.6))
                    .child(
                        Center::new().child(
                            Text::new(heading)
                                .style(theme.text.label)
                                .color(theme.colors.on_surface_variant),
                        ),
                    )
                    .into()
            })
            .collect::<Vec<WidgetNode>>();

        // Excluded from semantics: a screen reader announcing "S M T W T F S"
        // before every month is noise, and each day cell already carries its own
        // full date.
        crate::ExcludeSemantics::new(true)
            .child(Flex::row().children(headings))
            .into()
    }

    /// One day, or an empty box where the grid pads.
    fn cell(
        &self,
        names: &CalendarNames,
        theme: &ThemeData,
        size: f32,
        index: usize,
    ) -> WidgetNode {
        let Some(day) = self.day_at(names, index) else {
            // A blank of the same size, not a zero-width nothing: the columns
            // have to line up whether or not the month reaches this cell.
            return SizedBox::from_size(Size::new(size, size)).into();
        };

        let Some(date) = Date::new(self.month.year(), self.month.month(), day) else {
            return SizedBox::from_size(Size::new(size, size)).into();
        };

        let is_selected = self.selected == Some(date);
        let is_today = self.today == Some(date);

        let ink = if is_selected {
            theme.colors.on_primary
        } else if is_today {
            theme.colors.primary
        } else {
            theme.colors.on_surface
        };

        let number =
            Center::new().child(Text::new(day.to_string()).style(theme.text.body).color(ink));

        // A stadium radius, so the marker is a circle in a square cell. Selected
        // wins over today when they are the same day: filled-and-outlined at once
        // reads as neither.
        let marked: WidgetNode = if is_selected {
            DecoratedBox::rounded(theme.colors.primary, f32::MAX)
                .child(number)
                .into()
        } else if is_today {
            DecoratedBox::outlined(theme.colors.primary, 1.0, f32::MAX)
                .child(number)
                .into()
        } else {
            number.into()
        };

        // Inset, so neighbouring circles do not touch across a 7-column row.
        let target = SizedBox::from_size(Size::new(size, size))
            .child(Padding::new(EdgeInsets::all(2.0)).child(marked));

        let node: WidgetNode = match &self.on_selected {
            Some(handler) => {
                let handler = Rc::clone(handler);
                GestureDetector::new()
                    .on_tap(move |_| handler(date))
                    .child(target)
                    .into()
            }
            None => target.into(),
        };

        // The full date, not the day number: "14" alone is meaningless read out
        // of a grid a screen reader user cannot see the shape of.
        let mut label = format!("{} {} {}", day, names.month(date.month()), date.year());
        if is_today {
            label.push_str(", today");
        }

        Semantics::button(label)
            .toggled(is_selected)
            .enabled(self.on_selected.is_some())
            .child(node)
            .into()
    }
}

/// Which arrow.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Previous,
    Next,
}

impl fmt::Debug for DatePicker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatePicker")
            .field("month", &self.month)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

widget_node_from!(DatePicker);

#[cfg(test)]
mod tests {
    use super::*;
    // Only the tests name a weekday: the widget reads `first_day` off the
    // names it is given rather than deciding which day starts a week.
    use vieww_foundation::Weekday;

    fn august_2026() -> DatePicker {
        DatePicker::new(Date::new(2026, 8, 1).unwrap())
    }

    #[test]
    fn the_first_of_the_month_lands_in_the_column_its_weekday_names() {
        // 1 August 2026 is a Saturday. Sunday-first English puts it in the last
        // column, so cells 0..=5 are blank and cell 6 is the 1st.
        let names = CalendarNames::english();
        let picker = august_2026();

        for cell in 0..6 {
            assert_eq!(picker.day_at(&names, cell), None, "cell {cell}");
        }
        assert_eq!(picker.day_at(&names, 6), Some(1));
        assert_eq!(picker.day_at(&names, 7), Some(2));
    }

    #[test]
    fn the_same_month_starts_elsewhere_when_the_week_starts_elsewhere() {
        // The identical month, Monday-first: Saturday is now the sixth column,
        // so the 1st moves from cell 6 to cell 5. Same arithmetic, different
        // `first_day` — which is the reason that field exists.
        let names = CalendarNames {
            first_day: Weekday::Monday,
            ..CalendarNames::english()
        };
        let picker = august_2026();

        assert_eq!(picker.day_at(&names, 4), None);
        assert_eq!(picker.day_at(&names, 5), Some(1));
    }

    #[test]
    fn the_grid_runs_out_after_the_last_day() {
        // August has 31 days starting at cell 6, so the last is cell 36 and
        // everything after it is padding rather than a wrapped 1st.
        let names = CalendarNames::english();
        let picker = august_2026();

        assert_eq!(picker.day_at(&names, 36), Some(31));
        assert_eq!(picker.day_at(&names, 37), None);
        assert_eq!(
            picker.day_at(&names, WEEK_ROWS * DAYS_IN_WEEK - 1),
            None,
            "the last cell of a six-row grid is blank for this month"
        );
    }

    #[test]
    fn february_in_a_leap_year_has_a_twenty_ninth() {
        let names = CalendarNames::english();
        // 1 February 2024 is a Thursday: four blanks, then the 1st at cell 4.
        let picker = DatePicker::new(Date::new(2024, 2, 1).unwrap());
        assert_eq!(picker.day_at(&names, 4), Some(1));
        assert_eq!(picker.day_at(&names, 32), Some(29));
        assert_eq!(picker.day_at(&names, 33), None, "and no thirtieth");
    }

    #[test]
    fn a_month_that_needs_six_rows_fits_in_six_rows() {
        // The worst case: 31 days beginning on the last column of the week needs
        // 1 + 31 = 37 cells, and six rows give 42. A five-row grid would lose a
        // day, which is why WEEK_ROWS is what it is.
        let names = CalendarNames::english();
        // 1 May 2027 is a Saturday — last column, Sunday-first.
        let picker = DatePicker::new(Date::new(2027, 5, 1).unwrap());
        assert_eq!(picker.day_at(&names, 6), Some(1));
        assert_eq!(picker.day_at(&names, 36), Some(31));
        // At compile time rather than at run time: the grid being big enough for
        // the worst month is a property of the two constants, so a build that
        // shrank `WEEK_ROWS` should fail to compile rather than fail a test.
        const { assert!(WEEK_ROWS * DAYS_IN_WEEK >= 37) };
    }

    #[test]
    fn the_grid_is_read_from_the_month_and_not_from_the_selection() {
        // Showing May while March is selected must draw May. A picker that
        // derived the month from the selection would snap back on every rebuild
        // and the arrows would appear not to work.
        let names = CalendarNames::english();
        let picker = DatePicker::new(Date::new(2026, 5, 1).unwrap())
            .selected(Some(Date::new(2026, 3, 9).unwrap()));

        // 1 May 2026 is a Friday: five blanks, the 1st at cell 5, 31 days.
        assert_eq!(picker.day_at(&names, 5), Some(1));
        assert_eq!(picker.day_at(&names, 35), Some(31));
    }

    #[test]
    fn the_anchor_keeps_its_day_across_the_arrows() {
        // Paging away and back returns to the day the caller started on, rather
        // than drifting to the 1st — which is why `new` does not normalise.
        let anchor = Date::new(2026, 8, 14).unwrap();
        let picker = DatePicker::new(anchor);
        assert_eq!(picker.month.next_month().previous_month(), anchor);
    }
}
