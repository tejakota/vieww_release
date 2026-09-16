//! A render object that panics does not take the process with it.
//!
//! `ElementTree::build`'s `catch_unwind` covers `widget.build()`. It does not
//! cover the render tree, which calls `layout`, `paint` and `hit_test` on
//! render objects directly — so a `CustomPainter::paint` that divides by zero
//! used to abort the studio with no red box and no Problems entry.
//!
//! The per-node catches in `RenderTree` were added with no test of their own,
//! which is the same as not having them: nothing here would notice if a future
//! edit dropped one, and an aborted process is not a failing assertion.
//!
//! Each test below panics in one phase and asserts three things: the process is
//! still running, the sink was told once, and the *sibling* still drew — a
//! catch that skipped the rest of the subtree would be a hole in the frame
//! rather than one missing node.

use std::cell::RefCell;
use std::rc::Rc;

use vieww_foundation::Axis;
use vieww_foundation::{Color, Constraints, Offset, Rect, Size};
use vieww_render::{
    set_render_panic_sink, LayoutCtx, PaintCtx, RenderColoredBox, RenderConstrainedBox, RenderFlex,
    RenderObject, RenderTree, Scene,
};

/// Which phase this object blows up in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boom {
    Layout,
    Paint,
    HitTest,
}

/// A render object that panics in exactly one phase and behaves in the others.
#[derive(Debug)]
struct Detonator {
    when: Boom,
}

impl RenderObject for Detonator {
    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        assert!(self.when != Boom::Layout, "this is what user code does");
        constraints.constrain(Size::new(10.0, 10.0))
    }

    fn paint(&self, _ctx: &mut PaintCtx<'_>) {
        assert!(self.when != Boom::Paint, "this is what user code does");
    }

    fn hit_test_self(&self, _point: Offset, _size: Size) -> bool {
        assert!(self.when != Boom::HitTest, "this is what user code does");
        true
    }

    fn debug_name(&self) -> &'static str {
        "Detonator"
    }
}

/// Collect what the sink is told, and put it back when the test ends — the sink
/// is a thread-local and the tests in one binary share a thread.
struct Sink {
    seen: Rc<RefCell<Vec<(String, String)>>>,
}

impl Sink {
    fn install() -> Self {
        let seen: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        set_render_panic_sink(Some(Box::new(move |name, message| {
            sink.borrow_mut()
                .push((name.to_string(), message.to_string()));
        })));
        Self { seen }
    }

    fn names(&self) -> Vec<String> {
        self.seen.borrow().iter().map(|(n, _)| n.clone()).collect()
    }

    fn messages(&self) -> Vec<String> {
        self.seen.borrow().iter().map(|(_, m)| m.clone()).collect()
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        set_render_panic_sink(None);
    }
}

/// A row with the detonator first and a red 20×20 box second, so every test can
/// ask whether the sibling survived.
fn tree_with(detonator: Boom) -> RenderTree {
    let mut tree = RenderTree::new();
    let row = tree.insert(None, Box::new(RenderFlex::new(Axis::Horizontal)));
    tree.insert(Some(row), Box::new(Detonator { when: detonator }));
    let sibling = tree.insert(Some(row), Box::new(RenderColoredBox::new(Color::RED)));
    tree.insert(
        Some(sibling),
        Box::new(RenderConstrainedBox::new(Constraints::tight(Size::new(
            20.0, 20.0,
        )))),
    );
    tree
}

/// The panic is silenced by the harness's own hook so the test output is
/// readable; the abort this is guarding against would not be silenced by it.
fn quietly<R>(body: impl FnOnce() -> R) -> R {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = body();
    std::panic::set_hook(previous);
    out
}

#[test]
fn a_render_object_that_panics_in_layout_does_not_abort_the_process() {
    let sink = Sink::install();
    let mut tree = tree_with(Boom::Layout);

    quietly(|| {
        tree.layout_root(Constraints::loose(Size::new(200.0, 200.0)));
    });

    assert_eq!(sink.names(), vec!["Detonator"], "the sink is told once");
    assert!(
        sink.messages()[0].contains("layout"),
        "and told which phase: {}",
        sink.messages()[0]
    );

    // The sibling still laid out and still draws.
    let mut scene = Scene::new();
    tree.paint(&mut scene);
    assert!(
        scene
            .fills()
            .into_iter()
            .any(|(rect, paint)| paint.color == Color::RED && rect.width() == 20.0),
        "the sibling of a panicking node still reaches the canvas"
    );
}

#[test]
fn a_render_object_that_panics_in_paint_does_not_abort_the_process() {
    let sink = Sink::install();
    let mut tree = tree_with(Boom::Paint);
    tree.layout_root(Constraints::loose(Size::new(200.0, 200.0)));

    let mut scene = Scene::new();
    quietly(|| tree.paint(&mut scene));

    assert_eq!(sink.names(), vec!["Detonator"]);
    assert!(sink.messages()[0].contains("paint"));
    assert!(
        scene
            .fills()
            .into_iter()
            .any(|(_, paint)| paint.color == Color::RED),
        "the rest of the frame is still drawn"
    );
}

#[test]
fn a_render_object_that_panics_in_hit_test_does_not_abort_the_process() {
    let sink = Sink::install();
    let mut tree = tree_with(Boom::HitTest);
    tree.layout_root(Constraints::loose(Size::new(200.0, 200.0)));

    quietly(|| {
        let _ = tree.hit_test(Offset::new(5.0, 5.0));
    });

    assert_eq!(sink.names(), vec!["Detonator"]);
    assert!(sink.messages()[0].contains("hit_test"));
}

#[test]
fn the_sink_hears_once_rather_than_once_per_frame() {
    let sink = Sink::install();
    let mut tree = tree_with(Boom::Paint);
    tree.layout_root(Constraints::loose(Size::new(200.0, 200.0)));

    // Sixty frames is one second of a panicking `CustomPainter::paint`.
    quietly(|| {
        for _ in 0..60 {
            let mut scene = Scene::new();
            tree.paint(&mut scene);
        }
    });

    assert_eq!(
        sink.names().len(),
        1,
        "a per-frame panic is reported once, not sixty times"
    );
}

/// Without a sink the report still happens — it goes to stderr — and, more to
/// the point, the process still survives. A host that never installs a sink
/// (every test binary in this workspace, and the studio before startup
/// finishes) must not be the case that aborts.
#[test]
fn a_panic_with_no_sink_installed_still_does_not_abort() {
    set_render_panic_sink(None);
    let mut tree = tree_with(Boom::Paint);
    tree.layout_root(Constraints::loose(Size::new(200.0, 200.0)));
    let mut scene = Scene::new();
    quietly(|| tree.paint(&mut scene));
    // Reaching this line is the assertion.
    assert!(!Rect::new(0.0, 0.0, 1.0, 1.0).is_empty());
}
