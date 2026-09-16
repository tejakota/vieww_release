//! Shared-element transitions: one thing that appears on two screens, moving
//! between them rather than being replaced.
//!
//! Every toolkit has a name for this — hero transition, shared-element
//! transition, view transition. The idea is the same everywhere and so is the
//! mechanism: the element exists twice, in two trees, at two rectangles; during
//! the change a *third* copy flies between them while both originals hide.
//!
//! # Why the route transition could not do this
//!
//! [`RouteTransition`](vieww_widget::RouteTransition) animates whole screens.
//! Its unit is a screen because that is all it has: a screen is a `WidgetNode`
//! and a `WidgetNode` has no position — position is assigned by layout, one
//! phase later, and is not knowable when the transition is built.
//!
//! So this needs a different input: the **laid-out rectangle** of a tagged
//! element, which only exists after paint. [`SharedElement`] captures it
//! through the same route [`Measured`] uses for
//! anchoring a popup, and stashes it in a [`SharedRegistry`] the transition
//! reads on the next frame.
//!
//! # The consequence: capture, then fly
//!
//! A flight cannot start on the same frame the tap happened, because the
//! destination screen has not been laid out yet — nobody knows where the
//! element is going. The sequence is:
//!
//! 1. **Frame N** — screen A is on stage. Its `SharedElement`s have reported
//!    their rectangles; [`SharedRegistry::snapshot`] is screen A's geometry.
//! 2. **Frame N+1** — screen B is mounted and painted, ordinarily off-stage or
//!    at zero opacity. Its `SharedElement`s report; that snapshot is the
//!    destination.
//! 3. **Frames N+2 onward** — [`SharedFlight`] interpolates between the two and
//!    the copies fly.
//!
//! One frame of latency between the tap and the movement, which is the same
//! frame every implementation of this spends and for the same reason. It is
//! written down here because the alternative — guessing the destination — is
//! what makes a shared-element transition land in the wrong place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vieww_foundation::{Offset, Rect, Transform};
use vieww_widget::{
    widget_node_from, BuildContext, Handler, Measured, Positioned, SizedBox, Stack, Widget,
    WidgetKind, WidgetNode,
};

/// Names the same conceptual element on two screens.
///
/// A string rather than a type id, because the two screens are usually two
/// different widgets — a grid cell and a detail header — and what they share is
/// an *identity in the data*, not a type. The tag is normally derived from
/// whatever the element is showing: `format!("photo-{id}")`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SharedTag(String);

impl SharedTag {
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SharedTag {
    fn from(tag: &str) -> Self {
        Self::new(tag)
    }
}

impl From<String> for SharedTag {
    fn from(tag: String) -> Self {
        Self::new(tag)
    }
}

/// Where every tagged element on one screen ended up.
///
/// A value, taken with [`SharedRegistry::snapshot`], because the registry keeps
/// changing as screens mount and unmount and a flight has to hold still.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SharedGeometry {
    rects: HashMap<SharedTag, Rect>,
}

impl SharedGeometry {
    /// Where `tag` was, if it was on this screen at all.
    #[must_use]
    pub fn get(&self, tag: &SharedTag) -> Option<Rect> {
        self.rects.get(tag).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// Every tag recorded, sorted, so a caller iterating gets a stable order.
    ///
    /// Sorted rather than in insertion order because the underlying map has
    /// none, and a transition whose flights change z-order frame to frame
    /// flickers.
    #[must_use]
    pub fn tags(&self) -> Vec<SharedTag> {
        let mut tags: Vec<SharedTag> = self.rects.keys().cloned().collect();
        tags.sort();
        tags
    }
}

/// Collects the rectangles of the [`SharedElement`]s currently on screen.
///
/// Cloning gives another handle to the same registry — the widgets writing into
/// it and the transition reading it hold one each.
#[derive(Debug, Clone, Default)]
pub struct SharedRegistry {
    inner: Rc<RefCell<HashMap<SharedTag, Rect>>>,
}

impl SharedRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record where `tag` is. Called by [`SharedElement`] from paint.
    pub fn record(&self, tag: SharedTag, rect: Rect) {
        self.inner.borrow_mut().insert(tag, rect);
    }

    /// Everything recorded so far, as a value that will not change underneath a
    /// flight.
    #[must_use]
    pub fn snapshot(&self) -> SharedGeometry {
        SharedGeometry {
            rects: self.inner.borrow().clone(),
        }
    }

    /// Forget everything.
    ///
    /// Called between screens: a tag left over from a screen that is gone would
    /// pair with the same tag on the next one and fly from a stale rectangle.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
    }

    /// How many elements have reported.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Marks its child as the same element as the one tagged the same way on
/// another screen.
///
/// ```
/// use vieww_element::{SharedElement, SharedRegistry};
/// use vieww_widget::prelude::*;
///
/// # fn example(registry: &SharedRegistry, id: u32) -> WidgetNode {
/// SharedElement::new(format!("photo-{id}"), registry)
///     .child(Container::new().color(Color::BLUE))
///     .into()
/// # }
/// ```
///
/// Layout, painting and hit testing are untouched — this reports a rectangle
/// and nothing else. It is [`Measured`] with a name attached, and the
/// documentation there explains why the rectangle arrives one frame after the
/// layout that produced it.
///
/// # Hiding the original during a flight
///
/// While a copy is flying, the two originals must not also be on screen or the
/// element appears three times. [`hidden`](Self::hidden) is how a screen says
/// so, and it is a parameter rather than something this works out for itself
/// because only the transition knows a flight is running.
#[derive(Clone)]
pub struct SharedElement {
    tag: SharedTag,
    registry: SharedRegistry,
    child: Option<WidgetNode>,
    hidden: bool,
}

impl SharedElement {
    #[must_use]
    pub fn new(tag: impl Into<SharedTag>, registry: &SharedRegistry) -> Self {
        Self {
            tag: tag.into(),
            registry: registry.clone(),
            child: None,
            hidden: false,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Take the original off screen while its copy is in flight.
    ///
    /// **It still reports its rectangle.** A hidden element that stopped
    /// measuring would break the flight that hid it — the destination geometry
    /// is captured from a screen that is, at that moment, hidden. So this
    /// removes the ink and keeps the box: the child is laid out and measured as
    /// usual, and simply not painted.
    #[must_use]
    pub const fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    #[must_use]
    pub fn tag(&self) -> &SharedTag {
        &self.tag
    }
}

impl std::fmt::Debug for SharedElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedElement")
            .field("tag", &self.tag.0)
            .field("hidden", &self.hidden)
            .finish_non_exhaustive()
    }
}

impl Widget for SharedElement {
    fn debug_name(&self) -> &'static str {
        "SharedElement"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let registry = self.registry.clone();
        let tag = self.tag.clone();
        let report: Handler<Rect> = Rc::new(move |rect| registry.record(tag.clone(), rect));

        let child = self
            .child
            .clone()
            .unwrap_or_else(|| SizedBox::shrink().into());

        // `Offstage` rather than `Opacity(0)`: opacity zero still records the
        // commands and still reports damage every time the hidden subtree
        // changes, which during a flight is every frame.
        let child: WidgetNode = if self.hidden {
            vieww_widget::Offstage::new(true).child(child).into()
        } else {
            child
        };

        Measured::new().on_measured(report).child(child).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("tag", self.tag.0.clone()),
            ("hidden", self.hidden.to_string()),
        ]
    }
}

widget_node_from!(SharedElement);

/// One element on its way from where it was to where it is going.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flight {
    /// Where it started, in global coordinates.
    pub from: Rect,
    /// Where it is going.
    pub to: Rect,
}

impl Flight {
    /// The rectangle at `t`, linearly between the two.
    ///
    /// Interpolating the *rectangle* rather than a position and a scale is what
    /// makes a flight between two differently-shaped boxes land exactly on the
    /// destination. A position-plus-uniform-scale flight cannot, because two
    /// rectangles of different aspect ratios are not related by one.
    #[must_use]
    pub fn rect_at(&self, t: f32) -> Rect {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Rect::new(
            mix(self.from.left, self.to.left),
            mix(self.from.top, self.to.top),
            mix(self.from.right, self.to.right),
            mix(self.from.bottom, self.to.bottom),
        )
    }

    /// The scale to apply to a child built at the **destination** size, so that
    /// it fills [`rect_at`](Self::rect_at).
    ///
    /// The child is built once, at one size, and scaled — rather than rebuilt
    /// per frame at the interpolated size. Rebuilding would re-lay-out a
    /// subtree every frame of the flight and re-wrap any text inside it, which
    /// is both expensive and visibly wrong: text that reflows mid-flight reads
    /// as a glitch rather than as movement.
    #[must_use]
    pub fn scale_at(&self, t: f32) -> (f32, f32) {
        let rect = self.rect_at(t);
        let width = self.to.width();
        let height = self.to.height();
        (
            if width.abs() < f32::EPSILON {
                1.0
            } else {
                rect.width() / width
            },
            if height.abs() < f32::EPSILON {
                1.0
            } else {
                rect.height() / height
            },
        )
    }
}

/// The set of elements flying between two screens.
///
/// Built by pairing tags: an element flies only when the **same tag exists on
/// both** screens. One that appears on only one side has nothing to fly to or
/// from, and is left to the route transition — which is the right answer, and
/// the reason this pairs rather than assuming.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SharedFlight {
    flights: Vec<(SharedTag, Flight)>,
}

impl SharedFlight {
    /// Pair the tags present in both geometries.
    ///
    /// In sorted tag order, so the flights keep a stable z-order between
    /// frames.
    #[must_use]
    pub fn between(from: &SharedGeometry, to: &SharedGeometry) -> Self {
        let flights = from
            .tags()
            .into_iter()
            .filter_map(|tag| {
                let start = from.get(&tag)?;
                let end = to.get(&tag)?;
                Some((
                    tag,
                    Flight {
                        from: start,
                        to: end,
                    },
                ))
            })
            .collect();
        Self { flights }
    }

    /// Every tag that is flying.
    #[must_use]
    pub fn tags(&self) -> Vec<SharedTag> {
        self.flights.iter().map(|(tag, _)| tag.clone()).collect()
    }

    /// `true` when this tag is in flight — what a screen passes to
    /// [`SharedElement::hidden`].
    #[must_use]
    pub fn is_flying(&self, tag: &SharedTag) -> bool {
        self.flights.iter().any(|(candidate, _)| candidate == tag)
    }

    /// The flight for one tag.
    #[must_use]
    pub fn get(&self, tag: &SharedTag) -> Option<Flight> {
        self.flights
            .iter()
            .find(|(candidate, _)| candidate == tag)
            .map(|(_, flight)| *flight)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.flights.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flights.is_empty()
    }

    /// The overlay to draw above both screens at progress `t`.
    ///
    /// `build` is called once per flying tag and should return the element as
    /// it looks **at its destination size** — see [`Flight::scale_at`] for why
    /// the destination and not the interpolated one.
    ///
    /// Returns a [`Stack`] positioned in global coordinates, so it belongs at
    /// the root of the tree, over the two screens. An empty flight returns an
    /// empty stack rather than `None`, so a caller can put it in the tree
    /// unconditionally.
    #[must_use]
    pub fn overlay(&self, t: f32, build: impl Fn(&SharedTag) -> WidgetNode) -> WidgetNode {
        let mut stack = Stack::new();
        for (tag, flight) in &self.flights {
            let rect = flight.rect_at(t);
            let (scale_x, scale_y) = flight.scale_at(t);
            stack = stack.push(
                Positioned::new().left(rect.left).top(rect.top).child(
                    vieww_widget::Transformed::new(Transform::scale(scale_x, scale_y))
                        .child(build(tag)),
                ),
            );
        }
        stack.into()
    }
}

/// The whole thing, driven by one progress value.
///
/// A convenience over [`SharedRegistry`] and [`SharedFlight`] for the ordinary
/// case: capture two screens, hand back an overlay per frame.
///
/// ```
/// use vieww_element::{SharedGeometry, SharedRegistry, SharedFlight};
/// use vieww_widget::prelude::*;
///
/// # fn example(from: SharedGeometry, to: SharedGeometry) -> WidgetNode {
/// let flight = SharedFlight::between(&from, &to);
/// // Every frame, with `t` from a spring:
/// flight.overlay(0.5, |tag| {
///     Container::new().color(Color::BLUE).into()
/// })
/// # }
/// ```
#[must_use]
pub fn offset_between(from: Rect, to: Rect, t: f32) -> Offset {
    let flight = Flight { from, to };
    let rect = flight.rect_at(t);
    Offset::new(rect.left, rect.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, width: f32, height: f32) -> Rect {
        Rect::new(left, top, left + width, top + height)
    }

    fn geometry(entries: &[(&str, Rect)]) -> SharedGeometry {
        let registry = SharedRegistry::new();
        for (tag, rect) in entries {
            registry.record(SharedTag::new(*tag), *rect);
        }
        registry.snapshot()
    }

    /// The pairing rule. An element with no counterpart has nowhere to fly, and
    /// inventing a destination for it is how a shared-element transition throws
    /// something across the screen for no reason.
    #[test]
    fn only_tags_present_on_both_screens_fly() {
        let from = geometry(&[
            ("photo-1", rect(10.0, 10.0, 40.0, 40.0)),
            ("only-on-a", rect(0.0, 0.0, 10.0, 10.0)),
        ]);
        let to = geometry(&[
            ("photo-1", rect(0.0, 0.0, 300.0, 200.0)),
            ("only-on-b", rect(0.0, 0.0, 10.0, 10.0)),
        ]);

        let flight = SharedFlight::between(&from, &to);
        assert_eq!(flight.len(), 1);
        assert_eq!(flight.tags(), vec![SharedTag::new("photo-1")]);
        assert!(flight.is_flying(&SharedTag::new("photo-1")));
        assert!(!flight.is_flying(&SharedTag::new("only-on-a")));
    }

    /// The two endpoints have to be exact, or the element visibly jumps at the
    /// moment the flight hands back to the real widget — the single most
    /// noticeable way this effect is implemented wrong.
    #[test]
    fn a_flight_starts_and_ends_exactly_on_its_endpoints() {
        let flight = Flight {
            from: rect(10.0, 20.0, 40.0, 30.0),
            to: rect(100.0, 200.0, 300.0, 180.0),
        };

        assert_eq!(flight.rect_at(0.0), flight.from);
        assert_eq!(flight.rect_at(1.0), flight.to);

        // And the scale resolves to 1 at the destination, since the child is
        // built at that size.
        let (x, y) = flight.scale_at(1.0);
        assert!((x - 1.0).abs() < 1e-6 && (y - 1.0).abs() < 1e-6);
    }

    /// Interpolating the rectangle rather than a position plus one scale is
    /// what lets a square land exactly on a wide rectangle.
    #[test]
    fn a_flight_between_different_aspect_ratios_lands_square_on_the_target() {
        let flight = Flight {
            from: rect(0.0, 0.0, 50.0, 50.0),
            to: rect(0.0, 0.0, 300.0, 100.0),
        };

        let (x, y) = flight.scale_at(0.5);
        assert!(
            (x - y).abs() > 0.1,
            "the two axes must scale differently: {x} vs {y}"
        );

        let midway = flight.rect_at(0.5);
        assert_eq!(midway.width(), 175.0);
        assert_eq!(midway.height(), 75.0);
    }

    #[test]
    fn a_flight_moves_monotonically_towards_its_destination() {
        let flight = Flight {
            from: rect(0.0, 0.0, 20.0, 20.0),
            to: rect(200.0, 100.0, 60.0, 60.0),
        };

        let mut previous = flight.rect_at(0.0).left;
        for step in 1..=10 {
            let now = flight.rect_at(step as f32 / 10.0).left;
            assert!(now > previous, "went backwards at step {step}");
            previous = now;
        }
    }

    /// A degenerate destination must not produce a NaN scale that poisons the
    /// transform and blanks the screen.
    #[test]
    fn a_zero_sized_destination_does_not_produce_a_nan_scale() {
        let flight = Flight {
            from: rect(0.0, 0.0, 10.0, 10.0),
            to: rect(50.0, 50.0, 0.0, 0.0),
        };
        let (x, y) = flight.scale_at(0.5);
        assert!(x.is_finite() && y.is_finite(), "{x} {y}");
    }

    /// Z-order has to be stable frame to frame, or two overlapping flights
    /// swap places at random and flicker. The registry is a `HashMap`, which
    /// has no order at all, so this is sorted deliberately.
    #[test]
    fn flights_keep_a_stable_order_between_frames() {
        let from = geometry(&[
            ("c", rect(0.0, 0.0, 1.0, 1.0)),
            ("a", rect(0.0, 0.0, 1.0, 1.0)),
            ("b", rect(0.0, 0.0, 1.0, 1.0)),
        ]);
        let to = from.clone();

        let first = SharedFlight::between(&from, &to).tags();
        for _ in 0..8 {
            assert_eq!(SharedFlight::between(&from, &to).tags(), first);
        }
        assert_eq!(
            first,
            vec![
                SharedTag::new("a"),
                SharedTag::new("b"),
                SharedTag::new("c")
            ]
        );
    }

    /// A registry left over from a screen that is gone would pair with the same
    /// tag on the next one and fly from a stale rectangle.
    #[test]
    fn clearing_the_registry_forgets_a_screen() {
        let registry = SharedRegistry::new();
        registry.record(SharedTag::new("a"), rect(0.0, 0.0, 1.0, 1.0));
        assert_eq!(registry.len(), 1);

        registry.clear();
        assert!(registry.is_empty());
        assert!(registry.snapshot().is_empty());
    }

    /// The overlay has to be usable unconditionally, so a screen does not need
    /// an `if` around it on every frame that is not a transition.
    #[test]
    fn an_empty_flight_still_builds_an_overlay() {
        let flight = SharedFlight::default();
        let overlay = flight.overlay(0.5, |_| SizedBox::shrink().into());
        assert_eq!(
            vieww_widget::debug_tree(overlay.clone()).lines().count(),
            1,
            "an empty stack and nothing else"
        );
    }

    /// A `SharedElement` reports even while hidden — the destination geometry
    /// is captured from a screen that is, at that moment, hidden by the flight
    /// that needs it.
    #[test]
    fn a_hidden_shared_element_still_reports_its_rectangle() {
        let registry = SharedRegistry::new();
        let hidden = SharedElement::new("photo", &registry)
            .hidden(true)
            .child(SizedBox::square(10.0));

        let built = hidden.build(&BuildContext::root());
        let dump = vieww_widget::debug_tree(built.clone());
        assert!(
            dump.contains("Measured"),
            "the measurement must survive being hidden:\\n{dump}"
        );
        assert!(
            dump.contains("Offstage"),
            "and it must actually be hidden:\\n{dump}"
        );
    }
}
