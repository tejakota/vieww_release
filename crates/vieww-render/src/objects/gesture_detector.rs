use std::cell::Cell;
use std::time::Duration;

use vieww_foundation::{
    Constraints, KeyEvent, LogicalKey, NamedKey, Offset, PointerButton, Rect, ScrollEvent, Size,
    TapDetails,
};
use vieww_gestures::{
    DragRecognizer, GestureRecognizer, LongPressRecognizer, Recognized, ScaleRecognizer,
    TapRecognizer,
};
use vieww_widget::{GestureHandlers, Handler};

use crate::{LayoutCtx, RenderObject, SemanticAction};

/// How much of a screen a screen reader's "scroll down" moves.
///
/// Not all of it. A page that scrolls by exactly its own height leaves nothing
/// on screen that was there before, and somebody navigating by ear loses their
/// place completely.
const SEMANTIC_PAGE: f32 = 0.8;

/// Runs gesture handlers for its subtree.
///
/// Layout-transparent, and opaque to hit testing: a detector wrapping a
/// transparent area is still a target, or a tap on the padding around a button
/// would fall through to whatever is behind it.
///
/// # A tap is also a keypress
///
/// A [focusable](RenderObject::is_focusable) detector activates on Space and
/// Enter, running the same `on_tap` a finger would. The two keys are
/// deliberately not the same gesture:
///
/// - **Enter fires on the key going down**, because that is what every desktop
///   platform does and because Enter is how a form is submitted.
/// - **Space fires on the key coming *up***, and shows the control pressed
///   while it is held. Holding Space on a button and sliding off it mentally —
///   deciding not to — is a real interaction, and firing on the way down would
///   also make auto-repeat activate a button dozens of times.
#[derive(Debug, Default)]
pub struct RenderGestureDetector {
    handlers: GestureHandlers,
    /// Whether this can take the keyboard. Off by default: a detector wrapped
    /// around a scrollable or a barrier is not a tab stop, and a tree where
    /// everything is focusable makes Tab useless.
    focusable: bool,
    /// Set while Space is held on this control, so the key coming up knows the
    /// key going down was ours.
    ///
    /// A `Cell` for the reason [`RenderSlider`](crate::RenderSlider)'s width is
    /// one: keys arrive through `&self`.
    space_held: Cell<bool>,
    /// The smallest square this must be reachable across, from the theme.
    ///
    /// **Not opt-in**, which is the whole of `docs/AIMS.md` §I's remaining half.
    /// Before this, every built-in control called a private `touch_target`
    /// helper by hand — so the built-ins were compliant, the test suite passed,
    /// and a control written by anybody else got nothing. A rule each call site
    /// has to remember is a rule that holds only for the call sites somebody
    /// remembered.
    ///
    /// Zero disables it. That exists for the cases where reach is genuinely not
    /// wanted — see `Chip`, whose own docs argue density over reach — rather
    /// than as a general escape hatch.
    min_touch_target: f32,
}

impl Clone for RenderGestureDetector {
    /// The armed flag is deliberately *not* carried over.
    ///
    /// A clone is a fresh description of the same control, and a half-finished
    /// keypress belongs to the object the key actually went down on.
    fn clone(&self) -> Self {
        Self {
            handlers: self.handlers.clone(),
            focusable: self.focusable,
            space_held: Cell::new(false),
            min_touch_target: self.min_touch_target,
        }
    }
}

/// `size`'s box, grown about its centre until it is at least `minimum` square.
///
/// Grown symmetrically rather than from one corner, so a control stays centred
/// in the area that reaches it — an expansion anchored at the origin puts the
/// generous half on one side, and a thumb approaching from the other feels the
/// control shift.
///
/// Never shrinks: an object already larger than the minimum on an axis keeps
/// its own extent on that axis, so a tall thin control gains width and not
/// height.
fn expanded_to(size: Size, minimum: f32) -> Rect {
    // `is_finite` first, and it is doing real work: `NAN` fails every
    // comparison, so a bare `minimum <= 0.0` would let it through and the
    // arithmetic below would produce a rect that contains nothing — a control
    // that silently stops taking taps at all.
    if !minimum.is_finite() || minimum <= 0.0 {
        return Rect::from_origin_size(Offset::ZERO, size);
    }
    let grow_x = ((minimum - size.width) / 2.0).max(0.0);
    let grow_y = ((minimum - size.height) / 2.0).max(0.0);
    Rect::new(-grow_x, -grow_y, size.width + grow_x, size.height + grow_y)
}

impl RenderGestureDetector {
    #[must_use]
    pub fn new(handlers: GestureHandlers) -> Self {
        Self {
            handlers,
            focusable: false,
            space_held: Cell::new(false),
            min_touch_target: 0.0,
        }
    }

    /// The smallest square this must be reachable across.
    ///
    /// Set from the theme by the widget layer. Zero leaves the hit area exactly
    /// equal to the layout box.
    #[must_use]
    pub const fn min_touch_target(mut self, minimum: f32) -> Self {
        self.min_touch_target = minimum;
        self
    }

    /// Whether this is a tab stop that Space and Enter activate.
    #[must_use]
    pub const fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }

    /// Report a press and then a tap, as a keypress does.
    ///
    /// The press first, so that anything highlighting on it is consistent with
    /// what a finger produces — a control should not have to know which of the
    /// two activated it.
    fn activate(&self) {
        // No position: a keypress happened *at* the control, not at a point in
        // it. Everything reading `local` here is a control that ignores it, and
        // the alternative — inventing a centre — would be a coordinate no finger
        // ever produced.
        let details = TapDetails::at(Offset::ZERO);
        call(self.handlers.on_tap_down.as_ref(), details);
        call(self.handlers.on_tap.as_ref(), details);
    }
}

impl RenderObject for RenderGestureDetector {
    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        let Some(&child) = ctx.children().first() else {
            return constraints.smallest();
        };
        let size = ctx.layout_child(child, constraints);
        ctx.place_child(child, Offset::ZERO);
        size
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        // Opaque: everything inside these bounds is a target, drawn or not.
        // "These bounds" is `hit_bounds` below, which may be wider than the box.
        true
    }

    fn hit_bounds(&self, size: Size) -> Rect {
        expanded_to(size, self.min_touch_target)
    }

    fn gesture_recognizers(&self) -> Vec<Box<dyn GestureRecognizer>> {
        let mut recognizers: Vec<Box<dyn GestureRecognizer>> = Vec::new();

        // Order is arena join order, and a sweep awards a still finger to the
        // first. Tap before long press before drag: the tap is what a press that
        // resolves nothing should become.
        if self.handlers.wants_tap() {
            recognizers.push(Box::new(TapRecognizer::new()));
        }
        if self.handlers.wants_secondary_tap() {
            // Its own member. A right-click and a left-click are different
            // gestures with different handlers, and one recogniser reporting
            // both would have already contested the arena on behalf of a
            // gesture the widget did not ask for.
            recognizers.push(Box::new(TapRecognizer::for_button(
                PointerButton::Secondary,
            )));
        }
        // Same argument again, twice. A detector that wants only the back
        // button must not contest an ordinary tap on the way.
        if self.handlers.wants_back_tap() {
            recognizers.push(Box::new(TapRecognizer::for_button(PointerButton::Back)));
        }
        if self.handlers.wants_forward_tap() {
            recognizers.push(Box::new(TapRecognizer::for_button(PointerButton::Forward)));
        }
        if self.handlers.on_long_press.is_some() {
            recognizers.push(Box::new(LongPressRecognizer::new()));
        }
        if self.handlers.wants_drag() {
            recognizers.push(match self.handlers.drag_axis {
                Some(axis) => Box::new(DragRecognizer::along(axis)),
                None => Box::new(DragRecognizer::new()),
            });
        }
        if self.handlers.wants_scale() {
            recognizers.push(Box::new(ScaleRecognizer::new()));
        }
        recognizers
    }

    fn handle_gesture(&self, gesture: &Recognized, _local: Offset) {
        match gesture {
            Recognized::TapDown(details) => call(self.handlers.on_tap_down.as_ref(), *details),
            // One object, up to four tap recognisers — the button on the
            // gesture is the only thing that says which of them produced it.
            //
            // **Every button is named.** This was a fallthrough sending
            // everything that was not primary to `on_secondary_tap`, which was
            // correct exactly while secondary was the only other button there
            // was: the moment back and forward existed, a thumb button would
            // have silently opened a context menu. A wildcard over an enum that
            // is expected to grow is a bug with a delay on it.
            Recognized::Tap(details) => match details.button {
                PointerButton::Primary => call(self.handlers.on_tap.as_ref(), *details),
                PointerButton::Secondary => {
                    call(self.handlers.on_secondary_tap.as_ref(), *details);
                }
                PointerButton::Back => call(self.handlers.on_back_tap.as_ref(), *details),
                PointerButton::Forward => call(self.handlers.on_forward_tap.as_ref(), *details),
                // No recogniser is ever registered for the middle button, so
                // this is unreachable rather than ignored — and it stays a
                // named arm so that adding a button breaks here, loudly, at
                // compile time, instead of quietly at run time.
                PointerButton::Middle => {}
            },
            Recognized::LongPress(details) => call(self.handlers.on_long_press.as_ref(), *details),
            Recognized::DragStart(details) => call(self.handlers.on_drag_start.as_ref(), *details),
            Recognized::DragUpdate(details) => {
                call(self.handlers.on_drag_update.as_ref(), *details)
            }
            Recognized::DragEnd(details) => call(self.handlers.on_drag_end.as_ref(), *details),
            Recognized::ScaleStart(details) => {
                call(self.handlers.on_scale_start.as_ref(), *details)
            }
            Recognized::ScaleUpdate(details) => {
                call(self.handlers.on_scale_update.as_ref(), *details)
            }
            Recognized::ScaleEnd(details) => call(self.handlers.on_scale_end.as_ref(), *details),
            Recognized::TapCancel => call(self.handlers.on_tap_cancel.as_ref(), ()),
        }
    }

    fn handle_semantic_action(&self, action: SemanticAction, size: Size) -> bool {
        match action {
            SemanticAction::Activate if self.handlers.on_tap.is_some() => {
                // The same path a keypress takes, press and all, so a control
                // has one notion of being activated rather than three.
                self.activate();
                true
            }
            SemanticAction::ScrollForward | SemanticAction::ScrollBackward => {
                let Some(handler) = self.handlers.on_scroll.as_ref() else {
                    return false;
                };
                // Most of a screen rather than all of it: a page that scrolls by
                // exactly its own height leaves nothing on screen to connect the
                // two, and somebody navigating by ear has no landmark at all.
                let page = size.height * SEMANTIC_PAGE;
                let dy = if action == SemanticAction::ScrollForward {
                    -page
                } else {
                    page
                };
                // `Duration::ZERO`, and it is honest rather than lazy: a
                // semantic scroll is a discrete jump requested by a screen
                // reader, not something a hand is doing over time, so there is
                // no moment for it to have happened at. A handler that starts a
                // simulation on this timestamp gets one that is already over,
                // which is the right answer for a page jump — it should land,
                // not glide.
                handler(ScrollEvent::new(
                    Offset::ZERO,
                    Offset::new(0.0, dy),
                    Duration::ZERO,
                ));
                true
            }
            _ => false,
        }
    }

    fn wants_hover(&self) -> bool {
        self.handlers.on_hover.is_some()
    }

    fn handle_hover(&self, inside: bool) {
        call(self.handlers.on_hover.as_ref(), inside);
    }

    fn handle_scroll(&self, event: &ScrollEvent) -> bool {
        let Some(handler) = self.handlers.on_scroll.as_ref() else {
            // Not ours. Saying so is what lets it reach the page around the
            // list once the list has nothing left to give.
            return false;
        };
        // **A wheel across a detector's axis is not that detector's wheel.**
        //
        // `Scrollable` sets `drag_axis` and its scroll handler already returns
        // without doing anything when the notch has no movement along that axis
        // — "not ours, and saying nothing here is what lets it reach whatever is
        // around us". Saying nothing was not enough: this returned `true`
        // regardless, so `FrameDriver::handle_scroll` stopped bubbling and the
        // event never reached the scrollable that *did* want it.
        //
        // What that looked like: a code pane is a horizontal `Scrollable`
        // around the text inside a vertical one around the pane. The horizontal
        // one is innermost, so it saw every wheel first, ignored the vertical
        // ones — and consumed them. The file could not be scrolled at all, and
        // nothing was wired up wrong.
        //
        // **Zero was still not the only "not ours".** The gate above passes any
        // event with a *non-zero* component along the axis, and a touchpad
        // almost never reports an exact zero on the axis the finger is not
        // using: two fingers on a trackpad produce a few pixels of drift
        // sideways on a vertical scroll and back. Under the old gate the
        // innermost horizontal scrollable claimed every mostly-vertical swipe
        // on its four pixels of noise, dropped them (the scroll handler keeps
        // only its own component), and the vertical scrollable behind it never
        // heard about a scroll the user was making in earnest. What that looked
        // like: the editor's vertical wheel worked on a mouse and not on a
        // touchpad, and the horizontal bar dragged fine — exactly the two facts
        // in the report.
        //
        // So the gate is **dominance**, not mere presence: the movement along
        // the declared axis has to be at least as large as the movement across
        // it. An honest diagonal (the finger meaning both) still belongs to the
        // innermost scrollable, which is the one under the pointer; the noise
        // case, where the declared axis merely tagged along, bubbles. A delta
        // of exactly zero along the axis is not ours either, as before.
        if let Some(axis) = self.handlers.drag_axis {
            let (along, across) = match axis {
                vieww_foundation::Axis::Vertical => (event.delta.dy, event.delta.dx),
                vieww_foundation::Axis::Horizontal => (event.delta.dx, event.delta.dy),
            };
            if along == 0.0 || along.abs() < across.abs() {
                return false;
            }
        }
        // The whole event. This was `handler(event.delta)`, which dropped the
        // only clock on the wheel path — and a wheel is the one input that has
        // to synthesise its own release, which needs one.
        handler(*event);
        true
    }

    fn is_focusable(&self) -> bool {
        // Focusable *and* activatable: a tab stop that does nothing when you
        // press Enter on it is a trap, and a control with no tap handler is
        // disabled.
        self.focusable && self.handlers.on_tap.is_some()
    }

    fn handle_key(&self, event: &KeyEvent) -> bool {
        if self.handlers.on_tap.is_none() {
            return false;
        }
        match event.key {
            LogicalKey::Named(NamedKey::Enter) if event.is_down() => {
                // Repeats included: holding Enter on a button repeating is the
                // platform behaviour, and a form that submits twice is the
                // application's problem to debounce rather than ours to hide.
                self.activate();
                true
            }
            LogicalKey::Named(NamedKey::Space) if event.is_down() => {
                if !self.space_held.replace(true) {
                    call(
                        self.handlers.on_tap_down.as_ref(),
                        TapDetails::at(Offset::ZERO),
                    );
                }
                true
            }
            LogicalKey::Named(NamedKey::Space) => {
                // The up. Only ours if the down was: focus can have moved here
                // while the key was already held, and activating then would fire
                // a control the user never pressed.
                if self.space_held.replace(false) {
                    call(self.handlers.on_tap.as_ref(), TapDetails::at(Offset::ZERO));
                }
                true
            }
            _ => false,
        }
    }

    crate::intrinsics::pass_through_intrinsic!();

    crate::baseline::pass_through_baseline!();

    fn layout_differs(&self, _new: &dyn RenderObject) -> bool {
        // Handlers cannot change geometry, and neither can being focusable. A
        // rebuild hands down new closures every time, and relaying out for them
        // would defeat the relayout boundary above.
        false
    }

    fn debug_name(&self) -> &'static str {
        "RenderGestureDetector"
    }
}

fn call<T>(handler: Option<&Handler<T>>, details: T) {
    if let Some(handler) = handler {
        handler(details);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;

    /// A detector counting activations, and the presses reported along the way.
    fn detector(focusable: bool) -> (RenderGestureDetector, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let taps = Rc::new(Cell::new(0));
        let downs = Rc::new(Cell::new(0));
        let (t, d) = (Rc::clone(&taps), Rc::clone(&downs));

        let handlers = GestureHandlers {
            on_tap: Some(Rc::new(move |_| t.set(t.get() + 1))),
            on_tap_down: Some(Rc::new(move |_| d.set(d.get() + 1))),
            ..GestureHandlers::default()
        };
        (
            RenderGestureDetector::new(handlers).focusable(focusable),
            taps,
            downs,
        )
    }

    /// A detector that scrolls along one axis must **decline** a wheel across
    /// it, so the event bubbles to whatever scrolls the other way.
    ///
    /// The case: a code pane is a horizontal `Scrollable` inside a vertical
    /// one. The horizontal detector is innermost and sees every wheel first;
    /// before this it consumed the vertical ones too and the pane could not be
    /// scrolled at all.
    #[test]
    fn a_wheel_across_the_axis_is_declined_so_it_can_bubble() {
        let seen = Rc::new(Cell::new(0));
        let counted = Rc::clone(&seen);
        let handlers = GestureHandlers {
            on_scroll: Some(Rc::new(move |_| counted.set(counted.get() + 1))),
            drag_axis: Some(vieww_foundation::Axis::Horizontal),
            ..GestureHandlers::default()
        };
        let detector = RenderGestureDetector::new(handlers);

        let wheel = |dx: f32, dy: f32| ScrollEvent {
            position: Offset::ZERO,
            delta: Offset::new(dx, dy),
            timestamp: Duration::from_millis(1),
        };

        assert!(
            !detector.handle_scroll(&wheel(0.0, -120.0)),
            "a vertical notch is not a horizontal scrollable's"
        );
        assert_eq!(seen.get(), 0, "and its handler is not even called");

        assert!(
            detector.handle_scroll(&wheel(-40.0, 0.0)),
            "its own axis still is"
        );
        assert_eq!(seen.get(), 1);
    }

    /// The second half of the same defect, and the one the report describes:
    /// a touchpad reports a few pixels of drift on the axis the finger is not
    /// using, and an innermost scrollable that claims on mere presence takes
    /// every mostly-other-axis scroll away from the scrollable behind it.
    #[test]
    fn a_wheel_dominated_by_the_other_axis_is_declined() {
        let seen = Rc::new(Cell::new(0));
        let counted = Rc::clone(&seen);
        let handlers = GestureHandlers {
            on_scroll: Some(Rc::new(move |_| counted.set(counted.get() + 1))),
            drag_axis: Some(vieww_foundation::Axis::Horizontal),
            ..GestureHandlers::default()
        };
        let detector = RenderGestureDetector::new(handlers);

        let wheel = |dx: f32, dy: f32| ScrollEvent {
            position: Offset::ZERO,
            delta: Offset::new(dx, dy),
            timestamp: Duration::from_millis(1),
        };

        assert!(
            !detector.handle_scroll(&wheel(-4.0, -120.0)),
            "four pixels of drift do not make a vertical swipe horizontal"
        );
        assert_eq!(seen.get(), 0, "the noise is not fed to the handler either");

        assert!(
            detector.handle_scroll(&wheel(-120.0, -4.0)),
            "a swipe dominated by the axis is still ours"
        );
        assert_eq!(seen.get(), 1);

        // An honest tie — the finger meant both — goes to the innermost
        // scrollable, which is the one under the pointer.
        assert!(detector.handle_scroll(&wheel(-60.0, -60.0)));
        assert_eq!(seen.get(), 2);
    }

    #[test]
    fn each_mouse_button_reaches_its_own_handler_and_no_others() {
        // The arm this pins used to be a wildcard: everything that was not
        // primary went to `on_secondary_tap`. That was correct exactly while
        // secondary was the only other button, so the back button would have
        // opened context menus the day it was added — a bug with a delay on it
        // rather than a mistake anybody would have made twice.
        let seen: Rc<Cell<(usize, usize, usize, usize)>> = Rc::new(Cell::new((0, 0, 0, 0)));
        let bump = |slot: usize| {
            let seen = Rc::clone(&seen);
            move |_| {
                let (a, b, c, d) = seen.get();
                seen.set(match slot {
                    0 => (a + 1, b, c, d),
                    1 => (a, b + 1, c, d),
                    2 => (a, b, c + 1, d),
                    _ => (a, b, c, d + 1),
                });
            }
        };

        let handlers = GestureHandlers {
            on_tap: Some(Rc::new(bump(0))),
            on_secondary_tap: Some(Rc::new(bump(1))),
            on_back_tap: Some(Rc::new(bump(2))),
            on_forward_tap: Some(Rc::new(bump(3))),
            ..GestureHandlers::default()
        };
        let detector = RenderGestureDetector::new(handlers);

        let tap = |button| {
            Recognized::Tap(TapDetails {
                position: Offset::ZERO,
                local: Offset::ZERO,
                button,
                timestamp: Duration::ZERO,
                modifiers: vieww_foundation::Modifiers::NONE,
            })
        };

        detector.handle_gesture(&tap(PointerButton::Primary), Offset::ZERO);
        assert_eq!(seen.get(), (1, 0, 0, 0), "a left click is a tap");

        detector.handle_gesture(&tap(PointerButton::Secondary), Offset::ZERO);
        assert_eq!(seen.get(), (1, 1, 0, 0), "a right click is a secondary tap");

        detector.handle_gesture(&tap(PointerButton::Back), Offset::ZERO);
        assert_eq!(
            seen.get(),
            (1, 1, 1, 0),
            "the back button must not reach the context menu"
        );

        detector.handle_gesture(&tap(PointerButton::Forward), Offset::ZERO);
        assert_eq!(
            seen.get(),
            (1, 1, 1, 1),
            "and forward is its own thing again"
        );
    }

    #[test]
    fn a_detector_registers_one_recogniser_per_button_it_was_given() {
        // Each button contests the arena separately, so a detector wanting only
        // the back button must not enter a recogniser for an ordinary tap — it
        // would win presses meant for whatever is underneath it.
        let only_back = RenderGestureDetector::new(GestureHandlers {
            on_back_tap: Some(Rc::new(|_| {})),
            ..GestureHandlers::default()
        });
        assert_eq!(only_back.gesture_recognizers().len(), 1);

        let all_four = RenderGestureDetector::new(GestureHandlers {
            on_tap: Some(Rc::new(|_| {})),
            on_secondary_tap: Some(Rc::new(|_| {})),
            on_back_tap: Some(Rc::new(|_| {})),
            on_forward_tap: Some(Rc::new(|_| {})),
            ..GestureHandlers::default()
        });
        assert_eq!(all_four.gesture_recognizers().len(), 4);
    }

    fn down(named: NamedKey) -> KeyEvent {
        KeyEvent::down(LogicalKey::Named(named), Duration::ZERO)
    }

    fn up(named: NamedKey) -> KeyEvent {
        KeyEvent::up(LogicalKey::Named(named), Duration::ZERO)
    }

    #[test]
    fn enter_activates_on_the_way_down() {
        let (detector, taps, downs) = detector(true);
        assert!(detector.handle_key(&down(NamedKey::Enter)));

        assert_eq!(taps.get(), 1, "Enter submits, and it submits immediately");
        assert_eq!(downs.get(), 1, "and reports a press, as a finger would");
    }

    #[test]
    fn space_shows_the_press_and_activates_on_the_way_up() {
        let (detector, taps, downs) = detector(true);

        detector.handle_key(&down(NamedKey::Space));
        assert_eq!(downs.get(), 1, "held Space looks pressed");
        assert_eq!(taps.get(), 0, "and has not fired yet");

        detector.handle_key(&up(NamedKey::Space));
        assert_eq!(taps.get(), 1);
    }

    #[test]
    fn holding_space_does_not_activate_once_per_repeat() {
        let (detector, taps, downs) = detector(true);
        for _ in 0..10 {
            detector.handle_key(&down(NamedKey::Space));
        }

        assert_eq!(taps.get(), 0);
        assert_eq!(downs.get(), 1, "one press, however long it is held");
    }

    #[test]
    fn a_space_that_went_down_somewhere_else_does_not_activate_this() {
        // Focus can move while the key is held — a screen reader does it, and so
        // does a handler that rebuilds. Activating on the stray up would fire a
        // control the user never pressed.
        let (detector, taps, _) = detector(true);
        detector.handle_key(&up(NamedKey::Space));
        assert_eq!(taps.get(), 0);
    }

    #[test]
    fn a_detector_is_not_a_tab_stop_unless_it_asks_to_be() {
        assert!(detector(true).0.is_focusable());
        assert!(
            !detector(false).0.is_focusable(),
            "a detector around a scrollable or a barrier is not a tab stop"
        );
        assert!(
            !RenderGestureDetector::new(GestureHandlers::default())
                .focusable(true)
                .is_focusable(),
            "and a tab stop that does nothing when activated is a trap"
        );
    }

    #[test]
    fn an_unrelated_key_is_left_for_whatever_is_above() {
        let (detector, ..) = detector(true);
        assert!(
            !detector.handle_key(&down(NamedKey::Escape)),
            "Escape belongs to the dialog around the button, not to the button"
        );
    }
}

#[cfg(test)]
mod touch_target_tests {
    use vieww_foundation::{Offset, Size};
    use vieww_widget::GestureHandlers;

    use super::{expanded_to, RenderGestureDetector};
    use crate::RenderObject;

    fn detector(minimum: f32) -> RenderGestureDetector {
        RenderGestureDetector::new(GestureHandlers::default()).min_touch_target(minimum)
    }

    #[test]
    fn a_small_control_is_reachable_across_the_minimum() {
        // **The gap this closes.** `docs/AIMS.md` §I asks that expansion not be
        // opt-in. A 20×20 control now takes a tap 8pt outside itself, without
        // any control having asked for it.
        let bounds = detector(48.0).hit_bounds(Size::new(20.0, 20.0));
        assert_eq!(bounds.width(), 48.0);
        assert_eq!(bounds.height(), 48.0);
        assert!(bounds.contains(Offset::new(-8.0, -8.0)));
        assert!(bounds.contains(Offset::new(27.0, 27.0)));
    }

    #[test]
    fn the_layout_box_is_not_what_moved() {
        // The load-bearing half: reach expands, pixels do not. An expanded area
        // has a *negative* origin, which is only possible because the box itself
        // stayed where it was — an accessibility fix that moved the control
        // would be a visual regression and would get reverted.
        let bounds = detector(48.0).hit_bounds(Size::new(20.0, 20.0));
        assert_eq!(bounds.left, -14.0);
        assert_eq!(bounds.top, -14.0);
        assert_eq!(bounds.right, 34.0);
    }

    #[test]
    fn expansion_is_centred_so_the_control_does_not_feel_offset() {
        // Anchored at the origin, the generous half lands on one side and a
        // thumb approaching from the other feels the control shift.
        let bounds = detector(48.0).hit_bounds(Size::new(20.0, 20.0));
        assert_eq!(-bounds.left, bounds.right - 20.0);
        assert_eq!(-bounds.top, bounds.bottom - 20.0);
    }

    #[test]
    fn a_control_already_big_enough_is_left_exactly_alone() {
        let size = Size::new(120.0, 60.0);
        let bounds = detector(48.0).hit_bounds(size);
        assert_eq!(bounds, expanded_to(size, 0.0));
    }

    #[test]
    fn a_tall_thin_control_gains_width_and_not_height() {
        // Per axis, because a slider's track is meant to be thin and making it
        // reachable across 48pt vertically is right while stretching it
        // horizontally past its own ends is not.
        let bounds = detector(48.0).hit_bounds(Size::new(10.0, 200.0));
        assert_eq!(bounds.width(), 48.0);
        assert_eq!(bounds.height(), 200.0, "it was already tall enough");
    }

    #[test]
    fn a_zero_minimum_is_exactly_the_layout_box() {
        // `Chip`'s considered exemption, and the only one.
        let size = Size::new(20.0, 20.0);
        let bounds = detector(0.0).hit_bounds(size);
        assert_eq!(bounds.left, 0.0);
        assert_eq!(bounds.top, 0.0);
        assert_eq!(bounds.width(), 20.0);
    }

    #[test]
    fn a_nonsense_minimum_does_not_produce_a_nonsense_area() {
        // `NAN` fails every comparison, so a naive `minimum > size` test would
        // let it through and produce a rect nothing can be inside.
        let size = Size::new(20.0, 20.0);
        for minimum in [f32::NAN, -10.0] {
            let bounds = expanded_to(size, minimum);
            assert!(
                bounds.contains(Offset::new(10.0, 10.0)),
                "a {minimum} minimum made the control unreachable"
            );
        }
    }
}
