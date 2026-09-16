//! Scroll-driven animation: the scroll position as a progress signal.
//!
//! # The approach
//!
//! `ScrollController` reports its offset. This module wraps that offset
//! in a [`Signal<f32>`] normalizing a scroll range to `[0, 1]`. Widgets
//! read the signal in `build` and rebuild when the scroll crosses
//! their range — the same reactive path as any other signal-driven UI.
//!
//! # Pull-based, matching ScrollController
//!
//! The ScrollController does not push changes. The application calls
//! [`update`](ScrollTimeline::update) from wherever the offset is
//! observed — a gesture handler, a scroll callback. This matches the
//! existing `on_near_end` pattern.
//!
//! # Triggers
//!
//! [`ScrollTrigger`] handles fire-once threshold crossing: an effect
//! fires when the scroll passes a point, and optionally when it comes
//! back. It does not refire while staying past the threshold.

use crate::{Runtime, Signal};

/// Maps a range of scroll offsets to a `[0, 1]` progress signal.
///
/// # Examples
///
/// ```ignore
/// let timeline = ScrollTimeline::new(&runtime, 0.0, 200.0);
/// let progress = timeline.progress();
///
/// // In a widget's build:
/// let p = progress.get();
/// // Use p to compute the header's offset, opacity, etc.
///
/// // From a scroll handler:
/// timeline.update(scroll_offset);
/// ```
#[derive(Debug)]
pub struct ScrollTimeline {
    progress: Signal<f32>,
    start: f32,
    end: f32,
}

impl ScrollTimeline {
    /// Create a timeline over the scroll range `[start, end]`.
    ///
    /// Progress is clamped: below `start` it is 0, above `end` it is 1.
    #[must_use]
    pub fn new(runtime: &Runtime, start: f32, end: f32) -> Self {
        let progress = runtime.signal(0.0f32);
        Self {
            progress,
            start,
            end,
        }
    }

    /// The progress signal. Reading it in a `build` subscribes the element.
    #[must_use]
    pub fn progress(&self) -> Signal<f32> {
        self.progress.clone()
    }

    /// The current progress without subscribing.
    #[must_use]
    pub fn peek(&self) -> f32 {
        self.progress.peek()
    }

    /// Update the progress from a scroll offset.
    ///
    /// Call this wherever the scroll offset is observed.
    pub fn update(&self, offset: f32) {
        let t = if self.end <= self.start {
            0.0
        } else {
            ((offset - self.start) / (self.end - self.start)).clamp(0.0, 1.0)
        };
        self.progress.set(t);
    }
}

/// What happened when a trigger was updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTriggerEvent {
    /// The scroll crossed above the threshold.
    Entered,
    /// The scroll crossed below the threshold.
    Exited,
    /// Nothing changed.
    None,
}

/// An effect that fires when the scroll crosses a threshold.
///
/// Fire-once semantics: fires when *entering* the range (crossing
/// upward past the threshold) and optionally when *leaving*. Does not
/// refire while staying past the threshold.
pub struct ScrollTrigger {
    /// The scroll offset at which the trigger fires.
    pub threshold: f32,
    /// Whether we are currently above the threshold.
    entered: bool,
    /// Callback on entering.
    on_enter: Option<Box<dyn Fn()>>,
    /// Callback on exiting.
    on_exit: Option<Box<dyn Fn()>>,
}

// By hand: the trigger stores `Box<dyn Fn()>` callbacks and no closure is
// `Debug`. Whether a callback is installed is the useful part; its address
// is not.
impl std::fmt::Debug for ScrollTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollTrigger")
            .field("threshold", &self.threshold)
            .field("entered", &self.entered)
            .field("on_enter", &self.on_enter.as_ref().map(|_| "<callback>"))
            .field("on_exit", &self.on_exit.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl ScrollTrigger {
    /// A trigger that fires when the scroll passes `threshold`.
    #[must_use]
    pub fn at(threshold: f32) -> Self {
        Self {
            threshold,
            entered: false,
            on_enter: None,
            on_exit: None,
        }
    }

    /// Set the entering callback.
    #[must_use]
    pub fn on_enter<F: Fn() + 'static>(mut self, f: F) -> Self {
        self.on_enter = Some(Box::new(f));
        self
    }

    /// Set the exiting callback.
    #[must_use]
    pub fn on_exit<F: Fn() + 'static>(mut self, f: F) -> Self {
        self.on_exit = Some(Box::new(f));
        self
    }

    /// Feed a scroll offset. Returns what happened.
    pub fn update(&mut self, offset: f32) -> ScrollTriggerEvent {
        let above = offset >= self.threshold;

        if above && !self.entered {
            self.entered = true;
            if let Some(f) = &self.on_enter {
                f();
            }
            ScrollTriggerEvent::Entered
        } else if !above && self.entered {
            self.entered = false;
            if let Some(f) = &self.on_exit {
                f();
            }
            ScrollTriggerEvent::Exited
        } else {
            ScrollTriggerEvent::None
        }
    }
}

/// Manages a collection of scroll triggers against one scroll offset.
///
/// The application feeds offsets; the manager dispatches to triggers.
///
/// # Examples
///
/// ```ignore
/// let mut manager = ScrollTriggerManager::new();
/// manager.on_scroll_past(100.0, || {
///     header.set_expanded(true);
/// });
/// manager.on_scroll_back(100.0, || {
///     header.set_expanded(false);
/// });
///
/// // From a scroll handler:
/// manager.update(scroll_offset);
/// ```
pub struct ScrollTriggerManager {
    triggers: Vec<ScrollTrigger>,
}

impl ScrollTriggerManager {
    /// An empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            triggers: Vec::new(),
        }
    }

    /// Add a trigger that fires `callback` when the scroll passes
    /// `threshold` (going up).
    pub fn on_scroll_past(&mut self, threshold: f32, callback: impl Fn() + 'static) {
        self.triggers
            .push(ScrollTrigger::at(threshold).on_enter(callback));
    }

    /// Add a trigger that fires `callback` when the scroll falls below
    /// `threshold` (coming back).
    pub fn on_scroll_back(&mut self, threshold: f32, callback: impl Fn() + 'static) {
        self.triggers
            .push(ScrollTrigger::at(threshold).on_exit(callback));
    }

    /// Add a trigger with both enter and exit callbacks.
    pub fn on_cross(
        &mut self,
        threshold: f32,
        on_enter: impl Fn() + 'static,
        on_exit: impl Fn() + 'static,
    ) {
        self.triggers.push(
            ScrollTrigger::at(threshold)
                .on_enter(on_enter)
                .on_exit(on_exit),
        );
    }

    /// Feed a scroll offset, dispatching to any triggers that fire.
    pub fn update(&mut self, offset: f32) {
        for trigger in &mut self.triggers {
            trigger.update(offset);
        }
    }
}

impl Default for ScrollTriggerManager {
    fn default() -> Self {
        Self::new()
    }
}

// By hand: the manager stores boxed closures, which are not `Debug`.
impl std::fmt::Debug for ScrollTriggerManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollTriggerManager")
            .field("triggers", &self.triggers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn timeline_progress_is_zero_below_start() {
        let runtime = Runtime::new();
        let timeline = ScrollTimeline::new(&runtime, 100.0, 200.0);
        timeline.update(50.0);
        assert_eq!(timeline.peek(), 0.0);
    }

    #[test]
    fn timeline_progress_is_one_above_end() {
        let runtime = Runtime::new();
        let timeline = ScrollTimeline::new(&runtime, 100.0, 200.0);
        timeline.update(250.0);
        assert_eq!(timeline.peek(), 1.0);
    }

    #[test]
    fn timeline_progress_interpolates_in_range() {
        let runtime = Runtime::new();
        let timeline = ScrollTimeline::new(&runtime, 100.0, 200.0);
        timeline.update(150.0);
        assert!((timeline.peek() - 0.5).abs() < 0.001);
    }

    #[test]
    fn timeline_progress_updates_the_signal() {
        let runtime = Runtime::new();
        let timeline = ScrollTimeline::new(&runtime, 0.0, 100.0);
        let progress = timeline.progress();

        timeline.update(50.0);
        assert_eq!(progress.peek(), 0.5);
    }

    #[test]
    fn trigger_fires_once_on_entering() {
        let mut trigger = ScrollTrigger::at(100.0);

        assert_eq!(trigger.update(50.0), ScrollTriggerEvent::None);
        assert_eq!(trigger.update(120.0), ScrollTriggerEvent::Entered);
        assert_eq!(
            trigger.update(130.0),
            ScrollTriggerEvent::None,
            "does not refire"
        );
    }

    #[test]
    fn trigger_fires_exit_when_crossing_back() {
        let mut trigger = ScrollTrigger::at(100.0);

        trigger.update(150.0); // enter
        assert_eq!(trigger.update(80.0), ScrollTriggerEvent::Exited);
    }

    #[test]
    fn trigger_callbacks_fire() {
        let fired = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);

        let mut trigger = ScrollTrigger::at(100.0).on_enter(move || {
            *fired_clone.borrow_mut() = true;
        });

        trigger.update(150.0);
        assert!(*fired.borrow(), "callback fired");
    }

    #[test]
    fn manager_dispatches_to_callbacks() {
        let mut manager = ScrollTriggerManager::new();
        let fired = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);

        manager.on_scroll_past(100.0, move || {
            *fired_clone.borrow_mut() = true;
        });

        manager.update(150.0);
        assert!(*fired.borrow(), "callback fired");
    }

    #[test]
    fn manager_both_directions() {
        let mut manager = ScrollTriggerManager::new();
        let expanded = Rc::new(RefCell::new(false));
        let expanded_enter = Rc::clone(&expanded);
        let expanded_exit = Rc::clone(&expanded);

        manager.on_cross(
            100.0,
            move || *expanded_enter.borrow_mut() = true,
            move || *expanded_exit.borrow_mut() = false,
        );

        manager.update(150.0);
        assert!(*expanded.borrow(), "expanded");

        manager.update(50.0);
        assert!(!*expanded.borrow(), "collapsed");
    }
}
