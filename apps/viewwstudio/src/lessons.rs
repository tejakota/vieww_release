//! The "Learn" view: editable lessons, one per feature.
//!
//! # What this is, and what it is not
//!
//! The repository ships ~50 `examples/features/*` programs, each a complete
//! `fn main()` that opens a window through `feature_harness::launch`. They
//! prove the framework does what it claims; they do not, on their own, teach
//! somebody how to write a screen, because the thing they have to write into
//! the editor is `pub fn screen() -> impl Widget` — a different shape.
//!
//! This module is the bridge. Each lesson is a complete `pub fn screen()`
//! source that compiles and renders in the studio's preview, written to
//! mirror the topic of one of the examples. Clicking a lesson in the sidebar
//! puts its source in the active buffer; Render then shows what the example
//! demonstrated, in the device frame the studio already has.
//!
//! # Why a curated subset rather than all fifty
//!
//! The examples overlap — `10-colors-rgba` and `09-colors-rgb` are the same
//! lesson in two colour spaces — and several (the splash screens, the scroll-
//! driven strips) depend on `feature_harness`'s clock and step machinery in a
//! way the preview does not replicate. A curated set that covers the catalog
//! is more useful than a literal port of all fifty, and it is one a
//! contributor can keep in step with the framework as it changes.
//!
//! # The shape
//!
//! Each lesson is a [`Lesson`] with a `name`, a one-line `summary`, the
//! `example` it mirrors (so a learner can find the screenshot), and the
//! `body` — Rust source, ready to drop into the editor. The body is a
//! `&'static str` rather than `include_str!`'d from a file, because the
//! examples are `fn main()`s and what the studio needs is `pub fn screen()`;
//! writing the lessons here keeps the transformation in one place and the
//! source in the same file as the metadata.

/// One editable lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lesson {
    /// What the list calls it. Mirrors the example directory's `NN-name`.
    pub name: &'static str,
    /// One line under the title, so the list is scannable without opening
    /// anything.
    pub summary: &'static str,
    /// The example directory this mirrors, e.g. `"00-rectangle"`. `None` for
    /// a lesson with no direct counterpart (the "screen entry point" lesson,
    /// which is the contract rather than a feature).
    pub example: Option<&'static str>,
    /// The Rust source, ready for the editor. Always defines
    /// `pub fn screen() -> impl Widget`.
    pub body: &'static str,
    /// The same lesson in Say, when the lesson's idea fits the v1
    /// vocabulary. `None` for the lessons that are about Rust-specific
    /// machinery (custom paint, animation drivers) rather than about a
    /// screen — the toggle simply does not appear for those.
    pub say_body: Option<&'static str>,
}

/// The lessons, in the order the sidebar lists them.
///
/// Ordered by what a new user reaches for first: the entry point, the shapes,
/// the controls, the painting, the animation. Not the example directory's
/// numeric order — that orders by feature-add, which is a contributor's view
/// rather than a learner's.
pub const LESSONS: [Lesson; 12] = [
    Lesson {
        name: "Your first screen",
        summary: "The function Render looks for, and the simplest screen that compiles.",
        example: None,
        body: r#"use vieww::prelude::*;

/// The screen the preview mounts.
///
/// `pub fn screen() -> impl Widget` is the contract: the studio appends an
/// entry point that calls it, builds the file as a library, and mounts what
/// comes back inside the device frame on the right.
pub fn screen() -> impl Widget {
    Container::new()
        .color(Color::hex(0xF7_F8FA))
        .alignment(Alignment::CENTER)
        .child(Text::new("Hello, vieww"))
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- The same screen in Say. Render it, then open the Generated Rust tab to
-- see what every line becomes.

screen "Home":
    a center:
        a heading "Hello, vieww"
"#,
        ),
    },
    Lesson {
        name: "A coloured rectangle",
        summary: "Container with a colour, sized by its parent.",
        example: Some("00-rectangle"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Container::new().color(Color::rgb(58, 122, 246))
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- A blue rectangle: a box with a colour and a size.

screen "Home":
    a center:
        a card, color #3A7AF6, 320 wide, 120 tall
"#,
        ),
    },
    Lesson {
        name: "Text, sized and styled",
        summary: "A Text with size, weight and colour — the building block.",
        example: Some("02-text-display"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Container::new()
        .color(Color::hex(0xF7_F8FA))
        .padding(EdgeInsets::all(24.0))
        .child(
            Text::new("the quick brown fox jumps over the lazy dog")
                .color(Color::hex(0x17_1E2A))
                .size(28.0)
                .bold(),
        )
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- One text, sized, weighted and coloured.

screen "Home":
    a card, color #F7F8FA, padded 24:
        a text "the quick brown fox jumps over the lazy dog", size 28, bold, color #171E2A
"#,
        ),
    },
    Lesson {
        name: "A row and a column",
        summary: "Flex lays out along an axis; crossAxisAlignment positions across.",
        example: Some("12-flex-row"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    // `SafeArea` keeps the content clear of the notch and the home indicator.
    // The preview publishes the chosen device's real insets, so this moves when
    // you change the device in the toolbar above.
    SafeArea::new().child(Container::new().padding(EdgeInsets::all(20.0)).child(
    Flex::row()
        .spacing(12.0)
        .cross_axis_alignment(CrossAxisAlignment::Center)
        .children(children![
            Container::new().color(Color::hex(0x58_7AF6)).size(60.0, 60.0),
            Container::new().color(Color::hex(0x33_88FF)).size(60.0, 60.0),
            Container::new().color(Color::hex(0x10_1418)).size(60.0, 60.0),
        ]),
    ))
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- Three fixed boxes laid out along a row, with space between them.

screen "Home":
    a row, spaced 12:
        a card, color #587AF6, 60 wide, 60 tall
        a card, color #3388FF, 60 wide, 60 tall
        a card, color #101418, 60 wide, 60 tall
"#,
        ),
    },
    Lesson {
        name: "Stacking with Positioned",
        summary: "A Stack with absolute children, for overlays and badges.",
        example: Some("14-stack-positioned"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Stack::new()
        .children(children![
            Container::new().color(Color::hex(0xF7_F8FA)).size(200.0, 200.0),
            Positioned::new()
                .top(12.0)
                .right(12.0)
                .child(
                    Container::new()
                        .color(Color::hex(0xE5_3935))
                        .radius(10.0)
                        .size(20.0, 20.0),
                ),
        ])
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- A stack layers its children; `pinned` positions one absolutely.

screen "Home":
    a stack:
        a card, color #F7F8FA, 200 wide, 200 tall
        a card, color #E53935, 20 wide, 20 tall, pinned top 12, pinned right 12
"#,
        ),
    },
    Lesson {
        name: "A button that does something",
        summary: "Button with on_pressed — the simplest interactive control.",
        example: Some("25-button-pressable"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Container::new()
        .padding(EdgeInsets::all(24.0))
        .child(
            Button::new("Press me").on_pressed(|| {
                // The studio's preview is real input: this fires on a tap
                // inside the device frame, with no forwarding code.
            }),
        )
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- A tap changes state; the label is alive because it reads the state.

keep a whole number called presses starting at 0

screen "Home":
    a card, padded 24:
        a column, spaced 12, children aligned to the start:
            a label "Pressed \(presses) times"
            a button "Press me" which when tapped:
                add 1 to presses
                show a snackbar "The tap reached the screen"
"#,
        ),
    },
    Lesson {
        name: "Switches and sliders",
        summary: "Controlled inputs, and the state that makes them move.",
        example: Some("47-form-controls"),
        // **This lesson used to render three disabled controls.**
        //
        // It was three bare constructors — `Switch::new(false)`,
        // `Slider::new(0.4)`, `Checkbox::new(true)` — with no `on_changed` on
        // any of them, under a summary reading "controlled, like every input
        // here". A handler is what *enables* an input in this framework
        // (`Switch::on_changed` says so, and it is this framework's rule), so the
        // lesson rendered greyed-out controls that did not answer a tap, and
        // taught the opposite of its own sentence to somebody meeting the
        // widget set for the first time. On a light theme they were also very
        // nearly invisible, which is a separate defect this found — see
        // `controls::switch::Switch::appearance`.
        //
        // The fix is not to add empty handlers: a control that looks live and
        // does nothing when touched is a worse lie than one that looks
        // disabled. The honest version is the whole pattern — state the screen
        // owns, read in `build`, written from the handler, and a rebuild asked
        // for — because "where does the value live" is the question these three
        // controls raise and the one nothing else in the lesson list answers.
        //
        // Longer than the other lessons, deliberately. It compiles and runs in
        // the preview: a tap inside the device frame moves the switch.
        body: r#"use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use vieww::prelude::*;

/// The state this screen owns. A `Cell` is enough: it is read
/// during build and written from a handler, never both at once.
#[derive(Debug, Default)]
struct FormState {
    on: Cell<bool>,
    amount: Cell<f32>,
    ticked: Cell<bool>,
    /// Set by a handler, taken by the tree. Writing state from
    /// outside a build changes nothing on its own — this is what
    /// asks for the rebuild that shows it.
    dirty: Cell<bool>,
}

impl ElementState for FormState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn take_pending(&mut self) -> bool {
        self.dirty.replace(false)
    }
}

#[derive(Debug)]
pub struct Form;

impl Widget for Form {
    fn debug_name(&self) -> &'static str {
        "Form"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    /// One state object per mounted element, made once.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(FormState {
            amount: Cell::new(0.4),
            ticked: Cell::new(true),
            ..FormState::default()
        }))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let (on, amount, ticked) = ctx
            .state::<FormState, _>(|s| (s.on.get(), s.amount.get(), s.ticked.get()))
            .unwrap_or((false, 0.4, true));

        // One writer, shared by the three handlers below.
        let handle = ctx.state_handle();
        let write = move |apply: &dyn Fn(&FormState)| {
            if let Some(handle) = &handle {
                let borrowed = handle.borrow();
                if let Some(state) = borrowed.as_any().downcast_ref::<FormState>() {
                    apply(state);
                    state.dirty.set(true);
                }
            }
        };
        let write = Rc::new(write);

        SafeArea::new().child(
            Container::new()
                .padding(EdgeInsets::all(20.0))
                .child(
                    Flex::column()
                        .spacing(16.0)
                        .cross_axis_alignment(CrossAxisAlignment::Start)
                        .children(children![
                            // A handler is what *enables* an input. Without
                            // one the control is disabled — which is how you
                            // show a value somebody may not edit.
                            Switch::new(on).on_changed({
                                let write = Rc::clone(&write);
                                Rc::new(move |next: bool| write(&|s: &FormState| s.on.set(next)))
                            }),
                            Slider::new(amount).range(0.0, 1.0).on_changed({
                                let write = Rc::clone(&write);
                                Rc::new(move |next: f32| write(&|s: &FormState| s.amount.set(next)))
                            }),
                            Checkbox::new(ticked).on_changed({
                                let write = Rc::clone(&write);
                                Rc::new(move |next: bool| write(&|s: &FormState| s.ticked.set(next)))
                            }),
                        ]),
                ),
        )
        .into()
    }
}

pub fn screen() -> impl Widget {
    Form
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- Controlled inputs: each one is bound to a piece of state, and a control
-- with no binding is disabled by design. Tap, drag and slide — the label
-- at the bottom reads the same state the controls write.

keep a yes-or-no called on starting at no
keep a number called amount starting at 0.4
keep a yes-or-no called ticked starting at yes

screen "Home":
    a card, padded 20:
        a column, spaced 16, children aligned to the start:
            a switch bound to on
            a slider bound to amount, from 0.0 to 1.0
            a checkbox bound to ticked
            a label "The slider is at \(amount)"
"#,
        ),
    },
    Lesson {
        name: "Opacity and clip",
        summary: "Opacity fades a subtree; Clip rounds its corners.",
        example: Some("15-opacity"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    Opacity::new(0.6).child(
        Clip::rounded(12.0).child(
            Container::new()
                .color(Color::hex(0x33_88FF))
                .child(SizedBox::square(80.0)),
        ),
    )
}
"#,
        say_body: None,
    },
    Lesson {
        name: "A virtualised list",
        summary: "ListView builds only the rows on screen — 200 rows is cheap.",
        example: Some("42-list-view"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    ListView::new(200, 28.0, std::rc::Rc::new(|index: usize| {
        Text::new(format!("row {index}")).into()
    }))
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- A list builds only the rows on screen. The template under the list is
-- built per row; `\(item)` reads the row's value.

keep a list of text called rows starting at "row 0", "row 1", "row 2", "row 3", "row 4"

screen "Home":
    a list of rows, each row 28:
        a text "row \(item)"
"#,
        ),
    },
    Lesson {
        name: "Custom paint",
        summary: "CustomPaint hands you a canvas; DrawInstruction is the brush.",
        example: Some("35-custom-paint"),
        body: r#"use vieww::prelude::*;

#[derive(Debug)]
struct Fill;

impl CustomPainter for Fill {
    fn paint(&self, size: Size) -> Vec<DrawInstruction> {
        vec![DrawInstruction::FillRect {
            rect: Rect::new(0.0, 0.0, size.width, size.height),
            color: Color::hex(0x20_2530),
        }]
    }
    fn as_any(&self) -> &dyn std::any::Any { self }
}

pub fn screen() -> impl Widget {
    CustomPaint::sized(Size::new(200.0, 120.0), Fill)
}
"#,
        say_body: None,
    },
    Lesson {
        name: "An animated container",
        summary: "AnimatedContainer tweens between values across rebuilds.",
        example: Some("38-animated-container"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    // Edit the colour or the size and Render again — the container tweens
    // between the old value and the new rather than snapping.
    AnimatedContainer::new()
        .color(Color::hex(0x22_88FF))
        .width(120.0)
        .height(60.0)
}
"#,
        say_body: None,
    },
    Lesson {
        name: "Theming: light and dark",
        summary: "ThemeData::adaptive follows the platform — and the preview's dark toggle.",
        example: Some("21-theme-light-dark"),
        body: r#"use vieww::prelude::*;

pub fn screen() -> impl Widget {
    // The preview's toolbar has a Dark toggle. The theme follows it, because
    // `adaptive` is what reads the dark argument the studio publishes.
    Theme::new(ThemeData::adaptive(TargetPlatform::Android, true))
        .child(
            Container::new()
                .padding(EdgeInsets::all(24.0))
                .child(Button::new("Themed").on_pressed(|| {})),
        )
}
"#,
        say_body: Some(
            r#"-- say-language: 1
-- Theme colours are read, not chosen: `the primary color` is whatever the
-- theme says, and the preview's dark toggle changes the theme. Flip the
-- toggle above the device frame and every line here follows.

screen "Home":
    a card, padded 24:
        a column, spaced 12, children aligned to the start:
            a label "Flip the preview's dark toggle — these follow it."
            a heading "Themed"
            a label "This label is the on-surface colour."
            a button "Themed" which when tapped:
                show a snackbar "The button uses the theme's own colours"
"#,
        ),
    },
];

/// The lesson at `index`, or the first.
#[must_use]
pub fn lesson(index: usize) -> &'static Lesson {
    LESSONS.get(index).unwrap_or(&LESSONS[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lesson_defines_screen() {
        // A lesson that does not define `pub fn screen()` will not render in
        // the studio, which is the one promise a lesson makes. Caught here
        // rather than at runtime, because the body is a `&'static str` and the
        // check is cheap.
        for lesson in LESSONS {
            assert!(
                lesson.body.contains("pub fn screen() -> impl Widget"),
                "{} does not define `pub fn screen()`",
                lesson.name
            );
        }
    }

    #[test]
    fn every_lesson_has_a_summary() {
        for lesson in LESSONS {
            assert!(!lesson.summary.is_empty(), "{} has no summary", lesson.name);
            assert!(
                lesson.summary.ends_with('.'),
                "{}'s summary should end with a period",
                lesson.name
            );
        }
    }

    /// The Say gate: every Say body a lesson ships must compile, or the
    /// toggle hands a beginner a file that renders as a wall of errors.
    /// Checked here rather than at runtime for the same reason the Rust
    /// bodies are checked for `screen()`.
    #[test]
    fn every_say_body_compiles() {
        for lesson in LESSONS {
            if let Some(say_body) = lesson.say_body {
                let result = vieww_say_codegen::compile("lesson.say", say_body);
                assert!(
                    result.is_ok(),
                    "{}: Say body does not compile: {:?}",
                    lesson.name,
                    result
                        .err()
                        .map(|d| d.iter().map(|d| d.to_string()).collect::<Vec<_>>())
                );
            }
        }
    }

    #[test]
    fn lesson_names_are_unique() {
        let mut names: Vec<_> = LESSONS.iter().map(|l| l.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate lesson names");
    }
}
