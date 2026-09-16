//! Dates and times, in the order a locale writes them.

use crate::date::{Date, HalfDay, Time, Weekday};
use crate::Locale;

/// The names of the months and the days.
///
/// # Why these are supplied rather than built in
///
/// For [`Locale`](crate::Locale)'s reason, applied one level down. Carrying the
/// month names of every language would put a translation table in a UI
/// framework — hundreds of kilobytes that most applications do not need, in a
/// crate that has said it does not ship a message catalogue. The *structure* is
/// the part vieww can know without data: which order the fields go in, which
/// separator sits between them, whether the clock has twelve hours or
/// twenty-four. That is [`DateTimeFormat`], and it is genuinely locale-specific
/// and genuinely not something an application should have to know.
///
/// The names are an application's, or a data crate's, or a server's. English
/// is built in because a framework whose examples cannot print a date is
/// useless, and it is [`ENGLISH`](Self::ENGLISH) rather than a default so that
/// nobody ships French dates with English month names by accident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarNames {
    /// January first, twelve entries.
    pub months: [&'static str; 12],
    /// The short forms, twelve entries.
    pub months_short: [&'static str; 12],
    /// Monday first, seven entries — matching [`Weekday`], which is ISO.
    pub weekdays: [&'static str; 7],
    /// The short forms, Monday first.
    pub weekdays_short: [&'static str; 7],
    /// What comes after a 12-hour time.
    pub am: &'static str,
    pub pm: &'static str,
}

impl CalendarNames {
    /// English names.
    pub const ENGLISH: Self = Self {
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
        months_short: [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ],
        weekdays: [
            "Monday",
            "Tuesday",
            "Wednesday",
            "Thursday",
            "Friday",
            "Saturday",
            "Sunday",
        ],
        weekdays_short: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
        am: "AM",
        pm: "PM",
    };

    fn month(&self, month: u8, long: bool) -> &'static str {
        let index = usize::from(month.clamp(1, 12)) - 1;
        if long {
            self.months[index]
        } else {
            self.months_short[index]
        }
    }

    fn weekday(&self, day: Weekday, long: bool) -> &'static str {
        let index = day as usize;
        if long {
            self.weekdays[index]
        } else {
            self.weekdays_short[index]
        }
    }
}

/// Which order a locale writes a numeric date in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateOrder {
    /// `31/12/2026` — most of the world.
    DayMonthYear,
    /// `12/31/2026` — the United States, and the Philippines.
    MonthDayYear,
    /// `2026-12-31` — ISO, and the everyday order in East Asia and Hungary.
    YearMonthDay,
}

/// How much of a date to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateLength {
    /// `31/12/2026` — all numbers.
    Short,
    /// `31 Dec 2026`.
    #[default]
    Medium,
    /// `31 December 2026`.
    Long,
    /// `Thursday, 31 December 2026`.
    Full,
}

/// How much of a time to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeLength {
    /// `14:30`, or `2:30 PM`.
    #[default]
    Short,
}

/// Writes a [`Date`] or a [`Time`] the way a locale writes it.
///
/// ```
/// use vieww_foundation::intl::{DateLength, DateTimeFormat};
/// use vieww_foundation::{Date, Locale};
///
/// let day = Date::new(2026, 12, 31).expect("a real day");
///
/// let british = DateTimeFormat::new(Locale::parse("en-GB").expect("a real tag"));
/// assert_eq!(british.date(day, DateLength::Short), "31/12/2026");
///
/// let american = DateTimeFormat::new(Locale::parse("en-US").expect("a real tag"));
/// assert_eq!(american.date(day, DateLength::Short), "12/31/2026");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeFormat {
    locale: Locale,
    names: CalendarNames,
    order: DateOrder,
    numeric_separator: char,
    twelve_hour: bool,
}

impl DateTimeFormat {
    /// A formatter for `locale`, with English names.
    ///
    /// Use [`with_names`](Self::with_names) for anything else — see
    /// [`CalendarNames`] for why those are not built in.
    #[must_use]
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            names: CalendarNames::ENGLISH,
            order: Self::order_for(locale),
            numeric_separator: Self::separator_for(locale),
            twelve_hour: Self::twelve_hour_for(locale),
        }
    }

    /// Supply the month and weekday names.
    #[must_use]
    pub fn with_names(mut self, names: CalendarNames) -> Self {
        self.names = names;
        self
    }

    /// Override the field order.
    #[must_use]
    pub const fn with_order(mut self, order: DateOrder) -> Self {
        self.order = order;
        self
    }

    /// Override the clock. `true` for `2:30 PM`, `false` for `14:30`.
    #[must_use]
    pub const fn with_twelve_hour(mut self, twelve_hour: bool) -> Self {
        self.twelve_hour = twelve_hour;
        self
    }

    /// Which order this locale writes a numeric date in.
    ///
    /// # Coverage
    ///
    /// | order | where |
    /// |---|---|
    /// | [`MonthDayYear`](DateOrder::MonthDayYear) | `en-US`, `en-PH` |
    /// | [`YearMonthDay`](DateOrder::YearMonthDay) | ja, zh, ko, hu, lt |
    /// | [`DayMonthYear`](DateOrder::DayMonthYear) *(also the fallback)* | everywhere else |
    ///
    /// Note that this is one of the few places where a *region* changes the
    /// answer within one language and the difference is dangerous rather than
    /// cosmetic: `03/04/2026` is two different days in London and New York,
    /// which is why the medium length — with the month named — is the default
    /// and the short one has to be asked for.
    #[must_use]
    pub fn order_for(locale: Locale) -> DateOrder {
        match (locale.language(), locale.region()) {
            ("en", Some("US" | "PH")) => DateOrder::MonthDayYear,
            ("ja" | "zh" | "ko" | "hu" | "lt", _) => DateOrder::YearMonthDay,
            _ => DateOrder::DayMonthYear,
        }
    }

    /// What sits between the numbers of a short date.
    #[must_use]
    pub fn separator_for(locale: Locale) -> char {
        match locale.language() {
            "de" | "ru" | "cs" | "pl" | "fi" | "nb" | "nn" | "no" | "tr" | "uk" | "bg" | "ro" => {
                '.'
            }
            "ja" | "zh" | "ko" | "hu" | "lt" | "sv" | "da" | "lv" => '-',
            _ => '/',
        }
    }

    /// Whether this locale reads a twelve-hour clock.
    ///
    /// English-speaking countries, and very little else — continental Europe,
    /// East Asia and most of the rest use twenty-four. The fallback is
    /// twenty-four hours, which is the majority answer and the unambiguous one.
    #[must_use]
    pub fn twelve_hour_for(locale: Locale) -> bool {
        matches!(locale.language(), "en") || matches!(locale.language(), "hi" | "bn" | "ta" | "te")
    }

    /// The locale this formatter was built for.
    #[must_use]
    pub const fn locale(&self) -> Locale {
        self.locale
    }

    /// Write a date.
    #[must_use]
    pub fn date(&self, date: Date, length: DateLength) -> String {
        match length {
            DateLength::Short => self.numeric_date(date),
            DateLength::Medium => self.named_date(date, false, false),
            DateLength::Long => self.named_date(date, true, false),
            DateLength::Full => self.named_date(date, true, true),
        }
    }

    /// Write a time.
    #[must_use]
    pub fn time(&self, time: Time, _length: TimeLength) -> String {
        if self.twelve_hour {
            let (hour, half) = time.hour12();
            let marker = match half {
                HalfDay::Am => self.names.am,
                HalfDay::Pm => self.names.pm,
            };
            format!("{hour}:{:02}\u{A0}{marker}", time.minute())
        } else {
            format!("{:02}:{:02}", time.hour(), time.minute())
        }
    }

    /// Write a date and a time together.
    ///
    /// The date first and the time after it, separated by a comma — which is
    /// what every locale in the coverage table above does, and the one piece of
    /// this that genuinely did not need a table.
    #[must_use]
    pub fn date_time(&self, date: Date, time: Time, length: DateLength) -> String {
        format!(
            "{}, {}",
            self.date(date, length),
            self.time(time, TimeLength::Short)
        )
    }

    /// Just the month and the year — a calendar header.
    #[must_use]
    pub fn month_year(&self, date: Date, long: bool) -> String {
        let month = self.names.month(date.month(), long);
        match self.order {
            DateOrder::YearMonthDay => format!("{} {month}", date.year()),
            DateOrder::DayMonthYear | DateOrder::MonthDayYear => {
                format!("{month} {}", date.year())
            }
        }
    }

    /// The weekday names in the order a calendar grid should show them, given
    /// which day the week starts on.
    #[must_use]
    pub fn weekday_headings(&self, first: Weekday, long: bool) -> Vec<&'static str> {
        (0..7)
            .map(|offset| {
                let index = (first as usize + offset) % 7;
                let day = [
                    Weekday::Monday,
                    Weekday::Tuesday,
                    Weekday::Wednesday,
                    Weekday::Thursday,
                    Weekday::Friday,
                    Weekday::Saturday,
                    Weekday::Sunday,
                ][index];
                self.names.weekday(day, long)
            })
            .collect()
    }

    /// Which day the week starts on, for a calendar grid.
    ///
    /// Monday almost everywhere, Sunday in the United States, Canada, Japan and
    /// much of Latin America, Saturday in much of the Middle East. This is the
    /// question [`Weekday`]'s own documentation declines to answer, answered
    /// here where the locale is in hand.
    #[must_use]
    pub fn first_weekday(&self) -> Weekday {
        match (self.locale.language(), self.locale.region()) {
            (_, Some("US" | "CA" | "JP" | "BR" | "MX" | "IL" | "KR" | "TW" | "PH" | "ZA")) => {
                Weekday::Sunday
            }
            ("ar" | "he" | "fa", _) => Weekday::Saturday,
            ("ja" | "ko", None) => Weekday::Sunday,
            _ => Weekday::Monday,
        }
    }

    fn numeric_date(&self, date: Date) -> String {
        let sep = self.numeric_separator;
        let (day, month, year) = (date.day(), date.month(), date.year());
        match self.order {
            DateOrder::DayMonthYear => format!("{day:02}{sep}{month:02}{sep}{year}"),
            DateOrder::MonthDayYear => format!("{month:02}{sep}{day:02}{sep}{year}"),
            DateOrder::YearMonthDay => format!("{year}{sep}{month:02}{sep}{day:02}"),
        }
    }

    fn named_date(&self, date: Date, long_month: bool, with_weekday: bool) -> String {
        let month = self.names.month(date.month(), long_month);
        let body = match self.order {
            DateOrder::DayMonthYear => format!("{} {month} {}", date.day(), date.year()),
            DateOrder::MonthDayYear => format!("{month} {}, {}", date.day(), date.year()),
            DateOrder::YearMonthDay => format!("{} {month} {}", date.year(), date.day()),
        };
        if with_weekday {
            format!("{}, {body}", self.names.weekday(date.weekday(), true))
        } else {
            body
        }
    }
}
