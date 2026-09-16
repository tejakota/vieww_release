use std::rc::Rc;

use vieww_foundation::{Constraints, Cursor, Offset, Size, TextAlign, TextDirection};
use vieww_gestures::{GestureRecognizer, Recognized, TapRecognizer};
use vieww_text::{Paragraph, TextSpan};
use vieww_widget::Handler;

use crate::{LayoutCtx, PaintCtx, RenderObject, Role, Semantics};

/// Text in more than one style, wrapped as a single paragraph.
///
/// # The primitive the framework was missing
///
/// [`RenderText`](crate::RenderText) is one string in one
/// [`TextStyle`](vieww_foundation::TextStyle), which
/// is what every text-bearing control in the catalogue was built on. That is a
/// real limit and it had a visible cost: [`Markdown`](vieww_widget::Markdown)
/// recognised `**bold**`, `*italic*` and `` `code` `` and **stripped the
/// markers**, and rendered `[text](url)` as `text` with the destination thrown
/// away. Its own documentation was candid about it, and it is still content
/// destruction — a document rendered through it looks finished and is not.
///
/// The obvious workaround is worse than the gap. Splitting a sentence into
/// several `Text` widgets in a [`Flex::row`](vieww_widget::Flex) does not
/// soft-wrap: a `Flex` breaks between children, and prose has to break between
/// *words*, so a bold word mid-paragraph would either overflow or need line
/// breaking reimplemented outside the shaper.
///
/// So the fix is here, at the layer that shapes. And almost all of it already
/// existed: [`Paragraph::layout_aligned`] has always taken a **slice of
/// spans**, each with its own style, and shaped them into one wrapped
/// paragraph. `RenderText` passes it a slice of length one. This passes the
/// whole list.
///
/// # Links
///
/// A span may carry a handler. The paragraph is shaped once, so a tap is
/// resolved the way the text layer already resolves one — [`Paragraph::hit_test`]
/// gives a byte offset into the concatenated text, and the offset says which
/// span was hit. No per-span geometry, no second layout, and a link that wraps
/// across a line break is still one link because it is still one span.
///
/// The pointer becomes a hand over a span that has a handler, and only over
/// that span, which is the affordance that tells a reader a word is a link
/// before they click it.
///
/// # Semantics
///
/// The whole paragraph announces as one label — a screen reader should read a
/// sentence, not seven fragments — and it is the concatenated text, so the
/// reader hears what a sighted reader sees. Per-span link roles would need
/// semantic children with their own geometry, which is the same second-layout
/// problem the tap path avoids; naming it here rather than leaving it to be
/// discovered.
pub struct RenderRichText {
    /// The spans, in reading order. Concatenated, they are the paragraph.
    pub spans: Vec<TextSpan>,
    /// What to run when a span is tapped, parallel to `spans`.
    ///
    /// Parallel rather than a field on `TextSpan` because `TextSpan` lives in
    /// `vieww-text`, which knows nothing about gestures and should not start.
    links: Vec<Option<Handler<()>>>,
    pub align: TextAlign,
    /// An explicit base direction, or `None` to infer it from the text.
    pub direction: Option<TextDirection>,
    /// The last shaped paragraph. Produced by `layout`, consumed by `paint`.
    shaped: Option<Paragraph>,
}

impl RenderRichText {
    #[must_use]
    pub fn new(spans: Vec<TextSpan>) -> Self {
        let links = vec![None; spans.len()];
        Self {
            spans,
            links,
            align: TextAlign::default(),
            direction: None,
            shaped: None,
        }
    }

    /// Attach the tap handlers, one slot per span.
    ///
    /// # Panics
    ///
    /// If `links` is not the same length as `spans`. A mismatch means a link
    /// pointing at the wrong words, which is worse than a crash at construction
    /// and would be found by a reader rather than by a test.
    #[must_use]
    pub fn with_links(mut self, links: Vec<Option<Handler<()>>>) -> Self {
        assert_eq!(
            links.len(),
            self.spans.len(),
            "one link slot per span, or a link lands on the wrong words"
        );
        self.links = links;
        self
    }

    #[must_use]
    pub const fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    #[must_use]
    pub const fn direction(mut self, direction: Option<TextDirection>) -> Self {
        self.direction = direction;
        self
    }

    /// The spans joined, which is what the paragraph shapes and what a screen
    /// reader is told.
    #[must_use]
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// The paragraph shaped by the last layout, if any.
    #[must_use]
    pub const fn shaped(&self) -> Option<&Paragraph> {
        self.shaped.as_ref()
    }

    /// Which span a byte offset into the concatenated text falls in.
    ///
    /// The end of the text belongs to the last span rather than to nothing,
    /// which is what makes a tap past the final character still hit the link it
    /// visually landed on.
    fn span_at(&self, offset: usize) -> Option<usize> {
        let mut start = 0;
        for (index, span) in self.spans.iter().enumerate() {
            let end = start + span.text.len();
            if offset < end || (offset == end && index + 1 == self.spans.len()) {
                return Some(index);
            }
            start = end;
        }
        None
    }

    /// The handler for the span under `local`, if that span has one.
    fn link_at(&self, local: Offset) -> Option<&Handler<()>> {
        let paragraph = self.shaped.as_ref()?;
        let position = paragraph.hit_test(local);
        let index = self.span_at(position.offset)?;
        self.links.get(index)?.as_ref()
    }
}

impl std::fmt::Debug for RenderRichText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderRichText")
            .field("spans", &self.spans.len())
            .field("links", &self.links.iter().filter(|l| l.is_some()).count())
            .field("align", &self.align)
            .finish_non_exhaustive()
    }
}

/// Equality ignores the cached paragraph and the handlers.
///
/// The paragraph for the reason [`RenderText`](crate::RenderText) gives: it is
/// derived, so including it would make a freshly-built object compare unequal to
/// the identical one that has already been laid out, and every rebuild would
/// reshape.
///
/// The handlers because an `Rc<dyn Fn>` is rebuilt on every build — a closure
/// written in `build` is a new allocation each time — so comparing them would
/// mean *every* rebuild reshaped every rich paragraph on screen. They also
/// provably move no geometry, which is the question `layout_differs` asks.
impl PartialEq for RenderRichText {
    fn eq(&self, other: &Self) -> bool {
        self.spans == other.spans && self.align == other.align && self.direction == other.direction
    }
}

impl RenderObject for RenderRichText {
    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![
            ("spans", self.spans.len().to_string()),
            (
                "links",
                self.links
                    .iter()
                    .filter(|l| l.is_some())
                    .count()
                    .to_string(),
            ),
            ("align", format!("{:?}", self.align)),
        ]
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let paragraph = Paragraph::layout_aligned(
            ctx.fonts_mut(),
            &self.spans,
            constraints.max_width,
            self.align,
            self.direction,
        );
        let size = constraints.constrain(paragraph.size());
        self.shaped = Some(paragraph);
        size
    }

    /// The first line's baseline, straight off the shaped paragraph — so a rich
    /// paragraph aligns to its neighbours exactly as a plain one does.
    fn baseline(&self, _ctx: &mut crate::BaselineCtx<'_>, _size: Size) -> Option<f32> {
        self.shaped.as_ref()?.line_baseline(0)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(paragraph) = &self.shaped else {
            return;
        };
        let origin = ctx.origin();
        // **The run origin is snapped to the physical pixel grid** — the same
        // rule `RenderText::paint` argues for, for the same reason: a glyph
        // rasterised at a fractional texel position is box-filtered across
        // texels and the text reads smeared. A paragraph that mixes weights
        // and sizes makes the artefact worse rather than better, because the
        // eye compares a crisp bold run against a soft regular one on the
        // same baseline. The per-glyph offsets from shaping stay fractional
        // deliberately — rounding them would re-space the letters, and it is
        // the run origin that decides whether the rasters sit on texel
        // boundaries.
        let dpr = ctx.device_pixel_ratio().max(1.0);
        let snap = |offset: Offset| {
            let x = (offset.dx * dpr).round() / dpr;
            let y = (offset.dy * dpr).round() / dpr;
            Offset::new(x, y)
        };
        for run in paragraph.runs() {
            let mut placed = run.clone();
            placed.origin = snap(Offset::new(
                run.origin.dx + origin.dx,
                run.origin.dy + origin.dy,
            ));
            ctx.canvas().draw_glyphs(&placed);
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        true
    }

    fn cursor(&self, local: Offset) -> Option<Cursor> {
        // A hand over a link and an I-beam over the prose around it, decided per
        // *span* rather than for the whole paragraph. A paragraph that showed a
        // hand everywhere because one word in it is a link tells the reader the
        // wrong thing about six other words.
        Some(if self.link_at(local).is_some() {
            Cursor::Pointer
        } else {
            Cursor::Text
        })
    }

    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        if self.links.iter().all(Option::is_none) {
            // No links, no recogniser — so a paragraph of plain rich text does
            // not enter the gesture arena and cannot win a tap away from a row
            // or a card underneath it.
            return Vec::new();
        }
        vec![Box::new(TapRecognizer::new())]
    }

    fn handle_gesture(&self, gesture: &Recognized, _local: Offset) {
        if let Recognized::Tap(details) = gesture {
            if let Some(handler) = self.link_at(details.local) {
                handler(());
            }
        }
    }

    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        use crate::Extremum;

        // Measured by shaping, as `RenderText` is and for the same reason: an
        // estimate here would be a second text engine to keep in agreement with
        // the first.
        let width = match (query.axis, query.extremum) {
            (Axis::Horizontal, Extremum::Max) => f32::INFINITY,
            (Axis::Horizontal, Extremum::Min) => 0.0,
            (Axis::Vertical, _) => query.cross.unwrap_or(f32::INFINITY),
        };
        let paragraph = Paragraph::layout_aligned(
            ctx.fonts_mut(),
            &self.spans,
            width,
            self.align,
            self.direction,
        );
        Some(match query.axis {
            Axis::Horizontal => paragraph.widest_line(),
            Axis::Vertical => paragraph.size().height,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        // Reached only when `layout_differs` said no, so the spans are the same
        // and the old shaping is still the right answer. Without this every
        // rebuild above a rich paragraph drops the shaping `paint` needs and the
        // paragraph disappears — the same defect `RenderText` documents.
        let old: &dyn std::any::Any = old;
        if let Some(old) = old.downcast_ref::<Self>() {
            self.shaped.clone_from(&old.shaped);
        }
    }

    fn semantics(&self) -> Option<Semantics> {
        let text = self.text();
        (!text.trim().is_empty()).then(|| Semantics::new(Role::Label).with_label(text))
    }

    fn debug_name(&self) -> &'static str {
        "RenderRichText"
    }
}

/// The handler type, re-exported so a caller building links does not have to
/// name `vieww_widget` for it.
pub type LinkHandler = Rc<dyn Fn(())>;
