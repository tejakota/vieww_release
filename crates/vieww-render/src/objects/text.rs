use vieww_foundation::{Constraints, Offset, Size, TextAlign, TextDirection, TextStyle};
use vieww_text::{Paragraph, TextOverflow, TextSpan};

use crate::{LayoutCtx, PaintCtx, RenderObject};

/// A run of text. A leaf: it has no children.
///
/// # Why the paragraph is cached
///
/// Shaping is the expensive part of text, and layout and paint need the *same*
/// answer: if paint reshaped, any disagreement with what layout measured would
/// show up as text overflowing its box. So `layout` shapes and stores the result,
/// and `paint` only reads it.
///
/// This is also why the cache is not an optimisation that could be removed. A
/// render object whose layout is skipped — the common case, behind a relayout
/// boundary — still gets painted, and the paragraph from the last layout is
/// exactly what should be drawn.
#[derive(Debug, Clone)]
pub struct RenderText {
    pub text: String,
    pub style: TextStyle,
    pub align: TextAlign,
    /// An explicit base direction, or `None` to infer it from the text.
    pub direction: Option<TextDirection>,
    /// What to do with text that does not fit the width it is given.
    ///
    /// Defaults to [`TextOverflow::Visible`], which is what every run in the
    /// framework did before this field existed: wrap, and if a single word
    /// cannot wrap, run past the edge.
    pub overflow: TextOverflow,
    /// How many lines an eliding run is allowed.
    ///
    /// Only [`Some(1)`](Some) is honoured today, and that is deliberate rather
    /// than unfinished: the surfaces that need an ellipsis are slots — a tab, a
    /// row, a breadcrumb — and every one of them is one line. A multi-line
    /// ellipsis needs the *last* line re-shaped against the remaining width
    /// after the ones above it were broken, which is a different piece of work
    /// in `vieww-text` and not one this field should pretend to have done.
    pub max_lines: Option<usize>,
    /// The last shaped paragraph. Produced by `layout`, consumed by `paint`.
    shaped: Option<Paragraph>,
    /// What `layout` actually shaped, when elision replaced it.
    ///
    /// `text` stays the string the caller asked for — semantics announce it, and
    /// a screen reader should hear the file's name rather than the abbreviation
    /// the strip had room for.
    elided: Option<String>,
}

impl RenderText {
    #[must_use]
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self {
            text: text.into(),
            style,
            align: TextAlign::default(),
            direction: None,
            overflow: TextOverflow::default(),
            max_lines: None,
            shaped: None,
            elided: None,
        }
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
    pub fn overflow(mut self, overflow: TextOverflow) -> Self {
        self.overflow = overflow;
        self
    }

    #[must_use]
    pub fn max_lines(mut self, max_lines: Option<usize>) -> Self {
        self.max_lines = max_lines;
        self
    }

    /// What was drawn, which is the text unless it had to be cut.
    #[must_use]
    pub fn displayed(&self) -> &str {
        self.elided.as_deref().unwrap_or(&self.text)
    }

    /// The paragraph shaped by the last layout, if any.
    #[must_use]
    pub const fn shaped(&self) -> Option<&Paragraph> {
        self.shaped.as_ref()
    }
}

/// Equality ignores the cached paragraph.
///
/// `layout_differs` is built on this, and it asks "would this change the
/// geometry?". The cache is *derived* from the fields, so including it would make
/// a freshly-built object with no cache compare unequal to the identical object
/// that has already been laid out — and every rebuild would relayout all text.
impl PartialEq for RenderText {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.style == other.style
            && self.align == other.align
            && self.direction == other.direction
            && self.overflow == other.overflow
            && self.max_lines == other.max_lines
    }
}

impl RenderObject for RenderText {
    fn describe(&self) -> Vec<(&'static str, String)> {
        vec![
            ("text", truncated(&self.text)),
            ("size", format!("{:.1}", self.style.size)),
            ("weight", format!("{:?}", self.style.weight)),
            ("color", format!("{:?}", self.style.color)),
            ("align", format!("{:?}", self.align)),
        ]
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        // **Elision runs first, because it decides what gets shaped.** It is
        // also the only place in the pipeline that can: the width a string has
        // to fit into is known here and nowhere earlier, and how wide a string
        // *is* is a question only the shaper answers. Every character-count
        // approximation in the framework's UI layer exists because this hook
        // did not.
        self.elided = if self.overflow.elides() && self.max_lines.unwrap_or(1) == 1 {
            vieww_text::elide_to_width(
                ctx.fonts_mut(),
                &self.text,
                &self.style,
                self.align,
                self.direction,
                constraints.max_width,
                self.overflow,
            )
        } else {
            None
        };

        let spans = [TextSpan::new(self.displayed().to_owned(), self.style)];
        let paragraph = Paragraph::layout_aligned(
            ctx.fonts_mut(),
            &spans,
            constraints.max_width,
            self.align,
            self.direction,
        );
        let size = constraints.constrain(paragraph.size());
        self.shaped = Some(paragraph);
        size
    }

    /// The first line's baseline, straight off the shaped paragraph.
    ///
    /// This is the one render object in the tree that genuinely knows where a
    /// baseline is; everything else either forwards this answer or has none.
    /// It comes from the same `Paragraph` that `paint` draws from, so the line
    /// a neighbour aligns to is the line the glyphs are actually sitting on —
    /// a second estimate here would be a second text engine to keep in
    /// agreement with the first, which is the same argument the intrinsic
    /// implementation below makes.
    fn baseline(&self, _ctx: &mut crate::BaselineCtx<'_>, _size: Size) -> Option<f32> {
        self.shaped.as_ref()?.line_baseline(0)
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let Some(paragraph) = &self.shaped else {
            // Painted without ever being laid out. Not reachable through the
            // pipeline, which always lays out first, so there is nothing sensible
            // to draw rather than something wrong.
            return;
        };
        let origin = ctx.origin();
        // **The run origin is snapped to the physical pixel grid.**
        //
        // A glyph rasterised at a fractional texel position is box-filtered
        // across texels: two source pixels averaged per destination pixel, the
        // effective resolution of the text halved, and a label that reads
        // smeared rather than soft. This is the single most reported
        // text-quality defect in 2D rasterisers, and the fix the ecosystem
        // converged on is the same everywhere — align the *placement* to the
        // grid and let the shaper keep the sub-pixel advances *within* the run.
        // Here that means rounding the run origin in physical pixels and
        // dividing back, so at 1:1 the snap is to whole logical pixels and at
        // a fractional scale it is to the display's own grid.
        //
        // The per-glyph offsets from shaping are left fractional deliberately:
        // rounding them would re-space the letters, and the grid the run
        // origin lands on is what determines whether the rasters themselves
        // sit on texel boundaries.
        let dpr = ctx.device_pixel_ratio().max(1.0);
        let snap = |offset: Offset| {
            let x = (offset.dx * dpr).round() / dpr;
            let y = (offset.dy * dpr).round() / dpr;
            Offset::new(x, y)
        };
        for run in paragraph.runs() {
            // The paragraph is shaped at its own origin; place it at this
            // object's. Baselines stay baselines — shifting by the offset only.
            let mut placed = run.clone();
            placed.origin = snap(Offset::new(
                run.origin.dx + origin.dx,
                run.origin.dy + origin.dy,
            ));
            ctx.canvas().draw_glyphs(&placed);
            paint_missing(ctx, &placed);
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Text is a target: a tap on a word has to reach it for selection and
        // for cursor placement.
        true
    }

    /// Shaped, not estimated.
    ///
    /// The maximum width is the paragraph on one line; the minimum is the
    /// paragraph wrapped as hard as it will go, which is the longest
    /// unbreakable word. Both fall out of `Paragraph::layout` at the two
    /// extreme widths, so this measures with the same code path that lays the
    /// text out — an intrinsic that estimated instead would be a second text
    /// engine to keep in agreement with the first.
    ///
    /// Height with no width to wrap against is the single-line height; that is
    /// the honest answer to a question that did not say how wide, and callers
    /// that care pass `IntrinsicQuery::across`.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        use crate::Extremum;

        let spans = [TextSpan::new(self.text.clone(), self.style)];
        let width = match (query.axis, query.extremum) {
            // As wide as it wants: no wrapping at all.
            (Axis::Horizontal, Extremum::Max) => f32::INFINITY,
            // As narrow as it can be: wrap at zero and the shaper still cannot
            // break inside a word, so the widest line it produces *is* the
            // minimum. Reading the width back out is the measurement.
            (Axis::Horizontal, Extremum::Min) => 0.0,
            // Height: whatever width the caller says, or one line if it did not
            // say. Min and max height are the same thing for a paragraph — it
            // has no slack once the width is fixed.
            (Axis::Vertical, _) => query.cross.unwrap_or(f32::INFINITY),
        };

        let paragraph =
            Paragraph::layout_aligned(ctx.fonts_mut(), &spans, width, self.align, self.direction);
        Some(match query.axis {
            // `widest_line`, not `size().width`. `size()` is clamped to the
            // width it was laid out against so that a caller can trust it fits
            // — which for the minimum, laid out at zero, is zero. The first
            // version of this returned exactly that.
            Axis::Horizontal => paragraph.widest_line(),
            Axis::Vertical => paragraph.size().height,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        // Reached only when `layout_differs` said no, so the two describe the
        // same text in the same style and the old shaping is still the right
        // answer. Without this, every rebuild above a label drops the paragraph
        // that `paint` needs and the label disappears.
        //
        // **The gate is load-bearing and was briefly removed.** `layout` reuses
        // `shaped` when it is present rather than recomputing it, so adopting a
        // paragraph across a genuine text change means the field never reshapes
        // and the caret is measured against the previous string. See
        // `RenderObject::adopt_reports` for the case that needs the opposite
        // rule, and why it is a separate hook rather than a looser gate here.
        let old: &dyn std::any::Any = old;
        if let Some(old) = old.downcast_ref::<Self>() {
            self.shaped.clone_from(&old.shaped);
            // The cut comes across with the paragraph it produced, or `paint`
            // would draw a shaping of `login_scr…test.rs` while `displayed`
            // reported the whole name.
            self.elided.clone_from(&old.elided);
        }
    }

    fn semantics(&self) -> Option<crate::Semantics> {
        // Empty text is a spacer that happens to be a label. Announcing it would
        // have a screen reader stop on nothing and say nothing.
        (!self.text.trim().is_empty()).then(|| crate::Semantics::label(self.text.clone()))
    }

    fn debug_name(&self) -> &'static str {
        "RenderText"
    }
}

/// An inspector row is one line and a paragraph is not. Cut on a *character*
/// boundary rather than a byte one, because slicing a `String` at byte 48 is a
/// panic on any text that is not ASCII — the failure mode the rest of this
/// crate's offset handling is careful about, and no less a panic here.
fn truncated(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut out: String = line.chars().take(48).collect();
    if out.chars().count() < line.chars().count() || text.contains('\n') {
        out.push('\u{2026}');
    }
    format!("{out:?}")
}

/// Stroke a box where the font had no glyph.
///
/// # Why the renderer draws this rather than the font
///
/// Every font reserves glyph 0 for "no coverage", and most draw it as a hollow
/// box — the "tofu" a reader recognises as *this text needs a font I do not
/// have*. The embedded DejaVu subset draws glyph 0 as **nothing**: correct
/// advance, no ink. So a screen of Chinese on a device with no CJK font came
/// out as perfectly-spaced blank space, which reads as a layout bug rather than
/// a font one and is the single most misleading way text can fail.
///
/// Drawing the box here, from the glyph ids, makes the behaviour independent of
/// which font happened to be picked — a font that *does* draw tofu gets its own
/// box plus this one, which overlap and read as one.
///
/// Cheap by construction: the filter is over glyph ids already in hand, and a
/// run with full coverage allocates nothing and issues no command.
fn paint_missing(ctx: &mut PaintCtx<'_>, run: &vieww_foundation::GlyphRun) {
    if run.missing_count() == 0 {
        return;
    }

    // The text's own colour, dimmed: a missing character is still text and
    // should not shout louder than the words around it.
    let color = run.color.with_alpha(0x8C);
    // Hairline at small sizes, still visible at large ones.
    let width = (run.size * 0.06).clamp(1.0, 2.0);

    for box_ in run.missing_boxes() {
        // The box is drawn as the **fill of a pre-expanded outline**, not as a
        // stroke: the same conversion `RenderIcon::paint` makes, for the same
        // reason — a hairline is the one primitive the rasterising backends
        // disagree about most, and a tofu box is exactly the thin box a
        // disagreement shows in. A join this conversion rounds is half the
        // hairline wide, which is not a corner any eye has resolved.
        ctx.canvas().fill_path(
            &vieww_foundation::Path::rect(box_).stroke_outline(width),
            vieww_paint::Paint::solid(color),
        );
    }
}

#[cfg(test)]
mod tests {
    use vieww_text::FontStore;

    use super::*;
    use crate::RenderTree;

    /// Lay a `RenderText` out in a tree and return its size.
    fn layout(object: RenderText, constraints: Constraints) -> (Size, RenderTree, crate::RenderId) {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        let size = tree.layout(id, constraints);
        (size, tree, id)
    }

    fn loose(width: f32) -> Constraints {
        Constraints::loose(Size::new(width, f32::INFINITY))
    }

    #[test]
    fn a_run_is_measured_by_the_font_rather_than_a_fixed_ratio() {
        let (narrow, ..) = layout(
            RenderText::new("iiiii", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );
        let (wide, ..) = layout(
            RenderText::new("WWWWW", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );

        assert!(
            wide.width > narrow.width * 1.5,
            "a proportional font makes W much wider than i: {} vs {}",
            wide.width,
            narrow.width
        );
    }

    #[test]
    fn a_run_wraps_when_it_exceeds_the_maximum_width() {
        let text = "the quick brown fox jumps over the lazy dog";
        let (unwrapped, ..) = layout(
            RenderText::new(text, TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );
        let (wrapped, ..) = layout(RenderText::new(text, TextStyle::new(16.0)), loose(120.0));

        assert!(wrapped.width <= 120.0, "{wrapped}");
        assert!(
            wrapped.height > unwrapped.height,
            "wrapping makes it taller: {wrapped} vs {unwrapped}"
        );
    }

    #[test]
    fn empty_text_still_occupies_one_line() {
        let (size, ..) = layout(
            RenderText::new("", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );

        assert_eq!(size.width, 0.0);
        assert!(size.height > 0.0, "a caret needs somewhere to sit: {size}");
    }

    #[test]
    fn explicit_newlines_start_new_lines() {
        let (one, ..) = layout(
            RenderText::new("ab", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );
        let (three, ..) = layout(
            RenderText::new("ab\ncd\nef", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );

        assert!(
            (three.height - one.height * 3.0).abs() < 1.0,
            "three lines is three times one: {three} vs {one}"
        );
    }

    #[test]
    fn what_layout_shaped_is_what_paint_draws() {
        let (_, tree, _) = layout(
            RenderText::new("hello", TextStyle::new(16.0)),
            loose(f32::INFINITY),
        );

        let mut scene = vieww_paint::Scene::new();
        tree.paint(&mut scene);

        let runs = scene.glyph_runs();
        assert_eq!(runs.len(), 1, "one style, one font, so one run");
        assert_eq!(
            runs[0].1.glyphs.len(),
            5,
            "shaped in layout and emitted unchanged in paint"
        );
    }

    #[test]
    fn painting_places_the_run_at_the_objects_origin() {
        let mut tree = RenderTree::new();
        // A padding pushes the text away from the origin, so a run drawn at the
        // paragraph's own origin rather than the object's would be visibly wrong.
        let pad = tree.insert(
            None,
            Box::new(crate::RenderPadding::new(
                vieww_foundation::EdgeInsets::all(20.0),
            )),
        );
        tree.insert(
            Some(pad),
            Box::new(RenderText::new("hello", TextStyle::new(16.0))),
        );
        tree.layout_root(Constraints::loose(Size::new(400.0, 400.0)));

        let mut scene = vieww_paint::Scene::new();
        tree.paint(&mut scene);

        let (baseline, run) = scene.glyph_runs()[0];
        assert!(
            baseline.dx >= 20.0,
            "shifted right by the padding: {baseline}"
        );
        // Within half a point, not exactly: the run origin is snapped to the
        // pixel grid (see `paint`), which can move the baseline by up to half
        // a point from where layout put it. The snap is deliberate — a glyph
        // rasterised at a fractional position is blurred across texels — and
        // half a point of baseline movement is invisible beside that.
        assert!(
            baseline.dy + 0.5 >= 20.0 + run.ascent,
            "and down by the padding plus an ascent: {baseline}"
        );
        // The run origin is snapped to the physical pixel grid (see `paint`),
        // so at 1:1 it is whole points — the placement the crisp-raster fix
        // exists to buy, and a regression guard against a future change
        // snapping somewhere other than the run origin.
        assert!(
            (baseline.dx - baseline.dx.round()).abs() < 1e-4,
            "the run origin sits on the pixel grid: {baseline}"
        );
        assert!(
            (baseline.dy - baseline.dy.round()).abs() < 1e-4,
            "the run origin sits on the pixel grid: {baseline}"
        );
    }

    #[test]
    fn a_cached_paragraph_does_not_make_an_identical_object_differ() {
        let mut laid_out = RenderText::new("hello", TextStyle::new(16.0));
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(laid_out.clone()));
        tree.layout(id, loose(f32::INFINITY));

        // Simulate the object having been laid out, then compared against the
        // freshly built one a rebuild would produce.
        let mut store = FontStore::embedded_only();
        let spans = [TextSpan::new("hello", TextStyle::new(16.0))];
        laid_out.shaped = Some(Paragraph::layout(&mut store, &spans, f32::INFINITY));
        let rebuilt = RenderText::new("hello", TextStyle::new(16.0));

        assert_eq!(
            laid_out, rebuilt,
            "the cache is derived, so it must not make a rebuild relayout all text"
        );
    }

    #[test]
    fn a_rebuild_that_changes_nothing_does_not_erase_the_text() {
        let mut tree = RenderTree::new();
        let id = tree.insert(
            None,
            Box::new(RenderText::new("hello", TextStyle::new(16.0))),
        );
        tree.layout(id, loose(f32::INFINITY));

        // What a rebuild above a label does: an equal-but-new object replaces the
        // old one. Nothing lays it out, because nothing about it differs — so the
        // shaped paragraph has to come across with it or `paint` has nothing to
        // draw and the label vanishes.
        tree.replace_object(id, Box::new(RenderText::new("hello", TextStyle::new(16.0))));

        let mut scene = vieww_paint::Scene::new();
        tree.paint(&mut scene);
        assert_eq!(
            scene.glyph_runs().len(),
            1,
            "a rebuild that changed nothing must not erase what was drawn"
        );
    }

    #[test]
    fn a_rebuild_that_changes_the_text_relayouts_rather_than_adopting() {
        let mut tree = RenderTree::new();
        let id = tree.insert(
            None,
            Box::new(RenderText::new("hello", TextStyle::new(16.0))),
        );
        tree.layout(id, loose(f32::INFINITY));

        // Different text, so the geometry moved and the old shaping is wrong.
        // Adopting it would draw "hello" where "goodbye" was asked for.
        tree.replace_object(
            id,
            Box::new(RenderText::new("goodbye", TextStyle::new(16.0))),
        );
        tree.layout_root(loose(f32::INFINITY));

        let mut scene = vieww_paint::Scene::new();
        tree.paint(&mut scene);
        assert_eq!(
            scene.glyph_runs()[0].1.glyphs.len(),
            7,
            "the new text was shaped, not the old one adopted"
        );
    }

    #[test]
    fn changing_the_text_makes_the_object_differ() {
        let a = RenderText::new("hello", TextStyle::new(16.0));
        let b = RenderText::new("goodbye", TextStyle::new(16.0));
        assert_ne!(a, b);
    }

    #[test]
    fn an_eliding_run_is_cut_to_the_width_it_was_given() {
        // The property a character count cannot have: the same slot, two names
        // of the same length, and the answer differs because the *glyphs* do.
        let wide = RenderText::new("W".repeat(40), TextStyle::new(12.5))
            .overflow(TextOverflow::EllipsisMiddle)
            .max_lines(Some(1));
        let narrow = RenderText::new("i".repeat(40), TextStyle::new(12.5))
            .overflow(TextOverflow::EllipsisMiddle)
            .max_lines(Some(1));

        let (wide_size, wide_tree, wide_id) = layout(wide, loose(150.0));
        let (narrow_size, ..) = layout(narrow, loose(150.0));

        assert!(wide_size.width <= 150.0, "it fits the slot: {wide_size}");
        assert!(
            narrow_size.width <= 150.0,
            "so does the other: {narrow_size}"
        );

        let drawn = wide_tree
            .object(wide_id)
            .and_then(|object| {
                let object: &dyn std::any::Any = object;
                object.downcast_ref::<RenderText>()
            })
            .expect("the object is still a RenderText")
            .displayed()
            .to_owned();
        assert!(
            drawn.contains('\u{2026}'),
            "and it says it was cut: {drawn:?}"
        );
        assert!(
            drawn.chars().count() < 40,
            "by removing characters: {drawn:?}"
        );
    }

    #[test]
    fn a_run_that_fits_is_never_cut() {
        let object = RenderText::new("main.rs", TextStyle::new(12.5))
            .overflow(TextOverflow::EllipsisMiddle)
            .max_lines(Some(1));
        let (_, tree, id) = layout(object, loose(150.0));
        let object = tree.object(id).expect("laid out");
        let object: &dyn std::any::Any = object;
        let text = object.downcast_ref::<RenderText>().expect("a RenderText");
        assert_eq!(text.displayed(), "main.rs");
    }

    #[test]
    fn elision_leaves_the_announced_text_alone() {
        // A screen reader should hear the file, not the abbreviation the strip
        // had room for.
        let mut object = RenderText::new("a_very_long_module_name.rs", TextStyle::new(12.5))
            .overflow(TextOverflow::Ellipsis)
            .max_lines(Some(1));
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(object.clone()));
        tree.layout(id, loose(60.0));
        object.text = object.text.clone();

        let laid_out = tree.object(id).expect("laid out");
        let laid_out: &dyn std::any::Any = laid_out;
        let laid_out = laid_out.downcast_ref::<RenderText>().expect("a RenderText");
        assert!(laid_out.displayed().chars().count() < laid_out.text.chars().count());
        assert_eq!(
            laid_out.semantics().map(|semantics| semantics.label),
            Some(Some("a_very_long_module_name.rs".to_owned())),
            "semantics announce the whole name"
        );
    }

    #[test]
    fn a_larger_style_measures_larger() {
        let (small, ..) = layout(
            RenderText::new("Handgloves", TextStyle::new(12.0)),
            loose(f32::INFINITY),
        );
        let (large, ..) = layout(
            RenderText::new("Handgloves", TextStyle::new(32.0)),
            loose(f32::INFINITY),
        );

        assert!(large.width > small.width);
        assert!(large.height > small.height);
    }
}
