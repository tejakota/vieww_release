//! Real pointer and key events, through the whole shell.
//!
//! # Why this file exists
//!
//! Every other test here drives [`Studio`] directly, which is fast and proves
//! behaviour. None of them touched the **input path** — hit testing, the
//! gesture arena, focus, the controlled-value round trip — and that is where
//! the reported defects lived: a caret that would not move on a click, a drag
//! that seemed to delete text, a menu that opened into nothing.
//!
//! So this one drives a `FrameDriver` the way a window does: pointer down,
//! move, up, with timestamps, and keys with modifiers. It is slower than the
//! rest and it is the only thing that can catch this class of bug.

use std::time::Duration;

use vieww_foundation::{
    Cursor, KeyEvent, LogicalKey, Modifiers, NamedKey, Offset, PointerEvent, PointerId, Size,
    TargetPlatform,
};
use vieww_render::FrameDriver;
use viewwstudio::{Shell, Studio};

/// A mounted shell, focused, with one frame drawn.
fn shell() -> (FrameDriver, Studio) {
    let mut driver = FrameDriver::new(Size::new(1440.0, 900.0));
    viewwstudio::install(&mut driver);
    let runtime = driver.elements().runtime().clone();
    let studio = Studio::new(&runtime).with_host(TargetPlatform::Linux);
    driver.set_root(Shell {
        studio: studio.clone(),
    });
    driver.draw_frame();
    viewwstudio::seed_focus(&mut driver);
    (driver, studio)
}

/// A point inside the code pane, past the sidebar and the gutter.
/// A point inside the code pane, on a line with real content on it.
///
/// `dy` is chosen to land on `impl Widget for Screen {` in the scratch
/// buffer, which is what several tests here quietly depend on: a word to
/// double-click and a longer line around it to triple-click. It was 300.0
/// while the code pane wrapped, and the same source line moved up two rows
/// when the pane stopped wrapping (`TextField::wrap(false)` in
/// `ui::editor` — the gutter is one row per *source* line, so the pane must
/// not reflow). Landing on `    }` instead, as 300.0 now does, makes
/// "double-click selects a word" a test about a run of spaces.
const IN_CODE: Offset = Offset {
    dx: 520.0 + SHELL_DX,
    dy: 260.0 + SHELL_DY,
};

/// How far right the card shell moved the editor, and how far down.
///
/// Written as two named offsets added to the original point rather than as a
/// new pair of magic numbers, because the point itself is still chosen for the
/// reason above — a source line with a word on it — and only the chrome in
/// front of it changed. Each is the sum of one window inset, the borders of the
/// cards crossed on the way, and the gutters between them:
///
/// ```text
/// dx  6 window  + 1+48+1 activity + 6 gutter + 1+248+1 sidebar + 6 divider + 1 editor
///     was         48                  1 hairline   248            1 hairline  5 divider
/// dy  6 window  + 1+HEIGHT+1 title bar + 6 gutter + 1 editor
///     was         HEIGHT + 1 hairline
/// ```
const SHELL_DX: f32 = 16.0;
const SHELL_DY: f32 = 14.0;

/// A press, and enough frames for the press to be recognised.
///
/// # Why the clock has to move
///
/// `TapRecognizer` emits `TapDown` — which is what moves the caret — at
/// `PRESS_TIMEOUT` after the finger lands, from `FrameDriver::tick_pointers`,
/// which runs inside the frame from the *frame's* timestamp. A finger resting
/// on the screen produces no events at all, so nothing else brings `now`
/// forward. A test that draws frames at time zero for ever is a test where the
/// press is still pending, which is not what a real window does — there, frames
/// keep arriving with real timestamps.
fn press(driver: &mut FrameDriver, at: Offset, ms: u64) {
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        at,
        Duration::from_millis(ms),
    ));
    driver.draw_frame_at(Duration::from_millis(ms));
    // One frame past `PRESS_TIMEOUT`, which is when the recogniser reports it.
    driver.draw_frame_at(Duration::from_millis(ms + 110));
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
    release(driver, at, ms + 120);
}

// ===================== the caret ========================================

#[test]
fn a_click_in_the_editor_moves_the_caret_on_the_press() {
    let (mut driver, studio) = shell();
    let before = studio.caret.get();

    // The press alone. The caret used to wait for the release, which is what
    // made a click feel dropped — see `RenderEditableText::handle_gesture`.
    press(&mut driver, IN_CODE, 1);

    assert_ne!(
        studio.caret.get(),
        before,
        "the caret did not move until the button came up"
    );
}

#[test]
fn clicking_does_not_change_the_text() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();
    click(&mut driver, IN_CODE, 1);
    assert_eq!(studio.active().unwrap().value.text, before);
}

#[test]
fn dragging_selects_and_never_deletes() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    let start = Offset::new(420.0, 300.0);
    press(&mut driver, start, 1);
    let mut previous = start;
    for step in 1u8..=6 {
        let at = Offset::new(start.dx + f32::from(step) * 40.0, start.dy);
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + u64::from(step)));
        previous = at;
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging, at step {step}"
        );
    }
    release(&mut driver, previous, 20);

    let after = studio.active().unwrap().value;
    assert_eq!(after.text, before, "a drag is a selection, not an edit");
    assert!(
        !after.selection.is_collapsed(),
        "dragging across text selected nothing"
    );
}

/// **The drag the screencast actually shows.**
///
/// `dragging_selects_and_never_deletes` above drags *sideways along one line*
/// and passes. The reported failure is a drag that goes **down the page**, or
/// up it — the user's words were "from top to bottom or bottom to up" — and
/// that is a different path: a vertical drag inside the code pane is contested
/// in the gesture arena by the `Scrollable` wrapped around it, and the sequence
/// of events the field sees is not the sequence a horizontal drag produces.
///
/// The assertion is the same one and it is the only one that matters: a
/// selection gesture may not change the document.
#[test]
fn dragging_down_the_page_selects_and_never_deletes() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    let start = Offset::new(IN_CODE.dx, IN_CODE.dy - 60.0);
    press(&mut driver, start, 1);
    let mut previous = start;
    for step in 1u8..=8 {
        let at = Offset::new(
            start.dx + f32::from(step) * 6.0,
            start.dy + f32::from(step) * 14.0,
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
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging down, at step {step}"
        );
    }
    release(&mut driver, previous, 30);
    driver.draw_frame_at(Duration::from_millis(31));

    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "a downward drag is a selection, not an edit"
    );
}

/// The same, upward. A drag whose anchor is *below* its extent reverses the
/// selection, and a path that assembles text from a reversed range is a path
/// that can splice it back in the wrong order.
#[test]
fn dragging_up_the_page_selects_and_never_deletes() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    let start = Offset::new(IN_CODE.dx, IN_CODE.dy + 60.0);
    press(&mut driver, start, 1);
    let mut previous = start;
    for step in 1u8..=8 {
        let at = Offset::new(
            start.dx - f32::from(step) * 6.0,
            start.dy - f32::from(step) * 14.0,
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
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging up, at step {step}"
        );
    }
    release(&mut driver, previous, 30);
    driver.draw_frame_at(Duration::from_millis(31));

    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "an upward drag is a selection, not an edit"
    );
}

/// A drag that runs past the bottom of the pane, which is what makes the view
/// scroll under the finger — the other half of "click and scroll".
#[test]
fn dragging_past_the_edge_of_the_pane_never_deletes() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    press(&mut driver, IN_CODE, 1);
    let mut previous = IN_CODE;
    for step in 1u8..=10 {
        // Straight down, well past the bottom of the editor card.
        let at = Offset::new(IN_CODE.dx, IN_CODE.dy + f32::from(step) * 60.0);
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + u64::from(step)));
        previous = at;
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging past the edge, at step {step}"
        );
    }
    release(&mut driver, previous, 40);
    driver.draw_frame_at(Duration::from_millis(41));

    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "a drag out of the pane is a selection, not an edit"
    );
}

/// **Click, then scroll, then click.** The sequence the second screencast
/// shows, in the user's own words: *"the text is vanishing when click and
/// scroll are used, from top to bottom or bottom to up"*.
///
/// The reason this is a different path from a plain drag is the scroll: the
/// code pane slides under the pointer, so the *second* click lands at a screen
/// point that maps to a different text offset than it would have before. Any
/// code that measured a position against a stale layout has its one chance to
/// be wrong here, and being wrong about an offset is how a range that should
/// be a selection becomes a range that gets spliced.
#[test]
fn scrolling_between_two_clicks_never_changes_the_text() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    click(&mut driver, IN_CODE, 1);
    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "the first click"
    );

    for (step, dy) in [(1u64, -120.0_f32), (2, -120.0), (3, 240.0), (4, 120.0)] {
        driver.handle_scroll(&vieww_foundation::ScrollEvent::new(
            IN_CODE,
            Offset::new(0.0, dy),
            Duration::from_millis(10 + step),
        ));
        driver.draw_frame_at(Duration::from_millis(10 + step));
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed on a scroll, at step {step}"
        );
    }

    click(&mut driver, IN_CODE, 40);
    driver.draw_frame_at(Duration::from_millis(41));
    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "a click after a scroll is a caret move, not an edit"
    );
}

/// And the same with a *drag* after the scroll, which is the version that
/// leaves a selection behind to be spliced.
#[test]
fn dragging_after_a_scroll_never_changes_the_text() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    driver.handle_scroll(&vieww_foundation::ScrollEvent::new(
        IN_CODE,
        Offset::new(0.0, -160.0),
        Duration::from_millis(5),
    ));
    driver.draw_frame_at(Duration::from_millis(5));

    let start = IN_CODE;
    press(&mut driver, start, 10);
    let mut previous = start;
    for step in 1u8..=8 {
        let at = Offset::new(
            start.dx + f32::from(step) * 8.0,
            start.dy + f32::from(step) * 16.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(10 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(10 + u64::from(step)));
        previous = at;
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging after a scroll, at step {step}"
        );
    }
    release(&mut driver, previous, 40);
    driver.draw_frame_at(Duration::from_millis(41));

    let after = studio.active().unwrap().value;
    assert_eq!(
        after.text, before,
        "a drag after a scroll is a selection, not an edit"
    );
    assert!(
        !after.selection.is_collapsed(),
        "and it did select something"
    );
}

/// Scrolling *while the button is held*, which is what a trackpad produces
/// when a two-finger scroll overlaps a drag, and what a wheel produces when
/// somebody scrolls without letting go.
#[test]
fn scrolling_mid_drag_never_changes_the_text() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    press(&mut driver, IN_CODE, 1);
    let mut previous = IN_CODE;
    for step in 1u8..=6 {
        let at = Offset::new(
            IN_CODE.dx + f32::from(step) * 10.0,
            IN_CODE.dy + f32::from(step) * 12.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(1 + u64::from(step)),
        ));
        // A wheel notch in the middle of the drag.
        driver.handle_scroll(&vieww_foundation::ScrollEvent::new(
            at,
            Offset::new(0.0, -40.0),
            Duration::from_millis(1 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(1 + u64::from(step)));
        previous = at;
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed on a scroll mid-drag, at step {step}"
        );
    }
    release(&mut driver, previous, 30);
    driver.draw_frame_at(Duration::from_millis(31));

    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "a scroll during a drag is not an edit"
    );
}

/// The one remaining combination: a drag **with a fold collapsed**.
///
/// This is the path where the text the field holds is not the text the buffer
/// holds — it is the *projection* — so `Studio::edit_projected` has to
/// un-project anything that comes back. A selection must still never get that
/// far, but a fold is visible in the screencast's gutter and this is the only
/// state in which a wrong answer here could splice the document.
#[test]
fn dragging_with_a_fold_collapsed_never_changes_the_text() {
    let (mut driver, studio) = shell();
    let before = studio.active().expect("a buffer").value.text.clone();

    studio.fold_all(true);
    driver.draw_frame_at(Duration::from_millis(1));
    assert!(
        !studio.folds.get().is_empty(),
        "the fixture has something folded to test with"
    );
    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "folding itself is not an edit"
    );

    press(&mut driver, IN_CODE, 5);
    let mut previous = IN_CODE;
    for step in 1u8..=6 {
        let at = Offset::new(
            IN_CODE.dx + f32::from(step) * 12.0,
            IN_CODE.dy + f32::from(step) * 10.0,
        );
        driver.handle_pointer(&PointerEvent::moved(
            PointerId(1),
            previous,
            at,
            Duration::from_millis(5 + u64::from(step)),
        ));
        driver.draw_frame_at(Duration::from_millis(5 + u64::from(step)));
        previous = at;
        assert_eq!(
            studio.active().unwrap().value.text,
            before,
            "the text changed while dragging over a fold, at step {step}"
        );
    }
    release(&mut driver, previous, 30);
    driver.draw_frame_at(Duration::from_millis(31));

    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "a drag across a folded document is a selection, not an edit"
    );
}

#[test]
fn a_double_click_selects_a_word() {
    let (mut driver, studio) = shell();

    click(&mut driver, IN_CODE, 1);
    assert!(
        studio.active().unwrap().value.selection.is_collapsed(),
        "one click is a caret"
    );

    click(&mut driver, IN_CODE, 200);
    let value = studio.active().unwrap().value;
    assert!(
        !value.selection.is_collapsed(),
        "a second click within the multi-tap window selects the word under it"
    );
    let word = value.selection.range().slice(&value.text);
    assert!(
        !word.contains(char::is_whitespace),
        "a word selection should not span whitespace, got {word:?}"
    );
}

#[test]
fn a_triple_click_selects_the_line() {
    let (mut driver, studio) = shell();
    click(&mut driver, IN_CODE, 1);
    click(&mut driver, IN_CODE, 200);
    let word = studio.active().unwrap().value.selection;
    click(&mut driver, IN_CODE, 400);
    let line = studio.active().unwrap().value.selection;

    assert!(
        line.end() - line.start() > word.end() - word.start(),
        "the third click should have widened the selection from a word to a line"
    );
}

// ===================== the cursor shape ==================================

#[test]
fn the_pointer_is_an_i_beam_over_the_code_and_an_arrow_over_the_chrome() {
    let (driver, _studio) = shell();

    assert_eq!(
        driver.cursor_at(IN_CODE),
        Cursor::Text,
        "text you can select has to say so before you click it"
    );
    // The status bar, which is not text you can edit.
    assert_eq!(driver.cursor_at(Offset::new(700.0, 890.0)), Cursor::Default);
}

// ===================== keys through the real path =======================

#[test]
fn typing_reaches_the_buffer() {
    let (mut driver, studio) = shell();
    click(&mut driver, IN_CODE, 1);
    let before = studio.active().unwrap().value.text.len();

    let handled = driver.handle_key(&KeyEvent::character("Z", Duration::from_millis(50)));
    driver.draw_frame();

    assert!(handled, "an ordinary character was not taken by the editor");
    assert_eq!(
        studio.active().unwrap().value.text.len(),
        before + 1,
        "the keystroke never reached the buffer"
    );
}

#[test]
fn a_shortcut_is_not_typed_into_the_buffer() {
    let (mut driver, studio) = shell();
    click(&mut driver, IN_CODE, 1);
    let before = studio.active().unwrap().value.text.clone();
    let panel = studio.panel_open.get();

    driver.handle_key(
        &KeyEvent::character("j", Duration::from_millis(50)).with_modifiers(Modifiers::CONTROL),
    );
    driver.draw_frame();

    assert_eq!(studio.panel_open.get(), !panel, "the shortcut did not run");
    assert_eq!(
        studio.active().unwrap().value.text,
        before,
        "the editor also inserted the character — a shortcut fell through"
    );
}

#[test]
fn home_and_end_reach_the_ends_of_the_line() {
    let (mut driver, studio) = shell();
    click(&mut driver, IN_CODE, 1);
    let (line, column) = studio.caret.get();
    assert!(
        column > 1,
        "the click should land mid-line for this to mean anything"
    );

    driver.handle_key(&KeyEvent::named(NamedKey::Home, Duration::from_millis(50)));
    driver.draw_frame();
    assert_eq!(
        studio.caret.get(),
        (line, 1),
        "Home did not reach the start"
    );

    driver.handle_key(&KeyEvent::named(NamedKey::End, Duration::from_millis(60)));
    driver.draw_frame();
    assert!(studio.caret.get().1 > 1, "End did not move");
    assert_eq!(
        studio.caret.get().0,
        line,
        "End landed on the next line — it stopped *after* the newline rather \
         than before it, so Home-then-shift-End would select the line break too"
    );
}

#[test]
fn page_down_moves_further_than_one_line() {
    let (mut driver, studio) = shell();
    click(&mut driver, Offset::new(420.0, 130.0), 1);
    let (start, _) = studio.caret.get();

    driver.handle_key(&KeyEvent::named(
        NamedKey::ArrowDown,
        Duration::from_millis(50),
    ));
    driver.draw_frame();
    let after_one = studio.caret.get().0;

    driver.handle_key(&KeyEvent::named(
        NamedKey::PageDown,
        Duration::from_millis(60),
    ));
    driver.draw_frame();
    let after_page = studio.caret.get().0;

    assert_eq!(after_one, start + 1, "ArrowDown moves one line");
    assert!(
        after_page > after_one,
        "PageDown moved {} lines, which is not a page",
        after_page - after_one
    );
}

#[test]
fn escape_reaches_the_shortcut_layer_past_a_focused_field() {
    let (mut driver, studio) = shell();
    click(&mut driver, IN_CODE, 1);
    studio.find_open.set(true);
    driver.draw_frame();

    driver.handle_key(&KeyEvent::named(
        NamedKey::Escape,
        Duration::from_millis(50),
    ));
    driver.draw_frame();

    assert!(
        !studio.find_open.get(),
        "Escape was swallowed by the field instead of bubbling out to the \
         shortcut layer"
    );
}

#[test]
fn an_unhandled_key_is_reported_unhandled() {
    let (mut driver, _studio) = shell();
    // F-keys have no meaning here. Nothing should claim them.
    let event = KeyEvent {
        key: LogicalKey::Unidentified,
        state: vieww_foundation::KeyState::Down,
        repeat: false,
        modifiers: Modifiers::NONE,
        timestamp: Duration::from_millis(10),
    };
    assert!(
        !driver.handle_key(&event),
        "something claimed a key it does not understand"
    );
}

// ===================== the gutter and the code agree =====================

/// A line too long for the pane must not reflow into a second visual row.
///
/// # Why this is a test and not a preference
///
/// The gutter is built as one fixed-height row per *source* line, straight
/// from `Buffer::line_count`, and the diagnostic markers the compiler sends
/// back are keyed by rustc's line number. Both assume one source line is one
/// visual row. A wrapping code pane breaks that assumption silently: the
/// first over-long line pushes every row beneath it down by one, so line
/// numbers stop pointing at their own text and an error lands its highlight
/// on the wrong line — which is what the block over line 13's indent was.
///
/// So the invariant is: for the code pane, one `\n` is one visual line, at
/// any pane width. Asserted through a caret move rather than by reading
/// geometry, because `MoveDown` is exactly the operation that would land
/// mid-line if the text had wrapped underneath it.
#[test]
fn a_long_line_in_the_code_pane_does_not_wrap_under_the_gutter() {
    let (mut driver, studio) = shell();

    // A click first, only to give the field the keyboard. Where it lands does
    // not matter — the caret is placed explicitly below, because this test is
    // about which line Down reaches and not about hit testing.
    click(&mut driver, Offset::new(420.0, 130.0), 1);

    // Far wider than the pane, and one source line, so a wrapping field shows
    // it as several visual rows and a non-wrapping one as exactly one. The
    // caret goes to the very start of it.
    let mut value = studio.active().expect("a buffer").value;
    value.text = format!("// {}\nsecond line\n", "x".repeat(400));
    value.selection = vieww_foundation::TextSelection::collapsed(0);
    studio.edit(value);
    driver.draw_frame();
    assert_eq!(studio.caret.get().0, 1, "the caret starts on the long line");

    driver.handle_key(&KeyEvent::named(
        NamedKey::ArrowDown,
        Duration::from_millis(50),
    ));
    driver.draw_frame();

    assert_eq!(
        studio.caret.get().0,
        2,
        "one press of Down left the caret inside the wrapped remainder of the \
         long line rather than on line 2 — the code pane is wrapping, and the \
         gutter beside it is not"
    );
}
