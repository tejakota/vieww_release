//! Lexing: text in, physical lines out, each carrying its indentation depth
//! and its tokens. Say is line-oriented and indentation-structured, exactly
//! like the language its users never have to learn first — but unlike
//! Python's, the indent rule is fixed at 4 spaces, so the lexer's job stays
//! small enough to check by eye.

/// One physical line, lexed.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    /// 1-based, as the Problems panel shows it.
    pub number: u32,
    /// 4-space units. `None` for a blank or comment-only line, which carries
    /// no indentation and no tokens and only exists so line numbers survive.
    pub indent: Option<u32>,
    /// The tokens after comment stripping. Empty for blanks and comments.
    pub tokens: Vec<Tok>,
    /// The `-- say-language: 1` pragma. It is comment-*shaped* and must be
    /// recognised before comment stripping, which is exactly the trap §3
    /// warns about — so the lexer owns it and hands it over as a flag.
    pub pragma: bool,
}

/// One token: a word, a quoted string (decoded), or punctuation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Word(String),
    /// A `"…"` literal, already unescaped. The column is where the quote
    /// began, which is where a bad escape should point.
    Str {
        text: String,
        column: u32,
    },
    Colon,
    Comma,
    /// `#RRGGBB` or `#AARRGGBB`, with the `#` retained and the digits parsed.
    Hex(u32),
    /// `12`, `-7` — a whole-number literal.
    Int(i64),
    /// `3.5` — a decimal literal. Integers also arrive here when a fractional
    /// property accepts them, but the lexer keeps the two apart so the parser
    /// can tell `padded 16` from `padded 16.0` if it ever needs to.
    Float(f32),
}

impl Tok {
    /// The word this token holds, if it is a word.
    pub(crate) fn word(&self) -> Option<&str> {
        match self {
            Tok::Word(w) => Some(w),
            _ => None,
        }
    }

    /// A human-readable form for "unexpected X" sentences.
    pub(crate) fn describe(&self) -> String {
        match self {
            Tok::Word(w) => format!("`{w}`"),
            Tok::Str { text, .. } => format!("the text \"{text}\""),
            Tok::Colon => "`:`".to_owned(),
            Tok::Comma => "`,`".to_owned(),
            Tok::Hex(h) => format!("the colour #{h:06X}"),
            Tok::Int(i) => format!("the number {i}"),
            Tok::Float(f) => format!("the number {f}"),
        }
    }
}

/// Lex the whole source into lines. Never fails: anything it cannot make
/// sense of becomes a word token the parser will reject with a position.
pub(crate) fn lex(source: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let number = (index + 1) as u32;
        let indent_chars = raw.len() - raw.trim_start_matches(' ').len();
        let body = raw.trim_start_matches(' ');
        let is_pragma = body.starts_with("-- say-language:");
        let blank = !is_pragma && (body.is_empty() || body.starts_with("--"));
        let indent = if blank {
            None
        } else {
            Some(indent_chars as u32 / 4)
        };
        let tokens = if is_pragma {
            vec![
                Tok::Word("--".into()),
                Tok::Word("say-language:".into()),
                Tok::Int(1),
            ]
        } else if blank {
            Vec::new()
        } else {
            tokenize(body, indent_chars as u32)
        };
        lines.push(Line {
            number,
            indent,
            tokens,
            pragma: is_pragma,
        });
    }
    lines
}

/// Tokenize one line's body. `prefix` is the indentation already consumed, so
/// columns are 1-based against the original line.
pub(crate) fn tokenize(body: &str, prefix: u32) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    let mut word = String::new();
    let mut word_start = 0u32;

    let column = |i: usize| prefix + i as u32 + 1;

    // Flush a word in progress. Words stop at spaces, commas, colons and
    // quotes — `padded 16:` is three tokens, `a card,` is two plus a comma.
    macro_rules! flush {
        () => {
            if !word.is_empty() {
                tokens.push(finish_word(std::mem::take(&mut word)));
            }
        };
    }

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b' ' => {
                flush!();
                i += 1;
            }
            b',' => {
                flush!();
                tokens.push(Tok::Comma);
                i += 1;
            }
            b':' => {
                flush!();
                tokens.push(Tok::Colon);
                i += 1;
            }
            b'"' => {
                flush!();
                let (text, next) = read_string(body, i);
                tokens.push(Tok::Str {
                    text,
                    column: column(i),
                });
                i = next;
            }
            b'#' => {
                flush!();
                let (hex, next) = read_hex(body, i);
                match hex {
                    Some(value) => tokens.push(Tok::Hex(value)),
                    None => {
                        // Not a colour after all: hand the `#` to the parser
                        // as a word so the error names what was written.
                        word_start = i as u32;
                        word.push('#');
                    }
                }
                i = next;
            }
            b'-' if word.is_empty() && peek_is_digit_or_sign(body, i) => {
                if word_start == 0 && word.is_empty() {
                    word_start = i as u32;
                }
                word.push('-');
                i += 1;
            }
            b'0'..=b'9' if word.is_empty() => {
                word_start = i as u32;
                let (tok, next) = read_number(body, i);
                tokens.push(tok);
                i = next;
            }
            _ => {
                if word.is_empty() {
                    word_start = i as u32;
                }
                // Multi-byte UTF-8: push the whole char, not the byte.
                let ch = body[i..].chars().next().unwrap_or(' ');
                word.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    let _ = word_start;
    flush!();
    tokens
}

/// A word that is entirely numeric was already taken by `read_number`; what
/// arrives here is either a name or something numeric the parser will reject
/// where it stands.
fn finish_word(word: String) -> Tok {
    Tok::Word(word)
}

fn peek_is_digit_or_sign(body: &str, i: usize) -> bool {
    body[i..].chars().next().is_some_and(|c| c == '-')
}

/// Read a quoted string starting at the `"` at `start`. Returns the decoded
/// text and the index just past the closing quote. An unterminated string
/// ends at the line's end and lets the parser report it — the lexer has no
/// cross-line context and should not pretend to.
fn read_string(body: &str, start: usize) -> (String, usize) {
    let mut text = String::new();
    let mut i = start + 1;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return (text, i + 1),
            b'\\' => {
                if i + 1 < bytes.len() {
                    match bytes[i + 1] {
                        // `\\(` is a literal paren — both consumed together,
                        // so the `(` cannot be mistaken for an interpolation.
                        b'\\' if i + 2 < bytes.len() && bytes[i + 2] == b'(' => {
                            text.push('(');
                            i += 3;
                        }
                        b'\\' => {
                            text.push('\\');
                            i += 2;
                        }
                        // A bare `\(` is an interpolation: it survives as
                        // `\(` in the decoded text, and the parser is what
                        // splits it out — it needs the expression, not the
                        // lexer.
                        b'(' => {
                            text.push('\\');
                            text.push('(');
                            i += 2;
                        }
                        b'n' => {
                            text.push('\n');
                            i += 2;
                        }
                        b't' => {
                            text.push('\t');
                            i += 2;
                        }
                        b'"' => {
                            text.push('"');
                            i += 2;
                        }
                        other => {
                            text.push('\\');
                            text.push_char_fallback(other);
                            i += 2;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            _ => {
                let ch = body[i..].chars().next().unwrap_or(' ');
                text.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    (text, bytes.len())
}

/// Read `#RRGGBB` or `#AARRGGBB` at `start` (the `#`). Returns `None` when
/// the digits do not form a colour, with `next` past what was consumed.
fn read_hex(body: &str, start: usize) -> (Option<u32>, usize) {
    let rest = &body[start + 1..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    let value = match digits.len() {
        6 => u32::from_str_radix(&digits, 16)
            .ok()
            .map(|rgb| 0xFF00_0000 | rgb),
        8 => u32::from_str_radix(&digits, 16).ok(),
        _ => None,
    };
    (value, start + 1 + digits.len())
}

/// Read a numeric literal at `start`. Whole numbers stay whole; anything with
/// a `.` becomes a float. `16px` becomes the number `16` and a word `px`,
/// which the parser rejects with a position — better than swallowing the
/// unit silently.
fn read_number(body: &str, start: usize) -> (Tok, usize) {
    let rest = &body[start..];
    let mut end = 0usize;
    let bytes = rest.as_bytes();
    let mut seen_dot = false;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_digit() {
            end += 1;
        } else if b == b'.' && !seen_dot && end + 1 < bytes.len() && bytes[end + 1].is_ascii_digit()
        {
            seen_dot = true;
            end += 1;
        } else {
            break;
        }
    }
    let digits = &rest[..end];
    if seen_dot {
        (
            Tok::Float(digits.parse::<f32>().unwrap_or(0.0)),
            start + end,
        )
    } else {
        match digits.parse::<i64>() {
            Ok(value) => (Tok::Int(value), start + end),
            Err(_) => (Tok::Word(digits.to_owned()), start + end),
        }
    }
}

trait PushCharFallback {
    fn push_char_fallback(&mut self, c: u8);
}

impl PushCharFallback for String {
    fn push_char_fallback(&mut self, c: u8) {
        self.push(c as char);
    }
}
