//! A code editor, cut down to the parts that break: selection, editing,
//! clipboard and scrolling.
//!
//! # What this is for
//!
//! `viewwstudio`'s editor pane is nine hundred lines of tab strip, breadcrumbs,
//! folding, diagnostics, minimap and find bar wrapped around one `TextField`.
//! When something goes wrong with a *selection* or a *paste*, almost none of
//! that is involved — but all of it has to be ruled out. This is the same
//! editor with everything that is not the text removed: one buffer, a gutter, a
//! field, two scroll controllers and a status line, in about two hundred lines.
//! If a bug reproduces here it is in the framework; if it does not, it is in
//! the studio's projection layer.
//!
//! # The shape it keeps from the studio, and why each piece is load-bearing
//!
//! | piece | why it is here |
//! |---|---|
//! | `TextField` holding a `TextEditingValue` in a signal | the application owns the document; the field is told what it is |
//! | `on_changed` **and** `on_selection`, separately | see below — this is the bug the studio hit hardest |
//! | `wrap(false)` inside a horizontal `Scrollable` | one visual line per source line, so the gutter stays in step |
//! | a vertical `ScrollController` with iOS physics | flings, overscroll, and `reveal` for the caret |
//! | `spans` for syntax colour | selection and caret geometry are computed over *styled* runs, which is where they get interesting |
//! | services published above the root | ⌘/Ctrl-C, X and V are `RenderEditableText`'s, and it reads the `Clipboard` out of the inherited scope |
//!
//! # Why the selection handler must not carry text
//!
//! A pointer reports a **selection**; it does not report text. Give the field
//! only `on_changed` and it assembles a whole `TextEditingValue` around the
//! reported selection out of *this frame's* string — which, mid-drag, is behind
//! the buffer, and which several drag updates in one frame all read. The
//! application then commits that string, and a drag over the text has silently
//! edited the document. That is a real bug the studio shipped and fixed; it is
//! reproduced here by design, one `on_selection` away.
//!
//! ```console
//! cargo run -p editor                       # a window
//! cargo run -p editor -- shots/editor       # the states, as PNGs, with no display
//! ```
//!
//! In the window: click to place the caret, drag or ⇧-click to select, type,
//! ⌘/Ctrl-C, X, V, wheel to scroll, and drag sideways for the long lines.

use std::rc::Rc;

use vieww_element::{Runtime, ScrollController, Signal};
use vieww_foundation::{
    Color, EdgeInsets, FontFamily, SharedServices, Size, TextEditingValue, TextSelection, TextStyle,
};
use vieww_gestures::ScrollPhysics;
use vieww_text::TextSpan;
use vieww_widget::prelude::*;
use vieww_widget::{Inherited, TextField};

// The studio's own numbers, so a layout bug reproduces at the same scale.
const CODE_SIZE: f32 = 13.0;
const CODE_LINE: f32 = 20.0;
const GUTTER_WIDTH: f32 = 52.0;

const INK: Color = Color::rgb(214, 221, 231);
const MUTED: Color = Color::rgb(110, 122, 140);
const PAPER: Color = Color::rgb(24, 28, 36);
const CHROME: Color = Color::rgb(18, 21, 27);
const ACCENT: Color = Color::rgb(88, 150, 255);
const SELECTION: Color = Color::rgba(88, 150, 255, 70);
const KEYWORD: Color = Color::rgb(197, 134, 246);
const STRING: Color = Color::rgb(140, 205, 140);
const COMMENT: Color = Color::rgb(106, 118, 134);

const SOURCE: &str = "\
// Selection, editing and the clipboard, with nothing else in the way.
// The long line below is here so that horizontal scrolling has something to do.

fn main() {
    let mut buffer = Buffer::new(\"the quick brown fox jumps over the lazy dog\");
    let selection = buffer.select(4, 19);

    // Try: drag across this line, copy it, and paste it at the end.
    println!(\"selected {} bytes: {}\", selection.len(), buffer.selected_text());

    for line in buffer.lines() {
        // A deliberately long line: it runs past the right edge of the window so that the horizontal scrollable has something to slide over, and so that a caret typed at its end has to be revealed.
        println!(\"{line}\");
    }
}

struct Buffer {
    text: String,
    selection: Range<usize>,
}

impl Buffer {
    fn new(text: &str) -> Self {
        Self { text: text.to_owned(), selection: 0..0 }
    }

    fn select(&mut self, from: usize, to: usize) -> Range<usize> {
        self.selection = from..to;
        self.selection.clone()
    }

    fn selected_text(&self) -> &str {
        &self.text[self.selection.clone()]
    }

    fn lines(&self) -> impl Iterator<Item = &str> {
        self.text.lines()
    }
}

// Everything below this point is here for one reason: the file has to be taller
// than the window, or the vertical scrollable has nothing to prove.

struct Cursor {
    offset: usize,
    affinity: Affinity,
}

impl Cursor {
    fn moved_to(&self, offset: usize) -> Self {
        Self { offset, affinity: self.affinity }
    }
}

struct History {
    undo: Vec<Buffer>,
    redo: Vec<Buffer>,
}

impl History {
    fn push(&mut self, state: Buffer) {
        self.undo.push(state);
        self.redo.clear();
    }

    fn undo(&mut self) -> Option<Buffer> {
        let state = self.undo.pop()?;
        self.redo.push(state.clone());
        Some(state)
    }
}
";

fn code_style() -> TextStyle {
    TextStyle {
        color: INK,
        size: CODE_SIZE,
        family: FontFamily::Monospace,
        line_height: CODE_LINE / CODE_SIZE,
        ..TextStyle::new(CODE_SIZE)
    }
}

/// Colour by keyword, string and comment.
///
/// Deliberately naive — this is not a highlighter, it is a way of making the
/// field hold **styled runs** rather than one flat string, because that is the
/// arrangement selection rectangles and caret hit-testing are computed over and
/// the one where they go wrong.
fn spans(text: &str) -> Vec<TextSpan> {
    const KEYWORDS: [&str; 8] = ["fn", "let", "mut", "for", "in", "struct", "impl", "Self"];
    let mut out = Vec::new();
    let base = code_style();
    let tinted = |color: Color| TextStyle { color, ..base };

    for (index, line) in text.split_inclusive('\n').enumerate() {
        let _ = index;
        if line.trim_start().starts_with("//") {
            out.push(TextSpan::new(line, tinted(COMMENT)));
            continue;
        }
        // Words, and the gaps between them, kept in order so the concatenation
        // is byte-for-byte the original — a span list that does not reassemble
        // into the buffer is a caret at the wrong offset.
        let mut rest = line;
        while !rest.is_empty() {
            let cut = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            if cut == 0 {
                let end = rest
                    .find(|c: char| c.is_alphanumeric() || c == '_')
                    .unwrap_or(rest.len());
                let (chunk, tail) = rest.split_at(end);
                out.push(TextSpan::new(chunk, base));
                rest = tail;
                continue;
            }
            let (word, tail) = rest.split_at(cut);
            let style = if KEYWORDS.contains(&word) {
                tinted(KEYWORD)
            } else if line.contains('"') && word.chars().all(char::is_alphabetic) {
                tinted(STRING)
            } else {
                base
            };
            out.push(TextSpan::new(word, style));
            rest = tail;
        }
    }
    out
}

#[derive(Debug)]
struct Editor {
    buffer: Signal<TextEditingValue>,
    vertical: ScrollController,
    horizontal: ScrollController,
}

impl Editor {
    /// The line and column the caret is on, one-based, and how much is selected.
    fn caret(&self) -> (usize, usize, usize) {
        let value = self.buffer.peek();
        let at = value.selection.cursor().offset.min(value.text.len());
        let before = &value.text[..at];
        let line = before.matches('\n').count() + 1;
        let column = before
            .rsplit('\n')
            .next()
            .map_or(1, |s| s.chars().count() + 1);
        (line, column, value.selection.range().len())
    }

    fn gutter(&self, lines: usize, caret_line: usize) -> WidgetNode {
        Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::End)
            .children(
                (1..=lines)
                    .map(|n| {
                        Container::new()
                            .height(CODE_LINE)
                            .alignment(Alignment::CENTER_RIGHT)
                            .child(
                                Text::new(n.to_string())
                                    .style(TextStyle {
                                        color: if n == caret_line { ACCENT } else { MUTED },
                                        ..code_style()
                                    })
                                    .size(CODE_SIZE),
                            )
                            .into()
                    })
                    .collect::<Vec<WidgetNode>>(),
            )
            .into()
    }

    fn status(&self) -> WidgetNode {
        let (line, column, selected) = self.caret();
        let selection = if selected == 0 {
            "no selection".to_owned()
        } else {
            format!("{selected} bytes selected")
        };
        Container::new()
            .color(CHROME)
            .padding(EdgeInsets::symmetric(12.0, 6.0))
            .child(Flex::row().spacing(18.0).children(children![
                    Text::new(format!("Ln {line}, Col {column}"))
                        .color(MUTED)
                        .size(12.0),
                    Text::new(selection).color(MUTED).size(12.0),
                    Text::new("⌘/Ctrl-C copy · X cut · V paste")
                        .color(MUTED)
                        .size(12.0),
                ]))
            .into()
    }
}

impl Widget for Editor {
    fn debug_name(&self) -> &'static str {
        "Editor"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Read, so this element is what rebuilds when the document changes.
        let value = self.buffer.get();
        let lines = value.text.lines().count().max(1);
        let (caret_line, _, _) = self.caret();

        let on_changed = {
            let buffer = self.buffer.clone();
            Rc::new(move |next: TextEditingValue| buffer.set(next))
        };
        // **Selection only.** See the module docs: giving the pointer path a
        // route to the text is how a drag becomes an edit.
        let on_selection = {
            let buffer = self.buffer.clone();
            Rc::new(move |selection: TextSelection| {
                let mut next = buffer.peek();
                next.selection = selection;
                buffer.set(next);
            })
        };

        let field = TextField::new(value.clone())
            .style(code_style())
            .spans(spans(&value.text))
            .show_cursor(true)
            .cursor(ACCENT, 2.0)
            .selection_color(SELECTION)
            // One visual line per source line, so row *n* of the gutter is
            // line *n* of the file. With wrapping on, a long line silently
            // pushes every number below it out of step.
            .wrap(false)
            .on_changed(on_changed)
            .on_selection(on_selection);

        let code = Scrollable::horizontal(self.horizontal.offset())
            .key("editor-columns")
            .on_drag(self.horizontal.on_drag())
            .on_drag_end(self.horizontal.on_drag_end())
            .on_extents(self.horizontal.on_extents())
            .child(field);

        let pane = Container::new()
            .color(PAPER)
            .padding(EdgeInsets::only(0.0, 8.0, 0.0, 8.0))
            .child(
                Flex::row()
                    .cross_axis_alignment(CrossAxisAlignment::Start)
                    .children(children![
                        Container::new()
                            .width(GUTTER_WIDTH)
                            .padding(EdgeInsets::only(0.0, 0.0, 10.0, 0.0))
                            .child(self.gutter(lines, caret_line)),
                        Flexible::expanded(1).child(
                            Container::new()
                                .padding(EdgeInsets::only(4.0, 0.0, 16.0, 0.0))
                                .child(code),
                        ),
                    ]),
            );

        Container::new()
            .color(PAPER)
            .child(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(children![
                        // Clipped *and* scrollable: the clip is what keeps a
                        // fling from painting over the status line, the scroll
                        // is what makes the rest of the file reachable.
                        Flexible::expanded(1).child(
                            Clip::rect().child(
                                Scrollable::vertical(self.vertical.offset())
                                    .on_drag(self.vertical.on_drag())
                                    .on_drag_end(self.vertical.on_drag_end())
                                    .on_extents(self.vertical.on_extents())
                                    .child(pane),
                            ),
                        ),
                        self.status(),
                    ]),
            )
            .into()
    }
}

vieww_widget::widget_node_from!(Editor);

fn editor(runtime: &Runtime, tickers: &mut vieww_animation::Tickers) -> Editor {
    let vertical = ScrollController::new(runtime, ScrollPhysics::ios());
    let horizontal = ScrollController::horizontal(runtime, ScrollPhysics::ios());
    // Attached, or a fling stops the moment the finger lifts — the controller
    // is registered weakly, exactly like an `Animation`.
    vertical.attach(tickers);
    horizontal.attach(tickers);
    Editor {
        buffer: runtime.signal(TextEditingValue::new(SOURCE)),
        vertical,
        horizontal,
    }
}

/// One state a person would produce by hand, applied to the mounted editor.
type State = Box<dyn Fn(&Editor)>;

/// Everything after `editor()` exists to be *driven*: a window drives it with a
/// mouse and a keyboard, and the shot path drives it with these.
///
/// A screenshot of an untouched file proves the buffer renders and nothing
/// else — and "renders" was never the part in doubt. Each state below is one a
/// person produces by hand, so the PNGs show a selection, an edit made through
/// that selection, and a pane scrolled in both axes.
fn states() -> Vec<(&'static str, State)> {
    vec![
        ("1-open", Box::new(|_editor: &Editor| {})),
        (
            "2-selected",
            Box::new(|editor: &Editor| {
                let mut value = editor.buffer.peek();
                value.selection = TextSelection::new(196, 268);
                editor.buffer.set(value);
            }),
        ),
        (
            "3-typed",
            Box::new(|editor: &Editor| {
                // Typing over a selection, which is the path a drag that
                // wrongly carried text used to corrupt.
                let mut value = editor.buffer.peek();
                value.replace_selection("SELECTED");
                editor.buffer.set(value);
            }),
        ),
        (
            "4-scrolled",
            Box::new(|editor: &Editor| {
                editor.vertical.jump_to(180.0);
                editor.horizontal.jump_to(260.0);
            }),
        ),
    ]
}

const WIDTH: f32 = 820.0;
const HEIGHT: f32 = 560.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args().nth(1) {
        Some(directory) => shoot(std::path::Path::new(&directory)),
        None => window(),
    }
}

/// A window, with real input.
fn window() -> Result<(), Box<dyn std::error::Error>> {
    vieww_platform_winit::App::new()
        .title("editor")
        .size(Size::new(WIDTH, HEIGHT))
        .background(PAPER)
        .run(|driver| {
            let runtime = driver.elements().runtime().clone();
            let editor = editor(&runtime, driver.tickers());
            // The clipboard is a *service*: `RenderEditableText` implements cut,
            // copy and paste itself and reads it out of the inherited scope.
            // Publish nothing here and the keystrokes are received and do
            // nothing, which looks exactly like a broken shortcut.
            let services = SharedServices::new(vieww_platform_winit::services::platform());
            driver.set_root(Inherited::new(services, editor));
        })?;
    Ok(())
}

/// The same tree, driven through [`states`] and written out as PNGs.
fn shoot(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use vieww_paint::native::NativeRenderer;

    std::fs::create_dir_all(directory)?;
    let mut driver = vieww_render::FrameDriver::new(Size::new(WIDTH, HEIGHT));
    let runtime = driver.elements().runtime().clone();
    let editor = editor(&runtime, driver.tickers());
    let handle = Editor {
        buffer: editor.buffer.clone(),
        vertical: editor.vertical.clone(),
        horizontal: editor.horizontal.clone(),
    };
    // No platform services headless: `Clipboard` would be a pasteboard on a
    // machine with no display. Nothing below presses ⌘C, and saying so is
    // better than publishing a service that answers every call with an error.
    driver.set_root(editor);

    let mut renderer = NativeRenderer::new();
    for (name, drive) in states() {
        drive(&handle);
        driver.draw_frame();
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let (png, report) =
            renderer.render_to_png(driver.scene(), WIDTH as u32, HEIGHT as u32, PAPER)?;
        let path = directory.join(format!("{name}.png"));
        std::fs::write(&path, png)?;
        println!(
            "  {}: {} shapes, {} glyph runs ({} glyphs), {} clips",
            path.display(),
            report.shapes,
            report.glyph_runs,
            report.glyphs,
            report.clips,
        );
    }
    println!("\nwrote {}", directory.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vieww_foundation::{Offset, PointerEvent, PointerId, ScrollEvent};
    use vieww_render::FrameDriver;

    /// The tree, mounted headless, plus a handle on its state.
    fn mounted() -> (FrameDriver, Editor) {
        let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
        let runtime = driver.elements().runtime().clone();
        let editor = editor(&runtime, driver.tickers());
        let handle = Editor {
            buffer: editor.buffer.clone(),
            vertical: editor.vertical.clone(),
            horizontal: editor.horizontal.clone(),
        };
        driver.set_root(editor);
        driver.draw_frame();
        (driver, handle)
    }

    /// A press, two moves and a release — a selection drag, as a mouse makes it.
    fn drag(driver: &mut FrameDriver, from: Offset, to: Offset) {
        let pointer = PointerId(1);
        let middle = Offset::new((from.dx + to.dx) / 2.0, (from.dy + to.dy) / 2.0);
        for event in [
            PointerEvent::down(pointer, from, Duration::from_millis(10)),
            PointerEvent::moved(pointer, from, middle, Duration::from_millis(30)),
            PointerEvent::moved(pointer, middle, to, Duration::from_millis(50)),
            PointerEvent::up(pointer, to, Duration::from_millis(70)),
        ] {
            driver.handle_pointer(&event);
            driver.draw_frame();
        }
    }

    /// **The bug this example was extracted to reproduce.**
    ///
    /// A pointer reports a selection, never text. If the field is given a route
    /// to rebuild a whole `TextEditingValue` from a stale build, a drag across
    /// the document *edits* it — silently, and by however much the build was
    /// behind. So: the selection must change and the text must not.
    #[test]
    fn dragging_selects_without_editing() {
        let (mut driver, editor) = mounted();
        let before = editor.buffer.peek().text.clone();

        drag(
            &mut driver,
            Offset::new(120.0, 60.0),
            Offset::new(420.0, 100.0),
        );

        let after = editor.buffer.peek();
        assert_eq!(after.text, before, "a drag must not change the document");
        assert!(
            !after.selection.is_collapsed(),
            "a drag across the text must leave a selection, not a caret"
        );
    }

    /// Typing replaces what is selected, and only what is selected.
    #[test]
    fn typing_over_a_selection_replaces_it() {
        let (_driver, editor) = mounted();
        let mut value = editor.buffer.peek();
        let before = value.text.len();
        value.selection = TextSelection::new(196, 268);
        let removed = value.selection.range().len();
        value.replace_selection("SELECTED");
        editor.buffer.set(value);

        let after = editor.buffer.peek();
        assert_eq!(after.text.len(), before - removed + "SELECTED".len());
        assert!(
            after.selection.is_collapsed(),
            "after typing there is a caret, not a selection"
        );
    }

    /// The pane scrolls, and stops where the content does.
    #[test]
    fn the_pane_scrolls_and_clamps() {
        let (mut driver, editor) = mounted();
        assert_eq!(editor.vertical.offset(), 0.0);

        let taken = driver.handle_scroll(&ScrollEvent {
            position: Offset::new(400.0, 200.0),
            delta: Offset::new(0.0, -120.0),
            timestamp: Duration::from_millis(16),
        });
        driver.draw_frame();

        assert!(taken, "the wheel reached the editor");
        assert!(
            editor.vertical.offset() > 0.0,
            "a file taller than the window scrolls"
        );

        editor.vertical.jump_to(100_000.0);
        driver.draw_frame();
        assert!(
            editor.vertical.offset() <= editor.vertical.max_offset(),
            "and stops at the end of the content rather than running past it"
        );
    }
}

#[cfg(test)]
mod ime {
    //! The screencast, as a test.
    use super::*;
    use std::time::Duration;
    use vieww_foundation::{ImeEvent, Offset, PointerEvent, PointerId};
    use vieww_render::FrameDriver;

    /// **Select, then let the input method reset: the document must survive.**
    ///
    /// This is the sequence a window produces and a synthetic drag does not.
    /// X11 and Wayland input methods send `Preedit("")` around a pointer press,
    /// and with a selection in flight that used to delete everything between
    /// the old caret and the pointer — the file visibly losing two thirds of
    /// itself the moment you dragged across it.
    #[test]
    fn selecting_then_an_ime_reset_keeps_the_document() {
        let mut driver = FrameDriver::new(Size::new(WIDTH, HEIGHT));
        let runtime = driver.elements().runtime().clone();
        let ed = editor(&runtime, driver.tickers());
        let buffer = ed.buffer.clone();
        driver.set_root(ed);
        driver.draw_frame();

        let before = buffer.peek().text.clone();
        let pointer = PointerId(1);
        let from = Offset::new(140.0, 60.0);
        let to = Offset::new(430.0, 108.0);
        let middle = Offset::new((from.dx + to.dx) / 2.0, (from.dy + to.dy) / 2.0);
        for event in [
            PointerEvent::down(pointer, from, Duration::from_millis(10)),
            // Two moves, not one: the first is the slop the drag recogniser
            // spends deciding it is a drag at all.
            PointerEvent::moved(pointer, from, middle, Duration::from_millis(30)),
            PointerEvent::moved(pointer, middle, to, Duration::from_millis(50)),
            PointerEvent::up(pointer, to, Duration::from_millis(70)),
        ] {
            driver.handle_pointer(&event);
            driver.draw_frame();
        }
        assert!(
            !buffer.peek().selection.is_collapsed(),
            "the drag selected something to lose"
        );

        driver.handle_ime(&ImeEvent::Enabled);
        driver.handle_ime(&ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        });
        driver.draw_frame();

        assert_eq!(buffer.peek().text, before, "an IME reset is not an edit");
        assert!(
            !buffer.peek().selection.is_collapsed(),
            "and it does not drop the selection either"
        );
    }
}
