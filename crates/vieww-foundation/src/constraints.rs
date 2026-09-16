use std::fmt;

use crate::{EdgeInsets, Size};

/// The immutable box constraints passed *down* the render tree during layout.
///
/// This is the "constraints down, sizes up" half of the layout protocol: a
/// parent hands its child a `Constraints`, and the child returns a [`Size`] that
/// satisfies it. A child never learns its own position from layout — the parent
/// assigns that afterwards.
///
/// Invariant: `0 <= min <= max` on each axis, with `max` allowed to be
/// [`f32::INFINITY`]. [`Constraints::is_normalized`] checks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
}

impl Constraints {
    /// Constraints permitting any size at all.
    pub const UNBOUNDED: Self = Self {
        min_width: 0.0,
        max_width: f32::INFINITY,
        min_height: 0.0,
        max_height: f32::INFINITY,
    };

    #[must_use]
    pub const fn new(min_width: f32, max_width: f32, min_height: f32, max_height: f32) -> Self {
        Self {
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }

    /// Constraints that permit exactly `size` and nothing else.
    #[must_use]
    pub const fn tight(size: Size) -> Self {
        Self::new(size.width, size.width, size.height, size.height)
    }

    /// Constraints permitting anything from zero up to `size`.
    #[must_use]
    pub const fn loose(size: Size) -> Self {
        Self::new(0.0, size.width, 0.0, size.height)
    }

    /// Tight on width, free on height. Useful for a full-width column child.
    #[must_use]
    pub const fn tight_for_width(width: f32) -> Self {
        Self::new(width, width, 0.0, f32::INFINITY)
    }

    /// Tight on height, free on width.
    #[must_use]
    pub const fn tight_for_height(height: f32) -> Self {
        Self::new(0.0, f32::INFINITY, height, height)
    }

    /// `true` if only one size satisfies these constraints.
    #[must_use]
    pub fn is_tight(self) -> bool {
        self.min_width >= self.max_width && self.min_height >= self.max_height
    }

    #[must_use]
    pub fn has_bounded_width(self) -> bool {
        self.max_width.is_finite()
    }

    #[must_use]
    pub fn has_bounded_height(self) -> bool {
        self.max_height.is_finite()
    }

    /// `true` if the invariant `0 <= min <= max` holds on both axes.
    #[must_use]
    pub fn is_normalized(self) -> bool {
        self.min_width >= 0.0
            && self.min_width <= self.max_width
            && self.min_height >= 0.0
            && self.min_height <= self.max_height
    }

    /// The size closest to `size` that satisfies these constraints.
    #[must_use]
    pub fn constrain(self, size: Size) -> Size {
        Size::new(
            size.width.clamp(self.min_width, self.max_width),
            size.height.clamp(self.min_height, self.max_height),
        )
    }

    /// The smallest size satisfying these constraints.
    #[must_use]
    pub fn smallest(self) -> Size {
        Size::new(self.min_width, self.min_height)
    }

    /// The largest size satisfying these constraints.
    ///
    /// May contain infinities; a render object that expands to fill must first
    /// check [`has_bounded_width`](Self::has_bounded_width).
    #[must_use]
    pub fn biggest(self) -> Size {
        Size::new(self.max_width, self.max_height)
    }

    /// These constraints with the minimums dropped to zero.
    #[must_use]
    pub fn loosen(self) -> Self {
        Self::new(0.0, self.max_width, 0.0, self.max_height)
    }

    /// These constraints shrunk by `insets`, clamped at zero.
    ///
    /// This is what a padding render object hands its child.
    #[must_use]
    pub fn deflate(self, insets: EdgeInsets) -> Self {
        let horizontal = insets.horizontal();
        let vertical = insets.vertical();
        let max_width = (self.max_width - horizontal).max(0.0);
        let max_height = (self.max_height - vertical).max(0.0);
        Self::new(
            (self.min_width - horizontal).max(0.0).min(max_width),
            max_width,
            (self.min_height - vertical).max(0.0).min(max_height),
            max_height,
        )
    }

    /// These constraints clamped so they also satisfy `outer`.
    ///
    /// Used where a widget requests a size that its parent may refuse — the
    /// parent's constraints always win.
    #[must_use]
    pub fn enforce(self, outer: Self) -> Self {
        let min_width = self.min_width.clamp(outer.min_width, outer.max_width);
        let max_width = self.max_width.clamp(outer.min_width, outer.max_width);
        let min_height = self.min_height.clamp(outer.min_height, outer.max_height);
        let max_height = self.max_height.clamp(outer.min_height, outer.max_height);
        Self::new(
            min_width.min(max_width),
            max_width,
            min_height.min(max_height),
            max_height,
        )
    }

    /// These constraints with any **infinite minimum** replaced by `outer`'s.
    ///
    /// # The failure this exists to make unwritable
    ///
    /// An infinite *maximum* is ordinary: it is what "unbounded" means, and
    /// every scrollable hands one to its content. An infinite **minimum** is
    /// not. It says "you must be infinitely large", which no size satisfies, and
    /// nothing downstream rejects it — [`smallest`](Self::smallest) returns
    /// infinity and [`constrain`](Self::constrain) passes it through. It only
    /// becomes visible when an ancestor subtracts one infinity from another: a
    /// flex dividing the room that is left, a padding adding its insets. That
    /// yields `NaN`, a long way from the widget that asked, and `NaN` spreads
    /// into every sibling's geometry — so the whole window lays out to nothing
    /// and the screen goes blank with no error naming a cause.
    ///
    /// One shape produces it, and the shape is common: a **full-surface
    /// overlay** — `SizedBox::expand`, a modal barrier, a scrim — mounted where
    /// an axis is unbounded, which is any flex's main axis and anything inside a
    /// scrollable. [`enforce`](Self::enforce) clamps the requested infinite
    /// minimum against an equally infinite maximum and lets it stand.
    ///
    /// So on an axis the parent leaves unbounded, "as large as you are allowed"
    /// resolves to the parent's minimum and the child shrink-wraps instead. That
    /// is the only finite reading of the request, and it keeps the failure local
    /// to the widget that asked for the impossible. `RenderStack`'s
    /// `StackFit::Expand` branch already reaches the same answer by
    /// construction, for the same reason and in the same words; this is that
    /// rule for the other half of the framework that can ask.
    ///
    /// A bounded axis is untouched, so this is a no-op everywhere that is not
    /// already broken.
    #[must_use]
    pub fn finite_minimums(self, outer: Self) -> Self {
        Self::new(
            if self.min_width.is_finite() {
                self.min_width
            } else {
                outer.min_width
            },
            self.max_width,
            if self.min_height.is_finite() {
                self.min_height
            } else {
                outer.min_height
            },
            self.max_height,
        )
    }

    /// These constraints tightened to `width`/`height` where each is given.
    #[must_use]
    pub fn tighten(self, width: Option<f32>, height: Option<f32>) -> Self {
        let (min_width, max_width) = match width {
            Some(w) => {
                let w = w.clamp(self.min_width, self.max_width);
                (w, w)
            }
            None => (self.min_width, self.max_width),
        };
        let (min_height, max_height) = match height {
            Some(h) => {
                let h = h.clamp(self.min_height, self.max_height);
                (h, h)
            }
            None => (self.min_height, self.max_height),
        };
        Self::new(min_width, max_width, min_height, max_height)
    }
}

impl Default for Constraints {
    fn default() -> Self {
        Self::UNBOUNDED
    }
}

impl fmt::Display for Constraints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "w[{}..{}] h[{}..{}]",
            self.min_width, self.max_width, self.min_height, self.max_height
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tight_constraints_admit_exactly_one_size() {
        let c = Constraints::tight(Size::new(100.0, 50.0));
        assert!(c.is_tight());
        assert_eq!(c.constrain(Size::new(999.0, 0.0)), Size::new(100.0, 50.0));
    }

    #[test]
    fn deflate_shrinks_both_bounds_and_clamps_at_zero() {
        let c = Constraints::new(50.0, 100.0, 50.0, 100.0);
        let d = c.deflate(EdgeInsets::all(10.0));
        assert_eq!(d, Constraints::new(30.0, 80.0, 30.0, 80.0));

        let tiny = Constraints::new(0.0, 5.0, 0.0, 5.0).deflate(EdgeInsets::all(10.0));
        assert_eq!(tiny, Constraints::new(0.0, 0.0, 0.0, 0.0));
        assert!(tiny.is_normalized());
    }

    #[test]
    fn deflate_stays_normalized_for_any_inset() {
        // Both bounds shrink by the same amount and clamp at the same floor, so
        // `0 <= min <= max` has to survive every inset — including ones larger
        // than the constraints themselves, which is what a deeply nested
        // padding chain in a narrow parent actually produces.
        let c = Constraints::new(90.0, 100.0, 40.0, 100.0);
        for inset in [0.0, 1.0, 5.0, 45.0, 49.0, 50.0, 51.0, 200.0] {
            let d = c.deflate(EdgeInsets::all(inset));
            assert!(d.is_normalized(), "inset {inset} produced {d}");
        }

        assert_eq!(
            c.deflate(EdgeInsets::all(5.0)),
            Constraints::new(80.0, 90.0, 30.0, 90.0)
        );
        assert_eq!(
            c.deflate(EdgeInsets::all(200.0)),
            Constraints::new(0.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn enforce_lets_the_outer_constraints_win() {
        let requested = Constraints::tight(Size::new(500.0, 500.0));
        let parent = Constraints::loose(Size::new(200.0, 200.0));
        let result = requested.enforce(parent);
        assert_eq!(result, Constraints::tight(Size::new(200.0, 200.0)));
        assert!(result.is_normalized());
    }

    #[test]
    fn unbounded_axes_are_reported_as_unbounded() {
        let c = Constraints::tight_for_width(300.0);
        assert!(c.has_bounded_width());
        assert!(!c.has_bounded_height());
        assert!(!c.is_tight());
    }

    #[test]
    fn tighten_respects_the_incoming_bounds() {
        let c = Constraints::loose(Size::new(100.0, 100.0));
        assert_eq!(
            c.tighten(Some(500.0), None),
            Constraints::new(100.0, 100.0, 0.0, 100.0)
        );
        assert_eq!(
            c.tighten(Some(40.0), Some(40.0)),
            Constraints::tight(Size::square(40.0))
        );
    }

    /// The shape that produces the infinite minimum, written out in full so the
    /// test names the widget rather than the arithmetic: a full-surface overlay
    /// inside a column.
    #[test]
    fn expanding_on_an_unbounded_axis_shrink_wraps_instead_of_demanding_infinity() {
        // What a column hands its child: bounded across, unbounded along.
        let column = Constraints::new(0.0, 400.0, 0.0, f32::INFINITY);
        let expand = Constraints::tight(Size::INFINITE);

        let naive = expand.enforce(column);
        assert!(
            naive.min_height.is_infinite(),
            "the premise: `enforce` alone clamps an infinite minimum against an \
             infinite maximum and lets it stand — {naive}"
        );

        let fixed = naive.finite_minimums(column);
        assert_eq!(
            fixed,
            Constraints::new(400.0, 400.0, 0.0, f32::INFINITY),
            "the bounded axis still expands; the unbounded one falls back to the \
             parent's minimum"
        );
        assert!(fixed.is_normalized());
        assert!(
            fixed.smallest().height.is_finite(),
            "and `smallest` — which is what a childless `SizedBox` returns — is \
             finite, which is the whole point"
        );
    }

    #[test]
    fn a_finite_minimum_is_left_exactly_as_it_was() {
        // The no-op case, asserted so this cannot quietly start rewriting
        // constraints that were never broken.
        for c in [
            Constraints::tight(Size::new(100.0, 50.0)),
            Constraints::loose(Size::new(100.0, 50.0)),
            Constraints::new(10.0, f32::INFINITY, 20.0, f32::INFINITY),
            Constraints::UNBOUNDED,
        ] {
            assert_eq!(c.finite_minimums(Constraints::UNBOUNDED), c);
        }
    }

    /// `NaN` is the symptom; an infinite minimum is the cause. This is the step
    /// between them, so a reader meeting a `NaN` size has somewhere to start.
    #[test]
    fn an_infinite_extent_becomes_nan_the_moment_an_ancestor_subtracts() {
        let infinite = f32::INFINITY;
        assert!(
            (infinite - infinite).is_nan(),
            "a flex dividing the room left over, or a padding removing its \
             insets, is a subtraction — and this is why one infinite child \
             makes every sibling's geometry `NaN`"
        );
    }
}
