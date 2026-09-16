use std::cell::Cell;
use std::fmt;

use vieww_foundation::{Color, Constraints, Offset, Path, Rect, Size};
use vieww_gestures::{DragRecognizer, GestureRecognizer, Recognized, TapRecognizer};
use vieww_widget::Handler;

use crate::{LayoutCtx, PaintCtx, RenderObject, Role, SemanticAction, Semantics};

/// The thickness of the track, and the radius of the thumb that rides it.
const TRACK_THICKNESS: f32 = 4.0;
const THUMB_RADIUS: f32 = 10.0;
/// How much larger than the thumb the pressed halo grows.
///
/// Twice the thumb, which is what every platform draws and what makes the thumb
/// still visible inside it. It reaches up to a thumb's width past each end of
/// the bar at the extremes of the value — deliberately, because the alternative
/// is a halo that goes lopsided exactly when the thumb is somewhere memorable.
///
/// A **factor** rather than a radius, because the thumb is no longer one size:
/// [`RenderSlider::thumb_radius`] is set per platform, and a constant computed
/// from `THUMB_RADIUS` would have drawn one platform's halo around the other's knob.
const HALO_SCALE: f32 = 2.0;

/// How wide a slider is when nothing constrains it.
///
/// A slider in unbounded width has no length to choose a value along, and
/// zero-width is worse than arbitrary — it would be invisible and unusable
/// rather than merely narrow.
const DEFAULT_WIDTH: f32 = 200.0;

/// A value chosen by dragging along a track.
///
/// # Why this is a render object and not a composition
///
/// Turning a drag into a value needs the track's **length**, and a widget cannot
/// know its own size — that is the whole content of constraints-down/sizes-up.
/// Only a render object is told how big it ended up, so only a render object can
/// do the arithmetic. `RenderEditableText` is here for the same reason: hit
/// testing a caret needs the laid-out text, which is also not knowable from a
/// description.
///
/// # It is as tall as a finger, not as tall as its track
///
/// The whole height is draggable, and the track is painted down the middle of
/// it. Wrapping a thin slider in a taller touch target instead would leave a
/// drag that began in the padding falling through to whatever is behind — the
/// recogniser lives on *this* object, so the object has to be the size of the
/// area that should respond.
pub struct RenderSlider {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    /// Snap to this many equal steps between the ends, if set.
    pub divisions: Option<u32>,
    pub height: f32,
    /// The part of the track behind the thumb.
    pub active: Color,
    /// The part ahead of it.
    pub inactive: Color,
    pub thumb: Color,
    /// How far the press has faded in, `0.0 ..= 1.0`. A plain field rather than
    /// a `Cell`: the press is tracked by a
    /// [`Pressable`](vieww_widget::Pressable) above, which rebuilds, so this
    /// arrives the same way the value does.
    pub press: f32,
    /// How thick the track is, and how big the thumb.
    ///
    /// Fields rather than the constants they default to, because an iOS slider
    /// and an Android one are visibly different objects: iOS is a thin track
    /// under a large white knob with a ring, Android is a thicker track under
    /// a smaller knob in the accent colour. The widget above reads
    /// `ThemeData::platform` and passes the two numbers down; the constants
    /// stay as the defaults everything that does not care still gets.
    pub track_thickness: f32,
    pub thumb_radius: f32,
    /// A ring drawn around the thumb, for a knob whose fill is the surface
    /// colour and would otherwise vanish against a light background.
    pub thumb_ring: Option<Color>,
    on_changed: Option<Handler<f32>>,
    /// The width layout arrived at, which is what a drag is measured against.
    ///
    /// A `Cell` because gestures arrive through `&self` — the same reason
    /// `RenderEditableText` keeps its drag anchor in one.
    width: Cell<f32>,
}

impl RenderSlider {
    #[must_use]
    pub fn new(value: f32, min: f32, max: f32) -> Self {
        Self {
            value,
            min,
            max,
            divisions: None,
            height: 48.0,
            active: Color::BLACK,
            inactive: Color::BLACK.with_alpha(0x40),
            thumb: Color::BLACK,
            press: 0.0,
            track_thickness: TRACK_THICKNESS,
            thumb_radius: THUMB_RADIUS,
            thumb_ring: None,
            on_changed: None,
            width: Cell::new(0.0),
        }
    }

    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    #[must_use]
    pub const fn divisions(mut self, divisions: Option<u32>) -> Self {
        self.divisions = divisions;
        self
    }

    #[must_use]
    pub const fn colors(mut self, active: Color, inactive: Color, thumb: Color) -> Self {
        self.active = active;
        self.inactive = inactive;
        self.thumb = thumb;
        self
    }

    /// How far the press has faded in.
    #[must_use]
    pub const fn press(mut self, press: f32) -> Self {
        self.press = press;
        self
    }

    /// Called with the value a drag or a tap asks for. Without it the slider is
    /// disabled and registers no recogniser.
    #[must_use]
    pub fn on_changed(mut self, handler: Handler<f32>) -> Self {
        self.on_changed = Some(handler);
        self
    }

    /// The track's thickness and the thumb's radius, with an optional ring
    /// behind the thumb. See the fields.
    #[must_use]
    pub const fn geometry(mut self, track: f32, thumb: f32, ring: Option<Color>) -> Self {
        self.track_thickness = track;
        self.thumb_radius = thumb;
        self.thumb_ring = ring;
        self
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.on_changed.is_some()
    }

    /// Where the current value sits on the track, as 0.0 ..= 1.0.
    #[must_use]
    pub fn fraction(&self) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            // A range with no width has exactly one value, and it is at the
            // start. Dividing by it would be a NaN thumb position, which paints
            // nothing and hit tests as nowhere.
            return 0.0;
        }
        ((self.value - self.min) / span).clamp(0.0, 1.0)
    }

    /// The length the thumb's centre travels: the width less a radius at each
    /// end, so the thumb stays inside the box at both extremes.
    fn travel(&self) -> f32 {
        (self.width.get() - self.thumb_radius * 2.0).max(0.0)
    }

    /// The value a point at `dx` from the left edge asks for.
    fn value_at(&self, dx: f32) -> f32 {
        let travel = self.travel();
        let fraction = if travel <= 0.0 {
            0.0
        } else {
            ((dx - self.thumb_radius) / travel).clamp(0.0, 1.0)
        };
        self.snap(self.min + fraction * (self.max - self.min))
    }

    /// `value` rounded to the nearest division, if there are any.
    ///
    /// # The zero-width guard
    ///
    /// [`fraction`](Self::fraction) has always refused to divide by a span of
    /// zero, and this — its sibling, doing the same division one step further
    /// along — did not. A slider declared `min == max` (which a range bound to
    /// data can legitimately become: one row in the table, one day in the
    /// filter) gives `step == 0.0`, and `(value - min) / 0.0` is `0.0 / 0.0`,
    /// which is **NaN**, not infinity. NaN then survives `clamp` — `f32::clamp`
    /// returns NaN for a NaN input rather than a bound — so it reaches the
    /// value the caller stores, the thumb position, and the change handler.
    ///
    /// A degenerate range has exactly one value in it and it is `min`, which is
    /// the same answer `fraction` gives for the same reason.
    fn snap(&self, value: f32) -> f32 {
        let span = self.max - self.min;
        if span.abs() < f32::EPSILON {
            return self.min;
        }
        match self.divisions {
            Some(divisions) if divisions > 0 => {
                let step = span / divisions as f32;
                let steps = ((value - self.min) / step).round();
                (self.min + steps * step).clamp(self.min, self.max)
            }
            _ => value.clamp(self.min, self.max),
        }
    }

    fn report(&self, value: f32) {
        if let Some(handler) = &self.on_changed {
            // Unconditionally, even when the value has not changed: the handler
            // writes a signal, and a signal that is set to what it already holds
            // does not mark anything pending. Filtering here would instead mean
            // deciding what "changed" means for a float.
            handler(value);
        }
    }

    /// The track's rectangle, centred vertically in `bounds`.
    fn track_rect(&self, bounds: Rect) -> Rect {
        let middle = bounds.top + bounds.height() / 2.0;
        Rect::new(
            bounds.left,
            middle - self.track_thickness / 2.0,
            bounds.right,
            middle + self.track_thickness / 2.0,
        )
    }

    /// The thumb's centre, in the same coordinates as `bounds`.
    fn thumb_center(&self, bounds: Rect) -> Offset {
        Offset::new(
            bounds.left + self.thumb_radius + self.fraction() * self.travel(),
            bounds.top + bounds.height() / 2.0,
        )
    }
}

impl fmt::Debug for RenderSlider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderSlider")
            .field("value", &self.value)
            .field("range", &(self.min, self.max))
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

impl RenderObject for RenderSlider {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let width = if constraints.has_bounded_width() {
            constraints.max_width
        } else {
            DEFAULT_WIDTH
        };
        let size = constraints.constrain(Size::new(width, self.height));
        self.width.set(size.width);
        size
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        let track = self.track_rect(bounds);
        let center = self.thumb_center(bounds);
        let radius = self.track_thickness / 2.0;

        // Inactive first, across the whole width, then the active part over it.
        // Two stadiums rather than one split in two, so the rounded ends are
        // right at both extremes of the value.
        ctx.canvas().fill_rrect(track, radius, self.inactive.into());
        if center.dx > track.left {
            let active = Rect::new(track.left, track.top, center.dx, track.bottom);
            ctx.canvas().fill_rrect(active, radius, self.active.into());
        }

        // Under the thumb, so the thumb stays the thing the eye follows. It
        // grows with the press as well as fading in — a halo that only fades
        // reads as the thumb going out of focus rather than as a response.
        if self.press > 0.0 {
            let halo_radius = self.thumb_radius * HALO_SCALE;
            let radius = self.thumb_radius + (halo_radius - self.thumb_radius) * self.press;
            let halo = Rect::new(
                center.dx - radius,
                center.dy - radius,
                center.dx + radius,
                center.dy + radius,
            );
            let alpha = (f32::from(vieww_widget::PRESSED_ALPHA) * self.press)
                .round()
                .clamp(0.0, 255.0) as u8;
            let wash = self.thumb.with_alpha(alpha);
            ctx.canvas()
                .fill_path(&Path::rounded_rect(halo, radius), wash.into());
        }

        let thumb = Rect::new(
            center.dx - self.thumb_radius,
            center.dy - self.thumb_radius,
            center.dx + self.thumb_radius,
            center.dy + self.thumb_radius,
        );
        // **The ring, and why it is under the fill rather than over it.** An
        // iOS knob is the surface colour, which on a light background is very
        // nearly the background: without an edge it disappears and the slider
        // looks like a track with a gap in it. Drawing a slightly larger disc
        // behind the knob is the cheapest edge there is, and unlike a stroke it
        // cannot be clipped away by the fill that follows.
        if let Some(ring) = self.thumb_ring {
            let outer = self.thumb_radius + 1.0;
            let edge = Rect::new(
                center.dx - outer,
                center.dy - outer,
                center.dx + outer,
                center.dy + outer,
            );
            ctx.canvas()
                .fill_path(&Path::rounded_rect(edge, outer), ring.into());
        }
        ctx.canvas().fill_path(
            &Path::rounded_rect(thumb, self.thumb_radius),
            self.thumb.into(),
        );
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // The whole strip, not the track and not the thumb: a finger aiming at a
        // 4px line will miss it, and a slider that only responds to a direct hit
        // on its thumb is the classic unusable slider.
        self.is_enabled()
    }

    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        if !self.is_enabled() {
            return Vec::new();
        }
        // Horizontal only, so a slider inside a vertical list does not eat the
        // scroll. Tap first, matching every other object here: a press that
        // resolves nothing should become a tap and move the thumb.
        vec![
            Box::new(TapRecognizer::new()),
            Box::new(DragRecognizer::along(vieww_foundation::Axis::Horizontal)),
        ]
    }

    fn handle_gesture(&self, gesture: &Recognized, _local: Offset) {
        // The thumb goes where the finger is, for a drag as well as a tap —
        // rather than moving by the drag's delta from wherever it was. Absolute
        // is what makes a tap and a drag the same arithmetic, and it means a
        // drag that started on the track does not have to be dragged the length
        // of the track to reach the end.
        match gesture {
            Recognized::Tap(details) => self.report(self.value_at(details.local.dx)),
            Recognized::DragStart(details) | Recognized::DragUpdate(details) => {
                self.report(self.value_at(details.local.dx));
            }
            _ => {}
        }
    }

    fn handle_semantic_action(&self, action: SemanticAction, _size: Size) -> bool {
        if !self.is_enabled() {
            return false;
        }
        // A tenth of the range, or one division where the slider has them —
        // stepping by a pixel would make a screen reader user press the key a
        // hundred times to cross a volume slider.
        let step = match self.divisions {
            Some(divisions) if divisions > 0 => (self.max - self.min) / divisions as f32,
            _ => (self.max - self.min) / 10.0,
        };
        let next = match action {
            SemanticAction::Increment => self.value + step,
            SemanticAction::Decrement => self.value - step,
            _ => return false,
        };
        self.report(next.clamp(self.min, self.max));
        true
    }

    fn semantics(&self) -> Option<Semantics> {
        // The number, as a string, because that is all `Semantics` carries.
        // AccessKit can take a real numeric value with a minimum, a maximum and
        // a step, which is what lets a screen reader offer "increment" — that
        // needs actions, which the tree does not have yet.
        Some(Semantics::new(Role::Slider).with_value(format!("{}", self.value)))
    }

    /// Its height outright; across, the length it takes when nothing stops it.
    ///
    /// The two axes are genuinely different questions here and neither of them
    /// is `None`.
    ///
    /// **Height** is [`height`](Self::height) and nothing else: `layout` uses
    /// that field whatever it is given, and the field is a finger's worth of
    /// touch target rather than anything derived from the content. Minimum and
    /// maximum coincide — a slider has no slack in the direction it does not
    /// run.
    ///
    /// **The maximum width** is `DEFAULT_WIDTH`, which is exactly what
    /// `layout` takes when the width is unbounded. That is the same question an
    /// intrinsic maximum asks — "how much would you take if nobody stopped you"
    /// — so answering with any other number would mean an `IntrinsicWidth` above
    /// a slider reported one width and then watched it lay out at another. It is
    /// a chosen constant rather than a measurement because a slider has no
    /// content to measure: it is a track, and a track is as long as it is
    /// allowed to be.
    ///
    /// **The minimum width** is the thumb's diameter, which is the width below
    /// which the control genuinely overflows rather than merely getting short.
    /// `travel` floors at zero, so at any narrower width the
    /// thumb is centred a radius in from the left with nothing left to travel
    /// and its right half hangs outside the box. Reporting `DEFAULT_WIDTH` here
    /// instead would be a lie in the other direction — sliders live in narrow
    /// columns all the time and a minimum of two hundred would force one to
    /// overflow its parent.
    ///
    /// The value, the divisions and the press are not consulted: none of them
    /// moves the box, which is what `layout_differs` returning `false`
    /// unconditionally already asserts, and an answer that moved with the value
    /// would go stale on every frame of a drag.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        use crate::Extremum;

        Some(match (query.axis, query.extremum) {
            (Axis::Horizontal, Extremum::Max) => DEFAULT_WIDTH,
            (Axis::Horizontal, Extremum::Min) => self.thumb_radius * 2.0,
            (Axis::Vertical, _) => self.height,
        })
    }

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // A slider's size comes from its constraints and its height, and the
        // height only changes with the theme. The *value* moves pixels but not
        // geometry, and relaying out for it would be a relayout per frame of
        // every drag.
        false
    }

    fn adopt_layout_cache(&mut self, old: &dyn RenderObject) {
        // Without this, the rebuild that a drag causes hands down a fresh object
        // whose width is zero, and — because `layout_differs` is false — nothing
        // lays it out again. The next drag update would then divide by a travel
        // of zero and pin the value to the start. Same lesson as the paragraph
        // cache in `RenderEditableText`.
        let any: &dyn std::any::Any = old;
        if let Some(old) = any.downcast_ref::<Self>() {
            self.width.set(old.width.get());
        }
    }

    fn debug_name(&self) -> &'static str {
        "RenderSlider"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use vieww_foundation::{DragDetails, TapDetails};
    use vieww_paint::Scene;

    use super::*;

    const WIDTH: f32 = 220.0;
    /// 220 wide, less a 10px radius at each end.
    const TRAVEL: f32 = WIDTH - THUMB_RADIUS * 2.0;

    fn laid_out(slider: RenderSlider) -> RenderSlider {
        slider.width.set(WIDTH);
        slider
    }

    fn reporting() -> (RenderSlider, Rc<RefCell<Vec<f32>>>) {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let slider = laid_out(
            RenderSlider::new(0.0, 0.0, 100.0)
                .on_changed(Rc::new(move |value| sink.borrow_mut().push(value))),
        );
        (slider, seen)
    }

    fn tap_at(dx: f32) -> Recognized {
        Recognized::Tap(TapDetails::at(Offset::new(dx, 0.0)))
    }

    /// **The regression test for the NaN the sibling helper already guarded
    /// against.**
    ///
    /// A slider whose minimum equals its maximum is degenerate but reachable —
    /// a range bound to a filtered dataset becomes one when the filter leaves a
    /// single row. `snap` divided by that zero-width span, and `0.0 / 0.0` is
    /// NaN rather than infinity, so `clamp` passed it straight through to the
    /// change handler and the thumb position.
    #[test]
    fn a_range_with_no_width_snaps_to_its_one_value_rather_than_nan() {
        for divisions in [None, Some(1), Some(4), Some(10)] {
            let mut slider = RenderSlider::new(7.0, 7.0, 7.0);
            slider.divisions = divisions;
            let slider = laid_out(slider);

            for dx in [0.0, THUMB_RADIUS, WIDTH / 2.0, WIDTH] {
                let got = slider.value_at(dx);
                assert!(
                    got.is_finite(),
                    "divisions {divisions:?} at {dx}: got {got}, which is not a number"
                );
                assert!(
                    (got - 7.0).abs() < f32::EPSILON,
                    "divisions {divisions:?} at {dx}: a one-value range has one value"
                );
            }
            assert_eq!(slider.fraction(), 0.0);
        }
    }

    /// The guard must not have changed what a real range does.
    #[test]
    fn snapping_a_real_range_still_lands_on_its_divisions() {
        let mut slider = RenderSlider::new(0.0, 0.0, 100.0);
        slider.divisions = Some(4);
        let slider = laid_out(slider);
        assert_eq!(slider.value_at(THUMB_RADIUS), 0.0);
        assert_eq!(slider.value_at(THUMB_RADIUS + TRAVEL), 100.0);
        assert_eq!(slider.value_at(THUMB_RADIUS + TRAVEL * 0.51), 50.0);
        assert_eq!(slider.value_at(THUMB_RADIUS + TRAVEL * 0.26), 25.0);
    }

    #[test]
    fn the_ends_of_the_track_are_the_ends_of_the_range() {
        let (slider, seen) = reporting();

        slider.handle_gesture(&tap_at(THUMB_RADIUS), Offset::ZERO);
        slider.handle_gesture(&tap_at(THUMB_RADIUS + TRAVEL), Offset::ZERO);
        // Past the end, which a finger routinely is.
        slider.handle_gesture(&tap_at(WIDTH + 50.0), Offset::ZERO);

        assert_eq!(*seen.borrow(), vec![0.0, 100.0, 100.0]);
    }

    #[test]
    fn the_thumb_stays_inside_the_box_at_both_extremes() {
        let bounds = Rect::new(0.0, 0.0, WIDTH, 48.0);
        let at_start = laid_out(RenderSlider::new(0.0, 0.0, 1.0)).thumb_center(bounds);
        let at_end = laid_out(RenderSlider::new(1.0, 0.0, 1.0)).thumb_center(bounds);

        assert!((at_start.dx - THUMB_RADIUS).abs() < 1e-4, "{at_start:?}");
        assert!(
            (at_end.dx - (WIDTH - THUMB_RADIUS)).abs() < 1e-4,
            "a thumb centred on the edge is half outside it: {at_end:?}"
        );
    }

    #[test]
    fn divisions_snap_to_the_nearest_step() {
        let (slider, seen) = reporting();
        let slider = slider.divisions(Some(4)); // 0, 25, 50, 75, 100

        slider.handle_gesture(&tap_at(THUMB_RADIUS + TRAVEL * 0.3), Offset::ZERO);
        slider.handle_gesture(&tap_at(THUMB_RADIUS + TRAVEL * 0.62), Offset::ZERO);

        assert_eq!(*seen.borrow(), vec![25.0, 50.0]);
    }

    #[test]
    fn a_drag_puts_the_thumb_where_the_finger_is() {
        let (slider, seen) = reporting();
        let details = DragDetails {
            position: Offset::new(THUMB_RADIUS + TRAVEL / 2.0, 0.0),
            local: Offset::new(THUMB_RADIUS + TRAVEL / 2.0, 0.0),
            delta: Offset::new(18.0, 0.0),
            velocity: Offset::ZERO,
            timestamp: std::time::Duration::ZERO,
            modifiers: vieww_foundation::Modifiers::NONE,
        };
        slider.handle_gesture(&Recognized::DragUpdate(details), Offset::ZERO);

        assert_eq!(
            *seen.borrow(),
            vec![50.0],
            "absolute, not relative to delta"
        );
    }

    #[test]
    fn a_disabled_slider_registers_nothing_and_absorbs_nothing() {
        let slider = laid_out(RenderSlider::new(50.0, 0.0, 100.0));
        assert!(slider.gesture_recognizers().is_empty());
        assert!(!slider.hit_test_self(Offset::ZERO, Size::new(WIDTH, 48.0)));
    }

    #[test]
    fn a_range_with_no_width_does_not_produce_a_nan_thumb() {
        let slider = laid_out(RenderSlider::new(5.0, 5.0, 5.0));
        assert_eq!(slider.fraction(), 0.0);
        assert!(slider
            .thumb_center(Rect::new(0.0, 0.0, WIDTH, 48.0))
            .dx
            .is_finite());
    }

    #[test]
    fn a_rebuild_keeps_the_width_a_drag_is_measured_against() {
        let (old, _) = reporting();
        let mut fresh = RenderSlider::new(50.0, 0.0, 100.0);
        assert_eq!(fresh.width.get(), 0.0);

        fresh.adopt_layout_cache(&old);
        assert_eq!(
            fresh.width.get(),
            WIDTH,
            "a drag rebuilds the slider every frame, and nothing lays it out again"
        );
    }

    #[test]
    fn the_track_is_painted_behind_the_thumb() {
        let slider = laid_out(RenderSlider::new(50.0, 0.0, 100.0));
        let mut scene = Scene::new();
        let mut ctx = PaintCtx {
            canvas: &mut scene,
            origin: Offset::ZERO,
            size: Size::new(WIDTH, 48.0),
            dpr: 1.0,
        };
        slider.paint(&mut ctx);

        assert_eq!(
            scene.commands().len(),
            3,
            "inactive track, active track, thumb"
        );
    }

    // -------------------------------------------------------------- intrinsics

    fn intrinsic(slider: RenderSlider, query: crate::IntrinsicQuery) -> Option<f32> {
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(slider));
        tree.intrinsic(id, query)
    }

    #[test]
    fn a_slider_is_as_tall_as_the_touch_target_it_declares() {
        // Both extrema, because there is no slack across the track: the height
        // is a finger, not something derived from content that could compress.
        for query in [
            crate::IntrinsicQuery::min_height(),
            crate::IntrinsicQuery::max_height(),
        ] {
            assert_eq!(
                intrinsic(RenderSlider::new(0.0, 0.0, 100.0).height(56.0), query),
                Some(56.0),
                "{query:?}"
            );
        }
    }

    #[test]
    fn the_maximum_width_is_the_length_an_unbounded_layout_takes() {
        // The two have to agree, or an `IntrinsicWidth` reports one width and
        // then hands the slider constraints it lays out to a different one.
        let mut tree = crate::RenderTree::new();
        let id = tree.insert(None, Box::new(RenderSlider::new(0.0, 0.0, 100.0)));
        let laid = tree.layout(id, Constraints::UNBOUNDED);

        assert_eq!(
            intrinsic(
                RenderSlider::new(0.0, 0.0, 100.0),
                crate::IntrinsicQuery::max_width()
            ),
            Some(laid.width)
        );
    }

    #[test]
    fn the_minimum_width_is_a_thumb_and_not_the_default_length() {
        let min = intrinsic(
            RenderSlider::new(0.0, 0.0, 100.0),
            crate::IntrinsicQuery::min_width(),
        )
        .expect("measurable");

        assert_eq!(min, THUMB_RADIUS * 2.0);
        assert!(
            min < DEFAULT_WIDTH,
            "a minimum of the default length would make every slider in a narrow \
             column overflow its parent"
        );
    }

    #[test]
    fn the_thumb_fits_inside_a_box_of_the_minimum_width() {
        // What the minimum is *for*: at anything narrower the travel floors at
        // zero and the thumb's right half hangs outside the box.
        let min = intrinsic(
            RenderSlider::new(1.0, 0.0, 1.0),
            crate::IntrinsicQuery::min_width(),
        )
        .expect("measurable");

        let slider = RenderSlider::new(1.0, 0.0, 1.0);
        slider.width.set(min);
        let center = slider.thumb_center(Rect::new(0.0, 0.0, min, 48.0));

        assert!(
            center.dx + THUMB_RADIUS <= min + 1e-4,
            "the thumb at the far end reaches {} in a box of {min}",
            center.dx + THUMB_RADIUS
        );
    }

    #[test]
    fn the_value_and_the_press_do_not_move_the_intrinsic() {
        // A drag rebuilds this object every frame. An answer that moved with the
        // value would be a cached measurement invalidated sixty times a second,
        // and the contract asks for a pure function of the configuration.
        let query = crate::IntrinsicQuery::max_width();
        assert_eq!(
            intrinsic(RenderSlider::new(0.0, 0.0, 100.0), query),
            intrinsic(
                RenderSlider::new(73.0, 0.0, 100.0)
                    .divisions(Some(4))
                    .press(1.0),
                query
            )
        );
    }

    #[test]
    fn a_cross_extent_does_not_change_either_axis() {
        // Nothing about a slider reflows, so being told how much room the other
        // axis has cannot change what this one wants.
        assert_eq!(
            intrinsic(
                RenderSlider::new(0.0, 0.0, 100.0),
                crate::IntrinsicQuery::max_width().across(20.0)
            ),
            Some(DEFAULT_WIDTH)
        );
        assert_eq!(
            intrinsic(
                RenderSlider::new(0.0, 0.0, 100.0),
                crate::IntrinsicQuery::max_height().across(1000.0)
            ),
            Some(48.0)
        );
    }
}
