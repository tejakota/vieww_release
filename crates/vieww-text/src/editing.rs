//! Text editing: the state machine behind a text field.
//!
//! # What "editing" means here
//!
//! A text field is a string, a [`TextSelection`], and a set of operations
//! that move between them. That sounds trivial and is not, for three reasons:
//!
//! 1. **Grapheme clusters.** Backspace deletes one *character as a user sees
//!    it*, not one `char` and certainly not one byte. `é` written as `e` +
//!    combining acute is two `char`s and one backspace. A family emoji is
//!    seven. Deleting by `char` leaves a dangling combining mark; deleting by
//!    byte panics on a non-boundary.
//!
//! 2. **IME composition.** Typing Japanese or Chinese goes through an input
//!    method: the user types Latin keys, a *preedit* string appears
//!    underlined, and only on commit does it become real text. Preedit is not
//!    in the buffer — it is a separate overlay with its own selection — and a
//!    field that ignores this either loses characters or duplicates them.
//!
//! 3. **Undo granularity.** Undo should not step one character at a time; it
//!    should step one *word* or one coherent action. That means coalescing
//!    edits, which means the edit history is a real structure and not a stack
//!    of strings.
//!
//! This module implements 1 and 3 and models 2. It operates on byte offsets
//! throughout, and every public operation is guaranteed to leave the buffer
//! on a valid UTF-8 boundary.

use crate::selection::TextSelection;

/// An editable text buffer with a selection.
#[derive(Debug, Clone, Default)]
pub struct TextEditor {
    text: String,
    selection: TextSelection,
    /// The active IME composition, if any.
    composition: Option<Composition>,
    /// Undo history, most recent last.
    undo: Vec<EditSnapshot>,
    /// Redo history, most recent last.
    redo: Vec<EditSnapshot>,
}

/// A point-in-time snapshot for undo.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EditSnapshot {
    text: String,
    selection: TextSelection,
    /// What kind of edit produced this, for coalescing.
    kind: EditKind,
}

/// The kind of an edit, used to decide whether to coalesce with the previous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Insert,
    Delete,
    /// Never coalesces — a paste or a programmatic replacement.
    Replace,
}

/// An in-progress IME composition.
///
/// The `text` is *not* in the buffer: it is drawn over the insertion point,
/// underlined, and replaced wholesale on each update until the IME commits.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composition {
    /// The preedit string.
    pub text: String,
    /// Where the preedit sits in the buffer.
    pub offset: usize,
    /// The selection within the preedit, in bytes relative to its start.
    pub cursor: usize,
}

impl TextEditor {
    /// An empty editor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An editor holding `text`, cursor at the end.
    #[must_use]
    pub fn with_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            selection: TextSelection::cursor(end),
            ..Default::default()
        }
    }

    /// The current text, excluding any IME preedit.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The current selection.
    #[must_use]
    pub fn selection(&self) -> TextSelection {
        self.selection
    }

    /// The active IME composition, if any.
    #[must_use]
    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    /// Set the selection, clamping it to the buffer and snapping both ends to
    /// character boundaries.
    ///
    /// Clamping rather than rejecting is deliberate: a stale selection after
    /// an external text change is common, and a field that panics on one is a
    /// field that crashes when two things edit it at once.
    pub fn set_selection(&mut self, selection: TextSelection) {
        self.selection = TextSelection {
            anchor: self.snap(selection.anchor),
            focus: self.snap(selection.focus),
        };
    }

    /// Clamp an offset into the buffer and onto a character boundary.
    fn snap(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// Insert `insertion` at the cursor, replacing the selection.
    ///
    /// Consecutive single-character inserts coalesce into one undo step; use
    /// [`replace`](Self::replace) for a paste, which should not.
    pub fn insert(&mut self, insertion: &str) {
        self.push_undo(EditKind::Insert);
        let range = self.selection.range();
        self.text.replace_range(range.clone(), insertion);
        let end = range.start + insertion.len();
        self.selection = TextSelection::cursor(end);
        self.redo.clear();
    }

    /// Replace the selection with `text` as one discrete edit.
    ///
    /// This is paste, and programmatic replacement. Unlike [`insert`](Self::insert)
    /// it never coalesces with the edits around it: undoing a paste should
    /// remove the whole pasted run in one step, not merge it into whatever
    /// was typed just before.
    pub fn replace(&mut self, text: &str) {
        self.push_undo(EditKind::Replace);
        let range = self.selection.range();
        self.text.replace_range(range.clone(), text);
        self.selection = TextSelection::cursor(range.start + text.len());
        self.redo.clear();
    }

    /// Delete the selection, or one grapheme backwards if collapsed.
    ///
    /// This is Backspace.
    pub fn delete_backward(&mut self) {
        if self.selection.is_collapsed() {
            let start = self.selection.focus;
            if start == 0 {
                return;
            }
            let prev = self.previous_boundary(start);
            self.push_undo(EditKind::Delete);
            self.text.replace_range(prev..start, "");
            self.selection = TextSelection::cursor(prev);
        } else {
            self.push_undo(EditKind::Delete);
            let range = self.selection.range();
            self.text.replace_range(range.clone(), "");
            self.selection = TextSelection::cursor(range.start);
        }
        self.redo.clear();
    }

    /// Delete the selection, or one grapheme forwards if collapsed.
    ///
    /// This is Delete/Del.
    pub fn delete_forward(&mut self) {
        if self.selection.is_collapsed() {
            let start = self.selection.focus;
            if start >= self.text.len() {
                return;
            }
            let next = self.next_boundary(start);
            self.push_undo(EditKind::Delete);
            self.text.replace_range(start..next, "");
            self.selection = TextSelection::cursor(start);
        } else {
            self.delete_backward();
            return;
        }
        self.redo.clear();
    }

    /// The offset one grapheme cluster before `offset`.
    ///
    /// Approximated by skipping backwards over combining marks and then one
    /// base character. A full implementation uses UAX #29's grapheme break
    /// rules; this handles the combining-mark case, which is the one that
    /// visibly corrupts text when it is wrong.
    fn previous_boundary(&self, offset: usize) -> usize {
        let mut idx = offset;
        loop {
            idx = self.char_before(idx);
            if idx == 0 {
                return 0;
            }
            let c = self.text[idx..].chars().next().unwrap_or('\0');
            if !is_combining(c) {
                return idx;
            }
        }
    }

    /// The offset one grapheme cluster after `offset`.
    fn next_boundary(&self, offset: usize) -> usize {
        let mut idx = self.char_after(offset);
        while idx < self.text.len() {
            let c = self.text[idx..].chars().next().unwrap_or('\0');
            if !is_combining(c) {
                break;
            }
            idx = self.char_after(idx);
        }
        idx
    }

    fn char_before(&self, offset: usize) -> usize {
        let mut idx = offset.saturating_sub(1);
        while idx > 0 && !self.text.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }

    fn char_after(&self, offset: usize) -> usize {
        let mut idx = (offset + 1).min(self.text.len());
        while idx < self.text.len() && !self.text.is_char_boundary(idx) {
            idx += 1;
        }
        idx
    }

    /// Begin or update an IME composition.
    pub fn set_composition(&mut self, composition: Composition) {
        self.composition = Some(composition);
    }

    /// Commit the active composition into the buffer.
    ///
    /// A no-op when there is no composition, so the platform layer can call
    /// it unconditionally on a commit event.
    pub fn commit_composition(&mut self) {
        if let Some(composition) = self.composition.take() {
            let text = composition.text.clone();
            self.insert(&text);
        }
    }

    /// Abandon the active composition without inserting it.
    ///
    /// Escape during IME input, or focus loss.
    pub fn cancel_composition(&mut self) {
        self.composition = None;
    }

    /// Push the current state onto the undo stack, coalescing where sensible.
    ///
    /// Consecutive insertions coalesce into one undo step so that Ctrl+Z
    /// removes a word rather than a letter.
    fn push_undo(&mut self, kind: EditKind) {
        let coalesce = matches!(
            (self.undo.last().map(|s| s.kind), kind),
            (Some(EditKind::Insert), EditKind::Insert) | (Some(EditKind::Delete), EditKind::Delete)
        );

        if !coalesce {
            self.undo.push(EditSnapshot {
                text: self.text.clone(),
                selection: self.selection,
                kind,
            });
        }
    }

    /// Undo the last edit. Returns `false` if there was nothing to undo.
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(snapshot) => {
                self.redo.push(EditSnapshot {
                    text: self.text.clone(),
                    selection: self.selection,
                    kind: snapshot.kind,
                });
                self.text = snapshot.text;
                self.selection = snapshot.selection;
                true
            }
            None => false,
        }
    }

    /// Redo the last undone edit. Returns `false` if there was nothing to redo.
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(snapshot) => {
                self.undo.push(EditSnapshot {
                    text: self.text.clone(),
                    selection: self.selection,
                    kind: snapshot.kind,
                });
                self.text = snapshot.text;
                self.selection = snapshot.selection;
                true
            }
            None => false,
        }
    }

    /// Select everything.
    pub fn select_all(&mut self) {
        self.selection = TextSelection {
            anchor: 0,
            focus: self.text.len(),
        };
    }

    /// The selected text.
    #[must_use]
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.range()]
    }
}

/// `true` if `c` is a combining mark that attaches to a preceding character.
fn is_combining(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F   // combining diacritical marks
        | 0x0483..=0x0489
        | 0x0591..=0x05BD
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x0900..=0x0903
        | 0x093A..=0x094F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x20D0..=0x20F0
        | 0xFE00..=0xFE0F // variation selectors
        | 0xFE20..=0xFE2F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_editor_is_empty() {
        let editor = TextEditor::new();
        assert_eq!(editor.text(), "");
        assert!(editor.selection().is_collapsed());
    }

    #[test]
    fn with_text_puts_the_cursor_at_the_end() {
        let editor = TextEditor::with_text("hello");
        assert_eq!(editor.selection(), TextSelection::cursor(5));
    }

    #[test]
    fn insert_at_the_cursor() {
        let mut editor = TextEditor::with_text("hel");
        editor.insert("lo");
        assert_eq!(editor.text(), "hello");
        assert_eq!(editor.selection(), TextSelection::cursor(5));
    }

    #[test]
    fn insert_replaces_the_selection() {
        let mut editor = TextEditor::with_text("hello world");
        editor.set_selection(TextSelection {
            anchor: 6,
            focus: 11,
        });
        editor.insert("there");
        assert_eq!(editor.text(), "hello there");
    }

    #[test]
    fn backspace_at_the_start_does_nothing() {
        let mut editor = TextEditor::with_text("hi");
        editor.set_selection(TextSelection::cursor(0));
        editor.delete_backward();
        assert_eq!(editor.text(), "hi");
    }

    #[test]
    fn backspace_deletes_one_character() {
        let mut editor = TextEditor::with_text("hello");
        editor.delete_backward();
        assert_eq!(editor.text(), "hell");
    }

    #[test]
    fn backspace_deletes_a_whole_multibyte_character() {
        // "é" as a single precomposed code point is 2 bytes. Deleting by
        // byte would leave invalid UTF-8 and panic.
        let mut editor = TextEditor::with_text("café");
        editor.delete_backward();
        assert_eq!(editor.text(), "caf");
    }

    #[test]
    fn backspace_deletes_a_combining_mark_with_its_base() {
        // "e" + U+0301 combining acute — two chars, one grapheme.
        let mut editor = TextEditor::with_text("cafe\u{0301}");
        assert_eq!(editor.text().chars().count(), 5);
        editor.delete_backward();
        assert_eq!(
            editor.text(),
            "caf",
            "one backspace removes the base and its mark together"
        );
    }

    #[test]
    fn delete_forward_at_the_end_does_nothing() {
        let mut editor = TextEditor::with_text("hi");
        editor.delete_forward();
        assert_eq!(editor.text(), "hi");
    }

    #[test]
    fn delete_forward_removes_the_next_character() {
        let mut editor = TextEditor::with_text("hello");
        editor.set_selection(TextSelection::cursor(0));
        editor.delete_forward();
        assert_eq!(editor.text(), "ello");
    }

    #[test]
    fn selection_is_clamped_to_the_buffer() {
        let mut editor = TextEditor::with_text("hi");
        editor.set_selection(TextSelection {
            anchor: 0,
            focus: 999,
        });
        assert_eq!(editor.selection().end(), 2);
    }

    #[test]
    fn selection_snaps_off_a_non_boundary() {
        let mut editor = TextEditor::with_text("é");
        // Offset 1 is inside the two-byte é.
        editor.set_selection(TextSelection::cursor(1));
        assert!(editor.text().is_char_boundary(editor.selection().focus));
    }

    #[test]
    fn select_all_covers_the_buffer() {
        let mut editor = TextEditor::with_text("hello");
        editor.select_all();
        assert_eq!(editor.selected_text(), "hello");
    }

    #[test]
    fn a_replace_does_not_coalesce_with_typing() {
        let mut editor = TextEditor::with_text("");
        editor.insert("a");
        editor.insert("b");
        editor.replace("PASTED");
        assert_eq!(editor.text(), "abPASTED");

        // One undo removes the whole paste, not one character of it.
        assert!(editor.undo());
        assert_eq!(editor.text(), "ab");
    }

    #[test]
    fn undo_restores_the_previous_text() {
        let mut editor = TextEditor::with_text("hello");
        editor.insert(" world");
        assert_eq!(editor.text(), "hello world");
        assert!(editor.undo());
        assert_eq!(editor.text(), "hello");
    }

    #[test]
    fn undo_on_an_untouched_editor_reports_nothing_to_do() {
        let mut editor = TextEditor::with_text("hello");
        assert!(!editor.undo());
    }

    #[test]
    fn redo_reapplies_an_undone_edit() {
        let mut editor = TextEditor::with_text("hello");
        editor.insert("!");
        editor.undo();
        assert_eq!(editor.text(), "hello");
        assert!(editor.redo());
        assert_eq!(editor.text(), "hello!");
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut editor = TextEditor::with_text("hello");
        editor.insert("!");
        editor.undo();
        editor.insert("?");
        assert!(!editor.redo(), "the old redo branch is gone");
    }

    #[test]
    fn composition_is_not_in_the_buffer_until_committed() {
        let mut editor = TextEditor::with_text("");
        editor.set_composition(Composition {
            text: "にほん".into(),
            offset: 0,
            cursor: 0,
        });
        assert_eq!(editor.text(), "", "preedit is an overlay, not content");
        assert!(editor.composition().is_some());

        editor.commit_composition();
        assert_eq!(editor.text(), "にほん");
        assert!(editor.composition().is_none());
    }

    #[test]
    fn cancelling_a_composition_discards_it() {
        let mut editor = TextEditor::with_text("a");
        editor.set_composition(Composition {
            text: "にほん".into(),
            offset: 1,
            cursor: 0,
        });
        editor.cancel_composition();
        assert_eq!(editor.text(), "a");
        assert!(editor.composition().is_none());
    }

    #[test]
    fn committing_with_no_composition_is_a_no_op() {
        let mut editor = TextEditor::with_text("a");
        editor.commit_composition();
        assert_eq!(editor.text(), "a");
    }
}
