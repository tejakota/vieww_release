//! A panicking `build` is contained rather than fatal.
//!
//! ```console
//! cargo test -p vieww-element --test error_boundary
//! ```
//!
//! Phase 10. The unit under test is `ElementTree::run_build`, which is the only
//! place any widget's `build` runs — so mounting and rebuilding are the same
//! code path and both are exercised here.
//!
//! Every test sets [`ErrorPolicy`] explicitly. The default is profile-dependent
//! by design, and an assertion that quietly means something different under
//! `cargo test --release` is worse than no assertion.

use std::cell::Cell;
use std::panic;
use std::rc::Rc;

use vieww_element::{ElementTree, ErrorPolicy};
use vieww_widget::prelude::*;
use vieww_widget::ErrorPlaceholder;

/// A tree with an explicit error policy.
///
/// # Do not install a silent panic hook here
///
/// Every test in this file panics on purpose, which makes suppressing the panic
/// hook look like an obvious tidy-up. An earlier version of this file did it.
/// It was wrong twice over:
///
/// - **It was unnecessary.** libtest captures each test's output and prints it
///   only if that test fails, so a passing run is already silent.
/// - **It was harmful.** A *failing* assertion is itself a panic, reported
///   through the same hook — so silencing it turns any real failure in this
///   file into a bare `FAILED` with no message and nothing to debug.
fn tree(policy: ErrorPolicy) -> ElementTree {
    let mut tree = ElementTree::new();
    tree.set_error_policy(policy);
    tree
}

/// A composed widget that panics while it is armed, and builds a `Text` once it
/// is not. The `Rc<Cell<_>>` is how a test disarms it between frames without
/// changing the widget's type or key, so the element is reused rather than
/// replaced.
#[derive(Debug, Clone)]
struct Explodes {
    armed: Rc<Cell<bool>>,
    message: &'static str,
}

impl Explodes {
    fn new(armed: &Rc<Cell<bool>>) -> Self {
        Self {
            armed: Rc::clone(armed),
            message: "widget exploded",
        }
    }

    fn always() -> Self {
        Self::new(&Rc::new(Cell::new(true)))
    }

    fn message(mut self, message: &'static str) -> Self {
        self.message = message;
        self
    }
}

impl Widget for Explodes {
    fn debug_name(&self) -> &'static str {
        "Explodes"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        assert!(!self.armed.get(), "{}", self.message);
        Text::new("recovered").into()
    }
}

widget_node_from!(Explodes);

/// Reads a signal during build, so a test can prove the reactive graph still
/// attributes reads to the right element after a panic went past.
#[derive(Debug, Clone)]
struct Reader {
    label: vieww_element::Signal<String>,
}

impl Widget for Reader {
    fn debug_name(&self) -> &'static str {
        "Reader"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(self.label.get()).into()
    }
}

widget_node_from!(Reader);

#[test]
fn a_panicking_build_mounts_a_placeholder_instead_of_unwinding() {
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::always());

    assert!(
        tree.find("ErrorPlaceholder").is_some(),
        "the placeholder stands in for what the build would have returned"
    );
    // The failed element itself is still mounted — what was replaced is its
    // output, not the element.
    assert!(tree.find("Explodes").is_some());
}

#[test]
fn the_error_records_the_widget_that_failed_and_what_it_said() {
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::always().message("the specific message"));

    let errors = tree.build_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].widget_name, "Explodes");
    assert!(
        errors[0].message.contains("the specific message"),
        "got {:?}",
        errors[0].message
    );
    assert_eq!(
        tree.get(errors[0].element)
            .map(vieww_element::Element::debug_name),
        Some("Explodes"),
        "the recorded element is the one that failed"
    );
}

#[test]
fn the_placeholder_carries_the_message_for_a_tree_dump() {
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::always().message("shown in the dump"));

    let placeholder = tree.find("ErrorPlaceholder").expect("mounted");
    let widget = placeholder
        .widget()
        .downcast_ref::<ErrorPlaceholder>()
        .expect("is an ErrorPlaceholder");

    assert_eq!(widget.widget_name(), "Explodes");
    assert!(widget.message().contains("shown in the dump"));
}

#[test]
fn one_broken_widget_does_not_take_its_siblings_with_it() {
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Flex::column().children(children![
        Text::new("before"),
        Explodes::always(),
        Text::new("after"),
    ]));

    // Both siblings built. Without the guard the panic would have unwound out
    // of `mount`, and the `Text` after it would never have been reached.
    assert_eq!(
        tree.find_all("Text").len(),
        2,
        "the sibling after the failure still built"
    );
    assert_eq!(tree.find_all("ErrorPlaceholder").len(), 1);
    assert_eq!(tree.build_errors().len(), 1);
}

#[test]
fn a_widget_that_always_panics_reports_once_rather_than_recursing() {
    // The placeholder is mounted like any other widget, so its own build runs.
    // If that build were also guarded, a substitute for a substitute would
    // recurse until the stack ran out. `catches_panics` excludes it, and the
    // observable consequence is exactly one error.
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::always());

    assert_eq!(tree.build_errors().len(), 1);
    assert_eq!(tree.find_all("ErrorPlaceholder").len(), 1);
}

/// An application's own placeholder — branding, wording, a report button.
///
/// Declares `catches_panics() -> false` exactly as `ErrorPlaceholder` does,
/// which is the protection that used to be a downcast to vieww's own type.
#[derive(Debug)]
struct Apology {
    failed: &'static str,
}

impl Widget for Apology {
    fn debug_name(&self) -> &'static str {
        "Apology"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        Text::new(format!("sorry about {}", self.failed)).into()
    }

    fn catches_panics(&self) -> bool {
        false
    }
}

widget_node_from!(Apology);

/// A placeholder that is itself broken, to prove the guard is not vieww's alone.
#[derive(Debug)]
struct BrokenApology;

impl Widget for BrokenApology {
    fn debug_name(&self) -> &'static str {
        "BrokenApology"
    }

    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::Composed
    }

    fn build(&self, _ctx: &BuildContext) -> WidgetNode {
        panic!("the apology exploded too");
    }

    fn catches_panics(&self) -> bool {
        false
    }
}

widget_node_from!(BrokenApology);

#[test]
fn an_application_can_supply_its_own_placeholder() {
    // **The gap this closes.** The tree constructed `ErrorPlaceholder` directly
    // and recognised it by type on the way back, so an application had no way to
    // brand the failure, word it, or offer to report it.
    let mut tree = tree(ErrorPolicy::Custom(|error| {
        Apology {
            failed: error.widget_name,
        }
        .into()
    }));
    tree.mount(Explodes::always());

    assert_eq!(tree.build_errors().len(), 1, "still recorded");
    assert_eq!(tree.find_all("Apology").len(), 1);
    assert!(
        tree.find_all("ErrorPlaceholder").is_empty(),
        "vieww's own placeholder must not appear when one was supplied"
    );
}

#[test]
fn a_custom_placeholder_is_told_which_widget_failed() {
    let mut tree = tree(ErrorPolicy::Custom(|error| {
        Apology {
            failed: error.widget_name,
        }
        .into()
    }));
    tree.mount(Explodes::always().message("boom"));

    let error = &tree.build_errors()[0];
    assert_eq!(error.widget_name, "Explodes");
    assert_eq!(error.message, "boom");
}

#[test]
fn the_recursion_guard_protects_a_third_party_placeholder_too() {
    // **The half that was silently missing.** The guard used to be a downcast to
    // `ErrorPlaceholder`, so a placeholder of somebody else's that panicked was
    // caught, replaced by another of itself, caught again — until the stack ran
    // out. A stack overflow instead of a message, which is worse than the crash
    // the whole feature exists to prevent.
    //
    // Reaching the assertion at all is the result: with the old guard this test
    // does not fail, it dies.
    let mut tree = tree(ErrorPolicy::Custom(|_| BrokenApology.into()));
    let caught = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        tree.mount(Explodes::always());
    }));

    assert!(
        caught.is_err(),
        "a placeholder that panics propagates rather than recursing"
    );
}

#[test]
fn a_panic_during_a_rebuild_is_caught_too() {
    let armed = Rc::new(Cell::new(false));
    let mut tree = tree(ErrorPolicy::Placeholder);
    let label = tree.runtime().signal(String::from("first"));

    let root = tree.mount(Flex::column().children(children![
        Explodes::new(&armed),
        Reader {
            label: label.clone()
        },
    ]));
    assert!(
        tree.build_errors().is_empty(),
        "builds cleanly while disarmed"
    );

    // Arm it, then force that element to rebuild.
    armed.set(true);
    let explodes = tree.find("Explodes").expect("mounted").id();
    tree.mark_pending(explodes);
    tree.rebuild_pending();

    assert_eq!(tree.build_errors().len(), 1);
    assert!(tree.find("ErrorPlaceholder").is_some());
    assert!(tree.is_alive(root));
}

#[test]
fn a_widget_that_stops_panicking_replaces_its_own_placeholder() {
    let armed = Rc::new(Cell::new(true));
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::new(&armed));
    assert!(tree.find("ErrorPlaceholder").is_some());

    // Nothing about the element changed except that its build now succeeds.
    // The placeholder is reconciled away like any other stale child, which is
    // what makes this recoverable rather than a one-way door.
    armed.set(false);
    let explodes = tree.find("Explodes").expect("mounted").id();
    tree.mark_pending(explodes);
    tree.rebuild_pending();

    assert!(
        tree.find("ErrorPlaceholder").is_none(),
        "the placeholder is gone once the build succeeds"
    );
    assert!(tree.find("Text").is_some());
}

#[test]
fn a_panic_does_not_leave_the_tracking_stack_holding_a_stale_element() {
    // The regression this exists for. `push_tracking` and `pop_tracking` sit
    // either side of user code, so an unwind skips the pop and leaves the
    // failed element on the stack.
    //
    // The read that exposes it is one from *outside* a build. A read from
    // inside a later build resolves to `last()`, which is that build's own
    // frame, so it stays correct even with rubbish underneath it — which is
    // exactly why this went unnoticed until something caught the panic. A read
    // with no build in progress is supposed to subscribe nothing at all
    // (`signal::tests::a_read_outside_a_build_subscribes_nothing`); with a
    // leaked frame it silently subscribes the element that died.
    let mut tree = tree(ErrorPolicy::Placeholder);
    let label = tree.runtime().signal(String::from("first"));

    tree.mount(Flex::column().children(children![
        Explodes::always(),
        Reader {
            label: label.clone()
        },
    ]));

    // Test code, not a build. This must record nothing.
    let _ = label.get();

    label.set(String::from("second"));
    assert_eq!(
        tree.pending_count(),
        1,
        "only the Reader read this signal during a build — a second pending \
         element means the read above was attributed to the one that panicked"
    );
    assert_eq!(tree.rebuild_pending(), 1);
}

#[test]
fn taking_the_errors_leaves_the_list_empty() {
    let mut tree = tree(ErrorPolicy::Placeholder);
    tree.mount(Explodes::always());

    let taken = tree.take_build_errors();
    assert_eq!(taken.len(), 1);
    assert!(
        tree.build_errors().is_empty(),
        "a reporter that has shown a failure should not show it again next frame"
    );
}

#[test]
fn propagate_lets_the_panic_out() {
    let result = panic::catch_unwind(|| {
        let mut tree = tree(ErrorPolicy::Propagate);
        tree.mount(Explodes::always());
    });

    assert!(
        result.is_err(),
        "under Propagate the panic is the caller's problem, not the tree's"
    );
}

#[test]
fn the_default_policy_follows_the_build_profile() {
    let expected = if cfg!(debug_assertions) {
        ErrorPolicy::Placeholder
    } else {
        ErrorPolicy::Propagate
    };
    assert_eq!(ElementTree::new().error_policy(), expected);
}
