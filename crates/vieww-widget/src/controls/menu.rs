use std::fmt;
use std::rc::Rc;

use vieww_foundation::{
    Alignment, BoxDecoration, Constraints, EdgeInsets, IconData, Key, Rect, Size, ViewMetrics,
};

use crate::{
    children, widget_node_from, Align, BuildContext, Constrained, DecoratedBox, Flex,
    GestureDetector, Handler, Icon, MainAxisSize, ModalBarrier, Padding, SemanticRole, Semantics,
    SizedBox, Stack, StackFit, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// How wide a menu is allowed to get before its labels wrap.
///
/// Menus are lists of short commands. One that grew to the width of a phone
/// would put its label at one edge of the screen and nothing at the other.
pub const MENU_MAX_WIDTH: f32 = 280.0;

/// How narrow a menu is allowed to get.
///
/// The width defaults to the anchor's, and an anchor nobody set is empty — so
/// without a floor a menu opened without one is **zero pixels wide**. It does
/// not fail visibly: the panel is a degenerate rounded rectangle nobody can see
/// and the labels, which nothing clips, paint straight onto the page behind it.
/// That is what `examples/controls` showed the first time its menu was opened,
/// and it read as a menu with no background rather than as a menu with no width.
pub const MENU_MIN_WIDTH: f32 = 112.0;

/// One line of a [`Menu`].
///
/// A value, not a widget: the menu owns the layout so that every row is the same
/// height and the same shape. Handing it arbitrary widgets would make a menu
/// whose rows disagree about where their text sits, which is the thing a menu
/// is for.
#[derive(Debug, Clone)]
pub struct MenuItem {
    label: String,
    icon: Option<IconData>,
    enabled: bool,
}

impl MenuItem {
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            enabled: true,
        }
    }

    /// A leading icon. Optional, and if *any* item has one the whole menu
    /// reserves the space — see [`Menu`]'s build.
    #[must_use]
    pub fn icon(mut self, icon: IconData) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A row that is visible and does nothing.
    ///
    /// Present rather than absent on purpose: a command that disappears when it
    /// is unavailable teaches nobody that it exists, and moves every row beneath
    /// it under the finger already travelling towards one.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub fn item_label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// A list of commands, opened against the control it belongs to.
///
/// ```
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Menu, MenuItem};
///
/// let menu = Menu::new(vec![MenuItem::new("Rename"), MenuItem::new("Delete")])
///     .anchor(Rect::new(20.0, 40.0, 140.0, 84.0))
///     .on_selected(Rc::new(|index| println!("chose {index}")));
/// ```
///
/// # It is positioned, not aligned
///
/// Everything else that floats in this framework — [`Dialog`](crate::Dialog),
/// [`BottomSheet`](crate::BottomSheet) — sits at an [`Alignment`] of the whole
/// surface, because a dialog belongs to the screen. A menu belongs to a
/// *control*, so it needs that control's rectangle, and there is no `Positioned`
/// widget here to place it with. `Align::TOP_LEFT` plus a computed `Padding` is
/// how an absolute offset is expressed in this layout model.
///
/// The rectangle comes from [`Measured`](crate::Measured), which is the only way
/// a widget can learn where it ended up.
///
/// # It flips rather than overflowing
///
/// A menu opened near the bottom of the screen would otherwise run off it, and a
/// menu near the right edge would run off that. So it opens **upward** when
/// there is not room below and is **pulled back** when it would pass the right
/// edge — the two rules a menu needs to be usable at the corner of a screen,
/// which is exactly where overflow menus live.
///
/// The surface it measures itself against comes from the ambient
/// [`ViewMetrics`], the same value `SafeArea` reads. Without one — a bare test
/// tree — it places the menu below and to the left and does not flip, which is
/// correct for an unbounded surface.
///
/// # The barrier is not optional
///
/// A menu with no scrim is one that stays open while the finger presses the
/// thing behind it. [`on_dismiss`](Self::on_dismiss) decides whether tapping
/// outside *closes* it; the tap is absorbed either way.
pub struct Menu {
    items: Vec<MenuItem>,
    anchor: Rect,
    on_selected: Option<Handler<usize>>,
    on_dismiss: Option<Rc<dyn Fn()>>,
    width: Option<f32>,
    key: Option<Key>,
}

impl Menu {
    #[must_use]
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            anchor: Rect::ZERO,
            on_selected: None,
            on_dismiss: None,
            width: None,
            key: None,
        }
    }

    /// The control this menu belongs to, in global coordinates.
    #[must_use]
    pub const fn anchor(mut self, anchor: Rect) -> Self {
        self.anchor = anchor;
        self
    }

    /// Called with the index of the item pressed.
    ///
    /// The index rather than the item, because the caller already owns the list
    /// and an owned copy of a row it wrote is not news.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<usize>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Called when a tap lands outside the menu.
    #[must_use]
    pub fn on_dismiss(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Force a width. By default the menu is as wide as its anchor, clamped
    /// into a readable range.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// How tall the menu will be, which placement has to know before layout has
    /// run.
    ///
    /// Sound because the menu owns its own row height: every item is exactly
    /// `touch_target` tall by construction, which is the reason [`MenuItem`] is
    /// a value and not a widget. A menu built from arbitrary children could not
    /// answer this, and could not flip.
    fn height(&self, theme: &ThemeData) -> f32 {
        let rows = self.items.len() as f32 * theme.metrics.touch_target;
        rows + theme.metrics.gap
    }

    fn width_for(&self) -> f32 {
        self.width
            .unwrap_or_else(|| self.anchor.width())
            .clamp(MENU_MIN_WIDTH, MENU_MAX_WIDTH)
    }

    /// Where the menu's top-left goes, given how much room there is.
    ///
    /// Split out so the two flip rules are testable without a render tree; see
    /// this module's tests.
    fn origin(&self, theme: &ThemeData, surface: Option<Size>) -> (f32, f32) {
        let below = self.anchor.bottom;
        let Some(surface) = surface else {
            // No ambient metrics: an unbounded surface, so nothing to flip
            // against and below-left is the honest placement.
            return (self.anchor.left, below);
        };

        let height = self.height(theme);
        // Upward when there is not room below *and* there is more room above.
        // The second half matters: on a short surface neither side fits, and
        // flipping to the worse one would be strictly worse.
        let above = self.anchor.top - height;
        let top = if below + height > surface.height && above >= 0.0 {
            above
        } else {
            below
        };

        // Pulled back from the right edge rather than flipped across the anchor:
        // a menu whose right edge lines up with its control still reads as
        // belonging to it, where one thrown to the far side does not.
        let width = self.width_for();
        let left = self
            .anchor
            .left
            .min((surface.width - width).max(0.0))
            .max(0.0);

        (left, top)
    }
}

impl Widget for Menu {
    fn debug_name(&self) -> &'static str {
        "Menu"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);
        let gap = theme.metrics.gap;
        let surface = ctx.inherit::<ViewMetrics>().map(|metrics| metrics.size);

        // Reserved for the whole menu when any single row asks for it, so that
        // labels line up in a column instead of stepping in and out around the
        // rows that happen to have an icon.
        let leading = self.items.iter().any(|item| item.icon.is_some());

        let mut column = Flex::column().main_axis_size(MainAxisSize::Min);
        for (index, item) in self.items.iter().enumerate() {
            column = column.push(self.row(&theme, index, item, leading));
        }

        // **`surface_variant`, not `surface`.** A menu is an overlay drawn over
        // a page, and the page is `surface` — so a panel in the same colour has
        // no edge at all: on the dark scheme the items floated over the content
        // underneath them with nothing between, which is what
        // `examples/controls` showed the first time anybody opened the menu.
        // Unlike a dialog or a drawer this has no scrim to separate it either,
        // deliberately — see `ModalBarrier` — so the colour is the only thing
        // left to do the job. `surface_variant` is the scheme's own word for
        // "the same background, raised".
        let panel = DecoratedBox::new(
            BoxDecoration::filled(theme.colors.surface_variant).radius(theme.metrics.corner),
        )
        .child(Padding::new(EdgeInsets::symmetric(0.0, gap / 2.0)).child(column));

        let width = self.width_for();
        let sized =
            Constrained::new(Constraints::new(width, width, 0.0, f32::INFINITY)).child(panel);

        let (left, top) = self.origin(&theme, surface);

        let barrier: WidgetNode = match &self.on_dismiss {
            Some(handler) => {
                let handler = Rc::clone(handler);
                ModalBarrier::new().on_dismiss(move || handler()).into()
            }
            None => ModalBarrier::new().into(),
        };

        // A container rather than a merging annotation, for the reason `Dialog`
        // gives: a menu that spoke for its subtree would announce itself and
        // swallow every command inside it.
        Semantics::container("Menu")
            .role(SemanticRole::Group)
            .child(Stack::new().fit(StackFit::Expand).children(children![
                barrier,
                Align::new(Alignment::TOP_LEFT)
                    .child(Padding::new(EdgeInsets::only(left, top, 0.0, 0.0)).child(sized)),
            ]))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("items", self.items.len().to_string()),
            ("anchor", format!("{:?}", self.anchor)),
            ("dismissible", self.on_dismiss.is_some().to_string()),
        ]
    }
}

impl Menu {
    /// One row: an optional icon, a label, and the whole width as a tap target.
    fn row(&self, theme: &ThemeData, index: usize, item: &MenuItem, leading: bool) -> WidgetNode {
        let gap = theme.metrics.gap;
        let colour = if item.enabled {
            theme.colors.on_surface
        } else {
            theme.colors.on_surface_variant
        };

        let mut row = Flex::row();
        if leading {
            // The slot exists on every row once any row wants it, so the labels
            // form a column. An empty box rather than a transparent icon: there
            // is nothing to draw, and a screen reader should not meet one.
            row = row.push(
                // **Width only.** Giving the slot a height of `gap` squashed
                // every leading icon to eight pixels tall — visible as a smudge
                // beside the label rather than as an icon. The column alignment
                // this exists for is a width question; the height is the icon's
                // own business.
                SizedBox::width(theme.metrics.touch_target).child(match &item.icon {
                    Some(icon) => WidgetNode::from(Icon::new(icon.clone()).color(colour)),
                    None => SizedBox::shrink().into(),
                }),
            );
        }
        row = row.push(Text::new(item.label.clone()).style(theme.text.body.color(colour)));

        // The *row* is the target, not the text: a menu where the gap beside a
        // short label does nothing is one that feels broken on a phone.
        let target = Constrained::new(Constraints::new(
            0.0,
            f32::INFINITY,
            theme.metrics.touch_target,
            f32::INFINITY,
        ))
        .child(Padding::new(EdgeInsets::symmetric(gap, 0.0)).child(row));

        let node: WidgetNode = match (&self.on_selected, item.enabled) {
            (Some(handler), true) => {
                let handler = Rc::clone(handler);
                GestureDetector::new()
                    .on_tap(move |_| handler(index))
                    .child(target)
                    .into()
            }
            // Disabled, or nobody listening: still laid out, still read out,
            // and inert. A row that silently vanished from the hit test while
            // looking identical would be the worse failure.
            _ => target.into(),
        };

        Semantics::button(item.label.clone())
            .enabled(item.enabled)
            .child(node)
            .into()
    }
}

impl fmt::Debug for Menu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Menu")
            .field("items", &self.items.len())
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Menu);

#[cfg(test)]
mod tests {
    use crate::{inflate, Theme};

    use super::*;

    /// Themed, because `build` reads `ThemeData::of` for every metric it uses.
    fn built(widget: impl Into<WidgetNode>) -> crate::DebugNode {
        inflate(Theme::new(ThemeData::light()).child(widget.into()))
    }

    /// A menu of `count` items anchored at `(left, top)`, 120 wide and 40 tall.
    fn menu(count: usize, left: f32, top: f32) -> Menu {
        let items = (0..count)
            .map(|i| MenuItem::new(format!("Item {i}")))
            .collect();
        Menu::new(items).anchor(Rect::new(left, top, left + 120.0, top + 40.0))
    }

    fn theme() -> ThemeData {
        ThemeData::light()
    }

    #[test]
    fn a_menu_with_no_anchor_is_still_wide_enough_to_see() {
        // The width defaults to the anchor's, and `Menu::new` starts with
        // `Rect::ZERO` — so an unanchored menu used to be zero pixels wide. The
        // panel then drew as a degenerate rounded rectangle nobody could see
        // while the labels, which nothing clips, painted onto the page behind.
        // Watched happening in `examples/controls`.
        let unanchored = Menu::new(vec![MenuItem::new("Cut"), MenuItem::new("Copy")]);
        assert_eq!(unanchored.width_for(), MENU_MIN_WIDTH);
    }

    #[test]
    fn a_menu_panel_is_a_different_colour_from_the_page_it_covers() {
        // Found by opening the menu in `examples/controls` and looking at it.
        // The panel was `surface`, the page is `surface`, and a menu has no
        // scrim — so the items floated over the content underneath with no edge
        // anywhere, reading as text drawn twice rather than as a menu.
        let scheme = ThemeData::dark();
        let node = inflate(Theme::new(scheme).child(WidgetNode::from(menu(3, 20.0, 100.0))));

        let panel = node
            .find("DecoratedBox")
            .expect("the menu draws its panel with one");
        assert_eq!(
            panel.property("color").map(str::to_owned),
            Some(scheme.colors.surface_variant.to_string()),
            "the panel has to differ from the page's own {}",
            scheme.colors.surface
        );
    }

    #[test]
    fn a_menu_opens_below_its_anchor() {
        let theme = theme();
        let (left, top) = menu(3, 20.0, 100.0).origin(&theme, Some(Size::new(400.0, 800.0)));

        assert_eq!(left, 20.0, "aligned with the control it belongs to");
        assert_eq!(top, 140.0, "immediately under it");
    }

    #[test]
    fn a_menu_with_no_room_below_opens_upward() {
        let theme = theme();
        // Anchored near the bottom: three rows will not fit under it.
        let anchored = menu(3, 20.0, 700.0);
        let height = anchored.height(&theme);
        let (_, top) = anchored.origin(&theme, Some(Size::new(400.0, 800.0)));

        assert!(
            (top - (700.0 - height)).abs() < f32::EPSILON,
            "it should sit above the anchor, at {}, not {top}",
            700.0 - height
        );
    }

    /// Flipping is only an improvement when the other side actually fits.
    #[test]
    fn a_menu_too_tall_for_either_side_still_opens_downward() {
        let theme = theme();
        // Twenty rows fit neither above nor below on a short surface.
        let (_, top) = menu(20, 20.0, 100.0).origin(&theme, Some(Size::new(400.0, 300.0)));

        assert_eq!(
            top, 140.0,
            "flipping to a side with even less room would be strictly worse"
        );
    }

    #[test]
    fn a_menu_near_the_right_edge_is_pulled_back_onto_the_screen() {
        let theme = theme();
        // Anchored at 320 on a 400-wide surface; the menu is 120 wide.
        let (left, _) = menu(2, 320.0, 100.0).origin(&theme, Some(Size::new(400.0, 800.0)));

        assert_eq!(
            left, 280.0,
            "its right edge should land on the surface edge"
        );
    }

    /// A menu with no room for its own width sits at the left edge, not a
    /// negative offset.
    ///
    /// **The surface has to be narrow for this to be reachable at all.**
    /// `width_for` clamps to `MENU_MAX_WIDTH` *before* placement, so asking for
    /// a 600-wide menu on a 400-wide screen gets a 280-wide one that fits with
    /// room to spare — which is what the first version of this test actually
    /// measured, and why it failed against correct code.
    #[test]
    fn a_menu_wider_than_the_surface_is_pinned_to_the_left_edge() {
        let theme = theme();
        let wide = Menu::new(vec![MenuItem::new("Only")])
            .anchor(Rect::new(50.0, 100.0, 170.0, 140.0))
            .width(600.0);
        // Narrower than `MENU_MAX_WIDTH`, so the clamped menu genuinely does
        // not fit however it is placed.
        let (left, _) = wide.origin(&theme, Some(Size::new(200.0, 800.0)));

        assert_eq!(
            left, 0.0,
            "never a negative offset, however little room there is"
        );
    }

    /// The clamp is what made the test above need a narrow surface, so it is
    /// worth pinning on its own rather than inferring it from a placement.
    #[test]
    fn a_menu_never_grows_past_a_readable_width() {
        let wide = Menu::new(vec![MenuItem::new("Only")]).width(600.0);
        assert_eq!(wide.width_for(), MENU_MAX_WIDTH);

        let narrow = Menu::new(vec![MenuItem::new("Only")]).width(120.0);
        assert_eq!(narrow.width_for(), 120.0, "a small menu is left alone");
    }

    /// Without ambient metrics there is no surface to flip against.
    #[test]
    fn a_menu_with_no_view_metrics_opens_below_and_does_not_flip() {
        let theme = theme();
        let (left, top) = menu(20, 20.0, 700.0).origin(&theme, None);

        assert_eq!((left, top), (20.0, 740.0));
    }

    #[test]
    fn every_item_is_a_button_a_screen_reader_can_find() {
        let tree = built(menu(2, 0.0, 0.0).on_selected(Rc::new(|_| {})));

        let nodes = tree.find_all("Semantics");
        let labels: Vec<&str> = nodes
            .iter()
            .filter_map(|node| node.property("label"))
            .collect();

        assert!(labels.contains(&"Item 0"), "{labels:?}");
        assert!(labels.contains(&"Item 1"), "{labels:?}");
    }

    /// A disabled row stays in the tree, and stays readable.
    #[test]
    fn a_disabled_item_is_still_present() {
        let items = vec![
            MenuItem::new("Rename"),
            MenuItem::new("Delete").enabled(false),
        ];
        let tree = built(Menu::new(items).on_selected(Rc::new(|_| {})));

        let nodes = tree.find_all("Semantics");
        let labels: Vec<&str> = nodes
            .iter()
            .filter_map(|node| node.property("label"))
            .collect();

        assert!(
            labels.contains(&"Delete"),
            "a command that vanishes when unavailable teaches nobody it exists: {labels:?}"
        );
    }
}
