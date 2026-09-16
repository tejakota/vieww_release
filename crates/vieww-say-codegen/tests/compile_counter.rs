//! The strongest assertion a code generator can make: the file it produced
//! compiles against vieww, hands back a widget, and the state behaves —
//! a snapshot survives a round trip, a restore refuses rubbish, and the
//! handler the button carries actually moves the counter.
//!
//! The generated file is `fixtures/counter_gen.rs`, produced by `saygen`
//! from `fixtures/counter.say` and committed, so a change in the generator
//! that changes the output shows up as a fixture diff in review.

mod counter {
    include!("fixtures/counter_gen.rs");

    /// The generated types are private to this module; the tests below are
    /// a child, which is how Rust lets them reach in without the generator
    /// having to make anything public just for testing.
    #[cfg(test)]
    mod behaviour {
        use super::*;

        #[test]
        fn snapshot_round_trips_the_counter() {
            let mut state = SayState {
                count: 41,
                ..SayState::default()
            };
            state.mark();
            let saved = state.snapshot().expect("the counter keeps state");
            let fresh = SayState::default();
            let mut restored = fresh;
            assert!(restored.restore(&saved), "a clean snapshot restores");
            assert_eq!(restored.count, 41);
        }

        #[test]
        fn restore_refuses_a_field_this_file_does_not_keep() {
            let mut state = SayState::default();
            let before = state.count;
            assert!(
                !state.restore("count=7;password=hunter2"),
                "an unknown field refuses the whole snapshot"
            );
            assert_eq!(state.count, before, "a refused snapshot changes nothing");
        }

        #[test]
        fn restore_refuses_garbage_without_half_applying() {
            let mut state = SayState {
                count: 5,
                ..SayState::default()
            };
            assert!(!state.restore("count=not_a_number"));
            assert_eq!(state.count, 5, "a refused snapshot leaves state alone");
        }

        #[test]
        fn take_pending_consumes_the_dirty_flag_once() {
            let mut state = SayState::default();
            assert!(!state.take_pending());
            state.mark();
            assert!(state.take_pending());
            assert!(!state.take_pending(), "the flag is taken, not read");
        }
    }
}

#[test]
fn the_counter_screen_exists_and_is_named() {
    use vieww::prelude::*;
    let app = counter::screen();
    assert_eq!(app.debug_name(), "SayApp");
}
