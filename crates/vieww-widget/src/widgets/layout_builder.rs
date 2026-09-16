use std::any::Any;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Constraints, Key, ViewMetrics};

use crate::{
    widget_node_from, BuildContext, ElementState, Handler, Widget, WidgetKind, WidgetNode,
};

/// Reports the constraints its child was offered, and changes nothing else.
///
/// The low-level half of [`LayoutBuilder`], which is what most code should
/// reach for. This is here for the case where the constraints are wanted
/// *without* rebuilding on them — a diagnostic overlay, a test, or a widget
/// that wants to drive its own signal.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::MeasuredConstraints;
///
/// let seen: Rc<Cell<Option<vieww_foundation::Constraints>>> = Rc::new(Cell::new(None));
/// let write = Rc::clone(&seen);
///
/// let probe = MeasuredConstraints::new()
///     .on_constraints(Rc::new(move |c| write.set(Some(c))))
///     .child(Text::new("measured"));
/// ```
///
/// See [`Measured`](crate::Measured) for the other question — *where* the child
/// ended up, rather than how much room it had.
#[derive(Clone)]
pub struct MeasuredConstraints {
    on_constraints: Option<Handler<Constraints>>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl MeasuredConstraints {
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_constraints: None,
            child: None,
            key: None,
        }
    }

    #[must_use]
    pub fn on_constraints(mut self, handler: Handler<Constraints>) -> Self {
        self.on_constraints = Some(handler);
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The handler, for the render layer to install.
    #[must_use]
    pub fn constraints_handler(&self) -> Option<Handler<Constraints>> {
        self.on_constraints.clone()
    }
}

impl Default for MeasuredConstraints {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for MeasuredConstraints {
    fn debug_name(&self) -> &'static str {
        "MeasuredConstraints"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("listening", self.on_constraints.is_some().to_string())]
    }
}

impl fmt::Debug for MeasuredConstraints {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeasuredConstraints")
            .field("listening", &self.on_constraints.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(MeasuredConstraints);

/// What a [`LayoutBuilder`] remembers between frames.
///
/// Public because [`ElementState`] is a public trait and this is one; nothing
/// outside the widget needs to name it.
#[derive(Debug, Default)]
pub struct LayoutBuilderState {
    /// The last constraints layout reported, and the ones `build` will use.
    seen: Option<Constraints>,
    /// Set when `seen` changed, taken by the element tree to schedule the
    /// rebuild that runs the builder again.
    pending: bool,
}

impl ElementState for LayoutBuilderState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

/// Builds a different widget tree depending on how much room there is.
///
/// The answer to breakpoints: a phone layout under a threshold, a desktop one
/// above it, chosen from the space this subtree was actually offered rather
/// than from the size of the window.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::LayoutBuilder;
///
/// let adaptive = LayoutBuilder::new(|constraints| {
///     if constraints.max_width < 600.0 {
///         Text::new("one column").into()
///     } else {
///         Text::new("two columns").into()
///     }
/// });
/// ```
///
/// # No frame is ever shown against the wrong answer
///
/// A frame that laid out and then discovered the constraints had changed does
/// not go to the screen. `FrameSink::layout` rebuilds and re-lays out until the
/// tree is quiet, and only then paints — so a rotation or a window drag never
/// shows a tree built for the previous size.
///
/// That is parity with the classic behaviour, reached differently and more safely. Its
/// `LayoutBuilder` re-enters the build pipeline from *inside* layout, through
/// `invokeLayoutCallback`, which needs a debug-only guard to police what a
/// callback may touch. Nothing re-enters here: a render object holds
/// `&mut RenderTree` and has no handle on the element tree — DESIGN §7 keeps
/// that closed — and the settling happens one level up, in the driver that owns
/// both trees.
///
/// # Two ways it does less work than the classic
///
/// **The seed.** The first build does not wait to be told. It reads
/// [`ViewMetrics`] — the surface — so a builder occupying most of the window is
/// already deciding on the right number before layout has said anything, and
/// the settle loop finds nothing to do. The classic runs its callback every time
/// regardless.
///
/// **Thresholds.** The classic rebuilds on *every* constraint change, including the
/// sub-pixel ones a resize or an animation produces. Given
/// [`breakpoints`](Self::breakpoints) this rebuilds only when the room crosses
/// one of them, so dragging a window from 999 to 998 costs nothing and 601 to
/// 599 costs one pass.
///
/// # Why it does not loop
///
/// A widget that rebuilds from layout is one guard away from a rebuild that
/// never settles — and this framework has the scars: two reverted attempts, and
/// a cap in `FrameDriver` written because of them. Two guards are in the way
/// here. `RenderMeasuredConstraints` reports only when the constraints changed,
/// and this state ignores a report equal to what it already holds. Either alone
/// would do; both are cheap.
///
/// # Container queries
///
/// This is this framework's answer to what CSS calls a container query: a
/// card, a chart, or a sidebar section that lays out differently depending on
/// how much room *its own container* offers it, independent of the viewport
/// — the same widget rendering a wide layout in a spacious panel and a
/// narrow one squeezed into a rail, at any window size. That is exactly "the
/// space this subtree was actually offered" from this doc's very first line:
/// a `LayoutBuilder` never looks at the window, only at the constraints its
/// direct parent gave it, which is what makes it re-usable inside containers
/// of any size without modification.
///
/// This was deliberately not duplicated as a second, differently-named
/// widget: a type whose whole job is "rebuild when the constraints I'm
/// offered cross a threshold" already exists, and giving the same behaviour
/// a second name would only leave two things to keep in sync and a choice
/// between them with no real difference behind it. If what you want instead
/// is a single, named, shared answer to "how much room does the *window*
/// have" — read from many places in the tree at once, not measured locally
/// by each — see [`WindowSizeClass`](crate::WindowSizeClass), which is the
/// other half of this distinction, not a competing way to do this one.
#[derive(Clone)]
pub struct LayoutBuilder {
    builder: Rc<dyn Fn(Constraints) -> WidgetNode>,
    /// Widths that matter, ascending. Empty means every change matters.
    thresholds: Rc<[f32]>,
    key: Option<Key>,
}

impl LayoutBuilder {
    /// Build from the constraints this subtree is offered.
    #[must_use]
    pub fn new(builder: impl Fn(Constraints) -> WidgetNode + 'static) -> Self {
        Self {
            builder: Rc::new(builder),
            thresholds: Rc::from([] as [f32; 0]),
            key: None,
        }
    }

    /// Rebuild only when the available width crosses one of these.
    ///
    /// **The saving the unfiltered rebuild cannot make.** Without this, any
    /// change in constraints rebuilds — which is correct, and is also what makes
    /// the widget a known cost during a window drag or an animated pane, where
    /// the width changes by a fraction of a point every frame and the resulting
    /// tree is identical every time.
    ///
    /// A builder that only asks "which side of 600 am I on" does not need to run
    /// for 999 → 998. Declaring the thresholds says so: the width is reduced to
    /// which band it falls in, and a rebuild happens only when the band changes.
    ///
    /// ```
    /// use vieww_widget::prelude::*;
    /// use vieww_widget::LayoutBuilder;
    ///
    /// let adaptive = LayoutBuilder::new(|c| {
    ///     if c.max_width < 600.0 { Text::new("phone").into() } else { Text::new("desktop").into() }
    /// })
    /// .breakpoints([600.0]);
    /// ```
    ///
    /// Order does not matter; they are sorted here. Heights are not quantised —
    /// a breakpoint is a width idea, and a builder that switches on height
    /// should leave this unset and take the rebuild.
    ///
    /// # The constraints the builder receives are the ones from the last change
    ///
    /// Not the live ones. That follows from not rebuilding: between 999 and 998
    /// nothing runs, so the builder still holds 999. For a threshold decision
    /// that is the same answer. **A builder that also uses the exact width for
    /// geometry — a proportional split, say — wants no thresholds at all**, and
    /// should size that part with `Flexible` or `Align` instead, which cost no
    /// rebuild whatsoever.
    #[must_use]
    pub fn breakpoints(mut self, thresholds: impl IntoIterator<Item = f32>) -> Self {
        let mut widths: Vec<f32> = thresholds.into_iter().collect();
        widths.sort_by(f32::total_cmp);
        self.thresholds = Rc::from(widths);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

/// Which band `width` falls in, as a count of thresholds at or below it.
///
/// A plain count rather than an index, so "below every threshold" is 0 and no
/// `Option` is needed. Two widths in the same band produce the same tree by the
/// caller's own definition, which is what makes skipping the rebuild sound.
fn band(thresholds: &[f32], width: f32) -> usize {
    thresholds
        .iter()
        .filter(|threshold| width >= **threshold)
        .count()
}

impl Widget for LayoutBuilder {
    fn debug_name(&self) -> &'static str {
        "LayoutBuilder"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(LayoutBuilderState::default()))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // What layout last reported, or the surface if it has not spoken yet.
        // The seed is the whole point of the fallback: building against the
        // surface is right for anything that fills the window, which is most of
        // what a breakpoint is asked about, so the common case never shows a
        // wrong tree at all.
        let constraints = ctx
            .state::<LayoutBuilderState, _>(|state| state.seen)
            .flatten()
            .unwrap_or_else(|| Constraints::loose(ctx.inherit_or(ViewMetrics::default()).size));

        let child = (self.builder)(constraints);
        let handle = ctx.state_handle();
        let thresholds = Rc::clone(&self.thresholds);

        MeasuredConstraints::new()
            .on_constraints(Rc::new(move |reported| {
                let Some(state) = &handle else {
                    return;
                };
                let mut state = state.borrow_mut();
                let Some(state) = state.as_any_mut().downcast_mut::<LayoutBuilderState>() else {
                    return;
                };

                // The second of the two guards against the loop, and where
                // `breakpoints` earns its keep.
                //
                // With no thresholds this is exact equality: writing `pending`
                // for constraints already held would ask for a rebuild that
                // produces the identical tree, and that rebuild would report
                // again — the loop, arrived at from the widget side.
                //
                // With thresholds it is coarser on purpose. Two widths in the
                // same band produce the same tree *by the caller's own
                // definition*, so the rebuild is skipped and `seen` is left
                // holding the width from the last real change.
                if let Some(previous) = state.seen {
                    let unchanged = if thresholds.is_empty() {
                        previous == reported
                    } else {
                        band(&thresholds, previous.max_width)
                            == band(&thresholds, reported.max_width)
                    };
                    if unchanged {
                        return;
                    }
                }

                state.seen = Some(reported);
                state.pending = true;
            }))
            .child(child)
            .into()
    }
}

impl fmt::Debug for LayoutBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The builder is an `Rc<dyn Fn>` with nothing printable about it.
        f.debug_struct("LayoutBuilder").finish_non_exhaustive()
    }
}

widget_node_from!(LayoutBuilder);
