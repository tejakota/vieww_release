use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Key, Rect};

use crate::{
    widget_node_from, BuildContext, GestureDetector, Handler, Measured, Widget, WidgetKind,
    WidgetNode,
};

/// Anything the drag widgets need from a controller.
///
/// # Why this is a trait and not `DragController`
///
/// Because `vieww-widget` sits **below** `vieww-element` — the widget layer is a
/// cheap description that knows nothing about elements, signals or the runtime,
/// and `docs/DESIGN.md` §7 forbids the dependency in that direction. The same
/// rule is why `RenderFactory` exists rather than
/// widgets constructing render objects.
///
/// `DragController` implements this from the crate above. An application never
/// names it: it passes the controller, and the compiler finds the impl.
pub trait DragSession {
    /// Note where a target is, and what it will take. Called every frame.
    fn register_target(&self, id: u64, rect: Rect, accepts: DragKind);
    /// Begin carrying whatever `data` identifies, from a global point.
    fn begin(&self, data: DragPayload, at: vieww_foundation::Offset);
    /// The pointer moved.
    fn moved(&self, at: vieww_foundation::Offset);
    /// Released. Returns the target it landed on, if any.
    fn released(&self) -> Option<u64>;
    /// Which target is under the pointer right now.
    fn hovering(&self) -> Option<u64>;
}

/// A payload on its way into a session, boxed because the widget layer cannot
/// name the application's type either.
pub type DragPayload = Rc<dyn std::any::Any>;

/// The type a target accepts, as an opaque token.
pub type DragKind = std::any::TypeId;

/// A control that can be picked up and carried.
///
/// ```
/// # use std::rc::Rc;
/// # use vieww_widget::prelude::*;
/// # use vieww_widget::{Draggable, DragSession};
/// # fn demo(session: Rc<dyn DragSession>) {
/// let row = Draggable::new(session, Rc::new(String::from("report.pdf")))
///     .child(Text::new("report.pdf"));
/// # }
/// ```
///
/// # It draws nothing while it is being dragged
///
/// No `feedback` widget, deliberately. The thing that follows the finger has to
/// be painted **above the whole tree**, and a widget cannot escape its own
/// subtree to do that — the same constraint that makes [`Menu`](crate::Menu) and
/// [`Tooltip`](crate::Tooltip) surfaces the application places rather than
/// self-contained popups.
///
/// So the application draws the feedback, from the position the session
/// publishes, in whatever overlay it already has. This widget's job is to start
/// the session, feed it the pointer, and end it.
pub struct Draggable {
    session: Rc<dyn DragSession>,
    data: DragPayload,
    child: Option<WidgetNode>,
    on_dropped: Option<Handler<Option<u64>>>,
    key: Option<Key>,
}

impl Draggable {
    #[must_use]
    pub fn new(session: Rc<dyn DragSession>, data: DragPayload) -> Self {
        Self {
            session,
            data,
            child: None,
            on_dropped: None,
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Called on release with the target it landed on, or `None` if it landed
    /// nowhere.
    ///
    /// Reported even when it lands nowhere, because "the drag ended" is what
    /// puts the feedback away, and a handler that only fired on success would
    /// leave it stuck to the finger after an abandoned drag.
    #[must_use]
    pub fn on_dropped(mut self, handler: Handler<Option<u64>>) -> Self {
        self.on_dropped = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Draggable {
    fn debug_name(&self) -> &'static str {
        "Draggable"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        let start = Rc::clone(&self.session);
        let data = Rc::clone(&self.data);
        let moving = Rc::clone(&self.session);
        let ending = Rc::clone(&self.session);
        let dropped = self.on_dropped.clone();

        let child: WidgetNode = self
            .child
            .clone()
            .unwrap_or_else(|| crate::SizedBox::shrink().into());

        GestureDetector::new()
            // `position` rather than `local`: a session resolves against target
            // rectangles which are themselves global, and mixing the two puts
            // the drop wherever the dragged widget happens to sit.
            .on_drag_start(move |details| start.begin(Rc::clone(&data), details.position))
            .on_drag_update(move |details| moving.moved(details.position))
            .on_drag_end(move |_| {
                let landed = ending.released();
                if let Some(handler) = &dropped {
                    handler(landed);
                }
            })
            .child(child)
            .into()
    }
}

impl fmt::Debug for Draggable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draggable").finish_non_exhaustive()
    }
}

widget_node_from!(Draggable);

/// A region that a [`Draggable`] can be released onto.
///
/// ```
/// # use std::rc::Rc;
/// # use vieww_widget::prelude::*;
/// # use vieww_widget::{DragTarget, DragSession};
/// # fn demo(session: Rc<dyn DragSession>) {
/// let bin = DragTarget::new(session, 1, std::any::TypeId::of::<String>())
///     .builder(Rc::new(|hovering| {
///         Text::new(if hovering { "Release to file" } else { "Drop here" }).into()
///     }));
/// # }
/// ```
///
/// # It measures itself rather than being hit tested
///
/// During a drag the thing under the pointer is the *feedback* travelling with
/// the finger, so a hit test answers the wrong question. Instead the target
/// reports its rectangle every frame through [`Measured`](crate::Measured) and
/// the session resolves against those — see `DragController`.
///
/// The consequence worth knowing: a target is droppable from the frame *after*
/// it is first painted, because that is when its rectangle exists. For a target
/// that was on screen before the drag began — which is all of them, since
/// something had to be dragged onto it — that is never observable.
pub struct DragTarget {
    session: Rc<dyn DragSession>,
    id: u64,
    accepts: DragKind,
    builder: Option<Rc<dyn Fn(bool) -> WidgetNode>>,
    key: Option<Key>,
}

impl DragTarget {
    /// A target that takes payloads of the type `accepts` names.
    #[must_use]
    pub fn new(session: Rc<dyn DragSession>, id: u64, accepts: DragKind) -> Self {
        Self {
            session,
            id,
            accepts,
            builder: None,
            key: None,
        }
    }

    /// Build the contents, told whether a matching drag is over it.
    ///
    /// A builder rather than a child, because a target that cannot show it is
    /// about to accept something is a target the user drops next to.
    #[must_use]
    pub fn builder(mut self, builder: Rc<dyn Fn(bool) -> WidgetNode>) -> Self {
        self.builder = Some(builder);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for DragTarget {
    fn debug_name(&self) -> &'static str {
        "DragTarget"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        // Read *before* the child is built, so the highlight and the rectangle
        // describe the same frame.
        let hovering = self.session.hovering() == Some(self.id);

        let contents = match &self.builder {
            Some(builder) => builder(hovering),
            None => crate::SizedBox::shrink().into(),
        };

        let session = Rc::clone(&self.session);
        let id = self.id;
        let accepts = self.accepts;

        Measured::new()
            .on_measured(Rc::new(move |rect| {
                session.register_target(id, rect, accepts);
            }))
            .child(contents)
            .into()
    }
}

impl fmt::Debug for DragTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragTarget")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

widget_node_from!(DragTarget);
