//! Where a byte offset is on screen, and which byte offset is under a finger.
//!
//! [`Paragraph`](crate::Paragraph)'s glyph runs are batched for *drawing*: one
//! run per span agreeing on font, size, colour and direction, which is the
//! largest thing a backend can submit in one call. That is the wrong shape for
//! every editing question. A caret needs to know which line an offset is on, a
//! tap needs the offset nearest a point, and a selection highlight needs the boxes
//! covering a byte range — and all three want the text organised by *line* and
//! keyed by *byte offset*, neither of which a draw batch preserves.
//!
//! So layout builds a second view of the same shaping pass. It costs one vector
//! per line and is built once, at the same time as the runs, from the same
//! `cosmic-text` buffer — which is what guarantees the caret is placed against
//! the layout that was actually drawn rather than a second one that might
//! disagree.
//!
//! # Clusters, not glyphs
//!
//! The unit here is the cluster: the smallest group of bytes that maps to a
//! group of glyphs. One cluster is one caret stop. It is not a `char` (`e` +
//! U+0301 is one cluster and two chars), not a glyph (a ligature is one glyph
//! and several clusters), and not a grapheme (though it usually coincides). It is
//! whatever the shaper says cannot be subdivided, and it is the only unit for
//! which "the caret goes here" has an answer.
//!
//! # Visual order
//!
//! Clusters within a line are stored **sorted by x**, left to right, whatever
//! their logical order. Hit testing walks the line left to right and a selection
//! highlight emits boxes left to right; both are visual questions. The logical
//! order is not lost — it is in the byte ranges, which is where it belongs.

use vieww_foundation::{Affinity, Offset, Rect, TextDirection, TextPosition, TextRange};

/// One caret stop: the bytes it covers and the box it occupies on its line.
#[derive(Debug, Clone)]
pub(crate) struct Cluster {
    /// Byte range in the paragraph's source text — global, not per line.
    pub range: TextRange,
    pub left: f32,
    pub width: f32,
    /// `true` if these bytes were laid out right-to-left, which reverses which
    /// screen edge the range starts at.
    pub rtl: bool,
}

impl Cluster {
    /// The offset at this cluster's **left** edge on screen.
    ///
    /// Not `range.start`: in a right-to-left cluster the text starts at the right
    /// edge, so the left edge is where it ends. Getting this backwards puts every
    /// Arabic caret one cluster out, in the direction that still looks plausible.
    const fn left_offset(&self) -> usize {
        if self.rtl {
            self.range.end
        } else {
            self.range.start
        }
    }

    /// The offset at this cluster's **right** edge on screen.
    const fn right_offset(&self) -> usize {
        if self.rtl {
            self.range.start
        } else {
            self.range.end
        }
    }

    pub(crate) const fn right(&self) -> f32 {
        self.left + self.width
    }
}

/// One laid-out line.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    /// The bytes this line owns, ending where the next line begins.
    ///
    /// A partition rather than "the bytes with glyphs on this line", so that
    /// every offset in the text belongs to exactly one line and the space
    /// swallowed by a soft wrap is not orphaned between two of them.
    pub range: TextRange,
    pub top: f32,
    pub bottom: f32,
    pub baseline: f32,
    /// Clusters sorted left to right.
    pub clusters: Vec<Cluster>,
    /// The inked extent of the line, or the aligned caret position when empty.
    pub left: f32,
    pub right: f32,
    /// The base direction, which decides where a caret sits on an empty line.
    pub rtl: bool,
    /// How many bytes of line break this line's range ends with.
    ///
    /// Zero for a soft wrap — there is no character there, the next line simply
    /// starts — and one or two for a hard break (`\n`, `\r\n`).
    ///
    /// # Why this is counted rather than inferred
    ///
    /// [`line_end`](LineIndex::line_end) is what End does, and End must put the
    /// caret at the *last character of the line*, not past the newline. Without
    /// this the offset it returned was the start of the following line: the
    /// caret visibly jumped down a row, and Home-then-shift-End selected the
    /// line break along with the text — so replacing a selected line also ate
    /// the break and joined it to the next one.
    ///
    /// It cannot be inferred from the range alone, because a soft wrap and a
    /// hard break produce the same shape: two lines whose ranges meet. Only the
    /// layout, which has the source text in hand, can tell them apart, so it
    /// records the answer here.
    pub break_len: usize,
}

impl Line {
    /// The offset nearest `x` on this line.
    fn offset_at(&self, x: f32) -> usize {
        let Some(first) = self.clusters.first() else {
            return self.range.start;
        };
        if x <= first.left {
            return first.left_offset();
        }
        for cluster in &self.clusters {
            if x < cluster.right() {
                // Which half of the cluster was touched decides which of its two
                // ends the caret goes to. Half rather than any other fraction
                // because it is the only split where a caret never appears on the
                // far side of the glyph the finger is over.
                return if x - cluster.left < cluster.width / 2.0 {
                    cluster.left_offset()
                } else {
                    cluster.right_offset()
                };
            }
        }
        self.clusters
            .last()
            .map_or(self.range.end, Cluster::right_offset)
    }

    /// Every place a caret can sit on this line, left to right.
    ///
    /// The list a *visual* left or right arrow walks, and the reason it cannot
    /// be derived from byte offsets: in bidi text the offset one to the left is
    /// not the offset one less. Two adjacent clusters of opposite direction
    /// share a screen edge and two different offsets meet there, which is why
    /// consecutive duplicates are dropped rather than the edges being unique by
    /// construction.
    fn caret_stops(&self) -> Vec<usize> {
        let mut stops = Vec::with_capacity(self.clusters.len() + 1);
        for cluster in &self.clusters {
            stops.push(cluster.left_offset());
            stops.push(cluster.right_offset());
        }
        if stops.is_empty() {
            return vec![self.range.start];
        }
        stops.dedup();
        stops
    }

    /// Where a caret drawn at `offset` sits horizontally on this line.
    fn caret_x(&self, offset: usize) -> f32 {
        // Inside a cluster: the caret goes at the edge the cluster's text begins
        // from, so typing there inserts before it.
        for cluster in &self.clusters {
            if cluster.range.contains(offset) {
                return if cluster.rtl {
                    cluster.right()
                } else {
                    cluster.left
                };
            }
        }
        // Otherwise it sits just after some cluster: take the one whose logical
        // end is nearest below the offset, and use its trailing edge. Nearest
        // rather than last-in-visual-order, because in a bidi line the cluster
        // preceding an offset can be anywhere across the line.
        let after = self
            .clusters
            .iter()
            .filter(|cluster| cluster.range.end <= offset)
            .max_by_key(|cluster| cluster.range.end);
        if let Some(cluster) = after {
            return if cluster.rtl {
                cluster.left
            } else {
                cluster.right()
            };
        }
        // Before everything on the line, or the line is empty.
        if self.rtl {
            self.right
        } else {
            self.left
        }
    }
}

/// Every line of a shaped paragraph, in order.
///
/// Always holds at least one line: an empty paragraph still has somewhere to put
/// a caret, which is the whole reason
/// [`Paragraph::line_count`](crate::Paragraph::line_count) never returns zero.
#[derive(Debug, Clone)]
pub(crate) struct LineIndex {
    lines: Vec<Line>,
}

impl LineIndex {
    pub(crate) fn new(lines: Vec<Line>) -> Self {
        debug_assert!(!lines.is_empty(), "a caret always needs a line to sit on");
        Self { lines }
    }

    pub(crate) fn len(&self) -> usize {
        self.lines.len()
    }

    /// The offset nearest `point`, with the affinity the tap implies.
    pub(crate) fn hit_test(&self, point: Offset) -> TextPosition {
        // The first line the point is above the bottom of. A tap above the
        // paragraph lands on the first line and one below it on the last, rather
        // than being refused: a text field is tapped at its edges constantly, and
        // "no position" is not an answer a caret can use.
        let index = self
            .lines
            .iter()
            .position(|line| point.dy < line.bottom)
            .unwrap_or(self.lines.len() - 1);
        let line = &self.lines[index];
        let offset = line.offset_at(point.dx);

        // Landing on the boundary between two lines means the tap chose which of
        // them the caret belongs to, and it chose the one it was on.
        let affinity = if offset == line.range.end && index + 1 < self.lines.len() {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        TextPosition { offset, affinity }
    }

    /// Which line a position draws on.
    pub(crate) fn line_of(&self, position: TextPosition) -> usize {
        let index = self
            .lines
            .iter()
            .rposition(|line| line.range.start <= position.offset)
            .unwrap_or(0);
        // An offset sitting exactly on a line's start is also the previous line's
        // end. Upstream is what says the caret meant the end of the line before.
        if position.affinity == Affinity::Upstream
            && index > 0
            && self.lines[index].range.start == position.offset
        {
            index - 1
        } else {
            index
        }
    }

    /// The zero-width box a caret occupies, spanning the full line height.
    ///
    /// Zero width because a caret's thickness is a paint decision — it is one
    /// physical pixel on some platforms and two logical ones on others — and
    /// widening it here would bake a device into a layout.
    pub(crate) fn cursor_rect(&self, position: TextPosition) -> Rect {
        let line = &self.lines[self.line_of(position)];
        let x = line.caret_x(position.offset);
        Rect::new(x, line.top, x, line.bottom)
    }

    /// The boxes covering `range`, left to right and top to bottom.
    ///
    /// More than one box per line where the range is visually discontiguous,
    /// which is what a bidi selection is: selecting across the boundary of an
    /// embedded English phrase in Hebrew covers two separate stretches of screen
    /// even though the bytes are contiguous. Returning a single bounding box
    /// would highlight text that is not selected.
    pub(crate) fn highlight(&self, range: TextRange) -> Vec<Rect> {
        if range.is_empty() {
            return Vec::new();
        }
        let mut rects = Vec::new();
        for line in &self.lines {
            let mut span: Option<(f32, f32)> = None;
            for cluster in &line.clusters {
                if cluster.range.intersect(range).is_some() {
                    match &mut span {
                        // Consecutive in visual order means contiguous on screen,
                        // so extending needs no gap test.
                        Some((_, right)) => *right = cluster.right(),
                        None => span = Some((cluster.left, cluster.right())),
                    }
                } else if let Some((left, right)) = span.take() {
                    rects.push(Rect::new(left, line.top, right, line.bottom));
                }
            }
            if let Some((left, right)) = span {
                rects.push(Rect::new(left, line.top, right, line.bottom));
            }
        }
        rects
    }

    /// The start of the line `position` is on.
    pub(crate) fn line_start(&self, position: TextPosition) -> TextPosition {
        TextPosition::new(self.lines[self.line_of(position)].range.start)
    }

    /// The end of the line `position` is on.
    ///
    /// [`Affinity::Upstream`], because the end of a wrapped line is also the
    /// start of the next one and pressing End means the line you were on.
    pub(crate) fn line_end(&self, position: TextPosition) -> TextPosition {
        let index = self.line_of(position);
        let line = &self.lines[index];
        // Past the text but before the break. On a hard-broken line the range
        // includes the `\n`, and stopping after it would put the caret on the
        // *next* line — which is what End used to do.
        // **Never before the line's own start.** A trailing empty line — the one
        // after a string that ends in `\n` — is `27..27` and still carries the
        // break length of the newline that created it, so subtracting it put End
        // at 26: on the *previous* line, and before the same line's Home.
        //
        // A `line_start` after its `line_end` is an inverted range, and anything
        // that builds a selection from the pair slices backwards. Pressing End on
        // the empty last line of a document is not an unusual thing to do.
        let end = line
            .range
            .end
            .saturating_sub(line.break_len)
            .max(line.range.start);
        if line.break_len == 0 && index + 1 < self.lines.len() {
            // A soft wrap: the offset is shared with the start of the next
            // line, and only the affinity says which of the two the caret is
            // on. There is no character to step back over.
            TextPosition::upstream(end)
        } else {
            TextPosition::new(end)
        }
    }

    /// The position `goal_x` across on the line above, or `None` on the first line.
    ///
    /// `goal_x` is the column to aim for rather than the caret's current x, and
    /// the caller is expected to remember it across consecutive presses. Taking
    /// the current x instead is the bug where moving down through a short line
    /// and back up lands somewhere other than where you started.
    /// The caret stop one to the *left* on screen, or `None` at the start of the
    /// first line.
    ///
    /// Visual, not logical. In left-to-right text the two agree and this is an
    /// expensive way to subtract one; in bidi text they do not, and an arrow key
    /// that moved by byte offset jumps across the screen in the middle of a
    /// mixed-direction line — the single most confusing thing a text field can
    /// do to somebody typing Arabic with a number in it.
    pub(crate) fn position_left_of(&self, position: TextPosition) -> Option<TextPosition> {
        self.step(position, -1)
    }

    /// The caret stop one to the *right* on screen, or `None` at the end of the
    /// last line.
    pub(crate) fn position_right_of(&self, position: TextPosition) -> Option<TextPosition> {
        self.step(position, 1)
    }

    /// One caret stop along, `direction` being -1 for left and 1 for right.
    ///
    /// # Why an offset alone cannot say where the caret is
    ///
    /// **At a direction boundary one offset is two caret stops.** The byte where
    /// a Hebrew run meets an English one is both the visual end of the first and
    /// the visual start of the second, and those are in different places on
    /// screen. `caret_stops` lists both, correctly.
    ///
    /// Finding the caret with `position(|&offset| offset == …)` therefore always
    /// picked the *first* of them — and stepping right from the second landed
    /// back at the first's successor. The result was a **cycle**: in
    /// `"\nשלוםword 👩‍👩‍👧"` the right-arrow key walks
    /// `13 → 9 → 10 → 11 → 12 → 13 → 9 …` forever and never reaches the end of
    /// the line. Found by `tests/shaping_properties.rs`, which walks the caret
    /// from one end to the other against randomised strings and asserts the walk
    /// terminates.
    ///
    /// [`Affinity`] is exactly the missing bit and is what it is for: it says
    /// which side of a boundary the caret is on. So a position carrying
    /// [`Upstream`](Affinity::Upstream) means the **earlier** of the duplicate
    /// stops and [`Downstream`](Affinity::Downstream) the later, and every
    /// position this returns is stamped with the affinity that identifies its own
    /// stop. The walk is then monotonic in the stop index, which is what makes it
    /// finite.
    fn step(&self, position: TextPosition, direction: isize) -> Option<TextPosition> {
        let index = self.line_of(position);
        let line = self.lines.get(index)?;
        let stops = line.caret_stops();

        let at = self.stop_index(&stops, position)?;
        let next = at.checked_add_signed(direction)?;
        if next < stops.len() {
            return Some(Self::at_stop(&stops, next));
        }

        // Off the end of the line: on to the next one, entering from the edge
        // the movement arrived at.
        let neighbour = index.checked_add_signed(direction)?;
        let neighbour = self.lines.get(neighbour)?;
        let stops = neighbour.caret_stops();
        if stops.is_empty() {
            return None;
        }
        let entering = if direction < 0 { stops.len() - 1 } else { 0 };
        Some(Self::at_stop(&stops, entering))
    }

    /// Which stop a position is on, disambiguated by affinity.
    ///
    /// See [`step`](Self::step) for why an offset alone is not enough.
    fn stop_index(&self, stops: &[usize], position: TextPosition) -> Option<usize> {
        let matching: Vec<usize> = stops
            .iter()
            .enumerate()
            .filter(|(_, &offset)| offset == position.offset)
            .map(|(index, _)| index)
            .collect();

        match matching.len() {
            0 => stops
                // Not on a stop at all — a caret inside a cluster, which happens
                // after an edit that split one. The nearest stop is the honest
                // place to start from.
                .iter()
                .enumerate()
                .min_by_key(|(_, &offset)| offset.abs_diff(position.offset))
                .map(|(index, _)| index),
            1 => Some(matching[0]),
            // A boundary. Upstream is the earlier stop, downstream the later.
            _ if position.affinity == Affinity::Upstream => Some(matching[0]),
            _ => matching.last().copied(),
        }
    }

    /// The position for a stop, stamped with the affinity that identifies it.
    ///
    /// A stop whose offset is unique needs no affinity to be found again, and
    /// gets `Downstream` — the default, and what every existing caller passes.
    /// One of a duplicated pair gets whichever affinity points back at itself,
    /// so the next step resolves to the same stop rather than to its twin.
    fn at_stop(stops: &[usize], index: usize) -> TextPosition {
        let offset = stops[index];
        let duplicated = stops
            .iter()
            .enumerate()
            .any(|(other, &value)| other != index && value == offset);
        if duplicated && stops[..index].contains(&offset) {
            TextPosition::new(offset)
        } else if duplicated {
            TextPosition::upstream(offset)
        } else {
            TextPosition::new(offset)
        }
    }

    pub(crate) fn position_above(
        &self,
        position: TextPosition,
        goal_x: f32,
    ) -> Option<TextPosition> {
        let index = self.line_of(position);
        self.position_on(index.checked_sub(1)?, goal_x)
    }

    /// The position `goal_x` across on the line below, or `None` on the last line.
    pub(crate) fn position_below(
        &self,
        position: TextPosition,
        goal_x: f32,
    ) -> Option<TextPosition> {
        let index = self.line_of(position) + 1;
        (index < self.lines.len()).then(|| self.position_on(index, goal_x))?
    }

    fn position_on(&self, index: usize, goal_x: f32) -> Option<TextPosition> {
        let line = self.lines.get(index)?;
        let offset = line.offset_at(goal_x);
        let affinity = if offset == line.range.end && index + 1 < self.lines.len() {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        Some(TextPosition { offset, affinity })
    }

    /// The full-width box of line `index`, for painting a line highlight.
    pub(crate) fn line_rect(&self, index: usize) -> Option<Rect> {
        let line = self.lines.get(index)?;
        Some(Rect::new(line.left, line.top, line.right, line.bottom))
    }

    /// The baseline of line `index`.
    pub(crate) fn baseline(&self, index: usize) -> Option<f32> {
        self.lines.get(index).map(|line| line.baseline)
    }
}

/// A single empty line, for a paragraph that shaped to nothing.
///
/// `cosmic-text` reports no layout runs for empty text, and a paragraph with no
/// lines has nowhere to draw a caret — which is exactly the state an empty text
/// field is in, so it is the common case rather than an edge one.
pub(crate) fn empty_line(height: f32, ascent: f32, x: f32, direction: TextDirection) -> Line {
    Line {
        range: TextRange::EMPTY,
        top: 0.0,
        bottom: height,
        baseline: ascent,
        clusters: Vec::new(),
        left: x,
        right: x,
        rtl: direction.is_rtl(),
        break_len: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a line of single-byte, fixed-width clusters starting at `start`.
    fn ltr_line(start: usize, count: usize, top: f32) -> Line {
        let clusters = (0..count)
            .map(|i| Cluster {
                range: TextRange::new(start + i, start + i + 1),
                left: i as f32 * 10.0,
                width: 10.0,
                rtl: false,
            })
            .collect();
        Line {
            range: TextRange::new(start, start + count),
            top,
            bottom: top + 20.0,
            baseline: top + 16.0,
            clusters,
            left: 0.0,
            right: count as f32 * 10.0,
            rtl: false,
            break_len: 0,
        }
    }

    fn index(lines: Vec<Line>) -> LineIndex {
        LineIndex::new(lines)
    }

    #[test]
    fn a_tap_in_the_left_half_of_a_cluster_puts_the_caret_before_it() {
        let index = index(vec![ltr_line(0, 3, 0.0)]);

        assert_eq!(
            index.hit_test(Offset::new(11.0, 5.0)).offset,
            1,
            "just past"
        );
        assert_eq!(
            index.hit_test(Offset::new(19.0, 5.0)).offset,
            2,
            "past the midpoint belongs to the next stop"
        );
    }

    #[test]
    fn a_tap_before_or_after_a_line_clamps_to_its_ends() {
        let index = index(vec![ltr_line(0, 3, 0.0)]);

        assert_eq!(index.hit_test(Offset::new(-50.0, 5.0)).offset, 0);
        assert_eq!(
            index.hit_test(Offset::new(500.0, 5.0)).offset,
            3,
            "tapping the empty space after a line means the end of it"
        );
    }

    #[test]
    fn a_tap_above_or_below_the_paragraph_lands_on_the_nearest_line() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);

        assert_eq!(index.hit_test(Offset::new(2.0, -100.0)).offset, 0);
        assert_eq!(
            index.hit_test(Offset::new(500.0, 900.0)).offset,
            6,
            "a text field is tapped at its edges constantly; refusing is not an answer"
        );
    }

    #[test]
    fn a_tap_at_the_end_of_a_wrapped_line_is_upstream_of_the_next_ones_start() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);
        let position = index.hit_test(Offset::new(500.0, 5.0));

        assert_eq!(position.offset, 3);
        assert_eq!(
            position.affinity,
            Affinity::Upstream,
            "offset 3 is both lines; the tap says which"
        );
        assert_eq!(
            index.line_of(position),
            0,
            "and the caret draws on the line that was tapped"
        );
    }

    #[test]
    fn the_same_offset_downstream_draws_on_the_following_line() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);
        let upstream = index.cursor_rect(TextPosition::upstream(3));
        let downstream = index.cursor_rect(TextPosition::new(3));

        assert_eq!(upstream.top, 0.0, "{upstream}");
        assert_eq!(downstream.top, 20.0, "{downstream}");
        assert_eq!(upstream.left, 30.0, "the end of the first line");
        assert_eq!(downstream.left, 0.0, "the start of the second");
    }

    #[test]
    fn a_caret_is_zero_width_and_spans_the_line() {
        let index = index(vec![ltr_line(0, 3, 0.0)]);
        let rect = index.cursor_rect(TextPosition::new(1));

        assert_eq!(rect.left, 10.0);
        assert_eq!(
            rect.width(),
            0.0,
            "thickness is a paint decision, not a layout one"
        );
        assert_eq!((rect.top, rect.bottom), (0.0, 20.0));
    }

    #[test]
    fn a_caret_at_the_end_of_the_text_sits_after_the_last_cluster() {
        let index = index(vec![ltr_line(0, 3, 0.0)]);
        assert_eq!(index.cursor_rect(TextPosition::new(3)).left, 30.0);
    }

    #[test]
    fn a_caret_on_an_empty_line_sits_at_the_aligned_origin() {
        let mut line = empty_line(20.0, 16.0, 100.0, TextDirection::Ltr);
        line.range = TextRange::EMPTY;
        let index = index(vec![line]);

        assert_eq!(
            index.cursor_rect(TextPosition::new(0)).left,
            100.0,
            "a centred empty field puts its caret in the middle, not at the left"
        );
    }

    // ------------------------------------------------------------ right to left

    /// One line of RTL clusters: logical order runs right to left on screen.
    fn rtl_line(count: usize) -> Line {
        let clusters = (0..count)
            .map(|i| Cluster {
                range: TextRange::new(i, i + 1),
                // Cluster 0 is the *rightmost*, so its left edge is furthest right.
                left: (count - 1 - i) as f32 * 10.0,
                width: 10.0,
                rtl: true,
            })
            // Stored left to right, so reverse the logical order.
            .rev()
            .collect();
        Line {
            range: TextRange::new(0, count),
            top: 0.0,
            bottom: 20.0,
            baseline: 16.0,
            clusters,
            left: 0.0,
            right: count as f32 * 10.0,
            rtl: true,
            break_len: 0,
        }
    }

    #[test]
    fn a_caret_in_right_to_left_text_sits_on_the_right_of_its_cluster() {
        let index = index(vec![rtl_line(3)]);

        // Offset 0 is the first logical character, which is the rightmost glyph;
        // the caret goes before it, meaning to its right.
        assert_eq!(index.cursor_rect(TextPosition::new(0)).left, 30.0);
        assert_eq!(index.cursor_rect(TextPosition::new(1)).left, 20.0);
        assert_eq!(
            index.cursor_rect(TextPosition::new(3)).left,
            0.0,
            "the end of the text is the far left"
        );
    }

    #[test]
    fn a_tap_on_the_right_of_right_to_left_text_selects_its_start() {
        let index = index(vec![rtl_line(3)]);

        assert_eq!(
            index.hit_test(Offset::new(29.0, 5.0)).offset,
            0,
            "the rightmost glyph is the first character, so tapping past it is offset 0"
        );
        assert_eq!(index.hit_test(Offset::new(1.0, 5.0)).offset, 3);
    }

    // ------------------------------------------------------------- highlighting

    #[test]
    fn a_collapsed_range_highlights_nothing() {
        let index = index(vec![ltr_line(0, 3, 0.0)]);
        assert!(index.highlight(TextRange::collapsed(1)).is_empty());
    }

    #[test]
    fn a_highlight_covers_exactly_the_selected_clusters() {
        let index = index(vec![ltr_line(0, 4, 0.0)]);
        let rects = index.highlight(TextRange::new(1, 3));

        assert_eq!(rects.len(), 1);
        assert_eq!((rects[0].left, rects[0].right), (10.0, 30.0));
        assert_eq!((rects[0].top, rects[0].bottom), (0.0, 20.0));
    }

    #[test]
    fn a_highlight_across_lines_produces_one_box_per_line() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);
        let rects = index.highlight(TextRange::new(2, 4));

        assert_eq!(rects.len(), 2, "{rects:?}");
        assert_eq!((rects[0].left, rects[0].right), (20.0, 30.0));
        assert_eq!((rects[1].left, rects[1].right), (0.0, 10.0));
    }

    #[test]
    fn a_visually_discontiguous_selection_produces_separate_boxes() {
        // "AB" then Hebrew "‏גב‏" then "CD" — the middle runs the other way, so a
        // byte range covering the A and the first Hebrew letter is two stretches
        // of screen with an unselected one between them.
        let line = Line {
            range: TextRange::new(0, 6),
            top: 0.0,
            bottom: 20.0,
            baseline: 16.0,
            clusters: vec![
                Cluster {
                    range: TextRange::new(0, 1),
                    left: 0.0,
                    width: 10.0,
                    rtl: false,
                },
                // The RTL pair sits reversed: bytes 3..4 draw left of 2..3.
                Cluster {
                    range: TextRange::new(3, 4),
                    left: 10.0,
                    width: 10.0,
                    rtl: true,
                },
                Cluster {
                    range: TextRange::new(2, 3),
                    left: 20.0,
                    width: 10.0,
                    rtl: true,
                },
                Cluster {
                    range: TextRange::new(4, 5),
                    left: 30.0,
                    width: 10.0,
                    rtl: false,
                },
            ],
            left: 0.0,
            right: 40.0,
            rtl: false,
            break_len: 0,
        };
        let rects = index(vec![line]).highlight(TextRange::new(0, 3));

        assert_eq!(
            rects.len(),
            2,
            "one box would paint over the unselected letter between them: {rects:?}"
        );
        assert_eq!((rects[0].left, rects[0].right), (0.0, 10.0));
        assert_eq!((rects[1].left, rects[1].right), (20.0, 30.0));
    }

    // ------------------------------------------------------- vertical movement

    #[test]
    fn moving_up_and_down_keeps_the_column() {
        let index = index(vec![
            ltr_line(0, 5, 0.0),
            ltr_line(5, 2, 20.0),
            ltr_line(7, 5, 40.0),
        ]);

        // Start at column 40 on the first line, aim for it on the way down.
        let goal = index.cursor_rect(TextPosition::new(4)).left;
        assert_eq!(goal, 40.0);

        let middle = index.position_below(TextPosition::new(4), goal).unwrap();
        assert_eq!(middle.offset, 7, "the short line clamps to its end");

        let bottom = index.position_below(middle, goal).unwrap();
        assert_eq!(
            index.cursor_rect(bottom).left,
            40.0,
            "the remembered column is recovered on a line long enough to hold it"
        );
    }

    #[test]
    fn moving_off_either_end_of_the_paragraph_reports_nothing() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);

        assert_eq!(
            index.position_above(TextPosition::new(1), 0.0),
            None,
            "the widget needs to know to move focus rather than the caret"
        );
        assert_eq!(index.position_below(TextPosition::new(4), 0.0), None);
    }

    #[test]
    fn home_and_end_stay_on_the_line_the_caret_is_on() {
        let index = index(vec![ltr_line(0, 3, 0.0), ltr_line(3, 3, 20.0)]);
        let caret = TextPosition::new(4);

        assert_eq!(index.line_start(caret).offset, 3);

        let end = index.line_end(caret);
        assert_eq!(end.offset, 6);
        assert_eq!(index.line_of(end), 1);

        // On the wrapped line above, End must not jump the caret to the next line.
        let end = index.line_end(TextPosition::new(1));
        assert_eq!(end.offset, 3);
        assert_eq!(
            end.affinity,
            Affinity::Upstream,
            "downstream would draw the caret at the start of the line below"
        );
        assert_eq!(index.line_of(end), 0);
    }
}
