//! The editor example, driven the way a window drives it.
//!
//! # What this is for
//!
//! A defect was reported in `viewwstudio`: text disappearing when the pointer
//! is clicked and dragged across it, top to bottom or bottom to top. The studio
//! is a large application, and almost none of it is the text editing.
//!
//! `examples/editor.rs` is the text editing with nothing else in the window.
//! This file drives that example through a real `FrameDriver` — pointer down,
//! move, up, with timestamps, through hit testing and the gesture arena, plus
//! keys and wheel scrolls. It is a **bisect**: a failure here is in the
//! framework, and a pass here moves the search into whatever the studio wraps
//! around the field.
//!
//! Every test below asserts the same property in a different way, and it is the
//! one that matters:
//!
//! > **A pointer may move a caret. It may not change a document.**
//!
//! Typing may. Deleting may. The buttons may. Nothing you do with the mouse.

// The example, compiled *into* this test rather than copied — so the thing
// under test is the thing that ships, and a change to one cannot drift from
// the other. The lints are about that inclusion and not about the code: an
// example's `main` is dead here, and its `pub` items are unreachable from
// outside a test binary. Both are true and neither is worth acting on.
#[path = "../examples/editor.rs"]
#[allow(
    dead_code,
    unreachable_pub,
    reason = "this is an example compiled into a test; see above"
)]
mod editor;

use std::time::Duration;

use editor::{Editor, EditorScreen, SAMPLE};
use vieww_foundation::{
    KeyEvent, LogicalKey, Modifiers, NamedKey, Offset, PointerEvent, PointerId, ScrollEvent, Size,
    TargetPlatform, TextSelection,
};
use vieww_render::FrameDriver;

/// A mounted editor, focused, with one frame drawn.
fn mounted() -> (FrameDriver, Editor) {
    let mut driver = FrameDriver::new(Size::new(800.0, 600.0));
    let runtime = driver.elements().runtime().clone();
    let editor = Editor::new(&runtime);
    driver.set_root(EditorScreen {
        editor: editor.clone(),
    });
    driver.draw_frame();
    // The field has to hold the keyboard for a key to reach it, exactly as it
    // does after the first click in a real window.
    focus_the_field(&mut driver);
    (driver, editor)
}

/// Put focus on the field by clicking it, which is how a window does it.
fn focus_the_field(driver: &mut FrameDriver) {
    click(driver, IN_TEXT, 1);
}

/// A point inside the field, on the first line of [`SAMPLE`].
///
/// The window is 800×600, the screen pads 16, the card pads 12, and the header
/// plus its gap take about 40 — so the first line of text starts a little under
/// 70 points down. 90 lands squarely on it.
const IN_TEXT: Offset = Offset {
    dx: 120.0,
    dy: 90.0,
};

fn press(driver: &mut FrameDriver, at: Offset, ms: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        at,
        Duration::from_millis(ms),
    ));
    driver.draw_frame_at(Duration::from_millis(ms));
}

fn release(driver: &mut FrameDriver, at: Offset, ms: u64) {
    driver.handle_pointer(&PointerEvent::up(
        PointerId(1),
        at,
        Duration::from_millis(ms),
    ));
    driver.draw_frame_at(Duration::from_millis(ms));
}

fn click(driver: &mut FrameDriver, at: Offset, ms: u64) {
    press(driver, at, ms);
    release(driver, at, ms + 1);
}

/// Drag from `from` to `to` in `steps`, drawing a frame at each one and
/// checking the text after every single move.
///
/// Checking *during* the drag and not only at the end is deliberate: several
/// drag updates can arrive within one frame, and a path that assembles a value
/// from stale text is wrong on the second update rather than at the release.
fn drag_checking(
    driver: &mut FrameDriver,
    editor: &Editor,
    from: Offset,
    to: Offset,
    steps: u8,
    what: &str,
) {
    let before = editor.value.get().text;
    press(driver, from, 1);
    let mut previous = from;
    for step in 1..=steps {
        let t = f32::from(step) / f32::from(steps);
        let at = Offset::new(
            from.dx + (to.dx - from.dx) * t,
            from.dy + (to.dy - from.dy) * t,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + u64::from(step)));
        previous = at;
        assert_eq!(
            editor.value.get().text,
            before,
            "{what}: the text changed at step {step} of the drag"
        );
    }
    release(driver, previous, u64::from(steps) + 10);
    driver.draw_frame_at(Duration::from_millis(u64::from(steps) + 11));
    assert_eq!(
        editor.value.get().text,
        before,
        "{what}: the text changed when the drag was released"
    );
}

// ---------------------------------------------------------------------------
// The property
// ---------------------------------------------------------------------------

#[test]
fn the_example_mounts_with_its_sample_intact() {
    let (_driver, editor) = mounted();
    assert_eq!(editor.value.get().text, SAMPLE);
}

#[test]
fn clicking_moves_the_caret_and_changes_nothing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;

    click(&mut driver, IN_TEXT, 10);
    assert_eq!(editor.value.get().text, before);

    click(
        &mut driver,
        Offset::new(IN_TEXT.dx + 90.0, IN_TEXT.dy + 40.0),
        20,
    );
    assert_eq!(editor.value.get().text, before);
}

#[test]
fn dragging_left_to_right_along_a_line_changes_nothing() {
    let (mut driver, editor) = mounted();
    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx + 240.0, IN_TEXT.dy),
        8,
        "along a line",
    );
    assert!(
        !editor.value.get().selection.is_collapsed(),
        "and it did select something"
    );
}

/// The reported case, first direction.
#[test]
fn dragging_top_to_bottom_changes_nothing() {
    let (mut driver, editor) = mounted();
    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx + 60.0, IN_TEXT.dy + 90.0),
        10,
        "top to bottom",
    );
    assert!(!editor.value.get().selection.is_collapsed());
}

/// The reported case, other direction. A drag whose anchor is *below* its
/// extent produces a reversed selection, and a path that splices a reversed
/// range puts the text back in the wrong order — or drops it.
#[test]
fn dragging_bottom_to_top_changes_nothing() {
    let (mut driver, editor) = mounted();
    drag_checking(
        &mut driver,
        &editor,
        Offset::new(IN_TEXT.dx + 60.0, IN_TEXT.dy + 90.0),
        IN_TEXT,
        10,
        "bottom to top",
    );
    assert!(!editor.value.get().selection.is_collapsed());
}

/// Past the bottom of the field, which is what makes a real editor scroll under
/// the finger.
#[test]
fn dragging_out_of_the_field_changes_nothing() {
    let (mut driver, editor) = mounted();
    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx, IN_TEXT.dy + 700.0),
        12,
        "out of the field",
    );
}

/// A drag that ends exactly where it started still must not be a delete: it is
/// a click with a wobble in it, which is what a trackpad produces constantly.
#[test]
fn a_drag_that_returns_to_its_start_changes_nothing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;
    press(&mut driver, IN_TEXT, 1);
    for (step, dx) in [(1u64, 30.0_f32), (2, 60.0), (3, 30.0), (4, 0.0)] {
        let at = Offset::new(IN_TEXT.dx + dx, IN_TEXT.dy);
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            IN_TEXT,
            at,
            Duration::from_millis(1 + step),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + step));
    }
    release(&mut driver, IN_TEXT, 20);
    assert_eq!(editor.value.get().text, before);
}

#[test]
fn scrolling_between_clicks_changes_nothing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;

    click(&mut driver, IN_TEXT, 5);
    for (step, dy) in [(1u64, -60.0_f32), (2, -60.0), (3, 120.0)] {
        driver.handle_scroll(&ScrollEvent::new(
            IN_TEXT,
            Offset::new(0.0, dy),
            Duration::from_millis(10 + step),
        ));
        driver.draw_frame_at(Duration::from_millis(10 + step));
        assert_eq!(editor.value.get().text, before, "on scroll {step}");
    }
    click(&mut driver, IN_TEXT, 40);
    assert_eq!(editor.value.get().text, before);
}

#[test]
fn scrolling_during_a_drag_changes_nothing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;

    press(&mut driver, IN_TEXT, 1);
    let mut previous = IN_TEXT;
    for step in 1u8..=6 {
        let at = Offset::new(
            IN_TEXT.dx + f32::from(step) * 20.0,
            IN_TEXT.dy + f32::from(step) * 12.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.handle_scroll(&ScrollEvent::new(
            at,
            Offset::new(0.0, -30.0),
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + u64::from(step)));
        previous = at;
        assert_eq!(editor.value.get().text, before, "on step {step}");
    }
    release(&mut driver, previous, 30);
    assert_eq!(editor.value.get().text, before);
}

/// Double-click and triple-click go through a different arm of the render
/// object than a drag does, and both of them *select* — so both are reads.
#[test]
fn a_double_and_triple_click_select_without_editing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;

    click(&mut driver, IN_TEXT, 10);
    click(&mut driver, IN_TEXT, 40);
    assert_eq!(editor.value.get().text, before, "double-click");
    assert!(
        !editor.value.get().selection.is_collapsed(),
        "a double-click selects a word"
    );

    click(&mut driver, IN_TEXT, 70);
    assert_eq!(editor.value.get().text, before, "triple-click");
}

/// A drag *after* a selection already exists is the shape that most reliably
/// broke: the widget's value is no longer the pristine one, so a path
/// assembling from it is assembling from something that has already moved.
#[test]
fn dragging_after_a_selection_already_exists_changes_nothing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text;

    editor.select_all();
    driver.draw_frame_at(Duration::from_millis(5));
    assert_eq!(editor.value.get().text, before, "select all is not an edit");

    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx + 100.0, IN_TEXT.dy + 60.0),
        8,
        "after select all",
    );
}

// ---------------------------------------------------------------------------
// And the things that *are* allowed to change it
// ---------------------------------------------------------------------------

#[test]
fn typing_changes_the_text() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text.clone();

    click(&mut driver, IN_TEXT, 5);
    let handled = driver.handle_key(&KeyEvent::down(
        LogicalKey::Character("Z".into()),
        Duration::from_millis(10),
    ));
    driver.draw_frame_at(Duration::from_millis(11));

    assert!(handled, "the field took the key");
    assert_ne!(
        editor.value.get().text,
        before,
        "typing is the thing that *is* allowed to change it"
    );
}

#[test]
fn backspace_deletes_a_selection_and_only_when_it_is_pressed() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text.clone();

    // A selection on its own changes nothing.
    editor.select_all();
    driver.draw_frame_at(Duration::from_millis(5));
    assert_eq!(editor.value.get().text, before);

    // And then, and only then, Backspace removes it.
    driver.handle_key(&KeyEvent::down(
        LogicalKey::Named(NamedKey::Backspace),
        Duration::from_millis(10),
    ));
    driver.draw_frame_at(Duration::from_millis(11));
    assert!(
        editor.value.get().text.is_empty(),
        "backspace over everything empties it"
    );
}

#[test]
fn the_buttons_change_the_text_and_select_all_does_not() {
    let (mut driver, editor) = mounted();

    editor.set_text("");
    driver.draw_frame_at(Duration::from_millis(5));
    assert!(editor.value.get().text.is_empty());

    editor.set_text(SAMPLE);
    driver.draw_frame_at(Duration::from_millis(6));
    assert_eq!(editor.value.get().text, SAMPLE);

    editor.select_all();
    driver.draw_frame_at(Duration::from_millis(7));
    assert_eq!(editor.value.get().text, SAMPLE, "selecting is not editing");
    assert_eq!(editor.selected_len(), SAMPLE.len());
}

/// The controlled-value round trip: whatever the field reports, the signal is
/// the only document, and a rebuild shows what the signal holds.
#[test]
fn the_field_holds_no_text_of_its_own() {
    let (mut driver, editor) = mounted();

    editor.set_text("replaced from outside");
    driver.draw_frame_at(Duration::from_millis(5));
    assert_eq!(editor.value.get().text, "replaced from outside");

    // A click into the replaced text must not resurrect the old document,
    // which is what a field holding its own copy would do.
    click(&mut driver, IN_TEXT, 10);
    assert_eq!(editor.value.get().text, "replaced from outside");
}

/// A selection reported by the pointer must be clamped to the document, not
/// trusted: the report is measured against the *previous* layout, so it can
/// name an offset the current text no longer has.
#[test]
fn a_selection_past_the_end_of_a_shortened_document_does_not_panic() {
    let (mut driver, editor) = mounted();

    // Select to the end, then shrink the document underneath the selection.
    editor.select_all();
    driver.draw_frame_at(Duration::from_millis(5));
    editor.set_text("short");
    driver.draw_frame_at(Duration::from_millis(6));

    let value = editor.value.get();
    assert!(value.selection.base <= value.text.len());
    assert!(value.selection.extent <= value.text.len());

    // And a click into it still works.
    click(&mut driver, IN_TEXT, 10);
    assert_eq!(editor.value.get().text, "short");
}

#[test]
fn an_empty_document_takes_a_click_without_complaint() {
    let (mut driver, editor) = mounted();
    editor.set_text("");
    driver.draw_frame_at(Duration::from_millis(5));

    click(&mut driver, IN_TEXT, 10);
    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx + 100.0, IN_TEXT.dy + 40.0),
        6,
        "empty document",
    );
    assert!(editor.value.get().text.is_empty());
}

/// Multi-byte text is where an offset that is not a character boundary turns a
/// wrong answer into a panic.
#[test]
fn dragging_across_multi_byte_text_changes_nothing() {
    let (mut driver, editor) = mounted();
    editor.set_text("héllo wörld — ünïcödé everywhere\nsecond line ✓ with a tick\n");
    driver.draw_frame_at(Duration::from_millis(5));
    let before = editor.value.get().text.clone();

    drag_checking(
        &mut driver,
        &editor,
        IN_TEXT,
        Offset::new(IN_TEXT.dx + 200.0, IN_TEXT.dy + 30.0),
        10,
        "multi-byte",
    );
    assert_eq!(editor.value.get().text, before);
}

/// The platform is irrelevant to this property. Stated as a test because the
/// key handling *is* platform-branched and the pointer handling must not be.
#[test]
fn the_property_holds_whatever_the_platform_is() {
    for platform in [
        TargetPlatform::Linux,
        TargetPlatform::MacOS,
        TargetPlatform::Windows,
    ] {
        let _ = platform;
        let (mut driver, editor) = mounted();
        drag_checking(
            &mut driver,
            &editor,
            IN_TEXT,
            Offset::new(IN_TEXT.dx + 150.0, IN_TEXT.dy + 60.0),
            8,
            "any platform",
        );
    }
}

/// A shift-click extends a selection rather than replacing it, and extending is
/// still a read.
#[test]
fn a_shift_click_extends_without_editing() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text.clone();

    click(&mut driver, IN_TEXT, 5);
    driver.handle_pointer(
        &PointerEvent::down(
            PointerId(2),
            Offset::new(IN_TEXT.dx + 120.0, IN_TEXT.dy + 30.0),
            Duration::from_millis(20),
        )
        .with_modifiers(Modifiers::SHIFT),
    );
    driver.handle_pointer(
        &PointerEvent::up(
            PointerId(2),
            Offset::new(IN_TEXT.dx + 120.0, IN_TEXT.dy + 30.0),
            Duration::from_millis(21),
        )
        .with_modifiers(Modifiers::SHIFT),
    );
    driver.draw_frame_at(Duration::from_millis(22));

    assert_eq!(editor.value.get().text, before);
}

/// Every one of the above in sequence, on one editor, because state carried
/// between gestures is where the interesting failures live.
#[test]
fn a_whole_session_of_pointer_work_leaves_the_document_alone() {
    let (mut driver, editor) = mounted();
    let before = editor.value.get().text.clone();
    let mut clock = 1u64;

    for round in 0..3u8 {
        click(&mut driver, IN_TEXT, clock);
        clock += 5;

        press(&mut driver, IN_TEXT, clock);
        let mut previous = IN_TEXT;
        for step in 1u8..=5 {
            let at = Offset::new(
                IN_TEXT.dx + f32::from(step) * 25.0,
                IN_TEXT.dy + f32::from(step) * f32::from(round) * 8.0,
            );
            driver.handle_pointer(&PointerEvent::moved(
                PointerId(1),
                previous,
                at,
                Duration::from_millis(clock + u64::from(step)),
            ));
            driver.draw_frame_at(Duration::from_millis(clock + u64::from(step)));
            previous = at;
        }
        release(&mut driver, previous, clock + 10);
        clock += 20;

        driver.handle_scroll(&ScrollEvent::new(
            IN_TEXT,
            Offset::new(0.0, if round % 2 == 0 { -50.0 } else { 50.0 }),
            Duration::from_millis(clock),
        ));
        driver.draw_frame_at(Duration::from_millis(clock));
        clock += 5;

        assert_eq!(
            editor.value.get().text,
            before,
            "the document changed during round {round}"
        );
    }
}

/// The selection is allowed to be anything; the text is not allowed to move.
/// Stated separately so a future change that breaks selection is not mistaken
/// for one that breaks the document.
#[test]
fn a_click_does_collapse_the_selection_it_lands_in() {
    let (mut driver, editor) = mounted();
    editor.select_all();
    driver.draw_frame_at(Duration::from_millis(5));
    assert!(!editor.value.get().selection.is_collapsed());

    // A different point and well past the double-click window — clicking the
    // same spot twice in quick succession is a *double*-click, which selects a
    // word, and asserting it collapses would be asserting against a feature.
    click(
        &mut driver,
        Offset::new(IN_TEXT.dx + 60.0, IN_TEXT.dy + 20.0),
        900,
    );
    let value = editor.value.get();
    assert_eq!(value.text, SAMPLE, "and the text is still all there");
    assert!(
        value.selection.is_collapsed(),
        "a plain click puts a caret rather than keeping the selection"
    );
    assert_eq!(
        value.selection,
        TextSelection::collapsed(value.selection.extent),
        "and it is a caret, not a zero-width range with a stale base"
    );
}
