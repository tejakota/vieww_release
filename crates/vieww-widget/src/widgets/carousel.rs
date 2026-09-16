//! A snapping carousel using ElementState for gesture state.
//!
//! # Why ElementState and not Signal
//!
//! `Signal` lives in `vieww-element`, which depends on `vieww-widget` —
//! not the other way around. A widget cannot hold a Signal without
//! creating a dependency cycle.
//!
//! The vieww pattern is: the widget is an immutable description; state
//! lives in `ElementState`; the widget's `build` reads state through
//! `BuildContext` and produces children; callbacks update state, which
//! marks the element pending and triggers a rebuild.
//!
//! This carousel follows that pattern exactly. The drag position, the
//! current page index, and the animation state all live in
//! [`CarouselState`], and the widget reads them in `build`.

use std::any::Any;

use vieww_foundation::{DragDetails, Size};

use crate::prelude::*;
use crate::{widget_node_from, ElementState};

/// State for the carousel: drag position, page index, animation.
#[derive(Debug)]
pub struct CarouselState {
    /// The horizontal scroll offset, in logical pixels.
    ///
    /// 0 means the first item is fully visible. Positive values scroll
    /// right (later items come into view).
    pub offset: f32,
    /// The index of the item closest to the snap position.
    pub current_page: usize,
    /// `true` while a drag is in progress.
    pub dragging: bool,
    /// The offset when the drag started, for delta computation.
    drag_start_offset: f32,
    /// Velocity estimate, for the fling on release.
    velocity: f32,
    /// `true` when the state has changed and the element should rebuild.
    pending: bool,
}

impl Default for CarouselState {
    fn default() -> Self {
        Self {
            offset: 0.0,
            current_page: 0,
            dragging: false,
            drag_start_offset: 0.0,
            velocity: 0.0,
            pending: false,
        }
    }
}

impl ElementState for CarouselState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Where the carousel is, so a live-reloaded screen comes back to the card
    /// the person was looking at rather than to the first one.
    ///
    /// The offset and the page, and nothing about the drag: a snapshot is taken
    /// while a screen is being replaced, and a drag that was in flight when
    /// that happened has no finger behind it any more.
    fn snapshot(&self) -> Option<String> {
        Some(format!("{} {}", self.offset, self.current_page))
    }

    fn restore(&mut self, saved: &str) -> bool {
        let mut parts = saved.split(' ');
        let (Some(offset), Some(page)) = (parts.next(), parts.next()) else {
            return false;
        };
        let (Ok(offset), Ok(page)) = (offset.parse::<f32>(), page.parse::<usize>()) else {
            return false;
        };
        if !offset.is_finite() {
            return false;
        }
        self.offset = offset;
        self.current_page = page;
        self.pending = true;
        true
    }
}

/// A horizontally scrolling, snapping carousel.
///
/// Drag to scroll; release to snap to the nearest item. Fast flicks
/// skip items.
///
/// # Examples
///
/// ```ignore
/// Carousel::new(240.0, 16.0)
///     .children(vec![
///         Card::new("One"),
///         Card::new("Two"),
///         Card::new("Three"),
///     ])
/// ```
#[derive(Debug)]
pub struct Carousel {
    /// The width of each item.
    item_width: f32,
    /// The gap between items.
    spacing: f32,
    /// The carousel's items.
    children: Vec<WidgetNode>,
    /// The height of the carousel viewport.
    viewport_height: f32,
}

impl Carousel {
    /// A carousel with items of `item_width` separated by `spacing`.
    #[must_use]
    pub fn new(item_width: f32, spacing: f32) -> Self {
        Self {
            item_width,
            spacing,
            children: Vec::new(),
            viewport_height: 320.0,
        }
    }

    /// Set the carousel items.
    #[must_use]
    pub fn children(mut self, children: Vec<WidgetNode>) -> Self {
        self.children = children;
        self
    }

    /// Set the viewport height.
    #[must_use]
    pub const fn viewport_height(mut self, height: f32) -> Self {
        self.viewport_height = height;
        self
    }

    /// The step between snap points.
    fn step(&self) -> f32 {
        self.item_width + self.spacing
    }

    /// The furthest the strip can scroll: the last item's snap point.
    ///
    /// Taken from patch 6's revision, which added the idea of bounding the
    /// snap range.
    #[must_use]
    pub fn max_offset(&self) -> f32 {
        let count = self.children.len();
        if count <= 1 {
            0.0
        } else {
            (count - 1) as f32 * self.step()
        }
    }

    /// The snap point nearest to `offset`, bounded by the item count.
    ///
    /// Clamping is patch 6's improvement; its own version clamped
    /// unconditionally to `max_offset()`, which is `0.0` for a carousel with
    /// no children — so on an empty carousel every snap collapsed to zero and
    /// its own `carousel_finds_nearest_snap` test failed. Clamp only when
    /// there is a range to clamp to.
    #[must_use]
    pub fn nearest_snap(&self, offset: f32) -> f32 {
        let snapped = snap_to(offset, self.step());
        let max = self.max_offset();
        if max > 0.0 {
            snapped.clamp(0.0, max)
        } else {
            snapped
        }
    }

    /// The page index for a given offset, clamped to the child count.
    #[must_use]
    pub fn page_for_offset(&self, offset: f32) -> usize {
        page_at(offset, self.step(), self.children.len())
    }
}

impl Widget for Carousel {
    fn debug_name(&self) -> &'static str {
        "Carousel"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(CarouselState::default()))
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let offset = ctx.state::<CarouselState, _>(|s| s.offset).unwrap_or(0.0);
        let current_page = ctx
            .state::<CarouselState, _>(|s| s.current_page)
            .unwrap_or(0);

        let step = self.step();

        // Position each child at its snap offset minus the scroll offset.
        let positioned: Vec<WidgetNode> = self
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let base = index as f32 * step;
                Positioned::new()
                    .left(base - offset)
                    .top(0.0)
                    .child(child.clone())
                    .into()
            })
            .collect();

        // The drag handler: a GestureDetector that updates CarouselState.
        let item_width = self.item_width;
        let children_count = self.children.len();
        let handle = ctx.state_handle();
        // One clone per closure: each `move` closure takes ownership, and
        // there are three of them.
        let handle_start = handle.clone();
        let handle_update = handle.clone();
        let handle_end = handle;

        // Every drag handler takes `DragDetails` — there is no zero-argument
        // or bare-delta form in this framework.
        let gesture = GestureDetector::new()
            .on_drag_start(move |_details: DragDetails| {
                if let Some(state) = &handle_start {
                    if let Some(carousel) = state
                        .borrow_mut()
                        .as_any_mut()
                        .downcast_mut::<CarouselState>()
                    {
                        carousel.dragging = true;
                        carousel.drag_start_offset = carousel.offset;
                        carousel.velocity = 0.0;
                        carousel.pending = true;
                    }
                }
            })
            .on_drag_update(move |details: DragDetails| {
                if let Some(state) = &handle_update {
                    if let Some(carousel) = state
                        .borrow_mut()
                        .as_any_mut()
                        .downcast_mut::<CarouselState>()
                    {
                        if carousel.dragging {
                            carousel.offset = (carousel.drag_start_offset - details.delta.dx)
                                .max(0.0)
                                .min((children_count.saturating_sub(1)) as f32 * step);
                            // `DragDetails` carries a real velocity from the tracker; deriving
                            // one by dividing this frame's delta by a nominal 16ms was an
                            // estimate of a number already measured properly.
                            carousel.velocity = -details.velocity.dx;
                            carousel.pending = true;
                        }
                    }
                }
            })
            .on_drag_end(move |_details: DragDetails| {
                if let Some(state) = &handle_end {
                    if let Some(carousel) = state
                        .borrow_mut()
                        .as_any_mut()
                        .downcast_mut::<CarouselState>()
                    {
                        carousel.dragging = false;

                        // Project the velocity forward to decide the snap target.
                        let projected = carousel.offset + carousel.velocity * 0.25;
                        let max_offset = (children_count.saturating_sub(1)) as f32 * step;
                        let target = projected.clamp(0.0, max_offset);

                        // Through the shared helpers rather than a second copy
                        // of the arithmetic. The copy that used to live here
                        // read the page back with `(snapped / step) as usize`,
                        // and `as usize` truncates: `snapped` is a multiple of
                        // `step` computed in f32, so the division can land at
                        // 1.9999994 and report page 1 while the strip sits on
                        // page 2. `page_at` rounds, and clamps to the child
                        // count as well.
                        let snapped = snap_to(target, step);
                        carousel.offset = snapped;
                        carousel.current_page = page_at(snapped, step, children_count);
                        carousel.pending = true;
                    }
                }
            });

        // **The dots take their colours from the scheme, not from two hex
        // literals.**
        //
        // They were `rgb(58, 122, 246)` and `rgb(203, 213, 225)` — a specific
        // blue and a specific grey, written into a framework widget. An
        // application that themed everything else still got those two, so a
        // carousel was the one control on the screen that had not heard about
        // the brand; in a dark theme the inactive dot was a light grey pip on a
        // dark ground, which is the wrong way round.
        //
        // `primary` is the accent role — the same one a filled button and a
        // checked switch use — and `outline` is the role for a mark that is
        // present but not active. Both are exactly what `ColorScheme`
        // documents them as, and both follow whatever the application set.
        let colors = ThemeData::of(ctx).colors;
        let indicators: Vec<WidgetNode> = (0..self.children.len())
            .map(|i| {
                let is_current = i == current_page;
                Container::new()
                    .color(if is_current {
                        colors.primary
                    } else {
                        colors.outline
                    })
                    .radius(3.0)
                    .child(SizedBox::from_size(Size::new(6.0, 6.0)))
                    .into()
            })
            .collect();

        // The carousel: clipped viewport containing the positioned strip,
        // with the gesture detector wrapping it, and page indicators below.
        Flex::column()
            .spacing(12.0)
            .children(children![
                Clip::rounded(12.0).child(gesture.child(Stack::new().children(positioned).push(
                    SizedBox::from_size(Size::new(item_width, self.viewport_height,))
                ),),),
                Flex::row().spacing(6.0).children(indicators),
            ])
            .into()
    }
}

/// The snap point nearest to `offset`, given the distance between stops.
fn snap_to(offset: f32, step: f32) -> f32 {
    if step <= 0.0 {
        return 0.0;
    }
    (offset / step).round() * step
}

/// The page index at `offset`, clamped to `count` pages.
fn page_at(offset: f32, step: f32, count: usize) -> usize {
    if step <= 0.0 {
        return 0;
    }
    let page = (offset / step).round().max(0.0) as usize;
    page.min(count.saturating_sub(1))
}

widget_node_from!(Carousel);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carousel_state_defaults_to_zero() {
        let state = CarouselState::default();
        assert_eq!(state.offset, 0.0);
        assert_eq!(state.current_page, 0);
        assert!(!state.dragging);
    }

    #[test]
    fn carousel_computes_step() {
        let carousel = Carousel::new(240.0, 16.0);
        assert_eq!(carousel.step(), 256.0);
    }

    #[test]
    fn carousel_finds_nearest_snap() {
        let carousel = Carousel::new(240.0, 16.0);
        // Step is 256. Offset 300 is nearest to 256.
        assert_eq!(carousel.nearest_snap(300.0), 256.0);
        // Offset 200 is nearest to 256 (rounds up).
        assert_eq!(carousel.nearest_snap(200.0), 256.0);
        // Offset 100 is nearest to 0.
        assert_eq!(carousel.nearest_snap(100.0), 0.0);
    }

    #[test]
    fn nearest_snap_is_bounded_by_the_item_count() {
        // Three items, step 256 -> the last snap point is 512.
        let carousel = Carousel::new(240.0, 16.0).children(vec![
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
            SizedBox::square(10.0).into(),
        ]);
        assert_eq!(carousel.max_offset(), 512.0);
        assert_eq!(
            carousel.nearest_snap(10_000.0),
            512.0,
            "clamped to the last item"
        );
        assert_eq!(carousel.nearest_snap(-500.0), 0.0, "clamped to the first");
    }

    #[test]
    fn carousel_computes_page_index() {
        let carousel = Carousel::new(240.0, 16.0).children(vec![
            SizedBox::square(100.0).into(),
            SizedBox::square(100.0).into(),
            SizedBox::square(100.0).into(),
        ]);

        assert_eq!(carousel.page_for_offset(0.0), 0);
        assert_eq!(carousel.page_for_offset(256.0), 1);
        assert_eq!(carousel.page_for_offset(512.0), 2);
        // Out of range clamps.
        assert_eq!(carousel.page_for_offset(10000.0), 2);
    }

    #[test]
    fn take_pending_resets_the_flag() {
        let mut state = CarouselState {
            pending: true,
            ..Default::default()
        };
        assert!(state.take_pending());
        assert!(!state.take_pending());
    }
}
