//! The rest of the Phase 5 exit test: a caret placed where the text was tapped,
//! a selection dragged out, and text edited around both.
//!
//! ```console
//! cargo test -p vieww --test tap_to_caret
//! ```
//!
//! # What the roadmap asks for
//!
//! > render a paragraph with mixed bold/italic runs, wrap correctly at width,
//! > place a blinking cursor at a tapped position, and successfully compose an
//! > IME input (e.g. Japanese or emoji picker) on-device.
//!
//! The first two clauses were met in Phase 5's first half and are asserted on a
//! real GPU by `text_to_pixels.rs`. This file is the third: **a blinking cursor
//! at a tapped position**, driven through the whole stack rather than against
//! `Paragraph` directly — a widget tree built and mounted, laid out, hit tested,
//! and synthetic pointer events routed to the render object the hit test found.
//! `vieww-text`'s own tests cover the geometry; this covers the wiring, which is
//! where a caret that is correct in a unit test still lands in the wrong place in
//! an app.
//!
//! The fourth clause — composing an IME input **on-device** — is still not met
//! and still cannot be. An input method is an OS service reached through the
//! Phase 8 platform bridges, and there is no window to receive one into. What
//! exists here is the half that does not need the OS: the composing region is
//! part of [`TextEditingValue`], survives editing, is drawn underlined, and is
//! cleared by the things that should clear it. When the bridge lands it has a
//! model to talk to rather than one to invent.
//!
//! # Why the value lives in a signal
//!
//! `TextField` is controlled: it is handed a value and reports the value it
//! would like to become. Something above it has to hold the real one and be
//! rebuilt when it changes, and a `Signal` is exactly that — see the type's own
//! documentation for why it cannot hold its own state. That is also what makes
//! this test honest about cost: writing the signal marks only the elements that
//! read it, so a keystroke rebuilds the field and not the page.

use std::rc::Rc;
use std::time::Duration;

use vieww::foundation::{Offset, PointerEvent, PointerId, Size, TextPosition};
use vieww::prelude::*;
use vieww::{Affinity, FrameDriver, TextEditingValue, TextRange, TextSelection};

const SURFACE: f32 = 400.0;
const FONT: f32 = 20.0;
const POINTER: PointerId = PointerId(1);

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// The field under test, reading its value from the signal above it.
///
/// `show_cursor` is a second signal rather than a timer inside the field: a
/// blink is an animation, and this framework's animations are driven by the
/// frame, not by a widget reading a clock (`docs/DESIGN.md` §15).
fn field(value: &Signal<TextEditingValue>, blink: &Signal<bool>, width: f32) -> WidgetNode {
    let sink = value.clone();
    Center::new()
        .child(
            SizedBox::width(width).child(
                TextField::new(value.get())
                    .size(FONT)
                    .show_cursor(blink.get())
                    .on_changed(Rc::new(move |next| sink.set(next))),
            ),
        )
        .into()
}

/// A driver with a field mounted and one frame drawn.
struct Harness {
    driver: FrameDriver,
    value: Signal<TextEditingValue>,
    blink: Signal<bool>,
    width: f32,
}

impl Harness {
    fn new(text: &str) -> Self {
        Self::wrapped(text, SURFACE)
    }

    /// A field constrained to `width`, so the text wraps.
    fn wrapped(text: &str, width: f32) -> Self {
        let mut driver = FrameDriver::new(Size::square(SURFACE));
        let value = driver
            .elements()
            .runtime()
            .signal(TextEditingValue::new(text));
        let blink = driver.elements().runtime().signal(true);

        let mut harness = Self {
            driver,
            value,
            blink,
            width,
        };
        harness.rebuild();
        harness
    }

    fn rebuild(&mut self) {
        let root = field(&self.value, &self.blink, self.width);
        self.driver.elements().set_root(root);
        self.driver.draw_frame();
    }

    fn send(&mut self, event: &PointerEvent) {
        self.driver.handle_pointer(event);
        self.rebuild();
    }

    fn tap(&mut self, at: Offset) {
        self.send(&PointerEvent::down(POINTER, at, ms(0)));
        self.send(&PointerEvent::up(POINTER, at, ms(20)));
    }

    /// A press, a series of moves and a release — a selection drag.
    fn drag(&mut self, from: Offset, to: Offset, steps: u32) {
        self.send(&PointerEvent::down(POINTER, from, ms(0)));
        let mut previous = from;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            let position = Offset::new(
                from.dx + (to.dx - from.dx) * t,
                from.dy + (to.dy - from.dy) * t,
            );
            self.send(&PointerEvent::moved(
                POINTER,
                previous,
                position,
                ms(u64::from(step) * 16),
            ));
            previous = position;
        }
        self.send(&PointerEvent::up(
            POINTER,
            to,
            ms(u64::from(steps) * 16 + 16),
        ));
    }

    fn value(&self) -> TextEditingValue {
        self.value.get()
    }

    /// Apply an edit the way a key handler will once Phase 8 provides one.
    fn edit(&mut self, apply: impl FnOnce(&mut TextEditingValue)) {
        let mut value = self.value.get();
        apply(&mut value);
        self.value.set(value);
        self.rebuild();
    }

    /// The mounted render object, as its own type.
    fn editable(&self) -> &vieww::render::RenderEditableText {
        let tree = self.driver.owner().tree();
        let id = tree
            .ids()
            .into_iter()
            .find(|&id| {
                tree.object(id)
                    .is_some_and(|object| object.debug_name() == "RenderEditableText")
            })
            .expect("the field is mounted");
        let object: &dyn std::any::Any = tree.object(id).expect("mounted");
        object
            .downcast_ref::<vieww::render::RenderEditableText>()
            .expect("its own type")
    }

    /// Where on screen the caret for `offset` is drawn, in global coordinates.
    fn caret_point(&self, offset: usize) -> Offset {
        let origin = self.field_origin();
        let paragraph = self.editable().shaped().expect("laid out");
        let rect = paragraph.cursor_rect(TextPosition::new(offset));
        Offset::new(
            origin.dx + rect.left,
            origin.dy + (rect.top + rect.bottom) / 2.0,
        )
    }

    fn field_origin(&self) -> Offset {
        let tree = self.driver.owner().tree();
        let id = tree
            .ids()
            .into_iter()
            .find(|&id| {
                tree.object(id)
                    .is_some_and(|object| object.debug_name() == "RenderEditableText")
            })
            .expect("the field is mounted");
        tree.global_offset(id)
    }

    /// How many filled rectangles the frame drew — the caret and any highlight.
    fn fills(&mut self) -> usize {
        let mut scene = vieww::Scene::new();
        self.driver.owner().tree().paint(&mut scene);
        scene.fills().len()
    }
}

// ------------------------------------------------ the roadmap's third clause

#[test]
fn tapping_the_text_puts_the_caret_where_it_was_tapped() {
    let mut harness = Harness::new("hello world");

    // Tap exactly where the caret for offset 6 — the w of "world" — would draw.
    let target = harness.caret_point(6);
    harness.tap(target);

    let value = harness.value();
    assert!(
        value.selection.is_collapsed(),
        "a tap places a caret, not a selection: {:?}",
        value.selection
    );
    assert_eq!(
        value.selection.extent, 6,
        "the caret landed where the finger did, through the whole stack"
    );
    assert_eq!(value.text, "hello world", "and a tap edits nothing");
}

#[test]
fn tapping_a_different_place_moves_the_caret_there() {
    let mut harness = Harness::new("hello world");

    harness.tap(harness.caret_point(2));
    assert_eq!(harness.value().selection.extent, 2);

    harness.tap(harness.caret_point(9));
    assert_eq!(
        harness.value().selection.extent,
        9,
        "the second tap replaces the first caret rather than extending from it"
    );
}

#[test]
fn tapping_past_the_end_of_the_text_puts_the_caret_at_the_end() {
    let mut harness = Harness::new("hello");

    // Move the caret off the end first. A new value already has it there, so
    // without this the assertion below is true before the tap even happens.
    harness.tap(harness.caret_point(1));
    assert_eq!(harness.value().selection.extent, 1);

    // The empty space to the right of the last character — where a user aiming
    // for "the end" actually taps — and still inside the field, since a tap
    // outside its bounds hits nothing and changes nothing.
    let line = harness
        .editable()
        .shaped()
        .expect("laid out")
        .line_rect(0)
        .expect("a first line");
    let origin = harness.field_origin();
    harness.tap(Offset::new(
        origin.dx + line.right + 50.0,
        origin.dy + FONT / 2.0,
    ));

    assert_eq!(harness.value().selection.extent, 5);
}

#[test]
fn tapping_an_empty_field_is_not_an_error() {
    let mut harness = Harness::new("");
    let origin = harness.field_origin();

    harness.tap(Offset::new(origin.dx + 20.0, origin.dy + FONT / 2.0));

    assert_eq!(harness.value().selection, TextSelection::collapsed(0));
    assert!(
        harness.fills() > 0,
        "an empty field still draws a caret to type at"
    );
}

#[test]
fn the_caret_is_drawn_and_the_blink_turns_it_off() {
    let mut harness = Harness::new("hello");
    harness.tap(harness.caret_point(2));

    assert_eq!(harness.fills(), 1, "the caret, and nothing else");

    // The off phase of a blink: one signal write, and the text stays put.
    harness.blink.set(false);
    harness.rebuild();
    assert_eq!(harness.fills(), 0, "the caret is off");

    let mut scene = vieww::Scene::new();
    harness.driver.owner().tree().paint(&mut scene);
    assert_eq!(
        scene.glyph_runs().len(),
        1,
        "blinking must not take the text with it — the shaped paragraph has to \
         survive a rebuild that changes only whether the caret is drawn"
    );

    harness.blink.set(true);
    harness.rebuild();
    assert_eq!(harness.fills(), 1, "and back on");
}

// ------------------------------------------------------------------ selection

#[test]
fn dragging_across_the_text_selects_it() {
    let mut harness = Harness::new("hello world");

    harness.drag(harness.caret_point(0), harness.caret_point(5), 5);

    let value = harness.value();
    assert!(!value.selection.is_collapsed(), "{:?}", value.selection);
    assert_eq!(value.selected_text(), "hello");
}

#[test]
fn a_selection_drag_that_reverses_keeps_its_anchor() {
    let mut harness = Harness::new("hello world");

    harness.drag(harness.caret_point(6), harness.caret_point(2), 4);

    let value = harness.value();
    assert_eq!(
        value.selection.base, 6,
        "the anchor is where the finger went down"
    );
    assert_eq!(value.selection.extent, 2);
    assert!(value.selection.is_reversed());
    // 2..6 of "hello world" — the anchor is exclusive, as the end of any
    // half-open range is.
    assert_eq!(value.selected_text(), "llo ");
}

#[test]
fn a_selection_is_painted_behind_the_text() {
    let mut harness = Harness::new("hello world");
    harness.drag(harness.caret_point(0), harness.caret_point(5), 5);

    assert!(
        harness.fills() >= 2,
        "a highlight and a caret, not just a caret"
    );
}

#[test]
fn selecting_across_a_wrap_highlights_both_lines() {
    // Narrow enough that "hello world" cannot fit on one line.
    let mut harness = Harness::wrapped("hello world", 80.0);
    assert!(
        harness.editable().shaped().expect("laid out").line_count() > 1,
        "the field has to wrap for this to mean anything"
    );

    harness.edit(|value| value.select_all());

    assert!(
        harness.fills() >= 3,
        "one highlight per line plus the caret: {}",
        harness.fills()
    );
}

#[test]
fn tapping_the_second_line_of_wrapped_text_lands_on_it() {
    let mut harness = Harness::wrapped("hello world", 80.0);
    let paragraph = harness.editable().shaped().expect("laid out").clone();
    assert!(paragraph.line_count() > 1);

    let last = paragraph
        .line_rect(paragraph.line_count() - 1)
        .expect("a last line");
    let origin = harness.field_origin();
    harness.tap(Offset::new(
        origin.dx + last.left + 1.0,
        origin.dy + (last.top + last.bottom) / 2.0,
    ));

    let offset = harness.value().selection.extent;
    assert!(
        offset > 0,
        "a tap on the second line is not the start of the text"
    );
    assert_eq!(
        paragraph.line_of(TextPosition::new(offset)),
        paragraph.line_count() - 1,
        "and it belongs to the line that was tapped"
    );
}

#[test]
fn tapping_the_end_of_a_wrapped_line_keeps_the_caret_on_that_line() {
    let mut harness = Harness::wrapped("abcdefghijklmnopqrst", 80.0);
    let paragraph = harness.editable().shaped().expect("laid out").clone();
    assert!(paragraph.line_count() > 1);

    // Past the last glyph on the first line but still *inside* the field, which
    // is only 80 wide: a tap outside its bounds hits nothing at all, and the
    // assertion below would then be reading the selection the field started with.
    let first = paragraph.line_rect(0).expect("a first line");
    let origin = harness.field_origin();
    harness.tap(Offset::new(
        origin.dx + first.right + 1.0,
        origin.dy + (first.top + first.bottom) / 2.0,
    ));

    let selection = harness.value().selection;
    assert_eq!(
        selection.affinity,
        Affinity::Upstream,
        "the offset at a wrap belongs to both lines; the tap says which"
    );
    assert_eq!(
        paragraph.line_of(selection.cursor()),
        0,
        "so the caret draws at the end of the line that was tapped, not the \
         start of the one below"
    );
}

// -------------------------------------------------------------------- editing

#[test]
fn typing_at_a_tapped_caret_inserts_there() {
    let mut harness = Harness::new("helo world");

    harness.tap(harness.caret_point(3));
    harness.edit(|value| value.insert("l"));

    let value = harness.value();
    assert_eq!(value.text, "hello world");
    assert_eq!(
        value.selection,
        TextSelection::collapsed(4),
        "the caret followed what was typed"
    );
}

#[test]
fn typing_over_a_dragged_selection_replaces_it() {
    let mut harness = Harness::new("hello world");

    harness.drag(harness.caret_point(6), harness.caret_point(11), 5);
    harness.edit(|value| value.insert("there"));

    assert_eq!(harness.value().text, "hello there");
}

#[test]
fn backspace_at_a_tapped_caret_deletes_the_character_before_it() {
    let mut harness = Harness::new("helllo");

    harness.tap(harness.caret_point(4));
    harness.edit(TextEditingValue::delete_backward);

    assert_eq!(harness.value().text, "hello");
    assert_eq!(harness.value().selection, TextSelection::collapsed(3));
}

#[test]
fn an_edit_reflows_the_text_that_is_drawn() {
    let mut harness = Harness::new("hi");
    let before = harness.editable().shaped().expect("laid out").glyph_count();

    harness.edit(|value| value.insert(" there"));
    let after = harness.editable().shaped().expect("laid out").glyph_count();

    assert!(
        after > before,
        "typing has to re-shape, or the caret and the glyphs disagree: {after} vs {before}"
    );
}

#[test]
fn the_caret_moves_with_the_text_it_is_in() {
    let mut harness = Harness::new("hello");
    harness.tap(harness.caret_point(5));
    let at_end = harness.editable().cursor_rect().expect("laid out");

    harness.edit(|value| value.insert("!!!"));
    let after = harness.editable().cursor_rect().expect("laid out");

    assert!(
        after.left > at_end.left,
        "the caret is drawn against the new layout, not the old one: \
         {} vs {}",
        after.left,
        at_end.left
    );
}

// ------------------------------------------------------- the IME model, alone

#[test]
fn a_composing_region_is_drawn_and_survives_until_it_is_committed() {
    let mut harness = Harness::new("");

    // What a bridge will do: put the provisional text in so it can be seen, and
    // mark it as still being decided.
    harness.edit(|value| {
        value.insert("ka");
        value.set_composing(Some(TextRange::new(0, 2)));
    });
    assert_eq!(harness.value().composing, Some(TextRange::new(0, 2)));
    assert!(
        harness.fills() >= 2,
        "an underline as well as the caret: provisional text has to look it"
    );

    // And what it does when the user accepts a candidate.
    harness.edit(|value| value.replace_range(TextRange::new(0, 2), "か"));
    let value = harness.value();
    assert_eq!(value.text, "か");
    assert_eq!(
        value.composing, None,
        "the bytes it covered are gone; underlining what replaced them is wrong"
    );
    assert_eq!(value.selection, TextSelection::collapsed("か".len()));
}

#[test]
fn moving_the_caret_ends_a_composition() {
    let mut harness = Harness::new("kana");
    harness.edit(|value| value.set_composing(Some(TextRange::new(0, 4))));
    assert!(harness.value().composing.is_some());

    harness.tap(harness.caret_point(2));

    assert_eq!(
        harness.value().composing,
        None,
        "the user navigated away from what the input method was deciding about"
    );
}

// ------------------------------------------------------------------- the cost

#[test]
fn a_read_only_field_takes_no_pointers() {
    let mut driver = FrameDriver::new(Size::square(SURFACE));
    let root: WidgetNode = Center::new()
        .child(TextField::text("hello").size(FONT))
        .into();
    driver.elements().set_root(root);
    driver.draw_frame();

    let tree = driver.owner().tree();
    let id = tree
        .ids()
        .into_iter()
        .find(|&id| {
            tree.object(id)
                .is_some_and(|object| object.debug_name() == "RenderEditableText")
        })
        .expect("mounted");
    assert!(
        tree.object(id)
            .expect("mounted")
            .gesture_recognizers()
            .is_empty(),
        "a label-shaped field must not swallow the drag that should scroll the \
         list it sits in"
    );
}

/// A caret blinking at 2 Hz must not shape the paragraph twice a second.
#[test]
fn blinking_does_not_reshape_the_paragraph() {
    let mut harness = Harness::new("hello world");
    harness.tap(harness.caret_point(3));

    let before = harness.editable().shaped().expect("laid out").clone();

    harness.blink.set(false);
    harness.rebuild();
    harness.blink.set(true);
    harness.rebuild();

    let after = harness.editable().shaped().expect("laid out");
    assert_eq!(
        before.glyph_count(),
        after.glyph_count(),
        "the same paragraph, carried across the rebuild rather than rebuilt"
    );
    assert_eq!(
        before.cursor_rect(TextPosition::new(3)).left,
        after.cursor_rect(TextPosition::new(3)).left,
    );
}
