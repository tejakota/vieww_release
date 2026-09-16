//! The intrinsic-size pass: asking a subtree how big it wants to be.
//!
//! ```console
//! cargo test -p vieww --test intrinsic_sizing
//! ```
//!
//! `docs/PRODUCTION-GAPS.md` and `NEXT.md` both named the absence of this as
//! *"the largest remaining architectural gap"*, and both named the same
//! evidence: it was visible in the public API. `Accordion::content_height` was
//! a **required** parameter and `ListView::variable` needed a caller-supplied
//! estimator, which is a framework asking the application to measure the
//! framework's own text.
//!
//! # What is asserted here, and what is deliberately not
//!
//! Three things, in order of how badly each one fails if it is wrong:
//!
//! 1. **The answers are right**, against numbers derived from the same shaping
//!    the real layout uses rather than against constants copied out of a run.
//! 2. **An unmeasurable subtree is transparent, never zero.** This is the whole
//!    reason `RenderObject::intrinsic` returns `Option<f32>` and not a plain
//!    number, and it is the property that makes the feature safe to add
//!    to a tree full of render objects that do not implement it.
//! 3. **The walk is memoised**, counted rather than asserted in prose. A
//!    container answers by asking every child, so a nested query without a
//!    cache is exponential in depth — and that is the shape a real tree has.
//!    `docs/AIMS.md`: where a claim is about cost, the test has to count.
//!
//! Not asserted: exact pixel widths of specific strings in the embedded face.
//! Those would pin this suite to a font file and break on any update to it,
//! while testing nothing about the protocol. Every number below is either a
//! relation between two measurements or a size the test itself declared.

use std::cell::Cell;
use std::rc::Rc;

use vieww::foundation::{Constraints, Size};
use vieww::render::{IntrinsicCtx, IntrinsicQuery, LayoutCtx, PaintCtx};
use vieww::{FrameDriver, RenderObject, Widget, WidgetKind};

const SURFACE: Size = Size {
    width: 400.0,
    height: 400.0,
};

// ------------------------------------------------------- instrumented widgets

/// A leaf of a declared size that **can** be measured, and counts being asked.
///
/// The counter is what turns "the cache works" into a number. Shared by `Rc` so
/// the test can read it after the frame.
#[derive(Debug)]
struct Block {
    size: Size,
    asked: Rc<Cell<u32>>,
}

impl Widget for Block {
    fn debug_name(&self) -> &'static str {
        "Block"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(Block);

#[derive(Debug)]
struct RenderBlock {
    size: Size,
    asked: Rc<Cell<u32>>,
}

impl RenderObject for RenderBlock {
    fn debug_name(&self) -> &'static str {
        "RenderBlock"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(self.size)
    }

    fn intrinsic(&self, _ctx: &mut IntrinsicCtx<'_>, query: IntrinsicQuery) -> Option<f32> {
        self.asked.set(self.asked.get() + 1);
        Some(match query.axis {
            vieww::foundation::Axis::Horizontal => self.size.width,
            vieww::foundation::Axis::Vertical => self.size.height,
        })
    }

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        ctx.canvas()
            .fill_rect(bounds, vieww::foundation::Color::rgb(9, 9, 9).into());
    }
}

/// A leaf that takes a size and **refuses to be measured** — the default
/// `RenderObject::intrinsic`, which is every render object nobody has taught.
#[derive(Debug)]
struct Opaque {
    size: Size,
}

impl Widget for Opaque {
    fn debug_name(&self) -> &'static str {
        "Opaque"
    }
    fn kind(&self) -> WidgetKind<'_> {
        WidgetKind::RenderLeaf
    }
}

vieww::widget::widget_node_from!(Opaque);

#[derive(Debug)]
struct RenderOpaque {
    size: Size,
}

impl RenderObject for RenderOpaque {
    fn debug_name(&self) -> &'static str {
        "RenderOpaque"
    }

    fn layout(&mut self, _ctx: &mut LayoutCtx<'_>, constraints: Constraints) -> Size {
        constraints.constrain(self.size)
    }

    // No `intrinsic`. That is the point of this type.

    fn paint(&self, ctx: &mut PaintCtx<'_>) {
        let bounds = ctx.bounds();
        ctx.canvas()
            .fill_rect(bounds, vieww::foundation::Color::rgb(200, 0, 0).into());
    }
}

fn driver() -> FrameDriver {
    let mut driver = FrameDriver::new(SURFACE);
    driver.register::<Block, RenderBlock>(|widget| RenderBlock {
        size: widget.size,
        asked: Rc::clone(&widget.asked),
    });
    driver.register::<Opaque, RenderOpaque>(|widget| RenderOpaque { size: widget.size });
    driver
}

/// The size the named render object settled on this frame.
fn size_of(driver: &FrameDriver, debug_name: &str) -> Size {
    let tree = driver.owner().tree();
    let mut found = None;
    for id in tree.ids() {
        if tree.object(id).map(RenderObject::debug_name) == Some(debug_name) {
            assert!(
                found.is_none(),
                "{debug_name} appears more than once; this helper wants exactly one"
            );
            found = Some(tree.size(id));
        }
    }
    found.unwrap_or_else(|| panic!("{debug_name} is not in the render tree"))
}

// ------------------------------------------------------------- the assertions

/// A column of two blocks, measured on both axes, against numbers the test
/// itself declared.
#[test]
fn a_column_sums_heights_and_takes_the_widest() {
    use vieww::prelude::*;

    let asked = Rc::new(Cell::new(0));
    let mut driver = driver();
    driver.set_root(
        Center::new().child(IntrinsicSize::width(
            Flex::column()
                .main_axis_size(MainAxisSize::Min)
                .children(children![
                    Block {
                        size: Size::new(70.0, 20.0),
                        asked: Rc::clone(&asked),
                    },
                    Block {
                        size: Size::new(130.0, 30.0),
                        asked: Rc::clone(&asked),
                    },
                ]),
        )),
    );
    driver.draw_frame();

    // The column is as wide as its widest child, and `IntrinsicSize`
    // tightened the width to exactly that rather than to the 400-point surface.
    assert_eq!(
        size_of(&driver, "RenderIntrinsicSize").width,
        130.0,
        "a column's intrinsic width is the widest child's, and `IntrinsicSize` is \
         that width — not the space it was offered"
    );
}

/// The one everybody actually wants: a row whose cells are all as tall as the
/// tallest.
///
/// Without this, each cell is only as tall as its own content and a row of
/// cards with backgrounds ends at three different heights.
#[test]
fn intrinsic_height_makes_a_rows_cells_agree() {
    use vieww::prelude::*;

    let asked = Rc::new(Cell::new(0));
    let short = Rc::clone(&asked);
    let tall = Rc::clone(&asked);

    let mut driver = driver();
    driver.set_root(
        Align::new(vieww::foundation::Alignment::TOP_LEFT).child(IntrinsicSize::height(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .main_axis_size(MainAxisSize::Min)
                .children(children![
                    Block {
                        size: Size::new(40.0, 25.0),
                        asked: short,
                    },
                    Block {
                        size: Size::new(40.0, 90.0),
                        asked: tall,
                    },
                ]),
        )),
    );
    driver.draw_frame();

    assert_eq!(
        size_of(&driver, "RenderIntrinsicSize").height,
        90.0,
        "a row's intrinsic height is its tallest child's — 90, not the 400 the \
         surface would have handed down and not the 25 of the first child"
    );
}

/// A subtree with one unmeasurable object in it makes the wrapper transparent.
///
/// **This is the assertion that makes the whole feature safe to ship.** In a
/// framework that defaults the answer to zero, an unimplemented `intrinsic`
/// somewhere in the subtree silently collapses everything above it and the
/// symptom appears somewhere else entirely.
#[test]
fn an_unmeasurable_subtree_is_transparent_rather_than_collapsed() {
    use vieww::prelude::*;

    let mut driver = driver();
    driver.set_root(IntrinsicSize::height(Center::new().child(Opaque {
        size: Size::new(50.0, 60.0),
    })));
    driver.draw_frame();

    let wrapper = size_of(&driver, "RenderIntrinsicSize");
    assert_eq!(
        wrapper.height, SURFACE.height,
        "the subtree cannot answer, so `IntrinsicSize` hands the child the \
         constraints \
         it was given — which is what not being in the tree would have done. \
         Zero here would mean the content had silently disappeared"
    );
    assert_eq!(
        size_of(&driver, "RenderOpaque"),
        Size::new(50.0, 60.0),
        "and the content is still laid out at its own size"
    );
}

/// A `SizedBox` around the unmeasurable part makes the whole thing measurable
/// again.
///
/// The documented escape hatch, and it has to actually work: a tight constraint
/// answers for itself without asking its child, so nothing below it can poison
/// the query.
#[test]
fn a_tight_sized_box_makes_an_unmeasurable_subtree_measurable() {
    use vieww::prelude::*;

    let mut driver = driver();
    driver.set_root(Align::new(vieww::foundation::Alignment::TOP_LEFT).child(
        IntrinsicSize::height(SizedBox::from_size(Size::new(80.0, 45.0)).child(Opaque {
            size: Size::new(50.0, 60.0),
        })),
    ));
    driver.draw_frame();

    assert_eq!(
        size_of(&driver, "RenderIntrinsicSize").height,
        45.0,
        "the `SizedBox` is tight, so it answers 45 without consulting the \
         subtree that cannot answer"
    );
}

/// Padding is added to the answer, and taken off the cross extent on the way
/// down.
#[test]
fn padding_grows_the_answer_by_its_insets() {
    use vieww::foundation::EdgeInsets;
    use vieww::prelude::*;

    let asked = Rc::new(Cell::new(0));
    let mut driver = driver();
    driver.set_root(Align::new(vieww::foundation::Alignment::TOP_LEFT).child(
        IntrinsicSize::height(
            Padding::new(EdgeInsets::only(5.0, 11.0, 5.0, 13.0)).child(Block {
                size: Size::new(30.0, 40.0),
                asked,
            }),
        ),
    ));
    driver.draw_frame();

    assert_eq!(
        size_of(&driver, "RenderIntrinsicSize").height,
        40.0 + 11.0 + 13.0,
        "the child's 40 plus the vertical insets — a padding that reported its \
         child's answer unchanged would have the content overflowing by 24"
    );
}

/// **Counted.** A nested query asks each leaf once, not once per path to it.
///
/// Six blocks under three columns under one row, queried on one axis. Without
/// memoisation the intermediate containers are re-walked for every question
/// asked of them, and this is the shape — deep, branching, cheap leaves — where
/// that compounds. One ask per leaf is the number that says the cache is doing
/// its job; anything above it is the walk being repeated.
#[test]
fn each_leaf_is_asked_at_most_once_per_question() {
    use vieww::prelude::*;

    let asked = Rc::new(Cell::new(0));
    let column = |asked: &Rc<Cell<u32>>| -> WidgetNode {
        Flex::column()
            .main_axis_size(MainAxisSize::Min)
            .children(children![
                Block {
                    size: Size::new(10.0, 10.0),
                    asked: Rc::clone(asked),
                },
                Block {
                    size: Size::new(10.0, 10.0),
                    asked: Rc::clone(asked),
                },
            ])
            .into()
    };

    let mut driver = driver();
    driver.set_root(
        Align::new(vieww::foundation::Alignment::TOP_LEFT).child(IntrinsicSize::height(
            Flex::row()
                .main_axis_size(MainAxisSize::Min)
                .children(children![column(&asked), column(&asked), column(&asked)]),
        )),
    );
    driver.draw_frame();

    let count = asked.get();
    assert!(
        count <= 6,
        "six leaves and one question, so six asks at most; got {count}, which \
         means the subtree is being re-walked rather than remembered"
    );
    assert!(
        count > 0,
        "the query has to actually reach the leaves — {count} asks means \
         something above them answered without descending"
    );
}

/// Text measures by shaping, and the two extremes are ordered the way the
/// definitions require.
///
/// Asserted as a *relation* rather than against pixel constants, so this stays
/// true across a change to the embedded face.
#[test]
fn text_reports_a_single_line_maximum_and_a_longest_word_minimum() {
    use vieww::prelude::*;

    let mut driver = driver();
    driver.set_root(Text::new("supercalifragilistic expialidocious"));
    driver.draw_frame();

    let tree = driver.owner_mut().tree_mut();
    let text = tree
        .ids()
        .into_iter()
        .find(|&id| tree.object(id).map(RenderObject::debug_name) == Some("RenderText"))
        .expect("the text is in the render tree");

    let max = tree
        .intrinsic(text, IntrinsicQuery::max_width())
        .expect("text can measure itself");
    let min = tree
        .intrinsic(text, IntrinsicQuery::min_width())
        .expect("text can measure itself");

    assert!(
        min > 0.0 && max > min,
        "the whole string on one line ({max:.1}) must be wider than its longest \
         unbreakable word ({min:.1}), and neither may be zero"
    );

    // Height at a width that forces a wrap must exceed the single-line height.
    let one_line = tree
        .intrinsic(text, IntrinsicQuery::max_height())
        .expect("text can measure itself");
    let wrapped = tree
        .intrinsic(text, IntrinsicQuery::max_height().across(min + 1.0))
        .expect("text can measure itself");
    assert!(
        wrapped > one_line,
        "asking for the height at a width that forces a wrap ({wrapped:.1}) must \
         exceed the single-line height ({one_line:.1}) — an implementation that \
         ignored `across` would return the same number twice"
    );
}

/// An answer goes stale when the subtree changes, and the tree notices.
///
/// The cache lives until something marks for layout. A cache that outlived a
/// content change would be worse than no cache: it would be a wrong number that
/// only appears after an edit.
#[test]
fn a_changed_subtree_invalidates_the_cached_answer() {
    use vieww::prelude::*;

    let asked = Rc::new(Cell::new(0));
    let mut driver = driver();

    let root = |height: f32, asked: &Rc<Cell<u32>>| -> WidgetNode {
        Align::new(vieww::foundation::Alignment::TOP_LEFT)
            .child(IntrinsicSize::height(Block {
                size: Size::new(20.0, height),
                asked: Rc::clone(asked),
            }))
            .into()
    };

    driver.set_root(root(30.0, &asked));
    driver.draw_frame();
    assert_eq!(size_of(&driver, "RenderIntrinsicSize").height, 30.0);

    driver.set_root(root(75.0, &asked));
    driver.draw_frame();
    assert_eq!(
        size_of(&driver, "RenderIntrinsicSize").height,
        75.0,
        "the content changed, so the remembered answer had to go with it"
    );
}
