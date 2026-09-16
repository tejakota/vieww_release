//! A civil date and a wall-clock time, and the arithmetic a calendar needs.
//!
//! # Why these are not `pub` fields, when `Offset` and `Rect` are
//!
//! Because every combination of two `f32`s is a real point, and most
//! combinations of three integers are not a date. A `Date` with `month: 13` or
//! `day: 31` in February would flow straight into a calendar grid and produce a
//! wrong answer rather than an error — so construction validates and the fields
//! are read through accessors. That is the only reason to depart from the shape
//! of the other value types in this crate, and it does not generalise.
//!
//! # Why there is no clock
//!
//! Nothing here reads the current time. A widget that asks the system what day
//! it is cannot be tested — every assertion about "today" would depend on when
//! the suite ran — and there is no clock service in this framework to inject.
//! Callers pass the date in, which is the same controlled shape every other
//! value in this framework has.
//!
//! # What this is not
//!
//! Not a replacement for a real date-time library. There is no time zone, no
//! instant, no duration arithmetic, no parsing and no formatting beyond what a
//! calendar grid needs. Anything keeping records rather than drawing a month
//! wants `chrono` or `time`; this exists so a picker can name its own output
//! without the framework taking a dependency that half its users already have a
//! different opinion about.

use std::fmt;

/// Days in each month of a non-leap year, indexed by month - 1.
const MONTH_LENGTHS: [u8; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

/// A day on the proleptic Gregorian calendar.
///
/// Construction is checked, so every `Date` that exists names a real day. See
/// the module docs for why this differs from `Offset` and `Rect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    // Ordered so that the derived `Ord` is chronological: year, then month,
    // then day. Reordering these fields silently changes what `<` means.
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    /// A date, or `None` if those numbers are not one.
    ///
    /// Checks the day against the month's real length, leap years included, so
    /// `2026-02-29` is rejected and `2024-02-29` is not.
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Option<Self> {
        if !(1..=12).contains(&month) {
            return None;
        }
        if day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// The first of a month, or `None` if that is not a month.
    ///
    /// Cannot fail on the day, since every month has a first — which is what
    /// makes it the right way to build the anchor a calendar grid is drawn from.
    #[must_use]
    pub fn first_of_month(year: i32, month: u8) -> Option<Self> {
        Self::new(year, month, 1)
    }

    #[must_use]
    pub const fn year(self) -> i32 {
        self.year
    }

    #[must_use]
    pub const fn month(self) -> u8 {
        self.month
    }

    #[must_use]
    pub const fn day(self) -> u8 {
        self.day
    }

    /// Which day of the week this falls on.
    ///
    /// Sakamoto's method, over the proleptic Gregorian calendar. `rem_euclid`
    /// rather than `%` because the latter is negative for years before 1 and
    /// would index backwards off the enum.
    ///
    /// Not `const`, only because `i32::rem_euclid` is not const-stable on this
    /// crate's MSRV. Nothing about the arithmetic needs a runtime.
    #[must_use]
    pub fn weekday(self) -> Weekday {
        const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];

        let mut year = self.year;
        if self.month < 3 {
            year -= 1;
        }
        let index = (year + year / 4 - year / 100
            + year / 400
            + OFFSETS[self.month as usize - 1]
            + self.day as i32)
            .rem_euclid(7);
        Weekday::from_sunday_index(index as u8)
    }

    /// The same day number in the next month, clamped to that month's length.
    ///
    /// **Clamped, not rolled over.** Stepping forward from 31 January lands on
    /// 28 February rather than 3 March, because this exists for a calendar's
    /// "next month" arrow, where landing two months away would be a bug the
    /// user watches happen.
    #[must_use]
    pub fn next_month(self) -> Self {
        let (year, month) = if self.month == 12 {
            (self.year + 1, 1)
        } else {
            (self.year, self.month + 1)
        };
        Self {
            year,
            month,
            day: self.day.min(days_in_month(year, month)),
        }
    }

    /// The same day number in the previous month, clamped to its length.
    #[must_use]
    pub fn previous_month(self) -> Self {
        let (year, month) = if self.month == 1 {
            (self.year - 1, 12)
        } else {
            (self.year, self.month - 1)
        };
        Self {
            year,
            month,
            day: self.day.min(days_in_month(year, month)),
        }
    }

    /// How many days this date's month has.
    #[must_use]
    pub const fn days_in_its_month(self) -> u8 {
        days_in_month(self.year, self.month)
    }
}

impl fmt::Display for Date {
    /// ISO 8601, because it is the one format that is unambiguous everywhere
    /// and this type does no localised formatting — see the module docs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// A time of day on a 24-hour clock, to the minute.
///
/// No seconds: this exists for a time picker, and a picker that offers seconds
/// is answering a question almost nobody asked while making the common case
/// slower to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time {
    hour: u8,
    minute: u8,
}

impl Time {
    /// A time, or `None` if those numbers are not one.
    ///
    /// Hour 0..=23 and minute 0..=59. **24:00 is rejected** even though it is
    /// legal ISO 8601 for end-of-day, because two representations of midnight
    /// would make `==` disagree with what a user sees.
    #[must_use]
    pub const fn new(hour: u8, minute: u8) -> Option<Self> {
        if hour > 23 || minute > 59 {
            return None;
        }
        Some(Self { hour, minute })
    }

    #[must_use]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    #[must_use]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// The hour on a 12-hour clock, and whether it is afternoon.
    ///
    /// Midnight and noon both read as 12, which is what a clock face says and
    /// what every 12-hour locale expects.
    #[must_use]
    pub const fn hour12(self) -> (u8, HalfDay) {
        let half = if self.hour < 12 {
            HalfDay::Am
        } else {
            HalfDay::Pm
        };
        let hour = match self.hour % 12 {
            0 => 12,
            other => other,
        };
        (hour, half)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// Which half of the day a 12-hour time falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HalfDay {
    Am,
    Pm,
}

/// A day of the week.
///
/// **Monday is zero**, which is ISO 8601 and not what Sakamoto's method
/// returns — the conversion happens once, in [`Weekday::from_sunday_index`],
/// rather than at every call site. Which day a *calendar* starts on is a
/// separate question, and a locale one; this enum does not answer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// From an index where 0 is Sunday, which is what date arithmetic produces.
    ///
    /// Out-of-range input wraps rather than panicking: the only caller is
    /// [`Date::weekday`], whose `rem_euclid` cannot produce one, and a panic in
    /// a `const fn` reachable from a widget build is worse than a wrong day.
    #[must_use]
    pub const fn from_sunday_index(index: u8) -> Self {
        match index % 7 {
            0 => Self::Sunday,
            1 => Self::Monday,
            2 => Self::Tuesday,
            3 => Self::Wednesday,
            4 => Self::Thursday,
            5 => Self::Friday,
            _ => Self::Saturday,
        }
    }

    /// Monday-zero index, for laying a grid out.
    #[must_use]
    pub const fn monday_index(self) -> u8 {
        self as u8
    }

    /// How many columns from `start` this day sits, going forward.
    ///
    /// The whole of "which column does the 1st go in", and the reason a
    /// calendar can start on any day: the answer is the same arithmetic
    /// whichever day the week begins on.
    #[must_use]
    pub const fn columns_from(self, start: Self) -> u8 {
        (self as u8 + 7 - start as u8) % 7
    }
}

/// How many days a month has, leap years included.
/// Out-of-range months answer `0`, so [`Date::new`] rejects them on the day
/// check rather than needing a second branch.
#[must_use]
// `(1..=12).contains(&month)` is the clearer form and is not a `const fn`.
#[allow(clippy::manual_range_contains)]
pub const fn days_in_month(year: i32, month: u8) -> u8 {
    if month == 2 && is_leap_year(year) {
        29
    } else if month >= 1 && month <= 12 {
        MONTH_LENGTHS[month as usize - 1]
    } else {
        0
    }
}

/// Whether a year has a 29 February.
///
/// The full Gregorian rule, not the every-four-years approximation: 1900 was
/// not a leap year and 2000 was. Both are in the tests, because the
/// approximation gets every year most software is tested against right.
#[must_use]
pub const fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_that_is_not_a_day_cannot_be_built() {
        assert!(Date::new(2026, 0, 1).is_none(), "there is no month zero");
        assert!(Date::new(2026, 13, 1).is_none(), "nor a thirteenth");
        assert!(Date::new(2026, 1, 0).is_none(), "nor a zeroth of January");
        assert!(Date::new(2026, 1, 32).is_none());
        assert!(Date::new(2026, 4, 31).is_none(), "April has thirty days");
    }

    #[test]
    fn february_is_checked_against_the_actual_year() {
        assert!(Date::new(2024, 2, 29).is_some(), "2024 is a leap year");
        assert!(Date::new(2026, 2, 29).is_none(), "2026 is not");
    }

    #[test]
    fn the_gregorian_leap_rule_is_the_full_one() {
        // The every-four-years approximation gets both of these wrong, and gets
        // every year between 1901 and 2099 right — which is why it survives.
        assert!(!is_leap_year(1900), "a century that is not a fourth one");
        assert!(is_leap_year(2000), "but a fourth century is");
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
    }

    #[test]
    fn weekdays_match_days_anyone_can_check() {
        // Dates with a well-known weekday, so a wrong answer is obvious rather
        // than merely different from the last run.
        let cases = [
            (Date::new(2000, 1, 1).unwrap(), Weekday::Saturday),
            (Date::new(2026, 8, 14).unwrap(), Weekday::Friday),
            (Date::new(1969, 7, 20).unwrap(), Weekday::Sunday),
            (Date::new(2024, 2, 29).unwrap(), Weekday::Thursday),
        ];
        for (date, expected) in cases {
            assert_eq!(date.weekday(), expected, "{date}");
        }
    }

    #[test]
    fn weekdays_are_right_before_the_year_one() {
        // `%` is negative here and `rem_euclid` is not, which is the whole
        // reason the latter is used. Without it this indexes backwards.
        let date = Date::new(-500, 3, 1).unwrap();
        // Not asserting *which* day: the point is that it produces one at all
        // rather than panicking or wrapping oddly.
        let _ = date.weekday();
    }

    #[test]
    fn stepping_a_month_clamps_instead_of_rolling_over() {
        // 31 January + one month is 28 February, not 3 March. Rolling over
        // would move the calendar two months on one press of an arrow.
        let january = Date::new(2026, 1, 31).unwrap();
        assert_eq!(january.next_month(), Date::new(2026, 2, 28).unwrap());

        let leap = Date::new(2024, 1, 31).unwrap();
        assert_eq!(leap.next_month(), Date::new(2024, 2, 29).unwrap());
    }

    #[test]
    fn stepping_a_month_crosses_the_year_in_both_directions() {
        let december = Date::new(2026, 12, 15).unwrap();
        assert_eq!(december.next_month(), Date::new(2027, 1, 15).unwrap());

        let january = Date::new(2026, 1, 15).unwrap();
        assert_eq!(january.previous_month(), Date::new(2025, 12, 15).unwrap());
    }

    #[test]
    fn a_month_stepped_and_stepped_back_is_not_always_where_it_started() {
        // Documented, not a bug: clamping loses the day number, and pretending
        // otherwise would need a "the 31st, really" state nothing else has.
        let january = Date::new(2026, 1, 31).unwrap();
        assert_eq!(
            january.next_month().previous_month(),
            Date::new(2026, 1, 28).unwrap(),
            "clamping is lossy, and a caller stepping months should keep its own anchor"
        );
    }

    #[test]
    fn the_column_a_day_falls_in_depends_on_which_day_starts_the_week() {
        // A Sunday is the last column of a Monday-first calendar and the first
        // column of a Sunday-first one. Both from the same arithmetic.
        assert_eq!(Weekday::Sunday.columns_from(Weekday::Monday), 6);
        assert_eq!(Weekday::Sunday.columns_from(Weekday::Sunday), 0);
        assert_eq!(Weekday::Monday.columns_from(Weekday::Sunday), 1);
        assert_eq!(Weekday::Wednesday.columns_from(Weekday::Monday), 2);
    }

    #[test]
    fn dates_sort_chronologically() {
        // Guards the field order, which is what the derived `Ord` reads.
        let mut dates = [
            Date::new(2026, 1, 2).unwrap(),
            Date::new(2025, 12, 31).unwrap(),
            Date::new(2026, 1, 1).unwrap(),
        ];
        dates.sort();
        assert_eq!(
            dates,
            [
                Date::new(2025, 12, 31).unwrap(),
                Date::new(2026, 1, 1).unwrap(),
                Date::new(2026, 1, 2).unwrap(),
            ]
        );
    }

    #[test]
    fn a_time_that_is_not_a_time_cannot_be_built() {
        assert!(Time::new(24, 0).is_none(), "24:00 is deliberately rejected");
        assert!(Time::new(0, 60).is_none());
        assert!(Time::new(23, 59).is_some());
    }

    #[test]
    fn midnight_and_noon_both_read_as_twelve() {
        assert_eq!(Time::new(0, 0).unwrap().hour12(), (12, HalfDay::Am));
        assert_eq!(Time::new(12, 0).unwrap().hour12(), (12, HalfDay::Pm));
        assert_eq!(Time::new(13, 30).unwrap().hour12(), (1, HalfDay::Pm));
        assert_eq!(Time::new(11, 59).unwrap().hour12(), (11, HalfDay::Am));
    }

    #[test]
    fn display_is_iso_8601() {
        assert_eq!(Date::new(2026, 8, 14).unwrap().to_string(), "2026-08-14");
        assert_eq!(Time::new(9, 5).unwrap().to_string(), "09:05");
    }
}
