//! The same values, written the way seven locales write them.
//!
//! Every cell is one `intl` call. What the picture shows is the part an
//! application cannot reasonably be expected to know: that French groups with a
//! narrow no-break space and a decimal comma, that Swiss German uses an
//! apostrophe, that Hindi groups `12,34,567` rather than `1,234,567`, that a
//! currency trails its number in German and leads in English, that the yen has
//! no minor unit, and that `03/04` is two different days in London and New
//! York.
//!
//! # Why the currencies are ISO codes and the plurals are Latin
//!
//! **The framework's embedded font is a Latin-1 subset** — 735 codepoints, no
//! `€`, no `₹`, no Cyrillic, no Devanagari, no CJK. That is a property of
//! `vieww-text`'s bundled fallback, not of anything here, and an application
//! shipping a real font sees the real symbols.
//!
//! Rather than draw a grid of tofu boxes and call it a demonstration, this
//! example uses what the font has: the ISO code where a symbol is missing,
//! which is exactly the fallback `Currency` documents, and plural messages in
//! languages written in Latin script. **Russian's four forms and Arabic's six
//! are asserted in `vieww_foundation::intl`'s own tests**, where a font cannot
//! get in the way of the thing being checked.

use vieww_foundation::intl::{
    Args, Currency, DateLength, DateTimeFormat, MessageFormat, NumberFormat,
};
use vieww_foundation::{Color, Date, Locale, Size};
use vieww_widget::prelude::*;

const INK: Color = Color::hex(0x1A_1A1A);
const MUTED: Color = Color::hex(0x6E_6E6E);
const LINE: Color = Color::hex(0xE4_E4E4);

const AMOUNT: f64 = 1_234_567.5;
const COUNT: i64 = 3;

/// The seven groupings, and the currency each is shown with:
/// `(tag, symbol-or-code, is_code, has_minor_units)`.
///
/// `Currency::whole` for the yen, because ¥1,200 and ¥12.00 are the same money
/// written two ways and only one of them is right.
const NUMBERS: [(&str, &str, bool, bool); 7] = [
    ("en-GB", "£", false, true),
    ("en-US", "$", false, true),
    ("fr-FR", "EUR", true, true),
    ("de-DE", "EUR", true, true),
    ("de-CH", "CHF", true, true),
    ("hi-IN", "INR", true, true),
    ("ja-JP", "¥", false, false),
];

/// Plural messages, in languages this font can draw.
const PLURALS: [(&str, &str); 4] = [
    ("en-GB", "{n, plural, one {# file} other {# files}}"),
    ("fr-FR", "{n, plural, one {# fichier} other {# fichiers}}"),
    ("de-DE", "{n, plural, one {# Datei} other {# Dateien}}"),
    ("es-ES", "{n, plural, one {# archivo} other {# archivos}}"),
];

fn cell(text: String, width: f32, muted: bool) -> WidgetNode {
    SizedBox::width(width)
        .child(Text::new(text).style(TextStyle::new(13.0).color(if muted { MUTED } else { INK })))
        .into()
}

/// A row of four cells, baseline-aligned so the small tag and the values sit on
/// one line rather than each on its own.
fn cells(cells: [(String, f32, bool); 4]) -> WidgetNode {
    let mut row = Flex::row().cross_axis_alignment(CrossAxisAlignment::Baseline);
    for (text, width, muted) in cells {
        row = row.push(cell(text, width, muted));
    }
    Container::new()
        .padding(EdgeInsets::symmetric(5.0, 0.0))
        .child(row)
        .into()
}

fn number_row(tag: &str, symbol: &str, is_code: bool, minor_units: bool) -> WidgetNode {
    let locale = Locale::parse(tag).expect("a real tag");
    let day = Date::new(2026, 4, 3).expect("a real day");
    // `code` rather than `new` where the symbol is an ISO code: it spaces
    // itself, because `INR12,34,567.50` reads as one token.
    let currency = match (is_code, minor_units) {
        (true, _) => Currency::code(symbol),
        (false, true) => Currency::new(symbol),
        (false, false) => Currency::whole(symbol),
    };

    cells([
        (tag.to_owned(), 66.0, true),
        (NumberFormat::decimal(locale).format(AMOUNT), 128.0, false),
        (
            NumberFormat::currency(locale, currency).format(AMOUNT),
            156.0,
            false,
        ),
        (
            DateTimeFormat::new(locale).date(day, DateLength::Short),
            100.0,
            false,
        ),
    ])
}

fn plural_row(tag: &str, pattern: &str) -> WidgetNode {
    let locale = Locale::parse(tag).expect("a real tag");
    let message = MessageFormat::parse(pattern).expect("a well-formed pattern");
    cells([
        (tag.to_owned(), 66.0, true),
        (
            message.format(locale, &Args::new().with("n", 1)),
            128.0,
            false,
        ),
        (
            message.format(locale, &Args::new().with("n", COUNT)),
            156.0,
            false,
        ),
        (
            NumberFormat::compact(locale).format(1_234_567.0),
            100.0,
            false,
        ),
    ])
}

fn rule() -> WidgetNode {
    Container::new()
        .color(LINE)
        .child(SizedBox::from_size(Size::new(450.0, 1.0)))
        .into()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    feature_harness::launch("51 — locale formatting", Size::new(540.0, 440.0), |d| {
        let mut column = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .spacing(2.0)
            .push(cells([
                ("locale".to_owned(), 66.0, true),
                ("number".to_owned(), 128.0, true),
                ("currency".to_owned(), 156.0, true),
                ("3 April 2026".to_owned(), 100.0, true),
            ]))
            .push(rule());

        for (tag, symbol, is_code, minor_units) in NUMBERS {
            column = column.push(number_row(tag, symbol, is_code, minor_units));
        }

        column = column
            .push(SizedBox::height(18.0))
            .push(cells([
                ("locale".to_owned(), 66.0, true),
                ("one".to_owned(), 128.0, true),
                ("three".to_owned(), 156.0, true),
                ("compact".to_owned(), 100.0, true),
            ]))
            .push(rule());

        for (tag, pattern) in PLURALS {
            column = column.push(plural_row(tag, pattern));
        }

        feature_harness::set_page(
            d,
            Container::new()
                .color(Color::WHITE)
                .padding(EdgeInsets::all(22.0))
                .child(column),
        );
    })
}
