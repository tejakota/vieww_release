use std::time::Duration;

use vieww_foundation::{Color, Constraints, Offset, Rect, Size};

use crate::{LayoutCtx, PaintCtx, RenderObject, Semantics};

/// Default height of the graph, in logical pixels.
pub const OVERLAY_HEIGHT: f32 = 48.0;

/// Where the budget line sits, as a fraction of the height measured from the
/// bottom.
///
/// Half, so a frame at exactly budget draws a bar to the middle and one at twice
/// budget fills the graph. A budget line at the *top* would be the obvious
/// choice and is the wrong one: every bar in a healthy app would then be a
/// sliver at the bottom, and the thing you actually watch for — a frame
/// creeping up on its budget — would be invisible until it was already late.
pub const BUDGET_FRACTION: f32 = 0.5;

/// Narrowest a bar is allowed to get before the graph shows fewer samples.
const MIN_BAR_WIDTH: f32 = 2.0;

/// Width used when the overlay is laid out with no bound on it at all.
///
/// An overlay belongs in a `Stack` over a screen, where the width is always
/// bounded. This exists so that putting one somewhere unbounded produces a
/// graph instead of an infinity.
const FALLBACK_WIDTH: f32 = 240.0;

/// A bar graph of what recent frames cost, drawn against their budget.
///
/// One bar per frame, oldest at the left. A bar's height is its duration
/// relative to the budget; a bar over budget is drawn in a different colour, and
/// a line marks the budget itself.
///
/// # Why it draws no text
///
/// A frame rate rendered as digits would need a font, and a
/// [`FontStore`](vieww_text::FontStore) that has not been handed faces loads the
/// system's — 33 seconds in a debug build, which is the build a performance
/// overlay is turned on in. The classic performance overlay is bars for the same
/// reason. The numbers are already available in prose from `FrameLog::report`;
/// what a graph adds is the *shape* of a stutter, which is the part digits are
/// bad at anyway.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPerformanceOverlay {
    /// Frame costs, oldest first.
    pub samples: Vec<Duration>,
    /// The per-frame budget the bars are measured against.
    pub budget: Duration,
    /// Height of the graph.
    pub height: f32,
    /// Fill behind the bars.
    pub background: Color,
    /// A frame that met its budget.
    pub within_budget: Color,
    /// A frame that did not.
    pub over_budget: Color,
}

impl RenderPerformanceOverlay {
    /// An overlay of `samples` against `budget`, in the default colours.
    #[must_use]
    pub fn new(samples: Vec<Duration>, budget: Duration) -> Self {
        Self {
            samples,
            budget,
            height: OVERLAY_HEIGHT,
            background: Color::argb(0xC0_20_20_20),
            within_budget: Color::hex(0x4C_AF_50),
            over_budget: Color::hex(0xE5_39_35),
        }
    }

    /// How many bars fit across `width`.
    fn capacity(width: f32) -> usize {
        if width < MIN_BAR_WIDTH {
            return 1;
        }
        // Truncation is the point: a partial bar is not a bar.
        let fits = (width / MIN_BAR_WIDTH) as usize;
        fits.max(1)
    }

    /// The bar height for one sample, clamped into the graph.
    fn bar_height(&self, sample: Duration, height: f32) -> f32 {
        let budget = self.budget.as_secs_f32();
        if budget <= 0.0 {
            // No budget to measure against. Every frame is drawn at the line
            // rather than at zero or at infinity, so the graph still shows that
            // frames are arriving.
            return height * BUDGET_FRACTION;
        }
        let ratio = sample.as_secs_f32() / budget;
        // A floor of one pixel: a frame that cost almost nothing still happened,
        // and a graph with gaps in it reads as dropped frames.
        (ratio * BUDGET_FRACTION * height).clamp(1.0, height)
    }
}

impl RenderObject for RenderPerformanceOverlay {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let width = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            FALLBACK_WIDTH
        };
        constraints.constrain(Size::new(width, self.height))
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        let (width, height) = (bounds.width(), bounds.height());
        if width <= 0.0 || height <= 0.0 {
            return;
        }

        if !self.background.is_transparent() {
            ctx.canvas().fill_rect(bounds, self.background.into());
        }

        let capacity = Self::capacity(width);
        let bar_width = width / capacity as f32;
        let bottom = bounds.bottom;

        // Only the newest `capacity` samples fit. Dropping from the front keeps
        // the graph scrolling rather than rescaling as history accumulates.
        let shown = self.samples.len().min(capacity);
        let first = self.samples.len() - shown;

        for (index, &sample) in self.samples[first..].iter().enumerate() {
            let bar = self.bar_height(sample, height);
            let left = bounds.left + index as f32 * bar_width;
            let color = if sample > self.budget {
                self.over_budget
            } else {
                self.within_budget
            };
            if color.is_transparent() {
                continue;
            }
            ctx.canvas().fill_rect(
                Rect::new(left, bottom - bar, left + bar_width, bottom),
                color.into(),
            );
        }

        // The budget line goes on top of the bars: it is the thing being read
        // against, and a bar drawn over it would hide exactly the crossing that
        // matters.
        let line_y = bottom - height * BUDGET_FRACTION;
        ctx.canvas().fill_rect(
            Rect::new(bounds.left, line_y, bounds.right, line_y + 1.0),
            Color::WHITE.into(),
        );
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Never. A diagnostic that swallowed taps would change the behaviour of
        // the application it is there to measure, and the first thing anybody
        // would notice is that the button underneath it stopped working.
        false
    }

    fn semantics(&self) -> Option<Semantics> {
        // Nothing to announce. This is a developer's instrument, not content,
        // and a screen reader stopping on it would be noise in every reading of
        // every screen it is enabled on.
        None
    }

    /// Its height outright; across, the width it falls back to, and no minimum.
    ///
    /// **Height** is [`height`](Self::height), at both extrema: `layout` uses
    /// that field whatever the constraints say, and a graph has no content that
    /// could make it want a different one.
    ///
    /// **The maximum width** is `FALLBACK_WIDTH`, which is what `layout` takes
    /// when nothing bounds it. Answering with anything else would put the
    /// intrinsic and the layout in disagreement about the same object in the
    /// same situation, which is the failure an intrinsic pass exists to avoid.
    ///
    /// **The minimum width is `Some(0.0)`, and that zero is a real answer rather
    /// than a stand-in for "I do not know".** The distinction is the whole
    /// reason this method returns an `Option` — see the
    /// `intrinsics` module documentation on the classic default of `0.0`.
    /// Here it happens to be true: the graph responds to a narrower box by
    /// showing fewer samples, `capacity` floors at one bar
    /// however little room there is, and nothing is ever clipped or drawn
    /// outside the box. There is genuinely no width below which this overflows,
    /// so zero is the honest minimum and reporting `None` would make an
    /// `IntrinsicWidth` above an overlay fall back for no reason.
    ///
    /// The samples and the budget are not consulted. They change every frame,
    /// `layout_differs` already refuses to relayout for them, and an intrinsic
    /// that moved with them would be a measurement invalidated at frame rate by
    /// the instrument that is supposed to be cheaper than what it measures.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        use crate::Extremum;

        Some(match (query.axis, query.extremum) {
            (Axis::Horizontal, Extremum::Max) => FALLBACK_WIDTH,
            (Axis::Horizontal, Extremum::Min) => 0.0,
            (Axis::Vertical, _) => self.height,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        // The samples change every single frame and the geometry never does.
        // `layout_differs_by_eq` would relayout the overlay — and everything
        // under it — sixty times a second, which is a measuring instrument
        // that costs more than what it measures.
        let any: &dyn std::any::Any = new;
        any.downcast_ref::<Self>()
            .is_none_or(|other| (self.height - other.height).abs() > f32::EPSILON)
    }

    fn debug_name(&self) -> &'static str {
        "RenderPerformanceOverlay"
    }
}

#[cfg(test)]
mod tests {
    use vieww_paint::{Command, Scene};

    use super::*;

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// 60Hz.
    fn budget() -> Duration {
        Duration::from_micros(16_667)
    }

    fn painted(object: &RenderPerformanceOverlay, size: Size) -> Scene {
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

    /// Every filled rectangle in the scene, in paint order.
    fn rects(scene: &Scene) -> Vec<(Rect, Color)> {
        scene
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::FillRect { rect, paint, .. } => Some((*rect, paint.color)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_frame_at_budget_reaches_the_budget_line() {
        let object = RenderPerformanceOverlay::new(vec![budget()], budget());
        let height = object.height;

        assert!((object.bar_height(budget(), height) - height * BUDGET_FRACTION).abs() < 0.01);
    }

    #[test]
    fn a_frame_over_budget_is_drawn_taller_and_in_the_warning_colour() {
        let object = RenderPerformanceOverlay::new(vec![ms(33)], budget());
        let scene = painted(&object, Size::new(100.0, object.height));
        let drawn = rects(&scene);

        let bar = drawn
            .iter()
            .find(|(_, color)| *color == object.over_budget)
            .expect("an over-budget bar");
        assert!(
            bar.0.height() > object.height * BUDGET_FRACTION,
            "a frame at twice budget rises past the line, got {:?}",
            bar.0
        );
    }

    #[test]
    fn a_frame_within_budget_stays_under_the_line_and_in_the_healthy_colour() {
        let object = RenderPerformanceOverlay::new(vec![ms(8)], budget());
        let scene = painted(&object, Size::new(100.0, object.height));
        let drawn = rects(&scene);

        let bar = drawn
            .iter()
            .find(|(_, color)| *color == object.within_budget)
            .expect("a within-budget bar");
        assert!(bar.0.height() < object.height * BUDGET_FRACTION);
    }

    #[test]
    fn a_ludicrous_frame_is_clamped_to_the_graph_rather_than_drawn_off_it() {
        let object = RenderPerformanceOverlay::new(vec![Duration::from_secs(10)], budget());
        assert!(
            (object.bar_height(Duration::from_secs(10), object.height) - object.height).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn a_frame_that_cost_almost_nothing_still_draws_a_bar() {
        let object = RenderPerformanceOverlay::new(vec![Duration::from_nanos(1)], budget());
        assert!(object.bar_height(Duration::from_nanos(1), object.height) >= 1.0);
    }

    #[test]
    fn a_zero_budget_does_not_divide_by_zero() {
        let object = RenderPerformanceOverlay::new(vec![ms(16)], Duration::ZERO);
        let bar = object.bar_height(ms(16), object.height);
        assert!(bar.is_finite(), "got {bar}");
    }

    #[test]
    fn only_the_newest_samples_are_drawn_when_history_outgrows_the_width() {
        // 10 wide fits 5 bars at the 2px minimum; 40 samples are supplied.
        let samples: Vec<Duration> = (0..40).map(|_| ms(8)).collect();
        let object = RenderPerformanceOverlay::new(samples, budget());
        let scene = painted(&object, Size::new(10.0, object.height));

        let bars = rects(&scene)
            .into_iter()
            .filter(|(_, color)| *color == object.within_budget)
            .count();
        assert_eq!(bars, 5, "the graph shows what fits, not what it was given");
    }

    #[test]
    fn an_empty_log_paints_the_background_and_the_line_but_no_bars() {
        let object = RenderPerformanceOverlay::new(Vec::new(), budget());
        let scene = painted(&object, Size::new(100.0, object.height));
        let drawn = rects(&scene);

        assert!(drawn.iter().any(|(_, color)| *color == object.background));
        assert!(!drawn
            .iter()
            .any(|(_, color)| *color == object.within_budget || *color == object.over_budget));
    }

    #[test]
    fn a_zero_sized_overlay_records_nothing() {
        let object = RenderPerformanceOverlay::new(vec![ms(8)], budget());
        assert!(painted(&object, Size::ZERO).is_empty());
    }

    #[test]
    fn new_samples_do_not_ask_for_a_relayout() {
        // The whole point. This object is re-supplied every frame.
        let object = RenderPerformanceOverlay::new(vec![ms(8)], budget());
        let updated = RenderPerformanceOverlay::new(vec![ms(8), ms(20)], budget());
        let mut taller = RenderPerformanceOverlay::new(vec![ms(8)], budget());
        taller.height = 96.0;

        assert!(!object.layout_differs(&updated));
        assert!(object.layout_differs(&taller));
    }

    #[test]
    fn the_overlay_takes_no_taps() {
        let object = RenderPerformanceOverlay::new(vec![ms(8)], budget());
        assert!(!object.hit_test_self(Offset::new(5.0, 5.0), Size::new(100.0, 48.0)));
    }

    #[test]
    fn the_overlay_is_not_something_a_screen_reader_stops_on() {
        assert!(RenderPerformanceOverlay::new(vec![ms(8)], budget())
            .semantics()
            .is_none());
    }

    // -------------------------------------------------------------- intrinsics

    fn intrinsic(object: RenderPerformanceOverlay, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(object));
        tree.intrinsic(id, query)
    }

    fn overlay() -> RenderPerformanceOverlay {
        RenderPerformanceOverlay::new(vec![ms(8), ms(20)], budget())
    }

    #[test]
    fn the_overlay_is_as_tall_as_the_graph_it_declares_at_both_extrema() {
        for query in [
            crate::IntrinsicQuery::min_height(),
            crate::IntrinsicQuery::max_height(),
        ] {
            assert_eq!(
                intrinsic(overlay(), query),
                Some(OVERLAY_HEIGHT),
                "{query:?}"
            );
        }

        let mut taller = overlay();
        taller.height = 96.0;
        assert_eq!(
            intrinsic(taller, crate::IntrinsicQuery::max_height()),
            Some(96.0)
        );
    }

    #[test]
    fn the_maximum_width_is_the_width_an_unbounded_layout_takes() {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(overlay()));
        let laid = tree.layout(id, Constraints::UNBOUNDED);

        assert_eq!(
            intrinsic(overlay(), crate::IntrinsicQuery::max_width()),
            Some(laid.width),
            "the intrinsic and the layout must not disagree about the same object"
        );
    }

    #[test]
    fn the_minimum_width_is_a_true_zero_rather_than_an_unknown() {
        // A graph that shows fewer samples in a narrower box never overflows it,
        // so zero really is the narrowest it can be laid out at. `Some(0.0)`
        // says exactly that, where `None` would make an `IntrinsicWidth` above
        // an overlay give up for no reason.
        assert_eq!(
            intrinsic(overlay(), crate::IntrinsicQuery::min_width()),
            Some(0.0)
        );
    }

    #[test]
    fn new_samples_do_not_move_the_intrinsic() {
        // This object is re-supplied every frame. An answer that moved with the
        // log would be an instrument that costs more than what it measures —
        // the same argument `layout_differs` makes.
        let query = crate::IntrinsicQuery::max_width();
        let busy = RenderPerformanceOverlay::new((0..500).map(|_| ms(33)).collect(), budget());
        assert_eq!(intrinsic(busy, query), intrinsic(overlay(), query));
    }

    #[test]
    fn a_cross_extent_changes_nothing() {
        // The bars rescale into whatever box they get; there is no width at
        // which the graph would like to be a different height.
        assert_eq!(
            intrinsic(
                overlay(),
                crate::IntrinsicQuery::max_height().across(1000.0)
            ),
            Some(OVERLAY_HEIGHT)
        );
    }
}
