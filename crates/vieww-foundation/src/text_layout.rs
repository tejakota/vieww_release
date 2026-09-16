//! What an editable field *measured*, published back to whoever built it.
//!
//! # The problem this solves
//!
//! Everything an editor draws **beside** its text rather than **on** it — a
//! completion popup at the caret, a hint at the end of a line, a scrollbar
//! thumb sized to the paragraph — needs geometry that only exists after
//! shaping. `Paragraph` knows all of it (`cursor_rect`, `line_rect`,
//! `line_baseline`, `selection_rects`) and the render object holds the
//! `Paragraph`, but an application holds a *widget*: it never sees a render
//! object, and by the time layout has run its `build` is long over.
//!
//! [`TextDecoration`](crate::TextDecoration) closed half of this — marks that
//! are *inside* the field can be described as ranges and positioned where the
//! geometry is known. It cannot close the other half, because a popup is not
//! part of the field's paint at all: it belongs to the application's tree,
//! above everything, and has to be given a position in advance.
//!
//! So the field measures, and hands the measurements back.
//!
//! # It is one frame behind, and that is the whole contract
//!
//! A [`TextLayoutProbe`] is written at the end of layout and read by the next
//! `build`. A caller positioning something from it is positioning it from the
//! *previous* frame's geometry. For a caret that moved one character since,
//! that is a popup a few pixels off for 16 milliseconds; for a caret that has
//! not moved, it is exact.
//!
//! What it must never be used for is the field's own layout. Feeding a report
//! back into the size of anything that contains the field is a cycle, and the
//! cycle will not converge — that is why this is a plain shared cell and not a
//! signal. Read it during `build`; do not subscribe to it.
//!
//! Callers that need the tree rebuilt when the numbers change ask for that
//! explicitly, with [`TextLayoutProbe::on_change`], which fires only when the
//! report actually differs from the one before it. Because a second layout of
//! unchanged text produces an equal report, that settles after one extra
//! frame rather than looping.
//!
//! # Coordinates
//!
//! Every rect is in the field's own coordinate space — the origin is the
//! field's top-left, before any scroll the application applies around it.
//! Nothing here is specific to a script, a writing direction or a platform:
//! the rects are whatever the shaper produced, so a right-to-left paragraph
//! reports right-to-left rects and a vertically-scrolled one reports the same
//! numbers a horizontally-scrolled one would.

use std::cell::RefCell;
use std::rc::Rc;

use crate::{Rect, Size};

/// The geometry an editable field had at its last layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextLayoutReport {
    /// One rect per **visual** line, in order.
    ///
    /// Visual, not source: a field that wraps reports one entry per wrapped
    /// row. A caller keying anything to source lines should be laying its
    /// field out with wrapping off, which is the only way the two agree.
    pub lines: Vec<Rect>,
    /// Where the caret is drawn, at the selection's extent.
    pub cursor: Rect,
    /// The paragraph's own size, which is not the field's size: a field given
    /// more room than its text needs is larger than this.
    pub paragraph: Size,
    /// The style's font size at the time of the measurement, so a caller can
    /// tell a report it should re-read from one it can trust.
    pub font_size: f32,
}

impl TextLayoutReport {
    /// The rect of visual line `index`, if the field has one.
    #[must_use]
    pub fn line(&self, index: usize) -> Option<Rect> {
        self.lines.get(index).copied()
    }

    /// Whether anything has been measured yet.
    ///
    /// A caller drawing from a report should check this rather than trusting a
    /// zeroed rect: the first frame of a field's life has no measurement, and
    /// a popup placed at the origin is a popup in the wrong corner.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// A cell an editable field writes its [`TextLayoutReport`] into.
///
/// Cloning shares the cell; both halves see one report.
#[derive(Clone, Default)]
pub struct TextLayoutProbe {
    report: Rc<RefCell<TextLayoutReport>>,
    #[expect(
        clippy::type_complexity,
        reason = "a shared, optional, interior-mutable callback is what a probe with no listener \
                  and a probe with one both have to be"
    )]
    on_change: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl TextLayoutProbe {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `callback` whenever a layout produces a report **different** from
    /// the last one.
    ///
    /// Different, not merely new: a field re-laid out with the same text at
    /// the same size reports the same rects, so a callback that rebuilds the
    /// tree settles after one extra frame instead of running forever.
    #[must_use]
    pub fn on_change(self, callback: Rc<dyn Fn()>) -> Self {
        *self.on_change.borrow_mut() = Some(callback);
        self
    }

    /// The same, on a probe that already exists.
    ///
    /// The consuming builder is the one to reach for while assembling a field;
    /// this is for a long-lived probe held in application state, which is
    /// constructed before the thing it has to wake.
    pub fn set_on_change(&self, callback: Rc<dyn Fn()>) {
        *self.on_change.borrow_mut() = Some(callback);
    }

    /// Read the last report.
    ///
    /// Borrows, so hold the result no longer than the expression that uses it
    /// — a field laying out while a caller holds this borrow would panic, and
    /// that is a caller keeping a report across a frame rather than reading
    /// one during a build.
    #[must_use]
    pub fn read(&self) -> std::cell::Ref<'_, TextLayoutReport> {
        self.report.borrow()
    }

    /// A copy of the last report, for a caller that would rather own it than
    /// hold a borrow.
    #[must_use]
    pub fn snapshot(&self) -> TextLayoutReport {
        self.report.borrow().clone()
    }

    /// Publish `report`. Called by the render object at the end of layout.
    ///
    /// Returns whether anything changed, which is also what decides whether
    /// the change callback runs.
    pub fn publish(&self, report: TextLayoutReport) -> bool {
        {
            let mut slot = self.report.borrow_mut();
            if *slot == report {
                return false;
            }
            *slot = report;
        }
        // The callback is taken out of the cell before it is called: it is
        // very likely to mark a signal, which is very likely to lead back into
        // a build, and a build must be free to touch this probe.
        let callback = self.on_change.borrow().clone();
        if let Some(callback) = callback {
            callback();
        }
        true
    }
}

impl std::fmt::Debug for TextLayoutProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextLayoutProbe")
            .field("report", &self.report.borrow())
            .field("on_change", &self.on_change.borrow().is_some())
            .finish()
    }
}

/// Two probes are equal when they are the *same* probe.
///
/// Identity rather than contents, because this is what a render object's
/// `layout_eq` asks: a field handed the same cell twice has not changed, and a
/// field handed a different one has to publish into it.
impl PartialEq for TextLayoutProbe {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.report, &other.report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(width: f32) -> TextLayoutReport {
        TextLayoutReport {
            lines: vec![Rect::new(0.0, 0.0, width, 20.0)],
            ..TextLayoutReport::default()
        }
    }

    #[test]
    fn an_empty_report_says_so() {
        assert!(TextLayoutReport::default().is_empty());
        assert!(!report(10.0).is_empty());
        assert_eq!(report(10.0).line(0).map(|r| r.right), Some(10.0));
        assert_eq!(report(10.0).line(1), None);
    }

    #[test]
    fn publishing_the_same_report_twice_notifies_once() {
        let seen = Rc::new(std::cell::Cell::new(0));
        let probe = TextLayoutProbe::new().on_change({
            let seen = Rc::clone(&seen);
            Rc::new(move || seen.set(seen.get() + 1))
        });

        assert!(probe.publish(report(10.0)));
        assert!(!probe.publish(report(10.0)));
        assert!(probe.publish(report(20.0)));

        // Twice, not three times: the middle publish was the same report.
        assert_eq!(seen.get(), 2);
        assert_eq!(probe.read().line(0).map(|r| r.right), Some(20.0));
    }

    #[test]
    fn a_callback_may_read_the_probe_it_was_woken_by() {
        // The callback is very likely to lead into a build, and a build reads
        // the report. If `publish` still held the borrow, this would panic.
        let probe = TextLayoutProbe::new();
        let inner = probe.clone();
        let probe = probe.on_change(Rc::new(move || {
            assert_eq!(inner.read().line(0).map(|r| r.right), Some(10.0));
        }));
        assert!(probe.publish(report(10.0)));
    }

    #[test]
    fn clones_share_one_report_and_compare_equal() {
        let probe = TextLayoutProbe::new();
        let clone = probe.clone();
        probe.publish(report(7.0));
        assert_eq!(clone.snapshot(), report(7.0));
        assert_eq!(probe, clone);
        assert_ne!(probe, TextLayoutProbe::new());
    }
}
