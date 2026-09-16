use std::any::{Any, TypeId};
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use vieww_foundation::Key;

use crate::Widget;

/// A reference-counted handle to a [`Widget`].
///
/// Cloning is a refcount bump, which is what makes "rebuild the widget tree
/// every frame" affordable — a rebuild that returns an identical subtree hands
/// back the same allocations.
///
/// `Rc`, not `Arc`: the tree is single-threaded by design (`docs/DESIGN.md` §3).
#[derive(Clone)]
pub struct WidgetNode(
    Rc<dyn Widget>,
    /// The type's path, captured where the concrete type is still known.
    ///
    /// Only under `hot-reload`, and only because [`TypeId`] cannot do this job
    /// there: it is derived partly from a compilation-session component, so
    /// `Counter` before a rebuild and `Counter` after it are different types to
    /// `Any`. A path is the same string in both. See [`can_update`].
    ///
    /// Costs a fat pointer per node, which is why it is not carried in an
    /// ordinary build.
    ///
    /// [`can_update`]: WidgetNode::can_update
    #[cfg(feature = "hot-reload")]
    &'static str,
);

impl WidgetNode {
    /// Wrap a widget.
    #[cfg(not(feature = "hot-reload"))]
    #[must_use]
    pub fn new<W: Widget>(widget: W) -> Self {
        Self(Rc::new(widget))
    }

    /// Wrap a widget, recording the type path reconciliation will identify it
    /// by.
    ///
    /// Named rather than `impl Widget` so [`type_name`](std::any::type_name) has
    /// a type to ask about. Every caller is unchanged.
    #[cfg(feature = "hot-reload")]
    #[must_use]
    pub fn new<W: Widget>(widget: W) -> Self {
        Self(Rc::new(widget), std::any::type_name::<W>())
    }

    /// Wrap a widget that is already behind an [`Rc`], keeping *its* type.
    ///
    /// # The defect this exists for
    ///
    /// A host that loads widgets from somewhere else — the studio's compiled
    /// preview, a plugin, anything behind `Rc<dyn Widget>` — cannot use
    /// [`new`](Self::new), which needs a sized type. The studio's answer was a
    /// shim widget that delegated `kind`, `build` and `debug_name` to the loaded
    /// one, and that shim is a different Rust type. Everything that reads a
    /// *type* rather than a method then read the shim's:
    /// [`widget_type_id`](Self::widget_type_id) most of all, which is what the
    /// render factory looks a widget up by.
    ///
    /// So a loaded screen whose root was a **render** widget — a bare
    /// `Flex::column()`, the shape half the studio's own lessons are written in
    /// — found no render object registered for the shim, drew nothing, and had
    /// its children spliced into whatever was above it. On screen: three
    /// controls stacked on top of each other in the middle of the frame. A
    /// screen whose root was a *composed* widget was unaffected, which is why
    /// this survived so long.
    ///
    /// Wrapping the `Rc` directly keeps the loaded widget's own identity, and
    /// there is no shim to disagree with it.
    /// # Why `id` is taken in both configurations
    ///
    /// It was once absent without `hot-reload` and present with it, and that is
    /// a **feature changing a public function's arity** — the one shape of API
    /// that cargo's feature unification is guaranteed to break. `viewwstudio`
    /// is the crate it broke: it does not enable `hot-reload` and its call site
    /// was written `#[cfg]`-split to match, but a workspace build also contains
    /// `vieww-reload`, which does enable it, and features unify per-crate
    /// across a build. So `vieww-widget` compiled with two fields while the
    /// studio compiled the one-argument arm, and `cargo build --workspace`
    /// failed at the root of the repository with `this function takes 2
    /// arguments but 1 argument was supplied`.
    ///
    /// Taking the argument unconditionally and ignoring it here costs nothing —
    /// a `&'static str` that is never read is not passed at all after
    /// optimisation — and it makes the signature a fact about the crate rather
    /// than about how somebody happened to resolve its features.
    #[cfg(not(feature = "hot-reload"))]
    #[must_use]
    pub fn from_rc(widget: Rc<dyn Widget>, _id: &'static str) -> Self {
        Self(widget)
    }

    /// As above; the reload identity is supplied by the caller, because an
    /// `Rc<dyn Widget>` has no concrete type left to read a path from.
    #[cfg(feature = "hot-reload")]
    #[must_use]
    pub fn from_rc(widget: Rc<dyn Widget>, id: &'static str) -> Self {
        Self(widget, id)
    }

    /// Wrap a widget under an identity it does not have.
    ///
    /// **The seam that makes reload identity testable without a loader.** Two
    /// builds of one application produce two Rust types with the same path;
    /// inside one binary that is impossible to construct, so a test says the
    /// path instead. A loader does not need this — `new` already reads the path
    /// from the guest's own type.
    #[cfg(feature = "hot-reload")]
    #[must_use]
    pub fn with_reload_id(widget: impl Widget, id: &'static str) -> Self {
        Self(Rc::new(widget), id)
    }

    /// What reconciliation identifies this widget by under `hot-reload`.
    #[cfg(feature = "hot-reload")]
    #[must_use]
    pub const fn reload_id(&self) -> &'static str {
        self.1
    }

    /// The concrete type of the wrapped widget.
    ///
    /// Half of reconciliation's identity check; [`key`](Self::key) is the other
    /// half. See `docs/DESIGN.md` §2.
    #[must_use]
    pub fn widget_type_id(&self) -> TypeId {
        // Resolves through the `Any` supertrait to the concrete widget's id,
        // not to `dyn Widget`.
        Any::type_id(&*self.0)
    }

    /// This widget's explicit identity, if it has one.
    #[must_use]
    pub fn key(&self) -> Option<&Key> {
        self.0.key()
    }

    /// `true` if an element built for `self` could be reused for `other`.
    ///
    /// This is the canonical `can_update` rule, and it is the single rule that
    /// decides whether state survives a rebuild.
    #[must_use]
    pub fn can_update(&self, other: &Self) -> bool {
        self.same_kind(other) && self.key() == other.key()
    }

    /// Whether these two describe the same *kind* of widget.
    ///
    /// [`TypeId`] in an ordinary build, and it is the right answer there: exact,
    /// free, and impossible to spoof.
    #[cfg(not(feature = "hot-reload"))]
    fn same_kind(&self, other: &Self) -> bool {
        self.widget_type_id() == other.widget_type_id()
    }

    /// The type's path instead, because a `TypeId` does not survive a rebuild.
    ///
    /// This is the whole of reload-mode identity. A `TypeId` carries a
    /// compilation-session component, so after the guest is rebuilt *every*
    /// widget in the tree fails `can_update`, every element is torn down, and
    /// every signal, scroll offset and half-finished animation goes with it —
    /// a slow restart wearing a reload's name. A path is the same string across
    /// builds.
    ///
    /// **It is a weaker check, and deliberately so.** Two distinct types sharing
    /// a path would share element state; that cannot happen within one binary,
    /// and across a reload it is exactly the identity we want. It stays behind a
    /// feature because a shipped build should keep the exact rule.
    #[cfg(feature = "hot-reload")]
    fn same_kind(&self, other: &Self) -> bool {
        self.1 == other.1
    }

    /// `true` if both handles point at the same allocation.
    ///
    /// A cheap early-out for reconciliation: an unchanged subtree that was
    /// cloned rather than rebuilt cannot have changed, so it can be skipped
    /// without comparing anything.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// `true` if `other` describes exactly what this one does.
    ///
    /// Forwards to [`Widget::same_configuration`]; see that method for when it
    /// is safe to implement and what it costs to implement wrongly.
    #[must_use]
    pub fn same_configuration(&self, other: &Self) -> bool {
        self.0.same_configuration(other)
    }

    /// Downcast to a concrete widget type.
    #[must_use]
    pub fn downcast_ref<W: Widget>(&self) -> Option<&W> {
        let any: &dyn Any = &*self.0;
        any.downcast_ref::<W>()
    }
}

impl Deref for WidgetNode {
    type Target = dyn Widget;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl fmt::Debug for WidgetNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.key() {
            Some(key) => write!(f, "{}{key}", self.debug_name()),
            None => f.write_str(self.debug_name()),
        }
    }
}

/// Generate `impl From<$widget> for WidgetNode` for each named widget type.
///
/// Widget builders take `impl Into<WidgetNode>` so they accept both a concrete
/// widget and an already-wrapped node. That works because core's reflexive
/// `impl<T> From<T> for T` covers the `WidgetNode -> WidgetNode` case; a blanket
/// `impl<W: Widget> From<W> for WidgetNode` would collide with it (E0119).
/// See `docs/DESIGN.md` §6.
#[macro_export]
macro_rules! widget_node_from {
    ($($widget:ty),+ $(,)?) => {
        $(
            impl ::core::convert::From<$widget> for $crate::WidgetNode {
                fn from(widget: $widget) -> Self {
                    $crate::WidgetNode::new(widget)
                }
            }
        )+
    };
}

/// Build a `Vec<WidgetNode>` from widgets of differing types.
///
/// ```
/// use vieww_widget::prelude::*;
///
/// let kids = children![Text::new("a"), Padding::all(4.0).child(Text::new("b"))];
/// assert_eq!(kids.len(), 2);
/// ```
#[macro_export]
macro_rules! children {
    () => { ::std::vec::Vec::<$crate::WidgetNode>::new() };
    ($($child:expr),+ $(,)?) => {
        ::std::vec![$( $crate::WidgetNode::from($child) ),+]
    };
}
