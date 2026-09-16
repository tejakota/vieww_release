//! Joining a list of things the way a language joins them.

use crate::Locale;

/// What a list means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListStyle {
    /// `a, b and c` — everything in the list.
    #[default]
    And,
    /// `a, b or c` — one of them.
    Or,
    /// `a, b, c` — no conjunction, for a list of tags or filters.
    Narrow,
}

/// The words a locale joins lists with.
///
/// Supplied rather than tabled, for the reason
/// [`CalendarNames`](super::CalendarNames) is: "and" is a word, words are
/// translations, and vieww does not carry a dictionary. What it carries is the
/// *punctuation* — where the commas go, whether the last one is there — which
/// is structural and which English alone has two answers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListWords {
    /// ` and `, with its spaces.
    pub and: &'static str,
    /// ` or `, with its spaces.
    pub or: &'static str,
}

impl ListWords {
    pub const ENGLISH: Self = Self {
        and: " and ",
        or: " or ",
    };
}

/// Join `items` the way `locale` would.
///
/// # The serial comma
///
/// `a, b and c` in British English and most of the world; `a, b, and c` in
/// American English. It is decided by region here rather than left to the
/// caller because a caller who has to decide will decide once, at one call
/// site, and the other nine will disagree with it.
///
/// ```
/// use vieww_foundation::intl::{format_list, ListStyle, ListWords};
/// use vieww_foundation::Locale;
///
/// let items = ["apples", "pears", "plums"];
/// let british = Locale::parse("en-GB").expect("a real tag");
/// let american = Locale::parse("en-US").expect("a real tag");
///
/// let words = ListWords::ENGLISH;
/// assert_eq!(format_list(british, &items, ListStyle::And, words), "apples, pears and plums");
/// assert_eq!(format_list(american, &items, ListStyle::And, words), "apples, pears, and plums");
/// ```
#[must_use]
pub fn format_list<S: AsRef<str>>(
    locale: Locale,
    items: &[S],
    style: ListStyle,
    words: ListWords,
) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_owned(),
        [first, last] => match style {
            ListStyle::Narrow => format!("{}, {}", first.as_ref(), last.as_ref()),
            ListStyle::And => format!("{}{}{}", first.as_ref(), words.and, last.as_ref()),
            ListStyle::Or => format!("{}{}{}", first.as_ref(), words.or, last.as_ref()),
        },
        [leading @ .., last] => {
            let body = leading
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", ");
            match style {
                ListStyle::Narrow => format!("{body}, {}", last.as_ref()),
                ListStyle::And | ListStyle::Or => {
                    let conjunction = if style == ListStyle::And {
                        words.and
                    } else {
                        words.or
                    };
                    let serial = if serial_comma(locale) { "," } else { "" };
                    format!("{body}{serial}{conjunction}{}", last.as_ref())
                }
            }
        }
    }
}

/// Whether this locale puts a comma before the final conjunction.
///
/// American English and Canadian English do; British English and everything
/// else in the coverage of this crate do not.
#[must_use]
pub fn serial_comma(locale: Locale) -> bool {
    matches!(
        (locale.language(), locale.region()),
        ("en", Some("US" | "CA"))
    )
}
