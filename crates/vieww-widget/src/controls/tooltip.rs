use std::fmt;

use vieww_foundation::{
    Alignment, BoxDecoration, Constraints, EdgeInsets, Key, Rect, Size, ViewMetrics,
};

use crate::{
    children, widget_node_from, Align, BuildContext, Constrained, DecoratedBox, Padding,
    SemanticRole, Semantics, Stack, StackFit, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The widest a tooltip gets before its text wraps.
///
/// Narrower than a menu on purpose: a tooltip is a phrase explaining one
/// control, and one that runs the width of a tablet has stopped being a tooltip
/// and become a paragraph nobody asked for.
pub const TOOLTIP_MAX_WIDTH: f32 = 220.0;

/// How far a tooltip sits from the control it explains.
///
/// Enough that the two read as separate things, little enough that the eye does
/// not have to look for the connection.
pub const TOOLTIP_OFFSET: f32 = 8.0;

/// A short phrase explaining the control it points at.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::Tooltip;
///
/// let tip = Tooltip::new("Delete this file")
///     .anchor(Rect::new(40.0, 200.0, 88.0, 248.0));
/// ```
///
/// # Pair it with `Measured` and a long press
///
/// Like [`Menu`](crate::Menu), this is the *surface* — the caller owns whether
/// it is showing and what it is pointing at. That is the same shape every other
/// control here has: the application holds the state, the widget draws it.
///
/// The rectangle comes from [`Measured`](crate::Measured) wrapped around the
/// control being explained, and the trigger from a
/// [`GestureDetector`](crate::GestureDetector) on the same subtree —
/// `on_long_press` for a finger, `on_hover` for a pointer. Both are already
/// there; a tooltip needs no mechanism of its own.
///
/// # It has no barrier, and that is the difference from a menu
///
/// A [`Menu`](crate::Menu) puts a [`ModalBarrier`](crate::ModalBarrier) under
/// itself, because a menu is a question and the next tap is meant to answer it.
/// A tooltip is an aside. It must not eat the tap that dismisses it, and it must
/// not stop the button underneath being pressed while it is up — a tooltip that
/// swallowed the press of the control it was explaining would be worse than no
/// tooltip.
///
/// This is why the tooltip is drawn but takes no pointers: it is a `Text` in a
/// box, with no recogniser anywhere in it.
///
/// # It prefers to sit above
///
/// Because the thing below a control being pressed is usually a finger. Above is
/// the one direction reliably not covered by the hand that triggered it, so the
/// tooltip goes there unless there is no room, and drops below only then — the
/// reverse of [`Menu`](crate::Menu)'s preference, for the opposite reason.
///
/// Horizontally it is centred on its anchor and then pulled back inside the
/// surface, so a tooltip on a control at the edge of the screen stays readable.
pub struct Tooltip {
    message: String,
    anchor: Rect,
    key: Option<Key>,
}

impl Tooltip {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            anchor: Rect::ZERO,
            key: None,
        }
    }

    /// The control this tooltip explains, in global coordinates.
    #[must_use]
    pub const fn anchor(mut self, anchor: Rect) -> Self {
        self.anchor = anchor;
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// How tall the tooltip will be, which placement needs before layout runs.
    ///
    /// One line, which is what a tooltip should be — the width clamp above is
    /// what keeps it one. A wrapped tooltip is taller than this says and will
    /// overlap its anchor by the difference; that is a deliberate limit rather
    /// than an oversight, because measuring text here would mean shaping it, and
    /// the widget layer has no shaper. See `RenderText`.
    fn height(&self, theme: &ThemeData) -> f32 {
        let line = theme.text.label.size * theme.text.label.line_height;
        line + theme.metrics.gap
    }

    /// Where the tooltip's top-left goes.
    ///
    /// Split out so the placement rules are testable without a render tree.
    fn origin(&self, theme: &ThemeData, surface: Option<Size>, width: f32) -> (f32, f32) {
        let height = self.height(theme);
        let above = self.anchor.top - height - TOOLTIP_OFFSET;
        let below = self.anchor.bottom + TOOLTIP_OFFSET;
        // Centred on the control rather than aligned to its edge: a tooltip is
        // a label *for* the thing, and the eye pairs them by their centres.
        let centred = self.anchor.left + (self.anchor.width() - width) / 2.0;

        let Some(surface) = surface else {
            // No ambient metrics. Above is still the preference; there is simply
            // nothing to check it against.
            return (centred, above);
        };

        // Above unless it would run off the top, and only then below — which is
        // the opposite of a menu, because the space under a pressed control is
        // where the finger is.
        let top = if above >= 0.0 || below + height > surface.height {
            // Clamped, because `above` is negative exactly when the tooltip does
            // not fit there — and a negative offset does not mean "higher", it
            // means the top of the message is off the screen. Pinned to the top
            // edge it is at least readable, and still on the side away from the
            // finger.
            above.max(0.0)
        } else {
            below
        };

        let left = centred.min((surface.width - width).max(0.0)).max(0.0);
        (left, top)
    }
}

impl Widget for Tooltip {
    fn debug_name(&self) -> &'static str {
        "Tooltip"
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

        // Inverted against the screen rather than themed like a card: a tooltip
        // sits *over* content and has to be legible against whatever it lands
        // on, which a surface-coloured box on a surface-coloured screen is not.
        let panel = DecoratedBox::new(
            BoxDecoration::filled(theme.colors.on_surface).radius(theme.metrics.corner / 2.0),
        )
        .child(Padding::new(EdgeInsets::symmetric(gap, gap / 2.0)).child(
            Text::new(self.message.clone()).style(theme.text.label.color(theme.colors.surface)),
        ));

        let width = TOOLTIP_MAX_WIDTH;
        let sized = Constrained::new(Constraints::new(0.0, width, 0.0, f32::INFINITY)).child(panel);
        let (left, top) = self.origin(&theme, surface, width);

        // Bound out of the chain rather than written inline: a `children!`
        // invocation nested inside a builder call is something rustfmt does not
        // settle on — it asked for two different layouts on consecutive runs.
        let positioned = Align::new(Alignment::TOP_LEFT)
            .child(Padding::new(EdgeInsets::only(left, top, 0.0, 0.0)).child(sized));

        // A container carrying the message, so a screen reader reaching the
        // tooltip hears the explanation. It is *not* merged into the control it
        // explains: that control has its own label, and a screen reader saying
        // both in one breath is the announcement nobody can parse.
        //
        // No `Stack` barrier and no gesture anywhere: see the type docs.
        // `tooltip` rather than `Group`, and the role is what carries the
        // announcement: `docs/AIMS.md` §J asks that a transient widget arrive
        // announced rather than be made announceable later, and a tooltip that
        // appeared in silence was the same defect as a silent snackbar. Polite
        // rather than assertive — a tooltip is an explanation somebody went
        // looking for, not news that should cut across what they are doing.
        Semantics::container(self.message.clone())
            .role(SemanticRole::Custom("tooltip"))
            .live(crate::SemanticLiveness::Polite)
            .child(
                Stack::new()
                    .fit(StackFit::Expand)
                    .children(children![positioned]),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("message", self.message.clone()),
            ("anchor", format!("{:?}", self.anchor)),
        ]
    }
}

impl fmt::Debug for Tooltip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tooltip")
            .field("message", &self.message)
            .field("anchor", &self.anchor)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Tooltip);

#[cfg(test)]
mod tests {
    use crate::{inflate, Theme};

    use super::*;

    fn theme() -> ThemeData {
        ThemeData::light()
    }

    /// A tooltip on a 48x48 control at `(left, top)`.
    fn tip(left: f32, top: f32) -> Tooltip {
        Tooltip::new("Delete").anchor(Rect::new(left, top, left + 48.0, top + 48.0))
    }

    fn built(widget: impl Into<WidgetNode>) -> crate::DebugNode {
        inflate(Theme::new(ThemeData::light()).child(widget.into()))
    }

    #[test]
    fn a_tooltip_sits_above_the_control_it_explains() {
        let theme = theme();
        let anchored = tip(100.0, 400.0);
        let height = anchored.height(&theme);
        let (_, top) = anchored.origin(&theme, Some(Size::new(400.0, 800.0)), 220.0);

        assert!(
            (top - (400.0 - height - TOOLTIP_OFFSET)).abs() < f32::EPSILON,
            "above is where the finger is not: expected {}, got {top}",
            400.0 - height - TOOLTIP_OFFSET
        );
    }

    #[test]
    fn a_tooltip_with_no_room_above_drops_below() {
        let theme = theme();
        // Hard against the top of the screen: nothing fits above it.
        let anchored = tip(100.0, 0.0);
        let (_, top) = anchored.origin(&theme, Some(Size::new(400.0, 800.0)), 220.0);

        assert!(
            (top - (48.0 + TOOLTIP_OFFSET)).abs() < f32::EPSILON,
            "expected it under the control at {}, got {top}",
            48.0 + TOOLTIP_OFFSET
        );
    }

    /// Neither side fits: it stays above rather than dropping onto the finger.
    #[test]
    fn a_tooltip_that_fits_nowhere_stays_above() {
        let theme = theme();
        let anchored = tip(100.0, 0.0);
        // A surface so short that below overflows too.
        let (_, top) = anchored.origin(&theme, Some(Size::new(400.0, 60.0)), 220.0);

        assert_eq!(
            top, 0.0,
            "pinned to the top edge rather than pushed off the screen"
        );
    }

    #[test]
    fn a_tooltip_is_centred_on_its_anchor() {
        let theme = theme();
        // A 48-wide control at 200 on a wide surface, with a 100-wide tooltip:
        // centring puts it at 200 + (48 - 100) / 2 = 174.
        let (left, _) = tip(200.0, 400.0).origin(&theme, Some(Size::new(600.0, 800.0)), 100.0);

        assert_eq!(left, 174.0);
    }

    #[test]
    fn a_tooltip_at_the_edge_is_pulled_back_onto_the_screen() {
        let theme = theme();
        // Centring would put a 220-wide tooltip well off the right edge.
        let (left, _) = tip(370.0, 400.0).origin(&theme, Some(Size::new(400.0, 800.0)), 220.0);

        assert_eq!(left, 180.0, "its right edge lands on the surface edge");
    }

    #[test]
    fn a_tooltip_at_the_left_edge_never_goes_negative() {
        let theme = theme();
        let (left, _) = tip(0.0, 400.0).origin(&theme, Some(Size::new(400.0, 800.0)), 220.0);

        assert_eq!(left, 0.0);
    }

    /// The difference from a menu, and the one most likely to be regressed.
    #[test]
    fn a_tooltip_puts_no_barrier_under_itself() {
        let tree = built(tip(100.0, 400.0));

        assert!(
            tree.find("ModalBarrier").is_none(),
            "a tooltip that ate the next tap would swallow the press of the \
             control it is explaining"
        );
    }

    #[test]
    fn a_tooltip_announces_its_message() {
        let tree = built(tip(100.0, 400.0));
        let nodes = tree.find_all("Semantics");
        let labels: Vec<&str> = nodes
            .iter()
            .filter_map(|node| node.property("label"))
            .collect();

        assert!(labels.contains(&"Delete"), "{labels:?}");
    }
}
