//! The locale in force, published to a subtree — and the direction with it.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::Localizations;
//!
//! // Arabic, and a mirrored layout, from one statement.
//! let app = Localizations::new(Locale::parse("ar").unwrap()).child(Text::new("مرحبا"));
//! ```
//!
//! # Why this publishes two things
//!
//! A language implies a reading direction, so requiring an application to set
//! both is requiring it to keep two facts in step for ever — and they drift the
//! first time somebody adds a locale and forgets the other line. This publishes
//! the [`Locale`] *and* the [`TextDirection`] it implies, so switching to Arabic
//! mirrors the layout with nothing else said.
//!
//! A subtree that needs the other direction anyway — a code block, a phone
//! number — wraps itself in a [`Directionality`](crate::Directionality), which
//! shadows this one exactly as any inner provider shadows an outer. That
//! composes without this type needing an override of its own.
//!
//! # Where the messages are
//!
//! **Not here.** See [`vieww_foundation::Locale`] for why there is no message
//! catalogue in the framework: an application declares its own message type and
//! publishes it through [`Inherited<T>`](crate::Inherited), which already
//! carries application-defined types. This widget supplies the *locale* those
//! messages are chosen by, and [`Locale::plural`] the form a count takes.
//!
//! ```
//! use std::rc::Rc;
//! use vieww_widget::prelude::*;
//! use vieww_widget::{Inherited, Localizations};
//!
//! // The application's own messages, checked by the compiler.
//! #[derive(Debug)]
//! struct Messages {
//!     greeting: &'static str,
//! }
//!
//! let arabic = Locale::parse("ar").unwrap();
//! let app = Localizations::new(arabic).child(Inherited::shared(
//!     Rc::new(Messages { greeting: "مرحبا" }),
//!     Text::new("…"),
//! ));
//! ```

use std::rc::Rc;

use vieww_foundation::{Key, Locale, TextDirection};

use crate::{
    widget_node_from, BuildContext, CalendarNames, Inherited, Widget, WidgetKind, WidgetNode,
};

/// Publishes a [`Locale`], and the [`TextDirection`] it implies, to its subtree.
#[derive(Debug, Clone)]
pub struct Localizations {
    locale: Rc<Locale>,
    direction: Rc<TextDirection>,
    calendar: Rc<CalendarNames>,
    child: WidgetNode,
    key: Option<Key>,
}

impl Localizations {
    #[must_use]
    pub fn new(locale: Locale) -> Self {
        Self {
            locale: Rc::new(locale),
            direction: Rc::new(locale.text_direction()),
            calendar: Rc::new(CalendarNames::english()),
            child: crate::SizedBox::shrink().into(),
            key: None,
        }
    }

    /// The month and weekday names for this subtree.
    ///
    /// English by default, because a `Locale` in this framework does not carry
    /// calendar names and nothing here ships CLDR — see
    /// [`CalendarNames`]. An application that translates its own strings has
    /// this data already; one that does not is English regardless.
    #[must_use]
    pub fn calendar(mut self, names: CalendarNames) -> Self {
        self.calendar = Rc::new(names);
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = child.into();
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The locale in force at this position, or [`Locale::ENGLISH`].
    ///
    /// Never fails, for the reason [`Directionality::of`](crate::Directionality)
    /// and [`ThemeData::of`](crate::ThemeData) never fail: a widget built with
    /// nothing above it — in a test, in a tree dump, in the first five minutes
    /// of an application — should render in *some* language rather than panic.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> Locale {
        // Copied rather than handed back as an `Rc`: a `Locale` is five bytes
        // and `Copy`, and every caller immediately asks it a question.
        ctx.inherit::<Locale>()
            .map_or(Locale::ENGLISH, |locale| *locale)
    }

    /// The calendar names in force here, or [`CalendarNames::english`].
    ///
    /// Falls back for the same reason [`of`](Self::of) does: a calendar built
    /// with nothing above it should draw *some* month names rather than panic.
    #[must_use]
    pub fn calendar_of(ctx: &BuildContext) -> CalendarNames {
        // `Copy`, like `Locale`, and for the same reason — it is fourteen
        // pointers and every caller immediately reads from it.
        ctx.inherit::<CalendarNames>()
            .map_or_else(CalendarNames::english, |names| *names)
    }
}

impl Widget for Localizations {
    fn debug_name(&self) -> &'static str {
        "Localizations"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("locale", self.locale.to_string())]
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Direction inside the locale, so a `Directionality` written by the
        // application anywhere below shadows only the direction and leaves the
        // locale alone — which is what a pinned phone number wants.
        //
        // Calendar names innermost, so the same trick works for them: a screen
        // showing one month grid in another language wraps just that subtree.
        Inherited::shared(
            Rc::clone(&self.locale),
            Inherited::shared(
                Rc::clone(&self.direction),
                Inherited::shared(Rc::clone(&self.calendar), self.child.clone()),
            ),
        )
        .into()
    }
}

widget_node_from!(Localizations);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{debug_tree, Directionality, Text};

    #[test]
    fn a_build_with_nothing_above_it_is_english() {
        assert_eq!(
            Localizations::of(&BuildContext::root()),
            Locale::ENGLISH,
            "a tree with no locale renders in some language rather than panicking"
        );
    }

    #[test]
    fn a_locale_reaches_a_descendant() {
        let arabic = Locale::parse("ar-EG").unwrap();
        let ctx = BuildContext::root().child_with(Rc::new(arabic));
        assert_eq!(Localizations::of(&ctx), arabic);
    }

    #[test]
    fn an_inner_locale_shadows_an_outer_one() {
        let outer = BuildContext::root().child_with(Rc::new(Locale::parse("en").unwrap()));
        let inner = outer.child_with(Rc::new(Locale::parse("fr").unwrap()));
        assert_eq!(Localizations::of(&outer).language(), "en");
        assert_eq!(Localizations::of(&inner).language(), "fr");
    }

    #[test]
    fn setting_arabic_mirrors_the_layout_without_saying_so_twice() {
        // The whole reason this widget publishes two things. If the direction
        // ever stops riding along, every RTL screen silently lays out backwards
        // while the text still shapes correctly — the exact failure `rtl.rs`
        // exists to make visible.
        let dump =
            debug_tree(Localizations::new(Locale::parse("ar").unwrap()).child(Text::new("م")));
        assert!(dump.contains("Inherited<Locale>"), "{dump}");
        assert!(dump.contains("Inherited<TextDirection>"), "{dump}");
    }

    #[test]
    fn a_left_to_right_locale_publishes_that_direction_too() {
        // Not only the interesting case: an English subtree inside an Arabic
        // app has to put the direction *back*, and it does that by being a
        // `Localizations` of its own.
        let english = Locale::parse("en").unwrap();
        assert_eq!(english.text_direction(), TextDirection::Ltr);
        let dump = debug_tree(Localizations::new(english).child(Text::new("hi")));
        assert!(dump.contains("Inherited<TextDirection>"), "{dump}");
    }

    #[test]
    fn a_directionality_below_pins_the_direction_and_leaves_the_locale_alone() {
        // A phone number in an Arabic screen: still Arabic for the purpose of
        // choosing messages and plural forms, laid out left to right.
        let arabic = BuildContext::root().child_with(Rc::new(Locale::parse("ar").unwrap()));
        let pinned = arabic.child_with(Rc::new(TextDirection::Ltr));
        assert_eq!(Localizations::of(&pinned).language(), "ar");
        assert_eq!(Directionality::of(&pinned), TextDirection::Ltr);
    }

    #[test]
    fn the_dump_names_the_locale_it_was_given() {
        let dump = debug_tree(Localizations::new(Locale::parse("pt_BR").unwrap()));
        assert!(dump.contains("pt-BR"), "{dump}");
    }
}
