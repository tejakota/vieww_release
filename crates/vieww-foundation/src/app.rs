//! Application fundamentals that are not the framework's to implement.
//!
//! Feature flags, telemetry and over-the-air updates. `feature-checklist.md`
//! lists all three under "core architecture", and `docs/AIMS.md` §A is explicit
//! that shipping any of them *in the framework* is specifically not an aim: an
//! OTA mechanism belongs to a product's release process, a flag service belongs
//! to whoever runs the experiments, and telemetry belongs to whoever is on the
//! hook for what is collected.
//!
//! # So why is this file here at all
//!
//! Because §A's actual claim is that **nothing on that list needs a framework
//! change to add**, and a claim like that is worth exactly as much as the
//! attempts to check it. This module is the check, taken on the three rows most
//! likely to want a special case:
//!
//! - [`FeatureFlags`] is read during `build`, which is the constraint that would
//!   otherwise push a flag service into the framework's own state;
//! - [`Telemetry`] is written from anywhere, including a background task, which
//!   is the constraint that would otherwise push it into the frame loop;
//! - [`UpdateChannel`] has to survive the process it is updating, which is the
//!   constraint that makes people reach for a lifecycle hook.
//!
//! All three fit the ordinary [`Services`] seam with
//! no framework change, and the fakes here are enough for an application to be
//! written and tested against before it has picked a vendor. If a fourth one
//! ever *does* need a framework change, that is a defect in the seam and this
//! file is where it will show up first.
//!
//! Nothing here talks to a network. That is the line: the vocabulary is the
//! framework's, the implementation is the application's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::service::{ServiceError, Services};

/// What a flag can be worth.
///
/// Three types rather than a string, because a flag read as a string is one an
/// application parses at every call site — and the parses disagree. The set is
/// small on purpose: a flag richer than this is configuration, and configuration
/// wants a document rather than a switch.
#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl FlagValue {
    /// The boolean, or `None` if it is not one.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            _ => None,
        }
    }
}

/// Switches an application reads to decide what to show.
///
/// # Reading a flag must be safe during `build`, and must not block
///
/// This is the whole contract, and it is the reason the trait looks this plain.
/// A `build` is required to have no side effects and to return promptly; a flag
/// service that fetched on read would break both, and the symptom is a frame
/// that takes a network round trip. So [`bool_flag`](Self::bool_flag) and its
/// siblings answer from whatever the implementation last knew, and *refreshing*
/// is a separate, explicit act.
///
/// An implementation that has never fetched anything answers with the caller's
/// default, which is the correct behaviour on a first launch offline — an
/// application that renders nothing until its flags arrive is one that renders
/// nothing on a train.
pub trait FeatureFlags: 'static {
    /// The flag's current value, or `None` if this service has never heard of it.
    fn flag(&self, name: &str) -> Option<FlagValue>;

    /// The flag as a boolean, falling back to `default`.
    ///
    /// The shape almost every call site wants, provided once here so that
    /// "unknown flag" and "flag that is not a boolean" cannot be handled two
    /// different ways in two different screens.
    fn bool_flag(&self, name: &str, default: bool) -> bool {
        self.flag(name)
            .as_ref()
            .and_then(FlagValue::as_bool)
            .unwrap_or(default)
    }

    /// The flag as a number, falling back to `default`.
    fn number_flag(&self, name: &str, default: f64) -> f64 {
        self.flag(name)
            .as_ref()
            .and_then(FlagValue::as_number)
            .unwrap_or(default)
    }

    /// The flag as text, falling back to `default`.
    fn text_flag(&self, name: &str, default: &str) -> String {
        self.flag(name)
            .as_ref()
            .and_then(FlagValue::as_text)
            .map_or_else(|| default.to_owned(), ToOwned::to_owned)
    }
}

/// Something worth recording that happened.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryEvent {
    pub name: String,
    /// Free-form, because every vendor's schema is different and translating
    /// between two of them is the implementation's job rather than this type's.
    pub properties: Vec<(String, String)>,
}

impl TelemetryEvent {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            properties: Vec::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push((key.into(), value.into()));
        self
    }
}

/// How serious a log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Structured logging and product analytics.
///
/// # Recording must never fail, and never block
///
/// Neither method returns a `Result`, deliberately. A telemetry call sits in the
/// middle of ordinary application code — in a button handler, in an error path —
/// and a fallible one grows a `let _ =` in front of it at every call site within
/// a week, at which point the `Result` is worse than useless because it looks
/// like it is being handled.
///
/// The implementation buffers, batches, and drops what it cannot send. Losing a
/// telemetry event is not an application's problem to solve; blocking a frame on
/// one is a bug the user can see.
pub trait Telemetry: 'static {
    /// Record a product event.
    fn record(&self, event: TelemetryEvent);

    /// Record a log line.
    fn log(&self, level: LogLevel, message: &str);
}

/// Where an over-the-air update stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    /// Nothing newer than what is running.
    UpToDate,
    /// A newer version exists and has not been fetched.
    Available { version: String },
    /// Fetched and staged; it takes effect on the next launch.
    Ready { version: String },
}

/// Over-the-air updates.
///
/// # Nothing here restarts anything
///
/// [`state`](Self::state) reports and [`fetch`](Self::fetch) downloads, and an
/// update that is [`Ready`](UpdateState::Ready) takes effect when the process
/// next starts. There is deliberately no `apply_now`: swapping an application's
/// code underneath a running frame is not something a UI framework can make safe
/// — every `Rc` in the tree points into the code being replaced — and an API
/// that offered it would be offering a crash.
///
/// The honest interface is therefore "tell the user there is an update", which
/// is what an application does with this. That is also all any mainstream ecosystem
/// does, at rather more ceremony.
pub trait UpdateChannel: 'static {
    /// What is known right now, without going to the network.
    ///
    /// Safe to call during `build`, for [`FeatureFlags`]' reason.
    fn state(&self) -> UpdateState;

    /// Look for a newer version, and stage it if there is one.
    ///
    /// `then` runs on the UI thread, possibly on a later frame, and possibly
    /// never — the same contract as
    /// [`Permissions::request`](crate::permission::Permissions::request), for
    /// the same reason.
    fn fetch(&self, then: Rc<dyn Fn(Result<UpdateState, ServiceError>)>);
}

/// Flags held in a map, with no network behind them.
///
/// The default answer for an unknown flag is `None`, which is what makes
/// [`FeatureFlags::bool_flag`]'s fallback the tested path rather than the
/// theoretical one.
#[derive(Debug, Default)]
pub struct InMemoryFlags {
    values: RefCell<HashMap<String, FlagValue>>,
}

impl InMemoryFlags {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a flag, as a remote service would after a fetch.
    pub fn set(&self, name: impl Into<String>, value: FlagValue) {
        self.values.borrow_mut().insert(name.into(), value);
    }

    /// Forget a flag, as a service does when an experiment ends.
    pub fn clear(&self, name: &str) {
        self.values.borrow_mut().remove(name);
    }
}

impl FeatureFlags for InMemoryFlags {
    fn flag(&self, name: &str) -> Option<FlagValue> {
        self.values.borrow().get(name).cloned()
    }
}

/// Telemetry that is kept rather than sent.
///
/// The point of it being a recorder rather than a no-op: "this button reports
/// the event it is supposed to" is a thing a test should be able to assert, and
/// it is exactly the thing that silently stops being true.
#[derive(Debug, Default)]
pub struct RecordingTelemetry {
    events: RefCell<Vec<TelemetryEvent>>,
    logs: RefCell<Vec<(LogLevel, String)>>,
}

impl RecordingTelemetry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every event recorded, in order.
    #[must_use]
    pub fn events(&self) -> Vec<TelemetryEvent> {
        self.events.borrow().clone()
    }

    /// Every log line, in order.
    #[must_use]
    pub fn logs(&self) -> Vec<(LogLevel, String)> {
        self.logs.borrow().clone()
    }

    /// Log lines at `level` or worse — what a crash report would attach.
    #[must_use]
    pub fn logs_at_least(&self, level: LogLevel) -> Vec<(LogLevel, String)> {
        self.logs
            .borrow()
            .iter()
            .filter(|(recorded, _)| *recorded >= level)
            .cloned()
            .collect()
    }
}

impl Telemetry for RecordingTelemetry {
    fn record(&self, event: TelemetryEvent) {
        self.events.borrow_mut().push(event);
    }

    fn log(&self, level: LogLevel, message: &str) {
        self.logs.borrow_mut().push((level, message.to_owned()));
    }
}

/// An update channel that answers whatever a test told it to.
#[derive(Debug)]
pub struct ScriptedUpdates {
    state: RefCell<UpdateState>,
    /// What the *next* fetch discovers, if anything.
    next: RefCell<Option<String>>,
    fetches: RefCell<u32>,
}

impl ScriptedUpdates {
    /// Nothing newer.
    #[must_use]
    pub fn up_to_date() -> Self {
        Self {
            state: RefCell::new(UpdateState::UpToDate),
            next: RefCell::new(None),
            fetches: RefCell::new(0),
        }
    }

    /// A version a fetch will find.
    #[must_use]
    pub fn offering(version: impl Into<String>) -> Self {
        Self {
            state: RefCell::new(UpdateState::UpToDate),
            next: RefCell::new(Some(version.into())),
            fetches: RefCell::new(0),
        }
    }

    /// How many times the network was asked.
    #[must_use]
    pub fn fetches(&self) -> u32 {
        *self.fetches.borrow()
    }
}

impl UpdateChannel for ScriptedUpdates {
    fn state(&self) -> UpdateState {
        self.state.borrow().clone()
    }

    fn fetch(&self, then: Rc<dyn Fn(Result<UpdateState, ServiceError>)>) {
        *self.fetches.borrow_mut() += 1;
        let found = self.next.borrow_mut().take();
        let state = match found {
            Some(version) => UpdateState::Ready { version },
            None => UpdateState::UpToDate,
        };
        *self.state.borrow_mut() = state.clone();
        then(Ok(state));
    }
}

/// Where the application is in its life.
///
/// Ordered by how much of the machine it currently has.
/// Widgets that need to pause a video, stop a timer, or save drafts
/// observe this through the [`AppLifecycle`] service rather than through
/// platform-specific hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LifecycleState {
    /// No surface yet, or the surface has been destroyed.
    #[default]
    Detached,
    /// On screen and focused.
    Resumed,
    /// On screen but not focused.
    Inactive,
    /// Alive but not visible: minimised, occluded, or behind another window.
    Hidden,
    /// The surface is gone; nothing may be drawn until a resume.
    Paused,
}

impl LifecycleState {
    /// `true` when a frame would reach a human being.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        matches!(self, Self::Resumed | Self::Inactive)
    }

    /// `true` when there is a surface to draw into.
    #[must_use]
    pub const fn has_surface(self) -> bool {
        !matches!(self, Self::Detached | Self::Paused)
    }
}

/// Application lifecycle, observed through the service seam.
///
/// A video player that pauses when the app backgrounds, a form that saves
/// a draft when the user switches away, a timer that stops ticking when the
/// surface vanishes — all of these read lifecycle state, and all of them
/// should work the same way in tests as on a phone.
///
/// # Why this is a service and not a callback
///
/// A callback needs somewhere to register, and the only `somewhere` the
/// framework owns is the element tree — which is the wrong place for
/// application logic. A service is looked up on demand, needs no
/// registration, and fits the seam every other capability uses.
pub trait AppLifecycle: 'static {
    /// Where the application is right now.
    fn state(&self) -> LifecycleState;

    /// `true` when the application last transitioned away from the
    /// foreground and has not yet returned. Useful for one-shot work like
    /// saving a draft.
    fn did_enter_background(&self) -> bool;
}

/// A lifecycle a test can script.
#[derive(Debug)]
pub struct HeadlessLifecycle {
    state: RefCell<LifecycleState>,
    entered_background: RefCell<bool>,
}

impl HeadlessLifecycle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RefCell::new(LifecycleState::default()),
            entered_background: RefCell::new(false),
        }
    }

    /// Put the application into a state.
    pub fn set_state(&self, state: LifecycleState) {
        let mut current = self.state.borrow_mut();
        if current.is_visible() && !state.is_visible() {
            *self.entered_background.borrow_mut() = true;
        }
        *current = state;
    }

    /// Acknowledge that the background entry has been handled.
    pub fn clear_background_flag(&self) {
        *self.entered_background.borrow_mut() = false;
    }
}

impl Default for HeadlessLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl AppLifecycle for HeadlessLifecycle {
    fn state(&self) -> LifecycleState {
        *self.state.borrow()
    }

    fn did_enter_background(&self) -> bool {
        *self.entered_background.borrow()
    }
}

// ------------------------------------------------------------------
// Accessibility announcements
// ------------------------------------------------------------------

/// A screen-reader announcement the framework wants delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The text to speak.
    pub message: String,
    /// If `true`, interrupts whatever is currently being spoken.
    pub assertive: bool,
}

/// Accessibility announcements, as a service.
///
/// The framework produces [`Announcement`] values (e.g. "3 items selected").
/// How they reach the screen reader is a platform concern, registered here
/// so the framework can produce them without knowing what is listening.
///
/// In a test, [`RecordingAnnouncements`] captures them for assertion.
pub trait AccessibilityAnnouncements: 'static {
    /// Deliver an announcement to the screen reader.
    fn announce(&self, announcement: Announcement);
}

/// Announcements that are kept rather than spoken.
///
/// A test asserts on these the same way it asserts on telemetry events:
/// "this button announced its selection count" is a claim that silently
/// stops being true.
#[derive(Debug, Default)]
pub struct RecordingAnnouncements {
    announcements: RefCell<Vec<Announcement>>,
}

impl RecordingAnnouncements {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every announcement delivered, in order.
    #[must_use]
    pub fn announcements(&self) -> Vec<Announcement> {
        self.announcements.borrow().clone()
    }

    /// Clear the recorded announcements.
    pub fn clear(&self) {
        self.announcements.borrow_mut().clear();
    }
}

impl AccessibilityAnnouncements for RecordingAnnouncements {
    fn announce(&self, announcement: Announcement) {
        self.announcements.borrow_mut().push(announcement);
    }
}

/// Register a headless implementation of all five.
pub fn provide_headless(services: &mut Services) {
    services.provide::<dyn FeatureFlags>(Rc::new(InMemoryFlags::new()));
    services.provide::<dyn Telemetry>(Rc::new(RecordingTelemetry::new()));
    services.provide::<dyn UpdateChannel>(Rc::new(ScriptedUpdates::up_to_date()));
    services.provide::<dyn AppLifecycle>(Rc::new(HeadlessLifecycle::new()));
    services.provide::<dyn AccessibilityAnnouncements>(Rc::new(RecordingAnnouncements::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_flag_answers_the_callers_default() {
        // The first-launch-offline case, which is the one that decides whether
        // an application renders at all before its flags arrive.
        let flags = InMemoryFlags::new();
        assert!(!flags.bool_flag("new-checkout", false));
        assert!(flags.bool_flag("new-checkout", true));
        assert_eq!(flags.text_flag("banner", "none"), "none");
        assert!((flags.number_flag("rollout", 0.25) - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn a_flag_of_the_wrong_type_falls_back_rather_than_guessing() {
        // The alternative is coercion — "true" as a `Bool`, `1.0` as `true` —
        // and coercion means a misconfigured flag silently enables a feature.
        let flags = InMemoryFlags::new();
        flags.set("new-checkout", FlagValue::Text("yes".into()));
        assert!(
            !flags.bool_flag("new-checkout", false),
            "text is not a boolean, and pretending otherwise ships the feature"
        );
    }

    #[test]
    fn a_flag_that_is_set_is_read_back() {
        let flags = InMemoryFlags::new();
        flags.set("new-checkout", FlagValue::Bool(true));
        assert!(flags.bool_flag("new-checkout", false));
        flags.clear("new-checkout");
        assert!(
            !flags.bool_flag("new-checkout", false),
            "an experiment that ended goes back to the default"
        );
    }

    #[test]
    fn telemetry_can_be_asserted_on() {
        // The reason the fake records instead of discarding: "this button
        // reports its event" is a claim that silently stops being true.
        let telemetry = RecordingTelemetry::new();
        telemetry.record(TelemetryEvent::new("checkout_started").with("cart_size", "3"));
        telemetry.log(LogLevel::Warn, "retrying");
        telemetry.log(LogLevel::Debug, "cache miss");

        let events = telemetry.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "checkout_started");
        assert_eq!(events[0].properties[0].1, "3");

        assert_eq!(
            telemetry.logs_at_least(LogLevel::Warn).len(),
            1,
            "a crash report attaches the serious lines, not every line"
        );
    }

    #[test]
    fn an_update_is_ready_rather_than_applied() {
        // The contract: fetching stages, and nothing swaps code under a running
        // frame.
        let updates = ScriptedUpdates::offering("1.4.0");
        assert_eq!(updates.state(), UpdateState::UpToDate);

        updates.fetch(Rc::new(|_| {}));
        assert_eq!(
            updates.state(),
            UpdateState::Ready {
                version: "1.4.0".into()
            }
        );
        assert_eq!(updates.fetches(), 1);
    }

    #[test]
    fn a_fetch_that_finds_nothing_says_so() {
        let updates = ScriptedUpdates::up_to_date();
        let seen = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        updates.fetch(Rc::new(move |result| {
            *sink.borrow_mut() = result.ok();
        }));
        assert_eq!(seen.borrow().clone(), Some(UpdateState::UpToDate));
    }

    #[test]
    fn none_of_the_five_needed_a_framework_change() {
        // §A's actual claim, on the five rows most likely to want a special
        // case. If this file ever has to reach into `Services` for a named field
        // rather than registering by type, the seam has stopped being one.
        let mut services = Services::new();
        provide_headless(&mut services);

        assert!(services.get::<dyn FeatureFlags>().is_some());
        assert!(services.get::<dyn Telemetry>().is_some());
        assert!(services.get::<dyn UpdateChannel>().is_some());
        assert!(services.get::<dyn AppLifecycle>().is_some());
        assert!(services.get::<dyn AccessibilityAnnouncements>().is_some());
    }

    #[test]
    fn a_lifecycle_starts_detached() {
        let lifecycle = HeadlessLifecycle::new();
        assert_eq!(lifecycle.state(), LifecycleState::Detached);
        assert!(!lifecycle.did_enter_background());
    }

    #[test]
    fn going_background_sets_the_flag() {
        let lifecycle = HeadlessLifecycle::new();
        lifecycle.set_state(LifecycleState::Resumed);
        assert!(!lifecycle.did_enter_background());

        lifecycle.set_state(LifecycleState::Hidden);
        assert!(lifecycle.did_enter_background());

        lifecycle.clear_background_flag();
        assert!(!lifecycle.did_enter_background());
    }

    #[test]
    fn going_hidden_then_paused_does_not_double_flag() {
        let lifecycle = HeadlessLifecycle::new();
        lifecycle.set_state(LifecycleState::Resumed);
        lifecycle.set_state(LifecycleState::Hidden);
        lifecycle.set_state(LifecycleState::Paused);
        assert!(lifecycle.did_enter_background());
    }

    #[test]
    fn announcements_can_be_asserted_on() {
        let a11y = RecordingAnnouncements::new();
        a11y.announce(Announcement {
            message: "3 items selected".into(),
            assertive: true,
        });
        a11y.announce(Announcement {
            message: "Tab opened".into(),
            assertive: false,
        });
        let list = a11y.announcements();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].message, "3 items selected");
        assert!(list[0].assertive);
        assert!(!list[1].assertive);

        a11y.clear();
        assert!(a11y.announcements().is_empty());
    }
}
