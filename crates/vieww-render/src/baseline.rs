//! Where the text inside a subtree sits, so a row can line its labels up.
//!
//! # The problem this solves
//!
//! A row of a 24pt heading, a 13pt label and an icon, centred on the cross
//! axis, looks wrong in a way that is hard to name and impossible to unsee:
//! the letters do not sit on a common line. Centring aligns the *boxes*, and
//! the boxes are as tall as their fonts' line heights, so a larger font's
//! letters ride high. Every design system that looks expensive aligns the
//! baselines instead, and until now this framework could not — the row had no
//! way to ask a child where its first line of type sits.
//!
//! [`CrossAxisAlignment::Baseline`](vieww_widget::CrossAxisAlignment::Baseline)
//! is the answer, and this module is what makes it answerable.
//!
//! # What a baseline is here
//!
//! A distance **down from the top of the object's own box**, in logical
//! pixels, to the baseline of its first line of text. It is measured after
//! layout, because until a child knows how wide it is it does not know where
//! it wrapped, and a wrapped line moves the first baseline.
//!
//! # `Option`, not zero — again
//!
//! Same rule as [`intrinsic`](crate::RenderObject::intrinsic), for the same
//! reason. `None` means *"there is no text in here"*, not *"the baseline is at
//! the top"*. A row that gets `None` from a child falls back to
//! [`CrossAxisAlignment::Start`](vieww_widget::CrossAxisAlignment::Start) for
//! **that child only**, which is what
//! every toolkit does, and it is the only behaviour that leaves an icon beside a label
//! looking sane. A default of `0.0` would instead hang every icon in the row
//! from its top edge and read as a layout bug somewhere else entirely.
//!
//! # What is transparent and what is not
//!
//! `pass_through_baseline!` is for a render object that places one child and
//! paints it exactly where it placed it — padding, a border, a gesture
//! listener, an opacity layer. The helper adds the child's own offset back in,
//! so padding needs no arithmetic of its own and cannot get it wrong.
//!
//! Three kinds of object deliberately do **not** get it, and answer `None`:
//!
//! - [`RenderTransform`](crate::RenderTransform) and
//!   [`RenderFitted`](crate::RenderFitted) scale or rotate their child. A
//!   rotated paragraph has no horizontal baseline to share, and a scaled one
//!   has a baseline the child does not know about. A proxy that does
//!   forward through its transform, and the result is text that aligns to a
//!   line it is not drawn on.
//! - [`RenderOffstage`](crate::RenderOffstage) hides its child. Aligning to
//!   something invisible is aligning to nothing.
//! - Viewports and scroll views move their content under a window. The
//!   baseline of a scrolled paragraph is wherever it has been scrolled to,
//!   which is not a property of the subtree and goes stale the moment somebody
//!   flicks it.
//!
//! [`CrossAxisAlignment::Start`]: vieww_widget::CrossAxisAlignment::Start

use std::fmt;

use vieww_foundation::{Offset, Size};

use crate::{RenderId, RenderTree};

/// Handed to [`RenderObject::baseline`](crate::RenderObject::baseline).
///
/// Deliberately smaller than [`LayoutCtx`](crate::LayoutCtx): it can read what
/// layout already decided and ask children the same question, and it cannot
/// lay anything out. A baseline query that laid a child out would do it under
/// constraints nobody chose, and the damage would surface a frame later — the
/// same trap [`IntrinsicCtx`](crate::IntrinsicCtx) is shaped to avoid.
pub struct BaselineCtx<'a> {
    pub(crate) tree: &'a mut RenderTree,
    pub(crate) id: RenderId,
}

impl BaselineCtx<'_> {
    /// This object's children, in paint order.
    #[must_use]
    pub fn children(&self) -> &[RenderId] {
        self.tree.children(self.id)
    }

    /// This object's children, copied out — for asking each one a question,
    /// which needs the tree back.
    #[must_use]
    pub fn children_owned(&self) -> Vec<RenderId> {
        self.tree.children(self.id).to_vec()
    }

    /// How many children this object has.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.tree.children(self.id).len()
    }

    /// The size a child settled on in the last layout.
    #[must_use]
    pub fn child_size(&self, child: RenderId) -> Size {
        self.tree.size(child)
    }

    /// Where a child was placed, relative to this object's own top-left.
    #[must_use]
    pub fn child_offset(&self, child: RenderId) -> Offset {
        self.tree.offset(child)
    }

    /// Ask a child where its first baseline is, **in this object's
    /// coordinates** — the child's own answer plus where it was placed.
    ///
    /// Doing the shift here rather than at every call site is what makes
    /// `pass_through_baseline!` a one-liner and what stops a padded label
    /// reporting a baseline that ignores its own padding.
    pub fn child_baseline(&mut self, child: RenderId) -> Option<f32> {
        let inside = self.tree.baseline(child)?;
        Some(inside + self.tree.offset(child).dy)
    }

    /// The baseline of the one child, or `None` for any other number of them.
    ///
    /// The count check is the point: a pass-through that quietly answered for
    /// the *first* of several children would be right until somebody added a
    /// second.
    pub fn only_child_baseline(&mut self) -> Option<f32> {
        let children = self.children();
        let [child] = *children else {
            return None;
        };
        self.child_baseline(child)
    }

    /// The first child that has a baseline at all, in tree order.
    ///
    /// What a stack and a column want: the topmost piece of type is the one a
    /// neighbour should line up with, and a leading icon that answers `None`
    /// should not veto the label behind it.
    pub fn first_child_baseline(&mut self) -> Option<f32> {
        for child in self.children_owned() {
            if let Some(found) = self.child_baseline(child) {
                return Some(found);
            }
        }
        None
    }
}

impl fmt::Debug for BaselineCtx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BaselineCtx")
            .field("id", &self.id)
            .field("children", &self.children().len())
            .finish_non_exhaustive()
    }
}

impl RenderTree {
    /// Where the first line of text inside `id` sits, measured down from the
    /// top of `id`'s own box.
    ///
    /// Only meaningful after `id` has been laid out — the answer depends on
    /// where the text wrapped. `None` means the subtree holds no text, or
    /// holds text whose position cannot honestly be reported (see the module
    /// documentation for which objects refuse and why).
    #[must_use]
    pub fn baseline(&mut self, id: RenderId) -> Option<f32> {
        let size = self.size(id);

        // Taken out for the same reason `layout` and `intrinsic` take it out:
        // the context below borrows the tree mutably to reach the children.
        let object = self.take_object(id)?;

        let mut ctx = BaselineCtx { tree: self, id };
        let answer = object.baseline(&mut ctx, size);
        self.put_object(id, object);

        // A baseline above the top of the box or below its bottom is a bug in
        // the render object rather than a position, and letting it through
        // produces a row that is taller than the sum of its parts for no
        // visible reason. Clamped here so every implementation does not have
        // to — and only when the size is known, since a zero-height box during
        // an early pass would otherwise clamp every answer to nothing.
        answer.map(|value| {
            if value.is_finite() {
                value.clamp(0.0, size.height.max(0.0))
            } else {
                0.0
            }
        })
    }
}

/// The [`baseline`](crate::RenderObject::baseline) implementation for a render
/// object that is transparent to it: one child, placed wherever this object
/// decided, and painted there unmodified.
///
/// The helper adds the child's offset back in, so padding, alignment and a
/// border all get the right answer without doing any arithmetic — and
/// `only_child_baseline` returns `None` for anything that is not exactly one
/// child, so the macro is also the check.
///
/// See the module documentation for the objects that deliberately do not use
/// this, and what they answer instead.
macro_rules! pass_through_baseline {
    () => {
        fn baseline(
            &self,
            ctx: &mut $crate::BaselineCtx<'_>,
            size: vieww_foundation::Size,
        ) -> Option<f32> {
            let _ = size;
            ctx.only_child_baseline()
        }
    };
}

pub(crate) use pass_through_baseline;
