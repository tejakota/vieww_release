use vieww_foundation::{Axis, Constraints, Offset, Size, TextDirection};
use vieww_widget::{CrossAxisAlignment, FlexFit, MainAxisAlignment, MainAxisSize};

use crate::{LayoutCtx, RenderObject};

/// Lays children out along one axis.
///
/// Note this is **not** CSS flexbox. This flex is a much smaller
/// algorithm that fits the constraints protocol directly: lay each child out
/// against the cross-axis constraint with the main axis unbounded, sum the main
/// extents, then distribute whatever is left over according to
/// [`MainAxisAlignment`]. `taffy` was considered here (the roadmap suggests it)
/// and rejected — it implements CSS semantics, which would have to be
/// translated in and out of constraints-down/sizes-up at every boundary.
///
/// # Spacing
///
/// [`spacing`](Self::spacing) is subtracted from the main axis before either
/// of the two things that compete for it — the flexible children's shares and
/// the alignment's slack. That ordering is the whole of its behaviour: a fixed
/// gap that neither an `Expanded` sibling nor `SpaceBetween` can eat into.
///
/// # Flex factors
///
/// A child wrapped in [`Flexible`](vieww_widget::Flexible) takes a share of
/// whatever the inflexible children leave, rather than its natural size. That
/// needs three passes rather than two: size the inflexible children, divide the
/// remainder, then place everything.
///
/// The factor is read off the child through [`RenderObject::flex`] rather than
/// held in a parent-owned slot — see `Flexible` for why, and for the rule that
/// it must be a *direct* child.
///
/// # Reading direction
///
/// [`text_direction`](Self::text_direction) mirrors the **horizontal** axis, and
/// which axis that is depends on [`direction`](Self::direction): in a row it is
/// the main axis, so `MainAxisAlignment::Start` packs children against the right
/// edge; in a column it is the cross axis, so `CrossAxisAlignment::Start` aligns
/// them right. A column's main axis is vertical and never mirrors, because no
/// script this framework targets runs bottom to top.
///
/// The mirroring is applied at **placement only**. Every child is laid out
/// against the same constraints and reports the same size either way, and the
/// children keep their order in the tree — so paint order, hit testing and the
/// order a screen reader walks are untouched. Only where each child lands moves.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFlex {
    pub direction: Axis,
    pub main_axis_alignment: MainAxisAlignment,
    pub cross_axis_alignment: CrossAxisAlignment,
    pub main_axis_size: MainAxisSize,
    /// A fixed gap held between adjacent children, reserved before anything
    /// else divides the main axis. See [`Flex::spacing`](vieww_widget::Flex).
    pub spacing: f32,
    /// Which way the interface reads, mirroring the horizontal axis.
    pub text_direction: TextDirection,
}

impl RenderFlex {
    #[must_use]
    pub const fn new(direction: Axis) -> Self {
        Self {
            direction,
            main_axis_alignment: MainAxisAlignment::Start,
            cross_axis_alignment: CrossAxisAlignment::Center,
            main_axis_size: MainAxisSize::Max,
            spacing: 0.0,
            text_direction: TextDirection::Ltr,
        }
    }

    /// Whether the main axis runs from the right edge towards the left.
    ///
    /// Only a row can: a column's main axis is vertical, and no script this
    /// framework targets runs bottom to top.
    const fn reverses_main(&self) -> bool {
        matches!(
            (self.direction, self.text_direction),
            (Axis::Horizontal, TextDirection::Rtl)
        )
    }

    /// Whether the cross axis is the mirrored one, which happens in a column.
    const fn flips_cross(&self) -> bool {
        matches!(
            (self.direction, self.text_direction),
            (Axis::Vertical, TextDirection::Rtl)
        )
    }

    /// Where a child of `child_main` extent sits, given how far along the main
    /// axis the cursor has walked and how long that axis is.
    ///
    /// Mirroring here rather than by reversing the child list is what keeps
    /// paint order, hit testing and semantics order in tree order — see the
    /// type's own docs.
    fn main_offset(&self, cursor: f32, child_main: f32, main_extent: f32) -> f32 {
        if self.reverses_main() {
            main_extent - cursor - child_main
        } else {
            cursor
        }
    }

    /// Whether baselines have to be measured: the alignment asks for it *and*
    /// there is a baseline to align to.
    ///
    /// A column's cross axis is horizontal and a baseline is a horizontal
    /// line, so there is nothing to align across — see
    /// [`CrossAxisAlignment::Baseline`] for why that degrades to `Start`
    /// rather than panicking.
    const fn aligns_baselines(&self) -> bool {
        self.cross_axis_alignment.needs_baselines() && matches!(self.direction, Axis::Horizontal)
    }

    /// Where a child sits across the axis, given the free space beside it.
    fn cross_offset(&self, cross_free: f32) -> f32 {
        let leading = self.flips_cross();
        match self.cross_axis_alignment {
            // Stretch leaves no free space, so the flip is a no-op for it.
            // Baseline reaches here only in a column, where it is Start — see
            // `aligns_baselines`; a row under Baseline never calls this.
            CrossAxisAlignment::Start
            | CrossAxisAlignment::Stretch
            | CrossAxisAlignment::Baseline => {
                if leading {
                    cross_free
                } else {
                    0.0
                }
            }
            CrossAxisAlignment::Center => cross_free / 2.0,
            CrossAxisAlignment::End => {
                if leading {
                    0.0
                } else {
                    cross_free
                }
            }
        }
    }

    /// How far down a child sits so its own baseline lands on the row's.
    ///
    /// A child that reports no baseline is pushed to the top of the row —
    /// `Start` for that child alone, which is what keeps an icon beside a
    /// label looking like an icon beside a label. See
    /// [`CrossAxisAlignment::Baseline`] for the argument against zero.
    fn baseline_offset(row: f32, child: Option<f32>) -> f32 {
        child.map_or(0.0, |child| (row - child).max(0.0))
    }

    /// The main-axis extent the gaps between `count` children take up.
    ///
    /// Zero children and one child both have no gaps, hence the saturating
    /// subtraction rather than `count - 1`.
    fn spacing_total(&self, count: usize) -> f32 {
        self.spacing * count.saturating_sub(1) as f32
    }

    const fn main(&self, size: Size) -> f32 {
        match self.direction {
            Axis::Horizontal => size.width,
            Axis::Vertical => size.height,
        }
    }

    const fn cross(&self, size: Size) -> f32 {
        match self.direction {
            Axis::Horizontal => size.height,
            Axis::Vertical => size.width,
        }
    }

    const fn size_from(&self, main: f32, cross: f32) -> Size {
        match self.direction {
            Axis::Horizontal => Size::new(main, cross),
            Axis::Vertical => Size::new(cross, main),
        }
    }

    const fn offset_from(&self, main: f32, cross: f32) -> Offset {
        match self.direction {
            Axis::Horizontal => Offset::new(main, cross),
            Axis::Vertical => Offset::new(cross, main),
        }
    }

    /// The constraints a child is laid out against.
    ///
    /// The main axis is left unbounded so each child reports its natural
    /// extent; the cross axis is tight only under
    /// [`CrossAxisAlignment::Stretch`], where children are forced to fill.
    fn child_constraints(&self, constraints: Constraints) -> Constraints {
        let cross_max = match self.direction {
            Axis::Horizontal => constraints.max_height,
            Axis::Vertical => constraints.max_width,
        };
        let stretch =
            self.cross_axis_alignment == CrossAxisAlignment::Stretch && cross_max.is_finite();
        let cross_min = if stretch { cross_max } else { 0.0 };

        match self.direction {
            Axis::Horizontal => Constraints::new(0.0, f32::INFINITY, cross_min, cross_max),
            Axis::Vertical => Constraints::new(cross_min, cross_max, 0.0, f32::INFINITY),
        }
    }

    /// The constraints a flexible child is laid out against.
    ///
    /// `extent` is its share of the leftover space. Under [`FlexFit::Tight`] the
    /// main axis is tight, so the child fills the share whatever it would rather
    /// be; under [`FlexFit::Loose`] the share is only a maximum. The cross axis
    /// is untouched — flex divides one axis, and stretching across the other is
    /// [`CrossAxisAlignment`]'s business.
    fn flex_constraints(&self, loose: Constraints, extent: f32, fit: FlexFit) -> Constraints {
        let min = match fit {
            FlexFit::Tight => extent,
            FlexFit::Loose => 0.0,
        };
        match self.direction {
            Axis::Horizontal => Constraints::new(min, extent, loose.min_height, loose.max_height),
            Axis::Vertical => Constraints::new(loose.min_width, loose.max_width, min, extent),
        }
    }

    /// Leading space before the first child, and the gap between children.
    fn distribute(&self, free: f32, count: usize) -> (f32, f32) {
        let free = free.max(0.0);
        let n = count as f32;
        match self.main_axis_alignment {
            MainAxisAlignment::Start => (0.0, 0.0),
            MainAxisAlignment::End => (free, 0.0),
            MainAxisAlignment::Center => (free / 2.0, 0.0),
            MainAxisAlignment::SpaceBetween => {
                if count < 2 {
                    (0.0, 0.0)
                } else {
                    (0.0, free / (n - 1.0))
                }
            }
            MainAxisAlignment::SpaceAround => {
                if count == 0 {
                    (0.0, 0.0)
                } else {
                    let gap = free / n;
                    (gap / 2.0, gap)
                }
            }
            MainAxisAlignment::SpaceEvenly => {
                if count == 0 {
                    (0.0, 0.0)
                } else {
                    let gap = free / (n + 1.0);
                    (gap, gap)
                }
            }
        }
    }
}

impl RenderFlex {
    /// What a reader wants to know about a row or a column: which way it runs,
    /// how the space was divided, and the gap.
    fn described(&self) -> Vec<(&'static str, String)> {
        vec![
            ("direction", format!("{:?}", self.direction)),
            (
                "main_axis_alignment",
                format!("{:?}", self.main_axis_alignment),
            ),
            (
                "cross_axis_alignment",
                format!("{:?}", self.cross_axis_alignment),
            ),
            ("main_axis_size", format!("{:?}", self.main_axis_size)),
            ("spacing", format!("{:.1}", self.spacing)),
            ("text_direction", format!("{:?}", self.text_direction)),
        ]
    }
}

impl RenderObject for RenderFlex {
    fn describe(&self) -> Vec<(&'static str, String)> {
        self.described()
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let children = ctx.children_owned();
        let child_constraints = self.child_constraints(constraints);
        let main_limit = match self.direction {
            Axis::Horizontal => constraints.max_width,
            Axis::Vertical => constraints.max_height,
        };

        // A share of infinity is not a number. Under an unbounded main axis
        // there is nothing to divide, so every child is laid out at its natural
        // size and the factors are ignored — the same situation as a flex with
        // no flexible children at all. A stricter toolkit asserts here; silently
        // shrink-wrapping is the friendlier answer for a row that has been
        // dropped into a scrollable, which is how this usually happens.
        let distributing = main_limit.is_finite();

        // Pass one: the inflexible children, at their natural size. The
        // flexible ones cannot be laid out yet — their share depends on how
        // much these leave.
        let mut factors = Vec::with_capacity(children.len());
        let mut sizes = vec![Size::ZERO; children.len()];
        let mut inflexible_main = 0.0_f32;
        let mut cross_max = 0.0_f32;
        let mut total_flex = 0_u32;

        // Measured only when the alignment asks for it. A row that centres its
        // children never walks a subtree looking for text, which is what keeps
        // baseline alignment a choice with a cost rather than a tax on every
        // row in the tree.
        let measuring = self.aligns_baselines();
        let mut baselines: Vec<Option<f32>> = vec![None; children.len()];

        for (index, &child) in children.iter().enumerate() {
            let factor = distributing.then(|| ctx.child_flex(child)).flatten();
            factors.push(factor);

            match factor {
                Some(factor) => total_flex += u32::from(factor.flex),
                None => {
                    let size = ctx.layout_child(child, child_constraints);
                    sizes[index] = size;
                    inflexible_main += self.main(size);
                    cross_max = cross_max.max(self.cross(size));
                    if measuring {
                        baselines[index] = ctx.child_baseline(child);
                    }
                }
            }
        }

        // The gaps are not up for division. Taking them off here rather than
        // after means a flexible child shrinks to make room for the spacing,
        // instead of the spacing pushing the last child past the edge.
        let spacing_total = self.spacing_total(children.len());

        // Pass two: divide what is left among the flexible children.
        if total_flex > 0 {
            let free = (main_limit - inflexible_main - spacing_total).max(0.0);
            let share = free / f32::from(u16::try_from(total_flex).unwrap_or(u16::MAX));

            for (index, &child) in children.iter().enumerate() {
                let Some(factor) = factors[index] else {
                    continue;
                };
                let extent = share * f32::from(factor.flex);
                let size = ctx.layout_child(
                    child,
                    self.flex_constraints(child_constraints, extent, factor.fit),
                );
                sizes[index] = size;
                cross_max = cross_max.max(self.cross(size));
                if measuring {
                    baselines[index] = ctx.child_baseline(child);
                }
            }
        }

        // A row of baselines is as tall as the deepest ascent plus the deepest
        // descent, which is taller than the tallest child whenever two
        // different type sizes are in it: a big heading's ascent over a small
        // label's descent needs room for both, and the tallest single box does
        // not have it. Children with no baseline keep contributing their whole
        // height, since they hang from the top.
        let row_baseline = if measuring {
            let above = baselines.iter().flatten().copied().fold(0.0_f32, f32::max);
            let below = baselines
                .iter()
                .zip(&sizes)
                .filter_map(|(baseline, size)| baseline.map(|at| self.cross(*size) - at))
                .fold(0.0_f32, f32::max);
            cross_max = cross_max.max(above + below);
            above
        } else {
            0.0
        };

        // Spacing counts as used space, which is what keeps it out of the
        // alignment's hands: `distribute` below is handed what is left *after*
        // the gaps, so SpaceBetween and friends add their slack on top of the
        // fixed gaps rather than absorbing them.
        let children_main: f32 = sizes.iter().map(|size| self.main(*size)).sum();
        let main_used = children_main + spacing_total;

        // The flex's own size: fill the main axis when asked to and able,
        // otherwise shrink-wrap.
        let main_extent = match self.main_axis_size {
            MainAxisSize::Max if main_limit.is_finite() => main_limit,
            _ => main_used,
        };
        let size = constraints.constrain(self.size_from(main_extent, cross_max));

        // Pass two: place children in the space now known. `main_span` is the
        // extent after constraining, which is what a mirrored placement counts
        // back from — the `main_extent` above is the extent that was *asked*
        // for, and a tight parent can have refused it.
        let main_span = self.main(size);

        // Children that do not fit are still placed — there is nowhere else to
        // put them — and nothing clips at the surface, so they are simply drawn
        // off the edge and vanish. That reads as a widget which was never built,
        // which sends the search to the wrong place entirely, so say it.
        //
        // A list inside a scrollable does not reach here with anything to
        // report: a viewport hands its child infinite room on the scroll axis,
        // so `main_used` and `main_span` agree however long the content is.
        if let Some(over) = crate::overflow::beyond(main_used, main_span) {
            crate::overflow::report(
                self.debug_name(),
                self.direction,
                main_used,
                main_span,
                over,
            );
        }

        let (leading, gap) = self.distribute(main_span - main_used, children.len());
        let cross_extent = self.cross(size);
        let mut main_cursor = leading;

        for (index, (&child, child_size)) in children.iter().zip(&sizes).enumerate() {
            let child_main = self.main(*child_size);
            let cross_offset = if measuring {
                Self::baseline_offset(row_baseline, baselines[index])
            } else {
                self.cross_offset(cross_extent - self.cross(*child_size))
            };
            let main_offset = self.main_offset(main_cursor, child_main, main_span);
            ctx.place_child(child, self.offset_from(main_offset, cross_offset));
            main_cursor += child_main + gap + self.spacing;
        }

        size
    }

    /// The first child that has one, offset by where that child was placed.
    ///
    /// A row nested in a row — a label-and-value pair inside a toolbar — is
    /// exactly the case baseline alignment is for, and it only composes if a
    /// row can answer the question it asks its own children. Under
    /// [`CrossAxisAlignment::Baseline`] every child that answered shares one
    /// line anyway, so *first* and *any* are the same answer; under the other
    /// alignments the first child is the one a reader's eye starts at.
    fn baseline(&self, ctx: &mut crate::BaselineCtx<'_>, _size: Size) -> Option<f32> {
        ctx.first_child_baseline()
    }

    /// # Along the main axis
    ///
    /// The children's answers plus the spacing between them. That is exactly
    /// right for the inflexible children and it is a *lower bound* for the
    /// flexible ones — a flex factor says "take a share of the leftover", and
    /// there is no leftover to share until someone has decided how much room
    /// this flex gets, which is the question being asked. Reporting the
    /// natural sum is the same answer `layout` produces under an unbounded main
    /// axis, so the two agree.
    ///
    /// # Along the cross axis
    ///
    /// The largest child's answer. A row is as tall as its tallest cell.
    ///
    /// # The cross extent handed down
    ///
    /// Only propagated on the cross axis, where every child genuinely gets the
    /// same extent. On the main axis the children *divide* what is available,
    /// and there is no way to say what each one's share is without laying them
    /// out — so `cross` is dropped rather than passed to each child as if it
    /// were the whole thing, which would ask a row's five children how tall
    /// they are at the row's full width and get five over-answers.
    fn intrinsic(
        &self,
        ctx: &mut crate::IntrinsicCtx<'_>,
        query: crate::IntrinsicQuery,
    ) -> Option<f32> {
        let children = ctx.children_owned();
        if children.is_empty() {
            return Some(0.0);
        }

        let along_main = query.axis == self.direction;
        let mut child_query = query;
        if along_main {
            child_query.cross = None;
        }

        let answers: Vec<Option<f32>> = children
            .iter()
            .map(|&child| ctx.child_intrinsic(child, child_query))
            .collect();

        if along_main {
            let gaps = self.spacing * (children.len() - 1) as f32;
            crate::sum(answers).map(|total| total + gaps)
        } else {
            crate::largest(answers)
        }
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        crate::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        match self.direction {
            Axis::Horizontal => "RenderRow",
            Axis::Vertical => "RenderColumn",
        }
    }
}
