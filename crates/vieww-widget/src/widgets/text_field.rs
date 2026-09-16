use vieww_foundation::{
    Color, Key, Obscured, TextAlign, TextDecoration, TextDirection, TextEditingValue,
    TextLayoutProbe, TextSelection, TextStyle, DEFAULT_OBSCURING_CHARACTER,
};

use crate::{widget_node_from, Handler, Widget, WidgetKind};
use std::rc::Rc;
use vieww_text::TextSpan;

/// Editable text: a caret, a selection, and the glyphs between them.
///
/// A render leaf. Its render object shapes the text through the text layer, paints
/// the selection behind it and the caret over it, and turns taps and drags into
/// offsets by hit-testing the same layout that was drawn.
///
/// # Controlled, not self-managing
///
/// A field does **not** own its value. It is handed a [`TextEditingValue`] and
/// reports every requested change back through
/// [`on_changed`](Self::on_changed); the caller applies it and hands down the
/// result. The classic controlled-input and editing-controller patterns,
/// pointed the other way round.
///
/// This is not a preference, it falls out of `docs/DESIGN.md` §7. Durable state
/// that a *gesture* can change needs something that both mutates it and marks the
/// element pending, and the only such thing is a `Signal` — which lives in
/// `vieww-element`, above this layer, where a widget cannot name it.
/// [`ElementState`](crate::ElementState) is not an alternative: its hooks run
/// during reconciliation and the animate phase, deliberately, so that a build
/// stays free of side effects (§1). There is nowhere for a tap to write.
///
/// The result is the same shape as everything else here — a value in a signal
/// above, a handler writing it — and it means the caret drawn on screen and the
/// caret in the model are the same one rather than two that have to be kept in
/// step.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::TextField;
/// use vieww_widget::foundation::TextEditingValue;
/// use std::rc::Rc;
///
/// # let value = TextEditingValue::new("hello");
/// # let apply: Rc<dyn Fn(TextEditingValue)> = Rc::new(|_| {});
/// let field = TextField::new(value.clone())
///     .size(16.0)
///     .on_changed(Rc::new(move |next| apply(next)));
/// ```
///
/// # Lines
///
/// **A field is multiline.** `Enter` becomes `TextIntent::Newline` and inserts a
/// line break, the paragraph breaks on it, and `ArrowUp`/`ArrowDown` move between
/// lines against a goal column so walking up through a short line and back down
/// returns to the column you started in. Text wraps within the available width
/// on its own; nothing has to be turned on.
///
/// **[`single_line`](Self::single_line) turns that off**, and
/// [`on_submit`](Self::on_submit) implies it: `Enter` then reports the text
/// instead of opening a line, which is what a login form or a search box wants.
/// It governs the key rather than the content — a `\n` handed down in the value
/// still lays out as two lines.
///
/// # What it does not do yet
///
/// No clipboard: cut, copy and paste have nowhere to go, because a pasteboard is
/// an OS service and no platform provides one to this tree yet.
#[derive(Clone)]
pub struct TextField {
    value: TextEditingValue,
    style: TextStyle,
    align: TextAlign,
    direction: Option<TextDirection>,
    cursor_color: Color,
    cursor_width: f32,
    selection_color: Option<Color>,
    show_cursor: bool,
    on_changed: Option<Handler<TextEditingValue>>,
    /// Where a *pointer* reports what it selected, when the caller wants that
    /// separately from [`on_changed`](Self::on_changed).
    ///
    /// See [`on_selection`](Self::on_selection) for why this exists at all.
    on_selection: Option<Handler<TextSelection>>,
    /// Where a modified click reports the caret it wants *added*.
    ///
    /// [`with_added_caret`](Self::with_added_caret) has been the widget-side
    /// half of multi-caret since it was written, and had no way to be reached:
    /// a pointer could not tell the application a modifier was held, because
    /// `PointerEvent` did not carry one. It does now, so this is the other half.
    on_add_caret: Option<Handler<TextSelection>>,
    single_line: bool,
    on_submit: Option<Handler<String>>,
    obscure: Obscured,
    placeholder: String,
    placeholder_color: Option<Color>,
    key: Option<Key>,
    spans: Option<Vec<TextSpan>>,
    diagnostic_lines: Option<(Rc<Vec<usize>>, Color)>,
    /// Marks keyed to ranges of the text rather than to lines.
    decorations: Option<Rc<Vec<TextDecoration>>>,
    wrap: bool,
    /// Where the field publishes what it measured. See [`TextLayoutProbe`].
    probe: Option<TextLayoutProbe>,
}

impl TextField {
    #[must_use]
    pub fn new(value: TextEditingValue) -> Self {
        Self {
            value,
            style: TextStyle::default(),
            align: TextAlign::default(),
            direction: None,
            cursor_color: Color::BLACK,
            cursor_width: 2.0,
            selection_color: None,
            show_cursor: true,
            on_changed: None,
            on_selection: None,
            on_add_caret: None,
            single_line: false,
            on_submit: None,
            obscure: Obscured::No,
            placeholder: String::new(),
            placeholder_color: None,
            key: None,
            spans: None,
            diagnostic_lines: None,
            decorations: None,
            wrap: true,
            probe: None,
        }
    }

    /// A field holding `text`, with the caret at the end.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::new(TextEditingValue::new(text))
    }

    /// Replace the whole style.
    #[must_use]
    pub const fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    #[must_use]
    pub const fn size(mut self, size: f32) -> Self {
        self.style.size = size;
        self
    }

    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.style.color = color;
        self
    }

    /// How lines sit within the available width.
    #[must_use]
    pub const fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    /// Force a base direction instead of inferring one from the text.
    #[must_use]
    pub const fn direction(mut self, direction: TextDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    /// The caret's colour and thickness in logical pixels.
    #[must_use]
    pub const fn cursor(mut self, color: Color, width: f32) -> Self {
        self.cursor_color = color;
        self.cursor_width = width;
        self
    }

    /// Whether to draw the caret at all.
    ///
    /// Both the blink's off phase and an unfocused field are this being `false`.
    /// The field does not blink on its own: a blink is an animation, and driving
    /// one needs a controller the application holds.
    #[must_use]
    pub const fn show_cursor(mut self, show: bool) -> Self {
        self.show_cursor = show;
        self
    }

    /// The colour painted behind a selection.
    #[must_use]
    pub const fn selection_color(mut self, color: Color) -> Self {
        self.selection_color = Some(color);
        self
    }

    /// Called with the value the field would like to become.
    ///
    /// Without it the field is read-only, and registers no gesture recognisers at
    /// all — so a field inside a scrollable does not swallow the drag that should
    /// have scrolled it.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<TextEditingValue>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// Report a pointer's selection as a *selection*, not as a whole value.
    ///
    /// # Why a field this small needs its own callback
    ///
    /// A pointer can only move the caret, so the render object reports a
    /// [`TextSelection`] and something has to turn that into the
    /// [`TextEditingValue`] the caller wants. Without this handler that
    /// assembly happens in [`with_selection`](Self::with_selection), out of the
    /// *widget's* text — and a widget is the tree as of the last build, which
    /// during a drag is one or more frames behind the value the caller is
    /// holding. Several drag updates can arrive within a single frame
    /// (`RenderEditableText` says so in its own docs), and each of them
    /// assembles from the same stale string.
    ///
    /// So the pointer path could hand back text nobody typed. A caller that
    /// applies what it is given then writes that text into its buffer, and a
    /// gesture that is supposed to be a pure read has edited the document.
    ///
    /// With this set, the pointer path carries **no text at all**: the caller
    /// receives the selection and applies it to whatever it currently holds,
    /// which is the only string that can be correct. `on_changed` continues to
    /// carry whole values, because a key genuinely can change text, selection
    /// and composing region at once and there is nothing to assemble.
    ///
    /// Optional, and the fallback is the old assembly, so every existing field
    /// keeps working unchanged.
    #[must_use]
    pub fn on_selection(mut self, handler: Handler<TextSelection>) -> Self {
        self.on_selection = Some(handler);
        self
    }

    /// Report a modified click as a caret to **add**, not one to move to.
    ///
    /// ⌘-click (Ctrl elsewhere) means "another caret here" in every editor that
    /// has more than one, and until `TapDetails` carried modifiers no
    /// application on vieww could implement it — which is why multi-caret
    /// shipped with commands only. Pair this with
    /// [`with_added_caret`](Self::with_added_caret), which builds the value the
    /// handler wants.
    ///
    /// Without it, a modified click behaves as a plain one. That is the right
    /// fallback: a search box should not sprout a second caret because
    /// somebody was holding Control.
    #[must_use]
    pub fn on_add_caret(mut self, handler: Handler<TextSelection>) -> Self {
        self.on_add_caret = Some(handler);
        self
    }

    /// One line only: `Enter` submits instead of opening a line.
    ///
    /// A field is multiline unless this is called, because that is the wider
    /// behaviour and the narrower one should be the thing asked for. This is what
    /// a login form, a search box or any single-value field wants.
    ///
    /// The key is consumed either way — with or without an
    /// [`on_submit`](Self::on_submit) handler — so a single-line field never
    /// grows. Nothing else changes: the value is still whatever the application
    /// hands down, so a `\n` set programmatically still lays out as two lines.
    /// This governs the key, not the content.
    #[must_use]
    pub fn single_line(mut self) -> Self {
        self.single_line = true;
        self
    }

    /// Called with the field's text when `Enter` is pressed.
    ///
    /// Implies [`single_line`](Self::single_line) — asking to be told about
    /// `Enter` and also having it insert a line break is not a coherent request,
    /// and requiring both calls would make forgetting one a silent bug.
    #[must_use]
    pub fn on_submit(mut self, handler: Handler<String>) -> Self {
        self.single_line = true;
        self.on_submit = Some(handler);
        self
    }

    /// Show a bullet for each character instead of the text.
    ///
    /// The ordinary password field. `obscure(false)` is the default and shows
    /// the text; the character is [`DEFAULT_OBSCURING_CHARACTER`] unless
    /// [`obscure_with`](Self::obscure_with) says otherwise.
    ///
    /// # This is not styling
    ///
    /// The paragraph is *shaped* from the mask, so the caret, the selection and
    /// every hit test resolve against what is on screen rather than against the
    /// text underneath. The field also stops publishing its contents to the
    /// accessibility tree and reports a character count instead — a screen
    /// reader is a second way to read the pixels, and the pixels no longer
    /// carry the password.
    ///
    /// What it deliberately does **not** do is change what the field holds.
    /// `on_changed` still reports the real value, because the application needs
    /// the password and the mask is about who can see it.
    #[must_use]
    pub const fn obscure(mut self, obscure: bool) -> Self {
        self.obscure = if obscure {
            Obscured::With(DEFAULT_OBSCURING_CHARACTER)
        } else {
            Obscured::No
        };
        self
    }

    /// Obscure with a character of your own.
    #[must_use]
    pub const fn obscure_with(mut self, mask: char) -> Self {
        self.obscure = Obscured::With(mask);
        self
    }

    /// Show `placeholder` while the field is empty.
    ///
    /// **Not the same as pre-filling the field.** The placeholder is never part
    /// of the value, never selected, never submitted, and the caret sits at
    /// offset zero in front of it. A screen reader hears it as the field's
    /// *label* rather than its value, which is the only honest reading: it says
    /// what the field is for, not what is in it.
    ///
    /// `color` falls back to the theme's `on_surface_variant` when left `None`
    /// by [`placeholder_color`](Self::placeholder_color) — a hint is a hint in
    /// whatever theme it lands in, and dimming the body colour by a fraction
    /// chosen here would not match the one the theme already has.
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The colour the placeholder is drawn in.
    #[must_use]
    pub const fn placeholder_color(mut self, color: Color) -> Self {
        self.placeholder_color = Some(color);
        self
    }

    /// What the field shows instead of what it holds.
    #[must_use]
    pub const fn obscuring(&self) -> Obscured {
        self.obscure
    }

    /// The hint shown while the field is empty.
    #[must_use]
    pub fn placeholder_text(&self) -> &str {
        &self.placeholder
    }

    /// The placeholder's colour, if one was set.
    #[must_use]
    pub const fn placeholder_shade(&self) -> Option<Color> {
        self.placeholder_color
    }

    /// Supply syntax-highlighted styled runs for the editable text.
    ///
    /// The runs must concatenate to the field value. The render object uses the
    /// same shaped paragraph for painting, hit testing, cursor geometry and
    /// selection, so syntax highlighting cannot get out of step with editing.
    #[must_use]
    pub fn spans(mut self, spans: Vec<TextSpan>) -> Self {
        self.spans = Some(spans);
        self
    }

    /// Mark ranges of the text: a squiggle under an error, a box around the
    /// bracket matching the one at the caret, a highlight on every other
    /// occurrence of the selected word.
    ///
    /// Unlike [`diagnostics`](Self::diagnostics), which marks whole *lines*,
    /// this is positioned from the shaped paragraph — so a mark follows
    /// wrapping, ends where the range ends rather than where the line does, and
    /// costs nothing at layout time because none of it can move a glyph.
    #[must_use]
    pub fn decorations(mut self, decorations: Rc<Vec<TextDecoration>>) -> Self {
        self.decorations = Some(decorations);
        self
    }

    /// Paint subtle line markers behind source lines with compiler diagnostics.
    #[must_use]
    pub fn diagnostics(mut self, lines: Rc<Vec<usize>>, color: Color) -> Self {
        self.diagnostic_lines = Some((lines, color));
        self
    }

    /// Turn off wrapping a line longer than the available width.
    ///
    /// Only meaningful alongside per-source-line chrome painted beside the
    /// field — a gutter, [`diagnostics`](Self::diagnostics) — which assumes
    /// one source line is one visual row. Leave this on (the default) for an
    /// ordinary field; a gutter that does not exist cannot disagree with a
    /// wrap it does not know about.
    #[must_use]
    pub const fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Publish the field's measured geometry into `probe` after every layout.
    ///
    /// What [`decorations`](Self::decorations) is for marks *inside* the
    /// field, this is for everything drawn *beside* it — a completion popup at
    /// the caret, a hint after the end of a line, a ruler sized to the
    /// paragraph. Those live in the caller's tree, above the field, and have
    /// to be given a position in advance; the position only exists after
    /// shaping, and only the render object has it.
    ///
    /// The report a caller reads during `build` is the **previous** frame's.
    /// Read [`TextLayoutProbe`]'s module documentation before relying on one.
    #[must_use]
    pub fn probe(mut self, probe: TextLayoutProbe) -> Self {
        self.probe = Some(probe);
        self
    }

    /// The probe this field publishes into, if it was given one.
    #[must_use]
    pub const fn layout_probe(&self) -> Option<&TextLayoutProbe> {
        self.probe.as_ref()
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub const fn editing_value(&self) -> &TextEditingValue {
        &self.value
    }

    #[must_use]
    pub const fn text_style(&self) -> &TextStyle {
        &self.style
    }

    #[must_use]
    pub const fn text_align(&self) -> TextAlign {
        self.align
    }

    #[must_use]
    pub const fn text_direction(&self) -> Option<TextDirection> {
        self.direction
    }

    #[must_use]
    pub const fn cursor_color(&self) -> Color {
        self.cursor_color
    }

    #[must_use]
    pub const fn cursor_width(&self) -> f32 {
        self.cursor_width
    }

    #[must_use]
    pub const fn selection_highlight(&self) -> Option<Color> {
        self.selection_color
    }

    #[must_use]
    pub fn diagnostic_markers(&self) -> Option<(&Rc<Vec<usize>>, Color)> {
        self.diagnostic_lines
            .as_ref()
            .map(|(lines, color)| (lines, *color))
    }

    #[must_use]
    pub fn text_decorations(&self) -> Option<&Rc<Vec<TextDecoration>>> {
        self.decorations.as_ref()
    }

    #[must_use]
    pub fn text_spans(&self) -> Option<&[TextSpan]> {
        self.spans.as_deref()
    }

    #[must_use]
    pub const fn cursor_shown(&self) -> bool {
        self.show_cursor
    }

    /// The change handler, if the field is editable.
    #[must_use]
    pub const fn changed_handler(&self) -> Option<&Handler<TextEditingValue>> {
        self.on_changed.as_ref()
    }

    /// The pointer-selection handler, if the caller wanted one.
    #[must_use]
    pub const fn selection_handler(&self) -> Option<&Handler<TextSelection>> {
        self.on_selection.as_ref()
    }

    /// The add-a-caret handler, if the caller wanted one.
    #[must_use]
    pub const fn add_caret_handler(&self) -> Option<&Handler<TextSelection>> {
        self.on_add_caret.as_ref()
    }

    /// Whether `Enter` submits rather than opening a line.
    #[must_use]
    pub const fn is_single_line(&self) -> bool {
        self.single_line
    }

    /// Whether a line longer than the available width wraps onto a second
    /// visual row. See [`wrap`](Self::wrap).
    #[must_use]
    pub const fn wraps(&self) -> bool {
        self.wrap
    }

    /// The submit handler, if there is one.
    #[must_use]
    pub const fn submit_handler(&self) -> Option<&Handler<String>> {
        self.on_submit.as_ref()
    }

    /// The value this field would become if the selection moved to `selection`.
    ///
    /// The render object reports a *selection*, because placing a caret is all a
    /// pointer can do; the handler wants a whole value. Doing the assembly here
    /// keeps the render layer from having to know what a `TextEditingValue` is
    /// made of.
    #[must_use]
    pub fn with_selection(&self, selection: TextSelection) -> TextEditingValue {
        TextEditingValue {
            text: self.value.text.clone(),
            selection,
            // Moving the caret ends a composition: the input method was deciding
            // about a run of text the user has just navigated away from.
            composing: None,
            // And it ends multiple carets, for the same reason a click does in
            // every editor: a plain click is a new, single place. The render
            // object reports an *added* caret through its own path rather than
            // through this one — see `on_add_caret`.
            secondary: Vec::new(),
        }
    }

    /// The value this field would become with `selection` added as a caret.
    ///
    /// Separate from [`with_selection`](Self::with_selection) because the two
    /// gestures mean opposite things: a click replaces every caret, and a
    /// modified click adds one. A single method with a flag would put that
    /// decision in the render layer, which is not where it belongs.
    #[must_use]
    pub fn with_added_caret(&self, selection: TextSelection) -> TextEditingValue {
        let mut value = self.value.clone();
        value.composing = None;
        value.add_caret(selection);
        value
    }
}

impl Widget for TextField {
    fn debug_name(&self) -> &'static str {
        "TextField"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("text", format!("{:?}", self.value.text)),
            (
                "selection",
                format!(
                    "{}..{}",
                    self.value.selection.start(),
                    self.value.selection.end()
                ),
            ),
        ];
        if let Some(composing) = self.value.composing {
            props.push((
                "composing",
                format!("{}..{}", composing.start, composing.end),
            ));
        }
        if self.on_changed.is_none() {
            props.push(("read_only", "true".to_owned()));
        }
        props
    }
}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The handler is a closure, which is neither `Debug` nor informative;
        // whether there *is* one is the part worth printing.
        f.debug_struct("TextField")
            .field("value", &self.value)
            .field("style", &self.style)
            .field("editable", &self.on_changed.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(TextField);

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vieww_foundation::TextRange;

    use super::*;

    #[test]
    fn a_field_with_no_handler_is_read_only() {
        let field = TextField::text("hello");
        assert!(field.changed_handler().is_none());
        assert!(field
            .debug_properties()
            .iter()
            .any(|(name, _)| *name == "read_only"));
    }

    #[test]
    fn moving_the_caret_keeps_the_text_and_drops_any_composition() {
        let mut value = TextEditingValue::new("kana");
        value.set_composing(Some(TextRange::new(0, 4)));
        let field = TextField::new(value);

        let moved = field.with_selection(TextSelection::collapsed(2));
        assert_eq!(moved.text, "kana", "placing a caret does not edit");
        assert_eq!(moved.selection, TextSelection::collapsed(2));
        assert_eq!(
            moved.composing, None,
            "the user navigated away from what the input method was deciding about"
        );
    }

    #[test]
    fn a_handler_receives_the_value_the_field_wants_to_become() {
        let seen: Rc<RefCell<Option<TextEditingValue>>> = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let field = TextField::text("hello").on_changed(Rc::new(move |value| {
            *sink.borrow_mut() = Some(value);
        }));

        let handler = field.changed_handler().expect("editable").clone();
        handler(field.with_selection(TextSelection::new(1, 3)));

        let seen = seen.borrow();
        let value = seen.as_ref().expect("reported");
        assert_eq!(value.selected_text(), "el");
    }

    #[test]
    fn the_debug_dump_shows_where_the_caret_is() {
        let mut value = TextEditingValue::new("hello");
        value.selection = TextSelection::new(1, 4);
        let props = TextField::new(value).debug_properties();

        assert!(
            props
                .iter()
                .any(|(name, shown)| *name == "selection" && shown == "1..4"),
            "{props:?}"
        );
    }
}
