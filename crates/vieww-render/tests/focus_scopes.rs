//! A modal traps the keyboard, and gives it back when it closes.
//!
//! # What these pin, and why none of them could have been written before
//!
//! The framework carried two focus systems. `vieww-foundation::focus` was a
//! complete scope-based `FocusManager` — register, traverse, `push_scope`,
//! `pop_scope`, `is_trapped` — with tests, and **nothing in the framework ever
//! called it**. The live system was `vieww-render::focus`, keyed by `RenderId`
//! because focus needs the geometry and the paint order only the render tree
//! has, and it had no scopes at all. Its own module docs said so: *"nothing here
//! has scopes yet ... when scopes arrive this is the thing that grows."*
//!
//! So dialogs could not trap focus. Three Tabs from a dialog's last button put
//! the keyboard in a field on the screen behind, under a scrim, with a caret
//! nobody could see — and reading the foundation module gave an entirely
//! accurate description of a feature that did not exist.
//!
//! The scopes are here now, declared by the tree rather than pushed onto a
//! stack, and the foundation copy is gone.

use vieww_element::Signal;
use vieww_foundation::{Constraints, Key, Offset, PointerEvent, PointerId, Size};
use vieww_render::{FrameDriver, RenderId, RenderObject};
use vieww_widget::prelude::*;
use vieww_widget::{widget_node_from, FocusTrap, Stack, StackFit, WidgetKind};

// ---------------------------------------------------------------- a focusable

/// A focusable box of a known size, so a test can name one and tap it.
///
/// The name is for the reader of a failing assertion rather than for the code —
/// `behind-2` in a tree dump is what makes "Tab reached the screen behind the
/// dialog" legible.
#[derive(Debug, Clone)]
struct Field {
    #[expect(dead_code, reason = "names the field in a tree dump")]
    name: &'static str,
}

impl Widget for Field {
    fn debug_name(&self) -> &'static str {
        "Field"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
    fn key(&self) -> Option<&Key> {
        None
    }
}

widget_node_from!(Field);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderField;

impl RenderObject for RenderField {
    fn layout(&mut self, _ctx: &mut vieww_render::LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(Size::new(100.0, 40.0))
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        true
    }

    fn layout_differs(&self, new: &dyn RenderObject) -> bool {
        vieww_render::layout_differs_by_eq(self, new)
    }

    fn debug_name(&self) -> &'static str {
        "RenderField"
    }
}

// ------------------------------------------------------------------ the trees

/// Three fields down the screen: nothing modal.
fn plain() -> WidgetNode {
    Flex::column()
        .children(children![
            Field { name: "behind-1" },
            Field { name: "behind-2" },
            Field { name: "behind-3" },
        ])
        .into()
}

/// The same three fields, with a two-field modal over them.
fn with_modal() -> WidgetNode {
    Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            plain(),
            FocusTrap::new(true).child(Flex::column().children(children![
                Field { name: "modal-1" },
                Field { name: "modal-2" }
            ])),
        ])
        .into()
}

fn driver(root: WidgetNode) -> FrameDriver {
    let mut driver = FrameDriver::new(Size::new(400.0, 600.0));
    driver.register::<Field, _>(|_| RenderField);
    driver.set_root(root);
    driver.draw_frame();
    driver
}

/// How many distinct objects Tab reaches before it comes back to where it
/// started.
fn tab_cycle(driver: &mut FrameDriver) -> Vec<RenderId> {
    let mut seen = Vec::new();
    for _ in 0..12 {
        driver.focus_next(true);
        match driver.focused() {
            Some(id) if seen.contains(&id) => break,
            Some(id) => seen.push(id),
            None => break,
        }
    }
    seen
}

// -------------------------------------------------------------------- the tests

#[test]
fn without_a_modal_tab_reaches_every_field() {
    let mut driver = driver(plain());
    assert_eq!(
        tab_cycle(&mut driver).len(),
        3,
        "three focusable fields, all reachable"
    );
    assert!(!driver.focus().is_trapped());
}

/// **The defect.** Tab used to walk straight out of the dialog and into the
/// screen behind it.
#[test]
fn a_modal_confines_tab_to_its_own_subtree() {
    let mut driver = driver(with_modal());
    assert!(
        driver.focus().is_trapped(),
        "the trap is derived from the tree"
    );

    let cycle = tab_cycle(&mut driver);
    assert_eq!(
        cycle.len(),
        2,
        "Tab wraps between the modal's two fields and never reaches the three \
         behind it; it visited {} objects",
        cycle.len()
    );
}

/// A press on the screen behind a modal moves no focus — not into the thing
/// pressed, and not to nothing either. A press on a scrim should not empty the
/// dialog's keyboard.
#[test]
fn a_press_outside_the_modal_moves_no_focus() {
    let mut driver = driver(with_modal());
    driver.focus_next(true);
    let inside = driver.focused().expect("focus landed in the modal");

    // The bottom of the screen, where the third field behind the modal is and
    // the two-field modal is not.
    driver.handle_pointer(&PointerEvent::down(
        PointerId(1),
        Offset::new(50.0, 550.0),
        std::time::Duration::ZERO,
    ));

    assert_eq!(
        driver.focused(),
        Some(inside),
        "the keyboard stays where the modal put it"
    );
}

/// Closing the modal gives the keyboard back to whatever had it — the whole
/// reason a scope remembers anything.
///
/// Driven by a **signal** rather than by `remount`, which matters: a remount
/// rebuilds the render tree from scratch and hands out fresh ids, so the field
/// that had focus is not the same object afterwards and there would be nothing
/// left to restore *to*. A real modal does not do that — a navigator pushing a
/// route leaves the screen behind it mounted — and reconciliation is what keeps
/// the id stable across the frames the dialog opens and closes on.
#[test]
fn closing_a_modal_restores_the_focus_it_interrupted() {
    /// A screen with a modal that opens and closes without the tree being
    /// rebuilt around it.
    #[derive(Debug, Clone)]
    struct Screen {
        open: Signal<bool>,
    }

    impl Widget for Screen {
        fn debug_name(&self) -> &'static str {
            "Screen"
        }
        fn kind(&self) -> WidgetKind<'_> {
            WidgetKind::Composed
        }
        fn build(&self, _ctx: &BuildContext) -> WidgetNode {
            Stack::new()
                .fit(StackFit::Expand)
                .children(children![
                    plain(),
                    FocusTrap::new(self.open.get()).child(Field { name: "modal-1" }),
                ])
                .into()
        }
    }

    widget_node_from!(Screen);

    let mut driver = FrameDriver::new(Size::new(400.0, 600.0));
    driver.register::<Field, _>(|_| RenderField);
    let open = driver.elements().runtime().signal(false);
    driver.set_root(Screen { open: open.clone() });
    driver.draw_frame();

    driver.focus_next(true);
    driver.focus_next(true);
    let before = driver.focused().expect("something behind has focus");

    open.set(true);
    driver.draw_frame();
    assert!(driver.focus().is_trapped());
    assert_ne!(
        driver.focused(),
        Some(before),
        "a focus left standing outside a new trap is dropped: a caret still \
         blinking in a field behind a modal is the visible half of this bug"
    );

    open.set(false);
    driver.draw_frame();
    assert!(!driver.focus().is_trapped());
    assert_eq!(
        driver.focused(),
        Some(before),
        "and the field that had the keyboard before the dialog opened gets it back"
    );
}

/// The trap is the subtree, so it cannot outlive the subtree. This is the case
/// an imperative push/pop stack gets wrong: a dialog dismissed from inside
/// itself never runs its own close path, and the trap it pushed would survive it
/// — leaving focus stuck for the rest of the session with nothing on screen to
/// explain why.
#[test]
fn a_modal_that_vanishes_without_closing_leaves_no_trap_behind() {
    let mut driver = driver(with_modal());
    assert!(driver.focus().is_trapped());

    // The dialog is simply gone from the next tree — no close path ran.
    driver.remount(plain());
    driver.draw_frame();

    assert!(
        !driver.focus().is_trapped(),
        "the trap went with the subtree that declared it"
    );
    assert_eq!(
        tab_cycle(&mut driver).len(),
        3,
        "and the keyboard can reach the whole screen again"
    );
}

/// A trap declared `false` is the same as not being there, so a modal that
/// becomes non-modal is a property change rather than a reparent.
#[test]
fn an_untrapping_trap_confines_nothing() {
    let root: WidgetNode = Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            plain(),
            FocusTrap::new(false).child(Field { name: "popover" }),
        ])
        .into();
    let mut driver = driver(root);
    assert!(!driver.focus().is_trapped());
    assert_eq!(tab_cycle(&mut driver).len(), 4);
}

/// A dialog above a dialog is ordinary: the innermost trap is the one in force.
#[test]
fn the_innermost_modal_is_the_one_that_traps() {
    let root: WidgetNode = Stack::new()
        .fit(StackFit::Expand)
        .children(children![
            plain(),
            FocusTrap::new(true).child(Stack::new().fit(StackFit::Expand).children(children![
                Flex::column().children(children![
                    Field { name: "outer-1" },
                    Field { name: "outer-2" },
                ]),
                FocusTrap::new(true).child(Field { name: "inner" }),
            ])),
        ])
        .into();
    let mut driver = driver(root);
    assert_eq!(
        tab_cycle(&mut driver).len(),
        1,
        "only the inner dialog's one field is reachable"
    );
}
