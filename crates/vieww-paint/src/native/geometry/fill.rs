//! Path verbs to coverage: the scanline rasterizer.
//!
//! Spec §12.1: "scanline rasterizer with 4x4 supersampling ... nonzero
//! winding ... deliberately slow and deliberately simple; its job is to be
//! obviously right so the GPU can be measurably right." This is that
//! rasterizer. It backs every CPU fill in this crate — shapes, strokes
//! (already expanded to a fill outline by [`super::stroke`]), glyphs and
//! shadow masks all end here — so there is exactly one place nonzero winding
//! is decided, matching spec §1.3's "even-odd is not supported; the display
//! list resolves fills to nonzero winding only."
//!
//! # Why 4 vertical samples and analytic horizontal coverage, not 4x4 points
//!
//! A 4x4 point grid (16 point-in-polygon tests per pixel) and "4 scanlines
//! with exact horizontal span coverage" converge to the same answer for
//! straight edges and are both called "4x4" informally; this module takes the
//! second because it is exact in one axis instead of approximate in both, for
//! the same sample count.

use vieww_foundation::Rect;

use super::flatten::Polyline;

/// Vertical supersamples per pixel row (spec §12.1's "4x4": 4 here, and
/// exact analytic coverage — not a 4th sample — in the horizontal axis).
const SUBSAMPLES: usize = 4;
const SUBSAMPLE_WEIGHT: f32 = 1.0 / SUBSAMPLES as f32;
/// Offsets within a pixel row, centred, matching a standard box filter.
const SUBSAMPLE_OFFSETS: [f32; SUBSAMPLES] = [0.125, 0.375, 0.625, 0.875];

/// A rasterized coverage mask: `data[y * width + x]` is how much of pixel
/// `(x0 + x, y0 + y)` the shape covers, `0.0` to `1.0`.
#[derive(Debug, Clone)]
pub(crate) struct CoverageMask {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) data: Vec<f32>,
}

impl CoverageMask {
    pub(crate) fn empty() -> Self {
        Self {
            x0: 0,
            y0: 0,
            width: 0,
            height: 0,
            data: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// How many coverage values this mask holds — what a cache charges itself
    /// for holding one.
    #[must_use]
    pub(crate) fn data_len(&self) -> usize {
        self.data.len()
    }

    /// This whole mask, as a view, at the position it was rasterised at.
    #[must_use]
    pub(crate) fn view(&self) -> MaskView<'_> {
        MaskView {
            data: &self.data,
            stride: self.width as usize,
            offset: 0,
            x0: self.x0,
            y0: self.y0,
            width: self.width,
            height: self.height,
        }
    }

    /// This mask, translated by `(dx, dy)` device pixels and then narrowed to
    /// `limit`.
    ///
    /// A **translation**, not a placement: the mask keeps its own `x0`/`y0`,
    /// which for a glyph is where its ink sits relative to the origin it was
    /// rasterised at — above the baseline, and usually left of the pen. Treating
    /// those as zero and placing the mask at the caller's origin instead cuts
    /// the top off every glyph in the window, which is what it looks like.
    ///
    /// # Why a cached glyph needs both
    ///
    /// A rasterised glyph is reusable across every place the same glyph is
    /// drawn at the same sub-pixel phase — which is what makes caching it worth
    /// doing — but only if it can be *moved*, because the whole point is that
    /// the next occurrence is somewhere else. And it has to be narrowed,
    /// because the mask was rasterised with no clip (a clip differs between
    /// occurrences, so baking one in would make the entry unshareable) and the
    /// clip has to be applied on the way out instead.
    ///
    /// Returns `None` when nothing survives the narrowing, which is the common
    /// answer for a line of text scrolled past the edge of its pane.
    #[must_use]
    pub(crate) fn view_translated(&self, dx: i32, dy: i32, limit: Rect) -> Option<MaskView<'_>> {
        if self.is_empty() {
            return None;
        }
        let x0 = self.x0 + dx;
        let y0 = self.y0 + dy;
        let x1 = x0 + self.width as i32;
        let y1 = y0 + self.height as i32;
        let lx0 = limit.left.floor().max(x0 as f32) as i32;
        let ly0 = limit.top.floor().max(y0 as f32) as i32;
        let lx1 = limit.right.ceil().min(x1 as f32) as i32;
        let ly1 = limit.bottom.ceil().min(y1 as f32) as i32;
        if lx1 <= lx0 || ly1 <= ly0 {
            return None;
        }
        let stride = self.width as usize;
        let offset = (ly0 - y0) as usize * stride + (lx0 - x0) as usize;
        Some(MaskView {
            data: &self.data,
            stride,
            offset,
            x0: lx0,
            y0: ly0,
            width: (lx1 - lx0) as u32,
            height: (ly1 - ly0) as u32,
        })
    }

    #[must_use]
    pub(crate) fn coverage_at(&self, x: i32, y: i32) -> f32 {
        if x < self.x0 || y < self.y0 {
            return 0.0;
        }
        let (dx, dy) = ((x - self.x0) as u32, (y - self.y0) as u32);
        if dx >= self.width || dy >= self.height {
            return 0.0;
        }
        self.data[(dy * self.width + dx) as usize]
    }
}

/// A rectangular window onto a [`CoverageMask`]'s pixels, positioned in device
/// space independently of where the mask was rasterised.
///
/// The compositing methods on `Target` take one of these rather than a
/// `&CoverageMask` so that a mask can be reused at a different position and
/// under a different clip without copying its pixels — see
/// [`CoverageMask::view_at`], and `native/glyph_raster.rs` for what needs that.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MaskView<'a> {
    /// The whole mask's coverage values.
    data: &'a [f32],
    /// How many values make up one row of `data`.
    stride: usize,
    /// Where in `data` this view's first row starts.
    offset: usize,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl MaskView<'_> {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Row `dy` of this view, exactly `width` values wide.
    #[must_use]
    pub(crate) fn row(&self, dy: u32) -> &[f32] {
        let start = self.offset + dy as usize * self.stride;
        &self.data[start..start + self.width as usize]
    }
}

/// How many sub-columns one pixel is split into for LCD coverage: the three
/// RGB stripes a colour display lays out horizontally.
pub(crate) const LCD_SUBPIXELS: usize = 3;

/// LCD-subpixel coverage: [`CoverageMask`]'s answer at three times the
/// horizontal resolution.
///
/// `data[(y * width + x) * 3 + channel]` is how much of sub-column
/// `channel` (0=R, 1=G, 2=B, left to right) of pixel `(x0 + x, y0 + y)` the
/// shape covers. A display's sub-pixels are physically ordered R-G-B across a
/// pixel, which is what makes per-channel coverage sharper than per-pixel
/// coverage: a vertical stem whose left edge sits at the pixel's centre
/// covers the B stripe fully and the R stripe not at all, and a grayscale
/// rasterizer has to average that into one number for all three.
///
/// # The one identity this mask is built to keep
///
/// The three channels of a pixel average to exactly what [`CoverageMask`]
/// holds for the same shape at the same position — both are analytic span
/// coverage, one summed over thirds of a pixel and one over the whole pixel —
/// so `(r + g + b) / 3 == gray`, per pixel, for any shape. That identity is
/// what lets an LCD rasteriser live beside the grayscale one without a second
/// definition of "correct", and `lcd_channels_average_to_the_gray_coverage`
/// asserts it. It is also the fallback: composite an LCD mask with all three
/// channels equal and the arithmetic is the gray path's, bit for bit.
#[derive(Debug, Clone)]
pub(crate) struct LcdMask {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Three values per pixel, RGB-interleaved: see the type documentation.
    pub(crate) data: Vec<f32>,
}

impl LcdMask {
    pub(crate) fn empty() -> Self {
        Self {
            x0: 0,
            y0: 0,
            width: 0,
            height: 0,
            data: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// How many coverage values this mask holds — the cache-budget cost.
    #[must_use]
    pub(crate) fn data_len(&self) -> usize {
        self.data.len()
    }

    /// The three channel coverages at pixel `(x, y)`, or `[0; 3]` outside.
    ///
    /// Test-only because the renderer composites through [`LcdMaskView`]
    /// rows, not through per-pixel reads — the assertions that need this
    /// accessor are the ones proving what the channels hold.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn coverage_at(&self, x: i32, y: i32) -> [f32; LCD_SUBPIXELS] {
        if x < self.x0 || y < self.y0 {
            return [0.0; LCD_SUBPIXELS];
        }
        let (dx, dy) = ((x - self.x0) as u32, (y - self.y0) as u32);
        if dx >= self.width || dy >= self.height {
            return [0.0; LCD_SUBPIXELS];
        }
        let base = ((dy * self.width + dx) as usize) * LCD_SUBPIXELS;
        [self.data[base], self.data[base + 1], self.data[base + 2]]
    }

    /// [`CoverageMask::view_translated`](CoverageMask::view_translated) for
    /// this mask: translated by `(dx, dy)` device pixels, narrowed to
    /// `limit`, with the same reuse-at-another-position contract.
    #[must_use]
    pub(crate) fn view_translated(&self, dx: i32, dy: i32, limit: Rect) -> Option<LcdMaskView<'_>> {
        if self.is_empty() {
            return None;
        }
        let x0 = self.x0 + dx;
        let y0 = self.y0 + dy;
        let x1 = x0 + self.width as i32;
        let y1 = y0 + self.height as i32;
        let lx0 = limit.left.floor().max(x0 as f32) as i32;
        let ly0 = limit.top.floor().max(y0 as f32) as i32;
        let lx1 = limit.right.ceil().min(x1 as f32) as i32;
        let ly1 = limit.bottom.ceil().min(y1 as f32) as i32;
        if lx1 <= lx0 || ly1 <= ly0 {
            return None;
        }
        let stride = self.width as usize * LCD_SUBPIXELS;
        let offset = (ly0 - y0) as usize * stride + (lx0 - x0) as usize * LCD_SUBPIXELS;
        Some(LcdMaskView {
            data: &self.data,
            stride,
            offset,
            x0: lx0,
            y0: ly0,
            width: (lx1 - lx0) as u32,
            height: (ly1 - ly0) as u32,
        })
    }
}

/// [`MaskView`], over an [`LcdMask`]: three coverage values per pixel.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LcdMaskView<'a> {
    data: &'a [f32],
    /// Values per mask row: `3 * width`.
    stride: usize,
    offset: usize,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl LcdMaskView<'_> {
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Row `dy` of this view, exactly `3 * width` values wide, RGB per pixel.
    #[must_use]
    pub(crate) fn row(&self, dy: u32) -> &[f32] {
        let start = self.offset + dy as usize * self.stride;
        &self.data[start..start + self.width as usize * LCD_SUBPIXELS]
    }
}

struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    /// The edge's vertical span, lowest first — `y0`/`y1` in whichever order
    /// puts the smaller one first. Precomputed because the scanline loop asks
    /// for it several times per row per edge, and because the active-edge
    /// table below sorts and retires edges on exactly these two numbers.
    ymin: f32,
    ymax: f32,
    /// +1 if the edge runs downward (y increasing), -1 if upward. Zero-length
    /// (in y) edges never reach here — see [`edges_from`].
    winding: i32,
}

fn edges_from(polylines: &[Polyline]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for line in polylines {
        let points = &line.points;
        if points.len() < 2 {
            continue;
        }
        // Fill always closes: an open contour is closed with an implicit
        // segment back to its start, which is what "nonzero winding" means
        // for a shape whose outline was never explicitly closed.
        let n = points.len();
        for i in 0..n {
            let (x0, y0) = points[i];
            let (x1, y1) = points[(i + 1) % n];
            if (y0 - y1).abs() < f32::EPSILON {
                continue; // horizontal edges never cross a scanline
            }
            let winding = if y1 > y0 { 1 } else { -1 };
            let (ymin, ymax) = if y0 < y1 { (y0, y1) } else { (y1, y0) };
            edges.push(Edge {
                x0,
                y0,
                x1,
                y1,
                ymin,
                ymax,
                winding,
            });
        }
    }
    edges
}

// How many times `rasterize` has actually run its scanline pass, this
// thread. Test-only instrumentation: it exists so `native/instancing.rs`'s
// pillar-D tests can *measure* the rasterization-work reduction batching
// gives, rather than only asserting it by construction (calling `rasterize`
// once vs. in a loop). A plain comment, not a doc comment: rustdoc has
// nothing to attach a doc comment to on a macro invocation.
#[cfg(test)]
thread_local! {
    static RASTERIZE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn rasterize_call_count() -> usize {
    RASTERIZE_CALLS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn reset_rasterize_call_count() {
    RASTERIZE_CALLS.with(|c| c.set(0));
}

/// Rasterize `polylines` (device space) into a coverage mask, clipped to
/// `clip_bounds` (also device space, already the intersection of every clip
/// and the target surface — see `compositor::ClipStack`).
pub(crate) fn rasterize(polylines: &[Polyline], clip_bounds: Rect) -> CoverageMask {
    #[cfg(test)]
    RASTERIZE_CALLS.with(|c| c.set(c.get() + 1));
    // **The axis-aligned rectangle does not need a scanline pass.**
    //
    // It is also, by a wide margin, the most common shape a user interface
    // draws: every panel, every divider, every row background, every table
    // cell, every solid button face — and, through `native/clip.rs`, every
    // rectangular clip, which the studio's shell alone pushes dozens of per
    // frame. The general path built four `Edge`s for it, sorted them, walked an
    // active-edge table over every pixel row, sorted a two-element crossings
    // list four times per row, and allocated a zeroed `f32` buffer to write the
    // answer into — to compute a function that is separable and closed-form.
    //
    // [`rect_mask`] computes that closed form. It is not an approximation of
    // what the scanline pass produces: it reproduces the same
    // four-subsample vertical quantisation and the same exact horizontal
    // coverage, deliberately, so a rectangle rasterised either way is the same
    // bytes — see `the_rect_fast_path_matches_the_scanline_pass` below, which
    // asserts exactly that over a sweep of sub-pixel offsets.
    if let Some(rect) = axis_aligned_rect(polylines) {
        return rect_mask(rect, clip_bounds);
    }
    let edges = edges_from(polylines);
    if edges.is_empty() || clip_bounds.is_empty() {
        return CoverageMask::empty();
    }

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for edge in &edges {
        min_x = min_x.min(edge.x0).min(edge.x1);
        max_x = max_x.max(edge.x0).max(edge.x1);
        min_y = min_y.min(edge.y0).min(edge.y1);
        max_y = max_y.max(edge.y0).max(edge.y1);
    }

    let bx0 = min_x.floor().max(clip_bounds.left) as i32;
    let bx1 = max_x.ceil().min(clip_bounds.right) as i32;
    let by0 = min_y.floor().max(clip_bounds.top) as i32;
    let by1 = max_y.ceil().min(clip_bounds.bottom) as i32;
    if bx1 <= bx0 || by1 <= by0 {
        return CoverageMask::empty();
    }
    let width = (bx1 - bx0) as u32;
    let height = (by1 - by0) as u32;
    let mut data = vec![0.0f32; (width as usize) * (height as usize)];

    let mut crossings: Vec<(f32, i32)> = Vec::new();
    let mut row = vec![0.0f32; width as usize];

    // **The active edge table.** Without it this loop asked every edge in the
    // shape whether it crossed every subsample of every scanline — `rows × 4 ×
    // edges` tests, of which all but a handful answer "no". That is invisible
    // on a rectangle (four edges) and dominant on the shapes a real interface
    // is actually made of: a flattened curve, a glyph outline, a stroke
    // expanded into a polygon. A 400-row portrait path with 500 edges paid
    // 800,000 rejections per fill to find perhaps 2,000 real crossings.
    //
    // Edges are visited in order of where they *start*, admitted to `active`
    // when the scanline reaches them and retired when it passes them, so each
    // row tests only the edges that genuinely span it. The inner
    // `sy < ymin || sy >= ymax` guard stays: an edge is admitted for the whole
    // pixel row and may still miss an individual subsample inside it.
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_unstable_by(|&a, &b| edges[a].ymin.total_cmp(&edges[b].ymin));
    let mut pending = 0usize;
    let mut active: Vec<usize> = Vec::new();

    for py in by0..by1 {
        let row_top = py as f32;
        let row_bottom = row_top + 1.0;
        while pending < order.len() && edges[order[pending]].ymin < row_bottom {
            active.push(order[pending]);
            pending += 1;
        }
        active.retain(|&index| edges[index].ymax > row_top);
        if active.is_empty() {
            continue;
        }

        row.iter_mut().for_each(|c| *c = 0.0);
        for &sub in &SUBSAMPLE_OFFSETS {
            let sy = py as f32 + sub;
            crossings.clear();
            for &index in &active {
                let edge = &edges[index];
                if sy < edge.ymin || sy >= edge.ymax {
                    continue;
                }
                let t = (sy - edge.y0) / (edge.y1 - edge.y0);
                let x = edge.x0 + t * (edge.x1 - edge.x0);
                crossings.push((x, edge.winding));
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding_number = 0i32;
            let mut span_start: Option<f32> = None;
            for &(x, w) in &crossings {
                let was_inside = winding_number != 0;
                winding_number += w;
                let is_inside = winding_number != 0;
                if !was_inside && is_inside {
                    span_start = Some(x);
                } else if was_inside && !is_inside {
                    if let Some(start) = span_start.take() {
                        accumulate_span(&mut row, bx0, width, start, x);
                    }
                }
            }
        }
        let dest_row = &mut data[(py - by0) as usize * width as usize..][..width as usize];
        for (d, s) in dest_row.iter_mut().zip(row.iter()) {
            *d = (*s * SUBSAMPLE_WEIGHT).clamp(0.0, 1.0);
        }
    }

    CoverageMask {
        x0: bx0,
        y0: by0,
        width,
        height,
        data,
    }
}

/// [`rasterize`], at three times the horizontal resolution: an
/// [`LcdMask`] instead of a [`CoverageMask`].
///
/// Deliberately **without** `rasterize`'s axis-aligned-rectangle fast path.
/// Glyph outlines — the only thing this is for — are never a four-point
/// axis-aligned rectangle, and a fast path here would be a second copy of
/// `rect_mask`'s sub-column arithmetic to maintain for a shape that never
/// arrives. The general scanline pass handles anything, which is exactly the
/// licence the gray fast path is careful about not overclaiming.
///
/// The vertical policy is the shared one — four subsamples per row at
/// [`SUBSAMPLE_OFFSETS`] — because LCD sharpens the *horizontal* axis, where
/// the display's sub-pixel stripes are, and changing both axes would make
/// this a different rasterizer rather than a finer reading of the same one.
pub(crate) fn rasterize_lcd(polylines: &[Polyline], clip_bounds: Rect) -> LcdMask {
    let edges = edges_from(polylines);
    if edges.is_empty() || clip_bounds.is_empty() {
        return LcdMask::empty();
    }

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for edge in &edges {
        min_x = min_x.min(edge.x0).min(edge.x1);
        max_x = max_x.max(edge.x0).max(edge.x1);
        min_y = min_y.min(edge.y0).min(edge.y1);
        max_y = max_y.max(edge.y0).max(edge.y1);
    }

    let bx0 = min_x.floor().max(clip_bounds.left) as i32;
    let bx1 = max_x.ceil().min(clip_bounds.right) as i32;
    let by0 = min_y.floor().max(clip_bounds.top) as i32;
    let by1 = max_y.ceil().min(clip_bounds.bottom) as i32;
    if bx1 <= bx0 || by1 <= by0 {
        return LcdMask::empty();
    }
    let width = (bx1 - bx0) as u32;
    let height = (by1 - by0) as u32;
    let sub_width = width as usize * LCD_SUBPIXELS;
    let mut data = vec![0.0f32; sub_width * height as usize];

    let mut crossings: Vec<(f32, i32)> = Vec::new();
    // Per-subsample accumulation, in *pixel fractions*: entry j holds how much
    // of sub-column j's third of the pixel the spans so far covered. The
    // finalisation pass multiplies by 3 to turn that into a fraction *of the
    // sub-column*, which is what a display's sub-pixel is.
    let mut row = vec![0.0f32; sub_width];

    // The same active-edge table `rasterize` uses, for the same reason: this
    // runs per glyph per miss, and the edges that don't span a row are the
    // common case for every row of every glyph.
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_unstable_by(|&a, &b| edges[a].ymin.total_cmp(&edges[b].ymin));
    let mut pending = 0usize;
    let mut active: Vec<usize> = Vec::new();

    for py in by0..by1 {
        let row_top = py as f32;
        let row_bottom = row_top + 1.0;
        while pending < order.len() && edges[order[pending]].ymin < row_bottom {
            active.push(order[pending]);
            pending += 1;
        }
        active.retain(|&index| edges[index].ymax > row_top);
        if active.is_empty() {
            continue;
        }

        row.iter_mut().for_each(|c| *c = 0.0);
        for &sub in &SUBSAMPLE_OFFSETS {
            let sy = py as f32 + sub;
            crossings.clear();
            for &index in &active {
                let edge = &edges[index];
                if sy < edge.ymin || sy >= edge.ymax {
                    continue;
                }
                let t = (sy - edge.y0) / (edge.y1 - edge.y0);
                let x = edge.x0 + t * (edge.x1 - edge.x0);
                crossings.push((x, edge.winding));
            }
            if crossings.is_empty() {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));

            let mut winding_number = 0i32;
            let mut span_start: Option<f32> = None;
            for &(x, w) in &crossings {
                let was_inside = winding_number != 0;
                winding_number += w;
                let is_inside = winding_number != 0;
                if !was_inside && is_inside {
                    span_start = Some(x);
                } else if was_inside && !is_inside {
                    if let Some(start) = span_start.take() {
                        accumulate_span_lcd(&mut row, bx0, sub_width, start, x);
                    }
                }
            }
        }
        let dest_row = &mut data[(py - by0) as usize * sub_width..][..sub_width];
        for (d, s) in dest_row.iter_mut().zip(row.iter()) {
            // *3 turns a fraction of the pixel into a fraction of the
            // sub-column; /4 averages the four vertical subsamples, exactly as
            // `rasterize` does. Clamped, as there, because a many-edge glyph
            // can accumulate a hair over 1.0 from float summation.
            *d = (s * SUBSAMPLE_WEIGHT * LCD_SUBPIXELS as f32).clamp(0.0, 1.0);
        }
    }

    LcdMask {
        x0: bx0,
        y0: by0,
        width,
        height,
        data,
    }
}

/// `polylines` as one axis-aligned rectangle, if that is what they are.
///
/// Deliberately strict: one contour, four corners, and each edge either
/// horizontal or vertical. Anything else — a rotated rectangle, a rounded one,
/// a triangle that happens to have an axis-aligned side — falls through to the
/// general rasterizer, which is always correct. A fast path that is wrong about
/// what it applies to is worse than no fast path, so this answers `None`
/// whenever it is not certain.
fn axis_aligned_rect(polylines: &[Polyline]) -> Option<Rect> {
    let [line] = polylines else { return None };
    let [a, b, c, d] = line.points[..] else {
        return None;
    };
    // Two windings produce a rectangle from four points: corners in order
    // starting with a horizontal edge, or starting with a vertical one.
    let horizontal_first = a.1 == b.1 && b.0 == c.0 && c.1 == d.1 && d.0 == a.0;
    let vertical_first = a.0 == b.0 && b.1 == c.1 && c.0 == d.0 && d.1 == a.1;
    if !horizontal_first && !vertical_first {
        return None;
    }
    let (left, right) = (a.0.min(c.0), a.0.max(c.0));
    let (top, bottom) = (a.1.min(c.1), a.1.max(c.1));
    if !(left.is_finite() && right.is_finite() && top.is_finite() && bottom.is_finite()) {
        return None;
    }
    Some(Rect::new(left, top, right, bottom))
}

/// [`rasterize`]'s answer for an axis-aligned rectangle, in closed form.
///
/// # The two quantisations, and why one of them is kept on purpose
///
/// Horizontal coverage is exact — the overlap of `[left, right)` with each
/// pixel column, which is what [`accumulate_span`] computes.
///
/// Vertical coverage is **not** exact, and must not be: the scanline pass
/// samples four times per pixel row at [`SUBSAMPLE_OFFSETS`] and averages, so a
/// row is covered in quarters. Computing the exact vertical overlap here would
/// be *better* anti-aliasing and would also make every rectangle in the
/// framework differ from every other shape by a fraction of a byte along its
/// top and bottom edges — two rasterizers disagreeing, which is the thing a
/// single rasterizer exists to prevent. So this counts subsamples, exactly as
/// the general path does.
fn rect_mask(rect: Rect, clip_bounds: Rect) -> CoverageMask {
    if clip_bounds.is_empty() {
        return CoverageMask::empty();
    }
    let bx0 = rect.left.floor().max(clip_bounds.left) as i32;
    let bx1 = rect.right.ceil().min(clip_bounds.right) as i32;
    let by0 = rect.top.floor().max(clip_bounds.top) as i32;
    let by1 = rect.bottom.ceil().min(clip_bounds.bottom) as i32;
    if bx1 <= bx0 || by1 <= by0 {
        return CoverageMask::empty();
    }
    let width = (bx1 - bx0) as u32;
    let height = (by1 - by0) as u32;

    // One row of horizontal coverage, computed once. Every pixel row of the
    // rectangle has the same horizontal profile — that is what "axis-aligned"
    // means — so the interior rows are a `extend_from_slice` of this and the
    // partial top and bottom rows are a scale of it.
    let mut hrow = vec![0.0f32; width as usize];
    accumulate_span(&mut hrow, bx0, width, rect.left, rect.right);

    // **`vec![0.0; n]` and then write only the rows that need it**, rather
    // than `Vec::with_capacity` and pushing every row including the empty ones.
    //
    // The two look equivalent and are not. A zeroed `Vec<f32>` of any size is
    // an allocation the allocator can satisfy with fresh pages that are
    // *already* zero — the kernel guarantees it, so nothing has to be written
    // at all — while pushing zeros writes every one of them. On
    // `examples/fixtures`' `00-fills-rrect-clip` the pushing version executed
    // 15% **fewer instructions** and ran 20% **slower**, which is what that
    // difference looks like from the outside: the work moved from the
    // instruction count into the memory system.
    //
    // Written down because "fewer instructions" was the wrong thing to
    // optimise here and the profile said it was the right thing.
    let mut data = vec![0.0f32; (width as usize) * (height as usize)];
    // The fully-covered profile, pre-clamped, so an interior row is a memcpy.
    let full: Vec<f32> = hrow.iter().map(|c| c.clamp(0.0, 1.0)).collect();

    for py in by0..by1 {
        let row_top = py as f32;
        #[expect(clippy::cast_precision_loss, reason = "subsample count, 0..=4")]
        let inside = SUBSAMPLE_OFFSETS
            .iter()
            .filter(|&&off| {
                let sy = row_top + off;
                sy >= rect.top && sy < rect.bottom
            })
            .count() as f32;
        if inside == 0.0 {
            // Already zero.
            continue;
        }
        let start = (py - by0) as usize * width as usize;
        let row = &mut data[start..start + width as usize];
        if inside == SUBSAMPLES as f32 {
            row.copy_from_slice(&full);
        } else {
            let weight = inside * SUBSAMPLE_WEIGHT;
            for (d, s) in row.iter_mut().zip(hrow.iter()) {
                *d = (s * weight).clamp(0.0, 1.0);
            }
        }
    }

    CoverageMask {
        x0: bx0,
        y0: by0,
        width,
        height,
        data,
    }
}

/// Add exact horizontal coverage for the span `[x_start, x_end)` into `row`,
/// one weight unit per subsample, splitting partial coverage at the two ends.
fn accumulate_span(row: &mut [f32], row_x0: i32, width: u32, x_start: f32, x_end: f32) {
    if x_end <= x_start {
        return;
    }
    let clamp_lo = row_x0 as f32;
    let clamp_hi = row_x0 as f32 + width as f32;
    let x_start = x_start.clamp(clamp_lo, clamp_hi);
    let x_end = x_end.clamp(clamp_lo, clamp_hi);
    if x_end <= x_start {
        return;
    }

    let px_start = x_start.floor() as i32;
    let px_end = (x_end.ceil() as i32 - 1).max(px_start);

    // # The interior is not a special case, it is the common case
    //
    // Every pixel strictly inside the span is covered exactly once — the
    // general `min`/`max` overlap arithmetic can only ever produce `1.0` for
    // it. Running that arithmetic anyway, plus two bounds comparisons, on
    // every interior pixel is four floating-point operations per pixel per
    // subsample to compute a constant: at four subsamples that is sixteen
    // operations per pixel of every filled span in the frame, which for a
    // full-width panel is millions of them.
    //
    // So the span is split the way it actually is: a partial pixel at each
    // end, and a run of whole ones between. The ends still get the exact
    // arithmetic — that is where the anti-aliasing lives and it must not
    // change — and the middle gets a straight `+= 1.0` over a slice, which
    // the compiler can vectorise.
    let lo = px_start.max(row_x0);
    let hi = px_end.min(row_x0 + width as i32 - 1);
    if hi < lo {
        return;
    }

    let interior_start = (x_start.ceil() as i32).max(lo);
    let interior_end = (x_end.floor() as i32 - 1).min(hi);

    if interior_start > interior_end {
        // A span narrower than a whole pixel, or straddling one boundary:
        // there is no fully-covered run, so every pixel is exact arithmetic.
        for px in lo..=hi {
            row[(px - row_x0) as usize] += partial(px, x_start, x_end);
        }
        return;
    }

    for px in lo..interior_start {
        row[(px - row_x0) as usize] += partial(px, x_start, x_end);
    }
    let from = (interior_start - row_x0) as usize;
    let to = (interior_end - row_x0) as usize;
    for cell in &mut row[from..=to] {
        *cell += 1.0;
    }
    for px in (interior_end + 1)..=hi {
        row[(px - row_x0) as usize] += partial(px, x_start, x_end);
    }
}

/// How much of pixel column `px` the span `[x_start, x_end)` covers.
fn partial(px: i32, x_start: f32, x_end: f32) -> f32 {
    let pixel_lo = px as f32;
    let pixel_hi = pixel_lo + 1.0;
    (x_end.min(pixel_hi) - x_start.max(pixel_lo)).max(0.0)
}

/// [`accumulate_span`]'s LCD counterpart: accumulate `[x_start, x_end)` into
/// `row`'s sub-columns instead of its pixel columns.
///
/// `row` is `width * 3` wide and entry `j` covers the device band
/// `[row_x0 + j/3, row_x0 + (j+1)/3)`. The accumulated value is the raw
/// overlap in pixel fractions — not yet scaled to a fraction of the band —
/// which is what keeps the three channels of a pixel summing to exactly the
/// gray span coverage.
///
/// Sub-column boundaries are `row_x0 + j/3` computed per band rather than
/// `row_x0 * 3 + j` divided back down, because the two round differently in
/// `f32` and this one is the form the finalisation pass's `* 3` inverts.
fn accumulate_span_lcd(row: &mut [f32], row_x0: i32, sub_width: usize, x_start: f32, x_end: f32) {
    if x_end <= x_start {
        return;
    }
    let origin = row_x0 as f32;
    // The bands the span can touch: from the one containing x_start to the one
    // before x_end. Clamped to the row, which is the clip's floor already.
    let first = (((x_start - origin) * LCD_SUBPIXELS as f32).floor() as usize).min(sub_width);
    let last = (((x_end - origin) * LCD_SUBPIXELS as f32).ceil() as usize).min(sub_width);
    for (j, cell) in row.iter_mut().enumerate().take(last).skip(first) {
        let band_lo = origin + j as f32 / LCD_SUBPIXELS as f32;
        let band_hi = origin + (j + 1) as f32 / LCD_SUBPIXELS as f32;
        let overlap = (x_end.min(band_hi) - x_start.max(band_lo)).max(0.0);
        if overlap > 0.0 {
            *cell += overlap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Polyline {
        Polyline {
            points: vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
            closed: true,
        }
    }

    #[test]
    fn a_pixel_aligned_square_is_fully_covered_inside() {
        let mask = rasterize(
            &[square(2.0, 2.0, 6.0, 6.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!((mask.coverage_at(3, 3) - 1.0).abs() < 1e-4);
        assert_eq!(mask.coverage_at(0, 0), 0.0);
        assert_eq!(mask.coverage_at(9, 9), 0.0);
    }

    #[test]
    fn a_half_covered_edge_pixel_is_half_covered() {
        // Right edge at x = 5.5: pixel column 5 is half in, half out.
        let mask = rasterize(
            &[square(2.0, 2.0, 5.5, 6.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!((mask.coverage_at(5, 3) - 0.5).abs() < 0.05);
        assert!((mask.coverage_at(4, 3) - 1.0).abs() < 1e-4);
        assert!(mask.coverage_at(6, 3) < 1e-4);
    }

    #[test]
    fn nonzero_winding_fills_overlap_of_same_direction_holes() {
        // Two same-wound squares overlapping: nonzero rule fills the union,
        // not xor.
        let mask = rasterize(
            &[square(0.0, 0.0, 6.0, 6.0), square(3.0, 3.0, 9.0, 9.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!(
            (mask.coverage_at(4, 4) - 1.0).abs() < 1e-4,
            "overlap should stay filled"
        );
        assert!((mask.coverage_at(1, 1) - 1.0).abs() < 1e-4);
        assert!((mask.coverage_at(7, 7) - 1.0).abs() < 1e-4);
    }

    /// The fast path's whole licence to exist: for every rectangle it claims,
    /// it must produce the bytes the scanline pass would have.
    ///
    /// Swept over sub-pixel offsets in both axes and over sizes from "narrower
    /// than a pixel" up, because that is where the two could differ — the
    /// interior of a large aligned rectangle is `1.0` under any implementation,
    /// and proves nothing.
    #[test]
    fn the_rect_fast_path_matches_the_scanline_pass() {
        let clip = Rect::new(0.0, 0.0, 24.0, 24.0);
        let steps = [0.0, 0.1, 0.25, 0.5, 0.6, 0.75, 0.9];
        for &ox in &steps {
            for &oy in &steps {
                for &w in &[0.3, 1.0, 2.5, 7.25, 19.0] {
                    for &h in &[0.3, 1.0, 2.5, 7.25, 19.0] {
                        let (x0, y0) = (2.0 + ox, 2.0 + oy);
                        let poly = square(x0, y0, x0 + w, y0 + h);
                        let fast = rasterize(std::slice::from_ref(&poly), clip);
                        // The same rectangle, described so `axis_aligned_rect`
                        // declines it: an extra collinear point makes it a
                        // five-point contour, which is the general path's
                        // problem and not the fast path's.
                        let mut general_points = poly.points.clone();
                        let mid = (
                            (general_points[0].0 + general_points[1].0) / 2.0,
                            general_points[0].1,
                        );
                        general_points.insert(1, mid);
                        let general = rasterize(
                            &[Polyline {
                                points: general_points,
                                closed: true,
                            }],
                            clip,
                        );

                        assert_eq!(
                            (fast.x0, fast.y0, fast.width, fast.height),
                            (general.x0, general.y0, general.width, general.height),
                            "bounds differ at {x0},{y0} {w}x{h}"
                        );
                        for (i, (f, g)) in fast.data.iter().zip(general.data.iter()).enumerate() {
                            assert!(
                                (f - g).abs() < 1e-6,
                                "coverage differs at index {i} for {x0},{y0} {w}x{h}: \
                                 fast={f} general={g}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A rectangle the fast path must decline, so it does not quietly claim a
    /// shape it would get wrong.
    #[test]
    fn a_rotated_rectangle_is_not_taken_by_the_fast_path() {
        let rotated = Polyline {
            points: vec![(4.0, 2.0), (8.0, 6.0), (4.0, 10.0), (0.0, 6.0)],
            closed: true,
        };
        assert!(axis_aligned_rect(std::slice::from_ref(&rotated)).is_none());
        // And still rasterises: a diamond's centre is covered.
        let mask = rasterize(&[rotated], Rect::new(0.0, 0.0, 12.0, 12.0));
        assert!((mask.coverage_at(4, 6) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_hole_cut_by_opposite_winding_is_empty() {
        let outer = square(0.0, 0.0, 10.0, 10.0);
        let mut inner = square(3.0, 3.0, 7.0, 7.0);
        inner.points.reverse(); // opposite winding: nonzero rule cancels
        let mask = rasterize(&[outer, inner], Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(mask.coverage_at(5, 5) < 1e-4, "hole should not be filled");
        assert!((mask.coverage_at(1, 1) - 1.0).abs() < 1e-4);
    }

    // ── LCD subpixel coverage ────────────────────────────────────────────

    /// A five-point contour: the same rectangle `square` draws, with a
    /// collinear point inserted so [`axis_aligned_rect`] declines it and the
    /// *general* scanline pass runs — the path `rasterize_lcd` always takes.
    fn general_rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Polyline {
        let mut points = vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        points.insert(1, ((x0 + x1) / 2.0, y0));
        Polyline {
            points,
            closed: true,
        }
    }

    /// The identity the LCD mask exists under: its three channels average to
    /// the gray coverage of the same shape, per pixel, for any shape and any
    /// sub-pixel placement. Swept over horizontal offsets in thirds of a
    /// pixel — where the channels disagree most — and over a rotated diamond,
    /// because a shape with slanted edges is where "sum of three bands ==
    /// whole-pixel overlap" is easiest to get subtly wrong.
    #[test]
    fn lcd_channels_average_to_the_gray_coverage() {
        let clip = Rect::new(0.0, 0.0, 24.0, 24.0);
        let shapes: Vec<Polyline> = vec![
            general_rect(2.0, 2.0, 9.5, 8.25),
            general_rect(2.25, 3.0, 10.0, 9.0),
            general_rect(3.1, 2.4, 11.75, 7.0),
            Polyline {
                points: vec![(12.0, 2.0), (18.0, 8.0), (12.0, 14.0), (6.0, 8.0)],
                closed: true,
            },
        ];
        for (i, poly) in shapes.iter().enumerate() {
            let gray = rasterize(std::slice::from_ref(poly), clip);
            let lcd = rasterize_lcd(std::slice::from_ref(poly), clip);
            assert_eq!(
                (gray.x0, gray.y0, gray.width, gray.height),
                (lcd.x0, lcd.y0, lcd.width, lcd.height),
                "bounds differ for shape {i}"
            );
            for y in 0..gray.height {
                for x in 0..gray.width {
                    let g = gray.coverage_at(gray.x0 + x as i32, gray.y0 + y as i32);
                    let [r, gr, b] = lcd.coverage_at(lcd.x0 + x as i32, lcd.y0 + y as i32);
                    let avg = (r + gr + b) / 3.0;
                    assert!(
                        (avg - g).abs() < 1e-5,
                        "shape {i} pixel ({x},{y}): lcd avg {avg} != gray {g} \
                         (channels {r}/{gr}/{b})"
                    );
                }
            }
        }
    }

    /// The whole point of the mode, in one assertion: a vertical edge at the
    /// pixel centre reads as full R, half G, no B — three different numbers
    /// where gray would have to say "two-thirds of a pixel" for all three
    /// sub-pixels at once.
    #[test]
    fn lcd_resolves_a_centred_vertical_edge_into_three_coverages() {
        // Ink covers [2.0, 2.5): pixel 2's R band [2.0, 2.333) is fully
        // covered, G band [2.333, 2.667) half, B band none. Rows 3..5 are
        // interior vertically, so all four subsamples agree.
        let mask = rasterize_lcd(
            &[general_rect(2.0, 2.0, 2.5, 6.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let [r, g, b] = mask.coverage_at(2, 3);
        assert!((r - 1.0).abs() < 1e-4, "R band fully covered, got {r}");
        assert!((g - 0.5).abs() < 1e-4, "G band half covered, got {g}");
        assert!(b < 1e-4, "B band uncovered, got {b}");
        // And to its left, pixel 1 is entirely outside the shape.
        assert_eq!(mask.coverage_at(1, 3), [0.0, 0.0, 0.0]);
        // While the gray rasterizer must average the same edge into one value.
        let gray = rasterize(
            &[general_rect(2.0, 2.0, 2.5, 6.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let c = gray.coverage_at(2, 3);
        assert!((c - 0.5).abs() < 1e-4, "gray edge, got {c}");
    }

    /// A purely *vertical* fraction — a horizontal edge, no sub-column
    /// asymmetry — must leave the three channels equal, because LCD changes
    /// the horizontal axis only. Equal channels is also exactly the case
    /// where LCD compositing degenerates to gray compositing.
    ///
    /// "Equal" here means equal to float rounding, not bit-identical: the
    /// band boundaries are `j/3`, which is not exact in `f32`, so the three
    /// bands' overlaps can differ in the last bit or two. A *split* — what
    /// this test exists to exclude — is orders of magnitude larger (the
    /// centred-edge test above reads 1.0/0.5/0.0), so the tolerance is tight
    /// against one and enormous against the other.
    #[test]
    fn lcd_leaves_channels_equal_for_purely_vertical_fractions() {
        // Top edge at y = 2.5: row 2 is covered by subsamples 2.625 and 2.875
        // only — one half vertically, fully horizontally.
        let mask = rasterize_lcd(
            &[general_rect(2.0, 2.5, 6.0, 6.0)],
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        let [r, g, b] = mask.coverage_at(3, 2);
        assert!((r - 0.5).abs() < 1e-4, "R half, got {r}");
        assert!(
            (r - g).abs() < 1e-5,
            "vertical fraction must not split channels: {r} vs {g}"
        );
        assert!(
            (g - b).abs() < 1e-5,
            "vertical fraction must not split channels: {g} vs {b}"
        );
    }
}
