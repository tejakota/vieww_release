//! A place at the root of the surface that anything, anywhere, can draw into.
//!
//! # The failure this exists to remove
//!
//! A menu, a dropdown list, a tooltip and a scrim are all the same shape: they
//! are decided **deep** in the tree and have to be drawn at the **top** of it.
//! Before this, the framework had no way to express that, so every overlay in it
//! was built where it was decided and every one of them was wrong in the same
//! two ways:
//!
//! - **It is clipped by whatever it happens to sit in.** A scrollable clips to
//!   its viewport, so a list opened from a row inside one is cut off or gone.
//! - **It is laid out against its parent rather than the window.** [`Menu`] and
//!   [`Tooltip`](crate::Tooltip) place themselves in *window* coordinates —
//!   which is the only sane space for "below the control, flipped up if there is
//!   no room" — while being handed the constraints of a 220×49 box. The panel
//!   lands hundreds of points outside its own parent, and the scrim covers the
//!   control instead of the screen.
//!
//! `examples/controls` worked around it by hoisting its menu into the root
//! `Stack` by hand, and [`Dropdown`](crate::Dropdown) could not, because a
//! dropdown that made every call site hoist its own list would be a worse
//! dropdown than the one it is shaped after. That was the whole of the
//! "`Dropdown` does not open on screen" report.
//!
//! # How it works, and why there is no frame of lag
//!
//! [`Overlay`] publishes an [`OverlayHandle`] to everything below it and builds
//! a `Stack` of its child plus whatever entries the handle currently holds. A
//! descendant calls [`OverlayHandle::show`] during its own build, which is
//! *after* the overlay above it has already built — so the entry would normally
//! appear one frame late, which is exactly the concession `docs/AIMS.md`
//! refuses.
//!
//! It does not, because the framework already solved this one level down.
//! `FrameSink::layout` rebuilds and re-lays out until the tree is quiet
//! **before painting**, polling [`ElementState`](crate::ElementState) as it
//! goes — the mechanism `LayoutBuilder` uses to settle a tree built from
//! constraints inside the frame that discovered them. An overlay request sets a
//! flag on the `Overlay`'s own state, `poll_states` sees it, and the overlay
//! rebuilds in the same frame. Nothing half-finished reaches a screen.
//!
//! # Why a request, and not a signal
//!
//! A build must stay side-effect free, because that is what makes the
//! dependency tracking sound (`docs/DESIGN.md` §1). [`OverlayHandle::show`]
//! writes no signal and reads none: it appends to a plain cell that the
//! overlay's state polls between phases, which is the same channel
//! `MeasuredConstraints` reports through and touches no subscription at all.
//!
//! # Entries are sticky, and are withdrawn by their owner
//!
//! An entry stays until the widget that asked withdraws it. That is what makes
//! this work at all: the contributor rebuilds far less often than the overlay
//! does, so an entry that had to be re-asserted every frame would flicker out
//! the moment anything else changed.
//!
//! It also means an entry outlives a contributor that is removed from the tree
//! while its overlay is up — a dropdown scrolled out of a list with its menu
//! open. [`OverlayLease`] is the answer: it withdraws on `Drop`, so the
//! contributor holds one in its `ElementState` and `dispose` does the rest
//! without anybody having to remember.
//!
//! # Order
//!
//! Entries are drawn in the order they were first shown, above the child and
//! below anything shown later. Re-showing the same [`OverlayId`] replaces its
//! builder **in place** rather than raising it, so a dropdown that rebuilds
//! while a dialog is over it does not jump in front of the dialog.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::Key;

use crate::{
    widget_node_from, BuildContext, ElementState, Inherited, Stack, StackFit, Widget, WidgetKind,
    WidgetNode,
};

/// Identifies one overlay entry for its whole life.
///
/// Minted once by the widget that owns the entry and kept in its state — a
/// fresh id per build would stack a new copy of the same menu on every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

impl OverlayId {
    /// A new id, unique within the process.
    #[must_use]
    #[allow(clippy::new_without_default)] // `Default` would read as "the id zero".
    pub fn new() -> Self {
        use std::cell::Cell;
        thread_local! {
            static NEXT: Cell<u64> = const { Cell::new(1) };
        }
        NEXT.with(|next| {
            let id = next.get();
            next.set(id + 1);
            Self(id)
        })
    }
}

/// Builds an entry's widget, every time the overlay rebuilds.
///
/// A closure rather than a `WidgetNode`, and that is the load-bearing half: a
/// node captured when the entry was shown is a **snapshot**, so a dropdown list
/// would keep the tick beside whichever option was current at the moment it
/// opened. The closure runs inside the overlay's build, so anything it reads
/// subscribes the overlay and the entry is never stale.
pub type OverlayBuilder = Rc<dyn Fn() -> WidgetNode>;

/// What the overlay is currently showing.
#[derive(Default)]
struct Entries {
    /// In the order they were first shown; the last is drawn on top.
    shown: Vec<(OverlayId, OverlayBuilder)>,
    /// Set by any change, taken by the overlay's state.
    changed: bool,
}

impl Entries {
    fn show(&mut self, id: OverlayId, build: OverlayBuilder) {
        match self.shown.iter_mut().find(|(existing, _)| *existing == id) {
            // Replaced in place rather than moved to the end — see the module
            // docs on order.
            Some(slot) => slot.1 = build,
            None => self.shown.push((id, build)),
        }
        self.changed = true;
    }

    fn hide(&mut self, id: OverlayId) {
        let before = self.shown.len();
        self.shown.retain(|(existing, _)| *existing != id);
        // Only when something actually went. Hiding what was never shown is
        // ordinary — `dispose` runs whether or not an entry was up — and
        // marking the overlay pending for it is a rebuild per teardown.
        self.changed |= self.shown.len() != before;
    }
}

impl fmt::Debug for Entries {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entries")
            .field("shown", &self.shown.len())
            .field("changed", &self.changed)
            .finish()
    }
}

/// The handle a descendant reaches the overlay through.
///
/// Read from a [`BuildContext`] with `ctx.inherit::<OverlayHandle>()`. Cheap to
/// clone, and safe to keep in an `ElementState` — holding one does not keep the
/// overlay alive, and writing through a handle whose overlay is gone is a write
/// nothing will read.
#[derive(Clone)]
pub struct OverlayHandle {
    entries: Rc<RefCell<Entries>>,
}

impl OverlayHandle {
    /// A handle attached to nothing, for a tree built without an [`Overlay`].
    ///
    /// Writes through it are discarded. It exists so a widget that wants an
    /// overlay has one uniform path rather than an `Option` at every call —
    /// [`Overlay::is_present`] is how to ask whether anything will come of it.
    #[must_use]
    pub fn detached() -> Self {
        Self {
            entries: Rc::new(RefCell::new(Entries::default())),
        }
    }

    /// Show `build`'s widget above everything, or replace what `id` shows.
    ///
    /// Safe to call from a build: it writes no signal and reads none, so it
    /// creates no dependency and invalidates none. See the module docs.
    pub fn show(&self, id: OverlayId, build: OverlayBuilder) {
        self.entries.borrow_mut().show(id, build);
    }

    /// Take `id`'s entry down. Hiding what is not shown does nothing.
    pub fn hide(&self, id: OverlayId) {
        self.entries.borrow_mut().hide(id);
    }

    /// An entry that withdraws itself when the returned lease is dropped.
    ///
    /// The form to prefer. A contributor keeps the lease in its
    /// [`ElementState`](crate::ElementState) and gets correct teardown from
    /// `dispose` without writing any, which is the case a hand-written `hide`
    /// is most likely to miss — a control removed from the tree while its
    /// overlay is still up.
    #[must_use]
    pub fn lease(&self, id: OverlayId) -> OverlayLease {
        OverlayLease {
            handle: self.clone(),
            id,
        }
    }

    /// How many entries are up. For tests and for `debug_properties`.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.borrow().shown.len()
    }

    /// `true` when nothing is showing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for OverlayHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverlayHandle")
            .field("shown", &self.len())
            .finish()
    }
}

/// An overlay entry, withdrawn when this is dropped.
///
/// See [`OverlayHandle::lease`].
#[derive(Debug)]
pub struct OverlayLease {
    handle: OverlayHandle,
    id: OverlayId,
}

impl OverlayLease {
    /// The id this lease withdraws.
    #[must_use]
    pub const fn id(&self) -> OverlayId {
        self.id
    }

    /// Show or replace what this lease holds.
    pub fn show(&self, build: OverlayBuilder) {
        self.handle.show(self.id, build);
    }

    /// Take it down now, without dropping the lease. Idempotent.
    pub fn hide(&self) {
        self.handle.hide(self.id);
    }
}

impl Drop for OverlayLease {
    fn drop(&mut self) {
        self.handle.hide(self.id);
    }
}

/// The overlay's own durable state: the entries, and the pending flag.
#[derive(Debug, Default)]
pub struct OverlayState {
    entries: Rc<RefCell<Entries>>,
}

impl OverlayState {
    /// A handle onto the same entries this state holds.
    fn handle(&self) -> OverlayHandle {
        OverlayHandle {
            entries: Rc::clone(&self.entries),
        }
    }

    /// The builders to draw, in order.
    fn shown(&self) -> Vec<OverlayBuilder> {
        self.entries
            .borrow()
            .shown
            .iter()
            .map(|(_, build)| Rc::clone(build))
            .collect()
    }
}

impl ElementState for OverlayState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn take_pending(&mut self) -> bool {
        let mut entries = self.entries.borrow_mut();
        std::mem::take(&mut entries.changed)
    }
}

/// A full-surface stack that anything below it can draw into.
///
/// Wrap an application's root in one. Everything inside can then put a widget
/// above the whole window without hoisting it by hand and without being clipped
/// by what it happens to sit in.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Overlay;
///
/// let root = Overlay::new().child(Text::new("the application"));
/// ```
///
/// See the module docs for what it is for and why it costs no frame of lag.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Overlay {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The application, drawn underneath everything the overlay shows.
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

    /// Whether there is an overlay above this position.
    ///
    /// What a widget asks before deciding to build its own fallback, and what it
    /// reports when there is none. A widget that silently degrades is worse than
    /// one that says so.
    #[must_use]
    pub fn is_present(ctx: &BuildContext) -> bool {
        ctx.inherit::<OverlayHandle>().is_some()
    }

    /// The overlay above this position, or a detached handle.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> OverlayHandle {
        ctx.inherit::<OverlayHandle>()
            .map_or_else(OverlayHandle::detached, |handle| (*handle).clone())
    }
}

impl Widget for Overlay {
    fn debug_name(&self) -> &'static str {
        "Overlay"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(OverlayState::default()))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // Outside an element tree — `debug_tree`, `inflate` — there is no state,
        // so the overlay is an empty one rather than a panic. Nothing can have
        // contributed to a tree that was never mounted.
        let handle = ctx
            .state::<OverlayState, _>(OverlayState::handle)
            .unwrap_or_else(OverlayHandle::detached);
        let shown = ctx
            .state::<OverlayState, _>(OverlayState::shown)
            .unwrap_or_default();

        // `Expand`, so an entry is measured against the whole window. That is
        // the half that makes window-coordinate placement mean what it says —
        // a `Menu` positions itself against `ViewMetrics::size`, and being laid
        // out against anything smaller is how it ended up off its own parent.
        let mut stack = Stack::new().fit(StackFit::Expand);
        if let Some(child) = &self.child {
            stack = stack.push(child.clone());
        }
        for build in shown {
            stack = stack.push(build());
        }

        // Published *around* the stack, so the child and every entry can both
        // reach it: an entry that opens another entry — a menu inside a dialog —
        // is ordinary, and an overlay only its child could see would break it.
        Inherited::new(handle, stack).into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("host", String::from("true"))]
    }
}

widget_node_from!(Overlay);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inflate, Text};

    fn text(label: &'static str) -> OverlayBuilder {
        Rc::new(move || Text::new(label.to_owned()).into())
    }

    #[test]
    fn ids_are_distinct() {
        assert_ne!(OverlayId::new(), OverlayId::new());
    }

    #[test]
    fn showing_the_same_id_replaces_in_place_rather_than_raising_it() {
        let mut entries = Entries::default();
        let (first, second) = (OverlayId::new(), OverlayId::new());
        entries.show(first, text("one"));
        entries.show(second, text("two"));
        entries.show(first, text("one again"));

        let order: Vec<OverlayId> = entries.shown.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            order,
            vec![first, second],
            "a dropdown rebuilding under a dialog must not jump in front of it"
        );
    }

    #[test]
    fn hiding_something_that_was_never_shown_does_not_ask_for_a_rebuild() {
        let mut entries = Entries::default();
        entries.show(OverlayId::new(), text("one"));
        entries.changed = false;

        entries.hide(OverlayId::new());
        assert!(
            !entries.changed,
            "`dispose` runs whether or not an entry was up, and a rebuild per \
             teardown is a rebuild for nothing"
        );
    }

    #[test]
    fn a_lease_withdraws_its_entry_when_it_is_dropped() {
        let handle = OverlayHandle::detached();
        let lease = handle.lease(OverlayId::new());
        lease.show(text("a menu"));
        assert_eq!(handle.len(), 1);

        drop(lease);
        assert_eq!(
            handle.len(),
            0,
            "a control removed from the tree with its overlay up has to take it \
             with it, and this is the half nobody remembers to write by hand"
        );
    }

    #[test]
    fn the_handle_reaches_the_subtree() {
        let tree = inflate(Overlay::new().child(Text::new("app")));
        assert!(
            tree.find("Inherited<OverlayHandle>").is_some(),
            "{}",
            crate::debug_tree(Overlay::new().child(Text::new("app")))
        );
    }

    #[test]
    fn an_overlay_with_nothing_shown_is_just_its_child() {
        let tree = inflate(Overlay::new().child(Text::new("app")));
        assert!(tree.find("Text").is_some(), "the application still draws");
    }
}
