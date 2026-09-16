use std::fmt;

use vieww_foundation::{
    Alignment, AlignmentDirectional, Border, BoxDecoration, Color, Constraints, EdgeInsets,
    EdgeInsetsDirectional, Gradient, Key, Shadow, TextDirection,
};

use crate::{
    widget_node_from, Align, BuildContext, ColoredBox, Constrained, DecoratedBox, Directionality,
    Padding, SizedBox, Widget, WidgetKind, WidgetNode,
};

/// A padding or margin, however it was expressed.
///
/// **Not a trait shared by the two inset types.**
/// [`EdgeInsetsDirectional`] deliberately offers no way to read `.left`, so that
/// treating a directional inset as a physical one is unwritable rather than
/// merely discouraged, and a shared trait would hand that back. An enum keeps
/// the property: [`resolve`](Self::resolve) is still the only way out.
///
/// Private, so this adds no public surface — the two builders on
/// [`Container`] are the whole API.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Insets {
    Physical(EdgeInsets),
    Directional(EdgeInsetsDirectional),
}

impl Insets {
    const fn resolve(self, direction: TextDirection) -> EdgeInsets {
        match self {
            Self::Physical(insets) => insets,
            Self::Directional(insets) => insets.resolve(direction),
        }
    }

    const fn is_directional(self) -> bool {
        matches!(self, Self::Directional(_))
    }
}

impl fmt::Display for Insets {
    /// Prints what was *written*, unresolved.
    ///
    /// A tree dump should say `start: 16` where the author wrote `start: 16`;
    /// printing the resolved value would make the dump silently disagree with
    /// the source on every right-to-left screen.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `write!` rather than `insets.fmt(f)`: `use std::fmt;` imports the
        // module, not the `Display` trait, so the delegating call would not
        // resolve — and if `Debug` were ever imported alongside it, it would
        // resolve to the wrong one instead of failing.
        match self {
            Self::Physical(insets) => write!(f, "{insets}"),
            Self::Directional(insets) => write!(f, "{insets}"),
        }
    }
}

/// An alignment, however it was expressed. See [`Insets`] for why this is an
/// enum rather than a trait.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Anchor {
    Physical(Alignment),
    Directional(AlignmentDirectional),
}

impl Anchor {
    const fn resolve(self, direction: TextDirection) -> Alignment {
        match self {
            Self::Physical(alignment) => alignment,
            Self::Directional(alignment) => alignment.resolve(direction),
        }
    }

    const fn is_directional(self) -> bool {
        matches!(self, Self::Directional(_))
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Physical(alignment) => write!(f, "{alignment}"),
            Self::Directional(alignment) => write!(f, "{alignment}"),
        }
    }
}

/// The convenience box: padding, margin, alignment, a background color and a
/// fixed size, in one widget.
///
/// `Container` owns no layout logic. It is [`Composed`](WidgetKind::Composed) —
/// it assembles [`Padding`], [`Align`], [`ColoredBox`] and [`Constrained`],
/// each of which owns exactly one behaviour, and only wraps the ones you
/// actually set. A `Container` with nothing configured builds to just its child.
///
/// # Wrapping order
///
/// From the child outwards: **align → padding → color → constraints → margin**.
///
/// The order is load-bearing and matches the classic one. Two consequences worth
/// knowing, because they are the usual source of "why doesn't this look right":
///
/// - padding is *inside* the color, so the background paints behind the padded
///   region; margin is outside it, so it does not;
/// - the alignment applies within the padded area, not the full box.
#[derive(Debug, Clone, Default)]
pub struct Container {
    child: Option<WidgetNode>,
    padding: Option<Insets>,
    margin: Option<Insets>,
    alignment: Option<Anchor>,
    color: Option<Color>,
    /// Set by [`radius`](Container::radius), [`gradient`](Container::gradient),
    /// [`shadow`](Container::shadow) and [`border`](Container::border).
    ///
    /// `None` is the fast path and is what most containers stay on: a flat fill
    /// becomes a `ColoredBox`, which records one `FillRect`. The moment any of
    /// the four is set the fill moves into a `DecoratedBox` instead, because a
    /// rounded corner or a shadow is not something a rectangle of colour can
    /// express.
    decoration: Option<BoxDecoration>,
    width: Option<f32>,
    height: Option<f32>,
    key: Option<Key>,
}

impl Container {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the child.
    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Inset the child. Painted over by [`color`](Self::color).
    #[must_use]
    pub const fn padding(mut self, padding: EdgeInsets) -> Self {
        self.padding = Some(Insets::Physical(padding));
        self
    }

    /// Inset the child against the *reading* direction.
    ///
    /// `start` is the edge text begins at, so a label indented from where
    /// reading starts stays indented in Arabic instead of jumping to the far
    /// side. Resolved during this container's own `build`, against the nearest
    /// [`Directionality`].
    ///
    /// **This costs no extra element**, unlike
    /// [`PaddingDirectional`](crate::PaddingDirectional): a `Container` is
    /// already composed and already runs a `build`, so it resolves in the one it
    /// was going to run anyway. Reaching for the wrapper around a `Container`
    /// is the same layout for one more node.
    ///
    /// The last of this and [`padding`](Self::padding) to be called wins; they
    /// are one property, not two.
    #[must_use]
    pub const fn padding_directional(mut self, padding: EdgeInsetsDirectional) -> Self {
        self.padding = Some(Insets::Directional(padding));
        self
    }

    /// Inset the whole container from its parent. Not painted over by
    /// [`color`](Self::color).
    #[must_use]
    pub const fn margin(mut self, margin: EdgeInsets) -> Self {
        self.margin = Some(Insets::Physical(margin));
        self
    }

    /// Inset the whole container against the reading direction.
    ///
    /// The last of this and [`margin`](Self::margin) to be called wins.
    #[must_use]
    pub const fn margin_directional(mut self, margin: EdgeInsetsDirectional) -> Self {
        self.margin = Some(Insets::Directional(margin));
        self
    }

    /// Position the child within the padded area. Setting this makes the
    /// container expand to fill its bounded constraints.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(Anchor::Physical(alignment));
        self
    }

    /// Position the child against the reading direction, so that
    /// [`CENTER_START`](AlignmentDirectional::CENTER_START) is the left edge in
    /// Latin and the right edge in Arabic.
    ///
    /// The last of this and [`alignment`](Self::alignment) to be called wins.
    #[must_use]
    pub const fn alignment_directional(mut self, alignment: AlignmentDirectional) -> Self {
        self.alignment = Some(Anchor::Directional(alignment));
        self
    }

    /// Fill the background.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Round the corners of the background.
    ///
    /// This rounds what the container *paints*. It does not clip the child —
    /// wrap it in [`Clip`](crate::Clip) for that, which costs a compositing
    /// operation this does not.
    #[must_use]
    pub fn radius(mut self, radius: f32) -> Self {
        self.decoration_mut().radius = radius;
        self
    }

    /// Fill the background with a ramp of colours, over any flat
    /// [`color`](Self::color).
    #[must_use]
    pub fn gradient(mut self, gradient: Gradient) -> Self {
        self.decoration_mut().gradient = Some(gradient);
        self
    }

    /// Cast a shadow behind the container.
    #[must_use]
    pub fn shadow(mut self, shadow: Shadow) -> Self {
        self.decoration_mut().shadow = Some(shadow);
        self
    }

    /// Draw a border just inside the container's bounds.
    #[must_use]
    pub fn border(mut self, border: Border) -> Self {
        self.decoration_mut().border = Some(border);
        self
    }

    /// Set the whole decoration at once, replacing anything already configured.
    ///
    /// The escape hatch for a decoration built elsewhere — a theme token, most
    /// often. [`color`](Self::color) still applies over it, because the two are
    /// set independently and the last writer of a *field* should win rather than
    /// the last writer of the struct.
    #[must_use]
    pub fn decoration(mut self, decoration: BoxDecoration) -> Self {
        self.decoration = Some(decoration);
        self
    }

    fn decoration_mut(&mut self) -> &mut BoxDecoration {
        self.decoration.get_or_insert_with(BoxDecoration::new)
    }

    /// Force a width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Force a height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Force both dimensions.
    #[must_use]
    pub const fn size(self, width: f32, height: f32) -> Self {
        self.width(width).height(height)
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Whether anything here has to ask which way the interface reads.
    ///
    /// Exists so that `build` can skip the ancestor walk entirely on the
    /// common path. A container that sets only physical values must cost the
    /// same as it did before directional support was added.
    fn reads_direction(&self) -> bool {
        self.padding.is_some_and(Insets::is_directional)
            || self.margin.is_some_and(Insets::is_directional)
            || self.alignment.is_some_and(Anchor::is_directional)
    }
}

impl Widget for Container {
    fn debug_name(&self) -> &'static str {
        "Container"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        /// Wrap the current subtree, if any, in `$wrapper`.
        macro_rules! wrap {
            ($current:expr, $wrapper:expr) => {{
                let wrapper = $wrapper;
                Some(match $current {
                    Some(child) => WidgetNode::from(wrapper.child(child)),
                    None => WidgetNode::from(wrapper),
                })
            }};
        }

        // Looked up only when something on this container actually needs it:
        // `inherit` walks a chain of providers, and the overwhelming majority of
        // containers are physical and would pay that walk for nothing.
        //
        // `Ltr` is a safe stand-in rather than a guess when nothing is
        // directional, because `Insets::Physical` and `Anchor::Physical` resolve
        // to themselves whatever direction they are handed — the value is
        // unused, not defaulted.
        let direction = if self.reads_direction() {
            Directionality::of(ctx)
        } else {
            TextDirection::Ltr
        };

        // Each wrapper is added only when its property was set, so an unused
        // `Container` costs nothing at layout time. Order is documented on the
        // type and must not be shuffled.
        let mut current: Option<WidgetNode> = self.child.clone();

        if let Some(alignment) = self.alignment {
            current = wrap!(current, Align::new(alignment.resolve(direction)));
        }

        if let Some(padding) = self.padding {
            current = wrap!(current, Padding::new(padding.resolve(direction)));
        }

        // A flat fill stays a `ColoredBox` — one `FillRect`, damage tracking's
        // cheapest case, and what most of a tree is. Anything a rectangle of
        // colour cannot express moves to a `DecoratedBox`.
        match (self.decoration, self.color) {
            (Some(decoration), color) => {
                let decoration = match color {
                    Some(color) => decoration.color(color),
                    None => decoration,
                };
                if !decoration.is_invisible() {
                    current = wrap!(current, DecoratedBox::new(decoration));
                }
            }
            (None, Some(color)) => {
                current = wrap!(current, ColoredBox::new(color));
            }
            (None, None) => {}
        }

        if self.width.is_some() || self.height.is_some() {
            let constraints = Constraints::UNBOUNDED.tighten(self.width, self.height);
            current = wrap!(current, Constrained::new(constraints));
        }

        if let Some(margin) = self.margin {
            current = wrap!(current, Padding::new(margin.resolve(direction)));
        }

        // A container with no child and no properties still has to build
        // *something*; a zero-size box is the honest answer.
        current.unwrap_or_else(|| SizedBox::shrink().into())
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = Vec::new();
        if let Some(alignment) = self.alignment {
            props.push(("alignment", alignment.to_string()));
        }
        if let Some(padding) = self.padding {
            props.push(("padding", padding.to_string()));
        }
        if let Some(color) = self.color {
            props.push(("color", color.to_string()));
        }
        if let Some(width) = self.width {
            props.push(("width", width.to_string()));
        }
        if let Some(height) = self.height {
            props.push(("height", height.to_string()));
        }
        if let Some(margin) = self.margin {
            props.push(("margin", margin.to_string()));
        }
        props
    }
}

widget_node_from!(Container);

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::debug_tree;

    /// A context reading `direction`, as a [`Directionality`] ancestor produces.
    fn reading(direction: TextDirection) -> BuildContext {
        BuildContext::root().child_with(Rc::new(direction))
    }

    /// The insets on the built subtree's outermost `Padding`.
    ///
    /// Sound only when padding or margin is the *only* wrapper set, since the
    /// build order puts a margin outside a padding.
    fn built_insets(container: Container, ctx: &BuildContext) -> EdgeInsets {
        container
            .build(ctx)
            .downcast_ref::<Padding>()
            .expect("a Padding is outermost when it is the only wrapper set")
            .insets()
    }

    fn built_alignment(container: Container, ctx: &BuildContext) -> Alignment {
        container
            .build(ctx)
            .downcast_ref::<Align>()
            .expect("an Align is outermost when it is the only wrapper set")
            .alignment()
    }

    #[test]
    fn a_physical_padding_does_not_move_in_a_right_to_left_subtree() {
        // The no-regression guarantee, and the reason the two builders are
        // separate: every `Container` written before this existed must lay out
        // identically, including inside an Arabic screen where the author
        // deliberately wants a physical edge.
        let container = Container::new().padding(EdgeInsets::only(16.0, 4.0, 8.0, 2.0));
        assert_eq!(
            built_insets(container.clone(), &reading(TextDirection::Rtl)),
            EdgeInsets::only(16.0, 4.0, 8.0, 2.0)
        );
        assert_eq!(
            built_insets(container, &reading(TextDirection::Ltr)),
            EdgeInsets::only(16.0, 4.0, 8.0, 2.0)
        );
    }

    #[test]
    fn a_directional_padding_lands_on_the_edge_reading_begins_at() {
        let container =
            Container::new().padding_directional(EdgeInsetsDirectional::only(16.0, 4.0, 8.0, 2.0));

        assert_eq!(
            built_insets(container.clone(), &reading(TextDirection::Ltr)),
            EdgeInsets::only(16.0, 4.0, 8.0, 2.0),
            "start is the left edge in Latin"
        );

        let rtl = built_insets(container, &reading(TextDirection::Rtl));
        assert_eq!(rtl.right, 16.0, "start is the right edge in Arabic");
        assert_eq!(rtl.left, 8.0, "end is the left edge in Arabic");
        assert_eq!(
            (rtl.top, rtl.bottom),
            (4.0, 2.0),
            "the vertical edges have no handedness"
        );
    }

    #[test]
    fn a_directional_margin_swaps_edges_too() {
        // Margin is the outermost wrapper, so this also pins that resolution
        // happens for every directional field rather than only the first one
        // `build` happens to reach.
        let container =
            Container::new().margin_directional(EdgeInsetsDirectional::only(24.0, 0.0, 6.0, 0.0));
        let rtl = built_insets(container, &reading(TextDirection::Rtl));
        assert_eq!((rtl.left, rtl.right), (6.0, 24.0));
    }

    #[test]
    fn a_directional_alignment_resolves_against_the_ancestor() {
        let container = Container::new().alignment_directional(AlignmentDirectional::CENTER_START);
        assert_eq!(
            built_alignment(container.clone(), &reading(TextDirection::Ltr)),
            Alignment::CENTER_LEFT
        );
        assert_eq!(
            built_alignment(container, &reading(TextDirection::Rtl)),
            Alignment::CENTER_RIGHT
        );
    }

    #[test]
    fn padding_and_padding_directional_are_one_property_and_the_last_one_wins() {
        // They write the same field on purpose. Two fields would let a container
        // carry both and leave "which applies" to build order — a question with
        // no good answer that the type system should not be asking.
        let directional_last = Container::new()
            .padding(EdgeInsets::all(4.0))
            .padding_directional(EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0));
        let rtl = built_insets(directional_last, &reading(TextDirection::Rtl));
        assert_eq!((rtl.left, rtl.right), (8.0, 16.0));

        let physical_last = Container::new()
            .padding_directional(EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0))
            .padding(EdgeInsets::all(4.0));
        assert_eq!(
            built_insets(physical_last, &reading(TextDirection::Rtl)),
            EdgeInsets::all(4.0),
            "a physical inset set last is not re-read as directional"
        );
    }

    #[test]
    fn a_container_with_nothing_directional_builds_with_no_directionality_above_it() {
        // `reads_direction` short-circuits the ancestor walk here, so this also
        // covers the path where `Directionality::of` is never called at all.
        let built = Container::new()
            .padding(EdgeInsets::all(8.0))
            .child(SizedBox::shrink())
            .build(&BuildContext::root());
        assert_eq!(
            built
                .downcast_ref::<Padding>()
                .expect("padding wraps the child")
                .insets(),
            EdgeInsets::all(8.0)
        );
    }

    #[test]
    fn a_tree_dump_prints_what_was_written_rather_than_what_it_resolved_to() {
        // A dump that said `left: 8` where the source says `start: 16` would be
        // actively misleading on the screens this feature exists for.
        let dump = debug_tree(
            Container::new()
                .padding_directional(EdgeInsetsDirectional::only(16.0, 0.0, 8.0, 0.0))
                .alignment_directional(AlignmentDirectional::TOP_END),
        );
        assert!(dump.contains("start: 16"), "{dump}");
        assert!(dump.contains("topEnd"), "{dump}");
    }
}
