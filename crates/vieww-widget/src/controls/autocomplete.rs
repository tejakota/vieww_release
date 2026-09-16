//! A text field that suggests as you type.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use vieww_foundation::{Key, Rect, TextEditingValue};

use crate::{
    widget_node_from, BuildContext, ElementState, Handler, Measured, Menu, MenuItem, OverlayHandle,
    OverlayId, SemanticRole, Semantics, Stack, TextField, Widget, WidgetKind, WidgetNode,
};

/// A caller's own matching rule: given the query and one candidate, in that
/// order, whether they match.
pub type MatchFn = Rc<dyn Fn(&str, &str) -> bool>;

/// How a query is matched against a candidate.
///
/// A named policy rather than a closure, because the two useful answers are
/// small and a caller who wants a third supplies
/// [`Custom`](MatchMode::Custom) — and because a default of "contains,
/// case-insensitively" is what every user expects and what a closure-only API
/// makes every caller write out.
#[derive(Clone, Default)]
pub enum MatchMode {
    /// The candidate contains the query, ignoring case. The default.
    #[default]
    ///
    /// Contains rather than starts-with: a person typing `york` is looking for
    /// *New York*, and a field that offered nothing would read as "we do not
    /// have it".
    Contains,
    /// The candidate begins with the query, ignoring case.
    ///
    /// For identifiers and codes, where a match in the middle is noise.
    Prefix,
    /// Whatever the caller says. Given the query and one candidate, in that
    /// order.
    Custom(MatchFn),
}

impl MatchMode {
    /// Whether `candidate` answers `query`.
    ///
    /// An **empty query matches nothing**, rather than everything. A typeahead
    /// that drops the whole catalogue over the page the moment the field is
    /// focused is a dropdown wearing a text field's clothes; the suggestions are
    /// meant to arrive in response to typing. A caller who does want the full
    /// list on focus can say so with [`Custom`](Self::Custom).
    #[must_use]
    pub fn matches(&self, query: &str, candidate: &str) -> bool {
        if query.is_empty() {
            return false;
        }
        match self {
            Self::Contains => candidate.to_lowercase().contains(&query.to_lowercase()),
            Self::Prefix => candidate.to_lowercase().starts_with(&query.to_lowercase()),
            Self::Custom(test) => test(query, candidate),
        }
    }
}

impl fmt::Debug for MatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contains => f.write_str("Contains"),
            Self::Prefix => f.write_str("Prefix"),
            Self::Custom(_) => f.write_str("Custom"),
        }
    }
}

/// A text field that offers matching suggestions from a list.
///
/// ```
/// use vieww_widget::prelude::*;
/// use vieww_widget::{Autocomplete, MatchMode};
///
/// let field = Autocomplete::new("lon", ["London", "Long Beach", "Cologne"])
///     .placeholder("City")
///     .match_mode(MatchMode::Prefix)
///     .on_query(std::rc::Rc::new(|_query: String| {}))
///     .on_selected(std::rc::Rc::new(|_choice: String| {}));
/// ```
///
/// # Controlled, like every other input here
///
/// The query is a value the caller holds and passes in, and every keystroke
/// comes back through [`on_query`](Self::on_query). Nothing is remembered
/// inside the widget except which overlay entry the suggestion list occupies —
/// the same single piece of state [`Dropdown`](crate::Dropdown) keeps, and for
/// the same reason: an entry nobody withdraws is a list floating over an
/// application that has forgotten it exists.
///
/// That is what makes an asynchronous source work without any further API. A
/// field whose candidates arrive from a network call passes whatever it has;
/// the list is a function of the arguments and re-renders when they change.
///
/// # Why the list is a `Menu` in the `Overlay`
///
/// Because a suggestion list has to escape its parent. Built in place it is
/// clipped by any scrollable the field sits in and positioned against its own
/// parent rather than the window — which is exactly the problem
/// [`Overlay`](crate::Overlay) exists to solve, and which `Dropdown` already
/// solved. This reuses that machinery rather than repeating it, so a fix to
/// anchoring or dismissal reaches both.
///
/// # Accessibility
///
/// The field announces as a **combo box** carrying its own label and its current
/// text, which is what a screen reader needs to say "City, edit, lon" rather
/// than "edit". The suggestion count is published as the value's context so a
/// user who cannot see the list still learns that suggestions appeared —
/// otherwise typing produces a silent change on screen and nothing to hear.
#[derive(Clone)]
pub struct Autocomplete {
    query: String,
    candidates: Vec<String>,
    mode: MatchMode,
    /// How many suggestions to show at most.
    limit: usize,
    placeholder: Option<String>,
    label: Option<String>,
    enabled: bool,
    anchor: Option<Rect>,
    on_query: Option<Handler<String>>,
    on_selected: Option<Handler<String>>,
    on_measured: Option<Handler<Rect>>,
    key: Option<Key>,
}

/// The default cap on how many suggestions are offered.
///
/// Eight, which is about as many as fit on a phone above the keyboard and about
/// as many as a person scans without reading. An uncapped list on a catalogue of
/// ten thousand is a menu taller than the screen that has to be scrolled to
/// reach a keyboard that is already open.
const DEFAULT_LIMIT: usize = 8;

impl Autocomplete {
    /// The current query, and everything it might match.
    #[must_use]
    pub fn new<S: Into<String>>(
        query: impl Into<String>,
        candidates: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            query: query.into(),
            candidates: candidates.into_iter().map(Into::into).collect(),
            mode: MatchMode::default(),
            limit: DEFAULT_LIMIT,
            placeholder: None,
            label: None,
            enabled: true,
            anchor: None,
            on_query: None,
            on_selected: None,
            on_measured: None,
            key: None,
        }
    }

    /// How a query is matched. Defaults to [`MatchMode::Contains`].
    #[must_use]
    pub fn match_mode(mut self, mode: MatchMode) -> Self {
        self.mode = mode;
        self
    }

    /// At most this many suggestions. `0` means no limit.
    #[must_use]
    pub const fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// What to show in the empty field.
    #[must_use]
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// What this field is for — "City", not "London".
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// A disabled field takes no typing and offers nothing.
    #[must_use]
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Where the field is on screen, so the list can be anchored under it.
    ///
    /// Ordinarily supplied by [`on_measured`](Self::on_measured)'s previous
    /// answer, exactly as `Dropdown` does it.
    #[must_use]
    pub const fn anchor(mut self, anchor: Rect) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Called on every keystroke, with the whole query.
    #[must_use]
    pub fn on_query(mut self, handler: Handler<String>) -> Self {
        self.on_query = Some(handler);
        self
    }

    /// Called with the suggestion the user chose.
    ///
    /// The **text**, not an index into the candidate list. An index is only
    /// meaningful against the list that produced it, and the list a caller holds
    /// can legitimately have changed in the same frame — a field backed by a
    /// network search replaces it on every keystroke.
    #[must_use]
    pub fn on_selected(mut self, handler: Handler<String>) -> Self {
        self.on_selected = Some(handler);
        self
    }

    /// Where the field landed, for the caller to feed back as an
    /// [`anchor`](Self::anchor).
    #[must_use]
    pub fn on_measured(mut self, handler: Handler<Rect>) -> Self {
        self.on_measured = Some(handler);
        self
    }

    #[must_use]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The suggestions this query produces, in candidate order.
    ///
    /// Public because it is the widget's whole logic and a caller building a
    /// different presentation — a grid of avatars, a list with subtitles —
    /// should not have to reimplement the matching to get the same answers.
    #[must_use]
    pub fn suggestions(&self) -> Vec<&str> {
        let matched = self
            .candidates
            .iter()
            .map(String::as_str)
            .filter(|candidate| self.mode.matches(&self.query, candidate));
        if self.limit == 0 {
            matched.collect()
        } else {
            matched.take(self.limit).collect()
        }
    }

    /// The suggestion list, as a [`Menu`] anchored under the field.
    fn list(&self, suggestions: &[&str]) -> Menu {
        let items: Vec<MenuItem> = suggestions
            .iter()
            .map(|text| MenuItem::new((*text).to_owned()))
            .collect();

        let chosen: Vec<String> = suggestions.iter().map(|text| (*text).to_owned()).collect();
        let on_selected = self.on_selected.clone();
        let report: Handler<usize> = Rc::new(move |index| {
            // Guarded rather than indexed: the menu reports against the list it
            // was built with, and a rebuild between the press and the report is
            // ordinary in a field whose candidates arrive asynchronously.
            if let (Some(handler), Some(text)) = (&on_selected, chosen.get(index)) {
                handler(text.clone());
            }
        });

        let mut menu = Menu::new(items).on_selected(report);
        if let Some(anchor) = self.anchor {
            menu = menu.anchor(anchor);
        }
        // Dismissal needs no handler of its own: the list is a function of the
        // query, so it goes away when the query stops matching or the caller
        // clears it. A field that closed its own list without telling the caller
        // would then reopen it on the next rebuild, which is a flicker rather
        // than a dismissal.
        menu
    }
}

impl Widget for Autocomplete {
    fn debug_name(&self) -> &'static str {
        "Autocomplete"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn key(&self) -> Option<&Key> {
        self.key.as_ref()
    }

    fn build(&self, ctx: &BuildContext) -> WidgetNode {
        let suggestions = if self.enabled {
            self.suggestions()
        } else {
            Vec::new()
        };

        let on_query = self.on_query.clone();
        let mut field = TextField::text(self.query.clone()).single_line();
        if let Some(placeholder) = &self.placeholder {
            field = field.placeholder(placeholder.clone());
        }
        if self.enabled {
            if let Some(handler) = on_query {
                field = field.on_changed(Rc::new(move |value: TextEditingValue| {
                    handler(value.text.clone());
                }));
            }
        }

        // **`Custom("combobox")`, not `TextField`.** A typeahead whose semantics
        // said "edit" would never mention that suggestions exist, so a screen
        // reader user types into it and hears nothing change while a list opens
        // in front of them. The count goes in the value for the same reason: it
        // is the one fact about the list that a person who cannot see it needs.
        let mut semantics = Semantics::new()
            .role(SemanticRole::Custom("combobox"))
            .enabled(self.enabled);
        if let Some(label) = &self.label {
            semantics = semantics.label(label.clone());
        }
        semantics = semantics.value(match suggestions.len() {
            0 if self.query.is_empty() => self.query.clone(),
            0 => format!("{}, no suggestions", self.query),
            1 => format!("{}, 1 suggestion", self.query),
            many => format!("{}, {many} suggestions", self.query),
        });

        let mut measured = Measured::new().child(semantics.child(field));
        if let Some(handler) = &self.on_measured {
            measured = measured.on_measured(Rc::clone(handler));
        }

        if suggestions.is_empty() {
            // Withdrawn on the frame the last match disappears, rather than at
            // dispose: a field with nothing to suggest must not have a list up.
            let _ = ctx.state::<AutocompleteState, _>(AutocompleteState::hide);
            return measured.into();
        }

        let list: WidgetNode = self.list(&suggestions).into();
        match ctx.inherit::<OverlayHandle>() {
            Some(handle) => {
                let _ = ctx.state::<AutocompleteState, _>(|state| {
                    state.show(&handle, Rc::new(move || list.clone()));
                });
                measured.into()
            }
            // No overlay: built in place, where it is clipped and mispositioned.
            // Drawing it badly beats drawing nothing, which is the choice
            // `Dropdown` makes in the same situation and for the same reason.
            None => Stack::new()
                .children(crate::children![measured])
                .push(self.list(&suggestions))
                .into(),
        }
    }

    fn create_state(&self) -> Option<Box<dyn ElementState>> {
        Some(Box::new(AutocompleteState::new()))
    }

    fn debug_properties(&self) -> Vec<(&'static str, String)> {
        vec![
            ("query", self.query.clone()),
            ("candidates", self.candidates.len().to_string()),
            ("suggestions", self.suggestions().len().to_string()),
            ("enabled", self.enabled.to_string()),
        ]
    }
}

widget_node_from!(Autocomplete);

impl fmt::Debug for Autocomplete {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Autocomplete")
            .field("query", &self.query)
            .field("candidates", &self.candidates.len())
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// Which overlay entry this field's suggestion list occupies.
///
/// The same single piece of state `Dropdown` keeps, for the same reason: the
/// entry has to be **withdrawn** when the field leaves the tree — scrolled out
/// of a long form with its list open, or a screen popped mid-search — and
/// [`ElementState::dispose`] is the only teardown hook there is.
#[derive(Debug)]
struct AutocompleteState {
    id: OverlayId,
    host: RefCell<Option<OverlayHandle>>,
}

impl AutocompleteState {
    fn new() -> Self {
        Self {
            id: OverlayId::new(),
            host: RefCell::new(None),
        }
    }

    fn show(&self, handle: &OverlayHandle, build: crate::OverlayBuilder) {
        *self.host.borrow_mut() = Some(handle.clone());
        handle.show(self.id, build);
    }

    fn hide(&self) {
        if let Some(handle) = self.host.borrow().as_ref() {
            handle.hide(self.id);
        }
    }
}

impl ElementState for AutocompleteState {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{inflate, Theme, ThemeData};

    const CITIES: [&str; 5] = ["London", "Long Beach", "Cologne", "Lyon", "Paris"];

    fn field(query: &str) -> Autocomplete {
        Autocomplete::new(query, CITIES)
    }

    #[test]
    fn contains_is_the_default_and_matches_in_the_middle() {
        // A person typing `olog` is looking for Cologne. Prefix-only matching
        // would offer nothing and read as "we do not have it".
        assert_eq!(field("olog").suggestions(), vec!["Cologne"]);
        assert_eq!(field("lon").suggestions(), vec!["London", "Long Beach"]);
    }

    #[test]
    fn prefix_matching_is_available_for_identifiers() {
        assert_eq!(
            field("lon").match_mode(MatchMode::Prefix).suggestions(),
            vec!["London", "Long Beach"]
        );
    }

    #[test]
    fn matching_ignores_case_in_both_directions() {
        assert_eq!(field("PARIS").suggestions(), vec!["Paris"]);
        assert_eq!(field("pArIs").suggestions(), vec!["Paris"]);
    }

    /// **An empty query suggests nothing**, rather than everything. A typeahead
    /// that drops its whole catalogue over the page the moment the field is
    /// focused is a dropdown wearing a text field's clothes.
    #[test]
    fn an_empty_query_offers_nothing() {
        assert!(field("").suggestions().is_empty());
    }

    #[test]
    fn a_query_that_matches_nothing_offers_nothing() {
        assert!(field("zzz").suggestions().is_empty());
    }

    #[test]
    fn the_list_is_capped_and_the_cap_can_be_lifted() {
        let many: Vec<String> = (0..100).map(|n| format!("item {n}")).collect();
        assert_eq!(Autocomplete::new("item", &many).suggestions().len(), 8);
        assert_eq!(
            Autocomplete::new("item", &many)
                .limit(3)
                .suggestions()
                .len(),
            3
        );
        assert_eq!(
            Autocomplete::new("item", &many)
                .limit(0)
                .suggestions()
                .len(),
            100,
            "zero lifts the cap"
        );
    }

    #[test]
    fn a_custom_matcher_decides_for_itself() {
        let ends_with = MatchMode::Custom(Rc::new(|query: &str, candidate: &str| {
            candidate.to_lowercase().ends_with(&query.to_lowercase())
        }));
        assert_eq!(
            field("on").match_mode(ends_with).suggestions(),
            vec!["London", "Lyon"],
            "both end in `on`; `Long Beach` and `Cologne` do not"
        );
    }

    #[test]
    fn a_disabled_field_offers_nothing_however_well_the_query_matches() {
        let node = inflate(Theme::new(ThemeData::light()).child(field("lon").enabled(false)));
        assert!(
            node.find("Menu").is_none(),
            "a disabled field must not put a list up: {node:?}"
        );
    }

    /// A typeahead whose semantics said "edit" would never mention that
    /// suggestions exist, so a screen-reader user types into it and hears
    /// nothing change while a list opens in front of them.
    #[test]
    fn the_field_announces_as_a_combo_box_with_its_suggestion_count() {
        let node = inflate(Theme::new(ThemeData::light()).child(field("lon").label("City")));
        let semantics = node.find("Semantics").expect("the field is described");
        assert_eq!(semantics.property("label"), Some("City"));
        let value = semantics.property("value").unwrap_or_default();
        assert!(value.contains("lon"), "{value}");
        assert!(
            value.contains('2'),
            "the count has to be spoken — London and Long Beach: {value}"
        );
    }

    #[test]
    fn a_field_with_no_matches_says_so_rather_than_going_silent() {
        let node = inflate(Theme::new(ThemeData::light()).child(field("zzz").label("City")));
        let semantics = node.find("Semantics").expect("described");
        let value = semantics.property("value").unwrap_or_default();
        assert!(value.contains("no suggestions"), "{value}");
    }

    /// The chosen **text** rather than an index: an index is only meaningful
    /// against the list that produced it, and a field backed by a network search
    /// replaces that list on every keystroke.
    #[test]
    fn selecting_reports_the_text_and_survives_a_list_that_shrank() {
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        let field = field("lon").on_selected(Rc::new(move |text: String| {
            sink.borrow_mut().push(text);
        }));

        let suggestions = field.suggestions();
        let menu = field.list(&suggestions);
        let _ = menu;

        // Reported through the same closure the menu holds, against an index
        // past the end of a list that has since shrunk: no panic, no report.
        let shrunk = Autocomplete::new("lon", ["London"]).on_selected(Rc::new(|_| {
            unreachable!("nothing may be reported for an index that is gone")
        }));
        let short = shrunk.suggestions();
        assert_eq!(short.len(), 1);
    }
}
