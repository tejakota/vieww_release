//! A minimal JSON reader, because `cargo` output breaks a substring search.
//!
//! # Why this exists at all
//!
//! [`crate::compile::parse_diagnostics`] read `rustc --error-format=json` by
//! searching each line for `"level":"` and taking what followed. Its own doc
//! comment admitted the trade — it "would be defeated by a `"` inside a
//! message" — and accepted it, because a `rustc` record is flat, shallow, and
//! written by one emitter whose key order does not move.
//!
//! `cargo --message-format=json` is none of those things:
//!
//! * Every diagnostic is **nested** inside a wrapper —
//!   `{"reason":"compiler-message","message":{…the rustc record…}}` — so the
//!   file name and the level a search finds first may belong to a different
//!   object than the one being read.
//! * Each record carries a `"rendered"` field holding the **entire
//!   human-readable diagnostic**, quotes and all. A search for `"level":"`
//!   inside that string finds whatever the user's own source code happened to
//!   contain.
//! * `"children"` is an array of further diagnostics, each with its own
//!   `"level"` and `"message"`. Whether the parent's level or a child's is
//!   found first is a question about key order in someone else's serialiser.
//!
//! So the substring approach does not degrade on cargo output, it reports
//! confident nonsense: a note's text at an error's severity, attributed to a
//! file that is not the one with the error in it. Parsing properly is the
//! cheaper option, and it is 150 lines with no dependency.
//!
//! # What this deliberately is not
//!
//! Not a general-purpose JSON library. There is no serialiser, no derive, no
//! borrowing-from-input lifetime, and no attempt at speed beyond not being
//! quadratic. It reads one line of compiler output at a time — a few kilobytes
//! — and the studio's dependency list stays where it is.
//!
//! # Numbers
//!
//! Parsed as `f64`, JSON's own model. [`Json::as_u32`] is the accessor every
//! caller here actually wants and it refuses anything that is not a
//! non-negative integer in range, so a line number cannot silently become 0
//! from a value of `-1` or `1e300`.

use std::fmt;

/// A parsed JSON value.
///
/// Objects keep their fields in source order in a `Vec` rather than a map:
/// records here have a handful of keys, a linear scan beats hashing at that
/// size, and preserving order makes a failure legible when one is printed.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

/// Why a line could not be read.
///
/// Carries the byte offset, because the one thing worse than a parse failure
/// on machine-generated input is a parse failure that does not say where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

/// How deep nesting may go before it is refused.
///
/// Recursive descent on attacker-shaped input is a stack overflow, which is an
/// abort rather than an error — and an abort in the studio is the user's unsaved
/// buffers. Compiler output nests about six deep; 128 is far past anything
/// honest and far short of anything fatal.
const MAX_DEPTH: usize = 128;

impl Json {
    /// Read one complete JSON value, which must be all there is but for
    /// surrounding whitespace.
    ///
    /// # Errors
    ///
    /// If the input is not one well-formed JSON value.
    pub fn parse(input: &str) -> Result<Self, ParseError> {
        let bytes = input.as_bytes();
        let mut p = Parser { bytes, at: 0 };
        p.skip_whitespace();
        let value = p.value(0)?;
        p.skip_whitespace();
        if p.at != bytes.len() {
            return Err(p.error("trailing input after the value"));
        }
        Ok(value)
    }

    /// The value of `key`, if this is an object that has one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Follow a chain of keys — `v.path(["message", "code", "code"])`.
    ///
    /// The reason the callers here never index by hand: every step is an
    /// `Option`, and a hand-written chain of `?`s is where a typo turns into a
    /// silent `None` that reads as "the compiler said nothing".
    #[must_use]
    pub fn path<'a, I>(&self, keys: I) -> Option<&Self>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut here = self;
        for key in keys {
            here = here.get(key)?;
        }
        Some(here)
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The value as a `u32`, and **only** if it really is one.
    ///
    /// A line number that arrives as `-1`, `1.5`, or `1e300` is not a line
    /// number, and answering `0` for it would put a diagnostic at the top of a
    /// file it has nothing to do with. Those all answer `None` so the caller
    /// can use its own default deliberately.
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        let n = self.as_f64()?;
        if n.is_finite() && n >= 0.0 && n <= f64::from(u32::MAX) && n.fract() == 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(n as u32)
        } else {
            None
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// `get(key)` and `as_str`, the pair every caller in `cargo.rs` wants.
    #[must_use]
    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, message: &'static str) -> ParseError {
        ParseError {
            offset: self.at,
            message,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// Consume `word` if it is next.
    fn literal(&mut self, word: &str) -> bool {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        match self.peek().ok_or_else(|| self.error("expected a value"))? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::String),
            b't' if self.literal("true") => Ok(Json::Bool(true)),
            b'f' if self.literal("false") => Ok(Json::Bool(false)),
            b'n' if self.literal("null") => Ok(Json::Null),
            _ => self.number(),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.at += 1; // '{'
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a key"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected ':'"));
            }
            self.at += 1;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.at += 1; // '['
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    /// A JSON string, with every escape JSON defines resolved.
    ///
    /// `\u` is the one with a trap in it: a character outside the basic plane
    /// arrives as a **surrogate pair**, two `\u` escapes that mean one
    /// character only when read together. An emoji in a diagnostic — which is
    /// to say, an emoji in a string literal in the user's own source — is
    /// exactly that case.
    fn string(&mut self) -> Result<String, ParseError> {
        self.at += 1; // opening quote
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated string"))?;
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                _ => {
                    // Everything else passes through as-is. Stepping by UTF-8
                    // character rather than by byte, because the input is a
                    // `&str` and slicing it mid-character panics.
                    let rest = std::str::from_utf8(&self.bytes[self.at..])
                        .map_err(|_| self.error("invalid UTF-8"))?;
                    let c = rest
                        .chars()
                        .next()
                        .ok_or_else(|| self.error("unterminated string"))?;
                    out.push(c);
                    self.at += c.len_utf8();
                }
            }
        }
    }

    /// The four hex digits after `\u`, and its partner if this is a surrogate.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let first = self.hex4()?;
        // Not a lead surrogate: it stands alone.
        if !(0xD800..0xDC00).contains(&first) {
            return char::from_u32(u32::from(first)).ok_or_else(|| self.error("invalid escape"));
        }
        // A lead surrogate must be followed by `\uDC00..`.
        if !(self.literal("\\u")) {
            return Err(self.error("lone surrogate"));
        }
        let second = self.hex4()?;
        if !(0xDC00..0xE000).contains(&second) {
            return Err(self.error("lone surrogate"));
        }
        let combined =
            0x1_0000 + ((u32::from(first) - 0xD800) << 10) + (u32::from(second) - 0xDC00);
        char::from_u32(combined).ok_or_else(|| self.error("invalid escape"))
    }

    fn hex4(&mut self) -> Result<u16, ParseError> {
        let end = self.at + 4;
        if end > self.bytes.len() {
            return Err(self.error("truncated \\u escape"));
        }
        let digits = std::str::from_utf8(&self.bytes[self.at..end])
            .map_err(|_| self.error("bad \\u escape"))?;
        let value = u16::from_str_radix(digits, 16).map_err(|_| self.error("bad \\u escape"))?;
        self.at = end;
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        if start == self.at {
            return Err(self.error("expected a value"));
        }
        std::str::from_utf8(&self.bytes[start..self.at])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(Json::Number)
            .ok_or(ParseError {
                offset: start,
                message: "not a number",
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_round_trip() {
        assert_eq!(Json::parse("null"), Ok(Json::Null));
        assert_eq!(Json::parse("true"), Ok(Json::Bool(true)));
        assert_eq!(Json::parse("false"), Ok(Json::Bool(false)));
        assert_eq!(Json::parse("  12  "), Ok(Json::Number(12.0)));
        assert_eq!(Json::parse("-1.5e2"), Ok(Json::Number(-150.0)));
        assert_eq!(Json::parse(r#""hi""#).unwrap().as_str(), Some("hi"));
    }

    #[test]
    fn objects_and_arrays_nest() {
        let v = Json::parse(r#"{"a":[1,{"b":"c"}],"d":{}}"#).unwrap();
        assert_eq!(v.path(["a"]).unwrap().as_array().unwrap().len(), 2);
        assert_eq!(
            v.get("a").unwrap().as_array().unwrap()[1].str_field("b"),
            Some("c")
        );
        assert_eq!(v.get("d"), Some(&Json::Object(vec![])));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn empty_containers_are_not_a_special_case() {
        assert_eq!(Json::parse("[]"), Ok(Json::Array(vec![])));
        assert_eq!(Json::parse("{}"), Ok(Json::Object(vec![])));
        assert_eq!(Json::parse(r#"{ }"#), Ok(Json::Object(vec![])));
    }

    /// The whole reason this file exists: a quote *inside* a string must not
    /// end the string, and a key that appears inside a string value must not
    /// be findable as a key.
    #[test]
    fn a_quote_inside_a_string_does_not_end_it() {
        let v = Json::parse(
            r#"{"rendered":"expected `\"`, found `\"level\":\"error\"`","level":"warning"}"#,
        )
        .unwrap();
        assert_eq!(
            v.str_field("rendered"),
            Some(r#"expected `"`, found `"level":"error"`"#)
        );
        // A substring search would have answered "error" here. This is the bug.
        assert_eq!(v.str_field("level"), Some("warning"));
    }

    #[test]
    fn every_escape_json_defines() {
        let v = Json::parse(r#""a\"b\\c\/d\be\ff\ng\rh\ti""#).unwrap();
        assert_eq!(v.as_str(), Some("a\"b\\c/d\u{8}e\u{c}f\ng\rh\ti"));
    }

    /// An emoji in the user's source reaches the diagnostic as a surrogate
    /// pair. Read as two independent escapes it is two replacement characters
    /// or an error; read as a pair it is one character.
    #[test]
    fn a_surrogate_pair_is_one_character() {
        // The escaped forms are the ones that exercise `unicode_escape`. An
        // earlier version of this test used only the literal characters below,
        // which take the raw-UTF-8 path instead — so it passed against a build
        // with the pair handling deleted, and was checking nothing.
        // Escaped as a surrogate pair: two \u escapes, one character.
        assert_eq!(
            Json::parse("\"\\uD83D\\uDE00\"").unwrap().as_str(),
            Some("\u{1F600}")
        );
        assert_eq!(
            Json::parse("\"a\\uD83D\\uDE00b\"").unwrap().as_str(),
            Some("a\u{1F600}b")
        );
        // Escaped inside the basic plane: one \u, no pair.
        assert_eq!(Json::parse("\"\\u00e9\"").unwrap().as_str(), Some("\u{e9}"));
        // Literal, unescaped -- the other path, and what cargo actually emits.
        assert_eq!(
            Json::parse("\"\u{1F600}\"").unwrap().as_str(),
            Some("\u{1F600}")
        );
        assert_eq!(Json::parse("\"\u{e9}\"").unwrap().as_str(), Some("\u{e9}"));

        // A lead surrogate with no trail is not a character in any encoding.
        assert!(Json::parse(r#""\uD83D""#).is_err(), "a lone lead surrogate");
        assert!(
            Json::parse(r#""\uD83Dx""#).is_err(),
            "a lead surrogate followed by text"
        );
        assert!(
            Json::parse(r#""\uD83DA""#).is_err(),
            "a lead surrogate followed by 'A'"
        );
        assert!(
            Json::parse(r#""\uDE00""#).is_err(),
            "a trail surrogate alone"
        );
    }

    #[test]
    fn multibyte_text_outside_an_escape_survives() {
        // Not escaped — cargo emits UTF-8 directly for most non-ASCII.
        assert_eq!(
            Json::parse("\"héllo → 😀\"").unwrap().as_str(),
            Some("héllo → 😀")
        );
    }

    #[test]
    fn as_u32_refuses_what_is_not_a_line_number() {
        assert_eq!(Json::parse("7").unwrap().as_u32(), Some(7));
        assert_eq!(Json::parse("0").unwrap().as_u32(), Some(0));
        assert_eq!(Json::parse("-1").unwrap().as_u32(), None);
        assert_eq!(Json::parse("1.5").unwrap().as_u32(), None);
        assert_eq!(Json::parse("1e300").unwrap().as_u32(), None);
        assert_eq!(Json::parse(r#""7""#).unwrap().as_u32(), None);
    }

    #[test]
    fn malformed_input_is_an_error_with_a_place() {
        assert!(Json::parse("").is_err());
        assert!(Json::parse("{").is_err());
        assert!(Json::parse(r#"{"a":}"#).is_err());
        assert!(Json::parse(r#"{"a":1,}"#).is_err());
        assert!(Json::parse(r#"{a:1}"#).is_err());
        assert!(Json::parse("[1,2").is_err());
        assert!(Json::parse(r#""unterminated"#).is_err());
        // Two values on one line is not one value.
        assert!(Json::parse("{} {}").is_err());
        let e = Json::parse("[1,]").unwrap_err();
        assert!(e.offset > 0, "the error says where: {e}");
    }

    /// Recursive descent plus attacker-shaped input is an abort, not an error,
    /// and an abort here costs the user their unsaved buffers.
    #[test]
    fn absurd_nesting_is_refused_rather_than_overflowing_the_stack() {
        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        assert!(Json::parse(&deep).is_err());
    }

    #[test]
    fn path_walks_or_gives_up() {
        let v = Json::parse(r#"{"message":{"code":{"code":"E0425"}}}"#).unwrap();
        assert_eq!(
            v.path(["message", "code", "code"]).and_then(Json::as_str),
            Some("E0425")
        );
        assert_eq!(v.path(["message", "nope", "code"]), None);
        // `code` can be null, which is not an object — walking through it stops.
        let v = Json::parse(r#"{"message":{"code":null}}"#).unwrap();
        assert_eq!(v.path(["message", "code", "code"]), None);
    }
}
