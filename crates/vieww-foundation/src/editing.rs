//! What an editable string *is*, before anything has been shaped.
//!
//! A caret position, a selection and a composing region are indices into a
//! `String`. None of them need a font, a glyph or a line box to be meaningful, so
//! none of them belong in `vieww-text` — and they cannot live there anyway,
//! because a `TextField` widget has to carry its value and the widget layer must
//! not depend on a shaper (`docs/DESIGN.md` §7).
//!
//! What *does* need shaping is every question involving position on screen:
//! hit-testing a tap to an offset, the rectangle to draw a caret in, the boxes
//! that highlight a selection, and moving up or down a line. Those are methods on
//! `Paragraph` in `vieww-text`. The split is the same one as everywhere else in
//! this crate: the data model here, the geometry where the glyphs are.
//!
//! # Offsets are byte offsets
//!
//! Every index in this module is a byte offset into the UTF-8 string, and every
//! one produced by this module lands on a **grapheme cluster** boundary. Bytes
//! rather than chars because that is what `String` slicing takes and what
//! `cosmic-text` reports glyph ranges in, so no index has to be translated at any
//! boundary. Grapheme clusters rather than chars because a `char` is not what a
//! user thinks the caret moves over: `é` written as `e` + U+0301 is two chars and
//! one thing to delete, and a flag emoji is two chars, or four, depending on the
//! flag.

use unicode_segmentation::UnicodeSegmentation;

use crate::{Color, ImeEvent, TextIntent};

/// Which side of a line break a position belongs to.
///
/// A wrapped line makes one offset into two places on screen. In `"ab cd"` broken
/// after the space, offset 3 is both the end of the first line and the start of
/// the second, and a caret has to be drawn at exactly one of them. Affinity is
/// the tie-break, and it is carried on the *position* rather than resolved at
/// draw time because the thing that knows the answer is whatever produced the
/// position — a tap knows which line was tapped, and pressing End knows the user
/// meant the end of the line they were on.
///
/// It has no effect on any offset that is not a soft line break, which is almost
/// all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Affinity {
    /// Belongs to the text *before* it: the end of the earlier line.
    Upstream,
    /// Belongs to the text *after* it: the start of the later line. The default,
    /// because it is the answer for every position that is not a break.
    #[default]
    Downstream,
}

/// A caret position: an offset, plus which line it sits on when that is ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TextPosition {
    /// Byte offset into the text. `0..=text.len()` — the end is a valid caret
    /// position, which is why this is not an index into anything.
    pub offset: usize,
    pub affinity: Affinity,
}

impl TextPosition {
    /// A position with the default [`Affinity::Downstream`].
    #[must_use]
    pub const fn new(offset: usize) -> Self {
        Self {
            offset,
            affinity: Affinity::Downstream,
        }
    }

    #[must_use]
    pub const fn upstream(offset: usize) -> Self {
        Self {
            offset,
            affinity: Affinity::Upstream,
        }
    }

    #[must_use]
    pub const fn with_affinity(mut self, affinity: Affinity) -> Self {
        self.affinity = affinity;
        self
    }
}

/// What a [`TextDecoration`] draws over a range of text.
///
/// # Why the shape is an enum and not a path
///
/// A decoration is positioned from the *shaped* paragraph — the rects a range
/// occupies, which only the render object knows — and drawn in the render
/// object's own paint pass. A caller handing over a `Path` would be describing
/// a shape in coordinates it cannot compute, so it names the kind and the
/// geometry is derived where the geometry is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextDecorationShape {
    /// A wave under the range: a compiler diagnostic, a misspelling.
    #[default]
    Squiggle,
    /// A straight rule under the range.
    Underline,
    /// A rule through the middle of it.
    Strike,
    /// A hairline outline around it: a matched bracket, a highlighted
    /// occurrence of the word under the caret.
    Box,
    /// A vertical rule down the **leading edge** of the range, the full height
    /// of the line box it sits on.
    ///
    /// The one shape whose geometry is a position rather than an extent: only
    /// the leading edge and the line's height are read, and the range's width
    /// is ignored. It exists because the marks an editor draws *between*
    /// characters — an indent guide, a ruler at a column, the seam of a
    /// vertical selection — are otherwise inexpressible. They have no text to
    /// underline, and the only thing that knows where a column falls after
    /// shaping is whatever holds the [`Paragraph`].
    ///
    /// Nothing about it assumes a script or a platform. The rule is drawn at
    /// the leading edge of whatever range is named, which in a right-to-left
    /// paragraph is that range's right edge, because `selection_rects` already
    /// reports its rects that way.
    ///
    /// A caller wanting a guide at a column names the one grapheme at that
    /// column as the range; an empty range draws nothing, as for every other
    /// shape.
    ///
    /// [`Paragraph`]: https://docs.rs/vieww-text
    Guide,
}

/// A mark drawn over a range of an editable field's text.
///
/// # What this exists for
///
/// Everything an editor draws that is *keyed to a span of text rather than to
/// a line*: the squiggle under an error, the box around the bracket matching
/// the one at the caret, the highlight on every other occurrence of the
/// selected word. None of it is expressible from outside the render object,
/// because all of it needs the rects a byte range occupies after shaping, and
/// those are only known to whatever holds the [`Paragraph`] — which is not the
/// application.
///
/// [`Paragraph`]: https://docs.rs/vieww-text
///
/// # Paint-only, and deliberately
///
/// Adding, removing or recolouring a decoration cannot move a glyph, so it is
/// excluded from the render object's layout comparison: a field that
/// re-squiggles on every keystroke must not re-shape its paragraph on every
/// keystroke. That is the same reason the caret's colour and the selection
/// highlight are excluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextDecoration {
    /// The range, in the *real* text's byte offsets. An obscured field maps it
    /// to display coordinates before drawing, the same way the selection is.
    pub range: TextRange,
    pub shape: TextDecorationShape,
    pub color: Color,
    /// Line width, in logical pixels. Zero asks the field to pick one from its
    /// font size, which is what a caller that has no opinion should do.
    pub thickness: f32,
}

impl TextDecoration {
    /// A squiggle under `range`, at a thickness derived from the font size.
    #[must_use]
    pub const fn squiggle(range: TextRange, color: Color) -> Self {
        Self {
            range,
            shape: TextDecorationShape::Squiggle,
            color,
            thickness: 0.0,
        }
    }

    /// An outline around `range`.
    #[must_use]
    pub const fn boxed(range: TextRange, color: Color) -> Self {
        Self {
            range,
            shape: TextDecorationShape::Box,
            color,
            thickness: 0.0,
        }
    }

    /// A vertical rule down the leading edge of `range`.
    ///
    /// `range` names the grapheme the rule stands in front of — one character,
    /// not a span — because the shape reads a position and ignores the extent.
    #[must_use]
    pub const fn guide(range: TextRange, color: Color) -> Self {
        Self {
            range,
            shape: TextDecorationShape::Guide,
            color,
            thickness: 0.0,
        }
    }

    #[must_use]
    pub const fn shape(mut self, shape: TextDecorationShape) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub const fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness;
        self
    }

    /// The line width to draw with, given the font size it sits under.
    #[must_use]
    pub fn resolved_thickness(&self, font_size: f32) -> f32 {
        if self.thickness > 0.0 {
            self.thickness
        } else {
            (font_size * 0.08).clamp(1.0, 3.0)
        }
    }
}

/// `selection` moved by `delta` bytes, clamped at zero.
///
/// Free rather than a method on `TextSelection` because it is arithmetic in
/// service of one algorithm — `apply_at_every_caret` — and a public "move this
/// selection by an arbitrary number of bytes" invites callers to do the shift
/// themselves, which is the thing that goes wrong.
fn shifted(selection: TextSelection, delta: isize) -> TextSelection {
    let move_one = |offset: usize| -> usize {
        if delta >= 0 {
            offset.saturating_add(delta.unsigned_abs())
        } else {
            offset.saturating_sub(delta.unsigned_abs())
        }
    };
    TextSelection {
        base: move_one(selection.base),
        extent: move_one(selection.extent),
        affinity: selection.affinity,
    }
}

/// A half-open byte range, always ordered.
///
/// Ordered on construction rather than by a `normalized()` a caller has to
/// remember: an unordered range slices a `String` by panicking, and the only
/// place order carries meaning is a selection, which keeps its own ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    #[must_use]
    pub const fn new(a: usize, b: usize) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }

    /// A range covering nothing, at `offset`.
    #[must_use]
    pub const fn collapsed(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// `true` if `offset` falls inside the range, excluding the end.
    #[must_use]
    pub const fn contains(&self, offset: usize) -> bool {
        offset >= self.start && offset < self.end
    }

    /// The overlap with `other`, or `None` where they do not touch.
    ///
    /// Selection highlighting asks this per line: the range that is selected
    /// intersected with the range the line covers.
    #[must_use]
    pub fn intersect(&self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end.min(other.end);
        (start < end).then_some(Self { start, end })
    }

    /// The slice of `text` this range covers, or `""` if it is out of bounds or
    /// does not land on character boundaries.
    #[must_use]
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        text.get(self.start..self.end).unwrap_or("")
    }
}

/// A selection: an anchor, a moving end, and the direction between them.
///
/// Two ends rather than a [`TextRange`] because which end moves is a real
/// distinction. Shift-clicking, or dragging, extends from the `base` and moves
/// the `extent`; reversing past the anchor keeps the anchor. A range cannot
/// express that, so it would be lost the first time a drag crossed its own start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct TextSelection {
    /// The fixed end — where the selection started.
    pub base: usize,
    /// The moving end — where the caret is.
    pub extent: usize,
    /// The affinity of the [`extent`](Self::extent), which is where a caret draws.
    pub affinity: Affinity,
}

impl TextSelection {
    /// A caret: a selection of nothing, at `offset`.
    #[must_use]
    pub const fn collapsed(offset: usize) -> Self {
        Self {
            base: offset,
            extent: offset,
            affinity: Affinity::Downstream,
        }
    }

    #[must_use]
    pub const fn new(base: usize, extent: usize) -> Self {
        Self {
            base,
            extent,
            affinity: Affinity::Downstream,
        }
    }

    #[must_use]
    pub const fn with_affinity(mut self, affinity: Affinity) -> Self {
        self.affinity = affinity;
        self
    }

    /// The lower offset, whichever end it is.
    #[must_use]
    pub const fn start(&self) -> usize {
        if self.base <= self.extent {
            self.base
        } else {
            self.extent
        }
    }

    /// The higher offset, whichever end it is.
    #[must_use]
    pub const fn end(&self) -> usize {
        if self.base <= self.extent {
            self.extent
        } else {
            self.base
        }
    }

    /// The selection as an ordered range, losing which end moved.
    #[must_use]
    pub const fn range(&self) -> TextRange {
        TextRange {
            start: self.start(),
            end: self.end(),
        }
    }

    /// `true` if this is a caret rather than a selection.
    #[must_use]
    pub const fn is_collapsed(&self) -> bool {
        self.base == self.extent
    }

    /// `true` if the selection was made backwards — the caret is before the anchor.
    #[must_use]
    pub const fn is_reversed(&self) -> bool {
        self.extent < self.base
    }

    /// The caret position: the moving end, with the selection's affinity.
    #[must_use]
    pub const fn cursor(&self) -> TextPosition {
        TextPosition {
            offset: self.extent,
            affinity: self.affinity,
        }
    }

    /// Move the caret to `offset`, dragging the anchor with it unless `extend`.
    #[must_use]
    pub const fn moved_to(&self, offset: usize, extend: bool) -> Self {
        Self {
            base: if extend { self.base } else { offset },
            extent: offset,
            affinity: Affinity::Downstream,
        }
    }
}

/// The complete state of an editable string: what it says, where the caret is,
/// and what the input method is still deciding.
///
/// One value rather than three fields on a widget, because they change together
/// and an input method bridge replaces all three at once. Typing a character
/// moves the caret; deleting a selection moves it *and* shortens the text; and an
/// IME's job is precisely to hand back a new text, selection and composing region
/// as a unit. Splitting them makes every one of those two or three assignments
/// that can be half-applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextEditingValue {
    pub text: String,
    pub selection: TextSelection,
    /// The range the platform input method is composing, if any.
    ///
    /// Text inside it is provisional — typing `ka` towards `か` puts `ka` in the
    /// text so it can be seen, but it is not committed and is drawn underlined.
    /// `None` whenever nothing is being composed, which is the whole time on a
    /// hardware keyboard typing Latin. Filled in by the Phase 8 IME bridge; kept
    /// here so the editing model does not have to change when that lands.
    pub composing: Option<TextRange>,
    /// Further selections, edited alongside [`selection`](Self::selection).
    ///
    /// # Why this is a second field and not `selection: Vec<TextSelection>`
    ///
    /// One caret is what a text field has, and every other kind of field in
    /// every application built on this — a search box, a name, a number — has
    /// exactly one forever. Making the common case a vector would put an
    /// allocation and an index in front of every one of them, and would change
    /// the type every existing caller reads.
    ///
    /// So the primary is still *the* selection: it is what a single-caret
    /// field uses, what the IME composes against, what a drag extends, and
    /// what [`selected_text`](Self::selected_text) returns. This is the set of
    /// extra ones, empty in every field that has not asked for them — and
    /// [`apply`](Self::apply) takes its single-caret path unchanged when it
    /// is, so nothing that does not use multiple carets pays for them or can
    /// be broken by them.
    ///
    /// # The ordering rule
    ///
    /// Kept sorted by [`TextSelection::start`] and non-overlapping, by
    /// [`normalise_carets`](Self::normalise_carets). Two carets at one offset
    /// are one caret; two selections that touch are one selection. Without
    /// that, typing at two carets in the same place inserts twice.
    pub secondary: Vec<TextSelection>,
}

impl TextEditingValue {
    /// A value holding `text`, with the caret at the end.
    ///
    /// At the end rather than the start because that is where a field
    /// pre-populated with a value wants it — the user is about to append or
    /// correct, not insert at the front.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            selection: TextSelection::collapsed(end),
            composing: None,
            secondary: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text the selection covers, or `""` when it is a caret.
    #[must_use]
    pub fn selected_text(&self) -> &str {
        self.selection.range().slice(&self.text)
    }

    /// Replace whatever is selected with `insertion`, leaving the caret after it.
    ///
    /// The single primitive every edit is built from. Typing a character, pasting,
    /// committing an IME composition and deleting are all this with different
    /// arguments, which is what keeps the selection bookkeeping in one place.
    pub fn replace_selection(&mut self, insertion: &str) {
        let range = self.selection.range();
        self.replace_range(range, insertion);
    }

    /// Replace `range` with `insertion` and put the caret after it.
    ///
    /// Clamped to grapheme boundaries, so a range arriving from a platform IME in
    /// the wrong units cannot panic the string slice or leave a broken cluster.
    pub fn replace_range(&mut self, range: TextRange, insertion: &str) {
        let start = self.clamp_to_boundary(range.start);
        let end = self.clamp_to_boundary(range.end).max(start);

        self.text.replace_range(start..end, insertion);
        self.selection = TextSelection::collapsed(start + insertion.len());
        // Any composition is over: the text underneath it just changed. A stale
        // composing range would underline whatever moved into those bytes.
        self.composing = None;
    }

    /// Insert `insertion` at the caret, replacing the selection if there is one.
    pub fn insert(&mut self, insertion: &str) {
        self.replace_selection(insertion);
    }

    /// Backspace: delete the selection, or the grapheme before the caret.
    ///
    /// A grapheme rather than a `char` is the whole point. Backspacing `é` typed
    /// as `e` + U+0301 must remove both, or the accent is left orphaned on
    /// whatever precedes it; backspacing a family emoji must not leave half a
    /// family behind.
    pub fn delete_backward(&mut self) {
        if self.selection.is_collapsed() {
            let caret = self.selection.extent;
            let Some(previous) = self.previous_boundary(caret) else {
                return;
            };
            self.replace_range(TextRange::new(previous, caret), "");
        } else {
            self.replace_selection("");
        }
    }

    /// Delete forward: the selection, or the grapheme after the caret.
    pub fn delete_forward(&mut self) {
        if self.selection.is_collapsed() {
            let caret = self.selection.extent;
            let Some(next) = self.next_boundary(caret) else {
                return;
            };
            self.replace_range(TextRange::new(caret, next), "");
        } else {
            self.replace_selection("");
        }
    }

    /// Delete the selection, or back to the start of the previous word.
    ///
    /// Back to the *start of a word* rather than to the nearest boundary, so any
    /// whitespace in between goes with it. That is what makes repeated presses
    /// walk back through a sentence one word per press, instead of alternating
    /// between eating a space and eating a word.
    pub fn delete_word_backward(&mut self) {
        if !self.selection.is_collapsed() {
            self.replace_selection("");
            return;
        }
        let caret = self.selection.extent;
        let target = self.word_boundary_before(caret);
        if target < caret {
            self.replace_range(TextRange::new(target, caret), "");
        }
    }

    /// Delete the selection, or the word after the caret.
    pub fn delete_word_forward(&mut self) {
        if !self.selection.is_collapsed() {
            self.replace_selection("");
            return;
        }
        let caret = self.selection.extent;
        let target = self.word_boundary_after(caret);
        if target > caret {
            self.replace_range(TextRange::new(caret, target), "");
        }
    }

    // ------------------------------------------------------------------ moving
    //
    // These are *logical* movement — towards the start or the end of the string.
    // Visual movement, where the left arrow key goes left on screen even in a
    // right-to-left run, needs the shaped paragraph and lives there. Naming them
    // `previous`/`next` rather than `left`/`right` is deliberate: a `move_left`
    // here would be silently wrong for Hebrew, and the bug would not show up in
    // any Latin test.

    /// Move the caret one grapheme towards the start of the text.
    ///
    /// With `extend`, the anchor stays put and the selection grows. Without it, a
    /// selection **collapses to its start** rather than stepping back from the
    /// caret — pressing left with text selected means "go to the beginning of what
    /// I had selected", which is what every editor does.
    pub fn move_previous(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = TextSelection::collapsed(self.selection.start());
            return;
        }
        if let Some(previous) = self.previous_boundary(self.selection.extent) {
            self.selection = self.selection.moved_to(previous, extend);
        }
    }

    /// Move the caret one grapheme towards the end of the text.
    pub fn move_next(&mut self, extend: bool) {
        if !extend && !self.selection.is_collapsed() {
            self.selection = TextSelection::collapsed(self.selection.end());
            return;
        }
        if let Some(next) = self.next_boundary(self.selection.extent) {
            self.selection = self.selection.moved_to(next, extend);
        }
    }

    /// Move the caret to the start of the previous word.
    pub fn move_word_previous(&mut self, extend: bool) {
        let target = self.word_boundary_before(self.selection.extent);
        self.selection = self.selection.moved_to(target, extend);
    }

    /// Move the caret to the end of the next word.
    pub fn move_word_next(&mut self, extend: bool) {
        let target = self.word_boundary_after(self.selection.extent);
        self.selection = self.selection.moved_to(target, extend);
    }

    /// Put the caret at `position`, or extend the selection to it.
    ///
    /// What a tap and a drag call. The affinity is preserved because the caller —
    /// a hit test — is the only thing that knows which line was touched.
    pub fn move_to(&mut self, position: TextPosition, extend: bool) {
        let offset = self.clamp_to_boundary(position.offset);
        self.selection = self
            .selection
            .moved_to(offset, extend)
            .with_affinity(position.affinity);
    }

    /// Select everything, with the caret at the end.
    pub fn select_all(&mut self) {
        self.selection = TextSelection::new(0, self.text.len());
    }

    /// Select the word containing `offset`, which is what a double tap does.
    ///
    /// On a boundary between a word and a space, the word wins: double-tapping
    /// just after a word selects that word rather than the whitespace after it.
    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.clamp_to_boundary(offset);
        let range = self.word_range_at(offset);
        self.selection = TextSelection::new(range.start, range.end);
    }

    // ------------------------------------------------------------ multi-caret

    /// Every selection, primary first, in the order they were added.
    ///
    /// For a field that never asked for more than one, this is a one-element
    /// vector — which is why the render object reads
    /// [`sorted_carets`](Self::sorted_carets) instead when it is drawing, and
    /// why nothing on the single-caret path calls either.
    #[must_use]
    pub fn carets(&self) -> Vec<TextSelection> {
        let mut all = Vec::with_capacity(1 + self.secondary.len());
        all.push(self.selection);
        all.extend_from_slice(&self.secondary);
        all
    }

    /// Every selection in document order.
    #[must_use]
    pub fn sorted_carets(&self) -> Vec<TextSelection> {
        let mut all = self.carets();
        all.sort_by_key(TextSelection::start);
        all
    }

    /// How many carets there are. One unless something added more.
    #[must_use]
    pub fn caret_count(&self) -> usize {
        1 + self.secondary.len()
    }

    /// Whether more than one caret is active.
    #[must_use]
    pub fn is_multi_caret(&self) -> bool {
        !self.secondary.is_empty()
    }

    /// Add a caret, unless one is already there.
    ///
    /// Returns whether anything was added, so a caller can say "no more
    /// occurrences" rather than silently doing nothing.
    pub fn add_caret(&mut self, selection: TextSelection) -> bool {
        let before = self.caret_count();
        self.secondary.push(selection);
        self.normalise_carets();
        self.caret_count() > before
    }

    /// Back to one caret: the primary, wherever it is.
    ///
    /// What Escape does. The *primary* survives rather than the first in
    /// document order, because the primary is the one the user's last
    /// deliberate action put somewhere.
    pub fn clear_secondary_carets(&mut self) -> bool {
        if self.secondary.is_empty() {
            return false;
        }
        self.secondary.clear();
        true
    }

    /// Sort the carets, drop duplicates, and merge any that overlap.
    ///
    /// # Why merging is not optional
    ///
    /// Two carets at one offset are one caret, and typing at both inserts the
    /// character twice. Two selections that overlap describe one region, and
    /// replacing both replaces the shared part twice. Every editor that
    /// supports this does the merge on every change, because every movement
    /// can create the collision: five carets on five lines all pressing Home
    /// stay distinct, but five carets on *one* line pressing Home become one.
    ///
    /// The primary keeps its identity through the merge — it is whichever
    /// merged selection swallowed it — so the IME, the drag anchor and
    /// `selected_text` all still mean something afterwards.
    pub fn normalise_carets(&mut self) {
        if self.secondary.is_empty() {
            return;
        }
        let primary_start = self.selection.start();
        let mut all = self.carets();
        all.sort_by_key(|selection| (selection.start(), selection.end()));

        let mut merged: Vec<TextSelection> = Vec::with_capacity(all.len());
        for selection in all {
            match merged.last_mut() {
                // Touching counts as overlapping: two carets at the same
                // offset, and a selection ending exactly where the next
                // begins, are both one region.
                Some(last) if selection.start() <= last.end() => {
                    if selection.end() > last.end() {
                        // Keep the direction of the one that reaches furthest,
                        // so a merged selection still knows which end moves.
                        *last = TextSelection::new(last.start(), selection.end());
                    }
                }
                _ => merged.push(selection),
            }
        }

        // Whichever survivor covers where the primary was.
        let primary = merged
            .iter()
            .position(|selection| {
                selection.start() <= primary_start && primary_start <= selection.end()
            })
            .unwrap_or(0);
        self.selection = merged.remove(primary);
        self.secondary = merged;
    }

    /// Apply `intent` at every caret.
    ///
    /// # Why the carets are visited from the end of the text backwards
    ///
    /// An edit at offset 100 does not move offset 10; an edit at offset 10
    /// moves offset 100 by however much it grew or shrank the text. Working
    /// backwards means every caret is applied against a string whose bytes
    /// below it are still the ones it was measured against, and only the
    /// *results* already computed — which are all above — need shifting.
    ///
    /// Doing it forwards would mean re-deriving every remaining caret after
    /// every edit, which is the same arithmetic with more chances to get it
    /// wrong.
    fn apply_at_every_caret(&mut self, intent: &TextIntent) -> bool {
        let carets = self.carets();
        let mut order: Vec<usize> = (0..carets.len()).collect();
        order.sort_by_key(|&index| std::cmp::Reverse(carets[index].start()));

        let mut results: Vec<TextSelection> = carets.clone();
        let mut changed = false;

        for (position, &index) in order.iter().enumerate() {
            let before = self.text.len();
            // One caret, on the single-caret path, over the text as it stands.
            let mut one = Self {
                text: std::mem::take(&mut self.text),
                selection: carets[index],
                composing: None,
                secondary: Vec::new(),
            };
            changed |= one.apply(intent);
            self.text = std::mem::take(&mut one.text);
            results[index] = one.selection;

            let delta = self.text.len() as isize - before as isize;
            if delta != 0 {
                // Everything already done is above this edit, so it all moves.
                for &done in &order[..position] {
                    results[done] = shifted(results[done], delta);
                }
            }
        }

        self.selection = results[0];
        self.secondary = results[1..].to_vec();
        self.composing = None;
        self.normalise_carets();
        changed
    }

    // ----------------------------------------------------------------- intents

    /// Carry out what a keypress asked for.
    ///
    /// Returns `false` for the intents this type cannot carry out on its own —
    /// see [`TextIntent::needs_layout`]. Those need to know where a line begins
    /// and ends, which is a fact about the *shaped* text, and this crate has no
    /// fonts in it by design (`docs/DESIGN.md` §16). Whatever holds the
    /// geometry handles them and calls [`move_to`](Self::move_to) with the
    /// answer.
    ///
    /// [`TextIntent::Newline`] inserts a line break here. A single-line field
    /// wanting to submit instead should match on the intent before calling
    /// this, rather than expecting a refusal.
    pub fn apply(&mut self, intent: &TextIntent) -> bool {
        // The single-caret path, byte for byte as it was, whenever there is
        // one caret — which is every field in an application that has not
        // asked for more. See `secondary`.
        if !self.secondary.is_empty() {
            match intent {
                // Collapses to one caret by definition: there is one "all".
                TextIntent::SelectAll => {
                    self.secondary.clear();
                }
                // Refused here for the reasons the arms below give, and
                // refused identically with several carets — whoever holds the
                // geometry or the pasteboard is the one that can do it, and
                // both of them know about `carets`.
                TextIntent::MoveUp { .. }
                | TextIntent::MoveDown { .. }
                | TextIntent::MovePageUp { .. }
                | TextIntent::MovePageDown { .. }
                | TextIntent::MoveLineStart { .. }
                | TextIntent::MoveLineEnd { .. }
                | TextIntent::Copy
                | TextIntent::Cut
                | TextIntent::Paste => return false,
                _ => return self.apply_at_every_caret(intent),
            }
        }
        match intent {
            TextIntent::Insert(text) => self.insert(text),
            TextIntent::Newline => self.insert("\n"),
            TextIntent::DeleteBackward => self.delete_backward(),
            TextIntent::DeleteForward => self.delete_forward(),
            TextIntent::DeleteWordBackward => self.delete_word_backward(),
            TextIntent::DeleteWordForward => self.delete_word_forward(),
            TextIntent::MovePrevious { extend } => self.move_previous(*extend),
            TextIntent::MoveNext { extend } => self.move_next(*extend),
            TextIntent::MoveWordPrevious { extend } => self.move_word_previous(*extend),
            TextIntent::MoveWordNext { extend } => self.move_word_next(*extend),
            TextIntent::SelectAll => self.select_all(),
            TextIntent::MoveUp { .. }
            | TextIntent::MoveDown { .. }
            | TextIntent::MovePageUp { .. }
            | TextIntent::MovePageDown { .. }
            | TextIntent::MoveLineStart { .. }
            | TextIntent::MoveLineEnd { .. } => return false,
            // Refused for a second reason, and it is worth keeping distinct from
            // the one above. Those four need *geometry*; these three need a
            // *pasteboard*, which is a platform service and is exactly as absent
            // from this crate as fonts are (`docs/DESIGN.md` §16). Whoever holds
            // the service carries them out — see
            // [`TextIntent::needs_clipboard`] — and calls back in through
            // [`insert`](Self::insert) or [`delete_backward`](Self::delete_backward).
            TextIntent::Copy | TextIntent::Cut | TextIntent::Paste => return false,
        }
        true
    }

    // ------------------------------------------------------------- composition

    /// Set the range the input method is composing.
    ///
    /// Clamped to the text, so a bridge that reports a range against a stale
    /// string leaves the composition off rather than panicking a slice.
    pub fn set_composing(&mut self, range: Option<TextRange>) {
        self.composing = range.filter(|range| {
            range.end <= self.text.len()
                && self.text.is_char_boundary(range.start)
                && self.text.is_char_boundary(range.end)
        });
    }

    /// Accept the composing text as committed, leaving the caret after it.
    pub fn commit_composing(&mut self) {
        if let Some(range) = self.composing.take() {
            self.selection = TextSelection::collapsed(range.end);
        }
    }

    /// The bytes an input method is currently entitled to replace.
    ///
    /// The composing region if there is one, and the selection otherwise. This
    /// is the one rule that makes composition work: the *first* preedit replaces
    /// what the user had selected, and every one after it replaces the preedit
    /// before it. Getting it wrong appends each keystroke of `ka` instead of
    /// refining it, and Japanese input produces `kkakan` rather than `か`.
    #[must_use]
    pub fn composing_or_selection(&self) -> TextRange {
        self.composing.unwrap_or_else(|| self.selection.range())
    }

    /// Replace the composing region with in-progress text from an input method.
    ///
    /// `cursor` is where the IME wants the caret or selection *within* the
    /// preedit, in bytes from its start — that is how a candidate window
    /// highlights the clause being converted. `None` puts the caret at the end,
    /// which is what a platform that does not report one means.
    ///
    /// Empty text ends the composition without committing anything, which is
    /// what pressing escape in a candidate window does.
    pub fn set_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        // **An empty preedit with nothing composing is a no-op, not a delete.**
        //
        // "Ends the composition" is only meaningful when there is one. With
        // `composing` at `None` the range below is the *selection*, and
        // replacing it with an empty string deletes text the user selected and
        // never asked to lose.
        //
        // That is not a hypothetical: X11 and Wayland input methods send
        // `Preedit("")` to mean "there is no composition", and they send it on
        // a pointer press — the exact moment a selection has just been made. So
        // clicking or dragging in a field deleted everything between the old
        // caret and the pointer, with no keystroke involved and no selection
        // left behind to explain it.
        if text.is_empty() && self.composing.is_none() {
            return;
        }
        let target = self.composing_or_selection();
        // Clears `composing` as a side effect, which is why it is set below
        // rather than adjusted.
        self.replace_range(target, text);

        let start = self.clamp_to_boundary(target.start);
        if text.is_empty() {
            self.composing = None;
            self.selection = TextSelection::collapsed(start);
            return;
        }

        let range = TextRange {
            start,
            end: start + text.len(),
        };
        self.composing = Some(range);
        self.selection = match cursor {
            // Offsets arrive from outside and describe a string we have just
            // spliced in, so they are clamped against the whole document rather
            // than trusted — a bridge reporting them in chars would otherwise
            // panic a slice somewhere far from the cause.
            Some((from, to)) => TextSelection::new(
                self.clamp_to_boundary(start + from),
                self.clamp_to_boundary(start + to),
            ),
            None => TextSelection::collapsed(range.end),
        };
    }

    /// Replace the composing region with text the input method has committed.
    ///
    /// The end of a composition: the provisional text becomes real, the
    /// underline goes, and the caret lands after it. Also how a platform
    /// delivers ordinary typing when an input method is active but not
    /// composing — a commit with no preedit before it is simply an insert.
    pub fn commit(&mut self, text: &str) {
        // The same guard [`set_preedit`] carries, for the same reason: a bridge
        // that sends an empty commit means "nothing to commit", and with no
        // composition in flight the range it would replace is the user's
        // selection.
        if text.is_empty() && self.composing.is_none() {
            return;
        }
        let target = self.composing_or_selection();
        self.replace_range(target, text);
    }

    /// Carry out one input method event.
    ///
    /// Returns whether anything changed, so a caller can decide whether the
    /// frame is worth asking for.
    pub fn apply_ime(&mut self, event: &ImeEvent) -> bool {
        let before = self.clone();
        match event {
            // Nothing to do to the text. The platform is telling us a session
            // opened or closed; what matters is that a `Disabled` abandons any
            // half-finished composition rather than leaving it underlined
            // forever.
            ImeEvent::Enabled => return false,
            ImeEvent::Disabled => {
                if self.composing.is_none() {
                    return false;
                }
                // The provisional text stays — the user typed it and can see it
                // — but it is no longer anybody's to replace.
                self.composing = None;
            }
            ImeEvent::Preedit { text, cursor } => self.set_preedit(text, *cursor),
            ImeEvent::Commit(text) => self.commit(text),
        }
        *self != before
    }

    // --------------------------------------------------------------- boundaries

    /// The grapheme boundary before `offset`, or `None` at the start of the text.
    #[must_use]
    pub fn previous_boundary(&self, offset: usize) -> Option<usize> {
        let offset = self.clamp_to_boundary(offset);
        if offset == 0 {
            return None;
        }
        // Segmenting only the text before the caret is sound because `offset` is
        // already on a boundary, and a grapheme never spans one.
        self.text[..offset]
            .grapheme_indices(true)
            .next_back()
            .map(|(index, _)| index)
    }

    /// The grapheme boundary after `offset`, or `None` at the end of the text.
    #[must_use]
    pub fn next_boundary(&self, offset: usize) -> Option<usize> {
        let offset = self.clamp_to_boundary(offset);
        if offset >= self.text.len() {
            return None;
        }
        self.text[offset..]
            .graphemes(true)
            .next()
            .map(|grapheme| offset + grapheme.len())
    }

    /// `offset` moved to the nearest grapheme boundary at or before it, and
    /// clamped to the text.
    ///
    /// Every offset entering this type goes through here. An offset from a
    /// platform IME, from a hit test against a stale layout, or from an
    /// application is not trusted to be on a boundary, and `String::replace_range`
    /// panics on one that is not.
    #[must_use]
    pub fn clamp_to_boundary(&self, offset: usize) -> usize {
        if offset >= self.text.len() {
            return self.text.len();
        }
        let mut boundary = 0;
        for (index, _) in self.text.grapheme_indices(true) {
            if index > offset {
                break;
            }
            boundary = index;
        }
        boundary
    }

    /// The range of the word containing `offset`.
    fn word_range_at(&self, offset: usize) -> TextRange {
        let mut containing = TextRange::collapsed(offset);
        for (index, word) in self.text.split_word_bound_indices() {
            let range = TextRange {
                start: index,
                end: index + word.len(),
            };
            // A word *ending* exactly at the offset wins outright over the run
            // that starts there, so a tap just past "hello" selects "hello"
            // rather than the space after it. Word bounds arrive in order, so the
            // one that ends here has already been seen when the one that starts
            // here comes round.
            if range.end == offset && !word.trim().is_empty() {
                return range;
            }
            if range.contains(offset) {
                containing = range;
            }
        }
        containing
    }

    /// The start of the last word beginning before `offset`.
    fn word_boundary_before(&self, offset: usize) -> usize {
        let offset = self.clamp_to_boundary(offset);
        self.text
            .split_word_bound_indices()
            .filter(|(index, word)| *index < offset && !word.trim().is_empty())
            .map(|(index, _)| index)
            .next_back()
            .unwrap_or(0)
    }

    /// The end of the first word reaching past `offset`.
    fn word_boundary_after(&self, offset: usize) -> usize {
        let offset = self.clamp_to_boundary(offset);
        self.text
            .split_word_bound_indices()
            .map(|(index, word)| (index + word.len(), word))
            .find(|(end, word)| *end > offset && !word.trim().is_empty())
            .map_or(self.text.len(), |(end, _)| end)
    }
}

impl From<&str> for TextEditingValue {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for TextEditingValue {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `e` + U+0301 combining acute — two chars, one grapheme, one thing to delete.
    const E_ACUTE: &str = "e\u{0301}";
    /// A family emoji: four emoji joined by zero-width joiners.
    const FAMILY: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";

    fn value(text: &str) -> TextEditingValue {
        TextEditingValue::new(text)
    }

    // --------------------------------------------------------------------- ime

    /// **An input method resetting must not eat the selection.**
    ///
    /// X11 and Wayland input methods send `Preedit("")` to say "there is no
    /// composition" — routinely, and in particular on a pointer press, which is
    /// the moment a selection has just been made. With nothing composing, the
    /// range an empty preedit would replace is the *selection*, so applying it
    /// deleted whatever the user had just selected.
    ///
    /// What that looked like: click, or drag to select, and the document lost
    /// everything between the old caret and the click. No keystroke, no
    /// selection left behind, and the text ending exactly where the pointer
    /// went down. Invisible to every headless test, because nothing headless
    /// sends an IME event.
    #[test]
    fn an_empty_preedit_with_nothing_composing_leaves_the_selection_alone() {
        let mut value = value("the quick brown fox");
        value.selection = TextSelection::new(4, 19);

        let changed = value.apply_ime(&ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        });

        assert!(!changed, "an IME reset changes nothing");
        assert_eq!(value.text, "the quick brown fox", "the text is untouched");
        assert_eq!(
            value.selection,
            TextSelection::new(4, 19),
            "and so is the selection"
        );
    }

    /// The case the empty preedit is *for*: a composition in flight is cancelled
    /// — escape in a candidate window — and the provisional text goes.
    #[test]
    fn an_empty_preedit_still_cancels_a_composition_in_flight() {
        let mut value = value("ka");
        value.selection = TextSelection::collapsed(2);
        value.set_preedit("か", None);
        assert_eq!(value.text, "kaか");

        value.apply_ime(&ImeEvent::Preedit {
            text: String::new(),
            cursor: None,
        });

        assert_eq!(value.text, "ka", "the provisional text is withdrawn");
        assert_eq!(value.selection, TextSelection::collapsed(2));
    }

    /// The same guard for a commit: some bridges send an empty one where they
    /// mean "nothing to commit", and with no composition that would be a
    /// deletion of the selection rather than a no-op.
    #[test]
    fn an_empty_commit_with_nothing_composing_leaves_the_selection_alone() {
        let mut value = value("the quick brown fox");
        value.selection = TextSelection::new(4, 19);

        let changed = value.apply_ime(&ImeEvent::Commit(String::new()));

        assert!(!changed);
        assert_eq!(value.text, "the quick brown fox");
        assert_eq!(value.selection, TextSelection::new(4, 19));
    }

    // ------------------------------------------------------------------- ranges

    #[test]
    fn a_range_orders_its_ends_on_construction() {
        // An unordered range is not a different range, it is a panicking slice.
        assert_eq!(TextRange::new(7, 2), TextRange::new(2, 7));
        assert_eq!(TextRange::new(7, 2).start, 2);
    }

    #[test]
    fn intersecting_ranges_that_only_touch_do_not_overlap() {
        let line = TextRange::new(0, 5);
        assert_eq!(
            line.intersect(TextRange::new(3, 8)),
            Some(TextRange::new(3, 5))
        );
        assert_eq!(
            line.intersect(TextRange::new(5, 8)),
            None,
            "a selection ending where a line starts highlights nothing on it"
        );
    }

    // --------------------------------------------------------------- selections

    #[test]
    fn a_reversed_selection_keeps_its_anchor() {
        let selection = TextSelection::new(10, 4);
        assert_eq!(selection.start(), 4);
        assert_eq!(selection.end(), 10);
        assert!(selection.is_reversed());
        assert_eq!(
            selection.base, 10,
            "dragging backwards past the anchor must not move the anchor"
        );
    }

    #[test]
    fn extending_moves_only_the_caret() {
        let selection = TextSelection::collapsed(5).moved_to(9, true);
        assert_eq!((selection.base, selection.extent), (5, 9));

        let replaced = selection.moved_to(2, false);
        assert_eq!(
            (replaced.base, replaced.extent),
            (2, 2),
            "without extend a move is a new caret, not a longer selection"
        );
    }

    // ------------------------------------------------------------------ editing

    #[test]
    fn a_new_value_puts_the_caret_at_the_end() {
        let value = value("hello");
        assert_eq!(value.selection, TextSelection::collapsed(5));
        assert!(value.selection.is_collapsed());
    }

    #[test]
    fn typing_inserts_at_the_caret_and_carries_it_along() {
        let mut value = value("helo");
        value.selection = TextSelection::collapsed(3);
        value.insert("l");

        assert_eq!(value.text, "hello");
        assert_eq!(
            value.selection,
            TextSelection::collapsed(4),
            "the caret follows what was typed, it does not stay behind it"
        );
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut value = value("hello world");
        value.selection = TextSelection::new(6, 11);
        value.insert("there");

        assert_eq!(value.text, "hello there");
        assert_eq!(value.selection, TextSelection::collapsed(11));
    }

    #[test]
    fn backspace_deletes_a_whole_grapheme_not_a_char() {
        let mut value = value(&format!("caf{E_ACUTE}"));
        assert_eq!(
            value.text.chars().count(),
            5,
            "e and its accent are two chars"
        );

        value.delete_backward();
        assert_eq!(
            value.text, "caf",
            "removing one char would leave the accent orphaned on the f"
        );
    }

    #[test]
    fn backspace_deletes_a_whole_emoji_cluster() {
        let mut value = value(FAMILY);
        value.delete_backward();
        assert_eq!(value.text, "", "half a family is not a character");
    }

    #[test]
    fn backspace_at_the_start_of_the_text_does_nothing() {
        let mut value = value("hello");
        value.selection = TextSelection::collapsed(0);
        value.delete_backward();

        assert_eq!(value.text, "hello");
        assert_eq!(value.selection, TextSelection::collapsed(0));
    }

    #[test]
    fn backspace_with_a_selection_deletes_the_selection_rather_than_one_more() {
        let mut value = value("hello world");
        value.selection = TextSelection::new(5, 11);
        value.delete_backward();

        assert_eq!(
            value.text, "hello",
            "the selection is what is deleted; taking a grapheme as well eats the o"
        );
    }

    #[test]
    fn delete_forward_removes_the_grapheme_after_the_caret() {
        let mut value = value(&format!("{E_ACUTE}tat"));
        value.selection = TextSelection::collapsed(0);
        value.delete_forward();

        assert_eq!(value.text, "tat");
    }

    #[test]
    fn delete_forward_at_the_end_of_the_text_does_nothing() {
        let mut value = value("hello");
        value.delete_forward();
        assert_eq!(value.text, "hello");
    }

    // ------------------------------------------------------------------- moving

    #[test]
    fn moving_steps_over_a_grapheme_rather_than_into_it() {
        let mut value = value(&format!("a{E_ACUTE}b"));
        value.selection = TextSelection::collapsed(0);

        value.move_next(false);
        assert_eq!(value.selection.extent, 1, "past the a");
        value.move_next(false);
        assert_eq!(
            value.selection.extent, 4,
            "past both bytes of the accented e, never landing between them"
        );
    }

    #[test]
    fn moving_left_out_of_a_selection_goes_to_its_start() {
        let mut value = value("hello world");
        value.selection = TextSelection::new(2, 8);
        value.move_previous(false);

        assert_eq!(
            value.selection,
            TextSelection::collapsed(2),
            "not 7 — pressing left with a selection means go to its beginning"
        );
    }

    #[test]
    fn moving_right_out_of_a_reversed_selection_still_goes_to_its_end() {
        let mut value = value("hello world");
        value.selection = TextSelection::new(8, 2);
        value.move_next(false);

        assert_eq!(
            value.selection,
            TextSelection::collapsed(8),
            "the end is the higher offset, not the end that moved"
        );
    }

    #[test]
    fn extending_grows_the_selection_from_a_fixed_anchor() {
        let mut value = value("hello");
        value.selection = TextSelection::collapsed(2);

        value.move_next(true);
        value.move_next(true);
        assert_eq!(value.selection, TextSelection::new(2, 4));

        value.move_previous(true);
        value.move_previous(true);
        value.move_previous(true);
        assert_eq!(
            value.selection,
            TextSelection::new(2, 1),
            "crossing the anchor reverses the selection rather than moving the anchor"
        );
    }

    #[test]
    fn moving_at_either_end_of_the_text_stays_put() {
        let mut value = value("hi");
        value.selection = TextSelection::collapsed(0);
        value.move_previous(false);
        assert_eq!(value.selection.extent, 0);

        value.selection = TextSelection::collapsed(2);
        value.move_next(false);
        assert_eq!(value.selection.extent, 2);
    }

    // -------------------------------------------------------------------- words

    #[test]
    fn word_movement_skips_the_whitespace_between_words() {
        let mut value = value("hello brave world");
        value.selection = TextSelection::collapsed(17);

        value.move_word_previous(false);
        assert_eq!(&value.text[value.selection.extent..], "world");

        value.move_word_previous(false);
        assert_eq!(
            &value.text[value.selection.extent..],
            "brave world",
            "one press per word, not one press per gap"
        );
    }

    #[test]
    fn word_movement_forward_lands_after_the_word() {
        let mut value = value("hello brave world");
        value.selection = TextSelection::collapsed(0);

        value.move_word_next(false);
        assert_eq!(&value.text[..value.selection.extent], "hello");

        value.move_word_next(false);
        assert_eq!(&value.text[..value.selection.extent], "hello brave");
    }

    #[test]
    fn deleting_a_word_backward_takes_the_space_with_it() {
        let mut value = value("hello brave world");
        value.delete_word_backward();
        assert_eq!(value.text, "hello brave ");

        value.delete_word_backward();
        assert_eq!(
            value.text, "hello ",
            "a second press removes a word, not just the space left behind"
        );
    }

    #[test]
    fn deleting_a_word_forward_takes_the_word_after_the_caret() {
        let mut value = value("hello brave world");
        value.selection = TextSelection::collapsed(5);
        value.delete_word_forward();
        assert_eq!(value.text, "hello world");
    }

    #[test]
    fn double_tapping_a_word_selects_exactly_that_word() {
        let mut value = value("hello brave world");
        value.select_word_at(8);
        assert_eq!(value.selected_text(), "brave");
    }

    #[test]
    fn double_tapping_just_past_a_word_selects_the_word_not_the_space() {
        let mut value = value("hello brave world");
        value.select_word_at(5);
        assert_eq!(
            value.selected_text(),
            "hello",
            "a tap on the boundary belongs to the word that ends there"
        );
    }

    #[test]
    fn select_all_covers_the_whole_text() {
        let mut value = value("hello");
        value.select_all();
        assert_eq!(value.selected_text(), "hello");
        assert!(!value.selection.is_collapsed());
    }

    // -------------------------------------------------------------- composition

    #[test]
    fn editing_ends_any_composition() {
        let mut value = value("ka");
        value.set_composing(Some(TextRange::new(0, 2)));
        assert!(value.composing.is_some());

        value.insert("x");
        assert_eq!(
            value.composing, None,
            "the bytes under a composing range just moved; underlining them is wrong"
        );
    }

    #[test]
    fn a_composing_range_past_the_end_of_the_text_is_refused() {
        let mut value = value("ka");
        value.set_composing(Some(TextRange::new(0, 9)));
        assert_eq!(
            value.composing, None,
            "a bridge reporting against a stale string must not panic a slice"
        );
    }

    #[test]
    fn committing_a_composition_leaves_the_caret_after_it() {
        let mut value = value("abc");
        value.set_composing(Some(TextRange::new(1, 3)));
        value.commit_composing();

        assert_eq!(
            value.text, "abc",
            "committing changes nothing but the state"
        );
        assert_eq!(value.composing, None);
        assert_eq!(value.selection, TextSelection::collapsed(3));
    }

    // ---------------------------------------------------------------- clamping

    #[test]
    fn an_offset_inside_a_grapheme_is_pulled_back_to_its_start() {
        let value = value(&format!("a{E_ACUTE}b"));
        // Byte 2 is between the e and its combining accent — a real offset for a
        // caller working in chars, and a panic for `replace_range`.
        assert_eq!(value.clamp_to_boundary(2), 1);
        assert_eq!(value.clamp_to_boundary(999), value.text.len());
    }

    #[test]
    fn moving_to_an_offset_inside_a_grapheme_never_splits_it() {
        let mut value = value(&format!("a{E_ACUTE}b"));
        value.move_to(TextPosition::new(2), false);
        value.insert("!");

        assert_eq!(
            value.text,
            format!("a!{E_ACUTE}b"),
            "the insertion landed on a boundary rather than inside the cluster"
        );
    }

    #[test]
    fn an_affinity_survives_a_move_because_only_the_hit_test_knows_it() {
        let mut value = value("ab cd");
        value.move_to(TextPosition::upstream(3), false);
        assert_eq!(value.selection.affinity, Affinity::Upstream);
        assert_eq!(value.selection.cursor().offset, 3);
    }
}

/// The bullet a password field shows when nothing else is asked for.
///
/// U+2022, which is what every desktop platform draws. Not an asterisk: an
/// asterisk sits on the text baseline and reads as a footnote marker rather
/// than as a hidden character, and a row of them is visibly ragged.
pub const DEFAULT_OBSCURING_CHARACTER: char = '\u{2022}';

/// What a field shows instead of what it holds.
///
/// # Why this is a type and not a `bool` plus a `char`
///
/// Because obscuring is not a paint-time effect, and treating it as one is the
/// bug every first implementation of it has. The caret, the selection
/// highlights, the hit test and the arrow keys all resolve against a *shaped
/// paragraph*, so a field that shaped its real text and drew bullets over the
/// top would put the caret in the wrong place the moment the mask's advance
/// differed from the glyph's — which is always, for a proportional font.
///
/// So the paragraph is shaped from the masked text, and every offset crossing
/// that boundary is converted. [`TextPosition`](crate::TextPosition) is a **byte** offset and one
/// character of input is not one byte, so the conversion is real work rather
/// than a cast: `"é"` is two bytes and its mask is three.
///
/// When a field is not obscured both conversions are the identity and nothing
/// allocates — the plain path is not merely fast, it is the same arithmetic it
/// ran before this type existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Obscured {
    /// Shows what it holds.
    #[default]
    No,
    /// Shows one `char` for each character held.
    ///
    /// Per *character*, not per byte: a password of five letters shows five
    /// bullets whatever alphabet it is written in. Masking per byte is the
    /// shortcut that makes the offsets line up for free, and it leaks how many
    /// bytes each character took — a real disclosure for a non-Latin password,
    /// and visibly wrong besides.
    With(char),
}

impl Obscured {
    /// Whether anything is being hidden.
    #[must_use]
    pub const fn is_obscured(self) -> bool {
        matches!(self, Self::With(_))
    }

    /// The string to shape, which is the real text when nothing is hidden.
    ///
    /// Borrowed in the common case, so a field nobody is obscuring does not
    /// allocate a copy of its own contents on every layout.
    #[must_use]
    pub fn display<'a>(self, text: &'a str) -> std::borrow::Cow<'a, str> {
        match self {
            Self::No => std::borrow::Cow::Borrowed(text),
            Self::With(mask) => std::borrow::Cow::Owned(text.chars().map(|_| mask).collect()),
        }
    }

    /// A byte offset in the real text, as one in the displayed text.
    #[must_use]
    pub fn to_display(self, text: &str, offset: usize) -> usize {
        match self {
            Self::No => offset,
            Self::With(mask) => {
                // `get(..offset)` rather than slicing: an offset landing inside
                // a character would panic, and a caret that takes the process
                // down is a worse answer to an off-by-one than a caret that
                // clamps to the end.
                let before = text
                    .get(..offset)
                    .map_or_else(|| text.chars().count(), |head| head.chars().count());
                before * mask.len_utf8()
            }
        }
    }

    /// A byte offset in the displayed text, as one in the real text.
    #[must_use]
    pub fn from_display(self, text: &str, offset: usize) -> usize {
        match self {
            Self::No => offset,
            Self::With(mask) => text
                .char_indices()
                .nth(offset / mask.len_utf8())
                .map_or_else(|| text.len(), |(index, _)| index),
        }
    }

    /// A whole position, converted into the displayed text's coordinates.
    #[must_use]
    pub fn position_to_display(self, text: &str, position: TextPosition) -> TextPosition {
        TextPosition {
            offset: self.to_display(text, position.offset),
            affinity: position.affinity,
        }
    }

    /// A whole position, converted back out of them.
    #[must_use]
    pub fn position_from_display(self, text: &str, position: TextPosition) -> TextPosition {
        TextPosition {
            offset: self.from_display(text, position.offset),
            affinity: position.affinity,
        }
    }
}

#[cfg(test)]
mod obscured_tests {
    use super::*;

    #[test]
    fn nothing_hidden_is_the_identity_and_borrows() {
        // The claim in `Obscured`'s docs: a field nobody obscures runs exactly
        // the arithmetic it ran before this type existed, and does not allocate
        // a second copy of its own contents every layout.
        let plain = Obscured::No;
        assert!(matches!(
            plain.display("hello"),
            std::borrow::Cow::Borrowed("hello")
        ));
        for offset in 0..=5 {
            assert_eq!(plain.to_display("hello", offset), offset);
            assert_eq!(plain.from_display("hello", offset), offset);
        }
    }

    #[test]
    fn one_bullet_per_character_not_per_byte() {
        // **The gap this closes.** Masking per byte is the shortcut that makes
        // the offsets line up for free. `é` is two bytes, so it would show two
        // bullets for one keypress — which leaks how many bytes each character
        // took and is visibly wrong to anybody typing an accent.
        let masked = Obscured::With('•');
        assert_eq!(masked.display("héllo").chars().count(), 5);
        assert_eq!("héllo".chars().count(), 5);
        assert_eq!(masked.display("日本語").chars().count(), 3);
    }

    #[test]
    fn a_caret_after_a_multibyte_character_lands_where_the_bullet_is() {
        // The conversion earning its keep. In `héllo` the caret after `é` is at
        // byte 3; the second bullet ends at byte 6, because `•` is three bytes.
        // A cast, or a per-byte mask, puts the caret inside a bullet.
        let masked = Obscured::With('•');
        assert_eq!(masked.to_display("héllo", 3), 6);
        assert_eq!(masked.from_display("héllo", 6), 3);
    }

    #[test]
    fn every_character_boundary_survives_the_round_trip() {
        // The property, rather than three examples of it: converting out and
        // back is the identity at every position a caret can actually occupy.
        let masked = Obscured::With('•');
        for text in ["hello", "héllo", "日本語", "aé日z", ""] {
            for (offset, _) in text
                .char_indices()
                .chain(std::iter::once((text.len(), ' ')))
            {
                assert_eq!(
                    masked.from_display(text, masked.to_display(text, offset)),
                    offset,
                    "{text:?} at {offset}"
                );
            }
        }
    }

    #[test]
    fn an_offset_past_the_end_clamps_rather_than_panicking() {
        // A caret that takes the process down is a worse answer to an off-by-one
        // than a caret that clamps — see the comment on `to_display`.
        let masked = Obscured::With('•');
        assert_eq!(masked.to_display("hi", 99), 2 * '•'.len_utf8());
        assert_eq!(masked.from_display("hi", 99), 2);
    }

    #[test]
    fn an_offset_inside_a_character_does_not_panic() {
        // Byte 1 is inside `é`. `get(..1)` returns `None` and the fallback runs.
        let masked = Obscured::With('•');
        let _ = masked.to_display("é", 1);
    }

    #[test]
    fn the_default_bullet_is_the_one_desktops_draw() {
        assert_eq!(DEFAULT_OBSCURING_CHARACTER, '\u{2022}');
        assert_ne!(
            DEFAULT_OBSCURING_CHARACTER, '*',
            "an asterisk sits on the baseline and reads as a footnote marker"
        );
    }
    // ===================== multiple carets ===============================

    fn multi(text: &str, offsets: &[usize]) -> TextEditingValue {
        let mut value = TextEditingValue::new(text);
        value.selection = TextSelection::collapsed(offsets[0]);
        for &offset in &offsets[1..] {
            value.add_caret(TextSelection::collapsed(offset));
        }
        value
    }

    fn offsets(value: &TextEditingValue) -> Vec<usize> {
        value
            .sorted_carets()
            .iter()
            .map(TextSelection::start)
            .collect()
    }

    #[test]
    fn a_value_has_one_caret_until_something_adds_another() {
        let value = TextEditingValue::new("hello");
        assert_eq!(value.caret_count(), 1);
        assert!(!value.is_multi_caret());
        assert_eq!(value.carets(), vec![value.selection]);
    }

    #[test]
    fn typing_at_several_carets_inserts_at_each_of_them() {
        // Three carets at the start of three lines. The whole feature.
        let mut value = multi("one\ntwo\nthree", &[0, 4, 8]);
        assert!(value.apply(&TextIntent::Insert("- ".into())));
        assert_eq!(value.text, "- one\n- two\n- three");
        assert_eq!(
            offsets(&value),
            vec![2, 8, 14],
            "and each caret is after its own insertion"
        );
    }

    #[test]
    fn an_insertion_low_in_the_text_moves_the_carets_above_it() {
        // The bug this ordering exists to prevent: insert at 0 and the caret
        // at 8 is now at 10, not 8.
        let mut value = multi("aaaa\nbbbb", &[0, 5]);
        value.apply(&TextIntent::Insert("XY".into()));
        assert_eq!(value.text, "XYaaaa\nXYbbbb");
        assert_eq!(offsets(&value), vec![2, 9]);
    }

    #[test]
    fn deleting_backwards_at_several_carets_deletes_at_each() {
        let mut value = multi("ab\ncd\nef", &[2, 5, 8]);
        assert!(value.apply(&TextIntent::DeleteBackward));
        assert_eq!(value.text, "a\nc\ne");
        assert_eq!(offsets(&value), vec![1, 3, 5]);
    }

    #[test]
    fn two_carets_at_one_offset_are_one_caret() {
        // Otherwise typing once inserts twice, which is the single most
        // visible way a multi-caret implementation can be wrong.
        let mut value = TextEditingValue::new("hello");
        value.selection = TextSelection::collapsed(2);
        assert!(!value.add_caret(TextSelection::collapsed(2)));
        assert_eq!(value.caret_count(), 1);

        value.apply(&TextIntent::Insert("X".into()));
        assert_eq!(value.text, "heXllo");
    }

    #[test]
    fn overlapping_selections_merge_into_one() {
        let mut value = TextEditingValue::new("abcdefgh");
        value.selection = TextSelection::new(0, 4);
        value.add_caret(TextSelection::new(2, 6));
        assert_eq!(value.caret_count(), 1);
        assert_eq!(value.selection.range(), TextRange::new(0, 6));
        // Replacing it replaces the union once, not the overlap twice.
        value.apply(&TextIntent::Insert("_".into()));
        assert_eq!(value.text, "_gh");
    }

    #[test]
    fn touching_selections_merge_too() {
        let mut value = TextEditingValue::new("abcdef");
        value.selection = TextSelection::new(0, 3);
        value.add_caret(TextSelection::new(3, 5));
        assert_eq!(value.caret_count(), 1);
        assert_eq!(value.selection.range(), TextRange::new(0, 5));
    }

    #[test]
    fn the_primary_survives_a_merge() {
        // The IME composes against the primary, a drag extends it, and
        // `selected_text` returns it — so it has to still mean something after
        // carets collide.
        let mut value = TextEditingValue::new("abcdefgh");
        value.selection = TextSelection::new(4, 6);
        value.add_caret(TextSelection::new(0, 2));
        value.add_caret(TextSelection::new(5, 7));
        assert_eq!(value.caret_count(), 2);
        assert_eq!(
            value.selection.range(),
            TextRange::new(4, 7),
            "the primary is the survivor that covers where it was"
        );
    }

    #[test]
    fn moving_moves_every_caret() {
        let mut value = multi("one\ntwo\nthree", &[1, 5, 9]);
        assert!(value.apply(&TextIntent::MoveNext { extend: false }));
        assert_eq!(offsets(&value), vec![2, 6, 10]);
        assert_eq!(value.text, "one\ntwo\nthree", "and changes nothing");
    }

    #[test]
    fn carets_that_collide_while_moving_become_one() {
        // Two carets on one line both pressing Home is the collision every
        // editor has to handle, and the reason `normalise_carets` runs after
        // movement and not only after an edit.
        let mut value = multi("abc", &[0, 1]);
        value.apply(&TextIntent::MovePrevious { extend: false });
        assert_eq!(offsets(&value), vec![0], "both reached the start");
    }

    #[test]
    fn select_all_collapses_to_one_selection() {
        // There is one "all".
        let mut value = multi("one\ntwo", &[0, 4]);
        assert!(value.apply(&TextIntent::SelectAll));
        assert_eq!(value.caret_count(), 1);
        assert_eq!(value.selection.range(), TextRange::new(0, value.text.len()));
    }

    #[test]
    fn the_intents_that_need_geometry_or_a_pasteboard_are_still_refused() {
        // Refused identically with several carets: whoever holds the geometry
        // or the pasteboard is the one that can carry them out, and both know
        // about `carets`. A silent no-op here would look like a dead key.
        let mut value = multi("one\ntwo", &[0, 4]);
        for intent in [
            TextIntent::MoveUp { extend: false },
            TextIntent::MoveLineEnd { extend: false },
            TextIntent::Copy,
            TextIntent::Cut,
            TextIntent::Paste,
        ] {
            assert!(!value.apply(&intent), "{intent:?} should be refused");
        }
        assert_eq!(value.caret_count(), 2, "and none of them dropped a caret");
    }

    #[test]
    fn escape_leaves_the_primary_and_nothing_else() {
        let mut value = multi("one\ntwo\nthree", &[8, 0, 4]);
        assert_eq!(value.caret_count(), 3);
        assert!(value.clear_secondary_carets());
        assert_eq!(value.caret_count(), 1);
        assert_eq!(
            value.selection.start(),
            8,
            "the primary is where the last deliberate action put it"
        );
        assert!(!value.clear_secondary_carets(), "and again is a no-op");
    }

    #[test]
    fn a_single_caret_value_takes_the_path_it_always_did() {
        // The guarantee the whole design rests on: nothing that does not use
        // multiple carets can be broken by them.
        let mut multi_caret = TextEditingValue::new("hello");
        multi_caret.selection = TextSelection::collapsed(0);
        let mut plain = multi_caret.clone();

        multi_caret.add_caret(TextSelection::collapsed(0));
        assert!(!multi_caret.is_multi_caret(), "a duplicate added nothing");

        for intent in [
            TextIntent::Insert("x".into()),
            TextIntent::MoveNext { extend: true },
            TextIntent::DeleteBackward,
        ] {
            assert_eq!(multi_caret.apply(&intent), plain.apply(&intent));
            assert_eq!(multi_caret.text, plain.text);
            assert_eq!(multi_caret.selection, plain.selection);
        }
    }

    #[test]
    fn multibyte_text_survives_several_carets() {
        // The offsets are bytes and the moves are grapheme clusters, so a
        // shift computed in bytes and a move computed in clusters have to
        // agree — this is where they would not.
        let mut value = multi("é\né\né", &[0, 3, 6]);
        value.apply(&TextIntent::Insert("a".into()));
        assert_eq!(value.text, "aé\naé\naé");
        for caret in value.carets() {
            assert!(value.text.is_char_boundary(caret.start()));
        }
    }
}
