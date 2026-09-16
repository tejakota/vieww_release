//! Undo and redo, per buffer.
//!
//! # Why the history lives on the buffer rather than on the editor
//!
//! An undo stack owned by the editor widget is an undo stack that belongs to
//! whichever file happens to be showing. Switch tabs, type, switch back, press
//! undo, and the keystrokes that come off are somebody else's. The stack is a
//! property of the *text*, so it sits beside the text.
//!
//! # What one undo takes back
//!
//! Not one keystroke. A history that steps character by character makes undo
//! useless for its actual job, which is "take back the thing I just did".
//! [`History::record`] therefore **coalesces**, and does it structurally rather
//! than on a timer:
//!
//! - Typing runs of word characters merge into one entry.
//! - Whitespace, a newline, or any punctuation **closes** the run, so undo lands
//!   on word and line boundaries — the granularity every editor has trained
//!   people to expect.
//! - A deletion never merges into an insertion, or the other way round.
//! - A caret that jumped somewhere else closes the run, because two edits in two
//!   places are two edits however fast they were made.
//!
//! Structural rather than timed for a reason beyond taste: a timer makes the
//! result depend on how fast the test typed, and a coalescing rule that cannot
//! be tested twice with the same answer is a rule nobody can rely on.
//!
//! # The caret is part of the entry
//!
//! Each entry holds a whole [`TextEditingValue`], not just the text, so undo
//! puts the caret back where the edit happened. An undo that restores the text
//! and leaves the caret at the end of the file makes the user hunt for what
//! just changed.

use vieww_foundation::TextEditingValue;

/// How many edits back a buffer can go.
///
/// A cap rather than unbounded, because the entries hold whole copies of the
/// text: a thousand of them on a large file is real memory for a session that
/// is never going to walk back that far. Two hundred is far past any undo run a
/// person performs by hand and small enough to not think about.
pub const DEPTH: usize = 200;

/// What kind of change an entry closed over, for coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Text got longer.
    Insert,
    /// Text got shorter.
    Delete,
    /// The very first state, or something that is neither.
    Anchor,
}

/// One buffer's undo and redo stacks.
///
/// `past` holds states the buffer has *been in*, oldest first, including the
/// one it is in now — which is why [`undo`](History::undo) can return the one
/// before it without the caller tracking a separate "current".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    past: Vec<TextEditingValue>,
    future: Vec<TextEditingValue>,
    /// The shape of the edit that produced the newest entry in `past`, and
    /// whether that entry is still open to being merged into.
    open: Option<Shape>,
}

impl History {
    /// A history whose only state is `value`.
    #[must_use]
    pub fn new(value: TextEditingValue) -> Self {
        Self {
            past: vec![value],
            future: Vec::new(),
            open: None,
        }
    }

    /// Note that the buffer moved from its current state to `next`.
    ///
    /// Call this **before** the buffer's own value is replaced, with the value
    /// it is about to hold. Text that did not change is not a history event at
    /// all — a click that only moves the caret records nothing, so undo does
    /// not spend a press putting a cursor back.
    pub fn record(&mut self, next: &TextEditingValue) {
        let Some(current) = self.past.last() else {
            self.past.push(next.clone());
            return;
        };
        if current.text == next.text {
            // The caret moved and nothing else. Keep the newest entry's caret
            // in step so an undo from *here* returns to where the user is,
            // but do not spend an entry on it.
            if let Some(last) = self.past.last_mut() {
                last.selection = next.selection;
                // **The extra carets too.** Adding a cursor
                // (`AddCursorBelow`, `AddCursorAtNextOccurrence`) changes no
                // text, so it lands in this branch — and only the primary
                // selection was copied across, leaving the entry holding the
                // carets from before the user added any. An undo then restored
                // the text correctly and dropped every secondary caret, which
                // is the middle of a multi-cursor edit turning into a
                // single-caret one with no way back.
                last.secondary.clone_from(&next.secondary);
            }
            return;
        }

        // A new edit invalidates anything that was undone: the timeline forked.
        self.future.clear();

        let shape = if next.text.len() > current.text.len() {
            Shape::Insert
        } else {
            Shape::Delete
        };

        if self.open == Some(shape) && continues(current, next, shape) {
            // Merge: replace the newest entry rather than pushing beside it.
            let index = self.past.len() - 1;
            self.past[index] = next.clone();
        } else {
            self.past.push(next.clone());
            if self.past.len() > DEPTH {
                self.past.remove(0);
            }
        }

        // A run stays open only while it is still inside a word. The moment it
        // reaches whitespace or punctuation the next edit starts a new entry.
        self.open = if closes(next, shape) {
            None
        } else {
            Some(shape)
        };
    }

    /// Step back one edit, if there is one.
    ///
    /// Returns the value the buffer should now hold. `None` means the history
    /// is at its oldest state and the caller should do nothing — importantly,
    /// *not* clear the buffer.
    pub fn undo(&mut self) -> Option<TextEditingValue> {
        if self.past.len() < 2 {
            return None;
        }
        let current = self.past.pop()?;
        self.future.push(current);
        // A run is never left open across an undo: typing after an undo must
        // not merge into the entry the undo landed on.
        self.open = None;
        self.past.last().cloned()
    }

    /// Step forward one edit, if one was undone.
    pub fn redo(&mut self) -> Option<TextEditingValue> {
        let next = self.future.pop()?;
        self.past.push(next.clone());
        self.open = None;
        Some(next)
    }

    /// Whether [`undo`](History::undo) would do anything. What greys the menu
    /// item out.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.past.len() >= 2
    }

    /// Whether [`redo`](History::redo) would do anything.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Forget everything before the current state.
    ///
    /// What a reload from disk does: the text on screen is no longer a state
    /// this buffer was ever edited into, so the entries behind it describe a
    /// different document.
    pub fn reset(&mut self, value: TextEditingValue) {
        self.past = vec![value];
        self.future.clear();
        self.open = None;
    }
}

/// Whether `next` continues `current` as one uninterrupted run.
///
/// The test is positional: an insertion continues only if it happened at the
/// caret the previous edit left, and a deletion only if it ate backwards from
/// there. Anything else is an edit somewhere else in the file.
fn continues(current: &TextEditingValue, next: &TextEditingValue, shape: Shape) -> bool {
    match shape {
        Shape::Insert => {
            next.selection.extent > current.selection.extent
                && next.selection.extent - current.selection.extent
                    == next.text.len() - current.text.len()
        }
        Shape::Delete => {
            current.selection.extent > next.selection.extent
                && current.selection.extent - next.selection.extent
                    == current.text.len() - next.text.len()
        }
        Shape::Anchor => false,
    }
}

/// Whether the character the run just reached ends it.
///
/// Looks at the character immediately behind the caret: the one that was just
/// typed, or the one now exposed by a backspace. Whitespace and punctuation
/// close the run so undo stops at word and line boundaries.
fn closes(next: &TextEditingValue, shape: Shape) -> bool {
    if shape == Shape::Anchor {
        return true;
    }
    let at = next.selection.extent.min(next.text.len());
    let Some(character) = next.text[..floor_boundary(&next.text, at)]
        .chars()
        .next_back()
    else {
        // Start of the buffer: nothing behind the caret to be inside a word of.
        return true;
    };
    !(character.is_alphanumeric() || character == '_')
}

/// The largest char boundary at or below `index`. Same reason `buffer.rs` has
/// one: a slice at a mid-character offset panics, and an offset arriving from a
/// hit test can be anywhere.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::TextSelection;

    /// A value holding `text` with the caret at `at`.
    fn value(text: &str, at: usize) -> TextEditingValue {
        let mut value = TextEditingValue::new(text);
        value.selection = TextSelection::collapsed(at);
        value
    }

    /// Type `text` one character at a time from an empty buffer.
    fn typed(text: &str) -> History {
        let mut history = History::new(value("", 0));
        let mut so_far = String::new();
        for character in text.chars() {
            so_far.push(character);
            history.record(&value(&so_far, so_far.len()));
        }
        history
    }

    #[test]
    fn a_typed_word_is_one_undo() {
        let mut history = typed("hello");
        assert_eq!(
            history.undo().map(|v| v.text),
            Some(String::new()),
            "five keystrokes, one undo — not five"
        );
        assert!(!history.can_undo());
    }

    #[test]
    fn whitespace_closes_the_run() {
        let mut history = typed("one two");
        assert_eq!(
            history.undo().map(|v| v.text),
            Some("one ".to_string()),
            "undo lands on the word boundary"
        );
        assert_eq!(history.undo().map(|v| v.text), Some(String::new()));
    }

    #[test]
    fn undo_restores_the_caret_as_well_as_the_text() {
        let mut history = History::new(value("abc", 3));
        history.record(&value("abcdef", 6));
        let back = history.undo().expect("one step back");
        assert_eq!(back.text, "abc");
        assert_eq!(
            back.selection.extent, 3,
            "the caret goes back to where the edit was, not to the end of the file"
        );
    }

    #[test]
    fn a_deletion_does_not_merge_into_an_insertion() {
        let mut history = History::new(value("", 0));
        history.record(&value("ab", 2));
        history.record(&value("a", 1));
        assert_eq!(history.undo().map(|v| v.text), Some("ab".to_string()));
        assert_eq!(history.undo().map(|v| v.text), Some(String::new()));
    }

    #[test]
    fn an_edit_somewhere_else_starts_its_own_entry() {
        let mut history = History::new(value("hello world", 5));
        // Typing at the end of "hello"…
        history.record(&value("helloX world", 6));
        // …then jumping to the front and typing there.
        history.record(&value("YhelloX world", 1));
        assert_eq!(
            history.undo().map(|v| v.text),
            Some("helloX world".to_string()),
            "two places is two edits, however fast"
        );
    }

    #[test]
    fn moving_the_caret_alone_is_not_an_edit() {
        let mut history = History::new(value("abc", 0));
        history.record(&value("abc", 2));
        assert!(
            !history.can_undo(),
            "a click that only moves the cursor must not consume an undo press"
        );
    }

    #[test]
    fn redo_replays_what_undo_took_back_and_a_new_edit_forks_the_timeline() {
        let mut history = typed("hi");
        history.undo();
        assert!(history.can_redo());
        assert_eq!(history.redo().map(|v| v.text), Some("hi".to_string()));

        history.undo();
        history.record(&value("yo", 2));
        assert!(
            !history.can_redo(),
            "editing after an undo discards the branch that was undone"
        );
    }

    #[test]
    fn typing_after_an_undo_does_not_merge_into_the_entry_it_landed_on() {
        let mut history = typed("abc def");
        // Back over " def", landing on "abc".
        assert_eq!(history.undo().map(|v| v.text), Some("abc ".to_string()));
        history.record(&value("abc Z", 5));
        assert_eq!(
            history.undo().map(|v| v.text),
            Some("abc ".to_string()),
            "the post-undo keystroke is its own entry, not a merge into the one \
             the undo landed on"
        );
    }

    #[test]
    fn the_stack_is_capped() {
        let mut history = History::new(value("", 0));
        // Each of these is its own entry: a space closes every run.
        for index in 0..DEPTH + 50 {
            let text = format!("{} ", "x ".repeat(index));
            history.record(&value(&text, text.len()));
        }
        assert!(
            history.past.len() <= DEPTH,
            "an unbounded history holds every version of the file in memory"
        );
        assert!(history.can_undo(), "and still undoes");
    }

    #[test]
    fn a_reset_forgets_the_document_that_came_before() {
        let mut history = typed("hello");
        history.reset(value("from disk", 9));
        assert!(
            !history.can_undo(),
            "the entries behind a reload describe a different document"
        );
    }

    #[test]
    fn a_multibyte_edit_does_not_panic_on_a_mid_character_offset() {
        let mut history = History::new(value("héllo", 0));
        // 2 is inside the two-byte 'é'.
        history.record(&value("héllo!", 2));
        assert!(history.can_undo());
    }

    #[test]
    fn undo_puts_every_caret_back_not_only_the_first() {
        // Adding a cursor changes no text, so it lands in `record`'s
        // caret-moved-only branch — which used to copy the primary selection
        // and drop the rest, so an undo mid-multi-cursor-edit left the user
        // with one caret where they had three.
        let mut history = History::new(value("one\ntwo\nthree", 0));

        let mut with_cursors = value("one\ntwo\nthree", 0);
        with_cursors.secondary = vec![TextSelection::collapsed(4), TextSelection::collapsed(8)];
        history.record(&with_cursors);

        let mut edited = with_cursors.clone();
        edited.text = "Xone\nXtwo\nXthree".to_owned();
        edited.selection = TextSelection::collapsed(1);
        edited.secondary = vec![TextSelection::collapsed(6), TextSelection::collapsed(11)];
        history.record(&edited);

        let back = history.undo().expect("an undo");
        assert_eq!(back.text, "one\ntwo\nthree");
        assert_eq!(
            back.secondary,
            vec![TextSelection::collapsed(4), TextSelection::collapsed(8)],
            "the carets the edit was made with come back with it"
        );
    }

    #[test]
    fn redo_restores_the_carets_the_edit_was_made_with() {
        let mut history = History::new(value("a\nb", 0));
        let mut edited = value("Xa\nXb", 1);
        edited.secondary = vec![TextSelection::collapsed(4)];
        history.record(&edited);
        history.undo().expect("an undo");
        let forward = history.redo().expect("a redo");
        assert_eq!(forward.secondary, vec![TextSelection::collapsed(4)]);
    }
}
