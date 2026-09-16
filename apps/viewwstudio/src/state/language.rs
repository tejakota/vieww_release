//! Completion.
//!
//! The one seam between the editor and the language server.
//!
//! # Why this is a separate file and not a separate type
//!
//! Because it is the same object. `Studio` holds the application's signals in
//! one place deliberately — that is what lets any widget read any of them
//! without a chain of props threaded through the tree, and it is why no widget
//! in the studio owns state of its own. Splitting the *type* would mean
//! inventing ownership boundaries the interface does not have.
//!
//! What was worth splitting is the **code**. An inherent `impl` may be written
//! in as many blocks as it has subjects; `state.rs` had already marked those
//! subjects with comment rules, and this file is one of those rules made
//! structural. Nothing moved between types and no signature changed.
//!
//! # `pub(super)` on the helpers
//!
//! The private helpers below were private *to a file* when there was one file,
//! and several are called from what are now sibling modules. `pub(super)` is
//! that same reachability written down: visible throughout `state` and nowhere
//! else. Nothing here became public, and the crate's outside surface is byte for
//! byte what it was.

use super::*;

impl Studio {
    // ----- Completion ----------------------------------------------------

    /// The identifier the caret is inside, as a byte range.
    ///
    /// Empty when the caret is not in one, which is what makes "complete here"
    /// mean "offer everything" rather than "offer nothing".
    #[must_use]
    pub fn word_at_caret(text: &str, caret: usize) -> std::ops::Range<usize> {
        let caret = caret.min(text.len());
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let start = text[..caret]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map_or(caret, |(at, _)| at);
        let end = text[caret..]
            .char_indices()
            .take_while(|(_, c)| is_word(*c))
            .last()
            .map_or(caret, |(at, c)| caret + at + c.len_utf8());
        start..end
    }

    /// Ask the language server what could go here.
    pub fn request_completion(&self) {
        let Some(buffer) = self.active() else { return };
        let Some(path) = buffer.path.clone() else {
            self.notify("Completion needs a file on disk — save this buffer first");
            return;
        };
        let mut analyzer = self.analyzer.borrow_mut();
        let Some(client) = analyzer.as_mut() else {
            self.notify("rust-analyzer is not running — start it from the palette");
            return;
        };
        let caret = buffer.value.selection.cursor().offset;
        match client.request_completion(&path, &buffer.value.text, caret) {
            Ok(id) => self.completion_request.set(Some(id)),
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Ask what is under the caret.
    pub fn request_hover(&self) {
        let Some(buffer) = self.active() else { return };
        let Some(path) = buffer.path.clone() else {
            self.notify("Hover needs a file on disk — save this buffer first");
            return;
        };
        let mut analyzer = self.analyzer.borrow_mut();
        let Some(client) = analyzer.as_mut() else {
            self.notify("rust-analyzer is not running — start it from the palette");
            return;
        };
        let caret = buffer.value.selection.cursor().offset;
        if let Err(error) = client.request_hover(&path, &buffer.value.text, caret) {
            self.notify(&error.to_string());
        }
    }

    /// Ask where the symbol under the caret is defined, and jump there.
    ///
    /// # The most-used IDE feature the studio did not have
    ///
    /// Goto-definition and find-references are what a developer navigating an
    /// unfamiliar codebase reaches for dozens of times an hour. The studio had
    /// neither, and its symbol search was a regex scan for lines starting with
    /// `fn`. The transport has been open since diagnostics landed — what was
    /// missing was two message shapes.
    pub fn goto_definition(&self) {
        self.ask_for_places(true);
    }

    /// Ask where the symbol under the caret is used, and list the answers.
    pub fn find_references(&self) {
        self.ask_for_places(false);
    }

    pub(super) fn ask_for_places(&self, definition: bool) {
        let Some(buffer) = self.active() else { return };
        let Some(path) = buffer.path.clone() else {
            self.notify("This needs a file on disk — save this buffer first");
            return;
        };
        let mut analyzer = self.analyzer.borrow_mut();
        let Some(client) = analyzer.as_mut() else {
            self.notify("rust-analyzer is not running — start it from the palette");
            return;
        };
        let caret = buffer.value.selection.cursor().offset;
        let asked = if definition {
            client.request_definition(&path, &buffer.value.text, caret)
        } else {
            client.request_references(&path, &buffer.value.text, caret)
        };
        match asked {
            Ok(id) => {
                // Which question was asked is remembered here rather than
                // decided from the reply, because the two answers have the same
                // shape on the wire — see `lsp::Event::Locations`.
                self.jump_request.set(Some((id, definition)));
            }
            Err(error) => self.notify(&error.to_string()),
        }
    }

    /// Take a definition or references reply.
    ///
    /// One place, and it was a goto: jump to it. Several, or a references
    /// request: list them in the Output panel with their file and line, which
    /// is where every other list of positions in this studio goes.
    pub(super) fn accept_locations(&self, id: u32, places: &[crate::lsp::Place]) {
        let Some((asked, was_definition)) = self.jump_request.get() else {
            return;
        };
        if asked != id {
            return;
        }
        self.jump_request.set(None);

        if places.is_empty() {
            self.notify(if was_definition {
                "rust-analyzer could not find a definition for that"
            } else {
                "No references to that"
            });
            return;
        }

        if was_definition {
            let first = &places[0];
            self.open_path(first.path.clone());
            // LSP counts from zero and `jump_to` counts from one, which is the
            // studio's convention everywhere a line number is shown.
            self.jump_to(first.line + 1, first.character + 1);
            if places.len() > 1 {
                self.notify(&format!(
                    "{} definitions — jumped to the first",
                    places.len()
                ));
            }
            return;
        }

        let mut lines = vec![format!("{} references:", places.len())];
        lines.extend(places.iter().map(|place| {
            let name = place.path.file_name().map_or_else(
                || place.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            format!("  {name}:{}:{}", place.line + 1, place.character + 1)
        }));
        self.append_output(lines);
        self.panel_open.set(true);
        self.panel_tab.set(PanelTab::Output);
    }

    /// Take a completion reply, if it is the one being waited for.
    pub(super) fn accept_completions(&self, id: u32, items: &[crate::lsp::Completion]) {
        if self.completion_request.get() != Some(id) {
            // A reply to a request the user has already typed past. Dropped
            // rather than shown: a list for a prefix they moved on from looks
            // exactly like a list that is wrong.
            return;
        }
        self.completion_request.set(None);
        let Some(buffer) = self.active() else { return };
        let caret = buffer.value.selection.cursor().offset;
        let replacing = Self::word_at_caret(&buffer.value.text, caret);
        let prefix = buffer.value.text[replacing.clone()].to_owned();

        // Filtered here as well as by the server, because the server answered
        // the prefix as it was when the request went out and the user has been
        // typing since.
        let items: Vec<crate::lsp::Completion> = items
            .iter()
            .filter(|item| {
                prefix.is_empty()
                    || item
                        .label
                        .to_lowercase()
                        .starts_with(&prefix.to_lowercase())
            })
            .take(200)
            .cloned()
            .collect();

        if items.is_empty() {
            self.completion.set(None);
            return;
        }
        self.completion.set(Some(Rc::new(Completions {
            items,
            index: 0,
            replacing,
        })));
    }

    /// Move the highlight in the completion list, wrapping.
    pub fn move_completion(&self, delta: isize) {
        let Some(open) = self.completion.get() else {
            return;
        };
        let count = open.items.len();
        if count == 0 {
            return;
        }
        // `usize::try_from` rather than a cast: the list is capped at 200
        // entries so the conversion cannot fail, and saying that with a
        // `try_from` costs nothing while a cast would need three lint
        // expectations to explain the same thing.
        let count = isize::try_from(count).unwrap_or(isize::MAX);
        let index =
            usize::try_from((isize::try_from(open.index).unwrap_or(0) + delta).rem_euclid(count))
                .unwrap_or(0);
        self.completion.set(Some(Rc::new(Completions {
            index,
            ..(*open).clone()
        })));
    }

    /// Open a completion list directly, for tests.
    ///
    /// The real path goes through a language server, which a test cannot have.
    /// This is the same state the reply would produce, so everything *after*
    /// the reply — moving, accepting, clamping, closing — is testable without
    /// one. Named for what it is rather than hidden behind `#[cfg(test)]`,
    /// because a `cfg(test)` method is invisible to the integration tests that
    /// need it most.
    pub fn open_completion_for_test(
        &self,
        items: Vec<crate::lsp::Completion>,
        replacing: std::ops::Range<usize>,
    ) {
        self.completion.set(Some(Rc::new(Completions {
            items,
            index: 0,
            replacing,
        })));
    }

    /// Move the highlight to a specific row, for a click.
    pub fn move_completion_to(&self, index: usize) {
        let Some(open) = self.completion.get() else {
            return;
        };
        if index >= open.items.len() {
            return;
        }
        self.completion.set(Some(Rc::new(Completions {
            index,
            ..(*open).clone()
        })));
    }

    /// Insert the highlighted completion.
    pub fn accept_completion(&self) {
        let Some(open) = self.completion.get() else {
            return;
        };
        let Some(item) = open.selected().cloned() else {
            return;
        };
        self.completion.set(None);
        let Some(buffer) = self.active() else { return };

        let text = &buffer.value.text;
        // Re-clamped: the range was recorded when the list opened and the
        // buffer can have shrunk since — a reload, an undo, a replace-all.
        let start = open.replacing.start.min(text.len());
        let end = open.replacing.end.min(text.len()).max(start);
        if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
            return;
        }

        let mut edited = text.clone();
        edited.replace_range(start..end, &item.insert);
        let caret = start + item.insert.len();
        let mut value = vieww_foundation::TextEditingValue::new(edited);
        value.selection = vieww_foundation::TextSelection::collapsed(caret);
        self.edit(value);
    }

    /// Shut the completion list.
    pub fn close_completion(&self) {
        self.completion.set(None);
        self.completion_request.set(None);
    }
}
