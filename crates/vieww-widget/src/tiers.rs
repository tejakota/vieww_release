//! Widget tiers: the three-level widget classification the external
//! architectural assessment asks for by name, applied to this crate's real
//! widget catalog.
//!
//! The assessment's §9 puts it directly: the widget layer should have three
//! levels — "Primitive widgets" (`Box`, `Flex`, `Stack`, `Scroll`, `Text`,
//! `Image`, `Transform`, `Clip`, `CustomPaint`), "Behavior widgets"
//! (`Focusable`, `Hoverable`, `Draggable`, `Dismissible`, `Scrollable`,
//! `Animated`, `Transition`, `Portal`, `Overlay`, `Semantics`), and
//! "Design-system widgets" (composed, opinionated, ready-to-use controls) —
//! closing with the point that matters most: "ViewW should allow custom
//! design systems without needing to become forked widgets." [`WidgetTier`]
//! is that classification, [`Tiered`] is how a widget type declares which
//! level it sits at, and [`TierBudget`]/[`TierGate`] is what makes the
//! classification actionable rather than only documentation — a design
//! system built at the [`WidgetTier::Pattern`] level is, definitionally,
//! composed entirely from this crate's [`WidgetTier::Primitive`] and
//! [`WidgetTier::Behavior`] widgets, and can fall back to one of those lower
//! levels under a tight [`TierBudget`] instead of forking anything.
//!
//! This is the design-system engine's third axis, alongside
//! [`ThemeData`](crate::ThemeData) (how a widget looks) and
//! [`WindowSizeClass`](crate::WindowSizeClass) (how much room a widget has)
//! in `tokens`.
//!
//! # What this module deliberately does not do
//!
//! It does not invent a fourth, framework-owned "Design-system widgets"
//! tier of actual components (a `Button`, a `Card`, an `AppBar`) — this
//! crate, by design, ships primitives and behaviors and stops there; see
//! [`WidgetTier::Pattern`]'s own doc for why drawing that line here, rather
//! than shipping opinionated platform-styled controls in
//! this crate, is the point of the assessment's closing line about custom
//! design systems. Nor does it invent a numeric performance cost — comparing
//! tiers with `<=` reflects the assessment's own build-up ordering
//! (a pattern is built from behaviors, a behavior wraps a primitive), not a
//! measured cost the way [`vieww_render_planner`](../vieww_render_planner)'s
//! `DeviceProfile` is. An application that wants a *measured* cost-aware
//! degradation policy composes this classification with that crate's real
//! cost model rather than this module reinventing one.

use std::fmt;

use vieww_foundation::Key;

use crate::{BuildContext, Inherited, Widget, WidgetKind, WidgetNode};

/// Which of the assessment's three widget levels a widget type belongs to.
///
/// Ordered by build-up dependency, not by cost: a [`Self::Pattern`] widget
/// is composed from [`Self::Behavior`] widgets, which wrap
/// [`Self::Primitive`] ones — so [`Self::Primitive`] < [`Self::Behavior`] <
/// [`Self::Pattern`] mirrors the direction composition actually flows, and
/// [`TierBudget::allows`] comparing with `<=` reads as "may this level, or
/// anything simpler than it, be kept mounted."
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WidgetTier {
    /// Raw building blocks with no opinion about behavior or design: layout
    /// (`Container`, `Padding`, `SizedBox`, `Flex`, `Stack`, `Grid`,
    /// `Positioned`, `Align`, `Center`, `Viewport`), static content
    /// (`Text`, `RichText`, `Image`, `Svg`, `Icon`), and drawing
    /// (`ColoredBox`, `DecoratedBox`, `Clip`, `Transformed`, `CustomPaint`).
    /// The assessment's own examples for this level — `Box`, `Flex`,
    /// `Stack`, `Scroll`, `Text`, `Image`, `Transform`, `Clip`,
    /// `CustomPaint` — are, almost one for one, this crate's actual
    /// primitive widgets.
    Primitive,
    /// Widgets that add one cross-cutting capability to whatever primitive
    /// they wrap, without being a finished control on their own:
    /// `GestureDetector`/`Pressable` (draggable/pressable), `FocusTrap`
    /// (focusable), `CursorArea` (hoverable), `Sensitive`/`SensitiveMask`
    /// (dismissible-under-recording), `RepaintBoundary`/`Opacity`
    /// (compositing behavior), `Semantics`/`BlockSemantics`/
    /// `ExcludeSemantics` (accessibility behavior), `Animated`/
    /// `AnimatedContainer` (transition behavior), `SafeArea`/
    /// `LayoutBuilder` (adaptive behavior). Matches the assessment's own
    /// examples — `Focusable`, `Hoverable`, `Draggable`, `Dismissible`,
    /// `Scrollable`, `Animated`, `Transition`, `Portal`, `Overlay`,
    /// `Semantics` — closely enough that several are named identically.
    Behavior,
    /// Composed, opinionated, ready-to-use controls built from the two
    /// levels below: `TextField`, `Carousel`, `ColorPicker`,
    /// `LineChart`/`BarChart`, `ShapeMorph`, `PerformanceOverlay`,
    /// `AsyncBuilder`/`StreamBuilder`. The assessment calls this level
    /// "Design-system widgets" and gives platform-styled
    /// examples; this crate deliberately ships this level's *mechanism*
    /// (real, composed, non-trivial controls, and the tokens in
    /// `tokens` a design system themes them with) without
    /// also shipping a framework-owned branded or platform widget set — doing
    /// that here is exactly the fork the assessment's closing line warns
    /// against; a design system is meant to build its *own* `Pattern`-level
    /// widgets from this crate's `Primitive` and `Behavior` ones instead.
    Pattern,
}

impl fmt::Display for WidgetTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Primitive => "primitive",
            Self::Behavior => "behavior",
            Self::Pattern => "pattern",
        };
        f.write_str(name)
    }
}

/// A widget type that declares which [`WidgetTier`] it belongs to.
///
/// An associated function rather than a method on `&self`, deliberately: a
/// widget's tier is a property of *what kind of widget it is*, not of any
/// one instance's field values — the same reasoning
/// [`Widget::kind`](crate::Widget::kind) documents for why its own return
/// value must not vary between instances of one type. Implemented here for
/// this crate's own widgets, classified against the assessment's three
/// levels rather than guessed; an application's own `Pattern`-level widgets
/// — the ones a real design system built on this crate would define —
/// implement this the same way to participate in [`TierGate`] fallbacks.
pub trait Tiered {
    /// This widget type's tier.
    fn tier() -> WidgetTier;
}

macro_rules! tiered {
    ($tier:expr => $($ty:ty),+ $(,)?) => {
        $(
            impl Tiered for $ty {
                fn tier() -> WidgetTier {
                    $tier
                }
            }
        )+
    };
}

tiered!(WidgetTier::Primitive =>
    crate::Container,
    crate::Padding,
    crate::SizedBox,
    crate::Constrained,
    crate::Flex,
    crate::Stack,
    crate::Grid,
    crate::Align,
    crate::Center,
    crate::ColoredBox,
    crate::DecoratedBox,
    crate::Text,
    crate::RichText,
    crate::Image,
    crate::Svg,
    crate::Icon,
    crate::Clip,
    crate::Transformed,
    crate::CustomPaint,
    crate::Viewport,
    crate::IntrinsicSize,
    crate::AspectRatio,
);

tiered!(WidgetTier::Behavior =>
    crate::GestureDetector,
    crate::Pressable,
    crate::FocusTrap,
    crate::CursorArea,
    crate::IgnorePointer,
    crate::Sensitive,
    crate::RepaintBoundary,
    crate::Opacity,
    crate::Semantics,
    crate::BlockSemantics,
    crate::ExcludeSemantics,
    crate::Animated,
    crate::AnimatedContainer,
    crate::SafeArea,
    crate::LayoutBuilder,
);

tiered!(WidgetTier::Pattern =>
    crate::TextField,
    crate::Carousel,
    crate::ColorPicker,
    crate::LineChart,
    crate::BarChart,
    crate::ShapeMorph,
    crate::PerformanceOverlay,
);

/// How many [`WidgetTier`] levels a subtree is currently willing to keep
/// mounted at.
///
/// Published near the root — or anywhere a subtree wants to restrict further
/// than its ancestor did — with [`TierBudgetProvider`], and read anywhere
/// below with [`TierBudget::of`]. This is not, on its own, a measured
/// performance policy: it is the same shared vocabulary
/// [`WindowSizeClass`](crate::WindowSizeClass) is for window size, applied
/// to "how far up the Primitive → Behavior → Pattern build-up is this
/// subtree allowed to reach." A real degradation *decision* — when to
/// actually lower a `TierBudget` — belongs to whatever real signal an
/// application has (a measured frame-time history, a
/// `vieww-render-planner::DeviceProfile`, a platform battery API); this type
/// is only the comparison, not the sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TierBudget {
    max: WidgetTier,
}

impl TierBudget {
    /// A budget that allows every tier up to and including `max`.
    #[must_use]
    pub const fn up_to(max: WidgetTier) -> Self {
        Self { max }
    }

    /// Every tier is allowed, including [`WidgetTier::Pattern`]. The default
    /// in the absence of any published budget — see [`Self::of`] for why
    /// that, and not [`Self::primitives_only`], is the right fallback.
    #[must_use]
    pub const fn all() -> Self {
        Self::up_to(WidgetTier::Pattern)
    }

    /// Only [`WidgetTier::Primitive`] widgets are allowed — the most
    /// restrictive budget, falling all the way back to raw layout and
    /// static content.
    #[must_use]
    pub const fn primitives_only() -> Self {
        Self::up_to(WidgetTier::Primitive)
    }

    /// The highest tier this budget currently allows.
    #[must_use]
    pub const fn max(&self) -> WidgetTier {
        self.max
    }

    /// Whether a widget at `tier` may be kept mounted under this budget.
    #[must_use]
    pub fn allows(&self, tier: WidgetTier) -> bool {
        tier <= self.max
    }

    /// The budget published above this position, or [`Self::all`] if none
    /// was.
    ///
    /// [`Self::all`] — not [`Self::primitives_only`] — is the fallback, and
    /// deliberately the opposite choice from
    /// [`WindowSizeClass::of`](crate::WindowSizeClass::of)'s conservative
    /// [`Compact`](crate::WindowSizeClass::Compact) default. A window size
    /// class is *always* true of some real window, so guessing the narrowest
    /// one is a safe layout default. Tier gating is opt-in degradation: a
    /// tree with nothing wired up to a `TierBudgetProvider` — nearly every
    /// tree, in this workspace or an application's, most of the time —
    /// should behave exactly as it did before this module existed, not have
    /// every `Pattern`-level widget silently fall back because nobody
    /// published a budget.
    #[must_use]
    pub fn of(ctx: &BuildContext) -> Self {
        ctx.inherit::<Self>().map_or(Self::all(), |budget| *budget)
    }
}

/// Publishes a [`TierBudget`] to its subtree.
///
/// Thin by design, the same shape as
/// [`WindowSizeClassProvider`](crate::WindowSizeClassProvider): this is
/// [`Inherited`]`<TierBudget>` under a named type, so "publish a tier
/// budget" reads the same way at a call site as "publish a window size
/// class" or "publish a theme."
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{TierBudget, TierBudgetProvider, WidgetTier};
///
/// let app = TierBudgetProvider::new(
///     TierBudget::up_to(WidgetTier::Behavior),
///     Text::new("body"),
/// );
/// let _ = vieww_widget::debug_tree(app);
/// ```
pub struct TierBudgetProvider {
    budget: TierBudget,
    child: WidgetNode,
    key: Option<Key>,
}

impl TierBudgetProvider {
    #[must_use]
    pub fn new(budget: TierBudget, child: impl Into<WidgetNode>) -> Self {
        Self {
            budget,
            child: child.into(),
            key: None,
        }
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl fmt::Debug for TierBudgetProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TierBudgetProvider")
            .field("budget", &self.budget)
            .field("child", &self.child)
            .finish()
    }
}

impl Widget for TierBudgetProvider {
    fn debug_name(&self) -> &'static str {
        "TierBudgetProvider"
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Inherited::new(self.budget, self.child.clone()).into()
    }
}

crate::widget_node_from!(TierBudgetProvider);

/// Renders one widget while the ambient [`TierBudget`] allows a given
/// [`WidgetTier`], and a `fallback` otherwise.
///
/// This is the mechanism that makes the assessment's closing line about
/// custom design systems actionable rather than aspirational: a
/// `Pattern`-level widget a design system built from this crate's
/// primitives and behaviors can name its own tier and a simpler
/// `Behavior`- or `Primitive`-level replacement, and fall back to it under a
/// tight [`TierBudget`] without the design system forking anything in this
/// crate to do so.
///
/// # Why this takes two already-built [`WidgetNode`]s rather than being
/// generic over `W: Widget + Tiered`
///
/// [`Tiered::tier`] is how a widget type declares its tier once, so it is
/// not repeated at every place that widget gets mounted. This type is the
/// other half — the gate itself — and it is deliberately not generic over
/// the gated widget's own type: forcing every call site through `W: Tiered`
/// would mean a plain `Text` fallback could not sit next to a gated
/// `Carousel` without `Text` also implementing `Tiered` at some tier, when
/// the honest fact is only the *gated* side has a tier opinion at all. A
/// call site that does have a `Tiered` type on hand uses [`Self::for_widget`]
/// instead of naming the tier by hand.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Carousel, TierGate, WidgetTier};
///
/// // Falls back to a plain label when the ambient budget is tight, rather
/// // than keeping an auto-advancing Pattern-level carousel mounted on a
/// // constrained device.
/// let gated = TierGate::new(
///     WidgetTier::Pattern,
///     Carousel::new(160.0, 8.0).children(vec![Text::new("slide one").into()]),
///     Text::new("gallery unavailable in low-power mode"),
/// );
/// let _ = vieww_widget::debug_tree(gated);
/// ```
pub struct TierGate {
    tier: WidgetTier,
    enhanced: WidgetNode,
    fallback: WidgetNode,
    key: Option<Key>,
}

impl TierGate {
    #[must_use]
    pub fn new(
        tier: WidgetTier,
        enhanced: impl Into<WidgetNode>,
        fallback: impl Into<WidgetNode>,
    ) -> Self {
        Self {
            tier,
            enhanced: enhanced.into(),
            fallback: fallback.into(),
            key: None,
        }
    }

    /// Gate a widget that already implements [`Tiered`], reading its tier
    /// with `T::tier()` instead of repeating it at the call site.
    #[must_use]
    pub fn for_widget<T>(enhanced: T, fallback: impl Into<WidgetNode>) -> Self
    where
        T: Tiered + Into<WidgetNode>,
    {
        let tier = T::tier();
        Self::new(tier, enhanced, fallback)
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl fmt::Debug for TierGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TierGate")
            .field("tier", &self.tier)
            .field("enhanced", &self.enhanced)
            .field("fallback", &self.fallback)
            .finish()
    }
}

impl Widget for TierGate {
    fn debug_name(&self) -> &'static str {
        "TierGate"
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("tier", self.tier.to_string())]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        if TierBudget::of(ctx).allows(self.tier) {
            self.enhanced.clone()
        } else {
            self.fallback.clone()
        }
    }
}

crate::widget_node_from!(TierGate);

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::debug_tree;

    #[test]
    fn tiers_order_primitive_below_behavior_below_pattern() {
        assert!(WidgetTier::Primitive < WidgetTier::Behavior);
        assert!(WidgetTier::Behavior < WidgetTier::Pattern);
        assert!(WidgetTier::Primitive < WidgetTier::Pattern);
    }

    #[test]
    fn curated_widgets_report_the_documented_tier() {
        assert_eq!(crate::Container::tier(), WidgetTier::Primitive);
        assert_eq!(crate::Text::tier(), WidgetTier::Primitive);
        assert_eq!(crate::CustomPaint::tier(), WidgetTier::Primitive);
        assert_eq!(crate::GestureDetector::tier(), WidgetTier::Behavior);
        assert_eq!(crate::Semantics::tier(), WidgetTier::Behavior);
        assert_eq!(crate::TextField::tier(), WidgetTier::Pattern);
        assert_eq!(crate::Carousel::tier(), WidgetTier::Pattern);
    }

    #[test]
    fn a_budget_up_to_behavior_allows_primitive_and_behavior_not_pattern() {
        let budget = TierBudget::up_to(WidgetTier::Behavior);
        assert!(budget.allows(WidgetTier::Primitive));
        assert!(budget.allows(WidgetTier::Behavior));
        assert!(!budget.allows(WidgetTier::Pattern));
    }

    #[test]
    fn primitives_only_allows_exactly_primitive() {
        let budget = TierBudget::primitives_only();
        assert!(budget.allows(WidgetTier::Primitive));
        assert!(!budget.allows(WidgetTier::Behavior));
        assert!(!budget.allows(WidgetTier::Pattern));
    }

    #[test]
    fn all_allows_every_tier() {
        let budget = TierBudget::all();
        assert!(budget.allows(WidgetTier::Primitive));
        assert!(budget.allows(WidgetTier::Behavior));
        assert!(budget.allows(WidgetTier::Pattern));
    }

    #[test]
    fn with_nothing_published_of_falls_back_to_all_not_primitives_only() {
        let budget = TierBudget::of(&BuildContext::root());
        assert_eq!(
            budget,
            TierBudget::all(),
            "an unwired tree must not silently lose Pattern-level widgets"
        );
    }

    #[test]
    fn of_reads_back_a_published_budget() {
        let published = TierBudget::up_to(WidgetTier::Behavior);
        let ctx = BuildContext::root().child_with(Rc::new(published));
        assert_eq!(TierBudget::of(&ctx), published);
    }

    #[test]
    fn the_provider_publishes_a_tier_budget_into_its_subtree() {
        let tree = TierBudgetProvider::new(TierBudget::primitives_only(), crate::Text::new("body"));
        let dump = debug_tree(tree);
        assert!(dump.contains("Inherited"), "{dump}");
    }

    #[test]
    fn a_gate_with_no_budget_published_renders_the_pattern_child() {
        let gate = TierGate::new(
            WidgetTier::Pattern,
            crate::Text::new("enhanced"),
            crate::Text::new("fallback"),
        );
        let dump = debug_tree(gate);
        assert!(dump.contains("enhanced"), "{dump}");
        assert!(!dump.contains("fallback"), "{dump}");
    }

    #[test]
    fn a_gate_under_a_primitives_only_budget_renders_the_fallback() {
        let gate = TierGate::new(
            WidgetTier::Pattern,
            crate::Text::new("enhanced"),
            crate::Text::new("fallback"),
        );
        let tree = TierBudgetProvider::new(TierBudget::primitives_only(), gate);
        let dump = debug_tree(tree);
        assert!(dump.contains("fallback"), "{dump}");
        assert!(!dump.contains("enhanced"), "{dump}");
    }

    #[test]
    fn a_gate_under_a_wide_enough_budget_renders_the_pattern_child() {
        let gate = TierGate::new(
            WidgetTier::Behavior,
            crate::Text::new("enhanced"),
            crate::Text::new("fallback"),
        );
        let tree = TierBudgetProvider::new(TierBudget::up_to(WidgetTier::Behavior), gate);
        let dump = debug_tree(tree);
        assert!(dump.contains("enhanced"), "{dump}");
        assert!(!dump.contains("fallback"), "{dump}");
    }

    #[test]
    fn a_narrower_inner_provider_overrides_a_wider_outer_one() {
        let gate = TierGate::new(
            WidgetTier::Behavior,
            crate::Text::new("enhanced"),
            crate::Text::new("fallback"),
        );
        let inner = TierBudgetProvider::new(TierBudget::primitives_only(), gate);
        let tree = TierBudgetProvider::new(TierBudget::all(), inner);
        let dump = debug_tree(tree);
        assert!(
            dump.contains("fallback"),
            "the innermost published budget must win: {dump}"
        );
    }

    #[test]
    fn for_widget_reads_the_tier_from_the_tiered_impl_without_naming_it() {
        let carousel =
            crate::Carousel::new(160.0, 8.0).children(vec![crate::Text::new("slide").into()]);
        let gate = TierGate::for_widget(carousel, crate::Text::new("fallback"));
        let tree = TierBudgetProvider::new(TierBudget::primitives_only(), gate);
        let dump = debug_tree(tree);
        assert!(dump.contains("fallback"), "{dump}");
    }

    #[test]
    fn tier_display_names_are_lowercase_words() {
        assert_eq!(WidgetTier::Primitive.to_string(), "primitive");
        assert_eq!(WidgetTier::Behavior.to_string(), "behavior");
        assert_eq!(WidgetTier::Pattern.to_string(), "pattern");
    }
}
