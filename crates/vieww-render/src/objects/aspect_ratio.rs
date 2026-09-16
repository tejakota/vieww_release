use vieww_foundation::{Constraints, Offset, Size};

use crate::{LayoutCtx, RenderObject};

/// Sizes itself to a width:height ratio, as large as its constraints allow.
///
/// The ratio is width divided by height, so 16:9 is `16.0 / 9.0` and a square
/// is `1.0`.
///
/// # How the size is chosen
///
/// Width is preferred: the box takes all the width it is allowed and derives
/// its height from the ratio. That first answer is then walked back through
/// four checks, in order — too wide, too tall, too narrow, too short — each of
/// which pins one axis to the bound it broke and re-derives the other. This is
/// The classic aspect-ratio algorithm, and the order matters: the maxima
/// are tried before the minima, so a box squeezed between conflicting bounds
/// ends up honouring the **minimum**, which is the one that cannot be silently
/// clipped.
///
/// Whatever survives those four is passed through the constraints one last
/// time, so the result is always legal even where no ratio-respecting size is.
/// **A ratio is a preference, not a guarantee**: constraints win, and a box
/// given a tight 100x100 is 100x100 whatever ratio it was asked for.
///
/// # Degenerate input has a defined answer here
///
/// Two cases have no meaningful ratio-derived size, and both would otherwise
/// produce a non-finite one. A `NaN` or infinite size is the worst possible
/// outcome in a layout tree: it survives `constrain`, propagates into every
/// ancestor's arithmetic, and turns hit testing into a silent no-op far from
/// wherever it started. Neither is worth trading for fidelity to a debug-only
/// assertion, so both are answered rather than asserted:
///
/// - **A ratio that is not positive and finite** cannot describe a box at all.
///   [`RenderAspectRatio::new`] substitutes [`FALLBACK_RATIO`] — a square — so
///   nothing downstream ever divides by zero or by `NaN`. This is checked once,
///   at construction, rather than on every layout pass.
/// - **Constraints unbounded on both axes** — inside a scroll view that is also
///   inside an unbounded row, say — say the box may be any size at all, and a
///   ratio alone cannot pick one. The answer is [`Constraints::smallest`],
///   which is normally zero: it collapses, visibly, instead of expanding to
///   infinity and taking the rest of the layout with it.
///
/// Both are asserted in this module's tests.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderAspectRatio {
    /// Width divided by height. Positive and finite, guaranteed by
    /// [`new`](Self::new) — the layout below relies on it and does not re-check.
    ratio: f32,
}

/// The ratio substituted for one that is not positive and finite: a square.
///
/// Chosen because it is the only ratio with no orientation, so a box that fell
/// back to it does not look like a deliberate landscape or portrait choice
/// somebody made and got wrong.
pub const FALLBACK_RATIO: f32 = 1.0;

impl RenderAspectRatio {
    /// A box of `ratio` width to height.
    ///
    /// A `ratio` that is not positive and finite — zero, negative, infinite or
    /// `NaN` — is replaced by [`FALLBACK_RATIO`]. See the type docs for why
    /// that is a substitution rather than a panic.
    #[must_use]
    pub fn new(ratio: f32) -> Self {
        Self {
            ratio: if ratio.is_finite() && ratio > 0.0 {
                ratio
            } else {
                FALLBACK_RATIO
            },
        }
    }

    /// The ratio actually in use, after any substitution.
    #[must_use]
    pub const fn ratio(&self) -> f32 {
        self.ratio
    }

    /// The size the ratio implies within `constraints`.
    ///
    /// Split out from `layout` because it is the whole of the interesting
    /// behaviour and depends on nothing but its argument, so the tests below
    /// can cover the four fallbacks without building a tree for each.
    fn size_for(&self, constraints: Constraints) -> Size {
        // Tight constraints leave nothing to decide, and going through the
        // arithmetic below would only arrive back here after four no-ops.
        if constraints.is_tight() {
            return constraints.smallest();
        }

        let ratio = self.ratio;

        let (mut width, mut height) = if constraints.has_bounded_width() {
            let width = constraints.max_width;
            (width, width / ratio)
        } else if constraints.has_bounded_height() {
            let height = constraints.max_height;
            (height * ratio, height)
        } else {
            // Neither axis bounded: see "Degenerate input" on the type.
            return constraints.smallest();
        };

        // Four attempts, maxima before minima. Each pins the axis that broke
        // its bound and re-derives the other from the ratio.
        if width > constraints.max_width {
            width = constraints.max_width;
            height = width / ratio;
        }
        if height > constraints.max_height {
            height = constraints.max_height;
            width = height * ratio;
        }
        if width < constraints.min_width {
            width = constraints.min_width;
            height = width / ratio;
        }
        if height < constraints.min_height {
            height = constraints.min_height;
            width = height * ratio;
        }

        // The ratio is a preference; the constraints are not.
        constraints.constrain(Size::new(width, height))
    }
}

impl RenderObject for RenderAspectRatio {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let size = self.size_for(constraints);

        // Copied out before the branch, not read inside its scrutinee: either
        // way — the `Vec` `children()` used to return, or the slice it borrows
        // now — the value would hold `ctx` borrowed for the whole `if let` body,
        // and the two calls below need it mutably.
        let child = ctx.children().first().copied();

        // The child is given the answer, tightly. It has no say: the whole
        // point of this box is that its shape is decided by the ratio, and a
        // child allowed to choose could return something that is not that
        // shape.
        if let Some(child) = child {
            ctx.layout_child(child, Constraints::tight(size));
            ctx.place_child(child, Offset::ZERO);
        }

        size
    }

    crate::baseline::pass_through_baseline!();

    /// The extent the ratio implies from the *other* axis, or `None` when the
    /// other axis is unknown.
    ///
    /// # With a cross extent, the answer is exact
    ///
    /// This is the case worth having, and it is why a feed puts an image row in
    /// an `IntrinsicHeight`: given a width, a ratio names exactly one height,
    /// and given a height exactly one width.
    /// The arithmetic is the same arithmetic `size_for` does — `width / ratio`
    /// and `height * ratio` — deliberately, because the two agreeing is the only
    /// property of this implementation worth having, and the tests below pin it
    /// by running both for the same configuration.
    ///
    /// Mirroring `size_for` means mirroring which axis leads. A query with
    /// `cross` set is the shape `size_for` sees when one axis is bounded and the
    /// other is not, and in that shape both of `size_for`'s leading branches
    /// reduce to the one multiplication here: the four fallbacks after it are
    /// no-ops, because there is no opposing bound left for the derived extent to
    /// break. Constraints the *caller* holds are none of this object's business
    /// — an intrinsic answers for content, and clamping here would report a size
    /// the caller had not asked about.
    ///
    /// # Without one, this is genuinely unknown
    ///
    /// `cross: None` says the other axis is unconstrained, and a ratio with no
    /// extent to scale is not a size — it is a shape. `size_for` says the same
    /// thing when neither axis is bounded: it gives up and returns
    /// [`Constraints::smallest`], which is a *deliberate collapse* chosen so an
    /// undecidable box does not expand to infinity and take the rest of the
    /// layout with it. It is a fallback, not a natural size, and reporting it
    /// here as `Some(0.0)` would be the module's cardinal error — the caller
    /// would read a confident zero, tighten this box to nothing, and a card
    /// would vanish from a screen where plain layout would have shown it at
    /// whatever width it was offered. `None` makes the asking widget
    /// transparent, layout runs as it did before the intrinsic pass existed, and
    /// the box takes the room it is given.
    ///
    /// The child is not consulted, and that is not an omission. `layout` gives
    /// the child [`Constraints::tight`] of the size the ratio picked, so the
    /// child has no vote in this object's size at all; deriving an answer from
    /// the child's own intrinsic would produce a number layout can never
    /// produce, which is exactly the disagreement this method exists to avoid.
    /// The classic algorithm does fall back to the child here — with a
    /// child it lays out just as tightly — and that is the inconsistency being
    /// declined rather than copied.
    ///
    /// # The extremum does not change the answer
    ///
    /// Nothing about a ratio reflows: there is no slack between "the narrowest
    /// it can be without clipping" and "the widest it would like to be", so
    /// [`Extremum::Min`](crate::Extremum) and `Max` are one number, the same way
    /// they are for a bitmap. Both are asserted below, so a later change that
    /// makes them differ has to say why.
    fn intrinsic(
        &self,
        _ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        use vieww_foundation::Axis;

        // A non-finite cross extent is the same statement as `None` — an axis
        // with no bound on it — and it must be filtered *before* the arithmetic
        // rather than after: `INFINITY * ratio` is infinity, which the tree
        // turns into a hard `0.0` on the way out, and that zero is precisely the
        // collapse the branch below refuses to report. A negative extent is a
        // caller's bug rather than a size, and clamping keeps it from inverting
        // the answer.
        let cross = query.cross.filter(|extent| extent.is_finite())?.max(0.0);

        Some(match query.axis {
            // The ratio is width over height, so height scales up to a width.
            Axis::Horizontal => cross * self.ratio,
            Axis::Vertical => cross / self.ratio,
        })
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderAspectRatio"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::{Color, Rect};
    use vieww_widget::{AspectRatio, Center, ColoredBox, Constrained, WidgetNode};

    use super::*;
    use crate::{FrameDriver, IntrinsicQuery, RenderTree};

    /// The size a ratio picks within `constraints`, without a tree.
    fn size(ratio: f32, constraints: Constraints) -> Size {
        RenderAspectRatio::new(ratio).size_for(constraints)
    }

    /// One frame of `root` on a `surface`-sized driver.
    fn drawn(surface: Size, root: impl Into<WidgetNode>) -> FrameDriver {
        let mut driver = FrameDriver::new(surface);
        driver.set_root(root);
        driver.draw_frame();
        driver
    }

    /// Where the blue child actually landed, in pixels.
    ///
    /// Every end-to-end test below asks the same question of the scene, and a
    /// zero-height box may not be recorded at all — so a missing fill is a
    /// failure of the thing under test rather than of this helper.
    fn painted_blue(driver: &FrameDriver) -> Rect {
        driver
            .scene()
            .fills()
            .into_iter()
            .find(|(_, paint)| paint.color == Color::BLUE)
            .map(|(rect, _)| rect)
            .expect("the child is painted")
    }

    /// A blue box at `ratio`, capped at `cap` wide by an ancestor.
    ///
    /// The gallery's fourth card, reduced to the part that can be asserted.
    fn capped_card(ratio: f32, cap: f32) -> WidgetNode {
        Center::new()
            .child(
                Constrained::new(Constraints::new(0.0, cap, 0.0, f32::INFINITY))
                    .child(AspectRatio::new(ratio).child(ColoredBox::new(Color::BLUE))),
            )
            .into()
    }

    #[test]
    fn width_is_taken_first_and_the_height_follows_from_the_ratio() {
        // The ordinary case: a 16:9 video in a column 320 wide.
        let chosen = size(16.0 / 9.0, Constraints::loose(Size::new(320.0, 1000.0)));
        assert_eq!(chosen.width, 320.0);
        assert!(
            (chosen.height - 180.0).abs() < 0.01,
            "320 / (16/9) is 180, got {}",
            chosen.height
        );
    }

    #[test]
    fn a_height_bound_takes_over_when_the_width_is_unbounded() {
        // In a horizontal scroll view there is no width to start from, so the
        // height leads instead. Without this branch the first answer would be
        // infinite and every later check would compare against infinity.
        let chosen = size(2.0, Constraints::new(0.0, f32::INFINITY, 0.0, 100.0));
        assert_eq!(chosen, Size::new(200.0, 100.0));
    }

    #[test]
    fn a_box_too_tall_for_its_bounds_is_pinned_by_height_instead() {
        // 320 wide at 1:2 wants 640 tall, but only 200 is allowed. The second
        // fallback re-derives the width from the height it can actually have.
        let chosen = size(0.5, Constraints::loose(Size::new(320.0, 200.0)));
        assert_eq!(
            chosen,
            Size::new(100.0, 200.0),
            "the height was pinned to its maximum and the width re-derived, \
             rather than the box overflowing by 440pt"
        );
    }

    #[test]
    fn a_width_minimum_widens_a_box_the_height_bound_had_already_shrunk() {
        // **The third fallback, and the only route to it.** Width starts at
        // `max_width`, so it can never begin below `min_width` — this check is
        // unreachable until the *second* fallback has re-derived a width from a
        // pinned height and undershot.
        //
        // 1:2 in a 320x200 space wants 320x640; the height bound pins it to
        // 100x200; a 150 minimum width then widens it to 150x300, which the
        // final `constrain` trims back to the 200 the height still allows.
        let chosen = size(0.5, Constraints::new(150.0, 320.0, 0.0, 200.0));
        assert_eq!(chosen, Size::new(150.0, 200.0));
    }

    #[test]
    fn a_height_minimum_grows_the_box_and_the_width_follows() {
        // The fourth fallback. 2:1 in a 100-wide space wants 100x50, but
        // something above demands at least 80 tall, so the height wins and the
        // width is re-derived to 160 — which the final `constrain` trims to the
        // 100 actually available.
        let chosen = size(2.0, Constraints::new(0.0, 100.0, 80.0, 200.0));
        assert_eq!(chosen, Size::new(100.0, 80.0));
    }

    #[test]
    fn every_answer_is_a_legal_size_even_where_no_ratio_respecting_one_exists() {
        // The last `constrain` is what makes that true, and it is easy to drop
        // as redundant — the four fallbacks look like they have already handled
        // everything. They have not: each re-derives one axis from the other
        // without rechecking the bound it just satisfied.
        for constraints in [
            Constraints::new(150.0, 320.0, 0.0, 200.0),
            Constraints::new(0.0, 100.0, 80.0, 200.0),
            Constraints::new(50.0, 60.0, 300.0, 400.0),
        ] {
            let chosen = size(3.0, constraints);
            assert_eq!(
                chosen,
                constraints.constrain(chosen),
                "{chosen:?} is not legal within {constraints:?}"
            );
        }
    }

    #[test]
    fn tight_constraints_win_over_the_ratio() {
        // A ratio is a preference. This is the case that makes that concrete,
        // and it is also the early return: a tight box has nothing to decide.
        let chosen = size(16.0 / 9.0, Constraints::tight(Size::new(100.0, 100.0)));
        assert_eq!(
            chosen,
            Size::new(100.0, 100.0),
            "no ratio survives constraints that allow exactly one size"
        );
    }

    #[test]
    fn a_ratio_that_is_not_a_positive_finite_number_becomes_a_square() {
        // **Not fidelity to a debug-only assertion.** Each of these would
        // otherwise divide into the size and produce `NaN` or infinity, which
        // `constrain` passes straight through and which then poisons every
        // ancestor's arithmetic and silently breaks hit testing far away from
        // here. A square is wrong; a `NaN` is undiagnosable.
        let space = Constraints::loose(Size::new(200.0, 500.0));
        for bad in [0.0, -2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let chosen = size(bad, space);
            assert!(
                chosen.width.is_finite() && chosen.height.is_finite(),
                "a ratio of {bad} produced {chosen:?}, which is not a size"
            );
            assert_eq!(
                chosen,
                Size::new(200.0, 200.0),
                "a ratio of {bad} falls back to a square"
            );
        }
    }

    #[test]
    fn the_substituted_ratio_is_what_the_object_reports() {
        // So a tree dump explains the square, rather than showing the `NaN`
        // that was asked for and a square that does not follow from it.
        assert_eq!(RenderAspectRatio::new(f32::NAN).ratio(), FALLBACK_RATIO);
        assert_eq!(RenderAspectRatio::new(2.0).ratio(), 2.0);
    }

    #[test]
    fn unbounded_on_both_axes_collapses_rather_than_expanding_to_infinity() {
        // A scroll view inside an unbounded row. There is no size a ratio can
        // pick here, and the alternative to collapsing is an infinite box that
        // takes the rest of the layout down with it.
        let chosen = size(
            16.0 / 9.0,
            Constraints::new(0.0, f32::INFINITY, 0.0, f32::INFINITY),
        );
        assert_eq!(chosen, Size::ZERO);
        assert!(chosen.width.is_finite() && chosen.height.is_finite());
    }

    #[test]
    fn an_unbounded_pair_still_honours_a_minimum() {
        // `smallest()` is not always zero, and a minimum is a demand from
        // something above that this box cannot see. Collapsing past it would
        // be a different bug from the one being avoided.
        let chosen = size(
            2.0,
            Constraints::new(30.0, f32::INFINITY, 40.0, f32::INFINITY),
        );
        assert_eq!(chosen, Size::new(30.0, 40.0));
    }

    #[test]
    fn the_widget_reaches_this_object_and_paints_at_the_ratio() {
        // **The end-to-end pass, and the only test here that would fail if the
        // factory registration were missing.** Everything above calls
        // `size_for` directly, so all of it would pass on a widget that never
        // reaches this type at all.
        //
        // Note the `Center`. The root is laid out under **tight** constraints of
        // the surface, so an `AspectRatio` mounted as the root reports
        // `smallest()` — the whole 320x1000 surface — and ignores its ratio
        // entirely. That is correct (constraints win), but it makes the obvious
        // version of this test assert 1000 and prove nothing about the ratio.
        // `Align` loosens; a `Constrained` alone would not.
        let driver = drawn(
            Size::new(320.0, 1000.0),
            Center::new().child(AspectRatio::new(16.0 / 9.0).child(ColoredBox::new(Color::BLUE))),
        );
        let blue = painted_blue(&driver);

        assert_eq!(blue.width(), 320.0, "all the width it was offered");
        assert!(
            (blue.height() - 180.0).abs() < 0.01,
            "320 at 16:9 is 180 tall, painted {}",
            blue.height()
        );
    }

    #[test]
    fn a_tight_parent_overrides_the_ratio_end_to_end() {
        // The other half of the pair, and the reason the test above needs a
        // `Center`. Mounted at the root the ratio is simply outvoted — asserted
        // rather than left as a footnote, because somebody will eventually
        // report it as a bug.
        let driver = drawn(
            Size::new(320.0, 1000.0),
            AspectRatio::new(16.0 / 9.0).child(ColoredBox::new(Color::BLUE)),
        );
        let blue = painted_blue(&driver);

        assert_eq!(
            (blue.width(), blue.height()),
            (320.0, 1000.0),
            "a tight root leaves exactly one legal size, and the ratio does not \
             get a vote"
        );
    }

    #[test]
    fn an_ancestor_cap_stops_the_box_growing_and_the_ratio_survives_it() {
        // **The gallery's fourth card, reduced to what can be asserted.** A bare
        // `AspectRatio` grows with its parent for ever, so the demo caps it so
        // the derived height stays inside a fixed band.
        //
        // That cap is *invisible* until there is enough room to reach it — over
        // a ~1104pt window in the gallery, which is wider than either screenshot
        // taken of it. Every observation so far would look identical with the
        // cap deleted, which is this project's own rule about a test that agrees
        // with the default. This is the test that disagrees with it.
        let driver = drawn(Size::new(900.0, 400.0), capped_card(16.0 / 9.0, 240.0));
        let blue = painted_blue(&driver);

        assert_eq!(
            blue.width(),
            240.0,
            "the cap held with 900pt of room going spare"
        );
        assert!(
            (blue.height() - 135.0).abs() < 0.01,
            "and 240 at 16:9 is 135, not {}",
            blue.height()
        );
    }

    #[test]
    fn the_cap_yields_to_a_parent_narrower_than_itself() {
        // The other half, and the reason the cap is a `Constrained` rather than
        // a `SizedBox`: `enforce` clamps the request into what the parent
        // actually offers, so the box keeps shrinking as the window closes in
        // instead of overflowing at a fixed 240.
        let driver = drawn(Size::new(160.0, 400.0), capped_card(16.0 / 9.0, 240.0));
        let blue = painted_blue(&driver);

        assert_eq!(blue.width(), 160.0, "the parent is narrower than the cap");
        assert!(
            (blue.height() - 90.0).abs() < 0.01,
            "160 at 16:9 is 90, not {}",
            blue.height()
        );
    }

    #[test]
    fn the_same_ratio_does_not_ask_for_a_relayout() {
        // `layout_differs_by_eq` over one `f32` field, so an unchanged ratio
        // costs nothing on rebuild.
        let old = RenderAspectRatio::new(2.0);
        assert!(!old.layout_differs(&RenderAspectRatio::new(2.0)));
        assert!(old.layout_differs(&RenderAspectRatio::new(3.0)));
    }

    // -------------------------------------------------------------- intrinsics

    /// Ask a ratio box a question, without ever laying it out.
    ///
    /// Never laid out on purpose: an intrinsic that only answered after a layout
    /// would be no use to the callers that need one — `Accordion` asks about
    /// content that is not on screen at all.
    fn intrinsic(ratio: f32, query: IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderAspectRatio::new(ratio)));
        tree.intrinsic(id, query)
    }

    /// The same, with a child that cannot answer anything.
    ///
    /// The pair with `intrinsic` above is the point: this box's size does not
    /// depend on its child, so an unmeasurable one must not poison the answer.
    fn intrinsic_with_child(ratio: f32, query: IntrinsicQuery) -> Option<f32> {
        let mut tree = RenderTree::new();
        let id = tree.insert(None, Box::new(RenderAspectRatio::new(ratio)));
        tree.insert(Some(id), Box::new(Unmeasurable));
        tree.intrinsic(id, query)
    }

    /// A child that answers nothing, like any render object that has not
    /// implemented `intrinsic`.
    #[derive(Debug)]
    struct Unmeasurable;

    impl RenderObject for Unmeasurable {
        fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
            constraints.smallest()
        }

        fn debug_name(&self) -> &'static str {
            "Unmeasurable"
        }
    }

    #[test]
    fn a_known_width_gives_an_exact_height_and_the_other_way_round() {
        // The case an `IntrinsicHeight` around a feed image is asking for: a
        // width is known, so the ratio names one height and there is nothing to
        // estimate.
        let tall = intrinsic(16.0 / 9.0, IntrinsicQuery::max_height().across(320.0));
        assert!(
            tall.is_some_and(|height| (height - 180.0).abs() < 0.01),
            "320 at 16:9 is 180 tall, got {tall:?}"
        );

        let wide = intrinsic(16.0 / 9.0, IntrinsicQuery::max_width().across(180.0));
        assert!(
            wide.is_some_and(|width| (width - 320.0).abs() < 0.01),
            "180 at 16:9 is 320 wide, got {wide:?}"
        );
    }

    #[test]
    fn the_intrinsic_answer_is_the_size_layout_actually_produces() {
        // **The property worth pinning, and the only reason this implementation
        // is allowed to exist.** An intrinsic that disagrees with `layout` is
        // worse than no intrinsic at all: the parent reserves one number and the
        // child takes another, and the gap shows up as unexplained slack far
        // from either.
        //
        // The constraints below are the ones an intrinsic query describes — one
        // axis pinned to the cross extent, the other left free — so `size_for`
        // is being asked the same question in its own language.
        for ratio in [16.0 / 9.0, 0.5, 1.0, 3.0] {
            for extent in [1.0, 40.0, 137.5, 1000.0] {
                let from_width = intrinsic(ratio, IntrinsicQuery::max_height().across(extent));
                let laid_out = size(ratio, Constraints::new(extent, extent, 0.0, f32::INFINITY));
                assert_eq!(
                    from_width,
                    Some(laid_out.height),
                    "height at {ratio} in {extent} wide: intrinsic and layout disagree"
                );

                let from_height = intrinsic(ratio, IntrinsicQuery::max_width().across(extent));
                let laid_out = size(ratio, Constraints::new(0.0, f32::INFINITY, extent, extent));
                assert_eq!(
                    from_height,
                    Some(laid_out.width),
                    "width at {ratio} in {extent} tall: intrinsic and layout disagree"
                );
            }
        }
    }

    #[test]
    fn a_ratio_has_no_slack_so_its_minimum_equals_its_maximum() {
        // Nothing here reflows. A minimum that came back smaller would let an
        // `IntrinsicWidth` squeeze the box off its ratio, which is the one thing
        // it exists to hold.
        for axis in [IntrinsicQuery::max_width(), IntrinsicQuery::min_width()] {
            assert_eq!(intrinsic(2.0, axis.across(50.0)), Some(100.0), "{axis:?}");
        }
        for axis in [IntrinsicQuery::max_height(), IntrinsicQuery::min_height()] {
            assert_eq!(intrinsic(2.0, axis.across(100.0)), Some(50.0), "{axis:?}");
        }
    }

    #[test]
    fn without_a_cross_extent_the_answer_is_unknown_rather_than_zero() {
        // **The whole point of `Option` here.** `size_for` collapses to
        // `smallest()` when neither axis is bounded, but that is a chosen
        // fallback against infinity, not a natural size — reporting it as
        // `Some(0.0)` would have a caller tighten this box to nothing and the
        // card would disappear from a screen that plain layout renders fine.
        for query in [
            IntrinsicQuery::max_width(),
            IntrinsicQuery::min_width(),
            IntrinsicQuery::max_height(),
            IntrinsicQuery::min_height(),
        ] {
            assert_eq!(intrinsic(16.0 / 9.0, query), None, "{query:?}");
        }
    }

    #[test]
    fn an_infinite_cross_extent_is_the_same_statement_as_none() {
        // And must be caught before the multiplication: infinity times a ratio
        // is infinity, which the tree rewrites to a hard `0.0` — the collapse
        // the test above refuses.
        assert_eq!(
            intrinsic(2.0, IntrinsicQuery::max_width().across(f32::INFINITY)),
            None
        );
    }

    #[test]
    fn a_childless_ratio_still_answers_and_a_silent_child_does_not_poison_it() {
        // This box's size comes from the ratio alone — `layout` hands the child
        // a tight size and gives it no vote — so neither the absence of a child
        // nor a child that cannot measure itself changes anything. A `None`
        // here would make an unmeasurable leaf collapse a whole card.
        let query = IntrinsicQuery::max_height().across(200.0);
        assert_eq!(intrinsic(2.0, query), Some(100.0));
        assert_eq!(intrinsic_with_child(2.0, query), Some(100.0));
        assert_eq!(
            intrinsic_with_child(2.0, IntrinsicQuery::max_height()),
            None
        );
    }

    #[test]
    fn a_substituted_ratio_is_the_one_the_intrinsic_uses_too() {
        // The square from `new` is not only a layout-time fallback: an intrinsic
        // that divided by the `NaN` as asked would hand back a `NaN` the tree
        // turns into zero, and the collapse would be blamed on the parent.
        assert_eq!(
            intrinsic(f32::NAN, IntrinsicQuery::max_width().across(60.0)),
            Some(60.0)
        );
    }
}
