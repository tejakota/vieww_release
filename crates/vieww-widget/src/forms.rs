//! Input that is parsed once, into the type the application wanted.
//!
//! # The bug this makes unwritable
//!
//! The classic form field takes `validator: (String?) => String?` — a function
//! that looks at a string and returns an error message or `null`. The field's
//! value stays a `String`, so the application parses it *again* later to get
//! the `int`, the `DateTime`, the `Email` it actually wanted.
//!
//! Two parses, in two places, that can disagree. The validator says a field is
//! fine; the second parse, written weeks later against a slightly different
//! rule, throws. And nothing stops a form being submitted with fields the
//! validator never ran on, because "was it validated" is bookkeeping the
//! framework keeps rather than something the types know.
//!
//! **Here the parse is the validation.** [`FromInput`] turns the raw text into
//! `T` or into an error, [`FormField<T>`] runs it, and the submit handler is
//! `Fn(T)` — it receives the parsed value and there is no `String` in its
//! signature. A handler that runs on unparsed input is not a bug that can be
//! introduced by forgetting something; it cannot be written at all.
//!
//! This is "parse, don't validate", which is not a new idea — it is simply not
//! available to a framework whose form values are all strings.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::forms::{FormField, FromInput};
//!
//! struct Age(u8);
//!
//! impl FromInput for Age {
//!     type Error = &'static str;
//!
//!     fn from_input(raw: &str) -> Result<Self, Self::Error> {
//!         let years: u8 = raw.trim().parse().map_err(|_| "whole years, please")?;
//!         if years >= 18 {
//!             Ok(Self(years))
//!         } else {
//!             Err("must be 18 or over")
//!         }
//!     }
//! }
//!
//! // `accept` is handed an `Age`. There is no path that hands it a string.
//! let field = FormField::<Age>::new(TextEditingValue::from("21"))
//!     .on_accepted(|age| println!("{} years", age.0));
//! ```
//!
//! # Aggregating several fields into one submit
//!
//! [`FormGroup`] is "these four, together, or none," and it needs no macro at
//! any call site. It closes three pieces of bookkeeping that repeat across
//! every multi-field form:
//!
//! - whether every field currently parses ([`ready`](FormGroup::ready)) — one
//!   `&&` chain, written once, that a fifth field cannot silently stop covering
//!   the way a hand-copied one can;
//! - whether a submit was already attempted and failed — one flag, applied to
//!   every field's [`eager`](FormField::eager) at once rather than threaded into
//!   each field by hand;
//! - and **what the parsed values actually are**
//!   ([`all_of`](FormGroup::all_of)).
//!
//! That third one is the load-bearing part, and it was missing for a session.
//! `ready` answers a *question* and throws the values away, so the handler it
//! gates had to parse the same text a second time — reopening, at the last step,
//! precisely the two-parse hole this module exists to close. `all_of` turns a
//! tuple of `Result`s inside out, so the handler closes over an `Age` and a
//! `Name` instead of over two strings and a promise.
//!
//! ```
//! use vieww_widget::prelude::*;
//! use vieww_widget::forms::{FormField, FormGroup, FromInput};
//!
//! struct Age(u8);
//! impl FromInput for Age {
//!     type Error = &'static str;
//!     fn from_input(raw: &str) -> Result<Self, Self::Error> {
//!         raw.trim().parse::<u8>().map(Self).map_err(|_| "whole years, please")
//!     }
//! }
//!
//! struct Name(String);
//! impl FromInput for Name {
//!     type Error = &'static str;
//!     fn from_input(raw: &str) -> Result<Self, Self::Error> {
//!         (!raw.trim().is_empty())
//!             .then(|| Self(raw.trim().to_owned()))
//!             .ok_or("a name is required")
//!     }
//! }
//!
//! # let attempted_before = false;
//! let group = FormGroup::new(attempted_before);
//! let age = group.field(FormField::<Age>::new(TextEditingValue::from("21")));
//! let name = group.field(FormField::<Name>::new(TextEditingValue::from("Ada")));
//!
//! // One parse, and the handler is built out of what it produced.
//! let mut submit = Button::new("Sign up");
//! if let Some((age, name)) = FormGroup::all_of((age.parse(), name.parse())) {
//!     submit = submit.on_pressed(move || {
//!         // `age` is an `Age` and `name` is a `Name`, parsed at the moment the
//!         // button was decided to be live. Nothing here can disagree with that
//!         // decision, because nothing here parses anything.
//!         let _ = (age.0, &name.0);
//!     });
//! }
//! // No values means no handler — the button is disabled, same as every other
//! // control in this module when it has nothing to do.
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use vieww_foundation::{Key, TextEditingValue};

use crate::{
    BuildContext, CrossAxisAlignment, Flex, Handler, MainAxisSize, SizedBox, Text, TextField,
    ThemeData, Widget, WidgetKind, WidgetNode,
};

/// A type that can be read out of what someone typed.
///
/// The implementation is the validation. There is no second function that
/// checks whether the parse *would* succeed, because that is the pair that
/// drifts apart.
pub trait FromInput: Sized {
    /// What to say when the text is not this type. Rendered under the field, so
    /// it is read by a person — "must be 18 or over", not "ParseIntError".
    type Error: fmt::Display;

    /// Parse, or explain why not.
    ///
    /// Called on every keystroke, so it should be cheap. It is also called
    /// again before the value is accepted, which costs one parse of a short
    /// string and removes any question of the two answers differing.
    fn from_input(raw: &str) -> Result<Self, Self::Error>;
}

/// A text field that yields a `T`, or explains why it cannot.
///
/// Controlled exactly like [`TextField`]: it holds no text of its own, reports
/// edits through [`on_changed`](Self::on_changed), and the caller applies them.
/// What it adds is that [`on_accepted`](Self::on_accepted) is `Fn(T)`.
///
/// # The error is shown after the field has been left, not while typing
///
/// Telling somebody their email address is invalid while they are three
/// characters into typing it is technically true and useless. The message
/// appears when the field is submitted, and stays until the text parses. Pass
/// [`eager`](Self::eager) for the cases where that is wrong — a numeric field
/// with a live total below it, say.
pub struct FormField<T: FromInput> {
    value: TextEditingValue,
    on_changed: Option<Handler<TextEditingValue>>,
    on_accepted: Option<Rc<dyn Fn(T)>>,
    label: Option<String>,
    eager: bool,
    key: Option<Key>,
    /// `T` appears only in the handler and the parse, so it needs naming here.
    parsed: PhantomData<fn() -> T>,
}

impl<T: FromInput> FormField<T> {
    #[must_use]
    pub fn new(value: TextEditingValue) -> Self {
        Self {
            value,
            on_changed: None,
            on_accepted: None,
            label: None,
            eager: false,
            key: None,
            parsed: PhantomData,
        }
    }

    /// Every edit, for the caller to apply. Without this the field is read-only,
    /// which is [`TextField`]'s rule and not a separate one here.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<TextEditingValue>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// Called with the **parsed** value when the field is submitted and the text
    /// is a `T`.
    ///
    /// Not called at all when it is not. That is the whole point: there is no
    /// arm of this that hands over a string and leaves the parsing to the
    /// caller, so the caller cannot skip it.
    #[must_use]
    pub fn on_accepted(mut self, handler: impl Fn(T) + 'static) -> Self {
        self.on_accepted = Some(Rc::new(handler));
        self
    }

    /// A name for the field, read out before its contents.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Show the error as soon as the text stops parsing, rather than on submit.
    #[must_use]
    pub const fn eager(mut self) -> Self {
        self.eager = true;
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// What the current text parses to, for a caller that wants to ask.
    ///
    /// A convenience over `T::from_input`, not a second source of truth — a
    /// submit re-parses rather than trusting anything cached, so this cannot
    /// disagree with what `on_accepted` sees.
    pub fn parse(&self) -> Result<T, T::Error> {
        T::from_input(&self.value.text)
    }
}

impl<T: FromInput + 'static> Widget for FormField<T> {
    fn debug_name(&self) -> &'static str {
        "FormField"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let mut field = TextField::new(self.value.clone()).single_line();
        if let Some(handler) = &self.on_changed {
            field = field.on_changed(Rc::clone(handler));
        }
        if let Some(accepted) = &self.on_accepted {
            let accepted = Rc::clone(accepted);
            // **The submit path re-parses.** It could carry the result of the
            // parse `build` already did, and that would be one parse of a short
            // string cheaper and one more place for the two answers to differ.
            // The whole module exists to remove that class of difference.
            field = field.on_submit(Rc::new(move |text: String| {
                if let Ok(value) = T::from_input(&text) {
                    accepted(value);
                }
            }));
        }

        // Shown when the text does not parse *and* the field is eager. On
        // submit the error is already on screen for an eager field and appears
        // on the rebuild for a lazy one, because a failed submit leaves the
        // text alone and the caller rebuilds with it.
        let message = match (self.eager, self.parse()) {
            (true, Err(error)) => Some(error.to_string()),
            _ => None,
        };

        let mut column = Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(CrossAxisAlignment::Start);

        if let Some(label) = &self.label {
            column = column.push(Text::new(label.clone()).style(theme.text.label));
            column = column.push(SizedBox::height(theme.metrics.gap / 2.0));
        }

        column = column.push(field);

        if let Some(message) = message {
            column = column.push(SizedBox::height(theme.metrics.gap / 2.0));
            column = column.push(
                Text::new(message)
                    .style(theme.text.label)
                    .color(theme.colors.error),
            );
        }

        column.into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("valid", self.parse().is_ok().to_string()),
            ("eager", self.eager.to_string()),
        ]
    }
}

impl<T: FromInput> fmt::Debug for FormField<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FormField")
            .field("text", &self.value.text)
            .field("valid", &self.parse().is_ok())
            .finish_non_exhaustive()
    }
}

// Written out rather than `widget_node_from!`, which takes a plain type and has
// no room for a parameter or a bound. One generic widget does not justify
// teaching the macro generics.
impl<T: FromInput + 'static> From<FormField<T>> for WidgetNode {
    fn from(widget: FormField<T>) -> Self {
        WidgetNode::new(widget)
    }
}

/// Ties several [`FormField`]s to one submit action — the piece the module
/// doc above walks through: "these four, together, or none," with the
/// heterogeneous values never needing to be collected into anything, and
/// with none of it behind a macro.
///
/// # Why this is data, not a widget
///
/// A widget owning the "was a submit already attempted" flag would be the
/// one control in this crate managing its own durable state — exactly what
/// this module's own rule (mirrored from every other control here) rules
/// out. [`FormGroup`] is instead a plain value the caller constructs fresh
/// from whatever it already holds in a `Signal<bool>`, the same shape
/// [`ThemeData::of`] reads and nothing more.
#[derive(Debug, Clone, Copy)]
pub struct FormGroup {
    attempted: bool,
}

impl FormGroup {
    /// `attempted` is `true` once the application has tried to submit at
    /// least once and it did not go through — ordinarily a `Signal<bool>`
    /// the application owns for exactly this and sets in the handler that
    /// runs when [`ready`](Self::ready) says no.
    #[must_use]
    pub const fn new(attempted: bool) -> Self {
        Self { attempted }
    }

    /// `field`, made to show its error immediately if the group has already
    /// had a failed submit.
    ///
    /// Without this, a field the user never touched stays silent even after
    /// a failed submit tried to include it — correct for the first look at a
    /// form, wrong the moment "Submit" has already been pressed once: at that
    /// point every invalid field should say why, not just the one the user
    /// happens to be looking at. Applying [`eager`](FormField::eager) here
    /// once, from one flag, is what makes that automatic instead of asking
    /// every call site to thread `attempted` into every field by hand.
    #[must_use]
    pub fn field<T: FromInput + 'static>(&self, field: FormField<T>) -> FormField<T> {
        if self.attempted {
            field.eager()
        } else {
            field
        }
    }

    /// `true` only if every one of `valid` is — "these fields, together, or
    /// none," decided in the one place a fifth field cannot silently stop
    /// covering the way a hand-copied `a.is_ok() && b.is_ok() && c.is_ok()`
    /// can.
    ///
    /// Takes `&[bool]` rather than a slice of `FormField<T>` or of
    /// `Result<T, E>`, deliberately: the fields being aggregated do not share
    /// one `T`, and Rust has no way to put an `Age` and a `Name` in the same
    /// `Vec` without a trait object neither side needs. `field.parse().is_ok()`
    /// is the one bit of information this actually needs from each field, so
    /// that is all it asks for.
    #[must_use]
    pub fn ready(valid: &[bool]) -> bool {
        valid.iter().all(|ok| *ok)
    }

    /// The same question as [`ready`](Self::ready), answered with the **values**
    /// instead of a verdict: `Some` of every parsed field, or `None`.
    ///
    /// # The hole this closes was in this module's own documentation
    ///
    /// `ready` answers "may the button be pressed" and throws the parsed values
    /// away, so every caller had to write `T::from_input` a second time inside
    /// the submit handler — against text it had already parsed in order to
    /// decide the button was live. That is the two-parse defect this whole
    /// module exists to make unwritable, reappearing at the last step. And the
    /// two parses run at *different moments*: a field edited between them is
    /// submitted with a value nobody validated.
    ///
    /// The doc comment above `ready` used to end with a worked example whose
    /// handler said, literally, `/* T::from_input each field here */`.
    ///
    /// # Why a tuple
    ///
    /// The fields deliberately do not share a `T` — that is the point of
    /// [`FormField<T>`] — so a `Vec` of them needs a trait object neither side
    /// wants. A tuple is the language's own heterogeneous list, `if let
    /// Some((name, age)) = ...` destructures it at the call site, and the arity
    /// is the compiler's problem rather than a runtime length check.
    ///
    /// The `macro_rules!` behind [`AllParsed`] writes the implementations and
    /// is never spelled by a caller, which is what `docs/AIMS.md` §H asked for.
    ///
    /// # Errors are deliberately not aggregated
    ///
    /// Each field already renders its own underneath itself, which is where
    /// somebody can act on it. Collecting them needs one error type across
    /// fields that have deliberately different ones, and produces a second place
    /// saying what is wrong — which is how a form ends up reporting "3 problems"
    /// above three fields that each already say what theirs is.
    ///
    /// ```
    /// use vieww_widget::forms::{FormGroup, FromInput};
    ///
    /// struct Age(u8);
    /// # impl FromInput for Age {
    /// #     type Error = &'static str;
    /// #     fn from_input(raw: &str) -> Result<Self, Self::Error> {
    /// #         raw.parse().map(Age).map_err(|_| "whole years, please")
    /// #     }
    /// # }
    /// struct Name(String);
    /// # impl FromInput for Name {
    /// #     type Error = &'static str;
    /// #     fn from_input(raw: &str) -> Result<Self, Self::Error> {
    /// #         if raw.is_empty() { Err("required") } else { Ok(Name(raw.into())) }
    /// #     }
    /// # }
    ///
    /// let parsed = (Name::from_input("Ada"), Age::from_input("36"));
    /// if let Some((name, age)) = FormGroup::all_of(parsed) {
    ///     // `name` is a `Name` and `age` is an `Age`. No second parse, and no
    ///     // `String` in sight.
    ///     assert_eq!(age.0, 36);
    ///     assert_eq!(name.0, "Ada");
    /// }
    /// ```
    pub fn all_of<T: AllParsed>(results: T) -> Option<T::Values> {
        results.all_parsed()
    }
}

/// A tuple of [`Result`]s that can be turned inside out into one `Option` of a
/// tuple of values.
///
/// The trait behind [`FormGroup::all_of`]. Implemented for tuples of one to
/// eight by a macro, because that is the range a form of hand-written fields
/// actually occupies — a ninth field is a screen that wants sections, and by
/// then the aggregate is per section.
///
/// Not `Result<Values, E>`, because the errors have deliberately different
/// types; see [`FormGroup::all_of`] for why they are not collected.
pub trait AllParsed {
    /// The same tuple with every `Result` unwrapped.
    type Values;

    /// Every value, or nothing.
    fn all_parsed(self) -> Option<Self::Values>;
}

/// One impl per arity. The error type of each field is named alongside its
/// value type rather than derived from it, because deriving one identifier from
/// another inside a macro is still unstable and an explicit pair costs one
/// token.
macro_rules! all_parsed_for_tuples {
    ($( ($($value:ident : $error:ident),+) ),+ $(,)?) => {
        $(
            #[allow(non_snake_case)]
            impl<$($value, $error,)+> AllParsed for ($(Result<$value, $error>,)+) {
                type Values = ($($value,)+);

                fn all_parsed(self) -> Option<Self::Values> {
                    let ($($value,)+) = self;
                    // `?` on the first `Err` — the whole group fails, and the
                    // fields after it are not even looked at, which is the
                    // "together, or none" this exists for.
                    Some(($($value.ok()?,)+))
                }
            }
        )+
    };
}

all_parsed_for_tuples! {
    (A: Ae),
    (A: Ae, B: Be),
    (A: Ae, B: Be, C: Ce),
    (A: Ae, B: Be, C: Ce, D: De),
    (A: Ae, B: Be, C: Ce, D: De, E: Ee),
    (A: Ae, B: Be, C: Ce, D: De, E: Ee, F: Fe),
    (A: Ae, B: Be, C: Ce, D: De, E: Ee, F: Fe, G: Ge),
    (A: Ae, B: Be, C: Ce, D: De, E: Ee, F: Fe, G: Ge, H: He),
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::{inflate, Theme};

    use super::*;

    /// Eighteen or over, and a whole number of years.
    #[derive(Debug, PartialEq)]
    struct Age(u8);

    impl FromInput for Age {
        type Error = &'static str;

        fn from_input(raw: &str) -> Result<Self, Self::Error> {
            let years: u8 = raw.trim().parse().map_err(|_| "whole years, please")?;
            if years >= 18 {
                Ok(Self(years))
            } else {
                Err("must be 18 or over")
            }
        }
    }

    fn built(widget: impl Into<WidgetNode>) -> crate::DebugNode {
        inflate(Theme::new(ThemeData::light()).child(widget.into()))
    }

    #[test]
    fn a_parsed_value_is_what_the_handler_receives() {
        let seen: Rc<RefCell<Option<Age>>> = Rc::new(RefCell::new(None));
        let record = Rc::clone(&seen);

        let field = FormField::<Age>::new(TextEditingValue::from("21"))
            .on_accepted(move |age| *record.borrow_mut() = Some(age));

        // Straight through the parse the widget would run on submit.
        assert_eq!(field.parse().ok(), Some(Age(21)));

        // And the handler's argument is an `Age`, not a string — which is the
        // claim, and is checked by this compiling at all.
        if let Ok(age) = field.parse() {
            if let Some(handler) = &field.on_accepted {
                handler(age);
            }
        }
        assert_eq!(*seen.borrow(), Some(Age(21)));
    }

    #[test]
    fn text_that_does_not_parse_yields_the_types_own_message() {
        let field = FormField::<Age>::new(TextEditingValue::from("twelve"));
        assert_eq!(field.parse().err(), Some("whole years, please"));

        let field = FormField::<Age>::new(TextEditingValue::from("12"));
        assert_eq!(
            field.parse().err(),
            Some("must be 18 or over"),
            "parsing and the rule are the same pass, so a number that is a \
             number but not an age fails here rather than downstream"
        );
    }

    #[test]
    fn an_eager_field_shows_the_error_and_a_lazy_one_does_not() {
        let eager = built(FormField::<Age>::new(TextEditingValue::from("12")).eager());
        let labels: Vec<String> = eager
            .find_all("Text")
            .iter()
            // `"text"`, and the value is `{:?}` of the string — so it arrives
            // quoted and these are `contains` rather than `==`.
            .filter_map(|node| node.property("text").map(str::to_owned))
            .collect();
        assert!(
            labels
                .iter()
                .any(|text| text.contains("must be 18 or over")),
            "an eager field explains itself while typing: {labels:?}"
        );

        let lazy = built(FormField::<Age>::new(TextEditingValue::from("12")));
        let labels: Vec<String> = lazy
            .find_all("Text")
            .iter()
            // `"text"`, and the value is `{:?}` of the string — so it arrives
            // quoted and these are `contains` rather than `==`.
            .filter_map(|node| node.property("text").map(str::to_owned))
            .collect();
        assert!(
            !labels
                .iter()
                .any(|text| text.contains("must be 18 or over")),
            "telling somebody they are wrong three characters in is useless: \
             {labels:?}"
        );
    }

    #[test]
    fn a_valid_field_shows_no_error_even_when_eager() {
        let tree = built(FormField::<Age>::new(TextEditingValue::from("21")).eager());
        let labels: Vec<String> = tree
            .find_all("Text")
            .iter()
            // `"text"`, and the value is `{:?}` of the string — so it arrives
            // quoted and these are `contains` rather than `==`.
            .filter_map(|node| node.property("text").map(str::to_owned))
            .collect();
        assert!(
            !labels.iter().any(|text| text.contains("18")),
            "nothing to complain about: {labels:?}"
        );
    }

    /// A second `FromInput` type, deliberately not `Age` — the point of
    /// `FormGroup` is that the fields it aggregates do not share a type.
    #[derive(Debug, PartialEq)]
    struct Name(String);

    impl FromInput for Name {
        type Error = &'static str;

        fn from_input(raw: &str) -> Result<Self, Self::Error> {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Err("a name is required")
            } else {
                Ok(Self(trimmed.to_owned()))
            }
        }
    }

    #[test]
    fn a_group_is_ready_only_when_every_field_is() {
        assert!(
            !FormGroup::ready(&[true, false]),
            "one bad field must hold the whole submit back"
        );
        assert!(
            FormGroup::ready(&[true, true]),
            "and every field parsing is what lets it through"
        );
        assert!(
            FormGroup::ready(&[]),
            "no fields to disagree about is vacuously ready"
        );
    }

    #[test]
    fn heterogeneous_fields_aggregate_with_no_shared_type() {
        // This compiling at all is most of the claim: `Age` and `Name` share
        // nothing but `FromInput`, and nothing here needed them to.
        let age = FormField::<Age>::new(TextEditingValue::from("21"));
        let name = FormField::<Name>::new(TextEditingValue::from("Ada"));

        assert!(FormGroup::ready(&[
            age.parse().is_ok(),
            name.parse().is_ok()
        ]));

        let bad_age = FormField::<Age>::new(TextEditingValue::from("12"));
        assert!(!FormGroup::ready(&[
            bad_age.parse().is_ok(),
            name.parse().is_ok()
        ]));
    }

    #[test]
    fn a_fresh_group_leaves_untouched_fields_lazy() {
        let group = FormGroup::new(false);
        let field = group.field(FormField::<Age>::new(TextEditingValue::from("12")));
        let tree = built(field);
        assert!(
            !tree
                .find_all("Text")
                .iter()
                .filter_map(|node| node.property("text"))
                .any(|text| text.contains("18 or over")),
            "nothing has been submitted yet, so nothing should be complaining"
        );
    }

    #[test]
    fn a_group_that_already_failed_makes_every_field_eager() {
        let group = FormGroup::new(true);
        let age = group.field(FormField::<Age>::new(TextEditingValue::from("12")));
        let name = group.field(FormField::<Name>::new(TextEditingValue::from("")));

        let age_tree = built(age);
        assert!(
            age_tree
                .find_all("Text")
                .iter()
                .filter_map(|node| node.property("text"))
                .any(|text| text.contains("18 or over")),
            "a field the user never touched must still speak up once the group \
             has already tried and failed"
        );

        let name_tree = built(name);
        assert!(
            name_tree
                .find_all("Text")
                .iter()
                .filter_map(|node| node.property("text"))
                .any(|text| text.contains("name is required")),
            "the flag applies to every field in the group, not just one"
        );
    }

    #[test]
    fn all_of_hands_over_the_values_rather_than_a_verdict() {
        let parsed = (Name::from_input("Ada"), Age::from_input("36"));
        let Some((name, age)) = FormGroup::all_of(parsed) else {
            panic!("both fields parse, so the group has to");
        };
        assert_eq!(name.0, "Ada");
        assert_eq!(age, Age(36));
    }

    #[test]
    fn one_bad_field_takes_the_whole_group_with_it() {
        assert!(
            FormGroup::all_of((Name::from_input("Ada"), Age::from_input("twelve"))).is_none(),
            "these fields together, or none"
        );
        assert!(
            FormGroup::all_of((Name::from_input(""), Age::from_input("36"))).is_none(),
            "and it does not matter which one"
        );
    }

    #[test]
    fn all_of_agrees_with_ready() {
        // The two answers cannot be allowed to disagree: `ready` decides whether
        // the button is live and `all_of` decides whether pressing it does
        // anything, so an application showing a live button that submits nothing
        // is exactly what a disagreement produces.
        //
        // All four combinations, rather than an example of each.
        for name in ["Ada", ""] {
            for age in ["36", "twelve"] {
                let (n, a) = (Name::from_input(name), Age::from_input(age));
                let ready = FormGroup::ready(&[n.is_ok(), a.is_ok()]);
                let all = FormGroup::all_of((n, a)).is_some();
                assert_eq!(
                    ready, all,
                    "ready and all_of disagreed on ({name:?}, {age:?})"
                );
            }
        }
    }

    #[test]
    fn a_group_of_one_and_a_group_of_eight_both_work() {
        // The arity range the macro covers, checked at both ends so that a
        // hand-edited macro invocation cannot quietly drop one.
        assert!(FormGroup::all_of((Age::from_input("36"),)).is_some());

        let eight = (
            Age::from_input("18"),
            Age::from_input("19"),
            Age::from_input("20"),
            Age::from_input("21"),
            Age::from_input("22"),
            Age::from_input("23"),
            Age::from_input("24"),
            Age::from_input("25"),
        );
        let Some(values) = FormGroup::all_of(eight) else {
            panic!("eight valid fields are still a valid group");
        };
        assert_eq!(values.7, Age(25));
    }
}
