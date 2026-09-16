//! Two columns of numbers, and an optional AM/PM.
//!
//! # Why a list and not a clock face
//!
//! The classic design draws a dial; this draws columns. A dial needs a drag gesture that
//! maps an angle to a value, a second mode for minutes, and a hit target that is
//! an annulus rather than a rectangle — and it is *worse* with a keyboard, worse
//! with a screen reader, and unusable at small sizes. Columns are two lists of
//! buttons, which every part of this framework already knows how to lay out,
//! announce and test.
//!
//! # Controlled, like [`DatePicker`](crate::DatePicker)
//!
//! The caller owns the value and is told what the user asked for. See that
//! type's docs for why every control in this crate is shaped this way.

use std::fmt;
use std::rc::Rc;

use vieww_foundation::{EdgeInsets, HalfDay, Key, Size, Time};

use crate::{
    widget_node_from, BuildContext, Center, DecoratedBox, ExcludeSemantics, Flex, Flexible,
    GestureDetector, Handler, ListView, Padding, Semantics, SizedBox, Text, ThemeData, Widget,
    WidgetKind, WidgetNode,
};

/// How many minutes apart the choices are.
///
/// Five, not one. Sixty rows of minutes is a scroll nobody finishes, and the
/// overwhelming majority of times anyone picks by hand land on a multiple of
/// five. A caller who needs 13:07 wants a text field, not a picker — and can
/// still set one through [`TimePicker::step`].
const DEFAULT_STEP: u8 = 5;

/// A time, chosen from columns.
///
/// ```
/// # use std::rc::Rc;
/// # use vieww_widget::prelude::*;
/// # use vieww_widget::TimePicker;
/// # use vieww_foundation::Time;
/// # fn demo(chosen: Time) {
/// let picker = TimePicker::new(chosen)
///     .twelve_hour(true)
///     .on_changed(Rc::new(|time| println!("chose {time}")));
/// # }
/// ```
pub struct TimePicker {
    value: Time,
    twelve_hour: bool,
    step: u8,
    on_changed: Option<Handler<Time>>,
    key: Option<Key>,
}

impl TimePicker {
    #[must_use]
    pub const fn new(value: Time) -> Self {
        Self {
            value,
            twelve_hour: false,
            step: DEFAULT_STEP,
            on_changed: None,
            key: None,
        }
    }

    /// Show a 12-hour clock with an AM/PM column.
    ///
    /// **Not derived from the locale**, deliberately. `Locale` here does not
    /// carry a clock preference, and guessing one from the language would be
    /// wrong for exactly the users least able to work around it — plenty of
    /// en-GB speakers want 24-hour, and the convention varies inside single
    /// countries. The application knows; it says.
    #[must_use]
    pub const fn twelve_hour(mut self, twelve_hour: bool) -> Self {
        self.twelve_hour = twelve_hour;
        self
    }

    /// Minutes between choices. Clamped to at least 1, and at most 60.
    ///
    /// A step of zero would divide by zero building the column, and a step above
    /// an hour offers nothing — both are clamped rather than rejected, because a
    /// picker that refused to build is worse than one that offers every minute.
    #[must_use]
    pub const fn step(mut self, minutes: u8) -> Self {
        self.step = if minutes == 0 {
            1
        } else if minutes > 60 {
            60
        } else {
            minutes
        };
        self
    }

    /// Called with the time the user asked for.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<Time>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The hours offered, in display order.
    ///
    /// On a 12-hour clock this is 12, 1, 2 … 11 — **12 first**, because that is
    /// where it sits on a clock face and where every other picker puts it, not
    /// 1..=12 which would read as an off-by-one to anyone glancing.
    fn hours(&self) -> Vec<u8> {
        if self.twelve_hour {
            std::iter::once(12).chain(1..=11).collect()
        } else {
            (0..=23).collect()
        }
    }

    /// The minutes offered, in display order.
    fn minutes(&self) -> Vec<u8> {
        (0..60).step_by(self.step as usize).collect()
    }

    /// What the value becomes if `hour` is chosen from the hour column.
    ///
    /// The whole of the 12-hour bookkeeping, in one place and testable without a
    /// tree: a displayed hour plus the current half-day is a 24-hour hour, and
    /// getting it wrong is a twelve-hour error nobody notices until they miss
    /// something.
    fn with_display_hour(&self, hour: u8) -> Option<Time> {
        if !self.twelve_hour {
            return Time::new(hour, self.value.minute());
        }
        let (_, half) = self.value.hour12();
        Self::from_half(hour, half, self.value.minute())
    }

    /// What the value becomes if `half` is chosen from the AM/PM column.
    fn with_half(&self, half: HalfDay) -> Option<Time> {
        let (hour, _) = self.value.hour12();
        Self::from_half(hour, half, self.value.minute())
    }

    /// A 12-hour reading back into a real time.
    ///
    /// 12 AM is 00 and 12 PM is 12, which is the case that makes this worth a
    /// function: the naive `hour + 12` gets both of them wrong.
    fn from_half(hour12: u8, half: HalfDay, minute: u8) -> Option<Time> {
        let base = hour12 % 12;
        let hour = match half {
            HalfDay::Am => base,
            HalfDay::Pm => base + 12,
        };
        Time::new(hour, minute)
    }
}

impl Widget for TimePicker {
    fn debug_name(&self) -> &'static str {
        "TimePicker"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("value", self.value.to_string()),
            ("twelve_hour", self.twelve_hour.to_string()),
            ("step", self.step.to_string()),
        ]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let row = theme.metrics.touch_target;

        let (display_hour, half) = if self.twelve_hour {
            let (hour, half) = self.value.hour12();
            (hour, Some(half))
        } else {
            (self.value.hour(), None)
        };

        let mut columns = vec![
            self.column(&theme, row, self.hours(), display_hour, Column::Hour),
            self.separator(&theme, row),
            self.column(
                &theme,
                row,
                self.minutes(),
                self.value.minute(),
                Column::Minute,
            ),
        ];

        if let Some(half) = half {
            columns.push(self.half_column(&theme, row, half));
        }

        Semantics::container("Time")
            .child(Flex::row().children(columns))
            .into()
    }
}

impl TimePicker {
    /// One scrolling column of numbers.
    fn column(
        &self,
        theme: &ThemeData,
        row: f32,
        values: Vec<u8>,
        current: u8,
        which: Column,
    ) -> WidgetNode {
        let handler = self.on_changed.clone();
        let picker_twelve = self.twelve_hour;
        let value = self.value;
        let step = self.step;
        let theme = *theme;

        // A `ListView` rather than a `Flex`, so twenty-four hours cost a
        // screenful of widgets instead of twenty-four — the same reason every
        // other long list in this crate is virtualised.
        let list = ListView::new(
            values.len(),
            row,
            Rc::new(move |index| {
                let Some(&number) = values.get(index) else {
                    return SizedBox::shrink().into();
                };
                let is_current = number == current;

                let ink = if is_current {
                    theme.colors.on_primary
                } else {
                    theme.colors.on_surface
                };
                let label = Center::new().child(
                    Text::new(format!("{number:02}"))
                        .style(theme.text.body)
                        .color(ink),
                );

                let marked: WidgetNode = if is_current {
                    DecoratedBox::rounded(theme.colors.primary, theme.metrics.corner)
                        .child(label)
                        .into()
                } else {
                    label.into()
                };

                let target = SizedBox::from_size(Size::new(f32::INFINITY, row))
                    .child(Padding::new(EdgeInsets::symmetric(4.0, 2.0)).child(marked));

                // Rebuilt here rather than captured, because the picker itself is
                // gone by the time a row is built — `ListView` builds lazily.
                let rebuilt = TimePicker {
                    value,
                    twelve_hour: picker_twelve,
                    step,
                    on_changed: None,
                    key: None,
                };
                let next = match which {
                    Column::Hour => rebuilt.with_display_hour(number),
                    Column::Minute => Time::new(value.hour(), number),
                };

                let node: WidgetNode = match (&handler, next) {
                    (Some(handler), Some(next)) => {
                        let handler = Rc::clone(handler);
                        GestureDetector::new()
                            .on_tap(move |_| handler(next))
                            .child(target)
                            .into()
                    }
                    _ => target.into(),
                };

                Semantics::button(format!("{} {number}", which.label()))
                    .toggled(is_current)
                    .enabled(handler.is_some() && next.is_some())
                    .child(node)
                    .into()
            }),
        );

        Flexible::expanded(1).child(list).into()
    }

    /// The colon between the columns.
    fn separator(&self, theme: &ThemeData, row: f32) -> WidgetNode {
        // Excluded from semantics: a screen reader reading ":" between two
        // numbers it has already announced as an hour and a minute is noise.
        ExcludeSemantics::new(true)
            .child(
                SizedBox::from_size(Size::new(row * 0.4, row)).child(
                    Center::new().child(
                        Text::new(":")
                            .style(theme.text.title)
                            .color(theme.colors.on_surface_variant),
                    ),
                ),
            )
            .into()
    }

    /// AM over PM, on a 12-hour clock.
    fn half_column(&self, theme: &ThemeData, row: f32, current: HalfDay) -> WidgetNode {
        let cells = [HalfDay::Am, HalfDay::Pm]
            .into_iter()
            .map(|half| self.half_cell(theme, row, half, half == current))
            .collect::<Vec<WidgetNode>>();

        Flex::column().children(cells).into()
    }

    fn half_cell(
        &self,
        theme: &ThemeData,
        row: f32,
        half: HalfDay,
        is_current: bool,
    ) -> WidgetNode {
        let text = match half {
            HalfDay::Am => "AM",
            HalfDay::Pm => "PM",
        };
        let ink = if is_current {
            theme.colors.on_primary
        } else {
            theme.colors.on_surface
        };

        let label = Center::new().child(Text::new(text).style(theme.text.label).color(ink));
        let marked: WidgetNode = if is_current {
            DecoratedBox::rounded(theme.colors.primary, theme.metrics.corner)
                .child(label)
                .into()
        } else {
            label.into()
        };

        let target = SizedBox::from_size(Size::new(row * 1.2, row))
            .child(Padding::new(EdgeInsets::all(2.0)).child(marked));

        let next = self.with_half(half);
        let node: WidgetNode = match (&self.on_changed, next) {
            (Some(handler), Some(next)) => {
                let handler = Rc::clone(handler);
                GestureDetector::new()
                    .on_tap(move |_| handler(next))
                    .child(target)
                    .into()
            }
            _ => target.into(),
        };

        Semantics::button(text)
            .toggled(is_current)
            .enabled(self.on_changed.is_some() && next.is_some())
            .child(node)
            .into()
    }
}

/// Which column a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Hour,
    Minute,
}

impl Column {
    const fn label(self) -> &'static str {
        match self {
            Self::Hour => "Hour",
            Self::Minute => "Minute",
        }
    }
}

impl fmt::Debug for TimePicker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TimePicker")
            .field("value", &self.value)
            .field("twelve_hour", &self.twelve_hour)
            .finish_non_exhaustive()
    }
}

widget_node_from!(TimePicker);

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u8, minute: u8) -> TimePicker {
        TimePicker::new(Time::new(hour, minute).unwrap())
    }

    #[test]
    fn a_twenty_four_hour_clock_offers_every_hour_from_zero() {
        let picker = at(9, 30);
        let hours = picker.hours();
        assert_eq!(hours.len(), 24);
        assert_eq!(hours[0], 0);
        assert_eq!(hours[23], 23);
    }

    #[test]
    fn a_twelve_hour_clock_puts_twelve_first() {
        // Where a clock face has it. 1..=12 would read as an off-by-one.
        let picker = at(9, 30).twelve_hour(true);
        assert_eq!(picker.hours(), vec![12, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    }

    #[test]
    fn minutes_come_in_steps_and_always_start_at_zero() {
        assert_eq!(
            at(9, 0).minutes(),
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
        assert_eq!(at(9, 0).step(15).minutes(), vec![0, 15, 30, 45]);
        assert_eq!(at(9, 0).step(1).minutes().len(), 60);
    }

    #[test]
    fn a_nonsense_step_is_clamped_rather_than_dividing_by_zero() {
        assert_eq!(
            at(9, 0).step(0).minutes().len(),
            60,
            "zero would divide by zero"
        );
        assert_eq!(
            at(9, 0).step(200).minutes(),
            vec![0],
            "and an hour offers one choice"
        );
    }

    #[test]
    fn the_two_twelves_are_the_cases_that_matter() {
        // 12 AM is 00 and 12 PM is 12. `hour + 12` gets both wrong, which is why
        // `from_half` exists rather than being written inline twice.
        assert_eq!(
            TimePicker::from_half(12, HalfDay::Am, 0),
            Time::new(0, 0),
            "12 AM is midnight"
        );
        assert_eq!(
            TimePicker::from_half(12, HalfDay::Pm, 0),
            Time::new(12, 0),
            "12 PM is noon"
        );
        assert_eq!(TimePicker::from_half(1, HalfDay::Am, 0), Time::new(1, 0));
        assert_eq!(TimePicker::from_half(11, HalfDay::Pm, 0), Time::new(23, 0));
    }

    #[test]
    fn choosing_an_hour_keeps_the_half_of_the_day_it_was_already_in() {
        // 14:30 is 2 PM. Choosing "9" means 9 PM, not 9 AM — a picker that
        // dropped the half-day here would be wrong by twelve hours half the
        // time, which is the sort of bug that gets noticed at an airport.
        let picker = at(14, 30).twelve_hour(true);
        assert_eq!(picker.with_display_hour(9), Time::new(21, 30));

        let morning = at(9, 30).twelve_hour(true);
        assert_eq!(morning.with_display_hour(11), Time::new(11, 30));
    }

    #[test]
    fn choosing_an_hour_on_a_twenty_four_hour_clock_is_just_that_hour() {
        let picker = at(14, 30);
        assert_eq!(picker.with_display_hour(9), Time::new(9, 30));
        assert_eq!(picker.with_display_hour(23), Time::new(23, 30));
    }

    #[test]
    fn switching_half_keeps_the_hour_on_the_face_and_the_minutes() {
        // 9:30 AM to PM is 21:30, not 9:30 with a flag set elsewhere.
        let picker = at(9, 30).twelve_hour(true);
        assert_eq!(picker.with_half(HalfDay::Pm), Time::new(21, 30));

        let afternoon = at(21, 30).twelve_hour(true);
        assert_eq!(afternoon.with_half(HalfDay::Am), Time::new(9, 30));
    }

    #[test]
    fn switching_half_at_midnight_and_noon_round_trips() {
        // The pair that the naive arithmetic sends to 12:00 and 24:00.
        let midnight = at(0, 0).twelve_hour(true);
        assert_eq!(midnight.with_half(HalfDay::Pm), Time::new(12, 0));

        let noon = at(12, 0).twelve_hour(true);
        assert_eq!(noon.with_half(HalfDay::Am), Time::new(0, 0));
    }

    #[test]
    fn every_offered_hour_produces_a_real_time() {
        // The guard on the whole hour column: nothing it lists can fail to
        // become a time, on either clock.
        for twelve in [false, true] {
            let picker = at(13, 45).twelve_hour(twelve);
            for hour in picker.hours() {
                assert!(
                    picker.with_display_hour(hour).is_some(),
                    "hour {hour} on a {} clock",
                    if twelve { "12-hour" } else { "24-hour" }
                );
            }
        }
    }
}
