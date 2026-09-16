use vieww_foundation::{Key, Rect};

use crate::{widget_node_from, Handler, Widget, WidgetKind, WidgetNode};

/// Reports where its child ended up on screen, and changes nothing else.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::Measured;
///
/// // A signal in a real application; a cell keeps the example runtime-free.
/// let anchor: Rc<Cell<Option<Rect>>> = Rc::new(Cell::new(None));
/// let write = Rc::clone(&anchor);
///
/// let button = Measured::new()
///     .on_measured(Rc::new(move |rect| write.set(Some(rect))))
///     .child(Button::new("Open"));
/// ```
///
/// The child is laid out, positioned and painted exactly as it would be without
/// this in the way. The only effect is that `on_measured` is called with the
/// child's rectangle in **global** coordinates, once, and again whenever it
/// moves or resizes.
///
/// # What it is for
///
/// Anchoring an overlay. A menu opens under a button and a tooltip beside the
/// thing it explains, but both are drawn above the whole tree — a popup clipped
/// by its parent's bounds is not a popup — so neither can be a child of the
/// control it belongs to, and ordinary layout cannot place them.
///
/// A widget cannot work its own position out: `build` runs before layout, and
/// layout hands a size *up* while position is assigned by the parent afterwards.
/// This is the way that answer gets back into the tree, through the same
/// signal-writing handler `Scrollable::on_extents` uses.
///
/// # It reports one frame late, on purpose
///
/// The rectangle exists for the first time during paint, which is after the
/// build that would have read it — so a menu opened against a *moving* anchor
/// trails it by a frame. That is invisible for a popup and it is the safe
/// direction: reporting from layout instead would put a signal write ahead of
/// the build, which is the one thing in this framework that can fail to settle.
/// See `RenderMeasured`.
#[derive(Clone)]
pub struct Measured {
    on_measured: Option<Handler<Rect>>,
    child: Option<WidgetNode>,
    key: Option<Key>,
}

impl Measured {
    #[must_use]
    pub fn new() -> Self {
        Self {
            on_measured: None,
            child: None,
            key: None,
        }
    }

    /// Called with the child's global rectangle whenever it changes.
    ///
    /// **Only when it changes.** A handler here writes a signal, and a signal
    /// written every frame marks its readers pending every frame — a rebuild that
    /// never settles and looks exactly like a runaway animation. The render
    /// object holds the guard; a caller does not have to.
    #[must_use]
    pub fn on_measured(mut self, handler: Handler<Rect>) -> Self {
        self.on_measured = Some(handler);
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

    /// The handler, for the render layer.
    #[must_use]
    pub fn measured_handler(&self) -> Option<Handler<Rect>> {
        self.on_measured.clone()
    }
}

impl Default for Measured {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Measured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Measured")
            .field("listening", &self.on_measured.is_some())
            .finish_non_exhaustive()
    }
}

impl Widget for Measured {
    fn debug_name(&self) -> &'static str {
        "Measured"
    }

    fn kind(&self) -> WidgetKind<'_> {
        match &self.child {
            Some(child) => WidgetKind::RenderSingleChild(child),
            None => WidgetKind::RenderLeaf,
        }
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }
}

widget_node_from!(Measured);
