//! Numbers, percentages and money, written the way a locale writes them.

use std::fmt::Write as _;

use crate::Locale;

/// Which way a locale groups and punctuates its digits.
///
/// Split out from [`NumberFormat`] because it is the part that is a *fact about
/// the language* rather than a choice the caller makes: a French application
/// does not get to decide that French uses a comma for the decimal point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberSymbols {
    /// Between the whole part and the fraction. `.` in English, `,` in French.
    pub decimal: char,
    /// Between groups of digits. `,` in English, a narrow no-break space in
    /// French, `.` in German, `'` in Swiss German.
    pub group: char,
    /// How many digits in the first group, counting from the decimal point.
    pub primary_group: u8,
    /// How many in every group after that.
    ///
    /// Two in the Indian system, where 1234567 is `12,34,567` rather than
    /// `1,234,567`. Equal to [`primary_group`](Self::primary_group) everywhere
    /// else, which is why this is a number and not an `Option` — the general
    /// case subsumes the common one and there is no branch to forget.
    pub secondary_group: u8,
    /// What a negative number is prefixed with. `-` almost everywhere; the
    /// field exists because it is not quite everywhere.
    pub minus: char,
    /// What a percentage is suffixed with.
    pub percent: char,
}

impl NumberSymbols {
    /// `1,234,567.89` — English, and the fallback.
    pub const LATIN: Self = Self {
        decimal: '.',
        group: ',',
        primary_group: 3,
        secondary_group: 3,
        minus: '-',
        percent: '%',
    };

    /// `1.234.567,89` — German, Spanish, Italian, Dutch, Portuguese, Turkish,
    /// Indonesian and most of Europe outside the Anglophone and Francophone
    /// parts.
    pub const EUROPEAN: Self = Self {
        decimal: ',',
        group: '.',
        ..Self::LATIN
    };

    /// `1 234 567,89` with a narrow no-break space — French, Russian, Polish,
    /// Czech, Swedish, Norwegian, Finnish, Ukrainian.
    ///
    /// The separator is U+202F NARROW NO-BREAK SPACE rather than an ordinary
    /// space, and the difference is visible: an ordinary space is a line-break
    /// opportunity, so a wrapped column of figures splits `1 234` across two
    /// lines.
    pub const SPACED: Self = Self {
        decimal: ',',
        group: '\u{202F}',
        ..Self::LATIN
    };

    /// `12,34,567.89` — Hindi, Bengali, Gujarati, Kannada, Marathi, Punjabi,
    /// Tamil, Telugu, Nepali, Urdu.
    ///
    /// The lakh-and-crore grouping, and the one that a hard-coded "every three
    /// digits" gets visibly wrong for a fifth of the world.
    pub const INDIAN: Self = Self {
        secondary_group: 2,
        ..Self::LATIN
    };

    /// `1'234'567.89` — Swiss German.
    pub const SWISS: Self = Self {
        group: '\u{2019}',
        ..Self::LATIN
    };

    /// The symbols `locale` writes numbers with.
    ///
    /// # Coverage, stated rather than implied
    ///
    /// | symbols | languages |
    /// |---|---|
    /// | [`EUROPEAN`](Self::EUROPEAN) | de, es, it, nl, pt, tr, id, da, ro, el, vi, ca, hr, sl, sr, bs, is, sq |
    /// | [`SPACED`](Self::SPACED) | fr, ru, uk, pl, cs, sk, sv, nb, nn, no, fi, lv, lt, et, hu, bg, be |
    /// | [`INDIAN`](Self::INDIAN) | hi, bn, gu, kn, mr, pa, ta, te, ne, ur, as, or, ml |
    /// | [`LATIN`](Self::LATIN) *(also the fallback)* | en, ja, zh, ko, th, he, ar, ms, and the rest |
    ///
    /// German in Switzerland is the one region that overrides its language, and
    /// it is here because the difference is visible on every price in the
    /// country. Everything not named falls back to `LATIN` — a *guess*, right
    /// for English and East Asia and wrong for anything European this table has
    /// missed, which is the same shape of honesty
    /// [`PluralCategory`](crate::PluralCategory) states about its own table.
    ///
    /// Digits are Western Arabic numerals throughout. Locales that also use
    /// their own digit shapes — Arabic-Indic, Devanagari, Bengali — accept
    /// these everywhere, and shaping them would need a digit table per script
    /// that this crate does not carry.
    #[must_use]
    pub fn of(locale: Locale) -> Self {
        if locale.language() == "de" && locale.region() == Some("CH") {
            return Self::SWISS;
        }
        match locale.language() {
            "de" | "es" | "it" | "nl" | "pt" | "tr" | "id" | "da" | "ro" | "el" | "vi" | "ca"
            | "hr" | "sl" | "sr" | "bs" | "is" | "sq" => Self::EUROPEAN,
            "fr" | "ru" | "uk" | "pl" | "cs" | "sk" | "sv" | "nb" | "nn" | "no" | "fi" | "lv"
            | "lt" | "et" | "hu" | "bg" | "be" => Self::SPACED,
            "hi" | "bn" | "gu" | "kn" | "mr" | "pa" | "ta" | "te" | "ne" | "ur" | "as" | "or"
            | "ml" => Self::INDIAN,
            _ => Self::LATIN,
        }
    }
}

/// Where a currency's symbol goes, and whether a space goes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolPosition {
    /// `$1.00`, `£1.00`.
    Before,
    /// `$ 1.00`.
    BeforeWithSpace,
    /// `1,00€`.
    After,
    /// `1,00 €`, `1,00 zł`.
    AfterWithSpace,
}

/// A currency, and how this locale writes it.
///
/// # Why the caller supplies the symbol
///
/// Because the framework does not carry an ISO 4217 table, and a partial one
/// would be worse than none — an application whose currency happened to be
/// missing would silently print a code where a symbol belongs. What vieww knows
/// is the *arrangement*: where the symbol sits, which separators are used, how
/// many decimals. The three-letter code and the glyph are the application's,
/// and usually its server's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Currency {
    /// `$`, `€`, `£`, `¥`, or the ISO code where there is no glyph.
    pub symbol: String,
    /// Digits after the point: 2 for most, 0 for the yen and the won, 3 for the
    /// dinar. Getting this wrong is the difference between ¥1,200 and ¥12.00.
    pub fraction_digits: u8,
    /// Force a space between the symbol and the number, against what the
    /// locale would do.
    ///
    /// `None` — the default — leaves it to the locale, which is right for a
    /// symbol: German writes `1.234,50 €` and English writes `$1,234.50`, and
    /// neither is a property of the euro or the dollar.
    ///
    /// `Some(true)` is for a symbol that is a *word*. `INR12,34,567.50` reads
    /// as one token and a reader has to stop and take it apart, in every locale
    /// including the ones that lead with their symbol. [`code`](Self::code)
    /// sets it, so the case that goes wrong is the one that is named and the
    /// common case needs no thought.
    pub spaced: Option<bool>,
}

impl Currency {
    /// A currency written with a symbol, and the usual two decimal places.
    #[must_use]
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            fraction_digits: 2,
            spaced: None,
        }
    }

    /// A currency written as its ISO 4217 code — `INR`, `SEK`, `NGN`.
    ///
    /// The right answer whenever the symbol would be a glyph the application's
    /// font does not have, which is most of them outside the handful everybody
    /// knows. Spaced, because a code butted against the digits reads as one
    /// token: `INR 12,34,567.50`, not `INR12,34,567.50`.
    #[must_use]
    pub fn code(code: impl Into<String>) -> Self {
        Self {
            symbol: code.into(),
            fraction_digits: 2,
            spaced: Some(true),
        }
    }

    /// A currency with no minor unit — the yen, the won, the Chilean peso.
    #[must_use]
    pub fn whole(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            fraction_digits: 0,
            spaced: None,
        }
    }

    /// Force the spacing, against what the locale would choose.
    #[must_use]
    pub const fn spaced(mut self, spaced: bool) -> Self {
        self.spaced = Some(spaced);
        self
    }

    /// Where this locale puts a currency symbol.
    ///
    /// English and most of Asia lead with it; continental Europe follows the
    /// number with it and a space. Not exhaustive, and the fallback is
    /// [`Before`](SymbolPosition::Before).
    #[must_use]
    pub fn position_for(locale: Locale) -> SymbolPosition {
        match locale.language() {
            "fr" | "de" | "es" | "it" | "pt" | "nl" | "fi" | "sv" | "nb" | "nn" | "no" | "da"
            | "pl" | "cs" | "sk" | "hu" | "ru" | "uk" | "tr" | "el" | "ro" | "bg" | "lv" | "lt"
            | "et" | "hr" | "sl" | "sr" | "is" | "ca" | "vi" => SymbolPosition::AfterWithSpace,
            _ => SymbolPosition::Before,
        }
    }
}

/// What kind of number is being written.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NumberStyle {
    /// `1,234.5`
    #[default]
    Decimal,
    /// `12%` — the value is multiplied by 100 first, which is the part callers
    /// get wrong in both directions when they do it themselves.
    Percent,
    /// `$1,234.50`
    Currency(Currency),
    /// `1.2K`, `3.4M` — see [`CompactStyle`].
    Compact,
}

/// How a locale abbreviates large numbers.
///
/// # Coverage, and the honest fallback
///
/// Compact notation is where a formatter is most tempted to lie. The suffixes
/// are genuinely different — Japanese counts in 万 (ten thousands) and 億
/// (hundred millions), not thousands and millions, so "1.2K" is not merely
/// untranslated there, it is the wrong *scale*. Indian numbering does the same
/// thing with lakh and crore.
///
/// So there are three arrangements here and everything else falls back to the
/// Latin one, which is stated rather than assumed. An application whose locale
/// is not covered and whose numbers are large should format them in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactStyle {
    /// Powers of a thousand: K, M, B, T.
    Latin,
    /// Powers of ten thousand: 万, 億, 兆. For Japanese and Chinese.
    MyriadCjk,
    /// Thousand, lakh (10⁵), crore (10⁷). For the Indian subcontinent.
    Indian,
}

impl CompactStyle {
    /// The style `locale` abbreviates in.
    #[must_use]
    pub fn of(locale: Locale) -> Self {
        match locale.language() {
            "ja" | "zh" => Self::MyriadCjk,
            "hi" | "bn" | "gu" | "kn" | "mr" | "pa" | "ta" | "te" | "ne" | "ur" | "as" | "or"
            | "ml" => Self::Indian,
            _ => Self::Latin,
        }
    }

    /// The (divisor, suffix) pairs, largest first.
    const fn steps(self) -> &'static [(f64, &'static str)] {
        match self {
            Self::Latin => &[(1e12, "T"), (1e9, "B"), (1e6, "M"), (1e3, "K")],
            Self::MyriadCjk => &[(1e12, "兆"), (1e8, "億"), (1e4, "万")],
            Self::Indian => &[(1e7, "Cr"), (1e5, "L"), (1e3, "K")],
        }
    }
}

/// Writes a number the way a locale writes it.
///
/// # Why this is a value rather than a function
///
/// Because the settings travel. A price column formats hundreds of numbers with
/// one arrangement, and a formatter built once and reused is both faster and
/// impossible to get inconsistent halfway down the list.
///
/// ```
/// use vieww_foundation::intl::NumberFormat;
/// use vieww_foundation::Locale;
///
/// let french = NumberFormat::decimal(Locale::parse("fr-FR").expect("a real tag"));
/// assert_eq!(french.format(1234.5), "1\u{202F}234,5");
///
/// let english = NumberFormat::decimal(Locale::ENGLISH).with_fraction_digits(2, 2);
/// assert_eq!(english.format(1234.5), "1,234.50");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct NumberFormat {
    locale: Locale,
    symbols: NumberSymbols,
    style: NumberStyle,
    min_fraction: u8,
    max_fraction: u8,
    grouped: bool,
    symbol_position: SymbolPosition,
}

impl NumberFormat {
    /// The most digits this will ever write after the point.
    ///
    /// Past about fifteen significant digits an `f64` has no more information
    /// to give, and printing further would be inventing it.
    const MAX_FRACTION: u8 = 15;

    /// A plain number: `1,234.5`.
    ///
    /// Up to three fraction digits and no trailing zeroes, which is the
    /// arrangement almost every "just show me the number" call wants and the
    /// one `{:.2}` is not.
    #[must_use]
    pub fn decimal(locale: Locale) -> Self {
        Self {
            locale,
            symbols: NumberSymbols::of(locale),
            style: NumberStyle::Decimal,
            min_fraction: 0,
            max_fraction: 3,
            grouped: true,
            symbol_position: Currency::position_for(locale),
        }
    }

    /// A whole number: `1,234`.
    #[must_use]
    pub fn integer(locale: Locale) -> Self {
        Self::decimal(locale).with_fraction_digits(0, 0)
    }

    /// A percentage. **The value is multiplied by 100**, so `0.125` is
    /// `12.5%` — the convention every platform formatter uses, and stated here
    /// because passing `12.5` and getting `1,250%` is the first mistake
    /// everybody makes.
    #[must_use]
    pub fn percent(locale: Locale) -> Self {
        Self {
            style: NumberStyle::Percent,
            max_fraction: 0,
            ..Self::decimal(locale)
        }
    }

    /// Money, with the symbol where this locale puts it and the number of
    /// decimals the currency has.
    #[must_use]
    pub fn currency(locale: Locale, currency: Currency) -> Self {
        let digits = currency.fraction_digits.min(Self::MAX_FRACTION);
        let base = Self::decimal(locale);
        // The locale decides which *side*, and normally the spacing too. A
        // currency only overrides the spacing, and only when it says so —
        // which is what lets an ISO code be spaced in a locale that butts its
        // symbol against the digits, without a caller having to know to ask.
        let position = match (base.symbol_position, currency.spaced) {
            (position, None) => position,
            (SymbolPosition::Before | SymbolPosition::BeforeWithSpace, Some(true)) => {
                SymbolPosition::BeforeWithSpace
            }
            (SymbolPosition::Before | SymbolPosition::BeforeWithSpace, Some(false)) => {
                SymbolPosition::Before
            }
            (SymbolPosition::After | SymbolPosition::AfterWithSpace, Some(true)) => {
                SymbolPosition::AfterWithSpace
            }
            (SymbolPosition::After | SymbolPosition::AfterWithSpace, Some(false)) => {
                SymbolPosition::After
            }
        };
        Self {
            style: NumberStyle::Currency(currency),
            min_fraction: digits,
            max_fraction: digits,
            symbol_position: position,
            ..base
        }
    }

    /// Abbreviated: `1.2K`, `3.4M`, `1.2万`.
    ///
    /// One fraction digit, because that is what a compact number is for — a
    /// glance, not an audit.
    #[must_use]
    pub fn compact(locale: Locale) -> Self {
        Self {
            style: NumberStyle::Compact,
            max_fraction: 1,
            ..Self::decimal(locale)
        }
    }

    /// Fix the number of digits after the point.
    ///
    /// `min` pads with zeroes and `max` rounds. Both are clamped to
    /// fifteen digits and `min` is never allowed above
    /// `max`, so no combination of arguments produces a panic or a nonsense
    /// arrangement.
    #[must_use]
    pub const fn with_fraction_digits(mut self, min: u8, max: u8) -> Self {
        let max = if max > Self::MAX_FRACTION {
            Self::MAX_FRACTION
        } else {
            max
        };
        self.max_fraction = max;
        self.min_fraction = if min > max { max } else { min };
        self
    }

    /// Turn the digit grouping off, for a number that is really an identifier.
    ///
    /// A year is the case: `2026`, never `2,026`.
    #[must_use]
    pub const fn without_grouping(mut self) -> Self {
        self.grouped = false;
        self
    }

    /// Put the currency symbol somewhere other than where the locale would.
    #[must_use]
    pub const fn with_symbol_position(mut self, position: SymbolPosition) -> Self {
        self.symbol_position = position;
        self
    }

    /// The locale this formatter was built for.
    #[must_use]
    pub const fn locale(&self) -> Locale {
        self.locale
    }

    /// The symbols in use, for a caller assembling something this does not do.
    #[must_use]
    pub const fn symbols(&self) -> NumberSymbols {
        self.symbols
    }

    /// Write `value`.
    ///
    /// Infinities and NaN come out as `∞`, `-∞` and `NaN` rather than as a
    /// grouped pile of digits, because there is no arrangement of separators
    /// that makes `inf` read correctly and a formatter that produced one would
    /// be hiding a division by zero somewhere upstream.
    #[must_use]
    pub fn format(&self, value: f64) -> String {
        if value.is_nan() {
            return "NaN".to_owned();
        }
        if value.is_infinite() {
            let mut out = String::new();
            if value < 0.0 {
                out.push(self.symbols.minus);
            }
            out.push('∞');
            return out;
        }

        let (scaled, suffix) = match &self.style {
            NumberStyle::Percent => (value * 100.0, String::new()),
            NumberStyle::Compact => {
                let (scaled, suffix) = self.compact_parts(value);
                (scaled, suffix.to_owned())
            }
            NumberStyle::Decimal | NumberStyle::Currency(_) => (value, String::new()),
        };

        let negative = scaled.is_sign_negative() && scaled != 0.0;
        let digits = self.digits(scaled.abs());

        let mut out = String::with_capacity(digits.len() + 8);
        if negative {
            out.push(self.symbols.minus);
        }
        out.push_str(&digits);
        out.push_str(&suffix);

        match &self.style {
            NumberStyle::Percent => out.push(self.symbols.percent),
            NumberStyle::Currency(currency) => {
                out = self.place_symbol(out, &currency.symbol, negative);
            }
            NumberStyle::Decimal | NumberStyle::Compact => {}
        }
        out
    }

    /// Write an integer without going through `f64`.
    ///
    /// Above 2⁵³ an `f64` cannot hold every integer, so a file size or an id
    /// formatted through [`format`](Self::format) can come back off by one.
    /// This path never converts, so it cannot.
    #[must_use]
    pub fn format_int(&self, value: i64) -> String {
        if matches!(self.style, NumberStyle::Decimal) && self.min_fraction == 0 {
            let negative = value < 0;
            let digits = value.unsigned_abs().to_string();
            let mut out = String::with_capacity(digits.len() + 4);
            if negative {
                out.push(self.symbols.minus);
            }
            out.push_str(&self.group(&digits));
            return out;
        }
        // Any other style is going through the scaling and rounding path
        // anyway, where the precision was already spent.
        #[expect(
            clippy::cast_precision_loss,
            reason = "the styles that reach here scale or round the value regardless"
        )]
        self.format(value as f64)
    }

    /// The digits of a non-negative number, grouped and with the fraction the
    /// settings ask for.
    fn digits(&self, value: f64) -> String {
        let rendered = format!("{value:.*}", self.max_fraction as usize);
        let (whole, fraction) = rendered.split_once('.').unwrap_or((rendered.as_str(), ""));

        // Trim the zeroes `{:.*}` padded on, down to the minimum asked for.
        let keep = fraction
            .trim_end_matches('0')
            .len()
            .max(self.min_fraction as usize)
            .min(fraction.len());

        let mut out = self.group(whole);
        if keep > 0 {
            out.push(self.symbols.decimal);
            out.push_str(&fraction[..keep]);
        }
        out
    }

    /// Insert the group separators into a string of ASCII digits.
    ///
    /// Walks from the right, because that is the end the grouping is anchored
    /// to and the only way the Indian 3-then-2 arrangement comes out right.
    fn group(&self, digits: &str) -> String {
        if !self.grouped || digits.len() <= self.symbols.primary_group as usize {
            return digits.to_owned();
        }

        let bytes = digits.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 2);
        let mut since_break = 0_u8;
        let mut group_size = self.symbols.primary_group.max(1);

        for &byte in bytes.iter().rev() {
            if since_break == group_size {
                out.push(self.symbols.group);
                since_break = 0;
                group_size = self.symbols.secondary_group.max(1);
            }
            out.push(byte as char);
            since_break += 1;
        }
        out.iter().rev().collect()
    }

    /// The scaled value and the suffix for a compact number.
    fn compact_parts(&self, value: f64) -> (f64, &'static str) {
        let magnitude = value.abs();
        for &(divisor, suffix) in CompactStyle::of(self.locale).steps() {
            if magnitude >= divisor {
                return (value / divisor, suffix);
            }
        }
        (value, "")
    }

    /// Put the currency symbol where this locale puts it.
    ///
    /// The minus stays on the outside — `-$5.00`, not `$-5.00` — because that
    /// is what every platform does and because a minus buried between the
    /// symbol and the digits is genuinely easy to miss on a statement.
    fn place_symbol(&self, digits: String, symbol: &str, negative: bool) -> String {
        let mut out = String::with_capacity(digits.len() + symbol.len() + 2);
        match self.symbol_position {
            SymbolPosition::Before | SymbolPosition::BeforeWithSpace => {
                let body = if negative {
                    out.push(self.symbols.minus);
                    &digits[self.symbols.minus.len_utf8()..]
                } else {
                    digits.as_str()
                };
                out.push_str(symbol);
                if self.symbol_position == SymbolPosition::BeforeWithSpace {
                    out.push('\u{A0}');
                }
                out.push_str(body);
            }
            SymbolPosition::After | SymbolPosition::AfterWithSpace => {
                out.push_str(&digits);
                if self.symbol_position == SymbolPosition::AfterWithSpace {
                    out.push('\u{A0}');
                }
                out.push_str(symbol);
            }
        }
        out
    }
}

/// Bytes, written for a person.
///
/// Not strictly a locale question — the units are the same everywhere — but the
/// *number* in front of them is, and every application that shows a file size
/// has hand-rolled this with a hard-coded `.` and `,`.
///
/// Powers of 1024 with the IEC-ish short names everybody actually reads (`KB`,
/// `MB`), rather than powers of 1000 with the pedantically correct ones. Stated
/// because the two differ by 2.4% at a gigabyte and somebody always notices.
#[must_use]
pub fn format_bytes(locale: Locale, bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    #[expect(
        clippy::cast_precision_loss,
        reason = "a byte count past 2^53 is not a file, and the result is rounded to one decimal"
    )]
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    let format = if unit == 0 {
        NumberFormat::integer(locale)
    } else {
        NumberFormat::decimal(locale).with_fraction_digits(0, 1)
    };

    let mut out = format.format(value);
    let _ = write!(out, "\u{A0}{}", UNITS[unit]);
    out
}
