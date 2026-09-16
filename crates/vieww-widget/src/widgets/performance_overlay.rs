use std::time::Duration;

use vieww_foundation::Key;

use crate::{widget_node_from, Widget, WidgetKind};

/// A bar graph of what recent frames cost, for laying over a running screen.
///
/// One bar per frame, oldest at the left, each measured against `budget`. A
/// frame that missed its budget is drawn in a warning colour, and a line marks
/// the budget itself.
///
/// ```
/// use std::time::Duration;
/// use vieww_widget::PerformanceOverlay;
///
/// let budget = Duration::from_micros(16_667); // 60Hz
/// let overlay = PerformanceOverlay::new(vec![Duration::from_millis(8)], budget);
/// ```
///
/// A render leaf that draws the whole graph in one pass. The obvious
/// alternative — a [`Flex`](crate::Flex) of one [`ColoredBox`](crate::ColoredBox)
/// per bar — would put a couple of hundred widgets in the tree and rebuild all
/// of them every frame, which is a measuring instrument that costs more than
/// the thing it measures.
///
/// # Where the samples come from
///
/// The platform layer's `FrameLog` is what actually times a presented frame;
/// this widget takes the durations and nothing else, because `vieww-widget`
/// sits well below the crate that owns the window. `FrameLog::work_samples`
/// produces exactly this.
///
/// # It takes no taps and says nothing to a screen reader
///
/// Both deliberate, and both in `RenderPerformanceOverlay`. A diagnostic that
/// swallowed input would change the behaviour of the application it exists to
/// measure.
#[derive(Debug, Clone)]
pub struct PerformanceOverlay {
    samples: Vec<Duration>,
    budget: Duration,
    height: Option<f32>,
    key: Option<Key>,
}

impl PerformanceOverlay {
    /// An overlay of `samples`, oldest first, measured against `budget`.
    #[must_use]
    pub fn new(samples: Vec<Duration>, budget: Duration) -> Self {
        Self {
            samples,
            budget,
            height: None,
            key: None,
        }
    }

    /// Override the graph's height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    // ----------------------------------------------------- read by the factory

    #[must_use]
    pub fn samples(&self) -> &[Duration] {
        &self.samples
    }

    #[must_use]
    pub const fn budget(&self) -> Duration {
        self.budget
    }

    #[must_use]
    pub const fn overlay_height(&self) -> Option<f32> {
        self.height
    }
}

impl Widget for PerformanceOverlay {
    fn debug_name(&self) -> &'static str {
        "PerformanceOverlay"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("samples", self.samples.len().to_string()),
            ("budget", format!("{:?}", self.budget)),
        ]
    }
}

widget_node_from!(PerformanceOverlay);

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> Duration {
        Duration::from_micros(16_667)
    }

    #[test]
    fn an_overlay_is_a_leaf_so_its_bars_cost_no_widgets() {
        let overlay = PerformanceOverlay::new(vec![Duration::from_millis(8); 200], budget());
        assert!(matches!(overlay.kind(), WidgetKind::RenderLeaf));
        assert_eq!(overlay.samples().len(), 200);
    }

    #[test]
    fn the_height_is_the_render_objects_to_choose_unless_it_is_given_one() {
        assert_eq!(
            PerformanceOverlay::new(Vec::new(), budget()).overlay_height(),
            None
        );
        assert_eq!(
            PerformanceOverlay::new(Vec::new(), budget())
                .height(96.0)
                .overlay_height(),
            Some(96.0)
        );
    }
}
