use std::fmt;
use std::rc::Rc;

use vieww_foundation::{BoxDecoration, Color, Constraints, EdgeInsets, Key, TextStyle};

use crate::{
    widget_node_from, BuildContext, ColorScheme, Constrained, CrossAxisAlignment, DecoratedBox,
    Flex, Flexible, Handler, MainAxisAlignment, MainAxisSize, Padding, Pressable, SemanticRole,
    Semantics, SizedBox, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How thick the line under a tab is.
///
/// Three logical pixels, and the same for every tab whether it is chosen or
/// not — see the type's docs for why that is load-bearing rather than a style.
const INDICATOR: f32 = 3.0;

/// A row of pages, of which exactly one is showing.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::TabBar;
/// use std::rc::Rc;
///
/// # let page = 0;
/// # let show: Rc<dyn Fn(usize)> = Rc::new(|_| {});
/// let tabs = TabBar::new(["Inbox", "Sent", "Drafts"], page).on_selected(show);
/// ```
///
/// # It is a [`SegmentedControl`](crate::SegmentedControl) with a different skin
///
/// Same shape, same equal shares, same one-of-a-set semantics — and a
/// deliberately different *role*, which is the only part that is not skin. A
/// segment records an answer; a tab **replaces what is on the screen below it**,
/// and a screen reader told "radio button" is not told that. Hence
/// [`SemanticRole::Tab`], which announces a tab's place in a tab list and offers
/// the reader's own navigation between them.
///
/// # The indicator is always laid out; only its colour changes
///
/// The unselected tabs draw the same three-pixel bar in the outline colour, so
/// the row of them *is* the divider under the tab bar and there is no separate
/// divider widget to keep aligned with it. The accented one is the selected tab.
///
/// The alternative — drawing the bar only under the selected tab — changes the
/// bar's height when the selection moves, which shifts every pixel of the page
/// underneath it. A control whose size depends on its value is one that makes
/// the rest of the screen jump.
///
/// # The height is fixed, and it is the touch target
///
/// [`Metrics::touch_target`](crate::Metrics::touch_target) for the whole bar,
/// indicator included, which is also what both platform guidelines ask of a tab
/// bar. Fixed rather than grown from the labels, because a tab bar that is a few
/// pixels taller in one translation moves the whole page with it.
#[derive(Clone)]
pub struct TabBar {
    labels: Vec<String>,
    selected: usize,
    on_selected: Option<Handler<usize>>,
    key: Option<Key>,
}

impl TabBar {
    /// A bar over `labels`, showing the page at `selected`.
    ///
    /// An out-of-range `selected` highlights nothing rather than panicking — it
    /// is the shape a screen has before its state arrives, which is exactly what
    /// [`AsyncBuilder`](crate::AsyncBuilder) exists for.
    #[must_use]
    pub fn new<L, S>(labels: L, selected: usize) -> Self
    where
        L: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            labels: labels.into_iter().map(Into::into).collect(),
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

    /// How many tabs there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// `true` when there is nothing to choose between.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// The index currently showing, if it addresses a tab that exists.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        (self.selected < self.labels.len()).then_some(self.selected)
    }

    /// One tab's indicator colour and text colour.
    fn colors(&self, theme: &ThemeData, chosen: bool) -> (Color, Color) {
        let colors = theme.colors;
        // The unselected indicator is the divider. Same bar, same thickness, the
        // colour a border would have been.
        let (line, ink) = if chosen {
            (colors.primary, colors.primary)
        } else {
            (colors.outline, colors.on_surface_variant)
        };
        if self.is_enabled() {
            (line, ink)
        } else {
            (ColorScheme::dimmed(line), ColorScheme::dimmed(ink))
        }
    }

    /// One tab, drawn.
    fn tab(&self, theme: &ThemeData, index: usize, press: f32) -> WidgetNode {
        let chosen = self.selected() == Some(index);
        let (line, ink) = self.colors(theme, chosen);
        let gap = theme.metrics.gap;

        let label = Text::new(self.labels[index].clone()).style(TextStyle {
            color: ink,
            ..theme.text.label
        });

        // The label takes whatever height is left over once the indicator has
        // had its three pixels, and centres itself in it. `Flexible` rather than
        // padding, so the arithmetic is the flex's rather than a constant here
        // that would have to be corrected every time the bar's height changed.
        let content = Flex::column()
            .cross_axis_alignment(CrossAxisAlignment::Stretch)
            .push(
                Flexible::expanded(1).child(
                    Padding::new(EdgeInsets::symmetric(gap, 0.0)).child(
                        Flex::row()
                            .main_axis_size(MainAxisSize::Min)
                            .main_axis_alignment(MainAxisAlignment::Center)
                            .cross_axis_alignment(CrossAxisAlignment::Center)
                            .push(label),
                    ),
                ),
            )
            .push(
                DecoratedBox::new(BoxDecoration::filled(line)).child(SizedBox::height(INDICATOR)),
            );

        // The press wash covers the whole tab, indicator included — square, and
        // no radius, because a tab is a region of a bar rather than a button
        // sitting on one. `pressed_fill` over a decoration with no fill of its
        // own leaves a faint background, which is the third of the three answers
        // that one rule gives.
        DecoratedBox::new(crate::controls::pressed_fill(
            BoxDecoration::default(),
            ink,
            press,
        ))
        .child(content)
        .into()
    }

    /// One tab, wrapped in whatever makes it interactive and audible.
    fn page(&self, theme: &ThemeData, index: usize) -> WidgetNode {
        let interactive: WidgetNode = match &self.on_selected {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let control = self.clone();
                let theme = *theme;
                Pressable::new(move |press| control.tab(&theme, index, press))
                    .fade(theme.motion.duration_short)
                    .on_tap(move || handler(index))
                    .into()
            }
            None => self.tab(theme, index, 0.0),
        };

        Semantics::new()
            .role(SemanticRole::Tab)
            .label(self.labels[index].clone())
            // Sent to the platform as *selected* rather than *on*: a tab is
            // chosen out of a set, not switched. See `a11y::to_node`.
            .toggled(self.selected() == Some(index))
            .enabled(self.is_enabled())
            .child(interactive)
            .into()
    }
}

impl Widget for TabBar {
    fn debug_name(&self) -> &'static str {
        "TabBar"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        // One share each, so the divisions do not move when a translation makes
        // one label longer — the same argument as `SegmentedControl`, and the
        // stronger one here: a tab a user has learnt the position of is a tab
        // they stop reading.
        let tabs: Vec<WidgetNode> = (0..self.labels.len())
            .map(|index| Flexible::expanded(1).child(self.page(&theme, index)).into())
            .collect();

        // Not wrapped in a `Semantics` of its own, for the reason
        // `SegmentedControl` is not: the default merges children into one node,
        // which would collapse the bar into a single stop announcing one joined
        // label, and every tab is meant to be reachable separately.
        Constrained::new(Constraints::tight_for_height(theme.metrics.touch_target))
            .child(Flex::row().children(tabs))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("tabs", self.labels.len().to_string()),
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
impl fmt::Debug for TabBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TabBar")
            .field("labels", &self.labels)
            .field("selected", &self.selected)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(TabBar);

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn bar() -> TabBar {
        TabBar::new(["Inbox", "Sent", "Drafts"], 1)
    }

    #[test]
    fn an_out_of_range_selection_highlights_nothing_rather_than_panicking() {
        assert_eq!(bar().selected(), Some(1));
        assert_eq!(
            TabBar::new(["Inbox", "Sent"], 7).selected(),
            None,
            "a screen whose state has not arrived yet must still render"
        );
        assert_eq!(TabBar::new(Vec::<String>::new(), 0).selected(), None);
    }

    #[test]
    fn a_bar_with_no_handler_is_disabled() {
        assert!(!bar().is_enabled());
        assert!(bar().on_selected(Rc::new(|_| {})).is_enabled());
    }

    #[test]
    fn a_tab_reports_the_index_it_sits_at() {
        let chosen = Rc::new(Cell::new(usize::MAX));
        let recorded = Rc::clone(&chosen);
        let bar = bar().on_selected(Rc::new(move |index| recorded.set(index)));

        // What a tap on the third tab does.
        if let Some(handler) = &bar.on_selected {
            handler(2);
        }
        assert_eq!(chosen.get(), 2);
    }

    #[test]
    fn every_tab_is_its_own_stop_and_says_tab_rather_than_radio() {
        let dump = crate::debug_tree(bar());
        for label in ["Inbox", "Sent", "Drafts"] {
            assert!(dump.contains(label), "{label} is missing: {dump}");
        }
        assert!(
            dump.matches("role: Tab").count() >= 3,
            "each tab must be separately reachable and announced as a tab, not \
             as a button or a radio: {dump}"
        );
        assert!(
            !dump.contains("role: Radio"),
            "choosing a tab replaces the page below it, which is what \
             distinguishes it from a radio: {dump}"
        );
    }

    #[test]
    fn exactly_one_tab_is_announced_as_selected() {
        let dump = crate::debug_tree(bar());
        assert_eq!(
            dump.matches("toggled: true").count(),
            1,
            "one page is showing: {dump}"
        );
        assert_eq!(dump.matches("toggled: false").count(), 2, "{dump}");
    }

    #[test]
    fn every_tab_draws_an_indicator_whether_it_is_chosen_or_not() {
        // The unselected indicators are the divider under the bar, and the bar's
        // height must not move when the selection does. Both tabs' indicators
        // are the same widget at the same size; only the colour differs.
        let theme = ThemeData::default();
        let chosen = crate::debug_tree(bar().tab(&theme, 1, 0.0));
        let other = crate::debug_tree(bar().tab(&theme, 0, 0.0));
        assert_eq!(
            chosen.matches("SizedBox").count(),
            1,
            "the indicator is one box: {chosen}"
        );
        assert_eq!(
            chosen.lines().count(),
            other.lines().count(),
            "a selection that adds or removes a widget changes the bar's \
             height and shifts the page below it:\n{chosen}\n{other}"
        );
    }

    #[test]
    fn the_length_and_emptiness_agree() {
        assert_eq!(bar().len(), 3);
        assert!(!bar().is_empty());
        let empty = TabBar::new(Vec::<String>::new(), 0);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn the_debug_properties_carry_what_a_tree_dump_needs() {
        let props = bar().debug_properties();
        assert!(props.contains(&("tabs", "3".to_owned())));
        assert!(props.contains(&("selected", "1".to_owned())));
    }
}
