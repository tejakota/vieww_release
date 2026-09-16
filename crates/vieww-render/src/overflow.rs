//! Saying that something did not fit.
//!
//! A render object that runs out of room still **places** its children — it has
//! nowhere else to put them — and nothing clips at the surface, so the only sign
//! is a screen with something missing off an edge. That is the worst kind of
//! layout bug: it looks like the widget was never built, so the search starts in
//! the wrong place entirely.
//!
//! # Why this prints rather than panicking
//!
//! Overflowing is not a contract violation. The tree is well-formed, the
//! arithmetic is right, and there is simply less space than the content asked
//! for — usually because a window is smaller than the design assumed. Killing
//! the application over it would be worse than the clipped pixel, and this
//! follows `FrameDriver::report_unsettled_layout`, which made the same choice
//! for the same reason.
//!
//! # Why it is not behind an environment variable
//!
//! Same argument that one makes: whoever needs this has no reason to have set a
//! variable first, because they do not yet know there is anything to look for.
//!
//! # What deliberately does *not* report
//!
//! **Content inside a scrollable.** A viewport lays its child out with infinite
//! room along the scroll axis, so a list longer than the screen has a main-axis
//! limit of infinity, shrink-wraps to its content, and comes out with nothing
//! beyond its box. Being longer than the window is that widget's entire purpose,
//! and it falls out of `beyond` rather than needing a special case.

use std::cell::RefCell;

use vieww_foundation::Axis;

/// Sub-pixel differences are rounding, not overflow.
///
/// Layout arithmetic on `f32` lands a hair either side of exact all the time —
/// a half-pixel report every frame would train everyone to ignore the message.
const NOTICEABLE: f32 = 0.5;

/// How far `used` runs past `available`, when that is worth saying.
///
/// Split out and pure so the decision is testable without a window, a frame, or
/// a captured stderr — the reporting below is the part that cannot be.
pub(crate) fn beyond(used: f32, available: f32) -> Option<f32> {
    let over = used - available;
    (over >= NOTICEABLE).then_some(over)
}

thread_local! {
    /// How many overflow reports this thread has produced.
    static REPORTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times something has overflowed on this thread.
///
/// # Why a count exists when the message already prints
///
/// The message goes to stderr, and stderr in a headless screenshot run is a
/// place nobody reads: `examples/fixtures` writes thirty-odd pictures and
/// prints a table, and an overflow report scrolls past between two of them
/// looking exactly like progress. A `Markdown` list that could not wrap —
/// every bulleted list in the studio's own reference pages painting past the
/// edge of the sidebar — printed this on every frame it was visible for as
/// long as it existed, and was found by reading a **screenshot**, not by
/// reading the output of the tool whose whole job is to say when layout is
/// wrong.
///
/// A number that a run can compare against zero is what turns that from
/// something a reviewer might notice into something a script can refuse.
///
/// # And for a long time nothing compared it
///
/// This function existed, with the paragraph above attached to it, and had
/// **no callers**. Five fixtures in `examples/fixtures` were overflowing the
/// whole time — a settings card cut off mid-row, a staggered list painting
/// three rows past the page, a shared-element transition laying a 192px detail
/// view into a 60px box — and the gallery printed all of it to the stderr this
/// doc had already identified as the place nobody reads.
///
/// `examples/fixtures`' runner now calls [`forget_reported`] before each
/// fixture and this after, names the fixture, and exits non-zero. The lesson
/// is not about overflow: a gate that is written but never wired is
/// indistinguishable from one that was never written, and reads in review as
/// though the problem is handled.
///
/// (The reference to `examples/tour` this paragraph used to open with was
/// itself an example of the same thing — that example has not existed for
/// some time.)
#[must_use]
pub fn reported() -> usize {
    REPORTED.with(std::cell::Cell::get)
}

/// Forget everything counted so far, so a caller can attribute the next batch.
pub fn forget_reported() {
    REPORTED.with(|count| count.set(0));
}

/// Say that `object` did not fit, once per distinct report.
///
/// **Layout runs every frame**, so an unconditional print here would be one line
/// per frame for as long as the screen is wrong — which buries the first one,
/// and the first one is the only one that says *when it started*. Deduplicated
/// on whole pixels, so a window being dragged reports each new shortfall once
/// rather than once per frame of the drag.
pub(crate) fn report(object: &'static str, axis: Axis, used: f32, available: f32, over: f32) {
    thread_local! {
        static LAST: RefCell<Option<(&'static str, Axis, f32)>> = const { RefCell::new(None) };
    }

    // Counted before the deduplication below, so a headless run can *fail* on
    // an overflow rather than only mention one. See `reported`.
    REPORTED.with(|count| count.set(count.get() + 1));

    // Rounded before comparing, or f32 jitter defeats the deduplication it is
    // there to do.
    let now = (object, axis, over.round());
    if LAST.with_borrow(|last| *last == Some(now)) {
        return;
    }
    LAST.with_borrow_mut(|last| *last = Some(now));

    let axis = match axis {
        Axis::Vertical => "vertical",
        Axis::Horizontal => "horizontal",
    };
    eprintln!(
        "vieww: {object} overflowed by {}px on the {axis} axis \
         — {} of children into {} of space",
        over.round(),
        used.round(),
        available.round(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_that_fits_says_nothing() {
        assert_eq!(beyond(300.0, 300.0), None);
        assert_eq!(beyond(100.0, 300.0), None);
    }

    #[test]
    fn a_shortfall_is_the_difference() {
        assert_eq!(beyond(600.0, 300.0), Some(300.0));
    }

    #[test]
    fn a_sub_pixel_shortfall_is_rounding_rather_than_news() {
        // The case that decides whether anybody keeps reading these messages:
        // layout on `f32` lands a hair over exact constantly, and a framework
        // that cried overflow every frame would be turned off within a day.
        assert_eq!(beyond(300.2, 300.0), None);
        assert_eq!(beyond(300.0 + f32::EPSILON, 300.0), None);
    }

    #[test]
    fn an_unbounded_axis_cannot_overflow() {
        // Why a list inside a scrollable never reports: the viewport hands it
        // infinite room, so it shrink-wraps and `used` equals `available`.
        // Asserted rather than assumed, because a special case for "am I in a
        // scrollable" is exactly what this must not need.
        assert_eq!(beyond(9000.0, f32::INFINITY), None);
    }
}
