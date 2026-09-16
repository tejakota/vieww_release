use std::any::Any;
use std::fmt::Debug;
use std::rc::Rc;
use std::time::Duration;

use vieww_foundation::Key;

use crate::{BuildContext, WidgetNode};

/// An immutable description of part of the UI.
///
/// Implementors fall into one of the shapes described by [`WidgetKind`], which
/// is what lets the element tree walk any widget's children without knowing its
/// concrete type.
///
/// # Contract
///
/// - A widget is immutable once constructed. `&self` methods must be pure.
/// - [`build`](Widget::build) must have no side effects. Phase 2 tracks signal
///   reads during `build` to decide what to mark pending, and that is only sound
///   if a build can be re-run freely.
/// - [`kind`](Widget::kind) must return the same variant every call for a given
///   instance. The *contents* may differ between instances; the variant may not.
pub trait Widget: Any + Debug + 'static {
    /// The name used in debug output and error messages.
    fn debug_name(&self) -> &'static str;

    /// How this widget participates in the tree.
    fn kind(&self) -> WidgetKind<'_>;

    /// Explicit identity for reconciliation. See [`Key`].
    fn key(&self) -> Option<&Key> {
        None
    }

    /// Produce the subtree this widget describes.
    ///
    /// Only called for [`WidgetKind::Composed`]; the default panics because
    /// reaching it means `kind` and `build` disagree, which is a bug in the
    /// widget rather than in user code.
    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let _ = ctx;
        panic!(
            "{} reported a non-Composed WidgetKind but build() was called; \
             kind() and build() must agree",
            self.debug_name()
        )
    }

    /// Fields worth showing in a tree dump, as `(name, value)` pairs.
    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    /// Whether a panic out of this widget's `build` should be caught.
    ///
    /// `true` — the default — for everything that is not itself standing in for
    /// a failure. Return `false` from a widget that a
    /// `ErrorPolicy` mounts *in place of* one, or an
    /// unconditional panic recurses: the substitute is built like any other
    /// widget, fails the same way, is replaced by another substitute, and the
    /// stack runs out. A stack overflow reported instead of the original panic
    /// is strictly worse than the crash the catch exists to prevent.
    ///
    /// # Why the tree asks instead of checking the type
    ///
    /// It used to `downcast_ref::<ErrorPlaceholder>()`, so the guard held for
    /// vieww's own placeholder and **for nothing else**. An application
    /// supplying its own — for branding, a message format, a "report this"
    /// button — got no protection from the recursion at all, and the way it
    /// failed was a stack overflow rather than a message.
    ///
    /// The downcast was doing the right job by naming a type rather than by
    /// asking a question. This is the question.
    fn catches_panics(&self) -> bool {
        true
    }

    /// Durable state for this widget's element, created once when the element
    /// mounts and dropped when it unmounts.
    ///
    /// This is the classic `create_state` shape. The widget itself stays immutable and
    /// disposable — anything that must survive a rebuild lives here, on the
    /// element, which is the whole reason the element tree exists.
    ///
    /// Most widgets return `None`. Reach for it when something needs *tearing
    /// down*: an animation controller, a stream subscription, a timer.
    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        None
    }

    /// For a [`WidgetKind::Inherited`] widget, the child context carrying its
    /// published value; `None` for every other widget.
    ///
    /// This exists because `Inherited<T>` erases `T` the moment it becomes a
    /// `dyn Widget`, and no trait method can hand back an `Rc<T>` without
    /// naming `T`. Letting the widget push its own value into the context
    /// keeps the payload type inside the one place that still knows it.
    fn publish_inherited(&self, ctx: &BuildContext) -> Option<BuildContext> {
        let _ = ctx;
        None
    }

    /// Publish this widget's value into a `Provision` the element already
    /// holds, returning the readers that must rebuild.
    ///
    /// The counterpart to [`publish_inherited`](Self::publish_inherited), and
    /// the reason both exist: the first *creates* a scope and is what a fresh
    /// mount needs; this one *updates* an existing one and is what every frame
    /// after that needs. Keeping the provision means the scope handed to the
    /// subtree is the same `Rc` as last frame, so a subtree that reads nothing
    /// is skipped entirely instead of rebuilt — see `Provision`.
    ///
    /// Returns `None` for every widget that is not an
    /// [`Inherited`](crate::Inherited).
    fn publish_into(&self, provision: &Rc<crate::Provision>) -> Vec<u64> {
        let _ = provision;
        Vec::new()
    }

    /// `true` if this widget publishes a value to its subtree.
    fn publishes(&self) -> bool {
        false
    }

    /// `true` if `other` describes exactly what this widget already describes,
    /// so the element holding it can be left alone.
    ///
    /// The default is `false`, which is always correct and sometimes wasteful —
    /// the same shape as [`Painter::should_repaint`](crate::Painter), and for
    /// the same reason. Override it when a widget's fields are cheap to compare
    /// and comparing them is exact.
    ///
    /// # What this is for
    ///
    /// Reconciliation already skips a subtree when the new widget is the *same
    /// allocation* as the old one, which covers a parent that cloned a child it
    /// did not touch. It cannot cover a parent that rebuilt and constructed the
    /// child afresh — a new `Rc` every time, however identical its contents —
    /// and that is the common case for a region built inside another region's
    /// `build`.
    ///
    /// `viewwstudio`'s gutter is the example this was written for. It is 175
    /// elements of line numbers, it lives inside the code pane because it has
    /// to scroll with the text, and the code pane rebuilds on every keystroke
    /// because it *shows* the text. So the gutter was rebuilt on every
    /// character typed to draw the numbers it already had, and no amount of
    /// narrowing its own subscriptions could help: it was not its own
    /// subscriptions waking it.
    ///
    /// # When it is wrong to implement this
    ///
    /// **Whenever the widget carries a closure over anything that changes.** A
    /// handler captured from the parent's build is part of the description, and
    /// two closures cannot be compared — a widget holding a stale one keeps
    /// calling into last frame's state, which is a correctness bug and a quiet
    /// one. Compare every field or return `false`.
    ///
    /// The same applies to any field left out of the comparison: this is a
    /// claim that *nothing observable* differs, not that the interesting parts
    /// match. An element that returns `true` here keeps the widget it already
    /// had — the new one is dropped — so a field that differs is simply lost.
    ///
    /// `other` is always the same concrete type as `self`: the caller has
    /// already established that through
    /// [`WidgetNode::can_update`](crate::WidgetNode::can_update). Implementors
    /// reach it with
    /// [`WidgetNode::downcast_ref`](crate::WidgetNode::downcast_ref), and a
    /// failed downcast can only mean a bug, so returning `false` there is the
    /// safe answer rather than a panic.
    fn same_configuration(&self, other: &crate::WidgetNode) -> bool {
        let _ = other;
        false
    }
}

/// Durable per-element state, with lifecycle hooks.
///
/// Created by [`Widget::create_state`] when an element mounts. `dispose` runs
/// when the element unmounts — before its children are torn down, matching
/// the standard order, so a parent can still reach a child's state while cleaning
/// up.
///
/// # Reading it back
///
/// A widget's `build` gets at its own state through
/// [`BuildContext::state`](crate::BuildContext::state), which downcasts to the
/// concrete type. That is what [`as_any`](ElementState::as_any) is for: Rust
/// cannot recover a concrete type from a trait object on its own, and this
/// crate's MSRV predates trait upcasting. Every implementation of it is the same
/// one line, `self`.
///
/// # Where the mutation goes
///
/// `build` sees state through a shared reference, deliberately: a build must
/// stay side-effect free, or the dependency tracking that decides what to
/// rebuild is unsound (`docs/DESIGN.md` §1). State changes belong in the hooks
/// below, which run *outside* a build:
/// [`widget_updated`](ElementState::widget_updated) when the description
/// changes, and [`tick`](ElementState::tick) when a frame advances.
pub trait ElementState: Debug {
    /// This state as an `Any`, so `BuildContext::state` can downcast it.
    ///
    /// Implement as `self`, always.
    fn as_any(&self) -> &dyn Any;

    /// The same, mutably, for a *handler* reaching its own state.
    ///
    /// Implement as `self`, always. See
    /// [`take_pending`](Self::take_pending) for the one thing this is for.
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// Called once, immediately after the element is mounted.
    fn mounted(&mut self) {}

    /// The element was reconfigured from a new widget of the same type and key.
    ///
    /// This is the update hook, and it is where a state notices that
    /// what it is animating *towards* has changed. It runs during the build
    /// phase, before the widget's own `build`, so whatever it decides is visible
    /// to that build.
    ///
    /// Not called on mount — [`Widget::create_state`] saw the first widget
    /// already — and not called when reconciliation skipped the element because
    /// the widget was the very same instance.
    fn widget_updated(&mut self, widget: &WidgetNode) {
        let _ = widget;
    }

    /// Advance anything time-driven to `now`.
    ///
    /// Returns `true` if the value changed and this element must rebuild — the
    /// element tree marks it pending rather than the state having to reach the
    /// runtime, which it cannot name. Runs in the frame's animate phase, ahead
    /// of the build that will show the result.
    fn tick(&mut self, now: Duration) -> bool {
        let _ = now;
        false
    }

    /// `true` while this state still needs frames.
    ///
    /// Asked after the build as well as before it, because a build is where a
    /// state finds out it has something new to animate towards.
    fn is_animating(&self) -> bool {
        false
    }

    /// Whether something wrote this state since the last frame asked. Returning
    /// `true` rebuilds the element; the flag is cleared by the asking.
    ///
    /// # The one thing this is for
    ///
    /// *Ephemeral view state a gesture writes* — a press highlight, a hover, a
    /// focus ring. Nothing else, and the distinction is worth holding onto.
    ///
    /// Durable state that a gesture changes still belongs in a `Signal` above
    /// the widget layer, for the reason `docs/DESIGN.md` §7 gives: a text
    /// field's content and a scroll offset are things the *application* owns and
    /// has to be able to read, so a control that hid them would be a control the
    /// application has to fight. A press highlight is the opposite — nobody
    /// wants to own it, and making every call site wire a signal for it is how a
    /// feature ends up unused.
    ///
    /// The build stays side-effect free either way (§1). A handler runs during
    /// input dispatch, not during a build; this hook is polled between the two.
    ///
    /// A state that returns `true` forever rebuilds its element forever, which
    /// looks exactly like a runaway animation. Take the flag, do not read it.
    fn take_pending(&mut self) -> bool {
        false
    }

    /// Called once, immediately before the element is unmounted.
    ///
    /// Release anything that outlives the tree here — subscriptions, timers,
    /// controllers. This is the only teardown hook there is.
    fn dispose(&mut self) {}

    /// This state as a string, so it can survive the tree that holds it.
    ///
    /// `None` — the default — means "nothing worth carrying across", which is
    /// the right answer for every state that is purely about *right now*: a
    /// press highlight, a hover, an animation's position.
    ///
    /// # What this is for
    ///
    /// A live-reloading editor recompiles the screen being previewed and mounts
    /// the result. The widget types come from a freshly `dlopen`ed library, so
    /// their `TypeId`s differ from the old ones and reconciliation cannot match
    /// a single element — the whole subtree is torn down and rebuilt. Everything
    /// the person had done to it goes with it: the tab they had selected, the
    /// row they had expanded, how far they had scrolled. `viewwstudio`'s own
    /// preview caption had to say so out loud — *"state in the previewed screen
    /// resets on every Render"* — and a developer tweaking one colour had to
    /// re-navigate to what they were looking at after every change.
    ///
    /// # Why a string
    ///
    /// Because the two sides of the crossing are two *compilations*: the state
    /// is saved by a type from the old library and restored into a type from
    /// the new one, and nothing structural survives that — not a `TypeId`, not
    /// a `Box<dyn Any>`, not a layout. A string does. Each state chooses its own
    /// format and is the only thing that reads it back, so the format is private
    /// even though the type is not.
    ///
    /// # The contract
    ///
    /// [`restore`](Self::restore) must treat its argument as **untrusted**: it
    /// may come from a different version of the same widget, written before the
    /// field it is being parsed into existed. A state that cannot make sense of
    /// what it is given must leave itself alone and return `false`, never panic
    /// and never half-apply.
    fn snapshot(&self) -> Option<String> {
        None
    }

    /// Put back what [`snapshot`](Self::snapshot) saved. See its contract.
    ///
    /// Returns whether anything was restored, so a caller can report how much
    /// of a screen came back rather than claiming all of it did.
    fn restore(&mut self, saved: &str) -> bool {
        let _ = saved;
        false
    }
}

/// A boxed state is state.
///
/// [`Widget::create_state`] returns a `Box`, and the element tree needs to share
/// the same state between the frame that mutates it and the build that reads it —
/// which means `Rc<RefCell<dyn ElementState>>`. Forwarding through the box is
/// what lets that conversion be a coercion instead of ceremony in every widget
/// that has state. `as_any` deliberately hands back the *inner* type, so a
/// downcast finds `AnimatedContainerState` rather than a box.
impl ElementState for Box<dyn ElementState> {
    fn as_any(&self) -> &dyn Any {
        (**self).as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        (**self).as_any_mut()
    }

    fn mounted(&mut self) {
        (**self).mounted();
    }

    fn widget_updated(&mut self, widget: &WidgetNode) {
        (**self).widget_updated(widget);
    }

    fn tick(&mut self, now: Duration) -> bool {
        (**self).tick(now)
    }

    fn is_animating(&self) -> bool {
        (**self).is_animating()
    }

    fn take_pending(&mut self) -> bool {
        (**self).take_pending()
    }

    fn dispose(&mut self) {
        (**self).dispose();
    }

    fn snapshot(&self) -> Option<String> {
        (**self).snapshot()
    }

    fn restore(&mut self, saved: &str) -> bool {
        (**self).restore(saved)
    }
}

/// How a widget participates in the tree.
///
/// This is the seam between the widget layer and everything below it: the
/// element tree reconciles against these variants, so a new widget type costs
/// nothing as long as it fits one of them.
#[derive(Debug)]
#[non_exhaustive]
pub enum WidgetKind<'w> {
    /// Describes its UI by composing other widgets. Call
    /// [`Widget::build`] to get the subtree.
    ///
    /// `Container` and `Center` are composed: they own no layout logic of their
    /// own, they just assemble widgets that do.
    Composed,

    /// Will own a render object with no children — `Text`, an image, a divider.
    RenderLeaf,

    /// Will own a render object with exactly one child.
    RenderSingleChild(&'w WidgetNode),

    /// Will own a render object with an ordered list of children.
    ///
    /// The order is paint order: later children paint on top.
    RenderMultiChild(&'w [WidgetNode]),

    /// Publishes a value to its whole subtree via [`BuildContext`], and
    /// otherwise renders exactly as its child does.
    Inherited(&'w WidgetNode),
}

impl WidgetKind<'_> {
    /// The children reachable without building, in paint order.
    ///
    /// Empty for [`Composed`](WidgetKind::Composed) — its children only exist
    /// once `build` has run.
    #[must_use]
    pub fn children(&self) -> &[WidgetNode] {
        match self {
            Self::Composed | Self::RenderLeaf => &[],
            Self::RenderSingleChild(child) | Self::Inherited(child) => std::slice::from_ref(child),
            Self::RenderMultiChild(children) => children,
        }
    }

    /// A short tag for tree dumps.
    #[must_use]
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::Composed => "composed",
            Self::RenderLeaf => "render:leaf",
            Self::RenderSingleChild(_) => "render:single",
            Self::RenderMultiChild(_) => "render:multi",
            Self::Inherited(_) => "inherited",
        }
    }
}
