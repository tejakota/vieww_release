//! ICU MessageFormat, enough of it to be useful and no more.

use std::collections::HashMap;
use std::fmt;

use crate::{Locale, PluralCategory};

use super::NumberFormat;

/// An argument a message can interpolate.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A whole number. The one that plurals select on, and the one an id or a
    /// count should use — see
    /// [`NumberFormat::format_int`](super::NumberFormat::format_int) for why
    /// going through `f64` loses large integers.
    Int(i64),
    /// A measured quantity.
    Number(f64),
    /// Anything else — a name, a place, a formatted date somebody else
    /// produced.
    Text(String),
}

impl Value {
    /// The count this value selects a plural form with, if it has one.
    const fn count(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a plural category is chosen from the integer part, which is what CLDR's integer rules use"
            )]
            Self::Number(value) => Some(*value as i64),
            Self::Text(_) => None,
        }
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Self::Int(i64::try_from(value).unwrap_or(i64::MAX))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

/// The arguments a message is formatted with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Args(HashMap<String, Value>);

impl Args {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one, and return the builder — `Args::new().with("count", 3)`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.0.insert(name.into(), value.into());
        self
    }

    /// Add one in place.
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<Value>) {
        self.0.insert(name.into(), value.into());
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }
}

/// What is wrong with a pattern.
///
/// Every variant carries the byte offset it was found at, because a translation
/// file has thousands of these in it and "unbalanced brace" without a position
/// is a message that costs somebody an afternoon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// A `{` with no matching `}`.
    UnclosedBrace { at: usize },
    /// A `}` that closes nothing.
    StrayBrace { at: usize },
    /// `{}` or `{, plural, …}` — no argument named.
    EmptyArgument { at: usize },
    /// An argument type this subset does not implement.
    UnknownType { at: usize, kind: String },
    /// A `plural` or `select` with no `other` branch.
    ///
    /// Required by ICU and required here: `other` is the only category every
    /// language has, so a message without one is a message that cannot be
    /// rendered in some locale. Catching it at parse time is the difference
    /// between a failing test and a blank label in Arabic.
    MissingOther { at: usize, argument: String },
    /// A malformed `offset:` value.
    BadOffset { at: usize },
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnclosedBrace { at } => write!(f, "unclosed '{{' at byte {at}"),
            Self::StrayBrace { at } => write!(f, "unmatched '}}' at byte {at}"),
            Self::EmptyArgument { at } => write!(f, "argument with no name at byte {at}"),
            Self::UnknownType { at, kind } => {
                write!(f, "unsupported argument type '{kind}' at byte {at}")
            }
            Self::MissingOther { at, argument } => write!(
                f,
                "'{argument}' at byte {at} has no 'other' branch, so some locale cannot render it"
            ),
            Self::BadOffset { at } => write!(f, "malformed 'offset:' at byte {at}"),
        }
    }
}

impl std::error::Error for MessageError {}

/// One piece of a parsed message.
#[derive(Debug, Clone, PartialEq)]
enum Part {
    Literal(String),
    /// `{name}` or `{name, number}` — ICU treats the two identically, and so
    /// does this: a bare argument holding a number is formatted as one.
    Simple {
        name: String,
    },
    /// `{name, plural, offset:1 one {…} other {…}}`
    Plural {
        name: String,
        offset: i64,
        /// Exact matches — ICU's `=0`, `=1`.
        exact: Vec<(i64, Vec<Part>)>,
        categories: Vec<(PluralCategory, Vec<Part>)>,
        other: Vec<Part>,
    },
    /// `{name, select, female {…} other {…}}`
    Select {
        name: String,
        branches: Vec<(String, Vec<Part>)>,
        other: Vec<Part>,
    },
    /// `#` inside a plural branch: the count, minus the offset, formatted.
    Hash,
}

/// A message pattern, parsed once and rendered many times.
///
/// # Why this exists, when [`Locale`](crate::Locale) argues against catalogues
///
/// It does not contradict that argument, it completes it. `locale`'s point is
/// that vieww should not own an application's *messages* — no
/// `catalogue.get("greeting")`, no stringly-typed lookup, because Rust can do
/// better and an application's own type can carry its own strings.
///
/// But the strings themselves still come from translators, and translators work
/// in tools that emit ICU MessageFormat. Every translation management system
/// speaks it; it is the interchange format of the industry. An application that
/// wants real translations either parses it or reimplements plural selection by
/// hand in every message — which is the thing
/// [`PluralCategory`](crate::PluralCategory) exists to stop.
///
/// So: **the catalogue is the application's, the pattern language is
/// interoperable, and the parse is checked.** A malformed pattern is a
/// [`MessageError`] with a byte offset at load time, not a wrong string on a
/// screen.
///
/// # What is implemented
///
/// - `{name}` — interpolation
/// - `{name, number}` — interpolation, formatted for the locale
/// - `{name, plural, …}` with `=n` exact matches, CLDR categories, `offset:n`
///   and `#`
/// - `{name, select, …}`
/// - `'` quoting: `'{'` is a literal brace, `''` is an apostrophe
///
/// # What is not, and why
///
/// `date`, `time` and `selectordinal`. The first two would embed a pattern
/// language inside a pattern language and produce dates with no
/// [`CalendarNames`](super::CalendarNames) to name their months —
/// [`DateTimeFormat`](super::DateTimeFormat) does that job properly and its
/// output goes in as [`Value::Text`]. `selectordinal` needs a second CLDR
/// table (ordinal rules differ from cardinal ones) that this crate does not
/// carry; a message needing "1st, 2nd, 3rd" selects on a
/// [`Value::Text`] the application produced.
///
/// Unimplemented types are a parse **error**, not a silent pass-through. A
/// pattern using one would otherwise render as its own source text.
///
/// ```
/// use vieww_foundation::intl::{Args, MessageFormat};
/// use vieww_foundation::Locale;
///
/// let message = MessageFormat::parse(
///     "{count, plural, =0 {No files} one {# file} other {# files}}",
/// )
/// .expect("a well-formed pattern");
///
/// let en = Locale::ENGLISH;
/// assert_eq!(message.format(en, &Args::new().with("count", 0)), "No files");
/// assert_eq!(message.format(en, &Args::new().with("count", 1)), "1 file");
/// assert_eq!(message.format(en, &Args::new().with("count", 7)), "7 files");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MessageFormat {
    parts: Vec<Part>,
}

impl MessageFormat {
    /// Parse a pattern.
    ///
    /// # Errors
    ///
    /// [`MessageError`], with the byte offset of the problem.
    pub fn parse(pattern: &str) -> Result<Self, MessageError> {
        let mut cursor = Cursor {
            source: pattern,
            at: 0,
        };
        let parts = cursor.parts(false)?;
        if cursor.at < pattern.len() {
            return Err(MessageError::StrayBrace { at: cursor.at });
        }
        Ok(Self { parts })
    }

    /// Render it.
    ///
    /// An argument the pattern names and `args` does not have renders as its own
    /// name in braces — `{count}` — rather than as an empty string or a panic.
    /// That is deliberate: a missing argument is a bug, and a visible one gets
    /// fixed while a silent gap ships.
    #[must_use]
    pub fn format(&self, locale: Locale, args: &Args) -> String {
        let mut out = String::new();
        render(&self.parts, locale, args, None, &mut out);
        out
    }

    /// The argument names this pattern uses, in first-appearance order.
    ///
    /// For a test that checks a translation file against the arguments the code
    /// actually passes — which is the check that catches a translator's typo
    /// before a user does.
    #[must_use]
    pub fn arguments(&self) -> Vec<String> {
        let mut names = Vec::new();
        collect(&self.parts, &mut names);
        names
    }
}

fn collect(parts: &[Part], names: &mut Vec<String>) {
    let push = |name: &String, names: &mut Vec<String>| {
        if !names.iter().any(|seen| seen == name) {
            names.push(name.clone());
        }
    };
    for part in parts {
        match part {
            Part::Literal(_) | Part::Hash => {}
            Part::Simple { name } => push(name, names),
            Part::Plural {
                name,
                exact,
                categories,
                other,
                ..
            } => {
                push(name, names);
                for (_, branch) in exact {
                    collect(branch, names);
                }
                for (_, branch) in categories {
                    collect(branch, names);
                }
                collect(other, names);
            }
            Part::Select {
                name,
                branches,
                other,
            } => {
                push(name, names);
                for (_, branch) in branches {
                    collect(branch, names);
                }
                collect(other, names);
            }
        }
    }
}

/// `hash` is the value `#` stands for inside the plural branch being rendered.
fn render(parts: &[Part], locale: Locale, args: &Args, hash: Option<i64>, out: &mut String) {
    for part in parts {
        match part {
            Part::Literal(text) => out.push_str(text),
            Part::Hash => {
                if let Some(count) = hash {
                    out.push_str(&NumberFormat::integer(locale).format_int(count));
                }
            }
            Part::Simple { name } => match args.get(name) {
                Some(Value::Text(text)) => out.push_str(text),
                Some(Value::Int(value)) => {
                    out.push_str(&NumberFormat::integer(locale).format_int(*value));
                }
                Some(Value::Number(value)) => {
                    out.push_str(&NumberFormat::decimal(locale).format(*value));
                }
                None => {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                }
            },
            Part::Plural {
                name,
                offset,
                exact,
                categories,
                other,
            } => {
                let Some(count) = args.get(name).and_then(Value::count) else {
                    out.push('{');
                    out.push_str(name);
                    out.push('}');
                    continue;
                };
                // ICU matches `=n` against the *original* count and the
                // categories against the offset one. That asymmetry is the
                // whole point of `offset`: "You and 2 others" is `=0`/`=1` on
                // the real number and a plural on the remainder.
                let shifted = count.saturating_sub(*offset);
                let branch = exact
                    .iter()
                    .find(|(value, _)| *value == count)
                    .map(|(_, branch)| branch)
                    .or_else(|| {
                        let category = locale.plural(shifted.unsigned_abs());
                        categories
                            .iter()
                            .find(|(candidate, _)| *candidate == category)
                            .map(|(_, branch)| branch)
                    })
                    .unwrap_or(other);
                render(branch, locale, args, Some(shifted), out);
            }
            Part::Select {
                name,
                branches,
                other,
            } => {
                let chosen = match args.get(name) {
                    Some(Value::Text(text)) => branches
                        .iter()
                        .find(|(key, _)| key == text)
                        .map(|(_, branch)| branch)
                        .unwrap_or(other),
                    _ => other,
                };
                render(chosen, locale, args, hash, out);
            }
        }
    }
}

/// A position in the pattern, and the recursive-descent parser over it.
struct Cursor<'a> {
    source: &'a str,
    at: usize,
}

impl Cursor<'_> {
    fn rest(&self) -> &str {
        &self.source[self.at..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.at += next.len_utf8();
        Some(next)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.at += expected.len_utf8();
            true
        } else {
            false
        }
    }

    /// Parse parts until `}` (when `nested`) or the end of the source.
    fn parts(&mut self, nested: bool) -> Result<Vec<Part>, MessageError> {
        let mut parts = Vec::new();
        let mut literal = String::new();

        while let Some(next) = self.peek() {
            match next {
                '}' if nested => break,
                '}' => return Err(MessageError::StrayBrace { at: self.at }),
                '{' => {
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(self.argument()?);
                }
                '#' if nested => {
                    self.at += 1;
                    if !literal.is_empty() {
                        parts.push(Part::Literal(std::mem::take(&mut literal)));
                    }
                    parts.push(Part::Hash);
                }
                '\'' => {
                    self.at += 1;
                    // `''` is a literal apostrophe. A single one starts a
                    // quoted run, which is how a pattern says `'{'`.
                    if self.eat('\'') {
                        literal.push('\'');
                        continue;
                    }
                    while let Some(inside) = self.bump() {
                        if inside == '\'' {
                            if self.eat('\'') {
                                literal.push('\'');
                                continue;
                            }
                            break;
                        }
                        literal.push(inside);
                    }
                }
                other => {
                    self.at += other.len_utf8();
                    literal.push(other);
                }
            }
        }

        if !literal.is_empty() {
            parts.push(Part::Literal(literal));
        }
        Ok(parts)
    }

    /// Parse one `{ … }`.
    fn argument(&mut self) -> Result<Part, MessageError> {
        let opened = self.at;
        self.at += 1; // the '{'
        self.skip_space();

        let name = self.name();
        if name.is_empty() {
            return Err(MessageError::EmptyArgument { at: opened });
        }
        self.skip_space();

        if self.eat('}') {
            return Ok(Part::Simple { name });
        }
        if !self.eat(',') {
            return Err(MessageError::UnclosedBrace { at: opened });
        }
        self.skip_space();

        let kind = self.name();
        self.skip_space();

        match kind.as_str() {
            "number" => {
                if !self.eat('}') {
                    return Err(MessageError::UnclosedBrace { at: opened });
                }
                Ok(Part::Simple { name })
            }
            "plural" => self.plural(name, opened),
            "select" => self.select(name, opened),
            other => Err(MessageError::UnknownType {
                at: opened,
                kind: other.to_owned(),
            }),
        }
    }

    fn plural(&mut self, name: String, opened: usize) -> Result<Part, MessageError> {
        if !self.eat(',') {
            return Err(MessageError::UnclosedBrace { at: opened });
        }
        self.skip_space();

        let mut offset = 0_i64;
        if self.rest().starts_with("offset:") {
            self.at += "offset:".len();
            let digits = self.name();
            offset = digits
                .parse()
                .map_err(|_| MessageError::BadOffset { at: self.at })?;
            self.skip_space();
        }

        let mut exact = Vec::new();
        let mut categories = Vec::new();
        let mut other = None;

        while !self.eat('}') {
            self.skip_space();
            if self.peek().is_none() {
                return Err(MessageError::UnclosedBrace { at: opened });
            }
            if self.eat('}') {
                break;
            }

            let key = self.key();
            self.skip_space();
            if !self.eat('{') {
                return Err(MessageError::UnclosedBrace { at: self.at });
            }
            let body = self.parts(true)?;
            if !self.eat('}') {
                return Err(MessageError::UnclosedBrace { at: opened });
            }
            self.skip_space();

            if let Some(literal) = key.strip_prefix('=') {
                let value = literal
                    .parse()
                    .map_err(|_| MessageError::BadOffset { at: opened })?;
                exact.push((value, body));
            } else if key == "other" {
                other = Some(body);
            } else if let Some(category) = category_of(&key) {
                categories.push((category, body));
            } else {
                return Err(MessageError::UnknownType {
                    at: opened,
                    kind: key,
                });
            }
        }

        let other = other.ok_or(MessageError::MissingOther {
            at: opened,
            argument: name.clone(),
        })?;
        Ok(Part::Plural {
            name,
            offset,
            exact,
            categories,
            other,
        })
    }

    fn select(&mut self, name: String, opened: usize) -> Result<Part, MessageError> {
        if !self.eat(',') {
            return Err(MessageError::UnclosedBrace { at: opened });
        }
        self.skip_space();

        let mut branches = Vec::new();
        let mut other = None;

        while !self.eat('}') {
            self.skip_space();
            if self.peek().is_none() {
                return Err(MessageError::UnclosedBrace { at: opened });
            }
            if self.eat('}') {
                break;
            }

            let key = self.key();
            self.skip_space();
            if !self.eat('{') {
                return Err(MessageError::UnclosedBrace { at: self.at });
            }
            let body = self.parts(true)?;
            if !self.eat('}') {
                return Err(MessageError::UnclosedBrace { at: opened });
            }
            self.skip_space();

            if key == "other" {
                other = Some(body);
            } else {
                branches.push((key, body));
            }
        }

        let other = other.ok_or(MessageError::MissingOther {
            at: opened,
            argument: name.clone(),
        })?;
        Ok(Part::Select {
            name,
            branches,
            other,
        })
    }

    /// An identifier: letters, digits, `_`, `-`, `+`.
    fn name(&mut self) -> String {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '-' | '+'))
        {
            self.at += self.peek().map_or(0, char::len_utf8);
        }
        self.source[start..self.at].to_owned()
    }

    /// A branch key, which may lead with `=`.
    fn key(&mut self) -> String {
        let leading = if self.eat('=') { "=" } else { "" };
        format!("{leading}{}", self.name())
    }
}

fn category_of(key: &str) -> Option<PluralCategory> {
    match key {
        "zero" => Some(PluralCategory::Zero),
        "one" => Some(PluralCategory::One),
        "two" => Some(PluralCategory::Two),
        "few" => Some(PluralCategory::Few),
        "many" => Some(PluralCategory::Many),
        "other" => Some(PluralCategory::Other),
        _ => None,
    }
}
