//! What language the machine is set to.
//!
//! # Why this is not in `vieww-foundation`
//!
//! [`Locale`] is, because a locale is a value. *Asking the operating system* for
//! one is a platform call, and `vieww-foundation` is deliberately free of those
//! — `insets.rs` is the same split, for the same reason: the foundation stays
//! testable and portable, and the platform crate is where the system is allowed
//! to be consulted.
//!
//! # Why the preference list rather than one answer
//!
//! Every desktop and both phones let a user rank languages, and the ranking is
//! the point: somebody who reads Welsh and English has said which they would
//! rather have *and* what to fall back to. An application that supports only
//! some of them wants the first one it can serve, not the first one the system
//! names — so [`preferred`] hands back the whole list in order and lets the
//! caller choose.
//!
//! ```no_run
//! # use vieww_platform_winit::locale;
//! # use vieww_foundation::Locale;
//! let supported = [Locale::ENGLISH, Locale::parse("ar").unwrap()];
//! let choice = locale::best_of(&supported).unwrap_or(Locale::ENGLISH);
//! ```

use vieww_foundation::Locale;

/// The language the system is set to, if it can be read and parsed.
///
/// `None` on a machine that reports nothing, or reports something this cannot
/// make sense of — a bare `C` or `POSIX` from a minimal Linux environment, most
/// often. That is not an error and should not be treated as one: the honest
/// response is the application's own default, which is why this returns an
/// `Option` rather than falling back to English on the caller's behalf.
#[must_use]
pub fn system() -> Option<Locale> {
    Locale::parse(&sys_locale::get_locale()?)
}

/// Every language the user has asked for, best first.
///
/// Tags the parser cannot make sense of are dropped rather than ending the
/// list, so one `C` in a `LANGUAGE` chain does not hide the real preferences
/// behind it.
#[must_use]
pub fn preferred() -> Vec<Locale> {
    sys_locale::get_locales()
        .filter_map(|tag| Locale::parse(&tag))
        .collect()
}

/// The first supported locale the user actually wants.
///
/// **The user's ranking is the outer loop.** Each language they asked for is
/// tried in turn, exactly first and then by language alone, before the next one
/// is considered at all.
///
/// That ordering is the whole design, and the alternative is tempting and
/// wrong: taking *every* exact match before *any* language match would hand
/// somebody who ranked `fr-CA` above `pt-BR` a Portuguese interface, because the
/// application happened to stock an exact `pt-BR` and only a general `fr`. They
/// said they wanted French. A shared language is nearly always more use than a
/// shared region, and a lower preference is never more use than a higher one.
///
/// So `en-GB` takes an application's `en` rather than nothing, and `pt-BR` still
/// prefers `pt-BR` over `pt-PT` when both are offered.
///
/// `None` when nothing overlaps, which is a real answer — the caller's default
/// is the right response and this cannot know what it is.
#[must_use]
pub fn best_of(supported: &[Locale]) -> Option<Locale> {
    best_among(&preferred(), supported)
}

/// [`best_of`] against a given preference list rather than the machine's.
///
/// Split out so the rule has a test at all: one that asserted what *this*
/// machine is set to would pass here and fail on a build server.
fn best_among(wanted: &[Locale], supported: &[Locale]) -> Option<Locale> {
    wanted.iter().find_map(|want| {
        supported
            .iter()
            .find(|have| *have == want)
            .or_else(|| {
                supported
                    .iter()
                    .find(|have| have.language() == want.language())
            })
            .copied()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale(tag: &str) -> Locale {
        Locale::parse(tag).expect("a test tag")
    }

    #[test]
    fn an_exact_match_wins() {
        let choice = best_among(&[locale("pt-BR")], &[locale("pt-PT"), locale("pt-BR")]);
        assert_eq!(choice, Some(locale("pt-BR")));
    }

    #[test]
    fn a_shared_language_beats_no_answer_at_all() {
        // `en-GB` against an application that ships `en`. Returning `None` here
        // would show a British user an application's default language while it
        // held one they read perfectly.
        let choice = best_among(&[locale("en-GB")], &[locale("en")]);
        assert_eq!(choice, Some(locale("en")));
    }

    #[test]
    fn the_users_order_decides_rather_than_the_applications() {
        // The application lists English first; the user asked for Arabic first
        // and can read both. The user wins — that is what a preference list is.
        let choice = best_among(&[locale("ar"), locale("en")], &[locale("en"), locale("ar")]);
        assert_eq!(choice, Some(locale("ar")));
    }

    #[test]
    fn a_higher_preference_wins_even_on_a_looser_match() {
        // The user prefers `fr-CA`, then `pt-BR`. The application has `fr` and
        // `pt-BR`. `fr` is only a *language* match; `pt-BR` is exact — and `fr`
        // still wins, because it answers what they asked for **first**.
        //
        // This test and the implementation disagreed at first, and the test was
        // right: taking every exact match before any language match would hand a
        // French speaker a Portuguese interface. The name said the opposite of
        // its own assertion, which is how it went unnoticed.
        let choice = best_among(
            &[locale("fr-CA"), locale("pt-BR")],
            &[locale("fr"), locale("pt-BR")],
        );
        assert_eq!(choice, Some(locale("fr")));
    }

    #[test]
    fn nothing_in_common_is_none_rather_than_a_guess() {
        assert_eq!(best_among(&[locale("ja")], &[locale("en")]), None);
        assert_eq!(best_among(&[], &[locale("en")]), None);
        assert_eq!(best_among(&[locale("en")], &[]), None);
    }

    #[test]
    fn whatever_this_machine_says_parses_or_is_honestly_none() {
        // Cannot assert *which* locale, since that is a property of the machine
        // this runs on. What it can assert is that the two functions agree and
        // that neither panics on whatever the system reports — including the
        // bare `C` a minimal container gives.
        if let Some(system) = system() {
            assert!(
                preferred().contains(&system),
                "the system locale should be in its own preference list"
            );
        }
    }
}
