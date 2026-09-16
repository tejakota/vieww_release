//! Laying a styled string out into lines of positioned glyphs.

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Style, Weight, Wrap};
use vieww_foundation::{
    FontFamily, Glyph, GlyphRun, Offset, Rect, Size, TextAlign, TextDirection, TextPosition,
    TextRange, TextStyle,
};

use crate::lines::{empty_line, Cluster, Line, LineIndex};
use crate::FontStore;

/// A span of text sharing one style.
///
/// A paragraph is a list of these, which is what makes "bold mid-sentence"
/// expressible: the run boundary is where the style changes, not where the
/// sentence does.
#[derive(Debug, Clone, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub style: TextStyle,
}

impl TextSpan {
    #[must_use]
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Text laid out into lines of positioned glyphs.
///
/// The output of the text layer and the input to the paint layer. Immutable:
/// re-layout produces a new one rather than mutating this, so a measured
/// paragraph can be held across a frame without its geometry changing underneath.
#[derive(Debug, Clone)]
pub struct Paragraph {
    runs: Vec<GlyphRun>,
    size: Size,
    direction: TextDirection,
    /// The same shaping pass, organised by line and keyed by byte offset — which
    /// is the shape every editing question needs and a draw batch destroys. See
    /// [`crate::lines`].
    lines: LineIndex,
    /// The widest line **before** it was clamped to what this was laid out
    /// against.
    ///
    /// `size().width` is deliberately clamped, so a caller can trust that what
    /// it reports fits inside the width it asked for. The unclamped figure is a
    /// different measurement and the one an intrinsic minimum needs: shaping at
    /// a width of zero wraps as hard as the shaper can, and the widest line it
    /// still produces is the longest unbreakable word. Read through `size()`
    /// that answer is zero, because zero is what it was clamped to — which is
    /// how `RenderText`'s minimum-width intrinsic first came out as nothing at
    /// all.
    widest: f32,
}

impl Paragraph {
    /// Lay out `spans`, wrapping at `max_width`.
    ///
    /// Pass [`f32::INFINITY`] for `max_width` to lay out on a single line, which
    /// is what an intrinsic-width measurement wants.
    #[must_use]
    pub fn layout(store: &mut FontStore, spans: &[TextSpan], max_width: f32) -> Self {
        Self::layout_aligned(store, spans, max_width, TextAlign::Start, None)
    }

    /// Lay out `spans` with an explicit alignment and base direction.
    ///
    /// `direction` of `None` means "infer from the content", which is what a UI
    /// should normally do — a Hebrew string should read right-to-left without the
    /// application having to say so. Pass it explicitly when the content cannot
    /// decide, which is the empty and digits-only cases.
    #[must_use]
    pub fn layout_aligned(
        store: &mut FontStore,
        spans: &[TextSpan],
        max_width: f32,
        align: TextAlign,
        direction: Option<TextDirection>,
    ) -> Self {
        let text: String = spans.iter().map(|span| span.text.as_str()).collect();
        let base = direction.unwrap_or_else(|| detect_direction(&text));

        // Checklist item 6. `base` rather than `direction` is what goes in the
        // key: an inferred direction and the same direction passed explicitly
        // produce the same paragraph, and keying on the argument would shape it
        // twice. See `crate::shape_cache`.
        let key = crate::shape_cache::Key::new(spans, max_width, align, base);
        if let Some(cached) = store.shapes_mut().get(&key) {
            return cached;
        }

        // One font size and one line height for the whole paragraph, both taken
        // from the largest span. They have to be uniform or lines would overlap
        // wherever a style changed mid-line, and the largest rather than the
        // first so that nothing is clipped.
        //
        // The two are maximised *independently*: the tallest line box need not
        // belong to the largest text — a small span asking for double spacing
        // wants its space, and taking the line height of whichever span happened
        // to be biggest would silently drop it.
        let nominal = spans.first().map_or(14.0, |span| span.style.size);
        let leading = spans
            .iter()
            .map(|span| span.style.size)
            .fold(nominal, f32::max);
        let line = spans
            .iter()
            .map(|span| span.style.line_extent())
            .fold(nominal * TextStyle::NORMAL_LINE_HEIGHT, f32::max);
        let metrics = Metrics::new(leading, line);

        let mut buffer = Buffer::new(store.system_mut(), metrics);
        buffer.set_size(max_width.is_finite().then_some(max_width), None);
        buffer.set_wrap(if max_width.is_finite() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });

        // Alignment is `cosmic-text`'s job, not ours: it has to be applied per
        // line, after line breaking, and it interacts with bidi reordering. Doing
        // it here by shifting run origins would get both wrong.
        let alignment = to_align(align, base);
        let default_style = spans
            .first()
            .map_or_else(TextStyle::default, |span| span.style);
        let default = to_attrs(&default_style);

        if text.is_empty() {
            buffer.set_text("", &default, Shaping::Advanced, alignment);
        } else {
            // Each span carries its own attributes, which is what makes a style
            // change mid-sentence a run boundary rather than a re-layout.
            let styled: Vec<(&str, Attrs<'_>)> = spans
                .iter()
                .map(|span| (span.text.as_str(), to_attrs(&span.style)))
                .collect();
            buffer.set_rich_text(styled, &default, Shaping::Advanced, alignment);
        }
        buffer.shape_until_scroll(store.system_mut(), false);

        let paragraph = Self::from_buffer(store, &buffer, spans, &text, max_width, align, base);
        store.shapes_mut().insert(key, paragraph.clone());
        paragraph
    }

    /// Read a shaped `cosmic-text` buffer out into glyph runs.
    ///
    /// One run per contiguous span of glyphs agreeing on everything a backend
    /// batches by: font, size, colour, direction and baseline. A style change, a
    /// script change that forces a font fallback, or a direction change all end
    /// the current run.
    fn from_buffer(
        store: &mut FontStore,
        buffer: &Buffer,
        spans: &[TextSpan],
        text: &str,
        max_width: f32,
        align: TextAlign,
        base: TextDirection,
    ) -> Self {
        struct Pending {
            font_id: cosmic_text::fontdb::ID,
            weight: cosmic_text::fontdb::Weight,
            size: f32,
            color: vieww_foundation::Color,
            rtl: bool,
            baseline: f32,
            ascent: f32,
            descent: f32,
            /// Absolute x within the paragraph; normalised against the run origin
            /// once the run is closed.
            glyphs: Vec<(u16, f32, f32)>,
        }

        // `LayoutGlyph::start` and `end` are offsets into the *buffer line* they
        // belong to, and a buffer line is one hard-broken paragraph. So they
        // restart at zero after every newline, and everything downstream — the
        // style lookup, every caret offset — needs them against the whole string
        // instead. This is the table that translates.
        let mut line_starts = Vec::with_capacity(buffer.lines.len());
        let mut cursor = 0_usize;
        for line in &buffer.lines {
            line_starts.push(cursor);
            cursor += line.text().len() + line.ending().as_str().len();
        }

        let mut pending: Vec<Pending> = Vec::new();
        let mut lines: Vec<Line> = Vec::new();
        let mut widest = 0.0_f32;
        let mut total_height = 0.0_f32;
        // Kept for the trailing-newline line below, which has no run of its own to
        // take a line box from.
        let mut last_box: Option<(f32, f32)> = None;

        for run in buffer.layout_runs() {
            widest = widest.max(run.line_w);
            total_height = total_height.max(run.line_top + run.line_height);

            // `line_y` is the baseline relative to the top of the buffer, so the
            // ascent and descent fall out of the line box around it.
            let baseline = run.line_y;
            let ascent = baseline - run.line_top;
            let descent = (run.line_top + run.line_height) - baseline;
            let line_base = line_starts.get(run.line_i).copied().unwrap_or(0);

            // One layout run is one *visual* line: a wrapped paragraph produces
            // several of them sharing a `line_i`.
            let mut clusters: Vec<Cluster> = Vec::new();

            for glyph in run.glyphs {
                let range = TextRange::new(line_base + glyph.start, line_base + glyph.end);
                let color = style_at(spans, range.start).color;
                let rtl = glyph.level.is_rtl();

                // Several glyphs can share one byte range — a base plus its
                // combining marks — and they are one caret stop, so they are one
                // cluster covering the union of their boxes.
                match clusters.last_mut() {
                    Some(last) if last.range == range => {
                        let right = last.right().max(glyph.x + glyph.w);
                        last.left = last.left.min(glyph.x);
                        last.width = right - last.left;
                    }
                    _ => clusters.push(Cluster {
                        range,
                        left: glyph.x,
                        width: glyph.w,
                        rtl,
                    }),
                }

                let extend = pending.last().is_some_and(|last| {
                    last.font_id == glyph.font_id
                        && (last.size - glyph.font_size).abs() < f32::EPSILON
                        && last.color == color
                        && last.rtl == rtl
                        && (last.baseline - baseline).abs() < f32::EPSILON
                });
                if !extend {
                    pending.push(Pending {
                        font_id: glyph.font_id,
                        weight: glyph.font_weight,
                        size: glyph.font_size,
                        color,
                        rtl,
                        baseline,
                        ascent,
                        descent,
                        glyphs: Vec::new(),
                    });
                }
                let current = pending.last_mut().expect("just pushed");
                current.glyphs.push((
                    glyph.glyph_id,
                    glyph.x + glyph.x_offset,
                    glyph.y + glyph.y_offset,
                ));
            }

            // Visual order, whatever order the shaper emitted them in. Sorting
            // rather than trusting `glyphs` because that order is the shaper's
            // business and hit testing walking a line left to right is not.
            clusters.sort_by(|a, b| a.left.total_cmp(&b.left));

            let start = clusters
                .iter()
                .map(|cluster| cluster.range.start)
                .min()
                .unwrap_or(line_base);
            let (left, right) = clusters.iter().fold(
                (f32::INFINITY, f32::NEG_INFINITY),
                |(left, right), cluster| (left.min(cluster.left), right.max(cluster.right())),
            );
            let aligned = align.leading_fraction(base)
                * if max_width.is_finite() {
                    max_width - run.line_w
                } else {
                    0.0
                };

            lines.push(Line {
                // Provisional: made a partition once every line is known.
                range: TextRange::collapsed(start),
                top: run.line_top,
                bottom: run.line_top + run.line_height,
                baseline,
                left: if left.is_finite() { left } else { aligned },
                right: if right.is_finite() { right } else { aligned },
                clusters,
                rtl: run.rtl,
                // Filled in with the partition below, which is the first point
                // at which this line's end is known.
                break_len: 0,
            });
            last_box = Some((run.line_height, baseline - run.line_top));
        }

        // A trailing newline opens a line, and `cosmic-text` does not give us one.
        //
        // It stores a line break as the *ending* of the line it terminates rather
        // than as a separator between two, so `"one\n"` is a single `BufferLine`
        // and `layout_runs` yields one run — the same as `"one"`. That is
        // reasonable for laying out a document and wrong for editing one: pressing
        // Enter at the end of a field has to put the caret somewhere, and with no
        // second line it is drawn at the end of the first, on top of the text it
        // was supposed to move past. The field does not grow either, so even a
        // correctly placed caret would be outside it.
        //
        // Only the *final* break needs this. Every interior one already separates
        // two runs, which is why `explicit_newlines_break_lines` passed throughout
        // and no test could see this: they all put text on both sides of a break.
        //
        // The line is empty, so it has no glyphs to take a box from — it borrows
        // the last real line's, which is the same font and size a caret would be
        // drawn at anyway.
        if text.ends_with('\n') {
            let (line_height, ascent) = last_box.unwrap_or_else(|| {
                let extent = spans
                    .first()
                    .map_or(14.0 * TextStyle::NORMAL_LINE_HEIGHT, |span| {
                        span.style.line_extent()
                    });
                (extent, extent)
            });
            // No glyphs means no inked extent, so the caret sits at the aligned
            // origin — the same fallback an empty line takes above, and with a
            // width of zero the alignment is against the full measure.
            let aligned = align.leading_fraction(base)
                * if max_width.is_finite() {
                    max_width
                } else {
                    0.0
                };
            let top = total_height;

            lines.push(Line {
                range: TextRange::collapsed(text.len()),
                top,
                bottom: top + line_height,
                baseline: top + ascent,
                left: aligned,
                right: aligned,
                clusters: Vec::new(),
                rtl: base.is_rtl(),
                break_len: 0,
            });
            total_height = top + line_height;
        }

        // Each line owns everything up to where the next one starts, so no offset
        // falls between two lines — in particular the space a soft wrap swallows,
        // which has no glyph and would otherwise belong to neither.
        for index in 0..lines.len() {
            let end = lines
                .get(index + 1)
                .map_or(text.len(), |next| next.range.start);
            lines[index].range = TextRange::new(lines[index].range.start, end);
            // And how much of that end is line break rather than text. A soft
            // wrap has none: the next line just starts. See `Line::break_len`.
            //
            // All three cases are real. `\r\n` when the whole pair is inside
            // this line's range; a bare `\n` for a Unix break; and a bare `\r`
            // because the shaper breaks a Windows file *between* the two bytes,
            // leaving the carriage return on the line above and the line feed
            // starting the next. Missing that last one put the caret between
            // the two halves of one line break.
            let owned = &text[..end];
            lines[index].break_len = if owned.ends_with("\r\n") {
                2
            } else if owned.ends_with('\n') || owned.ends_with('\r') {
                1
            } else {
                0
            };
        }

        let line_count = lines.len();

        let mut runs = Vec::with_capacity(pending.len());
        for item in pending {
            let Some(font) = store.font_data(item.font_id, item.weight) else {
                // Unreachable while the store outlives the layout, which it always
                // does; a glyph whose font vanished simply cannot be drawn.
                continue;
            };

            // The run's origin is its leftmost glyph, with glyph offsets relative
            // to it — so `origin` is meaningful for hit testing and caret
            // placement rather than always being the paragraph's corner.
            let left = item
                .glyphs
                .iter()
                .map(|&(_, x, _)| x)
                .fold(f32::INFINITY, f32::min);
            let left = if left.is_finite() { left } else { 0.0 };

            runs.push(GlyphRun {
                font,
                size: item.size,
                color: item.color,
                origin: Offset::new(left, item.baseline),
                glyphs: item
                    .glyphs
                    .iter()
                    .map(|&(id, x, y)| Glyph::new(id, Offset::new(x - left, y)))
                    .collect(),
                is_rtl: item.rtl,
                ascent: item.ascent,
                descent: item.descent,
            });
        }

        let height = if line_count == 0 {
            spans
                .first()
                .map_or(14.0 * TextStyle::NORMAL_LINE_HEIGHT, |span| {
                    span.style.line_extent()
                })
        } else {
            total_height
        };
        // Width is the widest line — an intrinsic measurement — clamped to what it
        // was laid out against so a caller can trust it fits.
        let width = if max_width.is_finite() {
            widest.min(max_width)
        } else {
            widest
        };

        if lines.is_empty() {
            // Text that shaped to nothing at all. An empty field is in exactly
            // this state and is still tapped and typed into, so it gets a line
            // rather than no line — the alternative is every caret query on an
            // empty field being a special case at every call site.
            let x = align.leading_fraction(base)
                * if max_width.is_finite() {
                    max_width
                } else {
                    0.0
                };
            // The ascent of a line with nothing on it: the caret has to be
            // somewhere, and the font's own is unavailable because no glyph was
            // shaped to ask. The line box scaled back by the nominal ratio is
            // the same answer every empty field has always had.
            let ascent = height / TextStyle::NORMAL_LINE_HEIGHT;
            lines.push(empty_line(height, ascent, x, base));
        }

        Self {
            runs,
            size: Size::new(width, height),
            direction: base,
            lines: LineIndex::new(lines),
            widest,
        }
    }

    /// The widest line, **unclamped** by the width this was laid out against.
    ///
    /// See the field's documentation for why this is not `size().width`.
    #[must_use]
    pub const fn widest_line(&self) -> f32 {
        self.widest
    }

    /// The glyph runs to draw, in order.
    #[must_use]
    pub fn runs(&self) -> &[GlyphRun] {
        &self.runs
    }

    /// The space this paragraph occupies.
    ///
    /// Width is the widest line, not the width it was laid out against — an
    /// intrinsic measurement, which is what a layout parent needs.
    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    /// How many lines the text broke into. At least 1, even when empty.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The base direction used, whether given or inferred.
    #[must_use]
    pub const fn direction(&self) -> TextDirection {
        self.direction
    }

    /// `true` if nothing would be drawn.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(GlyphRun::is_empty)
    }

    /// Total glyphs across every run.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.runs.iter().map(|run| run.glyphs.len()).sum()
    }

    // ------------------------------------------------------------------ editing
    //
    // Everything below answers a question about *position* rather than about
    // drawing, and all of it is served from the line index built during layout —
    // never from a second shaping pass. A caret placed against a different layout
    // than the one on screen lands in the wrong place by exactly the amount the
    // two disagree, which is the bug that is impossible to see and impossible to
    // reproduce.

    /// The offset nearest `point`, in this paragraph's own coordinates.
    ///
    /// Never fails: a point outside the paragraph clamps to the nearest line and
    /// the nearest end of it. A text field is tapped at its edges constantly, and
    /// "no position" is not something a caret can be put at.
    #[must_use]
    pub fn hit_test(&self, point: Offset) -> TextPosition {
        self.lines.hit_test(point)
    }

    /// The zero-width box a caret at `position` occupies, spanning its line.
    ///
    /// Zero width because thickness is a paint decision — one physical pixel on
    /// some platforms, two logical ones on others — and a layout that chose one
    /// would have baked a device into itself.
    #[must_use]
    pub fn cursor_rect(&self, position: TextPosition) -> Rect {
        self.lines.cursor_rect(position)
    }

    /// The boxes to paint behind a selection of `range`.
    ///
    /// One per line, and more than one on a line whose selection is visually
    /// discontiguous — which is what selecting across the edge of an embedded
    /// left-to-right phrase in right-to-left text produces. A single bounding box
    /// would highlight text that is not selected.
    #[must_use]
    pub fn selection_rects(&self, range: TextRange) -> Vec<Rect> {
        self.lines.highlight(range)
    }

    /// Which line `position` draws on, honouring its affinity.
    #[must_use]
    pub fn line_of(&self, position: TextPosition) -> usize {
        self.lines.line_of(position)
    }

    /// The start of the line `position` is on. What Home does.
    #[must_use]
    pub fn line_start(&self, position: TextPosition) -> TextPosition {
        self.lines.line_start(position)
    }

    /// The end of the line `position` is on. What End does.
    #[must_use]
    pub fn line_end(&self, position: TextPosition) -> TextPosition {
        self.lines.line_end(position)
    }

    /// The caret stop one to the left on screen, or `None` at the very start.
    ///
    /// Visual movement, which is what an arrow key means. See
    /// `lines.rs` for how the caret stops on a line are enumerated.
    #[must_use]
    pub fn position_left_of(&self, position: TextPosition) -> Option<TextPosition> {
        self.lines.position_left_of(position)
    }

    /// The caret stop one to the right on screen, or `None` at the very end.
    #[must_use]
    pub fn position_right_of(&self, position: TextPosition) -> Option<TextPosition> {
        self.lines.position_right_of(position)
    }

    /// The position `goal_x` across on the line above, or `None` on the first line.
    ///
    /// `goal_x` is the column to aim for, not the caret's current x, and the
    /// caller keeps it across consecutive presses — the caret's own x is only the
    /// right answer for the first one. Reading it fresh each time is the bug
    /// where moving down through a short line and back up does not return you to
    /// where you started.
    ///
    /// `None` means there is no line above, which is how a widget knows to move
    /// focus rather than the caret.
    #[must_use]
    pub fn position_above(&self, position: TextPosition, goal_x: f32) -> Option<TextPosition> {
        self.lines.position_above(position, goal_x)
    }

    /// The position `goal_x` across on the line below, or `None` on the last line.
    #[must_use]
    pub fn position_below(&self, position: TextPosition, goal_x: f32) -> Option<TextPosition> {
        self.lines.position_below(position, goal_x)
    }

    /// The box line `index` occupies, or `None` past the end.
    #[must_use]
    pub fn line_rect(&self, index: usize) -> Option<Rect> {
        self.lines.line_rect(index)
    }

    /// The baseline of line `index`, or `None` past the end.
    #[must_use]
    pub fn line_baseline(&self, index: usize) -> Option<f32> {
        self.lines.baseline(index)
    }
}

/// The base direction implied by the first strongly-directional character.
///
/// This is the Unicode Bidirectional Algorithm's rule P2/P3, reduced to the
/// scripts that matter here. `cosmic-text` resolves per-run direction properly
/// via `unicode-bidi`; this decides only the paragraph-level fallback, which
/// affects where [`TextAlign::Start`] points.
fn detect_direction(text: &str) -> TextDirection {
    for ch in text.chars() {
        match ch as u32 {
            // Hebrew, Arabic, Syriac, Thaana, and the Arabic supplements.
            0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF => {
                return TextDirection::Rtl;
            }
            // Latin, Greek, Cyrillic, Armenian — strongly left-to-right.
            0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x058F => {
                return TextDirection::Ltr;
            }
            _ => {}
        }
    }
    TextDirection::Ltr
}

/// The style covering a byte offset in the concatenated text.
fn style_at(spans: &[TextSpan], offset: usize) -> TextStyle {
    let mut cursor = 0;
    for span in spans {
        let end = cursor + span.text.len();
        if offset < end {
            return span.style;
        }
        cursor = end;
    }
    spans.last().map_or_else(TextStyle::default, |s| s.style)
}

fn to_attrs(style: &TextStyle) -> Attrs<'static> {
    let mut attrs = Attrs::new()
        .family(to_family(style.family))
        .weight(Weight(style.weight.value()))
        // The span's own size and line box, rather than inheriting the
        // buffer's. Without this every span is drawn at the buffer's size —
        // which is the *largest* span's — so one 24pt word in a 14pt sentence
        // silently promoted the whole sentence to 24pt.
        //
        // `cosmic-text` is built for this: `metrics_opt` is a per-glyph
        // override it consults when placing (`shape.rs`, "use overridden font
        // size"), and each line takes the tallest box on it, so a mixed-size
        // line grows rather than overlapping.
        .metrics(Metrics::new(style.size, style.line_extent()));
    if style.italic {
        attrs = attrs.style(Style::Italic);
    }
    if style.letter_spacing != 0.0 && style.size > 0.0 {
        // The shaper's, not ours. Adding it to advances afterwards would leave
        // line breaking measuring one width and the caret walking another.
        //
        // **Divided by the size, because `cosmic-text` counts in ems here.** It
        // adds this to an advance already divided by the font's units-per-em,
        // and multiplies the sum by the font size when placing the glyph — so a
        // value handed over unconverted is scaled by the size.
        // `TextStyle::letter_spacing` is documented in pixels, and the theme's
        // -0.25 on a 24pt headline was tightening by six pixels a glyph, which
        // turned "Create an account" into a row of overlapping letters.
        attrs = attrs.letter_spacing(style.letter_spacing / style.size);
    }
    attrs
}

/// Our family as `cosmic-text`'s.
///
/// A borrowed `Family<'static>`, which is why [`FontFamily::Named`] carries a
/// `&'static str`: `cosmic-text` takes the name by reference for the life of
/// the `Attrs`, and a `String` here would mean either an allocation per run per
/// frame or a lifetime on `TextStyle`.
const fn to_family(family: FontFamily) -> Family<'static> {
    match family {
        FontFamily::SansSerif => Family::SansSerif,
        FontFamily::Serif => Family::Serif,
        FontFamily::Monospace => Family::Monospace,
        FontFamily::Named(name) => Family::Name(name),
    }
}

/// Our direction-relative alignment as `cosmic-text`'s absolute one.
///
/// `Start` and `End` are resolved here rather than passed through, so that the
/// meaning of "start" is decided by one function
/// ([`TextAlign::leading_fraction`]) shared with everything else that needs it.
fn to_align(align: TextAlign, base: TextDirection) -> Option<cosmic_text::Align> {
    Some(match align.leading_fraction(base) {
        f if f <= 0.0 => cosmic_text::Align::Left,
        f if f >= 1.0 => cosmic_text::Align::Right,
        _ => cosmic_text::Align::Center,
    })
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Affinity, Color};

    use super::*;

    fn fonts() -> FontStore {
        FontStore::embedded_only()
    }

    fn spans(text: &str) -> Vec<TextSpan> {
        vec![TextSpan::new(text, TextStyle::new(16.0))]
    }

    #[test]
    fn text_shapes_into_glyphs() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);

        assert_eq!(para.glyph_count(), 5, "one glyph per Latin letter");
        assert!(!para.is_empty());
        assert!(para.size().width > 0.0);
    }

    #[test]
    fn end_stops_before_a_hard_line_break() {
        // What End does. It used to return the offset *after* the `\n`, which is
        // the start of the next line — so the caret visibly jumped down a row,
        // and Home-then-shift-End selected the break along with the text, so
        // replacing a "line" also joined it to the one below.
        let mut store = FontStore::new();
        let text = "alpha\nbeta";
        let paragraph = Paragraph::layout(
            &mut store,
            &[TextSpan::new(text, TextStyle::new(16.0))],
            f32::INFINITY,
        );

        let on_first = TextPosition::new(2);
        assert_eq!(paragraph.line_start(on_first).offset, 0);
        assert_eq!(
            paragraph.line_end(on_first).offset,
            5,
            "the end of `alpha`, before the newline — not the start of `beta`"
        );

        // And the last line, which has no break to stop before.
        let on_second = TextPosition::new(7);
        assert_eq!(paragraph.line_start(on_second).offset, 6);
        assert_eq!(paragraph.line_end(on_second).offset, text.len());
    }

    #[test]
    fn a_windows_line_break_counts_as_two_bytes() {
        let mut store = FontStore::new();
        let text = "alpha\r\nbeta";
        let paragraph = Paragraph::layout(
            &mut store,
            &[TextSpan::new(text, TextStyle::new(16.0))],
            f32::INFINITY,
        );
        assert_eq!(
            paragraph.line_end(TextPosition::new(2)).offset,
            5,
            "stepping back one byte would leave the caret between \\r and \\n"
        );
    }

    #[test]
    fn line_height_changes_the_height_and_not_the_width() {
        let mut fonts = fonts();
        let tight = vec![TextSpan::new(
            "one\ntwo",
            TextStyle::new(16.0).line_height(1.0),
        )];
        let airy = vec![TextSpan::new(
            "one\ntwo",
            TextStyle::new(16.0).line_height(2.0),
        )];

        let tight = Paragraph::layout(&mut fonts, &tight, f32::INFINITY);
        let airy = Paragraph::layout(&mut fonts, &airy, f32::INFINITY);

        assert!(
            airy.size().height > tight.size().height * 1.5,
            "doubling the line box has to move the second line down: {} vs {}",
            airy.size().height,
            tight.size().height
        );
        assert!(
            (airy.size().width - tight.size().width).abs() < 0.01,
            "and must not touch the width"
        );
    }

    #[test]
    fn letter_spacing_lands_in_the_advances_rather_than_beside_them() {
        let mut fonts = fonts();
        let plain = vec![TextSpan::new("iiii", TextStyle::new(16.0))];
        let tracked = vec![TextSpan::new(
            "iiii",
            TextStyle::new(16.0).letter_spacing(4.0),
        )];

        let plain = Paragraph::layout(&mut fonts, &plain, f32::INFINITY);
        let tracked = Paragraph::layout(&mut fonts, &tracked, f32::INFINITY);

        assert!(
            tracked.size().width > plain.size().width + 8.0,
            "four glyphs at four pixels each: {} vs {}",
            tracked.size().width,
            plain.size().width
        );

        // The part that matters, and the reason this is the shaper's job: the
        // caret walks the same advances the glyphs were placed with. Measuring
        // one way and drawing another puts the caret between the wrong letters.
        let caret = tracked.cursor_rect(TextPosition::new(2));
        let plain_caret = plain.cursor_rect(TextPosition::new(2));
        assert!(
            caret.left > plain_caret.left + 4.0,
            "the caret has to move with the letters: {} vs {}",
            caret.left,
            plain_caret.left
        );
    }

    /// Monospace is a *different typeface*, not a different name for one.
    ///
    /// The failure this guards is a silent no-op: with only the proportional
    /// faces embedded, the database mapped the monospace family back onto
    /// DejaVu Sans, so `FontFamily::Monospace` shaped identically to the
    /// default and nothing reported a problem. A column of figures still
    /// jittered as its digits changed, which is the only reason anyone asks
    /// for monospace in an interface.
    ///
    /// Asserted on advances rather than on the family name, because the name
    /// resolving is not the claim — the digits being the same width is.
    #[test]
    fn monospace_digits_all_have_the_same_advance_and_the_default_does_not() {
        let mut fonts = fonts();

        let mut widths = |style: TextStyle| -> Vec<f32> {
            // `i` and `m`, not digits: DejaVu Sans has **tabular figures**, so
            // its digits are already one width in the proportional face and a
            // digits-only test passes with monospace resolving back to it.
            // That is the failure this test exists to catch, and it caught
            // itself first.
            "iM8"
                .chars()
                .map(|ch| {
                    Paragraph::layout(
                        &mut fonts,
                        &[TextSpan::new(ch.to_string(), style)],
                        f32::INFINITY,
                    )
                    .size()
                    .width
                })
                .collect()
        };

        let mono = widths(TextStyle::new(20.0).monospace());
        assert!(
            mono.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "in a fixed-pitch face every glyph is one column wide: {mono:?}"
        );

        let proportional = widths(TextStyle::new(20.0));
        assert!(
            proportional
                .windows(2)
                .any(|pair| (pair[0] - pair[1]).abs() > 0.5),
            "and the default face must *not* be fixed-pitch, or this test \
             passes with monospace resolving back to it: {proportional:?}"
        );
    }

    #[test]
    fn the_characters_the_framework_draws_are_all_in_the_embedded_font() {
        // The bug this guards, which cost a password field. `.notdef` in DejaVu
        // is an **empty** glyph with an ordinary advance, so a character the
        // embedded subset does not cover takes its space on the line and inks
        // nothing: shaping succeeds, the glyph count is right, the backend
        // reports it drawn, and the screen is blank. A masked field showed an
        // empty box, which reads to a user as typing that is not registering
        // rather than as a missing glyph.
        //
        // Every character below is one the framework or its own examples put on
        // screen without an application choosing it, so covering it is the
        // framework's problem. Asserted on the glyph *id*, because zero is
        // exactly the fact and it needs no rasteriser.
        let mut fonts = fonts();
        for (name, ch) in [
            (
                "the password mask",
                vieww_foundation::DEFAULT_OBSCURING_CHARACTER,
            ),
            ("an em dash", '\u{2014}'),
            ("an ellipsis", '\u{2026}'),
            ("a typographic apostrophe", '\u{2019}'),
            ("a left arrow", '\u{2190}'),
            // The one that was actually missing. A quality gate reading
            // "SSIMULACRA2 \u{2265} 90" drew "SSIMULACRA2 90" — the comparison
            // silently inverted in meaning, in a screen whose whole job is to
            // say what will and will not be re-encoded.
            ("greater-or-equal", '\u{2265}'),
            ("less-or-equal", '\u{2264}'),
            ("not-equal", '\u{2260}'),
            ("a true minus sign", '\u{2212}'),
            // **The Macintosh modifier keys.** Not framework-drawn, and here
            // anyway, because the embedded subset is the only font a packaged
            // application is guaranteed to have and these are what *any* menu
            // written by *any* application on that platform contains. vieww
            // Studio wrote `⌘⇧P` beside every command in every menu and drew
            // three empty advances and a `P`, on macOS only, for as long as
            // this list did not mention them — and nobody caught it, because
            // the person reading the screenshots was on Linux, where the same
            // code writes `Ctrl+Shift+P` in ASCII.
            //
            // That asymmetry is the whole argument for testing them here: the
            // platform where they are wrong is the platform where nothing else
            // in this repository runs.
            ("the command key", '\u{2318}'),
            ("the shift key", '\u{21E7}'),
            ("the option key", '\u{2325}'),
            ("the return key", '\u{23CE}'),
            ("the delete key", '\u{232B}'),
            // Disclosure triangles, a tick and a cross: the characters an
            // interface points, confirms and dismisses with.
            ("a disclosure triangle, open", '\u{25BE}'),
            ("a disclosure triangle, closed", '\u{25B8}'),
            ("a check mark", '\u{2713}'),
            ("a dismissal cross", '\u{2715}'),
            ("a single left angle quote", '\u{2039}'),
            ("a single right angle quote", '\u{203A}'),
        ] {
            let para = Paragraph::layout(
                &mut fonts,
                &[TextSpan::new(ch.to_string(), TextStyle::new(16.0))],
                f32::INFINITY,
            );
            let id = para.runs()[0].glyphs[0].id;

            assert_ne!(
                id, 0,
                "{name} (U+{:04X}) shapes to .notdef, which draws nothing at all",
                ch as u32
            );
        }
    }

    #[test]
    fn a_small_span_keeps_its_size_beside_a_large_one() {
        // The second half of the same defect. The buffer carries one size, and
        // a span that did not override it was drawn at the buffer's — which is
        // the largest span's — so a 10pt run next to a 30pt one came out at
        // 30pt. Measured rather than looked at: the mixed paragraph has to be
        // as wide as the two laid out separately, and under the old behaviour
        // it was exactly twice the large one.
        let mut fonts = fonts();
        let large = Paragraph::layout(
            &mut fonts,
            &[TextSpan::new("MMM", TextStyle::new(30.0))],
            f32::INFINITY,
        )
        .size()
        .width;
        let small = Paragraph::layout(
            &mut fonts,
            &[TextSpan::new("MMM", TextStyle::new(10.0))],
            f32::INFINITY,
        )
        .size()
        .width;
        let mixed = Paragraph::layout(
            &mut fonts,
            &[
                TextSpan::new("MMM", TextStyle::new(30.0)),
                TextSpan::new("MMM", TextStyle::new(10.0)),
            ],
            f32::INFINITY,
        )
        .size()
        .width;

        assert!(
            (mixed - (large + small)).abs() < 1.0,
            "each span at its own size is {} + {} = {}, not {mixed}",
            large,
            small,
            large + small
        );
    }

    #[test]
    fn a_mixed_size_line_grows_rather_than_overlapping() {
        // The reason the old code gave for one uniform size, checked instead of
        // assumed: the line box has to be the tallest span's, or a large glyph
        // is clipped by a small span's leading.
        let mut fonts = fonts();
        let mixed = Paragraph::layout(
            &mut fonts,
            &[
                TextSpan::new("small ", TextStyle::new(10.0)),
                TextSpan::new("LARGE", TextStyle::new(30.0)),
            ],
            f32::INFINITY,
        );

        assert_eq!(mixed.line_count(), 1);
        assert!(
            mixed.size().height >= 30.0,
            "a 30pt glyph needs at least its own size of line box: {}",
            mixed.size().height
        );
    }

    #[test]
    fn letter_spacing_is_pixels_rather_than_ems() {
        // The bug this pins. `TextStyle::letter_spacing` is documented in
        // logical pixels, and `cosmic-text` adds its value to an advance
        // expressed in **em units** — every advance is divided by the font's
        // units-per-em before the addition and multiplied by the font size
        // afterwards. Handing the pixel value straight over therefore scaled it
        // by the size, so the theme's -0.25 on a 24px headline tightened by six
        // pixels a glyph and the letters of "Create an account" overlapped.
        //
        // The direction-only assertion above passed throughout, which is why
        // this one measures.
        let mut fonts = fonts();
        let plain = vec![TextSpan::new("iiii", TextStyle::new(16.0))];
        let tracked = vec![TextSpan::new(
            "iiii",
            TextStyle::new(16.0).letter_spacing(4.0),
        )];

        let plain = Paragraph::layout(&mut fonts, &plain, f32::INFINITY);
        let tracked = Paragraph::layout(&mut fonts, &tracked, f32::INFINITY);

        let added = tracked.size().width - plain.size().width;
        assert!(
            (added - 16.0).abs() < 1.0,
            "four glyphs at four pixels each is sixteen pixels, not {added}"
        );
    }

    #[test]
    fn tracking_takes_the_same_pixels_off_at_any_size() {
        // Size-independence is the whole claim: a pixel is a pixel, so -1.0 has
        // to shave the same four pixels off four glyphs whether the text is
        // small or large. Under the em reading it shaved four times as much off
        // the larger one, which is why this was invisible in the 14px label
        // style and disfiguring in the 24px headline.
        let mut fonts = fonts();
        let mut shaved = |size: f32| {
            let plain = vec![TextSpan::new("iiii", TextStyle::new(size))];
            let tight = vec![TextSpan::new(
                "iiii",
                TextStyle::new(size).letter_spacing(-1.0),
            )];
            let plain = Paragraph::layout(&mut fonts, &plain, f32::INFINITY);
            let tight = Paragraph::layout(&mut fonts, &tight, f32::INFINITY);
            plain.size().width - tight.size().width
        };

        let small = shaved(12.0);
        let large = shaved(48.0);
        assert!(
            (small - 4.0).abs() < 1.0 && (large - 4.0).abs() < 1.0,
            "four pixels off at both sizes: {small} at 12px, {large} at 48px"
        );
    }

    #[test]
    fn the_tallest_line_box_wins_even_when_it_is_not_the_biggest_text() {
        let mut fonts = fonts();
        // A small span asking for generous spacing, beside a large one that is
        // not. Taking the line height of whichever span was biggest would drop
        // the request on the floor.
        let mixed = vec![
            TextSpan::new("big ", TextStyle::new(20.0).line_height(1.0)),
            TextSpan::new("roomy", TextStyle::new(10.0).line_height(4.0)),
        ];
        let para = Paragraph::layout(&mut fonts, &mixed, f32::INFINITY);

        assert!(
            para.size().height >= 40.0,
            "10pt at 4.0 is a 40px line box: {}",
            para.size().height
        );
    }

    #[test]
    fn glyph_advances_come_from_the_font_not_a_fixed_ratio() {
        let mut fonts = fonts();
        let narrow = Paragraph::layout(&mut fonts, &spans("iii"), f32::INFINITY);
        let wide = Paragraph::layout(&mut fonts, &spans("WWW"), f32::INFINITY);

        assert!(
            wide.size().width > narrow.size().width * 1.5,
            "a proportional font makes W much wider than i: {} vs {}",
            wide.size().width,
            narrow.size().width
        );
    }

    #[test]
    fn empty_text_still_occupies_one_line() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans(""), f32::INFINITY);

        assert_eq!(para.line_count(), 1, "a caret needs somewhere to sit");
        assert!(para.size().height > 0.0);
        assert!(para.is_empty());
    }

    #[test]
    fn a_long_run_wraps_at_the_given_width() {
        let mut fonts = fonts();
        let text = spans("the quick brown fox jumps over the lazy dog");

        let unwrapped = Paragraph::layout(&mut fonts, &text, f32::INFINITY);
        let wrapped = Paragraph::layout(&mut fonts, &text, 120.0);

        assert_eq!(unwrapped.line_count(), 1);
        assert!(wrapped.line_count() > 1, "{}", wrapped.line_count());
        assert!(
            wrapped.size().width <= 120.0,
            "wrapped text must fit: {}",
            wrapped.size().width
        );
        assert!(
            wrapped.size().height > unwrapped.size().height,
            "more lines is taller"
        );
    }

    #[test]
    fn wrapping_moves_glyphs_rather_than_dropping_them() {
        let mut fonts = fonts();
        // No spaces, so there are no break-point spaces to be collapsed and the
        // count must match exactly. Wrapping falls back to breaking mid-word.
        let text = spans("abcdefghijklmnopqrstuvwxyz");

        let unwrapped = Paragraph::layout(&mut fonts, &text, f32::INFINITY);
        let wrapped = Paragraph::layout(&mut fonts, &text, 60.0);

        assert!(wrapped.line_count() > 1, "{}", wrapped.line_count());
        assert_eq!(
            wrapped.glyph_count(),
            unwrapped.glyph_count(),
            "every glyph survives the break"
        );
    }

    #[test]
    fn a_space_at_a_wrap_point_is_not_drawn() {
        let mut fonts = fonts();
        let text = spans("the quick brown fox jumps over the lazy dog");

        let unwrapped = Paragraph::layout(&mut fonts, &text, f32::INFINITY);
        let wrapped = Paragraph::layout(&mut fonts, &text, 120.0);

        // The space a line breaks at is consumed by the break, so the wrapped
        // paragraph has one fewer glyph per break. Drawing it would push the next
        // line's first character inward by a space it cannot see.
        let breaks = wrapped.line_count() - 1;
        assert_eq!(
            wrapped.glyph_count(),
            unwrapped.glyph_count() - breaks,
            "{} lines, so {breaks} spaces consumed",
            wrapped.line_count()
        );
    }

    #[test]
    fn explicit_newlines_break_lines() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("one\ntwo\nthree"), f32::INFINITY);
        assert_eq!(para.line_count(), 3);
    }

    /// A caret needs somewhere to sit after `Enter` at the end of the text.
    ///
    /// Separator semantics, not terminator semantics: `"one\n"` is two lines, the
    /// second empty. Rust's own `str::lines()` disagrees — it yields one — and
    /// that is the distinction being pinned here, because getting it wrong is
    /// invisible in every test that puts text on both sides of the break.
    ///
    /// Found through `a_line_break_makes_the_field_taller_because_it_really_is_
    /// two_lines` in `vieww/tests/keys_to_semantics.rs`: pressing Enter changed
    /// the value and moved the caret, and the field did not grow.
    #[test]
    fn a_trailing_newline_opens_a_line_for_the_caret_to_sit_on() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("one\n"), f32::INFINITY);
        assert_eq!(
            para.line_count(),
            2,
            "pressing Enter at the end of a field must open a line, or the caret \
             is drawn outside the text it belongs to"
        );
    }

    /// The height has to follow the line count, since that is what a field is
    /// sized by. Separate from the count because a paragraph could plausibly
    /// count the line and not reserve space for it.
    #[test]
    fn a_trailing_newline_is_taller_than_the_text_before_it() {
        let mut fonts = fonts();
        let one = Paragraph::layout(&mut fonts, &spans("one"), f32::INFINITY);
        let two = Paragraph::layout(&mut fonts, &spans("one\n"), f32::INFINITY);
        assert!(
            two.size().height > one.size().height,
            "{} then {}",
            one.size().height,
            two.size().height
        );
    }

    #[test]
    fn a_style_change_mid_sentence_splits_the_run() {
        let mut fonts = fonts();
        let mixed = [
            TextSpan::new("plain ", TextStyle::new(16.0)),
            TextSpan::new("bold", TextStyle::new(16.0).bold()),
        ];

        let para = Paragraph::layout(&mut fonts, &mixed, f32::INFINITY);
        assert!(
            para.runs().len() >= 2,
            "bold mid-sentence needs its own run: {:?}",
            para.runs().len()
        );
    }

    #[test]
    fn a_colour_change_splits_the_run_because_a_backend_batches_on_colour() {
        let mut fonts = fonts();
        let mixed = [
            TextSpan::new("red", TextStyle::new(16.0).color(Color::RED)),
            TextSpan::new("blue", TextStyle::new(16.0).color(Color::BLUE)),
        ];

        let para = Paragraph::layout(&mut fonts, &mixed, f32::INFINITY);
        let colors: Vec<_> = para.runs().iter().map(|run| run.color).collect();
        assert!(colors.contains(&Color::RED), "{colors:?}");
        assert!(colors.contains(&Color::BLUE), "{colors:?}");
    }

    #[test]
    fn bold_is_wider_than_regular_at_the_same_size() {
        let mut fonts = fonts();
        let regular = Paragraph::layout(
            &mut fonts,
            &[TextSpan::new("Handgloves", TextStyle::new(16.0))],
            f32::INFINITY,
        );
        let bold = Paragraph::layout(
            &mut fonts,
            &[TextSpan::new("Handgloves", TextStyle::new(16.0).bold())],
            f32::INFINITY,
        );

        assert!(
            bold.size().width > regular.size().width,
            "a real bold face is wider: {} vs {}",
            bold.size().width,
            regular.size().width
        );
    }

    // ------------------------------------------------------------ bidi and RTL

    /// "shalom" in Hebrew — right-to-left, and covered by the embedded font.
    const HEBREW: &str = "שלום";

    #[test]
    fn hebrew_is_detected_as_right_to_left() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans(HEBREW), f32::INFINITY);

        assert_eq!(
            para.direction(),
            TextDirection::Rtl,
            "direction must be inferred, not demanded of the application"
        );
        assert!(
            para.glyph_count() > 0,
            "the embedded font must cover Hebrew"
        );
    }

    #[test]
    fn latin_is_detected_as_left_to_right() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);
        assert_eq!(para.direction(), TextDirection::Ltr);
    }

    #[test]
    fn an_rtl_run_is_marked_rtl() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans(HEBREW), f32::INFINITY);

        assert!(
            para.runs().iter().any(|run| run.is_rtl),
            "a caret and a selection both need to know which end is the start"
        );
    }

    #[test]
    fn rtl_glyphs_are_in_logical_order_running_right_to_left() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans(HEBREW), f32::INFINITY);

        let run = para
            .runs()
            .iter()
            .find(|run| run.is_rtl)
            .expect("an RTL run");
        let xs: Vec<f32> = run.glyphs.iter().map(|g| g.offset.dx).collect();

        // Glyphs stay in *logical* order — first character first — so in an RTL
        // run the x positions descend. Positions are already final either way,
        // which is what lets a backend draw the run in one call without caring.
        assert!(xs.len() > 1, "{xs:?}");
        assert!(
            xs.windows(2).all(|pair| pair[0] >= pair[1]),
            "the first logical glyph is the rightmost: {xs:?}"
        );
        assert!(
            run.advance() > 0.0,
            "and a width is still positive: {}",
            run.advance()
        );
    }

    #[test]
    fn ltr_glyphs_are_in_logical_order_running_left_to_right() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("abcd"), f32::INFINITY);
        let xs: Vec<f32> = para.runs()[0].glyphs.iter().map(|g| g.offset.dx).collect();

        assert!(xs.windows(2).all(|pair| pair[0] <= pair[1]), "{xs:?}");
    }

    #[test]
    fn a_mixed_direction_line_produces_runs_of_both_directions() {
        let mut fonts = fonts();
        // Hebrew with an embedded Latin word: the classic bidi case.
        let text = format!("{HEBREW} hello {HEBREW}");
        let para = Paragraph::layout(&mut fonts, &spans(&text), f32::INFINITY);

        let rtl = para.runs().iter().filter(|run| run.is_rtl).count();
        let ltr = para.runs().iter().filter(|run| !run.is_rtl).count();
        assert!(
            rtl > 0 && ltr > 0,
            "one line, both directions — this is why a run carries direction \
             rather than the paragraph: {rtl} rtl, {ltr} ltr"
        );
    }

    #[test]
    fn an_explicit_base_direction_overrides_detection() {
        let mut fonts = fonts();
        let para = Paragraph::layout_aligned(
            &mut fonts,
            &spans("123"),
            200.0,
            TextAlign::Start,
            Some(TextDirection::Rtl),
        );
        assert_eq!(
            para.direction(),
            TextDirection::Rtl,
            "digits alone cannot decide, so the caller must be able to"
        );
    }

    // ------------------------------------------------------------------ align

    #[test]
    fn centering_shifts_a_short_line_inward() {
        let mut fonts = fonts();
        let left = Paragraph::layout_aligned(
            &mut fonts,
            &spans("hi"),
            200.0,
            TextAlign::Left,
            Some(TextDirection::Ltr),
        );
        let centered = Paragraph::layout_aligned(
            &mut fonts,
            &spans("hi"),
            200.0,
            TextAlign::Center,
            Some(TextDirection::Ltr),
        );

        let left_x = left.runs()[0].origin.dx;
        let center_x = centered.runs()[0].origin.dx;
        assert!(
            center_x > left_x,
            "centred text starts further right: {center_x} vs {left_x}"
        );
    }

    #[test]
    fn right_alignment_pushes_a_short_line_to_the_trailing_edge() {
        let mut fonts = fonts();
        let right = Paragraph::layout_aligned(
            &mut fonts,
            &spans("hi"),
            200.0,
            TextAlign::Right,
            Some(TextDirection::Ltr),
        );
        let centered = Paragraph::layout_aligned(
            &mut fonts,
            &spans("hi"),
            200.0,
            TextAlign::Center,
            Some(TextDirection::Ltr),
        );

        assert!(right.runs()[0].origin.dx > centered.runs()[0].origin.dx);
    }

    // ----------------------------------------------------------------- metrics

    #[test]
    fn runs_sit_on_a_baseline_below_the_top_of_the_line() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("Hxy"), f32::INFINITY);
        let run = &para.runs()[0];

        assert!(run.ascent > 0.0, "ascent {}", run.ascent);
        assert!(run.descent > 0.0, "descent {}", run.descent);
        assert!(
            run.origin.dy >= run.ascent,
            "the baseline must sit at least an ascent below the top: {} vs {}",
            run.origin.dy,
            run.ascent
        );
    }

    #[test]
    fn successive_lines_have_descending_baselines() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("one\ntwo"), f32::INFINITY);

        let baselines: Vec<f32> = para.runs().iter().map(|run| run.origin.dy).collect();
        assert!(baselines.len() >= 2, "{baselines:?}");
        assert!(
            baselines[1] > baselines[0],
            "the second line sits below the first: {baselines:?}"
        );
    }

    // ---------------------------------------------------- carets and hit testing
    //
    // The tests in `lines.rs` cover the geometry against hand-built lines. These
    // cover the half that only real shaping can exercise: that the byte offsets
    // coming out of `cosmic-text` are translated into the whole string's
    // coordinates, and that clusters are built from the same pass that produced
    // the glyphs.

    #[test]
    fn tapping_either_end_of_a_line_gives_either_end_of_the_text() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);
        let middle = para.size().height / 2.0;

        assert_eq!(para.hit_test(Offset::new(-10.0, middle)).offset, 0);
        assert_eq!(
            para.hit_test(Offset::new(para.size().width + 50.0, middle))
                .offset,
            5,
            "past the end of the text is the end of the text"
        );
    }

    #[test]
    fn a_caret_placed_at_an_offset_hit_tests_back_to_it() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);

        for offset in 0..=5 {
            let rect = para.cursor_rect(TextPosition::new(offset));
            let point = Offset::new(rect.left, (rect.top + rect.bottom) / 2.0);
            assert_eq!(
                para.hit_test(point).offset,
                offset,
                "tapping exactly where the caret was drawn must not move it"
            );
        }
    }

    #[test]
    fn carets_advance_across_a_line() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);

        let xs: Vec<f32> = (0..=5)
            .map(|offset| para.cursor_rect(TextPosition::new(offset)).left)
            .collect();
        assert!(
            xs.windows(2).all(|pair| pair[1] > pair[0]),
            "every character moves the caret right: {xs:?}"
        );
    }

    #[test]
    fn offsets_after_a_newline_are_against_the_whole_string() {
        let mut fonts = fonts();
        // `cosmic-text` reports glyph offsets against the *buffer line*, so they
        // restart at zero after every newline. Untranslated, a tap anywhere on the
        // second line here would report 0..3 — indistinguishable from the first.
        let para = Paragraph::layout(&mut fonts, &spans("one\ntwo"), f32::INFINITY);
        let second_line = para.size().height * 0.75;

        let start = para.hit_test(Offset::new(-10.0, second_line));
        assert_eq!(start.offset, 4, "the t of two, past the newline at 3");

        let end = para.hit_test(Offset::new(1000.0, second_line));
        assert_eq!(
            end.offset, 7,
            "the end of the text, not the end of the line"
        );
    }

    #[test]
    fn a_caret_on_the_second_line_is_drawn_on_the_second_line() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("one\ntwo"), f32::INFINITY);

        let first = para.cursor_rect(TextPosition::new(1));
        let second = para.cursor_rect(TextPosition::new(5));
        assert!(second.top > first.top, "{second} vs {first}");
        assert_eq!(para.line_of(TextPosition::new(5)), 1);
    }

    #[test]
    fn a_style_after_a_newline_applies_to_the_line_it_is_on() {
        let mut fonts = fonts();
        // The same untranslated-offset bug, seen from the styling side: looking a
        // colour up with a per-line offset finds the first line's style for every
        // line, and the second line comes out red.
        let mixed = [
            TextSpan::new("one\n", TextStyle::new(16.0).color(Color::RED)),
            TextSpan::new("two", TextStyle::new(16.0).color(Color::BLUE)),
        ];
        let para = Paragraph::layout(&mut fonts, &mixed, f32::INFINITY);

        let colors: Vec<_> = para.runs().iter().map(|run| run.color).collect();
        assert!(colors.contains(&Color::BLUE), "{colors:?}");
    }

    #[test]
    fn tapping_the_second_line_of_wrapped_text_gives_an_offset_on_it() {
        let mut fonts = fonts();
        let text = spans("the quick brown fox jumps over the lazy dog");
        let para = Paragraph::layout(&mut fonts, &text, 120.0);
        assert!(para.line_count() > 1);

        let first = para.hit_test(Offset::new(0.0, 1.0));
        let second = para.hit_test(Offset::new(0.0, para.size().height - 1.0));

        assert_eq!(first.offset, 0);
        assert!(
            second.offset > first.offset,
            "a later line means a later offset: {} vs {}",
            second.offset,
            first.offset
        );
        assert_eq!(para.line_of(second), para.line_count() - 1);
    }

    #[test]
    fn the_end_of_a_wrapped_line_is_upstream_so_the_caret_stays_on_it() {
        let mut fonts = fonts();
        // No spaces, so the break is mid-word and the offset either side of it is
        // the same one — which is exactly when affinity has to decide.
        let para = Paragraph::layout(&mut fonts, &spans("abcdefghijklmnop"), 60.0);
        assert!(para.line_count() > 1);

        let tapped = para.hit_test(Offset::new(1000.0, 1.0));
        assert_eq!(tapped.affinity, Affinity::Upstream);
        assert_eq!(
            para.line_of(tapped),
            0,
            "the caret stays where it was tapped"
        );
        assert_eq!(
            para.line_of(TextPosition::new(tapped.offset)),
            1,
            "and the same offset downstream is the line below"
        );
    }

    #[test]
    fn an_empty_paragraph_still_places_a_caret() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans(""), f32::INFINITY);

        assert_eq!(para.line_count(), 1);
        let rect = para.cursor_rect(TextPosition::new(0));
        assert!(rect.height() > 0.0, "an empty field is typed into: {rect}");
        assert_eq!(para.hit_test(Offset::new(50.0, 5.0)).offset, 0);
    }

    #[test]
    fn a_caret_in_hebrew_starts_at_the_right_hand_side() {
        let mut fonts = fonts();
        let para =
            Paragraph::layout_aligned(&mut fonts, &spans(HEBREW), 200.0, TextAlign::Start, None);

        let start = para.cursor_rect(TextPosition::new(0)).left;
        let end = para.cursor_rect(TextPosition::new(HEBREW.len())).left;
        assert!(
            start > end,
            "the first character of RTL text is the rightmost: {start} vs {end}"
        );
    }

    #[test]
    fn selecting_across_a_wrap_highlights_both_lines() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("abcdefghijklmnop"), 60.0);
        assert!(para.line_count() > 1);

        let rects = para.selection_rects(TextRange::new(0, 16));
        assert_eq!(
            rects.len(),
            para.line_count(),
            "one box per line, not one box around everything: {rects:?}"
        );
        assert!(rects.windows(2).all(|pair| pair[1].top > pair[0].top));
    }

    #[test]
    fn selecting_nothing_highlights_nothing() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello"), f32::INFINITY);
        assert!(para.selection_rects(TextRange::collapsed(2)).is_empty());
    }

    #[test]
    fn moving_down_a_line_keeps_the_column() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("hello\nhello"), f32::INFINITY);

        let start = TextPosition::new(3);
        let goal = para.cursor_rect(start).left;
        let below = para.position_below(start, goal).expect("a line below");

        assert_eq!(below.offset, 9, "three characters into the second line");
        assert!(
            (para.cursor_rect(below).left - goal).abs() < 0.5,
            "identical lines put the same column at the same x"
        );
        assert_eq!(para.position_below(below, goal), None, "no third line");
        assert_eq!(para.position_above(start, goal), None);
    }

    #[test]
    fn home_and_end_do_not_leave_the_line() {
        let mut fonts = fonts();
        let para = Paragraph::layout(&mut fonts, &spans("one\ntwo"), f32::INFINITY);
        let caret = TextPosition::new(5);

        assert_eq!(para.line_start(caret).offset, 4);
        assert_eq!(para.line_end(caret).offset, 7);
        assert_eq!(para.line_of(para.line_end(caret)), 1);
    }

    #[test]
    fn a_larger_size_produces_a_taller_and_wider_paragraph() {
        let mut fonts = fonts();
        let small = Paragraph::layout(&mut fonts, &spans("Handgloves"), f32::INFINITY);
        let large = Paragraph::layout(
            &mut fonts,
            &[TextSpan::new("Handgloves", TextStyle::new(32.0))],
            f32::INFINITY,
        );

        assert!(large.size().width > small.size().width);
        assert!(large.size().height > small.size().height);
    }
}
