use std::fmt;
use std::rc::Rc;

use vieww_foundation::{BoxDecoration, Constraints, EdgeInsets, Key, TextStyle};

use crate::{
    children, widget_node_from, Align, BuildContext, Constrained, CrossAxisAlignment, DecoratedBox,
    Flex, MainAxisAlignment, MainAxisSize, ModalBarrier, OverlayPosition, Padding, SemanticRole,
    Semantics, SizedBox, Stack, StackFit, Text, ThemeData, Widget, WidgetKind, WidgetNode,
};

/// The widest a dialog gets, however much room there is.
///
/// A dialog stretched across a tablet is a line of text with a button at each
/// end of the room; the eye cannot follow it. Every platform picks a number
/// around here for the same reason.
/// The divider above and between an Apple alert's buttons.
const HAIRLINE: f32 = 0.5;

const DIALOG_MAX_WIDTH: f32 = 400.0;

/// A surface floating above the screen, with a scrim behind it.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Dialog, OverlayPosition};
///
/// let confirm = Dialog::new()
///     .title("Delete this file?")
///     .content(Text::new("This cannot be undone."))
///     .actions(children![
///         Button::new("Cancel").style(ButtonStyle::Text).on_pressed(|| {}),
///         Button::new("Delete").on_pressed(|| {}),
///     ])
///     .on_dismiss(|| {});
/// ```
///
/// Push it as a **modal route** — `Route::modal` — rather than putting it inside
/// a screen: it is meant to cover the whole surface, its scrim has to sit above
/// everything, and back should dismiss it rather than leave the screen. The
/// navigator's stack is what gives it all three.
///
/// [`OverlayPosition`] decides where the surface sits;
/// [`BottomSheet`](crate::BottomSheet) and [`Snackbar`](crate::Snackbar) are the
/// same machinery at the bottom of the screen with different shapes.
#[derive(Clone)]
pub struct Dialog {
    title: Option<String>,
    content: Option<WidgetNode>,
    actions: Vec<WidgetNode>,
    position: OverlayPosition,
    on_dismiss: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Default for Dialog {
    fn default() -> Self {
        Self::new()
    }
}

impl Dialog {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            title: None,
            content: None,
            actions: Vec::new(),
            position: OverlayPosition::Center,
            on_dismiss: None,
            key: None,
        }
    }

    /// The one line saying what this is about.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The body.
    #[must_use]
    pub fn content(mut self, content: impl Into<WidgetNode>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// The buttons along the bottom, in reading order — so the confirming one
    /// goes last, where every platform puts it.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = WidgetNode>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    #[must_use]
    pub const fn position(mut self, position: OverlayPosition) -> Self {
        self.position = position;
        self
    }

    /// Called when the scrim is tapped.
    ///
    /// Without it the scrim still swallows the tap — see [`ModalBarrier`].
    #[must_use]
    pub fn on_dismiss(mut self, handler: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Set the reconciliation key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Dialog {
    fn debug_name(&self) -> &'static str {
        "Dialog"
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

        // **An iOS alert is a different object from an Android dialog**, and
        // the difference is not decoration: its text is centred, its buttons
        // are full-width rows under a hairline rather than a right-aligned row
        // inside the padding, and its corner is much rounder. An Android dialog
        // shown on iOS is the single most obvious "this is not a native app"
        // tell there is.
        let apple = theme.platform.is_apple();

        let mut column = Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .cross_axis_alignment(if apple {
                CrossAxisAlignment::Center
            } else {
                CrossAxisAlignment::Start
            });

        if let Some(title) = &self.title {
            let style = theme.text.title;
            column = column.push(
                Text::new(title.clone())
                    .align(if apple {
                        vieww_foundation::TextAlign::Center
                    } else {
                        vieww_foundation::TextAlign::Start
                    })
                    .style(style),
            );
        }
        if let Some(content) = &self.content {
            if self.title.is_some() {
                column = column.push(SizedBox::height(gap));
            }
            column = column.push(content.clone());
        }

        // The buttons, and on Apple they live *outside* the padded body so the
        // hairline above them can run the full width of the alert.
        let mut actions: Option<WidgetNode> = None;
        if !self.actions.is_empty() {
            if apple {
                let mut stack = Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch);
                for action in &self.actions {
                    stack = stack.push(
                        DecoratedBox::new(BoxDecoration::filled(theme.colors.outline))
                            .child(SizedBox::height(HAIRLINE)),
                    );
                    stack = stack
                        .push(Padding::new(EdgeInsets::symmetric(0.0, gap)).child(action.clone()));
                }
                actions = Some(stack.into());
            } else {
                column = column.push(SizedBox::height(gap * 2.0));
                column = column.push(
                    // Pushed to the trailing edge, which is where a hand expects
                    // to find the button it is about to press.
                    Flex::row()
                        .main_axis_alignment(MainAxisAlignment::End)
                        .children(self.actions.clone()),
                );
            }
        }

        let body: WidgetNode = Padding::new(EdgeInsets::all(gap * 3.0))
            .child(column)
            .into();
        let contents: WidgetNode = match actions {
            Some(actions) => Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .push(body)
                .push(actions)
                .into(),
            None => body,
        };

        let surface = DecoratedBox::new(BoxDecoration::filled(theme.colors.surface).radius(
            if apple {
                // 14 points, which is what the system draws and is visibly
                // rounder than an Android dialog's corner.
                14.0
            } else {
                theme.metrics.corner * 1.5
            },
        ))
        .child(contents);

        // Wide enough to read, never wider; and free to be as tall as it needs.
        let sized = Constrained::new(Constraints::new(0.0, DIALOG_MAX_WIDTH, 0.0, f32::INFINITY))
            .child(surface);

        let barrier: WidgetNode = match &self.on_dismiss {
            Some(handler) => {
                let handler = Rc::clone(handler);
                ModalBarrier::new().on_dismiss(move || handler()).into()
            }
            None => ModalBarrier::new().into(),
        };

        // The scrim first, the surface over it. A `Stack` paints in order, so
        // this is also the order a hit test tries them in reverse — the surface
        // gets the tap, and the scrim gets everything that misses.
        //
        // A *container* annotation, not a merging one: a dialog that spoke for
        // its subtree would announce its title and swallow its own buttons,
        // which is a modal a screen reader user cannot answer.
        // **The surface is wrapped in a focus trap, and the barrier is not.**
        //
        // A modal that Tab walks straight out of is not modal: three presses
        // from the dialog's last button and the keyboard is in a field on the
        // screen behind, under a scrim, with a caret nobody can see. That was
        // the framework's behaviour, and a complete scope-based focus manager
        // sat unwired in `vieww-foundation` describing the feature accurately
        // while the live system had no scopes at all.
        //
        // Around the surface rather than around the `Stack`, deliberately: the
        // barrier covers the whole screen, so a trap that included it would be a
        // trap over everything and would confine nothing.
        let trapped: WidgetNode = crate::FocusTrap::new(true)
            .child(
                Align::new(self.position.alignment())
                    .child(Padding::new(EdgeInsets::all(gap * 2.0)).child(sized)),
            )
            .into();

        Semantics::container(self.title.clone().unwrap_or_else(|| "Dialog".to_owned()))
            .role(SemanticRole::Group)
            .child(
                Stack::new()
                    .fit(StackFit::Expand)
                    .children(children![barrier, trapped]),
            )
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("actions", self.actions.len().to_string())];
        if let Some(title) = &self.title {
            props.push(("title", title.clone()));
        }
        props.push(("dismissible", self.on_dismiss.is_some().to_string()));
        props
    }
}

impl fmt::Debug for Dialog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dialog")
            .field("title", &self.title)
            .field("actions", &self.actions.len())
            .finish_non_exhaustive()
    }
}

widget_node_from!(Dialog);

/// A surface that comes up from the bottom of the screen.
///
/// The same thing as a [`Dialog`] with different geometry: full width, rounded
/// only at the top, anchored to the bottom edge. Push it as a modal route.
#[derive(Clone)]
pub struct BottomSheet {
    child: Option<WidgetNode>,
    on_dismiss: Option<Rc<dyn Fn()>>,
    key: Option<Key>,
}

impl Default for BottomSheet {
    fn default() -> Self {
        Self::new()
    }
}

impl BottomSheet {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            child: None,
            on_dismiss: None,
            key: None,
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl Into<WidgetNode>) -> Self {
        self.child = Some(child.into());
        self
    }

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
}

impl Widget for BottomSheet {
    fn debug_name(&self) -> &'static str {
        "BottomSheet"
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

        // Rounded on all four corners but with the bottom two off the screen —
        // which is what a sheet anchored to the bottom edge looks like, without
        // needing per-corner radii the decoration does not have.
        let surface = DecoratedBox::new(
            BoxDecoration::filled(theme.colors.surface).radius(theme.metrics.corner * 2.0),
        )
        .child(
            Padding::new(EdgeInsets::all(gap * 2.0)).child(
                Flex::column()
                    .main_axis_size(MainAxisSize::Min)
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .children(match &self.child {
                        Some(child) => vec![child.clone()],
                        None => Vec::new(),
                    }),
            ),
        );

        let barrier: WidgetNode = match &self.on_dismiss {
            Some(handler) => {
                let handler = Rc::clone(handler);
                ModalBarrier::new().on_dismiss(move || handler()).into()
            }
            None => ModalBarrier::new().into(),
        };

        // Trapped for the same reason a `Dialog` is, and around the surface for
        // the same reason: the barrier covers the screen, so a trap that
        // included it would confine nothing.
        let trapped: WidgetNode = crate::FocusTrap::new(true)
            .child(Align::new(OverlayPosition::Bottom.alignment()).child(surface))
            .into();

        Stack::new()
            .fit(StackFit::Expand)
            .children(children![barrier, trapped])
            .into()
    }
}

impl fmt::Debug for BottomSheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BottomSheet")
            .field("dismissible", &self.on_dismiss.is_some())
            .finish_non_exhaustive()
    }
}

widget_node_from!(BottomSheet);

/// A short message at the bottom of the screen, with an optional action.
///
/// **No scrim and no barrier**: a snackbar is not modal. It says something
/// happened and gets out of the way, and the screen behind it stays usable —
/// which is the entire difference between a snackbar and a dialog, and the
/// reason it is not built on one.
///
/// It also does not dismiss itself. That needs a timer, and the only clock in
/// this framework is the frame — so an application that wants one drives it from
/// an animation or a scheduled rebuild, deliberately, rather than this widget
/// spawning something that outlives the tree.
#[derive(Clone)]
pub struct Snackbar {
    message: String,
    action: Option<(String, Rc<dyn Fn()>)>,
    key: Option<Key>,
}

impl Snackbar {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            action: None,
            key: None,
        }
    }

    /// A single trailing action — "Undo", "Retry", "View".
    #[must_use]
    pub fn action(mut self, label: impl Into<String>, handler: impl Fn() + 'static) -> Self {
        self.action = Some((label.into(), Rc::new(handler)));
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl Widget for Snackbar {
    fn debug_name(&self) -> &'static str {
        "Snackbar"
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

        // On the *inverse* of the surface, so a message reads as a thing laid on
        // top of the screen rather than part of it.
        let message = Text::new(self.message.clone()).style(TextStyle {
            color: theme.colors.surface,
            ..theme.text.body
        });

        let mut row = Flex::row()
            .main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .cross_axis_alignment(CrossAxisAlignment::Center)
            .push(message);

        if let Some((label, handler)) = &self.action {
            let handler = Rc::clone(handler);
            row = row.push(
                crate::Button::new(label.clone())
                    .style(crate::ButtonStyle::Text)
                    .on_pressed(move || handler()),
            );
        }

        let bar = DecoratedBox::new(
            BoxDecoration::filled(theme.colors.on_surface).radius(theme.metrics.corner),
        )
        .child(Padding::new(EdgeInsets::symmetric(gap * 2.0, gap)).child(row));

        // `container`, not a merging `Semantics::new()` — a snackbar appearing
        // and disappearing without being announced is invisible to a screen
        // reader — the canonical case of where the
        // default is silently wrong. `SemanticRole::Custom("alert")` reaches
        // AccessKit's `Role::Alert`, which every platform screen reader treats
        // as an implicit **live region** — announced the moment it mounts,
        // with no focus needed, the same way a toast or a form error is. And
        // because this is `container` rather than a merging annotation, the
        // action button underneath stays independently reachable — the thing
        // the old, Semantics-free version of this method was actually trying
        // to protect, achieved here without leaving the message unannounced
        // to get it.
        let bar = Semantics::container(self.message.clone())
            .role(SemanticRole::Custom("alert"))
            .child(bar);

        Align::new(OverlayPosition::Bottom.alignment())
            .child(Padding::new(EdgeInsets::all(gap * 2.0)).child(
                Constrained::new(Constraints::new(0.0, 600.0, 0.0, f32::INFINITY)).child(bar),
            ))
            .into()
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        let mut props = vec![("message", self.message.clone())];
        if let Some((label, _)) = &self.action {
            props.push(("action", label.clone()));
        }
        props
    }
}

impl fmt::Debug for Snackbar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snackbar")
            .field("message", &self.message)
            .field("action", &self.action.as_ref().map(|(label, _)| label))
            .finish_non_exhaustive()
    }
}

widget_node_from!(Snackbar);

#[cfg(test)]
mod tests {
    use vieww_foundation::Size;

    use crate::{inflate, Theme};

    use super::*;

    /// A box of a fixed size, used where a test needs content.
    fn stub() -> SizedBox {
        SizedBox::from_size(Size::square(40.0))
    }

    fn built(widget: impl Into<WidgetNode>) -> crate::DebugNode {
        inflate(Theme::new(ThemeData::light()).child(widget.into()))
    }

    #[test]
    fn a_dialog_puts_a_scrim_under_its_surface() {
        let tree = built(Dialog::new().title("Delete?").content(stub()));
        assert!(tree.find("ModalBarrier").is_some());
    }

    #[test]
    fn a_dialog_is_never_wider_than_a_line_anyone_can_read() {
        let tree = built(Dialog::new().content(stub()));
        let limit = format!("w[0..{DIALOG_MAX_WIDTH}]");

        assert!(
            tree.find_all("Constrained")
                .iter()
                .filter_map(|node| node.property("constraints"))
                .any(|shown| shown.starts_with(&limit)),
            "no width limit among {:?}",
            tree.find_all("Constrained")
                .iter()
                .filter_map(|node| node.property("constraints"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_dialog_announces_its_title_to_a_screen_reader() {
        let tree = built(Dialog::new().title("Delete this file?").content(stub()));
        let semantics = tree.find("Semantics").expect("annotated");
        assert_eq!(semantics.property("label"), Some("Delete this file?"));
    }

    #[test]
    fn a_snackbar_has_no_scrim_because_it_is_not_modal() {
        let tree = built(Snackbar::new("Message sent"));
        assert!(
            tree.find("ModalBarrier").is_none(),
            "a snackbar that blocked the screen behind it would be a dialog"
        );
    }

    /// The scrim hides the screen behind it from a screen reader too.
    ///
    /// Taps were always absorbed; the semantics tree is walked rather than hit
    /// tested, so it needed saying separately. See `BlockSemantics`.
    #[test]
    fn a_dialogs_scrim_hides_the_screen_behind_it_from_a_screen_reader() {
        let tree = built(Dialog::new().title("Delete?").content(stub()));
        assert!(
            tree.find("BlockSemantics").is_some(),
            "the barrier carries the marker that silences what is behind it"
        );
    }

    /// And a snackbar does not, which is why the marker is on the barrier
    /// rather than on the route.
    ///
    /// Both are modal *routes* — neither is opaque, because both are meant to
    /// let the screen show through. Only one of them covers it. Keying the
    /// block off `!Route::is_opaque()` would have silenced an entire screen for
    /// the duration of a "Message sent".
    #[test]
    fn a_snackbar_silences_nothing_behind_it() {
        let tree = built(Snackbar::new("Message sent"));
        assert!(
            tree.find("BlockSemantics").is_none(),
            "a snackbar is an aside, not a modal"
        );
    }

    #[test]
    fn a_snackbars_action_is_a_button_that_reads_as_one() {
        let tree = built(Snackbar::new("Deleted").action("Undo", || {}));
        let button = tree.find("Button").expect("an action");
        assert_eq!(button.property("label"), Some("Undo"));
    }

    #[test]
    fn a_snackbar_announces_itself_as_an_alert_and_still_leaves_its_action_reachable() {
        // `alert` is what reaches AccessKit's implicit live region — the
        // announcement `docs/AIMS.md` §J asks for happening by construction
        // rather than by an application remembering to wire one up.
        let tree = built(Snackbar::new("Deleted").action("Undo", || {}));
        let semantics = tree.find("Semantics").expect("an announcement");
        assert_eq!(semantics.property("role"), Some("Custom(\"alert\")"));
        assert_eq!(semantics.property("label"), Some("Deleted"));

        // `container`, not a merging annotation — the action must still be
        // its own reachable stop, not swallowed into the alert's one node.
        assert!(
            tree.find("Button").is_some(),
            "the action must survive being wrapped in the alert's semantics"
        );
    }

    #[test]
    fn a_sheet_sits_against_the_bottom_edge() {
        let tree = built(BottomSheet::new().child(stub()));
        let align = tree.find("Align").expect("anchored");
        assert_eq!(align.property("alignment"), Some("bottomCenter"));
    }
}
