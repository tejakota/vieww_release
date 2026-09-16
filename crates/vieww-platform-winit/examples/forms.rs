//! A form that cannot submit an unparsed field, and a layout that reflows.
//!
//! ```console
//! cargo run -p vieww-platform-winit --example forms --release
//! ```
//!
//! # What this is for
//!
//! `docs/AIMS.md` §H calls this **the clearest place in the whole checklist
//! where the type system buys something stringly APIs cannot**. The mechanism is in
//! `vieww-widget/src/forms.rs` and is checked by the compiler: `on_accepted` is
//! `Fn(T)`, so there is no arm that hands a `String` to the caller and the
//! double-parse hole has no representation.
//!
//! What none of that shows is **when the errors appear**, which is the entire
//! user-facing behaviour of a form. Errors here are lazy by default — telling
//! somebody their email is wrong three characters in is true and useless — and
//! become eager once a submit has been attempted. That is a judgement about
//! timing, and timing is only judgeable by typing.
//!
//! # Things to check
//!
//! 1. **Type an age of `7` and do not leave the field.** Nothing goes red.
//!    That is `eager` being off: the field is wrong and the user is still
//!    typing, and interrupting them is the failure most forms ship with.
//!
//! 2. **Press *Sign up* while something is invalid.** *Now* every bad field
//!    shows its message, and keeps showing it as you type. One flag flipped for
//!    the whole group — `FormGroup::new(attempted)` — rather than threaded into
//!    each field by hand.
//!
//! 3. **Fix everything.** The button enables. It is disabled by having **no
//!    handler at all** rather than by a greyed-out flag, which is why an
//!    unparsed field cannot reach the submit path even if the styling is wrong.
//!
//! 4. **Drag the window narrow.** Past 640pt the two columns become one.
//!    `LayoutBuilder::breakpoints` means that rebuild happens *only* when the
//!    threshold is crossed — dragging within a column count is free, which is
//!    the *Cheaper* win §F banks and which you can feel as the absence of jank.
//!
//! # Run it in release
//!
//! Debug builds of the layout pass are ten to thirty times slower.

use std::rc::Rc;

use vieww_element::{Runtime, Signal};
use vieww_foundation::Size;
use vieww_platform_winit::App;
use vieww_render::FrameDriver;
use vieww_widget::forms::{FormField, FormGroup, FromInput};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, Handler};

const SURFACE: Size = Size {
    width: 900.0,
    height: 620.0,
};

const BACKGROUND: Color = ThemeData::dark().colors.surface;

/// Where the layout changes from two columns to one.
///
/// One threshold, named, because it appears twice — once as the breakpoint the
/// rebuild is quantised on and once as the test the builder makes. Two copies
/// of a number that must agree is how a responsive layout ends up rebuilding on
/// one side of a boundary and laying out for the other.
const NARROW: f32 = 640.0;

fn main() {
    let app = App::new()
        .title("vieww — forms")
        .size(SURFACE)
        .background(BACKGROUND);

    let result = app.run(move |driver: &mut FrameDriver| {
        let runtime = driver.elements().runtime().clone();
        driver.set_root(Shell {
            state: State::new(&runtime),
        });
    });

    match result {
        Ok(report) => println!("{report}"),
        Err(error) => eprintln!("forms failed: {error}"),
    }
}

/// An age in whole years, over eighteen.
///
/// The parse **is** the validation — there is no separate validator that could
/// disagree with it, because there is no second parse for it to disagree with.
#[derive(Debug, Clone, Copy)]
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

/// A name that is not blank.
#[derive(Debug, Clone)]
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

/// An address with an `@` and something either side of it.
///
/// Deliberately not a real email validator. The point being demonstrated is
/// where the error surfaces, not how well the rule is written — and a rule
/// nobody can read makes the timing harder to see rather than easier.
#[derive(Debug, Clone)]
struct Email(String);

impl FromInput for Email {
    type Error = &'static str;

    fn from_input(raw: &str) -> Result<Self, Self::Error> {
        let trimmed = raw.trim();
        match trimmed.split_once('@') {
            Some((user, host)) if !user.is_empty() && host.contains('.') => {
                Ok(Self(trimmed.to_owned()))
            }
            _ => Err("something@example.com"),
        }
    }
}

#[derive(Debug, Clone)]
struct State {
    name: Signal<TextEditingValue>,
    age: Signal<TextEditingValue>,
    email: Signal<TextEditingValue>,
    password: Signal<TextEditingValue>,
    reveal: Signal<bool>,
    /// Whether a submit has been tried and failed. The one flag that turns every
    /// field eager at once — see the module docs.
    attempted: Signal<bool>,
    accepted: Signal<Option<String>>,
}

impl State {
    fn new(runtime: &Runtime) -> Self {
        Self {
            name: runtime.signal(TextEditingValue::new("Ada")),
            age: runtime.signal(TextEditingValue::new("7")),
            email: runtime.signal(TextEditingValue::new("ada@")),
            password: runtime.signal(TextEditingValue::default()),
            reveal: runtime.signal(false),
            attempted: runtime.signal(false),
            accepted: runtime.signal(None),
        }
    }
}

#[derive(Debug)]
struct Shell {
    state: State,
}

impl Widget for Shell {
    fn debug_name(&self) -> &'static str {
        "Shell"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Theme::new(ThemeData::dark())
            .child(Body {
                state: self.state.clone(),
            })
            .into()
    }
}

widget_node_from!(Shell);

#[derive(Debug)]
struct Body {
    state: State,
}

impl Widget for Body {
    fn debug_name(&self) -> &'static str {
        "Body"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let state = self.state.clone();
        // The rebuild is dropped unless the width crosses `NARROW`, so dragging
        // the window inside one column count costs nothing. That is the whole
        // claim; see the module docs.
        SafeArea::new()
            .child(
                Padding::new(EdgeInsets::all(28.0)).child(
                    LayoutBuilder::new(move |constraints: Constraints| {
                        Form {
                            state: state.clone(),
                            narrow: constraints.max_width < NARROW,
                        }
                        .into()
                    })
                    .breakpoints([NARROW]),
                ),
            )
            .into()
    }
}

widget_node_from!(Body);

#[derive(Debug)]
struct Form {
    state: State,
    narrow: bool,
}

impl Widget for Form {
    fn debug_name(&self) -> &'static str {
        "Form"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let attempted = self.state.attempted.get();
        let group = FormGroup::new(attempted);

        // Each field is built once and asked twice: once for its widget and
        // once for whether it parses. `parse` is the same call the field makes
        // internally, so "is the form ready" and "does this field show an
        // error" cannot disagree.
        let name = group.field(
            FormField::<Name>::new(self.state.name.get())
                .label("Name")
                .on_changed(handler(&self.state.name)),
        );
        let age = group.field(
            FormField::<Age>::new(self.state.age.get())
                .label("Age")
                .on_changed(handler(&self.state.age)),
        );
        let email = group.field(
            FormField::<Email>::new(self.state.email.get())
                .label("Email")
                .on_changed(handler(&self.state.email)),
        );

        // Parsed once, here, and used for both questions the form asks: whether
        // it is ready, and what the submit handler is given. **There is no
        // second parse** — this is §H's whole point, and a form that asked
        // `is_ok()` here and re-parsed inside the handler would have reopened
        // exactly the hole `forms.rs` closes.
        let parsed_name = name.parse();
        let parsed_age = age.parse();
        let parsed_email = email.parse();
        let ready = FormGroup::ready(&[
            parsed_name.is_ok(),
            parsed_age.is_ok(),
            parsed_email.is_ok(),
        ]);

        let fields: Vec<WidgetNode> = vec![name.into(), age.into(), email.into()];
        let laid_out: WidgetNode = if self.narrow {
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .main_axis_size(MainAxisSize::Min)
                .spacing(16.0)
                .children(fields)
                .into()
        } else {
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Start)
                .spacing(20.0)
                .children(
                    fields
                        .into_iter()
                        .map(|field| Flexible::new(1).child(field).into())
                        .collect::<Vec<WidgetNode>>(),
                )
                .into()
        };

        let mark = self.state.attempted.clone();
        let accepted = self.state.accepted.clone();

        let mut submit = Button::new("Sign up");
        match (parsed_name, parsed_age, parsed_email) {
            // The enabled arm is the *only* place a handler is attached, and it
            // can only be built from three values that already parsed — so the
            // handler closes over `Name`, `Age` and `Email` rather than over
            // three strings it would have to parse again. A submit path that
            // sees unparsed input is not a bug that can be introduced here; it
            // has no representation.
            (Ok(Name(who)), Ok(Age(years)), Ok(Email(address))) if ready => {
                submit = submit.on_pressed(move || {
                    accepted.set(Some(format!("{who}, {years}, {address}")));
                    mark.set(false);
                });
            }
            _ => {
                submit = submit.on_pressed(move || mark.set(true));
            }
        }

        let reveal = self.state.reveal.get();
        let toggle_reveal = self.state.reveal.clone();
        let password = self.state.password.get();
        let write_password = self.state.password.clone();
        let empty = password.text.is_empty();

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .main_axis_size(MainAxisSize::Min)
            .spacing(20.0)
            .children(children![
                Text::new("Create an account").style(theme.text.headline),
                Text::new(if self.narrow {
                    "One column — the window is under 640pt."
                } else {
                    "Two columns — drag the window narrow to reflow."
                })
                .style(theme.text.label)
                .color(theme.colors.on_surface_variant),
                laid_out,
                // The password field and its placeholder, which are the two
                // things you can only judge by typing into them. Watch that the
                // caret lands *between* bullets rather than inside one — the
                // paragraph is shaped from the mask, so it has to.
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(12.0)
                    .children(children![
                        Constrained::new(Constraints::tight_for_width(240.0)).child(
                            Container::new()
                                .color(theme.colors.surface_variant)
                                .padding(EdgeInsets::all(12.0))
                                .child(
                                    TextField::new(password)
                                        .obscure(!reveal)
                                        .placeholder("Password")
                                        .placeholder_color(theme.colors.on_surface_variant)
                                        .color(theme.colors.on_surface)
                                        .single_line()
                                        .on_changed(Rc::new(move |next| write_password.set(next)))
                                )
                        ),
                        Button::new(if reveal { "Hide" } else { "Show" })
                            .style(ButtonStyle::Text)
                            .on_pressed(move || toggle_reveal.set(!reveal)),
                        Text::new(if empty {
                            "empty — the placeholder is showing, and is not the value"
                        } else if reveal {
                            "revealed"
                        } else {
                            "masked — one bullet per character, not per byte"
                        })
                        .style(theme.text.label)
                        .color(theme.colors.on_surface_variant),
                    ]),
                Flex::row()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .spacing(16.0)
                    .children(children![
                        submit,
                        Text::new(match self.state.accepted.get() {
                            Some(summary) => format!("welcome, {summary}"),
                            None if attempted => "fix the fields above".to_owned(),
                            None => "nothing submitted yet".to_owned(),
                        })
                        .style(theme.text.label)
                        .color(if attempted {
                            theme.colors.error
                        } else {
                            theme.colors.on_surface_variant
                        }),
                    ]),
            ])
            .into()
    }
}

widget_node_from!(Form);

/// A field's `on_changed`, writing straight back to the signal it reads from.
///
/// Every field here is controlled, for `controls.rs`'s reason: the widget shows
/// what it is given and reports what it wants, and never holds a second copy of
/// the text that could drift from this one.
fn handler(signal: &Signal<TextEditingValue>) -> Handler<TextEditingValue> {
    let write = signal.clone();
    Rc::new(move |next| write.set(next))
}
