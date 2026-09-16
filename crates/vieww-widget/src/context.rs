use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::Key;

use crate::{ElementState, Widget, WidgetKind, WidgetNode};

/// The read-only handle a widget uses to reach values published by its
/// ancestors during `build`.
///
/// A `BuildContext` is scoped to one position in the tree. It is cheap to clone
/// (one `Rc` bump) and must not outlive the build that produced it — holding one
/// across frames would let a widget read a stale ancestor.
///
/// [`inherit`](BuildContext::inherit) registers a dependency, so that changing
/// a published value marks exactly the elements that read it — see
/// [`Provision`].
#[derive(Clone, Default)]
pub struct BuildContext {
    scope: InheritedScope,
    depth: usize,
    /// The building element's durable state, if it has any.
    ///
    /// Shared rather than borrowed because the element tree hands the same
    /// handle to the frame's animate phase, which mutates it. A build only ever
    /// reads — see [`BuildContext::state`].
    state: Option<Rc<RefCell<dyn ElementState>>>,
    /// Which element is building, as an opaque handle.
    ///
    /// Set by the element tree so that [`inherit`](Self::inherit) can record a
    /// dependency: change the value and *this* element rebuilds, rather than
    /// every element below the provider. `None` outside an element tree — a
    /// tree dump reads without subscribing, which is right.
    reader: Option<u64>,
}

impl BuildContext {
    /// A context at the root of the tree, with nothing published.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// A context at an arbitrary position, for a tree that tracks scope and
    /// depth itself — which the element tree does, since it keeps them across
    /// frames rather than rebuilding them by recursion.
    #[must_use]
    pub const fn at(scope: InheritedScope, depth: usize) -> Self {
        Self {
            scope,
            depth,
            state: None,
            reader: None,
        }
    }

    /// The same context, attributed to the element that is building.
    ///
    /// The handle is opaque here and meaningful only to whoever minted it. See
    /// [`Provision`].
    #[must_use]
    pub const fn with_reader(mut self, reader: u64) -> Self {
        self.reader = Some(reader);
        self
    }

    /// Which element is building, if this context came from an element tree.
    #[must_use]
    pub const fn reader(&self) -> Option<u64> {
        self.reader
    }

    /// The same context, with the building element's state attached.
    #[must_use]
    pub fn with_state(mut self, state: Rc<RefCell<dyn ElementState>>) -> Self {
        self.state = Some(state);
        self
    }

    /// Read the building element's own durable state.
    ///
    /// `None` when the widget created no state, or created state of a different
    /// type — which for a widget reading its *own* state can only happen if it
    /// is being built outside an element tree, as `debug_tree` does. A widget
    /// should have something sensible to build in that case rather than
    /// panicking; an animated widget builds its target values, un-animated.
    ///
    /// Read-only on purpose: `build` must stay side-effect free for the
    /// dependency tracking to be sound. State changes go in
    /// [`ElementState::widget_updated`] and [`ElementState::tick`].
    ///
    /// # Panics
    ///
    /// If called while the state is already mutably borrowed — that is, from
    /// inside one of the state's own hooks. Those run outside a build, so there
    /// is no legitimate path to it.
    #[must_use]
    pub fn state<S: ElementState + 'static, R>(&self, read: impl FnOnce(&S) -> R) -> Option<R> {
        let state = self.state.as_ref()?;
        let borrowed = state.borrow();
        let concrete = borrowed.as_any().downcast_ref::<S>()?;
        Some(read(concrete))
    }

    /// A handle to the building element's state, for a *handler* to write later.
    ///
    /// The build itself must not write through this — that is what
    /// [`state`](Self::state) is for, and why it is read-only. What this exists
    /// for is the closure a build hands to a
    /// [`GestureDetector`](crate::GestureDetector): it runs during input
    /// dispatch, long after the build has returned, and it is the only way
    /// ephemeral view state like a press highlight can be written at all. See
    /// [`ElementState::take_pending`], which is how the write becomes a rebuild.
    ///
    /// `None` outside an element tree, or when the widget created no state.
    ///
    /// Holding it does not keep the element alive: the state is the element's,
    /// and a handler outliving its element writes to something nothing will
    /// read again.
    #[must_use]
    pub fn state_handle(&self) -> Option<Rc<RefCell<dyn ElementState>>> {
        self.state.clone()
    }

    /// The nearest ancestor-provided value of type `T`, if any.
    ///
    /// "Nearest" means the innermost [`Inherited<T>`] above this position, so an
    /// inner provider shadows an outer one.
    #[must_use]
    pub fn inherit<T: 'static>(&self) -> Option<Rc<T>> {
        self.scope.get_for::<T>(self.reader)
    }

    /// The nearest ancestor-provided value of type `T`, or a default.
    #[must_use]
    pub fn inherit_or<T: 'static>(&self, fallback: T) -> Rc<T> {
        self.inherit::<T>().unwrap_or_else(|| Rc::new(fallback))
    }

    /// How deep this position sits below the root. Useful for tree dumps and
    /// for the depth-ordered rebuild queue in Phase 2.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// A context one level deeper, same published values.
    ///
    /// The state is *not* carried down: it belongs to this element, and the
    /// child has its own or none.
    #[must_use]
    pub fn child(&self) -> Self {
        Self {
            scope: self.scope.clone(),
            depth: self.depth + 1,
            state: None,
            reader: None,
        }
    }

    /// A context one level deeper, with `value` published to it and everything
    /// below.
    #[must_use]
    pub fn child_with<T: 'static>(&self, value: Rc<T>) -> Self {
        Self {
            scope: self.scope.push(value),
            depth: self.depth + 1,
            state: None,
            reader: None,
        }
    }

    /// The set of values visible at this position.
    #[must_use]
    pub const fn scope(&self) -> &InheritedScope {
        &self.scope
    }
}

impl fmt::Debug for BuildContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuildContext")
            .field("depth", &self.depth)
            .field("inherited", &self.scope.len())
            .field("has_state", &self.state.is_some())
            .finish()
    }
}

/// An immutable stack of values published by ancestor [`Inherited`] widgets.
///
/// Implemented as a persistent linked list so that pushing a value for one
/// subtree costs a single allocation and leaves every sibling's scope untouched
/// — a `HashMap` clone per provider would be O(providers) per subtree instead.
///
/// Lookup is a linear walk. That is the right trade at realistic provider counts
/// (theme, media query, localization, a handful of app scopes); if a tree ever
/// carries enough providers for this to show up in a profile, the fix is a
/// per-element resolved map cached in Phase 2, not a different data structure
/// here.
#[derive(Clone, Default)]
pub struct InheritedScope {
    head: Option<Rc<Entry>>,
}

struct Entry {
    provision: Rc<Provision>,
    next: Option<Rc<Entry>>,
}

/// One value published by one [`Inherited`] element, and who has read it.
///
/// # Why the value is behind a `RefCell` instead of being a fresh scope
///
/// A scope used to be a list of *values*, so publishing a new value meant a new
/// scope, and a new scope meant every element below the provider failed
/// `ElementTree::update`'s `scope.ptr_eq` early-out and rebuilt. The whole
/// subtree, on every change, whatever it read.
///
/// That is invisible when the inherited value is a theme somebody toggles once.
/// It is the dominant cost of a frame when it is `ScrollMetrics`, which is
/// republished **on every frame of a scroll**: a wall of a hundred photographs
/// rebuilt all hundred tiles per frame — a `Photo::clone` and six `String`
/// allocations each — to draw them in the place they already were. Measured at
/// 336 element rebuilds per scroll step in `vavlt`'s `idle_cost` suite, and it
/// is what a phone reports as "the app is laggy".
///
/// So the *provision* is the stable thing and the value inside it moves. The
/// scope a child sees is then the same `Rc` from frame to frame, the early-out
/// holds, and only the elements that actually read the value are told.
///
/// This is the inherited-element pattern and its dependents set, arrived at from
/// the same direction.
pub struct Provision {
    type_id: TypeId,
    value: RefCell<Rc<dyn Any>>,
    /// Elements that read this during their last build, as opaque handles.
    ///
    /// Opaque because this crate must not depend on the element tree — it is
    /// the layer below. The element tree encodes an id into a `u64` and decodes
    /// it back; nothing here interprets the number.
    ///
    /// Drained when the value changes rather than kept: an element that is told
    /// to rebuild will read again if it still cares, and one that no longer
    /// reads this value should stop hearing about it. A handle for an element
    /// that has since died decodes to something the tree ignores.
    readers: RefCell<Vec<u64>>,
}

impl Provision {
    /// Publish `value` for the first time.
    #[must_use]
    pub fn new<T: 'static>(value: Rc<T>) -> Rc<Self> {
        Rc::new(Self {
            type_id: TypeId::of::<T>(),
            value: RefCell::new(value),
            readers: RefCell::new(Vec::new()),
        })
    }

    /// Whether this provision carries a `T`.
    #[must_use]
    pub fn provides<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Swap in a new value, returning the readers that have to be rebuilt.
    ///
    /// Returns nothing when the value is the *same allocation* as before, which
    /// is the case whenever a provider rebuilds without its value moving —
    /// common enough to be worth the pointer comparison, and free.
    #[must_use]
    pub fn republish<T: 'static>(&self, value: Rc<T>) -> Vec<u64> {
        debug_assert!(
            self.provides::<T>(),
            "a provision's type cannot change: an element publishing a \
             different type is a different element"
        );
        {
            let mut slot = self.value.borrow_mut();
            if Rc::ptr_eq(&*slot, &(value.clone() as Rc<dyn Any>)) {
                return Vec::new();
            }
            *slot = value;
        }
        std::mem::take(&mut *self.readers.borrow_mut())
    }

    /// Read the value, recording `reader` as depending on it.
    fn read<T: 'static>(&self, reader: Option<u64>) -> Option<Rc<T>> {
        if !self.provides::<T>() {
            return None;
        }
        if let Some(reader) = reader {
            let mut readers = self.readers.borrow_mut();
            if !readers.contains(&reader) {
                readers.push(reader);
            }
        }
        // Recorded under `TypeId::of::<T>()` at construction, so the value is
        // an `Rc<T>` by construction.
        Rc::downcast::<T>(self.value.borrow().clone()).ok()
    }
}

impl fmt::Debug for Provision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provision")
            .field("readers", &self.readers.borrow().len())
            .finish_non_exhaustive()
    }
}

impl InheritedScope {
    /// An empty scope.
    #[must_use]
    pub const fn new() -> Self {
        Self { head: None }
    }

    /// This scope with `value` pushed in front, shadowing any outer `T`.
    ///
    /// Creates a fresh [`Provision`], so the resulting scope is a *new* scope.
    /// An element tree that wants the subtree-skipping early-out to survive a
    /// republish should keep its provision and use
    /// [`push_provision`](Self::push_provision) instead.
    #[must_use]
    pub fn push<T: 'static>(&self, value: Rc<T>) -> Self {
        self.push_provision(Provision::new(value))
    }

    /// This scope with an existing provision pushed in front.
    #[must_use]
    pub fn push_provision(&self, provision: Rc<Provision>) -> Self {
        Self {
            head: Some(Rc::new(Entry {
                provision,
                next: self.head.clone(),
            })),
        }
    }

    /// The innermost value of type `T`.
    #[must_use]
    pub fn get<T: 'static>(&self) -> Option<Rc<T>> {
        self.get_for::<T>(None)
    }

    /// The innermost value of type `T`, recording `reader` as depending on it.
    ///
    /// A read with no reader — a tree dump, a test — subscribes to nothing,
    /// which is what those callers want.
    #[must_use]
    pub fn get_for<T: 'static>(&self, reader: Option<u64>) -> Option<Rc<T>> {
        let mut cursor = self.head.as_ref();
        while let Some(entry) = cursor {
            if let Some(value) = entry.provision.read::<T>(reader) {
                return Some(value);
            }
            cursor = entry.next.as_ref();
        }
        None
    }

    /// The innermost provision, if this scope publishes anything.
    ///
    /// For an element tree keeping a provision alive across rebuilds.
    #[must_use]
    pub fn head_provision(&self) -> Option<Rc<Provision>> {
        self.head.as_ref().map(|entry| Rc::clone(&entry.provision))
    }

    /// `true` if `outer` is exactly the scope this one was pushed onto.
    ///
    /// The test an element runs before reusing a cached child scope: a
    /// provision's meaning includes *where in the chain* it sits, so a cached
    /// scope is only still correct while the chain above it is the same one.
    #[must_use]
    pub fn parent_is(&self, outer: &Self) -> bool {
        match &self.head {
            Some(entry) => match (&entry.next, &outer.head) {
                (None, None) => true,
                (Some(mine), Some(theirs)) => Rc::ptr_eq(mine, theirs),
                _ => false,
            },
            None => outer.head.is_none(),
        }
    }

    /// Number of values visible, counting shadowed ones.
    #[must_use]
    pub fn len(&self) -> usize {
        let mut count = 0;
        let mut cursor = self.head.as_ref();
        while let Some(entry) = cursor {
            count += 1;
            cursor = entry.next.as_ref();
        }
        count
    }

    /// `true` if nothing has been published.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// `true` if both handles denote the same scope.
    ///
    /// Scopes are immutable and shared, so pointer equality is a sound "nothing
    /// an ancestor published has changed" test. Reconciliation uses it, with
    /// [`WidgetNode::ptr_eq`](crate::WidgetNode::ptr_eq), to skip a subtree
    /// entirely instead of walking it to discover it is unchanged.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.head, &other.head) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl fmt::Debug for InheritedScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InheritedScope({} entries)", self.len())
    }
}

/// Publishes a value of type `T` to its entire subtree.
///
/// This is the inherited widget, generic over the payload instead of
/// requiring a new widget type per value. Descendants read it with
/// [`BuildContext::inherit`].
///
/// ```
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::{BuildContext, Inherited};
///
/// #[derive(Debug)]
/// struct Theme { accent: Color }
///
/// let tree = Inherited::new(Theme { accent: Color::BLUE }, Text::new("hi"));
/// let dump = vieww_widget::debug_tree(tree);
/// assert!(dump.contains("Inherited<Theme>"));
/// ```
pub struct Inherited<T: 'static> {
    value: Rc<T>,
    child: WidgetNode,
    key: Option<Key>,
}

impl<T: 'static> Inherited<T> {
    /// Publish `value` to `child`'s subtree.
    #[must_use]
    pub fn new(value: T, child: impl Into<WidgetNode>) -> Self {
        Self::shared(Rc::new(value), child)
    }

    /// Publish an already-shared `value`.
    ///
    /// Prefer this when the same value is published in several places — it
    /// keeps one allocation instead of one per provider.
    #[must_use]
    pub fn shared(value: Rc<T>, child: impl Into<WidgetNode>) -> Self {
        Self {
            value,
            child: child.into(),
            key: None,
        }
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The published value.
    #[must_use]
    pub fn value(&self) -> &Rc<T> {
        &self.value
    }
}

impl<T: 'static> fmt::Debug for Inherited<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inherited")
            .field("type", &short_type_name::<T>())
            .field("child", &self.child)
            .finish()
    }
}

impl<T: 'static> Widget for Inherited<T> {
    fn debug_name(&self) -> &'static str {
        // `Widget::debug_name` returns `&'static str`, and `T`'s name is only
        // known at runtime, so the type parameter is reported as a property
        // instead. `DebugNode` splices it back into the display name.
        "Inherited"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Inherited(&self.child)
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("provides", short_type_name::<T>())]
    }

    fn publish_inherited(&self, ctx: &BuildContext) -> Option<BuildContext> {
        Some(ctx.child_with(self.value.clone()))
    }

    fn publish_into(&self, provision: &Rc<Provision>) -> Vec<u64> {
        provision.republish(self.value.clone())
    }

    fn publishes(&self) -> bool {
        true
    }
}

impl<T: 'static> From<Inherited<T>> for WidgetNode {
    fn from(widget: Inherited<T>) -> Self {
        Self::new(widget)
    }
}

/// `std::any::type_name` with module paths and generic arguments stripped.
pub(crate) fn short_type_name<T: 'static>() -> String {
    let full = std::any::type_name::<T>();
    let base = full.split('<').next().unwrap_or(full);
    base.rsplit("::").next().unwrap_or(base).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Theme(&'static str);

    #[derive(Debug, PartialEq)]
    struct Locale(&'static str);

    #[test]
    fn inner_provider_shadows_outer_one_of_the_same_type() {
        let ctx = BuildContext::root()
            .child_with(Rc::new(Theme("light")))
            .child_with(Rc::new(Theme("dark")));
        assert_eq!(*ctx.inherit::<Theme>().unwrap(), Theme("dark"));
    }

    #[test]
    fn different_types_coexist_without_shadowing() {
        let ctx = BuildContext::root()
            .child_with(Rc::new(Theme("light")))
            .child_with(Rc::new(Locale("en")));
        assert_eq!(*ctx.inherit::<Theme>().unwrap(), Theme("light"));
        assert_eq!(*ctx.inherit::<Locale>().unwrap(), Locale("en"));
    }

    #[test]
    fn a_sibling_scope_is_unaffected_by_what_the_other_publishes() {
        let parent = BuildContext::root().child_with(Rc::new(Theme("light")));
        let publishing_sibling = parent.child_with(Rc::new(Locale("fr")));
        let plain_sibling = parent.child();

        assert!(publishing_sibling.inherit::<Locale>().is_some());
        assert!(plain_sibling.inherit::<Locale>().is_none());
        assert!(plain_sibling.inherit::<Theme>().is_some());
    }

    #[test]
    fn missing_values_read_as_none_rather_than_panicking() {
        assert!(BuildContext::root().inherit::<Theme>().is_none());
    }

    #[test]
    fn depth_counts_levels_below_the_root() {
        let ctx = BuildContext::root().child().child_with(Rc::new(Theme("x")));
        assert_eq!(ctx.depth(), 2);
    }

    #[test]
    fn short_type_name_strips_modules_and_generics() {
        assert_eq!(short_type_name::<Theme>(), "Theme");
        assert_eq!(short_type_name::<Vec<Theme>>(), "Vec");
    }
}
