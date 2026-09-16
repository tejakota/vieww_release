use vieww_foundation::{Alignment, Constraints, Offset, Size};
use vieww_widget::StackFit;

use crate::{LayoutCtx, RenderObject};

/// Overlays its children, sizing itself to the largest **unpositioned** one.
///
/// A child wrapped in [`Positioned`](vieww_widget::Positioned) is left out of
/// that measurement and placed against the stack's edges afterwards, which is
/// what lets a badge hang off a corner without making the stack bigger. See
/// [`StackPosition`](vieww_widget::StackPosition) for how each axis resolves.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderStack {
    pub alignment: Alignment,
    pub fit: StackFit,
}

impl RenderStack {
    #[must_use]
    pub const fn new(alignment: Alignment, fit: StackFit) -> Self {
        Self { alignment, fit }
    }

    /// The size a stack takes when nothing in it has a natural size — every
    /// child positioned, or no children at all that measure.
    ///
    /// The largest the constraints allow, which is the classic rule, **with one
    /// difference**: the classic reads `constraints.biggest` unguarded, so an axis
    /// its parent left unbounded comes back infinite. An infinite size survives
    /// `constrain` and then propagates into every ancestor's arithmetic, which
    /// is the failure `RenderAspectRatio` documents at length. An unbounded axis
    /// falls back to its **minimum** here instead: a stack that collapses is
    /// visible and local, where one of infinite extent is neither.
    fn size_without_unpositioned(constraints: Constraints) -> Size {
        Size::new(
            if constraints.has_bounded_width() {
                constraints.max_width
            } else {
                constraints.min_width
            },
            if constraints.has_bounded_height() {
                constraints.max_height
            } else {
                constraints.min_height
            },
        )
    }
}

impl RenderObject for RenderStack {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let children = ctx.children_owned();
        if children.is_empty() {
            return constraints.smallest();
        }

        let child_constraints = match self.fit {
            StackFit::Loose => constraints.loosen(),
            // **Tight only where there is something to be tight against.**
            // `biggest()` is infinite on an unbounded axis, and tightening a
            // child to infinity makes a child of infinite size — which survives
            // `constrain`, propagates into every ancestor, and paints nothing at
            // all. This is the same failure `size_without_unpositioned` above
            // was written to avoid, one branch along: a `Tooltip` is a
            // full-screen overlay built on `Expand`, and dropping one into a row
            // — where the main axis is unbounded — turned the whole application
            // black with no error beyond an `inf` in the overflow report.
            //
            // On an unbounded axis the child is left to shrink-wrap instead. It
            // is the only finite reading of "as large as you are allowed" when
            // what you are allowed is unlimited, and it keeps the failure local
            // to the widget that asked for the impossible.
            StackFit::Expand => Constraints::new(
                if constraints.has_bounded_width() {
                    constraints.max_width
                } else {
                    constraints.min_width
                },
                constraints.max_width,
                if constraints.has_bounded_height() {
                    constraints.max_height
                } else {
                    constraints.min_height
                },
                constraints.max_height,
            ),
            StackFit::Passthrough => constraints,
        };

        // Pass one: the unpositioned children, which are the only ones that
        // decide how big the stack is. A positioned child is measured against
        // edges that do not exist yet, so it cannot be laid out here.
        let mut widest = 0.0_f32;
        let mut tallest = 0.0_f32;
        let mut any_unpositioned = false;
        let mut positions = Vec::with_capacity(children.len());
        let mut sizes = vec![Size::ZERO; children.len()];
        for (index, &child) in children.iter().enumerate() {
            let position = ctx.child_stack_position(child);
            positions.push(position);
            if position.is_none() {
                any_unpositioned = true;
                let size = ctx.layout_child(child, child_constraints);
                widest = widest.max(size.width);
                tallest = tallest.max(size.height);
                sizes[index] = size;
            }
        }

        let size = if any_unpositioned {
            constraints.constrain(Size::new(widest, tallest))
        } else {
            Self::size_without_unpositioned(constraints)
        };

        // Pass two: the positioned children, now that there is something for
        // their edges to be measured against.
        for (index, &child) in children.iter().enumerate() {
            if let Some(position) = positions[index] {
                // From unbounded rather than from the stack's own constraints:
                // an axis this does not pin leaves the child at its natural
                // size. That is the classic rule and it has a sharp edge worth
                // knowing — a `Text` pinned on one edge only is laid out
                // unbounded and will not wrap, exactly as inside a `Fitted`.
                let pinned = Constraints::UNBOUNDED.tighten(
                    position.width_within(size.width),
                    position.height_within(size.height),
                );
                sizes[index] = ctx.layout_child(child, pinned);
            }
        }

        // Placement happens last because both the stack's alignment and a
        // right- or bottom-pinned edge need the final size, which is not known
        // until every unpositioned child has reported.
        for (index, &child) in children.iter().enumerate() {
            let child_size = sizes[index];
            let aligned = self.alignment.inscribe(child_size, size);
            let offset = match positions[index] {
                // Per axis, so a child pinned to the bottom is still placed
                // horizontally by the stack's alignment.
                Some(position) => Offset::new(
                    position.x_within(size.width, child_size.width, aligned.dx),
                    position.y_within(size.height, child_size.height, aligned.dy),
                ),
                None => aligned,
            };
            ctx.place_child(child, offset);
        }

        size
    }

    /// The first child that has one, in paint order — the bottom of the pile.
    ///
    /// A stack is used for overlays: a badge over an avatar, a caption over an
    /// image. The thing a neighbour should line up with is the one the stack
    /// was built around, which is the child it was given first; a badge that
    /// happens to contain a digit should not drag the row's baseline to it.
    fn baseline(
        &self,
        ctx: &mut crate::BaselineCtx<'_>,
        _size: vieww_foundation::Size,
    ) -> Option<f32> {
        ctx.first_child_baseline()
    }

    /// The largest of the **non-positioned** children.
    ///
    /// Positioned children are excluded for the same reason `layout` excludes
    /// them from sizing the stack: their extent is a function of the stack's
    /// size, so including them here would be circular — a `Positioned` with
    /// `left` and `right` set is as wide as whatever it is put in.
    ///
    /// A stack of only positioned children reports zero, which is what it
    /// shrink-wraps to.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        let unpositioned: Vec<_> = ctx
            .children_owned()
            .into_iter()
            .filter(|&child| ctx.child_stack_position(child).is_none())
            .collect();

        if unpositioned.is_empty() {
            return Some(0.0);
        }

        let answers: Vec<Option<f32>> = unpositioned
            .into_iter()
            .map(|child| ctx.child_intrinsic(child, query))
            .collect();
        crate::largest(answers)
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderStack"
    }
}

#[cfg(test)]
mod tests {
    use vieww_foundation::TextDirection;
    use vieww_foundation::{Color, Rect};
    use vieww_widget::{
        children, Center, ColoredBox, Directionality, Positioned, PositionedDirectional, SizedBox,
        Stack, WidgetNode,
    };

    use super::*;
    use crate::FrameDriver;

    const BACKDROP: Color = Color::rgb(10, 20, 30);
    const BADGE: Color = Color::rgb(40, 50, 60);

    /// One frame of `root` on a `surface`-sized driver.
    fn drawn(surface: Size, root: impl Into<WidgetNode>) -> FrameDriver {
        let mut driver = FrameDriver::new(surface);
        driver.set_root(root);
        driver.draw_frame();
        driver
    }

    /// Where a colour landed, in surface coordinates.
    fn rect_of(driver: &FrameDriver, color: Color) -> Rect {
        driver
            .scene()
            .fills()
            .into_iter()
            .find(|(_, paint)| paint.color == color)
            .map(|(rect, _)| rect)
            .expect("the box is painted")
    }

    /// A `size`-square box in `color`.
    fn square(size: f32, color: Color) -> WidgetNode {
        SizedBox::square(size).child(ColoredBox::new(color)).into()
    }

    // ------------------------------------------------- the behaviour that was

    #[test]
    fn unpositioned_children_still_size_the_stack_and_share_its_corner() {
        // **A regression guard for behaviour that had none.** `RenderStack` had
        // no tests at all before `Positioned` was added to it, so this pins what
        // it already did: size to the largest child, place every child at the
        // stack's alignment.
        //
        // The `Center` is what makes the stack's own size observable — at the
        // root it would be laid out tight and fill the surface whatever its
        // children did.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(
                Stack::new().children(children![square(100.0, BACKDROP), square(40.0, BADGE),]),
            ),
        );

        let backdrop = rect_of(&driver, BACKDROP);
        let badge = rect_of(&driver, BADGE);

        assert_eq!((backdrop.width(), backdrop.height()), (100.0, 100.0));
        assert_eq!(
            (backdrop.left, backdrop.top),
            (50.0, 50.0),
            "a 100pt stack centred on a 200pt surface"
        );
        assert_eq!(
            (badge.left, badge.top),
            (50.0, 50.0),
            "TOP_LEFT is the default, so both children start at the same corner"
        );
    }

    #[test]
    fn an_expanding_stack_on_an_unbounded_axis_wraps_its_child_rather_than_growing_forever() {
        // Found on a screen. `Tooltip` is a full-screen overlay built on
        // `StackFit::Expand`, and one dropped into a `Flex::row` — where the
        // main axis is unbounded — tightened its child to an infinite width. The
        // infinity survived `constrain`, reached the root, and the application
        // drew **nothing at all**: a black window, whose only trace was an
        // `inf` in the overflow report.
        //
        // A row with `MainAxisSize::Min` is the real shape of that mistake, so
        // it is the shape tested, rather than a synthetic constraint.
        let driver = drawn(
            Size::new(200.0, 200.0),
            vieww_widget::Flex::row()
                .main_axis_size(vieww_widget::MainAxisSize::Min)
                // Start rather than the default stretch, so the *cross* axis is
                // not doing the work: the claim is about the unbounded one.
                .cross_axis_alignment(vieww_widget::CrossAxisAlignment::Start)
                .children(children![vieww_widget::WidgetNode::from(
                    Stack::new()
                        .fit(vieww_widget::StackFit::Expand)
                        .children(children![square(40.0, BADGE)])
                )]),
        );

        let badge = rect_of(&driver, BADGE);
        assert_eq!(
            badge.width(),
            40.0,
            "the child keeps its own width when there is no bound to expand to"
        );
        // The cross axis is bounded by the surface, so filling it is correct and
        // is what the test below pins deliberately. What must never happen is
        // this: an extent nothing can paint.
        assert!(
            badge.height().is_finite(),
            "the bounded axis is still finite: {}",
            badge.height()
        );
    }

    #[test]
    fn an_expanding_stack_still_fills_a_bounded_axis() {
        // The other half, and the reason the fix is per-axis rather than a
        // blanket loosen: expanding to fill is what `StackFit::Expand` is *for*,
        // and a fix that gave that up would break every overlay it exists for.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Stack::new()
                .fit(vieww_widget::StackFit::Expand)
                .children(children![WidgetNode::from(ColoredBox::new(BADGE))]),
        );

        let badge = rect_of(&driver, BADGE);
        assert_eq!(
            (badge.width(), badge.height()),
            (200.0, 200.0),
            "a bounded stack still hands its child the whole surface"
        );
    }

    // ------------------------------------------------------ what it does now

    #[test]
    fn a_positioned_child_does_not_size_the_stack_and_hangs_off_its_corner() {
        // **The claim the whole feature rests on.** The badge is placed against
        // an edge that the backdrop alone decided, so a corner ornament cannot
        // make its own stack bigger.
        //
        // The badge's position is also the proof: 140 is `50 + 100 − 0 − 10`.
        // Had the badge been allowed to size the stack, or had `right` been
        // measured from the wrong edge, it would land somewhere else entirely.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(Stack::new().children(children![
                square(100.0, BACKDROP),
                Positioned::new()
                    .right(0.0)
                    .bottom(0.0)
                    .child(square(10.0, BADGE)),
            ])),
        );

        let backdrop = rect_of(&driver, BACKDROP);
        let badge = rect_of(&driver, BADGE);

        assert_eq!(
            (backdrop.width(), backdrop.height()),
            (100.0, 100.0),
            "the stack is still the backdrop's size"
        );
        assert_eq!((badge.width(), badge.height()), (10.0, 10.0));
        assert_eq!(
            (badge.left, badge.top),
            (140.0, 140.0),
            "pinned to the stack's bottom right, not the surface's"
        );
    }

    #[test]
    fn pinning_both_edges_stretches_the_child_between_them() {
        // The case where the child has no size of its own and gets one entirely
        // from its edges — a scrim inset from every side.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(Stack::new().children(children![
                square(100.0, BACKDROP),
                Positioned::new()
                    .left(10.0)
                    .top(20.0)
                    .right(30.0)
                    .bottom(40.0)
                    .child(ColoredBox::new(BADGE)),
            ])),
        );

        let badge = rect_of(&driver, BADGE);

        assert_eq!(
            (badge.width(), badge.height()),
            (60.0, 40.0),
            "100 − 10 − 30 across, 100 − 20 − 40 down"
        );
        assert_eq!(
            (badge.left, badge.top),
            (60.0, 70.0),
            "the stack's own origin is (50, 50), so the insets land here"
        );
    }

    #[test]
    fn an_axis_that_pins_nothing_falls_back_to_the_stacks_alignment() {
        // **Why positioning is resolved per axis rather than per child.** This
        // child says only "at the bottom"; where it sits horizontally is still
        // the stack's business, and centring it is the useful answer.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(
                Stack::new()
                    .alignment(Alignment::CENTER)
                    .children(children![
                        square(100.0, BACKDROP),
                        Positioned::new().bottom(0.0).child(square(20.0, BADGE)),
                    ]),
            ),
        );

        let badge = rect_of(&driver, BADGE);

        assert_eq!(
            badge.left, 90.0,
            "centred across the stack: 50 + (100 − 20) / 2"
        );
        assert_eq!(
            badge.top, 130.0,
            "and hard against the bottom: 50 + 100 − 20"
        );
    }

    #[test]
    fn a_positioned_that_pins_nothing_is_an_ordinary_child() {
        // A position built up conditionally can end up empty, and the useful
        // answer is "behave normally" rather than "jump to a corner". This is
        // also what keeps the wrapper inert outside a stack.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(
                Stack::new().children(children![Positioned::new().child(square(80.0, BACKDROP))]),
            ),
        );

        let backdrop = rect_of(&driver, BACKDROP);

        assert_eq!(
            (backdrop.width(), backdrop.height()),
            (80.0, 80.0),
            "it sized the stack, which a positioned child never does"
        );
        assert_eq!((backdrop.left, backdrop.top), (60.0, 60.0));
    }

    #[test]
    fn a_stack_of_only_positioned_children_takes_the_largest_it_may() {
        // With nothing to measure, the stack has no natural size and expands.
        // If it had instead shrunk to its positioned child, the badge would be
        // centred at (95, 95) rather than sitting at the origin.
        let driver = drawn(
            Size::new(200.0, 200.0),
            Center::new().child(Stack::new().children(children![Positioned::new()
                .left(0.0)
                .top(0.0)
                .child(square(10.0, BADGE))])),
        );

        let badge = rect_of(&driver, BADGE);
        assert_eq!(
            (badge.left, badge.top),
            (0.0, 0.0),
            "the stack filled the surface, so its origin is the surface's"
        );
    }

    #[test]
    fn a_directional_position_reaches_the_stack_through_its_composed_wrapper() {
        // **Two claims at once, and the first is the one that could quietly be
        // false.** `PositionedDirectional` is `Composed`, so it produces no
        // render object of its own; what `RenderStack` must see is the ordinary
        // `Positioned` it builds, sitting exactly where the wrapper was. If a
        // composed widget did leave anything in the render tree, the stack would
        // read `None` off it and treat the badge as an ordinary child — sizing
        // the stack and landing in a corner.
        //
        // The second is the flip itself: `end` is the right edge reading left to
        // right and the left edge reading right to left.
        let badge_at = |direction: TextDirection| {
            let driver = drawn(
                Size::new(200.0, 200.0),
                Directionality::new(direction).child(Center::new().child(Stack::new().children(
                    children![
                        square(100.0, BACKDROP),
                        PositionedDirectional::new()
                            .end(0.0)
                            .bottom(0.0)
                            .child(square(10.0, BADGE)),
                    ],
                ))),
            );
            let backdrop = rect_of(&driver, BACKDROP);
            assert_eq!(
                (backdrop.width(), backdrop.height()),
                (100.0, 100.0),
                "the badge still sized nothing, so it did reach the stack as a \
                 positioned child"
            );
            let badge = rect_of(&driver, BADGE);
            (badge.left, badge.top)
        };

        assert_eq!(
            badge_at(TextDirection::Ltr),
            (140.0, 140.0),
            "end is the right edge: 50 + 100 − 0 − 10"
        );
        assert_eq!(
            badge_at(TextDirection::Rtl),
            (50.0, 140.0),
            "end is the left edge, so the badge crosses to the stack's near side \
             while its vertical pin does not move"
        );
    }

    // ------------------------------------------- the size nobody can observe

    #[test]
    fn an_unbounded_axis_collapses_instead_of_growing_forever() {
        // Reading `constraints.biggest` here, on an unbounded axis,
        // is infinity — a size that survives `constrain` and then poisons every
        // ancestor's arithmetic. Tested directly because a tree cannot show it:
        // by the time an infinity reaches the scene there is nothing left to
        // assert against.
        let bounded = Constraints::loose(Size::new(300.0, 400.0));
        assert_eq!(
            RenderStack::size_without_unpositioned(bounded),
            Size::new(300.0, 400.0),
            "bounded on both axes, so the largest allowed"
        );

        let tall = Constraints::new(0.0, 300.0, 0.0, f32::INFINITY);
        assert_eq!(
            RenderStack::size_without_unpositioned(tall),
            Size::new(300.0, 0.0),
            "the unbounded axis takes its minimum rather than infinity"
        );

        let both = Constraints::new(20.0, f32::INFINITY, 30.0, f32::INFINITY);
        let size = RenderStack::size_without_unpositioned(both);
        assert_eq!(
            size,
            Size::new(20.0, 30.0),
            "and a minimum is still honoured"
        );
        assert!(size.width.is_finite() && size.height.is_finite());
    }
}
