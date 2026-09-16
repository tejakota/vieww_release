//! Responsive vocabulary that does not fit [`ThemeData`](crate::ThemeData)'s
//! fixed struct set.
//!
//! [`ThemeData`] is deliberately closed: [`ColorScheme`](crate::ColorScheme),
//! [`Typography`](crate::Typography), [`Metrics`](crate::Metrics) and
//! [`Motion`](crate::Motion) are named, well-known roles this framework itself
//! draws with, and adding a fifth would mean editing this crate's own source.
//! Two real needs sit next to that, not inside it:
//!
//! - **[`WindowSizeClass`]** — a shared, named vocabulary for *how much room
//!   the window has*, so "on Expanded show a navigation rail" means the same
//!   thing everywhere an application says it. See its own doc for how this
//!   differs from [`LayoutBuilder`](crate::LayoutBuilder)'s breakpoints, which
//!   solve a different problem.
//! - **[`TokenSet`]** — an open, named registry for a design token this
//!   framework has no opinion about at all: an elevation scale, a spacing
//!   scale with a brand's own step names, an icon-size ramp. Where
//!   `ThemeData`'s fields are answers this framework already knows, a
//!   `TokenSet` is a place for an application's own vocabulary to live
//!   alongside it.

use std::any::Any;
use std::collections::HashMap;
use std::fmt;

/// How much horizontal room a window currently has, named rather than
/// measured — the vocabulary an application reasons about layout in, as
/// distinct from the raw width a specific widget happens to be measuring.
///
/// # Real, published thresholds
///
/// These are the published window size classes, not an invented scale:
/// **Compact** below 600dp (a phone in portrait), **Medium** 600–839dp (a
/// phone in landscape, a small tablet, a foldable), **Expanded** 840dp and up
/// (a tablet, a desktop window). Reusing a published breakpoint set means an
/// application porting a design from — or to — a framework that already
/// speaks these terms carries the same three categories forward unchanged,
/// rather than translating between two custom scales that drew the lines in
/// slightly different places.
///
/// # How this differs from [`LayoutBuilder`](crate::LayoutBuilder)
///
/// `LayoutBuilder::breakpoints` is a *local, per-widget* optimisation: an
/// ascending list of widths one specific `LayoutBuilder` cares about, so it
/// rebuilds less often. It answers "how much room did *this subtree* get",
/// which can be much narrower than the window — a sidebar's `LayoutBuilder`
/// might never see more than 300 logical pixels even in an Expanded window.
///
/// `WindowSizeClass` answers a different question: "how much room does the
/// *window* have", a single, shared fact meant to be read the same way from
/// many places in the tree at once — a nav rail here, a two-pane layout
/// there, a dialog's own width limit somewhere else — all agreeing on the
/// same three categories without each computing its own thresholds. Publish
/// it once near the root with [`WindowSizeClassProvider`] and read it
/// anywhere below with [`WindowSizeClass::of`], the same shape
/// [`ThemeData::of`](crate::ThemeData::of) already established for exactly
/// this kind of ambient, root-published fact.
///
/// If what you actually want is "rebuild this one subtree when the room it
/// personally has crosses a threshold", reach for `LayoutBuilder` instead —
/// see its own doc's "Container queries" section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowSizeClass {
    /// Narrower than 600 logical pixels.
    Compact,
    /// 600 up to (not including) 840 logical pixels.
    Medium,
    /// 840 logical pixels and wider.
    Expanded,
}

/// The Compact/Medium boundary, in logical pixels. See [`WindowSizeClass`]'s
/// own doc for the source of these numbers.
pub const COMPACT_MEDIUM_BOUNDARY: f32 = 600.0;
/// The Medium/Expanded boundary, in logical pixels.
pub const MEDIUM_EXPANDED_BOUNDARY: f32 = 840.0;

impl WindowSizeClass {
    /// The size class a window of this `width` falls into.
    ///
    /// A pure function of a width in logical pixels — nothing here reads a
    /// `BuildContext` or a window directly, so a test can call it with any
    /// number without mounting a tree. [`WindowSizeClassProvider`] is what
    /// calls it from a live layout and publishes the result.
    #[must_use]
    pub fn from_width(width: f32) -> Self {
        if width < COMPACT_MEDIUM_BOUNDARY {
            Self::Compact
        } else if width < MEDIUM_EXPANDED_BOUNDARY {
            Self::Medium
        } else {
            Self::Expanded
        }
    }

    /// The size class published above this position, or [`Self::Compact`] if
    /// nothing published one.
    ///
    /// [`Self::Compact`] — the narrowest, most conservative class — is the
    /// fallback for the same reason [`ThemeData::of`](crate::ThemeData::of)
    /// falls back to the light theme rather than panicking: a control built
    /// with no [`WindowSizeClassProvider`] above it (a unit test, a tree
    /// dump, a widget preview) should degrade to the simplest layout rather
    /// than fail, and assuming a phone-sized window is the safer of the two
    /// wrong guesses — a Compact layout squeezed into an Expanded window
    /// merely wastes space, where an Expanded layout assumed on a Compact one
    /// clips.
    #[must_use]
    pub fn of(ctx: &crate::BuildContext) -> Self {
        ctx.inherit::<Self>().map_or(Self::Compact, |class| *class)
    }
}

impl fmt::Display for WindowSizeClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Compact => "compact",
            Self::Medium => "medium",
            Self::Expanded => "expanded",
        };
        f.write_str(name)
    }
}

/// Measures the window's own width and publishes the matching
/// [`WindowSizeClass`] to its subtree.
///
/// Thin by design: this is
/// [`MeasuredConstraints`](crate::MeasuredConstraints) (report the offered
/// constraints, change nothing else) feeding
/// [`Inherited`](crate::Inherited)`<WindowSizeClass>` (publish a value to the
/// subtree) — the same two building blocks `ThemeData`'s own publication
/// (`Theme`, a widget wrapping `Inherited<ThemeData>`) is built from, applied
/// once near the root of an application rather than once per theme-like
/// value this framework happens to ship.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{WindowSizeClass, WindowSizeClassProvider};
///
/// let app = WindowSizeClassProvider::new(
///     LayoutBuilder::new(|_| Text::new("body").into())
/// );
/// let _ = vieww_widget::debug_tree(app);
/// ```
#[derive(Clone)]
pub struct WindowSizeClassProvider {
    child: crate::WidgetNode,
    key: Option<vieww_foundation::Key>,
}

impl WindowSizeClassProvider {
    #[must_use]
    pub fn new(child: impl Into<crate::WidgetNode>) -> Self {
        Self {
            child: child.into(),
            key: None,
        }
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<vieww_foundation::Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl fmt::Debug for WindowSizeClassProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowSizeClassProvider")
            .field("child", &self.child)
            .finish()
    }
}

impl crate::Widget for WindowSizeClassProvider {
    fn debug_name(&self) -> &'static str {
        "WindowSizeClassProvider"
    }

    fn key(&self) -> Option<&vieww_foundation::Key> {
        self.key.as_ref()
    }

    fn kind(&self) -> crate::WidgetKind<'_> {
        crate::WidgetKind::Composed
    }

    fn build(&self, _ctx: &crate::BuildContext) -> crate::WidgetNode {
        let child = self.child.clone();
        crate::LayoutBuilder::new(move |constraints| {
            let width = if constraints.max_width.is_finite() {
                constraints.max_width
            } else {
                // An unbounded axis (a scroll view's cross axis, say) has no
                // window-relative meaning to report, so this falls back to
                // the tightest bound available rather than treating infinity
                // as `Expanded` — an unbounded width is not evidence of a
                // wide window, only of a parent that imposed none.
                constraints.min_width
            };
            crate::Inherited::new(WindowSizeClass::from_width(width), child.clone()).into()
        })
        // Rebuild only when a size-class *boundary* is actually crossed —
        // matching `LayoutBuilder::breakpoints`'s own stated purpose, and
        // meaning a window resized within one class costs nothing here.
        .breakpoints([COMPACT_MEDIUM_BOUNDARY, MEDIUM_EXPANDED_BOUNDARY])
        .into()
    }
}

crate::widget_node_from!(WindowSizeClassProvider);

/// One named, typed value in a [`TokenSet`].
///
/// A thin pair — a `'static` name and a value — rather than just inserting
/// bare values under a name: keeping the name attached to the value (instead
/// of only ever appearing as a `HashMap` key) is what lets
/// [`TokenSet::insert`] take a single argument and what makes a token
/// meaningful on its own if it is ever pulled out of the set and passed
/// around by itself — a debug print, a token inspector, a design-system
/// export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignToken<T> {
    pub name: &'static str,
    pub value: T,
}

impl<T> DesignToken<T> {
    #[must_use]
    pub const fn new(name: &'static str, value: T) -> Self {
        Self { name, value }
    }
}

/// An open, extensible registry of named [`DesignToken`]s, for vocabulary
/// [`ThemeData`](crate::ThemeData)'s fixed fields have no place for.
///
/// # Why this exists alongside the fixed theme structs
///
/// `ThemeData`'s fields are answers this framework already knows: every
/// button reads `colors.primary`, every scale reads `text.body`. A
/// `TokenSet` is the opposite kind of thing — a place for an application's
/// *own* named values, ones this framework has no opinion about and will
/// never read itself: an elevation scale (`"elevation.card"`,
/// `"elevation.dialog"`), a spacing scale with a brand's own step names
/// (`"space.xs"`, `"space.xl"`), an icon-size ramp. Putting these in
/// `ThemeData` would mean either this crate's source growing a field per
/// application's vocabulary, or every application editing a fork of it —
/// neither of which scales. A `TokenSet` is instead published and read the
/// same ambient way `ThemeData` is (see [`Inherited`](crate::Inherited)), but
/// holds whatever an application chooses to put in it.
///
/// # Typed lookup without a panic on the wrong type
///
/// Two different tokens may share a name only by mistake, or two unrelated
/// parts of an application may pick the same name for different kinds of
/// value. [`get`](Self::get) returns `None` for either "no such name" or
/// "that name holds a different type" — a caller cannot tell the two apart
/// from `get` alone, which is deliberate: both mean "the value I expected
/// is not there", and a caller that needs to know which should check
/// [`contains_name`](Self::contains_name) first.
#[derive(Default)]
pub struct TokenSet {
    values: HashMap<&'static str, Box<dyn Any>>,
}

impl TokenSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `token`, replacing any existing value under the same name
    /// (whatever its type was).
    pub fn insert<T: 'static>(&mut self, token: DesignToken<T>) {
        self.values.insert(token.name, Box::new(token.value));
    }

    /// The value named `name`, if one exists and was inserted as a `T`.
    #[must_use]
    pub fn get<T: 'static>(&self, name: &str) -> Option<&T> {
        self.values
            .get(name)
            .and_then(|value| value.downcast_ref::<T>())
    }

    /// `true` if some value — of any type — is stored under `name`.
    ///
    /// Distinguishes "nothing there" from "there, but not the type asked
    /// for" in a way [`get`](Self::get) alone cannot — see this type's own
    /// doc.
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// Remove and return the value named `name`, if it existed and was a
    /// `T`.
    ///
    /// If a value exists under `name` but as a different type, it is **not**
    /// removed — an application asking to take out a token it thinks is a
    /// `Color` should not silently delete an unrelated `f32` that happens to
    /// share the name.
    pub fn remove<T: 'static>(&mut self, name: &str) -> Option<T> {
        self.values.get(name)?.downcast_ref::<T>()?;
        self.values
            .remove(name)
            .and_then(|value| value.downcast::<T>().ok())
            .map(|boxed| *boxed)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("names", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_maps_to_the_documented_three_classes() {
        assert_eq!(WindowSizeClass::from_width(0.0), WindowSizeClass::Compact);
        assert_eq!(WindowSizeClass::from_width(599.9), WindowSizeClass::Compact);
        assert_eq!(WindowSizeClass::from_width(600.0), WindowSizeClass::Medium);
        assert_eq!(WindowSizeClass::from_width(839.9), WindowSizeClass::Medium);
        assert_eq!(
            WindowSizeClass::from_width(840.0),
            WindowSizeClass::Expanded
        );
        assert_eq!(
            WindowSizeClass::from_width(3000.0),
            WindowSizeClass::Expanded
        );
    }

    #[test]
    fn with_nothing_published_of_falls_back_to_compact() {
        assert_eq!(
            WindowSizeClass::of(&crate::BuildContext::root()),
            WindowSizeClass::Compact
        );
    }

    #[test]
    fn of_reads_back_a_published_class() {
        use std::rc::Rc;

        let ctx = crate::BuildContext::root().child_with(Rc::new(WindowSizeClass::Expanded));
        assert_eq!(WindowSizeClass::of(&ctx), WindowSizeClass::Expanded);
    }

    #[test]
    fn the_provider_publishes_a_window_size_class_into_its_subtree() {
        use crate::debug_tree;

        let narrow = WindowSizeClassProvider::new(crate::Text::new("body"));
        let dump = debug_tree(narrow);
        assert!(dump.contains("Inherited"), "{dump}");
    }

    #[test]
    fn a_token_round_trips_through_the_set() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("elevation.card", 2.0_f32));
        tokens.insert(DesignToken::new("space.xl", 32.0_f32));

        assert_eq!(tokens.get::<f32>("elevation.card"), Some(&2.0));
        assert_eq!(tokens.get::<f32>("space.xl"), Some(&32.0));
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn two_different_types_can_share_the_set_under_different_names() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("elevation.card", 2.0_f32));
        tokens.insert(DesignToken::new("brand.name", "Acme".to_owned()));

        assert_eq!(tokens.get::<f32>("elevation.card"), Some(&2.0));
        assert_eq!(tokens.get::<String>("brand.name"), Some(&"Acme".to_owned()));
    }

    #[test]
    fn a_missing_name_returns_none_rather_than_panicking() {
        let tokens = TokenSet::new();
        assert_eq!(tokens.get::<f32>("does.not.exist"), None);
        assert!(!tokens.contains_name("does.not.exist"));
    }

    #[test]
    fn asking_for_the_wrong_type_returns_none_rather_than_panicking() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("space.xl", 32.0_f32));

        assert_eq!(
            tokens.get::<String>("space.xl"),
            None,
            "wrong type must not panic or coerce"
        );
        assert!(
            tokens.contains_name("space.xl"),
            "the name is still there, just under a different type"
        );
    }

    #[test]
    fn inserting_under_an_existing_name_replaces_the_old_value() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("space.xl", 32.0_f32));
        tokens.insert(DesignToken::new("space.xl", 40.0_f32));

        assert_eq!(tokens.get::<f32>("space.xl"), Some(&40.0));
        assert_eq!(tokens.len(), 1);
    }

    #[test]
    fn removing_with_the_right_type_takes_the_value_out() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("space.xl", 32.0_f32));

        assert_eq!(tokens.remove::<f32>("space.xl"), Some(32.0));
        assert!(!tokens.contains_name("space.xl"));
    }

    #[test]
    fn removing_with_the_wrong_type_leaves_the_real_value_untouched() {
        let mut tokens = TokenSet::new();
        tokens.insert(DesignToken::new("space.xl", 32.0_f32));

        assert_eq!(tokens.remove::<String>("space.xl"), None);
        assert!(
            tokens.contains_name("space.xl"),
            "a wrong-typed remove must not delete the real value"
        );
        assert_eq!(tokens.get::<f32>("space.xl"), Some(&32.0));
    }
}
