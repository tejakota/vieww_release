//! Carets, folding and the edits an editor makes for you.
//!
//! Everything that changes the text in a buffer without the user typing it
//! character by character.
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
    // ----- N6: multiple carets ----------------------------------------------

    /// Put another caret one line above or below the outermost one.
    ///
    /// # Which caret it grows from
    ///
    /// The *outermost* in the direction of travel — the lowest when going
    /// down, the highest when going up — so pressing the key five times makes
    /// five carets in a column rather than two carets fighting over one line.
    ///
    /// The column is the one the caret it grows from is in, measured in bytes
    /// on that line and clamped to the new line's length. Bytes rather than a
    /// shaped x: a column here is a position in the *text*, the pane is not
    /// wrapping (see `ui::editor`), and asking the render object would mean
    /// asking for geometry that is one frame old to answer a question about
    /// where a character is.
    pub fn add_caret_line(&self, down: bool) {
        let Some(buffer) = self.active() else {
            return;
        };
        let text = buffer.value.text.clone();
        let carets = buffer.value.sorted_carets();
        let from = if down {
            carets.last().copied()
        } else {
            carets.first().copied()
        };
        let Some(from) = from else {
            return;
        };

        let offset = from.cursor().offset.min(text.len());
        let line = line_of(&text, offset);
        let line_start: usize = text.split('\n').take(line).map(|l| l.len() + 1).sum();
        let column = offset - line_start;

        let lines: Vec<&str> = text.split('\n').collect();
        let target = if down {
            line + 1
        } else {
            line.checked_sub(1).unwrap_or(line)
        };
        if down && target >= lines.len() {
            // At the last line already. Nothing to add, and adding a caret at
            // the same place would silently do nothing.
            return;
        }
        if !down && line == 0 {
            return;
        }

        let target_start: usize = lines[..target].iter().map(|l| l.len() + 1).sum();
        let at = target_start + column.min(lines[target].len());

        let mut value = buffer.value.clone();
        if value.add_caret(vieww_foundation::TextSelection::collapsed(at)) {
            self.edit(value);
        }
    }

    /// Select the word at the caret, or add the next occurrence of what is
    /// already selected — the chord every editor binds to ⌘D.
    ///
    /// Wraps to the start of the file, because the alternative is a key that
    /// stops working two thirds of the way down a file for no visible reason.
    pub fn add_caret_at_next_occurrence(&self) {
        let Some(buffer) = self.active() else {
            return;
        };
        let mut value = buffer.value.clone();
        let text = value.text.clone();

        if value.selection.is_collapsed() {
            // Nothing selected yet: the first press selects the word, which is
            // what gives the second press something to look for.
            value.select_word_at(value.selection.cursor().offset);
            if !value.selection.is_collapsed() {
                self.edit(value);
            }
            return;
        }

        let needle = value.selection.range().slice(&text).to_string();
        if needle.is_empty() {
            return;
        }
        let taken: Vec<(usize, usize)> = value
            .carets()
            .iter()
            .map(|caret| (caret.start(), caret.end()))
            .collect();

        // From just after the last selection, wrapping once.
        let from = value
            .sorted_carets()
            .last()
            .map_or(0, vieww_foundation::TextSelection::end);
        let next = text[from..]
            .find(&needle)
            .map(|at| at + from)
            .or_else(|| text.find(&needle))
            .filter(|&at| !taken.contains(&(at, at + needle.len())));

        if let Some(at) = next {
            if value.add_caret(vieww_foundation::TextSelection::new(at, at + needle.len())) {
                self.edit(value);
            }
        }
    }

    /// Back to one caret.
    pub fn clear_extra_carets(&self) -> bool {
        let Some(buffer) = self.active() else {
            return false;
        };
        let mut value = buffer.value.clone();
        if !value.clear_secondary_carets() {
            return false;
        }
        self.edit(value);
        true
    }

    /// How many carets the active buffer has. The status bar's cell.
    #[must_use]
    pub fn caret_count(&self) -> usize {
        self.active().map_or(1, |buffer| buffer.value.caret_count())
    }

    // ----- N6: folding -------------------------------------------------------

    /// What the code pane is actually showing.
    ///
    /// The whole buffer when nothing is folded, which is the common case and
    /// costs one `project` over text that is already in hand.
    #[must_use]
    pub fn folded_view(&self) -> crate::folding::View {
        let Some(buffer) = self.active() else {
            return crate::folding::View::default();
        };
        crate::folding::project(&buffer.value.text, &self.folds.get())
    }

    /// Every foldable region of the active buffer.
    #[must_use]
    pub fn foldable(&self) -> Vec<crate::folding::Region> {
        // Through the gutter's cache rather than a fresh scan: the gutter
        // derives the same regions from the same text on every write, so this
        // is nearly always a hit, and a miss fills a cache the derivation was
        // about to fill anyway.
        self.active()
            .map(|buffer| (*self.gutter_source().fold_regions(&buffer.value.text)).clone())
            .unwrap_or_default()
    }

    /// Fold or unfold the region the caret is inside.
    ///
    /// The *innermost* region containing the caret, which is what a person
    /// means by "fold this": standing in a nested `if` and pressing fold
    /// should collapse the `if`, not the function around it.
    pub fn toggle_fold(&self) {
        let Some(buffer) = self.active() else {
            return;
        };
        let line = line_of(&buffer.value.text, buffer.value.selection.cursor().offset);
        let mut folds = (*self.folds.get()).clone();

        // Already folded at the caret's line? Unfold it: the caret sits on a
        // collapsed header, and pressing the same key again must open it.
        if folds.remove(&line) {
            self.folds.set(Rc::new(folds));
            return;
        }

        let innermost = crate::folding::regions(&buffer.value.text)
            .into_iter()
            .filter(|region| region.header <= line && line <= region.last)
            .min_by_key(crate::folding::Region::hidden);
        if let Some(region) = innermost {
            folds.insert(region.header);
            self.folds.set(Rc::new(folds));
        }
    }

    /// Fold or unfold the region whose header is `line`. The gutter marker.
    pub fn toggle_fold_at(&self, line: usize) {
        let mut folds = (*self.folds.get()).clone();
        if !folds.remove(&line) {
            folds.insert(line);
        }
        self.folds.set(Rc::new(folds));
    }

    /// Fold every region, or unfold everything.
    pub fn fold_all(&self, folded: bool) {
        if !folded {
            self.folds.set(Rc::new(crate::folding::Folds::new()));
            return;
        }
        let folds: crate::folding::Folds = self
            .foldable()
            .into_iter()
            .map(|region| region.header)
            .collect();
        self.folds.set(Rc::new(folds));
    }

    /// An edit reported by the code field, which may be showing a projection.
    ///
    /// # Why this is a separate entry point
    ///
    /// [`edit`](Self::edit) takes a value in the *buffer's* coordinates and
    /// every other caller has one. The code field does not: while anything is
    /// folded it is holding text with lines removed, so what it reports has to
    /// be turned back into an edit of the real buffer before anything else
    /// looks at it. Doing that inside `edit` would mean every caller paying
    /// for a projection none of them needs.
    pub fn edit_projected(&self, value: vieww_foundation::TextEditingValue) {
        if self.folds.get().is_empty() {
            self.edit(value);
            return;
        }
        let Some(buffer) = self.active() else {
            return;
        };
        let source = buffer.value.text.clone();
        let view = crate::folding::project(&source, &self.folds.get());
        let caret = value.selection.cursor().offset;

        match crate::folding::unproject(&source, &view, &value.text) {
            // A caret move, not a change. Mapped through the same view, so a
            // click on a collapsed line puts the caret on its header rather
            // than somewhere inside what is hidden.
            None => {
                let mut mapped = buffer.value.clone();
                mapped.selection = vieww_foundation::TextSelection::collapsed(
                    crate::folding::source_offset(&source, &view, caret),
                );
                self.edit(mapped);
            }
            Some(result) => {
                let folds = crate::folding::surviving(&self.folds.get(), &result);
                let next_view = crate::folding::project(&result.text, &folds);
                let offset = crate::folding::source_offset(&result.text, &next_view, caret);
                self.folds.set(Rc::new(folds));

                let mut mapped = vieww_foundation::TextEditingValue::new(result.text);
                mapped.selection = vieww_foundation::TextSelection::collapsed(offset);
                self.edit(mapped);
            }
        }
    }

    /// A pointer's selection, mapped back through the fold projection.
    ///
    /// The mirror of [`Studio::edit_projected`] for the read-only path: while
    /// anything is folded the field holds a different string, so an offset it
    /// reports is a view offset and has to be turned into a source one before
    /// it means anything. With no folds the two are the same string and this is
    /// a straight call.
    pub fn select_projected(&self, selection: vieww_foundation::TextSelection) {
        if self.folds.get().is_empty() {
            self.select(selection);
            return;
        }
        let Some(buffer) = self.active() else { return };
        let source = &buffer.value.text;
        let view = crate::folding::project(source, &self.folds.get());
        let map = |offset: usize| crate::folding::source_offset(source, &view, offset);
        self.select(vieww_foundation::TextSelection {
            base: map(selection.base),
            extent: map(selection.extent),
            affinity: selection.affinity,
        });
    }

    /// Move the selection, and only the selection.
    ///
    /// # The gesture that must not be able to edit
    ///
    /// This is where a click, a drag, a double-click and a triple-click land.
    /// All four are *reads*: they choose a range of a document they do not
    /// change. Routing them through [`Studio::edit`] — which takes a whole
    /// `TextEditingValue`, text included — meant every one of them arrived
    /// carrying a string, and a string that arrives is a string that gets
    /// committed. Where that string came from is
    /// [`TextField::on_selection`](vieww_widget::TextField::on_selection)'s
    /// problem and it is now solved at the source; this is the second half,
    /// and it is the half that makes the guarantee structural rather than
    /// careful. **There is no text in this function's arguments, so there is no
    /// text it can get wrong.**
    ///
    /// Everything else `edit` does still has to happen: the caret cell in the
    /// status bar, the blink restarting so the caret is visible where it just
    /// landed, and the history's record of where the caret is — but *not*
    /// `mark_dirty`, because moving a caret does not invalidate a render.
    ///
    /// Extra carets are cleared, which is what a plain click means in every
    /// editor. A modified click would mean the opposite, and cannot happen:
    /// `TapDetails` carries no modifiers yet.
    pub fn select(&self, selection: vieww_foundation::TextSelection) {
        let index = self.active_buffer.get();
        let mut buffers = (*self.buffers.get()).clone();
        let Some(buffer) = buffers.get_mut(index) else {
            return;
        };

        // Clamped rather than trusted. The report was measured against the
        // paragraph from the previous layout, so a selection can name an offset
        // the current text no longer has — and slicing a string at one of those
        // is a panic in the middle of a drag.
        let text = &buffer.value.text;
        let clamp = |offset: usize| {
            let offset = offset.min(text.len());
            (0..=offset)
                .rev()
                .find(|&at| text.is_char_boundary(at))
                .unwrap_or(0)
        };
        let selection = vieww_foundation::TextSelection {
            base: clamp(selection.base),
            extent: clamp(selection.extent),
            affinity: selection.affinity,
        };
        if buffer.value.selection == selection && buffer.value.secondary.is_empty() {
            return;
        }

        self.blink.borrow_mut().wake();
        buffer.value.selection = selection;
        buffer.value.secondary.clear();
        buffer.value.composing = None;
        // The history's coalescing rule wants the newest entry's caret kept in
        // step even when nothing was typed — see `history::History::record`.
        buffer.history.record(&buffer.value);
        let caret = buffer.caret();
        self.put_buffers(buffers);
        self.caret.set(caret);
    }

    /// The active buffer's syntax runs, projected through the fold view.
    ///
    /// # Why this exists rather than dropping them
    ///
    /// Because a folded file with no colour was the most visible rough edge
    /// left in the editor pane, and `ui/editor.rs` said so at the line that
    /// dropped them. Spans and decorations are computed over the *source's*
    /// byte offsets; with anything folded the field holds a different string,
    /// so every offset in them points at the wrong place.
    ///
    /// The fix is a mapping, not a redesign: `folding::line_map` already
    /// knows which source line each visible line came from, so a run is
    /// re-cut line by line and the hidden lines simply do not appear.
    ///
    /// Runs are re-cut rather than re-parsed. Re-highlighting the *view* would
    /// be the obvious alternative and is wrong: the view is not valid Rust —
    /// it is a file with its middles removed — and a parser handed one would
    /// colour the wreckage.
    #[must_use]
    pub fn projected_spans(
        &self,
        style: vieww_foundation::TextStyle,
        theme: crate::theme::StudioTheme,
    ) -> Vec<vieww_text::TextSpan> {
        let spans = self.highlighted_spans(style, theme);
        let folds = self.folds.get();
        if folds.is_empty() || spans.is_empty() {
            return spans;
        }
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        let source = &buffer.value.text;
        let view = crate::folding::project(source, &folds);

        // Where each run starts in the source, so a line's slice can be taken
        // without searching. Runs are contiguous and cover the whole text.
        let mut starts = Vec::with_capacity(spans.len());
        let mut at = 0;
        for span in &spans {
            starts.push(at);
            at += span.text.len();
        }

        // Merged with the previous run when the style matches, which is the
        // common case at a line break and roughly halves the run count on a
        // heavily folded file. A free function rather than a closure because a
        // closure capturing `out` mutably cannot also read `out.last()`.
        fn push(
            out: &mut Vec<vieww_text::TextSpan>,
            text: &str,
            style: vieww_foundation::TextStyle,
        ) {
            if text.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(last) if last.style == style => last.text.push_str(text),
                _ => out.push(vieww_text::TextSpan::new(text, style)),
            }
        }

        let mut out: Vec<vieww_text::TextSpan> = Vec::new();
        for (index, (source_range, _)) in crate::folding::line_map(source, &view)
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                // The line break the map deliberately excludes. Carried at the
                // *previous* run's style so it never opens a run of its own.
                let carried = out.last().map_or(style, |last| last.style);
                push(&mut out, "\n", carried);
            }
            for (span, &start) in spans.iter().zip(starts.iter()) {
                let end = start + span.text.len();
                let from = start.max(source_range.start);
                let to = end.min(source_range.end);
                if from >= to {
                    continue;
                }
                push(&mut out, &span.text[from - start..to - start], span.style);
            }
        }
        out
    }

    /// The active buffer's marks, projected through the fold view.
    ///
    /// A mark whose range has either end inside a fold is **dropped** —
    /// `folding::project_range` has the argument. Squiggling a line that is
    /// not the line with the problem is not a step towards squiggling the
    /// right one.
    #[must_use]
    pub fn projected_decorations(
        &self,
        chrome: crate::theme::StudioTheme,
    ) -> Vec<vieww_foundation::TextDecoration> {
        let marks = self.editor_decorations(chrome);
        let folds = self.folds.get();
        if folds.is_empty() || marks.is_empty() {
            return marks;
        }
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        let source = &buffer.value.text;
        let view = crate::folding::project(source, &folds);

        marks
            .into_iter()
            .filter_map(|mark| {
                let (start, end) =
                    crate::folding::project_range(source, &view, mark.range.start, mark.range.end)?;
                Some(vieww_foundation::TextDecoration {
                    range: vieww_foundation::TextRange { start, end },
                    ..mark
                })
            })
            .collect()
    }

    /// Highlight the active buffer, reusing the parser/cache when possible.
    pub fn highlighted_spans(
        &self,
        style: vieww_foundation::TextStyle,
        theme: crate::theme::StudioTheme,
    ) -> Vec<vieww_text::TextSpan> {
        if !self.highlight_enabled.get() {
            return Vec::new();
        }
        let Some(buffer) = self.active() else {
            return Vec::new();
        };
        // **The Rust grammar is for Rust files.** The studio has one
        // highlighter and used to run it over whatever was in the buffer, so a
        // `Cargo.toml` was parsed as Rust and coloured by what that parse
        // produced — which is not "no highlighting", it is *wrong*
        // highlighting, and the two look the same to someone who has not seen
        // the file before. Everything without a grammar is drawn as plain
        // text, which is legible and honest. `Language::highlighted` is the
        // one place that decides.
        if !buffer.language.highlighted() {
            return Vec::new();
        }
        self.highlighter
            .borrow_mut()
            .highlight(&buffer.value.text, buffer.language, style, theme)
    }

    /// Put a widget-library entry into the active buffer.
    ///
    /// # What this used to do, and why it is worth naming
    ///
    /// It was `value.insert(text)` — the snippet's characters, at the caret,
    /// whatever was there. Two of the three entries were bare expressions, so
    /// one click on a fresh buffer produced ``expected one of `!` or `::`,
    /// found `(` `` and four clicks produced a single 388-character line with
    /// four snippets glued end to end. Both are visible in the screencast this
    /// was written from.
    ///
    /// [`edit_ops::insert_snippet`](crate::edit_ops::insert_snippet) does the
    /// work now: whole lines, indented to where they land, and a *refusal*
    /// carrying a message when the caret is somewhere the entry cannot go.
    /// This function's only job is to route that message somewhere a person
    /// reads it.
    pub fn insert_snippet(&self, snippet: &crate::edit_ops::Snippet) {
        let Some(buffer) = self.active() else {
            self.notify("There is no buffer to insert into.");
            return;
        };
        let selection = (buffer.value.selection.base, buffer.value.selection.extent);
        match crate::edit_ops::insert_snippet(&buffer.value.text, selection, snippet) {
            Ok(edit) => {
                self.apply_edit(edit);
                self.notify(&format!("Inserted \u{201c}{}\u{201d}.", snippet.name));
            }
            // Not `append_output`: the Output panel is where a *build* talks,
            // and a refusal here is about the click that just happened.
            Err(why) => self.notify(&why),
        }
    }

    /// Insert a user-defined snippet, which is text rather than a `Snippet`.
    ///
    /// Goes through the same whole-line, indent-aware insertion the built-in
    /// ones use — a user snippet that pasted at the caret would produce the
    /// exact 388-character glued-together line that `insert_snippet`'s own
    /// documentation exists to describe.
    pub fn insert_text_snippet(&self, body: &str) {
        let Some(buffer) = self.active() else {
            self.notify("There is no buffer to insert into.");
            return;
        };
        let selection = (buffer.value.selection.base, buffer.value.selection.extent);
        // Every user snippet is an `Expression`: the studio cannot know
        // whether a block of text is one or a top-level item, and the vast
        // majority of what somebody keeps in a snippets file is widget-building
        // code that belongs inside a `build`. Treating it as an item would
        // refuse to insert it exactly where it is wanted.
        match crate::edit_ops::insert_body(
            &buffer.value.text,
            selection,
            "snippet",
            crate::edit_ops::SnippetKind::Expression,
            body,
        ) {
            Ok(edit) => self.apply_edit(edit),
            Err(why) => self.notify(&why),
        }
    }

    /// Write a template into the open project and open it.
    ///
    /// # Named from the buffer, not from a dialog
    ///
    /// The studio has one text input that is always ready — the palette — and
    /// a second modal for a single word would be a second modal to build,
    /// dismiss and test. So the *name* comes from the active buffer's file
    /// name, incremented until it is free, and the file is opened for renaming
    /// the moment it exists. A dialog is the better answer and this is the
    /// honest smaller one; it is written down here rather than discovered.
    pub(super) fn new_from_template(&self, template: crate::scaffold::Template) {
        let Some(root) = self.root.peek().clone() else {
            self.notify("A template goes into a project — open a folder first.");
            return;
        };

        // `screen`, then `screen_2`, then `screen_3`. Never a name that is
        // already a file: a template that silently overwrote somebody's screen
        // would be the worst possible thing for this command to do.
        let stem = match template {
            crate::scaffold::Template::Screen => "screen",
            crate::scaffold::Template::Widget => "widget",
        };
        let mut name = stem.to_owned();
        for suffix in 2..1000_u32 {
            if !root.join("src/screens").join(format!("{name}.rs")).exists() {
                break;
            }
            name = format!("{stem}_{suffix}");
        }

        let made = match crate::scaffold::scaffold(template, &name) {
            Ok(made) => made,
            Err(error) => {
                self.notify(&format!("{}: {error}", template.title()));
                return;
            }
        };

        let path = root.join(&made.path);
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.notify(&format!("Could not create {}: {error}", parent.display()));
                return;
            }
        }
        if let Err(error) = std::fs::write(&path, &made.contents) {
            self.notify(&format!("Could not write {}: {error}", path.display()));
            return;
        }

        // **And the module list, or the file compiles into nothing.** A screen
        // that exists on disk and is not in `mod.rs` is a file the project does
        // not have, which is the failure this command would otherwise create
        // every single time.
        if let Some(line) = made.module_line.as_ref() {
            let mod_rs = root.join("src/screens/mod.rs");
            match std::fs::read_to_string(&mod_rs) {
                Ok(existing) if existing.contains(line.as_str()) => {}
                Ok(existing) => {
                    let joined = format!("{}\n{line}\n", existing.trim_end());
                    if let Err(error) = std::fs::write(&mod_rs, joined) {
                        self.notify(&format!("Wrote the file, but not mod.rs: {error}"));
                    }
                }
                // No `mod.rs` at all is a project laid out some other way. The
                // file is still written and the user is told what is missing,
                // rather than the studio inventing a module tree for them.
                Err(_) => self.notify(&format!(
                    "Wrote {}. Add `{line}` wherever this project lists its modules.",
                    made.path.display()
                )),
            }
        }

        self.open_workspace(&root);
        self.open_path(path.clone());
        self.notify(&format!(
            "Created {}. Rename it before you build on it.",
            made.path.display()
        ));
    }

    /// Multiply the preview zoom, or start from the fit.
    ///
    /// Clamped to a range a device frame is still legible in: below a quarter
    /// the screen is a smudge, and above four times the frame is off the pane
    /// in every direction with no way to pan it.
    pub(super) fn zoom_by(&self, factor: f32) {
        let current = self.preview_zoom.peek().unwrap_or(1.0);
        let next = (current * factor).clamp(0.25, 4.0);
        self.preview_zoom.set(Some(next));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a percentage of a clamped 0.25..4.0 is a small integer"
        )]
        self.notify(&format!("Preview at {}%.", (next * 100.0).round() as u32));
    }

    /// Cut, copy or paste, against the code buffer.
    ///
    /// # Why the studio does this rather than the field
    ///
    /// Cut, copy and paste are the three intents `TextEditingValue::apply`
    /// refuses by design: they need a pasteboard, which is a platform service
    /// and is exactly as absent from `vieww-foundation` as fonts are. Whatever
    /// holds the service carries them out. `RenderEditableText` does when the
    /// *keys* are pressed, because it is focused and knows it. A menu row is
    /// not focused and cannot know, so this does the same work against the
    /// active buffer — which is what an editor's Edit menu means everywhere.
    ///
    /// A cut whose write to the pasteboard fails does **not** delete, because a
    /// cut that could not reach the pasteboard would destroy the text instead
    /// of moving it. That rule is `RenderEditableText::use_clipboard`'s and is
    /// repeated here rather than referred to, because the failure it prevents
    /// is silent.
    pub(super) fn clipboard_command(&self, command: Command) {
        use vieww_foundation::Clipboard;
        let Some(buffer) = self.active() else { return };
        let Some(clipboard) = self.services.get::<dyn Clipboard>() else {
            self.notify("This window has no pasteboard.");
            return;
        };

        match command {
            Command::Copy | Command::Cut => {
                let selected = buffer.value.selected_text();
                if selected.is_empty() {
                    // A caret is not a selection. Copying the whole buffer here
                    // would overwrite whatever was actually on the pasteboard.
                    self.notify("Nothing is selected.");
                    return;
                }
                if clipboard.write_text(selected).is_err() {
                    self.notify("The pasteboard would not accept it.");
                    return;
                }
                if command == Command::Cut {
                    let mut value = buffer.value.clone();
                    value.delete_backward();
                    self.edit(value);
                }
                self.notify(if command == Command::Cut {
                    "Cut."
                } else {
                    "Copied."
                });
            }
            Command::Paste => match clipboard.read_text() {
                Ok(Some(text)) => {
                    let mut value = buffer.value.clone();
                    value.insert(&text);
                    self.edit(value);
                }
                // Empty, unreadable, or holding something that is not text. All
                // three mean the same thing to a buffer.
                _ => self.notify("There is no text on the pasteboard."),
            },
            _ => {}
        }
    }

    /// Take whatever panicked in a build this frame and put it in Problems.
    ///
    /// # The panic boundary the plan asked for already exists
    ///
    /// `M3-M5.md` said the uncovered case was *"a panic on a later rebuild,
    /// inside a host frame"*, and that plan §4.4's answer was a second
    /// `FrameDriver` the host drives inside `catch_unwind`. **That note is
    /// out of date and the second driver is not needed.** `ElementTree`
    /// catches a panic out of any `build`, mounts a substitute in that
    /// widget's place and records a `BuildError` — which is a *better*
    /// boundary than a second driver, because it is per-widget rather than
    /// per-frame: the rest of the previewed screen keeps working.
    ///
    /// Two things were missing, and this is both of them.
    ///
    /// **The policy was never set.** It defaults to `Placeholder` in a debug
    /// build and `Propagate` in a release one, which is the right default for
    /// an *application* — a magenta box on a user's phone is worth less than a
    /// crash report. A studio is the other case: the code that panics is the
    /// user's, it is the thing they are working on, and the release build is
    /// the one they will run. `viewwstudio::install` now sets it explicitly.
    ///
    /// **Nothing read the errors.** They were recorded and dropped. A caught
    /// panic that nobody reports is a screen that silently shows a magenta box.
    ///
    /// # What is still not covered
    ///
    /// A panic in **layout or paint** of a render object. `ElementTree` guards
    /// `build`; `RenderOwner::draw_frame` guards nothing, and a `layout` that
    /// divides by a zero-height constraint still takes the process down. Said
    /// here rather than left to be found, because the honest boundary is
    /// narrower than "panics are handled".
    pub fn drain_build_errors(&self, driver: &mut vieww_render::FrameDriver) {
        // Layout and paint first, because they are the pair this used to have
        // no answer for: `ElementTree` caught a panicking `build` and the
        // process died on a panicking `layout`. `FrameDriver` guards both now,
        // and this is where the studio says so.
        self.drain_frame_errors(driver);
        let errors = driver.elements().take_build_errors();
        if errors.is_empty() {
            return;
        }

        let name = self
            .active()
            .map_or_else(|| "preview".to_owned(), |buffer| buffer.name.clone());
        let mut all = (*self.diagnostics.get()).clone();
        for error in &errors {
            // Line 1, because a panic has no source position: it happened in
            // the *built* widget, and the byte offset that produced it is not
            // something the element tree knows. Pointing at line 1 is honest
            // about that; inventing a line would not be.
            all.push(Diagnostic {
                file: name.clone(),
                severity: Severity::Error,
                code: "panic".to_owned(),
                message: format!("{} panicked while building: {}", error.widget_name, error.message),
                help: Some(
                    "The widget was replaced with a placeholder and the rest of the screen                      kept rendering. Fix the panic and press Render."
                        .to_owned(),
                ),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            });
        }
        self.diagnostics.set(Rc::new(all));
        self.panel_tab.set(PanelTab::Problems);
        self.panel_open.set(true);
        self.notify(&format!(
            "{} widget{} panicked — the rest of the screen still rendered.",
            errors.len(),
            if errors.len() == 1 { "" } else { "s" }
        ));
    }

    /// Take the previewed screen's state, before the screen is replaced.
    ///
    /// # The gap this closes
    ///
    /// Every Render recompiled the buffer, `dlopen`ed the result and mounted it
    /// from scratch. The widget types come from a new library, so their
    /// `TypeId`s differ and reconciliation cannot match a single element — the
    /// whole subtree is torn down. **Everything the developer had done to the
    /// screen went with it**: the card they had scrolled to, the colour they
    /// had picked, the page they were on. The preview caption had to say so,
    /// and a developer tweaking one padding value had to re-navigate to what
    /// they were looking at after every change.
    ///
    /// Called from the frame hook because that is the only place with a
    /// [`FrameDriver`](vieww_render::FrameDriver), and armed by
    /// [`render`](Self::render) so it costs a `Cell` read on every frame that
    /// is not one.
    pub fn capture_preview_state(&self, driver: &mut vieww_render::FrameDriver) {
        if !self.preview_capture_due.replace(false) {
            return;
        }
        let elements = driver.elements();
        let Some(root) = elements
            .find(crate::ui::preview::GUEST_ROOT)
            .map(|e| e.id())
        else {
            return;
        };
        *self.preview_state.borrow_mut() = elements.snapshot_states(root);
    }

    /// Put it back, into the tree that replaced the one it came from.
    ///
    /// A frame later than the capture, deliberately: the new guest is mounted
    /// during the build phase of the frame that installed it, so the earliest
    /// moment its elements exist is the hook of the frame after.
    ///
    /// A path that no longer exists is skipped — see
    /// [`ElementTree::restore_states`](vieww_element::ElementTree::restore_states)
    /// — so an edit that moved a widget restores everything around it and
    /// leaves that one as new. The count is reported rather than assumed,
    /// because "some of your state came back" and "all of it did" are different
    /// sentences and only one of them is usually true.
    pub fn restore_preview_state(&self, driver: &mut vieww_render::FrameDriver) {
        if !self.preview_restore_due.replace(false) {
            return;
        }
        let snapshot = std::mem::take(&mut *self.preview_state.borrow_mut());
        if snapshot.is_empty() {
            return;
        }
        let elements = driver.elements();
        let Some(root) = elements
            .find(crate::ui::preview::GUEST_ROOT)
            .map(|e| e.id())
        else {
            return;
        };
        let restored = elements.restore_states(root, &snapshot);
        if restored > 0 {
            self.append_output(vec![format!(
                "restored {restored} of {} pieces of screen state across the render",
                snapshot.len()
            )]);
        }
    }

    /// Report the layout and paint panics the frame driver caught.
    ///
    /// Same shape as the build-panic path above and a different sentence,
    /// because the outcome is different: a build panic leaves a placeholder in
    /// one widget and the rest of the screen intact, while a layout or paint
    /// panic abandons the frame — what is on screen is the last one that
    /// finished, and it will stay that way until the panic is fixed.
    pub(super) fn drain_frame_errors(&self, driver: &mut vieww_render::FrameDriver) {
        let errors = driver.take_frame_errors();
        if errors.is_empty() {
            return;
        }
        let name = self
            .active()
            .map_or_else(|| "preview".to_owned(), |buffer| buffer.name.clone());
        let mut all = (*self.diagnostics.get()).clone();
        for error in &errors {
            all.push(Diagnostic {
                file: name.clone(),
                severity: Severity::Error,
                code: "panic".to_owned(),
                message: format!(
                    "a render object panicked during {}: {}",
                    error.phase.name(),
                    error.message
                ),
                help: Some(
                    "The frame was abandoned and the previous one is still on screen.                      This used to end the process and take unsaved buffers with it."
                        .to_owned(),
                ),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            });
        }
        self.diagnostics.set(Rc::new(all));
        self.panel_tab.set(PanelTab::Problems);
        self.panel_open.set(true);
        self.notify(&format!(
            "{} render panic{} — the last good frame is still on screen.",
            errors.len(),
            if errors.len() == 1 { "" } else { "es" }
        ));
    }

    /// Every token in the live theme, with any override applied.
    ///
    /// Read from `ThemeData` and `StudioTheme` rather than from a list written
    /// here, so a token added to either shows up without this file changing —
    /// which is the property the prototype's own note asks for when it says the
    /// groups match `ThemeData` field for field.
    #[must_use]
    pub fn tokens(&self, chrome: crate::theme::StudioTheme) -> Vec<crate::tokens::Token> {
        use crate::tokens::{Group, Role, Token};
        // **The theme the shell is actually drawn with**, not `ColorScheme`'s
        // own defaults. `theme_data` replaces the framework's stock greys
        // and accent with the studio's — an editor's foregrounds, neutral so
        // nothing in the chrome competes with the syntax colours — and a token
        // editor whose first line says "read from the live theme" while showing
        // `#ADC6FF` for an accent that is drawn purple is worse than one that
        // did not claim it.
        let colors = crate::theme::theme_data(self.dark.get()).colors;
        let rgb = |color: vieww_foundation::Color| -> u32 {
            (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
        };
        let edits = self.token_edits.get();
        let make =
            |group: Group, role: Role, name: &'static str, color: vieww_foundation::Color| Token {
                group,
                name,
                role,
                rgb: edits.get(name).copied().unwrap_or_else(|| rgb(color)),
            };

        let mut out = vec![
            make(Group::Colors, Role::Surface, "primary", colors.primary),
            make(Group::Colors, Role::Text, "on_primary", colors.on_primary),
            make(Group::Colors, Role::Surface, "surface", colors.surface),
            make(Group::Colors, Role::Text, "on_surface", colors.on_surface),
            make(
                Group::Colors,
                Role::Surface,
                "surface_variant",
                colors.surface_variant,
            ),
            make(
                Group::Colors,
                Role::Text,
                "on_surface_variant",
                colors.on_surface_variant,
            ),
            make(Group::Colors, Role::Boundary, "outline", colors.outline),
            make(Group::Colors, Role::Surface, "error", colors.error),
            make(Group::Colors, Role::Text, "on_error", colors.on_error),
            make(Group::Colors, Role::Surface, "success", colors.success),
            make(Group::Colors, Role::Text, "on_success", colors.on_success),
        ];
        out.extend([
            make(Group::Chrome, Role::Surface, "window", chrome.window),
            make(Group::Chrome, Role::Surface, "chrome_0", chrome.chrome_0),
            make(Group::Chrome, Role::Surface, "chrome_1", chrome.chrome_1),
            make(Group::Chrome, Role::Surface, "chrome_2", chrome.chrome_2),
            make(Group::Chrome, Role::Surface, "chrome_3", chrome.chrome_3),
            make(Group::Chrome, Role::Surface, "chrome_4", chrome.chrome_4),
            make(Group::Chrome, Role::Boundary, "line", chrome.line),
            make(Group::Chrome, Role::Text, "gutter", chrome.gutter),
            make(
                Group::Chrome,
                Role::Text,
                "gutter_active",
                chrome.gutter_active,
            ),
            make(Group::Chrome, Role::Surface, "selection", chrome.selection),
            make(Group::Chrome, Role::Text, "warning", chrome.warning),
        ]);
        out.extend([
            make(Group::Syntax, Role::Text, "keyword", chrome.syntax.keyword),
            make(
                Group::Syntax,
                Role::Text,
                "function",
                chrome.syntax.function,
            ),
            make(
                Group::Syntax,
                Role::Text,
                "type_name",
                chrome.syntax.type_name,
            ),
            make(Group::Syntax, Role::Text, "string", chrome.syntax.string),
            make(Group::Syntax, Role::Text, "number", chrome.syntax.number),
            make(Group::Syntax, Role::Text, "comment", chrome.syntax.comment),
            make(
                Group::Syntax,
                Role::Text,
                "macro_name",
                chrome.syntax.macro_name,
            ),
            make(
                Group::Syntax,
                Role::Text,
                "punctuation",
                chrome.syntax.punctuation,
            ),
        ]);
        out
    }

    /// The theme the shell mounts, with any token overrides applied.
    ///
    /// # Why the overrides are applied here rather than stored as a theme
    ///
    /// Because the base is a *function of the dark flag*, and an edited theme
    /// stored whole would stop following it: nudge one token, switch to light,
    /// and every other value would still be the dark one. Keeping the edits as
    /// a sparse map and re-applying them over whichever base is current means
    /// the two settings compose instead of one silently winning.
    #[must_use]
    pub fn themed(&self) -> (vieww_widget::ThemeData, crate::theme::StudioTheme) {
        let dark = self.dark.get();
        let brand = crate::theme::accent_named(&self.accent.get());
        let mut data = crate::theme::theme_data(dark);
        let mut chrome = crate::theme::studio_theme_with(dark, brand);
        // The previewed application's `primary` follows the studio's accent
        // too, so a screen with a button in it looks like it belongs to the
        // studio it is being drawn in — until the person edits `primary`
        // themselves, which the token overrides below still win.
        data.colors.primary = brand.near(dark);
        // **High contrast first, then the user's own overrides.** A token the
        // user set by hand is a decision; pushing contrast on top of it would
        // silently move the colour they chose. This way an explicit override
        // always wins.
        if self.high_contrast.get() {
            let towards = if dark {
                vieww_foundation::Color::WHITE
            } else {
                vieww_foundation::Color::BLACK
            };
            // The greys the chrome uses for secondary text and hairlines are
            // where the WCAG bar is actually missed; the primary foreground is
            // usually already fine and is lifted anyway so the hierarchy keeps
            // its spacing rather than collapsing.
            for target in [
                &mut data.colors.on_surface,
                &mut data.colors.on_surface_variant,
                &mut data.colors.outline,
            ] {
                *target = towards_color(*target, towards, CONTRAST_LIFT);
            }
            chrome.gutter = towards_color(chrome.gutter, towards, CONTRAST_LIFT);
            chrome.line = towards_color(chrome.line, towards, CONTRAST_LIFT);
        }

        let edits = self.token_edits.get();
        if edits.is_empty() {
            return (data, chrome);
        }

        let color = |rgb: u32| vieww_foundation::Color::hex(rgb);
        for (name, &rgb) in edits.iter() {
            match name.as_str() {
                "primary" => data.colors.primary = color(rgb),
                "on_primary" => data.colors.on_primary = color(rgb),
                "surface" => data.colors.surface = color(rgb),
                "on_surface" => data.colors.on_surface = color(rgb),
                "surface_variant" => data.colors.surface_variant = color(rgb),
                "on_surface_variant" => data.colors.on_surface_variant = color(rgb),
                "outline" => data.colors.outline = color(rgb),
                "error" => data.colors.error = color(rgb),
                "on_error" => data.colors.on_error = color(rgb),
                "success" => data.colors.success = color(rgb),
                "on_success" => data.colors.on_success = color(rgb),
                "window" => chrome.window = color(rgb),
                "chrome_0" => chrome.chrome_0 = color(rgb),
                "chrome_1" => chrome.chrome_1 = color(rgb),
                "chrome_2" => chrome.chrome_2 = color(rgb),
                "chrome_3" => chrome.chrome_3 = color(rgb),
                "chrome_4" => chrome.chrome_4 = color(rgb),
                "line" => chrome.line = color(rgb),
                "gutter" => chrome.gutter = color(rgb),
                "gutter_active" => chrome.gutter_active = color(rgb),
                "selection" => chrome.selection = color(rgb),
                "warning" => chrome.warning = color(rgb),
                "keyword" => chrome.syntax.keyword = color(rgb),
                "function" => chrome.syntax.function = color(rgb),
                "type_name" => chrome.syntax.type_name = color(rgb),
                "string" => chrome.syntax.string = color(rgb),
                "number" => chrome.syntax.number = color(rgb),
                "comment" => chrome.syntax.comment = color(rgb),
                "macro_name" => chrome.syntax.macro_name = color(rgb),
                "punctuation" => chrome.syntax.punctuation = color(rgb),
                // A name that is in the map and not in the theme is a token
                // that was renamed since the edit was made. Ignored rather than
                // panicking: an old override is not worth losing a window over.
                _ => {}
            }
        }
        (data, chrome)
    }

    /// Nudge one token lighter or darker.
    pub fn nudge_token(&self, name: &str, current: u32, factor: f32) {
        let mut edits = (*self.token_edits.get()).clone();
        edits.insert(name.to_owned(), crate::tokens::lightened(current, factor));
        self.token_edits.set(Rc::new(edits));
    }

    /// Put every token back where the theme had it.
    pub fn reset_tokens(&self) {
        if self.token_edits.peek().is_empty() {
            return;
        }
        self.token_edits
            .set(Rc::new(std::collections::BTreeMap::new()));
        self.notify("Tokens reset to the theme's own values.");
    }

    /// Write the edited token set into the open project as `src/theme.rs`.
    ///
    /// Beside an existing one rather than over it — see [`crate::tokens`] for
    /// why rewriting somebody's source by pattern-matching hex literals is not
    /// something this should attempt.
    pub fn export_tokens(&self, chrome: crate::theme::StudioTheme) {
        let Some(root) = self.root.peek().clone() else {
            self.notify("Exporting tokens writes a file into a project — open a folder first.");
            return;
        };
        let source = crate::tokens::module(&self.tokens(chrome));
        let mut path = root.join("src").join("theme.rs");
        for suffix in 1..100_u32 {
            if !path.exists() {
                break;
            }
            path = root.join("src").join(format!("theme_{suffix}.rs"));
        }
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.notify(&format!("Could not create {}: {error}", parent.display()));
                return;
            }
        }
        match std::fs::write(&path, source) {
            Ok(()) => {
                self.open_workspace(&root);
                self.open_path(path.clone());
                self.notify(&format!(
                    "Wrote {}. Merge it with your own theme rather than replacing it blind.",
                    path.file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
                ));
            }
            Err(error) => self.notify(&format!("Could not write {}: {error}", path.display())),
        }
    }

    /// Start `rust-analyzer` for the open project, or stop it.
    ///
    /// # Why it is not started automatically
    ///
    /// Because it indexes the whole dependency graph on the first open — tens
    /// of seconds and a gigabyte on a real project — and a studio that did that
    /// to somebody who opened a folder to read one file would be a studio
    /// people learn to open with the folder closed. It is a thing you ask for.
    /// Start rust-analyzer if a project is open and the binary is there.
    ///
    /// # Why the studio now does this on its own
    ///
    /// The analyzer was off until somebody found `rust-analyzer: Start / Stop`
    /// in the palette, and *everything that depends on it* — completion, hover,
    /// go-to-definition, find-references — silently did nothing until they did.
    /// Four features that look broken is a worse first impression than a
    /// language server that takes a few seconds to index, and no editor people
    /// arrive from makes them ask for it.
    ///
    /// `announce` is the difference between the user asking and the studio
    /// deciding: a toast saying "could not start rust-analyzer" is the right
    /// answer to a menu item and pure noise on every folder that happens not to
    /// have the component installed. Unasked, it leaves the reason in the
    /// status bar and the Output log instead, where somebody wondering why
    /// completion is quiet will find it.
    pub fn start_analyzer(&self, announce: bool) {
        if self.analyzer.borrow().is_some() {
            return;
        }
        let Some(root) = self.root.peek().clone() else {
            if announce {
                self.notify("rust-analyzer needs a project — open a folder first.");
            }
            return;
        };
        // Nothing to serve. `Client::start` would spawn a server that indexes an
        // empty project and answers nothing.
        if !root.join("Cargo.toml").is_file() {
            return;
        }
        let waker = self.waker.clone();
        let wake = move || {
            use vieww_foundation::task::FrameWaker as _;
            waker.wake();
        };
        match crate::lsp::Client::start(&root, wake) {
            Ok(mut client) => {
                if let Some(buffer) = self.active() {
                    if let Some(path) = buffer.path.as_ref() {
                        let _ = client.open(path, &buffer.value.text);
                    }
                }
                *self.analyzer.borrow_mut() = Some(client);
                self.analyzer_state.set("starting");
                if announce {
                    self.notify("rust-analyzer starting — the first index takes a while.");
                }
            }
            Err(error) => {
                self.analyzer_state.set("unavailable");
                let message = format!(
                    "Could not start rust-analyzer ({error}). `rustup component add rust-analyzer`."
                );
                if announce {
                    self.notify(&message);
                } else {
                    self.append_output(vec![message]);
                }
            }
        }
    }

    pub fn toggle_analyzer(&self) {
        if self.analyzer.borrow().is_some() {
            // Dropped, which kills it — see `Client::drop`.
            *self.analyzer.borrow_mut() = None;
            self.analyzer_state.set("off");
            self.notify("rust-analyzer stopped.");
            return;
        }

        // Asked for by name, so it says what happened either way.
        self.start_analyzer(true);
    }

    /// Take whatever the language server has said, and show it.
    ///
    /// Called from `before_frame`, beside the compile poll and for the same
    /// reason: it is a `try_recv` that costs nothing when the channel is empty,
    /// and a message applied between frames rather than during one.
    pub fn poll_analyzer(&self) {
        let events = match self.analyzer.borrow().as_ref() {
            Some(client) => client.drain(),
            None => return,
        };
        if events.is_empty() {
            return;
        }

        let mut replaced: Option<(String, Vec<crate::lsp::Report>)> = None;
        for event in events {
            match event {
                crate::lsp::Event::Ready => self.analyzer_state.set("ready"),
                crate::lsp::Event::Diagnostics { path, reports } => {
                    replaced = Some((path, reports));
                }
                crate::lsp::Event::Completions { id, items } => self.accept_completions(id, &items),
                crate::lsp::Event::Hover { id, text } => {
                    // Hover replies go to the Output panel rather than to a
                    // popup, and that is a deliberate half-measure said out
                    // loud: a hover popup needs a pointer-position-to-offset
                    // mapping the studio does not have outside the field, and
                    // the type under the caret is useful in a panel today
                    // rather than in a popup at some point.
                    if let Some(text) = text {
                        self.append_output(
                            text.lines().map(|line| format!("ra: {line}")).collect(),
                        );
                        self.panel_open.set(true);
                        self.panel_tab.set(PanelTab::Output);
                    } else {
                        self.notify("rust-analyzer had nothing to say about that");
                    }
                    let _ = id;
                }
                crate::lsp::Event::Locations { id, places } => {
                    self.accept_locations(id, &places);
                }
                crate::lsp::Event::Other(text) => self.append_output(vec![format!("ra: {text}")]),
                crate::lsp::Event::Stopped(why) => {
                    self.analyzer_state.set("off");
                    *self.analyzer.borrow_mut() = None;
                    self.notify(&format!("rust-analyzer: {why}"));
                }
            }
        }

        let Some((path, reports)) = replaced else {
            return;
        };
        let name = std::path::Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        // **Replaced, not appended.** `publishDiagnostics` is the whole list for
        // one document; treating it as an append is how an error somebody just
        // fixed stays on screen until the next full compile.
        let mut kept: Vec<Diagnostic> = self
            .diagnostics
            .get()
            .iter()
            .filter(|existing| existing.file != name)
            .cloned()
            .collect();
        kept.extend(reports.into_iter().map(|report| Diagnostic {
            file: name.clone(),
            severity: if report.severity <= 1 {
                Severity::Error
            } else {
                Severity::Warning
            },
            code: report.code,
            message: report.message,
            help: None,
            line: report.line,
            column: report.column,
            end_line: report.end_line,
            end_column: report.end_column,
        }));
        self.diagnostics.set(Rc::new(kept));
    }

    /// N10: render when the buffer has stopped changing.
    ///
    /// # Why this is the hot reload a studio needs
    ///
    /// `crates/vieww-reload` exists, has three examples and is unused here, and
    /// the obvious reading of N10 is "wire it up". That reading is wrong.
    /// `Reloader` is for an application hot-reloading *itself* from a cdylib
    /// somebody else rebuilds — it watches a path, stages it, `dlopen`s it and
    /// swaps a root. The studio already does every one of those things, in
    /// `compile.rs` and `loaded.rs`, because it is the thing doing the
    /// rebuilding. Routing its own preview through `Reloader` would be two
    /// implementations of one loop.
    ///
    /// What was missing is the *trigger*. `PRODUCTION-GAPS.md` says iteration
    /// is slow while hot reload is unbuilt; the slow part is pressing a button
    /// after every edit. This is the button pressing itself.
    ///
    /// Debounced rather than immediate, because a compile per keystroke is a
    /// compile that is always out of date: `rustc` takes about 0.2s for a
    /// screen and a typist beats that comfortably.
    pub fn poll_auto_render(&self) {
        if !self.auto_render.peek() {
            // Cleared, so turning the setting off does not leave a render
            // armed to fire once more.
            if self.render_due.borrow().is_some() {
                *self.render_due.borrow_mut() = None;
            }
            return;
        }
        let due = *self.render_due.borrow();
        let Some(due) = due else { return };
        if std::time::Instant::now() < due {
            return;
        }
        *self.render_due.borrow_mut() = None;
        // Not while one is already running, and not with no compiler. Both are
        // `can_run`'s answer, so the automatic path and the button cannot
        // disagree about when a render is possible.
        if self.can_run(Command::Render) && self.dirty.peek() {
            self.render();
        }
    }

    /// Arm the auto-render debounce, and tell the language server.
    ///
    /// One function because both are "the buffer changed and something outside
    /// the editor should know", and both have to happen on exactly the edits
    /// that changed *text* — a caret move is neither.
    pub(super) fn buffer_changed(&self) {
        if self.auto_render.peek() {
            *self.render_due.borrow_mut() =
                Some(std::time::Instant::now() + std::time::Duration::from_millis(600));
        }
        if let Some(client) = self.analyzer.borrow_mut().as_mut() {
            if let Some(buffer) = self.active() {
                if let Some(path) = buffer.path.as_ref() {
                    let _ = client.change(path, &buffer.value.text);
                }
            }
        }
    }

    /// Read the damage and the semantics tree, and resolve a pending pick.
    ///
    /// All three need a `FrameDriver` and none of them can be asked for during
    /// a `build`, which is why they live here beside the tree capture rather
    /// than in the widgets that draw them.
    ///
    /// Each half returns early when its overlay is off, so a studio with none
    /// of them on pays three boolean reads a frame.
    pub fn capture_overlays(&self, driver: &mut vieww_render::FrameDriver) {
        // A pick is resolved first: it is a thing the user just did, and the
        // overlays below are a picture of a frame that has already happened.
        if let Some((x, y)) = self.pick_request.peek() {
            self.pick_request.set(None);
            if let Some(id) = driver.hit_test_identify(vieww_foundation::Offset::new(x, y)) {
                // The snapshot is a flat list in the same order
                // `describe_subtree` walked, so the row is a search by id
                // rather than a second traversal.
                let found = self
                    .tree_snapshot
                    .borrow()
                    .iter()
                    .position(|(_, node)| node.id == id);
                match found {
                    Some(index) => {
                        self.inspect_selected.set(Some(index));
                        self.right_tab.set(RightTab::Inspector);
                        self.right_open.set(true);
                    }
                    // Hit something the capture did not reach — past the node
                    // cap, or in a frame the Inspector was shut for. Saying so
                    // beats selecting the wrong row.
                    None => self.notify(
                        "That is outside the captured tree — open the Inspector and click again.",
                    ),
                }
            }
            self.pick_mode.set(false);
        }

        let want_damage = self.show_damage.peek();
        let want_semantics = self.show_semantics.peek();
        if !want_damage && !want_semantics {
            // Cleared rather than left, so turning an overlay off takes its
            // boxes with it instead of freezing the last frame's on screen.
            if !self.damage_snapshot.borrow().is_empty()
                || !self.semantics_snapshot.borrow().is_empty()
            {
                self.damage_snapshot.borrow_mut().clear();
                self.semantics_snapshot.borrow_mut().clear();
                self.overlay_generation.update(|n| *n = n.wrapping_add(1));
            }
            return;
        }

        let mut changed = false;
        if want_damage {
            let regions: Vec<vieww_foundation::Rect> = driver.damage().regions().to_vec();
            if *self.damage_snapshot.borrow() != regions {
                *self.damage_snapshot.borrow_mut() = regions;
                changed = true;
            }
        }
        if want_semantics {
            let nodes: Vec<(vieww_foundation::Rect, String)> = driver
                .semantics()
                .nodes()
                .iter()
                .map(|node| {
                    (
                        node.bounds,
                        // Role first, because the question this overlay answers
                        // is "is this announced as the right *kind* of thing",
                        // and a label with no role is half an answer.
                        match node.label.as_ref() {
                            Some(label) => format!("{:?} · {label}", node.role),
                            None => format!("{:?}", node.role),
                        },
                    )
                })
                .collect();
            if *self.semantics_snapshot.borrow() != nodes {
                *self.semantics_snapshot.borrow_mut() = nodes;
                changed = true;
            }
        }
        if changed {
            self.overlay_generation.update(|n| *n = n.wrapping_add(1));
        }
    }

    /// Read the render tree into [`Studio::tree_snapshot`].
    ///
    /// Called from `main`'s `before_frame`, beside the compile poll and the
    /// watcher drain, and for the same reason: it is the one place in the
    /// application that holds a `FrameDriver` and is not inside a `build`.
    ///
    /// Costs nothing when the Inspector is shut, which is nearly always — the
    /// early return is the whole reason this is safe to call every frame.
    pub fn capture_tree(&self, driver: &vieww_render::FrameDriver) {
        if !self.right_open.peek() || self.right_tab.peek() != RightTab::Inspector {
            return;
        }
        let tree = driver.renders();
        let Some(root) = tree.root() else { return };
        // Bounded: a render tree has no size limit and the pane does. 400 is
        // about thirty screens' worth of rows, and the count is shown so a
        // truncated tree is never mistaken for a small one.
        let described = tree.describe_subtree(root, 400);
        let changed = {
            let held = self.tree_snapshot.borrow();
            held.len() != described.len()
                || held
                    .iter()
                    .zip(described.iter())
                    .any(|(a, b)| a.0 != b.0 || a.1 != b.1)
        };
        if !changed {
            // The generation is what wakes the pane, and bumping it every frame
            // would rebuild the Inspector sixty times a second on a still
            // window. Same rule `RenderViewport::report_extents` follows.
            return;
        }
        *self.tree_snapshot.borrow_mut() = described;
        self.tree_generation.update(|n| *n = n.wrapping_add(1));
    }
}
