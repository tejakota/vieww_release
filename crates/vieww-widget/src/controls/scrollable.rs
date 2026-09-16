use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Axis, DragDetails, Key, Offset, ScrollEvent, TextDirection};

use crate::{
    widget_node_from, BuildContext, Directionality, GestureDetector, Handler, Inherited,
    LayoutBuilder, ScrollExtents, Viewport, Widget, WidgetKind, WidgetNode,
};

/// A drag as the scroll position needs to read it.
///
/// `ScrollPosition` works on a scalar from zero to a maximum and knows nothing
/// about reading direction — deliberately, because an offset is *a distance
/// into the content*, not a place on the screen. So the one place that knows
/// both the axis and the direction does the flip, and everything below it stays
/// direction-free. (Named without a link: `vieww-gestures` is not a dependency
/// of this crate, which is itself the point — the physics are that far away.)
///
/// Only `dx` is negated: `reversed` is set for a horizontal viewport and nothing
/// else, since no script this framework targets runs bottom to top.
fn as_read(details: DragDetails, reversed: bool) -> DragDetails {
    if !reversed {
        return details;
    }
    DragDetails {
        delta: Offset::new(-details.delta.dx, details.delta.dy),
        velocity: Offset::new(-details.velocity.dx, details.velocity.dy),
        ..details
    }
}

/// Where a scrollable currently is, published to everything inside it.
///
/// A [`ListView`](crate::ListView) reads this instead of being told the offset
/// and the window size twice — the scrollable above it already knows both, and
/// passing them down by hand through whatever sits between is how they get out
/// of step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollMetrics {
    pub axis: Axis,
    /// How far into the content the window has moved.
    pub offset: f32,
    /// The window's own length along the axis, or `None` before the first
    /// layout has measured it.
    pub viewport: Option<f32>,
}

impl ScrollMetrics {
    /// The range of content currently visible, given a window length.
    ///
    /// `None` until the viewport has been measured — the first frame of a
    /// scrollable that has never been laid out.
    #[must_use]
    pub fn visible(self) -> Option<(f32, f32)> {
        self.viewport
            .map(|viewport| (self.offset, self.offset + viewport))
    }
}

/// A window onto content longer than itself, that a drag moves.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Scrollable;
/// use std::rc::Rc;
///
/// # let offset = 0.0;
/// # let on_drag: Rc<dyn Fn(vieww_widget::foundation::DragDetails)> = Rc::new(|_| {});
/// let list = Scrollable::vertical(offset)
///     .on_drag(on_drag)
///     .child(Flex::column().children(children![Text::new("one"), Text::new("two")]));
/// ```
///
/// # Controlled, like everything else here
///
/// It is *told* where it is and reports the drags it receives; it does not hold
/// a position, run physics or fling. Those need state that survives a rebuild
/// and a way to mark the tree pending when a finger moves, which is a signal —
/// and signals live in the element layer, above this one
/// (`docs/DESIGN.md` §7). `vieww_element::ScrollController` is the piece that
/// owns all three and hands this widget its offset; this is the half that can be
/// described without one.
///
/// # What it does for you
///
/// The drag axis, so a vertical list inside a horizontal pager does not steal
/// sideways drags; the [`Viewport`] and its clipping; reporting
/// [`ScrollExtents`] back out of layout, which is the only way anything above
/// can learn how long the content is; and publishing [`ScrollMetrics`] to its
/// subtree so a virtualised list underneath knows what is on screen.
#[derive(Clone)]
pub struct Scrollable {
    axis: Axis,
    offset: f32,
    viewport: Option<f32>,
    on_drag: Option<Handler<DragDetails>>,
    on_drag_end: Option<Handler<DragDetails>>,
    on_extents: Option<Handler<ScrollExtents>>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Scrollable {
    #[must_use]
    pub const fn new(axis: Axis, offset: f32) -> Self {
        Self {
            axis,
            offset,
            viewport: None,
            on_drag: None,
            on_drag_end: None,
            on_extents: None,
            child: None,
            key: None,
        }
    }

    #[must_use]
    pub const fn vertical(offset: f32) -> Self {
        Self::new(Axis::Vertical, offset)
    }

    #[must_use]
    pub const fn horizontal(offset: f32) -> Self {
        Self::new(Axis::Horizontal, offset)
    }

    /// Declare the window's length along the axis, instead of measuring it.
    ///
    /// **Rarely needed.** The window is measured during layout and published in
    /// [`ScrollMetrics`] without being told — see the type docs — so a
    /// [`ListView`](crate::ListView) inside virtualises correctly with nothing
    /// declared. This is the fallback for the one case the measurement cannot
    /// answer: a scrollable whose own main axis is **unbounded**, where there is
    /// no window length to read.
    ///
    /// A declared length **wins over the measured one**, because it is an
    /// explicit statement and the one case that needs it is the one the
    /// measurement cannot answer. Declaring a length that disagrees with the
    /// laid-out window is a list building the wrong rows, so declare nothing
    /// unless the axis is genuinely unbounded.
    #[must_use]
    pub const fn viewport(mut self, extent: f32) -> Self {
        self.viewport = Some(extent);
        self
    }

    /// Called for the start and every update of a drag along the axis.
    ///
    /// One handler for both, because they must do the same thing:
    /// `DragStart` carries the movement that *earned* the drag — the slop the
    /// finger crossed before it was certain — and handling only the updates
    /// throws that away, which the list shows as lagging the finger by 18
    /// pixels on every touch.
    #[must_use]
    pub fn on_drag(mut self, handler: Handler<DragDetails>) -> Self {
        self.on_drag = Some(handler);
        self
    }

    /// Called when the finger lifts, carrying the release velocity a fling is
    /// simulated from.
    #[must_use]
    pub fn on_drag_end(mut self, handler: Handler<DragDetails>) -> Self {
        self.on_drag_end = Some(handler);
        self
    }

    /// Called from layout when the window's or the content's length changes.
    #[must_use]
    pub fn on_extents(mut self, handler: Handler<ScrollExtents>) -> Self {
        self.on_extents = Some(handler);
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn scroll_offset(&self) -> f32 {
        self.offset
    }

    /// What this scrollable publishes to its subtree.
    #[must_use]
    pub const fn metrics(&self) -> ScrollMetrics {
        ScrollMetrics {
            axis: self.axis,
            offset: self.offset,
            viewport: self.viewport,
        }
    }
}

impl Widget for Scrollable {
    fn debug_name(&self) -> &'static str {
        "Scrollable"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // The same condition the factory uses to anchor the viewport at the far
        // edge. Both read the one ambient direction, so they cannot disagree —
        // but they are two decisions, and a horizontal list that scrolled the
        // wrong way with the right anchor would look like a physics bug.
        let reversed =
            self.axis == Axis::Horizontal && Directionality::of(ctx) == TextDirection::Rtl;

        let mut viewport = Viewport::new(self.axis).offset(self.offset);
        if let Some(handler) = &self.on_extents {
            viewport = viewport.on_extents(Rc::clone(handler));
        }
        if let Some(child) = &self.child {
            viewport = viewport.child(child.clone());
        }

        // The metrics are published *inside* the gesture detector and outside
        // the viewport, so everything that can see the window can see where the
        // window is.
        //
        // # Why the window length is measured here rather than declared
        //
        // `ScrollMetrics::viewport` used to be whatever the caller passed to
        // [`Scrollable::viewport`], and nothing measured it despite the doc
        // saying something had. Nobody passes it — not one call site in this
        // repository's own examples did — so `visible()` was `None` for every
        // scrollable in the framework, and every `ListView` inside one fell back
        // to its "nothing has measured a window yet" guess of twelve rows.
        //
        // That guess is invisible until you scroll. The list builds rows 0..14
        // **forever**, the viewport translates them by the offset, and at about
        // four hundred points they leave the window and the list is blank while
        // still reporting its full height. Reported as "`examples/data`'s feed
        // goes blank when scrolled hard"; it was every list, not that one.
        //
        // The length is the *incoming* main-axis constraint: the window is what
        // the scrollable was given, not what it hands its child — a `Viewport`
        // deliberately gives its child an unbounded main axis, so measuring
        // inside it would read infinity. `LayoutBuilder` is what turns a
        // constraint into a build, and `FrameSink::layout` settles it inside the
        // frame that discovered it, so there is no lag to trade against.
        //
        // An unbounded main axis has no window to read and falls back to the
        // declared value, which is exactly what that setter is now for.
        let axis = self.axis;
        let offset = self.offset;
        let declared = self.viewport;
        let inner = LayoutBuilder::new(move |constraints| {
            let measured = match axis {
                Axis::Vertical => constraints.max_height,
                Axis::Horizontal => constraints.max_width,
            };
            Inherited::new(
                ScrollMetrics {
                    axis,
                    offset,
                    // A declared length wins. It is an explicit statement by the
                    // caller and there is exactly one case that needs it — an
                    // unbounded main axis, where `measured` is infinite and
                    // there is nothing to read. Preferring the measurement
                    // instead would also mean overriding it with
                    // `LayoutBuilder`'s *seed*, which is the surface size, in
                    // any tree built without a layout pass.
                    viewport: declared.or(measured.is_finite().then_some(measured)),
                },
                viewport.clone(),
            )
            .into()
        });

        // Each phase gets a closure that calls the shared handler. An
        // `Rc<dyn Fn(_)>` cannot be passed straight in — it derefs to something
        // callable but does not itself implement `Fn` — so the wrapper is the
        // cost of one handler serving both drag phases.
        let mut detector = GestureDetector::new().drag_axis(self.axis);
        if let Some(handler) = &self.on_drag {
            let start = Rc::clone(handler);
            let update = Rc::clone(handler);
            detector = detector
                .on_drag_start(move |details| start(as_read(details, reversed)))
                .on_drag_update(move |details| update(as_read(details, reversed)));
        }
        if let Some(handler) = &self.on_drag_end {
            let ended = Rc::clone(handler);
            // The velocity is flipped too, or a fling would be thrown the way
            // the finger moved rather than the way the content scrolls.
            detector = detector.on_drag_end(move |details| ended(as_read(details, reversed)));
        }

        // A wheel notch is reported to the same handler a drag is, as a
        // movement with no velocity: a scrollable that treated the two
        // separately would need two sets of clamping and two sets of overscroll,
        // and they would drift apart.
        //
        // # A notch is a whole gesture, and used to be reported as a third of one
        //
        // It is a press, a movement and a release, all at one instant. This used
        // to report only the movement, and **that is a bug rather than a
        // simplification**: under `Overscroll::Bounce` nothing retracts an
        // overscroll except a release, and there is no such thing as letting go
        // of a wheel. So a wheel that pushed a list past its edge left it there,
        // permanently, with no gesture available to bring it back.
        //
        // Invisible under `Overscroll::Clamp` — the platform default on a
        // desktop — because clamping leaves no overscroll to retract. It shows
        // the moment an application opts into `ScrollPhysics::ios()`, and it is
        // the reason a pull-to-refresh spinner could be left hanging open on
        // screen with nothing able to close it.
        if let Some(handler) = &self.on_drag {
            let axis = self.axis;
            let scrolled = Rc::clone(handler);
            let released = self.on_drag_end.as_ref().map(Rc::clone);
            detector = detector.on_scroll(move |event: ScrollEvent| {
                let along = match axis {
                    Axis::Vertical => Offset::new(0.0, event.delta.dy),
                    Axis::Horizontal => Offset::new(event.delta.dx, 0.0),
                };
                if along == Offset::ZERO {
                    // Sideways over a vertical list. Not ours, and saying
                    // nothing here is what lets it reach whatever is around us.
                    return;
                }
                let details = DragDetails {
                    position: event.position,
                    local: Offset::ZERO,
                    delta: along,
                    // A wheel has no fling. Reporting one would launch the list
                    // into a simulation the user never asked for by flicking.
                    velocity: Offset::ZERO,
                    // The event's, not `Duration::ZERO`. A release is only worth
                    // reporting if whatever receives it can start a spring on
                    // the same clock the frames run on; a simulation begun at
                    // time zero is already over, and snaps instead of settling.
                    timestamp: event.timestamp,
                    // A `ScrollEvent` carries none — a wheel notch is not a
                    // press and nothing downstream reads them off a scroll.
                    modifiers: vieww_foundation::Modifiers::NONE,
                };
                // Through the same flip as a finger. A wheel over a mirrored
                // list has to move it the way the same wheel moves an
                // unmirrored one, or the two input paths disagree.
                let details = as_read(details, reversed);
                scrolled(details);
                if let Some(released) = &released {
                    // Zero velocity, so this settles an overscroll rather than
                    // throwing the list onward — `ScrollPosition::fling` springs
                    // back when it is past an edge and does nothing when it is
                    // not, which is exactly the two cases here.
                    released(details);
                }
            });
        }

        detector.child(inner).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![
            ("axis", format!("{:?}", self.axis)),
            ("offset", self.offset.to_string()),
        ];
        if let Some(viewport) = self.viewport {
            props.push(("viewport", viewport.to_string()));
        }
        props
    }
}

impl fmt::Debug for Scrollable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scrollable")
            .field("axis", &self.axis)
            .field("offset", &self.offset)
            .field("viewport", &self.viewport)
            .field("draggable", &self.on_drag.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Scrollable);

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use crate::{inflate, GestureDetector, Text};

    use super::*;

    /// A wheel notch of `delta`, at an unremarkable moment.
    fn notch(delta: Offset) -> ScrollEvent {
        ScrollEvent::new(Offset::ZERO, delta, Duration::from_millis(1000))
    }

    #[test]
    fn a_drag_reaches_one_handler_from_both_the_start_and_the_updates() {
        let scrollable = Scrollable::vertical(0.0)
            .on_drag(Rc::new(|_| {}))
            .child(Text::new("content"));
        let tree = inflate(scrollable);
        let detector = tree.find("GestureDetector").expect("draggable");

        assert_eq!(
            detector.property("gestures"),
            Some("scroll, drag"),
            "a drag and a wheel, and deliberately no tap — a tap recogniser \
             here would take taps from the rows inside"
        );
    }

    #[test]
    fn a_wheel_moves_the_list_the_way_a_drag_would() {
        // One handler for both, so there is one set of clamping and one set of
        // overscroll rather than two that drift apart.
        let seen: Rc<RefCell<Vec<DragDetails>>> = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        let scrollable = Scrollable::vertical(0.0)
            .on_drag(Rc::new(move |details| record.borrow_mut().push(details)))
            .child(Text::new("content"));

        let handlers = scrollable.build(&BuildContext::root());
        let detector = handlers
            .downcast_ref::<GestureDetector>()
            .expect("a detector at the root");
        let scroll = detector
            .handlers()
            .on_scroll
            .clone()
            .expect("a wheel handler");

        scroll(notch(Offset::new(0.0, -40.0)));
        let reported = seen.borrow();
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].delta, Offset::new(0.0, -40.0));
        assert_eq!(
            reported[0].velocity,
            Offset::ZERO,
            "a wheel has no fling; reporting one would launch the list into a \
             simulation nobody asked for"
        );
    }

    #[test]
    fn a_sideways_wheel_over_a_vertical_list_is_left_for_something_else() {
        let seen: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let record = Rc::clone(&seen);
        let scrollable = Scrollable::vertical(0.0)
            .on_drag(Rc::new(move |_| *record.borrow_mut() += 1))
            .child(Text::new("content"));

        let built = scrollable.build(&BuildContext::root());
        let scroll = built
            .downcast_ref::<GestureDetector>()
            .expect("a detector")
            .handlers()
            .on_scroll
            .clone()
            .expect("a wheel handler");

        scroll(notch(Offset::new(-40.0, 0.0)));
        assert_eq!(*seen.borrow(), 0);
    }

    #[test]
    fn a_wheel_notch_reports_a_release_because_nothing_else_ever_will() {
        // **The bug this is the regression test for.** A notch is a press, a
        // movement and a release at one instant, and only the movement was
        // reported. Under `Overscroll::Bounce` a release is the only thing that
        // retracts an overscroll, and there is no such thing as letting go of a
        // wheel — so a wheel that pushed a list past its edge left it there with
        // no gesture able to bring it back.
        let ends: Rc<RefCell<Vec<DragDetails>>> = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&ends);
        let scrollable = Scrollable::vertical(0.0)
            .on_drag(Rc::new(|_| {}))
            .on_drag_end(Rc::new(move |details| record.borrow_mut().push(details)))
            .child(Text::new("content"));

        let handlers = scrollable.build(&BuildContext::root());
        let scroll = handlers
            .downcast_ref::<GestureDetector>()
            .expect("a detector at the root")
            .handlers()
            .on_scroll
            .clone()
            .expect("a wheel handler");

        scroll(ScrollEvent::new(
            Offset::ZERO,
            Offset::new(0.0, -40.0),
            Duration::from_millis(1200),
        ));

        let reported = ends.borrow();
        assert_eq!(reported.len(), 1, "the notch let go of itself");
        assert_eq!(
            reported[0].velocity,
            Offset::ZERO,
            "and settles rather than throwing the list onward"
        );
        assert_eq!(
            reported[0].timestamp,
            Duration::from_millis(1200),
            "on the frame clock, or the spring it starts is already over and snaps"
        );
    }

    #[test]
    fn a_sideways_wheel_reports_no_release_either() {
        // It was never ours, so neither half of the gesture is.
        let ends: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let record = Rc::clone(&ends);
        let scrollable = Scrollable::vertical(0.0)
            .on_drag(Rc::new(|_| {}))
            .on_drag_end(Rc::new(move |_| *record.borrow_mut() += 1))
            .child(Text::new("content"));

        let handlers = scrollable.build(&BuildContext::root());
        let scroll = handlers
            .downcast_ref::<GestureDetector>()
            .expect("a detector")
            .handlers()
            .on_scroll
            .clone()
            .expect("a wheel handler");

        scroll(notch(Offset::new(-40.0, 0.0)));
        assert_eq!(*ends.borrow(), 0);
    }

    #[test]
    fn a_scrollable_with_no_handler_registers_no_recogniser() {
        let tree = inflate(Scrollable::vertical(0.0).child(Text::new("content")));
        let detector = tree.find("GestureDetector").expect("still present");
        assert_eq!(detector.property("gestures"), Some(""));
    }

    #[test]
    fn the_metrics_reach_the_subtree() {
        let tree = inflate(
            Scrollable::vertical(120.0)
                .viewport(300.0)
                .child(Text::new("content")),
        );
        assert!(
            tree.find("Inherited<ScrollMetrics>").is_some(),
            "{}",
            crate::debug_tree(Scrollable::vertical(120.0).child(Text::new("x")))
        );
    }

    #[test]
    fn visible_is_unknown_until_something_has_been_measured() {
        let unmeasured = Scrollable::vertical(120.0).metrics();
        assert_eq!(unmeasured.visible(), None);

        let measured = Scrollable::vertical(120.0).viewport(300.0).metrics();
        assert_eq!(measured.visible(), Some((120.0, 420.0)));
    }
}
