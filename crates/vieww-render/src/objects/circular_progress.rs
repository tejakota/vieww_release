use std::f32::consts::{FRAC_PI_2, PI, TAU};

use vieww_foundation::{Color, Constraints, Offset, Path, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject, Semantics};

/// Where a determinate arc begins, and where an indeterminate one begins its
/// first cycle: twelve o'clock.
///
/// Zero radians is three o'clock (see [`Path::arc`]), and a progress indicator
/// that started there would read as already a quarter done.
const TWELVE: f32 = -FRAC_PI_2;

/// The shortest an indeterminate arc gets, as a fraction of the circle.
///
/// Not zero. An arc that vanishes completely at the ends of its cycle reads as
/// a spinner that has stopped and restarted rather than one that is still
/// going, and the pause is exactly where somebody decides the app has hung.
const MIN_SWEEP: f32 = 0.08;

/// The longest it gets. Short of a full circle on purpose: an arc that closes
/// is a ring, and a ring is rotationally symmetric, so the rotation it is in
/// the middle of stops being visible at the one moment it is fastest.
const MAX_SWEEP: f32 = 0.75;

/// A circular progress indicator: a band of a circle, filled or sweeping.
///
/// Its own render object rather than a composition, which makes it the second
/// control after [`RenderSlider`](crate::RenderSlider) to need one. The reason
/// is simply that an arc is a [`Path`] and nothing in the widget layer draws an
/// arbitrary path — [`Icon`](vieww_foundation::IconData) draws *an icon*, and an
/// arc built into an `IconData` every frame would be both a lie about what an
/// icon is and an allocation per frame in the widget layer.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCircularProgress {
    /// How far along, `0.0..=1.0` — or `None` when there is no number to give,
    /// which is what makes it sweep instead.
    pub value: Option<f32>,
    /// Where in its cycle an indeterminate arc is, `0.0..=1.0`. Ignored by a
    /// determinate one.
    pub phase: f32,
    /// The size it asks for, before constraints have their say.
    pub diameter: f32,
    /// How thick the band is, measured inwards from the outer edge.
    pub thickness: f32,
    /// The full circle drawn behind the indicator.
    pub track: Color,
    /// The part that represents progress.
    pub indicator: Color,
    /// Draw an indeterminate spinner as a ring of fading spokes rather than as
    /// a sweeping arc.
    ///
    /// # Why a second shape rather than a second widget
    ///
    /// Apple's activity indicator has been a ring of tapered spokes since the
    /// first iPhone and is one of the two or three shapes people identify a
    /// platform by; Android's has been a sweeping arc for as long. They are the
    /// same control with the same meaning and the same API, so this is a flag
    /// rather than a separately-named activity widget nobody would remember to
    /// reach for.
    ///
    /// Ignored by a *determinate* indicator: a fraction is a fraction, and both
    /// platforms draw it as an arc.
    pub spokes: bool,
}

impl RenderCircularProgress {
    #[must_use]
    pub const fn new(value: Option<f32>, diameter: f32, thickness: f32) -> Self {
        Self {
            value,
            phase: 0.0,
            diameter,
            thickness,
            track: Color::TRANSPARENT,
            indicator: Color::TRANSPARENT,
            spokes: false,
        }
    }

    /// Draw the indeterminate form as Apple's ring of spokes.
    #[must_use]
    pub const fn spokes(mut self, spokes: bool) -> Self {
        self.spokes = spokes;
        self
    }

    #[must_use]
    pub const fn phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }

    #[must_use]
    pub const fn colors(mut self, track: Color, indicator: Color) -> Self {
        self.track = track;
        self.indicator = indicator;
        self
    }

    /// Where the arc starts and how far it goes, in radians.
    ///
    /// # One animation, not two
    ///
    /// The classic spinner rotates *and* grows and shrinks, and the obvious build
    /// is two controllers. Both fall out of one `phase` here instead, which
    /// keeps them in step by construction — two controllers drifting apart is a
    /// spinner that stutters once every few seconds and looks like dropped
    /// frames.
    ///
    /// The tail advances **monotonically**, one full turn per cycle, and the
    /// sweep grows and shrinks on top of it. That ordering is the part worth
    /// keeping: anchoring the tail and moving only the head makes the arc appear
    /// to reverse as it shrinks, which reads as an error rather than as motion.
    #[must_use]
    pub fn geometry(&self) -> (f32, f32) {
        match self.value {
            // `is_finite` first, and **not** `clamp` alone: `f32::clamp`
            // *propagates* NaN rather than sanitising it, so `NaN.clamp(0, 1)`
            // is NaN and the sweep becomes a path with NaN coordinates in it.
            // Worth knowing rather than rediscovering — clamp reads like it
            // bounds a number into a range, and it only does that for numbers.
            Some(value) if value.is_finite() => (TWELVE, TAU * value.clamp(0.0, 1.0)),
            // A `NaN` fraction is `0 / 0` — an empty file, a zero-length
            // response. Empty is the honest answer, and the one that draws.
            Some(_) => (TWELVE, 0.0),
            None => {
                let phase = if self.phase.is_finite() {
                    self.phase.rem_euclid(1.0)
                } else {
                    0.0
                };
                // `sin²` rather than a triangle wave: it is smooth at both ends,
                // so the arc eases into and out of its longest rather than
                // reversing at a corner.
                let grow = (phase * PI).sin().powi(2);
                let sweep = MIN_SWEEP + (MAX_SWEEP - MIN_SWEEP) * grow;
                (TWELVE + TAU * phase, TAU * sweep)
            }
        }
    }
}

impl RenderObject for RenderCircularProgress {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(Size::square(self.diameter))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        // The inscribed circle, so a box that is not square still gets a circle
        // rather than an ellipse — the same choice `RenderIcon` makes about a
        // shape in a box the wrong shape.
        let radius = bounds.width().min(bounds.height()) / 2.0;
        let thickness = self.thickness.min(radius);
        if radius <= 0.0 || thickness <= 0.0 {
            return;
        }
        let center = Offset::new(
            bounds.left + bounds.width() / 2.0,
            bounds.top + bounds.height() / 2.0,
        );

        if !self.track.is_transparent() {
            let ring = Path::arc_ring(center, radius, thickness, 0.0, TAU);
            ctx.canvas().fill_path(&ring, self.track.into());
        }
        if self.indicator.is_transparent() {
            return;
        }

        // **Apple's activity indicator**: twelve spokes around the circle, each
        // a short arc, their opacity trailing the phase so the ring appears to
        // rotate without anything actually moving. Twelve because that is what
        // the system draws, and because a ring with fewer reads as a dashed
        // circle rather than as motion.
        if self.spokes && self.value.is_none() {
            const SPOKES: usize = 12;
            let step = TAU / SPOKES as f32;
            for index in 0..SPOKES {
                // The lead spoke is the brightest and the one behind it fades,
                // which is what gives the ring its direction.
                #[expect(clippy::cast_precision_loss, reason = "twelve is exact in f32")]
                let behind = (index as f32 / SPOKES as f32 - self.phase).rem_euclid(1.0);
                let alpha = 0.15 + 0.85 * (1.0 - behind);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a fraction of 255 is a byte"
                )]
                let color = self
                    .indicator
                    .with_alpha((f32::from(self.indicator.a) * alpha).round() as u8);
                #[expect(clippy::cast_precision_loss, reason = "twelve is exact in f32")]
                let start = TWELVE + index as f32 * step;
                // Two thirds of the gap, so the spokes are separated rather
                // than meeting into a ring.
                let spoke = Path::arc_ring(center, radius, thickness, start, step * 0.66);
                if !spoke.is_empty() {
                    ctx.canvas().fill_path(&spoke, color.into());
                }
            }
            return;
        }

        let (start, sweep) = self.geometry();
        let arc = Path::arc_ring(center, radius, thickness, start, sweep);
        if !arc.is_empty() {
            ctx.canvas().fill_path(&arc, self.indicator.into());
        }
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // A spinner absorbs nothing. It is frequently the *only* thing on screen
        // while something loads, and one that swallowed taps would make a
        // loading screen feel broken rather than busy.
        false
    }

    fn semantics(&self) -> Option<Semantics> {
        // Announced by the control that wraps this, which is where the label and
        // the percentage live. Publishing here as well would make every spinner
        // two stops.
        None
    }

    /// The diameter, on either axis and at either end of the range.
    ///
    /// `layout` is `constrain(Size::square(self.diameter))` and nothing else, so
    /// the size this wants is a field — the same situation
    /// [`RenderIcon`](crate::RenderIcon) is in, and answered the same way. A
    /// spinner does not reflow, so there is no cross extent that would change
    /// the answer and no slack between the minimum and the maximum.
    ///
    /// # The animation is not consulted, and must not be
    ///
    /// [`phase`](Self::phase) and [`value`](Self::value) move sixty times a
    /// second and change nothing geometric — `layout_differs` already says so.
    /// An intrinsic is required to be a pure function of the subtree's
    /// configuration precisely so that its cached answer can be thrown away by
    /// the same thing that invalidates layout; an answer that moved with the
    /// clock would be stale the frame after it was cached, and a parent sized
    /// from it would resize while the spinner span.
    ///
    /// [`thickness`](Self::thickness) is not consulted either. It is measured
    /// *inwards* from the outer edge and clamped to the radius in `paint`, so it
    /// can never ask for more room than the diameter already provides.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        _query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        Some(self.diameter)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // The phase changes every frame and the geometry does not. The same
        // trap `RenderPerformanceOverlay` documents: comparing by equality here
        // would relayout this and everything under it sixty times a second, to
        // arrive at the same size each time.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>().is_none_or(|other| {
            (self.diameter - other.diameter).abs() > f32::EPSILON
                || (self.thickness - other.thickness).abs() > f32::EPSILON
        })
    }

    fn debug_name(&self) -> &'static str {
        "RenderCircularProgress"
    }
}

#[cfg(test)]
mod tests {
    use vieww_paint::Scene;

    use super::*;

    fn spinner(value: Option<f32>) -> RenderCircularProgress {
        RenderCircularProgress::new(value, 36.0, 4.0).colors(Color::WHITE, Color::BLACK)
    }

    fn painted(object: &RenderCircularProgress, size: Size) -> Scene {
        let mut scene = Scene::new();
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin: Offset::ZERO,
            size,
            dpr: 1.0,
        };
        object.paint(&mut ctx);
        scene
    }

    #[test]
    fn a_determinate_arc_starts_at_twelve_and_spans_its_value() {
        let (start, sweep) = spinner(Some(0.25)).geometry();
        assert!((start - TWELVE).abs() < 1e-5, "progress starts at the top");
        assert!(
            (sweep - TAU / 4.0).abs() < 1e-4,
            "a quarter done is a quarter turn, got {sweep}"
        );
    }

    #[test]
    fn a_nonsense_value_is_clamped_rather_than_drawn() {
        // Progress is a division, and `downloaded / total` is happily above one
        // for a server that under-reports its length. The same argument
        // `LinearProgress::new` makes about not panicking inside a loading
        // indicator.
        assert!((spinner(Some(4.0)).geometry().1 - TAU).abs() < 1e-4);
        assert_eq!(spinner(Some(-1.0)).geometry().1, 0.0);
        assert_eq!(spinner(Some(f32::NAN)).geometry().1, 0.0);
    }

    #[test]
    fn an_indeterminate_arc_never_disappears_and_never_closes() {
        // Vanishing reads as "stopped"; closing makes a ring, and a ring is
        // rotationally symmetric, so the spin becomes invisible at speed.
        for step in 0..=100 {
            let phase = step as f32 / 100.0;
            let (_, sweep) = spinner(None).phase(phase).geometry();
            assert!(
                sweep >= TAU * MIN_SWEEP - 1e-4,
                "vanished at {phase}: {sweep}"
            );
            assert!(
                sweep <= TAU * MAX_SWEEP + 1e-4,
                "closed at {phase}: {sweep}"
            );
        }
    }

    #[test]
    fn the_tail_only_ever_advances() {
        // Anchoring the tail and moving only the head makes a shrinking arc look
        // like it is running backwards. Whatever the sweep is doing, where the
        // arc *starts* must keep going the same way.
        let mut last = f32::NEG_INFINITY;
        for step in 0..100 {
            let phase = step as f32 / 100.0;
            let (start, _) = spinner(None).phase(phase).geometry();
            assert!(start >= last, "went backwards at {phase}: {start} < {last}");
            last = start;
        }
    }

    #[test]
    fn the_cycle_joins_up_at_both_ends() {
        // Phase 0 and phase 1 are the same moment. A sweep that differed between
        // them would jump once per cycle, which is the kind of stutter that gets
        // blamed on the frame scheduler.
        let (_, first) = spinner(None).phase(0.0).geometry();
        let (_, last) = spinner(None).phase(1.0).geometry();
        assert!((first - last).abs() < 1e-4, "{first} vs {last}");
    }

    #[test]
    fn a_wild_phase_does_not_produce_a_wild_arc() {
        // A phase outside 0..1 is what an animation reports when it has been
        // asked to repeat, and NaN is what a zero-length duration produces.
        for phase in [3.25_f32, -0.75, f32::NAN, f32::INFINITY] {
            let (start, sweep) = spinner(None).phase(phase).geometry();
            assert!(start.is_finite() && sweep.is_finite(), "{phase} -> {start}");
            assert!(sweep >= TAU * MIN_SWEEP - 1e-4);
        }
    }

    #[test]
    fn both_the_track_and_the_indicator_are_drawn() {
        let scene = painted(&spinner(Some(0.5)), Size::square(36.0));
        assert_eq!(
            scene.commands().len(),
            2,
            "a track behind an indicator: {scene:?}"
        );
    }

    #[test]
    fn nothing_is_drawn_into_a_box_with_no_room() {
        let scene = painted(&spinner(Some(0.5)), Size::ZERO);
        assert!(scene.commands().is_empty(), "{scene:?}");
    }

    #[test]
    fn a_new_phase_does_not_ask_for_a_relayout() {
        let a = spinner(None).phase(0.1);
        let b = spinner(None).phase(0.9);
        assert!(
            !a.layout_differs(&b),
            "a spinner that relaid out every frame would relayout the screen"
        );

        let mut wider = spinner(None);
        wider.diameter = 48.0;
        assert!(a.layout_differs(&wider), "a different size is a relayout");
    }

    // -------------------------------------------------------------- intrinsics

    fn intrinsic(object: RenderCircularProgress, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.intrinsic(id, query)
    }

    #[test]
    fn a_spinner_reports_its_diameter_on_both_axes_at_both_ends() {
        for query in [
            crate::IntrinsicQuery::min_width(),
            crate::IntrinsicQuery::max_width(),
            crate::IntrinsicQuery::min_height(),
            crate::IntrinsicQuery::max_height(),
        ] {
            assert_eq!(
                intrinsic(spinner(Some(0.5)), query),
                Some(36.0),
                "a circle is square and has no slack: {query:?}"
            );
        }
    }

    #[test]
    fn a_cross_extent_does_not_change_the_diameter() {
        // Nothing here reflows. A spinner squeezed into a narrow box draws a
        // smaller circle, but what it *wants* is unchanged — and an intrinsic
        // answers for the content, not for the constraints.
        assert_eq!(
            intrinsic(
                spinner(None),
                crate::IntrinsicQuery::max_height().across(4.0)
            ),
            Some(36.0)
        );
    }

    #[test]
    fn the_phase_and_the_value_do_not_move_the_intrinsic() {
        // The purity the contract requires: this object is re-supplied every
        // frame with a new phase, and an answer that moved with it would be a
        // cached measurement that goes stale sixty times a second.
        let query = crate::IntrinsicQuery::max_width();
        assert_eq!(
            intrinsic(spinner(None).phase(0.9), query),
            intrinsic(spinner(Some(0.25)).phase(0.1), query)
        );
    }

    #[test]
    fn the_intrinsic_is_the_size_an_unbounded_layout_arrives_at() {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(spinner(Some(0.5))));
        let laid = tree.layout(id, Constraints::UNBOUNDED);

        assert_eq!(
            intrinsic(spinner(Some(0.5)), crate::IntrinsicQuery::max_width()),
            Some(laid.width)
        );
    }
}
