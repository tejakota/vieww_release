//! Rasterize a repeated shape once, reuse its coverage at every position —
//! "Renderer v2" pillar D (`docs/RENDERER-V2-NOTES.md`): "Selectively move
//! large, repeated UI workloads (icon grids, virtualized lists, tables,
//! charts, scrolling surfaces) onto a GPU-driven path... retained draw
//! records → GPU-visible instance buffers → GPU culling → indirect draw
//! generation → batched execution."
//!
//! # The CPU-realizable version of that idea
//!
//! There is no GPU-driven execution pipeline in this renderer — no GPU
//! culling, no indirect draw buffer, no compute-shader instancing, because
//! there is no GPU rendering pipeline at all (`vieww-hal`'s Vulkan backend
//! only *presents* pixels this CPU rasterizer already produced; see
//! `native/mod.rs`'s module docs). What survives the translation to a CPU
//! scanline rasterizer is the one idea underneath all that GPU vocabulary
//! that has a direct, exact CPU analogue: **do the expensive geometric work
//! once for identical shapes, not once per instance.**
//!
//! [`InstancedMask`] is that: [`InstancedMask::rasterize_once`] runs
//! `geometry::fill::rasterize` — the expensive scanline pass — exactly
//! once, and [`InstancedMask::translated`] produces every further instance
//! by copying the resulting coverage samples to a new device-space origin,
//! never re-walking a single edge.
//!
//! # Why this is exact, not approximated, and only for whole-pixel shifts
//!
//! A coverage mask's values are how much of each pixel a shape's edges
//! cover — a property of the shape's position *relative to the pixel
//! grid*, not of its absolute position. Shifting a shape by a **whole**
//! number of device pixels moves every edge by an integer amount, so every
//! pixel's coverage value relative to its own cell is completely unchanged
//! — only *which* pixel each sample belongs to changes, which is exactly
//! what [`InstancedMask::translated`] does by adjusting `x0`/`y0` and nothing
//! else. Shifting by a fractional pixel amount would change antialiasing at
//! every edge, which is a real difference this module refuses to
//! approximate away — [`InstancedMask::translated`] only accepts a whole-pixel
//! `(dx, dy)`, and a caller whose instances do not line up on the pixel
//! grid needs a fresh rasterization for those, same as before this module
//! existed. This is the same choice `docs/RENDERER-V2-NOTES.md`'s own
//! accounting of pillar E records: reuse only when it is exact, not a
//! quietly chosen quality/speed tradeoff.
//!
//! # Integration status
//!
//! This module is real and tested standalone — see this module's own
//! `tests` submodule, in particular
//! `n_identical_shapes_at_different_positions_match_todays_rendering_with_less_rasterization_work`,
//! which renders the same five shapes through today's ordinary
//! `Scene`/`NativeRenderer` path and through this module directly, checks
//! the pixels come out identical, and — via a `#[cfg(test)]`-only call
//! counter on `geometry::fill::rasterize` itself — measures that the
//! instanced path actually rasterizes once where the ordinary path
//! rasterizes once per shape, rather than only asserting the reduction by
//! construction. This can only be an internal test (not
//! `vieww-paint/tests/*.rs`) because `InstancedMask` is `pub(crate)` — see
//! below for why it stays that way for now. This module is **not wired
//! into
//! `NativeRenderer::apply`'s per-command loop**, and is not exposed as
//! public API outside this crate (`pub(crate)`, like every other piece of
//! `native`'s internals). That loop processes one `Command` at a time in
//! scene order and has no pre-pass that would notice two
//! `FillRect`/`FillPath` commands share a shape, which is exactly the
//! detection this module needs to be handed rather than doing on its own —
//! and there is also no repeated-geometry detection anywhere upstream of
//! `native` (in `vieww-render`/`vieww-widget`) that would produce such a
//! hint today. Both of those are real further work, not done here; what is
//! done is the reuse mechanism itself — proven correct and measured — that
//! either layer could be built against once it exists.

use vieww_foundation::Rect;

use super::geometry::fill::{rasterize, CoverageMask};
use super::geometry::flatten::Polyline;

/// A shape's coverage, rasterized once at whatever device position its
/// `polylines` were originally flattened to, ready to be copied to any
/// other whole-pixel device position without re-rasterizing.
///
/// `#[allow(dead_code)]`: real outside `#[cfg(test)]` only once something
/// upstream calls it — see the module docs' "Integration status". Kept
/// rather than deleted, the same way this crate keeps other tested,
/// documented, not-yet-wired-in pieces (`crate::graph`'s
/// `co_schedulable_siblings`, for one).
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct InstancedMask {
    base: CoverageMask,
}

#[allow(dead_code)] // See the struct's own `#[allow(dead_code)]` doc comment above.
impl InstancedMask {
    /// Rasterize `polylines` — already flattened into **device space** at
    /// whatever position the first instance sits at, same as any ordinary
    /// (non-instanced) fill — with no clip bound narrower than the shape's
    /// own extent, so the resulting mask represents the *whole* shape and
    /// can be safely translated later: clipping is applied separately, at
    /// composite time, by every caller in this crate already (a
    /// `CoverageMask` carries geometry coverage only — see
    /// `native/reference.rs`'s `paint_shape`, which multiplies by a clip's
    /// own per-pixel coverage after sampling this kind of mask, not before).
    #[must_use]
    pub(crate) fn rasterize_once(polylines: &[Polyline], local_bounds: Rect) -> Self {
        // A margin, not a clip: `rasterize`'s `clip_bounds` only ever
        // narrows the mask, and this call needs it to narrow nothing, so
        // `local_bounds` is grown well past any antialiasing bleed a real
        // clip elsewhere might have trimmed.
        const MARGIN: f32 = 4.0;
        Self {
            base: rasterize(polylines, local_bounds.inflate(MARGIN)),
        }
    }

    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.base.is_empty()
    }

    /// This shape's coverage, copied to `(dx, dy)` whole device pixels away
    /// from where it was originally rasterized — no edges re-walked, no
    /// coverage recomputed, just a new origin over the same samples. See
    /// the module docs for why `dx`/`dy` must be whole pixels for this to
    /// be exact.
    #[must_use]
    pub(crate) fn translated(&self, dx: i32, dy: i32) -> CoverageMask {
        CoverageMask {
            x0: self.base.x0 + dx,
            y0: self.base.y0 + dy,
            width: self.base.width,
            height: self.base.height,
            data: self.base.data.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vieww_foundation::{BlendMode, Color, Offset, Path, Transform};

    use super::super::color::Premul;
    use super::super::geometry::fill::{rasterize_call_count, reset_rasterize_call_count};
    use super::super::linear::ColorPipeline;
    use super::super::pixels::Pixels;
    use super::super::reference::NativeRenderer;
    use super::super::target::Target;
    use crate::{Canvas, Scene};

    fn square_polylines(size: f32) -> Vec<Polyline> {
        let mut path = Path::new();
        path.move_to(Offset::new(0.0, 0.0));
        path.line_to(Offset::new(size, 0.0));
        path.line_to(Offset::new(size, size));
        path.line_to(Offset::new(0.0, size));
        path.close();
        super::super::geometry::flatten::flatten_path(&path, Transform::IDENTITY)
    }

    #[test]
    fn translating_reuses_the_same_coverage_data_at_a_new_origin() {
        let polylines = square_polylines(10.0);
        let bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
        let mask = InstancedMask::rasterize_once(&polylines, bounds);
        assert!(!mask.is_empty());

        let moved = mask.translated(50, 30);
        assert_eq!(moved.x0, mask.base.x0 + 50);
        assert_eq!(moved.y0, mask.base.y0 + 30);
        assert_eq!(moved.width, mask.base.width);
        assert_eq!(moved.height, mask.base.height);
        assert_eq!(
            moved.data, mask.base.data,
            "translated coverage samples must be byte-identical, not recomputed"
        );
    }

    #[test]
    fn a_translated_mask_samples_at_its_new_position_not_its_old_one() {
        let polylines = square_polylines(10.0);
        let bounds = Rect::new(0.0, 0.0, 10.0, 10.0);
        let mask = InstancedMask::rasterize_once(&polylines, bounds);
        let original_center = mask.base.coverage_at(5, 5);
        assert!(
            original_center > 0.99,
            "center of a 10x10 square must be fully covered"
        );

        let moved = mask.translated(100, 0);
        assert_eq!(
            moved.coverage_at(5, 5),
            0.0,
            "the old position must read as uncovered after translation"
        );
        assert!(
            moved.coverage_at(105, 5) > 0.99,
            "the new position must carry the same coverage the old one had"
        );
    }

    /// The pillar-D deliverable, exactly as specified: "N identical shapes
    /// at different positions produce identical pixels to today's
    /// per-shape rasterization, with a measured rasterization-work
    /// reduction." Two independent code paths render the same five
    /// identically-shaped, differently-positioned squares — one calling
    /// `rasterize` once per square (today's ordinary `Scene`/
    /// `NativeRenderer` path, entirely unmodified by this pillar), the
    /// other calling it exactly once total via `InstancedMask` — and their
    /// output pixels are compared byte for byte.
    #[test]
    fn n_identical_shapes_at_different_positions_match_todays_rendering_with_less_rasterization_work(
    ) {
        const POSITIONS: [(i32, i32); 5] = [(4, 4), (24, 4), (44, 4), (4, 24), (24, 24)];
        const SIZE: f32 = 12.0;
        const CANVAS: u32 = 60;

        // Baseline: today's ordinary per-shape path, completely untouched
        // by this pillar — five separate `FillRect` commands.
        reset_rasterize_call_count();
        let mut scene = Scene::new();
        for &(x, y) in &POSITIONS {
            scene.fill_rect(
                Rect::new(x as f32, y as f32, x as f32 + SIZE, y as f32 + SIZE),
                Color::rgba(220, 30, 30, 255).into(),
            );
        }
        let (baseline_pixels, _) = NativeRenderer::new()
            .render_to_pixels(&scene, CANVAS, CANVAS, Color::WHITE)
            .expect("headless render needs no display");
        let baseline_rasterize_calls = rasterize_call_count();
        assert_eq!(
            baseline_rasterize_calls,
            POSITIONS.len(),
            "today's path rasterizes once per shape"
        );

        // Instanced: rasterize the shared 12x12 square exactly once — at the
        // first instance's own device position — then translate it to each
        // of the other four positions.
        reset_rasterize_call_count();
        let (first_x, first_y) = POSITIONS[0];
        let mut square = Path::new();
        square.move_to(Offset::new(0.0, 0.0));
        square.line_to(Offset::new(SIZE, 0.0));
        square.line_to(Offset::new(SIZE, SIZE));
        square.line_to(Offset::new(0.0, SIZE));
        square.close();
        let device_transform = Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: first_x as f32,
            ty: first_y as f32,
        };
        let device_polylines =
            super::super::geometry::flatten::flatten_path(&square, device_transform);
        let mask = InstancedMask::rasterize_once(
            &device_polylines,
            Rect::new(
                first_x as f32,
                first_y as f32,
                first_x as f32 + SIZE,
                first_y as f32 + SIZE,
            ),
        );

        let mut target = Target::filled(CANVAS, CANVAS, Premul::from_straight(Color::WHITE));
        let color = Premul::from_straight(Color::rgba(220, 30, 30, 255));
        for &(x, y) in &POSITIONS {
            let translated = mask.translated(x - first_x, y - first_y);
            target.composite_coverage(
                translated.view(),
                (0, 0),
                BlendMode::Normal,
                ColorPipeline::GammaSpace,
                |_, _| color,
            );
        }
        let instanced_rasterize_calls = rasterize_call_count();
        assert_eq!(
            instanced_rasterize_calls, 1,
            "instanced batching must rasterize the shared shape exactly once"
        );

        let instanced_pixels = Pixels::from_target(&target);
        assert_eq!(
            baseline_pixels.data(),
            instanced_pixels.data(),
            "instanced rendering must produce pixel-identical output to today's per-shape rasterization"
        );

        assert!(
            instanced_rasterize_calls < baseline_rasterize_calls,
            "instancing must measurably reduce rasterization work: baseline={baseline_rasterize_calls}, instanced={instanced_rasterize_calls}"
        );
    }
}
