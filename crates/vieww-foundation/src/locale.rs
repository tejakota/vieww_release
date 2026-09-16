//! Which language an interface is in, and what that implies.
//!
//! # What this is not
//!
//! **There is no message catalogue here, deliberately.** A framework-supplied
//! `catalogue.get("greeting")` is stringly typed: a missing key, a typo or a
//! renamed message are all runtime surprises in a language whose entire point is
//! catching that at compile time. An application declares its own messages —
//! a struct, a trait, whatever suits it — and publishes them through
//! [`Inherited<T>`](https://docs.rs/vieww-widget), which already carries
//! application-defined types and is proven by the extensibility sweep.
//!
//! So what a framework owes an application here is the part it *cannot*
//! reasonably write itself:
//!
//! - **which locale is in force**, ambient, reachable anywhere in the tree;
//! - **which plural form a number takes**, which is a linguistic table rather
//!   than a programming problem, and which is wrong in most hand-rolled
//!   implementations;
//! - **which way the interface reads**, because that follows from the language
//!   and nobody should have to say both.
//!
//! # The tie to reading direction
//!
//! [`Locale::text_direction`] is why setting a locale is enough. An application
//! that switches to Arabic gets a mirrored layout without also remembering to
//! set a [`TextDirection`], and the two can no longer disagree — which they
//! would, eventually, if both had to be set by hand.

use std::fmt;

use crate::TextDirection;

/// A language, and optionally a region.
///
/// Stored as fixed bytes rather than `String`s so this stays [`Copy`] and
/// allocation-free: a locale is read on every build that formats anything, and
/// it travels through the same inherited scope a [`TextDirection`] does, where
/// an allocation per lookup would be paid for by every screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Locale {
    /// ISO 639 code, two or three letters, zero-padded.
    language: [u8; 3],
    /// ISO 3166 region, or zeroes for "unspecified".
    region: [u8; 2],
}

impl Locale {
    /// `en`, and what an application gets before it says otherwise.
    pub const ENGLISH: Self = Self {
        language: [b'e', b'n', 0],
        region: [0, 0],
    };

    /// Parse a BCP-47-ish tag: `en`, `en-US`, `pt_BR`, `fil`.
    ///
    /// Both separators are accepted because both turn up in the wild — the
    /// hyphen from BCP 47 and web platforms, the underscore from POSIX
    /// `LANG` and from Java. Case is normalised, so `EN-us` and `en-US` are one
    /// locale rather than two that never compare equal.
    ///
    /// `None` for anything that is not a plausible tag, rather than a silent
    /// fallback to English: a typo in a locale should be visible where it is
    /// written, not at the far end as an interface in the wrong language.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let mut parts = tag.split(['-', '_']);
        let language = parts.next()?;
        if !matches!(language.len(), 2 | 3) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
            return None;
        }

        let mut code = [0u8; 3];
        for (slot, byte) in code.iter_mut().zip(language.bytes()) {
            *slot = byte.to_ascii_lowercase();
        }

        // Anything after the language that is two letters is the region; a
        // four-letter subtag is a script (`zh-Hant`) and everything else is a
        // variant. Both are kept out rather than guessed at — this type does not
        // model them, and pretending a script is a region would make `zh-Hant`
        // and `zh-HK` collide.
        let region = parts
            .find(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_alphabetic()))
            .map_or([0, 0], |part| {
                let bytes = part.as_bytes();
                [bytes[0].to_ascii_uppercase(), bytes[1].to_ascii_uppercase()]
            });

        Some(Self {
            language: code,
            region,
        })
    }

    /// The language code, lowercase.
    #[must_use]
    pub fn language(&self) -> &str {
        let end = self.language.iter().position(|&b| b == 0).unwrap_or(3);
        // Written by `parse` from ASCII alphabetic bytes only.
        std::str::from_utf8(&self.language[..end]).unwrap_or("")
    }

    /// The region code, uppercase, if one was given.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        if self.region[0] == 0 {
            return None;
        }
        std::str::from_utf8(&self.region).ok()
    }

    /// Which way an interface in this language reads.
    ///
    /// The list is the right-to-left scripts in current use, by language rather
    /// than by script, because that is what a locale carries. `ckb` (Sorani
    /// Kurdish) and `ug` (Uyghur) are included and are the two most often
    /// forgotten.
    ///
    /// `iw` and `ji` are the superseded codes for Hebrew and Yiddish. They are
    /// still emitted by older platforms and by the JVM, so accepting them costs
    /// two entries and saves a locale silently laying out the wrong way.
    #[must_use]
    pub fn text_direction(&self) -> TextDirection {
        match self.language() {
            "ar" | "he" | "iw" | "fa" | "ur" | "ps" | "sd" | "yi" | "ji" | "dv" | "ug" | "ckb" => {
                TextDirection::Rtl
            }
            _ => TextDirection::Ltr,
        }
    }

    /// Which plural form `count` takes in this language.
    ///
    /// See [`PluralCategory`] for what the categories mean and for exactly which
    /// languages are covered.
    #[must_use]
    pub fn plural(&self, count: u64) -> PluralCategory {
        PluralCategory::of(self.language(), count)
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::ENGLISH
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.region() {
            Some(region) => write!(f, "{}-{region}", self.language()),
            None => f.write_str(self.language()),
        }
    }
}

/// The plural form a number takes, in CLDR's categories.
///
/// # Why six, when English has two
///
/// Because the interface is not only in English. Arabic uses all six, Russian
/// four, Polish three — and a message written as "one or many" cannot be
/// translated into them at all without changing the code that chose it. Naming
/// the full set means an application's message type can carry the forms its
/// translators actually need.
///
/// [`Other`](Self::Other) is the form every language has and the one a message
/// must always supply.
///
/// # Coverage, stated rather than implied
///
/// Rules are implemented for the language families below. **Anything else falls
/// back to the one/other shape**, which is the commonest and is a *guess* — it
/// is right for most European languages and wrong for Japanese, which has no
/// plural, and for Arabic, which has six.
///
/// | shape | languages |
/// |---|---|
/// | no plural — always `Other` | ja, zh, ko, th, vi, id, ms, lo, my, km |
/// | one at 0 and 1 | fr, hi, am, bn, gu, kn, mr, fa, zu |
/// | one/few/many, Slavic | ru, uk, be, sr, hr, bs |
/// | one/few/many, Polish | pl |
/// | one/few, Czech | cs, sk |
/// | all six | ar |
/// | one/other *(also the fallback)* | en, de, es, it, nl, sv, and the rest |
///
/// Integers only. CLDR distinguishes `1.0` from `1` in some languages; a count
/// is a count here, and pretending otherwise would be a decimal API that is
/// wrong in a different way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    /// The category `count` takes in `language`.
    fn of(language: &str, count: u64) -> Self {
        match language {
            // No plural distinction at all. A message here has one form, and
            // offering it six would invite five identical translations.
            "ja" | "zh" | "ko" | "th" | "vi" | "id" | "ms" | "lo" | "my" | "km" => Self::Other,

            // Zero counts as singular: "0 heure", not "0 heures".
            "fr" | "hi" | "am" | "bn" | "gu" | "kn" | "mr" | "fa" | "zu" => {
                if count <= 1 {
                    Self::One
                } else {
                    Self::Other
                }
            }

            "ru" | "uk" | "be" | "sr" | "hr" | "bs" => Self::slavic(count),
            "pl" => Self::polish(count),
            "cs" | "sk" => match count {
                1 => Self::One,
                2..=4 => Self::Few,
                _ => Self::Other,
            },
            "ar" => Self::arabic(count),

            // en, de, es, it, nl, sv … and every language not named above.
            _ => {
                if count == 1 {
                    Self::One
                } else {
                    Self::Other
                }
            }
        }
    }

    /// Russian, Ukrainian, Belarusian, Serbian, Croatian, Bosnian.
    ///
    /// The teens are the trap: 11 is `Many` though it ends in 1, and 12–14 are
    /// `Many` though they end in 2–4. Every hand-rolled implementation that gets
    /// this wrong gets it wrong there.
    fn slavic(count: u64) -> Self {
        let unit = count % 10;
        let teen = count % 100;
        if unit == 1 && teen != 11 {
            Self::One
        } else if (2..=4).contains(&unit) && !(12..=14).contains(&teen) {
            Self::Few
        } else {
            Self::Many
        }
    }

    /// Polish, which differs from its Slavic neighbours at exactly one point:
    /// **only bare 1 is `One`**. 21 and 101 end in 1 and are `Many`, where
    /// Russian calls them `One` — *21 książek* takes the same form as *5
    /// książek*, while Russian's *21 книга* takes the singular.
    fn polish(count: u64) -> Self {
        let unit = count % 10;
        let teen = count % 100;
        if count == 1 {
            Self::One
        } else if (2..=4).contains(&unit) && !(12..=14).contains(&teen) {
            Self::Few
        } else {
            Self::Many
        }
    }

    /// Arabic, the language this whole enum is shaped for.
    ///
    /// It is the only one here using all six, and it is why `Zero` and `Two`
    /// exist at all — a message type offering only one/other cannot be
    /// translated into it.
    fn arabic(count: u64) -> Self {
        let hundred = count % 100;
        match count {
            0 => Self::Zero,
            1 => Self::One,
            2 => Self::Two,
            _ if (3..=10).contains(&hundred) => Self::Few,
            _ if (11..=99).contains(&hundred) => Self::Many,
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_language_and_a_full_tag_both_parse() {
        let bare = Locale::parse("en").expect("a bare language is a locale");
        assert_eq!(bare.language(), "en");
        assert_eq!(bare.region(), None);
        assert_eq!(bare.to_string(), "en");

        let full = Locale::parse("en-US").expect("a language and region");
        assert_eq!(full.language(), "en");
        assert_eq!(full.region(), Some("US"));
        assert_eq!(full.to_string(), "en-US");
    }

    #[test]
    fn case_and_separator_do_not_make_two_locales_out_of_one() {
        // `pt_BR` comes from POSIX and the JVM, `pt-BR` from the web. A type
        // where those two never compare equal is one that silently keeps two
        // translations of the same language.
        assert_eq!(Locale::parse("PT_br"), Locale::parse("pt-BR"));
        assert_eq!(Locale::parse("EN"), Locale::parse("en"));
    }

    #[test]
    fn a_three_letter_language_is_a_language() {
        let filipino = Locale::parse("fil-PH").expect("ISO 639-2 is three letters");
        assert_eq!(filipino.language(), "fil");
        assert_eq!(filipino.region(), Some("PH"));
    }

    #[test]
    fn a_script_is_not_mistaken_for_a_region() {
        // `zh-Hant` is traditional Chinese with no region. Reading `Hant` as one
        // would make it collide with `zh-HK`, and two scripts would share a
        // translation.
        let script = Locale::parse("zh-Hant").expect("a script subtag is legal");
        assert_eq!(script.language(), "zh");
        assert_eq!(script.region(), None);

        let both = Locale::parse("zh-Hant-HK").expect("script and region together");
        assert_eq!(both.region(), Some("HK"));
    }

    #[test]
    fn nonsense_is_rejected_rather_than_quietly_becoming_english() {
        // A typo should be visible where it is written. Falling back here would
        // surface as an interface in the wrong language, a long way from the
        // line that caused it.
        assert_eq!(Locale::parse(""), None);
        assert_eq!(Locale::parse("e"), None);
        assert_eq!(Locale::parse("engl"), None);
        assert_eq!(Locale::parse("12"), None);
    }

    #[test]
    fn a_language_carries_its_reading_direction() {
        // The tie that means an application sets one thing rather than two, and
        // that the two can never disagree.
        for tag in ["ar", "he", "fa", "ur", "ckb", "ug", "iw"] {
            let locale = Locale::parse(tag).expect("a known RTL language");
            assert_eq!(
                locale.text_direction(),
                TextDirection::Rtl,
                "{tag} reads right to left"
            );
        }
        for tag in ["en", "de", "ja", "hi", "ru"] {
            let locale = Locale::parse(tag).expect("a known LTR language");
            assert_eq!(locale.text_direction(), TextDirection::Ltr, "{tag}");
        }
    }

    #[test]
    fn a_region_does_not_change_the_direction() {
        assert_eq!(
            Locale::parse("ar-EG").unwrap().text_direction(),
            TextDirection::Rtl
        );
    }

    /// The category `tag` gives `count`.
    fn plural(tag: &str, count: u64) -> PluralCategory {
        Locale::parse(tag).expect("a test locale").plural(count)
    }

    #[test]
    fn english_is_one_and_other_and_zero_is_plural() {
        assert_eq!(plural("en", 0), PluralCategory::Other, "zero items");
        assert_eq!(plural("en", 1), PluralCategory::One);
        assert_eq!(plural("en", 2), PluralCategory::Other);
    }

    #[test]
    fn french_counts_zero_as_singular() {
        // "0 heure", not "0 heures" — the difference from English, and the whole
        // reason these two are separate shapes.
        assert_eq!(plural("fr", 0), PluralCategory::One);
        assert_eq!(plural("fr", 1), PluralCategory::One);
        assert_eq!(plural("fr", 2), PluralCategory::Other);
    }

    #[test]
    fn a_language_with_no_plural_never_reports_one() {
        for count in [0, 1, 2, 5, 100] {
            assert_eq!(plural("ja", count), PluralCategory::Other, "{count}");
        }
    }

    #[test]
    fn the_slavic_teens_are_the_trap_and_they_are_handled() {
        // 1, 21, 31 are One; **11 is not**, though it ends in 1.
        assert_eq!(plural("ru", 1), PluralCategory::One);
        assert_eq!(plural("ru", 21), PluralCategory::One);
        assert_eq!(plural("ru", 11), PluralCategory::Many, "eleven is not one");

        // 2-4, 22-24 are Few; **12-14 are not**, though they end in 2-4.
        assert_eq!(plural("ru", 2), PluralCategory::Few);
        assert_eq!(plural("ru", 23), PluralCategory::Few);
        assert_eq!(
            plural("ru", 13),
            PluralCategory::Many,
            "thirteen is not few"
        );

        assert_eq!(plural("ru", 5), PluralCategory::Many);
        assert_eq!(plural("ru", 0), PluralCategory::Many);
    }

    #[test]
    fn polish_differs_from_its_neighbours_at_exactly_one_point() {
        // 1 is One in both; **21 is One in Russian and Many in Polish**. That one
        // difference is why it is not folded into `slavic`.
        //
        // This assertion was written the wrong way round first — it expected
        // `Few` — and the implementation was right. Kept as a note because the
        // mistake is the natural one: 21 *looks* like it should behave as 2-4
        // do, and it does not, because Polish keys `few` off the last digit
        // being 2-4 and 21's is 1.
        assert_eq!(plural("pl", 1), PluralCategory::One);
        assert_eq!(plural("ru", 21), PluralCategory::One);
        assert_eq!(plural("pl", 21), PluralCategory::Many);
        assert_eq!(plural("pl", 101), PluralCategory::Many);

        // And where they agree, so the split is not overstated.
        assert_eq!(plural("pl", 2), PluralCategory::Few);
        assert_eq!(plural("ru", 2), PluralCategory::Few);
        assert_eq!(plural("pl", 22), PluralCategory::Few);
        assert_eq!(plural("ru", 22), PluralCategory::Few);
    }

    #[test]
    fn arabic_uses_all_six_which_is_why_there_are_six() {
        assert_eq!(plural("ar", 0), PluralCategory::Zero);
        assert_eq!(plural("ar", 1), PluralCategory::One);
        assert_eq!(plural("ar", 2), PluralCategory::Two);
        assert_eq!(plural("ar", 3), PluralCategory::Few);
        assert_eq!(plural("ar", 10), PluralCategory::Few);
        assert_eq!(plural("ar", 11), PluralCategory::Many);
        assert_eq!(plural("ar", 99), PluralCategory::Many);
        assert_eq!(plural("ar", 100), PluralCategory::Other);
        assert_eq!(plural("ar", 102), PluralCategory::Other);
        assert_eq!(plural("ar", 103), PluralCategory::Few, "103 % 100 == 3");
    }

    #[test]
    fn an_unknown_language_guesses_the_commonest_shape() {
        // Documented as a guess rather than a rule. It is right for most of
        // Europe and wrong for Japanese and Arabic, which is why both of those
        // are named explicitly above.
        assert_eq!(plural("xx", 1), PluralCategory::One);
        assert_eq!(plural("xx", 2), PluralCategory::Other);
    }

    #[test]
    fn the_default_locale_is_english_reading_left_to_right() {
        let locale = Locale::default();
        assert_eq!(locale, Locale::ENGLISH);
        assert_eq!(locale.text_direction(), TextDirection::Ltr);
        assert_eq!(locale.to_string(), "en");
    }
}
