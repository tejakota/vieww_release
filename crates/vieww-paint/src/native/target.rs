//! The premultiplied working buffer every CPU draw operation writes into.
//!
//! One type serves the root frame buffer and every offscreen layer (spec
//! §7.1's layer pool, minus the pooling — see `docs/RENDERER-MIGRATION.md`
//! for that as tracked follow-up work): a `Target` is just premultiplied
//! RGBA32F pixels plus a size, and [`Target::composite_coverage`] is the one
//! place a shape's coverage mask, a paint colour and a [`BlendMode`] become a
//! pixel write — so a fill, a stroke, a glyph and a shadow patch all go
//! through the same compositing arithmetic [`super::color`] defines.

use vieww_foundation::BlendMode;

use super::color::{over, Premul};
use super::geometry::fill::{LcdMaskView, MaskView, LCD_SUBPIXELS};
use super::gradient::DeviceRamp;
use super::linear::{blend_with_pipeline, ColorPipeline};

#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<Premul>,
    /// Writes are confined to this rectangle, when one is set.
    ///
    /// # Why a write mask and not a smaller surface
    ///
    /// A damaged repaint has to restrict what reaches the screen to the
    /// damaged region — but it must **not** restrict what the drawing
    /// operations *think* they are drawing into. The two are different, and
    /// the difference is not subtle: a shadow is a silhouette blurred over
    /// its own patch, and a patch cut off at the edge of a damage region
    /// blurs to different values along that edge than the same patch cut off
    /// at the edge of the window. Same for a blurred layer, whose kernel
    /// reaches across the boundary by design.
    ///
    /// Shrinking the surface therefore produces a repaint that is *nearly*
    /// right — wrong only in a band a few pixels wide along each region edge,
    /// which is precisely the fault nobody sees in review and everybody sees
    /// as "the shadows look slightly wrong after you click something".
    ///
    /// So effects are computed at their true extent, against the real window,
    /// and this mask decides which of the resulting pixels are allowed to
    /// land. Found by `examples/fixtures`' interaction fixture, which
    /// compares a retained repaint against a full one on a real screen every
    /// time it runs.
    write_mask: Option<(i32, i32, i32, i32)>,
    /// The bounding box of every pixel written since this buffer was last
    /// cleared, as `(x0, y0, x1, y1)` half-open in this buffer's own
    /// coordinates. `None` means nothing has been written at all.
    ///
    /// # Why a layer needs to know what it touched
    ///
    /// A layer's buffer is sized to the *bounds it declared*, which is a
    /// bounding box of what might be drawn — a `Stack` with one small badge in
    /// a corner declares the whole stack. `PopLayer` then composited every
    /// pixel of that buffer down into the parent, reading and testing millions
    /// of fully transparent pixels to find the badge. Nested layers pay it
    /// again per level: `fixtures`' `00-layers-nested` (24 deep) spent 39 ms of
    /// a 16.7 ms budget almost entirely here.
    ///
    /// Tracking the union of what actually landed costs four comparisons per
    /// *draw call* — not per pixel — and turns that composite into a walk of
    /// the ink instead of a walk of the buffer.
    ///
    /// It is a bounding box, not a region: it can over-report (an L-shape
    /// reports its enclosing rectangle) and that is fine, because
    /// over-reporting only costs time. It must never *under*-report, which is
    /// why every method that writes a pixel updates it, and why the two
    /// filters — which read across the whole buffer — say so explicitly rather
    /// than relying on what was there before them.
    ink: Option<(i32, i32, i32, i32)>,
}

impl Target {
    #[must_use]
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![Premul::TRANSPARENT; (width as usize) * (height as usize)],
            write_mask: None,
            ink: None,
        }
    }

    /// Confine every subsequent write to `region`, or lift the restriction.
    ///
    /// See the field's own documentation for why this exists rather than a
    /// smaller surface.
    pub(crate) fn set_write_mask(&mut self, region: Option<(i32, i32, i32, i32)>) {
        self.write_mask = region;
    }

    /// The horizontal and vertical ranges a write may land in.
    fn writable(&self) -> (i32, i32, i32, i32) {
        match self.write_mask {
            Some((x0, y0, x1, y1)) => (
                x0.max(0),
                y0.max(0),
                x1.min(self.width as i32),
                y1.min(self.height as i32),
            ),
            None => (0, 0, self.width as i32, self.height as i32),
        }
    }

    /// Reset every pixel to fully transparent, keeping the buffer's own
    /// allocation — what [`TargetPool`](super::pool::TargetPool) calls on a
    /// buffer it is about to hand back out, so reuse costs a fill, not an
    /// allocation.
    pub(crate) fn clear(&mut self) {
        // **Only the part that was written.** A pooled buffer handed back out
        // is usually the same buffer that was just used, and what needs
        // resetting is what the last user actually drew — not the megabytes
        // around it that were already transparent. `ink` is exactly that
        // answer, and it is already being maintained for the composite path.
        if let Some((x0, y0, x1, y1)) = self.ink.take() {
            let width = self.width as usize;
            for y in y0.max(0)..y1.min(self.height as i32) {
                let row = y as usize * width;
                let lo = row + x0.max(0) as usize;
                let hi = row + (x1.min(self.width as i32)).max(0) as usize;
                if hi > lo {
                    self.pixels[lo..hi].fill(Premul::TRANSPARENT);
                }
            }
        }
    }

    /// Widen [`Self::ink`] to include the half-open box `(x0, y0, x1, y1)`.
    fn mark_ink(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        self.ink = Some(match self.ink {
            Some((ax0, ay0, ax1, ay1)) => (ax0.min(x0), ay0.min(y0), ax1.max(x1), ay1.max(y1)),
            None => (x0, y0, x1, y1),
        });
    }

    /// Declare that every pixel of this buffer now counts as written — what
    /// the two layer filters do, since both read across the whole buffer and
    /// can turn a transparent pixel into a visible one.
    pub(crate) fn mark_all_ink(&mut self) {
        self.ink = Some((0, 0, self.width as i32, self.height as i32));
    }

    /// Grow the recorded ink outward by `margin` pixels, clamped to the
    /// buffer — what a blur does to the extent of what is visible in it.
    pub(crate) fn grow_ink(&mut self, margin: i32) {
        if let Some((x0, y0, x1, y1)) = self.ink {
            self.ink = Some((
                (x0 - margin).max(0),
                (y0 - margin).max(0),
                (x1 + margin).min(self.width as i32),
                (y1 + margin).min(self.height as i32),
            ));
        }
    }

    /// The box every written pixel of this buffer falls inside, if any.
    #[must_use]
    pub(crate) fn ink(&self) -> Option<(i32, i32, i32, i32)> {
        self.ink
    }

    /// A buffer every pixel of which is `color`.
    ///
    /// Test-only: `native/instancing.rs`'s tests composite an instanced draw
    /// onto a known ground and read the result back. Nothing in the renderer
    /// needs it — the root buffer is filled through `reset`, and a layer's
    /// buffer starts transparent — which is why it is gated rather than
    /// carrying an `#[allow(dead_code)]` that would also hide a real one.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn filled(width: u32, height: u32, color: Premul) -> Self {
        Self {
            width,
            height,
            pixels: vec![color; (width as usize) * (height as usize)],
            write_mask: None,
            ink: Some((0, 0, width as i32, height as i32)),
        }
    }

    /// This buffer's own pixels, straight-alpha RGBA8, top row first.
    ///
    /// Test-only, for the reason [`Self::filled`] gives: the renderer's own
    /// output path is `write_rgba8` into a buffer it keeps between frames, so
    /// nothing outside a test wants a freshly allocated `Vec`.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.pixels.len() * 4];
        self.write_rgba8(&mut out);
        out
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }

    #[must_use]
    pub(crate) fn get(&self, x: i32, y: i32) -> Premul {
        self.index(x, y)
            .map_or(Premul::TRANSPARENT, |i| self.pixels[i])
    }

    pub(crate) fn set(&mut self, x: i32, y: i32, color: Premul) {
        if let Some(i) = self.index(x, y) {
            self.pixels[i] = color;
            self.mark_ink(x, y, x + 1, y + 1);
        }
    }

    /// Composite a shape into this buffer: `color_at(x, y)` gives the
    /// shape's own (unmasked) colour at **absolute device** coordinates
    /// (matching `coverage`, which every rasterizer entry point in this
    /// crate produces in device space regardless of which layer it is
    /// destined for), `coverage` scales it, and `mode` decides how it meets
    /// what's already there.
    ///
    /// `origin` is this buffer's own device-space top-left — `(0, 0)` for
    /// the root frame, a `PushLayer`'s device bounds for anything nested —
    /// and is what maps `coverage`'s absolute coordinates onto this
    /// buffer's own 0-based pixel array. Forgetting it is exactly the bug
    /// class this parameter exists to make impossible to forget: every
    /// caller has to say which space `coverage` was measured in relative to.
    pub(crate) fn composite_coverage(
        &mut self,
        coverage: MaskView<'_>,
        origin: (i32, i32),
        mode: BlendMode,
        pipeline: ColorPipeline,
        mut color_at: impl FnMut(i32, i32) -> Premul,
    ) {
        if coverage.is_empty() {
            return;
        }

        // # Why the bounds arithmetic is hoisted out of the loop
        //
        // This used to call `self.get(tx, ty)` and `self.set(tx, ty, …)` per
        // pixel, each of which recomputed `y * width + x` and re-checked the
        // pixel against all four edges of the buffer through an `Option`. Both
        // answers are the same for an entire row and knowable before the loop
        // starts: the mask's rectangle intersected with this buffer's own is a
        // range of rows and, within each, a range of columns. Clipping once
        // and then walking two slices removes four comparisons, a
        // multiplication and two `Option`s from every pixel of every fill,
        // stroke, glyph and shadow in the frame — which, at roughly a million
        // pixels a frame before overdraw, is not a micro-optimisation.
        //
        // It also removes the silent-clamp behaviour that `set` had: a write
        // outside the buffer was discarded pixel by pixel. Now it is excluded
        // by the ranges, which is the same result computed once.
        // The writable window is the buffer, narrowed to the write mask when
        // a damaged repaint has set one.
        let (wx0, wy0, wx1, wy1) = self.writable();
        let dy_start = (wy0 - (coverage.y0 - origin.1)).max(0);
        let dy_end = (wy1 - (coverage.y0 - origin.1)).min(coverage.height as i32);
        let dx_start = (wx0 - (coverage.x0 - origin.0)).max(0);
        let dx_end = (wx1 - (coverage.x0 - origin.0)).min(coverage.width as i32);
        if dy_start >= dy_end || dx_start >= dx_end {
            return;
        }

        let normal = mode.is_normal() && matches!(pipeline, ColorPipeline::GammaSpace);
        // The window this call may write into, recorded before the loop: it is
        // a bound on the ink, which is all `ink` promises to be.
        self.mark_ink(
            coverage.x0 - origin.0 + dx_start,
            coverage.y0 - origin.1 + dy_start,
            coverage.x0 - origin.0 + dx_end,
            coverage.y0 - origin.1 + dy_end,
        );

        for dy in dy_start..dy_end {
            let y = coverage.y0 + dy;
            let ty = y - origin.1;
            let mask_row = coverage.row(dy as u32);
            let pixel_row = ty as usize * self.width as usize;

            for dx in dx_start..dx_end {
                let c = mask_row[dx as usize];
                if c <= 0.0 {
                    continue;
                }
                let x = coverage.x0 + dx;
                let src = color_at(x, y).scaled(c);
                if src.a <= 0.0 && mode.is_normal() {
                    continue;
                }
                let index = pixel_row + (x - origin.0) as usize;
                // **The opaque fast path.** A fully opaque source under a
                // normal blend in gamma space *replaces* the destination —
                // there is nothing to read, mix and write back. That is the
                // single most common pixel in any interface: the interior of
                // every solid fill, every panel, every icon. Taking it before
                // the general blend skips a load and a 28-way match per pixel.
                // Exactly 1.0, not `>=`: `over` with an alpha above one
                // produces `src + dst * (1 - a)` — a *negative* destination
                // term — so a pixel whose accumulated alpha overshot is not
                // the same as a straight replacement. At exactly one it is.
                if normal && src.a == 1.0 {
                    self.pixels[index] = src;
                    continue;
                }
                let dst = self.pixels[index];
                // `normal` is `mode.is_normal() && GammaSpace`, decided once
                // above — so where it holds, the twenty-eight-way match in
                // `blend` and the pipeline match in `blend_with_pipeline` have
                // only one possible answer and are pure dispatch. See
                // `color::over`.
                self.pixels[index] = if normal {
                    over(src, dst)
                } else {
                    blend_with_pipeline(pipeline, mode, src, dst)
                };
            }
        }
    }

    /// Per-channel — LCD subpixel — glyph compositing, for an
    /// [`LcdMaskView`]: the counterpart of [`composite_coverage`] for text
    /// rasterised at three times the horizontal resolution.
    ///
    /// # The contract: an opaque destination
    ///
    /// Each channel blends with its **own** coverage: a stem edge that covers
    /// the R stripe but not the B stripe pushes red in and keeps blue out.
    /// That is only meaningful over a destination whose alpha is already 1.0 —
    /// over anything translucent, the per-channel result carries fringes that
    /// a later blend would expose rather than resolve, which is why every
    /// engine that offers LCD text restricts it to opaque surfaces and why
    /// `reference.rs` only routes root-surface glyphs here (layer-surface
    /// glyphs fall back to gray coverage, keyed separately in the raster
    /// cache). The method takes the destination as it finds it and writes
    /// `a: dst.a` — the honest statement of "this blend does not change
    /// coverage" — which on the opaque destinations it is called on is 1.0
    /// before and after.
    ///
    /// # The degenerate case is the gray case, bit for bit
    ///
    /// A mask whose three channels are equal — a purely vertical edge, or the
    /// interior of any glyph — is a gray-coverage mask in disguise, and the
    /// equal-channel branch below runs the gray path's own
    /// `over(color.scaled(k), dst)` rather than the per-channel formula, so
    /// the two paths cannot drift apart on the pixels where they agree.
    pub(crate) fn composite_lcd(
        &mut self,
        coverage: LcdMaskView<'_>,
        origin: (i32, i32),
        color: vieww_foundation::Color,
        mut clip_at: impl FnMut(i32, i32) -> f32,
    ) {
        if coverage.is_empty() {
            return;
        }
        // The same hoisted window arithmetic as `composite_coverage`: the
        // mask's rectangle against this buffer's writable one, once, instead
        // of four comparisons and two `Option`s per pixel.
        let (wx0, wy0, wx1, wy1) = self.writable();
        let dy_start = (wy0 - (coverage.y0 - origin.1)).max(0);
        let dy_end = (wy1 - (coverage.y0 - origin.1)).min(coverage.height as i32);
        let dx_start = (wx0 - (coverage.x0 - origin.0)).max(0);
        let dx_end = (wx1 - (coverage.x0 - origin.0)).min(coverage.width as i32);
        if dy_start >= dy_end || dx_start >= dx_end {
            return;
        }

        self.mark_ink(
            coverage.x0 - origin.0 + dx_start,
            coverage.y0 - origin.1 + dy_start,
            coverage.x0 - origin.0 + dx_end,
            coverage.y0 - origin.1 + dy_end,
        );

        // The run's colour, premultiplied at full coverage. `scaled(k)` below
        // is the same helper the gray path scales its source with, so the
        // equal-channel branch and the gray path are one implementation.
        let c = Premul::from_straight(color);

        for dy in dy_start..dy_end {
            let y = coverage.y0 + dy;
            let ty = y - origin.1;
            let mask_row = coverage.row(dy as u32);
            let pixel_row = ty as usize * self.width as usize;

            for dx in dx_start..dx_end {
                let base = dx as usize * LCD_SUBPIXELS;
                let kr = mask_row[base];
                let kg = mask_row[base + 1];
                let kb = mask_row[base + 2];
                let x = coverage.x0 + dx;
                // A clip is a mask over the whole pixel, not over individual
                // sub-columns, so it multiplies all three channels equally.
                let clip = clip_at(x, y);
                if clip <= 0.0 {
                    continue;
                }
                let (kr, kg, kb) = if clip >= 1.0 {
                    (kr, kg, kb)
                } else {
                    (kr * clip, kg * clip, kb * clip)
                };
                if kr <= 0.0 && kg <= 0.0 && kb <= 0.0 {
                    continue;
                }
                let index = pixel_row + (x - origin.0) as usize;
                // The opaque interior fast path, same licence as
                // `composite_coverage`'s: full coverage on every channel, an
                // opaque colour — there is nothing to read and blend.
                if kr == 1.0 && kg == 1.0 && kb == 1.0 && c.a == 1.0 {
                    self.pixels[index] = c;
                    continue;
                }
                let dst = self.pixels[index];
                if kr == kg && kg == kb {
                    // Equal channels: gray coverage in disguise. The gray
                    // path's own arithmetic, so the bytes agree with it.
                    self.pixels[index] = over(c.scaled(kr), dst);
                    continue;
                }
                // Genuinely split channels: each colour channel blends with
                // its own effective alpha `e_ch = a * k_ch` — the source term
                // `c_ch * k_ch` is exactly what `scaled(k_ch)` would give
                // that channel, so this is the gray formula evaluated per
                // channel rather than a new definition of blending.
                let e_r = c.a * kr;
                let e_g = c.a * kg;
                let e_b = c.a * kb;
                self.pixels[index] = Premul {
                    r: c.r * kr + dst.r * (1.0 - e_r),
                    g: c.g * kg + dst.g * (1.0 - e_g),
                    b: c.b * kb + dst.b * (1.0 - e_b),
                    a: dst.a,
                };
            }
        }
    }

    /// [`composite_coverage`](Self::composite_coverage) for one flat colour
    /// with no clip mask over it.
    ///
    /// # Why this is a separate method rather than a branch inside the loop
    ///
    /// The general path takes a `FnMut(i32, i32) -> Premul` and calls it once
    /// per pixel. For a flat fill that closure's whole body is a clip lookup
    /// that always answers `1.0` and a match on a `None` gradient — work that
    /// is identical for every pixel of the shape and that the compiler cannot
    /// hoist, because it cannot see that the closure's answer does not vary.
    ///
    /// Splitting it out lets each case be what it is. Here the source colour is
    /// a constant, so the inner loop is a coverage load, one `scaled` and a
    /// blend — and, in the case that matters most, not even that: an opaque
    /// colour at full coverage is a store of a value already in a register.
    /// That is the interior of every solid panel, button face and icon path in
    /// the window, and it was previously paying an indirect call per pixel for
    /// the privilege.
    ///
    /// The result is bit-identical to the general path: the same arithmetic on
    /// the same numbers, with `mode` and `pipeline` reaching
    /// [`blend_with_pipeline`] unchanged for every pixel that is not the
    /// fully-covered opaque case — which the general path already
    /// special-cases, the same way.
    pub(crate) fn composite_flat(
        &mut self,
        coverage: MaskView<'_>,
        origin: (i32, i32),
        mode: BlendMode,
        pipeline: ColorPipeline,
        color: Premul,
    ) {
        if coverage.is_empty() || (color.a <= 0.0 && mode.is_normal()) {
            return;
        }
        let (wx0, wy0, wx1, wy1) = self.writable();
        let dy_start = (wy0 - (coverage.y0 - origin.1)).max(0);
        let dy_end = (wy1 - (coverage.y0 - origin.1)).min(coverage.height as i32);
        let dx_start = (wx0 - (coverage.x0 - origin.0)).max(0);
        let dx_end = (wx1 - (coverage.x0 - origin.0)).min(coverage.width as i32);
        if dy_start >= dy_end || dx_start >= dx_end {
            return;
        }

        let normal = mode.is_normal() && matches!(pipeline, ColorPipeline::GammaSpace);
        // An opaque colour under a normal blend replaces whatever it lands on,
        // wherever coverage is full — which is every pixel of a shape but its
        // antialiased outline. Hoisted, so the test per pixel is one bool.
        let replaces = normal && color.a == 1.0;
        self.mark_ink(
            coverage.x0 - origin.0 + dx_start,
            coverage.y0 - origin.1 + dy_start,
            coverage.x0 - origin.0 + dx_end,
            coverage.y0 - origin.1 + dy_end,
        );

        for dy in dy_start..dy_end {
            let ty = coverage.y0 + dy - origin.1;
            // The left edge of the row's visible span, computed in `i32`
            // **before** the cast. `coverage.x0 - origin.0` can legitimately be
            // negative — a glyph whose bounding box starts a pixel left of its
            // layer's declared bounds is the common case — and casting that
            // negative straight to `usize` wraps it to near 2^64 and overflows
            // the add below. The clamping above guarantees
            // `coverage.x0 - origin.0 + dx_start >= 0`: `dx_start` is either 0
            // (the offset itself was >= `wx0 >= 0`) or exactly
            // `wx0 - (coverage.x0 - origin.0)`, which cancels the offset back
            // up to `wx0`. The general `composite_coverage` path already does
            // its add-then-cast in this order, per pixel; this fast path must
            // match it with `dx_start`.
            let x_left = coverage.x0 - origin.0 + dx_start;
            debug_assert!(
                ty >= 0 && x_left >= 0,
                "clamped span must land inside the buffer"
            );
            let base = ty as usize * self.width as usize + x_left as usize;
            let span = (dx_end - dx_start) as usize;

            let mask = &coverage.row(dy as u32)[dx_start as usize..dx_end as usize];
            let row = &mut self.pixels[base..base + span];
            for (px, &c) in row.iter_mut().zip(mask.iter()) {
                if c <= 0.0 {
                    continue;
                }
                if replaces && c >= 1.0 {
                    *px = color;
                    continue;
                }
                // No `if c >= 1.0 { color }` fast path to skip `scaled`'s four
                // multiplies-by-one on a translucent full-coverage pixel: it
                // was tried, and it cost more than it saved. See
                // `composite_layer` for the measurement of the pair.
                let src = color.scaled(c);
                if normal && src.a == 1.0 {
                    *px = src;
                    continue;
                }
                *px = if normal {
                    over(src, *px)
                } else {
                    blend_with_pipeline(pipeline, mode, src, *px)
                };
            }
        }
    }

    /// [`composite_coverage`](Self::composite_coverage) for a gradient with no
    /// clip mask over it — the other half of the split [`composite_flat`] made.
    ///
    /// The gradient is evaluated through a [`DeviceRamp`], which has the
    /// shape's inverse transform and bounds already folded into it, so the
    /// row's constants are computed once at the top of each row and the inner
    /// loop is a multiply, an add and a ramp lookup. See `native/gradient.rs`
    /// for what that replaced.
    pub(crate) fn composite_gradient(
        &mut self,
        coverage: MaskView<'_>,
        origin: (i32, i32),
        mode: BlendMode,
        pipeline: ColorPipeline,
        ramp: &DeviceRamp,
    ) {
        if coverage.is_empty() {
            return;
        }
        let (wx0, wy0, wx1, wy1) = self.writable();
        let dy_start = (wy0 - (coverage.y0 - origin.1)).max(0);
        let dy_end = (wy1 - (coverage.y0 - origin.1)).min(coverage.height as i32);
        let dx_start = (wx0 - (coverage.x0 - origin.0)).max(0);
        let dx_end = (wx1 - (coverage.x0 - origin.0)).min(coverage.width as i32);
        if dy_start >= dy_end || dx_start >= dx_end {
            return;
        }
        let normal = mode.is_normal() && matches!(pipeline, ColorPipeline::GammaSpace);
        self.mark_ink(
            coverage.x0 - origin.0 + dx_start,
            coverage.y0 - origin.1 + dy_start,
            coverage.x0 - origin.0 + dx_end,
            coverage.y0 - origin.1 + dy_end,
        );

        for dy in dy_start..dy_end {
            let y = coverage.y0 + dy;
            let ty = y - origin.1;
            let mask_row = coverage.row(dy as u32);
            // Add-then-cast in `i32`, for the reason `composite_flat`'s own
            // comment gives: `coverage.x0 - origin.0` can be negative when the
            // shape starts left of this buffer's origin, and a negative cast
            // straight to `usize` wraps and overflows. `dx_start`'s clamping
            // guarantees the sum lands at or right of `wx0`.
            let x_left = coverage.x0 - origin.0 + dx_start;
            debug_assert!(
                ty >= 0 && x_left >= 0,
                "clamped span must land inside the buffer"
            );
            let base = ty as usize * self.width as usize + x_left as usize;
            // Once per row, not once per pixel.
            let row = ramp.row(y);

            for dx in dx_start..dx_end {
                let c = mask_row[dx as usize];
                if c <= 0.0 {
                    continue;
                }
                let src = ramp.at(&row, coverage.x0 + dx, y).scaled(c);
                if src.a <= 0.0 && normal {
                    continue;
                }
                let index = base + (dx - dx_start) as usize;
                if normal && src.a == 1.0 {
                    self.pixels[index] = src;
                    continue;
                }
                let dst = self.pixels[index];
                // `normal` is `mode.is_normal() && GammaSpace`, decided once
                // above — so where it holds, the twenty-eight-way match in
                // `blend` and the pipeline match in `blend_with_pipeline` have
                // only one possible answer and are pure dispatch. See
                // `color::over`.
                self.pixels[index] = if normal {
                    over(src, dst)
                } else {
                    blend_with_pipeline(pipeline, mode, src, dst)
                };
            }
        }
    }

    /// Seed this buffer with a copy of `source`'s pixels underneath this
    /// buffer's own device-space region — real backdrop sampling for
    /// [`vieww_foundation::ImageFilter::backdrop`] (see `native/reference.rs`'s
    /// `Command::PushLayer` handling for where this is called and why it
    /// happens before any of this layer's own content paints).
    ///
    /// `source_origin` and `dest_origin` are both device-space top-lefts,
    /// the same convention every other method on this type uses: a pixel at
    /// absolute position `(x, y)` is read from `source` at
    /// `(x - source_origin.0, y - source_origin.1)` (out of bounds reads as
    /// transparent, via [`Self::get`]) and written to `self` at
    /// `(x - dest_origin.0, y - dest_origin.1)`.
    pub(crate) fn snapshot_from(
        &mut self,
        source: &Target,
        source_origin: (i32, i32),
        dest_origin: (i32, i32),
    ) {
        // Every pixel of this buffer is written, including the transparent
        // ones — a snapshot is a copy, not a draw.
        self.mark_all_ink();
        // Row at a time. This used to call `get`/`set` per pixel — two bounds
        // checks, two index multiplications and an `Option` each — to copy a
        // rectangle whose overlap with the source is one range of rows and one
        // range of columns, both knowable before the loop. A backdrop-filtered
        // layer snapshots its whole area on every frame it is drawn, so this is
        // a full-window copy per glass panel per frame.
        //
        // Rows outside the overlap stay as they are, which is what the
        // per-pixel version did too: `get` answered transparent off the end of
        // the source, and the buffer arrived from the pool already transparent.
        let (sw, sh) = (source.width as i32, source.height as i32);
        let dy0 = (source_origin.1 - dest_origin.1).max(0);
        let dy1 = (source_origin.1 + sh - dest_origin.1).min(self.height as i32);
        let dx0 = (source_origin.0 - dest_origin.0).max(0);
        let dx1 = (source_origin.0 + sw - dest_origin.0).min(self.width as i32);
        if dy1 <= dy0 || dx1 <= dx0 {
            return;
        }
        for dy in dy0..dy1 {
            let sy = dest_origin.1 + dy - source_origin.1;
            let src_row = sy as usize * source.width as usize;
            let dst_row = dy as usize * self.width as usize;
            let sx = dest_origin.0 + dx0 - source_origin.0;
            let count = (dx1 - dx0) as usize;
            self.pixels[dst_row + dx0 as usize..dst_row + dx0 as usize + count].copy_from_slice(
                &source.pixels[src_row + sx as usize..src_row + sx as usize + count],
            );
        }
    }

    /// Composite another whole buffer (a resolved layer) over this one at
    /// `(ox, oy)`, through `mode` and a flat `alpha` (spec §7.1's group
    /// opacity, applied at resolve — "opacity animatable without re-recording
    /// children").
    pub(crate) fn composite_layer(
        &mut self,
        other: &Target,
        ox: i32,
        oy: i32,
        mode: BlendMode,
        alpha: f32,
        pipeline: ColorPipeline,
    ) {
        // Row ranges computed once, for the reason `composite_coverage` gives:
        // a layer resolve reads and writes every pixel of the layer, and the
        // per-pixel `get`/`set` this used to call re-derived the same bounds
        // answer for each of them. Nesting makes it worse than a single fill,
        // because every level of depth composites the whole thing again — a
        // 24-deep stack of layers ran this loop 24 times over.
        let (wx0, wy0, wx1, wy1) = self.writable();
        let mut y_start = (wy0 - oy).max(0);
        let mut y_end = (wy1 - oy).min(other.height as i32);
        let mut x_start = (wx0 - ox).max(0);
        let mut x_end = (wx1 - ox).min(other.width as i32);
        // **Only where the layer actually has ink.**
        //
        // A layer's buffer is as big as the bounds it declared, and the pixels
        // outside what it drew are fully transparent — compositing them is a
        // read, an alpha test and a branch each, for a guaranteed no-op. A
        // 24-deep nest pays it once per level. `Target::ink`'s own doc has the
        // measurement; `None` means the layer drew nothing at all and there is
        // nothing to composite.
        match other.ink() {
            Some((ix0, iy0, ix1, iy1)) => {
                x_start = x_start.max(ix0);
                y_start = y_start.max(iy0);
                x_end = x_end.min(ix1);
                y_end = y_end.min(iy1);
            }
            None => return,
        }
        if y_start >= y_end || x_start >= x_end {
            return;
        }
        self.mark_ink(x_start + ox, y_start + oy, x_end + ox, y_end + oy);

        let normal = mode.is_normal() && matches!(pipeline, ColorPipeline::GammaSpace);
        let opaque_copy = normal && alpha == 1.0;

        for y in y_start..y_end {
            let src_row = y as usize * other.width as usize;
            let dst_row = (y + oy) as usize * self.width as usize;
            // **No opaque-row `copy_from_slice` fast path here, and it was
            // measured rather than assumed.**
            //
            // `over` with `src.a == 1` is `src`, so a fully opaque run of a
            // layer is a copy, and scanning the row's alphas first to turn it
            // into one `memcpy` looks like an obvious win — panels and popovers
            // are mostly opaque. Together with the matching `c >= 1.0` skip in
            // `composite_flat`, it took `examples/fixtures`' `23-editor-glass`
            // from **1,203,538,312 instructions to 1,223,637,694** — 1.7%
            // *worse*. The scan is a second pass over the row, the per-pixel
            // blend it replaces is already only a few instructions, and the
            // branch it adds is one the compiler had already arranged for.
            //
            // Recorded because it is the third plausible-sounding change in
            // this file to measure worse than the code it replaced, and the
            // fourth to measure the opposite of a single-sample estimate.
            for x in x_start..x_end {
                let src = other.pixels[src_row + x as usize];
                let src = if alpha == 1.0 { src } else { src.scaled(alpha) };
                if src.a <= 0.0 && mode.is_normal() {
                    continue;
                }
                let index = dst_row + (x + ox) as usize;
                if opaque_copy && src.a == 1.0 {
                    self.pixels[index] = src;
                    continue;
                }
                let dst = self.pixels[index];
                // `normal` is `mode.is_normal() && GammaSpace`, decided once
                // above — so where it holds, the twenty-eight-way match in
                // `blend` and the pipeline match in `blend_with_pipeline` have
                // only one possible answer and are pure dispatch. See
                // `color::over`.
                self.pixels[index] = if normal {
                    over(src, dst)
                } else {
                    blend_with_pipeline(pipeline, mode, src, dst)
                };
            }
        }
    }

    /// [`to_rgba8`](Self::to_rgba8) into a buffer the caller already owns.
    ///
    /// A frame's output is the same size as the last frame's, so a renderer
    /// presenting continuously can keep one buffer and refill it instead of
    /// allocating and freeing a megabyte or two every frame — which at
    /// 1366x679 is 3.7 MB of allocation per frame for nothing.
    ///
    /// # Panics
    ///
    /// If `out` is not exactly four bytes per pixel.
    pub(crate) fn write_rgba8(&self, out: &mut [u8]) {
        assert_eq!(out.len(), self.pixels.len() * 4);
        // `chunks_exact_mut(4)` and `copy_from_slice` rather than
        // `as_chunks_mut::<4>()` and an assignment. The typed-length version
        // looks like it must be cheaper — no runtime length, no panic branch —
        // and it was tried: **1,203,539,448 instructions against
        // 1,203,539,565**, a difference of 117 across a whole fixture render.
        // LLVM already turns this into the same four-byte store. Left as the
        // more familiar spelling.
        for (pixel, slot) in self.pixels.iter().zip(out.chunks_exact_mut(4)) {
            slot.copy_from_slice(&pixel.to_straight_u8());
        }
    }

    /// Resize if needed and refill with `color`, reusing this buffer's
    /// allocation whenever the size has not changed.
    ///
    /// The root frame buffer is the same size on frame two as on frame one
    /// essentially always — a window resize is the exception, not the rule —
    /// so allocating a fresh one per frame is pure waste. At 1366x679 that
    /// allocation is 14.8 MB of `Premul`, and measurably: an *empty* scene
    /// cost 11.9 ms a frame before this, which is 71% of a 60 Hz budget spent
    /// on a buffer nothing had drawn into yet.
    pub(crate) fn reset(&mut self, width: u32, height: u32, color: Premul) {
        let wanted = (width as usize) * (height as usize);
        if self.pixels.len() == wanted {
            self.pixels.fill(color);
        } else {
            self.pixels.clear();
            self.pixels.resize(wanted, color);
        }
        self.width = width;
        self.height = height;
        // A full reset fills every pixel with `color`, which for the root
        // frame buffer is the window's background — a real, visible colour, not
        // "nothing". Every pixel of it is therefore ink.
        self.ink = Some((0, 0, width as i32, height as i32));
    }

    /// Refill one rectangle with `color`, leaving the rest of the buffer as
    /// it is.
    ///
    /// A damaged repaint clears the region it is about to redraw and nothing
    /// else — that is what makes the buffer *retained*: the pixels outside
    /// the region are last frame's, and they are still correct precisely
    /// because nothing damaged them.
    ///
    /// Clearing is not optional inside the region, though. A damage region
    /// says what to redraw, not what to skip clearing: a shape that shrank or
    /// moved must have its old pixels gone, and a partial-background repaint
    /// would leave them standing.
    pub(crate) fn reset_region(&mut self, region: (i32, i32, i32, i32), color: Premul) {
        let (x0, y0, x1, y1) = region;
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(self.width as i32);
        let y1 = y1.min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        for y in y0..y1 {
            let row = y as usize * self.width as usize;
            self.pixels[row + x0 as usize..row + x1 as usize].fill(color);
        }
        self.mark_ink(x0, y0, x1, y1);
    }

    /// [`write_rgba8`](Self::write_rgba8) for one rectangle only.
    ///
    /// The other half of a retained surface. Converting the whole buffer to
    /// RGBA8 costs the same whether one pixel changed or all of them — at
    /// 1440x900 that conversion alone is most of an 11 ms floor — so a
    /// damaged frame converts only the rows it actually repainted.
    ///
    /// # Panics
    ///
    /// If `out` is not exactly four bytes per pixel of this buffer.
    pub(crate) fn write_rgba8_region(&self, out: &mut [u8], region: (i32, i32, i32, i32)) {
        assert_eq!(out.len(), self.pixels.len() * 4);
        let (x0, y0, x1, y1) = region;
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(self.width as i32);
        let y1 = y1.min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let width = self.width as usize;
        for y in y0..y1 {
            let row = y as usize * width;
            let pixels = &self.pixels[row + x0 as usize..row + x1 as usize];
            let bytes = &mut out[(row + x0 as usize) * 4..(row + x1 as usize) * 4];
            for (pixel, slot) in pixels.iter().zip(bytes.chunks_exact_mut(4)) {
                slot.copy_from_slice(&pixel.to_straight_u8());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red() -> Premul {
        Premul::from_straight(vieww_foundation::Color::rgba(255, 0, 0, 255))
    }

    fn blue() -> Premul {
        Premul::from_straight(vieww_foundation::Color::rgba(0, 0, 255, 255))
    }

    /// A coverage rectangle that starts left of the buffer's origin — a glyph
    /// whose ink begins a pixel or two left of its layer's declared bounds,
    /// which is exactly what the launch animation produced at its device
    /// switch: `cov=(1096,172 20x67) origin=(1097,160)`.
    ///
    /// `composite_flat` used to hoist `(coverage.x0 - origin.0) as usize` out
    /// of the loop, so the negative offset wrapped to near 2^64 and the row
    /// base add overflowed. The regression this pins is that the draw clips
    /// the overhang instead of panicking, and lands the surviving columns at
    /// the right pixels — `coverage.x0 + dx` for the clamped `dx` range, the
    /// same arithmetic the general path performs per pixel.
    #[test]
    fn composite_flat_clips_a_coverage_that_starts_left_of_the_origin() {
        use super::super::geometry::fill::CoverageMask;

        let mut target = Target::new(10, 10);
        // An 8x4 mask whose device-space box starts at x = -3: only the 5
        // columns that overlap the buffer should land.
        let mask = CoverageMask {
            x0: -3,
            y0: 2,
            width: 8,
            height: 4,
            data: vec![1.0; 8 * 4],
        };
        target.composite_flat(
            mask.view(),
            (0, 0),
            vieww_foundation::BlendMode::Normal,
            ColorPipeline::GammaSpace,
            red(),
        );

        for y in 2..6 {
            for x in 0..5 {
                assert_eq!(target.get(x, y), red(), "visible column {x},{y} is painted");
            }
            assert_eq!(
                target.get(5, y),
                Premul::TRANSPARENT,
                "the column past the clipped span stays clear"
            );
        }
        assert_eq!(target.get(0, 1), Premul::TRANSPARENT, "row above the mask");
    }

    /// The nested-layer flavour of the same geometry: a coverage measured in
    /// device space against a target whose own origin is nonzero, with the
    /// overhang on the left — the exact `PushLayer` + glyph situation from
    /// the film. With origin (5, 5) and a mask box starting at x = 2, the
    /// first three columns fall before the buffer and must be dropped.
    #[test]
    fn composite_flat_clips_a_left_overhang_against_a_layer_origin() {
        use super::super::geometry::fill::CoverageMask;

        let mut target = Target::new(6, 6);
        let mask = CoverageMask {
            x0: 2,
            y0: 6,
            width: 8,
            height: 3,
            data: vec![1.0; 8 * 3],
        };
        target.composite_flat(
            mask.view(),
            (5, 5),
            vieww_foundation::BlendMode::Normal,
            ColorPipeline::GammaSpace,
            blue(),
        );

        // Device x 2..10 against origin 5: local 0..5 painted, the two columns
        // left of the origin dropped rather than wrapped around the address
        // space.
        for y in 1..4 {
            for x in 0..5 {
                assert_eq!(target.get(x, y), blue(), "local column {x},{y} is painted");
            }
        }
        assert_eq!(target.get(5, 1), Premul::TRANSPARENT);
    }

    /// The plain case: both buffers share the same device-space origin, so a
    /// snapshot is a straight copy.
    #[test]
    fn snapshot_from_copies_pixels_when_origins_match() {
        let mut source = Target::new(4, 4);
        source.set(1, 1, red());
        source.set(3, 3, blue());

        let mut dest = Target::new(4, 4);
        dest.snapshot_from(&source, (0, 0), (0, 0));

        assert_eq!(dest.get(1, 1), red());
        assert_eq!(dest.get(3, 3), blue());
        assert_eq!(dest.get(0, 0), Premul::TRANSPARENT);
    }

    /// The case `Command::PushLayer` actually hits: the destination
    /// (`self`) is a smaller offscreen buffer sitting somewhere inside a
    /// larger source at a nonzero device-space offset. Each is expressed in
    /// its own local coordinates, and it is exactly this arithmetic —
    /// mapping one buffer's origin through the other's — that a snapshot
    /// implementation gets wrong by an off-by-origin bug if either offset is
    /// dropped.
    #[test]
    fn snapshot_from_maps_a_nested_layers_origin_correctly() {
        // A 10x10 source (e.g. the root frame) with a single red pixel at
        // absolute device position (7, 7).
        let mut source = Target::new(10, 10);
        source.set(7, 7, red());

        // A 4x4 destination layer whose device-space top-left is (5, 5) —
        // so absolute (7, 7) is local (2, 2) inside it.
        let mut dest = Target::new(4, 4);
        dest.snapshot_from(&source, (0, 0), (5, 5));

        assert_eq!(
            dest.get(2, 2),
            red(),
            "the marked pixel lands at the mapped local position"
        );
        // Every other pixel in range is still whatever the source had there:
        // transparent.
        assert_eq!(dest.get(0, 0), Premul::TRANSPARENT);
        assert_eq!(dest.get(3, 3), Premul::TRANSPARENT);
    }

    /// A destination region that reaches beyond the source's own bounds
    /// reads back transparent for the out-of-range part rather than
    /// panicking or wrapping — `Target::get`'s existing out-of-bounds
    /// contract, which a snapshot must inherit rather than special-case.
    #[test]
    fn snapshot_from_reads_transparent_beyond_the_sources_edge() {
        let mut source = Target::new(4, 4);
        source.set(3, 3, red());

        // A destination positioned so it overhangs the source's bottom-right
        // edge.
        let mut dest = Target::new(4, 4);
        dest.snapshot_from(&source, (0, 0), (2, 2));

        assert_eq!(
            dest.get(1, 1),
            red(),
            "the one in-range pixel is still copied correctly"
        );
        assert_eq!(
            dest.get(3, 3),
            Premul::TRANSPARENT,
            "out of the source's range reads as transparent"
        );
    }

    /// Both `source_origin` and `dest_origin` participate — a fully general
    /// case where neither buffer sits at the device's `(0, 0)`, which is
    /// what a backdrop-filtered layer nested inside another layer produces
    /// (the parent frame is itself an offscreen buffer with its own
    /// nonzero origin).
    #[test]
    fn snapshot_from_composes_both_origins() {
        // Source is itself an offscreen buffer whose device-space top-left
        // is (100, 100), holding one marked pixel at absolute (108, 104).
        let mut source = Target::new(20, 20);
        source.set(8, 4, blue());

        // Destination's device-space top-left is (105, 102): absolute
        // (108, 104) should land at local (3, 2).
        let mut dest = Target::new(6, 6);
        dest.snapshot_from(&source, (100, 100), (105, 102));

        assert_eq!(dest.get(3, 2), blue());
    }

    // ── LCD subpixel compositing ─────────────────────────────────────────

    use super::super::geometry::fill::{CoverageMask, LcdMask, LCD_SUBPIXELS};
    use vieww_foundation::Rect as DeviceRect;

    /// An LCD mask whose three channels are equal is a gray mask in disguise,
    /// and `composite_lcd` must produce the *same bytes* the gray path
    /// produces for it — not close, not within a tolerance: the same, because
    /// the equal-channel branch deliberately calls the gray path's own
    /// `over(color.scaled(k), dst)` rather than re-deriving the arithmetic.
    /// This is what makes the LCD and gray paths unable to drift apart on the
    /// pixels where they agree.
    #[test]
    fn an_equal_channel_lcd_composite_is_the_gray_composite_bit_for_bit() {
        let ink = vieww_foundation::Color::rgb(30, 30, 30);
        let coverage = 0.6_f32;

        let gray = {
            let mut target = Target::new(8, 8);
            target.reset(8, 8, Premul::from_straight(vieww_foundation::Color::WHITE));
            let mask = CoverageMask {
                x0: 1,
                y0: 1,
                width: 4,
                height: 4,
                data: vec![coverage; 16],
            };
            target.composite_flat(
                mask.view(),
                (0, 0),
                vieww_foundation::BlendMode::Normal,
                ColorPipeline::GammaSpace,
                Premul::from_straight(ink),
            );
            target
        };
        let lcd = {
            let mut target = Target::new(8, 8);
            target.reset(8, 8, Premul::from_straight(vieww_foundation::Color::WHITE));
            let mask = LcdMask {
                x0: 1,
                y0: 1,
                width: 4,
                height: 4,
                data: vec![coverage; 16 * LCD_SUBPIXELS],
            };
            target.composite_lcd(
                mask.view_translated(0, 0, DeviceRect::new(-1.0e3, -1.0e3, 1.0e3, 1.0e3))
                    .expect("in range"),
                (0, 0),
                ink,
                |_, _| 1.0,
            );
            target
        };

        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    gray.get(x, y),
                    lcd.get(x, y),
                    "pixel ({x},{y}) differs between the gray and equal-channel LCD paths"
                );
            }
        }
    }

    /// The split-channel case, asserted in the direction that makes physical
    /// sense: a stripe fully covering the R sub-column and not the B one
    /// blends *more red in and keeps more blue out*, over a white background
    /// with dark ink — so the resulting pixel's blue channel must sit above
    /// its red channel. White is 255 in every channel; dark ink is 30; the R
    /// stripe at full coverage pulls R all the way to 30 while B stays at
    /// 255, G lands between.
    #[test]
    fn a_split_channel_lcd_composite_pushes_red_in_and_keeps_blue_out() {
        let ink = vieww_foundation::Color::rgb(30, 30, 30);
        let mut target = Target::new(4, 4);
        target.reset(4, 4, Premul::from_straight(vieww_foundation::Color::WHITE));

        let mask = LcdMask {
            x0: 0,
            y0: 0,
            width: 2,
            height: 2,
            // Pixel 0: [1, 0.5, 0] — the centred-vertical-stem edge. Pixel 1:
            // [0, 0, 0], uncovered, same row.
            data: vec![1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.0, 0.0, 0.0, 0.0],
        };
        target.composite_lcd(
            mask.view_translated(0, 0, DeviceRect::new(-1.0e3, -1.0e3, 1.0e3, 1.0e3))
                .expect("in range"),
            (0, 0),
            ink,
            |_, _| 1.0,
        );

        let px = target.get(0, 0).to_straight_u8();
        assert_eq!(px[0], 30, "full R coverage takes R to the ink colour");
        assert!(
            px[2] > px[1] && px[1] > px[0],
            "G at half coverage lands between R (full) and B (none): {:?}",
            px
        );
        assert_eq!(px[2], 255, "no B coverage leaves the background's B alone");
        // And the uncovered neighbour is untouched white.
        assert_eq!(
            target.get(1, 0).to_straight_u8(),
            [255, 255, 255, 255],
            "zero coverage on every channel writes nothing"
        );
    }
}
