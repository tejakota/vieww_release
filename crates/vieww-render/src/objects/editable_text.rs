use std::any::Any;
use std::cell::Cell;
use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::{
    Clipboard, Color, Constraints, Cursor, Modifiers, Obscured, Offset, Path, Rect, Size,
    TapDetails, TargetPlatform, TextAlign, TextDecoration, TextDecorationShape, TextDirection,
    TextEditingValue, TextIntent, TextLayoutProbe, TextLayoutReport, TextPosition, TextRange,
    TextSelection, TextStyle, MULTI_TAP_SLOP, MULTI_TAP_TIMEOUT,
};
use vieww_gestures::{DragRecognizer, GestureRecognizer, Recognized, TapRecognizer};
use vieww_paint::Paint;
use vieww_text::{Paragraph, TextSpan};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// A caret's width in logical pixels when nothing says otherwise.
const DEFAULT_CURSOR_WIDTH: f32 = 2.0;

/// How thick the line under composing text is, as a fraction of the font size.
const COMPOSING_UNDERLINE_RATIO: f32 = 0.06;

/// How opaque a selection highlight is, so the glyphs under it stay readable.
const SELECTION_ALPHA: u8 = 77;

/// Told when a gesture moves the caret or changes the selection.
///
/// `Rc<dyn Fn>` for the same reason
/// [`Handler`](vieww_widget::Handler) is: the render object is rebuilt from
/// scratch constantly, so a handler cannot own state — its job is to push the new
/// selection back into the element's state, which is what survives.
pub type SelectionChanged = Rc<dyn Fn(TextSelection)>;

/// Told when a key or an input method changes the text itself.
///
/// Separate from [`SelectionChanged`] because a pointer can only ever move the
/// caret, while a key can change all three of text, selection and composing
/// region at once — and reporting those one at a time would put the value
/// through states that never existed.
pub type ValueChanged = Rc<dyn Fn(TextEditingValue)>;

/// Told that a single-line field was submitted, with the text it held.
///
/// The text rather than the whole [`TextEditingValue`], because a submit is
/// about the content and not about where the caret happened to be.
pub type Submitted = Rc<dyn Fn(String)>;

/// Text that can be selected and have a caret placed in it.
///
/// The drawing half of text editing. It owns no editing state: the
/// [`TextEditingValue`] is handed down from whatever holds it, and gestures are
/// reported back out through [`on_selection_changed`](Self::on_selection_changed)
/// rather than applied here. That is what keeps the caret on screen and the caret
/// in the model from ever disagreeing — there is only one of them.
///
/// # Why it is not `RenderText` with extras
///
/// A label and a field differ in more than decoration. This one is a gesture
/// target with recognisers of its own, it paints three layers rather than one,
/// its geometry has to leave room for a caret past the last character, and its
/// paint-only properties — the caret's visibility above all, which changes twice
/// a second — must not force the paragraph to be shaped again. Folding that into
/// `RenderText` would make every label in the tree pay for it.
#[derive(Clone)]
pub struct RenderEditableText {
    /// The text, the selection, and any composing region.
    pub value: TextEditingValue,
    pub style: TextStyle,
    pub align: TextAlign,
    /// An explicit base direction, or `None` to infer it from the text.
    pub direction: Option<TextDirection>,
    /// Painted behind the selection. Transparent draws no highlight.
    pub selection_color: Color,
    /// Lines containing compiler diagnostics, painted as a subtle background
    /// behind the text. These are paint-only and therefore do not affect shaping.
    pub diagnostic_lines: Rc<Vec<usize>>,
    pub diagnostic_color: Color,
    /// Marks keyed to *ranges* rather than to lines: the squiggle under an
    /// error, the box around a matched bracket, the highlight on every other
    /// occurrence of the selected word.
    ///
    /// Paint-only, like the two above, and excluded from
    /// `layout_eq` for the same reason: nothing here can
    /// move a glyph, and a field that re-squiggles on every keystroke must not
    /// re-shape its paragraph on every keystroke.
    pub decorations: Rc<Vec<TextDecoration>>,
    pub cursor_color: Color,
    pub cursor_width: f32,
    /// Whether to draw the caret at all.
    ///
    /// The blink, and the unfocused state, are both this. It is deliberately not
    /// a phase or a timer — a render object that knew the time would have to be
    /// asked to repaint by something that also knew it, and the frame driver
    /// already does that job.
    pub show_cursor: bool,
    /// What the field shows instead of what it holds. See [`Obscured`].
    pub obscure: Obscured,
    /// Shown when the value is empty, in [`placeholder_color`](Self::placeholder_color).
    ///
    /// **Not part of the value**, which is the whole distinction between a
    /// placeholder and pre-filled text: it never reaches `on_changed`, it is
    /// never selected, the caret sits at offset zero in front of it, and a
    /// screen reader hears it as the field's *label* rather than its value —
    /// see `semantics`. A placeholder that is really text in the field is the
    /// oldest bug in web forms and submits itself the moment nobody looks.
    pub placeholder: String,
    /// The colour the placeholder is drawn in.
    ///
    /// Separate from `style.color` rather than an alpha applied to it, because
    /// the theme has a colour for exactly this — a placeholder is a hint, and
    /// dimming the body colour by a fraction chosen here would not match it.
    pub placeholder_color: Color,
    pub on_selection_changed: Option<SelectionChanged>,
    /// Where a **modified** click reports the caret it wants *added*.
    ///
    /// Separate from `on_selection_changed` because the two gestures mean
    /// opposite things: a plain click replaces every caret, a ⌘-click adds one.
    /// One channel with a flag would put that decision here, which is not where
    /// it belongs — this object knows a modifier was held, not what the
    /// application does about it.
    ///
    /// `None` leaves a modified click behaving as a plain one, which is the
    /// right fallback: every field that has not asked for extra carets should
    /// not grow them by accident.
    pub on_add_caret: Option<SelectionChanged>,
    /// Told the whole value a keypress or a composition would produce.
    ///
    /// Its presence is also what makes the field a keyboard target — see
    /// [`RenderObject::is_focusable`].
    pub on_changed: Option<ValueChanged>,
    /// Optional styled runs supplied by the editor, already covering the value text.
    pub spans: Option<Rc<Vec<TextSpan>>>,
    /// The last shaped paragraph. Produced by `layout`, consumed by `paint` and
    /// by every gesture, so that a tap is resolved against the layout on screen.
    shaped: Option<Paragraph>,
    /// Where the in-progress selection drag started.
    ///
    /// Held here rather than read back from `value.selection.base` because
    /// several drag updates can arrive within one frame, and until a frame runs
    /// the value is still the one from before the drag began — so the anchor read
    /// out of it would be wherever the caret happened to be beforehand.
    drag_anchor: Cell<Option<usize>>,
    /// The x a run of up- or down-presses is aiming for.
    ///
    /// Read from the caret at the *start* of the run and kept for its whole
    /// length, which is what stops the caret drifting left every time it passes
    /// through a short line. Cleared by anything that is not a vertical move —
    /// a tap, a horizontal arrow, a keystroke — because the run is over.
    ///
    /// A `Cell`, and adopted across rebuilds, for the same two reasons
    /// `drag_anchor` is: keys arrive through `&self`, and this object is rebuilt
    /// between every keystroke.
    goal_column: Cell<Option<f32>>,
    /// How many lines PageUp and PageDown move.
    ///
    /// # Why this is a number and not the field's own height
    ///
    /// A page is "one screenful", and this object does not know how big a
    /// screenful is. Inside a scrollable — which is where any field long enough
    /// for PageDown to mean anything lives — the field is laid out at the *full
    /// height of its content*, so its own size is the whole document and paging
    /// by it would jump to the end every time.
    ///
    /// Only the viewport knows, and it is somebody else's render object. So the
    /// page is a property the caller sets, defaulting to something reasonable
    /// for an editor rather than to a guess dressed up as a measurement.
    page_lines: usize,
    /// The last tap, for counting repeats.
    ///
    /// `(when, where, how many)`. A `Cell` for the reason the two above are:
    /// gestures arrive through `&self`. Carried across rebuilds by
    /// [`adopt_reports`], because a rebuild between the two clicks of a
    /// double-click is routine — the first one moved the caret, which is a
    /// signal write, which is a rebuild.
    ///
    /// [`adopt_reports`]: RenderObject::adopt_reports
    last_tap: Cell<Option<(Duration, Offset, u32)>>,
    /// The width `shaped` was laid out against.
    ///
    /// Shaping is the expensive part of a text field by a wide margin, and it
    /// depends on exactly two things: what the text and style are, and how much
    /// room there was. The first is settled before `layout` is reached — a
    /// change to either produces an object whose `shaped` was not adopted — so
    /// this is the only thing left to check. `NAN` before the first layout,
    /// which no width ever equals.
    shaped_width: f32,
    /// Whether `Enter` opens a line or submits.
    ///
    /// `true` by default, because that is what this object has always done and
    /// it is the behaviour a field falls back to when nobody says otherwise. A
    /// single-line field is the *narrower* claim and so is the one that has to be
    /// asked for.
    multiline: bool,
    /// Told that `Enter` was pressed on a single-line field.
    ///
    /// Only consulted when [`multiline`](Self::multiline) is false. A field with
    /// no handler still refuses to open a line — declaring the field single-line
    /// is what decides that, and swallowing the key is the honest outcome when
    /// nobody is listening for it.
    on_submit: Option<Submitted>,
    /// The system pasteboard, if the application provided one.
    ///
    /// `None` is an ordinary state, not a misconfiguration: a test tree, a
    /// headless render, or a platform with no pasteboard. Cut, copy and paste are
    /// then reported unhandled, so the key travels on to whatever else might want
    /// it rather than being eaten by a field that cannot act on it.
    clipboard: Option<Rc<dyn Clipboard>>,
    /// Whether a line longer than the available width breaks onto a second
    /// visual row.
    ///
    /// `true` by default — see the type's own doc comment: "text wraps within
    /// the available width on its own". A caller that paints its own per-line
    /// chrome alongside the field — a gutter, in particular — cannot afford
    /// that: a gutter is one fixed-height row per *source* line, and the
    /// moment any line wraps, every row below it is shaped against a `lines`
    /// list one longer than the gutter's, and everything from there down
    /// points at the wrong line. [`wrap`](Self::wrap) is the escape hatch —
    /// laying the paragraph out at effectively infinite width so one source
    /// line is always exactly one visual line, at the cost of letting long
    /// lines run past the field's own box (which `paint` already does not
    /// clip against, so this is not a new kind of overflow).
    wrap: bool,
    /// Where this field publishes what it measured, if anyone asked.
    ///
    /// Written at the end of every layout — including the one that reuses an
    /// existing paragraph, because a field re-laid out at a new origin has the
    /// same rects and a caller must not be told they went away.
    ///
    /// Deliberately absent from [`layout_eq`](Self::layout_eq) in one
    /// direction only: attaching or swapping the probe does not require the
    /// paragraph to be shaped again, since publishing reads the paragraph
    /// rather than changing it. See [`TextLayoutProbe`].
    probe: Option<TextLayoutProbe>,
}

/// How many lines a page is, when nobody says.
///
/// Twenty is about a screenful in an editor pane at a readable size, and is the
/// number to override rather than to rely on.
pub const DEFAULT_PAGE_LINES: usize = 20;

impl RenderEditableText {
    /// How many lines PageUp and PageDown move. See [`page_lines`].
    ///
    /// [`page_lines`]: RenderEditableText::page_lines
    #[must_use]
    pub const fn page_lines(mut self, lines: usize) -> Self {
        // Zero would make both keys no-ops, which is a stranger outcome than
        // moving by one.
        self.page_lines = if lines == 0 { 1 } else { lines };
        self
    }

    #[must_use]
    pub fn new(value: TextEditingValue, style: TextStyle) -> Self {
        Self {
            value,
            style,
            align: TextAlign::default(),
            direction: None,
            selection_color: Color::BLUE.with_alpha(SELECTION_ALPHA),
            diagnostic_lines: Rc::new(Vec::new()),
            decorations: Rc::new(Vec::new()),
            diagnostic_color: Color::RED.with_alpha(28),
            cursor_color: Color::BLACK,
            cursor_width: DEFAULT_CURSOR_WIDTH,
            show_cursor: true,
            obscure: Obscured::No,
            placeholder: String::new(),
            placeholder_color: Color::BLACK,
            on_selection_changed: None,
            on_changed: None,
            spans: None,
            shaped: None,
            drag_anchor: Cell::new(None),
            on_add_caret: None,
            goal_column: Cell::new(None),
            page_lines: DEFAULT_PAGE_LINES,
            last_tap: Cell::new(None),
            shaped_width: f32::NAN,
            multiline: true,
            on_submit: None,
            clipboard: None,
            wrap: true,
            probe: None,
        }
    }

    /// Show one character per character held, instead of the text.
    #[must_use]
    pub const fn obscure(mut self, obscure: Obscured) -> Self {
        self.obscure = obscure;
        self
    }

    /// Whether a line longer than the available width breaks onto a second
    /// visual row. `true` (wrap) unless turned off.
    ///
    /// Turn this off for a field painted alongside per-source-line chrome —
    /// a gutter, line-keyed diagnostics — that assumes one source line is one
    /// visual row. See the field's own doc comment for why.
    #[must_use]
    pub const fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Publish this field's measured geometry into `probe` after every layout.
    ///
    /// The seam for everything an editor draws *beside* its text — a
    /// completion popup at the caret, a hint at the end of a line — none of
    /// which the field can draw itself, and all of which needs geometry that
    /// exists only after shaping. Read the module docs on
    /// [`TextLayoutProbe`]: the report a caller reads during `build` is the
    /// previous frame's.
    #[must_use]
    pub fn probe(mut self, probe: TextLayoutProbe) -> Self {
        self.probe = Some(probe);
        self
    }

    /// Publish the geometry of `paragraph`, if a probe was attached.
    fn publish_layout(&self, paragraph: &Paragraph) {
        let Some(probe) = &self.probe else {
            return;
        };
        let lines = (0..paragraph.line_count())
            .filter_map(|index| paragraph.line_rect(index))
            .collect();
        probe.publish(TextLayoutReport {
            lines,
            cursor: self.cursor_rect().unwrap_or_default(),
            paragraph: paragraph.size(),
            font_size: self.style.size,
        });
    }

    /// Show `placeholder` while the field is empty.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>, color: Color) -> Self {
        self.placeholder = placeholder.into();
        self.placeholder_color = color;
        self
    }

    /// Give the field a pasteboard, enabling cut, copy and paste.
    /// Mark source lines containing compiler diagnostics.
    #[must_use]
    pub fn diagnostics(mut self, lines: Rc<Vec<usize>>, color: Color) -> Self {
        self.diagnostic_lines = lines;
        self.diagnostic_color = color;
        self
    }

    /// Mark ranges of the text with squiggles, underlines, strikes or boxes.
    #[must_use]
    pub fn decorations(mut self, decorations: Rc<Vec<TextDecoration>>) -> Self {
        self.decorations = decorations;
        self
    }

    #[must_use]
    pub fn clipboard(mut self, clipboard: Option<Rc<dyn Clipboard>>) -> Self {
        self.clipboard = clipboard;
        self
    }

    /// Make `Enter` submit rather than open a line.
    #[must_use]
    pub fn single_line(mut self, on_submit: Option<Submitted>) -> Self {
        self.multiline = false;
        self.on_submit = on_submit;
        self
    }

    #[must_use]
    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub fn direction(mut self, direction: Option<TextDirection>) -> Self {
        self.direction = direction;
        self
    }

    #[must_use]
    pub fn cursor(mut self, color: Color, width: f32) -> Self {
        self.cursor_color = color;
        self.cursor_width = width;
        self
    }

    #[must_use]
    pub fn show_cursor(mut self, show: bool) -> Self {
        self.show_cursor = show;
        self
    }

    #[must_use]
    pub fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = color;
        self
    }

    #[must_use]
    pub fn on_selection_changed(mut self, handler: SelectionChanged) -> Self {
        self.on_selection_changed = Some(handler);
        self
    }

    /// Report the whole value a keypress or a composition would produce.
    #[must_use]
    pub fn on_changed(mut self, handler: ValueChanged) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// Supply styled text runs for syntax-highlighted editing.
    ///
    /// The spans are used only for shaping and painting. The underlying
    /// `TextEditingValue` remains the source of truth for cursor, selection,
    /// editing, and hit-testing offsets.
    #[must_use]
    pub fn spans(mut self, spans: Option<Rc<Vec<TextSpan>>>) -> Self {
        self.spans = spans;
        self
    }

    /// Carry out an intent that needs to know where the lines are.
    ///
    /// [`TextEditingValue::apply`] refuses these — where a line starts is a fact
    /// about the shaped text, and foundation has no fonts in it. Here the
    /// paragraph is at hand, so this is where they are answered.
    ///
    /// Returns `false` when the movement has nowhere to go: pressing up on the
    /// first line is not an edit, and reporting it as one would ask for a frame
    /// that draws exactly what is already there.
    /// Carry out a geometry-dependent intent, at every caret.
    ///
    /// # Why the loop is here and not in `TextEditingValue`
    ///
    /// These are the intents that crate refuses, and refuses for every caret
    /// alike: "up a line" is a fact about the shaped paragraph and nothing in
    /// the editing model can answer it. So this is where multiple carets have
    /// to be handled for them, and it is done exactly as the model does it for
    /// the intents it *can* carry out — each caret independently, then
    /// normalised, because two carets that reach the same place are one caret.
    ///
    /// Movement changes no bytes, so unlike an edit there is nothing to shift
    /// and the order does not matter.
    fn apply_with_layout(&self, value: &mut TextEditingValue, intent: &TextIntent) -> bool {
        if self.shaped.is_none() {
            return false;
        }
        if !value.is_multi_caret() {
            return match self.moved_selection(value, value.selection, intent) {
                Some(selection) => {
                    value.selection = selection;
                    true
                }
                None => false,
            };
        }

        let carets = value.carets();
        let mut moved = false;
        let mut results = Vec::with_capacity(carets.len());
        for caret in carets {
            match self.moved_selection(value, caret, intent) {
                Some(selection) => {
                    moved = true;
                    results.push(selection);
                }
                // A caret already at the top of the file cannot go up, and
                // must not be dropped because of it — every editor keeps it
                // where it is while the others move.
                None => results.push(caret),
            }
        }
        if !moved {
            return false;
        }
        value.selection = results[0];
        value.secondary = results[1..].to_vec();
        value.normalise_carets();
        true
    }

    /// Where `selection` lands under a geometry-dependent `intent`.
    fn moved_selection(
        &self,
        value: &TextEditingValue,
        selection: TextSelection,
        intent: &TextIntent,
    ) -> Option<TextSelection> {
        let paragraph = self.shaped.as_ref()?;
        // Into the paragraph's coordinates for the whole of this function, and
        // back out once at the end. Every `paragraph.*` call below answers in
        // display offsets, so converting per arm would be six chances to miss
        // one.
        let cursor = self
            .obscure
            .position_to_display(&value.text, selection.cursor());

        let (target, extend) = match intent {
            TextIntent::MovePageUp { extend } | TextIntent::MovePageDown { extend } => {
                let up = matches!(intent, TextIntent::MovePageUp { .. });
                let goal = self.goal(paragraph, cursor);
                let mut at = cursor;
                let mut moved = false;
                for _ in 0..self.page_lines {
                    let next = if up {
                        paragraph.position_above(at, goal)
                    } else {
                        paragraph.position_below(at, goal)
                    };
                    match next {
                        Some(position) => {
                            at = position;
                            moved = true;
                        }
                        // Fewer than a page of lines left. Stopping at the edge
                        // is what every editor does — PageDown near the bottom
                        // goes to the bottom rather than doing nothing.
                        None => break,
                    }
                }
                if !moved {
                    return None;
                }
                (at, *extend)
            }
            TextIntent::MoveLineStart { extend } => (paragraph.line_start(cursor), *extend),
            TextIntent::MoveLineEnd { extend } => (paragraph.line_end(cursor), *extend),
            // The column is read once, at the start of the run, and kept for
            // the whole of it. Reading it fresh each time is the version that
            // looks right until you press up through a short line: the caret
            // lands at that line's end, and every press after it aims there.
            TextIntent::MoveUp { extend } => {
                let goal = self.goal(paragraph, cursor);
                (paragraph.position_above(cursor, goal)?, *extend)
            }
            TextIntent::MoveDown { extend } => {
                let goal = self.goal(paragraph, cursor);
                (paragraph.position_below(cursor, goal)?, *extend)
            }
            // The arrows, and they move *visually*. In left-to-right text that
            // is the same as one byte along; in a line with a Hebrew phrase and
            // a number in it it is not, and a caret that stepped by byte offset
            // jumps across the screen in the middle of a word — the single most
            // confusing thing a field can do to somebody typing bidi text.
            //
            // `None` here is the start or the end of the whole paragraph, and
            // the fallback in `handle_key` covers the case where there is no
            // geometry to move through at all.
            TextIntent::MovePrevious { extend } => (paragraph.position_left_of(cursor)?, *extend),
            TextIntent::MoveNext { extend } => (paragraph.position_right_of(cursor)?, *extend),
            _ => return None,
        };

        let target = self.obscure.position_from_display(&value.text, target);
        // Clamped by the model, which owns what a legal offset is.
        let mut one = value.clone();
        one.selection = selection;
        one.move_to(target, extend);
        Some(one.selection)
    }

    /// The column a vertical run is aiming for, remembering it if this is the
    /// first press of the run.
    fn goal(&self, paragraph: &Paragraph, cursor: TextPosition) -> f32 {
        match self.goal_column.get() {
            Some(goal) => goal,
            None => {
                let goal = paragraph.cursor_rect(cursor).left;
                self.goal_column.set(Some(goal));
                goal
            }
        }
    }

    /// The spans this field actually shapes: the mask, placeholder, or syntax-highlighted text.
    fn display_spans(&self) -> Vec<TextSpan> {
        if self.value.text.is_empty() && !self.placeholder.is_empty() {
            let mut style = self.style;
            style.color = self.placeholder_color;
            return vec![TextSpan::new(self.placeholder.clone(), style)];
        }
        if matches!(self.obscure, Obscured::No) {
            if let Some(spans) = &self.spans {
                if !spans.is_empty() {
                    return (**spans).clone();
                }
            }
        }
        vec![TextSpan::new(
            self.obscure.display(&self.value.text).into_owned(),
            self.style,
        )]
    }

    /// Whether the paragraph currently holds the placeholder rather than a value.
    ///
    /// The caret and the selection have to ignore the paragraph's extent in this
    /// state: an empty field's caret belongs at offset zero, not at the end of
    /// the hint, and dragging across a placeholder must select nothing because
    /// there is nothing there to select.
    fn showing_placeholder(&self) -> bool {
        self.value.text.is_empty() && !self.placeholder.is_empty()
    }

    /// A range in the real text, in the displayed text's coordinates.
    fn display_range(&self, range: TextRange) -> TextRange {
        TextRange::new(
            self.obscure.to_display(&self.value.text, range.start),
            self.obscure.to_display(&self.value.text, range.end),
        )
    }

    /// A position reported by the paragraph, back in the real text's coordinates.
    ///
    /// Clamped to zero while the placeholder is showing: the paragraph on screen
    /// is the hint, so a tap in the middle of it answers with an offset into
    /// text the field does not contain.
    fn text_position(&self, position: TextPosition) -> TextPosition {
        if self.showing_placeholder() {
            return TextPosition::new(0);
        }
        self.obscure
            .position_from_display(&self.value.text, position)
    }

    /// The paragraph shaped by the last layout, if any.
    #[must_use]
    pub const fn shaped(&self) -> Option<&Paragraph> {
        self.shaped.as_ref()
    }

    /// The caret's box in local coordinates, or `None` before the first layout.
    ///
    /// The width comes from [`cursor_width`](Self::cursor_width); the paragraph
    /// reports a zero-width rect, since thickness is not a layout question.
    #[must_use]
    pub fn cursor_rect(&self) -> Option<Rect> {
        // **Into the paragraph's coordinates first.** What is on screen is the
        // *display* text — the mask, or the placeholder — and the selection is
        // in the real text's. Handing a real offset straight over is the bug
        // that put the caret of a seven-character password between the second
        // and third bullet: `TextPosition` is a byte offset and one masked
        // character is three bytes, so the seven bytes of "hunter2" walked two
        // and a third bullets into the mask. Everything else crossing this
        // boundary converts — `display_range` for the highlight,
        // `apply_with_layout` for movement — and this was the one that did not.
        let cursor = self
            .obscure
            .position_to_display(&self.value.text, self.value.selection.cursor());
        let caret = self.shaped.as_ref()?.cursor_rect(cursor);
        Some(Rect::new(
            caret.left,
            caret.top,
            caret.left + self.cursor_width,
            caret.bottom,
        ))
    }

    /// Where every caret is drawn, in the field's own coordinates.
    ///
    /// One entry for a single-caret field, which is every field that has not
    /// asked for more — see [`TextEditingValue::secondary`].
    #[must_use]
    pub fn caret_rects(&self) -> Vec<Rect> {
        let Some(paragraph) = &self.shaped else {
            return Vec::new();
        };
        self.value
            .carets()
            .into_iter()
            .map(|selection| {
                let cursor = self
                    .obscure
                    .position_to_display(&self.value.text, selection.cursor());
                let caret = paragraph.cursor_rect(cursor);
                Rect::new(
                    caret.left,
                    caret.top,
                    caret.left + self.cursor_width,
                    caret.bottom,
                )
            })
            .collect()
    }

    /// Draw the range-keyed marks: squiggles, underlines, strikes, boxes.
    ///
    /// # Why every shape is derived from `selection_rects`
    ///
    /// A range can span several visual lines, and on a wrapped line it can
    /// start mid-row and end mid-row. `Paragraph::selection_rects` already
    /// solves exactly that problem for the selection highlight, so a
    /// decoration that asked its own question would be a second answer to a
    /// settled one — and the two would disagree on the day the shaper changes
    /// how it breaks.
    ///
    /// # The squiggle and the box arrive as fills, not strokes
    ///
    /// Their pens are expanded into fillable outlines before they reach the
    /// canvas — the same conversion `RenderIcon::paint` makes, for the same
    /// reason: a thin stroke is the primitive the backends rasterise least
    /// agreeably, and a squiggle under a misspelled word is exactly where a
    /// disagreement is noticed. `Canvas::stroke_path` still exists and every
    /// real backend implements it, but a mark drawn through it is a mark whose
    /// crispness depends on where it was drawn.
    fn paint_decorations(&self, ctx: &mut PaintCtx<'_>, paragraph: &Paragraph, origin: Offset) {
        if self.decorations.is_empty() {
            return;
        }
        for decoration in self.decorations.iter() {
            if decoration.color.is_transparent() || decoration.range.is_empty() {
                continue;
            }
            let paint = Paint::solid(decoration.color);
            let thickness = decoration.resolved_thickness(self.style.size);
            // Into display coordinates, the same crossing the selection makes.
            // An obscured field's decorations would otherwise land wherever the
            // real byte offsets happen to fall inside the mask.
            let rects = paragraph.selection_rects(self.display_range(decoration.range));

            for rect in rects {
                let rect = rect.translate(origin);
                match decoration.shape {
                    TextDecorationShape::Underline => {
                        ctx.canvas().fill_rect(
                            Rect::new(rect.left, rect.bottom - thickness, rect.right, rect.bottom),
                            paint,
                        );
                    }
                    TextDecorationShape::Strike => {
                        let middle = (rect.top + rect.bottom) / 2.0;
                        ctx.canvas().fill_rect(
                            Rect::new(
                                rect.left,
                                middle - thickness / 2.0,
                                rect.right,
                                middle + thickness / 2.0,
                            ),
                            paint,
                        );
                    }
                    // Box and squiggle are strokes by design, but the pen is
                    // expanded into its fillable outline rather than handed to
                    // the rasteriser as a stroke: the same conversion
                    // `RenderIcon::paint` makes, for the same reason — a
                    // thin stroke is the primitive backends rasterise least
                    // consistently, and a squiggle under a misspelled word is
                    // exactly where an inconsistent one is noticed.
                    TextDecorationShape::Box => {
                        ctx.canvas().fill_path(
                            &vieww_foundation::Path::rect(rect).stroke_outline(thickness),
                            paint,
                        );
                    }
                    TextDecorationShape::Squiggle => {
                        ctx.canvas()
                            .fill_path(&squiggle(rect, thickness).stroke_outline(thickness), paint);
                    }
                    // A position, not an extent. `rect.left` is the leading
                    // edge in a left-to-right paragraph and `rect.right` in a
                    // right-to-left one, which is what `selection_rects`
                    // already reports — so reading the leading edge means
                    // asking the rect, not assuming a direction.
                    TextDecorationShape::Guide => {
                        let leading = if paragraph.direction() == TextDirection::Rtl {
                            rect.right - thickness
                        } else {
                            rect.left
                        };
                        ctx.canvas().fill_rect(
                            Rect::new(leading, rect.top, leading + thickness, rect.bottom),
                            paint,
                        );
                    }
                }
            }
        }
    }

    /// Whether `new` describes the same geometry as this one.
    ///
    /// The caret's colour and visibility, the highlight colour and the selection
    /// itself are all missing on purpose: none of them can move a glyph, and a
    /// caret blinking at 2 Hz must not re-shape the paragraph twice a second.
    fn layout_eq(&self, new: &Self) -> bool {
        self.value.text == new.value.text
            && self.style == new.style
            && self.align == new.align
            && self.direction == new.direction
            && self.cursor_width == new.cursor_width
            // Both change what is *shaped*, so a paragraph kept across a change
            // to either is a field drawing its old contents. Cheap to compare
            // and catastrophic to omit: without the first line, turning
            // obscuring on leaves the password on screen.
            && self.obscure == new.obscure
            && self.placeholder == new.placeholder
            && self.spans == new.spans
            && self.wrap == new.wrap
    }

    /// The width the paragraph is shaped against.
    ///
    /// `constraints.max_width` with wrapping on — the ordinary case, and the
    /// field's own doc comment. Effectively infinite with it off, so a source
    /// line is always exactly one visual line; see [`wrap`](Self::wrap).
    fn wrap_width(&self, constraints: Constraints) -> f32 {
        if self.wrap {
            constraints.max_width
        } else {
            f32::INFINITY
        }
    }

    /// The offset under `local`, against the layout currently on screen.
    ///
    /// The paragraph answers in *display* coordinates — masked, or the
    /// placeholder — so the answer is converted before it leaves. This is the
    /// one place a tap crosses that boundary.
    /// Count this tap as a repeat of the last, and select what that implies.
    ///
    /// One click places a caret (already done on the press — see
    /// `Recognized::TapDown`), two select the word under it, three select the
    /// line. Four starts over at one, which is what every editor does and what
    /// stops a drum roll on the mouse from selecting ever-larger things.
    fn count_tap(&self, details: &TapDetails) {
        let count = match self.last_tap.get() {
            Some((when, at, count))
                if details.timestamp.saturating_sub(when) <= MULTI_TAP_TIMEOUT
                    && (details.local - at).distance() <= MULTI_TAP_SLOP =>
            {
                count % 3 + 1
            }
            _ => 1,
        };
        self.last_tap
            .set(Some((details.timestamp, details.local, count)));

        // A single click has already been handled on the press. Repeating that
        // work here would be harmless but would also undo a drag that started
        // and ended inside the slop radius.
        if count == 1 {
            return;
        }
        let Some(position) = self.position_at(details.local) else {
            return;
        };

        let mut value = self.value.clone();
        if count == 2 {
            value.select_word_at(position.offset);
        } else {
            // The visual line, not the paragraph: a wrapped line is what the
            // user sees and what they mean by "this line".
            let Some(paragraph) = self.shaped() else {
                return;
            };
            let start = paragraph.line_start(position);
            let end = paragraph.line_end(position);
            value.selection = TextSelection::new(start.offset, end.offset);
        }
        // Through `report`, so the anchor a following drag extends from is the
        // selection this made — dragging after a double-click extends by word
        // in every editor, and this is the half of that which lives here.
        self.report(value.selection);
    }

    fn position_at(&self, local: Offset) -> Option<TextPosition> {
        let hit = self.shaped.as_ref()?.hit_test(local);
        Some(self.text_position(hit))
    }

    /// Carry out cut, copy or paste, reporting whether the key was used.
    ///
    /// Separate from `handle_key` because it is the only part of this object that
    /// can *fail for an external reason* — a pasteboard the platform would not
    /// let us read — and mixing that into the intent match would put error
    /// handling in the middle of what is otherwise a lookup table.
    ///
    /// # Failures are silent, and that is the intended behaviour
    ///
    /// A refused pasteboard leaves the text alone and reports the key handled.
    /// The alternatives are worse: propagating means a widget needs an error
    /// channel it has nowhere to show, and reporting it unhandled sends Ctrl+V
    /// to an ancestor, so a failed paste could trigger some unrelated shortcut.
    /// Nothing happening is what a user expects from a pasteboard that would not
    /// open.
    fn use_clipboard(&self, intent: &TextIntent, handler: &ValueChanged) -> bool {
        let Some(clipboard) = &self.clipboard else {
            // Unhandled rather than swallowed: with no pasteboard this field has
            // no claim on the key, and something above may.
            return false;
        };

        match intent {
            TextIntent::Copy => {
                // A caret is not a selection. Copying the whole field here would
                // overwrite whatever the user actually had on their pasteboard.
                if self.value.selected_text().is_empty() {
                    return true;
                }
                let _ = clipboard.write_text(self.value.selected_text());
                true
            }
            TextIntent::Cut => {
                if self.value.selected_text().is_empty() {
                    return true;
                }
                if clipboard.write_text(self.value.selected_text()).is_err() {
                    // The text is *not* deleted if the write failed, or a cut
                    // that could not reach the pasteboard would destroy it.
                    return true;
                }
                let mut value = self.value.clone();
                // Deletes the selection rather than one grapheme, because there
                // is one — see `backspace_with_a_selection_deletes_the_selection`.
                value.delete_backward();
                handler(value);
                true
            }
            TextIntent::Paste => {
                let Ok(Some(text)) = clipboard.read_text() else {
                    // Empty, unreadable, or holding something that is not text.
                    // All three mean the same thing to a field.
                    return true;
                };
                let mut value = self.value.clone();
                // `insert` replaces the selection when there is one, which is
                // exactly paste's semantics.
                value.insert(&text);
                handler(value);
                true
            }
            // `needs_clipboard` is what routes here, so nothing else arrives.
            _ => false,
        }
    }

    /// Where a modified click reports the caret it wants added.
    #[must_use]
    pub fn on_add_caret(mut self, handler: SelectionChanged) -> Self {
        self.on_add_caret = Some(handler);
        self
    }

    fn report(&self, selection: TextSelection) {
        if let Some(handler) = &self.on_selection_changed {
            handler(selection);
        }
    }
}

/// A wave along the bottom of `rect`, one full period every four thicknesses.
///
/// Period tied to the line width rather than to a constant so the wave stays
/// recognisable as a wave at every font size: a fixed 4px period under 24pt
/// text reads as a grey smear, and under 8pt text as a sawtooth.
fn squiggle(rect: Rect, thickness: f32) -> Path {
    let period = (thickness * 4.0).max(3.0);
    let amplitude = thickness;
    // Half a wave of clearance above the baseline of the box, so the crest of
    // the wave sits under the glyphs rather than through their descenders.
    let middle = rect.bottom - amplitude;
    let mut path = Path::new();
    path.move_to(Offset::new(rect.left, middle));
    let mut x = rect.left;
    let mut up = true;
    while x < rect.right {
        let next = (x + period / 2.0).min(rect.right);
        path.line_to(Offset::new(
            next,
            if up {
                middle - amplitude
            } else {
                middle + amplitude
            },
        ));
        up = !up;
        x = next;
    }
    path
}

impl RenderObject for RenderEditableText {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // Reshape only when there is something new to shape against. A field is
        // laid out again for all sorts of reasons that have nothing to do with
        // it — a sibling changed, the window resized, a parent's constraints
        // shifted — and shaping a paragraph is the most expensive thing in this
        // object by a long way.
        let wrap_width = self.wrap_width(constraints);
        let reusable = self
            .shaped
            .as_ref()
            .is_some_and(|_| self.shaped_width == wrap_width);
        if reusable {
            let paragraph = self.shaped.as_ref().expect("checked");
            let size = Size::new(
                paragraph.size().width + self.cursor_width,
                paragraph.size().height,
            );
            // Published on the cheap path too. The rects are unchanged, so
            // `publish` finds them equal and wakes nobody — but a field that
            // only published when it re-shaped would leave a caller that
            // attached a probe *after* the last shape waiting forever.
            self.publish_layout(paragraph);
            return constraints.constrain(size);
        }

        // What is shaped is what is *shown*: the mask when obscured, the
        // placeholder when empty. Shaping the real text and drawing something
        // else over it is what puts a caret in the wrong place.
        let spans = self.display_spans();
        let paragraph = Paragraph::layout_aligned(
            ctx.fonts_mut(),
            &spans,
            wrap_width,
            self.align,
            self.direction,
        );
        // A caret sitting after the last character is one cursor width past the
        // text's own extent. Measuring without it clips the caret at the end of
        // every line, which is precisely where it spends most of its time.
        let size = Size::new(
            paragraph.size().width + self.cursor_width,
            paragraph.size().height,
        );
        self.shaped = Some(paragraph);
        self.shaped_width = wrap_width;
        self.publish_layout(self.shaped.as_ref().expect("just set"));
        // The reported size still respects `constraints`, even with wrapping off
        // and a paragraph wider than they allow: this object's box is what its
        // parent lays other things out against (a gutter, a minimap beside it),
        // and letting an unwrapped long line stretch that box would move them
        // every time a line grew. `paint` below draws the glyph runs at their
        // shaped positions regardless, so a long line still runs past the box —
        // overflowing rather than reflowing, which is what turning wrap off asked
        // for.
        constraints.constrain(size)
    }

    /// The first line's baseline, off the same paragraph `paint` draws from.
    ///
    /// A field answers even when it is empty, because what is shaped then is
    /// the placeholder — and a label beside an empty field should sit on the
    /// line the user is about to type on, not jump when they type.
    fn baseline(&self, _ctx: &mut crate::BaselineCtx<'_>, _size: Size) -> Option<f32> {
        self.shaped.as_ref()?.line_baseline(0)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(paragraph) = &self.shaped else {
            return;
        };
        let origin = ctx.origin();

        // Diagnostics are painted from the same line geometry as the text, so
        // the marker follows wrapping and never drifts from the reported line.
        if !self.diagnostic_lines.is_empty() && !self.diagnostic_color.is_transparent() {
            let paint = Paint::solid(self.diagnostic_color);
            for &line in self.diagnostic_lines.iter() {
                if let Some(rect) = paragraph.line_rect(line) {
                    ctx.canvas().fill_rect(rect.translate(origin), paint);
                }
            }
        }

        // Behind the glyphs: a highlight drawn on top would hide what is
        // selected. Every caret's, not only the primary's — a field with three
        // selections that highlighted one of them would be showing the user a
        // different edit from the one about to happen.
        if !self.selection_color.is_transparent() {
            let paint = Paint::solid(self.selection_color);
            for selection in self.value.carets() {
                if selection.is_collapsed() {
                    continue;
                }
                for rect in paragraph.selection_rects(self.display_range(selection.range())) {
                    ctx.canvas().fill_rect(rect.translate(origin), paint);
                }
            }
        }

        // The run origin snapped to the physical pixel grid — see
        // `RenderText::paint` for the argument. A field's glyphs deserve the
        // same texel alignment a label's get, and a field is where the eye is
        // parked while typing, which is where soft text is noticed first.
        let dpr = ctx.device_pixel_ratio().max(1.0);
        for run in paragraph.runs() {
            let mut placed = run.clone();
            let dx = ((run.origin.dx + origin.dx) * dpr).round() / dpr;
            let dy = ((run.origin.dy + origin.dy) * dpr).round() / dpr;
            placed.origin = Offset::new(dx, dy);
            ctx.canvas().draw_glyphs(&placed);
        }

        // Composing text is provisional, and the underline is how a user is told
        // so. Under the glyphs' baseline rather than behind them, or the marks it
        // is meant to distinguish would be the thing it covers.
        if let Some(composing) = self.value.composing {
            let thickness = (self.style.size * COMPOSING_UNDERLINE_RATIO).max(1.0);
            let paint = Paint::solid(self.style.color);
            for rect in paragraph.selection_rects(self.display_range(composing)) {
                let underline =
                    Rect::new(rect.left, rect.bottom - thickness, rect.right, rect.bottom);
                ctx.canvas().fill_rect(underline.translate(origin), paint);
            }
        }

        // **Over the glyphs, not under them.** A squiggle is read against the
        // word it marks, and one drawn behind the text disappears under every
        // descender it was supposed to sit beside. The composing underline
        // above is the opposite case on purpose — it marks text that is
        // *provisional*, so it must not cover the marks it distinguishes.
        self.paint_decorations(ctx, paragraph, origin);

        // On top, so it stays visible over a glyph it happens to sit against.
        // One per caret, all blinking together: carets that blinked out of
        // phase would read as several fields rather than one.
        if self.show_cursor && !self.cursor_color.is_transparent() {
            let paint = Paint::solid(self.cursor_color);
            for caret in self.caret_rects() {
                ctx.canvas().fill_rect(caret.translate(origin), paint);
            }
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Opaque, including the empty space after the last character: tapping
        // past the end of a line is how a caret is put at the end of it.
        true
    }

    fn is_focusable(&self) -> bool {
        // A field with nowhere to report changes is a label that happens to be
        // drawn by this object. Taking the keyboard would be a black hole: every
        // key would be swallowed and nothing would come of any of them.
        self.on_changed.is_some()
    }

    fn handle_key(&self, event: &vieww_foundation::KeyEvent) -> bool {
        let Some(handler) = &self.on_changed else {
            return false;
        };
        // The platform decides what the chord means — Command on Apple, Control
        // elsewhere, Alt-arrow versus Ctrl-arrow for word movement. That is a
        // `TargetPlatform` branch by DESIGN §8, and it is resolved at compile
        // time, so the arm for the other platform is discarded.
        let Some(intent) = event.text_intent(TargetPlatform::current()) else {
            return false;
        };

        // Anything that is not a vertical move ends the run, so the next up or
        // down starts aiming from wherever the caret has ended up.
        if !matches!(
            intent,
            TextIntent::MoveUp { .. }
                | TextIntent::MoveDown { .. }
                | TextIntent::MovePageUp { .. }
                | TextIntent::MovePageDown { .. }
        ) {
            self.goal_column.set(None);
        }

        // A single-line field takes `Enter` off the table before the value ever
        // sees it. `TextEditingValue::apply` turns the intent into a `\n`
        // unconditionally and says so in its own documentation: a field that
        // wants to submit has to match on the intent first, which is here.
        //
        // The key is reported handled either way. A submit is a use of the key,
        // and a single-line field with no handler must still not grow — leaving
        // it unhandled would offer it to an ancestor, and a scroll view or a
        // shortcut layer taking `Enter` out of a text field is a worse surprise
        // than nothing happening.
        if !self.multiline && matches!(intent, TextIntent::Newline) {
            if let Some(submit) = &self.on_submit {
                submit(self.value.text.clone());
            }
            return true;
        }

        if intent.needs_clipboard() {
            return self.use_clipboard(&intent, handler);
        }

        let mut value = self.value.clone();
        // Geometry first, logic second. Everything `apply_with_layout` does not
        // recognise falls straight through, and so does an arrow key on a field
        // that has never been laid out — there is no geometry to move through
        // there, and moving by byte offset is the right answer when there is no
        // better one.
        let handled = self.apply_with_layout(&mut value, &intent)
            || (!intent.needs_layout() && value.apply(&intent));

        if handled {
            handler(value);
        }
        handled
    }

    fn accepts_text(&self) -> bool {
        self.on_changed.is_some()
    }

    fn ime_cursor_area(&self) -> Option<Rect> {
        self.cursor_rect()
    }

    fn handle_ime(&self, event: &vieww_foundation::ImeEvent) -> bool {
        let Some(handler) = &self.on_changed else {
            return false;
        };
        let mut value = self.value.clone();
        if !value.apply_ime(event) {
            return false;
        }
        handler(value);
        true
    }

    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        if self.on_selection_changed.is_none() {
            return Vec::new();
        }
        // Tap before drag, matching `RenderGestureDetector`: a press that
        // resolves nothing should become a tap and place the caret.
        vec![
            Box::new(TapRecognizer::new()),
            Box::new(DragRecognizer::new()),
        ]
    }

    fn handle_gesture(&self, gesture: &Recognized, _local: Offset) {
        // A finger moving the caret ends any vertical run: the next up-press
        // aims from where the tap put it, not from wherever the arrows had been
        // heading before it.
        self.goal_column.set(None);
        match gesture {
            // **The caret lands on the press, not on the release.**
            //
            // It used to be placed on `Tap`, which is emitted when the finger
            // *lifts* and the arena has granted the gesture. That is right for a
            // button — a press you slide off should not activate it — and wrong
            // for text: every editor on every platform moves the caret the
            // moment the button goes down, and a caret that waits for the
            // release feels like the click was dropped.
            //
            // `TapDown` carries the *origin* — where the finger came down — so
            // the caret lands where the user pointed rather than where they
            // happened to let go. It arrives `PRESS_TIMEOUT` after the down, or
            // immediately if the tap was faster than that; a tenth of a second
            // is below the threshold at which a caret reads as lagging.
            //
            // A press that goes on to become a drag has already placed the
            // caret at the same origin `DragStart` would use, so the two agree
            // and the selection grows from where it was put.
            //
            // **And a modified click means something else.** `TapDetails`
            // carries the keys that were held (`PointerEvent::modifiers`),
            // which is what makes the two behaviours every editor has
            // implementable at all:
            //
            // * **⇧-click extends** — the anchor stays where it was and the
            //   focus moves to the click, exactly as a drag to the same point
            //   would leave it. Kept on `on_selection_changed`, because it is
            //   still one selection.
            // * **⌘-click (Ctrl elsewhere) adds a caret** — reported on
            //   `on_add_caret` instead, and falling back to a plain click when
            //   nothing is listening, so a field that never asked for extra
            //   carets cannot grow one.
            //
            // The order matters: Shift wins when both are held, because
            // extending is the older and more universal behaviour and a
            // ⌘⇧-click that silently added a caret would be a surprise.
            Recognized::TapDown(details) => {
                if let Some(position) = self.position_at(details.local) {
                    self.drag_anchor.set(None);
                    let at =
                        TextSelection::collapsed(position.offset).with_affinity(position.affinity);

                    if details.modifiers.contains(Modifiers::SHIFT) {
                        // From wherever the selection is anchored now. `base`
                        // rather than `start()`: a selection built backwards
                        // has its anchor at the higher offset, and extending
                        // from the lower one would flip it under the finger.
                        let extended =
                            TextSelection::new(self.value.selection.base, position.offset)
                                .with_affinity(position.affinity);
                        // The anchor a following drag extends from, so
                        // ⇧-click-then-drag keeps growing from the same place.
                        self.drag_anchor.set(Some(self.value.selection.base));
                        self.report(extended);
                    } else if details.modifiers.contains(Modifiers::META)
                        || details.modifiers.contains(Modifiers::CONTROL)
                    {
                        // Either, rather than the host's shortcut modifier:
                        // ⌘-click is the Apple convention and Ctrl-click the
                        // one everywhere else, and no text field does anything
                        // else with the other, so accepting both costs nothing
                        // and saves this object having to know its platform.
                        match &self.on_add_caret {
                            Some(handler) => handler(at),
                            None => self.report(at),
                        }
                    } else {
                        self.report(at);
                    }
                }
            }
            // The release is no longer where the caret moves, but it is still
            // where a *repeat* click is counted — see `taps` below.
            Recognized::Tap(details) => self.count_tap(details),
            Recognized::DragStart(details) => {
                // A drag is only certain once the finger has passed the slop
                // radius, so `local` here is where it got to, not where it went
                // down — and `delta` is exactly the distance between the two. A
                // selection anchored at `local` starts a character or two into
                // the word the user began dragging from, which looks like the
                // field ignoring the start of every selection.
                if let Some(position) = self.position_at(details.local - details.delta) {
                    self.drag_anchor.set(Some(position.offset));
                    self.report(
                        TextSelection::collapsed(position.offset).with_affinity(position.affinity),
                    );
                }
            }
            Recognized::DragUpdate(details) => {
                if let Some(position) = self.position_at(details.local) {
                    let anchor = self.drag_anchor.get().unwrap_or(self.value.selection.base);
                    self.report(
                        TextSelection::new(anchor, position.offset)
                            .with_affinity(position.affinity),
                    );
                }
            }
            Recognized::DragEnd(_) => self.drag_anchor.set(None),
            _ => {}
        }
    }

    /// Shaped, exactly as [`RenderText`](crate::RenderText) does it, plus a
    /// caret.
    ///
    /// # Why this can be answered at all
    ///
    /// A field looks like the one object here that would have to say "I do not
    /// know": it holds a caret, a selection, a blink and a drag anchor, and none
    /// of those may be consulted by a function the contract calls pure. But none
    /// of them is *needed*. What a field measures is
    /// `display_span` — the mask, the placeholder, or the
    /// text — and every input to that is configuration handed down from above:
    /// the value's text, the style, the obscuring, the placeholder. The
    /// selection cannot move a glyph, which is the same fact `layout_eq` is
    /// built on.
    ///
    /// So this shapes fresh rather than reading the cached
    /// [`shaped`](Self::shaped) paragraph. The cache is a *layout* artefact: it
    /// was shaped against whatever width the last layout happened to have, and
    /// an intrinsic asked at a different width that returned it would answer a
    /// question nobody asked — and would return `None` before the first layout,
    /// which is the moment an `Accordion` or an `IntrinsicWidth` actually asks.
    /// Shaping is expensive, and the per-node cache in
    /// the `intrinsics` module is what stops it being expensive twice.
    ///
    /// # The caret is part of the answer
    ///
    /// `layout` adds [`cursor_width`](Self::cursor_width) to the paragraph's
    /// width, because a caret sitting after the last character is that much past
    /// the text's own extent. An intrinsic that reported the bare paragraph
    /// would hand an `IntrinsicWidth` a box one caret too narrow, and the caret
    /// at the end of the line — where it spends most of its life — would be
    /// clipped. It is added to the minimum as well as the maximum for the same
    /// reason: there is no width at which the field stops needing somewhere to
    /// put the caret.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        use crate::Extremum;

        let spans = self.display_spans();
        let width = match (query.axis, query.extremum) {
            // As wide as it wants: one line, no wrapping.
            (Axis::Horizontal, Extremum::Max) => f32::INFINITY,
            // As narrow as it goes: shaped against zero, the shaper still
            // refuses to break inside a word, so the widest line it produces is
            // the narrowest the field can be without a word overflowing.
            (Axis::Horizontal, Extremum::Min) => 0.0,
            // Height is a function of the width, so it is whatever the caller
            // said, or one line if it said nothing. Shaped against `cross`
            // unchanged, which is what `layout` does with `max_width` — the two
            // have to use the same number or the height reported here would not
            // be the height the field ends up with.
            (Axis::Vertical, _) => query.cross.unwrap_or(f32::INFINITY),
        };

        let paragraph =
            Paragraph::layout_aligned(ctx.fonts_mut(), &spans, width, self.align, self.direction);
        Some(match query.axis {
            // `widest_line` rather than `size().width`: `size()` is clamped to
            // the width it was shaped against, so the minimum — shaped against
            // zero — would come back as zero. That is the mistake `RenderText`
            // documents having made, and it is worth stating twice because zero
            // is precisely the wrong answer to report here.
            Axis::Horizontal => paragraph.widest_line() + self.cursor_width,
            Axis::Vertical => paragraph.size().height,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        let new: &dyn Any = new;
        new.downcast_ref::<Self>()
            .is_none_or(|new| !self.layout_eq(new))
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        let old: &dyn Any = old;
        let Some(old) = old.downcast_ref::<Self>() else {
            return;
        };
        // Reached only when `layout_differs` said no, so the text and style match
        // and the old shaping is still the right answer. Without it the field
        // disappears on the first blink, since a blink is a rebuild that changes
        // nothing about the geometry.
        //
        // That gate is load-bearing: `layout` reuses `shaped` rather than
        // recomputing it, so adopting across a real edit means the field never
        // reshapes. Three tests in `tap_to_caret` pin it.
        self.shaped.clone_from(&old.shaped);
        self.shaped_width = old.shaped_width;
        // And the drag in progress survives with it: a selection drag rebuilds
        // between every update, and an anchor that reset each time would collapse
        // the selection back to a caret on every frame.
        self.drag_anchor.set(old.drag_anchor.get());
        // And so does the column a run of arrow presses is aiming for, for
        // exactly the same reason: every press rebuilds this object, and a goal
        // that reset each time is a goal that never survives to be used.
        self.goal_column.set(old.goal_column.get());
    }

    fn semantics(&self) -> Option<crate::Semantics> {
        // The text is the *value*, not the label: a screen reader says "text
        // field, hello" and needs to know which word is which. A label — what
        // the field is *for* — normally has to come from outside, since this
        // object has only ever been told what is in it.
        let mut semantics = crate::Semantics::new(crate::Role::TextField);

        // **An obscured field does not put its contents in the semantics tree.**
        // A screen reader is a second way to read the pixels, and the whole
        // point of the mask is that the pixels do not carry the password — a
        // tree that carried it anyway would hand it to anything that can read
        // the accessibility API, which on desktop is quite a lot of things.
        //
        // It reports the *length* instead, which is what the mask already shows
        // on screen, so somebody who cannot see it learns exactly as much as
        // somebody who can and no more.
        if self.obscure.is_obscured() {
            let count = self.value.text.chars().count();
            semantics = semantics.with_value(match count {
                0 => String::new(),
                1 => "1 character".to_owned(),
                many => format!("{many} characters"),
            });
        } else {
            semantics = semantics.with_value(self.value.text.clone());
        }

        // A placeholder is what the field is *for*, so it is the label — the one
        // role it can honestly play. Announcing it as the value would tell
        // somebody the field already contains the hint, which is the same lie
        // the pixels would tell if the placeholder were really in the text.
        if !self.placeholder.is_empty() {
            semantics = semantics.with_label(self.placeholder.clone());
        }

        Some(semantics)
    }

    /// Carry the repeat-click count across a rebuild.
    ///
    /// Without this, double-click never works: the first click moves the caret,
    /// moving the caret writes a signal, writing a signal rebuilds the field,
    /// and the fresh object has no memory of the click that just happened. The
    /// goal column travels for the same reason — a rebuild between two presses
    /// of ArrowDown must not lose which column the run started in — and so does
    /// the drag anchor.
    fn adopt_reports(&mut self, old: &dyn RenderObject) {
        let any: &dyn Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.last_tap.set(old.last_tap.get());
            self.goal_column.set(old.goal_column.get());
            self.drag_anchor.set(old.drag_anchor.get());
        }
    }

    /// An I-beam, which is what "you can type here" looks like on every desktop.
    ///
    /// Unconditional: a read-only field is still text a person can select, and
    /// selection is what the I-beam actually promises.
    fn cursor(&self, _local: Offset) -> Option<Cursor> {
        Some(Cursor::Text)
    }

    fn debug_name(&self) -> &'static str {
        "RenderEditableText"
    }
}

impl fmt::Debug for RenderEditableText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The handler is a closure, and the paragraph is a page of glyph
        // positions that says nothing useful in a tree dump.
        f.debug_struct("RenderEditableText")
            .field("text", &self.value.text)
            .field("selection", &self.value.selection)
            .field("composing", &self.value.composing)
            .field("show_cursor", &self.show_cursor)
            .field("obscure", &self.obscure)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use vieww_foundation::{Affinity, DragDetails, TapDetails};

    use super::*;
    use crate::RenderTree;

    fn value(text: &str) -> TextEditingValue {
        TextEditingValue::new(text)
    }

    fn style() -> TextStyle {
        TextStyle::new(16.0)
    }

    fn loose(width: f32) -> Constraints {
        Constraints::loose(Size::new(width, f32::INFINITY))
    }

    /// Lay an object out in a tree and hand back the tree and its id.
    fn mount(object: RenderEditableText) -> (RenderTree, crate::RenderId, Size) {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        let size = tree.layout(id, loose(f32::INFINITY));
        (tree, id, size)
    }

    fn scene_of(tree: &RenderTree) -> vieww_paint::Scene {
        let mut scene = vieww_paint::Scene::new();
        tree.paint(&mut scene);
        scene
    }

    /// The mounted object, as its own type.
    fn editable(tree: &RenderTree, id: crate::RenderId) -> &RenderEditableText {
        let object: &dyn Any = tree.object(id).expect("mounted");
        object
            .downcast_ref::<RenderEditableText>()
            .expect("mounted as its own type")
    }

    /// Where the caret for `offset` is drawn, as a point to tap.
    fn caret_point(paragraph: &Paragraph, offset: usize) -> Offset {
        let rect = paragraph.cursor_rect(TextPosition::new(offset));
        Offset::new(rect.left, (rect.top + rect.bottom) / 2.0)
    }

    /// The press. **This** is where the caret moves — see the `TapDown` arm of
    /// `handle_gesture`.
    fn press(local: Offset) -> Recognized {
        Recognized::TapDown(TapDetails::at(local))
    }

    /// The release, which counts the click for double- and triple-click but
    /// does not move the caret.
    fn tap_at(local: Offset, at: Duration) -> Recognized {
        let mut details = TapDetails::at(local);
        details.timestamp = at;
        Recognized::Tap(details)
    }

    fn drag(local: Offset) -> DragDetails {
        DragDetails {
            position: local,
            local,
            delta: Offset::ZERO,
            velocity: Offset::ZERO,
            timestamp: std::time::Duration::ZERO,
            modifiers: Modifiers::NONE,
        }
    }

    // ------------------------------------------------------------------ layout

    #[test]
    fn the_box_leaves_room_for_a_caret_after_the_last_character() {
        let (.., size) = mount(RenderEditableText::new(value("hello"), style()));

        let mut plain = RenderTree::new();
        let label = plain.insert(None, Box::new(crate::RenderText::new("hello", style())));
        let text_only = plain.layout(label, loose(f32::INFINITY));

        assert!(
            size.width > text_only.width,
            "a caret at the end of the text would be clipped otherwise: {size} vs {text_only}"
        );
    }

    #[test]
    fn an_empty_field_is_still_tall_enough_to_type_in() {
        let (.., size) = mount(RenderEditableText::new(value(""), style()));
        assert!(size.height > 0.0, "{size}");
        assert!(
            size.width > 0.0,
            "the caret has width even with no text: {size}"
        );
    }

    // ------------------------------------------------------------------- paint

    #[test]
    fn a_caret_is_painted_when_shown_and_not_when_hidden() {
        let (shown, ..) = mount(RenderEditableText::new(value("hi"), style()));
        let (hidden, ..) = mount(RenderEditableText::new(value("hi"), style()).show_cursor(false));

        assert_eq!(
            scene_of(&shown).fills().len(),
            1,
            "the caret is the only rect: no selection, no composition"
        );
        assert!(
            scene_of(&hidden).fills().is_empty(),
            "hiding the caret is how the blink's off phase is drawn"
        );
    }

    #[test]
    fn a_selection_is_painted_behind_the_glyphs() {
        let mut editable = RenderEditableText::new(value("hello"), style());
        editable.value.selection = TextSelection::new(1, 4);
        let (tree, ..) = mount(editable);
        let scene = scene_of(&tree);

        assert!(
            scene.fills().len() >= 2,
            "a highlight and a caret: {}",
            scene.fills().len()
        );
        assert!(
            !scene.glyph_runs().is_empty(),
            "and the text is still drawn over it"
        );
    }

    #[test]
    fn composing_text_is_underlined() {
        let mut editable = RenderEditableText::new(value("kana"), style());
        editable.value.composing = Some(TextRange::new(0, 4));
        let (tree, ..) = mount(editable);

        let underlines = scene_of(&tree)
            .fills()
            .into_iter()
            .filter(|(rect, _)| rect.height() < 4.0)
            .count();
        assert!(underlines > 0, "provisional text has to look provisional");
    }

    /// **A field with a selection must survive an input method resetting.**
    ///
    /// The whole path, not just the value: X11 and Wayland input methods send
    /// `Preedit("")` on a pointer press, and this object hands every IME event
    /// to `apply_ime` and reports whatever comes back through `on_changed`. So
    /// the field is what turned an IME reset into an edit of the document —
    /// selecting text deleted it.
    #[test]
    fn an_ime_reset_does_not_report_a_change_over_a_selection() {
        let changes: Rc<RefCell<Vec<TextEditingValue>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&changes);
        let mut initial = value("the quick brown fox");
        initial.selection = TextSelection::new(4, 19);

        let field = RenderEditableText::new(initial, style())
            .on_changed(Rc::new(move |value| sink.borrow_mut().push(value)));

        let handled = field.handle_ime(&vieww_foundation::ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        });

        assert!(!handled, "there was no composition to end");
        assert!(
            changes.borrow().is_empty(),
            "and so nothing to report: {:?}",
            changes.borrow()
        );
    }

    // ------------------------------------------------------------- hit testing

    #[test]
    fn pressing_reports_the_offset_under_the_finger() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let paragraph = object.shaped().expect("laid out").clone();

        // Press exactly where the caret for offset 3 would be drawn.
        //
        // The *press*, not the release. This used to send `Tap`, and the caret
        // moved when the button came up — which is what every other control
        // wants and what no editor does. A caret that waits for the release
        // reads as a click that was dropped.
        object.handle_gesture(&press(caret_point(&paragraph, 3)), Offset::ZERO);

        let selections = reported.borrow();
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0], TextSelection::collapsed(3));
        assert!(
            selections[0].is_collapsed(),
            "a press places a caret, not a selection"
        );
    }

    #[test]
    fn the_release_does_not_move_the_caret_a_second_time() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let paragraph = object.shaped().expect("laid out").clone();
        let point = caret_point(&paragraph, 3);

        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::ZERO), Offset::ZERO);

        assert_eq!(
            reported.borrow().len(),
            1,
            "one click is one caret move, reported once"
        );
    }

    #[test]
    fn a_second_click_selects_the_word_and_a_third_the_line() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("alpha beta"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let paragraph = object.shaped().expect("laid out").clone();
        // Inside "beta", which starts at offset 6.
        let point = caret_point(&paragraph, 7);

        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::from_millis(0)), Offset::ZERO);
        assert_eq!(
            reported.borrow().last().copied(),
            Some(TextSelection::collapsed(7)),
            "one click is a caret"
        );

        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::from_millis(120)), Offset::ZERO);
        let word = reported.borrow().last().copied().expect("a selection");
        assert_eq!(
            (word.start(), word.end()),
            (6, 10),
            "two clicks select the word under them"
        );

        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::from_millis(240)), Offset::ZERO);
        let line = reported.borrow().last().copied().expect("a selection");
        assert_eq!(
            (line.start(), line.end()),
            (0, 10),
            "three clicks select the line"
        );
    }

    #[test]
    fn two_slow_clicks_are_two_clicks() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("alpha beta"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let paragraph = object.shaped().expect("laid out").clone();
        let point = caret_point(&paragraph, 7);

        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::ZERO), Offset::ZERO);
        // Well past `MULTI_TAP_TIMEOUT`. Placing the caret, thinking, and
        // clicking the same spot again must not select a word.
        object.handle_gesture(&press(point), Offset::ZERO);
        object.handle_gesture(&tap_at(point, Duration::from_secs(5)), Offset::ZERO);

        assert!(
            reported
                .borrow()
                .last()
                .is_some_and(TextSelection::is_collapsed),
            "a slow second click is a caret, not a word selection"
        );
    }

    #[test]
    fn a_second_click_somewhere_else_is_not_a_double_click() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("alpha beta"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let paragraph = object.shaped().expect("laid out").clone();

        object.handle_gesture(&press(caret_point(&paragraph, 1)), Offset::ZERO);
        object.handle_gesture(
            &tap_at(caret_point(&paragraph, 1), Duration::ZERO),
            Offset::ZERO,
        );
        object.handle_gesture(&press(caret_point(&paragraph, 8)), Offset::ZERO);
        object.handle_gesture(
            &tap_at(caret_point(&paragraph, 8), Duration::from_millis(80)),
            Offset::ZERO,
        );

        assert!(
            reported
                .borrow()
                .last()
                .is_some_and(TextSelection::is_collapsed),
            "quick, but a hand-span apart — two clicks, not a double-click"
        );
    }

    #[test]
    fn page_down_moves_a_page_and_stops_at_the_end() {
        // Ten lines, a page of three.
        let text = (0..10)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let field = RenderEditableText::new(value(&text), style()).page_lines(3);
        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);

        let mut current = TextEditingValue::new(&text);
        current.selection = TextSelection::collapsed(0);

        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageDown { extend: false }
        ));
        let after_one = line_of(&current);
        assert_eq!(after_one, 3, "a page is three lines here");

        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageDown { extend: false }
        ));
        assert_eq!(line_of(&current), 6);

        // 6 + 3 is exactly the last line, so this one is a full page.
        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageDown { extend: false }
        ));
        assert_eq!(line_of(&current), 9, "the bottom");

        // And now there is nowhere left, which is not an edit.
        assert!(
            !object_apply(
                object,
                &mut current,
                &TextIntent::MovePageDown { extend: false }
            ),
            "a movement with nowhere to go must not ask for a frame"
        );
    }

    #[test]
    fn page_up_is_the_same_journey_backwards() {
        let text = (0..10)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let field = RenderEditableText::new(value(&text), style()).page_lines(4);
        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);

        let mut current = TextEditingValue::new(&text);
        current.selection = TextSelection::collapsed(text.len());
        assert_eq!(line_of(&current), 9);

        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageUp { extend: false }
        ));
        assert_eq!(line_of(&current), 5);
        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageUp { extend: false }
        ));
        assert_eq!(line_of(&current), 1);
        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageUp { extend: false }
        ));
        assert_eq!(line_of(&current), 0, "stops at the top");
        assert!(!object_apply(
            object,
            &mut current,
            &TextIntent::MovePageUp { extend: false }
        ));
    }

    #[test]
    fn a_page_of_zero_lines_still_moves_one() {
        // Zero would make both keys silently dead, which is stranger than
        // moving by one.
        let field = RenderEditableText::new(value("a\nb\nc"), style()).page_lines(0);
        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let mut current = TextEditingValue::new("a\nb\nc");
        current.selection = TextSelection::collapsed(0);
        assert!(object_apply(
            object,
            &mut current,
            &TextIntent::MovePageDown { extend: false }
        ));
        assert_eq!(line_of(&current), 1);
    }

    #[test]
    fn a_text_field_asks_for_an_i_beam() {
        let field = RenderEditableText::new(value("hello"), style());
        assert_eq!(
            RenderObject::cursor(&field, Offset::ZERO),
            Some(vieww_foundation::Cursor::Text),
            "an I-beam is what `you can type here` looks like on every desktop"
        );
    }

    /// The line the caret is on, counted in `\n`s.
    fn line_of(value: &TextEditingValue) -> usize {
        value.text[..value.selection.extent].matches('\n').count()
    }

    /// Run a layout-dependent intent against a mounted object.
    fn object_apply(
        object: &RenderEditableText,
        value: &mut TextEditingValue,
        intent: &TextIntent,
    ) -> bool {
        // The object holds the *original* value; these tests walk a caret
        // through the same text, so the paragraph is the one that matters and
        // the value is carried by the caller.
        let mut scratch = object.value.clone();
        scratch.selection = value.selection;
        let moved = object.apply_with_layout(&mut scratch, intent);
        value.selection = scratch.selection;
        moved
    }

    /// A press carrying modifiers, for the modified-click tests.
    fn press_with(local: Offset, modifiers: vieww_foundation::Modifiers) -> Recognized {
        let mut details = TapDetails::at(local);
        details.modifiers = modifiers;
        Recognized::TapDown(details)
    }

    /// ⇧-click extends from the anchor, exactly as dragging there would.
    /// Before `TapDetails` carried modifiers this was unreachable, and so was
    /// range-select in every list, tree and table built on vieww.
    #[test]
    fn a_shift_click_extends_the_selection_rather_than_replacing_it() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let mut value = value("hello there");
        value.selection = TextSelection::collapsed(2);
        let field = RenderEditableText::new(value, style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();

        object.handle_gesture(
            &press_with(caret_point(&shaped, 7), Modifiers::SHIFT),
            Offset::ZERO,
        );

        let last = *reported.borrow().last().expect("nothing was reported");
        assert_eq!((last.base, last.extent), (2, 7), "the anchor did not hold");
    }

    /// ⌘-click reports on the *other* channel, because adding a caret and
    /// moving one are opposite intentions. `TextField::with_added_caret` has
    /// existed unused since multi-caret shipped; this is what reaches it.
    #[test]
    fn a_modified_click_asks_for_a_caret_rather_than_moving_one() {
        let moved: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let added: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let move_sink = Rc::clone(&moved);
        let add_sink = Rc::clone(&added);
        let field = RenderEditableText::new(value("hello there"), style())
            .on_selection_changed(Rc::new(move |_| *move_sink.borrow_mut() += 1))
            .on_add_caret(Rc::new(move |selection| {
                add_sink.borrow_mut().push(selection)
            }));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();

        object.handle_gesture(
            &press_with(caret_point(&shaped, 6), Modifiers::META),
            Offset::ZERO,
        );

        assert_eq!(*moved.borrow(), 0, "a ⌘-click moved the caret");
        assert_eq!(added.borrow().len(), 1);
        assert_eq!(added.borrow()[0].extent, 6);
    }

    /// And with nothing listening it is a plain click, so a search box does not
    /// sprout a second caret because somebody was holding Control.
    #[test]
    fn a_modified_click_with_no_handler_is_a_plain_click() {
        let moved: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let sink = Rc::clone(&moved);
        let field = RenderEditableText::new(value("hello there"), style())
            .on_selection_changed(Rc::new(move |_| *sink.borrow_mut() += 1));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();

        object.handle_gesture(
            &press_with(caret_point(&shaped, 6), Modifiers::CONTROL),
            Offset::ZERO,
        );
        assert_eq!(*moved.borrow(), 1);
    }

    /// **A selection gesture is a read.**
    ///
    /// Every test beside this one asserts what a drag *selects*, and not one of
    /// them asserted what it leaves behind — which is how a click-and-drag that
    /// destroyed 247 characters of a buffer got past 2,114 green tests. The
    /// gestures here are the four that can place or extend a selection; none of
    /// them may change a byte.
    #[test]
    fn no_selection_gesture_changes_the_text() {
        let changed: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let sink = Rc::clone(&changed);
        let field = RenderEditableText::new(value("hello there"), style())
            .on_selection_changed(Rc::new(|_| {}))
            // A value handler is what makes the field a keyboard target, so it
            // has to be present for this to be the real configuration — and
            // nothing a pointer does may reach it.
            .on_changed(Rc::new(move |_| *sink.borrow_mut() += 1));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();
        let at = |offset: usize| caret_point(&shaped, offset);
        let before = object.value.text.clone();

        object.handle_gesture(&Recognized::TapDown(TapDetails::at(at(2))), Offset::ZERO);
        object.handle_gesture(&tap_at(at(2), Duration::ZERO), Offset::ZERO);
        object.handle_gesture(&Recognized::DragStart(drag(at(2))), Offset::ZERO);
        for offset in [4, 7, 9, 11, 6, 1] {
            object.handle_gesture(&Recognized::DragUpdate(drag(at(offset))), Offset::ZERO);
        }
        object.handle_gesture(&Recognized::DragEnd(drag(at(1))), Offset::ZERO);

        assert_eq!(
            object.value.text, before,
            "a selection gesture edited the text"
        );
        assert_eq!(
            *changed.borrow(),
            0,
            "a pointer reached the value handler, which only a key may do"
        );
    }

    /// With `on_selection` set on the widget, the pointer path carries a
    /// selection and nothing else — so there is no text for it to get wrong,
    /// however far behind the widget's own copy has fallen.
    #[test]
    fn a_selection_handler_receives_no_text_to_be_wrong_about() {
        let seen: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let field = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();
        let at = |offset: usize| caret_point(&shaped, offset);

        object.handle_gesture(&Recognized::DragStart(drag(at(0))), Offset::ZERO);
        object.handle_gesture(&Recognized::DragUpdate(drag(at(5))), Offset::ZERO);

        let seen = seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[1].base, seen[1].extent), (0, 5));
    }

    #[test]
    fn dragging_selects_from_where_the_drag_began() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();
        let at = |offset: usize| caret_point(&shaped, offset);

        object.handle_gesture(&Recognized::DragStart(drag(at(1))), Offset::ZERO);
        object.handle_gesture(&Recognized::DragUpdate(drag(at(3))), Offset::ZERO);
        object.handle_gesture(&Recognized::DragUpdate(drag(at(4))), Offset::ZERO);

        let selections = reported.borrow();
        assert_eq!(
            (selections[1].base, selections[1].extent),
            (1, 3),
            "the anchor is where the finger went down"
        );
        assert_eq!(
            (selections[2].base, selections[2].extent),
            (1, 4),
            "and it does not follow the finger"
        );
    }

    #[test]
    fn a_drag_that_reverses_past_its_anchor_keeps_the_anchor() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let field = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::new(move |selection| sink.borrow_mut().push(selection)));

        let (tree, id, _) = mount(field);
        let object = editable(&tree, id);
        let shaped = object.shaped().expect("laid out").clone();
        let at = |offset: usize| caret_point(&shaped, offset);

        object.handle_gesture(&Recognized::DragStart(drag(at(3))), Offset::ZERO);
        object.handle_gesture(&Recognized::DragUpdate(drag(at(1))), Offset::ZERO);

        let selections = reported.borrow();
        let last = selections.last().expect("an update");
        assert_eq!((last.base, last.extent), (3, 1));
        assert!(last.is_reversed());
        assert_eq!(last.range(), TextRange::new(1, 3));
    }

    #[test]
    fn a_field_with_no_handler_registers_no_recognisers() {
        let editable = RenderEditableText::new(value("hello"), style());
        assert!(
            editable.gesture_recognizers().is_empty(),
            "a read-only field must not take pointers from a scrollable above it"
        );
    }

    // ------------------------------------------------------------- the rebuild

    #[test]
    fn blinking_the_caret_does_not_reshape_the_paragraph() {
        let editable = RenderEditableText::new(value("hello"), style());
        let blinked = RenderEditableText::new(value("hello"), style()).show_cursor(false);

        assert!(
            !editable.layout_differs(&blinked),
            "a caret blinking at 2 Hz must not shape the text twice a second"
        );
    }

    #[test]
    fn a_blink_does_not_erase_the_text() {
        let (mut tree, id, _) = mount(RenderEditableText::new(value("hello"), style()));

        // The off phase of the blink: a rebuild that changes only `show_cursor`.
        tree.replace_object(
            id,
            Box::new(RenderEditableText::new(value("hello"), style()).show_cursor(false)),
        );
        let scene = scene_of(&tree);

        assert_eq!(
            scene.glyph_runs().len(),
            1,
            "the shaped paragraph has to come across with the replacement"
        );
        assert!(scene.fills().is_empty(), "and the caret is genuinely off");
    }

    #[test]
    fn typing_a_character_does_reshape() {
        let editable = RenderEditableText::new(value("hello"), style());
        let typed = RenderEditableText::new(value("hello!"), style());

        assert!(
            editable.layout_differs(&typed),
            "different text is different geometry"
        );
    }

    #[test]
    fn changing_only_the_selection_does_not_reshape() {
        let editable = RenderEditableText::new(value("hello"), style());
        let mut selected = RenderEditableText::new(value("hello"), style());
        selected.value.selection = TextSelection::new(0, 5);

        assert!(!editable.layout_differs(&selected));
    }

    #[test]
    fn a_selection_drag_survives_the_rebuild_between_its_updates() {
        let reported: Rc<RefCell<Vec<TextSelection>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&reported);
        let handler: SelectionChanged = Rc::new(move |selection| sink.borrow_mut().push(selection));

        let (mut tree, id, _) = mount(
            RenderEditableText::new(value("hello"), style())
                .on_selection_changed(Rc::clone(&handler)),
        );
        let shaped = editable(&tree, id).shaped().expect("laid out").clone();
        let at = |offset: usize| caret_point(&shaped, offset);

        editable(&tree, id).handle_gesture(&Recognized::DragStart(drag(at(1))), Offset::ZERO);

        // What the frame after the drag start does: the selection changed, so a
        // new object is built and swapped in.
        let mut updated = RenderEditableText::new(value("hello"), style())
            .on_selection_changed(Rc::clone(&handler));
        updated.value.selection = TextSelection::collapsed(1);
        tree.replace_object(id, Box::new(updated));

        editable(&tree, id).handle_gesture(&Recognized::DragUpdate(drag(at(4))), Offset::ZERO);

        let selections = reported.borrow();
        let last = selections.last().expect("an update");
        assert_eq!(
            (last.base, last.extent),
            (1, 4),
            "the anchor has to outlive the rebuild or every frame collapses the selection"
        );
    }

    // -------------------------------------------------------------- intrinsics

    /// Ask a field a question **without laying it out first**.
    ///
    /// Deliberately never laid out: the callers that need intrinsics — an
    /// `Accordion` measuring content that is still closed, an `IntrinsicWidth`
    /// deciding what to hand down — ask before any layout has happened, so a
    /// field that could only answer from its shaping cache would answer `None`
    /// exactly when it was needed.
    fn intrinsic(object: RenderEditableText, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.intrinsic(id, query)
    }

    /// The same question put to a plain label, for comparing against.
    fn text_intrinsic(text: &str, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(crate::RenderText::new(text, style())));
        tree.intrinsic(id, query)
    }

    #[test]
    fn a_field_measures_its_text_plus_room_for_the_caret() {
        let field = RenderEditableText::new(value("hello"), style());
        let width = intrinsic(field, crate::IntrinsicQuery::max_width()).expect("measurable");
        let label =
            text_intrinsic("hello", crate::IntrinsicQuery::max_width()).expect("measurable");

        assert!(
            (width - (label + DEFAULT_CURSOR_WIDTH)).abs() < 0.01,
            "a box measured without the caret clips it at the end of the line: {width} vs {label}"
        );
    }

    #[test]
    fn an_intrinsic_agrees_with_the_size_an_unbounded_layout_arrives_at() {
        // The property that makes the answer usable: an `IntrinsicWidth` above a
        // field must not report a width the field then refuses to take.
        let asked = intrinsic(
            RenderEditableText::new(value("hello"), style()),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");
        let (.., laid) = mount(RenderEditableText::new(value("hello"), style()));

        assert!((asked - laid.width).abs() < 0.01, "{asked} vs {laid}");
    }

    #[test]
    fn the_minimum_width_is_the_longest_word_rather_than_zero() {
        // Shaped against a width of zero, and read back with `widest_line`. The
        // trap `RenderText` documents is `size().width`, which is clamped to the
        // width it was shaped against — so this would come back as a caret's
        // width and a field in an `IntrinsicWidth` would collapse.
        let min = intrinsic(
            RenderEditableText::new(value("hello world"), style()),
            crate::IntrinsicQuery::min_width(),
        )
        .expect("measurable");
        let max = intrinsic(
            RenderEditableText::new(value("hello world"), style()),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");

        assert!(min > DEFAULT_CURSOR_WIDTH, "collapsed to a caret: {min}");
        assert!(
            min < max,
            "two words wrap, so the minimum is the longer one: {min} vs {max}"
        );
    }

    #[test]
    fn the_height_of_a_wrapped_field_needs_the_width_to_be_stated() {
        let one_line = intrinsic(
            RenderEditableText::new(value("abcdefghijklmnop"), style()),
            crate::IntrinsicQuery::max_height(),
        )
        .expect("measurable");
        let wrapped = intrinsic(
            RenderEditableText::new(value("abcdefghijklmnop"), style()),
            crate::IntrinsicQuery::max_height().across(60.0),
        )
        .expect("measurable");

        assert!(
            wrapped > one_line,
            "asking without a width gets the single-line answer: {wrapped} vs {one_line}"
        );
    }

    #[test]
    fn a_paragraph_has_no_slack_so_its_two_heights_agree() {
        let query = crate::IntrinsicQuery::max_height().across(60.0);
        let max = intrinsic(RenderEditableText::new(value("abcdefgh"), style()), query);
        let min = intrinsic(
            RenderEditableText::new(value("abcdefgh"), style()),
            crate::IntrinsicQuery::min_height().across(60.0),
        );

        assert_eq!(min, max, "once the width is fixed there is one height");
    }

    #[test]
    fn an_obscured_field_measures_its_mask_and_not_its_text() {
        // The same reasoning the mask itself rests on: what is measured has to
        // be what is shown, or a parent sized to the real glyphs would leak how
        // wide the password's letters are.
        let plain = intrinsic(
            RenderEditableText::new(value("iiiiiii"), style()),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");
        let masked = intrinsic(
            RenderEditableText::new(value("iiiiiii"), style()).obscure(Obscured::With('•')),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");

        assert!(
            masked > plain,
            "seven bullets are wider than seven narrow letters: {masked} vs {plain}"
        );
    }

    #[test]
    fn an_empty_field_with_a_placeholder_measures_the_placeholder() {
        // It is what is on screen, so it is what the field takes room for. A
        // field that measured its empty value would be a caret wide and the hint
        // would spill out of it.
        let bare = intrinsic(
            RenderEditableText::new(value(""), style()),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");
        let hinted = intrinsic(
            RenderEditableText::new(value(""), style()).placeholder("Email", Color::BLACK),
            crate::IntrinsicQuery::max_width(),
        )
        .expect("measurable");

        assert!((bare - DEFAULT_CURSOR_WIDTH).abs() < 0.01, "{bare}");
        assert!(hinted > bare, "{hinted} vs {bare}");
    }

    #[test]
    fn the_caret_and_the_selection_do_not_change_what_is_measured() {
        // The purity the contract asks for, pinned. The blink is a rebuild twice
        // a second and the selection changes on every arrow key; an intrinsic
        // that moved with either would invalidate a parent's measurement for
        // reasons that cannot move a glyph.
        let plain = intrinsic(
            RenderEditableText::new(value("hello"), style()),
            crate::IntrinsicQuery::max_width(),
        );

        let mut selected = RenderEditableText::new(value("hello"), style()).show_cursor(false);
        selected.value.selection = TextSelection::new(1, 4);

        assert_eq!(
            intrinsic(selected, crate::IntrinsicQuery::max_width()),
            plain
        );
    }

    #[test]
    fn a_tap_at_the_end_of_a_wrapped_line_reports_upstream() {
        let mut tree = RenderTree::new();
        let id = tree.insert(
            None,
            Box::new(RenderEditableText::new(value("abcdefghijklmnop"), style())),
        );
        tree.layout(id, Constraints::loose(Size::new(60.0, f32::INFINITY)));

        let shaped = editable(&tree, id).shaped().expect("laid out");
        assert!(shaped.line_count() > 1);

        let first_line = shaped.line_rect(0).expect("a first line");
        let position = shaped.hit_test(Offset::new(
            1000.0,
            (first_line.top + first_line.bottom) / 2.0,
        ));
        assert_eq!(
            position.affinity,
            Affinity::Upstream,
            "the caret belongs to the line that was tapped"
        );
    }
    // ===================== range decorations ==============================

    /// The filled decoration shapes in a scene.
    ///
    /// The squiggle and box used to be recorded as strokes and are now the
    /// pre-expanded fills of those strokes' outlines — see `paint` — so this
    /// collects `FillPath` commands, which in these tests are exactly the
    /// decorations.
    fn strokes(scene: &vieww_paint::Scene) -> Vec<(Path, Paint)> {
        scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                vieww_paint::Command::FillPath { path, paint, .. } => Some((path.clone(), *paint)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_squiggle_is_drawn_under_the_range_it_names_and_nowhere_else() {
        let object =
            RenderEditableText::new(value("let x = 1;"), style()).decorations(Rc::new(vec![
                TextDecoration::squiggle(TextRange::new(4, 5), Color::RED),
            ]));
        let (tree, id, _) = mount(object);
        let scene = scene_of(&tree);

        let waves = strokes(&scene);
        assert_eq!(waves.len(), 1, "one range, one wave");
        assert_eq!(waves[0].1, Paint::solid(Color::RED));

        // Under the `x`, not under the whole line. The paragraph is the
        // authority on where that is, so the assertion is against it rather
        // than against a number typed here.
        let paragraph = editable(&tree, id).shaped().expect("laid out");
        let expected = paragraph.selection_rects(TextRange::new(4, 5));
        assert_eq!(expected.len(), 1);
        // The wave's *ink* now extends by half its thickness on either side of
        // the path — the outline a stroke would have covered, drawn as the
        // fill that replaced it — so the tolerance is the thickness, not a
        // rounding allowance. What the assertion still buys is the property
        // that matters: the wave tracks the range, not the line.
        let thickness = 1.3;
        let bounds = waves[0].0.bounds();
        assert!(
            bounds.left >= expected[0].left - thickness
                && bounds.right <= expected[0].right + thickness,
            "the wave spans the range and not the line: {bounds:?} against {:?}",
            expected[0]
        );
    }

    #[test]
    fn a_box_decoration_outlines_the_range() {
        let object =
            RenderEditableText::new(value("fn main() {}"), style()).decorations(Rc::new(vec![
                TextDecoration::boxed(TextRange::new(7, 8), Color::BLUE),
            ]));
        let (tree, _, _) = mount(object);
        assert_eq!(
            strokes(&scene_of(&tree)).len(),
            1,
            "a matched bracket is one filled outline"
        );
    }

    #[test]
    fn an_underline_and_a_strike_are_fills_rather_than_strokes() {
        // Deliberate: a straight rule is a rectangle, and asking a backend to
        // stroke one is asking it to do more work for the same pixels.
        let object =
            RenderEditableText::new(value("deprecated"), style()).decorations(Rc::new(vec![
                TextDecoration::squiggle(TextRange::new(0, 4), Color::RED)
                    .shape(TextDecorationShape::Underline),
                TextDecoration::squiggle(TextRange::new(4, 10), Color::RED)
                    .shape(TextDecorationShape::Strike),
            ]));
        let (tree, _, _) = mount(object);
        let scene = scene_of(&tree);
        assert!(
            strokes(&scene).is_empty(),
            "neither shape needs a stroked path"
        );
        assert!(
            scene
                .fills()
                .iter()
                .any(|(_, paint)| *paint == Paint::solid(Color::RED)),
            "and both are drawn"
        );
    }

    #[test]
    fn a_transparent_or_empty_decoration_draws_nothing() {
        let object =
            RenderEditableText::new(value("let x = 1;"), style()).decorations(Rc::new(vec![
                TextDecoration::squiggle(TextRange::new(4, 5), Color::TRANSPARENT),
                TextDecoration::squiggle(TextRange::new(4, 4), Color::RED),
            ]));
        let (tree, _, _) = mount(object);
        assert!(
            strokes(&scene_of(&tree)).is_empty(),
            "an invisible mark and a zero-width one are both nothing to draw"
        );
    }

    #[test]
    fn a_guide_is_a_thin_rule_at_the_range_leading_edge_the_height_of_the_line() {
        // The one shape whose geometry is a position rather than an extent.
        // A guide over a *wide* range must still be a hairline: if it filled
        // the range it would be a highlight, and every indent guide in an
        // editor would be a solid block over the code.
        let object =
            RenderEditableText::new(value("        x = 1;"), style()).decorations(Rc::new(vec![
                TextDecoration::guide(TextRange::new(4, 8), Color::BLUE).thickness(1.0),
            ]));
        let (tree, id, _) = mount(object);
        let scene = scene_of(&tree);

        let paragraph = editable(&tree, id).shaped().expect("laid out");
        let expected = paragraph.selection_rects(TextRange::new(4, 8));
        assert_eq!(expected.len(), 1);
        let expected = expected[0];

        let rule = scene
            .fills()
            .iter()
            .find(|(_, paint)| *paint == Paint::solid(Color::BLUE))
            .map(|(rect, _)| *rect)
            .expect("the guide is drawn");

        assert!(
            (rule.right - rule.left - 1.0).abs() < 0.01,
            "a guide is its thickness wide, not the range's width: {rule:?}"
        );
        assert!(
            (rule.left - expected.left).abs() < 0.01,
            "and it sits on the leading edge: {rule:?} against {expected:?}"
        );
        assert!(
            (rule.bottom - rule.top - (expected.bottom - expected.top)).abs() < 0.01,
            "spanning the line box it is on: {rule:?} against {expected:?}"
        );
    }

    #[test]
    fn a_guide_takes_the_leading_edge_of_a_right_to_left_paragraph() {
        // "Leading" is not "left". A paragraph the shaper laid out
        // right-to-left has its first column on the right, and a guide that
        // read `rect.left` would draw every indent rule on the wrong side of
        // the character it belongs to.
        let object = RenderEditableText::new(
            value("\u{5e9}\u{5dc}\u{5d5}\u{5dd} \u{5e2}\u{5d5}\u{5dc}\u{5dd}"),
            style(),
        )
        .direction(Some(TextDirection::Rtl))
        .decorations(Rc::new(vec![TextDecoration::guide(
            TextRange::new(0, 2),
            Color::BLUE,
        )
        .thickness(1.0)]));
        let (tree, id, _) = mount(object);
        let scene = scene_of(&tree);
        let paragraph = editable(&tree, id).shaped().expect("laid out");
        let expected = paragraph.selection_rects(TextRange::new(0, 2));
        assert_eq!(expected.len(), 1);
        let expected = expected[0];

        let rule = scene
            .fills()
            .iter()
            .find(|(_, paint)| *paint == Paint::solid(Color::BLUE))
            .map(|(rect, _)| *rect)
            .expect("the guide is drawn");
        assert!(
            (rule.right - expected.right).abs() < 0.01,
            "the leading edge of an RTL range is its right: {rule:?} against {expected:?}"
        );
    }

    // ===================== the layout probe ===============================

    #[test]
    fn a_probe_is_told_one_rect_per_visual_line_and_where_the_caret_is() {
        let probe = TextLayoutProbe::new();
        let object = RenderEditableText::new(value("one\ntwo\nthree"), style())
            .wrap(false)
            .probe(probe.clone());
        let (tree, id, _) = mount(object);

        let report = probe.snapshot();
        let paragraph = editable(&tree, id).shaped().expect("laid out");
        assert_eq!(report.lines.len(), paragraph.line_count());
        assert_eq!(report.lines.len(), 3, "three source lines, wrapping off");
        assert_eq!(report.line(0), paragraph.line_rect(0));
        assert_eq!(report.line(2), paragraph.line_rect(2));
        assert_eq!(report.font_size, style().size);
        assert!(!report.is_empty());
        // The caret's rect, which is the other half of what a popup needs.
        assert_eq!(
            Some(report.cursor),
            editable(&tree, id).cursor_rect(),
            "the report's caret is the field's caret"
        );
    }

    #[test]
    fn attaching_a_probe_does_not_cost_a_re_shape() {
        // The report is read *out of* the paragraph, so publishing it cannot
        // move a glyph — and a field that re-shaped because somebody wanted to
        // know where its lines were would be paying for the measurement twice.
        let bare = RenderEditableText::new(value("let x = 1;"), style());
        let probed =
            RenderEditableText::new(value("let x = 1;"), style()).probe(TextLayoutProbe::new());
        assert!(bare.layout_eq(&probed));
    }

    #[test]
    fn every_caret_is_drawn_and_they_are_all_at_different_places() {
        let mut value = value("one\ntwo\nthree");
        value.selection = TextSelection::collapsed(0);
        value.add_caret(TextSelection::collapsed(4));
        value.add_caret(TextSelection::collapsed(8));

        let object = RenderEditableText::new(value, style())
            .wrap(false)
            .cursor(Color::RED, 2.0);
        let (tree, id, _) = mount(object);

        let rects = editable(&tree, id).caret_rects();
        assert_eq!(rects.len(), 3, "one per caret");
        // Three different lines, so three different tops.
        let mut tops: Vec<i32> = rects.iter().map(|r| r.top.round() as i32).collect();
        tops.dedup();
        assert_eq!(tops.len(), 3, "{rects:?}");

        let drawn = scene_of(&tree)
            .fills()
            .iter()
            .filter(|(_, paint)| *paint == Paint::solid(Color::RED))
            .count();
        assert_eq!(drawn, 3, "and all three reach the canvas");
    }

    #[test]
    fn every_selection_is_highlighted_not_only_the_primary() {
        // A field with three selections that highlighted one would be showing
        // a different edit from the one about to happen.
        let mut value = value("aaaa bbbb cccc");
        value.selection = TextSelection::new(0, 4);
        value.add_caret(TextSelection::new(5, 9));
        value.add_caret(TextSelection::new(10, 14));

        let object = RenderEditableText::new(value, style())
            .wrap(false)
            .selection_color(Color::GREEN)
            // Off, so the only fills of interest are the highlights.
            .cursor(Color::TRANSPARENT, 2.0);
        let (tree, _, _) = mount(object);
        let highlights = scene_of(&tree)
            .fills()
            .iter()
            .filter(|(_, paint)| *paint == Paint::solid(Color::GREEN))
            .count();
        assert_eq!(highlights, 3);
    }

    #[test]
    fn moving_up_a_line_moves_every_caret() {
        // The intents the editing model refuses because they need geometry.
        // They are refused for every caret alike, so this is where multiple
        // carets have to be handled for them.
        let mut value = value("one\ntwo\nthree");
        value.selection = TextSelection::collapsed(5);
        value.add_caret(TextSelection::collapsed(9));

        let object = RenderEditableText::new(value.clone(), style()).wrap(false);
        let (tree, id, _) = mount(object);
        let object = editable(&tree, id);

        let mut moved = value;
        assert!(object.apply_with_layout(&mut moved, &TextIntent::MoveUp { extend: false }));
        let lines: Vec<usize> = moved
            .sorted_carets()
            .iter()
            .map(|caret| moved.text[..caret.start()].matches('\n').count())
            .collect();
        assert_eq!(lines, vec![0, 1], "both went up one line");
    }

    #[test]
    fn a_caret_that_cannot_move_stays_where_it_is_rather_than_disappearing() {
        // One caret on the first line and one on the third, both pressing up.
        // The first cannot go anywhere; dropping it would lose a caret the
        // user placed. The third line rather than the second so the two do not
        // land on each other, which would legitimately merge them.
        let mut value = value("one\ntwo\nthree");
        value.selection = TextSelection::collapsed(1);
        value.add_caret(TextSelection::collapsed(9));

        let object = RenderEditableText::new(value.clone(), style()).wrap(false);
        let (tree, id, _) = mount(object);

        let mut moved = value;
        assert!(editable(&tree, id)
            .apply_with_layout(&mut moved, &TextIntent::MoveUp { extend: false }));
        assert_eq!(moved.caret_count(), 2, "{:?}", moved.sorted_carets());
    }

    #[test]
    fn adding_a_caret_does_not_cost_a_re_shape() {
        // Carets are paint-only, exactly as the selection and the blink are.
        let bare = RenderEditableText::new(value("let x = 1;"), style());
        let mut many = value("let x = 1;");
        many.add_caret(TextSelection::collapsed(0));
        many.add_caret(TextSelection::collapsed(4));
        let multi = RenderEditableText::new(many, style());
        assert!(bare.layout_eq(&multi));
    }

    #[test]
    fn decorations_do_not_re_shape_the_paragraph() {
        // The whole reason they are paint-only. A field that re-squiggles on
        // every keystroke must not re-shape on every keystroke, and the check
        // for that is `layout_eq` — the same gate the caret's blink passes.
        let bare = RenderEditableText::new(value("let x = 1;"), style());
        let marked =
            RenderEditableText::new(value("let x = 1;"), style()).decorations(Rc::new(vec![
                TextDecoration::squiggle(TextRange::new(4, 5), Color::RED),
            ]));
        assert!(
            bare.layout_eq(&marked),
            "adding a decoration changed the geometry, so every mark costs a re-shape"
        );
    }

    #[test]
    fn a_squiggle_follows_the_mask_of_an_obscured_field() {
        // The bug this is here to prevent is the one `cursor_rect` already
        // documents: a `TextPosition` is a byte offset, one masked character is
        // three bytes, and a decoration handed real offsets straight to the
        // paragraph lands a third of the way through the wrong bullet.
        let object = RenderEditableText::new(value("hunter2"), style())
            .obscure(Obscured::With('\u{2022}'))
            .decorations(Rc::new(vec![TextDecoration::squiggle(
                TextRange::new(0, 7),
                Color::RED,
            )]));
        let (tree, id, _) = mount(object);
        let scene = scene_of(&tree);
        let waves = strokes(&scene);
        assert_eq!(waves.len(), 1);

        // Seven characters of "hunter2" become seven bullets of three bytes
        // each. A decoration handed the real range straight over would cover
        // the first seven bytes — two and a third bullets — so the check is
        // that the wave reaches the mask's own right edge and no further.
        let paragraph = editable(&tree, id).shaped().expect("laid out");
        let mask = paragraph.line_rect(0).expect("one line");
        let wave = waves[0].0.bounds();
        assert!(
            wave.right > mask.right * 0.9,
            "the wave covered {:.1} of the mask's {:.1} — it was positioned in real \
             byte offsets rather than display ones",
            wave.right,
            mask.right
        );
        assert!(
            wave.right <= mask.right + 1.0,
            "and it did not run off the end of it"
        );
    }
}

#[cfg(test)]
mod obscure_and_placeholder_tests {
    use vieww_foundation::{Constraints, Obscured, Offset, Size, TextEditingValue, TextStyle};

    use super::RenderEditableText;
    use crate::{RenderId, RenderTree};

    fn mount(object: RenderEditableText) -> (RenderTree, RenderId) {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.layout(id, Constraints::loose(Size::new(400.0, f32::INFINITY)));
        (tree, id)
    }

    fn field(text: &str) -> RenderEditableText {
        RenderEditableText::new(TextEditingValue::new(text), TextStyle::new(16.0))
    }

    /// The mounted object, for the cases that ask it a question directly.
    fn editable(tree: &RenderTree, id: RenderId) -> &RenderEditableText {
        let object: &dyn std::any::Any = tree.object(id).expect("mounted");
        object
            .downcast_ref::<RenderEditableText>()
            .expect("mounted as its own type")
    }

    /// The glyph ids actually shaped, in order.
    ///
    /// Ids rather than characters, because a `Glyph` is an index into a font and
    /// carries no text — which is the honest thing to assert against anyway:
    /// what reaches the screen is glyphs, and "every glyph is the same one" is a
    /// stronger statement about a mask than any string comparison.
    fn shaped_ids(tree: &RenderTree, id: RenderId) -> Vec<u16> {
        let object: &dyn std::any::Any = tree.object(id).expect("mounted");
        let editable = object
            .downcast_ref::<RenderEditableText>()
            .expect("mounted as its own type");
        editable
            .shaped()
            .expect("laid out")
            .runs()
            .iter()
            .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.id))
            .collect()
    }

    #[test]
    fn an_obscured_field_shapes_bullets_rather_than_drawing_over_the_text() {
        // **The gap this closes, and the reason it is not a paint-time effect.**
        // The paragraph itself has to be the mask: everything that resolves a
        // caret, a selection rect or a tap goes through it, so a field that
        // shaped its real text and painted bullets on top would put the caret
        // wherever the real glyph's advance happened to fall.
        let (tree, id) = mount(field("hunter2").obscure(Obscured::With('•')));
        let masked = shaped_ids(&tree, id);
        assert_eq!(masked.len(), 7, "one glyph per character held");
        assert!(
            masked.windows(2).all(|pair| pair[0] == pair[1]),
            "every glyph is the same one, so none of them is a letter: {masked:?}"
        );

        let (plain_tree, plain_id) = mount(field("hunter2"));
        let plain = shaped_ids(&plain_tree, plain_id);
        assert_ne!(
            masked, plain,
            "the masked field shaped the same glyphs as the plain one"
        );
    }

    #[test]
    fn a_plain_field_is_untouched() {
        let (tree, id) = mount(field("hunter2"));
        let ids = shaped_ids(&tree, id);
        assert_eq!(ids.len(), 7);
        assert!(
            !ids.windows(2).all(|pair| pair[0] == pair[1]),
            "an unobscured field of seven different letters shaped seven identical glyphs"
        );
    }

    #[test]
    fn the_caret_of_an_obscured_field_sits_after_the_last_bullet() {
        // Found on a screen rather than in a test: typing three characters into
        // a masked field drew three bullets with the caret after the *first*.
        // `TextPosition` is a byte offset, one bullet is three bytes, so a
        // cursor at byte three — the end of "abc" — landed one bullet in.
        //
        // Asserted against the plain field's caret at the same *character*
        // count rather than against a number: what the mask is worth is that
        // nobody can tell how wide the real glyphs were, so the only honest
        // reference is the mask's own advance.
        let mut object = field("abc").obscure(Obscured::With('•'));
        object.value.selection = vieww_foundation::TextSelection::collapsed(3);
        let (tree, id) = mount(object);
        let editable = editable(&tree, id);

        let caret = editable.cursor_rect().expect("laid out");
        let width = editable.shaped().expect("laid out").size().width;

        assert!(
            (caret.left - width).abs() < 1.0,
            "three characters means the caret is past all three bullets, at {width}, not {}",
            caret.left
        );
    }

    #[test]
    fn the_caret_of_an_empty_obscured_field_is_at_the_start() {
        // The other end of the same conversion, and the one that would hide a
        // sign-flip: zero converts to zero, so nothing moves.
        let object = field("").obscure(Obscured::With('•'));
        let (tree, id) = mount(object);

        let caret = editable(&tree, id).cursor_rect().expect("laid out");
        assert!(
            caret.left < 1.0,
            "an empty field's caret is at {}",
            caret.left
        );
    }

    #[test]
    fn an_obscured_field_keeps_its_real_value() {
        // The mask is about who can see it. The application still needs the
        // password, so `value` is deliberately not masked.
        let object = field("hunter2").obscure(Obscured::With('•'));
        assert_eq!(object.value.text, "hunter2");
    }

    #[test]
    fn an_obscured_field_does_not_put_its_contents_in_the_semantics_tree() {
        // A screen reader is a second way to read the pixels, and the pixels no
        // longer carry the password. Reporting the length matches exactly what
        // the mask already shows, so somebody who cannot see the screen learns
        // the same amount and no more.
        use crate::RenderObject;
        let semantics = field("hunter2")
            .obscure(Obscured::With('•'))
            .semantics()
            .expect("a text field is announced");
        let value = semantics.value.clone().unwrap_or_default();
        assert!(
            !value.contains("hunter2"),
            "the password reached the accessibility tree: {value:?}"
        );
        assert_eq!(value, "7 characters");
    }

    #[test]
    fn a_plain_field_still_announces_what_it_holds() {
        use crate::RenderObject;
        let semantics = field("hello").semantics().expect("announced");
        assert_eq!(semantics.value.as_deref(), Some("hello"));
    }

    #[test]
    fn a_placeholder_is_shown_only_while_the_field_is_empty() {
        let (tree, id) = mount(field("").placeholder("Email", vieww_foundation::Color::BLACK));
        let hint = shaped_ids(&tree, id);
        assert_eq!(hint.len(), 5, "the hint is shaped while the field is empty");

        let (empty_tree, empty_id) = mount(field(""));
        assert!(
            shaped_ids(&empty_tree, empty_id).is_empty(),
            "a field with no placeholder shapes nothing when empty"
        );

        let (typed_tree, typed_id) =
            mount(field("ab").placeholder("Email", vieww_foundation::Color::BLACK));
        assert_eq!(
            shaped_ids(&typed_tree, typed_id).len(),
            2,
            "a typed character replaces the hint rather than joining it"
        );
    }

    #[test]
    fn a_placeholder_is_never_part_of_the_value() {
        // The oldest bug in web forms: a hint that is really text in the field
        // and submits itself the moment nobody looks.
        let object = field("").placeholder("Email", vieww_foundation::Color::BLACK);
        assert_eq!(object.value.text, "");
    }

    #[test]
    fn tapping_a_placeholder_puts_the_caret_at_the_start() {
        // The paragraph on screen is the hint, so an unconverted hit test would
        // answer with an offset into text the field does not contain — and the
        // caret would sit three characters into an empty field.
        let (tree, id) = mount(field("").placeholder("Email", vieww_foundation::Color::BLACK));
        let object: &dyn std::any::Any = tree.object(id).expect("mounted");
        let editable = object
            .downcast_ref::<RenderEditableText>()
            .expect("mounted as its own type");
        let position = editable
            .position_at(Offset::new(30.0, 4.0))
            .expect("laid out");
        assert_eq!(position.offset, 0);
    }

    #[test]
    fn a_placeholder_is_the_label_and_not_the_value() {
        // It says what the field is *for*. Announcing it as the value would tell
        // somebody the field already contains the hint — the same lie the pixels
        // would tell if the placeholder were really in the text.
        use crate::RenderObject;
        let semantics = field("")
            .placeholder("Email", vieww_foundation::Color::BLACK)
            .semantics()
            .expect("announced");
        assert_eq!(semantics.label.as_deref(), Some("Email"));
        assert_eq!(semantics.value.as_deref(), Some(""));
    }
}
