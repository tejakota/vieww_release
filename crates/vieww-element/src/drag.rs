//! What a drag is carrying, where it is, and what is underneath it.
//!
//! # Why a drag needs a controller and a tap does not
//!
//! Every other gesture in this framework begins and ends inside one widget's
//! subtree. A tap presses the thing it landed on; a scroll moves the viewport it
//! started in. A drag is the exception: it starts in one widget, travels across
//! others that know nothing about it, and finishes in a third — and the thing
//! being carried has to survive all of that.
//!
//! Nothing in the widget tree can hold that. A widget is rebuilt from scratch
//! every frame and owns no state; an element owns state but only its own. So the
//! session lives here, beside [`ScrollController`](crate::ScrollController) and
//! [`NavigatorController`](crate::NavigatorController), for the same reason
//! those do: it is application state that several widgets read.
//!
//! # Targets register themselves rather than being searched for
//!
//! Hit testing cannot answer "what is under the pointer" here, because the thing
//! under the pointer during a drag is the *feedback* — the widget travelling with
//! the finger — and beneath that, whatever the drag started on. Both are wrong.
//!
//! So a target reports its rectangle each frame (through
//! `Measured`) and this resolves the pointer against those rectangles instead.
//! That is a smaller mechanism than a second hit-test pass, and it is the same
//! rectangle a screen reader would be given.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Offset, Rect};

use crate::{Runtime, Signal};

/// A drag that was released over something willing to take it.
#[derive(Clone)]
pub struct Dropped {
    /// The payload, still type-erased. Use [`take`](Self::take) to get it back.
    data: Rc<dyn Any>,
    /// Which target it landed on.
    pub target: DragTargetId,
}

impl Dropped {
    /// The payload, if it is of type `T`.
    ///
    /// Returns `None` rather than panicking when the type is wrong: a target
    /// that registered for one type and is handed another is a bug in the
    /// caller, and a crash in a drop handler loses the user's data along with
    /// the drag.
    #[must_use]
    pub fn take<T: 'static>(&self) -> Option<Rc<T>> {
        Rc::clone(&self.data).downcast::<T>().ok()
    }
}

impl fmt::Debug for Dropped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The payload is `dyn Any` and has no `Debug`; the target is the part
        // worth seeing in a log.
        f.debug_struct("Dropped")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Identifies one drop target within a [`DragController`].
///
/// A plain number the application chooses, rather than something derived from
/// the tree: a target's identity has to survive the rebuild that happens *while
/// it is being dragged over*, and nothing derived from a widget does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DragTargetId(pub u64);

/// One registered target.
#[derive(Debug, Clone, Copy)]
struct Target {
    id: DragTargetId,
    rect: Rect,
    /// The payload type this target will take. Checked here rather than in the
    /// widget so that "what is highlighted" and "what will accept" can never
    /// disagree — a target that lit up and then refused the drop would be the
    /// worst of both.
    accepts: TypeId,
}

/// What is being dragged, and where.
struct Session {
    data: Rc<dyn Any>,
    /// The payload's concrete type, kept separately because `Rc<dyn Any>`
    /// reports the `TypeId` of the *erased* value only through `type_id`, and
    /// reading it back through the trait object at every move is needless.
    kind: TypeId,
}

/// The state of a drag in progress.
///
/// ```
/// use vieww_element::{DragController, DragTargetId, Runtime};
/// use vieww_foundation::{Offset, Rect};
///
/// let runtime = Runtime::new();
/// let drag = DragController::new(&runtime);
///
/// drag.register::<String>(DragTargetId(1), Rect::new(0.0, 0.0, 100.0, 100.0));
/// drag.start(String::from("a file"), Offset::new(200.0, 200.0));
/// drag.update(Offset::new(50.0, 50.0));
///
/// assert_eq!(drag.over(), Some(DragTargetId(1)));
/// let dropped = drag.finish().expect("released over the target");
/// assert_eq!(dropped.take::<String>().as_deref(), Some(&String::from("a file")));
/// ```
#[derive(Clone)]
pub struct DragController {
    /// Shared, so every widget holding a clone sees one drag rather than its
    /// own. The same shape [`ScrollController`](crate::ScrollController) and
    /// [`NavigatorController`](crate::NavigatorController) use, and for the
    /// same reason: a controller is passed *by value* into the widgets that
    /// read it, and a copy of the session would be a second drag.
    inner: Rc<RefCell<Inner>>,
    /// Where the pointer is, or `None` when nothing is being dragged.
    ///
    /// A signal because the feedback widget follows it, and following it means
    /// rebuilding on every move.
    position: Signal<Option<Offset>>,
    /// Which target the pointer is over, if any.
    ///
    /// Its own signal rather than derived on read: a target highlights itself
    /// when this names it, and deriving would mean every target recomputing the
    /// answer on every frame of a drag that is nowhere near it.
    over: Signal<Option<DragTargetId>>,
}

/// The parts a clone must share rather than copy.
struct Inner {
    session: Option<Session>,
    /// Rebuilt every frame by the targets themselves, so a target that moved,
    /// resized or went away is simply not in the next list.
    targets: Vec<Target>,
}

impl DragController {
    #[must_use]
    pub fn new(runtime: &Runtime) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                session: None,
                targets: Vec::new(),
            })),
            position: runtime.signal(None),
            over: runtime.signal(None),
        }
    }

    /// Declare where a target is, and what it takes.
    ///
    /// Called once per frame per target, from a `Measured` handler. Registering
    /// the same id twice replaces the earlier rectangle rather than adding a
    /// second, so a target that moves does not leave a ghost behind.
    pub fn register<T: 'static>(&self, id: DragTargetId, rect: Rect) {
        self.register_kind(id, rect, TypeId::of::<T>());
    }

    /// [`register`](Self::register) with the type already erased.
    ///
    /// What `DragSession::register_target` calls. The widget layer cannot name
    /// the application's payload type, so it passes the `TypeId` the target was
    /// built with instead.
    pub fn register_kind(&self, id: DragTargetId, rect: Rect, accepts: TypeId) {
        let entry = Target { id, rect, accepts };
        let mut inner = self.inner.borrow_mut();
        let targets = &mut inner.targets;
        match targets.iter_mut().find(|target| target.id == id) {
            Some(existing) => *existing = entry,
            None => targets.push(entry),
        }
    }

    /// [`start`](Self::start) with the payload already boxed.
    ///
    /// The kind is read back off the value rather than passed alongside it:
    /// `Rc<dyn Any>` knows its own concrete type, and a caller that supplied
    /// both could get them out of step — which would silently make every target
    /// refuse the drop.
    pub fn begin_erased(&self, data: Rc<dyn Any>, at: Offset) {
        let kind = (*data).type_id();
        self.inner.borrow_mut().session = Some(Session { data, kind });
        self.position.set(Some(at));
        self.over.set(self.resolve(at));
    }

    /// Forget a target — it has left the tree.
    pub fn unregister(&self, id: DragTargetId) {
        self.inner
            .borrow_mut()
            .targets
            .retain(|target| target.id != id);
    }

    /// Begin carrying `data` from `at`.
    ///
    /// Starting a drag while one is already running replaces it. That is not a
    /// state a pointer can reach — a second finger is a separate pointer id and
    /// the gesture arena will not hand both to a drag — but a *program* can, and
    /// silently keeping the first would strand it with no way to release.
    pub fn start<T: 'static>(&self, data: T, at: Offset) {
        self.begin_erased(Rc::new(data), at);
    }

    /// The pointer has moved.
    ///
    /// Does nothing when no drag is running, rather than starting one: a move
    /// without a press is not a drag, and inventing a session here would make
    /// the controller disagree with the arena about whether one exists.
    pub fn update(&self, at: Offset) {
        if self.inner.borrow().session.is_none() {
            return;
        }
        self.position.set(Some(at));
        self.over.set(self.resolve(at));
    }

    /// Release, and report where it landed.
    ///
    /// `None` when nothing was being dragged, or when it was let go somewhere no
    /// target would take it — which is the ordinary way a drag is abandoned, not
    /// an error.
    ///
    /// The session ends either way. A drag that failed to land still has to stop
    /// following the finger.
    pub fn finish(&self) -> Option<Dropped> {
        let session = self.inner.borrow_mut().session.take();
        let over = self.over.peek();
        self.position.set(None);
        self.over.set(None);

        let session = session?;
        Some(Dropped {
            data: session.data,
            target: over?,
        })
    }

    /// Give up without delivering anything.
    pub fn cancel(&self) {
        self.inner.borrow_mut().session = None;
        self.position.set(None);
        self.over.set(None);
    }

    /// `true` while something is being carried.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.inner.borrow().session.is_some()
    }

    /// Where the pointer is. Reading this in a build follows the drag.
    #[must_use]
    pub fn position(&self) -> Option<Offset> {
        self.position.get()
    }

    /// Which target the pointer is over. Reading this in a build highlights it.
    #[must_use]
    pub fn over(&self) -> Option<DragTargetId> {
        self.over.get()
    }

    /// The signal behind [`position`](Self::position), for a widget that wants
    /// to subscribe without reading.
    #[must_use]
    pub fn position_signal(&self) -> Signal<Option<Offset>> {
        self.position.clone()
    }

    /// The signal behind [`over`](Self::over).
    #[must_use]
    pub fn over_signal(&self) -> Signal<Option<DragTargetId>> {
        self.over.clone()
    }

    /// Which registered target is under `at` and will take what is being carried.
    ///
    /// **Last match wins**, because targets register in build order and a later
    /// one is drawn over an earlier one. Overlapping targets are unusual, and
    /// the alternative — first match — would hand the drop to the thing
    /// underneath, which is never what the eye expects.
    fn resolve(&self, at: Offset) -> Option<DragTargetId> {
        let inner = self.inner.borrow();
        let kind = inner.session.as_ref()?.kind;
        inner
            .targets
            .iter()
            .rev()
            .find(|target| target.accepts == kind && target.rect.contains(at))
            .map(|target| target.id)
    }
}

impl fmt::Debug for DragController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragController")
            .field("dragging", &self.is_dragging())
            .field("targets", &self.inner.borrow().targets.len())
            .field("over", &self.over.peek())
            .finish_non_exhaustive()
    }
}

/// The widget layer's view of a session.
///
/// Implemented here rather than there because `vieww-widget` sits **below**
/// `vieww-element` and may not depend on it — the same direction rule that puts
/// `RenderFactory` in the render crate instead of on the widgets. An
/// application never names this trait: it hands a `DragController` to
/// `Draggable` and `DragTarget`, and the compiler finds this impl.
impl vieww_widget::DragSession for DragController {
    fn register_target(&self, id: u64, rect: Rect, accepts: TypeId) {
        self.register_kind(DragTargetId(id), rect, accepts);
    }

    fn begin(&self, data: Rc<dyn Any>, at: Offset) {
        self.begin_erased(data, at);
    }

    fn moved(&self, at: Offset) {
        self.update(at);
    }

    /// **The payload is deliberately not returned here.**
    ///
    /// `Draggable` already owns what it is carrying — the application built it
    /// with that value — so handing the same thing back would be a second copy
    /// of something the caller has. What it cannot know is *where the drag
    /// ended*, and that is what this answers. An application that wants the
    /// payload at the target instead calls [`DragController::finish`] directly.
    fn released(&self) -> Option<u64> {
        self.finish().map(|dropped| dropped.target.0)
    }

    fn hovering(&self) -> Option<u64> {
        self.over().map(|id| id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Runtime, DragController) {
        let runtime = Runtime::new();
        let drag = DragController::new(&runtime);
        (runtime, drag)
    }

    fn square(x: f32, y: f32) -> Rect {
        Rect::new(x, y, x + 100.0, y + 100.0)
    }

    #[test]
    fn nothing_is_being_dragged_to_begin_with() {
        let (_runtime, drag) = setup();

        assert!(!drag.is_dragging());
        assert_eq!(drag.position(), None);
        assert_eq!(drag.over(), None);
        assert!(drag.finish().is_none(), "nothing to deliver");
    }

    #[test]
    fn a_drag_released_over_a_target_delivers_its_payload() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("a file"), Offset::new(500.0, 500.0));
        assert!(drag.is_dragging());
        drag.update(Offset::new(50.0, 50.0));

        assert_eq!(drag.over(), Some(DragTargetId(1)));
        let dropped = drag.finish().expect("released over the target");
        assert_eq!(dropped.target, DragTargetId(1));
        assert_eq!(
            dropped.take::<String>().as_deref(),
            Some(&String::from("a file"))
        );
        assert!(!drag.is_dragging(), "the session is over either way");
    }

    #[test]
    fn a_drag_released_over_nothing_is_abandoned_rather_than_failing() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("a file"), Offset::new(500.0, 500.0));
        assert_eq!(drag.over(), None);

        assert!(drag.finish().is_none(), "it landed nowhere");
        assert!(
            !drag.is_dragging(),
            "and still stopped following the finger"
        );
        assert_eq!(drag.position(), None);
    }

    /// The check that keeps highlighting and accepting from disagreeing.
    #[test]
    fn a_target_does_not_light_up_for_a_payload_it_cannot_take() {
        let (_runtime, drag) = setup();
        drag.register::<u32>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("a file"), Offset::new(50.0, 50.0));

        assert_eq!(
            drag.over(),
            None,
            "it takes numbers, and this drag is carrying text"
        );
        assert!(drag.finish().is_none());
    }

    #[test]
    fn the_target_drawn_on_top_takes_the_drop() {
        let (_runtime, drag) = setup();
        // Registered in build order, so the second is painted over the first.
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));
        drag.register::<String>(DragTargetId(2), square(50.0, 50.0));

        drag.start(String::from("x"), Offset::new(60.0, 60.0));

        assert_eq!(
            drag.over(),
            Some(DragTargetId(2)),
            "handing it to the one underneath is never what the eye expects"
        );
    }

    #[test]
    fn a_target_that_moves_does_not_leave_a_ghost_behind() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));
        drag.register::<String>(DragTargetId(1), square(200.0, 200.0));

        drag.start(String::from("x"), Offset::new(50.0, 50.0));
        assert_eq!(drag.over(), None, "it is not there any more");

        drag.update(Offset::new(250.0, 250.0));
        assert_eq!(drag.over(), Some(DragTargetId(1)), "it is here");
    }

    #[test]
    fn an_unregistered_target_stops_taking_drops() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));
        drag.unregister(DragTargetId(1));

        drag.start(String::from("x"), Offset::new(50.0, 50.0));

        assert_eq!(drag.over(), None);
    }

    #[test]
    fn moving_with_nothing_held_starts_nothing() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.update(Offset::new(50.0, 50.0));

        assert!(!drag.is_dragging());
        assert_eq!(
            drag.position(),
            None,
            "a move without a press is not a drag"
        );

        assert_eq!(drag.over(), None);
    }

    #[test]
    fn cancelling_delivers_nothing_and_ends_the_session() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("x"), Offset::new(50.0, 50.0));
        assert_eq!(drag.over(), Some(DragTargetId(1)));

        drag.cancel();

        assert!(!drag.is_dragging());
        assert_eq!(drag.over(), None);
        assert!(drag.finish().is_none(), "cancelling is not a drop");
    }

    /// Two drags cannot overlap through a pointer, but they can through a bug.
    #[test]
    fn starting_a_second_drag_replaces_the_first_rather_than_stranding_it() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("first"), Offset::new(500.0, 500.0));
        drag.start(String::from("second"), Offset::new(50.0, 50.0));

        let dropped = drag.finish().expect("the second one is live");
        assert_eq!(
            dropped.take::<String>().as_deref(),
            Some(&String::from("second"))
        );
    }

    #[test]
    fn the_wrong_type_comes_back_as_none_rather_than_panicking() {
        let (_runtime, drag) = setup();
        drag.register::<String>(DragTargetId(1), square(0.0, 0.0));

        drag.start(String::from("x"), Offset::new(50.0, 50.0));
        let dropped = drag.finish().expect("landed");

        assert!(dropped.take::<u32>().is_none());
        assert!(dropped.take::<String>().is_some());
    }
}
