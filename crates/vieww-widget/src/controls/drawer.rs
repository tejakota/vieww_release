use std::fmt;
use std::rc::Rc;

use vieww_foundation::{BoxDecoration, EdgeInsets, Key};

use crate::{
    children, widget_node_from, BuildContext, DecoratedBox, LayoutBuilder, ModalBarrier, Padding,
    PositionedDirectional, Stack, StackFit, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The widest a drawer gets, however much room there is.
///
/// A navigation panel half a tablet wide is not a panel, it is a second screen
/// with the first one greyed out behind it. Every platform lands within a few
/// points of this.
pub const DRAWER_MAX_WIDTH: f32 = 304.0;

/// Which edge a [`Drawer`] comes in from.
///
/// **Directional, and deliberately with no physical alternative.** Everywhere
/// else in this framework a caller opts *in* to reading-order behaviour —
/// [`Padding`] is physical, [`PaddingDirectional`](crate::PaddingDirectional) is
/// the opt-in — and this is the second exception after [`Flex`](crate::Flex),
/// which earns it the same way: navigation lives where reading *begins*, so the
/// edge is a property of the reading order rather than a named side somebody
/// chose. A `Drawer` pinned to the left in Arabic would be a bug in every
/// application that ever used it, which makes an option to do it a trap rather
/// than a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DrawerSide {
    /// The edge reading begins at — left in Latin, right in Arabic.
    #[default]
    Start,
    /// The edge reading finishes at. The second drawer, where an application
    /// has one: filters, details, a cart.
    End,
}

/// A panel that comes in from the side, over a scrim.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Drawer;
///
/// let nav = Drawer::new()
///     .child(Flex::column().children(children![
///         Text::new("Inbox"),
///         Text::new("Archive"),
///     ]))
///     .on_dismiss(|| {});
/// ```
///
/// Push it as a **modal route** — `Route::modal` — for the reasons
/// [`Dialog`](crate::Dialog) gives: it covers the surface, its scrim belongs
/// above everything, and back should close it rather than leave the screen.
///
/// # It always leaves scrim to tap
///
/// The panel is [`DRAWER_MAX_WIDTH`] wide, **or the room available minus one
/// touch target, whichever is less**. That second clause is not cosmetic. The
/// scrim is the dismiss affordance, so a drawer that covered the full width of a
/// narrow screen would be a panel with no way out but the back gesture — and on
/// desktop, no way out at all. Tying the gap to
/// [`Metrics::touch_target`](crate::Metrics) rather than to a number of points
/// says why it exists: what is left has to be big enough for a finger.
///
/// This is why it is built through a [`LayoutBuilder`] rather than a
/// [`Constrained`](crate::Constrained). A maximum width can be expressed with
/// constraints; *this* width cannot, because it depends on how much room there
/// is, and a `Constrained` inside a tight parent is overruled by it.
///
/// # Square corners, on purpose
///
/// A full-height panel flush against an edge is square there. The decoration
/// here carries **one** radius for all four corners — see
/// [`BottomSheet`](crate::BottomSheet), which works around the same limit — so
/// rounding the two edges that show would also round the two lying along the
/// screen edge, and the background would come through at the top and bottom
/// corners. A rounded trailing edge needs per-corner radii; until those exist,
/// square is the honest shape rather than a nearly-right one.
#[derive(Clone)]
pub struct Drawer {
    child: Option<WidgetNode>,
    side: DrawerSide,
    on_dismiss: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Default for Drawer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drawer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            child: None,
            side: DrawerSide::Start,
            on_dismiss: None,
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

    /// Which edge it comes in from. [`DrawerSide::Start`] unless said otherwise.
    #[must_use]
    pub const fn side(mut self, side: DrawerSide) -> Self {
        self.side = side;
        self
    }

    /// Called when the scrim is tapped.
    ///
    /// Without one the drawer is not dismissible by tapping outside it, which is
    /// a legitimate choice for a panel the application closes itself — and the
    /// same choice [`Dialog`](crate::Dialog) offers.
    #[must_use]
    pub fn on_dismiss(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The panel width within `available`, before any of it is drawn.
    ///
    /// Split out because it is the whole of the interesting arithmetic and the
    /// one part worth testing without a screen. `available` may be infinite —
    /// an unbounded parent — and the answer is then simply the maximum.
    #[must_use]
    pub fn width_within(available: f32, touch_target: f32) -> f32 {
        DRAWER_MAX_WIDTH.min(available - touch_target).max(0.0)
    }
}

impl Widget for Drawer {
    fn debug_name(&self) -> &'static str {
        "Drawer"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![("side", format!("{:?}", self.side))]
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        // Read out here rather than inside the closure: the builder must be
        // `'static`, and these are plain `Copy` values where `ThemeData` is not.
        let surface = theme.colors.surface;
        let gap = theme.metrics.gap;
        let touch_target = theme.metrics.touch_target;

        let child = self.child.clone();
        let side = self.side;
        let on_dismiss = self.on_dismiss.clone();

        LayoutBuilder::new(move |constraints| {
            let width = Self::width_within(constraints.max_width, touch_target);

            // A childless drawer is an empty panel rather than a panic — the
            // same answer `Padding` itself gives, since it is a render leaf with
            // no child.
            let inset = EdgeInsets::all(gap * 2.0);
            let content = match &child {
                Some(child) => Padding::new(inset).child(child.clone()),
                None => Padding::new(inset),
            };
            let panel = DecoratedBox::new(BoxDecoration::filled(surface)).child(content);

            // `top` and `bottom` together make it full height; `width` sizes the
            // other axis, because only one horizontal edge is pinned. Resolved
            // against the reading direction, which is the whole point of the
            // side being start/end rather than left/right.
            let pinned = PositionedDirectional::new()
                .top(0.0)
                .bottom(0.0)
                .width(width);
            let pinned = match side {
                DrawerSide::Start => pinned.start(0.0),
                DrawerSide::End => pinned.end(0.0),
            };

            let barrier: WidgetNode = match &on_dismiss {
                Some(handler) => {
                    let handler = Rc::clone(handler);
                    ModalBarrier::new().on_dismiss(move || handler()).into()
                }
                None => ModalBarrier::new().into(),
            };

            // A modal drawer traps focus, as `Dialog` and `BottomSheet` do:
            // Tab that walked out of it would land on the screen behind the
            // scrim. Around the panel rather than the whole stack, because the
            // barrier covers the screen.
            let trapped: WidgetNode = crate::FocusTrap::new(true)
                .child(pinned.child(panel))
                .into();

            Stack::new()
                .fit(StackFit::Expand)
                .children(children![barrier, trapped])
                .into()
        })
        .into()
    }
}

impl fmt::Debug for Drawer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Drawer")
            .field("side", &self.side)
            .field("dismissible", &self.on_dismiss.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Drawer);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{debug_tree, Text};

    /// The default touch target, so the arithmetic below reads against the
    /// number an application actually gets.
    const TOUCH: f32 = 48.0;

    #[test]
    fn a_wide_screen_gets_the_maximum_and_no_more() {
        assert_eq!(Drawer::width_within(1200.0, TOUCH), DRAWER_MAX_WIDTH);
    }

    #[test]
    fn a_narrow_screen_gives_up_width_to_keep_the_scrim_tappable() {
        // **The claim worth having.** 320 is a small phone; 304 would leave 16
        // of scrim, which is a third of a finger. The panel yields instead.
        assert_eq!(Drawer::width_within(320.0, TOUCH), 272.0);
        assert_eq!(
            320.0 - Drawer::width_within(320.0, TOUCH),
            TOUCH,
            "and what is left is exactly one touch target, by construction"
        );
    }

    #[test]
    fn the_scrim_never_falls_below_a_touch_target_at_any_width() {
        // A property rather than an example: sweep the widths a real device
        // might report and check the invariant the whole rule exists for.
        for available in [200.0_f32, 240.0, 320.0, 360.0, 411.0, 600.0, 1024.0] {
            let scrim = available - Drawer::width_within(available, TOUCH);
            assert!(
                scrim >= TOUCH,
                "a {available}pt screen left {scrim}pt of scrim, which is not \
                 enough to tap"
            );
        }
    }

    #[test]
    fn a_screen_narrower_than_a_touch_target_collapses_rather_than_going_negative() {
        // Not a real device, but a negative width is a panic or a nonsense
        // layout rather than a small one, and `max(0.0)` is one character.
        assert_eq!(Drawer::width_within(30.0, TOUCH), 0.0);
        assert_eq!(Drawer::width_within(0.0, TOUCH), 0.0);
    }

    #[test]
    fn an_unbounded_parent_gets_the_maximum_rather_than_an_infinite_panel() {
        // `INFINITY - 48` is still infinity, so the `min` is what saves this.
        let width = Drawer::width_within(f32::INFINITY, TOUCH);
        assert_eq!(width, DRAWER_MAX_WIDTH);
        assert!(width.is_finite());
    }

    #[test]
    fn it_builds_a_barrier_and_a_directional_panel() {
        let dump = debug_tree(Drawer::new().child(Text::new("Inbox")).on_dismiss(|| {}));
        assert!(dump.contains("Drawer"), "{dump}");
        assert!(dump.contains("ModalBarrier"), "{dump}");
        assert!(
            dump.contains("PositionedDirectional"),
            "the panel is pinned against the reading direction, not a side: \
             {dump}"
        );
    }

    #[test]
    fn the_side_is_start_unless_it_is_asked_otherwise() {
        assert_eq!(Drawer::new().side, DrawerSide::Start);
        assert_eq!(Drawer::new().side(DrawerSide::End).side, DrawerSide::End);
        assert!(debug_tree(Drawer::new()).contains("side: Start"));
        assert!(debug_tree(Drawer::new().side(DrawerSide::End)).contains("side: End"));
    }
}
