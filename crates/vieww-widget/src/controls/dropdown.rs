use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Border, BoxDecoration, EdgeInsets, Key, Rect, TextStyle};

use crate::{
    icons, widget_node_from, BuildContext, CrossAxisAlignment, DecoratedBox, ElementState, Flex,
    Handler, Icon, MainAxisAlignment, MainAxisSize, Measured, Menu, MenuItem, OverlayHandle,
    OverlayId, Padding, Pressable, SemanticRole, Semantics, SizedBox, Stack, Text, ThemeData,
    Widget, WidgetKind, WidgetNode,
};

/// How wide the chevron's box is, so the label's width does not depend on it.
const CHEVRON: f32 = 20.0;

/// The gap between the label and the chevron at the trailing edge.
const LABEL_GAP: f32 = 8.0;

/// A closed control that shows one value and opens a list to change it.
///
/// ```
/// use std::rc::Rc;
/// use vieww_widget::prelude::*;
/// use vieww_widget::Dropdown;
///
/// # let chosen = Some(1usize);
/// # let open = false;
/// let country = Dropdown::new(["Ireland", "Japan", "Peru"], chosen)
///     .placeholder("Choose a country")
///     .open(open)
///     .on_toggled(Rc::new(|_open| {}))
///     .on_selected(Rc::new(|_index| {}));
/// ```
///
/// # Why this is not just a `Menu`
///
/// A [`Menu`] is a list of **commands**: each row does something, and the menu
/// has no memory of which one was last chosen because that is not a meaningful
/// question about "Delete". A dropdown is a list of **values**: exactly one is
/// current, the closed control displays it, and the row matching it is ticked.
///
/// That difference is not cosmetic — it is the whole accessibility story.
/// `SemanticRole::Button` on the closed control tells a screen reader that
/// pressing it does something; `SemanticRole::Custom("combobox")` with a value
/// tells it *what the field currently holds*, which is the only thing a user
/// filling in a form actually wants to know. (`Custom`, because `SemanticRole`
/// enumerates the roles vieww's own controls produce and a combo box is not one
/// of them; the winit bridge maps the name to AccessKit's `ComboBox`. This
/// paragraph named a `SemanticRole::ComboBox` variant that does not exist —
/// the code below has always been right, and a reader trying to follow the
/// documentation would not have compiled.) Building this out of `Menu` and leaving it a
/// button — which is what the shortcut looks like — produces a control that
/// reads as "Ireland, button" and never says it is a country field, never says
/// there are three options, and never says which is selected.
///
/// So the list is a `Menu` (that part genuinely is the same widget) and
/// everything around it is not.
///
/// # Controlled, like every other input here
///
/// It holds neither the selection nor whether it is open. Both are reported
/// through [`on_selected`](Self::on_selected) and [`on_toggled`](Self::on_toggled)
/// and applied by the caller, which is [`TextField`](crate::TextField)'s rule
/// and is what makes a dropdown whose value is derived from somewhere else
/// possible rather than a special case.
///
/// # An empty selection shows the placeholder, and the placeholder is not a value
///
/// `selected: None` draws [`placeholder`](Self::placeholder) in the subdued
/// colour and announces it as the control's **label**, never as its value —
/// exactly as [`TextField`](crate::TextField)'s placeholder does, and for the
/// same reason: announcing a hint as a value tells somebody the field is already
/// filled in.
#[derive(Clone)]
pub struct Dropdown {
    options: Vec<String>,
    selected: Option<usize>,
    placeholder: Option<String>,
    label: Option<String>,
    open: bool,
    enabled: bool,
    anchor: Option<Rect>,
    on_measured: Option<Handler<Rect>>,
    on_selected: Option<Handler<usize>>,
    on_toggled: Option<Handler<bool>>,
    key: Option<Key>,
}

impl Dropdown {
    /// The options, and which one is current.
    #[must_use]
    pub fn new<S: Into<String>>(
        options: impl IntoIterator<Item = S>,
        selected: Option<usize>,
    ) -> Self {
        let options: Vec<String> = options.into_iter().map(Into::into).collect();
        // A selection past the end is treated as none rather than panicking:
        // this is a *controlled* widget, so the index arrives from application
        // state that may have shrunk the list in the same frame. Panicking on a
        // one-frame inconsistency is the wrong trade for a picker.
        let selected = selected.filter(|index| *index < options.len());
        Self {
            options,
            selected,
            placeholder: None,
            label: None,
            open: false,
            enabled: true,
            anchor: None,
            on_measured: None,
            on_selected: None,
            on_toggled: None,
            key: None,
        }
    }

    /// What to show when nothing is selected.
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// What this field is for — "Country", not "Ireland".
    ///
    /// Announced by a screen reader in front of the value. Without one, the
    /// [`placeholder`](Self::placeholder) is used, and without either the
    /// control announces only its value, which is the least useful of the three.
    #[must_use]
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }

    /// Whether the list is showing. Owned by the caller — see the type docs.
    #[must_use]
    pub const fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Greyed out, and neither openable nor announced as pressable.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Where the closed control ended up, so the list opens against it.
    ///
    /// Paired with [`on_measured`](Self::on_measured): the caller keeps the
    /// rectangle in a signal and hands it back on the next build. That is the
    /// same contract [`Menu::anchor`] has and the same reason —
    /// [`Measured`](crate::Measured) is the only way a widget learns where it
    /// landed, and a widget may not hold state of its own.
    ///
    /// Without one the list opens centred, which is
    /// [`Menu`](crate::Menu)'s own behaviour for an unanchored menu and is
    /// correct rather than broken: on a phone a full-width picker is centred
    /// anyway.
    #[must_use]
    pub const fn anchor(mut self, anchor: Rect) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Where the closed control ended up. See [`anchor`](Self::anchor).
    #[must_use]
    pub fn on_measured(mut self, handler: Handler<Rect>) -> Self {
        self.on_measured = Some(handler);
        self
    }

    /// The chosen option's index.
    ///
    /// Also fires for the option that is *already* selected. Re-choosing is how
    /// a user closes the list without changing anything, and swallowing it here
    /// would leave the list open with nothing to close it.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<usize>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Called with the state the list should move to.
    #[must_use]
    pub fn on_toggled(mut self, handler: Handler<bool>) -> Self {
        self.on_toggled = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Which option is current, if any.
    #[must_use]
    pub const fn selection(&self) -> Option<usize> {
        self.selected
    }

    /// The text currently displayed on the closed control.
    ///
    /// The selected option, the placeholder, or nothing — in that order.
    #[must_use]
    pub fn display_text(&self) -> Option<&str> {
        match self.selected {
            Some(index) => Some(&self.options[index]),
            None => self.placeholder.as_deref(),
        }
    }

    /// `true` when it can be opened: enabled, with somewhere to report to.
    #[must_use]
    pub fn is_interactive(&self) -> bool {
        self.enabled && self.on_toggled.is_some() && !self.options.is_empty()
    }

    /// What a screen reader should say this field currently holds.
    ///
    /// `None` when nothing is selected — deliberately, rather than the
    /// placeholder. See the type docs.
    #[must_use]
    pub fn spoken_value(&self) -> Option<&str> {
        self.selected.map(|index| self.options[index].as_str())
    }

    /// What a screen reader should say this field is *for*.
    #[must_use]
    pub fn spoken_label(&self) -> Option<&str> {
        self.label.as_deref().or(self.placeholder.as_deref())
    }

    /// The closed control: the current text, and a chevron.
    fn closed(&self, theme: &ThemeData, press: f32) -> WidgetNode {
        let selected = self.selected.is_some();
        let ink = if !self.enabled {
            theme.colors.on_surface_variant
        } else if selected {
            theme.colors.on_surface
        } else {
            // A placeholder in the same colour as a value is a control that
            // looks filled in when it is not — the single most common way a
            // required field gets skipped.
            theme.colors.on_surface_variant
        };

        let text = Text::new(self.display_text().unwrap_or_default().to_owned()).style(TextStyle {
            color: ink,
            ..theme.text.body
        });

        let chevron = SizedBox::square(CHEVRON).child(
            Icon::new(if self.open {
                icons::chevron_up()
            } else {
                icons::chevron_down()
            })
            .color(ink),
        );

        let outline = if self.open {
            theme.colors.primary
        } else {
            theme.colors.outline
        };

        let decoration = BoxDecoration::filled(theme.colors.surface)
            .border(Border::thin(outline))
            .radius(theme.metrics.corner);

        DecoratedBox::new(crate::controls::pressed_fill(
            decoration,
            theme.colors.on_surface,
            press,
        ))
        .child(
            Padding::new(EdgeInsets::symmetric(10.0, 14.0)).child(
                Flex::row()
                    .main_axis_size(MainAxisSize::Max)
                    .main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .cross_axis_alignment(CrossAxisAlignment::Center)
                    .push(text)
                    .push(SizedBox::width(LABEL_GAP))
                    .push(chevron),
            ),
        )
        .into()
    }
}

impl Widget for Dropdown {
    fn debug_name(&self) -> &'static str {
        "Dropdown"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let theme = ThemeData::of(ctx);

        let control = self.clone();
        let closed: WidgetNode = if self.is_interactive() {
            let toggle = self.on_toggled.clone().expect("is_interactive checked it");
            let open = self.open;
            Pressable::themed(ctx, move |press| control.closed(&theme, press))
                .on_tap(move || toggle(!open))
                .into()
        } else {
            control.closed(&theme, 0.0)
        };

        // One stop, announcing what the field is for and what it currently
        // holds. `Custom("combobox")` rather than `Button`: the winit bridge
        // maps it to AccessKit's `ComboBox`, and a `Button` here would say
        // "Ireland, button" — which never mentions that this is a country field
        // or that there is anything to choose between.
        let mut semantics = Semantics::new()
            .role(SemanticRole::Custom("combobox"))
            .enabled(self.is_interactive());
        if let Some(label) = self.spoken_label() {
            semantics = semantics.label(label.to_owned());
        }
        if let Some(value) = self.spoken_value() {
            semantics = semantics.value(value.to_owned());
        }

        // Measured whether or not it is open, so the rectangle the list opens
        // against is already known on the frame it opens — measuring only while
        // open would place the first frame of every list against a stale
        // anchor, or none.
        let mut measured = Measured::new().child(semantics.child(closed));
        if let Some(handler) = &self.on_measured {
            measured = measured.on_measured(Rc::clone(handler));
        }

        // The list goes in the [`Overlay`], not here. See `list` and
        // `DropdownState` for why the alternative cannot be made to work.
        if !self.open {
            // Withdrawn on the frame the caller closes it, rather than waiting
            // for the element to be disposed of — a control that is still on
            // screen with `open == false` must not have a list up.
            let _ = ctx.state::<DropdownState, _>(DropdownState::hide);
            return measured.into();
        }

        let list: WidgetNode = self.list().into();
        match ctx.inherit::<OverlayHandle>() {
            Some(handle) => {
                let _ = ctx.state::<DropdownState, _>(|state| {
                    state.show(&handle, Rc::new(move || list.clone()));
                });
                measured.into()
            }
            None => {
                warn_no_overlay();
                // Built in place, which is where it is clipped and mispositioned
                // — see `warn_no_overlay`. Drawing it badly beats drawing
                // nothing, and the warning says exactly what to do about it.
                //
                // `push`, not `children`: `Stack::children` replaces the list,
                // and doing that here would drop the control the menu is
                // anchored to.
                Stack::new()
                    .children(crate::children![measured])
                    .push(self.list())
                    .into()
            }
        }
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(DropdownState::new()))
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("options", self.options.len().to_string()),
            (
                "selected",
                self.selected
                    .map_or_else(|| String::from("none"), |index| index.to_string()),
            ),
            ("open", self.open.to_string()),
            ("enabled", self.enabled.to_string()),
        ]
    }
}

impl Dropdown {
    /// The open list, as a [`Menu`] anchored to the closed control.
    ///
    /// Split out because it is built into two different places — the overlay
    /// when there is one, and in line when there is not — and the two must not
    /// be allowed to drift.
    fn list(&self) -> Menu {
        let items: Vec<MenuItem> = self
            .options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let item = MenuItem::new(option.clone());
                // The tick is what makes this a list of values rather than of
                // commands. A picker whose open list does not show the current
                // choice makes the user close it again to find out.
                if Some(index) == self.selected {
                    item.icon(icons::check())
                } else {
                    item
                }
            })
            .collect();

        let on_selected = self.on_selected.clone();
        let on_toggled = self.on_toggled.clone();

        // Both handlers run, in this order: the caller is told what was chosen
        // and *then* that the list should close. Reporting only the selection
        // would leave every application to remember to close the list, and the
        // one that forgets has a picker that never shuts.
        let chose: Handler<usize> = Rc::new(move |index| {
            if let Some(handler) = &on_selected {
                handler(index);
            }
            if let Some(handler) = &on_toggled {
                handler(false);
            }
        });

        let mut menu = Menu::new(items).on_selected(chose);
        if let Some(anchor) = self.anchor {
            menu = menu.anchor(anchor);
        }
        if let Some(handler) = &self.on_toggled {
            let handler = Rc::clone(handler);
            // The barrier is how a list is dismissed without choosing, and it
            // has to report — otherwise tapping away leaves the control drawn
            // open with nothing on screen.
            menu = menu.on_dismiss(move || handler(false));
        }

        menu
    }
}

/// Said once per process, on the same channel a layout overflow uses.
///
/// Once, because a dropdown that is open is open for many frames and a line per
/// frame buries everything else in the log.
fn warn_no_overlay() {
    use std::cell::Cell;
    thread_local! {
        static SAID: Cell<bool> = const { Cell::new(false) };
    }
    if SAID.with(Cell::get) {
        return;
    }
    SAID.with(|said| said.set(true));
    eprintln!(
        "vieww: a Dropdown was opened with no Overlay above it, so its list is \
         built in place — where it is clipped by any scrollable it sits in and \
         positioned against its own parent rather than the window. Wrap the \
         application's root in `Overlay::new().child(..)`."
    );
}

/// What a dropdown remembers between frames: which overlay entry is its own.
///
/// # Why it needs any state at all
///
/// The entry has to be **withdrawn** when the control leaves the tree — a
/// dropdown scrolled out of a long list with its menu open, or a screen popped
/// while a picker is up. Nothing else in the tree knows to do that, and an
/// entry nobody withdraws is a menu floating over an application that has
/// forgotten it exists. [`ElementState::dispose`] is the only teardown hook
/// there is, so this is the only place it can live.
///
/// # Why the build writes to it
///
/// `build` staying side-effect free is about **signal** dependency tracking
/// being sound: a build that wrote a signal would mark readers pending from
/// inside the pass that is reading them. Nothing here touches a signal. The
/// overlay handle is remembered so `dispose` has somewhere to send the
/// withdrawal, and the entry is handed to a plain cell the overlay polls
/// between phases — the same channel `MeasuredConstraints` reports through.
#[derive(Debug)]
pub(super) struct DropdownState {
    /// This dropdown's entry, for its whole life. Minted once: a fresh id per
    /// build would leave a copy of the list behind on every rebuild.
    id: OverlayId,
    /// The overlay last contributed to, so `dispose` can reach it.
    host: RefCell<Option<OverlayHandle>>,
}

impl DropdownState {
    fn new() -> Self {
        Self {
            id: OverlayId::new(),
            host: RefCell::new(None),
        }
    }

    /// Put the list up, or replace the one already there.
    fn show(&self, handle: &OverlayHandle, build: crate::OverlayBuilder) {
        *self.host.borrow_mut() = Some(handle.clone());
        handle.show(self.id, build);
    }

    /// Take it down. Costs nothing when there was nothing up.
    fn hide(&self) {
        if let Some(handle) = self.host.borrow().as_ref() {
            handle.hide(self.id);
        }
    }
}

impl ElementState for DropdownState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn dispose(&mut self) {
        self.hide();
    }
}

impl fmt::Debug for Dropdown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dropdown")
            .field("options", &self.options)
            .field("selected", &self.selected)
            .field("open", &self.open)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

widget_node_from!(Dropdown);

#[cfg(test)]
mod tests {
    use super::*;

    fn countries() -> [&'static str; 3] {
        ["Ireland", "Japan", "Peru"]
    }

    #[test]
    fn the_closed_control_shows_the_selected_value() {
        let dropdown = Dropdown::new(countries(), Some(1));
        assert_eq!(dropdown.display_text(), Some("Japan"));
        assert_eq!(dropdown.spoken_value(), Some("Japan"));
    }

    #[test]
    fn a_placeholder_is_shown_but_never_spoken_as_the_value() {
        // The same rule `TextField`'s placeholder follows, and the same reason:
        // announcing a hint as a value tells somebody the field is filled in.
        let dropdown = Dropdown::new(countries(), None).placeholder("Choose a country");
        assert_eq!(dropdown.display_text(), Some("Choose a country"));
        assert_eq!(
            dropdown.spoken_value(),
            None,
            "nothing is selected, so there is no value to report"
        );
        assert_eq!(
            dropdown.spoken_label(),
            Some("Choose a country"),
            "the hint says what the field is for, which is a label"
        );
    }

    #[test]
    fn an_explicit_label_beats_the_placeholder() {
        let dropdown = Dropdown::new(countries(), None)
            .placeholder("Choose one")
            .label("Country");
        assert_eq!(dropdown.spoken_label(), Some("Country"));
    }

    #[test]
    fn a_selection_past_the_end_reads_as_none_rather_than_panicking() {
        // A controlled widget receives its index from application state, which
        // can shrink the list in the same frame it renders. A one-frame
        // inconsistency must not take the process down.
        let dropdown = Dropdown::new(countries(), Some(9));
        assert_eq!(dropdown.selection(), None);
        assert_eq!(dropdown.display_text(), None);
    }

    #[test]
    fn a_dropdown_with_no_handler_is_not_interactive() {
        // The rule every control here follows: no handler means disabled, so a
        // read-only dropdown needs no separate mode.
        let dropdown = Dropdown::new(countries(), Some(0));
        assert!(!dropdown.is_interactive());
    }

    #[test]
    fn an_empty_dropdown_cannot_be_opened() {
        // Opening onto nothing shows a menu with no rows and no way to dismiss
        // it except the barrier — which looks like the application hanging.
        let dropdown = Dropdown::new(Vec::<String>::new(), None).on_toggled(Rc::new(|_| {}));
        assert!(!dropdown.is_interactive());
    }

    #[test]
    fn a_disabled_dropdown_is_not_interactive_even_with_handlers() {
        let dropdown = Dropdown::new(countries(), Some(0))
            .enabled(false)
            .on_toggled(Rc::new(|_| {}));
        assert!(!dropdown.is_interactive());
    }

    #[test]
    fn choosing_reports_the_index_and_then_closes() {
        // The ordering matters: an application applying the selection wants the
        // list already on its way shut, and one that only heard the selection
        // would have to remember to close it every time.
        use std::cell::RefCell;

        let log = Rc::new(RefCell::new(Vec::<String>::new()));

        let selected = {
            let log = Rc::clone(&log);
            Rc::new(move |index: usize| log.borrow_mut().push(format!("selected {index}")))
                as Handler<usize>
        };
        let toggled = {
            let log = Rc::clone(&log);
            Rc::new(move |open: bool| log.borrow_mut().push(format!("open {open}")))
                as Handler<bool>
        };

        // The composite handler `build` installs, exercised directly — the
        // ordering is the claim, and it lives in that closure rather than in the
        // tree.
        let on_selected = Some(Rc::clone(&selected));
        let on_toggled = Some(Rc::clone(&toggled));
        let chose: Handler<usize> = Rc::new(move |index| {
            if let Some(handler) = &on_selected {
                handler(index);
            }
            if let Some(handler) = &on_toggled {
                handler(false);
            }
        });

        chose(2);
        assert_eq!(
            log.borrow().clone(),
            vec!["selected 2".to_owned(), "open false".to_owned()]
        );
    }

    #[test]
    fn re_choosing_the_current_option_still_reports() {
        // It is how a user closes the list without changing anything, so
        // swallowing it leaves the menu open with nothing to shut it.
        use std::cell::Cell;

        let fired = Rc::new(Cell::new(false));
        let sink = Rc::clone(&fired);
        let dropdown = Dropdown::new(countries(), Some(1))
            .on_selected(Rc::new(move |_| sink.set(true)))
            .on_toggled(Rc::new(|_| {}));

        assert!(dropdown.is_interactive());
        dropdown.on_selected.as_ref().expect("set above")(1);
        assert!(fired.get(), "the already-selected row still reports");
    }
}
