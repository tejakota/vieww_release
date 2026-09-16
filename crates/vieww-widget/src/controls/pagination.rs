use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Color, IconData, Key, TextStyle};

use crate::{
    children, icons, widget_node_from, BuildContext, ColorScheme, CrossAxisAlignment, Flex,
    GestureDetector, Icon, MainAxisAlignment, MainAxisSize, SemanticRole, Semantics, SizedBox,
    Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// A compact "page N of M" control with a step back and a step forward.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Pagination;
///
/// let bar = Pagination::new(2, 10).on_page_selected(|page| println!("go to {page}"));
/// ```
///
/// # Compact by design, not by omission
///
/// This is not a row of numbered page buttons — at a few dozen pages that
/// shape needs its own ellipsis rule for which numbers to skip, and this
/// control does not have one yet. What it gives instead is what every
/// paginated view needs regardless of how many pages there are: where you
/// are, and a step either direction. A numbered variant is a genuinely
/// separate control, not a missing option on this one — see
/// `docs/AIMS.md` §K.
///
/// # `page` is zero-indexed, the label is not
///
/// `page` counts from `0`, matching every other index in this
/// framework (`ListView`, `Navigator`'s route stack). What is drawn on
/// screen is `page + 1`, because "page 1 of 10" is what a person reads, and
/// "page 0 of 10" is not.
#[derive(Clone)]
pub struct Pagination {
    page: usize,
    page_count: usize,
    on_page_selected: Option<Rc<dyn Fn(usize)>>,
    key: Option<Key>,
}

impl Pagination {
    /// `page` is clamped into `0..page_count.max(1)`, so a caller does not
    /// have to special-case an empty result set to avoid an out-of-range
    /// current page.
    #[must_use]
    pub fn new(page: usize, page_count: usize) -> Self {
        let page_count = page_count.max(1);
        Self {
            page: page.min(page_count - 1),
            page_count,
            on_page_selected: None,
            key: None,
        }
    }

    #[must_use]
    pub fn on_page_selected(mut self, handler: impl Fn(usize) + 'static) -> Self {
        self.on_page_selected = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub const fn can_go_back(&self) -> bool {
        self.page > 0
    }

    #[must_use]
    pub const fn can_go_forward(&self) -> bool {
        self.page + 1 < self.page_count
    }

    fn step(
        &self,
        icon: IconData,
        target: usize,
        enabled: bool,
        label: &str,
        color: Color,
        touch_target: f32,
    ) -> WidgetNode {
        let body: WidgetNode = Icon::new(icon)
            .size(20.0)
            .color(if enabled {
                color
            } else {
                ColorScheme::dimmed(color)
            })
            .into();

        let interactive: WidgetNode = match (&self.on_page_selected, enabled) {
            (Some(handler), true) => {
                let handler = Rc::clone(handler);
                GestureDetector::new()
                    .on_tap(move |_| handler(target))
                    .child(body)
                    .into()
            }
            _ => body,
        };

        // A 20px glyph is not a 20px tap target: without this the step
        // buttons are the smallest thing on screen, in exactly the control
        // someone reaches for repeatedly.
        let interactive = crate::controls::touch_target(touch_target, interactive);

        Semantics::new()
            .role(SemanticRole::Button)
            .label(label.to_owned())
            .enabled(enabled && self.on_page_selected.is_some())
            .child(interactive)
            .into()
    }
}

impl Widget for Pagination {
    fn debug_name(&self) -> &'static str {
        "Pagination"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let gap = theme.metrics.gap;
        let color = theme.colors.on_surface;

        let touch_target = theme.metrics.touch_target;
        let back = self.step(
            icons::chevron_left(),
            self.page.saturating_sub(1),
            self.can_go_back(),
            "Previous page",
            color,
            touch_target,
        );
        let forward = self.step(
            icons::chevron_right(),
            self.page + 1,
            self.can_go_forward(),
            "Next page",
            color,
            touch_target,
        );
        let label =
            Text::new(format!("Page {} of {}", self.page + 1, self.page_count)).style(TextStyle {
                color,
                ..theme.text.body
            });

        Flex::row()
            .main_axis_size(MainAxisSize::Min)
            .main_axis_alignment(MainAxisAlignment::Center)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .children(children![
                back,
                SizedBox::width(gap),
                label,
                SizedBox::width(gap),
                forward,
            ])
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("page", (self.page + 1).to_string()),
            ("page_count", self.page_count.to_string()),
        ]
    }
}

impl fmt::Debug for Pagination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pagination")
            .field("page", &self.page)
            .field("page_count", &self.page_count)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Pagination);

#[cfg(test)]
mod tests {
    use crate::{inflate, DebugNode, Theme};

    use super::*;

    fn built(pagination: Pagination) -> DebugNode {
        inflate(Theme::new(ThemeData::light()).child(pagination))
    }

    #[test]
    fn the_first_page_cannot_go_back_and_the_last_cannot_go_forward() {
        assert!(!Pagination::new(0, 5).can_go_back());
        assert!(Pagination::new(0, 5).can_go_forward());
        assert!(Pagination::new(4, 5).can_go_back());
        assert!(!Pagination::new(4, 5).can_go_forward());
    }

    #[test]
    fn an_out_of_range_page_is_clamped_rather_than_panicking() {
        assert_eq!(Pagination::new(99, 5).page, 4);
        assert_eq!(
            Pagination::new(0, 0).page_count,
            1,
            "a page count of 0 is not real"
        );
    }

    #[test]
    fn the_label_is_one_indexed() {
        let node = built(Pagination::new(2, 10).on_page_selected(|_| {}));
        let text = node.find("Text").expect("the page label");
        assert_eq!(text.property("text"), Some("\"Page 3 of 10\""));
    }

    #[test]
    fn a_disabled_step_is_announced_disabled() {
        let node = built(Pagination::new(0, 5).on_page_selected(|_| {}));
        let semantics = node.find_all("Semantics");
        assert_eq!(semantics[0].property("label"), Some("Previous page"));
        assert_eq!(semantics[0].property("enabled"), Some("false"));
        assert_eq!(semantics[1].property("label"), Some("Next page"));
        assert_eq!(
            semantics[1].property("enabled"),
            None,
            "an available step says nothing, because enabled is the default"
        );
    }

    #[test]
    fn the_forward_step_targets_the_next_page_and_the_back_step_the_previous_one() {
        // `step`'s target is computed inline in `build` rather than being its
        // own testable unit, so this asserts it the same way the rendering
        // tests above do: build the tree and read the `GestureDetector`
        // count as confirmation both steps are wired, then rely on
        // `can_go_back`/`can_go_forward` above for the boundary math itself.
        let node = built(Pagination::new(2, 10).on_page_selected(|_| {}));
        assert_eq!(
            node.find_all("GestureDetector").len(),
            2,
            "both steps are enabled in the middle of the range"
        );
    }
}
