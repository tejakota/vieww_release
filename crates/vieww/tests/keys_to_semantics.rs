//! The Phase 8 exit test: a keyboard, an input method, a screen reader, and an
//! application that survives being backgrounded.
//!
//! ```console
//! cargo test -p vieww --test keys_to_semantics
//! ```
//!
//! # What the roadmap asks for
//!
//! > VoiceOver/TalkBack can navigate and read your widget tree correctly; IME
//! > composition (including autocomplete/spellcheck) works in a text field; app
//! > correctly handles backgrounding without losing state.
//!
//! Three clauses, and each is asserted here against the real pipeline rather
//! than described:
//!
//! - **navigate and read**: a semantics tree is built from a mounted, laid-out
//!   widget tree; a screen reader's reading order is checked; a button is one
//!   stop and not two; the focused node is the one reported as focused.
//! - **IME composition**: `ka` → `か` through preedit and commit, on the
//!   focused field, with the provisional text replaced rather than appended —
//!   which is the whole difficulty.
//! - **backgrounding without losing state**: the surface is destroyed and
//!   recreated, and the text, the caret, the focus and the element state all
//!   come back.
//!
//! # What this cannot assert, and why it is still worth having
//!
//! Whether *VoiceOver itself* reads it correctly needs VoiceOver, a Mac and a
//! human ear. What is checkable without those is that the tree handed to
//! AccessKit says the right things, and that is what is checked. The conversion
//! into AccessKit's own types is tested in `vieww-platform-winit`, which is
//! where the dependency lives.

use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{
    Clipboard, EdgeInsets, ImeEvent, KeyEvent, LogicalKey, MemoryClipboard, Modifiers, NamedKey,
    Offset, PointerEvent, PointerId, Services, SharedServices, Size, TargetPlatform,
    TextEditingValue, ViewMetrics,
};
use vieww::prelude::*;
use vieww::{FrameDriver, Role, Semantics};

const SURFACE: Size = Size {
    width: 400.0,
    height: 300.0,
};
const POINTER: PointerId = PointerId(1);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A screen with a labelled button and an editable field.
#[derive(Debug)]
struct Screen {
    value: Signal<TextEditingValue>,
    /// `Some` makes the field single-line and records what it submits.
    submitted: Option<Signal<Vec<String>>>,
}

impl Widget for Screen {
    fn debug_name(&self) -> &'static str {
        "Screen"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let value = self.value.clone();
        let apply = self.value.clone();

        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Start)
            .children(children![
                // A button is a box with a label in it. Nothing about that shape
                // says "button", which is exactly why it has to be declared.
                Semantics::button("Submit").child(
                    Container::new()
                        .padding(EdgeInsets::all(8.0))
                        .child(Text::new("Submit"))
                ),
                Text::new("Your name"),
                {
                    let field = TextField::new(value.get())
                        .size(16.0)
                        .on_changed(Rc::new(move |next| apply.set(next)));
                    match &self.submitted {
                        Some(log) => {
                            let log = log.clone();
                            field.on_submit(Rc::new(move |text| {
                                let mut all = log.get();
                                all.push(text);
                                log.set(all);
                            }))
                        }
                        None => field,
                    }
                },
            ])
            .into()
    }
}

vieww::widget::widget_node_from!(Screen);

struct Harness {
    driver: FrameDriver,
    value: Signal<TextEditingValue>,
    submitted: Option<Signal<Vec<String>>>,
    clipboard: Option<Rc<MemoryClipboard>>,
}

impl Harness {
    fn new() -> Self {
        Self::with_field(false)
    }

    /// A harness whose field is single-line, recording what it submits.
    fn single_line() -> Self {
        Self::with_field(true)
    }

    /// A harness whose tree has a pasteboard published above it.
    ///
    /// A [`MemoryClipboard`] in a real [`Services`] registry, so everything
    /// between the key and the pasteboard is the production path — only the
    /// platform's own implementation is swapped.
    fn with_clipboard() -> Self {
        let clipboard = Rc::new(MemoryClipboard::new());
        let mut registry = Services::new();
        registry.provide::<dyn Clipboard>(Rc::clone(&clipboard) as Rc<dyn Clipboard>);

        let mut harness = Self::build(false, Some(SharedServices::new(registry)));
        harness.clipboard = Some(clipboard);
        harness
    }

    fn with_field(single_line: bool) -> Self {
        Self::build(single_line, None)
    }

    fn build(single_line: bool, services: Option<SharedServices>) -> Self {
        let mut driver = FrameDriver::new(SURFACE);
        let value = driver
            .elements()
            .runtime()
            .signal(TextEditingValue::default());
        let submitted = single_line.then(|| driver.elements().runtime().signal(Vec::new()));

        let screen = Screen {
            value: value.clone(),
            submitted: submitted.clone(),
        };

        // Through `set_root` rather than `elements().set_root`, so the view
        // metrics are published above the tree exactly as an application gets
        // them.
        match services {
            Some(services) => driver.set_root(Inherited::new(services, screen)),
            None => driver.set_root(screen),
        }
        driver.draw_frame();

        Self {
            driver,
            value,
            submitted,
            clipboard: None,
        }
    }

    /// Press `key` with the platform's shortcut modifier held.
    fn shortcut(&mut self, key: &str) {
        let modifier = Modifiers::shortcut_for(TargetPlatform::current());
        self.press_with(LogicalKey::Character(key.to_owned()), modifier);
    }

    fn select_all(&mut self) {
        self.shortcut("a");
    }

    /// What is on the harness's pasteboard.
    fn pasteboard(&self) -> Option<String> {
        self.clipboard
            .as_ref()
            .expect("built with Harness::with_clipboard")
            .read_text()
            .expect("a memory pasteboard cannot fail")
    }

    /// What the single-line field has submitted, in order.
    fn submissions(&self) -> Vec<String> {
        self.submitted
            .as_ref()
            .expect("built with Harness::single_line")
            .get()
    }

    fn text(&self) -> String {
        self.value.get().text
    }

    fn press(&mut self, key: LogicalKey) {
        self.driver.handle_key(&KeyEvent::down(key, ms(0)));
        self.driver.draw_frame();
    }

    fn press_with(&mut self, key: LogicalKey, modifiers: Modifiers) {
        self.driver
            .handle_key(&KeyEvent::down(key, ms(0)).with_modifiers(modifiers));
        self.driver.draw_frame();
    }

    fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.press(LogicalKey::Character(character.to_string()));
        }
    }

    fn ime(&mut self, event: &ImeEvent) {
        self.driver.handle_ime(event);
        self.driver.draw_frame();
    }

    /// Tap at a point, which is what moves focus.
    fn tap(&mut self, at: Offset) {
        self.driver
            .handle_pointer(&PointerEvent::down(POINTER, at, ms(0)));
        self.driver
            .handle_pointer(&PointerEvent::up(POINTER, at, ms(50)));
        self.driver.draw_frame();
    }

    /// Where the field is, found through the semantics tree rather than
    /// hand-computed, so the test does not encode the layout.
    fn field_centre(&self) -> Offset {
        let semantics = self.driver.semantics();
        let node = semantics
            .nodes()
            .iter()
            .find(|node| node.role == Role::TextField)
            .expect("the screen has a field in it");
        Offset::new(
            node.bounds.left + node.bounds.width() / 2.0,
            node.bounds.top + node.bounds.height() / 2.0,
        )
    }

    fn focus_field(&mut self) {
        let centre = self.field_centre();
        self.tap(centre);
    }

    /// The field's laid-out height, read from the semantics tree for the same
    /// reason [`field_centre`](Self::field_centre) is: so the test does not
    /// encode the layout it is checking.
    fn field_height(&self) -> f32 {
        let semantics = self.driver.semantics();
        semantics
            .nodes()
            .iter()
            .find(|node| node.role == Role::TextField)
            .expect("the screen has a field in it")
            .bounds
            .height()
    }
}

// ---------------------------------------------------------------- navigate and read

#[test]
fn a_screen_reader_is_told_what_is_on_the_screen() {
    let harness = Harness::new();
    let semantics = harness.driver.semantics();

    let described = semantics.describe();
    assert!(
        described.contains("Button \"Submit\""),
        "a button has to announce itself as one:\n{described}"
    );
    assert!(
        described.contains("Label \"Your name\""),
        "static text has to be readable:\n{described}"
    );
    assert!(
        described.contains("TextField"),
        "a field has to announce itself as editable:\n{described}"
    );
}

#[test]
fn a_button_is_one_stop_and_not_two() {
    let harness = Harness::new();
    let semantics = harness.driver.semantics();

    let buttons = semantics
        .nodes()
        .iter()
        .filter(|node| node.role == Role::Button)
        .count();
    assert_eq!(buttons, 1);

    let submit = semantics
        .nodes()
        .iter()
        .find(|node| node.role == Role::Button)
        .expect("a button");
    assert!(
        submit.children.is_empty(),
        "the label inside the button must be merged into it, or every button \
         in the application is announced twice"
    );
    assert_eq!(
        semantics
            .nodes()
            .iter()
            .filter(|node| node.label.as_deref() == Some("Submit"))
            .count(),
        1,
        "and the text it contains must not survive as a second node"
    );
}

#[test]
fn the_reading_order_follows_the_screen() {
    let harness = Harness::new();
    let semantics = harness.driver.semantics();

    let roles: Vec<Role> = semantics
        .reading_order()
        .into_iter()
        .filter_map(|id| semantics.node(id).map(|node| node.role))
        .collect();

    let interesting: Vec<Role> = roles
        .into_iter()
        .filter(|role| !matches!(role, Role::Window | Role::Group))
        .collect();
    assert_eq!(
        interesting,
        [Role::Button, Role::Label, Role::TextField],
        "a screen reader swiping forward must find them in the order they are \
         laid out, not in whatever order the arena allocated them"
    );
}

#[test]
fn a_layout_box_is_not_something_a_screen_reader_stops_on() {
    let harness = Harness::new();
    let semantics = harness.driver.semantics();

    // The tree has a column, containers, padding and coloured boxes in it. None
    // of them are worth announcing, and a tree that reported them would take a
    // dozen swipes to cross.
    assert!(
        semantics.len() <= 5,
        "expected the window plus a handful of real controls, got {}:\n{}",
        semantics.len(),
        semantics.describe()
    );
}

#[test]
fn the_focused_control_is_the_one_reported_as_focused() {
    let mut harness = Harness::new();
    assert!(
        harness
            .driver
            .semantics()
            .nodes()
            .iter()
            .all(|node| !node.focused),
        "nothing has focus before anything is touched"
    );

    harness.focus_field();

    let semantics = harness.driver.semantics();
    let focused: Vec<Role> = semantics
        .nodes()
        .iter()
        .filter(|node| node.focused)
        .map(|node| node.role)
        .collect();
    assert_eq!(
        focused,
        [Role::TextField],
        "a screen reader follows this to know where the user is"
    );
}

// ------------------------------------------------------------------------- keyboard

#[test]
fn tapping_a_field_lets_the_keyboard_type_into_it() {
    let mut harness = Harness::new();
    harness.focus_field();

    harness.type_text("hi");

    assert_eq!(harness.text(), "hi");
}

#[test]
fn typing_with_nothing_focused_goes_nowhere() {
    let mut harness = Harness::new();
    harness.type_text("hi");
    assert_eq!(
        harness.text(),
        "",
        "a keypress with no focus must not reach a field the user never chose"
    );
}

#[test]
fn backspace_and_the_arrows_edit_around_the_caret() {
    let mut harness = Harness::new();
    harness.focus_field();
    harness.type_text("abc");

    harness.press(LogicalKey::Named(NamedKey::Backspace));
    assert_eq!(harness.text(), "ab");

    harness.press(LogicalKey::Named(NamedKey::ArrowLeft));
    harness.type_text("X");
    assert_eq!(harness.text(), "aXb", "the caret moved before the insert");
}

// Multiline. Every layer of this already worked and none of it was joined up in
// a test: `NamedKey::Enter` maps to `TextIntent::Newline`, `apply` inserts a
// `\n`, the paragraph breaks on it (`explicit_newlines_break_lines`), and the
// render object moves the caret between lines against a goal column. Four green
// units and no evidence the feature worked, which is the exact shape of every
// bug this project has recorded.

#[test]
fn pressing_enter_inserts_a_line_break_rather_than_submitting() {
    let mut harness = Harness::new();
    harness.focus_field();

    harness.type_text("ab");
    harness.press(LogicalKey::Named(NamedKey::Enter));
    harness.type_text("cd");

    assert_eq!(
        harness.text(),
        "ab\ncd",
        "a field is multiline by default; a single-line one is what needs to opt \
         out, and there is currently no way to"
    );
}

/// This one found a real defect on its first run: 19.20px before the break and
/// 19.20px after, one line either way, while the model and the caret were both
/// correct. `cosmic-text` stores a line break as the *ending* of the line it
/// terminates, so a trailing newline opened no line to grow into — fixed in
/// `vieww-text`, which now pins it directly.
///
/// Kept as the end-to-end half: the paragraph's own tests say the *layout* has
/// two lines, and only this one says the *field* grew to hold them.
#[test]
fn a_line_break_makes_the_field_taller_because_it_really_is_two_lines() {
    let mut harness = Harness::new();
    harness.focus_field();

    harness.type_text("ab");
    let one_line = harness.field_height();

    harness.press(LogicalKey::Named(NamedKey::Enter));
    let two_lines = harness.field_height();

    // The string containing a `\n` proves the *model* took it. This proves the
    // text was laid out as two lines and the field grew to hold them, which is
    // the half a value assertion cannot reach.
    assert!(
        two_lines > one_line,
        "a second line must occupy space: {one_line} then {two_lines}"
    );
}

#[test]
fn arrow_up_from_the_second_line_lands_on_the_first() {
    let mut harness = Harness::new();
    harness.focus_field();

    harness.type_text("ab");
    harness.press(LogicalKey::Named(NamedKey::Enter));
    harness.type_text("cd");

    // The caret is after `d`, column two of line two. Up should hold that column
    // rather than the offset — an offset-based move would land two bytes back,
    // in the middle of the first line's `a`.
    harness.press(LogicalKey::Named(NamedKey::ArrowUp));
    harness.type_text("X");

    assert_eq!(
        harness.text(),
        "abX\ncd",
        "the vertical move kept its column, so the insert landed at the end of \
         the first line"
    );
}

// Cut, copy and paste. Driven through a `MemoryClipboard` in the real services
// registry, so the path under test is the whole one an application uses —
// `SharedServices` above the tree, the factory reading it, the render object
// acting on it — with only the platform's own pasteboard swapped out.

#[test]
fn copy_puts_the_selection_on_the_pasteboard_and_leaves_it_alone() {
    let mut harness = Harness::with_clipboard();
    harness.focus_field();
    harness.type_text("hello");
    harness.select_all();

    harness.shortcut("c");

    assert_eq!(harness.text(), "hello", "copy does not change the text");
    assert_eq!(harness.pasteboard(), Some("hello".to_string()));
}

#[test]
fn cut_takes_the_selection_with_it() {
    let mut harness = Harness::with_clipboard();
    harness.focus_field();
    harness.type_text("hello");
    harness.select_all();

    harness.shortcut("x");

    assert_eq!(harness.text(), "");
    assert_eq!(harness.pasteboard(), Some("hello".to_string()));
}

#[test]
fn paste_replaces_the_selection() {
    let mut harness = Harness::with_clipboard();
    harness.focus_field();
    harness.type_text("hello");
    harness.select_all();
    harness.shortcut("x");
    harness.type_text("say ");

    harness.shortcut("v");

    assert_eq!(
        harness.text(),
        "say hello",
        "paste inserts at the caret when nothing is selected"
    );
}

#[test]
fn copying_nothing_does_not_overwrite_the_pasteboard() {
    let mut harness = Harness::with_clipboard();
    harness.focus_field();
    harness.type_text("kept");
    harness.select_all();
    harness.shortcut("c");

    // Caret only, no selection. The tempting behaviour is to copy the whole
    // field; it would silently destroy what the user had.
    harness.press(LogicalKey::Named(NamedKey::ArrowRight));
    harness.type_text("more");
    harness.shortcut("c");

    assert_eq!(harness.pasteboard(), Some("kept".to_string()));
}

#[test]
fn a_field_with_no_pasteboard_leaves_the_text_alone() {
    // The ordinary state of every tree in this repository, and of any headless
    // render: no `Clipboard` was provided. The keys must do nothing rather than
    // panic or half-apply.
    let mut harness = Harness::new();
    harness.focus_field();
    harness.type_text("hello");
    harness.select_all();

    harness.shortcut("x");

    assert_eq!(harness.text(), "hello");
}

// Single line. The narrower mode, and so the one that has to be asked for.

#[test]
fn enter_on_a_single_line_field_submits_instead_of_opening_a_line() {
    let mut harness = Harness::single_line();
    harness.focus_field();
    harness.type_text("hello");

    harness.press(LogicalKey::Named(NamedKey::Enter));

    assert_eq!(
        harness.text(),
        "hello",
        "the break must not reach the value — this is what a login form needs \
         and what every field did before"
    );
    assert_eq!(harness.submissions(), vec!["hello".to_string()]);
}

#[test]
fn a_single_line_field_does_not_grow_on_enter() {
    let mut harness = Harness::single_line();
    harness.focus_field();
    harness.type_text("hello");
    let before = harness.field_height();

    harness.press(LogicalKey::Named(NamedKey::Enter));

    // The value assertion above cannot see this. A field that swallowed the
    // intent but still laid out a second line would pass that one and be wrong
    // on screen — which is the exact failure the multiline pair caught.
    assert!(
        (harness.field_height() - before).abs() < f32::EPSILON,
        "a single-line field is one line: {before} then {}",
        harness.field_height()
    );
}

#[test]
fn submitting_twice_reports_twice() {
    let mut harness = Harness::single_line();
    harness.focus_field();

    harness.type_text("one");
    harness.press(LogicalKey::Named(NamedKey::Enter));
    harness.type_text("!");
    harness.press(LogicalKey::Named(NamedKey::Enter));

    // Submitting does not clear the field — the application decides that, the
    // same way it decides every other change to the value.
    assert_eq!(
        harness.submissions(),
        vec!["one".to_string(), "one!".to_string()]
    );
}

#[test]
fn select_all_and_type_replaces_everything() {
    let mut harness = Harness::new();
    harness.focus_field();
    harness.type_text("old text");

    let shortcut = Modifiers::shortcut_for(TargetPlatform::current());
    harness.press_with(LogicalKey::Character("a".to_owned()), shortcut);
    harness.type_text("new");

    assert_eq!(harness.text(), "new");
}

#[test]
fn tab_moves_focus_rather_than_typing_a_tab() {
    let mut harness = Harness::new();
    harness.focus_field();
    let before = harness.driver.focused();

    harness.press(LogicalKey::Named(NamedKey::Tab));

    assert_eq!(
        harness.text(),
        "",
        "a tab character in the document is the bug this prevents"
    );
    // One focusable thing in this tree, so Tab wraps back to it.
    assert_eq!(harness.driver.focused(), before);
}

// ------------------------------------------------------------------------------ IME

#[test]
fn exit_test_an_input_method_composes_and_commits() {
    let mut harness = Harness::new();
    harness.focus_field();

    // Typing `k`, then `a`, towards `か`. Each preedit *replaces* the last.
    harness.ime(&ImeEvent::Enabled);
    harness.ime(&ImeEvent::preedit("k"));
    assert_eq!(
        harness.text(),
        "k",
        "provisional text is visible while composing"
    );

    harness.ime(&ImeEvent::preedit("ka"));
    assert_eq!(
        harness.text(),
        "ka",
        "a preedit replaces the one before it; appending gives `kka` and there \
         is no recovering from that"
    );

    let composing = harness.value.get().composing.expect("a composing region");
    assert_eq!(
        composing.slice(&harness.text()),
        "ka",
        "the region has to cover exactly the provisional text, since that is \
         what is drawn underlined and what the next preedit replaces"
    );

    harness.ime(&ImeEvent::commit("か"));
    assert_eq!(harness.text(), "か");
    assert!(
        harness.value.get().composing.is_none(),
        "a committed composition must stop being underlined"
    );
    assert_eq!(
        harness.value.get().selection.extent,
        "か".len(),
        "the caret belongs after the committed text"
    );
}

#[test]
fn a_composition_replaces_the_selection_it_started_in() {
    let mut harness = Harness::new();
    harness.focus_field();
    harness.type_text("abc");

    let shortcut = Modifiers::shortcut_for(TargetPlatform::current());
    harness.press_with(LogicalKey::Character("a".to_owned()), shortcut);

    harness.ime(&ImeEvent::preedit("x"));
    assert_eq!(
        harness.text(),
        "x",
        "the first preedit replaces what was selected"
    );
}

#[test]
fn an_abandoned_composition_leaves_no_underline_behind() {
    let mut harness = Harness::new();
    harness.focus_field();

    harness.ime(&ImeEvent::preedit("ka"));
    harness.ime(&ImeEvent::Disabled);

    assert_eq!(harness.text(), "ka", "what was typed is still there");
    assert!(
        harness.value.get().composing.is_none(),
        "but nothing owns it any more, and a stale region underlines whatever \
         moves into those bytes"
    );
}

#[test]
fn autocomplete_replacing_a_word_is_the_same_mechanism() {
    // A mobile keyboard correcting `teh` to `the` is a preedit spanning the
    // word followed by a commit — exactly what composition already does, which
    // is why it comes out working for free.
    let mut harness = Harness::new();
    harness.focus_field();

    harness.ime(&ImeEvent::preedit("teh"));
    harness.ime(&ImeEvent::commit("the"));

    assert_eq!(harness.text(), "the");
    assert!(harness.value.get().composing.is_none());
}

// --------------------------------------------------------------------- backgrounding

#[test]
fn exit_test_backgrounding_loses_neither_the_text_nor_the_focus() {
    let mut harness = Harness::new();
    harness.focus_field();
    harness.type_text("half typed");
    harness.press(LogicalKey::Named(NamedKey::ArrowLeft));

    let text = harness.text();
    let selection = harness.value.get().selection;
    let focused = harness.driver.focused();

    // What a suspend does to the framework: the surface is gone and every live
    // pointer is cancelled, and then a resume brings a surface back — possibly
    // a differently sized one, since the device may have rotated.
    harness
        .driver
        .handle_pointer(&PointerEvent::cancel(POINTER, Offset::ZERO, ms(100)));
    harness.driver.invalidate();
    harness.driver.resize(Size::new(300.0, 400.0));
    harness.driver.draw_frame();

    assert_eq!(harness.text(), text, "the text survived");
    assert_eq!(
        harness.value.get().selection,
        selection,
        "and so did the caret, which is what makes it possible to carry on"
    );
    assert_eq!(
        harness.driver.focused(),
        focused,
        "and the field still has the keyboard"
    );

    // And it is still usable, which is the part that a preserved-but-detached
    // state would fail.
    harness.type_text("!");
    assert!(harness.text().contains('!'));
}

#[test]
fn a_resize_republishes_the_view_metrics_rather_than_stranding_them() {
    let mut harness = Harness::new();
    harness.driver.set_view_metrics(ViewMetrics {
        size: SURFACE,
        device_pixel_ratio: 3.0,
        safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
        view_insets: EdgeInsets::ZERO,
    });
    harness.driver.draw_frame();

    harness.driver.resize(Size::new(300.0, 400.0));
    harness.driver.draw_frame();

    let metrics = harness.driver.view_metrics();
    assert_eq!(
        metrics.size,
        Size::new(300.0, 400.0),
        "the size follows the surface"
    );
    assert_eq!(
        metrics.safe_area.top, 47.0,
        "and the notch does not evaporate when the device rotates"
    );
}

#[test]
fn a_safe_area_insets_its_child_by_what_the_platform_reported() {
    let mut driver = FrameDriver::new(SURFACE);

    driver
        .set_root(SafeArea::new().child(ColoredBox::new(Color::RED).child(SizedBox::square(10.0))));
    driver.set_view_metrics(ViewMetrics {
        size: SURFACE,
        device_pixel_ratio: 1.0,
        safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
        view_insets: EdgeInsets::ZERO,
    });
    driver.draw_frame();

    // The red box is inset from the top by the notch.
    let fill = driver
        .scene()
        .fills()
        .into_iter()
        .find(|(_, paint)| paint.color == Color::RED)
        .expect("the box is drawn");
    assert_eq!(
        fill.0.top, 47.0,
        "a title drawn under the notch is the bug SafeArea exists to prevent"
    );
}
