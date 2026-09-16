use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, Color, EdgeInsets, Key, TextStyle};

use crate::{
    widget_node_from, BuildContext, CrossAxisAlignment, DecoratedBox, Flex, Flexible, Handler,
    MainAxisAlignment, Padding, Pressable, SemanticRole, Semantics, Text, ThemeData, Widget,
    WidgetKind, WidgetNode,
};

/// How much smaller the selected fill's corners are than the container's.
///
/// The fill sits *inside* the outline, so matching radii would leave the
/// outline's curve visible through the corner of the fill. Two pixels is the
/// outline's own width, which is exactly the offset between the two curves.
const INSET: f32 = 2.0;

/// A row of options in one container, of which exactly one is chosen.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::SegmentedControl;
/// use std::rc::Rc;
///
/// # let period = 1;
/// # let choose: Rc<dyn Fn(usize)> = Rc::new(|_| {});
/// let range = SegmentedControl::new(["Day", "Week", "Month"], period)
///     .on_selected(choose);
/// ```
///
/// # It is a radio group that looks like buttons
///
/// Each segment is announced as a radio, not a button, and that is the whole
/// reason `SemanticRole::Radio` exists rather than reusing `Button`. A screen
/// reader user hearing "Week, button" learns nothing about whether pressing it
/// releases "Day"; hearing "Week, radio button, selected, 2 of 3" learns
/// everything. The visual — a joined row rather than separate rings — is a
/// density choice and says nothing about the semantics.
///
/// # Segments share the width equally
///
/// Every segment gets one flex share, so a three-way control is three thirds
/// whatever the labels say. The alternative — sizing each to its text — makes
/// the control's shape depend on its content and its translations, and a row
/// whose divisions move when the language changes is a row nobody can aim at
/// from muscle memory.
///
/// Long labels are the cost. A segment whose text does not fit clips rather
/// than wrapping, because a segmented control that grows to two lines has
/// stopped being one.
#[derive(Clone)]
pub struct SegmentedControl {
    labels: Vec<String>,
    selected: usize,
    on_selected: Option<Handler<usize>>,
    key: Option<Key>,
}

impl SegmentedControl {
    /// A control over `labels`, with `selected` chosen.
    ///
    /// An out-of-range `selected` selects nothing rather than panicking — it is
    /// the shape a freshly loaded screen has before its state arrives, and a
    /// control that panicked on it would be unusable during exactly the moment
    /// `AsyncBuilder` exists for.
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

    /// How many options there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// `true` when there is nothing to choose between.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// The index currently chosen, if it addresses a segment that exists.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        (self.selected < self.labels.len()).then_some(self.selected)
    }

    /// One segment's fill and text colour.
    fn colors(&self, theme: &ThemeData, chosen: bool) -> (Option<Color>, Color) {
        let colors = theme.colors;
        // **iOS inverts the figure and the ground.** Its segmented control is a
        // grey trough with a *raised white* pill on the chosen segment and dark
        // text throughout; Android fills the chosen one with the accent and
        // puts light text on it. Same control, opposite emphasis — and the iOS
        // one is instantly recognisable, which is the point.
        let (fill, ink) = if chosen {
            if theme.platform.is_apple() {
                (Some(colors.surface), colors.on_surface)
            } else {
                (Some(colors.primary), colors.on_primary)
            }
        } else {
            // No fill at all rather than a background matching the container:
            // one fewer command per unselected segment, on a control where all
            // but one of them are unselected.
            (None, colors.on_surface_variant)
        };
        if self.is_enabled() {
            (fill, ink)
        } else {
            (
                fill.map(crate::ColorScheme::dimmed),
                crate::ColorScheme::dimmed(ink),
            )
        }
    }

    /// One segment, drawn.
    fn segment(&self, theme: &ThemeData, index: usize, press: f32) -> WidgetNode {
        let chosen = self.selected() == Some(index);
        let (fill, ink) = self.colors(theme, chosen);
        // Apple's pill is nearly a stadium inside its trough; Android's fill
        // follows the container's corner.
        let radius = if theme.platform.is_apple() {
            7.0
        } else {
            (theme.metrics.corner - INSET).max(0.0)
        };
        let gap = theme.metrics.gap;

        let decoration = crate::controls::pressed_fill(
            match fill {
                Some(color) => BoxDecoration::filled(color).radius(radius),
                None => BoxDecoration::default().radius(radius),
            },
            ink,
            press,
        );

        let label = Text::new(self.labels[index].clone()).style(TextStyle {
            color: ink,
            ..theme.text.label
        });

        DecoratedBox::new(decoration)
            .child(
                Padding::new(EdgeInsets::symmetric(gap, gap * 1.5)).child(
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::Center)
                        .cross_axis_alignment(CrossAxisAlignment::Center)
                        .push(label),
                ),
            )
            .into()
    }

    /// One segment, wrapped in whatever makes it interactive and audible.
    fn option(&self, theme: &ThemeData, index: usize) -> WidgetNode {
        let interactive: WidgetNode = match &self.on_selected {
            Some(handler) => {
                let handler = Rc::clone(handler);
                let control = self.clone();
                let theme = *theme;
                Pressable::new(move |press| control.segment(&theme, index, press))
                    .fade(theme.motion.duration_short)
                    .on_tap(move || handler(index))
                    .into()
            }
            None => self.segment(theme, index, 0.0),
        };

        // The width is already shared equally by the surrounding `Flexible`;
        // only the height needs a floor, so `min_height` rather than
        // `touch_target` — the latter would shrink-wrap each segment back to
        // its label's width and undo the equal split.
        let interactive = crate::controls::min_height(theme.metrics.touch_target, interactive);

        Semantics::new()
            .role(SemanticRole::Radio)
            .label(self.labels[index].clone())
            .toggled(self.selected() == Some(index))
            .enabled(self.is_enabled())
            .child(interactive)
            .into()
    }
}

impl Widget for SegmentedControl {
    fn debug_name(&self) -> &'static str {
        "SegmentedControl"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        // Every segment one share, so the divisions do not move when a
        // translation makes one label longer.
        let segments: Vec<WidgetNode> = (0..self.labels.len())
            .map(|index| {
                Flexible::expanded(1)
                    .child(self.option(&theme, index))
                    .into()
            })
            .collect();

        // Not wrapped in a `Semantics` of its own. The default merges children
        // into one node, which would collapse the whole control into a single
        // stop announcing one joined label — and every segment here is meant to
        // be reachable separately.
        // The trough on Apple, an outline everywhere else — see `colors`.
        let container = if theme.platform.is_apple() {
            BoxDecoration::filled(theme.colors.surface_variant).radius(9.0)
        } else {
            BoxDecoration::outlined(Border::thin(theme.colors.outline)).radius(theme.metrics.corner)
        };

        DecoratedBox::new(container)
            .child(Padding::new(EdgeInsets::all(INSET)).child(Flex::row().children(segments)))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("segments", self.labels.len().to_string()),
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
impl fmt::Debug for SegmentedControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentedControl")
            .field("labels", &self.labels)
            .field("selected", &self.selected)
            .field("enabled", &self.is_enabled())
            .finish_non_exhaustive()
    }
}

widget_node_from!(SegmentedControl);

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn control() -> SegmentedControl {
        SegmentedControl::new(["Day", "Week", "Month"], 1)
    }

    #[test]
    fn an_out_of_range_selection_chooses_nothing_rather_than_panicking() {
        assert_eq!(control().selected(), Some(1));
        assert_eq!(
            SegmentedControl::new(["Day", "Week"], 7).selected(),
            None,
            "a screen whose state has not arrived yet must still render"
        );
        assert_eq!(
            SegmentedControl::new(Vec::<String>::new(), 0).selected(),
            None
        );
    }

    #[test]
    fn a_control_with_no_handler_is_disabled() {
        assert!(!control().is_enabled());
        assert!(control().on_selected(Rc::new(|_| {})).is_enabled());
    }

    #[test]
    fn a_segment_reports_the_index_it_sits_at() {
        let chosen = Rc::new(Cell::new(usize::MAX));
        let recorded = Rc::clone(&chosen);
        let control = control().on_selected(Rc::new(move |index| recorded.set(index)));

        // What a tap on the third segment does.
        if let Some(handler) = &control.on_selected {
            handler(2);
        }
        assert_eq!(chosen.get(), 2);
    }

    #[test]
    fn every_segment_is_its_own_stop_and_says_radio() {
        let dump = crate::debug_tree(control());
        for label in ["Day", "Week", "Month"] {
            assert!(dump.contains(label), "{label} is missing: {dump}");
        }
        assert!(
            dump.matches("Radio").count() >= 3,
            "each segment must be separately reachable and announced as one of \
             a set, not as a button: {dump}"
        );
    }

    #[test]
    fn the_length_and_emptiness_agree() {
        assert_eq!(control().len(), 3);
        assert!(!control().is_empty());
        let empty = SegmentedControl::new(Vec::<String>::new(), 0);
        assert_eq!(empty.len(), 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn the_debug_properties_carry_what_a_tree_dump_needs() {
        let props = control().debug_properties();
        assert!(props.contains(&("segments", "3".to_owned())));
        assert!(props.contains(&("selected", "1".to_owned())));
    }
}
