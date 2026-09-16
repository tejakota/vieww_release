//! Formatting numbers, money, dates, lists and messages for a locale.
//!
//! [`Locale`](crate::Locale) says which language an interface is in and which
//! plural form a count takes. This module is what an application does with that
//! — the formatting layer that every application ends up hand-rolling, gets
//! subtly wrong, and gets wrong in a way that only speakers of the affected
//! language notice.
//!
//! # The line this module draws
//!
//! **vieww supplies structure. The application supplies words.**
//!
//! That line is the same one [`Locale`](crate::Locale) draws about message
//! catalogues, applied consistently:
//!
//! | vieww knows | the application supplies |
//! |---|---|
//! | that French groups with a narrow space and a comma | — |
//! | that Hindi groups `12,34,567` and not `1,234,567` | — |
//! | that `en-US` writes month-day-year and `en-GB` day-month-year | the month names |
//! | that a currency symbol trails the number in German | the symbol and its minor units |
//! | that Russian has four plural forms and which one a count takes | the four strings |
//! | that American English takes a serial comma | the word "and" |
//!
//! Everything on the left is a *fact about the language* that an application
//! cannot reasonably be expected to know and that does not vary between
//! applications. Everything on the right is content, is different for every
//! product, and would put a translation database inside a UI framework.
//!
//! A framework that shipped the right-hand column would be several megabytes of
//! CLDR data that most applications do not need. A framework that shipped
//! neither — which is where this one was — leaves every application writing
//! `format!("{:.2}", price)` and shipping `1234.50 €` to France.
//!
//! # Coverage, said out loud
//!
//! This is **not** a CLDR implementation and does not claim to be. It is a
//! curated table covering the same language families
//! [`PluralCategory`](crate::PluralCategory) covers, with an explicit fallback
//! and an explicit statement of what the fallback is a guess about. Every type
//! here documents its own coverage table and names the languages it does not
//! handle. An application that needs the full CLDR should reach for `icu`, and
//! it can — these are plain values with public fields, so a caller can build a
//! [`NumberSymbols`] from anywhere.
//!
//! What this module refuses to do is guess silently. Every fallback is written
//! down at the function that falls back.
//!
//! # No clock, no time zone
//!
//! [`DateTimeFormat`] formats a [`Date`](crate::Date) and a
//! [`Time`](crate::Time) that the caller already has, for
//! [`Date`](crate::Date)'s stated reason: nothing in this crate reads the
//! system clock, because a widget that asks what day it is cannot be tested.
//! Time zones are the same answer one level up — a zone database is a hundred
//! kilobytes that changes several times a year, and an application that needs
//! one wants `chrono-tz` or `jiff`.

mod datetime;
mod list;
mod message;
mod number;

pub use datetime::{CalendarNames, DateLength, DateOrder, DateTimeFormat, TimeLength};
pub use list::{format_list, serial_comma, ListStyle, ListWords};
pub use message::{Args, MessageError, MessageFormat, Value};
pub use number::{
    format_bytes, CompactStyle, Currency, NumberFormat, NumberStyle, NumberSymbols, SymbolPosition,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::{Date, Time, Weekday};
    use crate::Locale;

    fn locale(tag: &str) -> Locale {
        Locale::parse(tag).expect("a real tag")
    }

    // ------------------------------------------------------------- numbers

    #[test]
    fn each_grouping_family_writes_the_same_number_differently() {
        let value = 1_234_567.5;
        assert_eq!(
            NumberFormat::decimal(Locale::ENGLISH).format(value),
            "1,234,567.5"
        );
        assert_eq!(
            NumberFormat::decimal(locale("de-DE")).format(value),
            "1.234.567,5"
        );
        assert_eq!(
            NumberFormat::decimal(locale("fr-FR")).format(value),
            "1\u{202F}234\u{202F}567,5"
        );
        assert_eq!(
            NumberFormat::decimal(locale("de-CH")).format(value),
            "1\u{2019}234\u{2019}567.5"
        );
    }

    #[test]
    fn the_indian_system_groups_in_twos_after_the_first_three() {
        // The case a hard-coded "every three digits" gets visibly wrong for a
        // fifth of the world.
        assert_eq!(
            NumberFormat::integer(locale("hi-IN")).format(1_234_567.0),
            "12,34,567"
        );
        assert_eq!(
            NumberFormat::integer(locale("hi-IN")).format(12_345.0),
            "12,345"
        );
    }

    #[test]
    fn fraction_digits_pad_to_the_minimum_and_round_at_the_maximum() {
        let two = NumberFormat::decimal(Locale::ENGLISH).with_fraction_digits(2, 2);
        assert_eq!(two.format(1.0), "1.00");
        assert_eq!(two.format(1.005), "1.00", "half-to-even, as f64 rounds");
        assert_eq!(two.format(1.006), "1.01");
    }

    #[test]
    fn trailing_zeroes_are_trimmed_down_to_the_minimum_and_no_further() {
        let format = NumberFormat::decimal(Locale::ENGLISH).with_fraction_digits(1, 4);
        assert_eq!(format.format(2.5), "2.5");
        assert_eq!(format.format(2.0), "2.0", "the minimum is honoured");
        assert_eq!(format.format(2.500_04), "2.5", "and the maximum rounds");
    }

    #[test]
    fn a_percentage_is_multiplied_by_a_hundred() {
        // The mistake everybody makes once, so it is asserted rather than
        // documented alone.
        //
        // 0.125 becomes 12.5, and 12.5 to no decimals is **12**, not 13:
        // rounding here is half-to-even, which is what Rust's `{:.0}` does and
        // what ICU does by default. Asserted because "round half up" is the
        // rule most people would predict and the difference shows up on
        // exactly the values a test picks.
        assert_eq!(NumberFormat::percent(Locale::ENGLISH).format(0.125), "12%");
        assert_eq!(NumberFormat::percent(Locale::ENGLISH).format(0.135), "14%");
        assert_eq!(
            NumberFormat::percent(Locale::ENGLISH)
                .with_fraction_digits(0, 1)
                .format(0.125),
            "12.5%"
        );
    }

    #[test]
    fn money_lands_on_the_side_of_the_number_the_locale_puts_it() {
        let dollars = NumberFormat::currency(Locale::ENGLISH, Currency::new("$"));
        assert_eq!(dollars.format(1234.5), "$1,234.50");

        let euros = NumberFormat::currency(locale("de-DE"), Currency::new("€"));
        assert_eq!(euros.format(1234.5), "1.234,50\u{A0}€");
    }

    #[test]
    fn a_negative_price_keeps_its_minus_outside_the_symbol() {
        // `-$5.00`, never `$-5.00`: a minus buried between the symbol and the
        // digits is genuinely easy to miss on a statement.
        let dollars = NumberFormat::currency(Locale::ENGLISH, Currency::new("$"));
        assert_eq!(dollars.format(-5.0), "-$5.00");
    }

    #[test]
    fn an_iso_code_gets_a_space_and_a_symbol_does_not() {
        // `INR12,34,567.50` reads as one token and a reader has to stop and
        // take it apart; `$5.00` does not, and putting a space in it would be
        // wrong. So the currency carries the answer rather than the caller
        // having to know to ask.
        let symbol = NumberFormat::currency(Locale::ENGLISH, Currency::new("$"));
        assert_eq!(symbol.format(5.0), "$5.00");

        let code = NumberFormat::currency(locale("hi-IN"), Currency::code("INR"));
        assert_eq!(code.format(1_234_567.5), "INR\u{A0}12,34,567.50");

        // The locale still decides the side; the currency only decides the
        // space. German trails, and a code trailing is spaced either way.
        let trailing = NumberFormat::currency(locale("de-DE"), Currency::code("EUR"));
        assert_eq!(trailing.format(5.0), "5,00\u{A0}EUR");
    }

    #[test]
    fn a_currency_with_no_minor_unit_shows_no_decimals() {
        // ¥1,200 and ¥12.00 are the same money written two ways, and only one
        // of them is right.
        let yen = NumberFormat::currency(locale("ja-JP"), Currency::whole("¥"));
        assert_eq!(yen.format(1200.0), "¥1,200");
    }

    #[test]
    fn compact_notation_changes_scale_and_not_only_letters() {
        // Japanese counts in ten-thousands. "12K" there is not untranslated, it
        // is the wrong magnitude.
        assert_eq!(
            NumberFormat::compact(Locale::ENGLISH).format(12_000.0),
            "12K"
        );
        assert_eq!(
            NumberFormat::compact(locale("ja-JP")).format(12_000.0),
            "1.2万"
        );
        assert_eq!(
            NumberFormat::compact(locale("hi-IN")).format(12_000_000.0),
            "1.2Cr"
        );
    }

    #[test]
    fn a_large_integer_survives_that_an_f64_would_not() {
        // 2^53 + 1 is the smallest integer an f64 cannot hold.
        let value = 9_007_199_254_740_993_i64;
        let format = NumberFormat::integer(Locale::ENGLISH);
        assert_eq!(format.format_int(value), "9,007,199,254,740,993");
    }

    #[test]
    fn grouping_can_be_turned_off_for_a_number_that_is_an_identifier() {
        let year = NumberFormat::integer(Locale::ENGLISH).without_grouping();
        assert_eq!(year.format(2026.0), "2026");
    }

    #[test]
    fn an_infinity_says_so_rather_than_printing_a_pile_of_digits() {
        let format = NumberFormat::decimal(Locale::ENGLISH);
        assert_eq!(format.format(f64::INFINITY), "∞");
        assert_eq!(format.format(f64::NEG_INFINITY), "-∞");
        assert_eq!(format.format(f64::NAN), "NaN");
    }

    #[test]
    fn a_file_size_is_grouped_for_its_locale() {
        assert_eq!(format_bytes(Locale::ENGLISH, 512), "512\u{A0}B");
        assert_eq!(format_bytes(Locale::ENGLISH, 1536), "1.5\u{A0}KB");
        assert_eq!(format_bytes(locale("de-DE"), 1536), "1,5\u{A0}KB");
    }

    // --------------------------------------------------------------- dates

    #[test]
    fn the_same_numeric_date_reads_differently_on_two_sides_of_an_ocean() {
        // 03/04 is two different days, which is why the medium length is the
        // default and this one has to be asked for.
        let day = Date::new(2026, 4, 3).expect("a real day");
        assert_eq!(
            DateTimeFormat::new(locale("en-GB")).date(day, DateLength::Short),
            "03/04/2026"
        );
        assert_eq!(
            DateTimeFormat::new(locale("en-US")).date(day, DateLength::Short),
            "04/03/2026"
        );
        assert_eq!(
            DateTimeFormat::new(locale("ja-JP")).date(day, DateLength::Short),
            "2026-04-03"
        );
    }

    #[test]
    fn a_named_date_puts_the_month_where_the_order_says() {
        let day = Date::new(2026, 12, 31).expect("a real day");
        let british = DateTimeFormat::new(locale("en-GB"));
        let american = DateTimeFormat::new(locale("en-US"));

        assert_eq!(british.date(day, DateLength::Medium), "31 Dec 2026");
        assert_eq!(british.date(day, DateLength::Long), "31 December 2026");
        assert_eq!(american.date(day, DateLength::Medium), "Dec 31, 2026");
    }

    #[test]
    fn a_full_date_leads_with_the_weekday() {
        let day = Date::new(2026, 12, 31).expect("a real day");
        let full = DateTimeFormat::new(locale("en-GB")).date(day, DateLength::Full);
        assert!(full.starts_with("Thursday, "), "{full}");
    }

    #[test]
    fn the_clock_has_twelve_hours_only_where_the_locale_reads_twelve() {
        let afternoon = Time::new(14, 30).expect("a real time");
        assert_eq!(
            DateTimeFormat::new(locale("en-US")).time(afternoon, TimeLength::Short),
            "2:30\u{A0}PM"
        );
        assert_eq!(
            DateTimeFormat::new(locale("de-DE")).time(afternoon, TimeLength::Short),
            "14:30"
        );
    }

    #[test]
    fn a_calendar_starts_its_week_where_the_region_starts_it() {
        assert_eq!(
            DateTimeFormat::new(locale("en-GB")).first_weekday(),
            Weekday::Monday
        );
        assert_eq!(
            DateTimeFormat::new(locale("en-US")).first_weekday(),
            Weekday::Sunday
        );
        assert_eq!(
            DateTimeFormat::new(locale("ar-EG")).first_weekday(),
            Weekday::Saturday
        );
    }

    #[test]
    fn the_weekday_headings_rotate_with_the_first_day() {
        let format = DateTimeFormat::new(locale("en-US"));
        let headings = format.weekday_headings(Weekday::Sunday, false);
        assert_eq!(headings[0], "Sun");
        assert_eq!(headings[1], "Mon");
        assert_eq!(headings.len(), 7);
    }

    // ------------------------------------------------------------ messages

    #[test]
    fn a_plural_picks_the_form_the_language_actually_uses() {
        let message =
            MessageFormat::parse("{n, plural, one {# file} other {# files}}").expect("well formed");
        assert_eq!(
            message.format(Locale::ENGLISH, &Args::new().with("n", 1)),
            "1 file"
        );
        assert_eq!(
            message.format(Locale::ENGLISH, &Args::new().with("n", 3)),
            "3 files"
        );
    }

    #[test]
    fn russian_reaches_a_branch_english_never_does() {
        // The reason plural selection belongs in the framework: an application
        // hand-rolling `if n == 1` cannot express this at all.
        let message = MessageFormat::parse(
            "{n, plural, one {# файл} few {# файла} many {# файлов} other {# файла}}",
        )
        .expect("well formed");
        let ru = locale("ru-RU");
        assert_eq!(message.format(ru, &Args::new().with("n", 1)), "1 файл");
        assert_eq!(message.format(ru, &Args::new().with("n", 3)), "3 файла");
        assert_eq!(message.format(ru, &Args::new().with("n", 11)), "11 файлов");
    }

    #[test]
    fn an_exact_match_beats_the_category() {
        let message =
            MessageFormat::parse("{n, plural, =0 {Nothing here} one {# thing} other {# things}}")
                .expect("well formed");
        assert_eq!(
            message.format(Locale::ENGLISH, &Args::new().with("n", 0)),
            "Nothing here"
        );
    }

    #[test]
    fn an_offset_shifts_the_hash_and_not_the_exact_matches() {
        // "You and 2 others" — the asymmetry is the whole point of `offset`.
        let message = MessageFormat::parse(
            "{n, plural, offset:1 =0 {Nobody} =1 {You} one {You and # other} other {You and # others}}",
        )
        .expect("well formed");
        let en = Locale::ENGLISH;
        assert_eq!(message.format(en, &Args::new().with("n", 0)), "Nobody");
        assert_eq!(message.format(en, &Args::new().with("n", 1)), "You");
        assert_eq!(
            message.format(en, &Args::new().with("n", 2)),
            "You and 1 other"
        );
        assert_eq!(
            message.format(en, &Args::new().with("n", 4)),
            "You and 3 others"
        );
    }

    #[test]
    fn a_select_chooses_on_a_string_and_falls_through_to_other() {
        let message =
            MessageFormat::parse("{who, select, her {She left} him {He left} other {They left}}")
                .expect("well formed");
        let en = Locale::ENGLISH;
        assert_eq!(
            message.format(en, &Args::new().with("who", "her")),
            "She left"
        );
        assert_eq!(
            message.format(en, &Args::new().with("who", "nobody-in-particular")),
            "They left"
        );
    }

    #[test]
    fn a_pattern_with_no_other_branch_is_rejected_at_parse_time() {
        // `other` is the only category every language has, so a pattern without
        // one cannot be rendered in some locale. Catching it here is the
        // difference between a failing test and a blank label in Arabic.
        let error = MessageFormat::parse("{n, plural, one {# file}}").expect_err("no other");
        assert!(
            matches!(error, MessageError::MissingOther { .. }),
            "{error}"
        );
    }

    #[test]
    fn an_unimplemented_argument_type_is_an_error_rather_than_a_pass_through() {
        // Otherwise the pattern renders as its own source text on screen.
        let error = MessageFormat::parse("{when, date, short}").expect_err("no date support");
        assert!(matches!(error, MessageError::UnknownType { .. }), "{error}");
    }

    #[test]
    fn a_malformed_pattern_reports_where() {
        let error = MessageFormat::parse("hello {name").expect_err("unclosed");
        assert!(
            matches!(error, MessageError::UnclosedBrace { at: 6 }),
            "{error}"
        );
    }

    #[test]
    fn a_missing_argument_renders_visibly_rather_than_silently() {
        let message = MessageFormat::parse("Hello {name}").expect("well formed");
        assert_eq!(
            message.format(Locale::ENGLISH, &Args::new()),
            "Hello {name}"
        );
    }

    #[test]
    fn quoting_lets_a_pattern_contain_a_literal_brace() {
        let message = MessageFormat::parse("use '{'name'}' here").expect("well formed");
        assert_eq!(
            message.format(Locale::ENGLISH, &Args::new()),
            "use {name} here"
        );

        let apostrophe = MessageFormat::parse("it''s fine").expect("well formed");
        assert_eq!(
            apostrophe.format(Locale::ENGLISH, &Args::new()),
            "it's fine"
        );
    }

    #[test]
    fn an_interpolated_number_is_formatted_for_the_locale() {
        let message = MessageFormat::parse("Total: {amount, number}").expect("well formed");
        assert_eq!(
            message.format(locale("de-DE"), &Args::new().with("amount", 1234.5)),
            "Total: 1.234,5"
        );
    }

    #[test]
    fn the_arguments_a_pattern_uses_can_be_listed_for_a_test_to_check() {
        let message = MessageFormat::parse(
            "{who, select, her {{n, plural, one {# thing} other {# things}}} other {nothing}}",
        )
        .expect("well formed");
        assert_eq!(message.arguments(), vec!["who".to_owned(), "n".to_owned()]);
    }

    #[test]
    fn a_nested_plural_inside_a_select_renders() {
        let message = MessageFormat::parse(
            "{who, select, her {She has {n, plural, one {# cat} other {# cats}}} other {—}}",
        )
        .expect("well formed");
        assert_eq!(
            message.format(
                Locale::ENGLISH,
                &Args::new().with("who", "her").with("n", 2)
            ),
            "She has 2 cats"
        );
    }

    // --------------------------------------------------------------- lists

    #[test]
    fn the_serial_comma_follows_the_region() {
        let items = ["apples", "pears", "plums"];
        let words = ListWords::ENGLISH;
        assert_eq!(
            format_list(locale("en-GB"), &items, ListStyle::And, words),
            "apples, pears and plums"
        );
        assert_eq!(
            format_list(locale("en-US"), &items, ListStyle::And, words),
            "apples, pears, and plums"
        );
    }

    #[test]
    fn a_two_item_list_has_no_comma_at_all() {
        let items = ["apples", "pears"];
        assert_eq!(
            format_list(locale("en-US"), &items, ListStyle::And, ListWords::ENGLISH),
            "apples and pears"
        );
    }

    #[test]
    fn the_degenerate_lists_do_not_produce_stray_punctuation() {
        let empty: [&str; 0] = [];
        assert_eq!(
            format_list(Locale::ENGLISH, &empty, ListStyle::And, ListWords::ENGLISH),
            ""
        );
        assert_eq!(
            format_list(
                Locale::ENGLISH,
                &["one"],
                ListStyle::And,
                ListWords::ENGLISH
            ),
            "one"
        );
    }
}
