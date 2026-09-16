use std::fmt;
use std::rc::Rc;

use vieww_foundation::{BoxDecoration, Color, Constraints, EdgeInsets, IconData, Key, TextStyle};

use crate::{
    widget_node_from, BuildContext, ColorScheme, Constrained, CrossAxisAlignment, DecoratedBox,
    Flex, Flexible, Handler, Icon, MainAxisAlignment, MainAxisSize, Padding, Pressable,
    SemanticRole, Semantics, SizedBox, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How tall the bar is, whatever is in it.
///
/// Sixty-four logical pixels: an icon at its designed 24, its pill, a label
/// under it, and the padding that keeps the whole destination inside a finger.
/// Fixed rather than grown from the content, for the reason
/// [`TabBar`](crate::TabBar)'s height is — a bar a few pixels taller in one
/// translation moves every pixel of the page above it.
const BAR_HEIGHT: f32 = 64.0;

/// How big the icon in a destination is.
const ICON: f32 = 24.0;

/// The rule separating the bar from the page above it.
///
/// One logical pixel — a *hairline*, which on a 2.75x screen is not one device
/// pixel and should not be. It is drawn in `outline`, so on a palette whose
/// separators are deliberately faint (Apple's are 1.79:1 against their own
/// surface) it stays faint, which is the platform being itself rather than this
/// control disagreeing with it.
const HAIRLINE: f32 = 1.0;

/// One destination: what it looks like and what it is called.
#[derive(Debug, Clone)]
pub struct BottomNavItem {
    icon: IconData,
    label: String,
}

impl BottomNavItem {
    #[must_use]
    pub fn new(icon: IconData, label: impl Into<String>) -> Self {
        Self {
            icon,
            label: label.into(),
        }
    }

    /// What a screen reader calls this destination, and what is written under
    /// the icon.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A row of destinations at the bottom of a screen, of which one is showing.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{icons, BottomNavItem, BottomNavigation};
/// use std::rc::Rc;
///
/// # let page = 0;
/// # let show: Rc<dyn Fn(usize)> = Rc::new(|_| {});
/// let nav = BottomNavigation::new(
///     [
///         BottomNavItem::new(icons::check(), "Done"),
///         BottomNavItem::new(icons::add(), "New"),
///     ],
///     page,
/// )
/// .on_selected(show);
/// ```
///
/// # It is a [`TabBar`](crate::TabBar) with an icon and a different skin
///
/// Same equal shares, same [`SemanticRole::Tab`], same rule about what may
/// change when the selection moves. A screen reader is told the same thing about
/// both, because to a screen reader they *are* the same thing — one of a set of
/// pages, and choosing it replaces what is below. Where they differ is entirely
/// visual: a tab bar underlines, a navigation bar puts a pill behind the icon.
///
/// # The label is always shown
///
/// One platform hides the labels of unselected destinations in one of its
/// configurations. That is not offered here: an icon with no word under it is a
/// guess, and the guess is worse in every language the icons were not designed
/// in. It also changes the bar's height when the selection moves, which is the
/// thing this control most needs not to do.
///
/// # The pill is always laid out; only its colour changes
///
/// Exactly [`TabBar`](crate::TabBar)'s rule, and the reason it is stated in both
/// places: an unselected destination's pill is transparent, and a transparent
/// decoration records no draw calls at all — so the cost of the rule is one
/// widget per destination and not one paint command.
///
/// The alternative is a pill that exists only under the selected icon, which
/// moves every other destination's icon by the pill's padding the moment the
/// selection changes.
///
/// # Icons come from the application
///
/// The built-in [`icons`](crate::icons) set is the handful the controls in this
/// crate draw with — a tick, a chevron, a plus — and is not a navigation set. A
/// destination takes whatever [`IconData`] the application has.
#[derive(Clone)]
pub struct BottomNavigation {
    items: Vec<BottomNavItem>,
    selected: usize,
    on_selected: Option<Handler<usize>>,
    key: Option<Key>,
}

impl BottomNavigation {
    /// A bar over `items`, showing the page at `selected`.
    ///
    /// An out-of-range `selected` highlights nothing rather than panicking, for
    /// the reason [`TabBar::new`](crate::TabBar::new) gives.
    #[must_use]
    pub fn new<I>(items: I, selected: usize) -> Self
    where
        I: IntoIterator<Item = BottomNavItem>,
    {
        Self {
            items: items.into_iter().collect(),
            selected,
            on_selected: None,
            key: None,
        }
    }

    /// Called with the index the user chose. Giving a handler enables it.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<usize>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.on_selected.is_some()
    }

    /// How many destinations there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` when there is nothing to choose between.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The index currently showing, if it addresses a destination that exists.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        (self.selected < self.items.len()).then_some(self.selected)
    }

    /// One destination's pill colour and ink.
    fn colors(&self, theme: &ThemeData, chosen: bool) -> (Color, Color) {
        let colors = theme.colors;
        let (pill, ink) = if chosen {
            (colors.surface_variant, colors.primary)
        } else {
            // Transparent rather than absent: the pill still takes its space, so
            // nothing moves when the selection does, and an invisible decoration
            // records no draw calls.
            (Color::TRANSPARENT, colors.on_surface_variant)
        };
        if self.is_enabled() {
            (pill, ink)
        } else {
            (ColorScheme::dimmed(pill), ColorScheme::dimmed(ink))
        }
    }

    /// One destination, drawn.
    fn destination(&self, theme: &ThemeData, index: usize, press: f32) -> WidgetNode {
        let chosen = self.selected() == Some(index);
        let (pill, ink) = self.colors(theme, chosen);
        let gap = theme.metrics.gap;
        let item = &self.items[index];

        // No label on the icon: the text under it says the same thing, and a
        // screen reader that read both would announce every destination twice.
        // The `Semantics` wrapper below speaks for the whole thing.
        let icon = Icon::new(item.icon.clone()).size(ICON).color(ink);

        let badge = DecoratedBox::new(crate::controls::pressed_fill(
            BoxDecoration::filled(pill).stadium(),
            ink,
            press,
        ))
        .child(Padding::new(EdgeInsets::symmetric(gap * 1.5, gap / 2.0)).child(icon));

        let label = Text::new(item.label.clone()).style(TextStyle {
            color: ink,
            ..theme.text.label
        });

        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .push(badge)
            .push(SizedBox::height(gap / 2.0))
            .push(label)
            .into()
    }

    /// One destination, wrapped in whatever makes it interactive and audible.
    fn page(&self, theme: &ThemeData, index: usize) -> WidgetNode {
        let interactive: WidgetNode = match &self.on_selected {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let control = self.clone();
                let theme = *theme;
                Pressable::new(move |press| control.destination(&theme, index, press))
                    .fade(theme.motion.duration_short)
                    .on_tap(move || handler(index))
                    .into()
            }
            None => self.destination(theme, index, 0.0),
        };

        Semantics::new()
            .role(SemanticRole::Tab)
            .label(self.items[index].label.clone())
            .toggled(self.selected() == Some(index))
            .enabled(self.is_enabled())
            .child(interactive)
            .into()
    }
}

impl Widget for BottomNavigation {
    fn debug_name(&self) -> &'static str {
        "BottomNavigation"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let destinations: Vec<WidgetNode> = (0..self.items.len())
            .map(|index| Flexible::expanded(1).child(self.page(&theme, index)).into())
            .collect();

        // Filled with the surface colour rather than left transparent: this bar
        // sits over the page's own content, and a navigation bar you can read
        // the last line of a list through is not one.
        //
        // **And a hairline above it, because opaque is not the same as
        // visible.** Filling with `surface` makes the bar exactly the colour of
        // the page it sits on, so it is opaque and indistinguishable — which is
        // what it looked like on a screen the moment the demo started using
        // `colors.surface` for its own background instead of a constant that
        // happened to differ. Both platforms separate this bar with a rule
        // rather than a shade: iOS draws a hairline, Android a tonal step this
        // palette has no entry for.
        //
        // A `DecoratedBox` above the row rather than a `Border`, because a
        // `Border` is uniform on all four sides and this wants one edge.
        let divider = DecoratedBox::new(BoxDecoration::filled(theme.colors.outline))
            .child(SizedBox::height(HAIRLINE));

        DecoratedBox::new(BoxDecoration::filled(theme.colors.surface))
            .child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .push(divider)
                    .push(
                        Constrained::new(Constraints::tight_for_height(BAR_HEIGHT))
                            .child(Flex::row().children(destinations)),
                    ),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("destinations", self.items.len().to_string()),
            (
                "selected",
                self.selected()
                    .map_or_else(|| String::from("none"), |index| index.to_string()),
            ),
            ("enabled", self.is_enabled().to_string()),
        ]
    }
}

/// Hand-written because `Handler` is a closure and closures are not `Debug`.
impl fmt::Debug for BottomNavigation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BottomNavigation")
            .field("destinations", &self.items.len())
            .field("selected", &self.selected)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(BottomNavigation);

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn nav() -> BottomNavigation {
        BottomNavigation::new(
            [
                BottomNavItem::new(crate::icons::check(), "Done"),
                BottomNavItem::new(crate::icons::add(), "New"),
                BottomNavItem::new(crate::icons::close(), "Archive"),
            ],
            1,
        )
    }

    #[test]
    fn an_out_of_range_selection_highlights_nothing_rather_than_panicking() {
        assert_eq!(nav().selected(), Some(1));
        let short = BottomNavigation::new([BottomNavItem::new(crate::icons::check(), "Done")], 7);
        assert_eq!(short.selected(), None);
        assert_eq!(BottomNavigation::new([], 0).selected(), None);
    }

    #[test]
    fn a_bar_with_no_handler_is_disabled() {
        assert!(!nav().is_enabled());
        assert!(nav().on_selected(Rc::new(|_| {})).is_enabled());
    }

    #[test]
    fn a_destination_reports_the_index_it_sits_at() {
        let chosen = Rc::new(Cell::new(usize::MAX));
        let recorded = Rc::clone(&chosen);
        let nav = nav().on_selected(Rc::new(move |index| recorded.set(index)));

        if let Some(handler) = &nav.on_selected {
            handler(2);
        }
        assert_eq!(chosen.get(), 2);
    }

    #[test]
    fn every_destination_is_its_own_stop_and_says_tab() {
        let dump = crate::debug_tree(nav());
        for label in ["Done", "New", "Archive"] {
            assert!(dump.contains(label), "{label} is missing: {dump}");
        }
        assert_eq!(
            dump.matches("role: Tab").count(),
            3,
            "a destination is one of a set of pages, which is what a tab is: \
             {dump}"
        );
    }

    #[test]
    fn exactly_one_destination_is_announced_as_selected() {
        let dump = crate::debug_tree(nav());
        assert_eq!(dump.matches("toggled: true").count(), 1, "{dump}");
        assert_eq!(dump.matches("toggled: false").count(), 2, "{dump}");
    }

    #[test]
    fn the_label_is_shown_whether_the_destination_is_chosen_or_not() {
        // Hiding an unselected label would change the bar's height when the
        // selection moved, and leave an icon to be guessed at meanwhile.
        let theme = ThemeData::default();
        let chosen = crate::debug_tree(nav().destination(&theme, 1, 0.0));
        let other = crate::debug_tree(nav().destination(&theme, 0, 0.0));
        assert!(chosen.contains("New"), "{chosen}");
        assert!(other.contains("Done"), "{other}");
        assert_eq!(
            chosen.lines().count(),
            other.lines().count(),
            "the selection may change colours and nothing else:\n{chosen}\n{other}"
        );
    }

    #[test]
    fn the_icon_carries_no_label_of_its_own() {
        // The text under it already says the word, and the `Semantics` wrapper
        // speaks for the whole destination. An icon labelled as well would be
        // announced twice.
        let dump = crate::debug_tree(nav());
        assert_eq!(
            dump.matches("label: Done").count(),
            1,
            "the `Semantics` wrapper is the only thing that names it; an `Icon` \
             with a label of its own would make it two stops: {dump}"
        );
    }

    #[test]
    fn the_length_and_emptiness_agree() {
        assert_eq!(nav().len(), 3);
        assert!(!nav().is_empty());
        let empty = BottomNavigation::new([], 0);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn the_debug_properties_carry_what_a_tree_dump_needs() {
        let props = nav().debug_properties();
        assert!(props.contains(&("destinations", "3".to_owned())));
        assert!(props.contains(&("selected", "1".to_owned())));
    }
}
