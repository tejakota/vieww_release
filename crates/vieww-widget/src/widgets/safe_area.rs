use vieww_foundation::{EdgeInsets, Key, ViewMetrics};

use crate::{widget_node_from, BuildContext, Padding, Widget, WidgetKind, WidgetNode};

/// Insets its child clear of notches, status bars and home indicators.
///
/// Reads the [`ViewMetrics`] the platform bridge published and turns them into a
/// [`Padding`]. On a desktop window every inset is zero and this is free; on a
/// phone it is the difference between a title under the notch and a title
/// beside it.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::SafeArea;
///
/// // Only the top and bottom, which is the usual case: the sides of a phone
/// // are safe in portrait and the horizontal inset is zero anyway.
/// let screen = SafeArea::new().horizontal(false).child(Text::new("hello"));
/// ```
///
/// # Which sides, and why that is a choice
///
/// A full-bleed background wants no insets at all; a list wants insets at the
/// top but must run to the bottom edge under a translucent home indicator; a
/// dialog wants all four. So every side can be turned off, and turning one off
/// means "I am handling this edge myself" rather than "there is nothing there".
///
/// # The keyboard is not a notch
///
/// [`ViewMetrics::padding`] is used rather than
/// [`safe_area`](ViewMetrics::safe_area), so when a soft keyboard is up the
/// bottom inset yields to it instead of stacking with it. What this deliberately
/// does *not* do is inset content above the keyboard: a scrollable is supposed
/// to scroll out from under one, and a widget that shrank the box would make the
/// field being typed into scroll away.
#[derive(Debug, Clone)]
pub struct SafeArea {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
    /// Applied on top of the platform's, for a screen that wants a margin as
    /// well as safety.
    minimum: EdgeInsets,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Default for SafeArea {
    fn default() -> Self {
        Self::new()
    }
}

impl SafeArea {
    /// Inset on all four sides.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            left: true,
            top: true,
            right: true,
            bottom: true,
            minimum: EdgeInsets::ZERO,
            child: None,
            key: None,
        }
    }

    /// Whether to inset the left and right edges.
    #[must_use]
    pub const fn horizontal(mut self, inset: bool) -> Self {
        self.left = inset;
        self.right = inset;
        self
    }

    /// Whether to inset the top and bottom edges.
    #[must_use]
    pub const fn vertical(mut self, inset: bool) -> Self {
        self.top = inset;
        self.bottom = inset;
        self
    }

    #[must_use]
    pub const fn top(mut self, inset: bool) -> Self {
        self.top = inset;
        self
    }

    #[must_use]
    pub const fn bottom(mut self, inset: bool) -> Self {
        self.bottom = inset;
        self
    }

    /// A floor under the platform's insets.
    ///
    /// Each side ends up at whichever is larger, so a screen asking for 16
    /// points of margin gets 16 on a laptop and 47 under a notch, rather than
    /// 63.
    #[must_use]
    pub const fn minimum(mut self, minimum: EdgeInsets) -> Self {
        self.minimum = minimum;
        self
    }

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

    /// The insets this would apply given some metrics.
    ///
    /// Public so the behaviour can be tested without mounting a tree, and so an
    /// application doing its own layout can ask the same question.
    #[must_use]
    pub fn resolve(&self, metrics: &ViewMetrics) -> EdgeInsets {
        let padding = metrics.padding();
        EdgeInsets {
            left: self.side(self.left, padding.left, self.minimum.left),
            top: self.side(self.top, padding.top, self.minimum.top),
            right: self.side(self.right, padding.right, self.minimum.right),
            bottom: self.side(self.bottom, padding.bottom, self.minimum.bottom),
        }
    }

    fn side(&self, enabled: bool, platform: f32, minimum: f32) -> f32 {
        if enabled {
            platform.max(minimum)
        } else {
            minimum
        }
    }
}

impl Widget for SafeArea {
    fn debug_name(&self) -> &'static str {
        "SafeArea"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        // No metrics published means no platform under us — a test, or a tree
        // being dumped. Zero insets is the right answer there and not a
        // failure worth panicking over.
        let metrics = ctx.inherit_or(ViewMetrics::default());
        let mut padding = Padding::new(self.resolve(&metrics));
        if let Some(child) = &self.child {
            padding = padding.child(child.clone());
        }
        padding.into()
    }
}

widget_node_from!(SafeArea);

#[cfg(test)]
mod tests {
    use vieww_foundation::Size;

    use super::*;

    fn phone() -> ViewMetrics {
        ViewMetrics {
            size: Size::new(390.0, 844.0),
            device_pixel_ratio: 3.0,
            safe_area: EdgeInsets::only(0.0, 47.0, 0.0, 34.0),
            view_insets: EdgeInsets::ZERO,
        }
    }

    #[test]
    fn a_desktop_window_costs_nothing() {
        let metrics = ViewMetrics::plain(Size::new(800.0, 600.0));
        assert_eq!(SafeArea::new().resolve(&metrics), EdgeInsets::ZERO);
    }

    #[test]
    fn a_notch_and_a_home_indicator_become_padding() {
        assert_eq!(
            SafeArea::new().resolve(&phone()),
            EdgeInsets::only(0.0, 47.0, 0.0, 34.0)
        );
    }

    #[test]
    fn a_disabled_edge_is_not_inset() {
        let insets = SafeArea::new().bottom(false).resolve(&phone());
        assert_eq!(insets.top, 47.0);
        assert_eq!(
            insets.bottom, 0.0,
            "a list is expected to run under the home indicator"
        );
    }

    #[test]
    fn a_minimum_is_a_floor_and_not_an_addition() {
        let insets = SafeArea::new()
            .minimum(EdgeInsets::all(16.0))
            .resolve(&phone());

        assert_eq!(
            insets.top, 47.0,
            "the notch is bigger than the margin asked for"
        );
        assert_eq!(
            insets.left, 16.0,
            "and the margin still applies where nothing is"
        );
        assert_eq!(
            insets.bottom, 34.0,
            "47 + 16 would be a gap nobody asked for"
        );
    }

    #[test]
    fn a_minimum_still_applies_on_an_edge_that_is_turned_off() {
        let insets = SafeArea::new()
            .top(false)
            .minimum(EdgeInsets::all(16.0))
            .resolve(&phone());
        assert_eq!(
            insets.top, 16.0,
            "turning the edge off declines the platform's inset, not the margin"
        );
    }

    #[test]
    fn the_keyboard_releases_the_bottom_inset() {
        let mut metrics = phone();
        metrics.view_insets = EdgeInsets::only(0.0, 0.0, 0.0, 300.0);
        assert_eq!(SafeArea::new().resolve(&metrics).bottom, 0.0);
    }
}
