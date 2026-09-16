//! The words a calendar needs, which no locale in this framework carries.
//!
//! # Why this is data an application supplies
//!
//! [`Locale`](vieww_foundation::Locale) knows a language, a region, a text
//! direction and a plural rule. It does **not** know that the eighth month is
//! called August, because nothing in this framework ships CLDR and taking an
//! ICU dependency to draw a month grid would be the largest dependency in the
//! tree by an order of magnitude.
//!
//! So the names are a value, defaulting to English, published by
//! [`Localizations`](crate::Localizations) and overridable by the application —
//! which is roughly what localization delegates do in any toolkit, for the
//! same reason. An application already translating its own strings has this
//! data; one that is not, is English anyway.
//!
//! **Bundling names for "the common languages" was considered and rejected.**
//! A partial list is worse than none: a missing language falls back to English
//! silently, and the gap is invisible until somebody who speaks it opens the
//! picker.
//!
//! ```
//! # use std::rc::Rc;
//! # use vieww_widget::prelude::*;
//! # use vieww_widget::{CalendarNames, Localizations};
//! # use vieww_foundation::{Locale, Weekday};
//! let spanish = CalendarNames {
//!     months: [
//!         "enero", "febrero", "marzo", "abril", "mayo", "junio",
//!         "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre",
//!     ],
//!     weekdays_short: ["L", "M", "X", "J", "V", "S", "D"],
//!     first_day: Weekday::Monday,
//! };
//! let app = Localizations::new(Locale::parse("es").unwrap())
//!     .calendar(spanish)
//!     .child(Text::new("…"));
//! ```

use vieww_foundation::Weekday;

/// Month names, weekday abbreviations, and which day a week starts on.
///
/// `&'static str` rather than `String`: these are compile-time constants in
/// every application that has them, and an owned type here would allocate
/// thirteen strings per rebuild of a calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarNames {
    /// January first, indexed by month - 1.
    pub months: [&'static str; 12],
    /// Column headings, **in the order they are displayed** — that is, starting
    /// at [`first_day`](Self::first_day) rather than always at Monday.
    ///
    /// Display order rather than a fixed order because the two together are how
    /// a caller expresses "my week starts on Sunday and here are my headings",
    /// and splitting them lets the pair disagree.
    pub weekdays_short: [&'static str; 7],
    /// Which day a week begins on. Sunday across much of the Americas, Monday
    /// across most of Europe, and Saturday in much of the Middle East.
    pub first_day: Weekday,
}

impl CalendarNames {
    /// English names, Sunday-first.
    ///
    /// The default, and the fallback when an application publishes none — see
    /// [`Localizations`](crate::Localizations).
    #[must_use]
    pub const fn english() -> Self {
        Self {
            months: [
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ],
            weekdays_short: ["S", "M", "T", "W", "T", "F", "S"],
            first_day: Weekday::Sunday,
        }
    }

    /// The name of a month, or `""` if that is not one.
    ///
    /// Empty rather than panicking: this is reached from a widget `build`, and
    /// a calendar missing a heading is a far better failure than an application
    /// that dies drawing one.
    #[must_use]
    pub fn month(&self, month: u8) -> &'static str {
        if (1..=12).contains(&month) {
            self.months[month as usize - 1]
        } else {
            ""
        }
    }

    /// The heading for the `column`-th column, left to right.
    ///
    /// Wraps rather than panicking, for the same reason.
    #[must_use]
    pub fn weekday_column(&self, column: u8) -> &'static str {
        self.weekdays_short[(column % 7) as usize]
    }
}

impl Default for CalendarNames {
    fn default() -> Self {
        Self::english()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_are_one_based_and_out_of_range_is_empty() {
        let names = CalendarNames::english();
        assert_eq!(names.month(1), "January");
        assert_eq!(names.month(12), "December");
        assert_eq!(names.month(0), "", "a build must not panic on a bad month");
        assert_eq!(names.month(13), "");
    }

    #[test]
    fn weekday_headings_are_read_in_display_order() {
        // English is Sunday-first, so column 0 is Sunday's heading. The array is
        // display order, which is the whole point of pairing it with `first_day`.
        let names = CalendarNames::english();
        assert_eq!(names.first_day, Weekday::Sunday);
        assert_eq!(names.weekday_column(0), "S");
        assert_eq!(names.weekday_column(1), "M");
        assert_eq!(names.weekday_column(7), "S", "column indices wrap");
    }
}
