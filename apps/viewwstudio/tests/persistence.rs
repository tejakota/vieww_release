//! What the studio remembers between launches, and what it refuses to lose.
//!
//! These are integration tests rather than unit tests in `settings.rs` because
//! the thing under test is the *wiring*: the format has its own tests, and the
//! bug this whole area exists to fix was never in the format. It was that
//! nothing ever called it.

use std::rc::Rc;

use vieww_element::Runtime;
use vieww_foundation::{MemoryStorage, Services, SharedServices, Storage};
use viewwstudio::settings::{Session, Settings, RECENT_LIMIT};
use viewwstudio::state::{PanelTab, View};
use viewwstudio::Studio;

/// A studio with a store that lives in memory.
///
/// Deliberately *not* the platform store: a test that wrote to the real
/// settings file would change the editor of whoever ran it, and would then
/// pass or fail depending on what they had set.
fn studio_with_store() -> (Runtime, Studio, Rc<MemoryStorage>) {
    let runtime = Runtime::new();
    let store = Rc::new(MemoryStorage::new());
    let mut services = Services::new();
    services.provide::<dyn Storage>(Rc::clone(&store) as Rc<dyn Storage>);
    let studio = Studio::new(&runtime).with_services(SharedServices::new(services));
    (runtime, studio, store)
}

#[test]
fn a_studio_with_no_store_persists_nothing_and_does_not_mind() {
    let runtime = Runtime::new();
    let studio = Studio::new(&runtime);
    assert!(studio.storage().is_none());
    // The point of the test: none of these may panic on a studio built the way
    // every other test in this crate builds one.
    studio.save_settings();
    studio.save_session(Some((1200.0, 800.0)));
    studio.load_persisted();
}

#[test]
fn settings_survive_a_relaunch() {
    let (_runtime, studio, store) = studio_with_store();
    studio.dark.set(false);
    studio.font_size.set(16.0);
    studio.minimap.set(false);
    studio.word_wrap.set(true);
    studio.tab_width.set(2);
    studio.save_settings();

    // A second studio over the same store is what "launching again" means.
    let runtime = Runtime::new();
    let mut services = Services::new();
    services.provide::<dyn Storage>(store as Rc<dyn Storage>);
    let next = Studio::new(&runtime).with_services(SharedServices::new(services));
    assert!(next.dark.get(), "before loading, the defaults");
    next.load_persisted();

    assert!(!next.dark.get());
    assert!((next.font_size.get() - 16.0).abs() < f32::EPSILON);
    assert!(!next.minimap.get());
    assert!(next.word_wrap.get());
    assert_eq!(next.tab_width.get(), 2);
}

#[test]
fn the_activity_view_and_panel_tab_come_back() {
    let (_runtime, studio, store) = studio_with_store();
    studio.view.set(View::Source);
    studio.panel_tab.set(PanelTab::Timings);
    studio.save_session(None);

    let runtime = Runtime::new();
    let mut services = Services::new();
    services.provide::<dyn Storage>(store as Rc<dyn Storage>);
    let next = Studio::new(&runtime).with_services(SharedServices::new(services));
    next.load_persisted();
    assert_eq!(next.view.get(), View::Source);
    assert_eq!(next.panel_tab.get(), PanelTab::Timings);
}

/// The session file names views and tabs by name, and a name that no longer
/// exists must not take the rest of the session down with it.
#[test]
fn a_session_naming_a_view_this_studio_does_not_have_still_restores_the_rest() {
    let (_runtime, studio, _store) = studio_with_store();
    let session = Session {
        view: "Holograms".to_owned(),
        panel_tab: "Timings".to_owned(),
        ..Session::default()
    };
    studio.apply_session(&session);
    assert_eq!(studio.view.get(), View::Explorer, "left at its default");
    assert_eq!(
        studio.panel_tab.get(),
        PanelTab::Timings,
        "and the good one applied"
    );
}

#[test]
fn the_recent_list_is_most_recent_first_and_bounded() {
    let (_runtime, studio, _store) = studio_with_store();
    let dir = std::env::temp_dir();
    studio.remember_workspace(&dir);
    assert_eq!(studio.recent.get().len(), 1);
    // Re-opening the same folder moves it rather than adding to it.
    studio.remember_workspace(&dir);
    assert_eq!(studio.recent.get().len(), 1);

    let mut session = Session::default();
    for n in 0..RECENT_LIMIT + 3 {
        session.remember(std::path::Path::new(&format!("/p/{n}")));
    }
    assert_eq!(session.recent.len(), RECENT_LIMIT);
}

// ---------------------------------------------------------------------------
// The quit guard
// ---------------------------------------------------------------------------

#[test]
fn a_clean_studio_closes_without_asking() {
    let (_runtime, studio, _store) = studio_with_store();
    assert!(studio.unsaved_names().is_empty());
    assert!(studio.may_close(Some((1200.0, 800.0))));
    assert!(studio.quit_prompt.get().is_none());
}

/// The finding this whole batch exists for: before the guard, this close lost
/// the buffer with no dialog and no recovery file.
#[test]
fn a_dirty_studio_vetoes_the_close_and_says_what_would_be_lost() {
    let (_runtime, studio, _store) = studio_with_store();
    studio.edit(vieww_foundation::TextEditingValue::new("fn main() {}"));
    assert!(!studio.unsaved_names().is_empty(), "the edit made it dirty");

    assert!(!studio.may_close(None), "the window must not go");
    let prompt = studio.quit_prompt.get().expect("the dialog was raised");
    assert_eq!(prompt.len(), studio.unsaved_names().len());
}

#[test]
fn answering_the_dialog_lets_the_second_request_through() {
    let (_runtime, studio, _store) = studio_with_store();
    studio.edit(vieww_foundation::TextEditingValue::new("fn main() {}"));
    assert!(!studio.may_close(None));

    studio.confirm_quit_discarding();
    assert!(studio.quit_prompt.get().is_none(), "the dialog is gone");
    assert!(
        studio.may_close(None),
        "and the close it was asked about now goes through"
    );
}

#[test]
fn cancelling_the_dialog_leaves_the_studio_exactly_as_it_was() {
    let (_runtime, studio, _store) = studio_with_store();
    studio.edit(vieww_foundation::TextEditingValue::new("fn main() {}"));
    assert!(!studio.may_close(None));
    studio.cancel_quit();
    assert!(studio.quit_prompt.get().is_none());
    assert!(
        !studio.may_close(None),
        "cancelling is not consent — the next close asks again"
    );
}

#[test]
fn closing_writes_the_session_out() {
    let (_runtime, studio, store) = studio_with_store();
    studio.view.set(View::Export);
    assert!(studio.may_close(Some((1440.0, 900.0))));

    let text = store
        .get(viewwstudio::settings::SESSION_KEY)
        .expect("readable")
        .expect("written on the way out");
    let session = Session::parse(&text);
    assert_eq!(session.view, "Export");
    assert_eq!(session.window, Some((1440.0, 900.0)));
}

/// A settings poll writes when something moved and not otherwise. The first
/// call is the baseline: without that rule, every launch would stamp the file.
#[test]
fn the_settings_poll_writes_on_a_change_and_not_on_a_still_frame() {
    let (_runtime, studio, store) = studio_with_store();
    let last = std::cell::RefCell::new(None::<Settings>);

    studio.poll_settings(&last);
    assert!(
        store
            .get(viewwstudio::settings::SETTINGS_KEY)
            .unwrap()
            .is_none(),
        "the first poll is a baseline, not a write"
    );

    studio.poll_settings(&last);
    assert!(
        store
            .get(viewwstudio::settings::SETTINGS_KEY)
            .unwrap()
            .is_none(),
        "and a frame where nothing moved writes nothing"
    );

    studio.dark.set(false);
    studio.poll_settings(&last);
    let text = store
        .get(viewwstudio::settings::SETTINGS_KEY)
        .unwrap()
        .expect("a change is written");
    assert!(!Settings::parse(&text).dark);
}
